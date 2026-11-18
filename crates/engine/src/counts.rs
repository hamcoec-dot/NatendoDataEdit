//! The counts tab (Shift+F): each value of a column, or each combination of
//! two or three columns, with how many shown rows hold it, the percent of
//! those rows and a bar, most common first. It opens as a small tab of its
//! own, linked back so a line can show its rows in the source.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::{check_cols, Cols, CHUNK};
use crate::reshape::{push_line, too_big, Table};
use crate::value::natural_cmp;
use crate::Error;

/// Columns counted together at most.
pub const MAX_COUNT_COLS: usize = 3;
/// Different values (or combinations) listed at most (the most common).
pub const MAX_COUNT_GROUPS: usize = 1_000_000;
/// Text held for the values at most.
const MAX_COUNT_BYTES: u64 = 512 << 20;
/// Between the cells of a combination (never in a CSV cell typed by hand).
const SEP: char = '\u{1f}';

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CountsSpec {
    /// One to three logical columns.
    pub cols: Vec<u32>,
    pub keep_first: bool,
}

/// One line: the values (one per column) and how many shown rows hold them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountLine {
    pub values: Vec<String>,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    /// Most common first (ties in natural order); blanks are a value too.
    pub lines: Vec<CountLine>,
    /// Shown data rows counted.
    pub rows: u64,
    /// False when there were more than [`MAX_COUNT_GROUPS`] different
    /// values (the listed counts are still exact).
    pub complete: bool,
}

/// Shards the counts are merged in.
const SHARDS: usize = 64;

/// A key's shard (FNV-1a).
fn shard_of(v: &str) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    for &b in v.as_bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    (h as usize) % SHARDS
}

impl Document {
    /// Count each value (or combination) over the shown data rows.
    pub fn value_counts(&self, spec: &CountsSpec, progress: &Progress) -> Result<Counts, Error> {
        self.ready_for_bulk()?;
        if spec.cols.is_empty() || spec.cols.len() > MAX_COUNT_COLS {
            return Err(Error::Invalid(format!(
                "count one to {MAX_COUNT_COLS} columns at once"
            )));
        }
        check_cols(self, &spec.cols)?;
        let reader = Cols::new(self, &spec.cols);
        let k = spec.cols.len();
        let rows = self.data_rows(spec.keep_first);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let chunks = ((r1 - r0) as usize).div_ceil(CHUNK);
        // The keys are split by hash into shards, each merged under its own
        // lock, so the parts of a column with millions of different values
        // merge in parallel.
        let merged: Vec<Mutex<HashMap<String, u64>>> =
            (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect();
        let over = AtomicBool::new(false);
        let bytes = AtomicU64::new(0);
        (0..chunks)
            .into_par_iter()
            .try_for_each(|c| -> Result<(), Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut local: Vec<HashMap<String, u64>> = vec![HashMap::new(); SHARDS];
                let mut buf = Vec::new();
                let mut key = String::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let v: &str = if k == 1 {
                        reader.get(&buf, 0)
                    } else {
                        key.clear();
                        for i in 0..k {
                            if i > 0 {
                                key.push(SEP);
                            }
                            key.push_str(reader.get(&buf, i));
                        }
                        &key
                    };
                    let m = &mut local[shard_of(v)];
                    match m.get_mut(v) {
                        Some(n) => *n += 1,
                        None => {
                            m.insert(v.to_string(), 1);
                        }
                    }
                }
                progress.step(e - s)?;
                // Start at a different shard in each part, so parts finishing
                // together seldom wait for the same lock.
                for i in 0..SHARDS {
                    let sh = (i + c) % SHARDS;
                    let part = std::mem::take(&mut local[sh]);
                    if part.is_empty() {
                        continue;
                    }
                    let mut m = merged[sh].lock().unwrap_or_else(|p| p.into_inner());
                    for (key, n) in part {
                        if let Some(x) = m.get_mut(&key) {
                            *x += n;
                        } else if bytes.fetch_add(key.len() as u64 + 32, Ordering::Relaxed)
                            < MAX_COUNT_BYTES
                        {
                            m.insert(key, n);
                        } else {
                            over.store(true, Ordering::Relaxed);
                        }
                    }
                }
                Ok(())
            })?;
        let mut lines: Vec<(String, u64)> = Vec::new();
        for m in merged {
            lines.extend(m.into_inner().unwrap_or_else(|p| p.into_inner()));
        }
        lines.par_sort_unstable_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| natural_cmp(a.0.as_bytes(), b.0.as_bytes()))
        });
        // Every value is counted (up to the text cap); the most common
        // ones are listed.
        if lines.len() > MAX_COUNT_GROUPS {
            lines.truncate(MAX_COUNT_GROUPS);
            over.store(true, Ordering::Relaxed);
        }
        Ok(Counts {
            lines: lines
                .into_iter()
                .map(|(key, count)| CountLine {
                    values: if k == 1 {
                        vec![key]
                    } else {
                        key.split(SEP).map(str::to_string).collect()
                    },
                    count,
                })
                .collect(),
            rows: r1 - r0,
            complete: !over.into_inner(),
        })
    }

    /// The counts as a table: the column titles, then Count and Percent.
    pub fn counts_table(
        &self,
        spec: &CountsSpec,
        progress: &Progress,
    ) -> Result<(Table, Counts), Error> {
        let counts = self.value_counts(spec, progress)?;
        let names = self.expr_names(spec.keep_first);
        let mut out = Vec::new();
        let mut head: Vec<String> = spec
            .cols
            .iter()
            .map(|&c| names.get(c as usize).cloned().unwrap_or_default())
            .collect();
        head.push("Count".into());
        head.push("Percent".into());
        push_line(&mut out, &head);
        let total = counts.rows.max(1) as f64;
        let mut line: Vec<String> = Vec::with_capacity(spec.cols.len() + 2);
        for l in &counts.lines {
            line.clear();
            line.extend(l.values.iter().cloned());
            line.push(l.count.to_string());
            line.push(format!("{:.1}%", l.count as f64 * 100.0 / total));
            push_line(&mut out, &line);
            if out.len() > crate::reshape::MAX_TABLE_BYTES {
                return Err(too_big());
            }
        }
        let rows = counts.lines.len() as u64 + 1;
        Ok((
            Table {
                bytes: out,
                rows,
                cols: spec.cols.len() as u32 + 2,
            },
            counts,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{ColumnFilter, FilterSpec, Rule};
    use crate::OpenOptions;

    fn doc() -> Document {
        Document::from_bytes(
            b"Party,Precinct\r\nD,12\r\nR,12\r\nD,3\r\nD,12\r\n,3\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap()
    }

    #[test]
    fn counts_one_and_two_columns() {
        let d = doc();
        let p = Progress::new();
        let one = d
            .value_counts(
                &CountsSpec {
                    cols: vec![0],
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        let got: Vec<(&str, u64)> = one
            .lines
            .iter()
            .map(|l| (l.values[0].as_str(), l.count))
            .collect();
        assert_eq!(got, vec![("D", 3), ("", 1), ("R", 1)]);
        assert_eq!(one.rows, 5);
        let two = d
            .value_counts(
                &CountsSpec {
                    cols: vec![0, 1],
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        assert_eq!(two.lines[0].values, ["D", "12"]);
        assert_eq!(two.lines[0].count, 2);
        assert_eq!(two.lines.len(), 4);
        let (t, _) = d
            .counts_table(
                &CountsSpec {
                    cols: vec![0],
                    keep_first: true,
                },
                &p,
            )
            .unwrap();
        let s = String::from_utf8(t.bytes).unwrap();
        assert!(s.starts_with("Party,Count,Percent\r\nD,3,60.0%\r\n"), "{s}");
    }

    #[test]
    fn counts_follow_the_filter() {
        let mut d = doc();
        d.set_filter(&FilterSpec {
            columns: vec![ColumnFilter {
                col: 1,
                rule: Rule::Values {
                    values: vec!["12".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        let c = d
            .value_counts(
                &CountsSpec {
                    cols: vec![0],
                    keep_first: true,
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(c.rows, 3);
        assert_eq!(c.lines[0].values[0], "D");
        assert_eq!(c.lines[0].count, 2);
        assert!(d
            .value_counts(
                &CountsSpec {
                    cols: vec![],
                    keep_first: true
                },
                &Progress::new()
            )
            .is_err());
    }
}
