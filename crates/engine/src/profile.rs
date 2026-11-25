//! Column profile: what a column holds at a glance. The shapes of its values
//! (`99999` 98%, `99999-9999` 1.5%: digits as 9, letters as A), a small
//! histogram (numbers, dates, or text lengths), the shortest and longest
//! values, and odd values: shapes seen only a few times, and numbers far from
//! the rest. Every line comes with an expression that shows its rows (Filter
//! by formula), so the UI can offer "show these rows".

use std::collections::HashMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress, View};
use crate::filter::FilterSpec;
use crate::ops::{check_cols, Cols, CHUNK};
use crate::reshape::RowPlan;
use crate::value::{parse_date, parse_number, ReadOptions};
use crate::Error;

/// Different shapes kept per chunk and in all.
const MAX_SHAPES: usize = 20_000;
/// Shape length kept (longer shapes end in "…").
pub const MAX_SHAPE_CHARS: usize = 40;
/// Shapes listed (most common first).
const TOP_SHAPES: usize = 12;
/// Bars in the histogram.
pub const BINS: usize = 20;
/// A shape making up at most this share of the filled cells (and at most
/// [`RARE_MAX`] cells) is odd.
const RARE_SHARE: f64 = 0.001;
const RARE_MAX: u64 = 100;
/// Example values kept per odd group.
const EXAMPLES: usize = 5;

/// A value's shape: each digit 9, each letter A (a or A kept by case is
/// noise here), spaces and punctuation as they are.
pub fn shape_of(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(MAX_SHAPE_CHARS + 3));
    for (n, c) in s.chars().enumerate() {
        if n >= MAX_SHAPE_CHARS {
            out.push('…');
            break;
        }
        out.push(shape_char(c));
    }
    out
}

/// One character of a shape: a digit 9, a letter A, anything else as it is.
fn shape_char(c: char) -> char {
    if c.is_ascii_digit() || c.is_numeric() {
        '9'
    } else if c.is_alphabetic() {
        'A'
    } else {
        c
    }
}

/// A 64-bit hash of `shape_of(s)` worked out while reading `s`, with no
/// allocation (the pill counts a shape per sampled cell).
pub fn shape_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |c: char| {
        h ^= u64::from(c as u32);
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    if s.is_ascii() {
        // Plain ASCII (most cells): bytes, no decoding; the same characters as below.
        for (n, &b) in s.as_bytes().iter().enumerate() {
            if n >= MAX_SHAPE_CHARS {
                feed('…');
                break;
            }
            feed(if b.is_ascii_digit() {
                '9'
            } else if b.is_ascii_alphabetic() {
                'A'
            } else {
                char::from(b)
            });
        }
        return h;
    }
    for (n, c) in s.chars().enumerate() {
        if n >= MAX_SHAPE_CHARS {
            feed('…');
            break;
        }
        feed(shape_char(c));
    }
    h
}

/// Exactly `shape_of(s) == target`, compared as `s` is read, with no
/// allocation: a value of more than [`MAX_SHAPE_CHARS`] characters matches
/// only a shape that ends in "…" after its first 40 characters.
pub fn shape_matches(s: &str, target: &str) -> bool {
    let mut t = target.chars();
    for (n, c) in s.chars().enumerate() {
        if n >= MAX_SHAPE_CHARS {
            return t.next() == Some('…') && t.next().is_none();
        }
        if t.next() != Some(shape_char(c)) {
            return false;
        }
    }
    t.next().is_none()
}

/// A regular expression that matches exactly the values whose shape is `shape` (compare
/// [`shape_matches`]): a 9 is any Unicode number (`\p{N}`, as `shape_of` counts them), an A any
/// alphabetic character (`\p{Alphabetic}`), every other character itself; a shape of 40 characters
/// and an ellipsis ("…") is the first 40 exactly and then at least one more character. Meant to be
/// used for the whole cell (a `Pattern` check matches the whole cell).
pub fn shape_regex(shape: &str) -> String {
    let chars: Vec<char> = shape.chars().collect();
    let capped = chars.len() == MAX_SHAPE_CHARS + 1 && chars[MAX_SHAPE_CHARS] == '…';
    let body = if capped {
        &chars[..MAX_SHAPE_CHARS]
    } else {
        &chars[..]
    };
    let mut out = String::new();
    for &c in body {
        match c {
            '9' => out.push_str(r"\p{N}"),
            'A' => out.push_str(r"\p{Alphabetic}"),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    if capped {
        out.push_str("(?s:.+)");
    }
    out
}

/// "Show the rows whose shape is (or is not) this one" (the pattern pill).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapeFilter {
    pub col: u32,
    pub keep_first: bool,
    /// The shape, as the pill shows it (9 for digits, A for letters).
    pub shape: String,
    /// Rows that do NOT have this shape (blank cells left out unless `include_blanks`).
    pub negate: bool,
    /// Blank cells count as rows to show (in either direction).
    pub include_blanks: bool,
    /// The data revision the pill was worked out for (what the rows hold or their
    /// order): a filter on a document whose data has changed since is refused
    /// ([`Error::Changed`]).
    pub revision: Option<u64>,
}

impl Document {
    /// A filter to the rows whose value in `spec.col` has (or, with `negate`,
    /// has not) the shape `spec.shape`. It reads every row SHOWN NOW, not a
    /// sample, so with a filter already on it narrows that filter (the other
    /// rows stay as they are). Not an undo step, like any filter.
    pub fn plan_shape_filter(
        &self,
        spec: &ShapeFilter,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[spec.col])?;
        if spec.revision.is_some_and(|r| r != self.data_revision()) {
            return Err(Error::Changed);
        }
        let reader = Cols::new(self, &[spec.col]);
        let rows = self.data_rows(spec.keep_first);
        let ids = self.par_rows(rows, progress, |_, id, buf| {
            reader.read(self, id, buf);
            let v = reader.get(buf, 0);
            let blank = v.trim().is_empty();
            let show = if blank {
                spec.include_blanks
            } else {
                shape_matches(v, &spec.shape) != spec.negate
            };
            Ok(show.then_some(id))
        })?;
        let base = self.next_row_id();
        let mut bits = vec![0u64; (base as usize).div_ceil(64)];
        for &id in &ids {
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        }
        if spec.keep_first && self.row_count() > 0 {
            let id = self.id_at(0).expect("a first row");
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        }
        let mut plan = RowPlan::new(self);
        plan.rows = ids.len() as u64;
        plan.view = Some(View::new(
            FilterSpec {
                keep_first: spec.keep_first,
                ..Default::default()
            },
            bits,
            base,
        ));
        Ok(plan)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileSpec {
    pub col: u32,
    pub keep_first: bool,
    pub read: ReadOptions,
}

/// One shape and how many filled cells have it, with an example.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeCount {
    pub shape: String,
    pub count: u64,
    pub example: String,
}

/// What the histogram counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HistKind {
    #[default]
    Numbers,
    Dates,
    /// Text length in characters.
    Lengths,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Histogram {
    pub kind: HistKind,
    /// Lower edge of each bar; the last bar includes `high`.
    pub low: f64,
    pub high: f64,
    pub counts: Vec<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub col: u32,
    /// Shown data rows looked at, and of them blank.
    pub rows: u64,
    pub blanks: u64,
    /// Most common shapes first.
    pub shapes: Vec<ShapeCount>,
    /// Different shapes (exact unless more than 20,000).
    pub shape_kinds: u64,
    pub histogram: Option<Histogram>,
    pub shortest: Option<String>,
    pub longest: Option<String>,
    pub min_len: u64,
    pub max_len: u64,
    /// Shapes seen only a few times.
    pub rare_shapes: Vec<ShapeCount>,
    /// Numbers more than four standard deviations from the mean.
    pub outliers: u64,
    pub outlier_examples: Vec<String>,
    pub mean: Option<f64>,
    pub sd: Option<f64>,
}

#[derive(Default)]
struct Acc {
    rows: u64,
    blanks: u64,
    shapes: HashMap<String, (u64, String)>,
    shapes_over: bool,
    numbers: u64,
    dates: u64,
    /// The numbers' running mean and sum of squared distances from it
    /// (Welford's method: steady for big numbers that vary little).
    mean: f64,
    m2: f64,
    nmin: f64,
    nmax: f64,
    dmin: f64,
    dmax: f64,
    shortest: Option<(usize, String)>,
    longest: Option<(usize, String)>,
    /// Some numbers (or dates) of each chunk, for the histogram's range.
    sample: Vec<f64>,
}

/// Numbers sampled per chunk, and in all.
const SAMPLE_CHUNK: usize = 2_048;
const SAMPLE_ALL: usize = 200_000;

impl Acc {
    fn new() -> Acc {
        Acc {
            nmin: f64::INFINITY,
            nmax: f64::NEG_INFINITY,
            dmin: f64::INFINITY,
            dmax: f64::NEG_INFINITY,
            ..Default::default()
        }
    }

    fn add(&mut self, v: &str, read: ReadOptions) {
        self.rows += 1;
        if v.trim().is_empty() {
            self.blanks += 1;
            return;
        }
        let shape = shape_of(v);
        let full = self.shapes.len() >= MAX_SHAPES;
        match self.shapes.get_mut(&shape) {
            Some(e) => e.0 += 1,
            None if !full => {
                self.shapes
                    .insert(shape, (1, v.chars().take(200).collect()));
            }
            None => self.shapes_over = true,
        }
        let len = v.chars().count();
        if self.shortest.as_ref().is_none_or(|s| len < s.0) {
            self.shortest = Some((len, v.chars().take(200).collect()));
        }
        if self.longest.as_ref().is_none_or(|s| len > s.0) {
            self.longest = Some((len, v.chars().take(200).collect()));
        }
        if let Some(x) = parse_number(v, read) {
            self.numbers += 1;
            let delta = x - self.mean;
            self.mean += delta / self.numbers as f64;
            self.m2 += delta * (x - self.mean);
            self.nmin = self.nmin.min(x);
            self.nmax = self.nmax.max(x);
            if self.sample.len() < SAMPLE_CHUNK {
                self.sample.push(x);
            }
        } else if let Some(d) = parse_date(v, read) {
            self.dates += 1;
            self.dmin = self.dmin.min(d);
            self.dmax = self.dmax.max(d);
        }
    }

    fn merge(mut self, o: Acc) -> Acc {
        self.rows += o.rows;
        self.blanks += o.blanks;
        self.shapes_over |= o.shapes_over;
        for (k, (n, ex)) in o.shapes {
            let full = self.shapes.len() >= MAX_SHAPES;
            match self.shapes.get_mut(&k) {
                Some(e) => e.0 += n,
                None if !full => {
                    self.shapes.insert(k, (n, ex));
                }
                None => self.shapes_over = true,
            }
        }
        // Two parts' means and spreads joined (Chan et al.).
        if o.numbers > 0 {
            if self.numbers == 0 {
                self.mean = o.mean;
                self.m2 = o.m2;
            } else {
                let (na, nb) = (self.numbers as f64, o.numbers as f64);
                let n = na + nb;
                let delta = o.mean - self.mean;
                self.mean += delta * nb / n;
                self.m2 += o.m2 + delta * delta * na * nb / n;
            }
        }
        self.numbers += o.numbers;
        self.dates += o.dates;
        let room = SAMPLE_ALL.saturating_sub(self.sample.len());
        self.sample.extend(o.sample.into_iter().take(room));
        self.nmin = self.nmin.min(o.nmin);
        self.nmax = self.nmax.max(o.nmax);
        self.dmin = self.dmin.min(o.dmin);
        self.dmax = self.dmax.max(o.dmax);
        if let Some(s) = o.shortest {
            if self.shortest.as_ref().is_none_or(|x| s.0 < x.0) {
                self.shortest = Some(s);
            }
        }
        if let Some(s) = o.longest {
            if self.longest.as_ref().is_none_or(|x| s.0 > x.0) {
                self.longest = Some(s);
            }
        }
        self
    }
}

impl Document {
    /// Profile one column over the shown data rows (two passes, both in
    /// parallel, with progress and Cancel).
    pub fn column_profile(
        &self,
        spec: &ProfileSpec,
        progress: &Progress,
    ) -> Result<Profile, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[spec.col])?;
        let reader = Cols::new(self, &[spec.col]);
        let rows = self.data_rows(spec.keep_first);
        let read = spec.read;
        let n = rows.end - rows.start;
        // Pass 1: shapes, lengths, ranges.
        progress.start(n * 2);
        let acc = self
            .chunks_of(rows.clone())
            .into_par_iter()
            .map(|(s, e)| -> Result<Acc, Error> {
                let mut a = Acc::new();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    a.add(reader.get(&buf, 0), read);
                }
                progress.step(e - s)?;
                Ok(a)
            })
            .try_reduce(Acc::new, |a, b| Ok(a.merge(b)))?;
        let filled = acc.rows - acc.blanks;
        let mut shapes: Vec<ShapeCount> = acc
            .shapes
            .into_iter()
            .map(|(shape, (count, example))| ShapeCount {
                shape,
                count,
                example,
            })
            .collect();
        shapes.sort_unstable_by(|a, b| b.count.cmp(&a.count).then_with(|| a.shape.cmp(&b.shape)));
        let shape_kinds = shapes.len() as u64;
        let rare_cap = ((filled as f64 * RARE_SHARE) as u64).min(RARE_MAX);
        // Odd shapes only where one shape clearly dominates (a free-text
        // column has no "normal" shape).
        let dominant = shapes.first().is_some_and(|s| s.count * 2 >= filled);
        let mut rare_shapes: Vec<ShapeCount> = if dominant {
            shapes
                .iter()
                // At most 1 in 1,000 (and 100) cells, or once or twice in a
                // small table.
                .filter(|s| {
                    (s.count <= rare_cap.max(1) && s.count * 1000 <= filled.max(1))
                        || (s.count <= 2 && s.count * 20 <= filled)
                })
                .take(TOP_SHAPES)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        rare_shapes
            .sort_unstable_by(|a, b| a.count.cmp(&b.count).then_with(|| a.shape.cmp(&b.shape)));
        shapes.truncate(TOP_SHAPES);

        // What the histogram shows: numbers or dates when most filled cells
        // are, else text lengths.
        let (kind, low, high) = if filled > 0 && acc.numbers * 10 >= filled * 8 {
            (HistKind::Numbers, acc.nmin, acc.nmax)
        } else if filled > 0 && acc.dates * 10 >= filled * 8 {
            (HistKind::Dates, acc.dmin.floor(), acc.dmax.floor())
        } else {
            let (lo, hi) = (
                acc.shortest.as_ref().map_or(0, |s| s.0) as f64,
                acc.longest.as_ref().map_or(0, |s| s.0) as f64,
            );
            (HistKind::Lengths, lo, hi)
        };
        let (mean, sd) = if acc.numbers > 1 {
            let var = (acc.m2 / acc.numbers as f64).max(0.0);
            (Some(acc.mean), Some(var.sqrt()))
        } else {
            (None, None)
        };
        // The bars cover the middle 98% of the numbers (from a sample);
        // the few beyond go in the end bars, so one stray value does not
        // squash every other bar into one.
        let (low, high) = if kind == HistKind::Numbers && acc.sample.len() >= 50 {
            let mut s = acc.sample.clone();
            s.sort_unstable_by(f64::total_cmp);
            let at = |q: f64| s[((s.len() - 1) as f64 * q) as usize];
            let (a, b) = (at(0.01), at(0.99));
            if b > a {
                (a, b)
            } else {
                (low, high)
            }
        } else {
            (low, high)
        };
        let has_hist = filled > 0 && low.is_finite() && high.is_finite();
        let cut = |x: f64| -> usize {
            if high <= low {
                0
            } else {
                (((x - low) / (high - low) * BINS as f64).max(0.0) as usize).min(BINS - 1)
            }
        };
        // Pass 2: the histogram and the numbers far from the rest.
        let far =
            |x: f64| matches!((mean, sd), (Some(m), Some(s)) if s > 0.0 && (x - m).abs() > 4.0 * s);
        let (counts, outliers, examples) = self
            .chunks_of(rows)
            .into_par_iter()
            .map(|(s, e)| -> Result<(Vec<u64>, u64, Vec<String>), Error> {
                let mut counts = vec![0u64; BINS];
                let (mut out, mut ex) = (0u64, Vec::new());
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let v = reader.get(&buf, 0);
                    if v.trim().is_empty() {
                        continue;
                    }
                    let x = match kind {
                        HistKind::Numbers => parse_number(v, read),
                        HistKind::Dates => parse_date(v, read).map(f64::floor),
                        HistKind::Lengths => Some(v.chars().count() as f64),
                    };
                    if let Some(x) = x {
                        if has_hist {
                            counts[cut(x)] += 1;
                        }
                        if kind == HistKind::Numbers && far(x) {
                            out += 1;
                            if ex.len() < EXAMPLES {
                                ex.push(v.chars().take(200).collect());
                            }
                        }
                    }
                }
                progress.step(e - s)?;
                Ok((counts, out, ex))
            })
            .try_reduce(
                || (vec![0u64; BINS], 0, Vec::new()),
                |mut a, b| {
                    a.0.iter_mut().zip(b.0).for_each(|(x, y)| *x += y);
                    a.1 += b.1;
                    for x in b.2 {
                        if a.2.len() < EXAMPLES {
                            a.2.push(x);
                        }
                    }
                    Ok(a)
                },
            )?;
        Ok(Profile {
            col: spec.col,
            rows: acc.rows,
            blanks: acc.blanks,
            shapes,
            shape_kinds,
            histogram: has_hist.then_some(Histogram {
                kind,
                low,
                high,
                counts,
            }),
            min_len: acc.shortest.as_ref().map_or(0, |s| s.0 as u64),
            max_len: acc.longest.as_ref().map_or(0, |s| s.0 as u64),
            shortest: acc.shortest.map(|s| s.1),
            longest: acc.longest.map(|s| s.1),
            rare_shapes,
            outliers,
            outlier_examples: examples,
            mean,
            sd,
        })
    }

    /// Shown rows `rows` in chunks.
    fn chunks_of(&self, rows: std::ops::Range<u64>) -> Vec<(u64, u64)> {
        (rows.start..rows.end)
            .step_by(CHUNK)
            .map(|s| (s, (s + CHUNK as u64).min(rows.end)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(body: &str) -> Document {
        Document::from_bytes(
            body.as_bytes().to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap()
    }

    fn lcg(seed: u64) -> impl FnMut() -> u64 {
        let mut x = seed;
        move || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            x >> 33
        }
    }

    /// A random value: ASCII and other digits, letters with accents, punctuation, the ellipsis,
    /// spaces, and sometimes more than 40 characters.
    fn random_value(r: &mut impl FnMut() -> u64) -> String {
        const ALPHA: [char; 16] = [
            '0', '7', 'a', 'Z', 'é', 'ß', '٣', '²', '-', '.', ' ', '(', '@', '…', 'ñ', '5',
        ];
        let len = match r() % 4 {
            0 => (r() % 6) as usize,
            1 => 38 + (r() % 5) as usize,
            2 => 40 + (r() % 30) as usize,
            _ => (r() % 15) as usize,
        };
        (0..len).map(|_| ALPHA[(r() % 16) as usize]).collect()
    }

    /// The hash of `shape_of(s)` the slow way (the same feed as `shape_hash`).
    fn slow_hash(s: &str) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for c in shape_of(s).chars() {
            h ^= u64::from(c as u32);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    }

    /// The values and shapes in crates/engine/src/testdata/shapes.json: the browser stand-in's
    /// `shapeOf` (ui/src/lib/insights.ts) is tested against the same file.
    const SHAPE_VALUES: [&str; 18] = [
        "555-1234",
        "(555) 123-4567",
        "José",
        "O'Neil",
        "12345-6789",
        "٣٤٥",
        "x²y",
        "ÀÉÎ ñ",
        "",
        "   ",
        "a…b",
        "ⅣⅤ",
        "1234567890123456789012345678901234567890",
        "12345678901234567890123456789012345678901",
        "abcdefghijklmnopqrstuvwxyz-0123456789-ABCDEFGHIJKLMNOPQRSTUVWXYZ",
        "ID#4471/B",
        "7 Maple St.",
        "e\u{301}",
    ];

    #[test]
    fn the_shapes_in_the_shared_fixture_are_what_shape_of_gives() {
        let fx: serde_json::Value =
            serde_json::from_str(include_str!("testdata/shapes.json")).unwrap();
        let rows = fx["cases"].as_array().unwrap();
        assert_eq!(rows.len(), SHAPE_VALUES.len());
        for (row, v) in rows.iter().zip(SHAPE_VALUES) {
            assert_eq!(row["value"].as_str().unwrap(), v);
            assert_eq!(row["shape"].as_str().unwrap(), shape_of(v), "{v:?}");
        }
    }

    /// Writes the fixture file (run when SHAPE_VALUES changes):
    /// `cargo test -p natendo-engine --lib print_shape_fixture -- --ignored`.
    #[test]
    #[ignore]
    fn print_shape_fixture() {
        let cases: Vec<serde_json::Value> = SHAPE_VALUES
            .iter()
            .map(|v| serde_json::json!({ "value": v, "shape": shape_of(v) }))
            .collect();
        let fx = serde_json::json!({
            "about": "values and the shapes the engine (profile.rs shape_of) gives them; the browser stand-in (insights.ts shapeOf) must agree",
            "cases": cases
        });
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata/shapes.json");
        std::fs::write(path, serde_json::to_string_pretty(&fx).unwrap() + "\n").unwrap();
    }

    #[test]
    fn the_rules_in_the_shared_fixture_are_what_shape_regex_gives() {
        let fx: serde_json::Value =
            serde_json::from_str(include_str!("testdata/shape_rules.json")).unwrap();
        let rows = fx["cases"].as_array().unwrap();
        assert_eq!(rows.len(), SHAPE_VALUES.len());
        for (row, v) in rows.iter().zip(SHAPE_VALUES) {
            let shape = shape_of(v);
            assert_eq!(row["shape"].as_str().unwrap(), shape);
            assert_eq!(row["regex"].as_str().unwrap(), shape_regex(&shape), "{shape:?}");
        }
    }

    /// Writes the fixture file: `cargo test -p natendo-engine --lib print_shape_rules -- --ignored`.
    #[test]
    #[ignore]
    fn print_shape_rules() {
        let cases: Vec<serde_json::Value> = SHAPE_VALUES
            .iter()
            .map(|v| {
                let shape = shape_of(v);
                serde_json::json!({ "shape": shape, "regex": shape_regex(&shape) })
            })
            .collect();
        let fx = serde_json::json!({
            "about": "shapes and the pattern the engine (profile.rs shape_regex) makes of each; the browser stand-in (lint.ts shapeRegex) must give the same text",
            "cases": cases
        });
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata/shape_rules.json");
        std::fs::write(path, serde_json::to_string_pretty(&fx).unwrap() + "
").unwrap();
    }

    #[test]
    fn shape_regex_matches_exactly_what_shape_matches_does() {
        let mut r = lcg(17);
        let pool: Vec<String> = (0..300).map(|_| random_value(&mut r)).collect();
        let mut shapes: Vec<String> = pool.iter().map(|v| shape_of(v)).collect();
        // (Only real shapes: a letter or digit in a "shape" cannot come from shape_of, which turns them into A and 9.)
        shapes.extend((0..100).map(|_| shape_of(&random_value(&mut r))));
        let compiled: Vec<regex::Regex> = shapes
            .iter()
            .map(|s| regex::Regex::new(&format!(r"\A(?:{})\z", shape_regex(s))).unwrap())
            .collect();
        let mut agreed_true = 0;
        for _ in 0..100_000 {
            let v = &pool[(r() % 300) as usize];
            let i = (r() % shapes.len() as u64) as usize;
            let want = shape_matches(v, &shapes[i]);
            assert_eq!(
                compiled[i].is_match(v),
                want,
                "value {v:?} shape {:?}",
                shapes[i]
            );
            agreed_true += u32::from(want);
        }
        assert!(agreed_true > 100);
        // Through the engine's own matcher, as a Pattern check uses it (whole cell).
        for (v, s) in [
            ("555-1234", "999-9999"),
            ("٣٤٥", "999"),
            ("x²y", "A9A"),
            ("José", "AAAA"),
        ] {
            let m = crate::find::Matcher::new(&crate::find::Pattern {
                text: shape_regex(s),
                kind: crate::find::PatternKind::Regex,
                match_case: true,
                whole_cell: true,
                ..Default::default()
            })
            .unwrap();
            assert!(m.is_match(v), "{v} {s}");
            assert!(!m.is_match(&format!("{v}!")), "{v} {s}");
        }
        // The cap form.
        let long = "7".repeat(55);
        let capped = shape_of(&long);
        let re = regex::Regex::new(&format!(r"\A(?:{})\z", shape_regex(&capped))).unwrap();
        assert!(
            re.is_match(&long) && re.is_match(&"1".repeat(41)) && !re.is_match(&"1".repeat(40))
        );
    }

    #[test]
    fn shape_matches_is_exactly_shape_of_equals_target() {
        let mut r = lcg(3);
        let pool: Vec<String> = (0..300).map(|_| random_value(&mut r)).collect();
        let shapes: Vec<String> = pool.iter().map(|v| shape_of(v)).collect();
        let mut matched = 0u32;
        for _ in 0..100_000 {
            let v = &pool[(r() % 300) as usize];
            // A target that is some value's own shape (often the value's), or a random string.
            let target = if r() % 3 == 0 {
                random_value(&mut r)
            } else {
                shapes[(r() % 300) as usize].clone()
            };
            let want = shape_of(v) == target;
            assert_eq!(
                shape_matches(v, &target),
                want,
                "value {v:?} target {target:?}"
            );
            matched += u32::from(want);
        }
        assert!(matched > 100, "the property test saw {matched} matches");
        // The cap: 40 characters match exactly; the 41st makes the shape end in an ellipsis.
        let forty = "1".repeat(40);
        assert!(shape_matches(&forty, &"9".repeat(40)));
        assert!(!shape_matches(&forty, &format!("{}…", "9".repeat(40))));
        let long = "1".repeat(55);
        assert!(shape_matches(&long, &format!("{}…", "9".repeat(40))));
        assert!(!shape_matches(&long, &"9".repeat(40)));
    }

    #[test]
    fn the_streaming_hash_is_the_hash_of_shape_of() {
        let mut r = lcg(9);
        for _ in 0..100_000 {
            let v = random_value(&mut r);
            assert_eq!(shape_hash(&v), slow_hash(&v), "{v:?}");
        }
        // Plain ASCII, short and over the 40-character cap (the byte fast path).
        for n in [0usize, 1, 9, 39, 40, 41, 80] {
            let v: String = (0..n)
                .map(|i| ['4', 'k', '-', ' ', '#', 'Z', '.'][(i * 7 + n) % 7])
                .collect();
            assert_eq!(shape_hash(&v), slow_hash(&v), "{v:?}");
        }
    }

    /// A column of made-up values and a brute-force list of the rows (document positions 1..) that match.
    fn column(r: &mut impl FnMut() -> u64, n: usize) -> (String, Vec<String>) {
        let vals: Vec<String> = (0..n)
            .map(|_| match r() % 7 {
                0 => String::new(),
                1 => format!("{:03}-{:04}", r() % 1000, r() % 10000),
                2 => format!("{:05}", r() % 100_000),
                3 => format!("{:05}-{:04}", r() % 100_000, r() % 10_000),
                4 => format!("Ana{}", r() % 9),
                _ => format!("{:03}-{:04}", r() % 1000, r() % 10000),
            })
            .collect();
        let mut csv = String::from("v\r\n");
        for v in &vals {
            csv.push_str(v);
            csv.push_str("\r\n");
        }
        (csv, vals)
    }

    fn shown_values(d: &Document) -> Vec<String> {
        (1..d.row_count()).map(|r| d.cell(r, 0).unwrap()).collect()
    }

    #[test]
    fn the_shape_filter_equals_a_brute_force_scan() {
        let mut r = lcg(21);
        let p = Progress::new();
        for case in 0..60 {
            let n = 20 + (r() % 200) as usize;
            let (csv, _) = column(&mut r, n);
            let target = "999-9999".to_string();
            // Sometimes rows are hidden by hand first (the filter then narrows what is shown).
            let hide = (case % 3 == 1).then(|| (1 + r() % 10, 1 + r() % 8));
            let make = || {
                let mut d = doc(&csv);
                if let Some((at, n)) = hide {
                    d.hide_rows(at, n).unwrap();
                }
                d
            };
            let before = shown_values(&make());
            for (negate, include_blanks) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                let mut d = make();
                let spec = ShapeFilter {
                    col: 0,
                    keep_first: true,
                    shape: target.clone(),
                    negate,
                    include_blanks,
                    revision: Some(d.data_revision()),
                };
                let plan = d.plan_shape_filter(&spec, &p).unwrap();
                let want: Vec<String> = before
                    .iter()
                    .filter(|v| {
                        if v.trim().is_empty() {
                            include_blanks
                        } else {
                            (shape_of(v) == target) != negate
                        }
                    })
                    .cloned()
                    .collect();
                assert_eq!(
                    plan.rows as usize,
                    want.len(),
                    "case {case} negate {negate} blanks {include_blanks}"
                );
                d.apply_rows(plan).unwrap();
                assert_eq!(
                    shown_values(&d),
                    want,
                    "case {case} negate {negate} blanks {include_blanks}"
                );
            }
        }
    }

    #[test]
    fn a_shape_filter_narrows_the_filter_that_is_already_on() {
        // 10 phone-shaped rows, 5 odd ones; a filter keeps rows 1..=8 only.
        let mut csv = String::from("v\r\n");
        for i in 0..15 {
            if i % 3 == 2 {
                csv.push_str(&format!("odd{i}\r\n"));
            } else {
                csv.push_str(&format!("555-{:04}\r\n", i));
            }
        }
        let mut d = doc(&csv);
        let p = Progress::new();
        d.hide_rows(9, 7).unwrap();
        let shown = shown_values(&d);
        assert_eq!(shown.len(), 8);
        let spec = ShapeFilter {
            col: 0,
            keep_first: true,
            shape: "999-9999".into(),
            negate: true,
            include_blanks: false,
            revision: None,
        };
        let plan = d.plan_shape_filter(&spec, &p).unwrap();
        d.apply_rows(plan).unwrap();
        // Only the odd values among the rows that were shown (rows 3, 6 of the 8), not the later ones.
        assert_eq!(
            shown_values(&d),
            vec!["odd2".to_string(), "odd5".to_string()]
        );
    }

    #[test]
    fn a_shape_filter_for_a_changed_document_is_refused_and_cancel_changes_nothing() {
        let (csv, _) = column(&mut lcg(5), 50);
        let mut d = doc(&csv);
        let p = Progress::new();
        let spec = ShapeFilter {
            col: 0,
            keep_first: true,
            shape: "999-9999".into(),
            revision: Some(d.data_revision()),
            ..Default::default()
        };
        d.set_cell(2, 0, "edited".into()).unwrap();
        assert!(matches!(
            d.plan_shape_filter(&spec, &p),
            Err(Error::Changed)
        ));
        let spec = ShapeFilter {
            revision: Some(d.data_revision()),
            ..spec
        };
        let cancelled = Progress::new();
        cancelled
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            d.plan_shape_filter(&spec, &cancelled),
            Err(Error::Cancelled)
        ));
        assert!(!d.is_filtered());
    }

    #[test]
    fn spread_of_big_numbers_that_vary_little() {
        // 13-digit numbers 1,000,000,000,000 to ...009, many times over,
        // across chunks: the spread is about 2.87.
        let mut csv = String::from("n\r\n");
        for i in 0..200_000u64 {
            csv.push_str(&format!("{}\r\n", 1_000_000_000_000u64 + i % 10));
        }
        let d = doc(&csv);
        let p = d
            .column_profile(
                &ProfileSpec {
                    col: 0,
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        let sd = p.sd.unwrap();
        assert!((sd - 2.8723).abs() < 1e-3, "{sd}");
        assert!((p.mean.unwrap() - 1_000_000_000_004.5).abs() < 1e-3);
        assert_eq!(p.outliers, 0);
    }

    #[test]
    fn shapes() {
        assert_eq!(shape_of("37421"), "99999");
        assert_eq!(shape_of("37421-1234"), "99999-9999");
        assert_eq!(shape_of("Ann Lee"), "AAA AAA");
        assert_eq!(shape_of("José"), "AAAA");
        assert_eq!(
            shape_of(&"x".repeat(50)).chars().count(),
            MAX_SHAPE_CHARS + 1
        );
    }

    #[test]
    fn profiles_zip_codes() {
        let mut body = String::from("ZIP\n");
        for i in 0..2000 {
            body.push_str(&format!("{}\n", 37000 + i % 900));
        }
        body.push_str("1234\n37421-1234\n\n");
        let d = doc(&body);
        let p = d
            .column_profile(
                &ProfileSpec {
                    col: 0,
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(p.rows, 2003);
        assert_eq!(p.blanks, 1);
        assert_eq!(p.shapes[0].shape, "99999");
        assert_eq!(p.shapes[0].count, 2000);
        let rare: Vec<&str> = p.rare_shapes.iter().map(|s| s.shape.as_str()).collect();
        assert!(
            rare.contains(&"9999") && rare.contains(&"99999-9999"),
            "{rare:?}"
        );
        let h = p.histogram.unwrap();
        assert_eq!(h.kind, HistKind::Numbers);
        assert_eq!(h.counts.iter().sum::<u64>(), 2001);
        // 1234 is far from the rest.
        assert_eq!(p.outliers, 1);
        assert_eq!(p.outlier_examples, ["1234"]);
        assert_eq!((p.min_len, p.max_len), (4, 10));
    }

    #[test]
    fn text_columns_have_lengths_and_no_odd_shapes() {
        let d = doc("Name\nAnn Lee\nBo\nCy Diaz\nDee Fox Smith\n");
        let p = d
            .column_profile(
                &ProfileSpec {
                    col: 0,
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(p.histogram.unwrap().kind, HistKind::Lengths);
        assert!(p.rare_shapes.is_empty());
        assert_eq!(p.shortest.as_deref(), Some("Bo"));
        assert_eq!(p.longest.as_deref(), Some("Dee Fox Smith"));
    }
}
