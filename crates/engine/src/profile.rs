//! Column profile: what a column holds at a glance. The shapes of its values
//! (`99999` 98%, `99999-9999` 1.5%: digits as 9, letters as A), a small
//! histogram (numbers, dates, or text lengths), the shortest and longest
//! values, and odd values: shapes seen only a few times, and numbers far from
//! the rest. Every line comes with an expression that shows its rows (Filter
//! by formula), so the UI can offer "show these rows".

use std::collections::HashMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::{check_cols, Cols, CHUNK};
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
        out.push(if c.is_ascii_digit() || c.is_numeric() {
            '9'
        } else if c.is_alphabetic() {
            'A'
        } else {
            c
        });
    }
    out
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
