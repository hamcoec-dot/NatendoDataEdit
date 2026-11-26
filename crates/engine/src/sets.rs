//! Data > Compare lists: which keys two to five tables share. Every key gets
//! a mask saying which lists hold it; each slice of the lists (`A and B, not
//! C`) is then a count of keys and of rows, and can be opened as a table.
//!
//! Keys are the exact text (under the join's own key rules), never hashes, so
//! counts are exact. Each table is read once: a row remembers where its key
//! sits, and its mask is looked up there when every list is in.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::join::{norm_key, JoinSpec};
use crate::ops::{check_cols, Cols, CHUNK};
use crate::reshape::{push_line, too_big, Table, MAX_TABLE_BYTES};
use crate::Error;

/// Lists compared at once.
pub const MAX_LISTS: usize = 5;
/// Key columns of a composite key.
pub const MAX_SET_KEYS: usize = 8;
/// Different keys held in all.
pub const MAX_SET_KEYS_TOTAL: u64 = 20_000_000;
/// Memory for the keys at most (and never more than a quarter of this
/// computer's).
pub const MAX_SET_BYTES: u64 = 2 << 30;
/// Rough cost of one key beyond its text.
const KEY_COST: u64 = 56;
const SHARDS: usize = 64;
/// A slot is a shard and a place in it.
const PLACE_BITS: u32 = 26;
const BLANK: u32 = u32::MAX;

/// One list: which columns make its key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SetList {
    pub cols: Vec<u32>,
    /// The first row holds the column titles.
    pub header: bool,
}

/// What to compare. One set of key rules for every list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SetsSpec {
    pub lists: Vec<SetList>,
    pub match_case: bool,
    pub trim: bool,
    /// The join's loose keys: spaces and punctuation ignored, leading zeros
    /// ignored, numbers compared by value.
    pub loose: bool,
}

impl Default for SetsSpec {
    fn default() -> SetsSpec {
        SetsSpec {
            lists: Vec::new(),
            match_case: false,
            trim: true,
            loose: false,
        }
    }
}

impl SetsSpec {
    pub fn check(&self) -> Result<(), Error> {
        if !(2..=MAX_LISTS).contains(&self.lists.len()) {
            return Err(Error::Invalid(format!(
                "compare 2 to {MAX_LISTS} lists at once"
            )));
        }
        let n = self.lists[0].cols.len();
        if !(1..=MAX_SET_KEYS).contains(&n) {
            return Err(Error::Invalid(format!(
                "pick 1 to {MAX_SET_KEYS} key columns"
            )));
        }
        if self.lists.iter().any(|l| l.cols.len() != n) {
            return Err(Error::Invalid(
                "pick the same number of key columns in every list".into(),
            ));
        }
        Ok(())
    }

    fn rules(&self) -> JoinSpec {
        JoinSpec {
            match_case: self.match_case,
            trim: self.trim,
            loose: self.loose,
            ..Default::default()
        }
    }

    /// The rules in words, for a slice's title.
    pub fn rules_text(&self) -> String {
        let mut out = vec![
            if self.match_case {
                "capitals count"
            } else {
                "capitals ignored"
            },
            if self.trim {
                "spaces trimmed"
            } else {
                "spaces kept"
            },
        ];
        if self.loose {
            out.push("loose keys");
        }
        out.join(", ")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ListTotals {
    /// Shown data rows read.
    pub rows: u64,
    /// Different keys.
    pub keys: u64,
    /// Rows whose key is blank (in no slice).
    pub blank: u64,
}

/// One slice: the keys in exactly the lists of `mask` (bit 0 = first list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Slice {
    pub mask: u8,
    pub keys: u64,
    /// Rows of each list holding those keys.
    pub rows: Vec<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SetsReport {
    pub lists: Vec<ListTotals>,
    /// Every combination of lists, by mask from 1.
    pub slices: Vec<Slice>,
}

/// A comparison: the counts, and for each list one byte a shown data row (its
/// key's mask, 0 for a blank key).
#[derive(Debug, Clone, Default)]
pub struct SetsResult {
    pub report: SetsReport,
    pub row_masks: Vec<Vec<u8>>,
}

#[derive(Default)]
struct Shard {
    places: HashMap<String, u32>,
    masks: Vec<u8>,
}

/// Builds a comparison one list at a time (so only one table need be held at
/// once).
pub struct SetsBuilder {
    spec: SetsSpec,
    rules: JoinSpec,
    /// Key parts compared exactly (a column typed Id or Text in any list).
    exact: Vec<bool>,
    shards: Vec<Mutex<Shard>>,
    /// Each list read so far: a slot for every shown data row.
    slots: Vec<Vec<u32>>,
    keys: AtomicU64,
    bytes: AtomicU64,
    max_keys: u64,
    max_bytes: u64,
}

impl Document {
    /// Which of the key columns `cols` are compared exactly in this table.
    pub fn exact_key_parts(&self, cols: &[u32]) -> Vec<bool> {
        let ctx = self.read_ctx(Default::default());
        cols.iter()
            .map(|&c| ctx.type_of(c).is_text_like())
            .collect()
    }
}

fn shard_of(v: &str) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    for &b in v.as_bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    (h as usize) % SHARDS
}

impl SetsBuilder {
    /// `exact`: for each key part, whether any list's column there is typed
    /// Id or Text (see [`Document::exact_key_parts`]).
    pub fn new(spec: &SetsSpec, exact: Vec<bool>) -> Result<SetsBuilder, Error> {
        spec.check()?;
        let ram = crate::sysmem::installed_bytes();
        Ok(SetsBuilder {
            spec: spec.clone(),
            rules: spec.rules(),
            exact,
            shards: (0..SHARDS).map(|_| Mutex::default()).collect(),
            slots: Vec::new(),
            keys: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            max_keys: MAX_SET_KEYS_TOTAL,
            max_bytes: ram.map_or(MAX_SET_BYTES, |r| MAX_SET_BYTES.min(r / 4)),
        })
    }

    /// Read the next list's shown rows.
    pub fn add(&mut self, d: &Document, progress: &Progress) -> Result<(), Error> {
        d.ready_for_bulk()?;
        let at = self.slots.len();
        let list = self.spec.lists.get(at).ok_or(Error::OutOfRange)?.clone();
        check_cols(d, &list.cols)?;
        let reader = Cols::new(d, &list.cols);
        let k = list.cols.len();
        let rows = d.data_rows(list.header);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let bit = 1u8 << at;
        let this = &*self;
        let parts: Vec<Vec<u32>> = (0..((r1 - r0) as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<Vec<u32>, Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let n = (e - s) as usize;
                let mut slots = vec![BLANK; n];
                // The chunk's keys in one piece, grouped by shard, so each
                // shard is locked once.
                let mut text = String::new();
                let mut by: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); SHARDS];
                let mut buf = Vec::new();
                let mut key = String::new();
                for (i, r) in (s..e).enumerate() {
                    let id = d.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(d, id, &mut buf);
                    let mut cells = [""; MAX_SET_KEYS];
                    for (j, cell) in cells.iter_mut().enumerate().take(k) {
                        *cell = reader.get(&buf, j);
                    }
                    if !norm_key(&cells[..k], &this.rules, &this.exact, &mut key) {
                        continue;
                    }
                    let from = text.len() as u32;
                    text.push_str(&key);
                    by[shard_of(&key)].push((i as u32, from, text.len() as u32));
                }
                progress.step(0)?;
                for step in 0..SHARDS {
                    let sh = (step + c) % SHARDS;
                    if by[sh].is_empty() {
                        continue;
                    }
                    let mut m = this.shards[sh].lock().unwrap_or_else(|p| p.into_inner());
                    for &(i, a, b) in &by[sh] {
                        let key = &text[a as usize..b as usize];
                        let place = match m.places.get(key) {
                            Some(&p) => p,
                            None => {
                                this.charge(key.len())?;
                                let p = m.masks.len() as u32;
                                m.masks.push(0);
                                m.places.insert(key.to_string(), p);
                                p
                            }
                        };
                        m.masks[place as usize] |= bit;
                        slots[i as usize] = ((sh as u32) << PLACE_BITS) | place;
                    }
                }
                progress.step(e - s)?;
                Ok(slots)
            })
            .collect::<Result<_, _>>()?;
        self.slots.push(parts.concat());
        Ok(())
    }

    /// Different keys so far, and the memory charged for them.
    pub fn used(&self) -> (u64, u64) {
        (
            self.keys.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
        )
    }

    fn charge(&self, len: usize) -> Result<(), Error> {
        if self.keys.fetch_add(1, Ordering::Relaxed) >= self.max_keys {
            return Err(Error::Invalid(format!(
                "more than {} million different keys: compare fewer rows",
                self.max_keys / 1_000_000
            )));
        }
        let cost = len as u64 + KEY_COST;
        if self.bytes.fetch_add(cost, Ordering::Relaxed) + cost > self.max_bytes {
            return Err(Error::OutOfMemory(
                "the keys need more memory than a comparison may use; compare fewer rows",
            ));
        }
        Ok(())
    }

    /// Count the slices. Every list must have been added.
    pub fn finish(self, progress: &Progress) -> Result<SetsResult, Error> {
        let n = self.spec.lists.len();
        if self.slots.len() != n {
            return Err(Error::OutOfRange);
        }
        progress.step(0)?;
        let shards: Vec<Shard> = self
            .shards
            .into_iter()
            .map(|m| m.into_inner().unwrap_or_else(|p| p.into_inner()))
            .collect();
        let slices = (1usize << n) - 1;
        // Keys in each slice.
        let keys: Vec<u64> = shards
            .par_iter()
            .map(|s| {
                let mut t = vec![0u64; slices + 1];
                for &m in &s.masks {
                    t[m as usize] += 1;
                }
                t
            })
            .reduce(
                || vec![0; slices + 1],
                |mut a, b| {
                    a.iter_mut().zip(b).for_each(|(a, b)| *a += b);
                    a
                },
            );
        let mut report = SetsReport {
            lists: Vec::with_capacity(n),
            slices: (1..=slices)
                .map(|mask| Slice {
                    mask: mask as u8,
                    keys: keys[mask],
                    rows: vec![0; n],
                })
                .collect(),
        };
        let mut row_masks = Vec::with_capacity(n);
        for (at, slots) in self.slots.into_iter().enumerate() {
            progress.step(0)?;
            let masks: Vec<u8> = slots
                .par_iter()
                .map(|&s| {
                    if s == BLANK {
                        0
                    } else {
                        shards[(s >> PLACE_BITS) as usize].masks
                            [(s & ((1 << PLACE_BITS) - 1)) as usize]
                    }
                })
                .collect();
            let tally: Vec<u64> = masks
                .par_chunks(CHUNK)
                .map(|c| {
                    let mut t = vec![0u64; slices + 1];
                    for &m in c {
                        t[m as usize] += 1;
                    }
                    t
                })
                .reduce(
                    || vec![0; slices + 1],
                    |mut a, b| {
                        a.iter_mut().zip(b).for_each(|(a, b)| *a += b);
                        a
                    },
                );
            for s in &mut report.slices {
                s.rows[at] = tally[s.mask as usize];
            }
            report.lists.push(ListTotals {
                rows: masks.len() as u64,
                keys: (1..=slices)
                    .filter(|m| m & (1 << at) != 0)
                    .map(|m| keys[m])
                    .sum(),
                blank: tally[0],
            });
            row_masks.push(masks);
        }
        Ok(SetsResult { report, row_masks })
    }
}

/// Compare tables all at hand (the command line and tests).
pub fn compare_lists(
    docs: &[&Document],
    spec: &SetsSpec,
    progress: &Progress,
) -> Result<SetsResult, Error> {
    spec.check()?;
    if docs.len() != spec.lists.len() {
        return Err(Error::OutOfRange);
    }
    let mut exact = vec![false; spec.lists[0].cols.len()];
    for (d, l) in docs.iter().zip(&spec.lists) {
        check_cols(d, &l.cols)?;
        for (a, b) in exact.iter_mut().zip(d.exact_key_parts(&l.cols)) {
            *a |= b;
        }
    }
    let mut b = SetsBuilder::new(spec, exact)?;
    for d in docs {
        b.add(d, progress)?;
    }
    b.finish(progress)
}

impl Document {
    /// The positions of the shown data rows in slice `mask`.
    fn slice_rows<'a>(
        &self,
        header: bool,
        masks: &'a [u8],
        mask: u8,
    ) -> Result<impl Iterator<Item = u64> + 'a, Error> {
        let rows = self.data_rows(header);
        if rows.end - rows.start != masks.len() as u64 {
            return Err(Error::Invalid(
                "the table changed since the comparison: run the comparison again".into(),
            ));
        }
        let first = rows.start;
        Ok(masks
            .iter()
            .enumerate()
            .filter(move |(_, &m)| m == mask)
            .map(move |(i, _)| first + i as u64))
    }

    /// This list's rows in slice `mask`, every column, as a new table.
    pub fn sets_slice_table(
        &self,
        list: &SetList,
        masks: &[u8],
        mask: u8,
        progress: &Progress,
    ) -> Result<Table, Error> {
        self.ready_for_bulk()?;
        let cols: Vec<u32> = (0..self.col_count()).collect();
        let mut at: Vec<u64> = Vec::new();
        if list.header && self.row_count() > 0 {
            at.push(0);
        }
        at.extend(self.slice_rows(list.header, masks, mask)?);
        let reader = Cols::new(self, &cols);
        progress.start(at.len() as u64);
        let held = AtomicU64::new(0);
        let parts: Vec<Vec<u8>> = at
            .par_chunks(CHUNK)
            .map(|rows| -> Result<Vec<u8>, Error> {
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for &r in rows {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let start = out.len();
                    {
                        let line: Vec<&str> =
                            (0..cols.len()).map(|k| reader.get(&buf, k)).collect();
                        push_line(&mut out, &line);
                    }
                    if held.fetch_add((out.len() - start) as u64, Ordering::Relaxed)
                        > MAX_TABLE_BYTES as u64
                    {
                        return Err(too_big());
                    }
                }
                progress.step(rows.len() as u64)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        Ok(Table {
            bytes: parts.concat(),
            rows: at.len() as u64,
            cols: cols.len() as u32,
        })
    }

    /// The different keys of slice `mask` as this list spells them (the first
    /// spelling met), under the key columns' own titles.
    pub fn sets_keys_table(
        &self,
        spec: &SetsSpec,
        exact: &[bool],
        list: usize,
        masks: &[u8],
        mask: u8,
        progress: &Progress,
    ) -> Result<Table, Error> {
        self.ready_for_bulk()?;
        let l = spec.lists.get(list).ok_or(Error::OutOfRange)?;
        check_cols(self, &l.cols)?;
        let rules = spec.rules();
        let reader = Cols::new(self, &l.cols);
        let k = l.cols.len();
        let mut out = Vec::new();
        push_line(&mut out, &self.titles(&l.cols, l.header));
        let mut rows = 1;
        let mut seen = std::collections::HashSet::new();
        let mut buf = Vec::new();
        let mut key = String::new();
        progress.start(masks.len() as u64);
        for (n, r) in self.slice_rows(l.header, masks, mask)?.enumerate() {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let cells: Vec<&str> = (0..k).map(|j| reader.get(&buf, j)).collect();
            if norm_key(&cells, &rules, exact, &mut key) && seen.insert(key.clone()) {
                push_line(&mut out, &cells);
                rows += 1;
                if out.len() > MAX_TABLE_BYTES {
                    return Err(too_big());
                }
            }
            if n % CHUNK == 0 {
                progress.step(CHUNK as u64)?;
            }
        }
        Ok(Table {
            bytes: out,
            rows,
            cols: k as u32,
        })
    }
}

#[cfg(test)]
#[path = "sets_tests.rs"]
mod tests;
