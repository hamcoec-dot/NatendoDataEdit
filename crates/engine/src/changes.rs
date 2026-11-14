//! "List of my changes" and "show my edits" (Milestone 14): what changed
//! since the file was opened or last saved, cell by cell, plus rows and
//! columns added, deleted, moved or sorted.

use std::collections::HashSet;
use std::io::Write;
use std::ops::Range;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::Error;

/// Most cell changes listed (the count goes on past it).
pub const MAX_LISTED: usize = 100_000;
/// Most added or deleted rows listed by number.
const MAX_ROWS_LISTED: usize = 10_000;

/// One changed cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellChange {
    /// Its row now (0-based position in the whole table, filter or not).
    pub row: u64,
    /// The row it was in the file (0-based), for rows the file had.
    pub file_row: u64,
    /// Its column now (logical).
    pub col: u32,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeList {
    /// The first [`MAX_LISTED`] changed cells, in table order.
    pub cells: Vec<CellChange>,
    /// Changed cells in all.
    pub cells_total: u64,
    /// Rows added (their positions now), the first ones.
    pub added_rows: Vec<u64>,
    pub added_total: u64,
    /// Rows of the file deleted (their row in the file), the first ones.
    pub deleted_rows: Vec<u64>,
    pub deleted_total: u64,
    /// Columns added (their position now).
    pub added_cols: Vec<u32>,
    /// Columns of the file deleted (their position in the file).
    pub deleted_cols: Vec<u32>,
    /// Columns are in another order than in the file.
    pub moved_cols: bool,
    /// Rows are in another order than in the file (sorted or moved).
    pub reordered: bool,
}

impl ChangeList {
    pub fn is_empty(&self) -> bool {
        self.cells_total == 0
            && self.added_total == 0
            && self.deleted_total == 0
            && self.added_cols.is_empty()
            && self.deleted_cols.is_empty()
            && !self.moved_cols
            && !self.reordered
    }
}

/// Edited rows compared per batch (bounds what one batch holds).
const BATCH_ROWS: usize = 16_384;
/// Most rows listed by number in a report (the count goes on past it).
const REPORT_ROWS_LISTED: usize = 1_000_000;

impl Document {
    /// What changed since the file was opened or last saved: the first
    /// [`MAX_LISTED`] changed cells and every count. Memory stays small
    /// however many cells changed (the rest are only counted).
    pub fn change_list(&self, progress: &Progress) -> Result<ChangeList, Error> {
        let mut cells: Vec<CellChange> = Vec::new();
        let mut list = self.walk_changes(progress, MAX_ROWS_LISTED, |batch, _| {
            let room = MAX_LISTED.saturating_sub(cells.len());
            cells.extend(batch.iter().take(room).cloned());
            Ok(cells.len() < MAX_LISTED)
        })?;
        list.cells = cells;
        Ok(list)
    }

    /// Rows, columns and the counts; each batch of changed cells (in table
    /// order) goes to `sink`, which says whether it wants more cells (when
    /// not, the rest are only counted, without copying their values).
    fn walk_changes(
        &self,
        progress: &Progress,
        rows_listed: usize,
        mut sink: impl FnMut(&[CellChange], &Progress) -> Result<bool, Error>,
    ) -> Result<ChangeList, Error> {
        self.ready_for_bulk()?;
        let original = self.original_rows();
        let width = self.original_width();
        let map = self.logical_to_physical();
        let mut list = ChangeList::default();
        // Columns.
        let present: HashSet<u32> = map.iter().copied().collect();
        list.deleted_cols = (0..width).filter(|p| !present.contains(p)).collect();
        list.added_cols = map
            .iter()
            .enumerate()
            .filter(|(_, &p)| p >= width)
            .map(|(c, _)| c as u32)
            .collect();
        let kept: Vec<u32> = map.iter().copied().filter(|&p| p < width).collect();
        list.moved_cols = kept.windows(2).any(|w| w[0] > w[1]);
        // Rows: one pass in table order.
        let edited: HashSet<u32> = self.edited_ids().into_iter().collect();
        let mut seen = vec![0u64; (original as usize).div_ceil(64)];
        let mut last: Option<u32> = None;
        let mut changed: Vec<(u64, u32)> = Vec::new();
        progress.start(self.total_rows() + edited.len() as u64);
        for (pos, id) in self.ids().enumerate() {
            if u64::from(id) < original {
                seen[(id >> 6) as usize] |= 1 << (id & 63);
                if last.is_some_and(|l| id < l) {
                    list.reordered = true;
                }
                last = Some(id);
                if edited.contains(&id) {
                    changed.push((pos as u64, id));
                }
            } else {
                list.added_total += 1;
                if list.added_rows.len() < rows_listed {
                    list.added_rows.push(pos as u64);
                }
            }
            if pos % 65_536 == 65_535 {
                progress.step(65_536)?;
            }
        }
        drop(edited);
        for id in 0..original as u32 {
            if seen[(id >> 6) as usize] & (1 << (id & 63)) == 0 {
                list.deleted_total += 1;
                if list.deleted_rows.len() < rows_listed {
                    list.deleted_rows.push(u64::from(id));
                }
            }
        }
        drop(seen);
        // Cells of edited original rows: batches in order, each compared in
        // parallel. Once the sink wants no more, values are not copied.
        let mut want = true;
        for batch in changed.chunks(BATCH_ROWS) {
            let parts: Vec<(Vec<CellChange>, u64)> = batch
                .par_chunks(512)
                .map(|part| -> Result<(Vec<CellChange>, u64), Error> {
                    if progress.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err(Error::Cancelled);
                    }
                    let mut out = Vec::new();
                    let mut count = 0u64;
                    for &(pos, id) in part {
                        let old = self.original_values(id)?;
                        let new = self.physical_values(id)?;
                        for (c, &p) in map.iter().enumerate() {
                            let o = old.get(p as usize).map_or("", String::as_str);
                            let n = new.get(p as usize).map_or("", String::as_str);
                            if o != n {
                                count += 1;
                                if want {
                                    out.push(CellChange {
                                        row: pos,
                                        file_row: u64::from(id),
                                        col: c as u32,
                                        old: o.to_string(),
                                        new: n.to_string(),
                                    });
                                }
                            }
                        }
                    }
                    Ok((out, count))
                })
                .collect::<Result<_, _>>()?;
            for (cells, count) in parts {
                list.cells_total += count;
                if want && !cells.is_empty() {
                    want = sink(&cells, progress)?;
                }
            }
            progress.step(batch.len() as u64)?;
        }
        Ok(list)
    }

    /// Shown cells in `rows` x `cols` that differ from the file: (shown
    /// row, column). Added rows count wholly where they hold something.
    pub fn edited_cells(
        &self,
        rows: Range<u64>,
        cols: Range<u32>,
    ) -> Result<Vec<(u64, u32)>, Error> {
        let original = self.original_rows();
        let map = self.logical_to_physical();
        let c1 = cols.end.min(map.len() as u32);
        let mut out = Vec::new();
        for r in rows.start..rows.end.min(self.row_count()) {
            let Some(id) = self.id_at(r) else { break };
            if u64::from(id) >= original {
                let new = self.physical_values(id)?;
                for c in cols.start..c1 {
                    if new
                        .get(map[c as usize] as usize)
                        .is_some_and(|v| !v.is_empty())
                    {
                        out.push((r, c));
                    }
                }
                continue;
            }
            if !self.has_edit(id) {
                // Columns added since: anything typed in them is an edit,
                // but an unedited row has none.
                continue;
            }
            let old = self.original_values(id)?;
            let new = self.physical_values(id)?;
            for c in cols.start..c1 {
                let p = map[c as usize] as usize;
                if old.get(p).map_or("", String::as_str) != new.get(p).map_or("", String::as_str) {
                    out.push((r, c));
                }
            }
        }
        Ok(out)
    }

    /// The changes as a CSV report: Change, Row now, Row in the file,
    /// Column, Old value, New value. `titles` names the columns. Every
    /// changed cell is written as it is found (nothing kept in memory);
    /// rows added or deleted are listed up to a million each. Returns the
    /// list (without its cells).
    pub fn write_change_report(
        &self,
        out: &mut impl Write,
        titles: &[String],
        progress: &Progress,
    ) -> Result<ChangeList, Error> {
        let d = crate::dialect::Dialect::CSV;
        let mut line = String::new();
        let mut row = |fields: &[&str], out: &mut dyn Write| -> Result<(), Error> {
            line.clear();
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    line.push(',');
                }
                crate::scan::write_field(f, d, &mut line);
            }
            line.push_str("\r\n");
            out.write_all(line.as_bytes())?;
            Ok(())
        };
        let title = |c: u32| {
            titles
                .get(c as usize)
                .cloned()
                .unwrap_or_else(|| format!("Column {}", c + 1))
        };
        row(
            &[
                "Change",
                "Row now",
                "Row in the file",
                "Column",
                "Old value",
                "New value",
            ],
            out,
        )?;
        let list = self.walk_changes(progress, REPORT_ROWS_LISTED, |cells, _| {
            for c in cells {
                row(
                    &[
                        "Cell changed",
                        &(c.row + 1).to_string(),
                        &(c.file_row + 1).to_string(),
                        &title(c.col),
                        &c.old,
                        &c.new,
                    ],
                    out,
                )?;
            }
            Ok(true)
        })?;
        for r in &list.added_rows {
            row(&["Row added", &(r + 1).to_string(), "", "", "", ""], out)?;
        }
        for r in &list.deleted_rows {
            row(&["Row deleted", "", &(r + 1).to_string(), "", "", ""], out)?;
        }
        for c in &list.added_cols {
            row(&["Column added", "", "", &title(*c), "", ""], out)?;
        }
        for c in &list.deleted_cols {
            row(
                &[
                    "Column deleted",
                    "",
                    "",
                    &format!("Column {} of the file", c + 1),
                    "",
                    "",
                ],
                out,
            )?;
        }
        if list.moved_cols {
            row(&["Columns moved", "", "", "", "", ""], out)?;
        }
        if list.reordered {
            row(&["Rows sorted or moved", "", "", "", "", ""], out)?;
        }
        let more = list
            .added_total
            .saturating_sub(list.added_rows.len() as u64)
            + list
                .deleted_total
                .saturating_sub(list.deleted_rows.len() as u64);
        if more > 0 {
            row(
                &[
                    &format!("{more} more rows added or deleted, not listed"),
                    "",
                    "",
                    "",
                    "",
                    "",
                ],
                out,
            )?;
        }
        Ok(list)
    }

    /// Write the CSV report to a file (a temp file moved into place).
    pub fn save_change_report(
        &self,
        path: &std::path::Path,
        titles: &[String],
        progress: &Progress,
    ) -> Result<ChangeList, Error> {
        if self.path().is_some_and(|p| crate::save::same_path(p, path)) {
            return Err(Error::Invalid(
                "choose another name: that is the open file".into(),
            ));
        }
        let (temp, file) = crate::save::create_temp(path)?;
        let done = (|| -> Result<ChangeList, Error> {
            let mut w = std::io::BufWriter::with_capacity(1 << 20, &file);
            let l = self.write_change_report(&mut w, titles, progress)?;
            w.flush()?;
            file.sync_all()?;
            Ok(l)
        })();
        drop(file);
        match done {
            Ok(l) => {
                if let Err(e) = crate::save::replace(&temp, path) {
                    let _ = std::fs::remove_file(&temp);
                    return Err(e.into());
                }
                Ok(l)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    #[test]
    fn lists_cells_rows_and_columns() {
        let mut d = doc("a,b,c\r\n1,2,3\r\n4,5,6\r\n7,8,9\r\n");
        let p = Progress::new();
        assert!(d.change_list(&p).unwrap().is_empty());
        d.set_cells(vec![(1, 1, "two".into()), (3, 2, "nine".into())])
            .unwrap();
        let l = d.change_list(&p).unwrap();
        assert_eq!(l.cells_total, 2);
        assert_eq!(
            (
                l.cells[0].row,
                l.cells[0].col,
                l.cells[0].old.as_str(),
                l.cells[0].new.as_str()
            ),
            (1, 1, "2", "two")
        );
        // Shown edits.
        assert_eq!(d.edited_cells(0..4, 0..3).unwrap(), vec![(1, 1), (3, 2)]);
        // Setting a cell back to what the file has is no change.
        d.set_cells(vec![(1, 1, "2".into())]).unwrap();
        assert_eq!(d.change_list(&p).unwrap().cells_total, 1);
        // Rows and columns.
        d.delete_rows(2, 1).unwrap();
        d.insert_rows(1, vec![vec!["x".into(), "y".into(), "z".into()]])
            .unwrap();
        d.insert_cols(3, 1).unwrap();
        d.delete_cols(0, 1).unwrap();
        let l = d.change_list(&p).unwrap();
        assert_eq!((l.added_total, l.added_rows.clone()), (1, vec![1]));
        assert_eq!((l.deleted_total, l.deleted_rows.clone()), (1, vec![2]));
        assert_eq!(l.deleted_cols, vec![0]);
        assert_eq!(l.added_cols, vec![2]);
        assert!(!l.moved_cols && !l.reordered);
        // The nine is now in column 1 (b, c, new).
        assert!(l.cells.iter().any(|c| c.col == 1 && c.new == "nine"));
        let mut out = Vec::new();
        d.write_change_report(&mut out, &["b".into(), "c".into(), "new".into()], &p)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("Change,Row now,Row in the file,Column,Old value,New value\r\n"));
        assert!(text.contains("Cell changed,4,4,c,9,nine\r\n"), "{text}");
        assert!(text.contains("Row deleted,,3,,,\r\n"));
        assert!(text.contains("Column deleted,,,Column 1 of the file,,\r\n"));
    }

    #[test]
    fn sorting_is_noticed() {
        let mut d = doc("n\r\n3\r\n1\r\n2\r\n");
        d.sort(&crate::sort::SortSpec {
            keys: vec![crate::sort::SortKey {
                col: 0,
                descending: false,
                kind: Default::default(),
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        let l = d.change_list(&Progress::new()).unwrap();
        assert!(l.reordered);
        assert_eq!(l.cells_total, 0);
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use crate::OpenOptions;

    #[test]
    fn many_changes_are_counted_not_kept() {
        let mut body = String::from("a,b\r\n");
        for i in 0..60_000 {
            body.push_str(&format!("{i},{i}\r\n"));
        }
        let mut d =
            Document::from_bytes(body.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let edits: Vec<(u64, u32, String)> = (1..=60_000u64)
            .flat_map(|r| [(r, 0, "x".to_string()), (r, 1, "y".to_string())])
            .collect();
        d.set_cells(edits).unwrap();
        let p = Progress::new();
        let l = d.change_list(&p).unwrap();
        assert_eq!(l.cells_total, 120_000);
        assert_eq!(l.cells.len(), MAX_LISTED);
        assert_eq!((l.cells[0].row, l.cells[0].col), (1, 0));
        // The report has every one.
        let mut out = Vec::new();
        d.write_change_report(&mut out, &[], &p).unwrap();
        let lines = out
            .split(|&b| b == b'\n')
            .filter(|l| l.starts_with(b"Cell changed"))
            .count();
        assert_eq!(lines, 120_000);
        // Stopped.
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(d.change_list(&p), Err(Error::Cancelled)));
    }
}
