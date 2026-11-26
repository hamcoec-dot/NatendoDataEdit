//! The values of one column of a table, for "value must exist in another table" checks
//! (foreign keys): 128-bit hashes of the values in the table's shown data rows, so a set
//! costs about 16 bytes per distinct value however long the values are. Blank values are
//! not in the set. Built once and shared (the app keeps one per table, revision, column and
//! way of comparing, while that table is unchanged).

use std::collections::HashSet;
use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::{check_cols, Cols};
use crate::updates::key_hash;
use crate::Error;

/// Rows per parallel task.
const CHUNK: u64 = 1 << 16;
/// Bytes counted per row read (a hash in the part lists, then the set's share).
const BYTES_PER_ROW: u64 = 48;

/// Where a key set's values came from, for the report (and for telling when it is out of date).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySource {
    /// The table's name as the rule names it.
    pub table: String,
    /// The column's title as the rule names it.
    pub column: String,
    /// The tab it was read from (set by the app; 0 when not known).
    pub doc: u32,
    /// The table's revision and data revision when it was read.
    pub revision: u64,
    pub data_revision: u64,
    /// Rows read (shown data rows), and rows hidden by a filter, which were not read.
    pub rows: u64,
    pub hidden: u64,
}

/// A column's values, hashed.
#[derive(Debug)]
pub struct KeySet {
    set: HashSet<u128>,
    match_case: bool,
    trim: bool,
    pub source: KeySource,
}

impl KeySet {
    /// The values of column `col` in `other`'s shown data rows (the first row skipped with
    /// `keep_first`). Refused when the table is too large for the memory `budget` (bytes).
    pub fn build(
        other: &Document,
        col: u32,
        keep_first: bool,
        match_case: bool,
        trim: bool,
        budget: u64,
        progress: &Progress,
    ) -> Result<KeySet, Error> {
        other.ready_for_bulk()?;
        check_cols(other, &[col])?;
        let rows = other.data_rows(keep_first);
        let n = rows.end - rows.start;
        if n.saturating_mul(BYTES_PER_ROW) > budget {
            return Err(Error::OutOfMemory(
                "the other table has too many rows to check values against",
            ));
        }
        let reader = Cols::new(other, &[col]);
        progress.start(n);
        let parts: Vec<HashSet<u128>> = (0..n.div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<HashSet<u128>, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut out = HashSet::new();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = other.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(other, id, &mut buf);
                    let v = reader.get(&buf, 0);
                    if !v.trim().is_empty() {
                        out.insert(key_hash(&[v], match_case, trim));
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let mut set = HashSet::with_capacity(parts.iter().map(HashSet::len).max().unwrap_or(0));
        for p in parts {
            if set.is_empty() {
                set = p;
            } else {
                set.extend(p);
            }
        }
        Ok(KeySet {
            set,
            match_case,
            trim,
            source: KeySource {
                revision: other.revision(),
                data_revision: other.data_revision(),
                rows: n,
                hidden: other.total_rows() - other.row_count(),
                ..Default::default()
            },
        })
    }

    /// Whether `v` is among the values (compared the way the set was built).
    pub fn has(&self, v: &str) -> bool {
        self.set
            .contains(&key_hash(&[v], self.match_case, self.trim))
    }

    /// Distinct values held.
    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// About how much memory the set holds, in bytes.
    pub fn bytes(&self) -> u64 {
        self.set.capacity() as u64 * 17 + 64
    }

    /// Built for this way of comparing.
    pub fn compares(&self, match_case: bool, trim: bool) -> bool {
        self.match_case == match_case && self.trim == trim
    }
}

/// What a "value must exist in" rule reads from, filled in by the app for each rule (by
/// rule number; never saved or sent).
#[derive(Debug, Clone, Default)]
pub enum KeyBinding {
    /// Nothing given: the rule does not run ("not run" with a reason).
    #[default]
    None,
    Ready(Arc<KeySet>),
    /// The table could not be read; the reason says why ("open precincts.csv").
    Missing(String),
}

impl PartialEq for KeyBinding {
    fn eq(&self, o: &Self) -> bool {
        match (self, o) {
            (KeyBinding::None, KeyBinding::None) => true,
            (KeyBinding::Ready(a), KeyBinding::Ready(b)) => Arc::ptr_eq(a, b),
            (KeyBinding::Missing(a), KeyBinding::Missing(b)) => a == b,
            _ => false,
        }
    }
}
