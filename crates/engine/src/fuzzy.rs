//! Match people across lists (Milestone 15): rows of this table and rows
//! of another list (or of this table itself, for near-duplicates) that
//! look like the same person even when spelled differently. Each pair gets
//! a score from 0 to 100; the user accepts or rejects pairs, and accepted
//! pairs add columns (the match's row, its score, and any columns brought
//! in from the other list).
//!
//! Scale: both sides are copied once into compact rows (normalized text).
//! Rows are grouped by several sound-alike keys (first and last name, last
//! name and birth date, ...), each group sorted, and each row compared
//! with its nearest neighbours in the group: every pair in a small group,
//! and the [`WINDOW`] closest on each side in a very large one (a common
//! name with nothing to narrow it), which the report counts.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::{ColPlan, Cols};
use crate::value::ReadOptions;
use crate::Error;

/// Most fields compared.
pub const MAX_FIELDS: usize = 8;
/// Most pairs listed (the count goes on).
pub const MAX_LISTED: usize = 200_000;
/// Rows of a group compared with each row, on each side of it in the
/// group's order, when the group is larger than that.
pub const WINDOW: usize = 24;
/// Rows copied per parallel chunk.
const CHUNK: u64 = 1 << 15;

/// How a field is compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// A person's name (or part of it): spelling slips, nicknames
    /// (Bill and William), initials, titles and suffixes ignored.
    #[default]
    Name,
    /// Other text (an address, a city): spelling slips.
    Text,
    /// Must be the same: dates (any layout), ZIP codes, phone numbers and
    /// ids compared by their digits; a date with day and month swapped
    /// counts partly.
    Exact,
}

/// One field compared: a column here and the matching column there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchField {
    pub col: u32,
    pub other_col: u32,
    #[serde(default)]
    pub kind: FieldKind,
    /// How much it counts (default 1).
    #[serde(default = "one")]
    pub weight: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchSpec {
    pub fields: Vec<MatchField>,
    /// This table's first row holds titles.
    pub keep_first: bool,
    /// The other list's first row holds titles.
    pub other_header: bool,
    /// Pairs scoring below this are not listed (0 to 100).
    pub min_score: f64,
    /// Read dates day first.
    pub read: ReadOptions,
}

impl Default for MatchSpec {
    fn default() -> Self {
        MatchSpec {
            fields: Vec::new(),
            keep_first: true,
            other_header: true,
            min_score: 80.0,
            read: ReadOptions::default(),
        }
    }
}

impl MatchSpec {
    /// The columns compared here and there, and how each is compared: what
    /// [`MatchSide::build`] takes for each side (the app and the command line
    /// both build their sides from this).
    pub fn columns(&self) -> (Vec<u32>, Vec<u32>, Vec<FieldKind>) {
        (
            self.fields.iter().map(|f| f.col).collect(),
            self.fields.iter().map(|f| f.other_col).collect(),
            self.fields.iter().map(|f| f.kind).collect(),
        )
    }
}

/// [`find_matches`] on two documents (or one, against itself), both readable
/// here: the sides are built as the app builds them.
pub fn match_documents(
    this: &Document,
    other: Option<&Document>,
    spec: &MatchSpec,
    progress: &Progress,
) -> Result<MatchReport, Error> {
    if spec.fields.is_empty() || spec.fields.len() > MAX_FIELDS {
        return Err(Error::Invalid(format!(
            "choose 1 to {MAX_FIELDS} columns to compare"
        )));
    }
    let (here, there, kinds) = spec.columns();
    let other_side = match other {
        Some(o) => Some(MatchSide::build(
            o,
            &there,
            &kinds,
            &[],
            spec.other_header,
            spec.read,
            progress,
        )?),
        None => None,
    };
    let this_side = MatchSide::build(
        this,
        &here,
        &kinds,
        &[],
        spec.keep_first,
        spec.read,
        progress,
    )?;
    find_matches(&this_side, other_side.as_ref(), spec, progress)
}

/// A possible match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Shown row here, and the row id (for applying).
    pub row: u64,
    pub id: u32,
    /// Its place on the other side (shown row there).
    pub other_row: u64,
    pub score: f32,
    /// Each field's likeness (0 to 1; -1 when either side is blank).
    pub parts: Vec<f32>,
    /// The compared values as matched (small letters, accents off), for
    /// the first [`MAX_SHOWN`] pairs; empty after that.
    #[serde(default)]
    pub here: Vec<String>,
    #[serde(default)]
    pub there: Vec<String>,
}

/// Pairs that carry their values for the review screen.
pub const MAX_SHOWN: usize = 5_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchReport {
    /// The best pairs first (at most [`MAX_LISTED`]).
    pub listed: Vec<Candidate>,
    /// Pairs at or above the score in all.
    pub total: u64,
    /// Rows here with at least one pair.
    pub rows_matched: u64,
    /// Pairs compared.
    pub compared: u64,
    /// Rows of this table that were in a very large group (a common name
    /// with nothing to narrow it), so compared only with their nearest
    /// [`WINDOW`] rows there.
    pub windowed: u64,
}

/// Grouping entries being gathered, and a buffer for one row's keys.
type Gather = (Vec<(u64, u64, u32)>, Vec<(u64, u64)>);

/// A pair found: (row here, row there, score, each field's likeness).
type Pair = (u32, u32, f32, Vec<f32>);

/// One side copied for matching: normalized field values, row by row.
pub struct MatchSide {
    /// Shown row positions (the first data row is `first`).
    first: u64,
    ids: Vec<u32>,
    /// Field `k` of row `i` is `text[offs[i * w + k]..offs[i * w + k + 1]]`.
    text: String,
    offs: Vec<u64>,
    width: usize,
    kinds: Vec<FieldKind>,
    /// Raw values of the columns brought in (other side only).
    bring: Vec<String>,
    bring_width: usize,
    revision: u64,
}

impl MatchSide {
    /// Copy `cols` (compared, normalized by `kinds`) and `bring` (as they
    /// are) of every shown data row.
    pub fn build(
        doc: &Document,
        cols: &[u32],
        kinds: &[FieldKind],
        bring: &[u32],
        header: bool,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<MatchSide, Error> {
        doc.ready_for_bulk()?;
        if cols.is_empty() || cols.len() > MAX_FIELDS {
            return Err(Error::Invalid(format!(
                "choose 1 to {MAX_FIELDS} columns to compare"
            )));
        }
        if bring.len() > 64 {
            return Err(Error::Invalid(
                "at most 64 columns can be brought in".into(),
            ));
        }
        let ncols = doc.col_count();
        if cols.iter().chain(bring).any(|&c| c >= ncols) {
            return Err(Error::OutOfRange);
        }
        let rows = doc.data_rows(header);
        let budget = crate::find::split_budget(doc);
        let used = AtomicUsize::new(0);
        let mut all = cols.to_vec();
        all.extend_from_slice(bring);
        let reader = Cols::new(doc, &all);
        let ctx = doc.read_ctx(read);
        let rds: Vec<crate::coltype::Reader> = cols.iter().map(|&c| ctx.reader(c)).collect();
        let w = cols.len();
        let bw = bring.len();
        progress.start(rows.end - rows.start);
        type Part = (Vec<u32>, String, Vec<u64>, Vec<String>);
        let parts: Vec<Part> = (0..(rows.end - rows.start).div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<Part, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut ids = Vec::with_capacity((e - s) as usize);
                let mut text = String::new();
                let mut offs = Vec::with_capacity((e - s) as usize * w);
                let mut out_bring = Vec::with_capacity((e - s) as usize * bw);
                let mut buf = Vec::new();
                for r in s..e {
                    let id = doc.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(doc, id, &mut buf);
                    ids.push(id);
                    let before = text.len();
                    for (f, kind) in kinds.iter().enumerate().take(w) {
                        normalize(reader.get(&buf, f), *kind, rds[f], &mut text);
                        offs.push(text.len() as u64);
                    }
                    let mut n = text.len() - before + 16 * w;
                    for b in 0..bw {
                        let v = reader.get(&buf, w + b);
                        n += v.len() + 24;
                        out_bring.push(v.to_string());
                    }
                    if used.fetch_add(n, Ordering::Relaxed) + n > budget {
                        return Err(Error::OutOfMemory(
                            "the list is too large to match at once; filter it first",
                        ));
                    }
                }
                progress.step(e - s)?;
                Ok((ids, text, offs, out_bring))
            })
            .collect::<Result<_, _>>()?;
        let mut side = MatchSide {
            first: rows.start,
            ids: Vec::new(),
            text: String::new(),
            offs: vec![0],
            width: w,
            kinds: kinds.to_vec(),
            bring: Vec::new(),
            bring_width: bw,
            revision: doc.revision(),
        };
        for (ids, text, offs, bring) in parts {
            let base = side.text.len() as u64;
            side.ids.extend(ids);
            side.text.push_str(&text);
            side.offs.extend(offs.into_iter().map(|o| o + base));
            side.bring.extend(bring);
        }
        Ok(side)
    }

    /// The document's revision when copied.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    fn field(&self, i: usize, k: usize) -> &str {
        let at = i * self.width + k;
        &self.text[self.offs[at] as usize..self.offs[at + 1] as usize]
    }

    /// The grouping keys of row `i`, with a value that sorts the group:
    /// (kind << 60 | key, sort value). Kinds: 1 first and last name
    /// (one per sound-alike form of the first name), 2 last name and the
    /// first exact field (a birth date, say), 3 first name and that field,
    /// 4 first letter and last name (initials).
    fn keys(&self, i: usize, out: &mut Vec<(u64, u64)>) {
        out.clear();
        let tag = |n: u64, v: u64| (n << 60) | (v & ((1 << 60) - 1));
        if self.kinds[0] == FieldKind::Exact {
            let v = self.field(i, 0);
            if !v.is_empty() {
                out.push((tag(5, hash(v)), 0));
            }
            return;
        }
        let f0 = self.field(i, 0);
        let (fname, lname) = if self.width > 1 && self.kinds[1] != FieldKind::Exact {
            let first = name_words(f0).next().unwrap_or("");
            let last = name_words(self.field(i, 1)).next().unwrap_or("");
            (first, last)
        } else {
            let mut w = name_words(f0);
            let first = w.next().unwrap_or("");
            (first, w.last().unwrap_or(""))
        };
        if fname.is_empty() && lname.is_empty() {
            return;
        }
        let ex = self
            .kinds
            .iter()
            .position(|&k| k == FieldKind::Exact)
            .map_or("", |k| self.field(i, k));
        let lcode = if lname.is_empty() {
            0
        } else {
            code(lname, FieldKind::Name)
        };
        let sort1 = if ex.is_empty() {
            prefix(lname)
        } else {
            prefix(ex)
        };
        if !fname.is_empty() {
            out.push((tag(1, mix(code(fname, FieldKind::Name), lcode)), sort1));
            let mut g = nickname_groups(fname);
            while g != 0 {
                let b = g.trailing_zeros() as u64;
                g &= g - 1;
                out.push((tag(1, mix(1 << 40 | b, lcode)), sort1));
            }
        }
        if !ex.is_empty() {
            let h = hash(ex);
            if !lname.is_empty() {
                out.push((tag(2, mix(lcode, h)), prefix(fname)));
            }
            if !fname.is_empty() {
                out.push((tag(3, mix(code(fname, FieldKind::Name), h)), prefix(lname)));
            }
        }
        // Initials (J Smith / Jane Smith): kind 2 covers them when there
        // is a birth date or the like.
        if ex.is_empty() && !fname.is_empty() && !lname.is_empty() {
            out.push((
                tag(4, mix(fname.as_bytes()[0] as u64, lcode)),
                prefix(fname),
            ));
        }
    }
}

/// The words of a normalized name that are not a generation suffix.
fn name_words(v: &str) -> impl Iterator<Item = &str> {
    v.split(' ')
        .enumerate()
        .filter(|(k, w)| !w.is_empty() && !(*k > 0 && is_suffix(w)))
        .map(|(_, w)| w)
}

/// A generation suffix as normalized (Jr, Sr, II, III, IV).
fn is_suffix(w: &str) -> bool {
    matches!(w, "jr" | "sr" | "ii" | "iii" | "iv")
}

/// The generation suffix of a normalized name (not its first word).
fn suffix_of(v: &str) -> Option<&str> {
    v.split(' ').skip(1).find(|w| is_suffix(w))
}

/// Two numbers mixed into one key.
fn mix(a: u64, b: u64) -> u64 {
    a.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29) ^ b
}

/// The first eight bytes of a value, as a number that sorts like it.
fn prefix(v: &str) -> u64 {
    let mut b = [0u8; 8];
    for (d, s) in b.iter_mut().zip(v.bytes()) {
        *d = s;
    }
    u64::from_be_bytes(b)
}

/// Find pairs: rows of `this` against `other`, or within `this` when
/// `other` is `None` (near-duplicates: each pair once, the later row
/// paired with the earlier). Each row of `this` gets its best pair.
pub fn find_matches(
    this: &MatchSide,
    other: Option<&MatchSide>,
    spec: &MatchSpec,
    progress: &Progress,
) -> Result<MatchReport, Error> {
    let same = other.is_none();
    let there = other.unwrap_or(this);
    if this.width != there.width || spec.fields.len() != this.width {
        return Err(Error::Invalid(
            "the columns to compare do not line up".into(),
        ));
    }
    if this.len() >= 1 << 31 || there.len() >= 1 << 31 {
        return Err(Error::Invalid("the list is too large to match".into()));
    }
    let min = spec.min_score.clamp(0.0, 100.0) as f32;
    let weights: Vec<f32> = spec
        .fields
        .iter()
        .map(|f| f.weight.clamp(0.0, 100.0) as f32)
        .collect();
    // Best pair of each row here: (score bits + 1) << 32 | !row there
    // (the higher score wins, then the earlier row); 0 = none yet.
    let best: Vec<AtomicU64> = (0..this.len()).map(|_| AtomicU64::new(0)).collect();
    let windowed: Vec<std::sync::atomic::AtomicBool> = (0..this.len())
        .map(|_| std::sync::atomic::AtomicBool::new(false))
        .collect();
    let compared = AtomicU64::new(0);
    const OTHER: u32 = 1 << 31;
    let kinds = 5u64;
    progress.start(kinds * this.len() as u64);
    for kind in 1..=kinds {
        // Every row's keys of this kind: (key, sort value, row | OTHER).
        let collect = |side: &MatchSide, flag: u32| -> Vec<(u64, u64, u32)> {
            (0..side.len())
                .into_par_iter()
                .with_min_len(4096)
                .fold(
                    || (Vec::new(), Vec::new()),
                    |(mut out, mut keys): Gather, i| {
                        side.keys(i, &mut keys);
                        for &(k, v) in &keys {
                            if k >> 60 == kind {
                                out.push((k, v, i as u32 | flag));
                            }
                        }
                        (out, keys)
                    },
                )
                .map(|(out, _)| out)
                .reduce(Vec::new, |mut a, b| {
                    a.extend(b);
                    a
                })
        };
        let mut ents = collect(this, 0);
        if !same {
            ents.extend(collect(there, OTHER));
        }
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        ents.par_sort_unstable();
        ents.dedup();
        // Groups of one key.
        let mut groups: Vec<(usize, usize)> = Vec::new();
        let mut a = 0;
        for b in 1..=ents.len() {
            if b == ents.len() || ents[b].0 != ents[a].0 {
                if b - a > 1 {
                    groups.push((a, b));
                }
                a = b;
            }
        }
        let offer = |i: u32, j: u32| -> u64 {
            let (score, _) = score(this, i as usize, there, j as usize, &weights);
            if score >= min && score > 0.0 {
                let packed = ((score.to_bits() as u64 + 1) << 32) | u64::from(!j);
                best[i as usize].fetch_max(packed, Ordering::Relaxed);
            }
            1
        };
        groups
            .par_iter()
            .with_min_len(64)
            .try_for_each(|&(a, b)| -> Result<(), Error> {
                let g = &ents[a..b];
                let mut n = 0u64;
                if same {
                    let big = g.len() > WINDOW + 1;
                    for (p, x) in g.iter().enumerate() {
                        if big {
                            windowed[x.2 as usize].store(true, Ordering::Relaxed);
                        }
                        for y in &g[p + 1..(p + 1 + WINDOW).min(g.len())] {
                            let (i, j) = if x.2 > y.2 { (x.2, y.2) } else { (y.2, x.2) };
                            if i != j {
                                n += offer(i, j);
                            }
                        }
                    }
                } else {
                    let others: Vec<usize> =
                        (0..g.len()).filter(|&p| g[p].2 & OTHER != 0).collect();
                    if others.is_empty() {
                        return Ok(());
                    }
                    let big = others.len() > 2 * WINDOW;
                    for (p, x) in g.iter().enumerate() {
                        if x.2 & OTHER != 0 {
                            continue;
                        }
                        if big {
                            windowed[x.2 as usize].store(true, Ordering::Relaxed);
                        }
                        let at = others.partition_point(|&o| o < p);
                        let lo = at.saturating_sub(WINDOW);
                        let hi = (at + WINDOW).min(others.len());
                        for &o in &others[lo..hi] {
                            n += offer(x.2, g[o].2 & !OTHER);
                        }
                    }
                }
                compared.fetch_add(n, Ordering::Relaxed);
                if progress.cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                Ok(())
            })?;
        progress.step(this.len() as u64)?;
    }
    let mut all: Vec<Pair> = best
        .par_iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let b = b.load(Ordering::Relaxed);
            (b != 0).then(|| {
                let j = !(b as u32);
                let (score, parts) = score(this, i, there, j as usize, &weights);
                (i as u32, j, score, parts)
            })
        })
        .collect();
    let total = all.len() as u64;
    all.par_sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    all.truncate(MAX_LISTED);
    let listed = all
        .into_iter()
        .enumerate()
        .map(|(n, (i, j, score, parts))| {
            let vals = |side: &MatchSide, r: usize| -> Vec<String> {
                if n < MAX_SHOWN {
                    (0..side.width)
                        .map(|k| side.field(r, k).to_string())
                        .collect()
                } else {
                    Vec::new()
                }
            };
            Candidate {
                row: this.first + i as u64,
                id: this.ids[i as usize],
                other_row: there.first + j as u64,
                score: (score * 10.0).round() / 10.0,
                parts,
                here: vals(this, i as usize),
                there: vals(there, j as usize),
            }
        })
        .collect();
    Ok(MatchReport {
        listed,
        total,
        rows_matched: total,
        compared: compared.into_inner(),
        windowed: windowed
            .iter()
            .filter(|w| w.load(Ordering::Relaxed))
            .count() as u64,
    })
}

/// The score of row `i` here against row `j` there, and each field's
/// likeness. A field blank on either side counts half. Names with
/// different generation suffixes (Jr and Sr) are not the same person:
/// the score is 0.
fn score(a: &MatchSide, i: usize, b: &MatchSide, j: usize, w: &[f32]) -> (f32, Vec<f32>) {
    let mut sum = 0.0;
    let mut total = 0.0;
    let mut parts = Vec::with_capacity(a.width);
    let mut blocked = false;
    for (k, &wk) in w.iter().enumerate().take(a.width) {
        let (x, y) = (a.field(i, k), b.field(j, k));
        let sim = if x.is_empty() || y.is_empty() {
            parts.push(-1.0);
            0.5
        } else {
            if a.kinds[k] == FieldKind::Name {
                if let (Some(p), Some(q)) = (suffix_of(x), suffix_of(y)) {
                    blocked |= p != q;
                }
            }
            let s = likeness(x, y, a.kinds[k]);
            parts.push(s);
            s
        };
        sum += wk * sim;
        total += wk;
    }
    let score = if blocked || total <= 0.0 {
        0.0
    } else {
        100.0 * sum / total
    };
    (score, parts)
}

/// How alike two normalized values are (0 to 1).
pub fn likeness(x: &str, y: &str, kind: FieldKind) -> f32 {
    if x == y {
        return 1.0;
    }
    match kind {
        FieldKind::Exact => {
            // Dates are kept as `yyyymmdd`: day and month swapped counts
            // partly.
            if x.len() == 8
                && y.len() == 8
                && x[..4] == y[..4]
                && x[4..6] == y[6..8]
                && x[6..8] == y[4..6]
            {
                0.8
            } else {
                0.0
            }
        }
        FieldKind::Text => jaro_winkler(x, y),
        FieldKind::Name => {
            // Word by word, in either order (Ann Marie / Marie Ann); a
            // generation suffix is not a word of the name (see `score`).
            let (x, y) = (
                &name_words(x).collect::<Vec<_>>().join(" "),
                &name_words(y).collect::<Vec<_>>().join(" "),
            );
            if x == y {
                return 1.0;
            }
            if x.is_empty() || y.is_empty() {
                return 0.0;
            }
            let xs: Vec<&str> = x.split(' ').collect();
            let ys: Vec<&str> = y.split(' ').collect();
            let one = |p: &str, q: &str| -> f32 {
                if p == q || nickname_groups(p) & nickname_groups(q) != 0 {
                    return 1.0;
                }
                if (p.len() == 1 && q.starts_with(p)) || (q.len() == 1 && p.starts_with(q)) {
                    return 0.85;
                }
                jaro_winkler(p, q)
            };
            let (short, long) = if xs.len() <= ys.len() {
                (&xs, &ys)
            } else {
                (&ys, &xs)
            };
            let mut sum = 0.0;
            for p in short.iter() {
                sum += long.iter().map(|q| one(p, q)).fold(0.0, f32::max);
            }
            let avg = sum / short.len() as f32;
            // A missing middle name costs a little.
            let extra = (long.len() - short.len()) as f32 * 0.03;
            (avg - extra).max(0.0).max(jaro_winkler(x, y) * 0.95)
        }
    }
}

/// Jaro-Winkler likeness of two strings (0 to 1), by characters.
pub fn jaro_winkler(a: &str, b: &str) -> f32 {
    if a.is_ascii() && b.is_ascii() {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        return jaro_winkler_of(&a[..a.len().min(64)], &b[..b.len().min(64)]);
    }
    let a: Vec<char> = a.chars().take(64).collect();
    let b: Vec<char> = b.chars().take(64).collect();
    jaro_winkler_of(&a, &b)
}

fn jaro_winkler_of<T: PartialEq + Copy>(a: &[T], b: &[T]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let range = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut am = [false; 64];
    let mut bm = [false; 64];
    let mut m = 0usize;
    for (i, &c) in a.iter().enumerate() {
        let lo = i.saturating_sub(range);
        let hi = (i + range + 1).min(b.len());
        for j in lo..hi {
            if !bm[j] && b[j] == c {
                am[i] = true;
                bm[j] = true;
                m += 1;
                break;
            }
        }
    }
    if m == 0 {
        return 0.0;
    }
    let mut t = 0usize;
    let mut k = 0usize;
    for (i, &c) in a.iter().enumerate() {
        if am[i] {
            while !bm[k] {
                k += 1;
            }
            if c != b[k] {
                t += 1;
            }
            k += 1;
        }
    }
    let m = m as f32;
    let jaro = (m / a.len() as f32 + m / b.len() as f32 + (m - t as f32 / 2.0) / m) / 3.0;
    let prefix = a.iter().zip(b).take(4).take_while(|(x, y)| x == y).count() as f32;
    jaro + prefix * 0.1 * (1.0 - jaro)
}

/// Common nicknames and short forms: the groups a name is in, one bit
/// each (a short form can be in several: Pat is Patrick or Patricia).
/// Only well-known pairs; ambiguous ones (Terry and Theresa, Nancy and
/// Ann) are left to spelling likeness.
fn nickname_groups(w: &str) -> u128 {
    const GROUPS: &[&[&str]] = &[
        &["william", "bill", "billy", "will", "willie", "liam"],
        &["robert", "bob", "bobby", "rob", "robbie", "bert"],
        &["albert", "al", "bert", "bertie"],
        &["herbert", "herb", "bert"],
        &["richard", "rick", "ricky", "rich", "dick"],
        &["james", "jim", "jimmy", "jamie"],
        &["john", "jack", "johnny", "jon"],
        &["jonathan", "jon", "jonny", "johnny"],
        &["nathaniel", "nathan", "nate", "nat"],
        &["joseph", "joe", "joey"],
        &["thomas", "tom", "tommy"],
        &["charles", "charlie", "chuck", "chas"],
        &["michael", "mike", "mikey", "mick"],
        &["david", "dave", "davey"],
        &["daniel", "dan", "danny"],
        &["edward", "ed", "eddie", "ted", "ned"],
        &["theodore", "ted", "teddy", "theo"],
        &["anthony", "tony"],
        &["christopher", "chris", "kit"],
        &["matthew", "matt"],
        &["andrew", "andy", "drew"],
        &["benjamin", "ben", "benny"],
        &["samuel", "sam", "sammy"],
        &["stephen", "steven", "steve"],
        &["gregory", "greg"],
        &["patrick", "pat", "patty"],
        &["peter", "pete"],
        &["alexander", "alex", "al"],
        &["nicholas", "nick", "nicky"],
        &["timothy", "tim", "timmy"],
        &["kenneth", "ken", "kenny"],
        &["ronald", "ron", "ronnie"],
        &["donald", "don", "donnie"],
        &["gerald", "jerry", "gerry"],
        &["jerome", "jerry"],
        &["lawrence", "larry"],
        &["leonard", "len", "lenny", "leo"],
        &["frederick", "fred", "freddie"],
        &["alfred", "al", "alf", "fred"],
        &["franklin", "frank"],
        &["francis", "frank", "fran"],
        &["harold", "harry", "hal"],
        &["henry", "hank", "harry"],
        &["walter", "walt", "wally"],
        &["raymond", "ray"],
        &["eugene", "gene"],
        &["douglas", "doug"],
        &["jeffrey", "geoffrey", "jeff"],
        &["terrence", "terence", "terry"],
        &[
            "elizabeth",
            "beth",
            "betty",
            "liz",
            "lizzie",
            "eliza",
            "betsy",
            "libby",
        ],
        &["margaret", "maggie", "peggy", "meg", "marge", "margie"],
        &[
            "katherine",
            "catherine",
            "kathryn",
            "kathy",
            "cathy",
            "kate",
            "katie",
            "kay",
        ],
        &["patricia", "pat", "patty", "trish", "tricia"],
        &["jennifer", "jenny", "jen"],
        &["susan", "sue", "susie", "suzanne"],
        &["deborah", "debra", "debbie", "deb"],
        &["barbara", "barb", "babs"],
        &["dorothy", "dot", "dottie", "dorothea"],
        &["rebecca", "becky", "becca"],
        &["victoria", "vicky", "tori"],
        &["kimberly", "kim"],
        &["christine", "christina", "chris", "tina", "chrissy"],
        &["jacqueline", "jackie"],
        &["abigail", "abby"],
        &["virginia", "ginny"],
        &["frances", "fran", "frankie"],
        &["judith", "judy"],
        &["theresa", "teresa", "tess", "tessa"],
        &["pamela", "pam"],
        &["cynthia", "cindy"],
        &["sandra", "sandy"],
        &["samantha", "sam", "sammie"],
        &["alexandra", "alex", "lexi"],
        &["mary", "molly", "polly", "mamie"],
        &["ann", "anne", "anna", "annie"],
        &["helen", "nell", "ellie"],
        &["eleanor", "ellie", "nora"],
        &["sarah", "sara", "sally", "sadie"],
        &["carolyn", "caroline", "carol", "carrie"],
        &["lillian", "lily", "lil"],
    ];
    const _: () = assert!(GROUPS.len() <= 128);
    let mut out = 0u128;
    for (i, g) in GROUPS.iter().enumerate() {
        if g.contains(&w) {
            out |= 1 << i;
        }
    }
    out
}

/// Normalize a value for comparing, appended to `out`.
fn normalize(v: &str, kind: FieldKind, rd: crate::coltype::Reader, out: &mut String) {
    let v = v.trim();
    if v.is_empty() {
        return;
    }
    match kind {
        FieldKind::Exact => {
            if let Some(d) = rd
                .date(v)
                .filter(|_| v.chars().any(|c| !c.is_ascii_digit()))
            {
                let (y, m, dd) = crate::fill::civil_from_days(d.floor() as i64);
                if (1..=9999).contains(&y) {
                    out.push_str(&format!("{y:04}{m:02}{dd:02}"));
                    return;
                }
            }
            out.extend(
                v.chars()
                    .filter(char::is_ascii_alphanumeric)
                    .map(|c| c.to_ascii_lowercase()),
            );
        }
        FieldKind::Text | FieldKind::Name => {
            let folded = crate::clean::fold_accents(v);
            let mut words: Vec<String> = Vec::new();
            let mut w = String::new();
            for c in folded.chars() {
                if c.is_alphanumeric() {
                    w.extend(c.to_lowercase());
                } else if c == '\'' || c == '\u{2019}' {
                    // O'Brien = OBrien
                } else if !w.is_empty() {
                    words.push(std::mem::take(&mut w));
                }
            }
            if !w.is_empty() {
                words.push(w);
            }
            if kind == FieldKind::Name {
                // Titles dropped; a generation suffix kept (written one
                // way, last) so Jr and Sr are told apart.
                const DROP: &[&str] = &["mr", "mrs", "ms", "miss", "dr", "rev", "esq"];
                let mut kept: Vec<String> = Vec::with_capacity(words.len());
                let mut suffix = None;
                for (k, w) in words.iter().enumerate() {
                    let gen = match w.as_str() {
                        "jr" | "junior" => Some("jr"),
                        "sr" | "senior" => Some("sr"),
                        "ii" | "2nd" => Some("ii"),
                        "iii" | "3rd" => Some("iii"),
                        "iv" | "4th" => Some("iv"),
                        _ => None,
                    };
                    match gen {
                        Some(g) if k > 0 => suffix = Some(g),
                        _ if DROP.contains(&w.as_str()) => {}
                        _ => kept.push(w.clone()),
                    }
                }
                if !kept.is_empty() {
                    if let Some(g) = suffix {
                        kept.push(g.to_string());
                    }
                    words = kept;
                }
            }
            out.push_str(&words.join(" "));
        }
    }
}

/// A sound-alike code of a value's first word (letters grouped by sound,
/// first letter kept, four places), as a number. Nicknames get their
/// groups' keys besides (see `MatchSide::keys`).
fn code(v: &str, kind: FieldKind) -> u64 {
    let word = v.split(' ').next().unwrap_or("");
    if kind == FieldKind::Exact {
        return hash(word);
    }
    let group = |c: char| match c {
        'b' | 'f' | 'p' | 'v' => 1,
        'c' | 'g' | 'j' | 'k' | 'q' | 's' | 'x' | 'z' => 2,
        'd' | 't' => 3,
        'l' => 4,
        'm' | 'n' => 5,
        'r' => 6,
        _ => 0,
    };
    let mut chars = word.chars();
    let Some(first) = chars.next() else { return 0 };
    let mut out = u64::from(first as u32 & 0xFFFF) << 16;
    let mut last = group(first);
    let mut n = 0;
    for c in chars {
        let g = group(c);
        if g != 0 && g != last {
            out |= (g as u64) << (12 - 4 * n);
            n += 1;
            if n == 3 {
                break;
            }
        }
        if c != 'h' && c != 'w' {
            last = g;
        }
    }
    out
}

fn hash(s: &str) -> u64 {
    // FNV-1a.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

impl Document {
    /// Add columns for accepted pairs: "Match row" (the row on the other
    /// side, as numbered on screen), "Match score", and the brought-in
    /// columns of `other` (or, for near-duplicates, "Duplicate of row").
    /// `accepted`: (row id here, the other side's shown row, score).
    pub fn plan_match_columns(
        &self,
        accepted: &[(u32, u64, f32)],
        other: Option<&MatchSide>,
        bring_titles: &[String],
        keep_first: bool,
    ) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        let same = other.is_none();
        let at = self.col_count();
        let bw = other.map_or(0, |o| o.bring_width);
        let ncols = 2 + bw as u32;
        let mut plan = ColPlan::new(self);
        plan.insert = Some((at, ncols));
        plan.cols = ncols;
        if keep_first && self.row_count() > 0 {
            let mut t = vec![
                (
                    at,
                    if same {
                        "Duplicate of row"
                    } else {
                        "Match row"
                    }
                    .to_string(),
                ),
                (at + 1, "Match score".to_string()),
            ];
            for b in 0..bw {
                let title = bring_titles
                    .get(b)
                    .cloned()
                    .unwrap_or_else(|| format!("Column {}", b + 1));
                t.push((at + 2 + b as u32, title));
            }
            plan.changes.push((self.id_at(0).expect("a first row"), t));
        }
        for &(id, other_row, score) in accepted.iter().take(MAX_LISTED) {
            let mut cells = vec![
                (at, (other_row + 1).to_string()),
                (at + 1, format!("{score}")),
            ];
            if let Some(o) = other {
                let j = other_row
                    .checked_sub(o.first)
                    .map(|j| j as usize)
                    .filter(|&j| j < o.len())
                    .ok_or(Error::OutOfRange)?;
                for b in 0..bw {
                    cells.push((at + 2 + b as u32, o.bring[j * bw + b].clone()));
                }
            }
            plan.changes.push((id, cells));
        }
        plan.cells = plan.changes.len() as u64 * u64::from(ncols);
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

    #[test]
    fn likeness_of_names() {
        assert!(jaro_winkler("martha", "marhta") > 0.95);
        assert!(jaro_winkler("smith", "smyth") > 0.85);
        assert!(jaro_winkler("smith", "jones") < 0.5);
        assert_eq!(likeness("bill", "william", FieldKind::Name), 1.0);
        assert!(likeness("j", "john", FieldKind::Name) >= 0.85);
        assert!(likeness("ann marie", "marie ann", FieldKind::Name) > 0.99);
        assert_eq!(likeness("19800513", "19801305", FieldKind::Exact), 0.8);
        assert_eq!(likeness("37402", "37403", FieldKind::Exact), 0.0);
        let mut s = String::new();
        normalize(
            "Dr. Seán O'Brien Jr.",
            FieldKind::Name,
            crate::coltype::Reader::plain(ReadOptions::default()),
            &mut s,
        );
        assert_eq!(s, "sean obrien jr");
        assert_eq!(
            code("smith", FieldKind::Name),
            code("smyth", FieldKind::Name)
        );
        assert_ne!(nickname_groups("bill") & nickname_groups("william"), 0);
    }

    /// Review M3: Jr and Sr are different people; short forms in several
    /// groups match each; risky pairs are not equal.
    #[test]
    fn suffixes_and_nicknames() {
        let n = |v: &str| {
            let mut s = String::new();
            normalize(
                v,
                FieldKind::Name,
                crate::coltype::Reader::plain(ReadOptions::default()),
                &mut s,
            );
            s
        };
        let d = doc("name
John Smith Jr
John Smith Sr.
John Smith
");
        let k = [FieldKind::Name];
        let p = Progress::new();
        let side = MatchSide::build(&d, &[0], &k, &[], true, ReadOptions::default(), &p).unwrap();
        let w = [1.0];
        assert_eq!(score(&side, 0, &side, 1, &w).0, 0.0, "Jr vs Sr");
        assert_eq!(
            score(&side, 0, &side, 2, &w).0,
            100.0,
            "no suffix on one side"
        );
        assert_eq!(n("John Smith, Junior"), "john smith jr");
        for (a, b) in [
            ("pat", "patricia"),
            ("pat", "patrick"),
            ("chris", "christine"),
            ("chris", "christopher"),
            ("sam", "samantha"),
            ("jon", "jonathan"),
            ("bert", "albert"),
            ("ted", "theodore"),
        ] {
            assert_eq!(likeness(a, b, FieldKind::Name), 1.0, "{a} {b}");
        }
        assert!(likeness("terry", "theresa", FieldKind::Name) < 0.9);
        assert!(likeness("nancy", "ann", FieldKind::Name) < 0.9);
        // Never compared before: now found.
        let a = doc("first,last
Pat,Kim
Chris,Ford
Sam,Cole
Jon,Ortiz
Bert,Hall
Ted,Ng
J,Smith
");
        let b = doc("first,last
Patricia,Kim
Christine,Ford
Samantha,Cole
Jonathan,Ortiz
Albert,Hall
Theodore,Ng
Jane,Smith
");
        let k2 = [FieldKind::Name, FieldKind::Name];
        let r = ReadOptions::default();
        let sa = MatchSide::build(&a, &[0, 1], &k2, &[], true, r, &p).unwrap();
        let sb = MatchSide::build(&b, &[0, 1], &k2, &[], true, r, &p).unwrap();
        let spec = MatchSpec {
            fields: (0..2)
                .map(|c| MatchField {
                    col: c,
                    other_col: c,
                    kind: k2[c as usize],
                    weight: 1.0,
                })
                .collect(),
            min_score: 85.0,
            ..Default::default()
        };
        let rep = find_matches(&sa, Some(&sb), &spec, &p).unwrap();
        let mut rows: Vec<(u64, u64)> = rep.listed.iter().map(|c| (c.row, c.other_row)).collect();
        rows.sort();
        assert_eq!(rows, (1..=7).map(|r| (r, r)).collect::<Vec<_>>(), "{rep:?}");
    }

    /// Review M2: a huge group (everyone named John Smith) is compared
    /// within a window, quickly, and the report says so; a duplicate pair
    /// late in the file is still found.
    #[test]
    fn huge_groups_are_windowed_not_dropped() {
        let mut csv = String::from(
            "first,last,dob
",
        );
        for i in 0..20_000u32 {
            csv.push_str(&format!(
                "John,Smith,{}-{:02}-{:02}
",
                1900 + i % 100,
                1 + i % 12,
                1 + i % 28
            ));
        }
        // A late duplicate pair (same birth date, a spelling slip).
        csv.push_str(
            "John,Smith,1850-07-04
Jon,Smyth,1850-07-04
",
        );
        let d = doc(&csv);
        let k = [FieldKind::Name, FieldKind::Name, FieldKind::Exact];
        let p = Progress::new();
        let side =
            MatchSide::build(&d, &[0, 1, 2], &k, &[], true, ReadOptions::default(), &p).unwrap();
        let spec = MatchSpec {
            fields: (0..3)
                .map(|c| MatchField {
                    col: c,
                    other_col: c,
                    kind: k[c as usize],
                    weight: 1.0,
                })
                .collect(),
            min_score: 95.0,
            ..Default::default()
        };
        let t = std::time::Instant::now();
        let rep = find_matches(&side, None, &spec, &p).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(20));
        assert!(rep.windowed > 0);
        assert!(rep.compared < 20_000 * 200, "{}", rep.compared);
        assert!(
            rep.listed
                .iter()
                .any(|c| c.row == 20_002 && c.other_row == 20_001),
            "{:?}",
            rep.listed.iter().take(5).collect::<Vec<_>>()
        );
    }

    #[test]
    fn matches_across_lists_and_within() {
        let a = doc("first,last,dob\r\nWilliam,Smith,5/13/1980\r\nMartha,Jones,1/2/1975\r\nZed,Quill,3/3/1990\r\n");
        let b = doc("given,family,born\r\nBill,Smyth,1980-05-13\r\nMarhta,Jones,1975-01-02\r\nAmy,Long,2000-01-01\r\n");
        let kinds = [FieldKind::Name, FieldKind::Name, FieldKind::Exact];
        let p = Progress::new();
        let r = ReadOptions::default();
        let sa = MatchSide::build(&a, &[0, 1, 2], &kinds, &[], true, r, &p).unwrap();
        let sb = MatchSide::build(&b, &[0, 1, 2], &kinds, &[2], true, r, &p).unwrap();
        let spec = MatchSpec {
            fields: (0..3)
                .map(|c| MatchField {
                    col: c,
                    other_col: c,
                    kind: kinds[c as usize],
                    weight: 1.0,
                })
                .collect(),
            min_score: 75.0,
            ..Default::default()
        };
        let rep = find_matches(&sa, Some(&sb), &spec, &p).unwrap();
        assert_eq!(rep.total, 2, "{rep:?}");
        let rows: Vec<(u64, u64)> = rep.listed.iter().map(|c| (c.row, c.other_row)).collect();
        assert!(rows.contains(&(1, 1)) && rows.contains(&(2, 2)));
        assert!(rep.listed.iter().all(|c| c.score >= 75.0));
        // Accepted pairs add columns.
        let mut a = a;
        let acc: Vec<(u32, u64, f32)> = rep
            .listed
            .iter()
            .map(|c| (c.id, c.other_row, c.score))
            .collect();
        let plan = a
            .plan_match_columns(&acc, Some(&sb), &["born".into()], true)
            .unwrap();
        a.apply_cols(plan).unwrap();
        assert_eq!(a.cell(0, 3).unwrap(), "Match row");
        assert_eq!(a.cell(1, 3).unwrap(), "2");
        assert_eq!(a.cell(1, 5).unwrap(), "1980-05-13");
        assert_eq!(a.cell(3, 3).unwrap(), "");
        // Near-duplicates in one list.
        let d = doc("first,last\r\nRobert,Brown\r\nAnn,Lee\r\nBob,Browne\r\n");
        let k2 = [FieldKind::Name, FieldKind::Name];
        let sd = MatchSide::build(&d, &[0, 1], &k2, &[], true, r, &p).unwrap();
        let spec2 = MatchSpec {
            fields: spec.fields[..2].to_vec(),
            min_score: 80.0,
            ..Default::default()
        };
        let rep = find_matches(&sd, None, &spec2, &p).unwrap();
        assert_eq!(rep.total, 1);
        assert_eq!((rep.listed[0].row, rep.listed[0].other_row), (3, 1));
    }
}
