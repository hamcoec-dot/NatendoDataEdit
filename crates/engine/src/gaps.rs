//! Data > Find gaps in a sequence: the numbers missing from a column of
//! numbers or IDs (`1001`, `INV-00123`, `A-17-0042`).
//!
//! A cell is a prefix, one run of ASCII digits and a suffix. The rows of one
//! such **shape** are sorted and walked neighbour by neighbour, so nothing is
//! ever expanded: `1` then `10000000000` is one line. Only text is read, so a
//! Text or Id column works and no cell is turned into a floating point number.

use std::borrow::Cow;
use std::collections::HashMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress, View};
use crate::filter::FilterSpec;
use crate::ops::{check_cols, Cols, CHUNK};
use crate::reshape::{push_line, too_big, RowPlan, Table, MAX_TABLE_BYTES};
use crate::value::natural_cmp;
use crate::Error;

/// "Series by" columns at most.
pub const MAX_SERIES_COLS: usize = 2;
/// Series audited at most.
pub const MAX_SERIES: usize = 100_000;
/// Text held for series keys and shapes at most.
const MAX_KEY_BYTES: usize = 64 << 20;
/// Range lines kept (totals stay exact).
pub const MAX_RANGES: usize = 10_000;
/// Repeated values listed.
pub const MAX_DUPLICATES: usize = 1_000;
/// Series lines kept.
const MAX_SERIES_LINES: usize = 10_000;
/// Shapes listed with their counts.
pub const TOP_PATTERNS: usize = 12;
/// Row numbers shown as examples.
const EXAMPLE_ROWS: usize = 3;
/// Rows looked at to choose the digit run.
const SAMPLE_ROWS: u64 = 10_000;
/// A longer run of digits is not a sequence number.
const MAX_DIGITS: usize = 64;
/// Numbers from here up are not audited.
const NUMBER_LIMIT: u64 = 1 << 63;
const SEP: char = '\u{1f}';

/// One shape: the text around the number, and the number's width when the
/// numbers are padded with zeros (0: not padded).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pattern {
    pub prefix: String,
    pub suffix: String,
    #[serde(default)]
    pub width: u32,
}

impl Pattern {
    /// The shape in words: `INV-#####`, `A-17-#`.
    pub fn label(&self) -> String {
        let n = self.width.max(1) as usize;
        format!("{}{}{}", self.prefix, "#".repeat(n), self.suffix)
    }

    /// A number written in this shape.
    pub fn format(&self, n: u64) -> String {
        format!(
            "{}{:0w$}{}",
            self.prefix,
            n,
            self.suffix,
            w = self.width as usize
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GapsSpec {
    /// The column audited.
    pub col: u32,
    /// Up to [`MAX_SERIES_COLS`] columns whose text splits the audit.
    pub series: Vec<u32>,
    pub keep_first: bool,
    /// Which run of digits holds the number, counted from the right (0 = the
    /// last). `None`: chosen from a sample.
    pub run: Option<u32>,
    /// The shape audited. `None`: the most common.
    pub pattern: Option<Pattern>,
    /// Audit every shape as its own series.
    pub each_shape: bool,
    /// Typed first and last numbers (bare or in the shape); empty for the
    /// first and last found.
    pub start: String,
    pub end: String,
}

/// A row, by its number in the file (0-based); `None` for a row added by hand.
pub type FileRow = Option<u64>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PatternCount {
    pub pattern: Pattern,
    pub label: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeriesLine {
    /// The "series by" values, then the shape with "audit each shape".
    pub key: Vec<String>,
    pub first: String,
    pub last: String,
    pub rows: u64,
    pub distinct: u64,
    /// Decimal text (can be past what a script number holds).
    pub missing: String,
    /// Fewer than 2 rows and no typed start or end: not audited.
    pub too_few: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RangeLine {
    pub series: Vec<String>,
    pub from: String,
    pub to: String,
    pub count: String,
}

impl RangeLine {
    /// `INV-01005 to INV-01009 (5 missing)`, the series first.
    pub fn text(&self) -> String {
        let mut s = String::new();
        if !self.series.is_empty() {
            s.push_str(&self.series.join(" / "));
            s.push_str(": ");
        }
        s.push_str(&self.from);
        if self.from != self.to {
            s.push_str(" to ");
            s.push_str(&self.to);
        }
        s.push_str(&format!(" ({} missing)", self.count));
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Duplicate {
    pub series: Vec<String>,
    pub value: String,
    pub count: u64,
    pub rows: Vec<FileRow>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GapReport {
    /// Digit run audited, counted from the right.
    pub run: u32,
    /// The shape audited; `None` with "audit each shape" or when no cell has
    /// a number.
    pub pattern: Option<Pattern>,
    /// The most common shapes (at most [`TOP_PATTERNS`]).
    pub patterns: Vec<PatternCount>,
    /// Shapes found in all.
    pub pattern_total: u64,
    /// Shown data rows read.
    pub shown: u64,
    /// Rows whose number was audited.
    pub rows: u64,
    pub distinct: u64,
    pub blank: u64,
    pub other_shape: u64,
    pub other_rows: Vec<FileRow>,
    /// Numbers outside a typed start or end.
    pub outside: u64,
    /// Rows lower than the row before them in their series, in file order.
    pub drops: u64,
    pub drop_rows: Vec<FileRow>,
    /// Rows added by hand: in the sequence, not in the order check.
    pub added: u64,
    pub duplicate_values: u64,
    pub duplicate_extra: u64,
    pub duplicates: Vec<Duplicate>,
    pub series: Vec<SeriesLine>,
    pub series_total: u64,
    pub ranges: Vec<RangeLine>,
    pub range_total: u64,
    /// Decimal text.
    pub missing: String,
    /// Most values hold "," or "." between runs of digits.
    pub looks_like_amounts: bool,
    #[serde(skip)]
    beside: Vec<u64>,
    #[serde(skip)]
    beside_rows: u64,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    spec: GapsSpec,
}

/// The runs of ASCII digits in `s`, as byte ranges.
fn runs(s: &str) -> impl DoubleEndedIterator<Item = (usize, usize)> + '_ {
    let b = s.as_bytes();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let from = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push((from, i));
        } else {
            i += 1;
        }
    }
    out.into_iter()
}

/// The `k`th run of digits from the right, with no allocation.
fn run_from_right(s: &str, k: u32) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut end = b.len();
    let mut seen = 0;
    loop {
        while end > 0 && !b[end - 1].is_ascii_digit() {
            end -= 1;
        }
        if end == 0 {
            return None;
        }
        let mut from = end;
        while from > 0 && b[from - 1].is_ascii_digit() {
            from -= 1;
        }
        if seen == k {
            return Some((from, end));
        }
        seen += 1;
        end = from;
    }
}

/// A cell read as prefix, number and suffix.
struct Split<'a> {
    prefix: &'a str,
    suffix: &'a str,
    number: u64,
    len: u8,
    zero_led: bool,
}

fn split(t: &str, run: u32) -> Option<Split<'_>> {
    let (a, b) = run_from_right(t, run)?;
    let digits = &t[a..b];
    if digits.len() > MAX_DIGITS {
        return None;
    }
    let number: u64 = digits.parse().ok()?;
    if number >= NUMBER_LIMIT {
        return None;
    }
    Some(Split {
        prefix: &t[..a],
        suffix: &t[b..],
        number,
        len: digits.len() as u8,
        zero_led: digits.len() > 1 && digits.starts_with('0'),
    })
}

/// Distinct strings numbered in the order first met.
#[derive(Default)]
struct Names {
    ids: HashMap<String, u32>,
    names: Vec<String>,
    bytes: usize,
}

impl Names {
    fn id(&mut self, s: &str) -> Result<u32, Error> {
        if let Some(&i) = self.ids.get(s) {
            return Ok(i);
        }
        self.bytes += s.len() * 2 + 64;
        if self.bytes > MAX_KEY_BYTES {
            return Err(Error::Invalid(
                "there are too many different series or shapes in this column to audit".into(),
            ));
        }
        let i = self.names.len() as u32;
        self.ids.insert(s.to_string(), i);
        self.names.push(s.to_string());
        Ok(i)
    }
}

/// One cell that holds a number.
#[derive(Clone, Copy)]
struct Cell {
    number: u64,
    row: u32,
    /// Prefix and suffix.
    around: u32,
    series: u32,
    len: u8,
    zero_led: bool,
}

#[derive(Default)]
struct Part {
    cells: Vec<Cell>,
    around: Names,
    series: Names,
    blank: u64,
    other: u64,
    /// Rows with no number (the first few, in order).
    other_rows: Vec<u32>,
    amounts: u64,
}

/// Which digit run holds the sequence: the one whose values differ most in a
/// sample; ties take the one further right.
fn choose_run(values: &[String]) -> u32 {
    let mut seen: Vec<std::collections::HashSet<&str>> = Vec::new();
    for v in values {
        for (k, (a, b)) in runs(v.trim()).rev().enumerate().take(8) {
            if seen.len() <= k {
                seen.push(Default::default());
            }
            seen[k].insert(&v.trim()[a..b]);
        }
    }
    let mut best = 0;
    for (k, s) in seen.iter().enumerate() {
        if s.len() > seen[best].len() {
            best = k;
        }
    }
    best as u32
}

/// Whether two runs of digits are joined by just "," or ".".
fn amount_like(t: &str) -> bool {
    let mut last: Option<usize> = None;
    for (a, b) in runs(t) {
        if let Some(e) = last {
            if matches!(&t[e..a], "," | ".") {
                return true;
            }
        }
        last = Some(b);
    }
    false
}

fn cancelled(progress: &Progress) -> Result<(), Error> {
    progress.step(0)
}

/// A typed start or end, read through every audited shape.
fn typed(text: &str, shapes: &[&Pattern], what: &str) -> Result<Option<u64>, Error> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let digits = |s: &str| -> Option<u64> {
        if s.is_empty() || s.len() > MAX_DIGITS || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok().filter(|&n| n < NUMBER_LIMIT)
    };
    if let Some(n) = digits(t) {
        return Ok(Some(n));
    }
    let full: Vec<Option<u64>> = shapes
        .iter()
        .map(|p| {
            t.strip_prefix(p.prefix.as_str())
                .and_then(|r| r.strip_suffix(p.suffix.as_str()))
                .and_then(digits)
        })
        .collect();
    match full.first() {
        Some(&Some(n)) if full.iter().all(|&x| x == Some(n)) => Ok(Some(n)),
        _ => {
            let form = match shapes {
                [one] => format!("a number, or the form {}", one.label()),
                _ => "a number".to_string(),
            };
            Err(Error::Invalid(format!("type the {what} as {form}")))
        }
    }
}

fn set_bit(bits: &mut [u64], id: u32) -> bool {
    let w = &mut bits[(id >> 6) as usize];
    let m = 1u64 << (id & 63);
    let new = *w & m == 0;
    *w |= m;
    new
}

impl Document {
    /// Audit one column of numbers or IDs over the shown rows.
    pub fn gap_audit(&self, spec: &GapsSpec, progress: &Progress) -> Result<GapReport, Error> {
        self.ready_for_bulk()?;
        if spec.series.len() > MAX_SERIES_COLS {
            return Err(Error::Invalid(format!(
                "split the audit by at most {MAX_SERIES_COLS} columns"
            )));
        }
        let mut cols = vec![spec.col];
        cols.extend(&spec.series);
        check_cols(self, &cols)?;
        let reader = Cols::new(self, &cols);
        let rows = self.data_rows(spec.keep_first);
        let (r0, r1) = (rows.start, rows.end);

        let run = match spec.run {
            Some(k) => k,
            None => {
                let mut sample = Vec::new();
                let mut buf = Vec::new();
                for r in r0..r1.min(r0 + SAMPLE_ROWS) {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    sample.push(reader.get(&buf, 0).to_string());
                }
                choose_run(&sample)
            }
        };

        // Collect: one pass, each chunk with its own names.
        progress.start(r1 - r0);
        let chunks = ((r1 - r0) as usize).div_ceil(CHUNK);
        let parts: Vec<Part> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Part, Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut p = Part::default();
                p.cells.reserve((e - s) as usize);
                let mut buf: Vec<Cow<'_, str>> = Vec::new();
                let mut key = String::new();
                // The names of the row before: shapes repeat row after row.
                let mut last_around: Option<(String, u32)> = None;
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let t = reader.get(&buf, 0).trim();
                    if t.is_empty() {
                        p.blank += 1;
                        continue;
                    }
                    p.amounts += u64::from((r - r0) < SAMPLE_ROWS && amount_like(t));
                    let Some(x) = split(t, run) else {
                        p.other += 1;
                        if p.other_rows.len() < EXAMPLE_ROWS {
                            p.other_rows.push(id);
                        }
                        continue;
                    };
                    // The separator cannot be told apart inside a shape.
                    if x.prefix.contains(SEP) || x.suffix.contains(SEP) {
                        p.other += 1;
                        continue;
                    }
                    key.clear();
                    key.push_str(x.prefix);
                    key.push(SEP);
                    key.push_str(x.suffix);
                    let around = match &last_around {
                        Some((k, i)) if *k == key => *i,
                        _ => {
                            let i = p.around.id(&key)?;
                            last_around = Some((key.clone(), i));
                            i
                        }
                    };
                    let series = if spec.series.is_empty() {
                        0
                    } else {
                        key.clear();
                        for k in 0..spec.series.len() {
                            let v = reader.get(&buf, k + 1);
                            key.push_str(&v.len().to_string());
                            key.push(':');
                            key.push_str(v);
                        }
                        p.series.id(&key)?
                    };
                    p.cells.push(Cell {
                        number: x.number,
                        row: id,
                        around,
                        series,
                        len: x.len,
                        zero_led: x.zero_led,
                    });
                }
                progress.step(e - s)?;
                Ok(p)
            })
            .collect::<Result<_, _>>()?;

        // One set of names, in the order first met.
        let mut around = Names::default();
        let mut series = Names::default();
        if spec.series.is_empty() {
            series.id("")?;
        }
        let mut out = GapReport {
            run,
            shown: r1 - r0,
            revision: self.data_revision(),
            spec: spec.clone(),
            ..Default::default()
        };
        let total: usize = parts.iter().map(|p| p.cells.len()).sum();
        let mut cells: Vec<Cell> = Vec::with_capacity(total);
        let mut amounts = 0;
        let mut other_ids: Vec<u32> = Vec::new();
        for mut p in parts {
            cancelled(progress)?;
            let a: Vec<u32> = p
                .around
                .names
                .iter()
                .map(|n| around.id(n))
                .collect::<Result<_, _>>()?;
            let s: Vec<u32> = p
                .series
                .names
                .iter()
                .map(|n| series.id(n))
                .collect::<Result<_, _>>()?;
            for c in &mut p.cells {
                c.around = a[c.around as usize];
                if !spec.series.is_empty() {
                    c.series = s[c.series as usize];
                }
            }
            cells.append(&mut p.cells);
            out.blank += p.blank;
            out.other_shape += p.other;
            other_ids.extend(p.other_rows);
            amounts += p.amounts;
        }
        out.looks_like_amounts = amounts * 2 > out.shown.min(SAMPLE_ROWS);
        if series.names.len() > MAX_SERIES {
            return Err(too_many_series());
        }

        // Shapes: for each (prefix and suffix, digit count, zero-led) the
        // width its cells belong to.
        let mut counts: HashMap<(u32, u8, bool), u64> = HashMap::new();
        for c in &cells {
            *counts.entry((c.around, c.len, c.zero_led)).or_default() += 1;
        }
        let mut padded: HashMap<u32, Vec<(u8, u64)>> = HashMap::new();
        for (&(a, len, zero), &n) in &counts {
            if zero {
                padded.entry(a).or_default().push((len, n));
            }
        }
        let mut shapes: Vec<(Pattern, u64)> = Vec::new();
        let mut shape_ids: HashMap<(u32, u8), usize> = HashMap::new();
        let mut of: HashMap<(u32, u8, bool), usize> = HashMap::new();
        let mut keys: Vec<(u32, u8, bool)> = counts.keys().copied().collect();
        keys.sort_unstable();
        for k in keys {
            let (a, len, zero) = k;
            let width = match padded.get(&a) {
                None => Some(0),
                Some(_) if zero => Some(len),
                // No leading zero: the most common padded width it fits.
                Some(w) => w
                    .iter()
                    .filter(|(w, _)| *w <= len)
                    .max_by_key(|(w, n)| (*n, *w))
                    .map(|(w, _)| *w),
            };
            let Some(width) = width else { continue };
            let i = *shape_ids.entry((a, width)).or_insert_with(|| {
                let (prefix, suffix) = around.names[a as usize]
                    .split_once(SEP)
                    .expect("joined above");
                shapes.push((
                    Pattern {
                        prefix: prefix.to_string(),
                        suffix: suffix.to_string(),
                        width: u32::from(width),
                    },
                    0,
                ));
                shapes.len() - 1
            });
            shapes[i].1 += counts[&k];
            of.insert(k, i);
        }
        // Most common first.
        let mut rank: Vec<usize> = (0..shapes.len()).collect();
        rank.sort_by(|&a, &b| {
            let (x, y) = (&shapes[a], &shapes[b]);
            y.1.cmp(&x.1)
                .then_with(|| natural_cmp(x.0.prefix.as_bytes(), y.0.prefix.as_bytes()))
                .then_with(|| natural_cmp(x.0.suffix.as_bytes(), y.0.suffix.as_bytes()))
                .then_with(|| x.0.width.cmp(&y.0.width))
        });
        out.pattern_total = shapes.len() as u64;
        out.patterns = rank
            .iter()
            .take(TOP_PATTERNS)
            .map(|&i| PatternCount {
                pattern: shapes[i].0.clone(),
                label: shapes[i].0.label(),
                count: shapes[i].1,
            })
            .collect();
        let chosen: Option<usize> = if spec.each_shape {
            None
        } else {
            match &spec.pattern {
                Some(want) => Some(shapes.iter().position(|(p, _)| p == want).ok_or_else(
                    || Error::Invalid("no shown row has the shape picked".into()),
                )?),
                None => rank.first().copied(),
            }
        };
        out.pattern = chosen.map(|i| shapes[i].0.clone());
        let audited: Vec<&Pattern> = match chosen {
            Some(i) => vec![&shapes[i].0],
            None => shapes.iter().map(|(p, _)| p).collect(),
        };
        let start = typed(&spec.start, &audited, "start")?;
        let end = typed(&spec.end, &audited, "end")?;
        if let (Some(a), Some(b)) = (start, end) {
            if a > b {
                return Err(Error::Invalid(
                    "the start is greater than the end".into(),
                ));
            }
        }

        // Keep the audited cells, each with its final series.
        cancelled(progress)?;
        let mut groups: HashMap<(u32, usize), u32> = HashMap::new();
        let mut group_of: Vec<(u32, usize)> = Vec::new();
        let mut other = 0u64;
        let mut kept = 0;
        for i in 0..cells.len() {
            let mut c = cells[i];
            let shape = of.get(&(c.around, c.len, c.zero_led)).copied();
            let Some(shape) = shape.filter(|&s| chosen.is_none_or(|w| w == s)) else {
                other += 1;
                // Rows are in the order read; keep the first few of these.
                if other <= EXAMPLE_ROWS as u64 {
                    other_ids.push(c.row);
                }
                continue;
            };
            let key = (c.series, shape);
            let g = match groups.get(&key) {
                Some(&g) => g,
                None => {
                    if group_of.len() >= MAX_SERIES {
                        return Err(too_many_series());
                    }
                    let g = group_of.len() as u32;
                    groups.insert(key, g);
                    group_of.push(key);
                    g
                }
            };
            c.series = g;
            cells[kept] = c;
            kept += 1;
        }
        cells.truncate(kept);
        out.other_shape += other;
        // A typed range with nothing in it is still a gap.
        if group_of.is_empty() && spec.series.is_empty() && start.is_some() && end.is_some() {
            if let Some(i) = chosen {
                group_of.push((0, i));
            }
        }
        other_ids.sort_by_key(|&id| id);
        other_ids.truncate(EXAMPLE_ROWS);
        let base = u64::from(self.next_row_id());
        let file_rows = self.original_rows();
        let file_row = |id: u32| (u64::from(id) < file_rows).then_some(u64::from(id));
        out.other_rows = other_ids.iter().map(|&i| file_row(i)).collect();
        out.rows = cells.len() as u64;

        // Order, in file order (row ids are file rows).
        cancelled(progress)?;
        let by_row = |c: &Cell| (c.series, c.row);
        if !cells.par_windows(2).all(|w| by_row(&w[0]) <= by_row(&w[1])) {
            cells.par_sort_unstable_by_key(by_row);
        }
        cancelled(progress)?;
        let mut before: Option<(u32, u64)> = None;
        for c in &cells {
            if u64::from(c.row) >= file_rows {
                out.added += 1;
                continue;
            }
            if before.is_some_and(|(s, n)| s == c.series && c.number < n) {
                out.drops += 1;
                if out.drop_rows.len() < EXAMPLE_ROWS {
                    out.drop_rows.push(file_row(c.row));
                }
            }
            before = Some((c.series, c.number));
        }

        // The sequence.
        let by_number = |c: &Cell| (c.series, c.number, c.row);
        if !cells.par_windows(2).all(|w| by_number(&w[0]) <= by_number(&w[1])) {
            cells.par_sort_unstable_by_key(by_number);
        }
        cancelled(progress)?;

        // Series in a fixed order: by their text.
        let key_of = |g: u32| -> Vec<String> {
            let (s, shape) = group_of[g as usize];
            let mut key = Vec::new();
            if !spec.series.is_empty() {
                let mut rest = series.names[s as usize].as_str();
                while let Some((n, tail)) = rest.split_once(':') {
                    let n: usize = n.parse().unwrap_or(0);
                    key.push(tail[..n].to_string());
                    rest = &tail[n..];
                }
            }
            if spec.each_shape {
                key.push(shapes[shape].0.label());
            }
            key
        };
        let mut spans: Vec<(u32, usize, usize)> = Vec::new();
        let mut i = 0;
        for g in 0..group_of.len() as u32 {
            let from = i;
            while i < cells.len() && cells[i].series == g {
                i += 1;
            }
            spans.push((g, from, i));
        }
        let mut keyed: Vec<(Vec<String>, u32, usize, usize)> = spans
            .into_iter()
            .map(|(g, a, b)| (key_of(g), g, a, b))
            .collect();
        keyed.sort_by(|x, y| {
            x.0.iter()
                .zip(&y.0)
                .map(|(a, b)| natural_cmp(a.as_bytes(), b.as_bytes()))
                .find(|o| o.is_ne())
                .unwrap_or(x.1.cmp(&y.1))
        });
        out.series_total = keyed.len() as u64;

        out.beside = vec![0u64; (base as usize).div_ceil(64)];
        let mut missing_all: u128 = 0;
        for (n, (key, g, a, b)) in keyed.into_iter().enumerate() {
            if n % 1024 == 0 {
                cancelled(progress)?;
            }
            let pattern = &shapes[group_of[g as usize].1].0;
            let own = &cells[a..b];
            let too_few = own.len() < 2 && start.is_none() && end.is_none();
            // The values in order, each with the rows that hold it.
            let mut missing: u128 = 0;
            let mut distinct = 0;
            let mut previous: Option<(usize, usize, u64)> = None;
            let mut gap = |out: &mut GapReport, from: u64, to: u64| {
                let count = u128::from(to - from) + 1;
                missing += count;
                out.range_total += 1;
                if out.ranges.len() < MAX_RANGES {
                    out.ranges.push(RangeLine {
                        series: key.clone(),
                        from: pattern.format(from),
                        to: pattern.format(to),
                        count: count.to_string(),
                    });
                }
            };
            let beside = |out: &mut GapReport, from: usize, to: usize| {
                for c in &own[from..to] {
                    out.beside_rows += u64::from(set_bit(&mut out.beside, c.row));
                }
            };
            // The next number wanted: from the typed start, else the first.
            let mut next: Option<u64> = start;
            let mut j = 0;
            while j < own.len() {
                let v = own[j].number;
                let mut k = j + 1;
                while k < own.len() && own[k].number == v {
                    k += 1;
                }
                distinct += 1;
                if k - j > 1 {
                    out.duplicate_values += 1;
                    out.duplicate_extra += (k - j - 1) as u64;
                    if out.duplicates.len() < MAX_DUPLICATES {
                        out.duplicates.push(Duplicate {
                            series: key.clone(),
                            value: pattern.format(v),
                            count: (k - j) as u64,
                            rows: own[j..k]
                                .iter()
                                .take(EXAMPLE_ROWS)
                                .map(|c| file_row(c.row))
                                .collect(),
                        });
                    }
                }
                if start.is_some_and(|s| v < s) || end.is_some_and(|e| v > e) {
                    out.outside += (k - j) as u64;
                    // The value just below a typed start is still a neighbour.
                    if start.is_some_and(|s| v + 1 == s) {
                        previous = Some((j, k, v));
                    }
                    // The first value past the end closes the last gap.
                    if let (Some(e), Some(n)) = (end, next) {
                        if v > e && !too_few && n <= e {
                            gap(&mut out, n, e);
                            if let Some((x, y, _)) = previous {
                                beside(&mut out, x, y);
                            }
                            if v - 1 == e {
                                beside(&mut out, j, k);
                            }
                            next = None;
                        }
                    }
                } else {
                    if let Some(n) = next {
                        if v > n && !too_few {
                            gap(&mut out, n, v - 1);
                            if let Some((x, y, p)) = previous {
                                if p + 1 == n {
                                    beside(&mut out, x, y);
                                }
                            }
                            beside(&mut out, j, k);
                        }
                    }
                    next = v.checked_add(1);
                    previous = Some((j, k, v));
                }
                j = k;
            }
            // A typed end past the last value (when nothing lay beyond it).
            if let (Some(e), Some(n)) = (end, next) {
                if n <= e && !too_few && own.last().is_none_or(|c| c.number <= e) {
                    gap(&mut out, n, e);
                    if let Some((x, y, _)) = previous {
                        beside(&mut out, x, y);
                    }
                }
            }
            missing_all += missing;
            out.distinct += distinct;
            if out.series.len() < MAX_SERIES_LINES {
                let show = |c: Option<&Cell>| c.map_or(String::new(), |c| pattern.format(c.number));
                out.series.push(SeriesLine {
                    key,
                    first: show(own.first()),
                    last: show(own.last()),
                    rows: own.len() as u64,
                    distinct,
                    missing: missing.to_string(),
                    too_few,
                });
            }
        }
        out.missing = missing_all.to_string();
        Ok(out)
    }

    /// A plan that shows only the rows next to a gap (and the title row).
    pub fn plan_show_gaps(&self, report: &GapReport) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        let shown = self.data_rows(report.spec.keep_first);
        if report.revision != self.data_revision() || shown.end - shown.start != report.shown {
            return Err(Error::Invalid(
                "the table changed since the audit: run the audit again".into(),
            ));
        }
        if report.beside_rows == 0 {
            return Err(Error::Invalid("no row is next to a gap".into()));
        }
        let base = self.next_row_id();
        let mut bits = report.beside.clone();
        bits.resize((base as usize).div_ceil(64), 0);
        if report.spec.keep_first && self.row_count() > 0 {
            let id = self.id_at(0).expect("a first row");
            set_bit(&mut bits, id);
        }
        let mut plan = RowPlan::new(self);
        plan.rows = report.beside_rows;
        plan.view = Some(View::new(
            FilterSpec {
                keep_first: report.spec.keep_first,
                ..Default::default()
            },
            bits,
            base,
        ));
        Ok(plan)
    }
}

fn too_many_series() -> Error {
    Error::Invalid(format!(
        "there are more than {MAX_SERIES} series: pick fewer columns to split by"
    ))
}

impl GapReport {
    /// What was audited.
    pub fn spec(&self) -> &GapsSpec {
        &self.spec
    }

    /// The summary and the range lines as a table. `titles`: the "series by"
    /// column titles. Numbers are never expanded.
    pub fn table(&self, titles: &[String]) -> Result<Table, Error> {
        let mut head: Vec<String> = titles.to_vec();
        if self.spec.each_shape {
            head.push("Shape".into());
        }
        let keys = head.len();
        let cols = keys.max(1) + 3;
        let mut out = Vec::new();
        let mut rows = 0u64;
        let mut line = |out: &mut Vec<u8>, mut cells: Vec<String>| {
            cells.resize(cols, String::new());
            push_line(out, &cells);
            rows += 1;
        };
        let mut total = |out: &mut Vec<u8>, name: &str, value: String| {
            line(out, vec![name.to_string(), value]);
        };
        if let [one] = self.series.as_slice() {
            total(&mut out, "First", one.first.clone());
            total(&mut out, "Last", one.last.clone());
        } else {
            total(&mut out, "Series", self.series_total.to_string());
        }
        if let Some(p) = &self.pattern {
            total(&mut out, "Shape", p.label());
        }
        total(&mut out, "Rows", self.rows.to_string());
        total(&mut out, "Different numbers", self.distinct.to_string());
        total(&mut out, "Missing", self.missing.clone());
        total(&mut out, "Repeated numbers", self.duplicate_values.to_string());
        total(&mut out, "Extra rows of repeats", self.duplicate_extra.to_string());
        total(&mut out, "Drops in file order", self.drops.to_string());
        total(&mut out, "Different shape", self.other_shape.to_string());
        total(&mut out, "Blank", self.blank.to_string());
        if !self.spec.start.trim().is_empty() || !self.spec.end.trim().is_empty() {
            total(&mut out, "Outside the range", self.outside.to_string());
        }
        if self.range_total > self.ranges.len() as u64 {
            total(
                &mut out,
                "Ranges not listed",
                (self.range_total - self.ranges.len() as u64).to_string(),
            );
        }
        line(&mut out, Vec::new());
        if keys == 0 {
            head.push(String::new());
        }
        head.extend(["From", "To", "Missing"].map(String::from));
        if keys == 0 {
            head.remove(0);
        }
        line(&mut out, head);
        for r in &self.ranges {
            let mut cells = r.series.clone();
            cells.extend([r.from.clone(), r.to.clone(), r.count.clone()]);
            line(&mut out, cells);
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
        }
        Ok(Table {
            bytes: out,
            rows,
            cols: cols as u32,
        })
    }

    /// The range lines as text, one per line (the lines listed, never one per
    /// missing number).
    pub fn copy_text(&self) -> String {
        let mut s = String::new();
        for r in &self.ranges {
            s.push_str(&r.text());
            s.push('\n');
        }
        let more = self.range_total - self.ranges.len() as u64;
        if more > 0 {
            s.push_str(&format!("{more} more ranges not copied\n"));
        }
        s
    }
}

#[cfg(test)]
#[path = "gaps_tests.rs"]
mod tests;
