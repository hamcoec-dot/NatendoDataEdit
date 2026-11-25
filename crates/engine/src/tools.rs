//! Row and column tools of Milestone 10: fill blanks, count or blank
//! repeats, breaks between groups, delete hidden rows, reshape a column into
//! a table and back, random samples, jumps to the next change or the
//! largest value, comparing two tables, and splitting a table into parts.
//!
//! Like the other row tools, these act on shown rows only (hidden or
//! filtered-out rows never change), and "first row is header" keeps the
//! title row out. Each change is one undo step.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::{check_cols, ColPlan, Cols, CHUNK, MAX_CELL_BYTES};
use crate::reshape::{
    push_line, too_big, Duplicates, RowPlan, Table, MAX_TABLE_BYTES, MAX_TABLE_CELLS,
};
use crate::value::ReadOptions;
use crate::Error;

/// Most columns one of these tools reads at once.
pub const MAX_TOOL_COLS: usize = 16_384;
/// Most blank rows put between groups.
pub const MAX_BREAK_ROWS: u32 = 100;
/// Most cells per row when reshaping a column.
pub const MAX_PER_ROW: u32 = 16_384;
/// Longest value (in characters) type-ahead suggests.
pub const TYPEAHEAD_CHARS: usize = 200;
/// Most text one type-ahead list keeps, in bytes (all chunks together).
pub const TYPEAHEAD_BYTES: usize = 64 << 20;
/// Most parts a split into files may make.
pub const MAX_PARTS: usize = 10_000;
/// What one planned cell costs in a row's change list (column + string
/// header), on top of the per-change cost the budget already counts.
const CELL_COST: usize = 32;

fn bad(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

fn blank(v: &str, spaces: bool) -> bool {
    if spaces {
        v.trim().is_empty()
    } else {
        v.is_empty()
    }
}

fn check_tool_cols(d: &Document, cols: &[u32]) -> Result<(), Error> {
    if cols.is_empty() {
        return Err(bad("choose at least one column"));
    }
    if cols.len() > MAX_TOOL_COLS {
        return Err(bad(format!("at most {MAX_TOOL_COLS} columns at once")));
    }
    check_cols(d, cols)
}

// ---- fill blanks ----

/// Where empty cells get their text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FillFrom {
    /// The nearest filled cell above (in shown rows).
    #[default]
    Above,
    /// The nearest filled cell below.
    Below,
    /// The same text in every empty cell.
    Value { text: String },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FillBlanks {
    pub cols: Vec<u32>,
    pub from: FillFrom,
    /// Cells holding only spaces count as empty.
    pub spaces: bool,
    pub keep_first: bool,
}

/// One chunk's view of one column for fill blanks.
#[derive(Default)]
struct FillChunk {
    /// First and last filled values of the chunk.
    first: Option<String>,
    last: Option<String>,
    /// Empty cells: row id and the value found inside the chunk (if any).
    blanks: Vec<(u32, Option<String>)>,
}

// ---- repeats ----

/// Count how often each key occurs, in a new column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Repeats {
    pub cols: Vec<u32>,
    pub match_case: bool,
    pub trim: bool,
    pub keep_first: bool,
    /// The new column's title (with `keep_first`).
    pub title: String,
}

// ---- breaks ----

/// Blank rows between groups, or every `every` rows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Breaks {
    /// A new group starts where this column's value changes.
    pub col: Option<u32>,
    /// Or: a break after every this many rows (with `col` unset).
    pub every: u64,
    /// Blank rows per break (1 to 100).
    pub rows: u32,
    pub keep_first: bool,
}

// ---- reshape ----

/// One column's cells laid out `per_row` to a row, in a new table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reshape {
    pub col: u32,
    pub per_row: u32,
    /// Fill down each column first (cells 1, 2, 3 go down column one).
    pub down: bool,
    pub keep_first: bool,
}

/// Several columns' cells stacked into one column, row by row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stack {
    pub cols: Vec<u32>,
    pub skip_blanks: bool,
    pub keep_first: bool,
}

// ---- sample ----

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SampleSize {
    /// This many rows at random.
    Rows { n: u64 },
    /// This percent of the rows at random.
    Percent { p: f64 },
    /// Every nth row, starting with the first.
    Every { n: u64 },
}

impl Default for SampleSize {
    fn default() -> Self {
        SampleSize::Rows { n: 100 }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sample {
    pub size: SampleSize,
    /// The same seed picks the same rows of the same table.
    pub seed: u64,
    pub keep_first: bool,
    /// Draw from each group of this column's values on its own ("10 per
    /// precinct"): rows or percent per group.
    pub per: Option<u32>,
    /// Rows in the order drawn (group by group) instead of table order.
    pub drawn_order: bool,
    /// A first column "Row" with each row's number in the table.
    pub row_numbers: bool,
}

/// What a sample drew, for the audit record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SampleReport {
    pub seed: u64,
    /// The method, in words (with how to draw it again).
    pub method: String,
    /// Rows drawn from, and rows taken.
    pub rows: u64,
    pub taken: u64,
    /// Per group: its value, rows in it, rows taken.
    pub groups: Vec<(String, u64, u64)>,
}

/// The SHA-256 of a file, in hex (read in pieces; `progress` can stop it).
pub fn file_sha256(path: &std::path::Path, progress: &Progress) -> Result<String, Error> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        progress.step(0)?;
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The audit record of a sample as a table: what was drawn and how, then
/// the rows in and taken per group.
pub fn sample_record(
    r: &SampleReport,
    source: &str,
    column: Option<&str>,
    fingerprint: &str,
) -> Vec<u8> {
    let mut out = Vec::new();
    let line = |out: &mut Vec<u8>, cells: &[String]| crate::reshape::push_line(out, cells);
    line(&mut out, &["What".into(), "Value".into(), "Rows".into()]);
    line(
        &mut out,
        &["Drawn from".into(), source.into(), r.rows.to_string()],
    );
    line(
        &mut out,
        &["File SHA-256".into(), fingerprint.into(), String::new()],
    );
    line(
        &mut out,
        &["Seed".into(), r.seed.to_string(), String::new()],
    );
    if let Some(c) = column {
        line(&mut out, &["Per group of".into(), c.into(), String::new()]);
    }
    line(
        &mut out,
        &["Method".into(), r.method.clone(), String::new()],
    );
    line(
        &mut out,
        &["Rows taken".into(), String::new(), r.taken.to_string()],
    );
    for (v, n, k) in &r.groups {
        line(
            &mut out,
            &[
                "Group".into(),
                if v.is_empty() {
                    "(blank)".into()
                } else {
                    v.clone()
                },
                format!("{k} of {n}"),
            ],
        );
    }
    out
}

/// SplitMix64: the next number and state.
fn split_mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a over a group's value (its seed is the sample's seed XOR this).
fn fnv64(v: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in v.as_bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// `k` of `items` at random, in the order drawn: a partial Fisher-Yates
/// shuffle driven by SplitMix64 from `seed`, each pick `i + floor(z * (n -
/// i) / 2^64)`.
fn draw(items: &mut [u32], k: usize, seed: u64) -> &[u32] {
    let n = items.len();
    let k = k.min(n);
    let mut state = seed;
    for i in 0..k {
        let z = split_mix(&mut state);
        let span = (n - i) as u64;
        let j = i + ((z as u128 * span as u128) >> 64) as usize;
        items.swap(i, j);
    }
    &items[..k]
}

// ---- compare ----

/// How two tables are compared.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompareSpec {
    /// Columns that identify a row (the same column numbers in both
    /// tables); empty: the whole row, so rows are only added or removed.
    pub keys: Vec<u32>,
    pub match_case: bool,
    pub trim: bool,
    /// Both tables start with a title row.
    pub keep_first: bool,
    /// Rows that are the same are listed too.
    pub show_same: bool,
}

/// Counts of a comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompareReport {
    pub same: u64,
    pub changed: u64,
    /// In the second table only.
    pub added: u64,
    /// In the first table only.
    pub removed: u64,
    /// Changed cells.
    pub cells: u64,
}

/// The second table of a comparison, reduced to fingerprints: per shown row
/// a 64-bit hash of its key and of the whole row (random per comparison),
/// sorted by key. Built under the second table's own lock; its rows are
/// read again only for the rows that differ ([`Document::compare_finish`]).
pub struct CompareIndex {
    hasher: RandomState,
    /// (key hash, row hash, position among the second table's data rows).
    rows: Vec<(u64, u64, u32)>,
    cols: u32,
    titles: Option<Vec<String>>,
    /// The second table's revision: it must not change before the finish.
    revision: u64,
    first: u64,
}

/// What [`Document::compare_rows`] found in the first table.
pub struct ComparePlan {
    /// Rows of the first table only: row number and cells.
    removed: Vec<(u64, Vec<String>)>,
    /// Same key, other cells differ (or a hash match to check): first
    /// table's row number and cells, second table's position.
    changed: Vec<(u64, Vec<String>, u32)>,
    /// Same rows, listed with `show_same`: row numbers in both.
    same: Vec<(u64, Vec<String>, u32)>,
    same_count: u64,
    /// Positions in the second table no row matched.
    added: Vec<u32>,
    cols: u32,
    titles: Option<Vec<String>>,
}

/// Most differences a comparison lists (rows of the result).
pub const MAX_COMPARE_ROWS: usize = 20_000_000;

fn hash_cells<'a>(
    hasher: &RandomState,
    cells: impl Iterator<Item = &'a str>,
    match_case: bool,
    trim: bool,
) -> u64 {
    // Empty cells at the end are left out, so a row reads the same however
    // many empty columns follow it (the two tables may differ in width).
    let mut h = hasher.build_hasher();
    let mut empties = 0usize;
    for v in cells {
        let v = norm(v, match_case, trim);
        if v.is_empty() {
            empties += 1;
            continue;
        }
        for _ in 0..empties {
            h.write_usize(0);
        }
        empties = 0;
        h.write_usize(v.len());
        h.write(v.as_bytes());
    }
    h.finish()
}

/// The key columns (all `cols` columns when none are chosen). With
/// `check`, a key column at or past `cols` is refused.
fn key_cols(spec: &CompareSpec, cols: u32, check: bool) -> Result<Vec<u32>, Error> {
    if spec.keys.len() > MAX_TOOL_COLS {
        return Err(bad(format!("at most {MAX_TOOL_COLS} key columns")));
    }
    if check && spec.keys.iter().any(|&k| k >= cols) {
        return Err(bad("a key column is past the last column of both files"));
    }
    Ok(if spec.keys.is_empty() {
        (0..cols).collect()
    } else {
        spec.keys.clone()
    })
}

impl CompareIndex {
    /// Fingerprint the second table's shown rows.
    pub fn build(d: &Document, spec: &CompareSpec, progress: &Progress) -> Result<Self, Error> {
        d.ready_for_bulk()?;
        let cols = d.col_count();
        // Key columns are checked against both widths when the first table is read.
        let keys = key_cols(spec, cols, false)?;
        let all: Vec<u32> = (0..cols).collect();
        let reader = Cols::new(d, &all);
        let rows = d.data_rows(spec.keep_first);
        if rows.end - rows.start > u32::MAX as u64 {
            return Err(Error::TooManyRows);
        }
        let hasher = RandomState::new();
        let mut out: Vec<(u64, u64, u32)> = d.par_rows(rows.clone(), progress, |r, id, buf| {
            reader.read(d, id, buf);
            let k = hash_cells(
                &hasher,
                keys.iter().map(|&c| {
                    if c < cols {
                        reader.get(buf, c as usize)
                    } else {
                        ""
                    }
                }),
                spec.match_case,
                spec.trim,
            );
            let w = hash_cells(
                &hasher,
                (0..all.len()).map(|c| reader.get(buf, c)),
                true,
                spec.trim,
            );
            Ok(Some((k, w, (r - rows.start) as u32)))
        })?;
        out.par_sort_unstable();
        Ok(CompareIndex {
            hasher,
            rows: out,
            cols,
            titles: (spec.keep_first && d.row_count() > 0)
                .then(|| d.row_values(0))
                .transpose()?,
            revision: d.revision(),
            first: rows.start,
        })
    }
}

fn norm(v: &str, match_case: bool, trim: bool) -> std::borrow::Cow<'_, str> {
    let v = if trim { v.trim() } else { v };
    if match_case {
        std::borrow::Cow::Borrowed(v)
    } else {
        std::borrow::Cow::Owned(v.to_lowercase())
    }
}

// ---- split into parts ----

/// How a table is split into parts (files).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitParts {
    /// One part per value of this column.
    pub col: Option<u32>,
    /// Or: a part every this many rows.
    pub every: u64,
    /// Each part starts with the title row.
    pub keep_first: bool,
    /// Give cells a spreadsheet could run as formulas an apostrophe
    /// ([`crate::scan::guard_formula`]). Off unless asked: parts are data.
    pub guard: bool,
}

/// One part: its name (a column value, or "1", "2", ...) and its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub ids: Vec<u32>,
}

impl Document {
    // ---- fill blanks ----

    /// Plan filling empty cells of shown rows from the cell above or below,
    /// or with one text.
    pub fn plan_fill_blanks(
        &self,
        spec: &FillBlanks,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        check_tool_cols(self, &spec.cols)?;
        if let FillFrom::Value { text } = &spec.from {
            if text.len() > MAX_CELL_BYTES {
                return Err(bad("the text is too long"));
            }
            if text.is_empty() {
                return Err(bad("type the text to put in empty cells"));
            }
        }
        let reader = Cols::new(self, &spec.cols);
        let rows = self.data_rows(spec.keep_first);
        let n = rows.end - rows.start;
        let below = spec.from == FillFrom::Below;
        let value = match &spec.from {
            FillFrom::Value { text } => Some(text.as_str()),
            _ => None,
        };
        let spent = AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        progress.start(n);
        let chunks: Vec<Vec<FillChunk>> = (0..(n as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<Vec<FillChunk>, Error> {
                let s = rows.start + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(rows.end);
                let mut out: Vec<FillChunk> = spec.cols.iter().map(|_| FillChunk::default()).collect();
                let mut buf = Vec::new();
                // Indexes of blanks waiting for the next filled value (below).
                let mut waiting: Vec<Vec<usize>> = vec![Vec::new(); spec.cols.len()];
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for (k, ch) in out.iter_mut().enumerate() {
                        let v = reader.get(&buf, k);
                        if blank(v, spec.spaces) {
                            let found = match value {
                                Some(t) => Some(t.to_string()),
                                None if below => {
                                    waiting[k].push(ch.blanks.len());
                                    None
                                }
                                None => ch.last.clone(),
                            };
                            if let Some(f) = &found {
                                if !crate::find::charge_within(&spent, f.len(), budget) {
                                    return Err(Error::OutOfMemory(
                                        "this would fill too many cells at once; fill fewer rows",
                                    ));
                                }
                            }
                            ch.blanks.push((id, found));
                        } else {
                            if ch.first.is_none() {
                                ch.first = Some(v.to_string());
                            }
                            if below {
                                for i in waiting[k].drain(..) {
                                    if !crate::find::charge_within(&spent, v.len(), budget) {
                                        return Err(Error::OutOfMemory(
                                            "this would fill too many cells at once; fill fewer rows",
                                        ));
                                    }
                                    ch.blanks[i].1 = Some(v.to_string());
                                }
                            }
                            ch.last = Some(v.to_string());
                        }
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        // Carry values across chunks: forward for "above", backward for "below".
        let mut by_row: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
        let mut cells = 0u64;
        for (k, &col) in spec.cols.iter().enumerate() {
            let mut carry: Option<String> = None;
            let order: Box<dyn Iterator<Item = &Vec<FillChunk>>> = if below {
                Box::new(chunks.iter().rev())
            } else {
                Box::new(chunks.iter())
            };
            for ch in order {
                let ch = &ch[k];
                for (id, found) in &ch.blanks {
                    if let Some(t) = found.as_ref().or(carry.as_ref()) {
                        by_row.entry(*id).or_default().push((col, t.clone()));
                        cells += 1;
                    }
                }
                let edge = if below { &ch.first } else { &ch.last };
                if edge.is_some() {
                    carry = edge.clone();
                }
            }
        }
        let mut plan = RowPlan::new(self);
        plan.rows = by_row.len() as u64;
        plan.groups = cells;
        plan.changes = by_row.into_iter().collect();
        Ok(plan)
    }

    // ---- repeats ----

    /// Plan a new column, after the last key column, holding how many shown
    /// rows share each row's key.
    pub fn plan_count_repeats(
        &self,
        spec: &Repeats,
        progress: &Progress,
    ) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        check_tool_cols(self, &spec.cols)?;
        let rows = self.data_rows(spec.keep_first);
        let leader = self.group_rows(
            rows.clone(),
            &spec.cols,
            spec.match_case,
            spec.trim,
            progress,
        )?;
        let mut count = vec![0u32; leader.len()];
        for &l in &leader {
            count[l as usize] += 1;
        }
        let at = spec.cols.iter().max().copied().unwrap_or(0) + 1;
        // Every row gets a new cell, so the plan is charged like any other
        // bulk change and refused before it can outgrow memory.
        let spent = AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        let too_many =
            || Error::OutOfMemory("this would change too many cells at once; use fewer rows");
        let mut plan = ColPlan::new(self);
        plan.insert = Some((at, 1));
        plan.cols = 1;
        if rows.start == 1 {
            let title = if spec.title.is_empty() {
                "Count"
            } else {
                &spec.title
            };
            plan.changes.push((
                self.id_at(0).expect("a first row"),
                vec![(at, title.to_string())],
            ));
        }
        for (i, &l) in leader.iter().enumerate() {
            let text = count[l as usize].to_string();
            if !crate::find::charge_within(&spent, text.len() + CELL_COST, budget) {
                return Err(too_many());
            }
            let id = self.id_at(rows.start + i as u64).ok_or(Error::OutOfRange)?;
            plan.changes.push((id, vec![(at, text)]));
        }
        plan.cells = plan.changes.len() as u64;
        Ok(plan)
    }

    /// Plan emptying the key cells of every shown row that repeats an
    /// earlier row's key (the first of each stays).
    pub fn plan_blank_repeats(
        &self,
        spec: &Duplicates,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        check_tool_cols(self, &spec.cols)?;
        let rows = self.data_rows(spec.keep_first);
        let leader = self.group_rows(
            rows.clone(),
            &spec.cols,
            spec.match_case,
            spec.trim,
            progress,
        )?;
        let spent = AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        let mut plan = RowPlan::new(self);
        let mut counted = vec![false; leader.len()];
        for (i, &l) in leader.iter().enumerate() {
            if l as usize == i {
                continue;
            }
            if !counted[l as usize] {
                counted[l as usize] = true;
                plan.groups += 1;
            }
            // Each emptied cell costs its place in the change list.
            if !crate::find::charge_within(&spent, spec.cols.len() * CELL_COST, budget) {
                return Err(Error::OutOfMemory(
                    "this would change too many cells at once; use fewer rows or columns",
                ));
            }
            let id = self.id_at(rows.start + i as u64).ok_or(Error::OutOfRange)?;
            plan.changes
                .push((id, spec.cols.iter().map(|&c| (c, String::new())).collect()));
        }
        plan.rows = plan.changes.len() as u64;
        Ok(plan)
    }

    // ---- breaks ----

    /// Plan blank rows after each group of shown rows (where `col` changes),
    /// or after every `every` rows. Never after the last row.
    pub fn plan_breaks(&self, spec: &Breaks, progress: &Progress) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        if spec.rows == 0 || spec.rows > MAX_BREAK_ROWS {
            return Err(bad(format!(
                "put between 1 and {MAX_BREAK_ROWS} blank rows"
            )));
        }
        let rows = self.data_rows(spec.keep_first);
        let width = self.col_count().max(1) as usize;
        let blank_rows = || vec![vec![String::new(); width]; spec.rows as usize];
        let after: Vec<u32> = match spec.col {
            Some(col) => {
                check_cols(self, &[col])?;
                let reader = Cols::new(self, &[col]);
                self.par_rows(rows.clone(), progress, |r, id, buf| {
                    if r + 1 >= rows.end {
                        return Ok(None);
                    }
                    reader.read(self, id, buf);
                    let a = reader.get(buf, 0).to_string();
                    let next = self.id_at(r + 1).ok_or(Error::OutOfRange)?;
                    reader.read(self, next, buf);
                    Ok((reader.get(buf, 0) != a).then_some(id))
                })?
            }
            None => {
                if spec.every == 0 {
                    return Err(bad("choose how many rows between breaks"));
                }
                let mut v = Vec::new();
                let mut r = rows.start + spec.every - 1;
                while r + 1 < rows.end {
                    v.push(self.id_at(r).ok_or(Error::OutOfRange)?);
                    r += spec.every;
                }
                v
            }
        };
        let added = after.len() as u64 * spec.rows as u64;
        if added > crate::reshape::MAX_NEW_ROWS {
            return Err(bad("this would add too many rows"));
        }
        let mut plan = RowPlan::new(self);
        plan.groups = after.len() as u64;
        plan.rows = added;
        plan.after = after.into_iter().map(|id| (id, blank_rows())).collect();
        Ok(plan)
    }

    // ---- hidden rows ----

    /// Plan deleting every row the user hid (rows a filter leaves out stay).
    pub fn plan_delete_hidden(&self) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        let mut plan = RowPlan::new(self);
        if let Some(bits) = self.hidden_bits() {
            let hidden = |id: u32| {
                bits.get((id >> 6) as usize)
                    .is_some_and(|w| (w >> (id & 63)) & 1 == 1)
            };
            plan.remove = self.ids().filter(|&id| hidden(id)).collect();
        }
        plan.rows = plan.remove.len() as u64;
        Ok(plan)
    }

    /// Plan deleting every row not shown now (left out by a filter, or
    /// hidden), e.g. to keep only a filter's rows.
    pub fn plan_keep_shown(&self) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        let mut plan = RowPlan::new(self);
        let base = self.next_row_id() as usize;
        let mut shown = vec![0u64; base.div_ceil(64)];
        for r in 0..self.row_count() {
            if let Some(id) = self.id_at(r) {
                shown[(id >> 6) as usize] |= 1 << (id & 63);
            }
        }
        plan.remove = self
            .ids()
            .filter(|&id| (shown[(id >> 6) as usize] >> (id & 63)) & 1 == 0)
            .collect();
        plan.rows = plan.remove.len() as u64;
        Ok(plan)
    }

    /// Plan deleting these columns (any order) as one undo step. At least one
    /// column always stays.
    pub fn plan_delete_cols(&self, cols: &[u32]) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        check_tool_cols(self, cols)?;
        let mut cols = cols.to_vec();
        cols.sort_unstable();
        cols.dedup();
        if cols.len() >= self.col_count() as usize {
            return Err(bad("at least one column must stay"));
        }
        let mut plan = ColPlan::new(self);
        plan.remove_cols(cols);
        Ok(plan)
    }

    // ---- reshape ----

    /// A new table: the column's shown cells, `per_row` to a row.
    pub fn reshape_column(&self, spec: &Reshape, progress: &Progress) -> Result<Table, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[spec.col])?;
        if spec.per_row == 0 || spec.per_row > MAX_PER_ROW {
            return Err(bad(format!(
                "put between 1 and {MAX_PER_ROW} cells on a row"
            )));
        }
        let reader = Cols::new(self, &[spec.col]);
        let rows = self.data_rows(spec.keep_first);
        let vals: Vec<String> = self.par_rows(rows, progress, |_, id, buf| {
            reader.read(self, id, buf);
            Ok(Some(reader.get(buf, 0).to_string()))
        })?;
        let per = spec.per_row as usize;
        let lines = vals.len().div_ceil(per);
        let mut out = Vec::new();
        let mut line: Vec<&str> = Vec::with_capacity(per);
        for r in 0..lines {
            line.clear();
            for k in 0..per {
                let i = if spec.down {
                    k * lines + r
                } else {
                    r * per + k
                };
                line.push(vals.get(i).map_or("", String::as_str));
            }
            push_line(&mut out, &line);
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
        }
        Ok(Table {
            bytes: out,
            rows: lines as u64,
            cols: spec.per_row,
        })
    }

    /// A new table of one column: the chosen columns' shown cells, row by
    /// row, left to right.
    pub fn stack_columns(&self, spec: &Stack, progress: &Progress) -> Result<Table, Error> {
        self.ready_for_bulk()?;
        check_tool_cols(self, &spec.cols)?;
        let reader = Cols::new(self, &spec.cols);
        let rows = self.data_rows(spec.keep_first);
        let n = (rows.end - rows.start) * spec.cols.len() as u64;
        if n > MAX_TABLE_CELLS {
            return Err(too_big());
        }
        let parts: Vec<Vec<u8>> = self.par_rows(rows, progress, |_, id, buf| {
            reader.read(self, id, buf);
            let mut out = Vec::new();
            for k in 0..spec.cols.len() {
                let v = reader.get(buf, k);
                if spec.skip_blanks && v.trim().is_empty() {
                    continue;
                }
                push_line(&mut out, &[v]);
            }
            Ok(Some(out))
        })?;
        let total: usize = parts.iter().map(Vec::len).sum();
        if total > MAX_TABLE_BYTES {
            return Err(too_big());
        }
        let mut bytes = Vec::with_capacity(total);
        let mut lines = 0u64;
        for p in parts {
            lines += p.iter().filter(|&&b| b == b'\n').count() as u64;
            bytes.extend_from_slice(&p);
        }
        Ok(Table {
            bytes,
            rows: lines,
            cols: 1,
        })
    }

    /// The shown positions a sample takes (in table order, or as drawn),
    /// and what it drew.
    fn sample_positions(
        &self,
        spec: &Sample,
    ) -> Result<(std::ops::Range<u64>, Vec<u64>, SampleReport), Error> {
        let rows = self.data_rows(spec.keep_first);
        let n = rows.end - rows.start;
        if n > u32::MAX as u64 {
            return Err(Error::TooManyRows);
        }
        let how_many = |size: u64| -> Result<usize, Error> {
            Ok(match spec.size {
                SampleSize::Rows { n: k } => k.min(size) as usize,
                SampleSize::Percent { p } => {
                    if !p.is_finite() || !(0.0..=100.0).contains(&p) {
                        return Err(bad("choose a percent from 0 to 100"));
                    }
                    ((size as f64 * p / 100.0).round() as u64).min(size) as usize
                }
                SampleSize::Every { .. } => 0,
            })
        };
        let size_words = match spec.size {
            SampleSize::Rows { n: k } => format!("{k} rows"),
            SampleSize::Percent { p } => format!("{p}% of the rows (rounded)"),
            SampleSize::Every { n: k } => format!("every {k}th row"),
        };
        let mut report = SampleReport {
            seed: spec.seed,
            rows: n,
            ..Default::default()
        };
        let mut pos: Vec<u64> = match (spec.size, spec.per) {
            (SampleSize::Every { n: k }, None) => {
                if k == 0 {
                    return Err(bad("choose how often to take a row"));
                }
                report.method = format!("Every {k}th shown row, starting with the first.");
                (rows.start..rows.end)
                    .step_by(k.min(u64::from(u32::MAX)) as usize)
                    .collect()
            }
            (SampleSize::Every { .. }, Some(_)) => {
                return Err(bad(
                    "a sample per group takes a number or a percent of rows",
                ))
            }
            (_, None) => {
                let mut idx: Vec<u32> = (0..n as u32).collect();
                let k = how_many(n)?;
                report.method = format!(
                    "{size_words} at random from the {n} shown rows, seed {}. Rows are numbered 0 up in table order; a partial Fisher-Yates shuffle driven by SplitMix64 from the seed picks row i + floor(z x (n - i) / 2^64) for each draw i.",
                    spec.seed
                );
                report.groups.push(("(all rows)".into(), n, k as u64));
                draw(&mut idx, k, spec.seed)
                    .iter()
                    .map(|&p| rows.start + u64::from(p))
                    .collect()
            }
            (_, Some(col)) => {
                if col >= self.col_count() {
                    return Err(Error::OutOfRange);
                }
                // The rows of each value, in table order; groups in the
                // order first seen.
                // Each part groups its rows (in parallel); the parts are
                // then joined in order, so groups keep table order.
                let reader = Cols::new(self, &[col]);
                const PART: u64 = 1 << 16;
                let parts = (rows.end - rows.start).div_ceil(PART);
                type Groups = Vec<(String, Vec<u32>)>;
                let found: Vec<Groups> = (0..parts)
                    .into_par_iter()
                    .map(|c| -> Result<Groups, Error> {
                        let s = rows.start + c * PART;
                        let e = (s + PART).min(rows.end);
                        let mut buf = Vec::new();
                        let mut index: HashMap<String, usize> = HashMap::new();
                        let mut groups: Groups = Vec::new();
                        for r in s..e {
                            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                            reader.read(self, id, &mut buf);
                            let v = reader.get(&buf, 0);
                            let g = match index.get(v) {
                                Some(&g) => g,
                                None => {
                                    if groups.len() >= 1_000_000 {
                                        return Err(bad("more than 1,000,000 groups"));
                                    }
                                    index.insert(v.to_string(), groups.len());
                                    groups.push((v.to_string(), Vec::new()));
                                    groups.len() - 1
                                }
                            };
                            groups[g].1.push((r - rows.start) as u32);
                        }
                        Ok(groups)
                    })
                    .collect::<Result<_, _>>()?;
                let mut index: HashMap<String, usize> = HashMap::new();
                let mut groups: Groups = Vec::new();
                for part in found {
                    for (v, members) in part {
                        match index.get(&v) {
                            Some(&g) => groups[g].1.extend(members),
                            None => {
                                if groups.len() >= 1_000_000 {
                                    return Err(bad("more than 1,000,000 groups"));
                                }
                                index.insert(v.clone(), groups.len());
                                groups.push((v, members));
                            }
                        }
                    }
                }
                report.method = format!(
                    "{size_words} at random from each group of the column's values, seed {}. Each group's rows are numbered 0 up in table order and drawn on their own with the seed XOR the FNV-1a 64-bit hash of the group's value: a partial Fisher-Yates shuffle driven by SplitMix64 picks row i + floor(z x (n - i) / 2^64) for each draw i.",
                    spec.seed
                );
                let mut out = Vec::new();
                for (v, mut members) in groups {
                    let size = members.len() as u64;
                    let k = how_many(size)?;
                    let picked = draw(&mut members, k, spec.seed ^ fnv64(&v));
                    out.extend(picked.iter().map(|&p| rows.start + u64::from(p)));
                    report.groups.push((v, size, k as u64));
                }
                out
            }
        };
        if !spec.drawn_order {
            pos.sort_unstable();
        }
        report.taken = pos.len() as u64;
        Ok((rows, pos, report))
    }

    /// A new table of a sample of shown rows (all columns), with the title
    /// row first when kept.
    pub fn sample(&self, spec: &Sample, progress: &Progress) -> Result<Table, Error> {
        Ok(self.sample_report(spec, progress)?.0)
    }

    /// As [`Document::sample`], with what it drew (for an audit record).
    pub fn sample_report(
        &self,
        spec: &Sample,
        progress: &Progress,
    ) -> Result<(Table, SampleReport), Error> {
        self.ready_for_bulk()?;
        let (rows, pos, report) = self.sample_positions(spec)?;
        let cols = self.col_count();
        let numbers = spec.row_numbers;
        if (pos.len() as u64 + 1) * cols as u64 > MAX_TABLE_CELLS {
            return Err(too_big());
        }
        let all: Vec<u32> = (0..cols).collect();
        let reader = Cols::new(self, &all);
        let mut out = Vec::new();
        let mut buf = Vec::new();
        let mut line: Vec<String> = Vec::with_capacity(cols as usize);
        let mut write = |id: u32, row: Option<u64>, out: &mut Vec<u8>| -> Result<(), Error> {
            reader.read(self, id, &mut buf);
            line.clear();
            if numbers {
                line.push(row.map_or_else(|| "Row".to_string(), |r| (r + 1).to_string()));
            }
            line.extend((0..all.len()).map(|k| reader.get(&buf, k).to_string()));
            push_line(out, &line);
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
            Ok(())
        };
        if rows.start == 1 {
            write(self.id_at(0).expect("a first row"), None, &mut out)?;
        }
        progress.start(pos.len() as u64);
        for (i, &p) in pos.iter().enumerate() {
            write(self.id_at(p).ok_or(Error::OutOfRange)?, Some(p), &mut out)?;
            if i % 65_536 == 65_535 {
                progress.step(65_536)?;
            }
        }
        Ok((
            Table {
                bytes: out,
                rows: pos.len() as u64 + rows.start,
                cols: cols + u32::from(numbers),
            },
            report,
        ))
    }

    // ---- jumps ----

    /// The next shown row after `row` (or before it, going up) whose cell in
    /// `col` differs from the cell in `row`.
    pub fn find_change(&self, row: u64, col: u32, up: bool) -> Result<Option<u64>, Error> {
        self.ready_for_bulk()?;
        let n = self.row_count();
        if row >= n {
            return Err(Error::OutOfRange);
        }
        if col >= self.col_count() {
            return Ok(None);
        }
        let reader = Cols::new(self, &[col]);
        let mut buf = Vec::new();
        let id = self.id_at(row).ok_or(Error::OutOfRange)?;
        reader.read(self, id, &mut buf);
        let here = reader.get(&buf, 0).to_string();
        // Look in blocks, in parallel within each block.
        const BLOCK: u64 = 1 << 16;
        let mut start = row;
        loop {
            let (s, e) = if up {
                if start == 0 {
                    return Ok(None);
                }
                (start.saturating_sub(BLOCK), start)
            } else {
                if start + 1 >= n {
                    return Ok(None);
                }
                (start + 1, (start + 1 + BLOCK).min(n))
            };
            let found: Vec<u64> = (s..e)
                .into_par_iter()
                .filter_map(|r| {
                    let id = self.id_at(r)?;
                    let mut b = Vec::new();
                    reader.read(self, id, &mut b);
                    (reader.get(&b, 0) != here).then_some(r)
                })
                .collect();
            let pick = if up {
                found.iter().max()
            } else {
                found.iter().min()
            };
            if let Some(&r) = pick {
                return Ok(Some(r));
            }
            start = if up { s } else { e - 1 };
        }
    }

    /// The shown row holding the column's largest (or smallest) number;
    /// the first such row on ties. `None` when no cell is a number.
    pub fn find_extreme(
        &self,
        col: u32,
        largest: bool,
        keep_first: bool,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Option<u64>, Error> {
        self.ready_for_bulk()?;
        if col >= self.col_count() {
            return Ok(None);
        }
        let reader = Cols::new(self, &[col]);
        let rd = self.read_ctx(read).reader(col);
        let rows = self.data_rows(keep_first);
        let n = rows.end - rows.start;
        progress.start(n);
        let best = (0..(n as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<Option<(f64, u64)>, Error> {
                let s = rows.start + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(rows.end);
                let mut best: Option<(f64, u64)> = None;
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    if let Some(v) = rd.number(reader.get(&buf, 0)) {
                        let better = match best {
                            None => true,
                            Some((b, _)) => (largest && v > b) || (!largest && v < b),
                        };
                        if better {
                            best = Some((v, r));
                        }
                    }
                }
                progress.step(e - s)?;
                Ok(best)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .fold(None::<(f64, u64)>, |acc, (v, r)| match acc {
                None => Some((v, r)),
                Some((b, br)) => {
                    let better = (largest && v > b) || (!largest && v < b) || (v == b && r < br);
                    Some(if better { (v, r) } else { (b, br) })
                }
            });
        Ok(best.map(|(_, r)| r))
    }

    /// Distinct values of a column for type-ahead: from the first
    /// `max_rows` shown rows (title row left out), at most `max_values`
    /// values of at most [`TYPEAHEAD_CHARS`] characters and
    /// [`TYPEAHEAD_BYTES`] of text in all, sorted by their lower-case form.
    /// Each entry is (lower case, as written). The caps are shared by all
    /// chunks, so collecting stops once they are reached anywhere (a very
    /// varied column then gives a partial list; which spelling of a value
    /// wins may then differ, which only changes a suggestion).
    pub fn typeahead_values(
        &self,
        col: u32,
        keep_first: bool,
        max_rows: u64,
        max_values: usize,
    ) -> Result<Vec<(String, String)>, Error> {
        self.ready_for_bulk()?;
        if col >= self.col_count() {
            return Ok(Vec::new());
        }
        let reader = Cols::new(self, &[col]);
        let rows = self.data_rows(keep_first);
        let end = rows.end.min(rows.start.saturating_add(max_rows));
        // Entries and bytes kept by all chunks together.
        let kept = AtomicUsize::new(0);
        let bytes = AtomicUsize::new(0);
        let full = || {
            kept.load(Ordering::Relaxed) >= max_values
                || bytes.load(Ordering::Relaxed) >= TYPEAHEAD_BYTES
        };
        // Written form kept only when it differs from the lower-case key.
        type Seen = HashMap<String, (u64, Option<String>)>;
        let parts: Vec<Seen> = (0..((end - rows.start) as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| {
                let s = rows.start + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(end);
                let mut m: Seen = HashMap::new();
                let mut buf = Vec::new();
                for r in s..e {
                    if full() {
                        break;
                    }
                    let Some(id) = self.id_at(r) else { break };
                    reader.read(self, id, &mut buf);
                    let v = reader.get(&buf, 0).trim();
                    if v.is_empty()
                        || v.len() > TYPEAHEAD_CHARS * 4
                        || v.chars().count() > TYPEAHEAD_CHARS
                    {
                        continue;
                    }
                    let low = v.to_lowercase();
                    if let std::collections::hash_map::Entry::Vacant(slot) = m.entry(low) {
                        let written = (slot.key() != v).then(|| v.to_string());
                        let cost = slot.key().len() + written.as_ref().map_or(0, |w| w.len()) + 64;
                        bytes.fetch_add(cost, Ordering::Relaxed);
                        kept.fetch_add(1, Ordering::Relaxed);
                        slot.insert((r, written));
                    }
                }
                m
            })
            .collect();
        let mut all: Seen = HashMap::new();
        for p in parts {
            for (k, (r, v)) in p {
                match all.get(&k) {
                    Some((r0, _)) if *r0 <= r => {}
                    _ => {
                        if all.len() < max_values || all.contains_key(&k) {
                            all.insert(k, (r, v));
                        }
                    }
                }
            }
        }
        let mut out: Vec<(String, String)> = all
            .into_iter()
            .map(|(k, (_, v))| {
                let v = v.unwrap_or_else(|| k.clone());
                (k, v)
            })
            .collect();
        out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Smallest and largest number in a column's shown rows (colour scales
    /// and data bars), or `None` without numbers.
    pub fn number_range(
        &self,
        col: u32,
        keep_first: bool,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Option<(f64, f64)>, Error> {
        self.ready_for_bulk()?;
        if col >= self.col_count() {
            return Ok(None);
        }
        let reader = Cols::new(self, &[col]);
        let rd = self.read_ctx(read).reader(col);
        let rows = self.data_rows(keep_first);
        let n = rows.end - rows.start;
        progress.start(n);
        let r = (0..(n as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<Option<(f64, f64)>, Error> {
                let s = rows.start + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(rows.end);
                let mut best: Option<(f64, f64)> = None;
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    if let Some(v) = rd.number(reader.get(&buf, 0)) {
                        best = Some(best.map_or((v, v), |(a, b)| (a.min(v), b.max(v))));
                    }
                }
                progress.step(e - s)?;
                Ok(best)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)));
        Ok(r)
    }

    // ---- compare ----

    /// Second step of a comparison, on the first table: pair each shown row
    /// with the first unpaired row of the second table with the same key
    /// fingerprint. Only rows that differ are kept (with their cells).
    pub fn compare_rows(
        &self,
        index: &CompareIndex,
        spec: &CompareSpec,
        progress: &Progress,
    ) -> Result<ComparePlan, Error> {
        self.ready_for_bulk()?;
        let cols = self.col_count();
        let width = cols.max(index.cols);
        let keys = key_cols(spec, width, true)?;
        let all: Vec<u32> = (0..cols).collect();
        let reader = Cols::new(self, &all);
        let rows = self.data_rows(spec.keep_first);
        // Fingerprints of our rows, in order.
        let ours: Vec<(u64, u64, u32, u64)> =
            self.par_rows(rows.clone(), progress, |r, id, buf| {
                reader.read(self, id, buf);
                let k = hash_cells(
                    &index.hasher,
                    keys.iter().map(|&c| {
                        if c < cols {
                            reader.get(buf, c as usize)
                        } else {
                            ""
                        }
                    }),
                    spec.match_case,
                    spec.trim,
                );
                // The whole row as wide as the second table, so trailing empty
                // columns on one side do not count as a change.
                let w = hash_cells(
                    &index.hasher,
                    (0..index.cols as usize).map(|c| {
                        if c < all.len() {
                            reader.get(buf, c)
                        } else {
                            ""
                        }
                    }),
                    true,
                    spec.trim,
                );
                let extra =
                    (index.cols as usize..all.len()).any(|c| !reader.get(buf, c).is_empty());
                Ok(Some((
                    k,
                    if extra { !w } else { w },
                    id,
                    self.row_number(r).unwrap_or(r) + 1,
                )))
            })?;
        let mut matched = vec![false; index.rows.len()];
        // Where the next unpaired row of each key run starts.
        let mut cursor: HashMap<usize, usize> = HashMap::new();
        let mut plan = ComparePlan {
            removed: Vec::new(),
            changed: Vec::new(),
            same: Vec::new(),
            same_count: 0,
            added: Vec::new(),
            cols: width,
            titles: (spec.keep_first && self.row_count() > 0)
                .then(|| self.row_values(0))
                .transpose()?,
        };
        // Cells kept for the result are counted: past the result table's own
        // size limit the comparison stops early instead of filling memory.
        let mut kept = 0usize;
        let mut cells_of = |id: u32| -> Result<Vec<String>, Error> {
            let mut buf = Vec::new();
            reader.read(self, id, &mut buf);
            let v: Vec<String> = (0..all.len())
                .map(|c| reader.get(&buf, c).to_string())
                .collect();
            kept += v.iter().map(|x| x.len() + 24).sum::<usize>() + 64;
            if kept > MAX_TABLE_BYTES {
                return Err(Error::OutOfMemory(
                    "the two files differ in too many places to list; compare fewer rows, or choose key columns",
                ));
            }
            Ok(v)
        };
        let mut listed = 0usize;
        for &(k, w, id, number) in &ours {
            let lo = index.rows.partition_point(|x| x.0 < k);
            let start = *cursor.get(&lo).unwrap_or(&lo);
            let mut hit = None;
            let mut i = start;
            while i < index.rows.len() && index.rows[i].0 == k {
                if !matched[i] {
                    hit = Some(i);
                    break;
                }
                i += 1;
            }
            cursor.insert(lo, hit.map_or(i, |h| h + 1));
            match hit {
                None => {
                    plan.removed.push((number, cells_of(id)?));
                    listed += 1;
                }
                Some(i) => {
                    matched[i] = true;
                    let pos = index.rows[i].2;
                    if index.rows[i].1 == w {
                        plan.same_count += 1;
                        if spec.show_same {
                            plan.same.push((number, cells_of(id)?, pos));
                            listed += 1;
                        }
                    } else {
                        plan.changed.push((number, cells_of(id)?, pos));
                        listed += 1;
                    }
                }
            }
            if listed > MAX_COMPARE_ROWS {
                return Err(Error::OutOfMemory(
                    "the two files differ in too many rows to list",
                ));
            }
        }
        plan.added = index
            .rows
            .iter()
            .zip(&matched)
            .filter(|(_, m)| !**m)
            .map(|(r, _)| r.2)
            .collect();
        plan.added.sort_unstable();
        if listed + plan.added.len() > MAX_COMPARE_ROWS {
            return Err(Error::OutOfMemory(
                "the two files differ in too many rows to list",
            ));
        }
        Ok(plan)
    }

    /// Last step, on the second table (the one `index` was built from): read
    /// its rows that differ and write the result table: rows removed,
    /// changed ("old → new" in changed cells) and added.
    pub fn compare_finish(
        &self,
        index: &CompareIndex,
        plan: ComparePlan,
        spec: &CompareSpec,
        progress: &Progress,
    ) -> Result<(Table, CompareReport), Error> {
        if self.revision() != index.revision {
            return Err(Error::Changed);
        }
        let cols = plan.cols;
        let ours = self.col_count();
        let all: Vec<u32> = (0..ours).collect();
        let reader = Cols::new(self, &all);
        let keys = key_cols(spec, cols, true)?;
        let num = |pos: u32| {
            let p = index.first + pos as u64;
            self.row_number(p).unwrap_or(p) + 1
        };
        let theirs = |pos: u32| -> Result<Vec<String>, Error> {
            let id = self.id_at(index.first + pos as u64).ok_or(Error::Changed)?;
            let mut buf = Vec::new();
            reader.read(self, id, &mut buf);
            Ok((0..cols as usize)
                .map(|c| {
                    if c < all.len() {
                        reader.get(&buf, c).to_string()
                    } else {
                        String::new()
                    }
                })
                .collect())
        };
        let mut header: Vec<String> =
            vec!["Change".into(), "Row (first)".into(), "Row (second)".into()];
        for c in 0..cols as usize {
            let t = plan
                .titles
                .as_ref()
                .and_then(|t| t.get(c).filter(|s| !s.is_empty()).cloned())
                .or_else(|| {
                    index
                        .titles
                        .as_ref()
                        .and_then(|t| t.get(c).filter(|s| !s.is_empty()).cloned())
                })
                .unwrap_or_else(|| format!("Column {}", c + 1));
            header.push(t);
        }
        header.push("Changed columns".into());
        let mut out = Vec::new();
        push_line(&mut out, &header);
        let mut report = CompareReport {
            same: plan.same_count,
            ..Default::default()
        };
        let mut line: Vec<String> = Vec::with_capacity(header.len());
        let pad = |mut v: Vec<String>| {
            v.resize(cols as usize, String::new());
            v
        };
        let mut rows = 1u64;
        let mut push = |line: &[String], out: &mut Vec<u8>| -> Result<(), Error> {
            push_line(out, line);
            rows += 1;
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
            Ok(())
        };
        let differs = |a: &str, b: &str| {
            if spec.trim {
                a.trim() != b.trim()
            } else {
                a != b
            }
        };
        let mut extra_added: Vec<u32> = Vec::new();
        progress.start((plan.removed.len() + plan.changed.len() + plan.added.len()) as u64);
        for (number, cells) in plan.removed {
            line.clear();
            line.extend(["removed".to_string(), number.to_string(), String::new()]);
            line.extend(pad(cells));
            line.push(String::new());
            report.removed += 1;
            push(&line, &mut out)?;
        }
        for (number, cells, pos) in plan.changed {
            let a = pad(cells);
            let b = theirs(pos)?;
            // A fingerprint match whose keys really differ: two rows, one
            // removed and one added.
            let same_key = keys.iter().all(|&k| {
                norm(&a[k as usize], spec.match_case, spec.trim)
                    == norm(&b[k as usize], spec.match_case, spec.trim)
            });
            line.clear();
            if !same_key {
                line.extend(["removed".to_string(), number.to_string(), String::new()]);
                line.extend(a);
                line.push(String::new());
                report.removed += 1;
                push(&line, &mut out)?;
                extra_added.push(pos);
                continue;
            }
            let mut changed = Vec::new();
            line.extend([
                "changed".to_string(),
                number.to_string(),
                num(pos).to_string(),
            ]);
            for c in 0..cols as usize {
                if differs(&a[c], &b[c]) {
                    changed.push(header[3 + c].clone());
                    line.push(format!("{} → {}", a[c], b[c]));
                } else {
                    line.push(a[c].clone());
                }
            }
            if changed.is_empty() {
                // Only the fingerprints differed (spaces at the ends).
                report.same += 1;
                continue;
            }
            report.changed += 1;
            report.cells += changed.len() as u64;
            line.push(changed.join(", "));
            push(&line, &mut out)?;
            progress.step(1)?;
        }
        for (number, cells, pos) in plan.same {
            line.clear();
            line.extend(["same".to_string(), number.to_string(), num(pos).to_string()]);
            line.extend(pad(cells));
            line.push(String::new());
            push(&line, &mut out)?;
        }
        let mut added = plan.added;
        added.extend(extra_added);
        added.sort_unstable();
        for pos in added {
            let b = theirs(pos)?;
            line.clear();
            line.extend(["added".to_string(), String::new(), num(pos).to_string()]);
            line.extend(b);
            line.push(String::new());
            report.added += 1;
            push(&line, &mut out)?;
        }
        Ok((
            Table {
                bytes: out,
                rows,
                cols: cols + 4,
            },
            report,
        ))
    }

    /// Compare this table with `other` in one go (both readable here).
    pub fn compare(
        &self,
        other: &Document,
        spec: &CompareSpec,
        progress: &Progress,
    ) -> Result<(Table, CompareReport), Error> {
        let index = CompareIndex::build(other, spec, progress)?;
        let plan = self.compare_rows(&index, spec, progress)?;
        other.compare_finish(&index, plan, spec, progress)
    }

    // ---- split into parts ----

    /// The parts a split makes, in order of first appearance: shown rows
    /// grouped by a column's exact value, or every `every` rows.
    pub fn split_parts(&self, spec: &SplitParts, progress: &Progress) -> Result<Vec<Part>, Error> {
        self.ready_for_bulk()?;
        let rows = self.data_rows(spec.keep_first);
        let mut parts: Vec<Part> = Vec::new();
        match spec.col {
            Some(col) => {
                check_cols(self, &[col])?;
                let reader = Cols::new(self, &[col]);
                let vals: Vec<(u32, String)> = self.par_rows(rows, progress, |_, id, buf| {
                    reader.read(self, id, buf);
                    Ok(Some((id, reader.get(buf, 0).to_string())))
                })?;
                let mut at: HashMap<String, usize> = HashMap::new();
                for (id, v) in vals {
                    let i = match at.get(&v) {
                        Some(&i) => i,
                        None => {
                            if parts.len() >= MAX_PARTS {
                                return Err(bad(format!(
                                    "this column has more than {MAX_PARTS} different values"
                                )));
                            }
                            at.insert(v.clone(), parts.len());
                            parts.push(Part {
                                name: v,
                                ids: Vec::new(),
                            });
                            parts.len() - 1
                        }
                    };
                    parts[i].ids.push(id);
                }
            }
            None => {
                if spec.every == 0 {
                    return Err(bad("choose how many rows go in each part"));
                }
                let n = rows.end - rows.start;
                if n.div_ceil(spec.every) > MAX_PARTS as u64 {
                    return Err(bad(format!("this would make more than {MAX_PARTS} parts")));
                }
                let mut r = rows.start;
                while r < rows.end {
                    let e = (r + spec.every).min(rows.end);
                    parts.push(Part {
                        name: (parts.len() + 1).to_string(),
                        ids: (r..e).filter_map(|p| self.id_at(p)).collect(),
                    });
                    r = e;
                }
            }
        }
        Ok(parts)
    }

    /// Write rows `ids` (with the title row first when `header`) to `out` in
    /// the document's own separator, quotes, line break and encoding, quoting
    /// where needed (streamed in 1 MB pieces, so a part may be any size).
    /// With `guard`, formula-like cells get an apostrophe. Returns the bytes
    /// written and the cells guarded.
    pub fn write_part(
        &self,
        ids: &[u32],
        header: bool,
        guard: bool,
        out: &mut dyn std::io::Write,
    ) -> Result<(u64, u64), Error> {
        let f = self.format();
        let all: Vec<u32> = (0..self.col_count()).collect();
        let reader = Cols::new(self, &all);
        let brk = match f.line_break {
            crate::dialect::LineBreak::CrLf => "\r\n",
            crate::dialect::LineBreak::Lf => "\n",
            crate::dialect::LineBreak::Cr => "\r",
        };
        let sep = String::from_utf8_lossy(f.dialect.delimiter()).into_owned();
        let mut piece: Vec<u8> = Vec::with_capacity(1 << 20);
        let mut written = 0u64;
        let mut guarded = 0u64;
        if f.bom {
            piece.extend_from_slice(f.encoding.bom());
        }
        let mut text = String::new();
        let mut buf = Vec::new();
        let first = if header { self.id_at(0) } else { None };
        for id in first.into_iter().chain(ids.iter().copied()) {
            reader.read(self, id, &mut buf);
            text.clear();
            for k in 0..all.len() {
                if k > 0 {
                    text.push_str(&sep);
                }
                guarded += u64::from(crate::scan::write_field_guarded(
                    reader.get(&buf, k),
                    f.dialect,
                    guard,
                    &mut text,
                ));
            }
            text.push_str(brk);
            crate::encoding::encode(&text, f.encoding, &mut piece)
                .map_err(|_| bad("a cell holds a character the file's encoding cannot store"))?;
            if piece.len() >= 1 << 20 {
                out.write_all(&piece)?;
                written += piece.len() as u64;
                piece.clear();
            }
        }
        out.write_all(&piece)?;
        written += piece.len() as u64;
        Ok((written, guarded))
    }
}

/// How several tables are stacked into one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Combine {
    /// Each table starts with a title row (written once, at the top).
    pub header: bool,
    /// Columns are matched by title (case and spaces at the ends ignored)
    /// instead of by position.
    pub by_title: bool,
    /// A first column holding the name of the file each row came from.
    pub source: bool,
}

/// Most tables combined at once.
pub const MAX_COMBINE: usize = 500;

/// Stack the shown rows of `tables` (name, table) into a new table.
pub fn combine(
    tables: &[(String, &Document)],
    spec: &Combine,
    progress: &Progress,
) -> Result<Table, Error> {
    if tables.is_empty() || tables.len() > MAX_COMBINE {
        return Err(bad(format!("combine 1 to {MAX_COMBINE} files")));
    }
    let by_title = spec.by_title && spec.header;
    // Output columns, and where each table's columns go.
    let mut titles: Vec<String> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut maps: Vec<Vec<usize>> = Vec::with_capacity(tables.len());
    for (_, d) in tables {
        d.ready_for_bulk()?;
        let w = d.col_count() as usize;
        let row0 = if spec.header && d.row_count() > 0 {
            d.row_values(0)?
        } else {
            Vec::new()
        };
        let mut map = Vec::with_capacity(w);
        for c in 0..w {
            let t = row0.get(c).cloned().unwrap_or_default();
            if by_title {
                let key = t.trim().to_lowercase();
                let key = if key.is_empty() {
                    format!("\u{0}{c}")
                } else {
                    key
                };
                let at = *seen.entry(key).or_insert_with(|| {
                    titles.push(if t.trim().is_empty() {
                        format!("Column {}", c + 1)
                    } else {
                        t.clone()
                    });
                    titles.len() - 1
                });
                map.push(at);
            } else {
                if c >= titles.len() {
                    titles.push(if t.is_empty() {
                        format!("Column {}", c + 1)
                    } else {
                        t.clone()
                    });
                }
                map.push(c);
            }
        }
        maps.push(map);
    }
    if titles.len() > crate::reshape::MAX_PIVOT_COLS {
        return Err(too_big());
    }
    let width = titles.len() + usize::from(spec.source);
    let total: u64 = tables.iter().map(|(_, d)| d.row_count()).sum();
    if total.saturating_mul(width as u64) > MAX_TABLE_CELLS {
        return Err(too_big());
    }
    progress.start(total);
    let mut out = Vec::new();
    let mut line: Vec<String> = Vec::with_capacity(width);
    let mut rows = 0u64;
    if spec.header {
        if spec.source {
            line.push("Source file".into());
        }
        line.extend(titles.iter().cloned());
        push_line(&mut out, &line);
        rows += 1;
    }
    for ((name, d), map) in tables.iter().zip(&maps) {
        let all: Vec<u32> = (0..d.col_count()).collect();
        let reader = Cols::new(d, &all);
        let range = d.data_rows(spec.header);
        let mut buf = Vec::new();
        for r in range.clone() {
            let id = d.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(d, id, &mut buf);
            line.clear();
            line.resize(width, String::new());
            let off = usize::from(spec.source);
            if spec.source {
                line[0] = name.clone();
            }
            for (k, &to) in map.iter().enumerate() {
                line[off + to] = reader.get(&buf, k).to_string();
            }
            push_line(&mut out, &line);
            rows += 1;
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
            if r % 65_536 == 0 {
                progress.step(65_536.min(range.end - r))?;
            }
        }
    }
    Ok(Table {
        bytes: out,
        rows,
        cols: width as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(text: &str) -> Document {
        let mut d = Document::from_bytes(
            text.as_bytes().to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        while !d.index_step(1 << 20).unwrap() {}
        d
    }

    fn col(d: &Document, c: u32) -> Vec<String> {
        (0..d.row_count()).map(|r| d.cell(r, c).unwrap()).collect()
    }

    fn table(t: Table) -> Vec<Vec<String>> {
        let d = t.into_document().unwrap();
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    #[test]
    fn fill_blanks_above_below_value() {
        let p = Progress::new();
        let src = "k,v\r\na,1\r\n,\r\n , \r\nb,\r\n,2\r\n";
        let mut d = doc(src);
        let mut spec = FillBlanks {
            cols: vec![0, 1],
            from: FillFrom::Above,
            spaces: true,
            keep_first: true,
        };
        let plan = d.plan_fill_blanks(&spec, &p).unwrap();
        assert_eq!(plan.groups, 6);
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 0), ["k", "a", "a", "a", "b", "b"]);
        assert_eq!(col(&d, 1), ["v", "1", "1", "1", "1", "2"]);
        d.undo();
        assert_eq!(col(&d, 1), ["v", "1", "", " ", "", "2"]);
        spec.from = FillFrom::Below;
        spec.spaces = false;
        let plan = d.plan_fill_blanks(&spec, &p).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 0), ["k", "a", " ", " ", "b", ""]);
        assert_eq!(col(&d, 1), ["v", "1", " ", " ", "2", "2"]);
        d.undo();
        spec.from = FillFrom::Value { text: "n/a".into() };
        let plan = d.plan_fill_blanks(&spec, &p).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 1), ["v", "1", "n/a", " ", "n/a", "2"]);
        spec.from = FillFrom::Value {
            text: String::new(),
        };
        assert!(d.plan_fill_blanks(&spec, &p).is_err());
        spec.cols = vec![];
        assert!(d.plan_fill_blanks(&spec, &p).is_err());
    }

    #[test]
    fn fill_blanks_cross_chunks() {
        // Values carry across chunk edges both ways.
        let mut text = String::from("v\r\nfirst\r\n");
        for _ in 0..(CHUNK * 2 + 10) {
            text.push_str("\r\n");
        }
        text.push_str("last\r\n");
        let mut d = doc(&text);
        let n = d.row_count();
        let spec = FillBlanks {
            cols: vec![0],
            from: FillFrom::Above,
            spaces: false,
            keep_first: true,
        };
        let plan = d.plan_fill_blanks(&spec, &Progress::new()).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.cell(n - 2, 0).unwrap(), "first");
        d.undo();
        let spec = FillBlanks {
            from: FillFrom::Below,
            ..spec
        };
        let plan = d.plan_fill_blanks(&spec, &Progress::new()).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.cell(2, 0).unwrap(), "last");
        assert_eq!(d.cell(1, 0).unwrap(), "first");
    }

    #[test]
    fn repeats_counted_and_blanked() {
        let p = Progress::new();
        let mut d = doc("city,n\r\nRome,1\r\nOslo,2\r\nrome,3\r\nRome,4\r\n");
        let spec = Repeats {
            cols: vec![0],
            match_case: false,
            trim: true,
            keep_first: true,
            title: String::new(),
        };
        let plan = d.plan_count_repeats(&spec, &p).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(col(&d, 1), ["Count", "3", "1", "3", "3"]);
        assert_eq!(col(&d, 2), ["n", "1", "2", "3", "4"]);
        d.undo();
        assert_eq!(d.col_count(), 2);
        let dup = Duplicates {
            cols: vec![0],
            match_case: true,
            trim: false,
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_blank_repeats(&dup, &p).unwrap();
        assert_eq!((plan.rows, plan.groups), (1, 1));
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 0), ["city", "Rome", "Oslo", "rome", ""]);
    }

    #[test]
    fn breaks_between_groups_and_every_n() {
        let p = Progress::new();
        let mut d = doc("g,v\r\na,1\r\na,2\r\nb,3\r\nc,4\r\nc,5\r\n");
        let spec = Breaks {
            col: Some(0),
            every: 0,
            rows: 1,
            keep_first: true,
        };
        let plan = d.plan_breaks(&spec, &p).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 0), ["g", "a", "a", "", "b", "", "c", "c"]);
        d.undo();
        let spec = Breaks {
            col: None,
            every: 2,
            rows: 2,
            keep_first: true,
        };
        let plan = d.plan_breaks(&spec, &p).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 1), ["v", "1", "2", "", "", "3", "4", "", "", "5"]);
        assert!(d
            .plan_breaks(
                &Breaks {
                    rows: 0,
                    ..spec.clone()
                },
                &p
            )
            .is_err());
    }

    #[test]
    fn delete_hidden_rows_only() {
        let mut d = doc("a\r\nb\r\nc\r\nd\r\n");
        d.hide_rows(1, 2).unwrap();
        assert_eq!(col(&d, 0), ["a", "d"]);
        let plan = d.plan_delete_hidden().unwrap();
        assert_eq!(plan.rows, 2);
        d.apply_rows(plan).unwrap();
        assert_eq!(d.total_rows(), 2);
        assert_eq!(d.hidden_rows(), 0);
        d.undo();
        assert_eq!(d.total_rows(), 4);
        assert_eq!(d.hidden_rows(), 2);
        let mut d = doc("a,b,c,d\r\n1,2,3,4\r\n");
        let plan = d.plan_delete_cols(&[3, 1]).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(d.row_values(0).unwrap(), ["a", "c"]);
        d.undo();
        assert_eq!(d.col_count(), 4);
        assert!(d.plan_delete_cols(&[0, 1, 2, 3]).is_err());
    }

    #[test]
    fn file_fingerprint_is_sha256() {
        let p = std::env::temp_dir().join(format!("nde-sha-{}.txt", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            file_sha256(&p, &Progress::new()).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn audit_samples_per_group() {
        let mut csv = String::from(
            "id,precinct
",
        );
        for i in 0..300 {
            csv.push_str(&format!(
                "{i},P{}
",
                i % 3
            ));
        }
        let d = doc(&csv);
        let spec = Sample {
            size: SampleSize::Rows { n: 10 },
            seed: 2024,
            keep_first: true,
            per: Some(1),
            row_numbers: true,
            ..Default::default()
        };
        let (t, r) = d.sample_report(&spec, &Progress::new()).unwrap();
        assert_eq!((r.rows, r.taken, r.seed), (300, 30, 2024));
        assert_eq!(
            r.groups,
            [
                ("P0".into(), 100, 10),
                ("P1".into(), 100, 10),
                ("P2".into(), 100, 10)
            ]
        );
        assert!(r.method.contains("FNV-1a") && r.method.contains("seed 2024"));
        let s = t.into_document().unwrap();
        assert_eq!(s.row_values(0).unwrap(), ["Row", "id", "precinct"]);
        assert_eq!(s.row_count(), 31);
        // Ten of each precinct, in table order, with their row numbers.
        let rows: Vec<Vec<String>> = (1..31).map(|r| s.row_values(r).unwrap()).collect();
        for p in ["P0", "P1", "P2"] {
            assert_eq!(rows.iter().filter(|r| r[2] == p).count(), 10);
        }
        assert!(rows
            .windows(2)
            .all(|w| w[0][1].parse::<u32>().unwrap() < w[1][1].parse::<u32>().unwrap()));
        assert!(rows
            .iter()
            .all(|r| r[0].parse::<u32>().unwrap() == r[1].parse::<u32>().unwrap() + 2));
        // The same seed draws the same rows; another seed others.
        let (t2, _) = d.sample_report(&spec, &Progress::new()).unwrap();
        assert_eq!(t2.into_document().unwrap().row_values(5).unwrap(), rows[4]);
        let other = Sample {
            seed: 2025,
            ..spec.clone()
        };
        let (t3, _) = d.sample_report(&other, &Progress::new()).unwrap();
        let t3 = t3.into_document().unwrap();
        assert!((1..31).any(|r| t3.row_values(r).unwrap() != rows[r as usize - 1]));
        // A group's draw depends on its own value, not on the other groups.
        let few = doc(&csv
            .lines()
            .filter(|l| !l.ends_with("P2"))
            .collect::<Vec<_>>()
            .join(
                "
",
            ));
        let (t4, _) = few.sample_report(&spec, &Progress::new()).unwrap();
        let t4 = t4.into_document().unwrap();
        let p0: Vec<String> = (1..t4.row_count())
            .map(|r| t4.row_values(r).unwrap()[1].clone())
            .filter(|v| v.parse::<u32>().unwrap() % 3 == 0)
            .collect();
        let p0_all: Vec<String> = rows
            .iter()
            .filter(|r| r[2] == "P0")
            .map(|r| r[1].clone())
            .collect();
        assert_eq!(p0, p0_all);
        // Percent per group; every nth per group is refused.
        let pct = Sample {
            size: SampleSize::Percent { p: 5.0 },
            ..spec.clone()
        };
        assert_eq!(d.sample_report(&pct, &Progress::new()).unwrap().1.taken, 15);
        let every = Sample {
            size: SampleSize::Every { n: 2 },
            ..spec
        };
        assert!(d.sample_report(&every, &Progress::new()).is_err());
    }

    #[test]
    fn reshape_stack_sample() {
        let p = Progress::new();
        let d = doc("v,w\r\n1,a\r\n2,b\r\n3,c\r\n4,d\r\n5,e\r\n");
        let t = d
            .reshape_column(
                &Reshape {
                    col: 0,
                    per_row: 2,
                    down: false,
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        assert_eq!(table(t), [["1", "2"], ["3", "4"], ["5", ""]]);
        let t = d
            .reshape_column(
                &Reshape {
                    col: 0,
                    per_row: 2,
                    down: true,
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        assert_eq!(table(t), [["1", "4"], ["2", "5"], ["3", ""]]);
        let t = d
            .stack_columns(
                &Stack {
                    cols: vec![1, 0],
                    skip_blanks: false,
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        assert_eq!(t.rows, 10);
        assert_eq!(table(t)[..3], [vec!["a"], vec!["1"], vec!["b"]]);
        let spec = Sample {
            size: SampleSize::Rows { n: 3 },
            seed: 7,
            keep_first: true,
            ..Default::default()
        };
        let a = table(d.sample(&spec, &p).unwrap());
        let b = table(d.sample(&spec, &p).unwrap());
        assert_eq!(a, b);
        assert_eq!(a.len(), 4);
        assert_eq!(a[0], ["v", "w"]);
        // In table order, no repeats.
        let nums: Vec<u32> = a[1..].iter().map(|r| r[0].parse().unwrap()).collect();
        assert!(nums.windows(2).all(|w| w[0] < w[1]));
        let every = table(
            d.sample(
                &Sample {
                    size: SampleSize::Every { n: 2 },
                    seed: 0,
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap(),
        );
        assert_eq!(
            every.iter().map(|r| r[0].as_str()).collect::<Vec<_>>(),
            ["v", "1", "3", "5"]
        );
        let all = table(
            d.sample(
                &Sample {
                    size: SampleSize::Percent { p: 100.0 },
                    seed: 1,
                    keep_first: false,
                    ..Default::default()
                },
                &p,
            )
            .unwrap(),
        );
        assert_eq!(all.len(), 6);
        assert!(d
            .sample(
                &Sample {
                    size: SampleSize::Percent { p: 101.0 },
                    seed: 1,
                    keep_first: false,
                    ..Default::default()
                },
                &p
            )
            .is_err());
    }

    #[test]
    fn jumps() {
        let p = Progress::new();
        let d = doc("v\r\n1\r\n1\r\n7\r\n7\r\n-3\r\nx\r\n");
        assert_eq!(d.find_change(1, 0, false).unwrap(), Some(3));
        assert_eq!(d.find_change(3, 0, false).unwrap(), Some(5));
        assert_eq!(d.find_change(4, 0, true).unwrap(), Some(2));
        assert_eq!(d.find_change(6, 0, false).unwrap(), None);
        assert_eq!(
            d.find_extreme(0, true, true, ReadOptions::default(), &p)
                .unwrap(),
            Some(3)
        );
        assert_eq!(
            d.find_extreme(0, false, true, ReadOptions::default(), &p)
                .unwrap(),
            Some(5)
        );
        assert_eq!(
            d.find_extreme(5, false, true, ReadOptions::default(), &p)
                .unwrap(),
            None
        );
        assert_eq!(
            d.number_range(0, true, ReadOptions::default(), &p).unwrap(),
            Some((-3.0, 7.0))
        );
        assert_eq!(
            d.number_range(0, false, ReadOptions::default(), &p)
                .unwrap(),
            Some((-3.0, 7.0))
        );
        assert_eq!(
            doc("a\r\nb\r\n")
                .number_range(0, false, ReadOptions::default(), &p)
                .unwrap(),
            None
        );
        let d = doc("city\r\nRome\r\nrome\r\n Oslo \r\n\r\nRiga\r\n");
        let v = d.typeahead_values(0, true, 100, 100).unwrap();
        assert_eq!(
            v,
            [
                ("oslo".to_string(), "Oslo".to_string()),
                ("riga".into(), "Riga".into()),
                ("rome".into(), "Rome".into())
            ]
        );
        assert_eq!(d.typeahead_values(0, true, 1, 100).unwrap().len(), 1);
        assert_eq!(d.typeahead_values(9, true, 100, 100).unwrap().len(), 0);
    }

    #[test]
    fn compare_tables() {
        let p = Progress::new();
        let a = doc("id,name,qty\r\n1,ann,5\r\n2,bob,3\r\n3,cat,1\r\n");
        let b = doc("id,name,qty\r\n2,bob,4\r\n3,cat,1\r\n4,dan,9\r\n");
        let spec = CompareSpec {
            keys: vec![0],
            keep_first: true,
            ..Default::default()
        };
        let (t, r) = a.compare(&b, &spec, &p).unwrap();
        assert_eq!(
            r,
            CompareReport {
                same: 1,
                changed: 1,
                added: 1,
                removed: 1,
                cells: 1
            }
        );
        let rows = table(t);
        assert_eq!(
            rows[0],
            [
                "Change",
                "Row (first)",
                "Row (second)",
                "id",
                "name",
                "qty",
                "Changed columns"
            ]
        );
        assert_eq!(rows[1], ["removed", "2", "", "1", "ann", "5", ""]);
        assert_eq!(rows[2], ["changed", "3", "2", "2", "bob", "3 → 4", "qty"]);
        assert_eq!(rows[3], ["added", "", "4", "4", "dan", "9", ""]);
        assert_eq!(rows.len(), 4);
        // Whole rows: the changed row is one removed and one added.
        let whole = CompareSpec {
            keep_first: true,
            show_same: true,
            ..Default::default()
        };
        let (_, r) = a.compare(&b, &whole, &p).unwrap();
        assert_eq!((r.same, r.changed, r.added, r.removed), (1, 0, 2, 2));
        // Duplicate keys pair up in order.
        let a = doc("k\r\nx\r\nx\r\n");
        let b = doc("k\r\nx\r\n");
        let (_, r) = a.compare(&b, &whole, &p).unwrap();
        assert_eq!((r.same, r.removed, r.added), (1, 1, 0));
        // Spaces at the ends with "trim" count as the same; an extra filled
        // column on one side is a change.
        let a = doc("k,v\r\n1, x \r\n2,y\r\n");
        let b = doc("k,v,w\r\n1,x,\r\n2,y,z\r\n");
        let spec = CompareSpec {
            keys: vec![0],
            trim: true,
            keep_first: true,
            ..Default::default()
        };
        let (t, r) = a.compare(&b, &spec, &p).unwrap();
        assert_eq!((r.same, r.changed, r.cells), (1, 1, 1));
        assert_eq!(table(t)[1], ["changed", "3", "3", "2", "y", " → z", "w"]);
        // The second table changing between the steps is refused.
        let mut b = doc("k\r\n1\r\n2\r\n");
        let idx = CompareIndex::build(&b, &spec, &p).unwrap();
        let plan = a.compare_rows(&idx, &spec, &p).unwrap();
        b.hide_rows(1, 1).unwrap();
        assert!(matches!(
            b.compare_finish(&idx, plan, &spec, &p),
            Err(Error::Changed)
        ));
    }

    #[test]
    fn combine_by_title_and_position() {
        let p = Progress::new();
        let a = doc("id,Name\r\n1,ann\r\n");
        let b = doc("name ,id,extra\r\nbob,2,x\r\ncat,3,\r\n");
        let spec = Combine {
            header: true,
            by_title: true,
            source: true,
        };
        let t = combine(&[("a.csv".into(), &a), ("b.csv".into(), &b)], &spec, &p).unwrap();
        assert_eq!(
            table(t),
            [
                vec!["Source file", "id", "Name", "extra"],
                vec!["a.csv", "1", "ann", ""],
                vec!["b.csv", "2", "bob", "x"],
                vec!["b.csv", "3", "cat", ""],
            ]
        );
        let spec = Combine {
            header: false,
            by_title: true,
            source: false,
        };
        let t = combine(&[("a".into(), &a), ("b".into(), &b)], &spec, &p).unwrap();
        let rows = table(t);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[2], ["name ", "id", "extra"]);
        assert!(combine(&[], &spec, &p).is_err());
    }

    #[test]
    fn split_into_parts() {
        let p = Progress::new();
        let d = doc("g;v\r\na;\"1;2\"\r\nb;3\r\na;4\r\n");
        let parts = d
            .split_parts(
                &SplitParts {
                    guard: false,
                    col: Some(0),
                    every: 0,
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        assert_eq!(
            parts.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        let mut out = Vec::new();
        d.write_part(&parts[0].ids, true, false, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "g;v\r\na;\"1;2\"\r\na;4\r\n"
        );
        let parts = d
            .split_parts(
                &SplitParts {
                    guard: false,
                    col: None,
                    every: 2,
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].ids.len(), 1);
        assert!(d
            .split_parts(
                &SplitParts {
                    guard: false,
                    col: None,
                    every: 0,
                    keep_first: true
                },
                &p
            )
            .is_err());
    }

    #[test]
    fn review_round_1() {
        let p = Progress::new();
        // H1: repeats plans are charged and refused before they grow too big.
        let many: String = (0..100_000).map(|_| "1\n").collect();
        let d = doc(&many);
        let spec = Repeats {
            cols: vec![0],
            ..Default::default()
        };
        assert!(matches!(
            d.plan_count_repeats(&spec, &p),
            Err(Error::OutOfMemory(_))
        ));
        let wide: String = (0..50_000)
            .map(|_| "a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q,r,s,t\n")
            .collect();
        let d = doc(&wide);
        let dup = Duplicates {
            cols: (0..20).collect(),
            ..Default::default()
        };
        assert!(matches!(
            d.plan_blank_repeats(&dup, &p),
            Err(Error::OutOfMemory(_))
        ));
        // Small runs still work.
        let d = doc("a\na\nb\n");
        assert_eq!(
            d.plan_blank_repeats(
                &Duplicates {
                    cols: vec![0],
                    ..Default::default()
                },
                &p
            )
            .unwrap()
            .rows,
            1
        );

        // M2: the type-ahead caps hold across the whole scan; long values are left out.
        let mut text: String = (0..300).map(|i| format!("v{i:04}\n")).collect();
        text.push_str(&"x".repeat(TYPEAHEAD_CHARS + 1));
        text.push('\n');
        text.push_str("Mixed\n");
        let d = doc(&text);
        let v = d.typeahead_values(0, false, 1_000_000, 50).unwrap();
        assert!(v.len() <= 50 && !v.is_empty());
        let v = d.typeahead_values(0, false, 1_000_000, 10_000).unwrap();
        assert_eq!(v.len(), 301);
        assert!(v.contains(&("mixed".to_string(), "Mixed".to_string())));
        assert!(v.iter().all(|(_, w)| w.chars().count() <= TYPEAHEAD_CHARS));

        // M4: a trailing empty column on one side does not make rows differ.
        let whole = CompareSpec::default();
        for (x, y) in [
            ("a,1\nb,2\nc,3,\n", "a,1\nb,2\nc,3\n"),
            ("a,1\nb,2\nc,3\n", "a,1\nb,2\nc,3,\n"),
        ] {
            let (_, r) = doc(x).compare(&doc(y), &whole, &p).unwrap();
            assert_eq!(
                (r.same, r.added, r.removed, r.changed),
                (3, 0, 0, 0),
                "{x:?} vs {y:?}"
            );
        }
        // L1: a key column past both tables is refused, not a panic.
        let far = CompareSpec {
            keys: vec![9999],
            ..Default::default()
        };
        assert!(doc("a,1\n").compare(&doc("a,1\n"), &far, &p).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn rename_a_mapped_file() {
        // M5: a file large enough to be mapped (and locked) can be renamed.
        let dir = std::env::temp_dir().join(format!("nde-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let from = dir.join("big.csv");
        let row = "1234567890,abcdefghij,ABCDEFGHIJ\n";
        let n = (crate::source::MAP_THRESHOLD as usize / row.len()) + 10;
        std::fs::write(&from, row.repeat(n)).unwrap();
        let mut d = Document::open(&from, OpenOptions::default()).unwrap();
        while !d.index_step(64 << 20).unwrap() {}
        assert!(d.source.is_mapped());
        let to = dir.join("renamed.csv");
        d.rename_to(&to).unwrap();
        assert!(!from.exists() && to.exists());
        assert_eq!(d.path(), Some(to.as_path()));
        assert!(d.source.is_mapped());
        assert_eq!(d.cell(n as u64 - 1, 2).unwrap(), "ABCDEFGHIJ");
        // Never replaces: a second file in the way makes it fail, the document stays readable.
        std::fs::write(&from, "x\n").unwrap();
        assert!(d.rename_to(&from).is_err());
        assert_eq!(d.cell(0, 1).unwrap(), "abcdefghij");
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "x\n");
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
