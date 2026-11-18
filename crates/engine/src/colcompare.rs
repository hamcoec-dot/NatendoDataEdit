//! Data › Compare two columns: for each shown row, are the two cells the
//! same, different, or is only one filled? Mark the rows that differ, show
//! only them, or add a column saying which.

use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress, View};
use crate::filter::FilterSpec;
use crate::ops::{ColPlan, Cols, CHUNK};
use crate::reshape::RowPlan;
use crate::value::{self, ReadOptions};
use crate::Error;

/// Most rows listed for marking.
pub const MAX_COMPARE_ROWS: usize = 5_000_000;

/// The two columns and how values are compared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompareCols {
    pub a: u32,
    pub b: u32,
    /// Upper and lower case count as different.
    pub case: bool,
    /// Spaces count (otherwise ends are trimmed and runs made one).
    pub spaces: bool,
    /// Numbers are compared by value (`5`, `5.00`, `$5` are the same).
    pub numbers: bool,
    #[serde(default)]
    pub read: ReadOptions,
    /// The first row holds titles (left out, and kept shown).
    pub keep_first: bool,
    /// Title of the added column (default "A vs B").
    pub title: String,
}

impl Default for CompareCols {
    fn default() -> Self {
        CompareCols {
            a: 0,
            b: 1,
            case: false,
            spaces: false,
            numbers: true,
            read: ReadOptions::default(),
            keep_first: true,
            title: String::new(),
        }
    }
}

/// One row's result: shown row, row id, how its two cells compare.
type Each = (u64, u32, Cmp);

/// How two cells compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Same,
    Different,
    OnlyFirst,
    OnlySecond,
}

impl Cmp {
    pub fn label(self) -> &'static str {
        match self {
            Cmp::Same => "Same",
            Cmp::Different => "Different",
            Cmp::OnlyFirst => "Only in first",
            Cmp::OnlySecond => "Only in second",
        }
    }
}

fn norm(s: &str, spec: &CompareCols) -> String {
    let t = if spec.spaces {
        s.to_string()
    } else {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    if spec.case {
        t
    } else {
        t.to_lowercase()
    }
}

/// Compare two cells.
pub fn compare_cells(a: &str, b: &str, spec: &CompareCols) -> Cmp {
    let blank = |s: &str| {
        if spec.spaces {
            s.is_empty()
        } else {
            s.trim().is_empty()
        }
    };
    match (blank(a), blank(b)) {
        (true, true) => return Cmp::Same,
        (false, true) => return Cmp::OnlyFirst,
        (true, false) => return Cmp::OnlySecond,
        _ => {}
    }
    if a == b {
        return Cmp::Same;
    }
    if spec.numbers {
        if let (Some(x), Some(y)) = (
            value::parse_number(a, spec.read),
            value::parse_number(b, spec.read),
        ) {
            return if x == y { Cmp::Same } else { Cmp::Different };
        }
    }
    if norm(a, spec) == norm(b, spec) {
        Cmp::Same
    } else {
        Cmp::Different
    }
}

/// What a comparison found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompareColsReport {
    pub same: u64,
    pub different: u64,
    pub only_first: u64,
    pub only_second: u64,
    /// Shown rows (not the title row) where the cells are not the same, at
    /// most [`MAX_COMPARE_ROWS`].
    pub rows: Vec<u64>,
}

impl Document {
    fn check_compare(&self, spec: &CompareCols) -> Result<(), Error> {
        self.ready_for_bulk()?;
        let n = self.col_count();
        if spec.a >= n || spec.b >= n {
            return Err(Error::Invalid("pick two columns of this table".into()));
        }
        if spec.a == spec.b {
            return Err(Error::Invalid("pick two different columns".into()));
        }
        Ok(())
    }

    /// Each shown data row's result, by row id (in row order), in parallel.
    fn compare_each(
        &self,
        spec: &CompareCols,
        progress: &Progress,
    ) -> Result<Vec<Vec<Each>>, Error> {
        self.check_compare(spec)?;
        let reader = Cols::new(self, &[spec.a, spec.b]);
        let rows = self.data_rows(spec.keep_first);
        let n = rows.end - rows.start;
        progress.start(n);
        (0..(n as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<Vec<Each>, Error> {
                let s = rows.start + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(rows.end);
                let mut out = Vec::with_capacity((e - s) as usize);
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    out.push((
                        r,
                        id,
                        compare_cells(reader.get(&buf, 0), reader.get(&buf, 1), spec),
                    ));
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect()
    }

    /// Count same and different rows, and list the rows that differ.
    pub fn compare_cols(
        &self,
        spec: &CompareCols,
        progress: &Progress,
    ) -> Result<CompareColsReport, Error> {
        let parts = self.compare_each(spec, progress)?;
        let mut rep = CompareColsReport::default();
        for part in parts {
            for (r, _, k) in part {
                match k {
                    Cmp::Same => rep.same += 1,
                    Cmp::Different => rep.different += 1,
                    Cmp::OnlyFirst => rep.only_first += 1,
                    Cmp::OnlySecond => rep.only_second += 1,
                }
                if k != Cmp::Same && rep.rows.len() < MAX_COMPARE_ROWS {
                    rep.rows.push(r);
                }
            }
        }
        Ok(rep)
    }

    /// Show only the shown rows whose two cells are not the same (the title
    /// row stays).
    pub fn plan_compare_show(
        &self,
        spec: &CompareCols,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        let parts = self.compare_each(spec, progress)?;
        let base = self.next_row_id();
        let mut bits = vec![0u64; (base as usize).div_ceil(64)];
        let mut shown = 0u64;
        for part in parts {
            for (_, id, k) in part {
                if k != Cmp::Same {
                    bits[(id >> 6) as usize] |= 1 << (id & 63);
                    shown += 1;
                }
            }
        }
        let keep_first = spec.keep_first && self.row_count() > 0;
        if keep_first {
            let id = self.id_at(0).expect("a first row");
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        }
        let mut plan = RowPlan::new(self);
        plan.rows = shown;
        plan.view = Some(View::new(
            FilterSpec {
                keep_first,
                ..Default::default()
            },
            bits,
            base,
        ));
        Ok(plan)
    }

    /// A new column right of the two saying Same, Different, Only in first
    /// or Only in second for each shown row.
    pub fn plan_compare_column(
        &self,
        spec: &CompareCols,
        progress: &Progress,
    ) -> Result<ColPlan, Error> {
        let parts = self.compare_each(spec, progress)?;
        let at = spec.a.max(spec.b) + 1;
        let spent = AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        let mut plan = ColPlan::new(self);
        plan.insert = Some((at, 1));
        plan.cols = 1;
        if spec.keep_first && self.row_count() > 0 {
            let title = if spec.title.trim().is_empty() {
                let name = |c: u32| {
                    self.cell(0, c)
                        .filter(|t| !t.trim().is_empty())
                        .unwrap_or_else(|| format!("Column {}", c + 1))
                };
                format!("{} vs {}", name(spec.a), name(spec.b))
            } else {
                spec.title.clone()
            };
            plan.changes
                .push((self.id_at(0).expect("a first row"), vec![(at, title)]));
        }
        for part in parts {
            for (_, id, k) in part {
                if spent.fetch_add(48, Ordering::Relaxed) > budget {
                    return Err(Error::OutOfMemory(
                        "this would change too many cells at once; use fewer rows",
                    ));
                }
                plan.changes.push((id, vec![(at, k.label().to_string())]));
            }
        }
        plan.cells = plan.changes.len() as u64;
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

    #[test]
    fn compares_cells() {
        let s = CompareCols::default();
        assert_eq!(compare_cells("Main St", "main  st ", &s), Cmp::Same);
        assert_eq!(compare_cells("5", "$5.00", &s), Cmp::Same);
        assert_eq!(compare_cells("x", "", &s), Cmp::OnlyFirst);
        assert_eq!(compare_cells(" ", "y", &s), Cmp::OnlySecond);
        assert_eq!(compare_cells("", "  ", &s), Cmp::Same);
        let strict = CompareCols {
            case: true,
            spaces: true,
            numbers: false,
            ..Default::default()
        };
        assert_eq!(compare_cells("Main St", "main st", &strict), Cmp::Different);
        assert_eq!(compare_cells("5", "5.00", &strict), Cmp::Different);
        assert_eq!(compare_cells(" ", "", &strict), Cmp::OnlyFirst);
    }

    #[test]
    fn report_show_and_column() {
        let mut d = doc("mail,res\n1 Main St,1 MAIN ST\n2 Oak,3 Oak\n,4 Elm\n5 Pine,\n");
        let spec = CompareCols::default();
        let rep = d.compare_cols(&spec, &Progress::default()).unwrap();
        assert_eq!(
            (rep.same, rep.different, rep.only_first, rep.only_second),
            (1, 1, 1, 1)
        );
        assert_eq!(rep.rows, vec![2, 3, 4]);
        let col = d.plan_compare_column(&spec, &Progress::default()).unwrap();
        d.apply_cols(col).unwrap();
        let c: Vec<String> = (0..d.row_count()).map(|r| d.cell(r, 2).unwrap()).collect();
        assert_eq!(
            c,
            [
                "mail vs res",
                "Same",
                "Different",
                "Only in second",
                "Only in first"
            ]
        );
        d.undo();
        let show = d.plan_compare_show(&spec, &Progress::default()).unwrap();
        d.apply_rows(show).unwrap();
        assert_eq!(d.row_count(), 4);
        assert_eq!(d.cell(1, 0).unwrap(), "2 Oak");
        assert!(d
            .compare_cols(
                &CompareCols {
                    a: 0,
                    b: 0,
                    ..Default::default()
                },
                &Progress::default()
            )
            .is_err());
    }
}
