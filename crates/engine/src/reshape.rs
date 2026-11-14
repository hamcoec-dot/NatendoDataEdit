//! Row reshaping of Milestone 4b: duplicates (show or remove), blank rows
//! and columns, split and merge rows, and pivot / unpivot into a new table.
//!
//! Row operations act on shown rows only (with a filter on, hidden rows are
//! never changed or removed); "first row is header" keeps the title row out.

use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::dialect::Dialect;
use crate::document::{Document, Edit, Progress, Step, View};
use crate::encoding::Encoding;
use crate::fill::format_number;
use crate::filter::FilterSpec;
use crate::ops::{check_cols, check_separator, ColPlan, Cols, CHUNK, MAX_CELL_BYTES};
use crate::value::{self, ReadOptions};
use crate::{Error, OpenOptions};

/// Most parts one cell may split into (split rows).
pub const MAX_PARTS: usize = 1_000;
/// Most rows a split may add.
pub const MAX_NEW_ROWS: u64 = 10_000_000;
/// Most columns a pivot may make.
pub const MAX_PIVOT_COLS: usize = 16_384;
/// Most cells a pivot or unpivot may make.
pub const MAX_TABLE_CELLS: u64 = 50_000_000;
/// Most text a new table may hold.
pub const MAX_TABLE_BYTES: usize = 1 << 30;
/// Most key columns.
pub const MAX_KEY_COLS: usize = 16_384;

/// Which columns make rows "the same", and how strictly.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Duplicates {
    /// Key columns (empty: every column).
    pub cols: Vec<u32>,
    pub match_case: bool,
    /// Ignore spaces at the start and end.
    pub trim: bool,
    pub keep_first: bool,
    /// Which copy stays (remove duplicates).
    pub keep: KeepCopy,
    /// The date (or number) column for newest / oldest.
    pub date_col: Option<u32>,
    /// Fill the kept copy's blank cells from the copies removed.
    pub fill_blanks: bool,
    /// How dates in `date_col` are read (day or month first).
    #[serde(default)]
    pub read: ReadOptions,
}

/// Which copy of a duplicate stays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepCopy {
    #[default]
    First,
    Last,
    /// The latest date in `date_col` (ties: the first).
    Newest,
    Oldest,
    /// The copy with the most cells filled in (ties: the first).
    MostFilled,
}

/// Blank rows or columns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Blanks {
    /// Cells holding only spaces count as blank.
    pub spaces: bool,
    pub keep_first: bool,
}

/// One row per part of a cell.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitRows {
    pub col: u32,
    pub separator: String,
    pub trim: bool,
    /// Leave out empty parts.
    pub skip_empty: bool,
    pub keep_first: bool,
}

/// Rows with the same key merged into the first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MergeRows {
    pub keys: Vec<u32>,
    pub separator: String,
    /// Each distinct value once.
    pub distinct: bool,
    pub match_case: bool,
    pub trim: bool,
    pub keep_first: bool,
}

/// How a pivot combines the values that land in one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    #[default]
    Count,
    Sum,
    Average,
    Min,
    Max,
    First,
    Join,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pivot {
    /// Columns whose values make the rows.
    pub rows: Vec<u32>,
    /// Column whose values become the new columns.
    pub col: u32,
    /// Column whose values fill the cells.
    pub value: u32,
    pub aggregate: Aggregate,
    /// Between joined values.
    pub separator: String,
    pub keep_first: bool,
    pub read: ReadOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Unpivot {
    /// Columns kept on every new row.
    pub ids: Vec<u32>,
    /// Columns turned into rows (empty: every other column).
    pub values: Vec<u32>,
    pub name_title: String,
    pub value_title: String,
    pub skip_blanks: bool,
    pub keep_first: bool,
}

/// A table built in memory (CSV bytes), opened as a new document.
#[derive(Debug)]
pub struct Table {
    pub bytes: Vec<u8>,
    pub rows: u64,
    pub cols: u32,
}

impl Table {
    /// Open the table as a new, unsaved document (comma separated, UTF-8).
    pub fn into_document(self) -> Result<Document, Error> {
        let opts = OpenOptions {
            dialect: Some(Dialect::new(b',', Some(b'"'))?),
            encoding: Some(Encoding::Utf8),
            ..Default::default()
        };
        let mut d = Document::from_bytes(self.bytes, opts, Some("csv"))?;
        d.mark_modified();
        Ok(d)
    }
}

/// Row changes as one undo step (or a filter, for "show duplicates").
pub struct RowPlan {
    pub(crate) revision: u64,
    /// Cell changes by row id: (logical column, text).
    pub(crate) changes: Vec<(u32, Vec<(u32, String)>)>,
    /// Row ids taken out.
    pub(crate) remove: Vec<u32>,
    /// New rows (logical values) put right after a row.
    pub(crate) after: Vec<(u32, Vec<Vec<String>>)>,
    pub(crate) view: Option<View>,
    /// Rows removed, added or shown.
    pub rows: u64,
    /// Groups found (duplicates, merges).
    pub groups: u64,
    /// Keep newest / oldest: copies whose date could not be read (they are
    /// never the one kept unless no copy of the group has a date).
    pub unreadable: u64,
}

impl RowPlan {
    pub(crate) fn new(d: &Document) -> RowPlan {
        RowPlan {
            revision: d.revision(),
            changes: Vec::new(),
            remove: Vec::new(),
            after: Vec::new(),
            view: None,
            rows: 0,
            groups: 0,
            unreadable: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
            && self.remove.is_empty()
            && self.after.is_empty()
            && self.view.is_none()
    }
}

fn norm<'a>(v: &'a str, match_case: bool, trim: bool) -> Cow<'a, str> {
    let v = if trim { v.trim() } else { v };
    if match_case {
        Cow::Borrowed(v)
    } else {
        Cow::Owned(v.to_lowercase())
    }
}

fn blank(v: &str, spaces: bool) -> bool {
    if spaces {
        v.trim().is_empty()
    } else {
        v.is_empty()
    }
}

fn check_keys(d: &Document, cols: &[u32]) -> Result<(), Error> {
    if cols.len() > MAX_KEY_COLS {
        return Err(Error::Invalid(format!(
            "at most {MAX_KEY_COLS} key columns"
        )));
    }
    check_cols(d, cols)
}

/// CSV field into `out` (quoted when needed).
fn push_csv(out: &mut Vec<u8>, v: &str) {
    if v.contains([',', '"', '\r', '\n']) {
        out.push(b'"');
        out.extend_from_slice(v.replace('"', "\"\"").as_bytes());
        out.push(b'"');
    } else {
        out.extend_from_slice(v.as_bytes());
    }
}

pub(crate) fn push_line<S: AsRef<str>>(out: &mut Vec<u8>, cells: &[S]) {
    for (i, c) in cells.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        push_csv(out, c.as_ref());
    }
    out.extend_from_slice(b"\r\n");
}

pub(crate) fn too_big() -> Error {
    Error::Invalid("the new table would be too large".into())
}

impl Document {
    /// Titles of logical columns: the first row's text with "first row is
    /// header", otherwise "Column N".
    pub fn titles(&self, cols: &[u32], keep_first: bool) -> Vec<String> {
        let id = if keep_first { self.id_at(0) } else { None };
        cols.iter()
            .map(|&c| match id.and_then(|id| self.cell_by_id(id, c)) {
                Some(t) if !t.is_empty() => t,
                _ => format!("Column {}", c + 1),
            })
            .collect()
    }

    /// For each shown row in `rows` (by offset), the offset of the first row
    /// with the same key. Keys are hashed with a random key, and rows whose
    /// hashes match are compared in full, so different keys are never
    /// treated as the same.
    pub(crate) fn group_rows(
        &self,
        rows: std::ops::Range<u64>,
        cols: &[u32],
        match_case: bool,
        trim: bool,
        progress: &Progress,
    ) -> Result<Vec<u32>, Error> {
        let n = rows.end.saturating_sub(rows.start);
        if n > u32::MAX as u64 {
            return Err(Error::TooManyRows);
        }
        let reader = Cols::new(self, cols);
        let rs = RandomState::new();
        let mut hashed: Vec<(u64, u32)> = self.par_rows(rows.clone(), progress, |r, id, buf| {
            reader.read(self, id, buf);
            let mut h = rs.build_hasher();
            for k in 0..cols.len() {
                let v = norm(reader.get(buf, k), match_case, trim);
                h.write_usize(v.len());
                h.write(v.as_bytes());
            }
            Ok(Some((h.finish(), (r - rows.start) as u32)))
        })?;
        hashed.par_sort_unstable();
        let key = |i: u32| -> Vec<String> {
            let id = self.id_at(rows.start + i as u64).expect("in range");
            let mut buf = Vec::new();
            reader.read(self, id, &mut buf);
            (0..cols.len())
                .map(|k| norm(reader.get(&buf, k), match_case, trim).into_owned())
                .collect()
        };
        // Runs of equal hashes, compared in parallel (runs are independent).
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut i = 0usize;
        while i < hashed.len() {
            let mut j = i + 1;
            while j < hashed.len() && hashed[j].0 == hashed[i].0 {
                j += 1;
            }
            if j - i > 1 {
                runs.push((i, j));
            }
            i = j;
        }
        progress.start(runs.len() as u64);
        let found: Vec<Vec<(u32, u32)>> = runs
            .par_iter()
            .map(|&(i, j)| -> Result<Vec<(u32, u32)>, Error> {
                // Same hash: compare the real keys, in row order. Keys are
                // read in parallel batches (a run can hold millions of rows
                // when one value repeats).
                let mut out = Vec::new();
                let mut firsts: Vec<(u32, Vec<String>)> = Vec::new();
                for batch in hashed[i..j].chunks(1 << 16) {
                    let keys: Vec<Vec<String>> =
                        batch.par_iter().map(|&(_, row)| key(row)).collect();
                    for (&(_, row), k) in batch.iter().zip(keys) {
                        match firsts.iter().find(|f| f.1 == k) {
                            Some(f) => out.push((row, f.0)),
                            None => firsts.push((row, k)),
                        }
                    }
                }
                progress.step(1)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let mut leader: Vec<u32> = (0..n as u32).collect();
        for (row, first) in found.into_iter().flatten() {
            leader[row as usize] = first;
        }
        Ok(leader)
    }

    fn dup_cols(&self, spec: &Duplicates) -> Result<Vec<u32>, Error> {
        check_keys(self, &spec.cols)?;
        Ok(if spec.cols.is_empty() {
            (0..self.col_count()).collect()
        } else {
            spec.cols.clone()
        })
    }

    /// Plan a filter that shows every row with a duplicate (the first one
    /// too). Not an undo step, like any filter.
    pub fn plan_show_duplicates(
        &self,
        spec: &Duplicates,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        let cols = self.dup_cols(spec)?;
        let rows = self.data_rows(spec.keep_first);
        let leader = self.group_rows(rows.clone(), &cols, spec.match_case, spec.trim, progress)?;
        let mut in_group = vec![false; leader.len()];
        let mut plan = RowPlan::new(self);
        for (i, &l) in leader.iter().enumerate() {
            if l as usize != i {
                if !in_group[l as usize] {
                    plan.groups += 1;
                    in_group[l as usize] = true;
                }
                in_group[i] = true;
            }
        }
        let base = self.next_row_id();
        let mut bits = vec![0u64; (base as usize).div_ceil(64)];
        let mut set = |id: u32| bits[(id >> 6) as usize] |= 1 << (id & 63);
        if rows.start == 1 {
            set(self.id_at(0).expect("a first row"));
        }
        for (i, g) in in_group.iter().enumerate() {
            if *g {
                set(self.id_at(rows.start + i as u64).expect("in range"));
                plan.rows += 1;
            }
        }
        let filter = FilterSpec {
            keep_first: spec.keep_first,
            ..Default::default()
        };
        plan.view = Some(View::new(filter, bits, base));
        Ok(plan)
    }

    /// Plan removing every row that repeats an earlier row's key.
    pub fn plan_remove_duplicates(
        &self,
        spec: &Duplicates,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        let cols = self.dup_cols(spec)?;
        let rows = self.data_rows(spec.keep_first);
        let leader = self.group_rows(rows.clone(), &cols, spec.match_case, spec.trim, progress)?;
        if spec.keep != KeepCopy::First || spec.fill_blanks {
            return self.plan_keep_copies(spec, rows, &leader);
        }
        let mut plan = RowPlan::new(self);
        let mut counted = vec![false; leader.len()];
        for (i, &l) in leader.iter().enumerate() {
            if l as usize != i {
                if !counted[l as usize] {
                    counted[l as usize] = true;
                    plan.groups += 1;
                }
                plan.remove
                    .push(self.id_at(rows.start + i as u64).expect("in range"));
            }
        }
        plan.rows = plan.remove.len() as u64;
        Ok(plan)
    }

    /// Remove duplicates keeping a chosen copy, optionally filling its
    /// blanks from the others.
    fn plan_keep_copies(
        &self,
        spec: &Duplicates,
        rows: std::ops::Range<u64>,
        leader: &[u32],
    ) -> Result<RowPlan, Error> {
        if matches!(spec.keep, KeepCopy::Newest | KeepCopy::Oldest) {
            match spec.date_col {
                Some(c) => check_cols(self, &[c])?,
                None => return Err(Error::Invalid("choose the date column".into())),
            }
        }
        // Members of each group with copies, in row order.
        let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
        for (i, &l) in leader.iter().enumerate() {
            if l as usize != i {
                groups.entry(l).or_insert_with(|| vec![l]).push(i as u32);
            }
        }
        let mut groups: Vec<Vec<u32>> = groups.into_values().collect();
        groups.sort_unstable_by_key(|g| g[0]);
        let all: Vec<u32> = (0..self.col_count()).collect();
        let whole = Cols::new(self, &all);
        let date = spec.date_col.map(|c| Cols::new(self, &[c]));
        let id_of = |i: u32| self.id_at(rows.start + i as u64).expect("in range");
        let read = spec.read;
        let unreadable = std::sync::atomic::AtomicU64::new(0);
        // (kept row, rows to drop, blanks filled in the kept row)
        type Pick = (u32, Vec<u32>, Vec<(u32, String)>);
        let picked: Vec<Pick> = groups
            .par_iter()
            .map(|g| {
                let ids: Vec<u32> = g.iter().map(|&i| id_of(i)).collect();
                // Rows are read one at a time (a group can be huge): first to
                // pick the copy kept, then, for filling, to find each blank's
                // first value among the others.
                let mut buf = Vec::new();
                let keep = match spec.keep {
                    KeepCopy::First => 0,
                    KeepCopy::Last => ids.len() - 1,
                    KeepCopy::MostFilled => {
                        let mut best = (0usize, 0usize);
                        for (k, &id) in ids.iter().enumerate() {
                            whole.read(self, id, &mut buf);
                            let n = (0..all.len())
                                .filter(|&c| !whole.get(&buf, c).trim().is_empty())
                                .count();
                            if k == 0 || n > best.1 {
                                best = (k, n);
                            }
                        }
                        best.0
                    }
                    KeepCopy::Newest | KeepCopy::Oldest => {
                        let rd = date.as_ref().expect("checked");
                        // Dates, or plain numbers (years, version numbers)
                        // when no copy holds a date: never the two mixed.
                        let values: Vec<String> = ids
                            .iter()
                            .map(|&id| {
                                rd.read(self, id, &mut buf);
                                rd.get(&buf, 0).trim().to_string()
                            })
                            .collect();
                        let dates: Vec<Option<f64>> = values
                            .iter()
                            .map(|v| {
                                crate::value::parse_date(v, read)
                                    .filter(|d| d.floor() != 0.0 || v.contains(['-', '/', '.']))
                            })
                            .collect();
                        let weights: Vec<Option<f64>> = if dates.iter().any(Option::is_some) {
                            dates
                        } else {
                            values
                                .iter()
                                .map(|v| crate::value::parse_number(v, read))
                                .collect()
                        };
                        let mut best: Option<(usize, f64)> = None;
                        for (k, w) in weights.iter().enumerate() {
                            let Some(w) = *w else {
                                if !values[k].is_empty() {
                                    unreadable.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                                continue;
                            };
                            let better = match best {
                                None => true,
                                Some((_, b)) if spec.keep == KeepCopy::Newest => w > b,
                                Some((_, b)) => w < b,
                            };
                            if better {
                                best = Some((k, w));
                            }
                        }
                        best.map_or(0, |b| b.0)
                    }
                };
                let mut fill = Vec::new();
                if spec.fill_blanks {
                    whole.read(self, ids[keep], &mut buf);
                    let mut blank: Vec<u32> = (0..all.len())
                        .filter(|&c| whole.get(&buf, c).trim().is_empty())
                        .map(|c| c as u32)
                        .collect();
                    for (k, &id) in ids.iter().enumerate() {
                        if blank.is_empty() {
                            break;
                        }
                        if k == keep {
                            continue;
                        }
                        whole.read(self, id, &mut buf);
                        blank.retain(|&c| {
                            let v = whole.get(&buf, c as usize);
                            if v.trim().is_empty() {
                                return true;
                            }
                            fill.push((c, v.to_string()));
                            false
                        });
                    }
                }
                let gone: Vec<u32> = ids
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k != keep)
                    .map(|(_, &id)| id)
                    .collect();
                (ids[keep], gone, fill)
            })
            .collect();
        let mut plan = RowPlan::new(self);
        plan.groups = picked.len() as u64;
        plan.unreadable = unreadable.into_inner();
        for (kept, gone, fill) in picked {
            plan.remove.extend(gone);
            if !fill.is_empty() {
                plan.changes.push((kept, fill));
            }
        }
        plan.rows = plan.remove.len() as u64;
        Ok(plan)
    }

    /// Plan removing shown rows whose every cell is blank.
    pub fn plan_remove_blank_rows(
        &self,
        spec: &Blanks,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        let cols: Vec<u32> = (0..self.col_count()).collect();
        let reader = Cols::new(self, &cols);
        let rows = self.data_rows(spec.keep_first);
        let remove = self.par_rows(rows, progress, |_, id, buf| {
            reader.read(self, id, buf);
            let empty = (0..cols.len()).all(|k| blank(reader.get(buf, k), spec.spaces));
            Ok(empty.then_some(id))
        })?;
        let mut plan = RowPlan::new(self);
        plan.rows = remove.len() as u64;
        plan.remove = remove;
        Ok(plan)
    }

    /// Plan removing columns that are blank in every row (hidden rows too;
    /// the title row is not looked at with `keep_first`). One column always
    /// stays.
    pub fn plan_remove_blank_cols(
        &self,
        spec: &Blanks,
        progress: &Progress,
    ) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        let w = self.col_count() as usize;
        let cols: Vec<u32> = (0..w as u32).collect();
        let reader = Cols::new(self, &cols);
        let total = self.total_rows();
        let from = u64::from(spec.keep_first && total > 0);
        progress.start(total - from);
        let all_filled = AtomicBool::new(false);
        let chunks = (total - from).div_ceil(CHUNK as u64);
        let filled: Vec<bool> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Vec<bool>, Error> {
                let mut seen = vec![false; w];
                if all_filled.load(Ordering::Relaxed) {
                    return Ok(seen);
                }
                let s = from + c * CHUNK as u64;
                let e = (s + CHUNK as u64).min(total);
                let mut buf = Vec::new();
                for p in s..e {
                    let id = self.doc_id_at(p).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for (k, f) in seen.iter_mut().enumerate() {
                        if !*f && !blank(reader.get(&buf, k), spec.spaces) {
                            *f = true;
                        }
                    }
                }
                if seen.iter().all(|&f| f) {
                    all_filled.store(true, Ordering::Relaxed);
                }
                progress.step(e - s)?;
                Ok(seen)
            })
            .try_reduce(
                || vec![false; w],
                |mut a, b| {
                    a.iter_mut().zip(b).for_each(|(x, y)| *x |= y);
                    Ok(a)
                },
            )?;
        let mut plan = ColPlan::new(self);
        let mut gone: Vec<u32> = (0..w as u32).filter(|&c| !filled[c as usize]).collect();
        if w > 0 && gone.len() == w {
            gone.remove(0);
        }
        plan.remove_cols(gone);
        Ok(plan)
    }

    /// Plan one row per part of a column's cell (the other cells repeat).
    pub fn plan_split_rows(&self, spec: &SplitRows, progress: &Progress) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[spec.col])?;
        check_separator(&spec.separator)?;
        if spec.separator.is_empty() {
            return Err(Error::Invalid("choose a separator to split at".into()));
        }
        let reader = Cols::new(self, &[spec.col]);
        let added = AtomicUsize::new(0);
        let rows = self.data_rows(spec.keep_first);
        let split: Vec<(u32, Vec<String>)> = self.par_rows(rows, progress, |_, id, buf| {
            reader.read(self, id, buf);
            let v = reader.get(buf, 0);
            if !v.contains(spec.separator.as_str()) {
                return Ok(None);
            }
            let mut parts: Vec<String> = Vec::new();
            for p in v.split(spec.separator.as_str()) {
                let p = if spec.trim { p.trim() } else { p };
                if spec.skip_empty && p.is_empty() {
                    continue;
                }
                if parts.len() == MAX_PARTS {
                    return Err(Error::Invalid(format!(
                        "a cell splits into more than {MAX_PARTS} parts"
                    )));
                }
                parts.push(p.to_string());
            }
            if parts.len() < 2 {
                return Ok(None);
            }
            if added.fetch_add(parts.len() - 1, Ordering::Relaxed) + parts.len() - 1
                > MAX_NEW_ROWS as usize
            {
                return Err(Error::Invalid(format!(
                    "a split can add at most {MAX_NEW_ROWS} rows at once"
                )));
            }
            Ok(Some((id, parts)))
        })?;
        let mut plan = RowPlan::new(self);
        let c = spec.col as usize;
        for (id, mut parts) in split {
            let first = std::mem::take(&mut parts[0]);
            let values = self.logical_values(id)?;
            let mut extra = Vec::with_capacity(parts.len() - 1);
            for p in parts.into_iter().skip(1) {
                let mut row = values.clone();
                if row.len() <= c {
                    row.resize(c + 1, String::new());
                }
                row[c] = p;
                extra.push(row);
            }
            plan.rows += extra.len() as u64;
            plan.groups += 1;
            plan.changes.push((id, vec![(spec.col, first)]));
            plan.after.push((id, extra));
        }
        Ok(plan)
    }

    /// Plan merging rows with the same key into the first of them: the
    /// other columns' values are joined with the separator, blanks left out.
    pub fn plan_merge_rows(&self, spec: &MergeRows, progress: &Progress) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        check_keys(self, &spec.keys)?;
        check_separator(&spec.separator)?;
        if spec.keys.is_empty() {
            return Err(Error::Invalid(
                "choose the column(s) that identify a row".into(),
            ));
        }
        let rows = self.data_rows(spec.keep_first);
        let leader = self.group_rows(
            rows.clone(),
            &spec.keys,
            spec.match_case,
            spec.trim,
            progress,
        )?;
        let mut members: HashMap<u32, Vec<u32>> = HashMap::new();
        for (i, &l) in leader.iter().enumerate() {
            if l as usize != i {
                members.entry(l).or_insert_with(|| vec![l]).push(i as u32);
            }
        }
        let mut plan = RowPlan::new(self);
        let w = self.col_count() as usize;
        let mut groups: Vec<(u32, Vec<u32>)> = members.into_iter().collect();
        groups.sort_unstable();
        let is_key: Vec<bool> = (0..w as u32).map(|c| spec.keys.contains(&c)).collect();
        let mut made = 0usize;
        for (_, rows_in) in groups {
            let ids: Vec<u32> = rows_in
                .iter()
                .map(|&i| self.id_at(rows.start + i as u64).expect("in range"))
                .collect();
            let values: Vec<Vec<String>> = ids
                .iter()
                .map(|&id| self.logical_values(id))
                .collect::<Result<_, _>>()?;
            let mut cells = Vec::new();
            for (c, &key_col) in is_key.iter().enumerate() {
                if key_col {
                    continue;
                }
                let mut out = String::new();
                let mut seen: Vec<&str> = Vec::new();
                for v in &values {
                    let x = v.get(c).map_or("", String::as_str);
                    if x.is_empty() || (spec.distinct && seen.contains(&x)) {
                        continue;
                    }
                    if !seen.is_empty() {
                        out.push_str(&spec.separator);
                    }
                    out.push_str(x);
                    seen.push(x);
                    if out.len() > MAX_CELL_BYTES {
                        return Err(Error::Invalid("a merged cell would be too long".into()));
                    }
                }
                if out != values[0].get(c).map_or("", String::as_str) {
                    made += out.len();
                    if made > MAX_TABLE_BYTES {
                        return Err(Error::Invalid(
                            "this merge would create too much text at once".into(),
                        ));
                    }
                    cells.push((c as u32, out));
                }
            }
            plan.groups += 1;
            plan.rows += ids.len() as u64 - 1;
            if !cells.is_empty() {
                plan.changes.push((ids[0], cells));
            }
            plan.remove.extend_from_slice(&ids[1..]);
            progress.step(0)?;
        }
        Ok(plan)
    }

    /// Apply a [`RowPlan`]: one undo step, or (show duplicates) a filter.
    pub fn apply_rows(&mut self, plan: RowPlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        self.ready_for_bulk()?;
        if let Some(v) = plan.view {
            self.set_view(Some(v));
            return Ok(());
        }
        let mut changes = Vec::with_capacity(plan.changes.len());
        for (id, cells) in plan.changes {
            let mut out = Vec::with_capacity(cells.len());
            for (c, v) in cells {
                out.push((self.phys(c).ok_or(Error::OutOfRange)?, v));
            }
            changes.push((id, out));
        }
        let mut states = self.patch_states(changes)?;
        let base = self.next_row_id() as usize;
        let mut gone = vec![0u64; base.div_ceil(64)];
        for &id in &plan.remove {
            gone[(id >> 6) as usize] |= 1 << (id & 63);
            if self.is_edited(id) {
                states.push((id, None));
            }
        }
        let is_gone =
            |id: u32| (id as usize) < base && (gone[(id >> 6) as usize] >> (id & 63)) & 1 == 1;
        let new_rows: u64 = plan.after.iter().map(|(_, r)| r.len() as u64).sum();
        let first = self.new_row_ids(u32::try_from(new_rows).map_err(|_| Error::TooManyRows)?)?;
        let mut next = first;
        let mut after: HashMap<u32, (u32, u32)> = HashMap::with_capacity(plan.after.len());
        for (id, rows) in plan.after {
            let n = rows.len() as u32;
            after.insert(id, (next, n));
            for r in rows {
                states.push((next, Some(Edit::Full(self.to_physical(r, None)))));
                next += 1;
            }
        }
        let order = if plan.remove.is_empty() && after.is_empty() {
            None
        } else {
            let mut o = Vec::with_capacity(self.total_rows() as usize + new_rows as usize);
            for id in self.ids() {
                if is_gone(id) {
                    continue;
                }
                o.push(id);
                if let Some(&(s, n)) = after.get(&id) {
                    o.extend(s..s + n);
                }
            }
            Some(o)
        };
        self.push_step(Step {
            states,
            order,
            ..Default::default()
        });
        Ok(())
    }

    // ---- pivot and unpivot ----

    /// Build a pivot table: one row per distinct row key, one column per
    /// distinct value of `col`, cells combining `value`.
    pub fn pivot(&self, spec: &Pivot, progress: &Progress) -> Result<Table, Error> {
        self.ready_for_bulk()?;
        check_keys(self, &spec.rows)?;
        check_cols(self, &[spec.col, spec.value])?;
        check_separator(&spec.separator)?;
        if spec.rows.is_empty() {
            return Err(Error::Invalid("choose the column(s) for the rows".into()));
        }
        let mut cols: Vec<u32> = spec.rows.clone();
        cols.push(spec.col);
        cols.push(spec.value);
        let reader = Cols::new(self, &cols);
        let nk = spec.rows.len();
        enum Acc {
            Count(u64),
            Num {
                sum: f64,
                min: f64,
                max: f64,
                n: u64,
            },
            First(String),
            Join(String),
        }
        let mut row_ix: HashMap<Vec<String>, u32> = HashMap::new();
        let mut row_keys: Vec<Vec<String>> = Vec::new();
        let mut col_ix: HashMap<String, u32> = HashMap::new();
        let mut col_keys: Vec<String> = Vec::new();
        let mut cells: HashMap<(u32, u32), Acc> = HashMap::new();
        let mut text = 0usize;
        let rows = self.data_rows(spec.keep_first);
        progress.start(rows.end - rows.start);
        let mut buf = Vec::new();
        for r in rows.clone() {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let rk: Vec<String> = (0..nk).map(|k| reader.get(&buf, k).to_string()).collect();
            let ck = reader.get(&buf, nk);
            let v = reader.get(&buf, nk + 1);
            let ri = match row_ix.get(&rk) {
                Some(&i) => i,
                None => {
                    text += rk.iter().map(String::len).sum::<usize>();
                    let i = row_keys.len() as u32;
                    row_ix.insert(rk.clone(), i);
                    row_keys.push(rk);
                    i
                }
            };
            let ci = match col_ix.get(ck) {
                Some(&i) => i,
                None => {
                    if col_keys.len() == MAX_PIVOT_COLS {
                        return Err(Error::Invalid(format!(
                            "a pivot can make at most {MAX_PIVOT_COLS} columns"
                        )));
                    }
                    let i = col_keys.len() as u32;
                    col_ix.insert(ck.to_string(), i);
                    col_keys.push(ck.to_string());
                    i
                }
            };
            let num = || value::parse_number(v, spec.read);
            match cells.get_mut(&(ri, ci)) {
                None => {
                    let acc = match spec.aggregate {
                        Aggregate::Count => Acc::Count(u64::from(!v.is_empty())),
                        Aggregate::First => Acc::First(v.to_string()),
                        Aggregate::Join => Acc::Join(v.to_string()),
                        _ => match num() {
                            Some(x) => Acc::Num {
                                sum: x,
                                min: x,
                                max: x,
                                n: 1,
                            },
                            None => Acc::Num {
                                sum: 0.0,
                                min: f64::INFINITY,
                                max: f64::NEG_INFINITY,
                                n: 0,
                            },
                        },
                    };
                    text += v.len().min(64);
                    cells.insert((ri, ci), acc);
                }
                Some(Acc::Count(n)) => *n += u64::from(!v.is_empty()),
                Some(Acc::First(_)) => {}
                Some(Acc::Join(s)) => {
                    if !v.is_empty() {
                        if !s.is_empty() {
                            s.push_str(&spec.separator);
                        }
                        s.push_str(v);
                        text += v.len() + spec.separator.len();
                        if s.len() > MAX_CELL_BYTES {
                            return Err(Error::Invalid("a joined cell would be too long".into()));
                        }
                    }
                }
                Some(Acc::Num { sum, min, max, n }) => {
                    if let Some(x) = num() {
                        *sum += x;
                        *min = min.min(x);
                        *max = max.max(x);
                        *n += 1;
                    }
                }
            }
            if text > MAX_TABLE_BYTES {
                return Err(too_big());
            }
            if (r - rows.start) % 65_536 == 0 {
                progress.step(65_536.min(rows.end - r))?;
            }
        }
        let width = nk as u64 + col_keys.len() as u64;
        if (row_keys.len() as u64 + 1).saturating_mul(width) > MAX_TABLE_CELLS {
            return Err(too_big());
        }
        let comma = spec.read.decimal_comma;
        let mut out = Vec::new();
        let mut header = self.titles(&spec.rows, spec.keep_first);
        header.extend(col_keys.iter().map(|k| {
            if k.is_empty() {
                "(blank)".to_string()
            } else {
                k.clone()
            }
        }));
        push_line(&mut out, &header);
        let mut line: Vec<String> = Vec::with_capacity(width as usize);
        for (ri, rk) in row_keys.iter().enumerate() {
            line.clear();
            // Blank keys get the same label as blank column values.
            line.extend(rk.iter().map(|k| {
                if k.is_empty() {
                    "(blank)".to_string()
                } else {
                    k.clone()
                }
            }));
            for ci in 0..col_keys.len() {
                let cell = match cells.get(&(ri as u32, ci as u32)) {
                    None => String::new(),
                    Some(Acc::Count(n)) => n.to_string(),
                    Some(Acc::First(s)) | Some(Acc::Join(s)) => s.clone(),
                    Some(Acc::Num { sum, min, max, n }) => {
                        if *n == 0 {
                            String::new()
                        } else {
                            let x = match spec.aggregate {
                                Aggregate::Sum => *sum,
                                Aggregate::Average => sum / *n as f64,
                                Aggregate::Min => *min,
                                _ => *max,
                            };
                            format_number(x, comma)
                        }
                    }
                };
                line.push(cell);
            }
            push_line(&mut out, &line);
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
        }
        Ok(Table {
            bytes: out,
            rows: row_keys.len() as u64 + 1,
            cols: width as u32,
        })
    }

    /// Build an unpivoted table: for each shown row and each value column,
    /// a row of the id columns, the column's title and its value.
    pub fn unpivot(&self, spec: &Unpivot, progress: &Progress) -> Result<Table, Error> {
        self.ready_for_bulk()?;
        check_keys(self, &spec.ids)?;
        check_keys(self, &spec.values)?;
        let values: Vec<u32> = if spec.values.is_empty() {
            (0..self.col_count())
                .filter(|c| !spec.ids.contains(c))
                .collect()
        } else {
            spec.values.clone()
        };
        if values.is_empty() {
            return Err(Error::Invalid(
                "choose the columns to turn into rows".into(),
            ));
        }
        let rows = self.data_rows(spec.keep_first);
        let out_rows = (rows.end - rows.start).saturating_mul(values.len() as u64);
        if out_rows.saturating_mul(spec.ids.len() as u64 + 2) > MAX_TABLE_CELLS {
            return Err(too_big());
        }
        let titles = self.titles(&values, spec.keep_first);
        let mut cols = spec.ids.clone();
        cols.extend_from_slice(&values);
        let reader = Cols::new(self, &cols);
        let nid = spec.ids.len();
        let size = AtomicUsize::new(0);
        let mut out = Vec::new();
        let mut header = self.titles(&spec.ids, spec.keep_first);
        let pick = |t: &str, d: &str| {
            if t.is_empty() {
                d.to_string()
            } else {
                t.to_string()
            }
        };
        header.push(pick(&spec.name_title, "Name"));
        header.push(pick(&spec.value_title, "Value"));
        push_line(&mut out, &header);
        let chunks: Vec<(Vec<u8>, u64)> = {
            let r0 = rows.start;
            progress.start(rows.end - r0);
            let n = (rows.end - r0).div_ceil(CHUNK as u64);
            (0..n)
                .into_par_iter()
                .map(|c| -> Result<(Vec<u8>, u64), Error> {
                    let s = r0 + c * CHUNK as u64;
                    let e = (s + CHUNK as u64).min(rows.end);
                    let mut bytes = Vec::new();
                    let mut count = 0u64;
                    let mut buf = Vec::new();
                    for r in s..e {
                        let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                        reader.read(self, id, &mut buf);
                        let mut line: Vec<&str> = Vec::with_capacity(nid + 2);
                        for (k, t) in titles.iter().enumerate() {
                            let v = reader.get(&buf, nid + k);
                            if spec.skip_blanks && v.is_empty() {
                                continue;
                            }
                            line.clear();
                            line.extend((0..nid).map(|i| reader.get(&buf, i)));
                            line.push(t);
                            line.push(v);
                            push_line(&mut bytes, &line);
                            count += 1;
                        }
                    }
                    if size.fetch_add(bytes.len(), Ordering::Relaxed) + bytes.len()
                        > MAX_TABLE_BYTES
                    {
                        return Err(too_big());
                    }
                    progress.step(e - s)?;
                    Ok((bytes, count))
                })
                .collect::<Result<_, _>>()?
        };
        let mut n = 1u64;
        out.reserve(chunks.iter().map(|c| c.0.len()).sum());
        for (b, c) in chunks {
            out.extend_from_slice(&b);
            n += c;
        }
        Ok(Table {
            bytes: out,
            rows: n,
            cols: nid as u32 + 2,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_keep_a_chosen_copy() {
        let base = "id,date,phone,email
1,2024-01-05,,a@x
2,2024-01-01,555,
1,2024-03-01,777,
1,2023-12-31,,
";
        let run = |keep: KeepCopy, fill: bool| {
            let mut d = doc(base);
            let spec = Duplicates {
                cols: vec![0],
                keep_first: true,
                keep,
                date_col: Some(1),
                fill_blanks: fill,
                ..Default::default()
            };
            let plan = d.plan_remove_duplicates(&spec, &Progress::new()).unwrap();
            assert_eq!((plan.rows, plan.groups), (2, 1));
            d.apply_rows(plan).unwrap();
            let rows = all(&d);
            assert!(d.undo());
            assert_eq!(all(&d), all(&doc(base)));
            rows
        };
        assert_eq!(
            run(KeepCopy::First, false)[1],
            vec!["1", "2024-01-05", "", "a@x"]
        );
        assert_eq!(
            run(KeepCopy::Last, false)[2],
            vec!["1", "2023-12-31", "", ""]
        );
        assert_eq!(
            run(KeepCopy::Newest, false)[2],
            vec!["1", "2024-03-01", "777", ""]
        );
        assert_eq!(
            run(KeepCopy::Oldest, false)[2],
            vec!["1", "2023-12-31", "", ""]
        );
        assert_eq!(
            run(KeepCopy::MostFilled, false)[1],
            vec!["1", "2024-01-05", "", "a@x"]
        );
        // Newest, with its blanks filled from the copies removed.
        assert_eq!(
            run(KeepCopy::Newest, true)[2],
            vec!["1", "2024-03-01", "777", "a@x"]
        );
        assert_eq!(
            run(KeepCopy::First, true)[1],
            vec!["1", "2024-01-05", "777", "a@x"]
        );
        // Review L6: the run's day order; unreadable dates counted, never
        // mixed with plain numbers.
        let newest = |text: &str, day_first: bool| {
            let d = doc(text);
            let spec = Duplicates {
                cols: vec![0],
                keep_first: true,
                keep: KeepCopy::Newest,
                date_col: Some(1),
                read: crate::value::ReadOptions {
                    date_order: if day_first {
                        crate::value::DateOrder::Dmy
                    } else {
                        crate::value::DateOrder::Mdy
                    },
                    ..Default::default()
                },
                ..Default::default()
            };
            let plan = d.plan_remove_duplicates(&spec, &Progress::new()).unwrap();
            let kept: Vec<u32> = (1..d.row_count())
                .filter_map(|r| d.id_at(r))
                .filter(|id| !plan.remove.contains(id))
                .collect();
            (kept, plan.unreadable)
        };
        let t = "id,date\r\n1,03/04/2026\r\n1,02/05/2026\r\n";
        assert_eq!(
            newest(t, false).0,
            vec![1],
            "March 4 is newer than February 5"
        );
        assert_eq!(newest(t, true).0, vec![2], "2 May is newer than 3 April");
        let t = "id,date\r\n1,2026-01-01\r\n1,99999\r\n1,soon\r\n";
        assert_eq!(newest(t, false), (vec![1], 2));
        let t = "id,date\r\n1,3\r\n1,7\r\n1,\r\n";
        assert_eq!(newest(t, false), (vec![2], 0));
        let mut d = doc(base);
        let bad = Duplicates {
            cols: vec![0],
            keep: KeepCopy::Newest,
            ..Default::default()
        };
        assert!(d.plan_remove_duplicates(&bad, &Progress::new()).is_err());
        let _ = &mut d;
    }

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn all(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    fn col(d: &Document, c: usize) -> Vec<String> {
        all(d)
            .into_iter()
            .map(|r| r.get(c).cloned().unwrap_or_default())
            .collect()
    }

    #[test]
    fn duplicates_show_and_remove() {
        let mut d = doc("k,v\na,1\nA,2\nb,3\na ,4\nb,5\nc,6\n");
        let p = Progress::new();
        let spec = Duplicates {
            cols: vec![0],
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_show_duplicates(&spec, &p).unwrap();
        assert_eq!((plan.groups, plan.rows), (2, 4));
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 1), ["v", "1", "2", "3", "5"]);
        d.clear_filter();
        let spec = Duplicates {
            cols: vec![0],
            trim: true,
            match_case: false,
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_remove_duplicates(&spec, &p).unwrap();
        assert_eq!((plan.groups, plan.rows), (2, 3));
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 1), ["v", "1", "3", "6"]);
        assert!(d.undo());
        assert_eq!(col(&d, 1), ["v", "1", "2", "3", "4", "5", "6"]);
        // Whole-row duplicates by default.
        let d2 = doc("x,y\n1,2\n1,3\n1,2\n");
        let plan = d2
            .plan_remove_duplicates(&Duplicates::default(), &p)
            .unwrap();
        assert_eq!(plan.rows, 1);
    }

    #[test]
    fn duplicates_with_a_filter_leave_hidden_rows() {
        let mut d = doc("k\na\nb\na\nb\n");
        d.set_filter(&FilterSpec {
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
        let p = Progress::new();
        let plan = d
            .plan_remove_duplicates(
                &Duplicates {
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        d.apply_rows(plan).unwrap();
        d.clear_filter();
        assert_eq!(col(&d, 0), ["k", "a", "b", "b"]);
    }

    #[test]
    fn blank_rows_and_columns() {
        let mut d = doc("a,,c,\n1,,3,\n,,,\n , ,,\n4,,6,\n");
        let p = Progress::new();
        let plan = d
            .plan_remove_blank_rows(
                &Blanks {
                    spaces: true,
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        assert_eq!(plan.rows, 2);
        d.apply_rows(plan).unwrap();
        assert_eq!(col(&d, 0), ["a", "1", "4"]);
        let plan = d
            .plan_remove_blank_cols(
                &Blanks {
                    spaces: false,
                    keep_first: false,
                },
                &p,
            )
            .unwrap();
        assert_eq!(plan.cols, 2);
        d.apply_cols(plan).unwrap();
        assert_eq!(all(&d), [["a", "c"], ["1", "3"], ["4", "6"]]);
        while d.undo() {}
        assert_eq!(d.row_count(), 5);
        assert_eq!(d.col_count(), 4);
    }

    #[test]
    fn split_and_merge_rows() {
        let mut d = doc("id,tags,n\n1,a;b;c,x\n2,d,y\n");
        let p = Progress::new();
        let spec = SplitRows {
            col: 1,
            separator: ";".into(),
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_split_rows(&spec, &p).unwrap();
        assert_eq!(plan.rows, 2);
        d.apply_rows(plan).unwrap();
        assert_eq!(
            all(&d),
            [
                ["id", "tags", "n"],
                ["1", "a", "x"],
                ["1", "b", "x"],
                ["1", "c", "x"],
                ["2", "d", "y"]
            ]
        );
        let spec = MergeRows {
            keys: vec![0],
            separator: "|".into(),
            distinct: true,
            match_case: true,
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_merge_rows(&spec, &p).unwrap();
        assert_eq!((plan.groups, plan.rows), (1, 2));
        d.apply_rows(plan).unwrap();
        assert_eq!(
            all(&d),
            [["id", "tags", "n"], ["1", "a|b|c", "x"], ["2", "d", "y"]]
        );
        assert!(d.undo());
        assert!(d.undo());
        assert_eq!(
            all(&d),
            [["id", "tags", "n"], ["1", "a;b;c", "x"], ["2", "d", "y"]]
        );
    }

    #[test]
    fn pivot_and_unpivot() {
        let d = doc("region,month,sales\nN,Jan,10\nS,Jan,5\nN,Feb,7\nN,Jan,1\nS,Feb,x\n");
        let p = Progress::new();
        let spec = Pivot {
            rows: vec![0],
            col: 1,
            value: 2,
            aggregate: Aggregate::Sum,
            keep_first: true,
            ..Default::default()
        };
        let t = d.pivot(&spec, &p).unwrap();
        assert_eq!(
            String::from_utf8(t.bytes).unwrap(),
            "region,Jan,Feb\r\nN,11,7\r\nS,5,\r\n"
        );
        let blanks = doc("k,c,v\n,x,1\n");
        let pv = Pivot {
            rows: vec![0],
            col: 1,
            value: 2,
            keep_first: true,
            ..Default::default()
        };
        let t = blanks.pivot(&pv, &p).unwrap();
        assert_eq!(String::from_utf8(t.bytes).unwrap(), "k,x\r\n(blank),1\r\n");
        let spec = Pivot {
            aggregate: Aggregate::Join,
            separator: ", ".into(),
            ..spec
        };
        let t = d.pivot(&spec, &p).unwrap();
        assert!(String::from_utf8(t.bytes)
            .unwrap()
            .contains("N,\"10, 1\",7"));
        let w = doc("id,a,b\n1,x,\n2,y,z\n");
        let spec = Unpivot {
            ids: vec![0],
            skip_blanks: true,
            keep_first: true,
            ..Default::default()
        };
        let t = w.unpivot(&spec, &p).unwrap();
        assert_eq!(t.rows, 4);
        let nd = t.into_document().unwrap();
        assert!(nd.is_modified() && nd.path().is_none());
        assert_eq!(
            all(&nd),
            [
                ["id", "Name", "Value"],
                ["1", "a", "x"],
                ["2", "a", "y"],
                ["2", "b", "z"]
            ]
        );
    }

    #[test]
    fn hash_collisions_never_merge_different_keys() {
        // Many distinct keys: every group must hold equal keys only.
        let mut s = String::from("k\n");
        for i in 0..20_000 {
            s.push_str(&format!("{}\n", i % 15_000));
        }
        let d = doc(&s);
        let p = Progress::new();
        let plan = d
            .plan_remove_duplicates(
                &Duplicates {
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        assert_eq!((plan.rows, plan.groups), (5_000, 5_000));
    }
}
