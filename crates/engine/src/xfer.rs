//! Whole columns and whole rows from one open document into another (cross-tab drag).
//!
//! The source is copied out under its own lock ([`Document::take_cols`], [`Document::take_rows`])
//! into one text buffer plus offsets (never a `String` per cell), then planned into the target
//! ([`Document::plan_insert_cols`], [`Document::plan_insert_rows`]) as a [`JoinPlan`], applied with
//! [`Document::apply_join`] as one undo step. The two documents are never locked together.

use std::collections::HashSet;

use serde::Serialize;

use crate::document::{Document, Progress};
use crate::join::JoinPlan;
use crate::Error;

fn too_big() -> Error {
    Error::Invalid("over 4 GB of text at once: copy fewer columns or rows".into())
}

/// Rows read between cancel checks.
const STEP: u64 = 4096;

/// Cells end to end: cell `i` is `text[ends[i - 1]..ends[i]]` (`0` before the first).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Packed {
    text: String,
    ends: Vec<u32>,
}

impl Packed {
    fn with_capacity(cells: usize) -> Packed {
        Packed {
            text: String::new(),
            ends: Vec::with_capacity(cells),
        }
    }
    /// False when the text would pass 4 GB (the offsets are 32-bit, to keep them small).
    fn push(&mut self, v: &str) -> bool {
        self.text.push_str(v);
        match u32::try_from(self.text.len()) {
            Ok(e) => {
                self.ends.push(e);
                true
            }
            Err(_) => false,
        }
    }
    pub fn len(&self) -> usize {
        self.ends.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }
    pub fn get(&self, i: usize) -> &str {
        let a = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        &self.text[a..self.ends[i] as usize]
    }
    /// Bytes held (text and offsets).
    pub fn bytes(&self) -> usize {
        self.text.capacity() + self.ends.capacity() * 4
    }
}

/// Whole columns copied out of a document: every row, hidden ones too, in file order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ColsOut {
    /// Titles (the first row, when it holds them; else empty).
    pub titles: Vec<String>,
    /// One per column: its data rows.
    pub cols: Vec<Packed>,
    /// Data rows, and those not shown (filtered out or hidden) among them.
    pub rows: u64,
    pub hidden: u64,
}

impl ColsOut {
    /// Columns from a grid of values (the clipboard text: rows of cells), with titles when known.
    pub fn from_grid(grid: &[Vec<String>], titles: Vec<String>) -> Result<ColsOut, Error> {
        let width = grid.iter().map(Vec::len).max().unwrap_or(0);
        let mut cols: Vec<Packed> = (0..width)
            .map(|_| Packed::with_capacity(grid.len()))
            .collect();
        for row in grid {
            for (k, col) in cols.iter_mut().enumerate() {
                if !col.push(row.get(k).map_or("", String::as_str)) {
                    return Err(too_big());
                }
            }
        }
        Ok(ColsOut {
            titles,
            cols,
            rows: grid.len() as u64,
            hidden: 0,
        })
    }
}

/// Whole rows copied out of a document: the shown rows asked for, all their columns.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RowsOut {
    pub titles: Vec<String>,
    /// Row `i`, cell `k`: `cells.get(i * width + k)`.
    pub cells: Packed,
    pub width: u32,
    pub rows: u64,
}

impl RowsOut {
    fn cell(&self, i: usize, k: usize) -> &str {
        self.cells.get(i * self.width as usize + k)
    }

    /// Rows from a grid of values (the clipboard text), by position; ragged rows are padded.
    pub fn from_grid(grid: &[Vec<String>]) -> Result<RowsOut, Error> {
        let width = grid.iter().map(Vec::len).max().unwrap_or(0) as u32;
        let mut cells = Packed::with_capacity(grid.len() * width as usize);
        for row in grid {
            for k in 0..width as usize {
                if !cells.push(row.get(k).map_or("", String::as_str)) {
                    return Err(too_big());
                }
            }
        }
        Ok(RowsOut {
            titles: Vec::new(),
            cells,
            width,
            rows: grid.len() as u64,
        })
    }
}

/// Where rows go in.
#[derive(Debug, Clone, Copy)]
enum RowPlace {
    After(u64),
    Before(u64),
    End,
}

/// What putting columns or rows into a document will do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct XferReport {
    /// Columns inserted.
    pub cols: u32,
    /// Data rows written (columns) or inserted (rows).
    pub rows: u64,
    /// Rows the target grew by (columns longer than the target).
    pub added_rows: u64,
    /// Columns the target grew by (rows wider than the target).
    pub added_cols: u32,
    /// Columns: the source's rows not shown there, carried along.
    pub hidden_carried: u64,
    /// Columns: the titles were left out (the target has no title row).
    pub title_left_out: bool,
    /// Rows: put under the matching titles (else by position).
    pub by_title: bool,
    /// The two tables have different numbers of data rows (columns line up by position).
    pub lengths_differ: bool,
}

/// How rows that are wider than the target fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Wider {
    /// New empty columns at the end of the target.
    #[default]
    AddColumns,
    /// The cells past the target's last column are left out.
    Cut,
}

impl Document {
    fn titles_row(&self, cols: &[u32]) -> Vec<String> {
        let Some(h) = self.doc_id_at(0) else {
            return vec![String::new(); cols.len()];
        };
        let phys: Vec<u32> = cols
            .iter()
            .map(|&c| self.phys(c).unwrap_or(u32::MAX))
            .collect();
        let mut buf = Vec::new();
        self.read_cells(h, &phys, &mut buf);
        buf.iter().map(|v| v.to_string()).collect()
    }

    /// Copy whole logical columns `cols` out: every data row in file order, hidden ones too.
    /// Cancellable through `progress`.
    pub fn take_cols(
        &self,
        cols: &[u32],
        keep_first: bool,
        progress: &Progress,
    ) -> Result<ColsOut, Error> {
        self.ready_for_bulk()?;
        if cols.is_empty() || cols.iter().any(|&c| c >= self.col_count()) {
            return Err(Error::OutOfRange);
        }
        let total = self.total_rows();
        let first = u64::from(keep_first && total > 0);
        let n = (total - first) as usize;
        let phys: Vec<u32> = cols
            .iter()
            .map(|&c| self.phys(c).ok_or(Error::OutOfRange))
            .collect::<Result<_, _>>()?;
        let mut out = ColsOut {
            titles: if keep_first {
                self.titles_row(cols)
            } else {
                Vec::new()
            },
            cols: (0..cols.len()).map(|_| Packed::with_capacity(n)).collect(),
            rows: n as u64,
            hidden: 0,
        };
        let filtered = self.is_filtered();
        progress.start(n as u64);
        let mut buf = Vec::new();
        for pos in first..total {
            if (pos - first) % STEP == STEP - 1 {
                progress.step(STEP)?;
            }
            let id = self.doc_id_at(pos).ok_or(Error::OutOfRange)?;
            if filtered && !self.is_shown(id) {
                out.hidden += 1;
            }
            self.read_cells(id, &phys, &mut buf);
            for (k, v) in buf.iter().enumerate() {
                if !out.cols[k].push(v) {
                    return Err(too_big());
                }
            }
        }
        progress.step(0)?;
        Ok(out)
    }

    /// Copy the shown rows at positions `rows` out (in the order given), every column.
    pub fn take_rows(
        &self,
        rows: &[u64],
        keep_first: bool,
        progress: &Progress,
    ) -> Result<RowsOut, Error> {
        self.ready_for_bulk()?;
        let width = self.col_count();
        let cols: Vec<u32> = (0..width).collect();
        let phys: Vec<u32> = cols
            .iter()
            .map(|&c| self.phys(c).ok_or(Error::OutOfRange))
            .collect::<Result<_, _>>()?;
        let mut out = RowsOut {
            titles: if keep_first {
                self.titles_row(&cols)
            } else {
                Vec::new()
            },
            cells: Packed::with_capacity(rows.len() * width as usize),
            width,
            rows: rows.len() as u64,
        };
        progress.start(rows.len() as u64);
        let mut buf = Vec::new();
        for (i, &r) in rows.iter().enumerate() {
            if i as u64 % STEP == STEP - 1 {
                progress.step(STEP)?;
            }
            if keep_first && r == 0 {
                return Err(Error::Invalid("the title row is not a data row".into()));
            }
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            self.read_cells(id, &phys, &mut buf);
            for v in &buf {
                if !out.cells.push(v) {
                    return Err(too_big());
                }
            }
        }
        progress.step(0)?;
        Ok(out)
    }

    /// Insert `data`'s columns at logical position `at`, lined up by position over every row
    /// (hidden ones too; the app refuses while a filter is on here). The title row takes their
    /// titles when this table has one; rows it lacks are added at the end.
    pub fn plan_insert_cols(
        &self,
        at: u32,
        data: &ColsOut,
        keep_first: bool,
        progress: &Progress,
    ) -> Result<(JoinPlan, XferReport), Error> {
        self.ready_for_bulk()?;
        let n = data.cols.len() as u32;
        if n == 0 || at > self.col_count() {
            return Err(Error::OutOfRange);
        }
        let total = self.total_rows();
        let first = u64::from(keep_first && total > 0);
        let mine = total - first;
        let mut plan = JoinPlan::new(self);
        plan.insert = Some((at, n));
        let dest: Vec<u32> = (at..at + n).collect();
        let mut rep = XferReport {
            cols: n,
            rows: data.rows,
            hidden_carried: data.hidden,
            title_left_out: !keep_first && data.titles.iter().any(|t| !t.is_empty()),
            lengths_differ: data.rows != mine,
            ..Default::default()
        };
        if keep_first {
            if let Some(h) = self.doc_id_at(0) {
                let cells: Vec<(u32, String)> = dest
                    .iter()
                    .zip(&data.titles)
                    .filter(|(_, t)| !t.is_empty())
                    .map(|(&c, t)| (c, t.clone()))
                    .collect();
                if !cells.is_empty() {
                    plan.changes.push((h, cells));
                }
            }
        }
        progress.start(data.rows);
        let common = data.rows.min(mine);
        for i in 0..common {
            if i % STEP == STEP - 1 {
                progress.step(STEP)?;
            }
            let id = self.doc_id_at(first + i).ok_or(Error::OutOfRange)?;
            let cells: Vec<(u32, String)> = dest
                .iter()
                .enumerate()
                .filter_map(|(k, &c)| {
                    let v = data.cols[k].get(i as usize);
                    (!v.is_empty()).then(|| (c, v.to_string()))
                })
                .collect();
            if !cells.is_empty() {
                plan.cells += cells.len() as u64;
                plan.changes.push((id, cells));
            }
        }
        // Rows this table lacks: new rows holding only the new columns.
        let width = (self.col_count() + n) as usize;
        for i in common..data.rows {
            if i % STEP == STEP - 1 {
                progress.step(STEP)?;
            }
            let mut row = vec![String::new(); width];
            for (k, &c) in dest.iter().enumerate() {
                let v = data.cols[k].get(i as usize);
                if !v.is_empty() {
                    plan.cells += 1;
                    row[c as usize] = v.to_string();
                }
            }
            plan.append.push(row);
            rep.added_rows += 1;
        }
        progress.step(0)?;
        Ok((plan, rep))
    }

    /// Whether `data`'s titles are this table's, in another order: the same set (exactly, or
    /// ignoring case), none twice and no two the same apart from case on either side. Then
    /// `Some(dest)`: where each of its columns goes here.
    pub fn rows_by_title(&self, data: &RowsOut) -> Option<Vec<u32>> {
        let mine = self.titles_row(&(0..self.col_count()).collect::<Vec<_>>());
        let theirs = &data.titles;
        if mine.len() != theirs.len() || mine.iter().any(String::is_empty) {
            return None;
        }
        let unique = |v: &[String]| {
            let mut seen = HashSet::new();
            v.iter().all(|t| seen.insert(t.to_lowercase()))
        };
        if !unique(&mine) || !unique(theirs) {
            return None;
        }
        let find = |t: &str| -> Option<u32> {
            mine.iter()
                .position(|m| m == t)
                .or_else(|| {
                    mine.iter()
                        .position(|m| m.to_lowercase() == t.to_lowercase())
                })
                .map(|p| p as u32)
        };
        let dest: Option<Vec<u32>> = theirs.iter().map(|t| find(t)).collect();
        let dest = dest?;
        // Only worth saying "by title" when the order differs.
        Some(dest)
    }

    /// Insert `data`'s rows after the shown row at position `after` (`None`: at the end),
    /// by title when [`Self::rows_by_title`] says so and `by_title`, else by position.
    pub fn plan_insert_rows(
        &self,
        after: Option<u64>,
        data: &RowsOut,
        by_title: bool,
        wider: Wider,
        progress: &Progress,
    ) -> Result<(JoinPlan, XferReport), Error> {
        let place = after.map_or(RowPlace::End, RowPlace::After);
        self.plan_rows_at(place, data, by_title, wider, progress)
    }

    fn plan_rows_at(
        &self,
        place: RowPlace,
        data: &RowsOut,
        by_title: bool,
        wider: Wider,
        progress: &Progress,
    ) -> Result<(JoinPlan, XferReport), Error> {
        self.ready_for_bulk()?;
        let mine = self.col_count();
        let by = if by_title {
            self.rows_by_title(data)
        } else {
            None
        };
        let mut rep = XferReport {
            rows: data.rows,
            by_title: by
                .as_ref()
                .is_some_and(|d| d.iter().enumerate().any(|(k, &c)| c != k as u32)),
            ..Default::default()
        };
        let mut plan = JoinPlan::new(self);
        let extra = data.width.saturating_sub(mine);
        let (dest, width) = match by {
            Some(d) => (d, mine),
            None if extra > 0 && wider == Wider::AddColumns => {
                plan.insert = Some((mine, extra));
                rep.added_cols = extra;
                ((0..data.width).collect(), data.width)
            }
            None => ((0..data.width.min(mine)).collect(), mine),
        };
        progress.start(data.rows);
        let mut rows = Vec::with_capacity(data.rows as usize);
        for i in 0..data.rows as usize {
            if i as u64 % STEP == STEP - 1 {
                progress.step(STEP)?;
            }
            let mut row = vec![String::new(); width as usize];
            for (k, &c) in dest.iter().enumerate() {
                let v = data.cell(i, k);
                if !v.is_empty() {
                    plan.cells += 1;
                    row[c as usize] = v.to_string();
                }
            }
            rows.push(row);
        }
        progress.step(0)?;
        match place {
            RowPlace::After(r) => {
                let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                plan.after.push((id, rows));
            }
            RowPlace::Before(r) => {
                let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                plan.before.push((id, rows));
            }
            RowPlace::End => plan.append = rows,
        }
        Ok((plan, rep))
    }

    /// Insert `data`'s rows right above the shown row `before` (Insert copied rows), by position.
    pub fn plan_insert_rows_above(
        &self,
        before: u64,
        data: &RowsOut,
        wider: Wider,
        progress: &Progress,
    ) -> Result<(JoinPlan, XferReport), Error> {
        self.plan_rows_at(RowPlace::Before(before), data, false, wider, progress)
    }
}

#[cfg(test)]
#[path = "xfer_tests.rs"]
mod tests;
