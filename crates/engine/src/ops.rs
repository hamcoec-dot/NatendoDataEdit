//! Cell and column operations of Milestone 4b: block writes (paste and
//! Paste Special), fill (copy and series), data edges (Ctrl+arrow), split and
//! merge columns, and sorting columns by their header. Each is planned under
//! a read lock and applied as one undo step.

use std::borrow::Cow;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Edit, Progress, Step, MAX_EDIT_COL};
use crate::fill::{format_number, FillMode, Series};
use crate::value::{self, ReadOptions};
use crate::Error;

/// Most cells one block write or fill may change.
pub const MAX_BLOCK_CELLS: u64 = 5_000_000;
/// Most text one operation may create.
pub const MAX_BLOCK_BYTES: usize = 256 << 20;
/// Longest cell an operation may create (bytes).
pub const MAX_CELL_BYTES: usize = 32 << 20;
/// Most columns a split may add.
pub const MAX_NEW_COLS: u32 = 1_000;
/// Longest separator.
pub const MAX_SEPARATOR_CHARS: usize = 1_000;
/// Rows per parallel chunk.
pub(crate) const CHUNK: usize = 1 << 15;

/// Reads a set of logical columns of a row (in the order asked).
pub(crate) struct Cols {
    wanted: Vec<u32>,
    slot: Vec<Option<usize>>,
}

impl Cols {
    pub(crate) fn new(d: &Document, cols: &[u32]) -> Cols {
        let mut wanted: Vec<u32> = cols.iter().filter_map(|&c| d.phys(c)).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let slot = cols
            .iter()
            .map(|&c| d.phys(c).map(|p| wanted.binary_search(&p).expect("listed")))
            .collect();
        Cols { wanted, slot }
    }

    /// Read row `id` into `buf`.
    pub(crate) fn read<'a>(&self, d: &'a Document, id: u32, buf: &mut Vec<Cow<'a, str>>) {
        d.read_cells(id, &self.wanted, buf);
    }

    /// Where each asked column sits in the buffer [`Cols::read`] fills.
    pub(crate) fn slots(&self) -> &[Option<usize>] {
        &self.slot
    }

    /// The `k`th asked column's text after [`Cols::read`].
    pub(crate) fn get<'b>(&self, buf: &'b [Cow<'_, str>], k: usize) -> &'b str {
        self.slot[k].map_or("", |s| &buf[s])
    }
}

pub(crate) fn check_cols(d: &Document, cols: &[u32]) -> Result<(), Error> {
    if cols.iter().any(|&c| c >= d.col_count()) {
        return Err(Error::OutOfRange);
    }
    Ok(())
}

pub(crate) fn check_separator(sep: &str) -> Result<(), Error> {
    if sep.chars().count() > MAX_SEPARATOR_CHARS {
        return Err(Error::Invalid(format!(
            "the separator can be at most {MAX_SEPARATOR_CHARS} characters"
        )));
    }
    Ok(())
}

/// Planned cell changes by logical column, applied as one undo step.
#[derive(Debug)]
pub struct CellPlan {
    pub(crate) revision: u64,
    /// Changed cells by row id: (logical column, new text).
    pub(crate) changes: Vec<(u32, Vec<(u32, String)>)>,
    /// Rows added at the end, by logical column.
    pub(crate) append: Vec<Vec<String>>,
    /// Cells that change.
    pub cells: u64,
    /// The table has at least this many columns afterwards (join).
    pub(crate) grow_to: Option<u32>,
    /// Cells left alone (Paste Special: not a number, division by zero).
    pub skipped: u64,
}

impl CellPlan {
    /// The number of columns the table needs for this plan.
    pub fn widest(&self) -> u32 {
        self.changes
            .iter()
            .flat_map(|(_, c)| c.iter().map(|x| x.0 + 1))
            .chain(self.append.iter().map(|r| r.len() as u32))
            .chain(self.grow_to)
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn new(d: &Document) -> CellPlan {
        CellPlan {
            revision: d.revision(),
            changes: Vec::new(),
            append: Vec::new(),
            cells: 0,
            skipped: 0,
            grow_to: None,
        }
    }
}

/// Paste Special arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteOp {
    #[default]
    None,
    Add,
    Subtract,
    Multiply,
    Divide,
}

/// Where and how to paste a block of text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PasteSpec {
    /// Top-left shown row and column.
    pub row: u64,
    pub col: u32,
    pub op: PasteOp,
    /// Blank pasted cells leave the cell under them alone.
    pub skip_blanks: bool,
    /// Rows become columns.
    pub transpose: bool,
    pub read: ReadOptions,
}

enum Outcome {
    Write(String),
    Keep,
    Skip,
}

fn combine(dest: &str, src: &str, op: PasteOp, read: ReadOptions) -> Outcome {
    if op == PasteOp::None {
        return Outcome::Write(src.to_string());
    }
    if src.trim().is_empty() {
        return Outcome::Keep;
    }
    // Arithmetic leaves the cell alone when the pasted cell is not a number
    // (review note M4b-4: text must not replace the number underneath).
    let Some(s) = value::parse_number(src, read) else {
        return Outcome::Skip;
    };
    let d = if dest.trim().is_empty() {
        0.0
    } else {
        match value::parse_number(dest, read) {
            Some(d) => d,
            None => return Outcome::Skip,
        }
    };
    let v = match op {
        PasteOp::Add => d + s,
        PasteOp::Subtract => d - s,
        PasteOp::Multiply => d * s,
        PasteOp::Divide if s == 0.0 => return Outcome::Skip,
        PasteOp::Divide => d / s,
        PasteOp::None => s,
    };
    if !v.is_finite() {
        return Outcome::Skip;
    }
    Outcome::Write(format_number(v, read.decimal_comma))
}

/// Tab-separated text as the clipboard holds it: tabs between cells, line
/// breaks between rows, `"` quoting for cells with tabs, quotes or line
/// breaks (doubled `""` inside). A last empty line is dropped.
pub fn parse_tsv(text: &str, max_cells: u64) -> Result<Vec<Vec<String>>, Error> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut cells = 0u64;
    let mut chars = text.chars().peekable();
    let mut at_start = true;
    let mut quoted = false;
    let too_many = || Error::Invalid(format!("at most {max_cells} cells can be pasted at once"));
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' if at_start => {
                quoted = true;
                at_start = false;
            }
            '\t' => {
                row.push(std::mem::take(&mut cell));
                cells += 1;
                at_start = true;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut cell));
                cells += 1;
                rows.push(std::mem::take(&mut row));
                at_start = true;
            }
            _ => {
                cell.push(c);
                at_start = false;
            }
        }
        if cells > max_cells {
            return Err(too_many());
        }
    }
    if !cell.is_empty() || !row.is_empty() || quoted {
        row.push(cell);
        rows.push(row);
        cells += 1;
    }
    if cells > max_cells {
        return Err(too_many());
    }
    Ok(rows)
}

/// Which way Ctrl+arrow (and fill) goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    Up,
    #[default]
    Down,
    Left,
    Right,
}

/// A fill: the starting block and how far to extend it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FillSpec {
    /// Starting block (shown rows).
    pub row: u64,
    pub col: u32,
    pub rows: u64,
    pub cols: u32,
    pub dir: Dir,
    /// Rows (or columns) to fill beyond the block.
    pub count: u64,
    pub mode: FillMode,
    pub read: ReadOptions,
}

/// How to split a column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitColumn {
    pub col: u32,
    pub separator: String,
    /// At most this many parts (0: no limit); the last part keeps the rest.
    pub max_parts: u32,
    /// Keep the column and put every part in new columns.
    pub keep_original: bool,
    pub trim: bool,
    /// The first row holds titles: new columns get "title 2", ...
    pub keep_first: bool,
    pub mode: SplitMode,
    /// Character positions to split at (from 1), for [`SplitMode::Positions`].
    pub positions: Vec<u32>,
}

/// Where a column is split.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitMode {
    /// At each `separator`.
    #[default]
    Text,
    /// At the last `separator` only (two parts).
    Last,
    /// At any of the characters in `separator`.
    Any,
    /// At fixed character positions.
    Positions,
    /// Where letters meet digits ("AB12" → "AB", "12").
    LettersDigits,
}

/// The parts of `v` for a split (`None`: nothing to split).
pub(crate) fn split_parts<'a>(v: &'a str, spec: &SplitColumn, cap: usize) -> Option<Vec<&'a str>> {
    let sep = spec.separator.as_str();
    let parts: Vec<&str> = match spec.mode {
        SplitMode::Text => {
            if !v.contains(sep) {
                return None;
            }
            v.splitn(cap, sep).collect()
        }
        SplitMode::Last => {
            let (a, b) = v.rsplit_once(sep)?;
            vec![a, b]
        }
        SplitMode::Any => {
            let set: Vec<char> = sep.chars().collect();
            if !v.contains(set.as_slice()) {
                return None;
            }
            v.splitn(cap, set.as_slice()).collect()
        }
        SplitMode::Positions => {
            let mut cuts: Vec<usize> = Vec::new();
            for &p in &spec.positions {
                if p == 0 {
                    continue;
                }
                if let Some((i, _)) = v.char_indices().nth(p as usize) {
                    cuts.push(i);
                }
            }
            cuts.sort_unstable();
            cuts.dedup();
            if cuts.is_empty() {
                return None;
            }
            cuts.truncate(cap.saturating_sub(1));
            let mut out = Vec::with_capacity(cuts.len() + 1);
            let mut last = 0;
            for c in cuts {
                out.push(&v[last..c]);
                last = c;
            }
            out.push(&v[last..]);
            out
        }
        SplitMode::LettersDigits => {
            let mut out = Vec::new();
            let mut start = 0;
            let mut prev: Option<bool> = None;
            for (i, c) in v.char_indices() {
                let kind = if c.is_ascii_digit() {
                    Some(true)
                } else if c.is_alphabetic() {
                    Some(false)
                } else {
                    None
                };
                if let (Some(k), Some(p)) = (kind, prev) {
                    if k != p && out.len() + 1 < cap {
                        out.push(&v[start..i]);
                        start = i;
                    }
                }
                if kind.is_some() {
                    prev = kind;
                }
            }
            if out.is_empty() {
                return None;
            }
            out.push(&v[start..]);
            out
        }
    };
    Some(parts)
}

/// How to merge neighbouring columns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MergeColumns {
    pub col: u32,
    /// Columns merged, at least 2.
    pub count: u32,
    pub separator: String,
    pub skip_blanks: bool,
}

/// Column layout change plus cell changes, as one undo step.
#[derive(Debug)]
pub struct ColPlan {
    pub(crate) revision: u64,
    /// New columns: (logical position, how many).
    pub(crate) insert: Option<(u32, u32)>,
    /// Cell changes by row id: (logical column after the insert, text).
    pub(crate) changes: Vec<(u32, Vec<(u32, String)>)>,
    /// Logical columns removed after the changes, `(at, count)`, last first.
    pub(crate) remove: Vec<(u32, u32)>,
    /// A new order of every column (sort columns), as old logical numbers.
    pub(crate) order: Option<Vec<u32>>,
    pub cells: u64,
    /// Columns added (split) or removed (merge, blanks).
    pub cols: u32,
    /// The new columns replace the removed ones, emptied (a quick clear).
    pub(crate) swap: bool,
}

impl ColPlan {
    pub(crate) fn new(d: &Document) -> ColPlan {
        ColPlan {
            revision: d.revision(),
            insert: None,
            changes: Vec::new(),
            remove: Vec::new(),
            order: None,
            cells: 0,
            cols: 0,
            swap: false,
        }
    }

    /// Removing these logical columns (any order), last first, in runs.
    pub(crate) fn remove_cols(&mut self, mut cols: Vec<u32>) {
        cols.sort_unstable();
        cols.dedup();
        self.cols = cols.len() as u32;
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for c in cols {
            match runs.last_mut() {
                Some((s, n)) if *s + *n == c => *n += 1,
                _ => runs.push((c, 1)),
            }
        }
        runs.reverse();
        self.remove = runs;
    }

    /// The plan adds columns (`cols` of them).
    pub fn is_grow(&self) -> bool {
        self.insert.is_some()
    }

    /// Nothing would change.
    pub fn is_empty(&self) -> bool {
        self.insert.is_none()
            && self.changes.is_empty()
            && self.remove.is_empty()
            && self.order.is_none()
    }
}

impl Document {
    /// Shown rows `first..` as a range (`keep_first` leaves out the title row).
    pub(crate) fn data_rows(&self, keep_first: bool) -> std::ops::Range<u64> {
        let n = self.row_count();
        u64::from(keep_first && n > 0).min(n)..n
    }

    /// Run `f` over shown rows `rows` in parallel chunks, in order.
    pub(crate) fn par_rows<'a, T: Send>(
        &'a self,
        rows: std::ops::Range<u64>,
        progress: &Progress,
        f: impl Fn(u64, u32, &mut Vec<Cow<'a, str>>) -> Result<Option<T>, Error> + Sync,
    ) -> Result<Vec<T>, Error> {
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1.saturating_sub(r0));
        let chunks = (r1.saturating_sub(r0) as usize).div_ceil(CHUNK);
        let parts: Vec<Vec<T>> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Vec<T>, Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    if let Some(t) = f(r, id, &mut buf)? {
                        out.push(t);
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        Ok(parts.into_iter().flatten().collect())
    }

    /// Like [`Document::par_rows`] over every row in document order (shown
    /// or hidden): for column-layout changes, which affect every row.
    pub(crate) fn par_all_rows<'a, T: Send>(
        &'a self,
        from: u64,
        progress: &Progress,
        f: impl Fn(u64, u32, &mut Vec<Cow<'a, str>>) -> Result<Option<T>, Error> + Sync,
    ) -> Result<Vec<T>, Error> {
        let total = self.total_rows();
        progress.start(total.saturating_sub(from));
        let chunks = (total.saturating_sub(from) as usize).div_ceil(CHUNK);
        let parts: Vec<Vec<T>> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Vec<T>, Error> {
                let s = from + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(total);
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for p in s..e {
                    let id = self.doc_id_at(p).ok_or(Error::OutOfRange)?;
                    if let Some(t) = f(p, id, &mut buf)? {
                        out.push(t);
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        Ok(parts.into_iter().flatten().collect())
    }

    // ---- paste ----

    /// Plan pasting `grid` (rows of cells) at `spec.row`/`spec.col`. Rows
    /// past the end are added. With a filter on, pasted rows go to the next
    /// shown rows.
    pub fn plan_paste(
        &self,
        spec: &PasteSpec,
        grid: Vec<Vec<String>>,
        progress: &Progress,
    ) -> Result<CellPlan, Error> {
        self.ready_for_bulk()?;
        let grid = if spec.transpose {
            transpose(grid)
        } else {
            grid
        };
        let height = grid.len() as u64;
        let width = grid.iter().map(Vec::len).max().unwrap_or(0) as u32;
        if height.saturating_mul(width as u64) > MAX_BLOCK_CELLS {
            return Err(Error::Invalid(format!(
                "at most {MAX_BLOCK_CELLS} cells can be pasted at once"
            )));
        }
        if spec.col as u64 + width as u64 > MAX_EDIT_COL as u64 + 1 {
            return Err(Error::OutOfRange);
        }
        let n = self.row_count();
        if spec.row > n {
            return Err(Error::OutOfRange);
        }
        let cols: Vec<u32> = (spec.col..spec.col + width).collect();
        let reader = Cols::new(self, &cols);
        let mut plan = CellPlan::new(self);
        let inside = (n - spec.row).min(height);
        let needs_dest = spec.op != PasteOp::None;
        type Row = (u32, Vec<(u32, String)>, u64, u64);
        let rows: Vec<Row> =
            self.par_rows(spec.row..spec.row + inside, progress, |r, id, buf| {
                let line = &grid[(r - spec.row) as usize];
                if needs_dest {
                    reader.read(self, id, buf);
                }
                let (mut cells, mut skipped) = (Vec::new(), 0u64);
                for (k, src) in line.iter().enumerate() {
                    if spec.skip_blanks && src.is_empty() {
                        continue;
                    }
                    let dest = if needs_dest { reader.get(buf, k) } else { "" };
                    match combine(dest, src, spec.op, spec.read) {
                        Outcome::Write(v) => {
                            if !needs_dest || v != dest {
                                cells.push((spec.col + k as u32, v));
                            }
                        }
                        Outcome::Keep => {}
                        Outcome::Skip => skipped += 1,
                    }
                }
                let c = cells.len() as u64;
                Ok((!cells.is_empty() || skipped > 0).then_some((id, cells, c, skipped)))
            })?;
        for (id, cells, c, s) in rows {
            plan.cells += c;
            plan.skipped += s;
            if !cells.is_empty() {
                plan.changes.push((id, cells));
            }
        }
        for line in grid.into_iter().skip(inside as usize) {
            let mut row = vec![String::new(); spec.col as usize + line.len()];
            for (k, src) in line.into_iter().enumerate() {
                if let Outcome::Write(v) = combine("", &src, spec.op, spec.read) {
                    plan.cells += u64::from(!v.is_empty());
                    row[spec.col as usize + k] = v;
                }
            }
            plan.append.push(row);
        }
        Ok(plan)
    }

    /// Apply a [`CellPlan`] as one undo step. Fails with
    /// [`Error::Changed`] if the document changed since it was planned.
    pub fn apply_cells(&mut self, plan: CellPlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        self.ready_for_bulk()?;
        let widest = plan
            .changes
            .iter()
            .flat_map(|(_, c)| c.iter().map(|x| x.0 + 1))
            .chain(plan.append.iter().map(|r| r.len() as u32))
            .chain(plan.grow_to)
            .max()
            .unwrap_or(0);
        let grow = if widest > 0 && self.phys(widest - 1).is_none() {
            let have = self.col_count();
            Some((have, self.new_col_ids(widest - have)?))
        } else {
            None
        };
        let phys = |d: &Document, c: u32| match d.phys(c) {
            Some(p) => p,
            None => {
                let (base, ids) = grow.as_ref().expect("grown to the widest column");
                ids[(c - base) as usize]
            }
        };
        let changes = plan
            .changes
            .into_iter()
            .map(|(id, cells)| {
                (
                    id,
                    cells.into_iter().map(|(c, v)| (phys(self, c), v)).collect(),
                )
            })
            .collect();
        let states = self.patch_states(changes)?;
        let first = self.new_row_ids(plan.append.len() as u32)?;
        let append = plan
            .append
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    first + i as u32,
                    Some(Edit::Full(self.to_physical(r, grow.as_ref()))),
                )
            })
            .collect();
        self.push_step(Step {
            col_insert: grow,
            states,
            append,
            ..Default::default()
        });
        Ok(())
    }

    // ---- fill ----

    /// Plan a fill from a starting block. Down/up go over shown rows and
    /// stop at the document's ends; right/left may add columns.
    pub fn plan_fill(&self, spec: &FillSpec, progress: &Progress) -> Result<CellPlan, Error> {
        self.ready_for_bulk()?;
        let n = self.row_count();
        let rows_end = spec.row.checked_add(spec.rows).ok_or(Error::OutOfRange)?;
        let cols_end = spec.col.checked_add(spec.cols).ok_or(Error::OutOfRange)?;
        if spec.rows == 0 || spec.cols == 0 || rows_end > n || cols_end > self.col_count() {
            return Err(Error::OutOfRange);
        }
        let vertical = matches!(spec.dir, Dir::Down | Dir::Up);
        // Cells to write, after clipping at the document's ends.
        let count = match spec.dir {
            Dir::Down => spec.count.min(n - rows_end),
            Dir::Up => spec.count.min(spec.row),
            Dir::Right => spec.count.min((MAX_EDIT_COL + 1 - cols_end) as u64),
            Dir::Left => spec.count.min(spec.col as u64),
        };
        let lines = if vertical {
            spec.cols as u64
        } else {
            spec.rows
        };
        let start_cells = spec.rows.saturating_mul(spec.cols as u64);
        if count.saturating_mul(lines) > MAX_BLOCK_CELLS || start_cells > MAX_BLOCK_CELLS {
            return Err(Error::Invalid(format!(
                "at most {MAX_BLOCK_CELLS} cells can be filled at once"
            )));
        }
        let mut plan = CellPlan::new(self);
        if count == 0 {
            return Ok(plan);
        }
        // The starting block, row by row.
        let cols: Vec<u32> = (spec.col..cols_end).collect();
        let reader = Cols::new(self, &cols);
        let block: Vec<Vec<String>> =
            self.par_rows(spec.row..rows_end, progress, |_, id, buf| {
                reader.read(self, id, buf);
                Ok(Some(
                    (0..cols.len())
                        .map(|k| reader.get(buf, k).to_string())
                        .collect(),
                ))
            })?;
        let series: Vec<Series> = (0..lines as usize)
            .map(|l| {
                let start: Vec<String> = if vertical {
                    block.iter().map(|r| r[l].clone()).collect()
                } else {
                    block[l].clone()
                };
                Series::new(&start, spec.mode, spec.read)
            })
            .collect::<Result<_, _>>()?;
        let made = std::sync::atomic::AtomicUsize::new(0);
        let too_much = || Error::Invalid("this fill would create too much text at once".into());
        // Position k (0-based) beyond the block, as a series index.
        let len = if vertical {
            spec.rows
        } else {
            spec.cols as u64
        } as i64;
        let index = |k: u64| match spec.dir {
            Dir::Down | Dir::Right => len + k as i64,
            Dir::Up | Dir::Left => -1 - k as i64,
        };
        if vertical {
            let target = match spec.dir {
                Dir::Down => rows_end..rows_end + count,
                _ => spec.row - count..spec.row,
            };
            let rows: Vec<(u32, Vec<(u32, String)>)> =
                self.par_rows(target, progress, |r, id, _| {
                    let k = match spec.dir {
                        Dir::Down => r - rows_end,
                        _ => spec.row - 1 - r,
                    };
                    let mut cells = Vec::with_capacity(lines as usize);
                    for (l, s) in series.iter().enumerate() {
                        if let Some(v) = s.at(index(k)) {
                            if made.fetch_add(v.len(), std::sync::atomic::Ordering::Relaxed)
                                + v.len()
                                > MAX_BLOCK_BYTES
                            {
                                return Err(too_much());
                            }
                            cells.push((spec.col + l as u32, v));
                        }
                    }
                    Ok((!cells.is_empty()).then_some((id, cells)))
                })?;
            plan.cells = rows.iter().map(|r| r.1.len() as u64).sum();
            plan.changes = rows;
        } else {
            for (l, s) in series.iter().enumerate() {
                let id = self.id_at(spec.row + l as u64).ok_or(Error::OutOfRange)?;
                let mut cells = Vec::new();
                for k in 0..count {
                    let Some(v) = s.at(index(k)) else { break };
                    if made.fetch_add(v.len(), std::sync::atomic::Ordering::Relaxed) + v.len()
                        > MAX_BLOCK_BYTES
                    {
                        return Err(too_much());
                    }
                    let c = match spec.dir {
                        Dir::Right => cols_end + k as u32,
                        _ => spec.col - 1 - k as u32,
                    };
                    cells.push((c, v));
                }
                plan.cells += cells.len() as u64;
                if !cells.is_empty() {
                    plan.changes.push((id, cells));
                }
            }
        }
        Ok(plan)
    }

    // ---- data edges ----

    fn cell_empty(&self, row: u64, col: u32) -> bool {
        let Some(p) = self.phys(col) else { return true };
        let Some(id) = self.id_at(row) else {
            return true;
        };
        let mut buf = Vec::new();
        self.read_cells(id, &[p], &mut buf);
        buf[0].is_empty()
    }

    /// First shown row in `rows` (searched from the end when `backward`)
    /// whose cell in `col` is empty (`want_empty`) or not.
    fn seek(
        &self,
        rows: std::ops::Range<u64>,
        backward: bool,
        col: u32,
        want_empty: bool,
    ) -> Option<u64> {
        let p = self.phys(col);
        let test = |r: u64| -> bool {
            let Some(id) = self.id_at(r) else {
                return false;
            };
            let empty = match p {
                None => true,
                Some(p) => {
                    let mut buf = Vec::new();
                    self.read_cells(id, &[p], &mut buf);
                    buf[0].is_empty()
                }
            };
            empty == want_empty
        };
        let len = rows.end.saturating_sub(rows.start);
        let chunks = len.div_ceil(CHUNK as u64);
        for c in 0..chunks {
            let (s, e) = if backward {
                let e = rows.end - c * CHUNK as u64;
                (e.saturating_sub(CHUNK as u64).max(rows.start), e)
            } else {
                let s = rows.start + c * CHUNK as u64;
                (s, (s + CHUNK as u64).min(rows.end))
            };
            let hit = if backward {
                (s..e).into_par_iter().find_last(|&r| test(r))
            } else {
                (s..e).into_par_iter().find_first(|&r| test(r))
            };
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// Where Ctrl+arrow lands from a cell (Excel's rule): from a filled cell
    /// with a filled neighbour, the last filled cell before a blank;
    /// otherwise the next filled cell, or the edge of the table.
    pub fn edge(&self, row: u64, col: u32, dir: Dir) -> Result<(u64, u32), Error> {
        let n = self.row_count();
        let w = self.col_count();
        if row >= n.max(1) || col >= w.max(1) {
            return Err(Error::OutOfRange);
        }
        if n == 0 {
            return Ok((0, col));
        }
        match dir {
            Dir::Down | Dir::Up => {
                let down = dir == Dir::Down;
                let next = if down { row + 1 } else { row.wrapping_sub(1) };
                if (down && next >= n) || (!down && row == 0) {
                    return Ok((row, col));
                }
                let filled_run = !self.cell_empty(row, col) && !self.cell_empty(next, col);
                let target = if filled_run {
                    let range = if down { next..n } else { 0..next + 1 };
                    match self.seek(range, !down, col, true) {
                        Some(r) if down => r - 1,
                        Some(r) => r + 1,
                        None if down => n - 1,
                        None => 0,
                    }
                } else {
                    let range = if down { next..n } else { 0..next + 1 };
                    self.seek(range, !down, col, false)
                        .unwrap_or(if down { n - 1 } else { 0 })
                };
                Ok((target, col))
            }
            Dir::Left | Dir::Right => {
                let values = self.row_values(row)?;
                let empty = |c: u32| values.get(c as usize).is_none_or(String::is_empty);
                let right = dir == Dir::Right;
                if (right && col + 1 >= w) || (!right && col == 0) {
                    return Ok((row, col));
                }
                let step = |c: u32| if right { c + 1 } else { c - 1 };
                let last = if right { w - 1 } else { 0 };
                let mut c = step(col);
                if !empty(col) && !empty(c) {
                    while c != last && !empty(step(c)) {
                        c = step(c);
                    }
                } else {
                    while c != last && empty(c) {
                        c = step(c);
                    }
                }
                Ok((row, c))
            }
        }
    }

    // ---- split and merge columns ----

    /// Plan splitting a column at a separator into new columns to its right.
    pub fn plan_split_column(
        &self,
        spec: &SplitColumn,
        progress: &Progress,
    ) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[spec.col])?;
        check_separator(&spec.separator)?;
        let needs_sep = matches!(
            spec.mode,
            SplitMode::Text | SplitMode::Last | SplitMode::Any
        );
        if needs_sep && spec.separator.is_empty() {
            return Err(Error::Invalid("choose a separator to split at".into()));
        }
        if spec.mode == SplitMode::Positions && spec.positions.iter().all(|&p| p == 0) {
            return Err(Error::Invalid("give the positions to split at".into()));
        }
        if spec.positions.len() > MAX_NEW_COLS as usize {
            return Err(Error::Invalid(format!(
                "a split can add at most {MAX_NEW_COLS} columns"
            )));
        }
        let cap = if spec.max_parts == 0 {
            usize::MAX
        } else {
            spec.max_parts as usize
        };
        let reader = Cols::new(self, &[spec.col]);
        let first = u64::from(spec.keep_first && self.total_rows() > 0);
        // Each new cell is counted as it is made (text plus bookkeeping), so
        // a huge split is refused before it is all in memory.
        let spent = std::sync::atomic::AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        // Every row (hidden ones too): the new columns exist for all rows.
        let split: Vec<(u32, Vec<String>)> = self.par_all_rows(first, progress, |_, id, buf| {
            reader.read(self, id, buf);
            let v = reader.get(buf, 0);
            let Some(found) = split_parts(v, spec, cap) else {
                return Ok(None);
            };
            let parts: Vec<String> = found
                .into_iter()
                .take(MAX_NEW_COLS as usize + 2)
                .map(|p| {
                    if spec.trim {
                        p.trim().to_string()
                    } else {
                        p.to_string()
                    }
                })
                .collect();
            for p in &parts {
                if !crate::find::charge_within(&spent, p.len(), budget) {
                    return Err(Error::OutOfMemory(
                    "this split would make too many new cells at once; split fewer rows or set a limit on the parts",
                ));
                }
            }
            Ok(Some((id, parts)))
        })?;
        let most = split.iter().map(|(_, p)| p.len()).max().unwrap_or(0) as u32;
        let mut plan = ColPlan::new(self);
        if most < 2 {
            return Ok(plan);
        }
        let new = if spec.keep_original { most } else { most - 1 };
        if new > MAX_NEW_COLS {
            return Err(Error::Invalid(format!(
                "a split can add at most {MAX_NEW_COLS} columns; set a limit on the parts"
            )));
        }
        let at = spec.col + 1;
        plan.insert = Some((at, new));
        plan.cols = new;
        // Where part k goes (logical column after the insert).
        let slot = |k: usize| -> u32 {
            if spec.keep_original {
                at + k as u32
            } else if k == 0 {
                spec.col
            } else {
                at + k as u32 - 1
            }
        };
        if first == 1 {
            let id = self.doc_id_at(0).expect("a first row");
            let title = self.cell_by_id(id, spec.col).unwrap_or_default();
            let mut cells = Vec::new();
            for k in 0..most as usize {
                let s = slot(k);
                if s != spec.col {
                    cells.push((s, format!("{title} {}", k + 1)));
                }
            }
            plan.cells += cells.len() as u64;
            plan.changes.push((id, cells));
        }
        for (id, parts) in split {
            let mut cells = Vec::with_capacity(parts.len());
            for (k, p) in parts.into_iter().enumerate() {
                if slot(k) == spec.col || !p.is_empty() {
                    cells.push((slot(k), p));
                }
            }
            plan.cells += cells.len() as u64;
            plan.changes.push((id, cells));
        }
        Ok(plan)
    }

    /// Plan joining `count` columns from `col` into the first, removing the
    /// others. Every row is merged, hidden ones too.
    pub fn plan_merge_columns(
        &self,
        spec: &MergeColumns,
        progress: &Progress,
    ) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        check_separator(&spec.separator)?;
        let end = spec.col.checked_add(spec.count).ok_or(Error::OutOfRange)?;
        if spec.count < 2 || end > self.col_count() {
            return Err(Error::Invalid(
                "choose two or more neighbouring columns to merge".into(),
            ));
        }
        let cols: Vec<u32> = (spec.col..end).collect();
        let reader = Cols::new(self, &cols);
        let made = std::sync::atomic::AtomicUsize::new(0);
        let rows: Vec<(u32, String)> = self.par_all_rows(0, progress, |_, id, buf| {
            reader.read(self, id, buf);
            let mut out = String::new();
            let mut any = false;
            for k in 0..cols.len() {
                let v = reader.get(buf, k);
                if spec.skip_blanks && v.is_empty() {
                    continue;
                }
                if any {
                    out.push_str(&spec.separator);
                }
                out.push_str(v);
                any = true;
                if out.len() > MAX_CELL_BYTES {
                    return Err(Error::Invalid("a merged cell would be too long".into()));
                }
            }
            if out == reader.get(buf, 0) {
                return Ok(None);
            }
            if made.fetch_add(out.len(), std::sync::atomic::Ordering::Relaxed) + out.len()
                > MAX_BLOCK_BYTES * 4
            {
                return Err(Error::Invalid(
                    "this merge would create too much text at once".into(),
                ));
            }
            Ok(Some((id, out)))
        })?;
        let mut plan = ColPlan::new(self);
        plan.cells = rows.len() as u64;
        plan.changes = rows
            .into_iter()
            .map(|(id, v)| (id, vec![(spec.col, v)]))
            .collect();
        plan.remove_cols((spec.col + 1..end).collect());
        Ok(plan)
    }

    /// Plan putting columns `col..col+count` (all with `count == 0`) in the
    /// order of their titles in the first row (natural order).
    pub fn plan_sort_columns(
        &self,
        col: u32,
        count: u32,
        descending: bool,
    ) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        let w = self.col_count();
        let end = if count == 0 {
            w
        } else {
            col.checked_add(count).ok_or(Error::OutOfRange)?
        };
        if col >= end || end > w {
            return Err(Error::OutOfRange);
        }
        let titles = match self.doc_id_at(0) {
            Some(_) => self.row_values(0)?,
            None => Vec::new(),
        };
        let mut keyed: Vec<(String, u32)> = (col..end)
            .map(|c| {
                let mut k = String::new();
                value::text_key(
                    titles.get(c as usize).map_or("", String::as_str),
                    false,
                    &mut k,
                );
                (k, c)
            })
            .collect();
        keyed.sort_by(|a, b| {
            let o = value::natural_cmp(a.0.as_bytes(), b.0.as_bytes());
            if descending {
                o.reverse()
            } else {
                o
            }
        });
        let mut plan = ColPlan::new(self);
        let order: Vec<u32> = (0..col)
            .chain(keyed.into_iter().map(|x| x.1))
            .chain(end..w)
            .collect();
        if order.iter().enumerate().any(|(i, &c)| i as u32 != c) {
            plan.order = Some(order);
            plan.cols = end - col;
        }
        Ok(plan)
    }

    /// Apply a [`ColPlan`] as one undo step.
    pub fn apply_cols(&mut self, plan: ColPlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        self.ready_for_bulk()?;
        let ins = match plan.insert {
            Some((at, n)) => Some((at, self.new_col_ids(n)?)),
            None => None,
        };
        if plan.swap {
            if let Some((at, ids)) = &ins {
                for (k, &id) in ids.iter().enumerate() {
                    if let Some(old) = self.phys(at + k as u32) {
                        self.swapped.insert(id, old);
                    }
                }
            }
        }
        let phys = |d: &Document, c: u32| -> Result<u32, Error> {
            match &ins {
                Some((at, ids)) if c >= *at => {
                    let n = ids.len() as u32;
                    if c < at + n {
                        Ok(ids[(c - at) as usize])
                    } else {
                        d.phys(c - n).ok_or(Error::OutOfRange)
                    }
                }
                _ => d.phys(c).ok_or(Error::OutOfRange),
            }
        };
        let mut changes = Vec::with_capacity(plan.changes.len());
        for (id, cells) in plan.changes {
            let mut out = Vec::with_capacity(cells.len());
            for (c, v) in cells {
                out.push((phys(self, c)?, v));
            }
            changes.push((id, out));
        }
        let states = self.patch_states(changes)?;
        let map = match plan.order {
            Some(order) => {
                let m = self.map_list();
                Some(order.into_iter().map(|c| m[c as usize]).collect())
            }
            None => None,
        };
        self.push_step(Step {
            col_insert: ins,
            states,
            col_remove: plan.remove,
            map,
            ..Default::default()
        });
        Ok(())
    }

    /// One cell of row `id` by logical column.
    pub(crate) fn cell_by_id(&self, id: u32, col: u32) -> Option<String> {
        let p = self.phys(col)?;
        let mut buf = Vec::new();
        self.read_cells(id, &[p], &mut buf);
        Some(buf[0].to_string())
    }
}

fn transpose(grid: Vec<Vec<String>>) -> Vec<Vec<String>> {
    let w = grid.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = vec![vec![String::new(); grid.len()]; w];
    for (r, line) in grid.into_iter().enumerate() {
        for (c, v) in line.into_iter().enumerate() {
            out[c][r] = v;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smarter_splits() {
        let sp = |mode: SplitMode, sep: &str, pos: Vec<u32>, v: &str| {
            let spec = SplitColumn {
                separator: sep.into(),
                mode,
                positions: pos,
                ..Default::default()
            };
            split_parts(v, &spec, usize::MAX)
                .map(|p| p.into_iter().map(str::to_string).collect::<Vec<_>>())
        };
        assert_eq!(
            sp(SplitMode::Last, " ", vec![], "Mary Ann Lee").unwrap(),
            ["Mary Ann", "Lee"]
        );
        assert_eq!(
            sp(SplitMode::Any, ",;/", vec![], "a,b;c/d").unwrap(),
            ["a", "b", "c", "d"]
        );
        assert_eq!(
            sp(SplitMode::Positions, "", vec![5, 2], "AB123é9").unwrap(),
            ["AB", "123", "é9"]
        );
        assert_eq!(sp(SplitMode::Positions, "", vec![99], "abc"), None);
        assert_eq!(
            sp(SplitMode::LettersDigits, "", vec![], "AB-12c").unwrap(),
            ["AB-", "12", "c"]
        );
        assert_eq!(sp(SplitMode::LettersDigits, "", vec![], "abc"), None);
        assert_eq!(sp(SplitMode::Text, ",", vec![], "abc"), None);
        let mut d = Document::from_bytes(
            b"code
AB12
X
"
            .to_vec(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let spec = SplitColumn {
            col: 0,
            mode: SplitMode::LettersDigits,
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_split_column(&spec, &Progress::new()).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(d.row_values(1).unwrap(), ["AB", "12"]);
    }
    use crate::fill::DateUnit;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn all(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    fn rows(v: &[&[&str]]) -> Vec<Vec<String>> {
        v.iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    fn saved(d: &mut Document) -> String {
        let dir = std::env::temp_dir().join(format!("nde-ops-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(format!("{:p}.csv", d as *const _));
        d.save(crate::SaveOptions {
            path: Some(p.clone()),
            ..Default::default()
        })
        .unwrap();
        String::from_utf8(std::fs::read(p).unwrap()).unwrap()
    }

    #[test]
    fn tsv_parsing() {
        assert_eq!(
            parse_tsv("a\tb\r\n\"x\ty\"\t\"say \"\"hi\"\"\"\n", 100).unwrap(),
            rows(&[&["a", "b"], &["x\ty", "say \"hi\""]])
        );
        assert_eq!(
            parse_tsv("\"two\nlines\"", 100).unwrap(),
            rows(&[&["two\nlines"]])
        );
        assert_eq!(parse_tsv("", 100).unwrap(), Vec::<Vec<String>>::new());
        assert!(parse_tsv("a\tb\tc", 2).is_err());
    }

    #[test]
    fn paste_values_ops_transpose_and_append() {
        let mut d = doc("a,b\n1,2\n3,x\n");
        let p = Progress::new();
        let spec = PasteSpec {
            row: 1,
            col: 0,
            op: PasteOp::Add,
            ..Default::default()
        };
        let plan = d
            .plan_paste(&spec, rows(&[&["10", "10"], &["", "5"], &["7", "8"]]), &p)
            .unwrap();
        assert_eq!((plan.cells, plan.skipped), (4, 1));
        // A text cell in an arithmetic paste is skipped, not pasted.
        let t = d
            .plan_paste(
                &PasteSpec {
                    row: 1,
                    col: 0,
                    op: PasteOp::Multiply,
                    ..Default::default()
                },
                rows(&[&["x"]]),
                &p,
            )
            .unwrap();
        assert_eq!((t.cells, t.skipped), (0, 1));
        d.apply_cells(plan).unwrap();
        assert_eq!(
            all(&d),
            rows(&[&["a", "b"], &["11", "12"], &["3", "x"], &["7", "8"]])
        );
        assert!(d.undo());
        assert_eq!(all(&d), rows(&[&["a", "b"], &["1", "2"], &["3", "x"]]));
        // Transpose, skip blanks.
        let spec = PasteSpec {
            row: 0,
            col: 1,
            transpose: true,
            skip_blanks: true,
            ..Default::default()
        };
        let plan = d.plan_paste(&spec, rows(&[&["p", "", "r"]]), &p).unwrap();
        d.apply_cells(plan).unwrap();
        assert_eq!(all(&d), rows(&[&["a", "p"], &["1", "2"], &["3", "r"]]));
        assert_eq!(saved(&mut d), "a,p\n1,2\n3,r\n");
    }

    #[test]
    fn paste_grows_mapped_columns() {
        let mut d = doc("a,b\n1,2\n");
        d.move_cols(0, 1, 1).unwrap();
        let p = Progress::new();
        let spec = PasteSpec {
            row: 1,
            col: 1,
            ..Default::default()
        };
        let plan = d.plan_paste(&spec, rows(&[&["x", "y", "z"]]), &p).unwrap();
        d.apply_cells(plan).unwrap();
        assert_eq!(all(&d), rows(&[&["b", "a", "", ""], &["2", "x", "y", "z"]]));
        assert!(d.undo());
        assert_eq!(all(&d), rows(&[&["b", "a"], &["2", "1"]]));
    }

    #[test]
    fn fill_directions_and_filters() {
        let mut d = doc("n,t\n1,Mon\n2,\n,\n,\n");
        let p = Progress::new();
        let spec = FillSpec {
            row: 1,
            col: 0,
            rows: 2,
            cols: 1,
            dir: Dir::Down,
            count: 99,
            ..Default::default()
        };
        let plan = d.plan_fill(&spec, &p).unwrap();
        assert_eq!(plan.cells, 2);
        d.apply_cells(plan).unwrap();
        assert_eq!(
            all(&d).iter().map(|r| r[0].clone()).collect::<Vec<_>>(),
            ["n", "1", "2", "3", "4"]
        );
        // Right from a day name adds a column.
        let spec = FillSpec {
            row: 1,
            col: 1,
            rows: 1,
            cols: 1,
            dir: Dir::Right,
            count: 2,
            ..Default::default()
        };
        let plan = d.plan_fill(&spec, &p).unwrap();
        d.apply_cells(plan).unwrap();
        assert_eq!(all(&d)[1], ["1", "Mon", "Tue", "Wed"]);
        // Up.
        let mut d = doc("x\n\n\n2026-11-12\n");
        let spec = FillSpec {
            row: 3,
            col: 0,
            rows: 1,
            cols: 1,
            dir: Dir::Up,
            count: 2,
            mode: FillMode::Date {
                unit: DateUnit::Day,
                step: 1,
            },
            ..Default::default()
        };
        let plan = d.plan_fill(&spec, &p).unwrap();
        d.apply_cells(plan).unwrap();
        assert_eq!(
            all(&d),
            rows(&[&["x"], &["2026-11-10"], &["2026-11-11"], &["2026-11-12"]])
        );
        // With a filter: only shown rows are filled.
        let mut d = doc("k,v\na,1\nb,\na,\na,\n");
        d.set_filter(&crate::filter::FilterSpec {
            columns: vec![crate::filter::ColumnFilter {
                col: 0,
                rule: crate::filter::Rule::Values {
                    values: vec!["a".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        let spec = FillSpec {
            row: 1,
            col: 1,
            rows: 1,
            cols: 1,
            dir: Dir::Down,
            count: 5,
            mode: FillMode::AutoSwap,
            ..Default::default()
        };
        let plan = d.plan_fill(&spec, &p).unwrap();
        d.apply_cells(plan).unwrap();
        d.clear_filter();
        assert_eq!(
            all(&d).iter().map(|r| r[1].clone()).collect::<Vec<_>>(),
            ["v", "1", "", "2", "3"]
        );
    }

    #[test]
    fn edges() {
        let d = doc("a,b,c,d\n1,,3,4\n2,,,\n,,,\n5,,,\n");
        assert_eq!(d.edge(0, 0, Dir::Down).unwrap(), (2, 0));
        assert_eq!(d.edge(2, 0, Dir::Down).unwrap(), (4, 0));
        assert_eq!(d.edge(4, 0, Dir::Down).unwrap(), (4, 0));
        assert_eq!(d.edge(4, 0, Dir::Up).unwrap(), (2, 0));
        assert_eq!(d.edge(0, 1, Dir::Down).unwrap(), (4, 1));
        assert_eq!(d.edge(1, 0, Dir::Right).unwrap(), (1, 2));
        assert_eq!(d.edge(1, 2, Dir::Right).unwrap(), (1, 3));
        assert_eq!(d.edge(1, 3, Dir::Left).unwrap(), (1, 2));
        assert_eq!(d.edge(1, 2, Dir::Left).unwrap(), (1, 0));
    }

    #[test]
    fn split_merge_and_sort_columns() {
        let mut d = doc("name,x\nAda Lovelace,1\nAlan,2\nGrace B Hopper,3\n");
        let p = Progress::new();
        let spec = SplitColumn {
            col: 0,
            separator: " ".into(),
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_split_column(&spec, &p).unwrap();
        assert_eq!(plan.cols, 2);
        d.apply_cols(plan).unwrap();
        assert_eq!(
            all(&d),
            rows(&[
                &["name", "name 2", "name 3", "x"],
                &["Ada", "Lovelace", "", "1"],
                &["Alan", "", "", "2"],
                &["Grace", "B", "Hopper", "3"],
            ])
        );
        let merge = MergeColumns {
            col: 0,
            count: 3,
            separator: " ".into(),
            skip_blanks: true,
        };
        let plan = d.plan_merge_columns(&merge, &p).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(all(&d)[3], ["Grace B Hopper", "3"]);
        assert_eq!(all(&d)[0], ["name name 2 name 3", "x"]);
        let plan = d.plan_sort_columns(0, 0, true).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(all(&d)[1], ["1", "Ada Lovelace"]);
        while d.undo() {}
        assert_eq!(
            saved(&mut d),
            "name,x\nAda Lovelace,1\nAlan,2\nGrace B Hopper,3\n"
        );
    }

    #[test]
    fn stale_plans_are_refused() {
        let mut d = doc("a\n1\n");
        let p = Progress::new();
        let plan = d
            .plan_paste(&PasteSpec::default(), rows(&[&["z"]]), &p)
            .unwrap();
        d.set_cell(1, 0, "2".into()).unwrap();
        assert!(matches!(d.apply_cells(plan), Err(Error::Changed)));
        let plan = d.plan_sort_columns(0, 0, false).unwrap();
        d.set_cell(1, 0, "3".into()).unwrap();
        assert!(matches!(d.apply_cols(plan), Err(Error::Changed)));
    }
}
