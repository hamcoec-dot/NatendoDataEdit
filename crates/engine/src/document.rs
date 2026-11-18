//! An open document: file bytes + row index + edits + undo history.
//!
//! The original bytes are never modified. Rows are identified by a `u32` id:
//! original rows are `0..n` in file order, inserted rows get ids from `n` up.
//! The document's row order is the identity until the first insert or delete,
//! after which it is an explicit list of ids. Edited and inserted rows live in
//! the overlay as parsed values; every other row costs only its offset.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

use crate::detect::{self, Detection, Hints};
use crate::dialect::{Dialect, LineBreak};
use crate::encoding::{self, Encoding};
use crate::filter::FilterSpec;
use crate::index::RowIndex;
use crate::scan::{self, Mode};
use crate::source::Source;
use crate::Error;

/// Highest column index an edit may write (keeps a typo from allocating
/// millions of empty cells).
pub const MAX_EDIT_COL: u32 = (1 << 20) - 1;
/// Undo steps kept; older ones are forgotten.
pub const MAX_UNDO: usize = 10_000;
/// Approximate memory the undo and redo history may use; the oldest steps
/// are forgotten beyond it (the latest step is always kept).
pub const MAX_UNDO_BYTES: usize = 512 << 20;
/// Bytes indexed per step by [`Document::open_indexed`] between cancel checks.
pub const INDEX_STEP: usize = 64 << 20;

/// How a file's text is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub encoding: Encoding,
    pub bom: bool,
    pub dialect: Dialect,
    /// Line break for new and edited rows (the file's most common one).
    pub line_break: LineBreak,
}

/// Choices that override detection when opening.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenOptions {
    pub encoding: Option<Encoding>,
    /// The whole dialect (separator, quote, escape). Wins over `delimiter`
    /// and `quote`, and turns detection off.
    pub dialect: Option<Dialect>,
    /// A one-byte separator (detection off; quote from `quote`).
    pub delimiter: Option<u8>,
    /// `Some(None)` means "no quoting". `None`: detected, or the default
    /// `"` when the separator is given.
    pub quote: Option<Option<u8>>,
    /// Lines above the table to keep aside (`None`: detected when the
    /// dialect is, else none).
    pub skip_lines: Option<u32>,
    /// Read every row line by line: quoted cells never span lines. For files
    /// where a stray quote swallowed rows.
    pub line_by_line: bool,
    /// Memory cap for the row index (default `index::DEFAULT_INDEX_BUDGET`).
    pub max_index_bytes: Option<usize>,
}

/// Rows with more fields than this cannot be edited or listed as a whole
/// (one 50 MB line of commas would otherwise need gigabytes).
pub const MAX_ROW_FIELDS: u64 = 1 << 20;

/// How an edited row is held.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Edit {
    /// Every value (inserted rows, and rows edited cell by cell).
    Full(Vec<String>),
    /// An original row with some cells changed (bulk changes such as
    /// replace-all): the file's bytes plus only the new cells, sorted by
    /// physical column. `width` is the row's field count after the change.
    Patch {
        width: u32,
        cells: Box<[(u32, Box<str>)]>,
    },
}

impl Edit {
    fn width(&self) -> u32 {
        match self {
            Edit::Full(v) => v.len() as u32,
            Edit::Patch { width, .. } => *width,
        }
    }

    fn cost(&self) -> usize {
        match self {
            Edit::Full(v) => v.iter().map(|s| 24 + s.len()).sum(),
            Edit::Patch { cells, .. } => {
                16 + cells.iter().map(|(_, s)| 24 + s.len()).sum::<usize>()
            }
        }
    }
}

/// Overlay of edited rows, tracking the widest row cheaply.
#[derive(Default)]
struct Overlay {
    rows: HashMap<u32, Edit>,
    widths: BTreeMap<u32, u32>,
}

impl Overlay {
    fn get(&self, id: u32) -> Option<&Edit> {
        self.rows.get(&id)
    }

    fn set(&mut self, id: u32, value: Option<Edit>) -> Option<Edit> {
        let old = match value {
            Some(v) => {
                *self.widths.entry(v.width()).or_default() += 1;
                self.rows.insert(id, v)
            }
            None => {
                let old = self.rows.remove(&id);
                // After a big bulk change is undone the table would stay huge
                // and every lookup (each saved row does one) would miss the
                // cache; give the memory back once it is mostly empty.
                let cap = self.rows.capacity();
                if cap > 4096 && self.rows.len() * 8 < cap {
                    self.rows.shrink_to(self.rows.len() * 2);
                }
                old
            }
        };
        if let Some(o) = &old {
            let w = o.width();
            if let Some(n) = self.widths.get_mut(&w) {
                *n -= 1;
                if *n == 0 {
                    self.widths.remove(&w);
                }
            }
        }
        old
    }

    fn max_width(&self) -> u32 {
        self.widths.keys().next_back().copied().unwrap_or(0)
    }

    fn len(&self) -> usize {
        self.rows.len()
    }
}

/// One undoable change. Applying an op returns its exact inverse.
enum Op {
    /// Set the overlay state of rows (None = back to the original bytes).
    Restore(Vec<(u32, Option<Edit>)>),
    /// Put rows (with their overlay states) back into the order at `at`.
    Insert {
        at: usize,
        rows: Vec<(u32, Option<Edit>)>,
    },
    /// Replace the whole row order (`None` = file order). Sorting.
    SetOrder(Option<Vec<u32>>),
    /// Take `count` rows out of the order at `at`.
    Remove { at: usize, count: usize },
    /// Move `count` rows at `at` so they start at `to` (positions after removal).
    MoveRows { at: usize, count: usize, to: usize },
    /// Put physical columns into the column map at `at`.
    ColInsert { at: usize, ids: Vec<u32> },
    /// Take `count` columns out of the column map at `at`.
    ColRemove { at: usize, count: usize },
    /// Move `count` columns at `at` so they start at `to` (positions after removal).
    MoveCols { at: usize, count: usize, to: usize },
    /// Replace the whole column map (`None` = no column changes). The first
    /// column op of a document starts with this, so undoing it removes the map.
    SetMap(Option<Vec<u32>>),
    /// Replace the set of hidden rows (`None` = none hidden).
    SetHidden(Option<Arc<Vec<u64>>>),
    /// Replace the filter (`None` = no filter; hidden rows stay as they
    /// are). Turning a filter on, changing it or clearing it is an undo step
    /// that changes no data.
    SetFilter(Option<Box<FilterPart>>),
    /// Several ops applied in order as one undo step.
    Batch(Vec<Op>),
}

/// A filter's rules and result, as an undo step keeps them (see [`View`]).
struct FilterPart {
    spec: FilterSpec,
    bits: Vec<u64>,
    base: u32,
}

impl Op {
    /// Changes only what shows (a filter, hidden rows), not the data: such
    /// steps do not make the document unsaved.
    fn view_only(&self) -> bool {
        match self {
            Op::SetHidden(_) | Op::SetFilter(_) => true,
            Op::Batch(ops) => ops.iter().all(Op::view_only),
            _ => false,
        }
    }

    /// Approximate bytes this op holds (for the undo memory budget).
    fn cost(&self) -> usize {
        fn row(r: &Option<Edit>) -> usize {
            r.as_ref().map_or(0, Edit::cost)
        }
        match self {
            Op::Restore(s) | Op::Insert { rows: s, .. } => {
                32 + s.iter().map(|(_, r)| 32 + row(r)).sum::<usize>()
            }
            Op::ColInsert { ids, .. } => 32 + ids.len() * 4,
            Op::SetMap(m) | Op::SetOrder(m) => 32 + m.as_ref().map_or(0, |m| m.len() * 4),
            Op::SetHidden(h) => 32 + h.as_ref().map_or(0, |h| h.len() * 8),
            Op::SetFilter(f) => {
                32 + f
                    .as_ref()
                    .map_or(0, |f| 64 + f.bits.len() * 8 + f.spec.columns.len() * 128)
            }
            Op::Batch(ops) => 32 + ops.iter().map(Op::cost).sum::<usize>(),
            Op::Remove { .. }
            | Op::MoveRows { .. }
            | Op::ColRemove { .. }
            | Op::MoveCols { .. } => 32,
        }
    }
}

/// Parts of one undo step built by a bulk operation (see
/// [`Document::push_step`]).
#[derive(Default)]
pub(crate) struct Step {
    pub col_insert: Option<(u32, Vec<u32>)>,
    pub states: Vec<(u32, Option<Edit>)>,
    pub col_remove: Vec<(u32, u32)>,
    pub map: Option<Vec<u32>>,
    pub order: Option<Vec<u32>>,
    /// New rows put at the end of the document (after everything above).
    pub append: Vec<(u32, Option<Edit>)>,
}

/// Most columns a column operation can handle (the column map stores one
/// `u32` per column).
pub const MAX_MAPPED_COLS: u32 = MAX_EDIT_COL + 1;

/// Where the table starts in a source, and the lines above it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Start {
    pub data: usize,
    pub preamble_len: usize,
    pub preamble_lines: u32,
}

/// Where a row's values come from.
pub(crate) enum RowSource<'a> {
    Edited(Cow<'a, [String]>),
    Raw { bytes: &'a [u8], mode: Mode },
}

/// Progress and cancel flag of a long operation (sort, filter, find,
/// replace). Shared with the caller, who may read it from another thread.
#[derive(Debug, Default)]
pub struct Progress {
    /// Rows processed so far.
    pub done: AtomicU64,
    /// Rows to process (0 while unknown).
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn new() -> Progress {
        Progress::default()
    }

    /// Fraction done, 0.0 to 1.0.
    pub fn fraction(&self) -> f64 {
        let t = self.total.load(Ordering::Relaxed);
        if t == 0 {
            0.0
        } else {
            (self.done.load(Ordering::Relaxed) as f64 / t as f64).min(1.0)
        }
    }

    pub fn start(&self, total: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }

    /// Count `n` more rows done; `Err(Cancelled)` once cancel is set.
    pub fn step(&self, n: u64) -> Result<(), Error> {
        self.done.fetch_add(n, Ordering::Relaxed);
        if self.cancel.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// A filter's result: which rows show. Fixed when the filter is applied
/// (editing a row does not hide it until the filter is applied again).
pub(crate) struct View {
    /// The rules, by physical column.
    pub spec: FilterSpec,
    /// One bit per row id below `base`: set = shown. Ids from `base` up were
    /// added after the filter was applied and always show.
    pub bits: Vec<u64>,
    pub base: u32,
    /// Rows the user hid (one bit per row id; ids past the end are not
    /// hidden). Kept when the filter changes; `None` = no hidden rows.
    pub hidden: Option<Arc<Vec<u64>>>,
    /// Document positions of the shown rows (rebuilt after row moves).
    visible: OnceLock<Vec<u32>>,
    /// Rows now in the document that are hidden (rebuilt with `visible`).
    hidden_rows: OnceLock<u64>,
}

impl View {
    pub(crate) fn new(spec: FilterSpec, bits: Vec<u64>, base: u32) -> View {
        View {
            spec,
            bits,
            base,
            hidden: None,
            visible: OnceLock::new(),
            hidden_rows: OnceLock::new(),
        }
    }

    /// A view that filters nothing (it only carries hidden rows).
    pub(crate) fn all() -> View {
        View::new(FilterSpec::default(), Vec::new(), 0)
    }

    /// Shown: passes the filter and is not hidden.
    pub(crate) fn shows(&self, id: u32) -> bool {
        self.passes(id) && !self.is_hidden(id)
    }

    /// Passes the filter (hidden or not).
    pub(crate) fn passes(&self, id: u32) -> bool {
        id >= self.base || (self.bits[(id >> 6) as usize] >> (id & 63)) & 1 == 1
    }

    pub(crate) fn is_hidden(&self, id: u32) -> bool {
        self.hidden.as_ref().is_some_and(|h| {
            h.get((id >> 6) as usize)
                .is_some_and(|w| (w >> (id & 63)) & 1 == 1)
        })
    }

    /// Whether this view filters anything (a filter, show duplicates, show
    /// problems), apart from hidden rows.
    pub(crate) fn filters(&self) -> bool {
        self.base > 0 || !self.spec.columns.is_empty()
    }
}

/// An open delimited-text document.
pub struct Document {
    pub(crate) source: Source,
    /// Offset of the table in `source` (after a BOM for direct encodings,
    /// and after the preamble).
    pub(crate) data_start: usize,
    /// Bytes just before `data_start` that hold the lines above the table.
    pub(crate) preamble_len: usize,
    pub(crate) preamble_lines: u32,
    /// What detection found, when it ran.
    detection: Option<Detection>,
    pub(crate) format: Format,
    /// Decoding a transcoded file replaced invalid sequences.
    pub(crate) lossy: bool,
    /// UTF-8 was detected although the sample had some invalid bytes.
    pub(crate) invalid_bytes: bool,
    pub(crate) index: RowIndex,
    order: Option<Vec<u32>>,
    overlay: Overlay,
    next_id: u32,
    /// Column map: logical column -> physical column. `None` is the identity.
    /// Physical columns past the widest row are new (inserted) ones.
    cols: Option<Vec<u32>>,
    /// Next physical id for an inserted column (never reused until a save).
    next_col: u32,
    /// Columns cleared by swapping in a new empty column (a quick clear of
    /// whole columns): new physical id -> the file's column it replaced, so
    /// the list of changes can say "cleared" instead of "deleted, added".
    /// Kept through undo and redo (a pair only counts while the new column
    /// is in the map and the old one is not); forgotten on save.
    pub(crate) swapped: HashMap<u32, u32>,
    /// Cached: the column map is exactly the identity over every physical
    /// column, so it changes nothing (cleared whenever anything changes).
    /// 0 = not known, 1 = no, 2 = yes. Atomic so a document can be read
    /// from several threads at once.
    map_identity: AtomicU8,
    /// The filter, if one is on.
    view: Option<View>,
    /// Bumped by every change, so a long operation computed under a read
    /// lock can tell whether the document changed before it applies.
    revision: u64,
    undo_bytes: usize,
    redo_bytes: usize,
    /// Memory budget for the history (MAX_UNDO_BYTES; smaller in tests).
    pub(crate) undo_budget: usize,
    undo: Vec<Op>,
    redo: Vec<Op>,
    clean_depth: Option<usize>,
    /// Changes done minus changes undone (never lowered by trimming the
    /// history or saving), so a step's place in the history can be told.
    history_pos: u64,
    /// Protected columns (physical ids, sorted): every change of cells
    /// passes them over. Renaming a column (its title cell) still works.
    protected: Vec<u32>,
    /// Cells the last change left alone because their column is protected.
    protected_skipped: AtomicU64,
}

impl Document {
    /// Open a file. Detection runs now; call [`Document::index_step`] until
    /// it returns `true` (or use [`Document::open_indexed`]).
    pub fn open(path: &Path, opts: OpenOptions) -> Result<Document, Error> {
        let source = Source::open(path)?;
        let mut ext = path.extension().and_then(|e| e.to_str());
        // voters.tsv.gz: the type is the one before ".gz".
        if ext.is_some_and(|e| e.eq_ignore_ascii_case("gz")) {
            ext = path
                .file_stem()
                .map(Path::new)
                .and_then(|s| s.extension())
                .and_then(|e| e.to_str());
        }
        Document::from_source(source, opts, ext)
    }

    /// Open and fully index a file, checking `cancel` between steps.
    pub fn open_indexed(
        path: &Path,
        opts: OpenOptions,
        cancel: Option<&AtomicBool>,
    ) -> Result<Document, Error> {
        let mut doc = Document::open(path, opts)?;
        while !doc.index_step(INDEX_STEP)? {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Cancelled);
            }
        }
        Ok(doc)
    }

    /// A fully indexed document over in-memory bytes (tests, clipboard, new files).
    pub fn from_bytes(
        bytes: Vec<u8>,
        opts: OpenOptions,
        ext: Option<&str>,
    ) -> Result<Document, Error> {
        let mut doc = Document::from_source(Source::from_vec(bytes), opts, ext)?;
        doc.index_step(usize::MAX)?;
        Ok(doc)
    }

    pub(crate) fn from_source(
        source: Source,
        opts: OpenOptions,
        ext: Option<&str>,
    ) -> Result<Document, Error> {
        let raw = source.bytes();
        let found_bom = encoding::bom_of(raw);
        let (encoding, invalid_bytes) = match opts.encoding {
            Some(e) => (e, false),
            None => {
                let d = encoding::detect(raw);
                (d.encoding, d.invalid_bytes)
            }
        };
        let (bom, bom_len) = match found_bom {
            Some(d) if d.encoding == encoding => (true, d.bom_len),
            _ => (false, 0),
        };
        let (source, data_start, lossy) = if encoding.is_direct() {
            (source, bom_len, false)
        } else {
            let (text, lossy) = encoding::decode_all(&raw[bom_len..], encoding);
            (source.with_bytes(text.into_bytes()), 0, lossy)
        };
        let data = &source.bytes()[data_start..];
        let (dialect, skip, detection) = match (opts.dialect, opts.delimiter) {
            (Some(d), _) => (d, opts.skip_lines.unwrap_or(0), None),
            (None, Some(sep)) => (
                Dialect::new(sep, opts.quote.unwrap_or(Some(b'"')))?,
                opts.skip_lines.unwrap_or(0),
                None,
            ),
            (None, None) => {
                let found = detect::detect(
                    data,
                    Hints {
                        ext,
                        quote: opts.quote,
                    },
                );
                let best = found.best();
                (
                    best.dialect,
                    opts.skip_lines.unwrap_or(best.skip_lines),
                    Some(found),
                )
            }
        };
        let preamble_len = detect::skip_lines_len(data, skip);
        // Fewer than asked when the file ends first.
        let pre = &data[..preamble_len];
        let preamble_lines = (scan::line_breaks_in(pre)
            + usize::from(!pre.is_empty() && LineBreak::at_end_of(pre).is_none()))
            as u32;
        let data_start = data_start + preamble_len;
        let index = RowIndex::new(
            dialect,
            data_start,
            opts.line_by_line,
            opts.max_index_bytes
                .unwrap_or(crate::index::DEFAULT_INDEX_BUDGET),
        );
        Ok(Document {
            source,
            data_start,
            preamble_len,
            preamble_lines,
            detection,
            format: Format {
                encoding,
                bom,
                dialect,
                line_break: LineBreak::CrLf,
            },
            lossy,
            invalid_bytes,
            index,
            order: None,
            overlay: Overlay::default(),
            next_id: 0,
            cols: None,
            next_col: 0,
            swapped: HashMap::new(),
            map_identity: AtomicU8::new(0),
            view: None,
            revision: 0,
            undo_bytes: 0,
            redo_bytes: 0,
            undo_budget: MAX_UNDO_BYTES,
            undo: Vec::new(),
            redo: Vec::new(),
            clean_depth: Some(0),
            history_pos: 0,
            protected: Vec::new(),
            protected_skipped: AtomicU64::new(0),
        })
    }

    /// Protect these columns (logical numbers) from changes, and no others.
    /// Protection follows a column when columns move.
    pub fn set_protected(&mut self, cols: &[u32]) {
        let mut p: Vec<u32> = cols.iter().filter_map(|&c| self.phys(c)).collect();
        p.sort_unstable();
        p.dedup();
        self.protected = p;
    }

    /// The protected columns, by logical number (in order).
    pub fn protected_cols(&self) -> Vec<u32> {
        if self.protected.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<u32> = (0..self.col_count())
            .filter(|&c| {
                self.phys(c)
                    .is_some_and(|p| self.protected.binary_search(&p).is_ok())
            })
            .collect();
        out.sort_unstable();
        out
    }

    /// Whether logical column `col` is protected.
    pub fn is_protected(&self, col: u32) -> bool {
        !self.protected.is_empty()
            && self
                .phys(col)
                .is_some_and(|p| self.protected.binary_search(&p).is_ok())
    }

    /// Cells the last change left alone in protected columns.
    pub fn protected_skipped(&self) -> u64 {
        self.protected_skipped.load(Ordering::Relaxed)
    }

    /// Drop the cells of protected columns (physical ids) from a change,
    /// counting them.
    fn drop_protected(&self, changes: &mut Vec<(u32, Vec<(u32, String)>)>) {
        self.protected_skipped.store(0, Ordering::Relaxed);
        if self.protected.is_empty() {
            return;
        }
        let mut skipped = 0u64;
        for (_, cells) in changes.iter_mut() {
            let before = cells.len();
            cells.retain(|(c, _)| self.protected.binary_search(c).is_err());
            skipped += (before - cells.len()) as u64;
        }
        changes.retain(|(_, cells)| !cells.is_empty());
        self.protected_skipped.store(skipped, Ordering::Relaxed);
    }

    /// `Err` when a change came to nothing because every cell it would
    /// change is in a protected column (`left`: nothing else is done).
    pub(crate) fn only_protected(&self, left: bool) -> Result<(), Error> {
        if left && self.protected_skipped() > 0 {
            return Err(Error::Invalid(
                "those cells are in protected columns, so nothing changed (right-click a heading › Protect column to switch it off)".into(),
            ));
        }
        Ok(())
    }

    /// Index about `budget` more bytes. Returns `true` when indexing is done.
    pub fn index_step(&mut self, budget: usize) -> Result<bool, Error> {
        if self.index.is_complete() {
            return Ok(true);
        }
        let done = self.index.step(self.source.bytes(), budget)?;
        self.revision += 1;
        if done {
            self.next_id = self.index.rows() as u32;
            if let Some(lb) = self.index.stats().dominant_line_break() {
                self.format.line_break = lb;
            }
        }
        Ok(done)
    }

    pub fn is_indexed(&self) -> bool {
        self.index.is_complete()
    }

    /// Fraction of the file indexed, 0.0 to 1.0.
    pub fn index_progress(&self) -> f64 {
        let total = self.source.bytes().len().saturating_sub(self.data_start);
        if total == 0 || self.index.is_complete() {
            return 1.0;
        }
        (self.index.reached() - self.data_start) as f64 / total as f64
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// What detection found when the file was opened (`None` when the
    /// caller fixed the dialect).
    pub fn detection(&self) -> Option<&Detection> {
        self.detection.as_ref()
    }

    /// Run layout detection on this document's file again (cheap: it reads
    /// at most about 100 KB), for offering other layouts.
    pub fn detect_layout(&self, ext: Option<&str>) -> Detection {
        let data = &self.source.bytes()[self.data_start - self.preamble_len..];
        detect::detect(data, Hints { ext, quote: None })
    }

    /// The lines above the table, as working bytes (kept unchanged on save).
    pub fn preamble(&self) -> &[u8] {
        &self.source.bytes()[self.data_start - self.preamble_len..self.data_start]
    }

    /// Number of lines above the table.
    pub fn preamble_lines(&self) -> u32 {
        self.preamble_lines
    }

    /// The lines above the table as text.
    pub fn preamble_text(&self) -> String {
        encoding::decode_direct(self.preamble(), self.working_encoding()).into_owned()
    }

    /// The first `rows` rows of this document's file read with `dialect`
    /// after skipping `skip_lines` lines, each cut to `max_cols` cells of at
    /// most `max_chars` characters. For previewing another way to open the
    /// file; reads at most 256 KB.
    pub fn preview(
        &self,
        dialect: Dialect,
        skip_lines: u32,
        rows: usize,
        max_cols: usize,
        max_chars: usize,
    ) -> Vec<Vec<String>> {
        let all = self.source.bytes();
        let start = self.data_start - self.preamble_len;
        let data = &all[start..all.len().min(start.saturating_add(256 * 1024))];
        let mut pos = detect::skip_lines_len(data, skip_lines);
        let mut out = Vec::new();
        let mut mode = Mode::Normal;
        while pos < data.len() && out.len() < rows {
            let mut cells = Vec::new();
            let end = scan::scan_row(data, pos, dialect, mode, |s, e| {
                if cells.len() < max_cols {
                    let v = scan::unquote(&data[s..e], dialect);
                    let text = encoding::decode_direct(&v, self.working_encoding());
                    cells.push(text.chars().take(max_chars).collect());
                }
                true
            });
            if end.unterminated {
                mode = Mode::LineBound;
                continue;
            }
            pos = end.next;
            out.push(cells);
        }
        out
    }

    /// True when decoding replaced invalid byte sequences (saving rewrites them).
    pub fn is_lossy(&self) -> bool {
        self.lossy
    }

    pub fn path(&self) -> Option<&Path> {
        self.source.path()
    }

    /// The open file now has this name (renamed on disk by the caller):
    /// later saves and change checks use it.
    pub fn renamed(&mut self, path: PathBuf) {
        self.source.set_path(path);
    }

    /// Rename (move) the open file to `to`, never replacing a file there.
    /// A large file is mapped and locked against moves, so the mapping is
    /// let go first and taken again under the new name; its size and time
    /// must still match what was open, or the rename is reported as failed
    /// (the old mapping is restored when the file is still at its old name).
    pub fn rename_to(&mut self, to: &Path) -> std::io::Result<()> {
        let Some(from) = self.source.path().map(Path::to_path_buf) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "this document has no file yet",
            ));
        };
        if !self.source.is_mapped() {
            crate::save::rename_no_replace(&from, to)?;
            self.renamed(to.to_path_buf());
            return Ok(());
        }
        let stamp = self.source.stamp();
        let len = self.source.bytes().len();
        drop(self.take_source());
        let reopen = |p: &Path| -> std::io::Result<Source> {
            let s = Source::open(p)?;
            if s.stamp() != stamp || s.bytes().len() != len {
                return Err(std::io::Error::other(
                    "the file changed on disk during the rename",
                ));
            }
            Ok(s)
        };
        // Only the very same file (size and time) is taken back. When it
        // cannot be, the document becomes empty (the file itself is never
        // touched) and the error says where the file is now.
        let lost = |this: &mut Self, at: &Path, e: std::io::Error| {
            this.make_empty(this.format);
            std::io::Error::other(format!(
                "the file is at {} but could not be read again ({e}); open it from there",
                at.display()
            ))
        };
        match crate::save::rename_no_replace(&from, to) {
            Ok(()) => match reopen(to) {
                Ok(s) => {
                    self.put_source(s);
                    Ok(())
                }
                Err(e) => {
                    // Move it back and continue from the old name if possible.
                    if crate::save::rename_no_replace(to, &from).is_ok() {
                        if let Ok(s) = reopen(&from) {
                            self.put_source(s);
                            return Err(e);
                        }
                        return Err(lost(self, &from, e));
                    }
                    Err(lost(self, to, e))
                }
            },
            Err(e) => match reopen(&from) {
                Ok(s) => {
                    self.put_source(s);
                    Err(e)
                }
                Err(e2) => Err(lost(self, &from, e2)),
            },
        }
    }

    pub fn index(&self) -> &RowIndex {
        &self.index
    }

    /// Rows shown: every row, or with a filter on, the rows it shows. Row
    /// numbers in the public API count shown rows.
    pub fn row_count(&self) -> u64 {
        match &self.view {
            Some(v) => self.visible(v).len() as u64,
            None => self.total_rows(),
        }
    }

    /// Every row in the document (indexed so far while loading), shown or not.
    pub fn total_rows(&self) -> u64 {
        match &self.order {
            Some(o) => o.len() as u64,
            None => self.index.rows(),
        }
    }

    /// Positions of the rows a filter shows.
    pub(crate) fn visible<'a>(&'a self, v: &'a View) -> &'a [u32] {
        v.visible.get_or_init(|| {
            let mut out = Vec::new();
            for (pos, id) in self.ids().enumerate() {
                if v.shows(id) {
                    out.push(pos as u32);
                }
            }
            out
        })
    }

    /// True while some rows are not shown (a filter is on or rows are hidden).
    pub fn is_filtered(&self) -> bool {
        self.view.is_some()
    }

    /// True while a filter is on (hidden rows aside).
    pub fn has_filter(&self) -> bool {
        self.view.as_ref().is_some_and(View::filters)
    }

    /// Rows the user hid that are still in the document.
    pub fn hidden_rows(&self) -> u64 {
        let Some(v) = &self.view else { return 0 };
        if v.hidden.is_none() {
            return 0;
        }
        *v.hidden_rows
            .get_or_init(|| self.ids().filter(|&id| v.is_hidden(id)).count() as u64)
    }

    pub(crate) fn hidden_bits(&self) -> Option<&Arc<Vec<u64>>> {
        self.view.as_ref().and_then(|v| v.hidden.as_ref())
    }

    /// Change which rows are hidden, as one undo step.
    pub(crate) fn set_hidden(&mut self, hidden: Option<Arc<Vec<u64>>>) {
        self.push(Op::SetHidden(hidden));
    }

    /// The row's number in the whole document (0-based), for showing real
    /// row numbers while a filter is on.
    pub fn row_number(&self, row: u64) -> Option<u64> {
        match &self.view {
            Some(v) => self
                .visible(v)
                .get(usize::try_from(row).ok()?)
                .map(|&p| p as u64),
            None => (row < self.total_rows()).then_some(row),
        }
    }

    /// The shown row with document row number `number` (0-based), or, when
    /// that row is hidden by a filter, the next shown one (the last shown
    /// row when none follows). `None` when no row is shown.
    pub fn shown_row_of(&self, number: u64) -> Option<(u64, bool)> {
        match &self.view {
            Some(v) => {
                let vis = self.visible(v);
                if vis.is_empty() {
                    return None;
                }
                let n = u32::try_from(number).unwrap_or(u32::MAX);
                Some(match vis.binary_search(&n) {
                    Ok(i) => (i as u64, true),
                    Err(i) => (i.min(vis.len() - 1) as u64, false),
                })
            }
            None => {
                let total = self.total_rows();
                (total > 0).then(|| (number.min(total - 1), number < total))
            }
        }
    }

    /// Changes whenever the document changes (edits, undo, filters, saves).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn view(&self) -> Option<&View> {
        self.view.as_ref()
    }

    /// Turn a filter on (or replace or remove it): one undo step, as in a
    /// spreadsheet, though it changes no data (it leaves the document saved).
    /// Hidden rows stay hidden whatever the new filter is.
    pub(crate) fn set_view(&mut self, view: Option<View>) {
        let part = view.filter(View::filters).map(|v| {
            Box::new(FilterPart {
                spec: v.spec,
                bits: v.bits,
                base: v.base,
            })
        });
        if part.is_none() && !self.has_filter() {
            // Clearing a filter that is not on changes nothing.
            return;
        }
        self.push(Op::SetFilter(part));
    }

    /// Columns: the widest row in the file or in the edits, or the column
    /// map's length once columns were inserted, deleted or moved.
    pub fn col_count(&self) -> u32 {
        match &self.cols {
            Some(m) => m.len() as u32,
            None => self.physical_width(),
        }
    }

    /// Physical columns in use by the file and the edits.
    fn physical_width(&self) -> u32 {
        self.index.stats().max_fields.max(self.overlay.max_width())
    }

    /// True while columns are inserted, deleted or moved (saving then
    /// rewrites every row). False again once those changes are undone.
    pub fn has_column_changes(&self) -> bool {
        self.col_map().is_some()
    }

    /// Physical column of logical column `col` (`None` past the map's end).
    pub(crate) fn phys(&self, col: u32) -> Option<u32> {
        match &self.cols {
            Some(m) => m.get(col as usize).copied(),
            None => Some(col),
        }
    }

    /// The column map, if it changes anything. A map that is the identity
    /// over every physical column (for example after inserting a column and
    /// deleting it again) counts as no change, so rows save byte-for-byte.
    pub(crate) fn col_map(&self) -> Option<&[u32]> {
        let m = self.cols.as_deref()?;
        let identity = match self.map_identity.load(Ordering::Relaxed) {
            1 => false,
            2 => true,
            _ => {
                let b = m.len() as u32 == self.physical_width()
                    && m.iter().enumerate().all(|(i, &p)| p as usize == i);
                self.map_identity.store(1 + b as u8, Ordering::Relaxed);
                b
            }
        };
        (!identity).then_some(m)
    }

    pub fn is_modified(&self) -> bool {
        // Unsaved unless every step between the saved state and now only
        // changed what shows (a filter or hidden rows).
        let Some(d) = self.clean_depth else {
            return true;
        };
        let n = self.undo.len();
        if d <= n {
            return !self.undo[d..].iter().all(Op::view_only);
        }
        let k = d - n;
        k > self.redo.len() || !self.redo[self.redo.len() - k..].iter().all(Op::view_only)
    }

    /// Changes done minus changes undone (see `history_pos`).
    pub fn history_pos(&self) -> u64 {
        self.history_pos
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Rows held as parsed values (edited or inserted).
    pub fn edited_rows(&self) -> usize {
        self.overlay.len()
    }

    /// Encoding used to decode working bytes.
    pub(crate) fn working_encoding(&self) -> Encoding {
        if self.format.encoding.is_direct() {
            self.format.encoding
        } else {
            Encoding::Utf8
        }
    }

    /// Id of the shown row `row` (see [`Document::row_count`]).
    pub(crate) fn id_at(&self, row: u64) -> Option<u32> {
        match &self.view {
            Some(v) => {
                let pos = *self.visible(v).get(usize::try_from(row).ok()?)?;
                self.doc_id_at(pos as u64)
            }
            None => self.doc_id_at(row),
        }
    }

    /// Id of the row at document position `row` (ignoring any filter).
    pub(crate) fn doc_id_at(&self, row: u64) -> Option<u32> {
        match &self.order {
            Some(o) => o.get(usize::try_from(row).ok()?).copied(),
            None => (row < self.index.rows()).then_some(row as u32),
        }
    }

    pub(crate) fn row_source(&self, id: u32) -> RowSource<'_> {
        match self.overlay.get(id) {
            Some(Edit::Full(v)) => RowSource::Edited(Cow::Borrowed(v)),
            Some(Edit::Patch { width, cells }) => {
                RowSource::Edited(Cow::Owned(self.patched(id, *width, cells)))
            }
            None => {
                let (bytes, mode) = self.raw_row(id);
                RowSource::Raw { bytes, mode }
            }
        }
    }

    /// The file bytes of an original row.
    pub(crate) fn raw_row(&self, id: u32) -> (&[u8], Mode) {
        let (a, b) = self
            .index
            .row_range(id as u64)
            .expect("ids not in the overlay are original rows");
        (&self.source.bytes()[a..b], self.index.mode_of(id as u64))
    }

    /// Every value of a patched row.
    fn patched(&self, id: u32, width: u32, cells: &[(u32, Box<str>)]) -> Vec<String> {
        let (bytes, mode) = self.raw_row(id);
        // A patch is only made for a row that parsed within MAX_ROW_FIELDS,
        // and the file's bytes never change, so this cannot fail.
        let mut v = self
            .original_fields(id, bytes, mode)
            .expect("patched rows were parsed when patched");
        v.resize(width as usize, String::new());
        for (c, s) in cells.iter() {
            v[*c as usize] = s.to_string();
        }
        v
    }

    /// One field's text, borrowed when no unquoting or decoding was needed.
    pub(crate) fn field_text<'a>(&self, raw: &'a [u8]) -> Cow<'a, str> {
        match scan::unquote(raw, self.format.dialect) {
            Cow::Borrowed(b) => encoding::decode_direct(b, self.working_encoding()),
            Cow::Owned(v) => {
                Cow::Owned(encoding::decode_direct(&v, self.working_encoding()).into_owned())
            }
        }
    }

    /// The text of physical columns `wanted` (sorted, no repeats) of row `id`,
    /// into `out` (one entry per wanted column; empty past the row's end).
    pub(crate) fn read_cells<'a>(&'a self, id: u32, wanted: &[u32], out: &mut Vec<Cow<'a, str>>) {
        out.clear();
        out.resize(wanted.len(), Cow::Borrowed(""));
        let patch = match self.overlay.get(id) {
            Some(Edit::Full(v)) => {
                for (k, &p) in wanted.iter().enumerate() {
                    if let Some(s) = v.get(p as usize) {
                        out[k] = Cow::Borrowed(s.as_str());
                    }
                }
                return;
            }
            Some(Edit::Patch { cells, .. }) => Some(cells),
            None => None,
        };
        let (bytes, mode) = self.raw_row(id);
        let mut i = 0u32;
        let mut k = 0usize;
        scan::scan_row(bytes, 0, self.format.dialect, mode, |s, e| {
            if k >= wanted.len() {
                return false;
            }
            if wanted[k] == i {
                out[k] = self.field_text(&bytes[s..e]);
                k += 1;
            }
            i += 1;
            true
        });
        if let Some(cells) = patch {
            for (k, &p) in wanted.iter().enumerate() {
                if let Ok(j) = cells.binary_search_by_key(&p, |c| c.0) {
                    out[k] = Cow::Borrowed(&cells[j].1);
                }
            }
        }
    }

    /// Decode one raw field.
    pub(crate) fn decode_field(&self, raw: &[u8]) -> String {
        let v = scan::unquote(raw, self.format.dialect);
        encoding::decode_direct(&v, self.working_encoding()).into_owned()
    }

    /// Every value of an original row, parsed from the file. Refuses rows
    /// wider than [`MAX_ROW_FIELDS`] before allocating anything.
    fn original_fields(&self, id: u32, bytes: &[u8], mode: Mode) -> Result<Vec<String>, Error> {
        let fields = scan::scan_row(bytes, 0, self.format.dialect, mode, |_, _| true).fields as u64;
        if fields > MAX_ROW_FIELDS {
            return Err(Error::RowTooWide {
                row: id as u64,
                fields,
            });
        }
        let mut out = Vec::new();
        out.try_reserve_exact(fields as usize)
            .map_err(|_| Error::OutOfMemory("not enough memory for the row"))?;
        scan::scan_row(bytes, 0, self.format.dialect, mode, |s, e| {
            out.push(self.decode_field(&bytes[s..e]));
            true
        });
        Ok(out)
    }

    /// Ids of rows with edits (any order).
    pub(crate) fn edited_ids(&self) -> Vec<u32> {
        self.overlay.rows.keys().copied().collect()
    }

    /// Whether row `id` has edits.
    pub(crate) fn has_edit(&self, id: u32) -> bool {
        self.overlay.get(id).is_some()
    }

    /// Rows the file had when it was opened or last saved (their ids are
    /// `0..original_rows`; inserted rows get larger ids).
    pub(crate) fn original_rows(&self) -> u64 {
        self.index.rows()
    }

    /// Columns the widest row of the file had.
    pub(crate) fn original_width(&self) -> u32 {
        self.index.stats().max_fields
    }

    /// An original row's values as the file has them (physical columns).
    pub(crate) fn original_values(&self, id: u32) -> Result<Vec<String>, Error> {
        let (bytes, mode) = self.raw_row(id);
        self.original_fields(id, bytes, mode)
    }

    /// A row's values now (physical columns).
    pub(crate) fn physical_values(&self, id: u32) -> Result<Vec<String>, Error> {
        self.values_of(id)
    }

    /// The column map as a list (logical -> physical), the identity when
    /// there is none.
    pub(crate) fn logical_to_physical(&self) -> Vec<u32> {
        match &self.cols {
            Some(m) => m.clone(),
            None => (0..self.col_count()).collect(),
        }
    }

    /// All values of the row at `row` (in column-map order).
    pub fn row_values(&self, row: u64) -> Result<Vec<String>, Error> {
        let id = self.id_at(row).ok_or(Error::OutOfRange)?;
        self.logical_values(id)
    }

    /// A row's values through the column map (map entries are unique).
    pub(crate) fn logical_values(&self, id: u32) -> Result<Vec<String>, Error> {
        let mut phys = self.values_of(id)?;
        Ok(match self.col_map() {
            None => phys,
            Some(m) => m
                .iter()
                .map(|&p| {
                    phys.get_mut(p as usize)
                        .map(std::mem::take)
                        .unwrap_or_default()
                })
                .collect(),
        })
    }

    fn values_of(&self, id: u32) -> Result<Vec<String>, Error> {
        match self.row_source(id) {
            RowSource::Edited(v) => Ok(v.into_owned()),
            RowSource::Raw { bytes, mode } => self.original_fields(id, bytes, mode),
        }
    }

    /// Hash of a row's values, streamed so wide rows need no memory. With
    /// `fffd_as_question`, U+FFFD and `?` hash alike (a lossy save to a legacy
    /// encoding writes one for the other).
    pub(crate) fn values_hash(&self, id: u32, fffd_as_question: bool) -> Result<u64, Error> {
        use std::hash::Hasher;
        let mut h = std::hash::DefaultHasher::new();
        let mut put = |v: &str| {
            if fffd_as_question && v.contains('\u{FFFD}') {
                h.write(v.replace('\u{FFFD}', "?").as_bytes());
            } else {
                h.write(v.as_bytes());
            }
            // 0xFF never occurs in UTF-8, so it frames values unambiguously.
            h.write_u8(0xFF);
        };
        let quote = self.format.dialect;
        let enc = self.working_encoding();
        match (self.col_map(), self.row_source(id)) {
            (None, RowSource::Edited(v)) => v.iter().for_each(|s| put(s)),
            (None, RowSource::Raw { bytes, mode }) => {
                scan::scan_row(bytes, 0, self.format.dialect, mode, |s, e| {
                    let v = scan::unquote(&bytes[s..e], quote);
                    put(&encoding::decode_direct(&v, enc));
                    true
                });
            }
            (Some(m), RowSource::Edited(v)) => {
                for &p in m {
                    put(v.get(p as usize).map_or("", String::as_str));
                }
            }
            (Some(m), RowSource::Raw { bytes, mode }) => {
                let spans = self.field_spans(id, bytes, mode)?;
                for &p in m {
                    match spans.get(p as usize) {
                        Some(&(s, e)) => {
                            let v = scan::unquote(&bytes[s..e], quote);
                            put(&encoding::decode_direct(&v, enc));
                        }
                        None => put(""),
                    }
                }
            }
        }
        Ok(h.finish())
    }

    /// Start and end of every field of a raw row, refusing rows wider than
    /// [`MAX_ROW_FIELDS`] without collecting them.
    pub(crate) fn field_spans(
        &self,
        id: u32,
        bytes: &[u8],
        mode: Mode,
    ) -> Result<Vec<(usize, usize)>, Error> {
        let mut spans = Vec::new();
        self.field_spans_into(id, bytes, mode, &mut spans)?;
        Ok(spans)
    }

    /// [`Document::field_spans`] into a reused buffer.
    pub(crate) fn field_spans_into(
        &self,
        id: u32,
        bytes: &[u8],
        mode: Mode,
        spans: &mut Vec<(usize, usize)>,
    ) -> Result<(), Error> {
        spans.clear();
        scan::scan_row(bytes, 0, self.format.dialect, mode, |s, e| {
            spans.push((s, e));
            (spans.len() as u64) <= MAX_ROW_FIELDS
        });
        if spans.len() as u64 > MAX_ROW_FIELDS {
            spans.clear();
            let fields = scan::scan_row(bytes, 0, self.format.dialect, mode, |_, _| true).fields;
            return Err(Error::RowTooWide {
                row: id as u64,
                fields: fields as u64,
            });
        }
        Ok(())
    }

    /// UTF-8 was chosen although some bytes near the start were invalid;
    /// they show as "�" but are kept byte-for-byte when saving.
    /// Size of the file's bytes (as opened).
    pub(crate) fn source_len(&self) -> usize {
        self.source.bytes().len()
    }

    /// Bytes of the file as read (before edits): a quick size estimate.
    pub fn source_bytes(&self) -> u64 {
        self.source_len() as u64
    }

    pub fn has_invalid_bytes(&self) -> bool {
        self.invalid_bytes
    }

    /// Full value of one cell (empty when the row is shorter). Not for
    /// untrusted callers: a cell can be as large as the file; use
    /// [`Document::viewport`] with a character cap instead.
    pub fn cell(&self, row: u64, col: u32) -> Option<String> {
        let id = self.id_at(row)?;
        let Some(col) = self.phys(col) else {
            return Some(String::new());
        };
        Some(match self.row_source(id) {
            RowSource::Edited(v) => v.get(col as usize).cloned().unwrap_or_default(),
            RowSource::Raw { bytes, mode } => {
                let mut found = String::new();
                let mut i = 0u32;
                scan::scan_row(bytes, 0, self.format.dialect, mode, |s, e| {
                    if i == col {
                        found = self.decode_field(&bytes[s..e]);
                        return false;
                    }
                    i += 1;
                    true
                });
                found
            }
        })
    }

    /// Next id a new row would get (every row id is below it).
    pub(crate) fn next_row_id(&self) -> u32 {
        self.next_id
    }

    /// Bulk operations need the whole file indexed.
    pub(crate) fn ready_for_bulk(&self) -> Result<(), Error> {
        self.ready()
    }

    fn ready(&self) -> Result<(), Error> {
        if self.index.is_complete() {
            Ok(())
        } else {
            Err(Error::StillLoading)
        }
    }

    /// Set one cell.
    pub fn set_cell(&mut self, row: u64, col: u32, value: String) -> Result<(), Error> {
        self.set_cells(vec![(row, col, value)])
    }

    /// Set many cells as one undo step.
    pub fn set_cells(&mut self, cells: Vec<(u64, u32, String)>) -> Result<(), Error> {
        self.ready()?;
        // Shown rows to ids, before anything changes.
        let cells: Vec<(u32, u32, String)> = cells
            .into_iter()
            .map(|(r, c, v)| self.id_at(r).map(|id| (id, c, v)))
            .collect::<Option<_>>()
            .ok_or(Error::OutOfRange)?;
        self.set_cells_by_id(cells)
    }

    /// The value in column `col` of the table's first row (the header row
    /// when there is one), whether or not a filter or hidden rows show it.
    pub fn first_row_cell(&self, col: u32) -> Result<String, Error> {
        self.ready()?;
        let id = self.doc_id_at(0).ok_or(Error::OutOfRange)?;
        let mut v = self.logical_values(id)?;
        let c = col as usize;
        Ok(if c < v.len() {
            std::mem::take(&mut v[c])
        } else {
            String::new()
        })
    }

    /// The table's first row (in column order), shown or not.
    pub fn first_row_values(&self) -> Result<Vec<String>, Error> {
        self.ready()?;
        let id = self.doc_id_at(0).ok_or(Error::OutOfRange)?;
        self.logical_values(id)
    }

    /// Set column `col` of the table's first row (renaming a column), as
    /// one undo step, even while a filter or hidden rows leave it unshown.
    pub fn set_first_row_cell(&mut self, col: u32, value: String) -> Result<(), Error> {
        self.ready()?;
        let id = self.doc_id_at(0).ok_or(Error::OutOfRange)?;
        // A rename: allowed in a protected column too.
        self.set_cells_guarded(vec![(id, col, value)], false)
    }

    fn set_cells_by_id(&mut self, cells: Vec<(u32, u32, String)>) -> Result<(), Error> {
        self.set_cells_guarded(cells, true)
    }

    /// Set several cells of the table's first row (renaming columns) as one
    /// undo step, shown or not; protected columns can be renamed.
    pub fn set_first_row_cells(&mut self, cells: Vec<(u32, String)>) -> Result<(), Error> {
        self.ready()?;
        let id = self.doc_id_at(0).ok_or(Error::OutOfRange)?;
        self.set_cells_guarded(cells.into_iter().map(|(c, v)| (id, c, v)).collect(), false)
    }

    fn set_cells_guarded(
        &mut self,
        mut cells: Vec<(u32, u32, String)>,
        guard: bool,
    ) -> Result<(), Error> {
        self.protected_skipped.store(0, Ordering::Relaxed);
        if guard && !self.protected.is_empty() {
            let before = cells.len();
            cells.retain(|(_, c, _)| !self.is_protected(*c));
            let skipped = (before - cells.len()) as u64;
            self.protected_skipped.store(skipped, Ordering::Relaxed);
            self.only_protected(before > 0 && cells.is_empty())?;
        }
        let mut widest = 0u32;
        for (_, col, _) in &cells {
            if *col > MAX_EDIT_COL {
                return Err(Error::OutOfRange);
            }
            widest = widest.max(col + 1);
        }
        let grow = self.grow_cols(widest)?;
        let mut by_id: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
        for (id, col, value) in cells {
            let col = match self.phys(col) {
                Some(p) => p,
                None => {
                    let (base, ids) = grow.as_ref().expect("grow_cols covers every column");
                    ids[(col - base) as usize]
                }
            };
            by_id.entry(id).or_default().push((col, value));
        }
        let mut states = Vec::with_capacity(by_id.len());
        for (id, changes) in by_id {
            // Parse the file's values once: they seed the row and are compared
            // against afterwards.
            let original = if id < self.index.rows() as u32 {
                let (a, b) = self.index.row_range(id as u64).expect("original row");
                let bytes = &self.source.bytes()[a..b];
                Some(self.original_fields(id, bytes, self.index.mode_of(id as u64))?)
            } else {
                None
            };
            let mut values = match (self.overlay.get(id), &original) {
                (Some(Edit::Full(v)), _) => v.clone(),
                (Some(Edit::Patch { width, cells }), _) => self.patched(id, *width, cells),
                (None, Some(o)) => o.clone(),
                // New rows are always in the overlay; empty is a safe default.
                (None, None) => Vec::new(),
            };
            for (col, value) in changes {
                let c = col as usize;
                if values.len() <= c {
                    values.resize(c + 1, String::new());
                }
                values[c] = value;
            }
            // An original row edited back to its file values returns to its
            // raw bytes, so it is saved byte-for-byte again.
            let state = if original.as_ref() == Some(&values) {
                None
            } else {
                Some(Edit::Full(values))
            };
            states.push((id, state));
        }
        match grow {
            Some((at, ids)) => self.push(Op::Batch(vec![
                Op::ColInsert {
                    at: at as usize,
                    ids,
                },
                Op::Restore(states),
            ])),
            None => self.push(Op::Restore(states)),
        }
        Ok(())
    }

    /// With a column map, new physical columns that make the map at least
    /// `width` long: `(first new logical column, physical ids)`.
    fn grow_cols(&mut self, width: u32) -> Result<Option<(u32, Vec<u32>)>, Error> {
        let Some(m) = &self.cols else {
            return Ok(None);
        };
        let have = m.len() as u32;
        if width <= have {
            return Ok(None);
        }
        let ids = self.fresh_cols(width - have)?;
        Ok(Some((have, ids)))
    }

    /// `n` unused physical column ids.
    fn fresh_cols(&mut self, n: u32) -> Result<Vec<u32>, Error> {
        let start = self.next_col.max(self.physical_width());
        let end = start.checked_add(n).ok_or(Error::OutOfRange)?;
        if end > MAX_MAPPED_COLS {
            return Err(Error::OutOfRange);
        }
        self.next_col = end;
        Ok((start..end).collect())
    }

    /// Push a column op. The first one also creates the column map (the
    /// identity over every physical column) in the same undo step, so
    /// undoing it removes the map and every row shows in full again.
    fn push_col_op(&mut self, op: Op) -> Result<(), Error> {
        if self.cols.is_some() {
            self.push(op);
            return Ok(());
        }
        let n = self.physical_width();
        if n > MAX_MAPPED_COLS {
            return Err(Error::OutOfRange);
        }
        self.next_col = self.next_col.max(n);
        self.push(Op::Batch(vec![Op::SetMap(Some((0..n).collect())), op]));
        Ok(())
    }

    /// Insert `count` empty columns before column `at` (`at == col_count()`
    /// appends). Saving then rewrites every row.
    pub fn insert_cols(&mut self, at: u32, count: u32) -> Result<(), Error> {
        self.ready()?;
        if at > self.col_count() {
            return Err(Error::OutOfRange);
        }
        if count == 0 {
            return Ok(());
        }
        if self.col_count() as u64 + count as u64 > MAX_MAPPED_COLS as u64 {
            return Err(Error::OutOfRange);
        }
        let ids = self.fresh_cols(count)?;
        self.push_col_op(Op::ColInsert {
            at: at as usize,
            ids,
        })
    }

    /// Delete `count` columns starting at column `at`. At least one column
    /// always remains.
    pub fn delete_cols(&mut self, at: u32, count: u32) -> Result<(), Error> {
        self.ready()?;
        let end = at.checked_add(count).ok_or(Error::OutOfRange)?;
        if end > self.col_count() || count >= self.col_count() {
            return Err(Error::OutOfRange);
        }
        if count == 0 {
            return Ok(());
        }
        self.push_col_op(Op::ColRemove {
            at: at as usize,
            count: count as usize,
        })
    }

    /// Move `count` columns at `at` so they start at column `to` afterwards.
    pub fn move_cols(&mut self, at: u32, count: u32, to: u32) -> Result<(), Error> {
        self.ready()?;
        let n = self.col_count() as u64;
        if at as u64 + count as u64 > n || to as u64 + count as u64 > n {
            return Err(Error::OutOfRange);
        }
        if count == 0 || at == to {
            return Ok(());
        }
        self.push_col_op(Op::MoveCols {
            at: at as usize,
            count: count as usize,
            to: to as usize,
        })
    }

    /// Move `count` rows at position `at` so they start at position `to`
    /// afterwards (for example, to make a row the header row).
    pub fn move_rows(&mut self, at: u64, count: u64, to: u64) -> Result<(), Error> {
        self.ready()?;
        if self.view.is_some() {
            return Err(Error::FilterActive);
        }
        let n = self.total_rows();
        let fits = |p: u64| p.checked_add(count).is_some_and(|e| e <= n);
        if !fits(at) || !fits(to) {
            return Err(Error::OutOfRange);
        }
        if count == 0 || at == to {
            return Ok(());
        }
        self.push(Op::MoveRows {
            at: at as usize,
            count: count as usize,
            to: to as usize,
        });
        Ok(())
    }

    /// Insert rows before position `at` (`at == row_count()` appends).
    pub fn insert_rows(&mut self, at: u64, rows: Vec<Vec<String>>) -> Result<(), Error> {
        self.ready()?;
        if at > self.row_count() || rows.iter().any(|r| r.len() > MAX_EDIT_COL as usize + 1) {
            return Err(Error::OutOfRange);
        }
        let n = u32::try_from(rows.len()).map_err(|_| Error::TooManyRows)?;
        let widest = rows.iter().map(|r| r.len() as u32).max().unwrap_or(0);
        let grow = self.grow_cols(widest)?;
        let first = self.next_id;
        self.next_id = first.checked_add(n).ok_or(Error::TooManyRows)?;
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    first + i as u32,
                    Some(Edit::Full(self.to_physical(r, grow.as_ref()))),
                )
            })
            .collect();
        // With a filter on, before the shown row `at` (new rows always show).
        let pos = match &self.view {
            Some(v) => {
                let vis = self.visible(v);
                match vis.get(at as usize) {
                    Some(&p) => p as u64,
                    None => vis.last().map_or(self.total_rows(), |&p| p as u64 + 1),
                }
            }
            None => at,
        };
        let insert = Op::Insert {
            at: pos as usize,
            rows,
        };
        match grow {
            Some((c, ids)) => self.push(Op::Batch(vec![
                Op::ColInsert {
                    at: c as usize,
                    ids,
                },
                insert,
            ])),
            None => self.push(insert),
        }
        Ok(())
    }

    /// Logical values of a new row placed at their physical columns.
    pub(crate) fn to_physical(
        &self,
        row: Vec<String>,
        grow: Option<&(u32, Vec<u32>)>,
    ) -> Vec<String> {
        let Some(m) = &self.cols else {
            return row;
        };
        let phys = |c: usize| match m.get(c) {
            Some(&p) => p,
            None => {
                let (base, ids) = grow.expect("grow_cols covers every column");
                ids[c - *base as usize]
            }
        };
        let width = (0..row.len()).map(|c| phys(c) + 1).max().unwrap_or(0);
        let mut out = vec![String::new(); width as usize];
        for (c, v) in row.into_iter().enumerate() {
            out[phys(c) as usize] = v;
        }
        out
    }

    /// Delete `count` rows starting at position `at`.
    pub fn delete_rows(&mut self, at: u64, count: u64) -> Result<(), Error> {
        self.ready()?;
        let end = at.checked_add(count).ok_or(Error::OutOfRange)?;
        if end > self.row_count() {
            return Err(Error::OutOfRange);
        }
        if count == 0 {
            return Ok(());
        }
        let Some(v) = &self.view else {
            self.push(Op::Remove {
                at: at as usize,
                count: count as usize,
            });
            return Ok(());
        };
        // With a filter on, only the shown rows go: one removal per run of
        // neighbouring rows, last run first so positions stay valid.
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for &p in &self.visible(v)[at as usize..end as usize] {
            match runs.last_mut() {
                Some((s, n)) if *s + *n == p as usize => *n += 1,
                _ => runs.push((p as usize, 1)),
            }
        }
        let mut ops: Vec<Op> = runs
            .into_iter()
            .rev()
            .map(|(at, count)| Op::Remove { at, count })
            .collect();
        self.push(if ops.len() == 1 {
            ops.pop().expect("one op")
        } else {
            Op::Batch(ops)
        });
        Ok(())
    }

    /// Undo the last change. Returns `false` when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(op) = self.undo.pop() else {
            return false;
        };
        self.undo_bytes -= op.cost();
        let inv = self.apply(op);
        self.redo_bytes += inv.cost();
        self.redo.push(inv);
        self.history_pos -= 1;
        true
    }

    /// Redo the last undone change.
    pub fn redo(&mut self) -> bool {
        let Some(op) = self.redo.pop() else {
            return false;
        };
        self.redo_bytes -= op.cost();
        let inv = self.apply(op);
        self.undo_bytes += inv.cost();
        self.undo.push(inv);
        self.trim_undo();
        self.history_pos += 1;
        true
    }

    fn push(&mut self, op: Op) {
        self.history_pos += 1;
        let inv = self.apply(op);
        self.undo_bytes += inv.cost();
        self.undo.push(inv);
        if self.clean_depth.is_some_and(|d| d > self.undo.len() - 1) {
            // The saved state was on the redo branch we are discarding.
            self.clean_depth = None;
        }
        self.redo.clear();
        self.redo_bytes = 0;
        self.trim_undo();
    }

    /// Forget the oldest undo steps beyond the step count or memory budget.
    fn trim_undo(&mut self) {
        let mut drop = 0;
        let mut bytes = self.undo_bytes + self.redo_bytes;
        while self.undo.len() - drop > 1
            && (self.undo.len() - drop > MAX_UNDO || bytes > self.undo_budget)
        {
            let c = self.undo[drop].cost();
            bytes -= c;
            self.undo_bytes -= c;
            drop += 1;
        }
        if drop > 0 {
            self.undo.drain(..drop);
            self.clean_depth = self.clean_depth.and_then(|d| d.checked_sub(drop));
        }
    }

    /// Approximate memory held by the undo and redo history.
    pub fn history_bytes(&self) -> usize {
        self.undo_bytes + self.redo_bytes
    }

    fn order_mut(&mut self) -> &mut Vec<u32> {
        let rows = self.index.rows() as u32;
        self.order.get_or_insert_with(|| (0..rows).collect())
    }

    /// The row order changed: a filter's list of shown positions is stale.
    fn rows_moved(&mut self) {
        if let Some(v) = &mut self.view {
            v.visible = OnceLock::new();
            v.hidden_rows = OnceLock::new();
        }
    }

    /// Put rows in a new order (every id exactly once), as one undo step.
    pub(crate) fn set_order(&mut self, order: Vec<u32>) {
        self.push(Op::SetOrder(Some(order)));
    }

    /// Change some cells of many rows as one undo step. Each change lists
    /// `(physical column, new text)` sorted by column. Original rows keep
    /// their file bytes and hold only the new cells.
    pub(crate) fn patch_rows(
        &mut self,
        changes: Vec<(u32, Vec<(u32, String)>)>,
    ) -> Result<(), Error> {
        self.ready()?;
        let states = self.patch_states(changes)?;
        self.only_protected(states.is_empty())?;
        if !states.is_empty() {
            self.push(Op::Restore(states));
        }
        Ok(())
    }

    /// The overlay states [`Document::patch_rows`] would set, without
    /// applying them (so they can join other ops in one undo step). Cells of
    /// a row may come in any order; for a column given twice the last wins.
    pub(crate) fn patch_states(
        &self,
        mut changes: Vec<(u32, Vec<(u32, String)>)>,
    ) -> Result<Vec<(u32, Option<Edit>)>, Error> {
        self.drop_protected(&mut changes);
        let mut states = Vec::with_capacity(changes.len());
        for (id, mut cells) in changes {
            if !cells.windows(2).all(|w| w[0].0 < w[1].0) {
                // Stable sort keeps the given order of repeats; keep the last.
                cells.sort_by_key(|c| c.0);
                let mut out: Vec<(u32, String)> = Vec::with_capacity(cells.len());
                for c in cells {
                    match out.last_mut() {
                        Some(l) if l.0 == c.0 => *l = c,
                        _ => out.push(c),
                    }
                }
                cells = out;
            }
            let Some(last) = cells.last().map(|c| c.0) else {
                continue;
            };
            let state = match self.overlay.get(id) {
                Some(Edit::Full(v)) => {
                    let mut v = v.clone();
                    if v.len() <= last as usize {
                        v.resize(last as usize + 1, String::new());
                    }
                    for (c, s) in cells {
                        v[c as usize] = s;
                    }
                    Edit::Full(v)
                }
                Some(Edit::Patch { width, cells: old }) => {
                    let mut merged: BTreeMap<u32, Box<str>> = old.iter().cloned().collect();
                    for (c, s) in cells {
                        merged.insert(c, s.into_boxed_str());
                    }
                    Edit::Patch {
                        width: (*width).max(last + 1),
                        cells: merged.into_iter().collect(),
                    }
                }
                None => {
                    let (bytes, mode) = self.raw_row(id);
                    let fields = scan::scan_row(bytes, 0, self.format.dialect, mode, |_, _| true)
                        .fields as u64;
                    if fields > MAX_ROW_FIELDS {
                        return Err(Error::RowTooWide {
                            row: id as u64,
                            fields,
                        });
                    }
                    Edit::Patch {
                        width: (fields as u32).max(last + 1),
                        cells: cells
                            .into_iter()
                            .map(|(c, s)| (c, s.into_boxed_str()))
                            .collect(),
                    }
                }
            };
            states.push((id, Some(state)));
        }
        Ok(states)
    }

    /// One undo step made of several parts, applied in this order:
    /// new columns (`(logical position, physical ids)`), cell states, removed
    /// logical columns (`(at, count)` runs, last first), a whole new column
    /// map, and a new row order. A column part on a document without a
    /// column map creates the map (the identity) in the same step, so
    /// undoing removes it again.
    pub(crate) fn push_step(&mut self, step: Step) {
        let mut ops = Vec::new();
        let touches_cols =
            step.col_insert.is_some() || !step.col_remove.is_empty() || step.map.is_some();
        if touches_cols && self.cols.is_none() {
            let n = self.physical_width();
            self.next_col = self.next_col.max(n);
            ops.push(Op::SetMap(Some((0..n).collect())));
        }
        if let Some((at, ids)) = step.col_insert {
            ops.push(Op::ColInsert {
                at: at as usize,
                ids,
            });
        }
        if !step.states.is_empty() {
            ops.push(Op::Restore(step.states));
        }
        for (at, count) in step.col_remove {
            ops.push(Op::ColRemove {
                at: at as usize,
                count: count as usize,
            });
        }
        if let Some(m) = step.map {
            ops.push(Op::SetMap(Some(m)));
        }
        if let Some(o) = step.order {
            ops.push(Op::SetOrder(Some(o)));
        }
        if !step.append.is_empty() {
            // Positions after the order above; the order holds every row.
            let at = match &ops.last() {
                Some(Op::SetOrder(Some(o))) => o.len(),
                _ => self.total_rows() as usize,
            };
            ops.push(Op::Insert {
                at,
                rows: step.append,
            });
        }
        match ops.len() {
            0 => {}
            1 => self.push(ops.pop().expect("one op")),
            _ => self.push(Op::Batch(ops)),
        }
    }

    /// `n` physical column ids for new columns (see [`Document::push_step`]).
    pub(crate) fn new_col_ids(&mut self, n: u32) -> Result<Vec<u32>, Error> {
        if self.cols.is_none() {
            self.next_col = self.next_col.max(self.physical_width());
        }
        if self.col_count() as u64 + n as u64 > MAX_MAPPED_COLS as u64 {
            return Err(Error::OutOfRange);
        }
        self.fresh_cols(n)
    }

    /// `n` fresh row ids for new rows.
    pub(crate) fn new_row_ids(&mut self, n: u32) -> Result<u32, Error> {
        let first = self.next_id;
        self.next_id = first.checked_add(n).ok_or(Error::TooManyRows)?;
        Ok(first)
    }

    /// The current column map as a list (identity when never changed).
    pub(crate) fn map_list(&self) -> Vec<u32> {
        match &self.cols {
            Some(m) => m.clone(),
            None => (0..self.physical_width()).collect(),
        }
    }

    /// Whether row `id` holds edits (removing it must restore them on undo).
    pub(crate) fn is_edited(&self, id: u32) -> bool {
        self.overlay.get(id).is_some()
    }

    /// Count a document built in memory (pivot, unpivot) as unsaved.
    pub fn mark_modified(&mut self) {
        self.clean_depth = None;
    }

    fn apply(&mut self, op: Op) -> Op {
        // Any change can change whether the column map is the identity.
        self.map_identity.store(0, Ordering::Relaxed);
        self.revision += 1;
        match op {
            Op::SetMap(m) => Op::SetMap(std::mem::replace(&mut self.cols, m)),
            Op::SetOrder(o) => {
                let old = std::mem::replace(&mut self.order, o);
                self.rows_moved();
                Op::SetOrder(old)
            }
            Op::Restore(states) => Op::Restore(
                states
                    .into_iter()
                    .map(|(id, s)| (id, self.overlay.set(id, s)))
                    .collect(),
            ),
            Op::Insert { at, rows } => {
                let count = rows.len();
                let ids: Vec<u32> = rows.iter().map(|(id, _)| *id).collect();
                for (id, s) in rows {
                    self.overlay.set(id, s);
                }
                self.order_mut().splice(at..at, ids);
                self.rows_moved();
                Op::Remove { at, count }
            }
            Op::Remove { at, count } => {
                let ids: Vec<u32> = self.order_mut().drain(at..at + count).collect();
                let rows = ids
                    .into_iter()
                    .map(|id| (id, self.overlay.set(id, None)))
                    .collect();
                self.rows_moved();
                Op::Insert { at, rows }
            }
            Op::MoveRows { at, count, to } => {
                let o = self.order_mut();
                let ids: Vec<u32> = o.drain(at..at + count).collect();
                o.splice(to..to, ids);
                self.rows_moved();
                Op::MoveRows {
                    at: to,
                    count,
                    to: at,
                }
            }
            Op::ColInsert { at, ids } => {
                let count = ids.len();
                let m = self.cols.as_mut().expect("column ops make the map first");
                m.splice(at..at, ids);
                Op::ColRemove { at, count }
            }
            Op::ColRemove { at, count } => {
                let m = self.cols.as_mut().expect("column ops make the map first");
                let ids = m.drain(at..at + count).collect();
                Op::ColInsert { at, ids }
            }
            Op::MoveCols { at, count, to } => {
                let m = self.cols.as_mut().expect("column ops make the map first");
                let ids: Vec<u32> = m.drain(at..at + count).collect();
                m.splice(to..to, ids);
                Op::MoveCols {
                    at: to,
                    count,
                    to: at,
                }
            }
            Op::SetHidden(h) => {
                let old = match self.view.take() {
                    Some(mut v) => {
                        let old = std::mem::replace(&mut v.hidden, h);
                        v.visible = OnceLock::new();
                        v.hidden_rows = OnceLock::new();
                        if v.filters() || v.hidden.is_some() {
                            self.view = Some(v);
                        }
                        old
                    }
                    None => {
                        if h.is_some() {
                            let mut v = View::all();
                            v.hidden = h;
                            self.view = Some(v);
                        }
                        None
                    }
                };
                Op::SetHidden(old)
            }
            Op::SetFilter(f) => {
                let (old, hidden) = match self.view.take() {
                    Some(v) => {
                        let old = v.filters().then(|| {
                            Box::new(FilterPart {
                                spec: v.spec,
                                bits: v.bits,
                                base: v.base,
                            })
                        });
                        (old, v.hidden)
                    }
                    None => (None, None),
                };
                let mut v = match f {
                    Some(f) => View::new(f.spec, f.bits, f.base),
                    None => View::all(),
                };
                v.hidden = hidden;
                if v.filters() || v.hidden.is_some() {
                    self.view = Some(v);
                }
                Op::SetFilter(old)
            }
            Op::Batch(ops) => {
                let mut inv: Vec<Op> = ops.into_iter().map(|o| self.apply(o)).collect();
                inv.reverse();
                Op::Batch(inv)
            }
        }
    }

    /// The id of the row at document position `pos` (shown or not).
    pub(crate) fn id_of_pos(&self, pos: u64) -> Option<u32> {
        match &self.order {
            Some(o) => o.get(usize::try_from(pos).ok()?).copied(),
            None => (pos < self.index.rows()).then_some(pos as u32),
        }
    }

    /// Iterate row ids in document order.
    pub(crate) fn ids(&self) -> Box<dyn Iterator<Item = u32> + '_> {
        match &self.order {
            Some(o) => Box::new(o.iter().copied()),
            None => Box::new(0..self.index.rows() as u32),
        }
    }

    /// Replace the document's contents with a freshly saved file.
    pub(crate) fn reset_to(
        &mut self,
        source: Source,
        start: Start,
        format: Format,
        index: RowIndex,
    ) {
        // A filter stays on: the saved rows keep their positions, and its
        // rules move from physical to (now equal) logical columns.
        let kept = self.view.take().map(|v| {
            let mut passing = Vec::new();
            let mut hidden = Vec::new();
            for (pos, id) in self.ids().enumerate() {
                if v.filters() && v.passes(id) {
                    passing.push(pos as u32);
                }
                if v.is_hidden(id) {
                    hidden.push(pos as u32);
                }
            }
            let spec = v.spec.map_cols(|p| match &self.cols {
                Some(m) => m.iter().position(|&x| x == p).map(|i| i as u32),
                None => Some(p),
            });
            (v.filters(), spec, passing, hidden)
        });
        self.revision += 1;
        self.source = source;
        self.data_start = start.data;
        self.preamble_len = start.preamble_len;
        self.preamble_lines = start.preamble_lines;
        self.format = format;
        self.lossy = false;
        self.next_id = index.rows() as u32;
        self.index = index;
        self.order = None;
        self.cols = None;
        self.next_col = 0;
        self.swapped.clear();
        self.map_identity.store(0, Ordering::Relaxed);
        // Freeing millions of edited cells (after a join, say) takes
        // seconds: big leftovers are let go on a background thread.
        let old = (
            std::mem::take(&mut self.overlay),
            std::mem::take(&mut self.undo),
            std::mem::take(&mut self.redo),
        );
        if self.undo_bytes + self.redo_bytes > 64 << 20 || old.0.len() > 100_000 {
            // (If no thread can be made, the closure and the edits are
            // dropped here instead.)
            let _ = std::thread::Builder::new()
                .name("free-edits".into())
                .spawn(move || drop(old));
        } else {
            drop(old);
        }
        self.undo_bytes = 0;
        self.redo_bytes = 0;
        self.clean_depth = Some(0);
        if let Some((filters, spec, passing, hidden)) = kept {
            let n = self.next_id;
            let to_bits = |list: Vec<u32>| {
                let mut bits = vec![0u64; (n as usize).div_ceil(64)];
                for p in list.into_iter().filter(|&p| p < n) {
                    bits[(p >> 6) as usize] |= 1 << (p & 63);
                }
                bits
            };
            let mut v = if filters {
                View::new(spec, to_bits(passing), n)
            } else {
                View::all()
            };
            if !hidden.is_empty() {
                v.hidden = Some(Arc::new(to_bits(hidden)));
            }
            if v.filters() || v.hidden.is_some() {
                self.view = Some(v);
            }
        }
    }

    /// Swap out the source (used while saving over the open file).
    pub(crate) fn take_source(&mut self) -> Source {
        std::mem::replace(&mut self.source, Source::from_vec(Vec::new()))
    }

    pub(crate) fn put_source(&mut self, source: Source) {
        self.source = source;
    }

    /// Source, data offset, preamble and index of a freshly opened document.
    pub(crate) fn into_parts(self) -> (Source, Start, RowIndex) {
        let start = Start {
            data: self.data_start,
            preamble_len: self.preamble_len,
            preamble_lines: self.preamble_lines,
        };
        (self.source, start, self.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(bytes: &[u8]) -> Document {
        Document::from_bytes(bytes.to_vec(), OpenOptions::default(), None).unwrap()
    }

    fn all(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    #[test]
    fn opens_and_reads() {
        let d = doc(b"a,b\r\n1,\"x,y\"\r\n2,z");
        assert_eq!(d.row_count(), 3);
        assert_eq!(d.col_count(), 2);
        assert_eq!(d.cell(1, 1).unwrap(), "x,y");
        assert_eq!(d.cell(2, 5).unwrap(), "");
        assert_eq!(d.format().line_break, LineBreak::CrLf);
        assert!(d.cell(3, 0).is_none());
    }

    #[test]
    fn edit_undo_redo() {
        let mut d = doc(b"a,b\n1,2\n");
        d.set_cell(1, 0, "one".into()).unwrap();
        d.set_cell(1, 3, "wide".into()).unwrap();
        assert_eq!(d.col_count(), 4);
        assert_eq!(d.row_values(1).unwrap(), ["one", "2", "", "wide"]);
        assert!(d.is_modified());
        assert!(d.undo());
        assert_eq!(d.col_count(), 2);
        assert!(d.undo());
        assert_eq!(d.row_values(1).unwrap(), ["1", "2"]);
        assert_eq!(d.edited_rows(), 0);
        assert!(!d.is_modified());
        assert!(d.redo());
        assert_eq!(d.cell(1, 0).unwrap(), "one");
    }

    #[test]
    fn editing_back_restores_raw_row() {
        let mut d = doc(b"a,b\n1,2\n");
        d.set_cell(1, 0, "x".into()).unwrap();
        d.set_cell(1, 0, "1".into()).unwrap();
        assert_eq!(d.edited_rows(), 0);
    }

    #[test]
    fn insert_delete_undo() {
        let mut d = doc(b"r0\nr1\nr2\n");
        d.insert_rows(1, vec![vec!["new".into()]]).unwrap();
        assert_eq!(all(&d), [["r0"], ["new"], ["r1"], ["r2"]]);
        d.set_cell(1, 0, "new2".into()).unwrap();
        d.delete_rows(0, 2).unwrap();
        assert_eq!(all(&d), [["r1"], ["r2"]]);
        assert_eq!(d.edited_rows(), 0);
        d.undo();
        assert_eq!(all(&d), [["r0"], ["new2"], ["r1"], ["r2"]]);
        d.undo();
        d.undo();
        assert_eq!(all(&d), [["r0"], ["r1"], ["r2"]]);
        assert!(!d.is_modified());
        assert!(d.delete_rows(2, 2).is_err());
        assert!(d.insert_rows(4, vec![]).is_err());
    }

    #[test]
    fn new_edit_after_undo_drops_clean_state() {
        let mut d = doc(b"a\n");
        d.set_cell(0, 0, "b".into()).unwrap();
        d.clean_depth = Some(1); // pretend saved here
        d.undo();
        d.set_cell(0, 0, "c".into()).unwrap();
        assert!(d.is_modified());
        d.undo();
        assert!(d.is_modified());
    }

    #[test]
    fn column_insert_delete_move_undo() {
        let mut d = doc(b"a,b,c\n1,2,3\n");
        d.insert_cols(1, 2).unwrap();
        assert_eq!(d.col_count(), 5);
        assert_eq!(all(&d), [["a", "", "", "b", "c"], ["1", "", "", "2", "3"]]);
        d.set_cell(1, 2, "new".into()).unwrap();
        assert_eq!(d.cell(1, 2).unwrap(), "new");
        d.delete_cols(3, 1).unwrap();
        assert_eq!(all(&d), [["a", "", "", "c"], ["1", "", "new", "3"]]);
        d.move_cols(3, 1, 0).unwrap();
        assert_eq!(all(&d), [["c", "a", "", ""], ["3", "1", "", "new"]]);
        // Editing past the end grows the map in the same undo step.
        d.set_cell(0, 5, "far".into()).unwrap();
        assert_eq!(d.col_count(), 6);
        assert_eq!(d.row_values(0).unwrap(), ["c", "a", "", "", "", "far"]);
        d.undo();
        assert_eq!(d.col_count(), 4);
        d.insert_rows(
            0,
            vec![vec![
                "h1".into(),
                "h2".into(),
                "h3".into(),
                "h4".into(),
                "h5".into(),
            ]],
        )
        .unwrap();
        assert_eq!(d.col_count(), 5);
        assert_eq!(d.row_values(0).unwrap(), ["h1", "h2", "h3", "h4", "h5"]);
        assert_eq!(d.row_values(1).unwrap(), ["c", "a", "", "", ""]);
        for _ in 0..5 {
            assert!(d.undo());
        }
        assert_eq!(all(&d), [["a", "b", "c"], ["1", "2", "3"]]);
        assert!(!d.is_modified());
        assert!(d.delete_cols(0, 3).is_err(), "one column must remain");
        assert!(d.move_cols(2, 2, 0).is_err());
        assert!(d.insert_cols(4, 1).is_err());
    }

    /// Review finding 1: a row wider than the map, brought back by Undo,
    /// must show (and save) in full.
    #[test]
    fn undo_past_first_column_op_removes_the_map() {
        let mut d = doc(b"a,b\n1,2\n");
        d.insert_rows(2, vec![vec!["x".into(), "y".into(), "z".into()]])
            .unwrap();
        d.delete_rows(2, 1).unwrap();
        d.move_cols(1, 1, 0).unwrap();
        assert_eq!(d.col_count(), 2);
        d.undo(); // move
        d.undo(); // delete
        assert_eq!(d.row_values(2).unwrap(), ["x", "y", "z"]);
        assert_eq!(d.col_count(), 3);
        assert!(!d.has_column_changes());
        let mut out = Vec::new();
        d.write_to(&mut out, d.format(), false).unwrap();
        assert_eq!(out, b"a,b\n1,2\nx,y,z\n");
    }

    /// Review finding 2: undoing everything restores the column count and
    /// byte-exact saving.
    #[test]
    fn undo_all_column_changes_restores_everything() {
        let src = b"\"q\",b,c\n\"a\"b,2\n";
        let mut d = doc(src);
        d.set_cell(1, 10, "far".into()).unwrap();
        d.insert_cols(1, 1).unwrap();
        d.undo();
        d.undo();
        assert_eq!(d.col_count(), 3);
        assert!(!d.has_column_changes());
        let mut out = Vec::new();
        d.write_to(&mut out, d.format(), false).unwrap();
        assert_eq!(out, src);
        // Insert then delete the same column: the map is the identity again.
        d.insert_cols(3, 1).unwrap();
        d.delete_cols(3, 1).unwrap();
        assert!(!d.has_column_changes());
        let mut out = Vec::new();
        d.write_to(&mut out, d.format(), false).unwrap();
        assert_eq!(out, src);
    }

    /// Review finding 5 / own finding 3: undo history is bounded by memory.
    #[test]
    fn undo_history_has_a_memory_budget() {
        let mut d = doc(b"a\n");
        d.undo_budget = 4_000;
        let big = "x".repeat(1_000);
        for i in 0..8 {
            d.set_cell(0, 0, format!("{i}{big}")).unwrap();
        }
        assert!(d.history_bytes() <= 4_000 + 2_000, "{}", d.history_bytes());
        let mut steps = 0;
        while d.undo() {
            steps += 1;
        }
        assert!((1..8).contains(&steps), "{steps}");
    }

    #[test]
    fn move_rows_and_undo() {
        let mut d = doc(b"r0\nr1\nr2\nr3\n");
        d.move_rows(2, 1, 0).unwrap();
        assert_eq!(all(&d), [["r2"], ["r0"], ["r1"], ["r3"]]);
        d.move_rows(0, 2, 2).unwrap();
        assert_eq!(all(&d), [["r1"], ["r3"], ["r2"], ["r0"]]);
        d.undo();
        d.undo();
        assert_eq!(all(&d), [["r0"], ["r1"], ["r2"], ["r3"]]);
        assert!(d.move_rows(3, 2, 0).is_err());
        assert!(d.move_rows(0, 1, 4).is_err());
    }

    #[test]
    fn rejects_huge_column() {
        let mut d = doc(b"a\n");
        assert!(d.set_cell(0, MAX_EDIT_COL + 1, "x".into()).is_err());
    }

    #[test]
    fn progressive_index() {
        let text: Vec<u8> = (0..1000)
            .flat_map(|i| format!("{i},x\n").into_bytes())
            .collect();
        let mut d =
            Document::from_source(Source::from_vec(text), OpenOptions::default(), None).unwrap();
        assert!(!d.index_step(100).unwrap());
        assert!(d.row_count() > 0 && d.row_count() < 1000);
        assert!(d.set_cell(0, 0, "x".into()).is_err());
        assert_eq!(d.cell(0, 0).unwrap(), "0");
        while !d.index_step(100).unwrap() {}
        assert_eq!(d.row_count(), 1000);
        assert_eq!(d.index_progress(), 1.0);
    }

    #[test]
    fn utf16_document() {
        let mut bytes = b"\xFF\xFE".to_vec();
        encoding::encode("név;kor\r\nÁdám;3\r\n", Encoding::Utf16Le, &mut bytes).unwrap();
        let d = Document::from_bytes(bytes, OpenOptions::default(), None).unwrap();
        assert_eq!(d.format().encoding, Encoding::Utf16Le);
        assert!(d.format().bom);
        assert_eq!(d.format().dialect.delimiter(), b";");
        assert_eq!(d.cell(1, 0).unwrap(), "Ádám");
    }

    #[test]
    fn legacy_document() {
        let w = Encoding::from_label("windows-1252").unwrap();
        let opts = OpenOptions {
            encoding: Some(w),
            ..Default::default()
        };
        let d = Document::from_bytes(b"caf\xE9,\x80\n".to_vec(), opts, None).unwrap();
        assert_eq!(d.row_values(0).unwrap(), ["café", "€"]);
    }
}
