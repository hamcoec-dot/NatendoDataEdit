//! The values of one column of a table, for "value must exist in another table" checks
//! (foreign keys): 128-bit hashes of the values in the table's shown data rows, so a set
//! costs about 16 bytes per distinct value however long the values are. Blank values are
//! not in the set. Built once and shared (the app keeps one per table, revision, column and
//! way of comparing, while that table is unchanged).

use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::{check_cols, Cols};
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

/// The set's own hasher: its keys are already hashes, so their low bits are used as they are.
#[derive(Default)]
struct Pass(u64);

impl Hasher for Pass {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }
    fn write_u128(&mut self, v: u128) {
        self.0 = v as u64;
    }
}

type Set = HashSet<u128, BuildHasherDefault<Pass>>;

/// A value's 128-bit hash (two differently seeded SipHashes), compared as the rule asks:
/// outer spaces left out with `trim`, any case with `match_case` off. ASCII text is
/// lower-cased byte by byte (the same text `to_lowercase` gives) with no copy made.
fn value_hash(v: &str, match_case: bool, trim: bool) -> u128 {
    let v = if trim { v.trim() } else { v };
    let mut a = std::collections::hash_map::DefaultHasher::new();
    let mut b = std::collections::hash_map::DefaultHasher::new();
    0x5a_u8.hash(&mut b);
    let mut feed = |bytes: &[u8]| {
        a.write(bytes);
        b.write(bytes);
    };
    if match_case {
        feed(v.as_bytes());
    } else if v.is_ascii() {
        let mut low = [0u8; 64];
        for part in v.as_bytes().chunks(64) {
            for (o, c) in low.iter_mut().zip(part) {
                *o = c.to_ascii_lowercase();
            }
            feed(&low[..part.len()]);
        }
    } else {
        feed(v.to_lowercase().as_bytes());
    }
    a.write_u8(0xff);
    b.write_u8(0xff);
    (u128::from(a.finish()) << 64) | u128::from(b.finish())
}

/// A column's values, hashed.
#[derive(Debug)]
pub struct KeySet {
    set: Set,
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
        // Hashed in parallel, then put in one set (its keys need no hashing again).
        let parts: Vec<Vec<u128>> = (0..n.div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<Vec<u128>, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut out = Vec::with_capacity((e - s) as usize);
                let mut buf = Vec::new();
                for r in s..e {
                    let id = other.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(other, id, &mut buf);
                    let v = reader.get(&buf, 0);
                    if !v.trim().is_empty() {
                        out.push(value_hash(v, match_case, trim));
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let mut set = Set::with_capacity_and_hasher(
            parts.iter().map(Vec::len).sum(),
            BuildHasherDefault::default(),
        );
        for p in parts {
            set.extend(p);
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
            .contains(&value_hash(v, self.match_case, self.trim))
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
