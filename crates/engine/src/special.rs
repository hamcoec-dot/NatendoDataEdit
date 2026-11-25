//! Go To Special: the cells of an area that are blank, hold numbers, hold
//! text, or differ from another column of their row. Shown rows only; the
//! list is capped so a whole-table pick on a huge file stays bounded.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::{CellPlan, Cols, CHUNK, MAX_BLOCK_BYTES, MAX_BLOCK_CELLS, MAX_CELL_BYTES};
use crate::validate::CheckSpec;
use crate::value::{self, ReadOptions};
use crate::Error;

/// Most cells one Go To Special pick lists (as many as the window lights
/// up; Fill and Clear work on every one in the engine).
pub const MAX_SPECIAL_CELLS: usize = 100_000;

/// Which cells to pick.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpecialKind {
    /// Empty, or only spaces.
    #[default]
    Blanks,
    /// A number (as numbers are read for this file).
    Numbers,
    /// Not blank and not a number.
    Text,
    /// Different from the cell of column `against` in the same row (the
    /// column itself is left out). `exact`: case and spaces count.
    RowDiff { against: u32, exact: bool },
    /// Changed since the file was opened or last saved.
    Edited,
    /// Failing one of these data checks (a row check picks every asked
    /// cell of its row).
    Problems { checks: CheckSpec },
}

/// What to look through.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpecialSpec {
    pub kind: SpecialKind,
    /// First shown row and how many.
    pub row: u64,
    pub rows: u64,
    pub cols: Vec<u32>,
    #[serde(default)]
    pub read: ReadOptions,
    /// For [`SpecialKind::Problems`]: rows whose value repeats, per unique
    /// check (from a check run; filled in by the caller).
    #[serde(skip)]
    pub repeats: Vec<(u32, Vec<u64>)>,
}

/// The cells picked, in row order then column order.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SpecialCells {
    /// (shown row, column), at most [`MAX_SPECIAL_CELLS`].
    pub cells: Vec<(u64, u32)>,
    /// Every matching cell, listed or not.
    pub total: u64,
}

enum Kind {
    Cells,
    Marks,
}

fn kind_of(k: &SpecialKind) -> Kind {
    match k {
        SpecialKind::Edited | SpecialKind::Problems { .. } => Kind::Marks,
        _ => Kind::Cells,
    }
}

fn blank(v: &str) -> bool {
    v.trim().is_empty()
}

fn same_loose(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    a == b || norm(a) == norm(b)
}

/// The matching cells of one stretch of rows: (shown row, row id, columns).
type Found = Vec<(u64, u32, Vec<u32>)>;

impl Document {
    /// Every matching cell of the area by row, and how many there are; stops
    /// listing (but keeps counting) past `cap` cells.
    fn special_scan(
        &self,
        spec: &SpecialSpec,
        cap: u64,
        progress: &Progress,
    ) -> Result<(Found, u64), Error> {
        self.ready_for_bulk()?;
        let ncols = self.col_count();
        let mut cols: Vec<u32> = spec.cols.iter().copied().filter(|&c| c < ncols).collect();
        cols.sort_unstable();
        cols.dedup();
        let against = match spec.kind {
            SpecialKind::RowDiff { against, .. } => {
                if against >= ncols {
                    return Err(Error::Invalid("pick a column to compare with".into()));
                }
                cols.retain(|&c| c != against);
                Some(against)
            }
            _ => None,
        };
        let start = spec.row.min(self.row_count());
        let end = spec.row.saturating_add(spec.rows).min(self.row_count());
        if cols.is_empty() || start >= end {
            return Ok((Vec::new(), 0));
        }
        if matches!(kind_of(&spec.kind), Kind::Marks) {
            return self.special_marks(spec, &cols, start..end, cap, progress);
        }
        let mut read_cols = cols.clone();
        if let Some(a) = against {
            read_cols.push(a);
        }
        let reader = Cols::new(self, &read_cols);
        let n = end - start;
        let kind = &spec.kind;
        let read = spec.read;
        progress.start(n);
        let nchunks = (n as usize).div_ceil(CHUNK);
        let group = rayon::current_num_threads().max(1) * 4;
        let mut found: Found = Vec::new();
        let mut total = 0u64;
        let mut listed = 0u64;
        for g in (0..nchunks).step_by(group) {
            let list = listed < cap;
            let parts: Vec<(Found, u64)> = (g..(g + group).min(nchunks))
                .into_par_iter()
                .map(|c| -> Result<(Found, u64), Error> {
                    let s = start + (c * CHUNK) as u64;
                    let e = (s + CHUNK as u64).min(end);
                    let mut out = Vec::new();
                    let mut total = 0u64;
                    let mut buf = Vec::new();
                    for r in s..e {
                        let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                        reader.read(self, id, &mut buf);
                        let other = against.map(|_| reader.get(&buf, cols.len()));
                        let mut hits = Vec::new();
                        for (k, &col) in cols.iter().enumerate() {
                            let v = reader.get(&buf, k);
                            let hit = match kind {
                                SpecialKind::Blanks => blank(v),
                                SpecialKind::Numbers => value::parse_number(v, read).is_some(),
                                SpecialKind::Text => {
                                    !blank(v) && value::parse_number(v, read).is_none()
                                }
                                SpecialKind::RowDiff { exact, .. } => {
                                    let o = other.unwrap_or("");
                                    if *exact {
                                        v != o
                                    } else {
                                        !same_loose(v, o)
                                    }
                                }
                                SpecialKind::Edited | SpecialKind::Problems { .. } => false,
                            };
                            if hit {
                                hits.push(col);
                            }
                        }
                        if !hits.is_empty() {
                            total += hits.len() as u64;
                            if list {
                                out.push((r, id, hits));
                            }
                        }
                    }
                    progress.step(e - s)?;
                    Ok((out, total))
                })
                .collect::<Result<_, _>>()?;
            for (rows, t) in parts {
                total += t;
                for row in rows {
                    if listed >= cap {
                        break;
                    }
                    listed += row.2.len() as u64;
                    found.push(row);
                }
            }
        }
        Ok((found, total))
    }

    /// Edited or failing cells of the area, from the change list or a
    /// check run.
    fn special_marks(
        &self,
        spec: &SpecialSpec,
        cols: &[u32],
        rows: std::ops::Range<u64>,
        cap: u64,
        progress: &Progress,
    ) -> Result<(Found, u64), Error> {
        let (lo, hi) = (cols[0], cols[cols.len() - 1]);
        let mut marks: Vec<(u64, Option<u32>)> = match &spec.kind {
            SpecialKind::Problems { checks } => {
                // Unique checks need to know which values repeat: a check run
                // works that out when the caller has no recent one.
                let run;
                let repeats = if spec.repeats.is_empty()
                    && checks
                        .checks
                        .iter()
                        .any(|c| matches!(c.kind, crate::validate::CheckKind::Unique { .. }))
                {
                    run = self.run_checks(checks, progress)?.repeats;
                    &run
                } else {
                    &spec.repeats
                };
                self.problem_marks(rows, checks, repeats, progress)?
                    .into_iter()
                    .map(|m| (m.row, m.col))
                    .collect()
            }
            _ => self
                .edited_cells(rows, lo..hi + 1)?
                .into_iter()
                .map(|(r, c)| (r, Some(c)))
                .collect(),
        };
        marks.sort_unstable();
        let mut found: Found = Vec::new();
        let mut total = 0u64;
        for (r, c) in marks {
            let picked: Vec<u32> = match c {
                Some(c) if cols.binary_search(&c).is_ok() => vec![c],
                Some(_) => continue,
                None => cols.to_vec(),
            };
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            match found.last_mut() {
                Some(last) if last.0 == r => {
                    for c in picked {
                        if !last.2.contains(&c) {
                            last.2.push(c);
                            total += 1;
                        }
                    }
                    last.2.sort_unstable();
                }
                _ => {
                    total += picked.len() as u64;
                    found.push((r, id, picked));
                }
            }
        }
        // Listed rows past the cap are dropped (the count stays whole).
        let mut listed = 0u64;
        found.retain(|f| {
            let keep = listed < cap;
            listed += f.2.len() as u64;
            keep
        });
        Ok((found, total))
    }

    /// The cells of the area that are of `spec.kind`.
    pub fn special_cells(
        &self,
        spec: &SpecialSpec,
        progress: &Progress,
    ) -> Result<SpecialCells, Error> {
        let (found, total) = self.special_scan(spec, MAX_SPECIAL_CELLS as u64, progress)?;
        let mut cells = Vec::new();
        'all: for (r, _, cols) in found {
            for c in cols {
                if cells.len() >= MAX_SPECIAL_CELLS {
                    break 'all;
                }
                cells.push((r, c));
            }
        }
        Ok(SpecialCells { cells, total })
    }

    /// Plan putting `value` in every cell Go To Special picks (an empty
    /// value clears them), one undo step. At most [`MAX_BLOCK_CELLS`].
    pub fn plan_fill_special(
        &self,
        spec: &SpecialSpec,
        value: &str,
        progress: &Progress,
    ) -> Result<CellPlan, Error> {
        if value.len() > MAX_CELL_BYTES {
            return Err(Error::Invalid("the text is too long".into()));
        }
        // Protected columns are left out, so the count is what changes.
        let mut spec = spec.clone();
        let picked = spec.cols.len();
        spec.cols.retain(|&c| !self.is_protected(c));
        if picked > 0 && spec.cols.is_empty() {
            return Err(Error::Invalid(
                "those columns are protected, so nothing changed (right-click a heading › Protect column to switch it off)".into(),
            ));
        }
        let (found, total) = self.special_scan(&spec, MAX_BLOCK_CELLS + 1, progress)?;
        if total > MAX_BLOCK_CELLS {
            return Err(Error::Invalid(format!(
                "{total} cells were found; one fill changes at most {MAX_BLOCK_CELLS}. Pick fewer columns or rows."
            )));
        }
        if (value.len() as u64).saturating_mul(total) > MAX_BLOCK_BYTES as u64 {
            return Err(Error::Invalid(
                "this would create too much text at once".into(),
            ));
        }
        let mut plan = CellPlan::new(self);
        let (first, last) = (found.first().map(|f| f.0), found.last().map(|f| f.0));
        plan.changes = found
            .into_iter()
            .map(|(_, id, cols)| {
                (
                    id,
                    cols.into_iter().map(|c| (c, value.to_string())).collect(),
                )
            })
            .collect();
        plan.cells = total;
        if let (Some(a), Some(b)) = (first, last) {
            plan.hidden_rows = self.hidden_between(a..b + 1);
        }
        Ok(plan)
    }
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

    fn pick(d: &Document, kind: SpecialKind, cols: Vec<u32>) -> SpecialCells {
        let spec = SpecialSpec {
            kind,
            row: 1,
            rows: u64::MAX,
            cols,
            ..Default::default()
        };
        d.special_cells(&spec, &Progress::default()).unwrap()
    }

    #[test]
    fn picks_by_kind() {
        let d = doc("a,b,c\n1,x, \n,2.5,$3\nfoo,,7\n");
        let b = pick(&d, SpecialKind::Blanks, vec![0, 1, 2]);
        assert_eq!(b.cells, vec![(1, 2), (2, 0), (3, 1)]);
        assert_eq!(b.total, 3);
        let n = pick(&d, SpecialKind::Numbers, vec![0, 1, 2]);
        assert_eq!(n.cells, vec![(1, 0), (2, 1), (2, 2), (3, 2)]);
        let t = pick(&d, SpecialKind::Text, vec![0, 1, 2]);
        assert_eq!(t.cells, vec![(1, 1), (3, 0)]);
        // Only the asked columns, and nothing past the table.
        assert_eq!(
            pick(&d, SpecialKind::Blanks, vec![1, 9]).cells,
            vec![(3, 1)]
        );
    }

    #[test]
    fn row_differences() {
        let d = doc("a,b,c\nx,x,X\ny,Y ,y\nz,q,z\n");
        let loose = pick(
            &d,
            SpecialKind::RowDiff {
                against: 0,
                exact: false,
            },
            vec![0, 1, 2],
        );
        assert_eq!(loose.cells, vec![(3, 1)]);
        let exact = pick(
            &d,
            SpecialKind::RowDiff {
                against: 0,
                exact: true,
            },
            vec![0, 1, 2],
        );
        assert_eq!(exact.cells, vec![(1, 2), (2, 1), (3, 1)]);
        let spec = SpecialSpec {
            kind: SpecialKind::RowDiff {
                against: 7,
                exact: true,
            },
            row: 0,
            rows: 9,
            cols: vec![0],
            ..Default::default()
        };
        assert!(d.special_cells(&spec, &Progress::default()).is_err());
    }

    #[test]
    fn edited_and_failing_cells() {
        use crate::validate::{Check, CheckKind};
        let mut d = doc("a,b\n1,x\n2,y\nq,z\n");
        d.set_cells(vec![(2, 1, "Y".into())]).unwrap();
        let e = pick(&d, SpecialKind::Edited, vec![0, 1]);
        assert_eq!(e.cells, vec![(2, 1)]);
        assert!(pick(&d, SpecialKind::Edited, vec![0]).cells.is_empty());
        let checks = CheckSpec {
            checks: vec![Check {
                name: None,
                col: Some(0),
                kind: CheckKind::Number {
                    min: None,
                    max: None,
                    whole: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        };
        let p = pick(&d, SpecialKind::Problems { checks }, vec![0, 1]);
        assert_eq!(p.cells, vec![(3, 0)]);
    }

    /// Protected columns: every change passes them over (and says how many
    /// cells it left); a change only of protected cells is refused; a rename
    /// still works; protection follows a moved column.
    #[test]
    fn protected_columns_are_passed_over() {
        let mut d = doc("id,name\n1,a\n2,\n");
        d.set_protected(&[0]);
        assert_eq!(d.protected_cols(), vec![0]);
        assert!(d.set_cell(1, 0, "9".into()).is_err());
        assert_eq!(d.cell(1, 0).unwrap(), "1");
        d.set_cells(vec![(1, 0, "9".into()), (1, 1, "b".into())])
            .unwrap();
        assert_eq!(d.protected_skipped(), 1);
        assert_eq!(d.row_values(1).unwrap(), ["1", "b"]);
        // A bulk fill of both columns changes only the other one.
        let spec = SpecialSpec {
            kind: SpecialKind::Text,
            row: 1,
            rows: 9,
            cols: vec![0, 1],
            ..Default::default()
        };
        let plan = d
            .plan_fill_special(&spec, "X", &Progress::default())
            .unwrap();
        d.apply_cells(plan).unwrap();
        assert_eq!(d.row_values(1).unwrap(), ["1", "X"]);
        // Only protected cells: refused, nothing changes.
        let spec = SpecialSpec {
            kind: SpecialKind::Numbers,
            row: 1,
            rows: 9,
            cols: vec![0],
            ..Default::default()
        };
        assert!(d
            .plan_fill_special(&spec, "X", &Progress::default())
            .is_err());
        // A rename of the protected column works.
        d.set_first_row_cell(0, "voter_id".into()).unwrap();
        assert_eq!(d.cell(0, 0).unwrap(), "voter_id");
        // Moved to the end, the column stays protected.
        d.move_cols(0, 1, 1).unwrap();
        assert_eq!(d.protected_cols(), vec![1]);
        assert!(d.set_cell(1, 1, "7".into()).is_err());
        d.set_protected(&[]);
        d.set_cell(1, 1, "7".into()).unwrap();
        assert_eq!(d.protected_skipped(), 0);
    }

    #[test]
    fn fill_plan() {
        let mut d = doc("a,b\n1,\n,x\n,\n");
        let spec = SpecialSpec {
            kind: SpecialKind::Blanks,
            row: 1,
            rows: 9,
            cols: vec![0, 1],
            ..Default::default()
        };
        let plan = d
            .plan_fill_special(&spec, "UNKNOWN", &Progress::default())
            .unwrap();
        assert_eq!(plan.cells, 4);
        d.apply_cells(plan).unwrap();
        let all: Vec<Vec<String>> = (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect();
        assert_eq!(
            all,
            [
                ["a", "b"],
                ["1", "UNKNOWN"],
                ["UNKNOWN", "x"],
                ["UNKNOWN", "UNKNOWN"]
            ]
        );
        d.undo();
        assert_eq!(d.cell(3, 1).unwrap(), "");
    }

    #[test]
    fn hidden_rows_are_left_out() {
        let mut d = doc("a\n\nx\n\ny\n");
        d.hide_rows(1, 1).unwrap();
        // Shown rows: a, x, (blank), y.
        assert_eq!(pick(&d, SpecialKind::Blanks, vec![0]).cells, vec![(2, 0)]);
    }
}
