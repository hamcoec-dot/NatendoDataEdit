//! Milestone 9 row tools: quick totals of a selection, hiding and showing
//! rows, and shuffling rows.
//!
//! - Totals read only the selected cells of shown rows, in parallel chunks,
//!   and stop as soon as the caller cancels (a newer selection), so a huge
//!   selection never holds the document for long.
//! - Hidden rows are a bit set over row ids kept with the filter view (see
//!   `document::View`). They are still saved; hiding and showing are one
//!   undo step each.
//! - Shuffling permutes the shown rows of a range among themselves (rows a
//!   filter or hiding leaves out keep their places), as one undo step.

use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::areas::Area;
use crate::document::{Document, Progress};
use crate::ops::Cols;
use crate::stats::Kahan;
use crate::value::{parse_number, ReadOptions};
use crate::Error;

/// Rows per parallel task (and between cancel checks).
const CHUNK: u64 = 8_192;
/// Most columns totalled at once (a selection wider than this is refused,
/// so one request cannot ask for unbounded work per row).
pub const MAX_TOTAL_COLS: u32 = 16_384;

/// Quick totals of a selection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Totals {
    /// Cells looked at (shown rows x columns).
    pub cells: u64,
    /// Cells that are not blank.
    pub filled: u64,
    /// Cells holding a number.
    pub numbers: u64,
    pub sum: Option<f64>,
    pub average: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The sum (and so the average) is past the largest number a double
    /// holds (about 1.8e308): `sum` and `average` are then empty, and the
    /// UI says "too large" instead of acting as if no numbers were there.
    pub too_large: bool,
    /// Characters in the cells (as written, spaces too).
    pub chars: u64,
}

/// Most distinct values counted exactly by [`Document::selection_distinct`].
pub const MAX_DISTINCT: usize = 2_000_000;

#[derive(Clone, Copy)]
struct Acc {
    chars: u64,
    cells: u64,
    filled: u64,
    numbers: u64,
    sum: Kahan,
    min: f64,
    max: f64,
}

impl Acc {
    fn new() -> Acc {
        Acc {
            chars: 0,
            cells: 0,
            filled: 0,
            numbers: 0,
            sum: Kahan::default(),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }

    fn merge(mut self, o: Acc) -> Acc {
        self.chars += o.chars;
        self.cells += o.cells;
        self.filled += o.filled;
        self.numbers += o.numbers;
        self.sum.merge(&o.sum);
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        self
    }
}

/// The totals of what was read; columns past the last one count as blank
/// cells (`cells` is what was asked for).
fn totals_of(acc: Acc, cells: u64) -> Totals {
    let has = acc.numbers > 0;
    let sum = acc.sum.value();
    let fits = sum.is_finite();
    Totals {
        cells,
        filled: acc.filled,
        numbers: acc.numbers,
        sum: (has && fits).then_some(sum),
        average: (has && fits).then(|| sum / acc.numbers as f64),
        too_large: has && !fits,
        min: has.then_some(acc.min),
        max: has.then_some(acc.max),
        chars: acc.chars,
    }
}

/// How rows are shuffled: a computed new order (see [`Document::apply_shuffle`]).
pub struct ShufflePlan {
    revision: u64,
    order: Vec<u32>,
    /// Rows that took part.
    pub rows: u64,
}

/// SplitMix64 step (not for secrets: a shuffle only needs to look random).
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A number below `n` (n > 0), by multiply-shift.
fn below(state: &mut u64, n: usize) -> usize {
    ((next(state) as u128 * n as u128) >> 64) as usize
}

/// What a "show rows" request covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowRows {
    /// Every hidden row.
    All,
    /// Hidden rows between the first and last of these shown rows (from the
    /// top when it starts at the first row, to the end when it reaches the
    /// last), as Excel's Unhide does.
    Around { at: u64, count: u64 },
}

impl Document {
    /// Shown rows `at..at + count` as document positions.
    fn shown_positions(&self, at: u64, count: u64) -> Result<Vec<u32>, Error> {
        let end = at.checked_add(count).ok_or(Error::OutOfRange)?;
        if end > self.row_count() {
            return Err(Error::OutOfRange);
        }
        Ok(match self.view() {
            Some(v) => self.visible(v)[at as usize..end as usize].to_vec(),
            None => (at as u32..end as u32).collect(),
        })
    }

    /// Sum, count, average, min and max of the cells in shown rows
    /// `row..row + rows` and columns `col..col + cols`. Cancelled through
    /// `progress`.
    pub fn selection_totals(
        &self,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Totals, Error> {
        let acc = self.totals_acc(row, rows, col, cols, read, progress)?;
        Ok(totals_of(acc, rows.saturating_mul(cols as u64)))
    }

    /// [`Document::selection_totals`] over several blocks (Ctrl+click): the
    /// blocks are made non-overlapping first, so a cell counts once, and all
    /// of them are read in one go.
    pub fn selection_totals_areas(
        &self,
        areas: &[Area],
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Totals, Error> {
        let parts = crate::areas::disjoint(areas);
        let n = self.col_count();
        // Blocks over the same rows are read in one pass (their columns together).
        let mut by_rows: std::collections::BTreeMap<(u64, u64), Vec<u32>> =
            std::collections::BTreeMap::new();
        let mut cells = 0u64;
        for a in &parts {
            cells = cells.saturating_add(a.cells());
            if a.cols > MAX_TOTAL_COLS {
                return Err(Error::OutOfRange);
            }
            let c1 = a.col.saturating_add(a.cols).min(n);
            by_rows
                .entry((a.row, a.rows))
                .or_default()
                .extend(a.col.min(c1)..c1);
        }
        let mut acc = Acc::new();
        for ((row, rows), mut cols) in by_rows {
            cols.sort_unstable();
            cols.dedup();
            acc = acc.merge(self.totals_acc_cols(row, rows, &cols, read, progress)?);
        }
        Ok(totals_of(acc, cells))
    }

    fn totals_acc(
        &self,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Acc, Error> {
        if cols > MAX_TOTAL_COLS {
            return Err(Error::OutOfRange);
        }
        let n = self.col_count();
        let c1 = col.saturating_add(cols).min(n);
        let wanted: Vec<u32> = (col.min(c1)..c1).collect();
        self.totals_acc_cols(row, rows, &wanted, read, progress)
    }

    /// The totals of shown rows `row..row + rows` in exactly these columns
    /// (one pass over the rows, however many columns).
    fn totals_acc_cols(
        &self,
        row: u64,
        rows: u64,
        wanted: &[u32],
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Acc, Error> {
        let end = row.checked_add(rows).ok_or(Error::OutOfRange)?;
        if end > self.row_count() || wanted.len() as u64 > u64::from(MAX_TOTAL_COLS) {
            return Err(Error::OutOfRange);
        }
        let reader = Cols::new(self, wanted);
        progress.start(rows);
        let chunks = rows.div_ceil(CHUNK);
        let acc = (0..chunks)
            .into_par_iter()
            .map(|k| -> Result<Acc, Error> {
                let s = row + k * CHUNK;
                let e = (s + CHUNK).min(end);
                let mut a = Acc::new();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for i in 0..wanted.len() {
                        let v = reader.get(&buf, i);
                        a.chars += v.chars().count() as u64;
                        let v = v.trim();
                        if v.is_empty() {
                            continue;
                        }
                        a.filled += 1;
                        if let Some(x) = parse_number(v, read) {
                            a.numbers += 1;
                            a.sum.add(x);
                            a.min = a.min.min(x);
                            a.max = a.max.max(x);
                        }
                    }
                }
                progress.step(e - s)?;
                Ok(a)
            })
            .try_reduce(Acc::new, |a, b| Ok(a.merge(b)))?;
        Ok(acc)
    }

    /// How many different non-blank values (spaces at the ends ignored) the
    /// same cells hold, and whether that count is exact: past
    /// [`MAX_DISTINCT`] values it stops and says "at least".
    pub fn selection_distinct(
        &self,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        progress: &Progress,
    ) -> Result<(u64, bool), Error> {
        use std::hash::{BuildHasher, Hasher};
        let end = row.checked_add(rows).ok_or(Error::OutOfRange)?;
        if end > self.row_count() || cols > MAX_TOTAL_COLS {
            return Err(Error::OutOfRange);
        }
        let n = self.col_count();
        let c1 = col.saturating_add(cols).min(n);
        let wanted: Vec<u32> = (col.min(c1)..c1).collect();
        let reader = Cols::new(self, &wanted);
        let rs = std::collections::hash_map::RandomState::new();
        let full = std::sync::atomic::AtomicBool::new(false);
        progress.start(rows);
        let sets: Vec<std::collections::HashSet<u64>> = (0..rows.div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<std::collections::HashSet<u64>, Error> {
                let s = row + k * CHUNK;
                let e = (s + CHUNK).min(end);
                let mut set = std::collections::HashSet::new();
                if full.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(set);
                }
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for i in 0..wanted.len() {
                        let v = reader.get(&buf, i).trim();
                        if !v.is_empty() {
                            let mut h = rs.build_hasher();
                            h.write(v.as_bytes());
                            set.insert(h.finish());
                        }
                    }
                }
                if set.len() > MAX_DISTINCT {
                    full.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                progress.step(e - s)?;
                Ok(set)
            })
            .collect::<Result<_, _>>()?;
        let mut all = std::collections::HashSet::new();
        for s in sets {
            all.extend(s);
            if all.len() > MAX_DISTINCT {
                return Ok((MAX_DISTINCT as u64, false));
            }
        }
        if full.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok((all.len() as u64, false));
        }
        Ok((all.len() as u64, true))
    }

    /// Plan putting shown rows `at..at + count` in the opposite order
    /// (applied like a shuffle, one undo step).
    pub fn plan_flip(&self, at: u64, count: u64) -> Result<ShufflePlan, Error> {
        self.ready_for_bulk()?;
        let pos = self.shown_positions(at, count)?;
        let mut order: Vec<u32> = self.ids().collect();
        let ids: Vec<u32> = pos.iter().rev().map(|&p| order[p as usize]).collect();
        for (&p, id) in pos.iter().zip(ids) {
            order[p as usize] = id;
        }
        Ok(ShufflePlan {
            revision: self.revision(),
            order,
            rows: pos.len() as u64,
        })
    }

    /// Hide shown rows `at..at + count` (one undo step). Returns how many
    /// rows were hidden. Hidden rows are still saved.
    pub fn hide_rows(&mut self, at: u64, count: u64) -> Result<u64, Error> {
        self.ready_for_bulk()?;
        let pos = self.shown_positions(at, count)?;
        if pos.is_empty() {
            return Ok(0);
        }
        let words = (self.next_row_id() as usize).div_ceil(64);
        let mut bits: Vec<u64> = self.hidden_bits().map(|h| h.to_vec()).unwrap_or_default();
        bits.resize(words.max(bits.len()), 0);
        for p in &pos {
            let id = self.doc_id_at(*p as u64).ok_or(Error::OutOfRange)?;
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        }
        self.set_hidden(Some(Arc::new(bits)));
        Ok(pos.len() as u64)
    }

    /// Show hidden rows again (one undo step). Returns how many came back.
    pub fn show_rows(&mut self, which: ShowRows) -> Result<u64, Error> {
        self.ready_for_bulk()?;
        let Some(old) = self.hidden_bits().cloned() else {
            return Ok(0);
        };
        let total = self.total_rows();
        let span = match which {
            ShowRows::All => 0..total,
            ShowRows::Around { at, count } => {
                let shown = self.row_count();
                let end = at.checked_add(count).ok_or(Error::OutOfRange)?;
                if count == 0 || end > shown {
                    return Err(Error::OutOfRange);
                }
                let pos = |r: u64| self.row_number(r).ok_or(Error::OutOfRange);
                let p0 = if at == 0 { 0 } else { pos(at)? };
                let p1 = if end == shown {
                    total
                } else {
                    pos(end - 1)? + 1
                };
                p0..p1
            }
        };
        let mut bits = old.to_vec();
        let mut shown = 0u64;
        for p in span {
            let id = self.doc_id_at(p).ok_or(Error::OutOfRange)?;
            if let Some(w) = bits.get_mut((id >> 6) as usize) {
                if (*w >> (id & 63)) & 1 == 1 {
                    *w &= !(1 << (id & 63));
                    shown += 1;
                }
            }
        }
        if shown == 0 {
            return Ok(0);
        }
        let any = bits.iter().any(|&w| w != 0);
        self.set_hidden(any.then(|| Arc::new(bits)));
        Ok(shown)
    }

    /// Put shown rows `at..at + count` in a random order among themselves.
    pub fn plan_shuffle(
        &self,
        at: u64,
        count: u64,
        seed: u64,
        progress: &Progress,
    ) -> Result<ShufflePlan, Error> {
        self.ready_for_bulk()?;
        let pos = self.shown_positions(at, count)?;
        progress.start(pos.len() as u64);
        let mut order: Vec<u32> = self.ids().collect();
        let mut ids: Vec<u32> = pos.iter().map(|&p| order[p as usize]).collect();
        let mut state = seed;
        for i in (1..ids.len()).rev() {
            let j = below(&mut state, i + 1);
            ids.swap(i, j);
            if i % (1 << 20) == 0 {
                progress.step(1 << 20)?;
            }
        }
        for (&p, id) in pos.iter().zip(ids) {
            order[p as usize] = id;
        }
        Ok(ShufflePlan {
            revision: self.revision(),
            order,
            rows: pos.len() as u64,
        })
    }

    /// Apply a shuffle (one undo step). Fails with [`Error::Changed`] if the
    /// document changed since it was planned.
    pub fn apply_shuffle(&mut self, plan: ShufflePlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        if plan.rows > 1 {
            self.set_order(plan.order);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{ColumnFilter, Condition, FilterSpec, Rule, TextOp};
    use crate::{OpenOptions, SaveOptions};

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
        (0..d.row_count())
            .map(|r| d.cell(r, c).unwrap_or_default())
            .collect()
    }

    #[test]
    fn totals_over_several_blocks_count_each_cell_once() {
        let d = doc("a,b
1,x
2.5,
,-4
");
        let one = |r, rs, c, cs| Area {
            row: r,
            rows: rs,
            col: c,
            cols: cs,
        };
        // Column a (rows 0..4), column b (rows 0..4) and an overlap with both.
        let t = d
            .selection_totals_areas(
                &[one(0, 4, 0, 1), one(0, 4, 1, 1), one(1, 2, 0, 2)],
                ReadOptions::default(),
                &Progress::new(),
            )
            .unwrap();
        let whole = d
            .selection_totals(0, 4, 0, 2, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert_eq!(t, whole);
        // Two separate blocks: a1..a2 and b3.
        let t = d
            .selection_totals_areas(
                &[one(1, 2, 0, 1), one(3, 1, 1, 1)],
                ReadOptions::default(),
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(t.cells, 3);
        assert_eq!(t.sum, Some(1.0 + 2.5 - 4.0));
    }

    #[test]
    fn totals_of_a_selection() {
        let d = doc("a,b\n1,x\n2.5,\n,-4\n");
        let t = d
            .selection_totals(0, 4, 0, 2, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert_eq!(t.cells, 8);
        assert_eq!(t.filled, 6);
        assert_eq!(t.numbers, 3);
        assert_eq!(t.sum, Some(-0.5));
        assert_eq!(t.min, Some(-4.0));
        assert_eq!(t.max, Some(2.5));
        let none = d
            .selection_totals(0, 1, 0, 2, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert_eq!((none.numbers, none.sum), (0, None));
        assert!(!t.too_large && !none.too_large);
        // A sum past the largest double says so instead of looking empty.
        let big = doc("a\n1e308\n1e308\n");
        let o = big
            .selection_totals(1, 2, 0, 1, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert_eq!((o.numbers, o.sum, o.average), (2, None, None));
        assert!(o.too_large);
        assert_eq!(o.max, Some(1e308));
        assert!(d
            .selection_totals(0, 5, 0, 1, ReadOptions::default(), &Progress::new())
            .is_err());
        assert!(d
            .selection_totals(
                0,
                1,
                0,
                MAX_TOTAL_COLS + 1,
                ReadOptions::default(),
                &Progress::new()
            )
            .is_err());
        // Past the last column: blank cells.
        let wide = d
            .selection_totals(1, 1, 1, 5, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert_eq!((wide.cells, wide.filled), (5, 1));
    }

    #[test]
    fn characters_distinct_and_flip() {
        let mut d = doc("a,b\nxy, xy \nz,\nxy,é\n");
        let p = Progress::new();
        let t = d
            .selection_totals(1, 3, 0, 2, ReadOptions::default(), &p)
            .unwrap();
        assert_eq!(t.chars, 2 + 4 + 1 + 2 + 1);
        assert_eq!(d.selection_distinct(1, 3, 0, 2, &p).unwrap(), (3, true));
        assert_eq!(d.selection_distinct(0, 1, 5, 2, &p).unwrap(), (0, true));
        let plan = d.plan_flip(1, 3).unwrap();
        d.apply_shuffle(plan).unwrap();
        assert_eq!(col(&d, 0), ["a", "xy", "z", "xy"]);
        assert_eq!(col(&d, 1), ["b", "é", "", " xy "]);
        d.undo();
        assert_eq!(col(&d, 1), ["b", " xy ", "", "é"]);
        // With hidden rows only shown rows swap places.
        d.hide_rows(2, 1).unwrap();
        let plan = d.plan_flip(0, 3).unwrap();
        d.apply_shuffle(plan).unwrap();
        d.show_rows(ShowRows::All).unwrap();
        assert_eq!(col(&d, 1), ["é", " xy ", "", "b"]);
    }

    #[test]
    fn totals_stop_when_cancelled() {
        let text: String = (0..100_000).map(|i| format!("{i}\n")).collect();
        let d = doc(&text);
        let p = Progress::new();
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            d.selection_totals(0, 100_000, 0, 1, ReadOptions::default(), &p),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn hide_and_show_rows_undo_and_save() {
        let mut d = doc("a\nb\nc\nd\ne\n");
        assert_eq!(d.hide_rows(1, 2).unwrap(), 2);
        assert_eq!(col(&d, 0), ["a", "d", "e"]);
        assert_eq!(d.hidden_rows(), 2);
        assert!(d.is_filtered() && !d.has_filter());
        assert!(d.filter_spec().is_none());
        d.undo();
        assert_eq!(col(&d, 0), ["a", "b", "c", "d", "e"]);
        assert!(!d.is_filtered());
        d.redo();
        assert_eq!(col(&d, 0), ["a", "d", "e"]);
        // Show around rows 0..2 (a, d): brings back b and c.
        assert_eq!(
            d.show_rows(ShowRows::Around { at: 0, count: 2 }).unwrap(),
            2
        );
        assert_eq!(d.row_count(), 5);
        assert!(!d.is_filtered());
        d.undo();
        assert_eq!(d.row_count(), 3);
        // Hidden rows are saved.
        let dir = std::env::temp_dir().join(format!("nde-hide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h.csv");
        std::fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();
        let mut f = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        f.hide_rows(0, 2).unwrap();
        f.set_cell(2, 0, "E".into()).unwrap();
        f.save(SaveOptions::default()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nb\nc\nd\nE\n");
        // Still hidden after the save reload.
        assert_eq!(col(&f, 0), ["c", "d", "E"]);
        assert_eq!(f.show_rows(ShowRows::All).unwrap(), 2);
        assert_eq!(f.row_count(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hidden_rows_survive_filter_changes() {
        let mut d = doc("n\n1\n2\n3\n4\n");
        d.hide_rows(1, 1).unwrap(); // hides "1"
        let spec = FilterSpec {
            columns: vec![ColumnFilter {
                col: 0,
                rule: Rule::Conditions {
                    first: Condition::Text {
                        op: TextOp::NotEquals,
                        text: "3".into(),
                        match_case: false,
                    },
                    second: None,
                    or: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        };
        d.set_filter(&spec).unwrap();
        assert_eq!(col(&d, 0), ["n", "2", "4"]);
        d.clear_filter();
        assert_eq!(col(&d, 0), ["n", "2", "3", "4"]);
        assert_eq!(d.hidden_rows(), 1);
    }

    #[test]
    fn shuffle_keeps_every_row_and_undoes() {
        let text: String = (0..1000).map(|i| format!("{i}\n")).collect();
        let mut d = doc(&format!("title\n{text}"));
        let before = col(&d, 0);
        let plan = d.plan_shuffle(1, 1000, 42, &Progress::new()).unwrap();
        d.apply_shuffle(plan).unwrap();
        let after = col(&d, 0);
        assert_eq!(after[0], "title");
        assert_ne!(after, before);
        let mut sorted = after.clone();
        sorted.sort();
        let mut want = before.clone();
        want.sort();
        assert_eq!(sorted, want);
        d.undo();
        assert_eq!(col(&d, 0), before);
        // A stale plan is refused.
        let plan = d.plan_shuffle(1, 10, 1, &Progress::new()).unwrap();
        d.set_cell(0, 0, "t".into()).unwrap();
        assert!(matches!(d.apply_shuffle(plan), Err(Error::Changed)));
    }

    #[test]
    fn shuffle_with_hidden_rows_moves_only_shown_rows() {
        let mut d = doc("a\nb\nc\nd\ne\nf\n");
        d.hide_rows(2, 1).unwrap(); // c
        let plan = d.plan_shuffle(0, 5, 7, &Progress::new()).unwrap();
        d.apply_shuffle(plan).unwrap();
        d.show_rows(ShowRows::All).unwrap();
        assert_eq!(d.cell(2, 0).as_deref(), Some("c"));
    }
}
