//! Column insights: a thin strip under each heading (View › Column
//! insights) with how full a column is, how many different values it holds
//! and a tiny bar chart. Worked out from a sample of the shown rows (the
//! start, middle and end of the file), so it never slows opening; the UI
//! asks for a small sample first and a bigger one after.

use std::collections::HashMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::ops::{check_cols, Cols};
use crate::value::{parse_number, ReadOptions};
use crate::Error;

/// Rows sampled at most.
pub const MAX_SAMPLE: u64 = 200_000;
/// Cells read at most for one strip (columns × sampled rows).
pub const MAX_CELLS: u64 = 4_000_000;
/// Columns asked about at once.
pub const MAX_COLS: usize = 512;
/// Bars in the strip's chart.
pub const BARS: usize = 8;

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
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Insights {
    pub cols: Vec<Insight>,
    /// Rows looked at, of `rows` shown data rows.
    pub sampled: u64,
    pub rows: u64,
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

fn one(col: u32, vals: &[String], read: ReadOptions) -> Insight {
    let n = vals.len();
    let mut counts: HashMap<&str, u32> = HashMap::new();
    let mut nums: Vec<f64> = Vec::new();
    let mut filled = 0usize;
    for v in vals {
        if v.trim().is_empty() {
            continue;
        }
        filled += 1;
        *counts.entry(v.as_str()).or_insert(0) += 1;
        if let Some(x) = parse_number(v, read) {
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

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
