//! Column insights: a thin strip under each heading (View › Column
//! insights) with how full a column is, how many different values it holds
//! and a tiny bar chart. Worked out from a sample of the shown rows (the
//! start, middle and end of the file), so it never slows opening; the UI
//! asks for a small sample first and a bigger one after.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::ops::{check_cols, Cols};
use crate::profile::{shape_hash, shape_of};
use crate::value::ReadOptions;
use crate::Error;

/// Rows sampled at most.
pub const MAX_SAMPLE: u64 = 200_000;
/// Cells read at most for one strip (columns × sampled rows).
pub const MAX_CELLS: u64 = 4_000_000;
/// Columns asked about at once.
pub const MAX_COLS: usize = 512;
/// Bars in the strip's chart.
pub const BARS: usize = 8;
/// Different shapes counted per column (past this, new shapes are not counted).
const MAX_SHAPES: usize = 20_000;
/// The pill shows when one shape makes up at least this share of the filled cells.
pub const PILL_SHARE: f64 = 0.9;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InsightSpec {
    /// Logical columns (those on screen).
    pub cols: Vec<u32>,
    pub keep_first: bool,
    /// Rows to sample (fewer when the file is smaller).
    pub sample: u64,
    pub read: ReadOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Insight {
    pub col: u32,
    /// Share of the sampled cells that are filled (0 to 1).
    pub filled: f64,
    /// Different values in the sample.
    pub distinct: u64,
    /// Every filled sampled value was different (an ID-like column).
    pub unique: bool,
    /// Mostly numbers: the bars are a histogram; else the most common
    /// values' shares.
    pub numbers: bool,
    /// Bar heights, the tallest 1.
    pub bars: Vec<f32>,
    /// The pattern pill: the shape (9 for digits, A for letters) that makes up
    /// at least 90% of the filled sampled cells while some cells have another
    /// shape; None otherwise. Ties are broken by the smallest shape text.
    pub top_shape: Option<String>,
    /// That shape's share of the filled sampled cells (0 when no pill).
    pub top_share: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Insights {
    pub cols: Vec<Insight>,
    /// Rows looked at, of `rows` shown data rows.
    pub sampled: u64,
    pub rows: u64,
    /// The data revision these figures are for (a pill's filter is refused once
    /// the rows have changed).
    pub revision: u64,
}

/// `take` positions out of `a..b`: all of them, or a third each from the
/// start, the middle and the end.
fn sample_rows(a: u64, b: u64, take: u64) -> Vec<u64> {
    let n = b.saturating_sub(a);
    if n <= take {
        return (a..b).collect();
    }
    let part = take / 3;
    let mid = a + n / 2 - part / 2;
    let mut out: Vec<u64> = (a..a + part).collect();
    out.extend(mid..mid + part);
    out.extend(b - (take - 2 * part)..b);
    out
}

/// A hasher for keys that already are hashes (the shape hash): the key itself.
#[derive(Default)]
struct KeyIsHash(u64);

impl Hasher for KeyIsHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8) | u64::from(b);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// Shapes counted over the filled sampled values: (hash of the shape, count, index of the first value with
/// it), at most MAX_SHAPES of them; the last one hit is checked first (neighbouring values mostly share a
/// shape), the rest through an index by hash.
#[derive(Default)]
struct ShapeCounts {
    entries: Vec<(u64, u32, usize)>,
    index: HashMap<u64, usize, BuildHasherDefault<KeyIsHash>>,
    last: usize,
}

impl ShapeCounts {
    fn add(&mut self, h: u64, at: usize) {
        if let Some(e) = self.entries.get_mut(self.last) {
            if e.0 == h {
                e.1 += 1;
                return;
            }
        }
        if let Some(&k) = self.index.get(&h) {
            self.entries[k].1 += 1;
            self.last = k;
        } else if self.entries.len() < MAX_SHAPES {
            self.last = self.entries.len();
            self.index.insert(h, self.last);
            self.entries.push((h, 1, at));
        }
    }
}

/// The dominant shape among the filled sampled values and its share, when it is the pill's. Ties go to the
/// smallest shape text (built from a real value of each tied shape), so the answer does not depend on how
/// the work was split.
fn pill(vals: &[String], shapes: &ShapeCounts, filled: usize) -> (Option<String>, f64) {
    if filled < 2 {
        return (None, 0.0);
    }
    let Some(most) = shapes.entries.iter().map(|e| e.1).max() else {
        return (None, 0.0);
    };
    let share = f64::from(most) / filled as f64;
    if share < PILL_SHARE || most as usize == filled {
        return (None, 0.0);
    }
    let top = shapes
        .entries
        .iter()
        .filter(|e| e.1 == most)
        .map(|e| shape_of(&vals[e.2]))
        .min();
    (top, share)
}

fn one(col: u32, vals: &[String], read: ReadOptions) -> Insight {
    let n = vals.len();
    let mut counts: HashMap<&str, u32> = HashMap::new();
    let mut nums: Vec<f64> = Vec::new();
    let mut filled = 0usize;
    let mut shapes = ShapeCounts::default();
    for (i, v) in vals.iter().enumerate() {
        if v.trim().is_empty() {
            continue;
        }
        filled += 1;
        *counts.entry(v.as_str()).or_insert(0) += 1;
        shapes.add(shape_hash(v), i);
        if let Some(x) = crate::coltype::Reader::plain(read).number(v) {
            if x.is_finite() {
                nums.push(x);
            }
        }
    }
    let distinct = counts.len() as u64;
    let numbers = filled > 0 && nums.len() * 10 >= filled * 9 && distinct > BARS as u64;
    let mut bars: Vec<f32> = if numbers {
        // A histogram over the middle 98% (far-off values would squash it).
        nums.sort_unstable_by(f64::total_cmp);
        let lo = nums[nums.len() / 100];
        let hi = nums[nums.len() - 1 - nums.len() / 100];
        let mut bins = [0u32; BARS];
        if hi > lo {
            for &x in &nums {
                let k = (((x - lo) / (hi - lo)) * BARS as f64).floor();
                bins[(k.max(0.0) as usize).min(BARS - 1)] += 1;
            }
        } else {
            bins[0] = nums.len() as u32;
        }
        bins.iter().map(|&c| c as f32).collect()
    } else {
        let mut top: Vec<u32> = counts.values().copied().collect();
        top.sort_unstable_by(|a, b| b.cmp(a));
        top.truncate(BARS);
        top.into_iter().map(|c| c as f32).collect()
    };
    let most = bars.iter().copied().fold(0.0f32, f32::max);
    if most > 0.0 {
        for b in &mut bars {
            *b /= most;
        }
    }
    let (top_shape, top_share) = pill(vals, &shapes, filled);
    Insight {
        col,
        filled: if n == 0 {
            0.0
        } else {
            filled as f64 / n as f64
        },
        distinct,
        unique: filled > 1 && distinct == filled as u64,
        numbers,
        bars,
        top_shape,
        top_share,
    }
}

impl Document {
    /// The insight strip for `spec.cols`, from a sample of the shown rows.
    pub fn column_insights(&self, spec: &InsightSpec) -> Result<Insights, Error> {
        if spec.cols.len() > MAX_COLS {
            return Err(Error::Invalid(format!(
                "column insights cover at most {MAX_COLS} columns at once"
            )));
        }
        check_cols(self, &spec.cols)?;
        let rows = self.data_rows(spec.keep_first);
        let ncols = spec.cols.len().max(1) as u64;
        let take = spec
            .sample
            .clamp(1, MAX_SAMPLE)
            .min(MAX_CELLS / ncols)
            .max(1);
        let picks = sample_rows(rows.start, rows.end, take);
        let reader = Cols::new(self, &spec.cols);
        // Read the sampled rows in parallel, each part by column.
        let parts: Vec<Vec<Vec<String>>> = picks
            .par_chunks(4096)
            .map(|chunk| -> Result<Vec<Vec<String>>, Error> {
                let mut out = vec![Vec::with_capacity(chunk.len()); spec.cols.len()];
                let mut buf = Vec::new();
                for &r in chunk {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for (i, o) in out.iter_mut().enumerate() {
                        o.push(reader.get(&buf, i).to_string());
                    }
                }
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let cols: Vec<Insight> = spec
            .cols
            .par_iter()
            .enumerate()
            .map(|(i, &c)| {
                let vals: Vec<String> = parts.iter().flat_map(|p| p[i].iter().cloned()).collect();
                one(c, &vals, spec.read)
            })
            .collect();
        Ok(Insights {
            cols,
            sampled: picks.len() as u64,
            rows: rows.end - rows.start,
            revision: self.data_revision(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn pills(values: &[&str]) -> (Option<String>, f64) {
        let mut csv = String::from("v\r\n");
        for v in values {
            csv.push_str(v);
            csv.push_str("\r\n");
        }
        let d =
            Document::from_bytes(csv.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let r = d
            .column_insights(&InsightSpec {
                cols: vec![0],
                keep_first: true,
                sample: 1000,
                ..Default::default()
            })
            .unwrap();
        (r.cols[0].top_shape.clone(), r.cols[0].top_share)
    }

    #[test]
    fn the_pill_names_a_dominant_shape_in_phones_zips_and_names() {
        // Phones in three layouts, ZIP vs ZIP+4, names with accents.
        let mut phones: Vec<String> = (0..99).map(|i| format!("555-{:04}", i)).collect();
        phones.push("(555) 123-4567".into());
        let refs: Vec<&str> = phones.iter().map(String::as_str).collect();
        let (shape, share) = pills(&refs);
        assert_eq!(shape.as_deref(), Some("999-9999"));
        assert!((share - 0.99).abs() < 1e-9);
        let mut zips: Vec<String> = (0..95).map(|i| format!("{:05}", 37000 + i)).collect();
        zips.extend((0..5).map(|i| format!("{:05}-{:04}", 37400 + i, i)));
        let refs: Vec<&str> = zips.iter().map(String::as_str).collect();
        let (shape, share) = pills(&refs);
        assert_eq!(shape.as_deref(), Some("99999"));
        assert!((share - 0.95).abs() < 1e-9);
        // Accented names are letters, so "José" and "Jose" share a shape.
        let names = vec!["José"; 9]
            .into_iter()
            .chain(["Jose"])
            .chain(["O'Neil"])
            .collect::<Vec<_>>();
        let (shape, _) = pills(&names);
        assert_eq!(shape.as_deref(), Some("AAAA"));
    }

    #[test]
    fn no_pill_when_the_shape_is_not_dominant_or_is_the_only_one() {
        // 80% / 20%: not dominant. Every value the same shape: nothing differs. Blanks do not count.
        let mut v: Vec<String> = (0..80).map(|i| format!("{:03}", i)).collect();
        v.extend((0..20).map(|i| format!("x{i}")));
        let refs: Vec<&str> = v.iter().map(String::as_str).collect();
        assert_eq!(pills(&refs).0, None);
        let same: Vec<String> = (0..50).map(|i| format!("{:03}", i)).collect();
        let refs: Vec<&str> = same.iter().map(String::as_str).collect();
        assert_eq!(pills(&refs).0, None);
        let mut blanks: Vec<String> = (0..50).map(|_| String::new()).collect();
        blanks.extend((0..9).map(|i| format!("{:03}", i)));
        blanks.push("zz".into());
        let refs: Vec<&str> = blanks.iter().map(String::as_str).collect();
        assert_eq!(pills(&refs).0.as_deref(), Some("999"));
    }

    #[test]
    fn the_pill_is_the_same_with_one_thread_and_many() {
        let mut vals: Vec<String> = (0..4000)
            .map(|i| format!("{:03}-{:04}", i % 1000, i))
            .collect();
        vals.extend((0..100).map(|i| format!("bad{i}")));
        let refs: Vec<&str> = vals.iter().map(String::as_str).collect();
        let run = || pills(&refs);
        let one = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(run);
        let many = rayon::ThreadPoolBuilder::new()
            .num_threads(8)
            .build()
            .unwrap()
            .install(run);
        assert_eq!(one, many);
        assert_eq!(one.0.as_deref(), Some("999-9999"));
    }

    #[test]
    fn samples_the_start_middle_and_end() {
        assert_eq!(sample_rows(1, 5, 10), vec![1, 2, 3, 4]);
        let s = sample_rows(0, 1000, 9);
        assert_eq!(s, vec![0, 1, 2, 499, 500, 501, 997, 998, 999]);
    }

    #[test]
    fn strip_figures() {
        let mut csv = String::from("id,kind,amount,notes\r\n");
        for i in 0..100 {
            let kind = ["a", "b", "b", "c"][i % 4];
            let notes = if i % 2 == 0 { "x" } else { "" };
            csv.push_str(&format!("{i},{kind},{},{notes}\r\n", i % 20));
        }
        let d =
            Document::from_bytes(csv.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let r = d
            .column_insights(&InsightSpec {
                cols: vec![0, 1, 2, 3],
                keep_first: true,
                sample: 1000,
                ..Default::default()
            })
            .unwrap();
        assert_eq!((r.sampled, r.rows), (100, 100));
        let [id, kind, amount, notes] = &r.cols[..] else {
            panic!("four columns")
        };
        assert!(id.unique && id.numbers && id.filled == 1.0);
        assert_eq!(kind.distinct, 3);
        assert!(!kind.numbers);
        // b is the most common: its bar is the tallest, first.
        assert_eq!(kind.bars, vec![1.0, 0.5, 0.5]);
        assert!(amount.numbers && amount.bars.len() == BARS);
        assert_eq!(notes.filled, 0.5);
        assert!(!notes.unique);
        // A smaller sample still reads every column.
        let r = d
            .column_insights(&InsightSpec {
                cols: vec![1],
                keep_first: true,
                sample: 12,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(r.sampled, 12);
    }
}
