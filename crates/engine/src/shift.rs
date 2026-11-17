//! Filling a block of cells with one value (clearing it, or Ctrl+Enter) on
//! any number of rows, and Excel's Insert and Delete cells: shifting the
//! cells below a block up or down, or those to its right left or right.
//! Each is one undo step.

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::ops::{CellPlan, ColPlan, Cols, MAX_BLOCK_BYTES, MAX_BLOCK_CELLS};
use crate::{Error, Progress};

/// Rows above a cleared block copied across when whole columns are swapped
/// for empty ones (the header row, usually); more than this, and the block
/// is cleared cell by cell.
const MAX_KEPT_ABOVE: u64 = 1_000;

/// Put one value into every cell of a block (shown rows).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RectFill {
    pub row: u64,
    pub rows: u64,
    pub col: u32,
    pub cols: u32,
    pub value: String,
    /// Columns in the block left as they are (hidden ones).
    #[serde(default)]
    pub skip_cols: Vec<u32>,
}

/// Which way the other cells move.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shift {
    /// Delete the block; the cells below move up.
    #[default]
    Up,
    /// Delete the block; the cells to its right move left.
    Left,
    /// Insert empty cells; the cells from the block down move down.
    Down,
    /// Insert empty cells; the cells from the block right move right.
    Right,
}

/// Insert or delete a block of cells (shown rows; no filter may be on).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShiftCells {
    pub row: u64,
    pub rows: u64,
    pub col: u32,
    pub cols: u32,
    pub shift: Shift,
}

fn check_block(d: &Document, row: u64, rows: u64, col: u32, cols: u32) -> Result<(), Error> {
    let r1 = row.checked_add(rows).ok_or(Error::OutOfRange)?;
    let c1 = col.checked_add(cols).ok_or(Error::OutOfRange)?;
    if rows == 0 || cols == 0 || r1 > d.row_count() || c1 > d.col_count() {
        return Err(Error::OutOfRange);
    }
    Ok(())
}

/// `n` with thousands separators (5,000,000).
fn commas(n: u64) -> String {
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

/// Over the cell cap: say how many, and what to do instead.
fn too_many(fill: bool) -> Error {
    let how = if fill {
        "To clear more, select whole columns down to the last row with no filter on: they clear at once, however many rows there are."
    } else {
        "Insert or delete fewer cells, or entire rows or columns instead."
    };
    Error::Invalid(format!(
        "At most {} cells can change at once. {how}",
        commas(MAX_BLOCK_CELLS)
    ))
}

impl Document {
    /// Clearing whole columns from `row` to the end, with no filter or hidden
    /// rows: swap the columns for new empty ones (keeping the cells above
    /// the block), which is quick and small however many rows there are.
    /// `None` when the block is not like that.
    pub fn plan_clear_columns(&self, spec: &RectFill) -> Result<Option<ColPlan>, Error> {
        check_block(self, spec.row, spec.rows, spec.col, spec.cols)?;
        if !spec.value.is_empty()
            || self.is_filtered()
            || spec
                .skip_cols
                .iter()
                .any(|c| (spec.col..spec.col + spec.cols).contains(c))
            || spec.row + spec.rows != self.row_count()
            || spec.row > MAX_KEPT_ABOVE
        {
            return Ok(None);
        }
        self.ready_for_bulk()?;
        let mut plan = ColPlan::new(self);
        plan.insert = Some((spec.col, spec.cols));
        let cols: Vec<u32> = (spec.col..spec.col + spec.cols).collect();
        let reader = Cols::new(self, &cols);
        let mut buf = Vec::new();
        for r in 0..spec.row {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let cells: Vec<(u32, String)> = (0..cols.len())
                .map(|k| (spec.col + k as u32, reader.get(&buf, k).to_string()))
                .filter(|(_, v)| !v.is_empty())
                .collect();
            if !cells.is_empty() {
                plan.changes.push((id, cells));
            }
        }
        plan.remove = vec![(spec.col + spec.cols, spec.cols)];
        plan.swap = true;
        plan.cells = spec.rows.saturating_mul(spec.cols as u64);
        Ok(Some(plan))
    }

    /// Put `spec.value` into every cell of the block, cell by cell.
    pub fn plan_fill_rect(&self, spec: &RectFill, progress: &Progress) -> Result<CellPlan, Error> {
        check_block(self, spec.row, spec.rows, spec.col, spec.cols)?;
        self.ready_for_bulk()?;
        let n = spec.rows.saturating_mul(spec.cols as u64);
        if n > MAX_BLOCK_CELLS {
            return Err(too_many(true));
        }
        if (spec.value.len() as u64).saturating_mul(n) > MAX_BLOCK_BYTES as u64 {
            return Err(Error::Invalid(
                "this would create too much text at once".into(),
            ));
        }
        let mut plan = CellPlan::new(self);
        let value = &spec.value;
        let cols: Vec<u32> = (spec.col..spec.col + spec.cols)
            .filter(|c| !spec.skip_cols.contains(c))
            .collect();
        if cols.is_empty() {
            return Ok(plan);
        }
        plan.changes = self.par_rows(spec.row..spec.row + spec.rows, progress, |_, id, _| {
            Ok(Some((
                id,
                cols.iter().map(|&c| (c, value.clone())).collect(),
            )))
        })?;
        plan.cells = spec.rows * cols.len() as u64;
        plan.hidden_rows = self.hidden_between(spec.row..spec.row + spec.rows);
        Ok(plan)
    }

    /// Insert or delete a block of cells, moving the others (Excel's Insert
    /// and Delete cells, "shift cells up / left / down / right").
    pub fn plan_shift_cells(
        &self,
        spec: &ShiftCells,
        progress: &Progress,
    ) -> Result<CellPlan, Error> {
        check_block(self, spec.row, spec.rows, spec.col, spec.cols)?;
        self.ready_for_bulk()?;
        if self.is_filtered() {
            return Err(Error::Invalid(
                "cells can't be shifted while a filter is on or rows are hidden; show every row first"
                    .into(),
            ));
        }
        let n = self.row_count();
        let ncols = self.col_count();
        let mut plan = CellPlan::new(self);
        match spec.shift {
            Shift::Up | Shift::Down => {
                let cols: Vec<u32> = (spec.col..spec.col + spec.cols).collect();
                if (n - spec.row).saturating_mul(spec.cols as u64) > MAX_BLOCK_CELLS {
                    return Err(too_many(false));
                }
                let reader = Cols::new(self, &cols);
                // Old values from the block's top row down, by row.
                let old: Vec<(u32, Vec<String>)> =
                    self.par_rows(spec.row..n, progress, |_, id, buf| {
                        reader.read(self, id, buf);
                        Ok(Some((
                            id,
                            (0..cols.len())
                                .map(|k| reader.get(buf, k).to_string())
                                .collect(),
                        )))
                    })?;
                let k = spec.rows as usize;
                let len = old.len();
                let blank = String::new();
                let value = |i: isize, c: usize| -> &str {
                    if i < 0 || i as usize >= len {
                        &blank
                    } else {
                        &old[i as usize].1[c]
                    }
                };
                for (i, (id, was)) in old.iter().enumerate() {
                    let from = if spec.shift == Shift::Up {
                        i as isize + k as isize
                    } else {
                        i as isize - k as isize
                    };
                    let cells: Vec<(u32, String)> = (0..cols.len())
                        .filter(|&c| value(from, c) != was[c])
                        .map(|c| (cols[c], value(from, c).to_string()))
                        .collect();
                    if !cells.is_empty() {
                        plan.cells += cells.len() as u64;
                        plan.changes.push((*id, cells));
                    }
                }
                if spec.shift == Shift::Down {
                    // Values pushed past the last row get new rows (only as
                    // many as hold something).
                    let tail = &old[len.saturating_sub(k)..];
                    let keep = tail
                        .iter()
                        .rposition(|(_, v)| v.iter().any(|s| !s.is_empty()))
                        .map_or(0, |p| p + 1);
                    for (_, v) in tail.iter().take(keep) {
                        let mut row = vec![String::new(); (spec.col + spec.cols) as usize];
                        for (c, s) in v.iter().enumerate() {
                            row[(spec.col as usize) + c] = s.clone();
                        }
                        plan.cells += v.iter().filter(|s| !s.is_empty()).count() as u64;
                        plan.append.push(row);
                    }
                }
            }
            Shift::Left | Shift::Right => {
                let cols: Vec<u32> = (spec.col..ncols).collect();
                if spec
                    .rows
                    .saturating_mul(cols.len() as u64 + spec.cols as u64)
                    > MAX_BLOCK_CELLS
                {
                    return Err(too_many(false));
                }
                let reader = Cols::new(self, &cols);
                let k = spec.cols as usize;
                let rows: Vec<(u32, Vec<(u32, String)>)> =
                    self.par_rows(spec.row..spec.row + spec.rows, progress, |_, id, buf| {
                        reader.read(self, id, buf);
                        let was: Vec<&str> = (0..cols.len()).map(|c| reader.get(buf, c)).collect();
                        let w = was.len();
                        let mut cells = Vec::new();
                        if spec.shift == Shift::Left {
                            for c in 0..w {
                                let v = if c + k < w { was[c + k] } else { "" };
                                if v != was[c] {
                                    cells.push((spec.col + c as u32, v.to_string()));
                                }
                            }
                        } else {
                            for c in 0..w + k {
                                let v = if c >= k { was[c - k] } else { "" };
                                let now = if c < w { was[c] } else { "" };
                                if v != now {
                                    cells.push((spec.col + c as u32, v.to_string()));
                                }
                            }
                        }
                        Ok((!cells.is_empty()).then_some((id, cells)))
                    })?;
                plan.cells = rows.iter().map(|r| r.1.len() as u64).sum();
                plan.changes = rows;
            }
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn all(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    fn shift(d: &mut Document, row: u64, rows: u64, col: u32, cols: u32, s: Shift) {
        let spec = ShiftCells {
            row,
            rows,
            col,
            cols,
            shift: s,
        };
        let plan = d.plan_shift_cells(&spec, &Progress::new()).unwrap();
        d.apply_cells(plan).unwrap();
    }

    const T: &str = "a,b,c\r\n1,2,3\r\n4,5,6\r\n7,8,9\r\n";

    #[test]
    fn delete_cells_shift_up_and_left() {
        let mut d = doc(T);
        shift(&mut d, 1, 1, 1, 1, Shift::Up);
        assert_eq!(
            all(&d),
            [
                ["a", "b", "c"],
                ["1", "5", "3"],
                ["4", "8", "6"],
                ["7", "", "9"]
            ]
        );
        assert!(d.undo());
        assert_eq!(all(&d), all(&doc(T)));
        shift(&mut d, 1, 2, 0, 1, Shift::Left);
        assert_eq!(
            all(&d),
            [
                ["a", "b", "c"],
                ["2", "3", ""],
                ["5", "6", ""],
                ["7", "8", "9"]
            ]
        );
    }

    #[test]
    fn insert_cells_shift_down_and_right() {
        let mut d = doc(T);
        shift(&mut d, 2, 1, 2, 1, Shift::Down);
        // 9 is pushed into a new row; the other columns stay put.
        assert_eq!(
            all(&d),
            [
                vec!["a", "b", "c"],
                vec!["1", "2", "3"],
                vec!["4", "5", ""],
                vec!["7", "8", "6"],
                vec!["", "", "9"],
            ]
        );
        assert!(d.undo());
        assert_eq!(all(&d), all(&doc(T)));
        shift(&mut d, 1, 1, 0, 2, Shift::Right);
        assert_eq!(all(&d)[1], ["", "", "1", "2", "3"]);
        assert_eq!(d.col_count(), 5, "the row grew two columns");
        assert!(d.undo());
        assert_eq!(all(&d), all(&doc(T)));
    }

    #[test]
    fn inserting_blank_tails_adds_no_rows() {
        let mut d = doc("a,b\r\n1,\r\n2,\r\n");
        shift(&mut d, 1, 1, 1, 1, Shift::Down);
        assert_eq!(d.row_count(), 3, "only blanks fell off the end");
    }

    #[test]
    fn shifting_refuses_with_a_filter() {
        let mut d = doc(T);
        d.hide_rows(2, 1).unwrap();
        let spec = ShiftCells {
            row: 1,
            rows: 1,
            col: 0,
            cols: 1,
            shift: Shift::Up,
        };
        assert!(matches!(
            d.plan_shift_cells(&spec, &Progress::new()),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn the_cap_message_has_commas_and_a_way_forward() {
        assert_eq!(commas(5_000_000), "5,000,000");
        assert_eq!(commas(999), "999");
        let Error::Invalid(m) = too_many(true) else {
            panic!("invalid")
        };
        assert!(m.starts_with("At most 5,000,000 cells") && m.contains("whole columns"));
    }

    #[test]
    fn a_quick_clear_lists_as_cleared() {
        let mut d = doc(T);
        let spec = RectFill {
            row: 1,
            rows: 3,
            col: 1,
            cols: 1,
            value: String::new(),
            skip_cols: Vec::new(),
        };
        let plan = d.plan_clear_columns(&spec).unwrap().unwrap();
        d.apply_cols(plan).unwrap();
        let l = d.change_list(&Progress::new()).unwrap();
        assert_eq!(l.cleared_cols, vec![1]);
        assert!(l.added_cols.is_empty() && l.deleted_cols.is_empty() && !l.moved_cols);
        assert_eq!(l.cells_total, 0, "the header cell is not an edit");
        assert_eq!(
            d.edited_cells(0..4, 0..3).unwrap(),
            vec![(1, 1), (2, 1), (3, 1)]
        );
        assert!(d.undo());
        assert!(d.change_list(&Progress::new()).unwrap().is_empty());
        assert!(d.redo());
        assert_eq!(
            d.change_list(&Progress::new()).unwrap().cleared_cols,
            vec![1]
        );
        // Typing into the cleared column, then clearing it again: still one
        // cleared column, and its cells are not listed one by one.
        d.set_cell(2, 1, "typed".into()).unwrap();
        let plan = d.plan_clear_columns(&spec).unwrap().unwrap();
        d.apply_cols(plan).unwrap();
        let l = d.change_list(&Progress::new()).unwrap();
        assert_eq!(l.cleared_cols, vec![1]);
        assert!(l.added_cols.is_empty() && l.deleted_cols.is_empty());
        assert_eq!(l.cells_total, 0);
        assert_eq!(
            d.edited_cells(0..4, 0..3).unwrap(),
            vec![(1, 1), (2, 1), (3, 1)]
        );
    }

    #[test]
    fn clearing_whole_columns_is_quick_and_undoes() {
        let mut d = doc(T);
        let spec = RectFill {
            row: 1,
            rows: 3,
            col: 1,
            cols: 2,
            value: String::new(),
            skip_cols: Vec::new(),
        };
        let plan = d.plan_clear_columns(&spec).unwrap().expect("whole columns");
        d.apply_cols(plan).unwrap();
        assert_eq!(
            all(&d),
            [["a", "b", "c"], ["1", "", ""], ["4", "", ""], ["7", "", ""]]
        );
        assert_eq!(d.col_count(), 3);
        assert!(d.undo());
        assert_eq!(all(&d), all(&doc(T)));
        // Part of the rows, a value, or a filter: cell by cell instead.
        let part = RectFill {
            rows: 2,
            ..spec.clone()
        };
        assert!(d.plan_clear_columns(&part).unwrap().is_none());
        let fill = RectFill {
            value: "x".into(),
            ..spec.clone()
        };
        assert!(d.plan_clear_columns(&fill).unwrap().is_none());
        let plan = d.plan_fill_rect(&fill, &Progress::new()).unwrap();
        d.apply_cells(plan).unwrap();
        assert_eq!(all(&d)[3], ["7", "x", "x"]);
    }
}
