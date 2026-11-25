//! Clean-up suggestions: one scan over the shown rows that lists likely
//! fixes, each with a ready one-click fix (a cleaning step or a row tool),
//! so the UI never has to work out what to run.
//!
//! The scan is bounded: at most [`MAX_SCAN_ROWS`] rows and [`MAX_SCAN_COLS`]
//! columns are read (in parallel chunks, with progress and cancel), a few
//! short examples are kept per suggestion, and duplicate rows are found by
//! 64-bit hashes held in a set of at most [`MAX_ROW_HASHES`] entries. The
//! fixes themselves run over every row and compare real values.

use std::collections::hash_map::RandomState;
use std::collections::HashSet;
use std::hash::{BuildHasher, Hash, Hasher};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::clean::{Spaces, Step};
use crate::document::{Document, Progress};
use crate::ops::Cols;
use crate::value::ReadOptions;
use crate::Error;

/// Rows looked at by one scan.
pub const MAX_SCAN_ROWS: u64 = 2_000_000;
/// Columns looked at by one scan.
pub const MAX_SCAN_COLS: u32 = 1_000;
/// Row hashes kept while looking for duplicate rows.
pub const MAX_ROW_HASHES: usize = 4_000_000;
/// Examples kept per suggestion, and their length in characters.
const EXAMPLES: usize = 3;
const EXAMPLE_CHARS: usize = 60;
/// Distinct date shapes tracked per column.
const SHAPES: usize = 8;
/// Rows per parallel task.
const CHUNK: u64 = 16_384;
/// Share of filled cells that must be numbers (or dates) for a column to
/// count as a number (or date) column.
const MOSTLY: f64 = 0.8;

/// What a suggestion is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Spaces at the start or end of cells, or doubled inside.
    Spaces,
    /// Numbers written with currency signs, thousands separators, a plus
    /// sign or brackets.
    NumberText,
    /// Dates written in more than one style.
    MixedDates,
    DuplicateRows,
    BlankRows,
}

/// The one-click fix for a suggestion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "fix", rename_all = "snake_case")]
pub enum Fix {
    /// Run this cleaning step on these columns (the first row left out
    /// when it holds titles).
    Clean {
        cols: Vec<u32>,
        step: Step,
    },
    /// Remove rows that repeat an earlier row exactly.
    RemoveDuplicates,
    RemoveBlankRows,
}

/// One likely fix.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub kind: Kind,
    /// The column it is about (none for row suggestions).
    pub col: Option<u32>,
    /// Cells (or rows) affected among those looked at.
    pub count: u64,
    /// A few values as they are now, shortened.
    pub examples: Vec<String>,
    pub fix: Fix,
}

/// What a scan found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestions {
    pub items: Vec<Suggestion>,
    /// Rows looked at.
    pub rows: u64,
    /// Not every row or column was looked at (counts are "at least").
    pub partial: bool,
}

#[derive(Default, Clone)]
struct ColAcc {
    filled: u64,
    /// Cells with spaces at an end or doubled inside.
    spaced: u64,
    double: u64,
    numbers: u64,
    number_text: u64,
    dates: u64,
    times: bool,
    shapes: Vec<(String, u64)>,
    spaces_ex: Vec<String>,
    number_ex: Vec<String>,
    date_ex: Vec<String>,
}

fn example(list: &mut Vec<String>, v: &str) {
    if list.len() < EXAMPLES && !list.iter().any(|x| x == v) {
        let mut s: String = v.chars().take(EXAMPLE_CHARS).collect();
        if v.chars().nth(EXAMPLE_CHARS).is_some() {
            s.push('…');
        }
        list.push(s);
    }
}

/// A date's shape: digits as 9, letters as a (so "3/4/2026" and
/// "2026-04-03" differ, "3/4/2026" and "12/31/2026" do not by much: runs of
/// digits count as one).
fn shape(v: &str) -> String {
    let mut out = String::new();
    let mut last = ' ';
    for ch in v.chars().take(40) {
        let k = if ch.is_ascii_digit() {
            '9'
        } else if ch.is_alphabetic() {
            'a'
        } else {
            ch
        };
        if !(k == last && (k == '9' || k == 'a')) {
            out.push(k);
        }
        last = k;
    }
    out
}

/// A plain number: optional minus, digits, optional decimals.
fn plain_number(t: &str) -> bool {
    let t = t.strip_prefix('-').unwrap_or(t);
    let (a, b) = t.split_once('.').unwrap_or((t, "0"));
    !a.is_empty()
        && a.bytes().all(|c| c.is_ascii_digit())
        && !b.is_empty()
        && b.bytes().all(|c| c.is_ascii_digit())
}

impl ColAcc {
    fn add(&mut self, v: &str, rd: crate::coltype::Reader) {
        if v.trim().is_empty() {
            return;
        }
        self.filled += 1;
        let t = v.trim();
        let edge = t.len() != v.len();
        let double = t.contains("  ");
        if edge || double {
            self.spaced += 1;
            self.double += double as u64;
            example(&mut self.spaces_ex, v);
        }
        if rd.number(t).is_some() {
            self.numbers += 1;
            if !plain_number(t) && !t.ends_with('%') && !t.to_ascii_lowercase().contains('e') {
                self.number_text += 1;
                example(&mut self.number_ex, t);
            }
        } else if rd.date(t).is_some() {
            self.dates += 1;
            self.times |= t.contains(':');
            let s = shape(t);
            if let Some(x) = self.shapes.iter_mut().find(|x| x.0 == s) {
                x.1 += 1;
            } else if self.shapes.len() < SHAPES {
                self.shapes.push((s, 1));
                example(&mut self.date_ex, t);
            }
        }
    }

    fn merge(mut self, o: ColAcc) -> ColAcc {
        self.filled += o.filled;
        self.spaced += o.spaced;
        self.double += o.double;
        self.numbers += o.numbers;
        self.number_text += o.number_text;
        self.dates += o.dates;
        self.times |= o.times;
        for (s, n) in o.shapes {
            if let Some(x) = self.shapes.iter_mut().find(|x| x.0 == s) {
                x.1 += n;
            } else if self.shapes.len() < SHAPES {
                self.shapes.push((s, n));
            }
        }
        for v in o.spaces_ex {
            example(&mut self.spaces_ex, &v);
        }
        for v in o.number_ex {
            example(&mut self.number_ex, &v);
        }
        for v in o.date_ex {
            example(&mut self.date_ex, &v);
        }
        self
    }
}

struct Chunk {
    cols: Vec<ColAcc>,
    blank: u64,
    blank_ex: bool,
    hashes: Vec<u64>,
}

impl Document {
    /// Look for likely clean-ups over the shown rows (the first row left
    /// out with `skip_first`).
    pub fn suggestions(
        &self,
        skip_first: bool,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Suggestions, Error> {
        self.ready_for_bulk()?;
        let rows = self.data_rows(skip_first);
        let (r0, r1) = (rows.start, rows.end.min(rows.start + MAX_SCAN_ROWS));
        let width = self.col_count();
        let n = width.min(MAX_SCAN_COLS);
        let partial = r1 < rows.end || n < width;
        let all: Vec<u32> = (0..n).collect();
        let reader = Cols::new(self, &all);
        let ctx = self.read_ctx(read);
        let rds: Vec<crate::coltype::Reader> = all.iter().map(|&c| ctx.reader(c)).collect();
        let state = RandomState::new();
        progress.start(r1 - r0);
        let chunks: Vec<Chunk> = (0..(r1 - r0).div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<Chunk, Error> {
                let s = r0 + k * CHUNK;
                let e = (s + CHUNK).min(r1);
                let mut c = Chunk {
                    cols: vec![ColAcc::default(); n as usize],
                    blank: 0,
                    blank_ex: false,
                    hashes: Vec::with_capacity((e - s) as usize),
                };
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let mut h = state.build_hasher();
                    let mut empty = true;
                    for (i, acc) in c.cols.iter_mut().enumerate() {
                        let v = reader.get(&buf, i);
                        empty &= v.trim().is_empty();
                        v.hash(&mut h);
                        acc.add(v, rds[i]);
                    }
                    if empty {
                        c.blank += 1;
                        c.blank_ex = true;
                    } else {
                        c.hashes.push(h.finish());
                    }
                }
                progress.step(e - s)?;
                Ok(c)
            })
            .collect::<Result<_, _>>()?;
        // Columns: merge in order. Duplicates: one pass over the hashes in
        // row order, bounded.
        let mut cols = vec![ColAcc::default(); n as usize];
        let mut blank = 0;
        let mut seen: HashSet<u64> = HashSet::new();
        let mut dupes = 0u64;
        let mut dupes_partial = false;
        for c in chunks {
            for (a, b) in cols.iter_mut().zip(c.cols) {
                *a = std::mem::take(a).merge(b);
            }
            blank += c.blank;
            for h in c.hashes {
                if seen.len() >= MAX_ROW_HASHES {
                    dupes_partial = true;
                    break;
                }
                if !seen.insert(h) {
                    dupes += 1;
                }
            }
        }
        drop(seen);
        let mut items = Vec::new();
        // Spaces: one suggestion per column; trimming when spaces sit only
        // at the ends, else also doubled spaces made single.
        for (i, a) in cols.iter().enumerate() {
            let col = i as u32;
            if a.spaced > 0 {
                let how = if a.double > 0 {
                    Spaces::Collapse
                } else {
                    Spaces::Trim
                };
                items.push(Suggestion {
                    kind: Kind::Spaces,
                    col: Some(col),
                    count: a.spaced,
                    examples: a.spaces_ex.clone(),
                    fix: Fix::Clean {
                        cols: vec![col],
                        step: Step::Spaces { how },
                    },
                });
            }
            let mostly = |k: u64| a.filled > 0 && k as f64 >= a.filled as f64 * MOSTLY;
            if mostly(a.numbers) && a.number_text > 0 {
                items.push(Suggestion {
                    kind: Kind::NumberText,
                    col: Some(col),
                    count: a.number_text,
                    examples: a.number_ex.clone(),
                    fix: Fix::Clean {
                        cols: vec![col],
                        step: Step::Numbers {
                            decimals: None,
                            thousands: false,
                            decimal_comma: read.decimal_comma,
                        },
                    },
                });
            }
            if mostly(a.dates) && a.shapes.len() > 1 {
                let main = a.shapes.iter().map(|x| x.1).max().unwrap_or(0);
                items.push(Suggestion {
                    kind: Kind::MixedDates,
                    col: Some(col),
                    count: a.dates - main,
                    examples: a.date_ex.clone(),
                    fix: Fix::Clean {
                        cols: vec![col],
                        step: Step::Dates {
                            from: String::new(),
                            past: false,
                            format: if a.times {
                                "yyyy-mm-dd hh:mm"
                            } else {
                                "yyyy-mm-dd"
                            }
                            .into(),
                        },
                    },
                });
            }
        }
        if dupes > 0 {
            items.push(Suggestion {
                kind: Kind::DuplicateRows,
                col: None,
                count: dupes,
                examples: Vec::new(),
                fix: Fix::RemoveDuplicates,
            });
        }
        if blank > 0 {
            items.push(Suggestion {
                kind: Kind::BlankRows,
                col: None,
                count: blank,
                examples: Vec::new(),
                fix: Fix::RemoveBlankRows,
            });
        }
        Ok(Suggestions {
            items,
            rows: r1 - r0,
            partial: partial || dupes_partial,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(text: &str) -> Document {
        let mut d = Document::from_bytes(
            text.as_bytes().to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        while !d.index_step(1 << 20).unwrap() {}
        d
    }

    fn kinds(s: &Suggestions) -> Vec<(Kind, Option<u32>, u64)> {
        s.items.iter().map(|i| (i.kind, i.col, i.count)).collect()
    }

    #[test]
    fn finds_the_usual_problems() {
        let d = doc(
            "name,amount,when\n Ann ,\"$1,200\",2026-01-05\nBob,300,1/6/2026\n,,\nBob,300,1/6/2026\nCy  D,\"1,000\",2026-01-07\n",
        );
        let s = d
            .suggestions(true, ReadOptions::default(), &Progress::new())
            .unwrap();
        let k = kinds(&s);
        assert!(k.contains(&(Kind::Spaces, Some(0), 2)), "{k:?}");
        assert!(k.contains(&(Kind::NumberText, Some(1), 2)), "{k:?}");
        assert!(
            k.iter().any(|x| x.0 == Kind::MixedDates && x.1 == Some(2)),
            "{k:?}"
        );
        assert!(k.contains(&(Kind::DuplicateRows, None, 1)), "{k:?}");
        assert!(k.contains(&(Kind::BlankRows, None, 1)), "{k:?}");
        assert!(!s.partial);
        let spaces = s.items.iter().find(|i| i.kind == Kind::Spaces).unwrap();
        assert_eq!(
            spaces.fix,
            Fix::Clean {
                cols: vec![0],
                step: Step::Spaces {
                    how: Spaces::Collapse
                }
            }
        );
        assert_eq!(spaces.examples, [" Ann ", "Cy  D"]);
    }

    #[test]
    fn clean_files_have_none() {
        let d = doc("a,b\n1,2026-01-01\n2,2026-01-02\n3.5,2026-01-03\n");
        let s = d
            .suggestions(true, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert!(s.items.is_empty(), "{:?}", s.items);
    }

    #[test]
    fn mostly_text_columns_are_left_alone() {
        let d = doc("a\nhello\nworld\n$5\nmore\n");
        let s = d
            .suggestions(false, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert!(!s.items.iter().any(|i| i.kind == Kind::NumberText));
    }

    #[test]
    fn shapes_group_runs_of_digits() {
        assert_eq!(shape("1/6/2026"), shape("12/31/2026"));
        assert_ne!(shape("2026-01-05"), shape("1/6/2026"));
        assert!(
            plain_number("-12.5")
                && plain_number("7")
                && !plain_number("1,000")
                && !plain_number("$5")
        );
    }
}
