//! Join (bring values in from another table by matching key values) and
//! append (add another table's rows), each as one undo step.
//!
//! The other table is first copied out under its own lock ([`JoinSource`],
//! [`AppendSource`]), so two documents are never locked together. Keys are
//! looked up in a hash map with a random key (no crafted collisions), and
//! the real key text is always compared. With a filter on, only shown rows
//! change; hidden rows are never touched.
//!
//! A join can add new columns or update existing ones (overwrite, or fill
//! blanks only), keep only matched or only unmatched rows, decide what to do
//! when the other table has a key more than once (first, last, every match
//! as its own row, all matches in one cell, count, sum, smallest, largest),
//! match keys loosely (00123 = 123, 1.0 = 1, "AB-12" = "ab 12") or by range
//! (nearest smaller or larger number or date). Results are plain values.

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::clean::exact_number;
use crate::document::{Document, Edit, Progress, Step};
use crate::ops::{CellPlan, Cols, CHUNK};
use crate::reshape::{MAX_TABLE_BYTES, MAX_TABLE_CELLS};
use crate::value::ReadOptions;
use crate::Error;

/// Most columns a join or append adds.
pub const MAX_JOIN_COLS: usize = 16_384;
/// Longest value "all matches in one cell" may build (bytes).
pub const MAX_JOINED_BYTES: usize = 1_000_000;
/// Rows and columns a join preview shows.
pub const PREVIEW_ROWS: usize = 200;
pub const PREVIEW_COLS: usize = 200;
const PREVIEW_CHARS: usize = 200;
const NO_GROUP: u32 = u32::MAX;

/// Where the values found go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinWrite {
    /// New columns.
    #[default]
    New,
    /// Existing columns (`targets`), replacing what is there. Blank values
    /// in the other table never erase anything.
    Overwrite,
    /// Existing columns (`targets`), only where the cell is blank.
    FillBlanks,
    /// Nothing is written: only keep or drop rows.
    Nothing,
}

/// Which of this table's shown rows stay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepRows {
    #[default]
    All,
    /// Only rows that found a match (rows with a blank key go too).
    Matched,
    /// Only rows that found no match (nothing is written to them).
    Unmatched,
}

/// What to do when the other table has a key more than once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeats {
    #[default]
    First,
    Last,
    /// Every match as its own row (copies of this row).
    Every,
    /// All matches in one cell, joined with `separator` (blanks left out).
    Joined,
    /// One column: how many rows matched.
    Count,
    /// Numbers of the matches added up (text left out).
    Sum,
    Min,
    Max,
}

/// How keys are compared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyMatch {
    #[default]
    Exact,
    /// The nearest key that is the same or smaller (numbers or dates):
    /// bands, tiers, rates in force from a date.
    Below,
    /// The nearest key that is the same or larger.
    Above,
}

/// Where new columns go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    #[default]
    End,
    /// Right after the last key column.
    AfterKey,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoinSpec {
    /// Key columns of this table and of the other (same count).
    pub keys: Vec<u32>,
    pub other_keys: Vec<u32>,
    /// The other table's columns to bring in (empty: every non-key column).
    pub columns: Vec<u32>,
    /// This table's first row is its header.
    pub keep_first: bool,
    /// The other table's first row is its header.
    pub other_header: bool,
    pub match_case: bool,
    pub trim: bool,
    /// Also add the other table's rows that matched nothing.
    pub add_unmatched: bool,
    pub write: JoinWrite,
    /// With overwrite or fill blanks: this table's column for each value
    /// brought in (same count as the values).
    pub targets: Vec<u32>,
    pub keep_rows: KeepRows,
    pub repeats: Repeats,
    /// Between values with [`Repeats::Joined`].
    pub separator: String,
    /// Loose keys: spaces and punctuation ignored, leading zeros ignored,
    /// numbers compared by value.
    pub loose: bool,
    /// Written in the new columns of rows that found no match.
    pub not_found: String,
    pub place: Place,
    /// Put before the titles of new columns.
    pub prefix: String,
    pub key_match: KeyMatch,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppendSpec {
    /// Line columns up by header text (else by position).
    pub by_name: bool,
    pub keep_first: bool,
    pub other_header: bool,
    /// By name: add columns for names this table does not have (else they
    /// are left out and counted).
    pub add_columns: bool,
}

/// What a join or append found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoinReport {
    /// Rows here that found a match.
    pub matched: u64,
    /// Rows here with no match.
    pub unmatched: u64,
    /// Rows added at the end.
    pub added: u64,
    /// Rows of the other table whose key an earlier row already has.
    pub repeated_keys: u64,
    /// Columns added.
    pub cols: u32,
    /// Other columns left out (append by name without adding columns).
    pub dropped_cols: u32,
    /// Rows here whose key is blank (blank keys match nothing).
    pub blank_keys: u64,
    /// Cells that get a new value.
    pub changed: u64,
    /// Rows taken out (keep only matched or unmatched rows).
    pub removed: u64,
    /// Extra rows added for "every match as its own row".
    pub extra_rows: u64,
    /// Range lookup: the other table's keys that are not numbers or dates.
    pub bad_keys: u64,
}

fn push_part(p: &str, match_case: bool, out: &mut String) {
    if match_case {
        out.push_str(p);
    } else {
        out.extend(p.chars().flat_map(char::to_lowercase));
    }
}

/// `00123` → `123`, `-1.50` → `-1.5`, `0.0` → `0`, `1,234.50` → `1234.5`:
/// a plain decimal number (thousands grouped by commas in threes, or not
/// grouped) written one way, without going through floating point (long
/// ids stay exact). `None` if `t` is not such a number.
fn canonical_decimal(t: &str) -> Option<String> {
    let (neg, body) = match t.as_bytes().first()? {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, f),
        None => (body, ""),
    };
    let int: Cow<str> = if int.contains(',') {
        // Groups of three after a first group of one to three digits.
        let mut groups = int.split(',');
        let first = groups.next()?;
        if first.is_empty() || first.len() > 3 {
            return None;
        }
        let rest: Vec<&str> = groups.collect();
        if rest.iter().any(|g| g.len() != 3) {
            return None;
        }
        Cow::Owned(int.replace(',', ""))
    } else {
        Cow::Borrowed(int)
    };
    if int.is_empty() && frac.is_empty()
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let int = int.trim_start_matches('0');
    let frac = frac.trim_end_matches('0');
    if int.is_empty() && frac.is_empty() {
        return Some("0".into());
    }
    let mut s = String::with_capacity(int.len() + frac.len() + 3);
    if neg {
        s.push('-');
    }
    s.push_str(if int.is_empty() { "0" } else { int });
    if !frac.is_empty() {
        s.push('.');
        s.push_str(frac);
    }
    Some(s)
}

/// A loose key part: a number by value; otherwise its runs of letters and
/// digits, case and accents of letters kept as asked, leading zeros of
/// each run of digits left out. Spaces and punctuation between runs are
/// ignored, except between two runs of digits, where a boundary stays:
/// "AB-012" = "ab 12" = "ab12", but "12-3" ≠ "1-23", "2024-11-5" ≠
/// "2024-1-15" and "555-0123" ≠ "5550123". A part with no letters or
/// digits ("-", "?", "#") is empty (a blank key).
fn loose_part(p: &str, match_case: bool, out: &mut String) {
    let t = p.trim();
    if let Some(n) = canonical_decimal(t) {
        out.push_str(&n);
        return;
    }
    let mut prev_ends_digit = false;
    for tok in t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|x| !x.is_empty())
    {
        let starts_digit = tok.starts_with(|c: char| c.is_ascii_digit());
        if prev_ends_digit && starts_digit {
            out.push('|');
        }
        let mut zeros = 0usize;
        let mut in_digits = false;
        for c in tok.chars() {
            if c.is_ascii_digit() {
                if !in_digits && c == '0' {
                    zeros += 1;
                    continue;
                }
                in_digits = true;
                zeros = 0;
                out.push(c);
            } else {
                if zeros > 0 {
                    out.push('0');
                    zeros = 0;
                }
                in_digits = false;
                if match_case {
                    out.push(c);
                } else {
                    out.extend(c.to_lowercase());
                }
            }
        }
        if zeros > 0 {
            out.push('0');
        }
        prev_ends_digit = tok.ends_with(|c: char| c.is_ascii_digit());
    }
}

/// The key of `parts` in `out`; `false` when it is blank (blank keys match
/// nothing). A loose key that keeps no letters or digits is blank too.
pub(crate) fn norm_key(parts: &[&str], spec: &JoinSpec, exact: &[bool], out: &mut String) -> bool {
    out.clear();
    let mut part = String::new();
    let mut any = false;
    for (i, p) in parts.iter().enumerate() {
        part.clear();
        // A key column typed Id or Text compares exactly (00123 is not 123); loose then means only
        // trimming and case.
        if spec.loose && !exact.get(i).copied().unwrap_or(false) {
            loose_part(p, spec.match_case, &mut part);
            any |= !part.is_empty();
        } else {
            any |= !p.trim().is_empty();
            let p = if spec.trim { p.trim() } else { p };
            push_part(p, spec.match_case, &mut part);
        }
        // Length-prefixed, so ("a|", "b") and ("a", "|b") differ.
        out.push_str(&part.len().to_string());
        out.push(':');
        out.push_str(&part);
    }
    any
}

/// What a range key column holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeKind {
    Number,
    Date,
}

fn range_kind_of(s: &str, rd: crate::coltype::Reader) -> Option<RangeKind> {
    if rd.number(s).is_some_and(f64::is_finite) {
        Some(RangeKind::Number)
    } else if rd.date(s).is_some_and(f64::is_finite) {
        Some(RangeKind::Date)
    } else {
        None
    }
}

/// A range key of the given kind (a date as a day number).
fn range_value(s: &str, kind: RangeKind, rd: crate::coltype::Reader) -> Option<f64> {
    let v = match kind {
        RangeKind::Number => rd.number(s),
        // A plain number is not a date here, even if it could be read as one.
        RangeKind::Date => rd.date(s).filter(|_| rd.number(s).is_none()),
    };
    v.filter(|v| v.is_finite())
        .map(|v| if v == 0.0 { 0.0 } else { v })
}

fn check_cols(d: &Document, cols: &[u32]) -> Result<(), Error> {
    if cols.len() > MAX_JOIN_COLS || cols.iter().any(|&c| c >= d.col_count()) {
        return Err(Error::OutOfRange);
    }
    Ok(())
}

/// The other table's rows, copied out for a join, with its keys indexed.
/// The text is kept in one buffer (no allocation per cell).
pub struct JoinSource {
    titles: Vec<String>,
    /// Every cell of every row (key parts then wanted columns), end to end.
    text: String,
    /// Cell `k` of row `i` is `text[offs[i * width + k]..offs[i * width + k + 1]]`.
    offs: Vec<u64>,
    width: usize,
    nkeys: usize,
    /// The options the keys were indexed with.
    spec: JoinSpec,
    /// How the key column (for a nearest-value match) and each added column are read.
    key_rd: crate::coltype::Reader,
    rds: Vec<crate::coltype::Reader>,
    /// Per key: compared exactly (the key column is typed Id or Text, in either table).
    exact: Vec<bool>,
    lookup: HashMap<String, u32>,
    /// Groups of rows with the same key, in first-seen order:
    /// `members[starts[g]..starts[g + 1]]`.
    starts: Vec<u32>,
    members: Vec<u32>,
    /// Range lookups: (key value, group), sorted, and what the keys are.
    sorted: Vec<(f64, u32)>,
    range_kind: RangeKind,
    /// Per group, the values written (all repeats rules but first, last and
    /// every match).
    agg: Vec<Vec<String>>,
    repeated: u64,
    bad_keys: u64,
    /// Built from a pasted list (no other table).
    list: bool,
}

/// Whether two specs index the other table the same way.
fn same_index(a: &JoinSpec, b: &JoinSpec, list: bool) -> bool {
    (list
        || a.other_keys == b.other_keys
            && a.columns == b.columns
            && a.other_header == b.other_header)
        && a.match_case == b.match_case
        && a.trim == b.trim
        && a.loose == b.loose
        && a.key_match == b.key_match
        && a.repeats == b.repeats
        && a.separator == b.separator
}

/// Rows copied into one text buffer, chunk by chunk.
struct Flat {
    text: String,
    offs: Vec<u64>,
}

impl Flat {
    fn new() -> Flat {
        Flat {
            text: String::new(),
            offs: vec![0],
        }
    }

    fn push(&mut self, cell: &str) {
        self.text.push_str(cell);
        self.offs.push(self.text.len() as u64);
    }

    /// Append another chunk's cells.
    fn extend(&mut self, other: Flat) {
        let base = self.text.len() as u64;
        self.text.push_str(&other.text);
        self.offs.extend(other.offs[1..].iter().map(|o| o + base));
    }
}

impl JoinSource {
    /// The same table indexed with other exact-key flags (a key column typed Id or Text in the first
    /// table makes that key exact on both sides).
    fn reindexed(&self, exact: Vec<bool>) -> Result<JoinSource, Error> {
        let rows = (self.offs.len() - 1) / self.width.max(1);
        JoinSource::index(
            self.titles.clone(),
            Flat {
                text: self.text.clone(),
                offs: self.offs.clone(),
            },
            self.width,
            rows,
            &self.spec,
            self.key_rd,
            self.rds.clone(),
            exact,
        )
    }

    pub fn build(
        other: &Document,
        spec: &JoinSpec,
        progress: &Progress,
    ) -> Result<Arc<JoinSource>, Error> {
        other.ready_for_bulk()?;
        if spec.keys.is_empty() || spec.keys.len() != spec.other_keys.len() {
            return Err(Error::Invalid(
                "choose matching key columns in both tables".into(),
            ));
        }
        check_cols(other, &spec.other_keys)?;
        check_cols(other, &spec.columns)?;
        let cols: Vec<u32> = if spec.columns.is_empty() {
            (0..other.col_count())
                .filter(|c| !spec.other_keys.contains(c))
                .collect()
        } else {
            spec.columns.clone()
        };
        if cols.len() > MAX_JOIN_COLS {
            return Err(Error::Invalid(format!(
                "at most {MAX_JOIN_COLS} columns can be added"
            )));
        }
        let rows = other.data_rows(spec.other_header);
        let width = cols.len() + spec.keys.len();
        // The index keeps text plus one 8-byte offset a cell: the cell count
        // bounds the offsets (1 GB), the text is charged below.
        if (rows.end - rows.start).saturating_mul(width as u64) > MAX_INDEX_CELLS {
            return Err(Error::Invalid(
                "the other table is too large to join".into(),
            ));
        }
        // Text: as much as the other file holds (at least 1 GB, at most 4).
        let text_budget = crate::find::split_budget(other);
        let mut all = spec.other_keys.clone();
        all.extend_from_slice(&cols);
        let reader = Cols::new(other, &all);
        let size = AtomicUsize::new(0);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let chunks = ((r1 - r0) as usize).div_ceil(CHUNK);
        let parts: Vec<Flat> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Flat, Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut f = Flat {
                    text: String::new(),
                    offs: vec![0],
                };
                let mut buf = Vec::new();
                for r in s..e {
                    let id = other.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(other, id, &mut buf);
                    let before = f.text.len();
                    for k in 0..width {
                        f.push(reader.get(&buf, k));
                    }
                    let len = f.text.len() - before;
                    if size.fetch_add(len, Ordering::Relaxed) + len > text_budget {
                        return Err(Error::Invalid(
                            "the other table is too large to join".into(),
                        ));
                    }
                }
                progress.step(e - s)?;
                Ok(f)
            })
            .collect::<Result<_, _>>()?;
        let mut flat = Flat::new();
        flat.text.reserve(parts.iter().map(|p| p.text.len()).sum());
        for p in parts {
            flat.extend(p);
        }
        let n = (r1 - r0) as usize;
        let ctx = other.read_ctx(ReadOptions::default());
        let key_rd = ctx.reader(spec.other_keys.first().copied().unwrap_or(0));
        let rds: Vec<crate::coltype::Reader> = cols.iter().map(|&c| ctx.reader(c)).collect();
        JoinSource::index(
            other.titles(&cols, spec.other_header),
            flat,
            width,
            n,
            spec,
            key_rd,
            rds,
            spec.other_keys
                .iter()
                .map(|&c| ctx.type_of(c).is_text_like())
                .collect(),
        )
        .map(Arc::new)
    }

    /// A list of keys (one per line, parts split by tabs), for keeping or
    /// dropping the rows on a list, or counting them. Blank lines are left
    /// out before anything is stored.
    pub fn from_list(text: &str, spec: &JoinSpec) -> Result<Arc<JoinSource>, Error> {
        if spec.keys.is_empty() {
            return Err(Error::Invalid("choose the key column(s)".into()));
        }
        if text.len() > MAX_TABLE_BYTES {
            return Err(Error::Invalid("the list is too long".into()));
        }
        let nk = spec.keys.len();
        let mut flat = Flat::new();
        let mut rows = 0usize;
        for line in text.lines() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.split('\t');
            for _ in 0..nk {
                flat.push(parts.next().unwrap_or(""));
            }
            rows += 1;
            if (rows as u64).saturating_mul(nk as u64) > MAX_TABLE_CELLS {
                return Err(Error::Invalid("the list is too long".into()));
            }
        }
        let mut spec = spec.clone();
        spec.other_keys = (0..nk as u32).collect();
        spec.columns = Vec::new();
        spec.other_header = false;
        let mut src = JoinSource::index(
            Vec::new(),
            flat,
            nk,
            rows,
            &spec,
            crate::coltype::Reader::plain(ReadOptions::default()),
            Vec::new(),
            Vec::new(),
        )?;
        src.list = true;
        Ok(Arc::new(src))
    }

    #[allow(clippy::too_many_arguments)] // the table, its text and how its keys and columns are read
    fn index(
        titles: Vec<String>,
        flat: Flat,
        width: usize,
        rows: usize,
        spec: &JoinSpec,
        key_rd: crate::coltype::Reader,
        rds: Vec<crate::coltype::Reader>,
        exact: Vec<bool>,
    ) -> Result<JoinSource, Error> {
        let nkeys = spec.other_keys.len();
        let range = spec.key_match != KeyMatch::Exact;
        if range && nkeys != 1 {
            return Err(Error::Invalid(
                "a nearest-value match uses one key column".into(),
            ));
        }
        check_separator(&spec.separator)?;
        let mut src = JoinSource {
            titles,
            text: flat.text,
            offs: flat.offs,
            width,
            nkeys,
            spec: spec.clone(),
            key_rd,
            rds,
            exact: exact.clone(),
            lookup: HashMap::new(),
            starts: Vec::new(),
            members: Vec::new(),
            sorted: Vec::new(),
            range_kind: RangeKind::Number,
            agg: Vec::new(),
            repeated: 0,
            bad_keys: 0,
            list: false,
        };
        if range {
            // Numbers or dates, whichever most keys are; keys of the other
            // kind are not used (a date is never compared with a number).
            let (mut nums, mut dates) = (0u64, 0u64);
            for i in 0..rows {
                match range_kind_of(src.cell(i, 0), src.key_rd) {
                    Some(RangeKind::Number) => nums += 1,
                    Some(RangeKind::Date) => dates += 1,
                    None => {}
                }
            }
            src.range_kind = if dates > nums {
                RangeKind::Date
            } else {
                RangeKind::Number
            };
        }
        let mut lookup: HashMap<String, u32> = HashMap::with_capacity(rows);
        let mut group_of = vec![NO_GROUP; rows];
        let mut sizes: Vec<u32> = Vec::new();
        let mut key = String::new();
        let mut values: Vec<f64> = Vec::new();
        let (mut bad_keys, mut repeated) = (0u64, 0u64);
        for (i, slot) in group_of.iter_mut().enumerate() {
            let parts: Vec<&str> = (0..nkeys).map(|k| src.cell(i, k)).collect();
            if parts.iter().all(|p| p.trim().is_empty()) {
                continue;
            }
            if range {
                let Some(v) = range_value(parts[0], src.range_kind, src.key_rd) else {
                    bad_keys += 1;
                    continue;
                };
                key.clear();
                key.push_str(&format!("{:x}", v.to_bits()));
                if !lookup.contains_key(&key) {
                    values.push(v);
                }
            } else if !norm_key(&parts, spec, &exact, &mut key) {
                // A loose key with no letters or digits: blank.
                continue;
            }
            let g = match lookup.entry(key.clone()) {
                Entry::Occupied(o) => {
                    repeated += 1;
                    *o.get()
                }
                Entry::Vacant(v) => {
                    let g = sizes.len() as u32;
                    v.insert(g);
                    sizes.push(0);
                    g
                }
            };
            sizes[g as usize] += 1;
            *slot = g;
        }
        let mut starts = Vec::with_capacity(sizes.len() + 1);
        let mut at = 0u32;
        for s in &sizes {
            starts.push(at);
            at += s;
        }
        starts.push(at);
        let mut fill = starts.clone();
        let mut members = vec![0u32; at as usize];
        for (i, &g) in group_of.iter().enumerate() {
            if g != NO_GROUP {
                members[fill[g as usize] as usize] = i as u32;
                fill[g as usize] += 1;
            }
        }
        let mut sorted: Vec<(f64, u32)> = values
            .into_iter()
            .enumerate()
            .map(|(g, v)| (v, g as u32))
            .collect();
        sorted.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        src.bad_keys = bad_keys;
        src.repeated = repeated;
        src.lookup = lookup;
        src.starts = starts;
        src.members = members;
        src.sorted = sorted;
        src.agg = src.aggregate()?;
        Ok(src)
    }

    fn cell(&self, i: usize, k: usize) -> &str {
        let at = i * self.width + k;
        &self.text[self.offs[at] as usize..self.offs[at + 1] as usize]
    }

    /// Value `k` (after the keys) of row `i`.
    fn val(&self, i: usize, k: usize) -> &str {
        self.cell(i, self.nkeys + k)
    }

    fn group(&self, g: u32) -> &[u32] {
        let g = g as usize;
        &self.members[self.starts[g] as usize..self.starts[g + 1] as usize]
    }

    fn groups(&self) -> usize {
        self.starts.len() - 1
    }

    /// The written values per group for the repeats rules that combine.
    fn aggregate(&self) -> Result<Vec<Vec<String>>, Error> {
        let w = self.titles.len();
        let rule = self.spec.repeats;
        if matches!(rule, Repeats::First | Repeats::Last | Repeats::Every) {
            return Ok(Vec::new());
        }
        let size = AtomicUsize::new(0);
        (0..self.groups() as u32)
            .into_par_iter()
            .map(|g| -> Result<Vec<String>, Error> {
                let ms = self.group(g);
                if rule == Repeats::Count {
                    return Ok(vec![ms.len().to_string()]);
                }
                let mut out = Vec::with_capacity(w);
                for k in 0..w {
                    let vals = ms.iter().map(|&m| self.val(m as usize, k));
                    let v = match rule {
                        Repeats::Joined => {
                            let mut s = String::new();
                            for v in vals.filter(|v| !v.is_empty()) {
                                if !s.is_empty() {
                                    s.push_str(&self.spec.separator);
                                }
                                s.push_str(v);
                                if s.len() > MAX_JOINED_BYTES {
                                    return Err(Error::Invalid(
                                        "the matches joined in one cell would be too long".into(),
                                    ));
                                }
                            }
                            s
                        }
                        _ => {
                            let rd = self.rds.get(k).copied().unwrap_or(self.key_rd);
                            let nums = vals.filter_map(|v| rd.number(v));
                            let r = match rule {
                                Repeats::Sum => {
                                    let mut sum = crate::stats::Kahan::default();
                                    let mut any = false;
                                    for x in nums {
                                        sum.add(x);
                                        any = true;
                                    }
                                    any.then(|| sum.value())
                                }
                                Repeats::Min => nums.reduce(f64::min),
                                _ => nums.reduce(f64::max),
                            };
                            match r {
                                Some(x) if x.is_finite() => exact_number(x, false),
                                _ => String::new(),
                            }
                        }
                    };
                    let len = v.len() + 24;
                    if size.fetch_add(len, Ordering::Relaxed) + len > MAX_TABLE_BYTES {
                        return Err(Error::Invalid(
                            "the matches joined together would be too long".into(),
                        ));
                    }
                    out.push(v);
                }
                Ok(out)
            })
            .collect()
    }

    /// The group this table's key finds.
    fn find(&self, parts: &[&str], key: &mut String) -> Hit {
        if parts.iter().all(|p| p.trim().is_empty()) {
            return Hit::Blank;
        }
        let g = match self.spec.key_match {
            KeyMatch::Exact => {
                if !norm_key(parts, &self.spec, &self.exact, key) {
                    return Hit::Blank;
                }
                self.lookup.get(key.as_str()).copied()
            }
            KeyMatch::Below => range_value(parts[0], self.range_kind, self.key_rd).and_then(|v| {
                let i = self.sorted.partition_point(|x| x.0 <= v);
                i.checked_sub(1).map(|i| self.sorted[i].1)
            }),
            KeyMatch::Above => range_value(parts[0], self.range_kind, self.key_rd).and_then(|v| {
                let i = self.sorted.partition_point(|x| x.0 < v);
                self.sorted.get(i).map(|x| x.1)
            }),
        };
        g.map_or(Hit::Missing, Hit::Group)
    }

    /// Titles of the values written (before the prefix).
    fn out_titles(&self) -> Vec<String> {
        if self.spec.repeats == Repeats::Count {
            vec!["Matches".into()]
        } else {
            self.titles.clone()
        }
    }

    /// Value `k` written for group `g` (one row's worth; for "every", the
    /// first match).
    fn value(&self, g: u32, k: usize) -> &str {
        match self.spec.repeats {
            Repeats::First | Repeats::Every => self.val(self.group(g)[0] as usize, k),
            Repeats::Last => self.val(
                *self.group(g).last().expect("groups are not empty") as usize,
                k,
            ),
            _ => &self.agg[g as usize][k],
        }
    }

    /// Dates (true) or numbers: what a range lookup's keys are read as.
    pub fn range_dates(&self) -> bool {
        self.range_kind == RangeKind::Date
    }
}

fn check_separator(sep: &str) -> Result<(), Error> {
    crate::ops::check_separator(sep)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Blank,
    Missing,
    Group(u32),
}

/// What a join does to this table, as one undo step.
pub struct JoinPlan {
    pub(crate) revision: u64,
    /// New columns: (logical position, how many).
    pub(crate) insert: Option<(u32, u32)>,
    /// Cell changes by row id, in logical columns after the insert.
    pub(crate) changes: Vec<(u32, Vec<(u32, String)>)>,
    pub(crate) remove: Vec<u32>,
    /// Extra rows (every match), right after a row, in logical columns
    /// after the insert.
    pub(crate) after: Vec<(u32, Vec<Vec<String>>)>,
    pub(crate) append: Vec<Vec<String>>,
    /// Rows that get their group's values (or the not-found text, group
    /// [`NOT_FOUND`]) in the new columns. Kept as group numbers so a wide
    /// join on a big file does not copy every value twice; the cells are
    /// made when the plan is applied.
    pub(crate) filled: Vec<(u32, u32)>,
    pub(crate) src: Option<Arc<JoinSource>>,
    /// Where value k goes (logical, after the insert), for `filled`.
    pub(crate) dest: Vec<u32>,
    pub(crate) not_found: String,
    pub cells: u64,
}

/// Most cells a join may fill from the other table's groups (made when
/// applied, about 40 bytes each plus text): ten columns on 10M rows.
const MAX_FILLED_CELLS: u64 = 100_000_000;

/// Most cells (keys plus values) a join index of another table holds:
/// their offsets take at most 1 GB.
const MAX_INDEX_CELLS: u64 = 1 << 27;

/// The group number in [`JoinPlan::filled`] for rows that get the
/// not-found text.
const NOT_FOUND: u32 = u32::MAX;

impl std::fmt::Debug for JoinPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinPlan")
            .field("insert", &self.insert)
            .field("changes", &self.changes.len())
            .field("filled", &self.filled.len())
            .field("remove", &self.remove.len())
            .field("after", &self.after.len())
            .field("append", &self.append.len())
            .field("cells", &self.cells)
            .finish()
    }
}

impl JoinPlan {
    /// Columns the table has afterwards.
    pub fn width(&self, d: &Document) -> u32 {
        let base = d.col_count() + self.insert.map_or(0, |x| x.1);
        self.append
            .iter()
            .chain(self.after.iter().flat_map(|x| x.1.iter()))
            .map(|r| r.len() as u32)
            .chain(std::iter::once(base))
            .max()
            .unwrap_or(base)
    }

    pub fn is_empty(&self) -> bool {
        self.insert.is_none()
            && self.filled.is_empty()
            && self.changes.is_empty()
            && self.remove.is_empty()
            && self.after.is_empty()
            && self.append.is_empty()
    }
}

/// One row of a join preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewRow {
    /// "matched", "unmatched", "blank", "removed", "extra" or "added".
    pub kind: String,
    /// Row number here (1-based, shown rows), none for new rows.
    pub row: Option<u64>,
    pub values: Vec<String>,
    /// Columns whose value changes.
    pub changed: Vec<u32>,
}

/// The first rows as they would look after a join, and the full counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinPreview {
    pub titles: Vec<String>,
    /// New or updated columns.
    pub columns: Vec<u32>,
    pub rows: Vec<PreviewRow>,
    pub report: JoinReport,
}

/// What happens to one of this table's rows.
struct RowOut {
    id: u32,
    pos: u64,
    hit: Hit,
    remove: bool,
    set: Vec<(u32, String)>,
    /// Filled from a group (or [`NOT_FOUND`]) when the plan is applied.
    group: Option<u32>,
    extra: Vec<Vec<String>>,
}

fn clip(s: &str) -> String {
    match s.char_indices().nth(PREVIEW_CHARS) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

impl Document {
    /// The exact-key flags this join needs when they differ from the source's (a key typed Id or Text in
    /// either table is exact on both sides), else `None`.
    fn aligned_exact(&self, src: &JoinSource, spec: &JoinSpec) -> Option<Vec<bool>> {
        let ctx = self.read_ctx(ReadOptions::default());
        let want: Vec<bool> = spec
            .keys
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                ctx.type_of(c).is_text_like() || src.exact.get(i).copied().unwrap_or(false)
            })
            .collect();
        (want != src.exact && !src.list).then_some(want)
    }

    /// Plan a join (see [`JoinSpec`]).
    pub fn plan_join(
        &self,
        src: &Arc<JoinSource>,
        spec: &JoinSpec,
        progress: &Progress,
    ) -> Result<(JoinPlan, JoinReport), Error> {
        let aligned;
        let src = match self.aligned_exact(src, spec) {
            Some(e) => {
                aligned = Arc::new(src.reindexed(e)?);
                &aligned
            }
            None => src,
        };
        let (mut plan, report, _) = self.join_walk(src, spec, progress, None)?;
        if !plan.filled.is_empty() {
            plan.src = Some(src.clone());
        }
        if plan.filled.is_empty()
            && plan.changes.is_empty()
            && plan.remove.is_empty()
            && plan.after.is_empty()
            && plan.append.is_empty()
        {
            return Err(Error::Invalid(match spec.keep_rows {
                KeepRows::All => "no keys matched and there is nothing to add".into(),
                _ => "no rows would change".into(),
            }));
        }
        Ok((plan, report))
    }

    /// The first [`PREVIEW_ROWS`] shown rows as they would look after the
    /// join, plus the counts for the whole table.
    pub fn preview_join(
        &self,
        src: &JoinSource,
        spec: &JoinSpec,
        progress: &Progress,
    ) -> Result<JoinPreview, Error> {
        let aligned;
        let src = match self.aligned_exact(src, spec) {
            Some(e) => {
                aligned = src.reindexed(e)?;
                &aligned
            }
            None => src,
        };
        let (plan, report, outs) = self.join_walk(src, spec, progress, Some(PREVIEW_ROWS))?;
        let (at, n) = plan.insert.unwrap_or((u32::MAX, 0));
        let width = (self.col_count() + n).min(PREVIEW_COLS as u32);
        let shift = |c: u32| if c >= at { c + n } else { c };
        let all: Vec<u32> = (0..self.col_count()).collect();
        let mut titles = vec![String::new(); width as usize];
        for (c, t) in self.titles(&all, spec.keep_first).into_iter().enumerate() {
            let c = shift(c as u32);
            if c < width {
                titles[c as usize] = t;
            }
        }
        let new_titles: Vec<String> = src
            .out_titles()
            .into_iter()
            .map(|t| format!("{}{t}", spec.prefix))
            .collect();
        let mut columns: Vec<u32> = match spec.write {
            JoinWrite::New if n > 0 => (at..at + n).collect(),
            JoinWrite::Overwrite | JoinWrite::FillBlanks => spec.targets.clone(),
            _ => Vec::new(),
        };
        for (k, t) in new_titles.iter().enumerate() {
            if n > 0 && at + (k as u32) < width {
                titles[(at as usize) + k] = t.clone();
            }
        }
        columns.retain(|&c| c < width);
        let row_of = |values: &[String], set: &[(u32, String)]| -> (Vec<String>, Vec<u32>) {
            let mut out = vec![String::new(); width as usize];
            for (c, v) in values.iter().enumerate() {
                let c = shift(c as u32);
                if c < width {
                    out[c as usize] = clip(v);
                }
            }
            let mut changed = Vec::new();
            for (c, v) in set {
                if *c < width {
                    out[*c as usize] = clip(v);
                    changed.push(*c);
                }
            }
            (out, changed)
        };
        let mut rows = Vec::new();
        for o in outs {
            let values = self.row_values(o.pos)?;
            let kind = if o.remove {
                "removed"
            } else {
                match o.hit {
                    Hit::Group(_) => "matched",
                    Hit::Missing => "unmatched",
                    Hit::Blank => "blank",
                }
            };
            let (vals, changed) = row_of(&values, if o.remove { &[] } else { &o.set });
            rows.push(PreviewRow {
                kind: kind.into(),
                row: Some(o.pos + 1),
                values: vals,
                changed,
            });
            for e in o.extra {
                let mut v: Vec<String> = e.iter().take(width as usize).map(|s| clip(s)).collect();
                v.resize(width as usize, String::new());
                rows.push(PreviewRow {
                    kind: "extra".into(),
                    row: None,
                    values: v,
                    changed: columns.clone(),
                });
            }
        }
        for a in plan.append.iter().take(PREVIEW_ROWS) {
            let mut v: Vec<String> = a.iter().take(width as usize).map(|s| clip(s)).collect();
            v.resize(width as usize, String::new());
            rows.push(PreviewRow {
                kind: "added".into(),
                row: None,
                values: v,
                changed: columns.clone(),
            });
        }
        Ok(JoinPreview {
            titles,
            columns,
            rows,
            report,
        })
    }

    /// The join's work. With `limit`, rows past the first `limit` shown
    /// data rows are counted but not planned, and the planned rows are
    /// also returned for the preview.
    fn join_walk(
        &self,
        src: &JoinSource,
        spec: &JoinSpec,
        progress: &Progress,
        limit: Option<usize>,
    ) -> Result<(JoinPlan, JoinReport, Vec<RowOut>), Error> {
        self.ready_for_bulk()?;
        check_cols(self, &spec.keys)?;
        if spec.keys.len() != src.nkeys || !same_index(&src.spec, spec, src.list) {
            return Err(Error::Invalid(
                "choose matching key columns in both tables".into(),
            ));
        }
        if spec.add_unmatched && spec.key_match != KeyMatch::Exact {
            return Err(Error::Invalid(
                "rows that matched nothing can only be added with an exact match".into(),
            ));
        }
        let out_w = src.out_titles().len() as u32;
        let writes = !(spec.write == JoinWrite::Nothing || spec.keep_rows == KeepRows::Unmatched);
        if writes && out_w == 0 {
            return Err(Error::Invalid(
                "the other table has no columns to bring in".into(),
            ));
        }
        let update = matches!(spec.write, JoinWrite::Overwrite | JoinWrite::FillBlanks);
        if update && writes {
            if spec.targets.len() != out_w as usize {
                return Err(Error::Invalid(format!(
                    "choose a column here for each of the {out_w} value(s) brought in"
                )));
            }
            check_cols(self, &spec.targets)?;
            let mut t = spec.targets.clone();
            t.sort_unstable();
            t.dedup();
            if t.len() != spec.targets.len() {
                return Err(Error::Invalid(
                    "each column can be updated only once".into(),
                ));
            }
            if spec.targets.iter().any(|c| spec.keys.contains(c)) {
                return Err(Error::Invalid("a key column cannot also be updated".into()));
            }
        }
        if spec.repeats == Repeats::Every && !writes {
            return Err(Error::Invalid(
                "every match as its own row needs values to bring in".into(),
            ));
        }
        let insert = (writes && spec.write == JoinWrite::New).then(|| {
            let at = match spec.place {
                Place::End => self.col_count(),
                Place::AfterKey => spec.keys.iter().max().map_or(0, |k| k + 1),
            };
            (at, out_w)
        });
        if let Some((_, n)) = insert {
            if self.col_count() as u64 + n as u64 > crate::document::MAX_MAPPED_COLS as u64 {
                return Err(Error::OutOfRange);
            }
        }
        let (at, n) = insert.unwrap_or((u32::MAX, 0));
        let shift = |c: u32| if c >= at { c + n } else { c };
        // Where value k goes (logical, after the insert).
        let dest: Vec<u32> = if !writes {
            Vec::new()
        } else if update {
            spec.targets.clone()
        } else {
            (at..at + n).collect()
        };
        let mut plan = JoinPlan {
            revision: self.revision(),
            insert,
            changes: Vec::new(),
            remove: Vec::new(),
            after: Vec::new(),
            append: Vec::new(),
            filled: Vec::new(),
            src: None,
            dest: dest.clone(),
            not_found: spec.not_found.clone(),
            cells: 0,
        };
        let mut report = JoinReport {
            repeated_keys: src.repeated,
            bad_keys: src.bad_keys,
            cols: n,
            ..Default::default()
        };
        if spec.keep_first && self.row_count() > 0 && n > 0 {
            let id = self.id_at(0).ok_or(Error::OutOfRange)?;
            let cells = src
                .out_titles()
                .into_iter()
                .enumerate()
                .map(|(k, t)| (at + k as u32, format!("{}{t}", spec.prefix)))
                .collect();
            plan.changes.push((id, cells));
        }
        let nk = spec.keys.len();
        let mut read = spec.keys.clone();
        if update && writes {
            read.extend_from_slice(&spec.targets);
        }
        let reader = Cols::new(self, &read);
        let width = self.col_count();
        let all_cols: Vec<u32> = (0..width).collect();
        let whole = Cols::new(self, &all_cols);
        let used: Vec<AtomicBool> = (0..src.groups()).map(|_| AtomicBool::new(false)).collect();
        let matched = AtomicU64::new(0);
        let unmatched = AtomicU64::new(0);
        let blank = AtomicU64::new(0);
        let removed = AtomicU64::new(0);
        let changed = AtomicU64::new(0);
        let extras = AtomicU64::new(0);
        let total = AtomicU64::new(0);
        let filled_cells = AtomicU64::new(0);
        let data = self.data_rows(spec.keep_first);
        let first = data.start;
        let not_found = !spec.not_found.is_empty() && n > 0;
        // The cells written to a row whose current values (of `dest`, for
        // updates) are `cur`.
        // Every value copied into the plan is charged (text plus
        // bookkeeping) before it is made, so a big cell matched many times
        // is refused early instead of filling memory.
        let budget = crate::find::split_budget(self);
        let spent = AtomicUsize::new(0);
        let charge = |len: usize| -> Result<(), Error> {
            if crate::find::charge_within(&spent, len, budget) {
                Ok(())
            } else {
                Err(Error::OutOfMemory(
                    "this join would copy too much text at once; bring in fewer or shorter columns, or choose another rule for repeated keys",
                ))
            }
        };
        // Cells filled from a group are made only when the plan is applied,
        // so they cost no bookkeeping here; their text still counts, so one
        // big value copied into many rows is refused.
        let filled_text = AtomicUsize::new(0);
        let fill = |len: usize| -> Result<(), Error> {
            if filled_text
                .fetch_add(len, Ordering::Relaxed)
                .saturating_add(len)
                > budget
            {
                Err(Error::OutOfMemory(
                    "this join would copy too much text at once; bring in fewer or shorter columns, or choose another rule for repeated keys",
                ))
            } else {
                Ok(())
            }
        };
        let cells_for =
            |vals: &[&str], cur: &dyn Fn(usize) -> String| -> Result<Vec<(u32, String)>, Error> {
                let mut set = Vec::new();
                for (k, &c) in dest.iter().enumerate() {
                    let v = vals[k];
                    if v.is_empty() {
                        continue;
                    }
                    let write = match spec.write {
                        JoinWrite::New => true,
                        JoinWrite::Overwrite => cur(k) != v,
                        _ => cur(k).trim().is_empty() && cur(k) != v,
                    };
                    if write {
                        charge(v.len())?;
                        set.push((c, v.to_string()));
                    }
                }
                Ok(set)
            };
        let outs: Vec<RowOut> = self.par_rows(data, progress, |pos, id, buf| {
            reader.read(self, id, buf);
            let parts: Vec<&str> = (0..nk).map(|k| reader.get(buf, k)).collect();
            let mut key = String::new();
            let hit = src.find(&parts, &mut key);
            match hit {
                Hit::Group(g) => {
                    matched.fetch_add(1, Ordering::Relaxed);
                    used[g as usize].store(true, Ordering::Relaxed);
                }
                Hit::Missing => {
                    unmatched.fetch_add(1, Ordering::Relaxed);
                }
                Hit::Blank => {
                    blank.fetch_add(1, Ordering::Relaxed);
                }
            }
            let past = limit.is_some_and(|l| pos - first >= l as u64);
            let is_match = matches!(hit, Hit::Group(_));
            let remove = match spec.keep_rows {
                KeepRows::All => false,
                KeepRows::Matched => !is_match,
                KeepRows::Unmatched => is_match,
            };
            let mut out = RowOut {
                id,
                pos,
                hit,
                remove,
                set: Vec::new(),
                group: None,
                extra: Vec::new(),
            };
            if remove {
                removed.fetch_add(1, Ordering::Relaxed);
            }
            if remove || !writes {
                return Ok((!past && (remove || limit.is_some())).then_some(out));
            }
            let current = |k: usize| reader.get(buf, nk + k).to_string();
            // New columns get the same values for every row of a group, so
            // outside a preview only the group number is kept.
            let by_group = !update && spec.repeats != Repeats::Every && (past || limit.is_none());
            let mut n_set = 0u64;
            match hit {
                Hit::Group(g) if by_group => {
                    let mut len = 0;
                    for k in 0..dest.len() {
                        let v = src.value(g, k);
                        if !v.is_empty() {
                            n_set += 1;
                            len += v.len();
                        }
                    }
                    fill(len)?;
                    if n_set > 0 {
                        out.group = Some(g);
                    }
                }
                _ if by_group && not_found && !matches!(hit, Hit::Group(_)) => {
                    n_set = dest.len() as u64;
                    fill(spec.not_found.len() * dest.len())?;
                    out.group = Some(NOT_FOUND);
                }
                Hit::Group(g) => {
                    let vals: Vec<&str> = (0..dest.len()).map(|k| src.value(g, k)).collect();
                    out.set = cells_for(&vals, &current)?;
                    if spec.repeats == Repeats::Every && src.group(g).len() > 1 {
                        let mut row_buf = Vec::new();
                        whole.read(self, id, &mut row_buf);
                        let row_len: usize = (0..width as usize)
                            .map(|c| whole.get(&row_buf, c).len() + crate::find::CHANGE_COST)
                            .sum();
                        let mut base = vec![String::new(); (width + n) as usize];
                        for c in 0..width {
                            base[shift(c) as usize] = whole.get(&row_buf, c as usize).to_string();
                        }
                        for &m in &src.group(g)[1..] {
                            charge(row_len)?;
                            let mut row = base.clone();
                            let cur = |k: usize| base[dest[k] as usize].clone();
                            let vals: Vec<&str> =
                                (0..dest.len()).map(|k| src.val(m as usize, k)).collect();
                            for (c, v) in cells_for(&vals, &cur)? {
                                row[c as usize] = v;
                            }
                            while row.last().is_some_and(String::is_empty) && row.len() > 1 {
                                row.pop();
                            }
                            out.extra.push(row);
                        }
                    }
                }
                _ if not_found => {
                    for &c in &dest {
                        charge(spec.not_found.len())?;
                        out.set.push((c, spec.not_found.clone()));
                    }
                }
                _ => {}
            }
            // Cells filled from a group (n_set so far) are made when the
            // plan is applied and have their own, larger limit.
            if n_set > 0
                && filled_cells.fetch_add(n_set, Ordering::Relaxed) + n_set > MAX_FILLED_CELLS
            {
                return Err(Error::Invalid(
                    "the join would change too many cells".into(),
                ));
            }
            let n_cells =
                out.set.len() as u64 + out.extra.iter().map(|r| r.len() as u64).sum::<u64>();
            let n_set = n_set + out.set.len() as u64;
            changed.fetch_add(n_set, Ordering::Relaxed);
            extras.fetch_add(out.extra.len() as u64, Ordering::Relaxed);
            if total.fetch_add(n_cells, Ordering::Relaxed) + n_cells > MAX_TABLE_CELLS {
                return Err(Error::Invalid(
                    "the join would change too many cells".into(),
                ));
            }
            if past {
                return Ok(None);
            }
            Ok((limit.is_some()
                || out.group.is_some()
                || !out.set.is_empty()
                || !out.extra.is_empty())
            .then_some(out))
        })?;
        report.matched = matched.into_inner();
        report.unmatched = unmatched.into_inner();
        report.blank_keys = blank.into_inner();
        report.removed = removed.into_inner();
        report.changed = changed.into_inner();
        report.extra_rows = extras.into_inner();
        if report.extra_rows > u32::MAX as u64 / 2 {
            return Err(Error::TooManyRows);
        }
        let mut cells = total.into_inner();
        if spec.add_unmatched && writes {
            let used = self.used_with_hidden(src, spec, used, progress)?;
            let final_w = (width + n) as usize;
            for g in 0..src.groups() as u32 {
                if used[g as usize] {
                    continue;
                }
                let members: Vec<u32> = if spec.repeats == Repeats::Every {
                    src.group(g).to_vec()
                } else {
                    vec![src.group(g)[0]]
                };
                for m in members {
                    let m = m as usize;
                    let mut line = vec![String::new(); final_w];
                    for (k, &c) in spec.keys.iter().enumerate() {
                        charge(src.cell(m, k).len())?;
                        line[shift(c) as usize] = src.cell(m, k).to_string();
                    }
                    for (k, &c) in dest.iter().enumerate() {
                        let v = if spec.repeats == Repeats::Every {
                            src.val(m, k)
                        } else {
                            src.value(g, k)
                        };
                        charge(v.len())?;
                        line[c as usize] = v.to_string();
                    }
                    while line.last().is_some_and(String::is_empty) && line.len() > 1 {
                        line.pop();
                    }
                    cells += line.len() as u64;
                    if cells > MAX_TABLE_CELLS {
                        return Err(Error::Invalid("the join would add too many cells".into()));
                    }
                    report.added += 1;
                    if limit.is_none_or(|l| plan.append.len() < l) {
                        plan.append.push(line);
                    }
                }
            }
        }
        plan.cells = cells + filled_cells.into_inner();
        let mut planned = Vec::new();
        for o in outs {
            if limit.is_some() {
                planned.push(o);
                continue;
            }
            if o.remove {
                plan.remove.push(o.id);
                continue;
            }
            if let Some(g) = o.group {
                plan.filled.push((o.id, g));
            }
            if !o.set.is_empty() {
                plan.changes.push((o.id, o.set));
            }
            if !o.extra.is_empty() {
                plan.after.push((o.id, o.extra));
            }
        }
        Ok((plan, report, planned))
    }

    /// Groups matched by any row, hidden ones included: a key held only by
    /// a hidden row is not new either.
    fn used_with_hidden(
        &self,
        src: &JoinSource,
        spec: &JoinSpec,
        used: Vec<AtomicBool>,
        progress: &Progress,
    ) -> Result<Vec<bool>, Error> {
        let mut used: Vec<bool> = used.into_iter().map(AtomicBool::into_inner).collect();
        if !self.is_filtered() {
            return Ok(used);
        }
        let header = if spec.keep_first { self.id_at(0) } else { None };
        let reader = Cols::new(self, &spec.keys);
        let nk = spec.keys.len();
        let hits: Vec<u32> = self.par_all_rows(0, progress, |_, id, buf| {
            if Some(id) == header {
                return Ok(None);
            }
            reader.read(self, id, buf);
            let parts: Vec<&str> = (0..nk).map(|k| reader.get(buf, k)).collect();
            let mut key = String::new();
            Ok(match src.find(&parts, &mut key) {
                Hit::Group(g) => Some(g),
                _ => None,
            })
        })?;
        for g in hits {
            used[g as usize] = true;
        }
        Ok(used)
    }

    /// Apply a [`JoinPlan`] as one undo step. Fails with
    /// [`Error::Changed`] if the document changed since it was planned.
    pub fn apply_join(&mut self, plan: JoinPlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        self.ready_for_bulk()?;
        let insert = match plan.insert {
            Some((at, n)) if n > 0 => Some((at, self.new_col_ids(n)?)),
            _ => None,
        };
        let phys = |d: &Document, c: u32| -> Result<u32, Error> {
            let mut c = c;
            if let Some((at, ids)) = &insert {
                let n = ids.len() as u32;
                if c >= *at && c < at + n {
                    return Ok(ids[(c - at) as usize]);
                }
                if c >= at + n {
                    c -= n;
                }
            }
            d.phys(c)
                .filter(|_| c < d.col_count())
                .ok_or(Error::OutOfRange)
        };
        let full = |d: &Document, row: Vec<String>| -> Result<Vec<String>, Error> {
            let mut cells = Vec::with_capacity(row.len());
            let mut w = 0;
            for (c, v) in row.into_iter().enumerate() {
                if !v.is_empty() {
                    let p = phys(d, c as u32)?;
                    w = w.max(p + 1);
                    cells.push((p, v));
                }
            }
            let mut out = vec![String::new(); w as usize];
            for (p, v) in cells {
                out[p as usize] = v;
            }
            Ok(out)
        };
        let mut changes = Vec::with_capacity(plan.changes.len() + plan.filled.len());
        for (id, cells) in plan.changes {
            let mut out = Vec::with_capacity(cells.len());
            for (c, v) in cells {
                out.push((phys(self, c)?, v));
            }
            changes.push((id, out));
        }
        if !plan.filled.is_empty() {
            let src = plan.src.as_ref().ok_or(Error::Changed)?;
            let dest: Vec<u32> = plan
                .dest
                .iter()
                .map(|&c| phys(self, c))
                .collect::<Result<_, _>>()?;
            let made: Vec<(u32, Vec<(u32, String)>)> = plan
                .filled
                .par_iter()
                .map(|&(id, g)| {
                    let cells = dest
                        .iter()
                        .enumerate()
                        .filter_map(|(k, &c)| {
                            let v = if g == NOT_FOUND {
                                plan.not_found.as_str()
                            } else {
                                src.value(g, k)
                            };
                            (!v.is_empty()).then(|| (c, v.to_string()))
                        })
                        .collect();
                    (id, cells)
                })
                .collect();
            changes.extend(made);
        }
        let mut states = self.patch_states(changes)?;
        let base = self.next_row_id() as usize;
        let mut gone = vec![0u64; base.div_ceil(64)];
        for &id in &plan.remove {
            gone[(id >> 6) as usize] |= 1 << (id & 63);
            if self.is_edited(id) {
                states.push((id, None));
            }
        }
        let is_gone =
            |id: u32| (id as usize) < base && (gone[(id >> 6) as usize] >> (id & 63)) & 1 == 1;
        let new_rows: u64 = plan.after.iter().map(|(_, r)| r.len() as u64).sum();
        let first = self.new_row_ids(u32::try_from(new_rows).map_err(|_| Error::TooManyRows)?)?;
        let mut next = first;
        let mut after: HashMap<u32, (u32, u32)> = HashMap::with_capacity(plan.after.len());
        for (id, rows) in plan.after {
            let n = rows.len() as u32;
            after.insert(id, (next, n));
            for r in rows {
                states.push((next, Some(Edit::Full(full(self, r)?))));
                next += 1;
            }
        }
        let order = if plan.remove.is_empty() && after.is_empty() {
            None
        } else {
            let mut o = Vec::with_capacity(self.total_rows() as usize + new_rows as usize);
            for id in self.ids() {
                if is_gone(id) {
                    continue;
                }
                o.push(id);
                if let Some(&(s, n)) = after.get(&id) {
                    o.extend(s..s + n);
                }
            }
            Some(o)
        };
        let app_first =
            self.new_row_ids(u32::try_from(plan.append.len()).map_err(|_| Error::TooManyRows)?)?;
        let mut append = Vec::with_capacity(plan.append.len());
        for (i, r) in plan.append.into_iter().enumerate() {
            append.push((app_first + i as u32, Some(Edit::Full(full(self, r)?))));
        }
        self.push_step(Step {
            col_insert: insert,
            states,
            order,
            append,
            ..Default::default()
        });
        Ok(())
    }

    /// Plan an append: the other table's rows added at the end.
    pub fn plan_append(
        &self,
        src: &AppendSource,
        spec: &AppendSpec,
    ) -> Result<(CellPlan, JoinReport), Error> {
        self.ready_for_bulk()?;
        let mut plan = CellPlan::new(self);
        let mut report = JoinReport::default();
        let have = self.col_count();
        // Where each of the other table's columns goes.
        let mut target: Vec<Option<u32>> = Vec::with_capacity(src.titles.len());
        let mut new_titles: Vec<(u32, String)> = Vec::new();
        if spec.by_name {
            let cols: Vec<u32> = (0..have).collect();
            let mine = self.titles(&cols, spec.keep_first);
            let mut exact: HashMap<&str, u32> = HashMap::new();
            let mut loose: HashMap<String, u32> = HashMap::new();
            for (i, t) in mine.iter().enumerate().rev() {
                exact.insert(t.as_str(), i as u32);
                loose.insert(t.trim().to_lowercase(), i as u32);
            }
            let mut next = have;
            for t in &src.titles {
                let hit = exact
                    .get(t.as_str())
                    .or_else(|| loose.get(&t.trim().to_lowercase()))
                    .copied();
                match hit {
                    Some(c) => target.push(Some(c)),
                    None if spec.add_columns => {
                        if (next - have) as usize >= MAX_JOIN_COLS {
                            return Err(Error::Invalid(format!(
                                "at most {MAX_JOIN_COLS} columns can be added"
                            )));
                        }
                        target.push(Some(next));
                        new_titles.push((next, t.clone()));
                        next += 1;
                    }
                    None => {
                        target.push(None);
                        report.dropped_cols += 1;
                    }
                }
            }
            report.cols = next - have;
        } else {
            target.extend((0..src.titles.len() as u32).map(Some));
            report.cols = (src.titles.len() as u32).saturating_sub(have);
        }
        if spec.keep_first && !new_titles.is_empty() && self.row_count() > 0 {
            let id = self.id_at(0).ok_or(Error::OutOfRange)?;
            plan.changes.push((id, new_titles));
        }
        let mut cells = 0u64;
        for row in &src.rows {
            let width = row
                .iter()
                .enumerate()
                .filter_map(|(k, _)| target.get(k).copied().flatten())
                .max()
                .map_or(1, |c| c as usize + 1);
            let mut line = vec![String::new(); width];
            for (k, v) in row.iter().enumerate() {
                if let Some(Some(c)) = target.get(k) {
                    line[*c as usize] = v.clone();
                }
            }
            cells += line.len() as u64;
            if cells > MAX_TABLE_CELLS {
                return Err(Error::Invalid("the append would add too many cells".into()));
            }
            plan.append.push(line);
        }
        report.added = plan.append.len() as u64;
        if plan.append.is_empty() {
            return Err(Error::Invalid("the other table has no rows to add".into()));
        }
        plan.cells = cells;
        Ok((plan, report))
    }
}

/// The other table's rows, copied out for an append.
pub struct AppendSource {
    titles: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl AppendSource {
    pub fn build(
        other: &Document,
        spec: &AppendSpec,
        progress: &Progress,
    ) -> Result<AppendSource, Error> {
        other.ready_for_bulk()?;
        let cols: Vec<u32> = (0..other.col_count()).collect();
        let rows = other.data_rows(spec.other_header);
        if (rows.end - rows.start).saturating_mul(cols.len() as u64) > MAX_TABLE_CELLS {
            return Err(Error::Invalid(
                "the other table is too large to append".into(),
            ));
        }
        let reader = Cols::new(other, &cols);
        let n = cols.len();
        let size = std::sync::atomic::AtomicUsize::new(0);
        let rows = other.par_rows(rows, progress, |_, id, buf| {
            reader.read(other, id, buf);
            // Trailing empty cells are not kept.
            let w = (0..n)
                .rposition(|k| !reader.get(buf, k).is_empty())
                .map_or(0, |i| i + 1);
            let row: Vec<String> = (0..w.max(1))
                .map(|k| reader.get(buf, k).to_string())
                .collect();
            let len: usize = row.iter().map(String::len).sum();
            if size.fetch_add(len, Ordering::Relaxed) + len > MAX_TABLE_BYTES {
                return Err(Error::Invalid(
                    "the other table is too large to append".into(),
                ));
            }
            Ok(Some(row))
        })?;
        Ok(AppendSource {
            titles: other.titles(&cols, spec.other_header),
            rows,
        })
    }
}

/// A lookup table for the `LOOKUP` function of computed columns: the other
/// table's key column and one value column, copied out.
#[derive(Debug)]
pub struct LookupTable {
    map: HashMap<String, String>,
}

impl LookupTable {
    /// Keys compared without case and outer spaces, numbers by value
    /// (00123 = 123); the first row with a key wins.
    pub fn build(
        other: &Document,
        key: u32,
        value: u32,
        header: bool,
        progress: &Progress,
    ) -> Result<LookupTable, Error> {
        other.ready_for_bulk()?;
        check_cols(other, &[key, value])?;
        let reader = Cols::new(other, &[key, value]);
        let rows = other.data_rows(header);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let size = AtomicUsize::new(0);
        let chunks = ((r1 - r0) as usize).div_ceil(CHUNK);
        // Each chunk keeps its own first value per key; merged in order, so
        // the first row with a key wins. Only the two columns are copied.
        let parts: Vec<HashMap<String, String>> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<HashMap<String, String>, Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut m: HashMap<String, String> = HashMap::new();
                let mut buf = Vec::new();
                let mut k = String::new();
                for r in s..e {
                    let id = other.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(other, id, &mut buf);
                    let kv = reader.get(&buf, 0);
                    if kv.trim().is_empty() {
                        continue;
                    }
                    lookup_key(kv, &mut k);
                    if m.contains_key(&k) {
                        continue;
                    }
                    let v = reader.get(&buf, 1);
                    let len = k.len() + v.len() + 64;
                    if size.fetch_add(len, Ordering::Relaxed) + len > MAX_TABLE_BYTES {
                        return Err(Error::Invalid("the lookup tab is too large".into()));
                    }
                    m.insert(k.clone(), v.to_string());
                }
                progress.step(e - s)?;
                Ok(m)
            })
            .collect::<Result<_, _>>()?;
        let mut map: HashMap<String, String> = HashMap::new();
        for p in parts {
            if map.is_empty() {
                map = p;
                continue;
            }
            for (k, v) in p {
                map.entry(k).or_insert(v);
            }
        }
        Ok(LookupTable { map })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        let mut k = String::new();
        lookup_key(key, &mut k);
        self.map.get(&k).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

fn lookup_key(s: &str, out: &mut String) {
    out.clear();
    let t = s.trim();
    match canonical_decimal(t) {
        Some(n) => out.push_str(&n),
        None => out.extend(t.chars().flat_map(char::to_lowercase)),
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

    fn join(a: &mut Document, b: &Document, spec: &JoinSpec) -> JoinReport {
        let src = JoinSource::build(b, spec, &Progress::new()).unwrap();
        let (plan, rep) = a.plan_join(&src, spec, &Progress::new()).unwrap();
        a.apply_join(plan).unwrap();
        rep
    }

    fn base() -> JoinSpec {
        JoinSpec {
            keys: vec![0],
            other_keys: vec![0],
            keep_first: true,
            other_header: true,
            match_case: true,
            ..Default::default()
        }
    }

    #[test]
    fn join_adds_columns_by_key() {
        let mut a = doc("id,name\r\n1,Ann\r\n2,Bob\r\n3,Cy\r\n");
        let b = doc("code,city,id\r\n2,Rome,x\r\n1,Oslo,y\r\n1,Dup,z\r\n9,Lima,w\r\n");
        let before = all(&a);
        let spec = JoinSpec {
            add_unmatched: true,
            ..base()
        };
        let rep = join(&mut a, &b, &spec);
        assert_eq!(
            rep,
            JoinReport {
                matched: 2,
                unmatched: 1,
                added: 1,
                repeated_keys: 1,
                cols: 2,
                changed: 4,
                ..Default::default()
            }
        );
        assert_eq!(
            all(&a),
            vec![
                vec!["id", "name", "city", "id"],
                vec!["1", "Ann", "Oslo", "y"],
                vec!["2", "Bob", "Rome", "x"],
                vec!["3", "Cy"],
                vec!["9", "", "Lima", "w"],
            ]
        );
        assert!(a.undo());
        assert_eq!(all(&a), before);
    }

    #[test]
    fn join_respects_filters_case_and_trim() {
        let mut a = doc("k,v\r\nA,1\r\nb ,2\r\nc,3\r\n");
        use crate::filter::{ColumnFilter, FilterSpec, Rule};
        // Hide the row "b ,2".
        a.set_filter(&FilterSpec {
            columns: vec![ColumnFilter {
                col: 1,
                rule: Rule::Values {
                    values: vec!["1".into(), "3".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        let b = doc("k,w\r\na,x\r\nB,y\r\nc,z\r\n");
        let spec = JoinSpec {
            match_case: false,
            trim: true,
            ..base()
        };
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        let (plan, rep) = a.plan_join(&src, &spec, &Progress::new()).unwrap();
        // Not matching case: "A" finds "a", but the hidden row is left alone.
        assert_eq!((rep.matched, rep.unmatched), (2, 0));
        a.apply_join(plan).unwrap();
        a.clear_filter();
        let rows = all(&a);
        assert_eq!(rows[0], vec!["k", "v", "w"]);
        assert_eq!(rows[1], vec!["A", "1", "x"]);
        assert_eq!(rows[2], vec!["b ", "2"]);
        assert_eq!(rows[3], vec!["c", "3", "z"]);
    }

    /// Review notes: blank keys match nothing; a key held only by a hidden
    /// row is not added again as a new row.
    #[test]
    fn blank_keys_and_hidden_rows() {
        use crate::filter::{ColumnFilter, FilterSpec, Rule};
        let mut a = doc("k,v\r\n,1\r\nb,2\r\nc,3\r\n");
        let b = doc("k,w\r\n,blank\r\nb,x\r\nc,y\r\nd,z\r\n");
        // Hide row "b,2".
        a.set_filter(&FilterSpec {
            columns: vec![ColumnFilter {
                col: 1,
                rule: Rule::Values {
                    values: vec!["1".into(), "3".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        let spec = JoinSpec {
            match_case: false,
            add_unmatched: true,
            ..base()
        };
        let rep = join(&mut a, &b, &spec);
        assert_eq!(
            (rep.matched, rep.unmatched, rep.blank_keys, rep.added),
            (1, 0, 1, 1)
        );
        a.clear_filter();
        assert_eq!(
            all(&a),
            vec![
                vec!["k", "v", "w"],
                vec!["", "1"],
                vec!["b", "2"],
                vec!["c", "3", "y"],
                vec!["d", "", "z"],
            ]
        );
    }

    #[test]
    fn update_overwrites_or_fills_blanks() {
        let a0 = "id,city,zip\r\n1,,111\r\n2,Old,\r\n3,Keep,333\r\n";
        let b = doc("id,city,zip\r\n1,Oslo,999\r\n2,Rome,\r\n3,,000\r\n");
        let mut a = doc(a0);
        let spec = JoinSpec {
            columns: vec![1, 2],
            write: JoinWrite::Overwrite,
            targets: vec![1, 2],
            ..base()
        };
        let rep = join(&mut a, &b, &spec);
        // Blanks in the other table never erase: row 2 keeps an empty zip,
        // row 3 keeps "Keep".
        assert_eq!(rep.changed, 4);
        assert_eq!(rep.cols, 0);
        assert_eq!(
            all(&a),
            vec![
                vec!["id", "city", "zip"],
                vec!["1", "Oslo", "999"],
                vec!["2", "Rome", ""],
                vec!["3", "Keep", "000"],
            ]
        );
        let mut a = doc(a0);
        let spec = JoinSpec {
            write: JoinWrite::FillBlanks,
            ..spec
        };
        let rep = join(&mut a, &b, &spec);
        assert_eq!(rep.changed, 1);
        assert_eq!(all(&a)[1], vec!["1", "Oslo", "111"]);
        assert_eq!(all(&a)[2], vec!["2", "Old", ""]);
        // A key column cannot be a target; targets must match the values.
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        let bad = JoinSpec {
            targets: vec![0, 1],
            ..spec.clone()
        };
        let src2 = JoinSource::build(&b, &bad, &Progress::new()).unwrap();
        assert!(a.plan_join(&src2, &bad, &Progress::new()).is_err());
        let bad = JoinSpec {
            targets: vec![1],
            ..spec.clone()
        };
        assert!(a.plan_join(&src, &bad, &Progress::new()).is_err());
    }

    /// The other table is read as shown: a key in one of its hidden rows is
    /// no match (the screen says so).
    #[test]
    fn a_match_in_a_hidden_row_of_the_other_table_counts_as_no_match() {
        let mut b = doc("id\r\n1\r\n3\r\n");
        b.hide_rows(2, 1).unwrap(); // the 3
        let mut a = doc("id,n\r\n1,a\r\n3,d\r\n");
        let spec = JoinSpec {
            write: JoinWrite::Nothing,
            keep_rows: KeepRows::Matched,
            ..base()
        };
        assert_eq!(join(&mut a, &b, &spec).removed, 1);
        assert_eq!(all(&a), vec![vec!["id", "n"], vec!["1", "a"]]);
    }

    #[test]
    fn keep_matched_or_unmatched_rows() {
        let a0 = "id,n\r\n1,a\r\n2,b\r\n,c\r\n3,d\r\n";
        let b = doc("id\r\n1\r\n3\r\n");
        let mut a = doc(a0);
        let spec = JoinSpec {
            write: JoinWrite::Nothing,
            keep_rows: KeepRows::Matched,
            ..base()
        };
        let rep = join(&mut a, &b, &spec);
        assert_eq!(rep.removed, 2);
        assert_eq!(
            all(&a),
            vec![vec!["id", "n"], vec!["1", "a"], vec!["3", "d"]]
        );
        assert!(a.undo());
        assert_eq!(all(&a).len(), 5);
        let spec = JoinSpec {
            keep_rows: KeepRows::Unmatched,
            ..spec
        };
        let rep = join(&mut a, &b, &spec);
        assert_eq!(rep.removed, 2);
        assert_eq!(
            all(&a),
            vec![vec!["id", "n"], vec!["2", "b"], vec!["", "c"]]
        );
        // Inner join: matched rows only, with the other columns added.
        let mut a = doc(a0);
        let b = doc("id,x\r\n1,X1\r\n3,X3\r\n");
        let spec = JoinSpec {
            keep_rows: KeepRows::Matched,
            ..base()
        };
        join(&mut a, &b, &spec);
        assert_eq!(
            all(&a),
            vec![
                vec!["id", "n", "x"],
                vec!["1", "a", "X1"],
                vec!["3", "d", "X3"]
            ]
        );
    }

    #[test]
    fn keys_from_a_pasted_list() {
        let mut a = doc("id,n\r\n001,a\r\n2,b\r\n3,c\r\n");
        let spec = JoinSpec {
            write: JoinWrite::Nothing,
            keep_rows: KeepRows::Unmatched,
            loose: true,
            ..base()
        };
        let src = JoinSource::from_list("1\r\n3\n\n", &spec).unwrap();
        let (plan, rep) = a.plan_join(&src, &spec, &Progress::new()).unwrap();
        assert_eq!(rep.removed, 2);
        a.apply_join(plan).unwrap();
        assert_eq!(all(&a), vec![vec!["id", "n"], vec!["2", "b"]]);
    }

    #[test]
    fn repeated_keys_rules() {
        let a0 = "id,n\r\n1,a\r\n2,b\r\n";
        let b = doc("id,v,w\r\n1,10,x\r\n1,,y\r\n1,2.5,z\r\n2,7,q\r\n");
        let run = |rule: Repeats| {
            let mut a = doc(a0);
            let spec = JoinSpec {
                repeats: rule,
                separator: "; ".into(),
                ..base()
            };
            let rep = join(&mut a, &b, &spec);
            (all(&a), rep)
        };
        assert_eq!(run(Repeats::First).0[1], vec!["1", "a", "10", "x"]);
        assert_eq!(run(Repeats::Last).0[1], vec!["1", "a", "2.5", "z"]);
        assert_eq!(
            run(Repeats::Joined).0[1],
            vec!["1", "a", "10; 2.5", "x; y; z"]
        );
        let (rows, _) = run(Repeats::Count);
        assert_eq!(rows[0], vec!["id", "n", "Matches"]);
        assert_eq!(rows[1], vec!["1", "a", "3"]);
        assert_eq!(rows[2], vec!["2", "b", "1"]);
        assert_eq!(run(Repeats::Sum).0[1], vec!["1", "a", "12.5"]);
        assert_eq!(run(Repeats::Min).0[1], vec!["1", "a", "2.5"]);
        assert_eq!(run(Repeats::Max).0[1], vec!["1", "a", "10"]);
        let (rows, rep) = run(Repeats::Every);
        assert_eq!(rep.extra_rows, 2);
        assert_eq!(
            rows,
            vec![
                vec!["id", "n", "v", "w"],
                vec!["1", "a", "10", "x"],
                vec!["1", "a", "", "y"],
                vec!["1", "a", "2.5", "z"],
                vec!["2", "b", "7", "q"],
            ]
        );
        // One undo step takes every extra row back out.
        let mut a = doc(a0);
        let spec = JoinSpec {
            repeats: Repeats::Every,
            ..base()
        };
        join(&mut a, &b, &spec);
        assert!(a.undo());
        assert_eq!(all(&a), all(&doc(a0)));
    }

    #[test]
    fn loose_keys() {
        for (x, y, same) in [
            ("00123", "123", true),
            ("1.0", "1", true),
            ("-0.50", "-.5", true),
            ("AB-012", "ab 12", true),
            ("A 1", "a1", true),
            ("123", "124", false),
            ("12345678901234567890", "12345678901234567891", false),
            // Review H2: digit runs split by punctuation stay apart.
            ("555-0123", "5550123", false),
            ("555-0123", "555 123", true),
            ("12-3", "1-23", false),
            ("2024-11-5", "2024-1-15", false),
            ("2024-11-05", "2024-11-5", true),
            ("1.0.0", "100", false),
            ("1,234.50", "123450", false),
            ("1,234.50", "1234.5", true),
            ("12,3", "123", false),
            ("Main St.", "main st", true),
            ("0", "000", true),
        ] {
            let spec = JoinSpec {
                loose: true,
                ..Default::default()
            };
            let (mut a, mut b) = (String::new(), String::new());
            norm_key(&[x], &spec, &[], &mut a);
            norm_key(&[y], &spec, &[], &mut b);
            assert_eq!(a == b, same, "{x} vs {y}");
        }
        // Keys with no letters or digits are blank: they match nothing.
        for (x, y) in [("-", "."), ("?", "#"), ("-", "-")] {
            let spec = JoinSpec {
                loose: true,
                ..Default::default()
            };
            let (mut a, mut b) = (String::new(), String::new());
            assert!(!norm_key(&[x], &spec, &[], &mut a));
            assert!(!norm_key(&[y], &spec, &[], &mut b));
        }
        let a = doc("id,n\r\n-,dash\r\n?,q\r\n");
        let b = doc("id,v\r\n.,dot\r\n#,hash\r\n");
        let spec = JoinSpec {
            loose: true,
            ..base()
        };
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        let p = a.preview_join(&src, &spec, &Progress::new()).unwrap();
        assert_eq!((p.report.matched, p.report.blank_keys), (0, 2));
        let mut a = doc("id,n\r\n00042,a\r\n");
        let b = doc("id,v\r\n42.0,yes\r\n");
        let spec = JoinSpec {
            loose: true,
            ..base()
        };
        assert_eq!(join(&mut a, &b, &spec).matched, 1);
    }

    #[test]
    fn not_found_place_and_prefix() {
        let mut a = doc("id,n,z\r\n1,a,q\r\n2,b,r\r\n");
        let b = doc("id,v\r\n1,X\r\n");
        let spec = JoinSpec {
            not_found: "(none)".into(),
            place: Place::AfterKey,
            prefix: "B ".into(),
            ..base()
        };
        join(&mut a, &b, &spec);
        assert_eq!(
            all(&a),
            vec![
                vec!["id", "B v", "n", "z"],
                vec!["1", "X", "a", "q"],
                vec!["2", "(none)", "b", "r"],
            ]
        );
        assert!(a.undo());
        assert_eq!(all(&a)[0], vec!["id", "n", "z"]);
    }

    #[test]
    fn range_lookup() {
        let mut a = doc("age\r\n5\r\n18\r\n40\r\n70\r\nn/a\r\n");
        let b = doc("from,band\r\n0,Child\r\n18,Adult\r\n65,Senior\r\nx,Bad\r\n");
        let spec = JoinSpec {
            key_match: KeyMatch::Below,
            ..base()
        };
        let rep = join(&mut a, &b, &spec);
        assert_eq!((rep.matched, rep.unmatched, rep.bad_keys), (4, 1, 1));
        let col: Vec<String> = all(&a)
            .iter()
            .map(|r| r.get(1).cloned().unwrap_or_default())
            .collect();
        assert_eq!(col, vec!["band", "Child", "Adult", "Adult", "Senior", ""]);
        let mut a = doc("d\r\n2024-01-15\r\n2024-03-01\r\n");
        let b = doc("until,period\r\n2024-01-31,Jan\r\n2024-02-29,Feb\r\n");
        let spec = JoinSpec {
            key_match: KeyMatch::Above,
            ..base()
        };
        let rep = join(&mut a, &b, &spec);
        assert_eq!((rep.matched, rep.unmatched), (1, 1));
        assert_eq!(all(&a)[1], vec!["2024-01-15", "Jan"]);
        // Two key columns cannot be a range match.
        let b = doc("a,b,c\r\n1,2,3\r\n");
        let spec = JoinSpec {
            keys: vec![0, 0],
            other_keys: vec![0, 1],
            key_match: KeyMatch::Below,
            ..base()
        };
        assert!(JoinSource::build(&b, &spec, &Progress::new()).is_err());
    }

    #[test]
    fn preview_matches_the_join() {
        let a0 = "id,n\r\n1,a\r\n2,b\r\n3,c\r\n";
        let b = doc("id,v\r\n1,X\r\n1,Y\r\n3,Z\r\n4,W\r\n");
        let spec = JoinSpec {
            repeats: Repeats::Every,
            keep_rows: KeepRows::Matched,
            add_unmatched: true,
            ..base()
        };
        let a = doc(a0);
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        let p = a.preview_join(&src, &spec, &Progress::new()).unwrap();
        assert_eq!(p.titles, vec!["id", "n", "v"]);
        assert_eq!(p.columns, vec![2]);
        let kinds: Vec<&str> = p.rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["matched", "extra", "removed", "matched", "added"]
        );
        assert_eq!(p.rows[1].values, vec!["1", "a", "Y"]);
        let (_, rep) = a.plan_join(&src, &spec, &Progress::new()).unwrap();
        assert_eq!(p.report, rep);
    }

    #[test]
    fn join_with_column_map_and_save() {
        // Columns moved first (a column map), then a join in the middle.
        let mut a = doc("id,n,z\r\n1,a,q\r\n2,b,r\r\n");
        a.move_cols(2, 1, 0).unwrap();
        let b = doc("id,v\r\n2,X\r\n");
        let spec = JoinSpec {
            keys: vec![1],
            place: Place::AfterKey,
            ..base()
        };
        join(&mut a, &b, &spec);
        assert_eq!(
            all(&a),
            vec![
                vec!["z", "id", "v", "n"],
                vec!["q", "1", "", "a"],
                vec!["r", "2", "X", "b"],
            ]
        );
        let mut out = Vec::new();
        a.write_copy(&mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "z,id,v,n\r\nq,1,,a\r\nr,2,X,b\r\n"
        );
    }

    /// Review round 2 N1: a wide join on many rows keeps group numbers, not
    /// a copy of every value, so it is not refused for bookkeeping alone.
    #[test]
    fn wide_joins_keep_group_numbers() {
        let rows = 40_000;
        let mut mine = String::from("k,n\r\n");
        let mut other = String::from("k,a,b,c\r\n");
        for i in 0..rows {
            mine.push_str(&format!("{},{i}\r\n", i % 1000));
        }
        for g in 0..1000 {
            other.push_str(&format!("{g},a{g},b{g},c{g}\r\n"));
        }
        other.push_str("zz,1,2,3\r\n");
        mine.push_str("nope,x\r\n");
        let mut a = doc(&mine);
        let b = doc(&other);
        let spec = JoinSpec {
            not_found: "?".into(),
            ..base()
        };
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        let (plan, rep) = a.plan_join(&src, &spec, &Progress::new()).unwrap();
        assert_eq!(plan.filled.len(), rows + 1);
        assert_eq!(plan.changes.len(), 1, "only the header is copied");
        assert_eq!(rep.changed, 3 * rows as u64 + 3);
        drop(src);
        a.apply_join(plan).unwrap();
        assert_eq!(a.row_values(1).unwrap(), vec!["0", "0", "a0", "b0", "c0"]);
        assert_eq!(
            a.row_values(1002).unwrap(),
            vec!["1", "1001", "a1", "b1", "c1"]
        );
        assert_eq!(
            a.row_values(rows as u64 + 1).unwrap(),
            vec!["nope", "x", "?", "?", "?"]
        );
        a.undo();
        assert_eq!(a.col_count(), 2);
    }

    /// Review H1: a big value matched by many rows (or a big row with many
    /// matches) is refused early, not copied until memory runs out.
    #[test]
    fn big_copies_are_refused() {
        let big = "x".repeat(1 << 20);
        let b = doc(&format!("k,v\r\nk,{big}\r\n"));
        let mut mine = String::from("k,n\r\n");
        for i in 0..200 {
            mine.push_str(&format!("k,{i}\r\n"));
        }
        let a = doc(&mine);
        let spec = base();
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        let t = std::time::Instant::now();
        assert!(matches!(
            a.plan_join(&src, &spec, &Progress::new()),
            Err(Error::OutOfMemory(_))
        ));
        assert!(matches!(
            a.preview_join(&src, &spec, &Progress::new()),
            Err(Error::OutOfMemory(_))
        ));
        assert!(t.elapsed().as_secs() < 5);
        // Every match: one big row here, its key repeated in the other table.
        let a = doc(&format!("k,n\r\nk,{big}\r\n"));
        let mut other = String::from("k,v\r\n");
        for i in 0..200 {
            other.push_str(&format!("k,{i}\r\n"));
        }
        let b = doc(&other);
        let spec = JoinSpec {
            repeats: Repeats::Every,
            ..base()
        };
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        assert!(matches!(
            a.plan_join(&src, &spec, &Progress::new()),
            Err(Error::OutOfMemory(_))
        ));
        // A long not-found text copied into many rows is charged too.
        let spec = JoinSpec {
            not_found: big.clone(),
            ..base()
        };
        let b = doc("k,v\r\nzz,1\r\n");
        let a = doc(&mine);
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        assert!(matches!(
            a.plan_join(&src, &spec, &Progress::new()),
            Err(Error::OutOfMemory(_))
        ));
    }

    /// Review M1: blank lines of a pasted list are never stored.
    #[test]
    fn lists_skip_blank_lines() {
        let spec = JoinSpec {
            keys: vec![0],
            ..Default::default()
        };
        let text = "\n".repeat(100_000) + "a\n\r\n  \nb";
        let src = JoinSource::from_list(&text, &spec).unwrap();
        assert_eq!(src.groups(), 2);
        assert_eq!(src.offs.len(), 3);
    }

    /// Review L1: a range lookup reads its keys as numbers or as dates,
    /// whichever most of the other table's keys are, never mixing them.
    #[test]
    fn range_keys_do_not_mix_dates_and_numbers() {
        let mut a = doc("when\r\n03/04/2024\r\n2500\r\n");
        let b = doc("from,band\r\n2000,Year band\r\n");
        let spec = JoinSpec {
            key_match: KeyMatch::Below,
            ..base()
        };
        let rep = join(&mut a, &b, &spec);
        assert_eq!((rep.matched, rep.unmatched), (1, 1));
        assert_eq!(all(&a)[1], vec!["03/04/2024"]);
        let mut a = doc("when\r\n03/04/2024\r\n2500\r\n");
        let b = doc("from,band\r\n01/01/2024,Q1\r\n04/01/2024,Q2\r\n7,odd\r\n");
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        assert!(src.range_dates());
        let (plan, rep) = a.plan_join(&src, &spec, &Progress::new()).unwrap();
        assert_eq!((rep.matched, rep.unmatched, rep.bad_keys), (1, 1, 1));
        a.apply_join(plan).unwrap();
        assert_eq!(all(&a)[1], vec!["03/04/2024", "Q1"]);
    }

    /// Review L3: exact-key joins agree with a plain reference join.
    #[test]
    fn matches_a_reference_join() {
        use proptest::prelude::*;
        use proptest::test_runner::{Config, TestRunner};
        let mut runner = TestRunner::new(Config {
            cases: 400,
            ..Config::default()
        });
        let table = proptest::collection::vec(
            proptest::collection::vec(
                prop_oneof![
                    Just(""),
                    Just("a"),
                    Just("b"),
                    Just("A"),
                    Just("1"),
                    Just("x y")
                ],
                3,
            ),
            0..8,
        );
        runner
            .run(
                &(table.clone(), table, 0u8..4, 0u8..3, any::<bool>()),
                |(mine, other, rule, keep, add)| {
                    let csv = |t: &Vec<Vec<&str>>| {
                        let mut s = String::from("k,p,q\r\n");
                        for r in t {
                            s.push_str(
                                &r.iter()
                                    .map(|v| {
                                        if v.contains(' ') {
                                            format!("\"{v}\"")
                                        } else {
                                            v.to_string()
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join(","),
                            );
                            s.push_str("\r\n");
                        }
                        s
                    };
                    let plain = |t: String| {
                        let o = crate::OpenOptions {
                            delimiter: Some(b','),
                            skip_lines: Some(0),
                            ..Default::default()
                        };
                        Document::from_bytes(t.into_bytes(), o, Some("csv")).unwrap()
                    };
                    let mut a = plain(csv(&mine));
                    let b = plain(csv(&other));
                    let repeats = [
                        Repeats::First,
                        Repeats::Last,
                        Repeats::Count,
                        Repeats::Joined,
                    ][rule as usize];
                    let keep_rows =
                        [KeepRows::All, KeepRows::Matched, KeepRows::Unmatched][keep as usize];
                    let spec = JoinSpec {
                        repeats,
                        keep_rows,
                        add_unmatched: add,
                        separator: "+".into(),
                        ..base()
                    };
                    // Reference.
                    let groups: Vec<(&str, Vec<&Vec<&str>>)> = {
                        let mut g: Vec<(&str, Vec<&Vec<&str>>)> = Vec::new();
                        for r in &other {
                            if r[0].trim().is_empty() {
                                continue;
                            }
                            match g.iter_mut().find(|x| x.0 == r[0]) {
                                Some(x) => x.1.push(r),
                                None => g.push((r[0], vec![r])),
                            }
                        }
                        g
                    };
                    let vals = |ms: &Vec<&Vec<&str>>| -> Vec<String> {
                        match repeats {
                            Repeats::First => vec![ms[0][1].into(), ms[0][2].into()],
                            Repeats::Last => {
                                vec![ms[ms.len() - 1][1].into(), ms[ms.len() - 1][2].into()]
                            }
                            Repeats::Count => vec![ms.len().to_string()],
                            _ => (1..3)
                                .map(|c| {
                                    ms.iter()
                                        .map(|r| r[c])
                                        .filter(|v| !v.is_empty())
                                        .collect::<Vec<_>>()
                                        .join("+")
                                })
                                .collect(),
                        }
                    };
                    let w = if repeats == Repeats::Count { 1 } else { 2 };
                    let mut titles = vec!["k".to_string(), "p".into(), "q".into()];
                    if keep_rows != KeepRows::Unmatched {
                        titles.extend(if repeats == Repeats::Count {
                            vec!["Matches".to_string()]
                        } else {
                            vec!["p".into(), "q".into()]
                        });
                    }
                    let mut want = vec![titles];
                    let mut used = vec![false; groups.len()];
                    for r in &mine {
                        let hit = groups
                            .iter()
                            .position(|g| g.0 == r[0] && !r[0].trim().is_empty());
                        if let Some(i) = hit {
                            used[i] = true;
                        }
                        let stays = match keep_rows {
                            KeepRows::All => true,
                            KeepRows::Matched => hit.is_some(),
                            KeepRows::Unmatched => hit.is_none(),
                        };
                        if !stays {
                            continue;
                        }
                        let mut row: Vec<String> = r.iter().map(|v| v.to_string()).collect();
                        if keep_rows != KeepRows::Unmatched {
                            match hit {
                                Some(i) => row.extend(vals(&groups[i].1)),
                                None => row.extend(vec![String::new(); w]),
                            }
                        }
                        want.push(row);
                    }
                    if add && keep_rows != KeepRows::Unmatched {
                        for (i, g) in groups.iter().enumerate() {
                            if !used[i] {
                                let mut row = vec![g.0.to_string(), String::new(), String::new()];
                                row.extend(vals(&g.1));
                                want.push(row);
                            }
                        }
                    }
                    // The engine.
                    let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
                    match a.plan_join(&src, &spec, &Progress::new()) {
                        Ok((plan, _)) => {
                            a.apply_join(plan).unwrap();
                            let width = a.col_count() as usize;
                            let got: Vec<Vec<String>> = all(&a)
                                .into_iter()
                                .map(|mut r| {
                                    r.resize(width, String::new());
                                    r
                                })
                                .collect();
                            let want: Vec<Vec<String>> = want
                                .into_iter()
                                .map(|mut r| {
                                    r.resize(width, String::new());
                                    r
                                })
                                .collect();
                            prop_assert_eq!(got, want);
                        }
                        // Nothing to do: the reference changes nothing either
                        // (besides titles of columns that would stay empty).
                        Err(Error::Invalid(_)) => {
                            prop_assert!(want[1..].iter().zip(&mine).all(|(w, m)| w[..3]
                                == m[..].iter().map(|v| v.to_string()).collect::<Vec<_>>()[..]
                                && w[3..].iter().all(String::is_empty)));
                        }
                        Err(e) => return Err(TestCaseError::fail(format!("{e}"))),
                    }
                    Ok(())
                },
            )
            .unwrap();
    }

    #[test]
    fn group_index_is_compact() {
        let b = doc("k,v\r\na,1\r\nb,2\r\na,3\r\n,4\r\n");
        let spec = base();
        let src = JoinSource::build(&b, &spec, &Progress::new()).unwrap();
        assert_eq!(src.groups(), 2);
        assert_eq!(src.group(0), &[0, 2]);
        assert_eq!(src.group(1), &[1]);
    }

    #[test]
    fn lookup_table() {
        let b = doc("code,name\r\n00123,Ann\r\nab,Bob\r\nAB,Dup\r\n");
        let t = LookupTable::build(&b, 0, 1, true, &Progress::new()).unwrap();
        assert_eq!(t.get("123"), Some("Ann"));
        assert_eq!(t.get(" Ab "), Some("Bob"));
        assert_eq!(t.get("x"), None);
    }

    #[test]
    fn append_by_name_and_position() {
        let mut a = doc("id,name\r\n1,Ann\r\n");
        let b = doc("Name,extra,ID\r\nBob,e,2\r\n");
        let spec = AppendSpec {
            by_name: true,
            keep_first: true,
            other_header: true,
            add_columns: true,
        };
        let src = AppendSource::build(&b, &spec, &Progress::new()).unwrap();
        let (plan, rep) = a.plan_append(&src, &spec).unwrap();
        assert_eq!((rep.added, rep.cols, rep.dropped_cols), (1, 1, 0));
        a.apply_cells(plan).unwrap();
        assert_eq!(
            all(&a),
            vec![
                vec!["id", "name", "extra"],
                vec!["1", "Ann"],
                vec!["2", "Bob", "e"],
            ]
        );
        assert!(a.undo());
        let spec = AppendSpec {
            by_name: true,
            keep_first: true,
            other_header: true,
            add_columns: false,
        };
        let src = AppendSource::build(&b, &spec, &Progress::new()).unwrap();
        let (plan, rep) = a.plan_append(&src, &spec).unwrap();
        assert_eq!(rep.dropped_cols, 1);
        a.apply_cells(plan).unwrap();
        assert_eq!(all(&a)[2], vec!["2", "Bob"]);
        assert!(a.undo());
        let spec = AppendSpec::default();
        let src = AppendSource::build(&b, &spec, &Progress::new()).unwrap();
        let (plan, _) = a.plan_append(&src, &spec).unwrap();
        a.apply_cells(plan).unwrap();
        assert_eq!(all(&a).len(), 4);
        assert_eq!(all(&a)[2], vec!["Name", "extra", "ID"]);
    }
}
