//! Row offset index: where every row starts, built in resumable steps.

use std::collections::HashMap;

use crate::dialect::{Dialect, LineBreak};
use crate::scan::{self, Mode};
use crate::Error;

/// Repair notes kept per document; further repairs are only counted.
pub const MAX_REPAIR_NOTES: usize = 10_000;
/// Distinct field counts tracked in the histogram; the rest are lumped.
const MAX_HISTOGRAM: usize = 1024;
/// Default cap on the memory the index may use (2 GiB, about 536M rows).
pub const DEFAULT_INDEX_BUDGET: usize = 2 << 30;

const BLOCK_BITS: usize = 12;
const BLOCK_MASK: usize = (1 << BLOCK_BITS) - 1;

/// A row the parser had to be lenient with, or that deserves a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repair {
    pub row: u64,
    /// `scan::TEXT_AFTER_QUOTE`, `scan::UNCLOSED_QUOTE`, `scan::LONG_QUOTED_CELL`.
    pub flags: u8,
}

/// Facts gathered while indexing.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub max_fields: u32,
    /// Field count → number of rows (bounded; see `other_field_counts`).
    pub field_counts: HashMap<u32, u64>,
    pub other_field_counts: u64,
    pub crlf: u64,
    pub lf: u64,
    pub cr: u64,
    /// Rows without a line break (only ever the last row).
    pub unterminated_rows: u64,
    pub repairs: Vec<Repair>,
    pub repair_count: u64,
}

impl Stats {
    /// Most common line break, or `None` when the file has none.
    pub fn dominant_line_break(&self) -> Option<LineBreak> {
        // `max_by_key` keeps the last maximum, so ties prefer CRLF, then LF.
        let best = [
            (self.cr, LineBreak::Cr),
            (self.lf, LineBreak::Lf),
            (self.crlf, LineBreak::CrLf),
        ]
        .into_iter()
        .max_by_key(|(n, _)| *n)?;
        (best.0 > 0).then_some(best.1)
    }

    /// True when rows end in more than one style.
    pub fn mixed_line_breaks(&self) -> bool {
        [self.crlf, self.lf, self.cr]
            .iter()
            .filter(|&&n| n > 0)
            .count()
            > 1
    }

    /// Most common field count.
    pub fn usual_fields(&self) -> u32 {
        self.field_counts
            .iter()
            .max_by_key(|(k, n)| (**n, std::cmp::Reverse(**k)))
            .map_or(0, |(k, _)| *k)
    }
}

/// Row start offsets in about 4 bytes per row: a `u64` base per block of
/// 4096 rows plus a `u32` offset from it. A block whose rows span more than
/// 4 GiB (only possible with enormous rows) keeps full `u64` offsets.
/// Growth is fallible and capped by a byte budget, so a crafted file gets an
/// error instead of exhausting memory.
#[derive(Debug, Clone)]
struct Offsets {
    bases: Vec<u64>,
    rel: Vec<u32>,
    /// (block, full offsets) for wide blocks, in block order.
    wide: Vec<(usize, Vec<u64>)>,
    budget: usize,
}

fn oom() -> Error {
    Error::OutOfMemory("not enough memory for the row index")
}

impl Offsets {
    fn new(budget: usize) -> Offsets {
        Offsets {
            bases: Vec::new(),
            rel: Vec::new(),
            wide: Vec::new(),
            budget,
        }
    }

    fn len(&self) -> usize {
        self.rel.len()
    }

    fn memory_bytes(&self) -> usize {
        self.rel.capacity() * 4
            + self.bases.capacity() * 8
            + self
                .wide
                .iter()
                .map(|(_, w)| w.capacity() * 8 + 32)
                .sum::<usize>()
    }

    /// Make room for one more element, doubling within the budget.
    fn grow<T>(v: &mut Vec<T>, used: usize, budget: usize) -> Result<(), Error> {
        if v.len() < v.capacity() {
            return Ok(());
        }
        let extra = v.capacity().max(1024);
        if used.saturating_add(extra * std::mem::size_of::<T>()) > budget {
            return Err(Error::OutOfMemory(
                "the row index would exceed its memory budget",
            ));
        }
        v.try_reserve_exact(extra).map_err(|_| oom())
    }

    fn push(&mut self, off: u64) -> Result<(), Error> {
        let i = self.rel.len();
        let b = i >> BLOCK_BITS;
        let used = self.memory_bytes();
        Self::grow(&mut self.rel, used, self.budget)?;
        if i & BLOCK_MASK == 0 {
            Self::grow(&mut self.bases, used, self.budget)?;
            self.bases.push(off);
        }
        let base = self.bases[b];
        if let Some((wb, w)) = self.wide.last_mut() {
            if *wb == b {
                w.try_reserve(1).map_err(|_| oom())?;
                w.push(off);
                self.rel.push(0);
                return Ok(());
            }
        }
        match u32::try_from(off - base) {
            Ok(d) => self.rel.push(d),
            Err(_) => {
                let start = b << BLOCK_BITS;
                let mut w: Vec<u64> = Vec::new();
                w.try_reserve_exact(BLOCK_MASK + 1).map_err(|_| oom())?;
                w.extend(self.rel[start..].iter().map(|&r| base + r as u64));
                w.push(off);
                self.wide.push((b, w));
                self.rel.push(0);
            }
        }
        Ok(())
    }

    #[inline]
    fn get(&self, i: usize) -> Option<u64> {
        let r = *self.rel.get(i)?;
        let b = i >> BLOCK_BITS;
        if !self.wide.is_empty() {
            if let Ok(k) = self.wide.binary_search_by_key(&b, |(wb, _)| *wb) {
                return Some(self.wide[k].1[i & BLOCK_MASK]);
            }
        }
        Some(self.bases[b] + r as u64)
    }

    fn last(&self) -> u64 {
        self.get(self.len() - 1).expect("offsets is never empty")
    }

    fn shrink(&mut self) {
        self.rel.shrink_to_fit();
        self.bases.shrink_to_fit();
    }
}

/// Offsets of every row of a byte buffer, plus how it was parsed.
#[derive(Debug, Clone)]
pub struct RowIndex {
    /// Entry `i`..`i + 1` is row `i` including its line break. The last
    /// entry is where indexing has reached.
    offsets: Offsets,
    /// First row parsed in line-bound mode, if the file had an unterminated
    /// quote (or line-by-line reading was asked for).
    line_bound_from: Option<u64>,
    dialect: Dialect,
    mode: Mode,
    complete: bool,
    stats: Stats,
}

impl RowIndex {
    /// Start indexing a buffer whose data begins at `start` (after any BOM).
    /// `line_by_line` reads every row in line-bound mode from the start;
    /// `budget` caps the index's memory in bytes.
    pub fn new(dialect: Dialect, start: usize, line_by_line: bool, budget: usize) -> RowIndex {
        let mut offsets = Offsets::new(budget.max(64 << 10));
        offsets
            .push(start as u64)
            .expect("the first offset fits the minimum budget");
        RowIndex {
            offsets,
            line_bound_from: line_by_line.then_some(0),
            dialect,
            mode: if line_by_line {
                Mode::LineBound
            } else {
                Mode::Normal
            },
            complete: false,
            stats: Stats::default(),
        }
    }

    /// Index the whole buffer at once with the default budget.
    pub fn build(buf: &[u8], dialect: Dialect, start: usize) -> Result<RowIndex, Error> {
        let mut idx = RowIndex::new(dialect, start, false, DEFAULT_INDEX_BUDGET);
        idx.step(buf, usize::MAX)?;
        Ok(idx)
    }

    /// Index about `budget` more bytes of `buf` (always whole rows). Returns
    /// `true` once the end is reached. `buf` must be the same buffer each call.
    pub fn step(&mut self, buf: &[u8], budget: usize) -> Result<bool, Error> {
        let mut pos = self.reached();
        let stop = pos.saturating_add(budget);
        while pos < buf.len() {
            if pos >= stop {
                return Ok(false);
            }
            let end = scan::scan_row(buf, pos, self.dialect, self.mode, |_, _| true);
            if end.unterminated {
                // Only possible once: from here on line breaks always end rows.
                self.mode = Mode::LineBound;
                self.line_bound_from = Some(self.rows());
                continue;
            }
            if self.offsets.len() > u32::MAX as usize {
                return Err(Error::TooManyRows);
            }
            self.offsets.push(end.next as u64)?;
            self.note_row(&end, buf[end.content_end..end.next].first().copied());
            pos = end.next;
        }
        self.complete = true;
        self.offsets.shrink();
        Ok(true)
    }

    fn note_row(&mut self, end: &scan::RowEnd, first_break_byte: Option<u8>) {
        let row = self.rows() - 1;
        let s = &mut self.stats;
        match (end.next - end.content_end, first_break_byte) {
            (2, _) => s.crlf += 1,
            (1, Some(b'\n')) => s.lf += 1,
            (1, _) => s.cr += 1,
            _ => s.unterminated_rows += 1,
        }
        s.max_fields = s.max_fields.max(end.fields);
        if let Some(n) = s.field_counts.get_mut(&end.fields) {
            *n += 1;
        } else if s.field_counts.len() < MAX_HISTOGRAM {
            s.field_counts.insert(end.fields, 1);
        } else {
            s.other_field_counts += 1;
        }
        if end.repairs != 0 {
            s.repair_count += 1;
            if s.repairs.len() < MAX_REPAIR_NOTES {
                s.repairs.push(Repair {
                    row,
                    flags: end.repairs,
                });
            }
        }
    }

    /// Byte position indexing has reached.
    pub fn reached(&self) -> usize {
        self.offsets.last() as usize
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Rows indexed so far.
    pub fn rows(&self) -> u64 {
        (self.offsets.len() - 1) as u64
    }

    /// Byte range of row `i` including its line break.
    #[inline]
    pub fn row_range(&self, i: u64) -> Option<(usize, usize)> {
        let i = usize::try_from(i).ok()?;
        let a = self.offsets.get(i)?;
        let b = self.offsets.get(i.checked_add(1)?)?;
        Some((a as usize, b as usize))
    }

    /// Parse mode for row `i`.
    pub fn mode_of(&self, i: u64) -> Mode {
        match self.line_bound_from {
            Some(r) if i >= r => Mode::LineBound,
            _ => Mode::Normal,
        }
    }

    pub fn line_bound_from(&self) -> Option<u64> {
        self.line_bound_from
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Bytes of memory the index itself uses.
    pub fn memory_bytes(&self) -> usize {
        self.offsets.memory_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_offsets_round_trip() {
        let mut o = Offsets::new(1 << 30);
        let mut want = Vec::new();
        let mut x = 0u64;
        for i in 0..20_000u64 {
            // Mostly small steps, one huge jump to force a wide block.
            x += if i == 9000 { 5 << 30 } else { i % 97 + 1 };
            o.push(x).unwrap();
            want.push(x);
        }
        assert_eq!(o.wide.len(), 1);
        for (i, w) in want.iter().enumerate() {
            assert_eq!(o.get(i), Some(*w), "{i}");
        }
        assert_eq!(o.get(want.len()), None);
    }

    #[test]
    fn budget_is_enforced() {
        let buf = vec![b'\n'; 1_000_000];
        let mut idx = RowIndex::new(Dialect::CSV, 0, false, 256 << 10);
        assert!(matches!(
            idx.step(&buf, usize::MAX),
            Err(Error::OutOfMemory(_))
        ));
        let idx = RowIndex::build(&buf, Dialect::CSV, 0).unwrap();
        assert_eq!(idx.rows(), 1_000_000);
        // About 4 bytes per row.
        assert!(idx.memory_bytes() < 4_200_000, "{}", idx.memory_bytes());
    }

    #[test]
    fn line_by_line_option() {
        let idx = RowIndex::new(Dialect::CSV, 0, true, DEFAULT_INDEX_BUDGET);
        assert_eq!(idx.mode_of(0), Mode::LineBound);
        let mut idx = idx;
        idx.step(b"\"a\nb\"\n", usize::MAX).unwrap();
        assert_eq!(idx.rows(), 2);
    }
}
