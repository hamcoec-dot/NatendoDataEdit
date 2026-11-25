//! Column statistics: guessed column types, a column summary, chart data
//! and subtotals. All of it runs over shown rows in bounded memory, so it
//! stays quick on 10M-row files: distinct values are counted exactly up to
//! [`MAX_DISTINCT`], charts are bucketed to [`MAX_POINTS`], subtotal tables
//! are capped like other new tables.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::coltype::Reader;
use crate::document::{Document, Progress};
use crate::ops::{check_cols, Cols};
use crate::reshape::{push_line, too_big, MAX_TABLE_BYTES, MAX_TABLE_CELLS};
use crate::table::Sink;
use crate::value::ReadOptions;
use crate::Error;

/// Distinct values counted exactly; past this the count is "at least".
pub const MAX_DISTINCT: usize = 100_000;
/// Values listed in a summary.
pub const TOP_VALUES: usize = 20;
/// Most points in a line or bar chart (rows are grouped into buckets).
pub const MAX_POINTS: usize = 2_000;
/// Most slices in a pie or count chart (the rest are "Other").
pub const MAX_SLICES: usize = 50;
/// Most value columns in a chart.
pub const MAX_SERIES: usize = 8;
/// Rows sampled to guess column types.
pub const TYPE_SAMPLE: u64 = 2_000;
/// Longest value kept as a summary or chart label, in characters.
const LABEL_CHARS: usize = 200;
/// Rows per parallel task.
const CHUNK: u64 = 16_384;

/// What a column holds, guessed from its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColType {
    Empty,
    Number,
    Date,
    YesNo,
    Text,
    /// Numbers or dates mixed with text.
    Mixed,
}

/// How many values of each kind a column holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TypeMix {
    pub blank: u64,
    pub number: u64,
    pub date: u64,
    pub yes_no: u64,
    pub text: u64,
}

impl TypeMix {
    fn add(&mut self, v: &str, rd: Reader) {
        let t = v.trim();
        if t.is_empty() {
            self.blank += 1;
        } else if rd.number(t).is_some() {
            self.number += 1;
        } else if is_yes_no(t) {
            self.yes_no += 1;
        } else if rd.date(t).is_some() {
            self.date += 1;
        } else {
            self.text += 1;
        }
    }

    fn merge(&mut self, o: &TypeMix) {
        self.blank += o.blank;
        self.number += o.number;
        self.date += o.date;
        self.yes_no += o.yes_no;
        self.text += o.text;
    }

    /// The type the mix suggests (one kind for at least 95% of the filled
    /// values; numbers and dates together count as mixed).
    pub fn kind(&self) -> ColType {
        let filled = self.number + self.date + self.yes_no + self.text;
        if filled == 0 {
            return ColType::Empty;
        }
        let most = |n: u64| n * 100 >= filled * 95;
        if most(self.number) {
            ColType::Number
        } else if most(self.date) {
            ColType::Date
        } else if most(self.yes_no) {
            ColType::YesNo
        } else if most(self.text) {
            ColType::Text
        } else {
            ColType::Mixed
        }
    }
}

fn is_yes_no(t: &str) -> bool {
    matches!(
        t.to_ascii_lowercase().as_str(),
        "yes" | "no" | "true" | "false" | "y" | "n"
    )
}

/// One value and how often it appears.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValueCount {
    /// The value, shortened to 200 characters and "…" when longer.
    pub value: String,
    /// `value` was shortened (filter on it with "begins with").
    pub cut: bool,
    pub count: u64,
}

/// A column's summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub col: u32,
    /// Shown rows looked at.
    pub rows: u64,
    pub mix: TypeMix,
    pub kind: ColType,
    /// Distinct values (exact unless `distinct_capped`).
    pub distinct: u64,
    pub distinct_capped: bool,
    /// Over the values that are numbers.
    pub sum: Option<f64>,
    pub mean: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Shortest and longest text, in characters.
    pub min_len: u64,
    pub max_len: u64,
    /// The most frequent values, with exact counts.
    pub top: Vec<ValueCount>,
    /// Distinct values were capped: the list holds the most frequent values
    /// found (any value making up more than 1 in 100,000 rows is always
    /// found), so a value that appears only a few times may be missing.
    pub top_partial: bool,
}

fn label(s: &str) -> String {
    match s.char_indices().nth(LABEL_CHARS) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Compensated (Neumaier) sum: adding 10M values of 0.1 gives 1,000,000,
/// not 999,999.9999997595.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Kahan {
    sum: f64,
    c: f64,
}

impl Kahan {
    pub(crate) fn add(&mut self, x: f64) {
        let t = self.sum + x;
        if !t.is_finite() {
            self.sum = t;
            return;
        }
        if self.sum.abs() >= x.abs() {
            self.c += (self.sum - t) + x;
        } else {
            self.c += (x - t) + self.sum;
        }
        self.sum = t;
    }

    pub(crate) fn merge(&mut self, o: &Kahan) {
        self.add(o.sum);
        self.c += o.c;
    }

    pub(crate) fn value(&self) -> f64 {
        if self.sum.is_finite() {
            self.sum + self.c
        } else {
            self.sum
        }
    }
}

/// Values recounted exactly in a second pass once distinct values pass
/// [`MAX_DISTINCT`].
const CANDIDATES: usize = 1_000;

/// Keep a value map bounded without losing frequent values (a mergeable
/// Misra-Gries summary): once it holds more than `limit` values, the
/// weight of the ([`MAX_DISTINCT`] + 1)-th heaviest is taken off every
/// value and values left at nothing are dropped. A value that makes up more
/// than 1 in [`MAX_DISTINCT`] of the rows is never dropped, however the
/// chunks are merged; kept weights are then lower bounds, which a second
/// pass makes exact. Returns whether anything was dropped.
fn shrink<V>(
    m: &mut HashMap<String, V>,
    limit: usize,
    w: impl Fn(&V) -> f64,
    cut: impl Fn(&mut V, f64),
) -> bool {
    if m.len() <= limit {
        return false;
    }
    let mut ws: Vec<f64> = m.values().map(&w).collect();
    let (_, c, _) = ws.select_nth_unstable_by(MAX_DISTINCT, |a, b| b.total_cmp(a));
    let c = *c;
    m.retain(|_, v| {
        cut(v, c);
        w(v) > 0.0
    });
    true
}

/// Merge `b` into `a` (the larger map takes the smaller one's values).
fn merge_maps<V>(
    mut a: HashMap<String, V>,
    mut b: HashMap<String, V>,
    add: impl Fn(&mut V, V),
) -> HashMap<String, V> {
    if a.len() < b.len() {
        std::mem::swap(&mut a, &mut b);
    }
    for (k, v) in b {
        match a.get_mut(&k) {
            Some(x) => add(x, v),
            None => {
                a.insert(k, v);
            }
        }
    }
    a
}

/// The [`CANDIDATES`] heaviest values.
fn leaders(list: impl Iterator<Item = (String, f64)>) -> Vec<String> {
    let mut list: Vec<(String, f64)> = list.collect();
    if list.len() > CANDIDATES {
        list.select_nth_unstable_by(CANDIDATES - 1, |a, b| b.1.total_cmp(&a.1));
        list.truncate(CANDIDATES);
    }
    list.into_iter().map(|x| x.0).collect()
}

// A chunk alone never needs shrinking.
const _: () = assert!((CHUNK as usize) <= MAX_DISTINCT);

#[derive(Default)]
struct Acc {
    mix: TypeMix,
    sum: Kahan,
    n: u64,
    min: f64,
    max: f64,
    min_len: u64,
    max_len: u64,
    counts: HashMap<String, u64>,
    capped: bool,
}

impl Acc {
    fn new() -> Acc {
        Acc {
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            min_len: u64::MAX,
            ..Default::default()
        }
    }

    fn add(&mut self, v: &str, rd: Reader) {
        self.mix.add(v, rd);
        if let Some(x) = rd.number(v.trim()) {
            self.sum.add(x);
            self.n += 1;
            self.min = self.min.min(x);
            self.max = self.max.max(x);
        }
        let len = v.chars().count() as u64;
        self.min_len = self.min_len.min(len);
        self.max_len = self.max_len.max(len);
        if let Some(c) = self.counts.get_mut(v) {
            *c += 1;
        } else {
            self.counts.insert(v.to_string(), 1);
        }
    }

    fn merge(mut self, o: Acc) -> Acc {
        self.mix.merge(&o.mix);
        self.sum.merge(&o.sum);
        self.n += o.n;
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        self.min_len = self.min_len.min(o.min_len);
        self.max_len = self.max_len.max(o.max_len);
        self.capped |= o.capped;
        self.counts = merge_maps(std::mem::take(&mut self.counts), o.counts, |a, b| *a += b);
        self.capped |= Self::shrink(&mut self.counts, 2 * MAX_DISTINCT);
        self
    }

    fn shrink(m: &mut HashMap<String, u64>, limit: usize) -> bool {
        shrink(
            m,
            limit,
            |&v| v as f64,
            |v, c| *v = v.saturating_sub(c as u64),
        )
    }
}

/// What a chart shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Line,
    Bar,
    Pie,
    /// How often each value of the label column appears.
    Count,
}

/// A chart request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartSpec {
    pub kind: ChartKind,
    /// The label column (`None`: row numbers).
    #[serde(default)]
    pub label: Option<u32>,
    /// Value columns (not for `Count`), at most [`MAX_SERIES`].
    #[serde(default)]
    pub values: Vec<u32>,
    #[serde(default)]
    pub skip_first: bool,
    #[serde(default)]
    pub read: ReadOptions,
}

/// Chart data ready to draw.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartData {
    pub labels: Vec<String>,
    /// One list per value column (one list of counts for `Count`), the same
    /// length as `labels`; `None` where no value was a number.
    pub series: Vec<Vec<Option<f64>>>,
    /// Rows behind each point (more than 1 when rows were bucketed).
    pub per_point: u64,
    /// Rows looked at.
    pub rows: u64,
    /// Slices or values left out and summed into "Other".
    pub other: bool,
}

/// What subtotals compute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Total {
    Sum,
    Count,
    Average,
    Min,
    Max,
}

/// Subtotals: a new table with a total row after each run of equal values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtotalSpec {
    /// Rows are grouped where this column's value changes (sort first).
    pub by: u32,
    /// The columns totalled.
    pub cols: Vec<u32>,
    pub total: Total,
    /// The first row holds titles (copied to the new table).
    #[serde(default)]
    pub keep_first: bool,
    #[serde(default)]
    pub read: ReadOptions,
}

impl Document {
    /// Guess each column's type from a sample of shown rows (spread over
    /// the whole table).
    pub fn column_types(&self, skip_first: bool, read: ReadOptions) -> Result<Vec<ColType>, Error> {
        let n = self.col_count().min(crate::import::MAX_IMPORT_COLS as u32);
        let rows = self.data_rows(skip_first);
        let total = rows.end - rows.start;
        let take = total.min(TYPE_SAMPLE);
        let cols: Vec<u32> = (0..n).collect();
        let reader = Cols::new(self, &cols);
        let mut mixes = vec![TypeMix::default(); n as usize];
        let ctx = self.read_ctx(read);
        let rds: Vec<Reader> = (0..n).map(|c| ctx.reader(c)).collect();
        let mut buf = Vec::new();
        for i in 0..take {
            let r = rows.start + i * total / take.max(1);
            let Some(id) = self.id_at(r) else { continue };
            reader.read(self, id, &mut buf);
            for (c, m) in mixes.iter_mut().enumerate() {
                m.add(reader.get(&buf, c), rds[c]);
            }
        }
        Ok(mixes.iter().map(TypeMix::kind).collect())
    }

    /// Summarize one column over shown rows.
    pub fn summary(
        &self,
        col: u32,
        skip_first: bool,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Summary, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[col])?;
        let reader = Cols::new(self, &[col]);
        let rd = self.read_ctx(read).reader(col);
        let rows = self.data_rows(skip_first);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let chunks = (r1 - r0).div_ceil(CHUNK);
        let acc = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Acc, Error> {
                let s = r0 + c * CHUNK;
                let e = (s + CHUNK).min(r1);
                let mut a = Acc::new();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    a.add(reader.get(&buf, 0), rd);
                }
                progress.step(e - s)?;
                Ok(a)
            })
            .try_reduce(Acc::new, |a, b| Ok(a.merge(b)))?;
        let mut acc = acc;
        acc.capped |= Acc::shrink(&mut acc.counts, MAX_DISTINCT);
        let mut top: Vec<(String, u64)> = if acc.capped {
            // Too many distinct values to count them all: the summary kept
            // the frequent ones; count the leading ones again, exactly.
            let keys = leaders(
                std::mem::take(&mut acc.counts)
                    .into_iter()
                    .map(|(k, c)| (k, c as f64)),
            );
            self.recount(&reader, r0, r1, keys, 0, None, rd, progress)?
                .into_iter()
                .map(|(k, c)| (k, c as u64))
                .filter(|x| x.1 > 0)
                .collect()
        } else {
            std::mem::take(&mut acc.counts).into_iter().collect()
        };
        let distinct = if acc.capped {
            MAX_DISTINCT as u64
        } else {
            top.len() as u64
        };
        top.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top.truncate(TOP_VALUES);
        let top = top
            .into_iter()
            .map(|(v, c)| ValueCount {
                cut: v.chars().nth(LABEL_CHARS).is_some(),
                value: label(&v),
                count: c,
            })
            .collect();
        let has = acc.n > 0;
        let sum = acc.sum.value();
        Ok(Summary {
            col,
            rows: r1 - r0,
            mix: acc.mix,
            kind: acc.mix.kind(),
            distinct,
            distinct_capped: acc.capped,
            top_partial: acc.capped,
            sum: has.then_some(sum),
            mean: has.then(|| sum / acc.n as f64),
            min: has.then_some(acc.min),
            max: has.then_some(acc.max),
            min_len: if acc.min_len == u64::MAX {
                0
            } else {
                acc.min_len
            },
            max_len: acc.max_len,
            top,
        })
    }

    /// The data for a chart, bucketed to at most [`MAX_POINTS`] points (or
    /// [`MAX_SLICES`] slices).
    pub fn chart_data(&self, spec: &ChartSpec, progress: &Progress) -> Result<ChartData, Error> {
        self.ready_for_bulk()?;
        if spec.values.len() > MAX_SERIES {
            return Err(Error::Invalid(format!(
                "a chart can show at most {MAX_SERIES} value columns"
            )));
        }
        let mut cols: Vec<u32> = spec.label.into_iter().collect();
        cols.extend(&spec.values);
        check_cols(self, &cols)?;
        if spec.kind != ChartKind::Count && spec.values.is_empty() {
            return Err(Error::Invalid(
                "choose at least one column of values".into(),
            ));
        }
        if spec.kind == ChartKind::Count && spec.label.is_none() {
            return Err(Error::Invalid(
                "choose the column whose values to count".into(),
            ));
        }
        let reader = Cols::new(self, &cols);
        let lab = usize::from(spec.label.is_some());
        let rows = self.data_rows(spec.skip_first);
        let (r0, r1) = (rows.start, rows.end);
        let total = r1 - r0;
        progress.start(total);
        let ctx = self.read_ctx(spec.read);
        let vrds: Vec<Reader> = spec.values.iter().map(|&c| ctx.reader(c)).collect();
        let num = |k: usize, s: &str| vrds.get(k).and_then(|r| r.number(s.trim()));
        match spec.kind {
            ChartKind::Line | ChartKind::Bar => {
                let per = total.div_ceil(MAX_POINTS as u64).max(1);
                let points = total.div_ceil(per);
                let ns = spec.values.len();
                type Point = (String, Vec<Option<f64>>);
                let out: Vec<Point> = (0..points)
                    .into_par_iter()
                    .map(|p| -> Result<Point, Error> {
                        let s = r0 + p * per;
                        let e = (s + per).min(r1);
                        let mut buf = Vec::new();
                        let mut sums = vec![0.0f64; ns];
                        let mut cnt = vec![0u64; ns];
                        let mut first = String::new();
                        for r in s..e {
                            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                            reader.read(self, id, &mut buf);
                            if r == s {
                                first = if lab == 1 {
                                    label(reader.get(&buf, 0))
                                } else {
                                    (self.row_number(r).unwrap_or(r) + 1).to_string()
                                };
                            }
                            for k in 0..ns {
                                if let Some(x) = num(k, reader.get(&buf, lab + k)) {
                                    sums[k] += x;
                                    cnt[k] += 1;
                                }
                            }
                        }
                        progress.step(e - s)?;
                        let vals = (0..ns)
                            .map(|k| (cnt[k] > 0).then(|| sums[k] / cnt[k] as f64))
                            .collect();
                        Ok((first, vals))
                    })
                    .collect::<Result<_, _>>()?;
                let mut series = vec![Vec::with_capacity(out.len()); ns];
                let mut labels = Vec::with_capacity(out.len());
                for (l, v) in out {
                    labels.push(l);
                    for (k, x) in v.into_iter().enumerate() {
                        series[k].push(x);
                    }
                }
                Ok(ChartData {
                    labels,
                    series,
                    per_point: per,
                    rows: total,
                    other: false,
                })
            }
            ChartKind::Pie | ChartKind::Count => {
                // Sum (pie: of the first value column) or count per label.
                // Each value keeps (weight for keeping it, its sum); the
                // weight is the count, or the size of the pie values.
                let chunks = total.div_ceil(CHUNK);
                type Groups = (HashMap<String, (f64, f64)>, Kahan, bool);
                let pie = spec.kind == ChartKind::Pie;
                let shrink_to = |m: &mut HashMap<String, (f64, f64)>, limit: usize| {
                    shrink(m, limit, |v| v.0, |v, c| v.0 -= c)
                };
                let merge = |a: Groups, b: Groups| -> Groups {
                    let mut m = merge_maps(a.0, b.0, |x, y| {
                        x.0 += y.0;
                        x.1 += y.1;
                    });
                    let cut = shrink_to(&mut m, 2 * MAX_DISTINCT);
                    let mut t = a.1;
                    t.merge(&b.1);
                    (m, t, a.2 | b.2 | cut)
                };
                let (mut groups, sum, mut capped) = (0..chunks)
                    .into_par_iter()
                    .map(|c| -> Result<Groups, Error> {
                        let s = r0 + c * CHUNK;
                        let e = (s + CHUNK).min(r1);
                        let mut g: Groups = (HashMap::new(), Kahan::default(), false);
                        let mut buf = Vec::new();
                        for r in s..e {
                            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                            reader.read(self, id, &mut buf);
                            let key = reader.get(&buf, 0);
                            let v = if pie {
                                match num(0, reader.get(&buf, 1)) {
                                    Some(x) => x,
                                    None => continue,
                                }
                            } else {
                                1.0
                            };
                            g.1.add(v);
                            if let Some(x) = g.0.get_mut(key) {
                                x.0 += v.abs();
                                x.1 += v;
                            } else {
                                g.0.insert(key.to_string(), (v.abs(), v));
                            }
                        }
                        progress.step(e - s)?;
                        Ok(g)
                    })
                    .try_reduce(
                        || (HashMap::new(), Kahan::default(), false),
                        |a, b| Ok(merge(a, b)),
                    )?;
                capped |= shrink_to(&mut groups, MAX_DISTINCT);
                let mut list: Vec<(String, f64)> = if capped {
                    // Count the leading values again, exactly.
                    let keys = leaders(groups.into_iter().map(|(k, v)| (k, v.0)));
                    let wc = pie.then_some(1);
                    self.recount(
                        &reader,
                        r0,
                        r1,
                        keys,
                        0,
                        wc,
                        self.read_ctx(spec.read)
                            .reader(spec.values.first().copied().unwrap_or(0)),
                        progress,
                    )?
                } else {
                    groups.into_iter().map(|(k, v)| (k, v.1)).collect()
                };
                list.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                let left = capped || list.len() > MAX_SLICES;
                list.truncate(MAX_SLICES);
                let mut other = sum;
                for x in &list {
                    other.add(-x.1);
                }
                let mut labels: Vec<String> = list.iter().map(|x| label(&x.0)).collect();
                let mut vals: Vec<Option<f64>> = list.iter().map(|x| Some(x.1)).collect();
                if left {
                    labels.push("Other".into());
                    vals.push(Some(other.value()));
                }
                Ok(ChartData {
                    labels,
                    series: vec![vals],
                    per_point: 1,
                    rows: total,
                    other: left,
                })
            }
        }
    }

    /// Exact totals of `keys` over rows `r0..r1` (a second pass): each row
    /// whose column `kc` holds one of them adds 1, or the number in column
    /// `wc`.
    #[allow(clippy::too_many_arguments)]
    fn recount(
        &self,
        reader: &Cols,
        r0: u64,
        r1: u64,
        keys: Vec<String>,
        kc: usize,
        wc: Option<usize>,
        wrd: Reader,
        progress: &Progress,
    ) -> Result<Vec<(String, f64)>, Error> {
        let index: HashMap<&str, usize> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_str(), i))
            .collect();
        progress.start(r1 - r0);
        let n = keys.len();
        let chunks = (r1 - r0).div_ceil(CHUNK);
        let sums = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Vec<Kahan>, Error> {
                let s = r0 + c * CHUNK;
                let e = (s + CHUNK).min(r1);
                let mut out = vec![Kahan::default(); n];
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let Some(&i) = index.get(reader.get(&buf, kc)) else {
                        continue;
                    };
                    let w = match wc {
                        None => 1.0,
                        Some(w) => match wrd.number(reader.get(&buf, w).trim()) {
                            Some(x) => x,
                            None => continue,
                        },
                    };
                    out[i].add(w);
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .try_reduce(
                || vec![Kahan::default(); n],
                |mut a, b| {
                    for (x, y) in a.iter_mut().zip(&b) {
                        x.merge(y);
                    }
                    Ok(a)
                },
            )?;
        drop(index);
        Ok(keys
            .into_iter()
            .zip(sums)
            .map(|(k, s)| (k, s.value()))
            .collect())
    }

    /// Subtotals as a new table: the shown rows, with a total row after
    /// each run of equal values in `spec.by` and a grand total at the end.
    ///
    /// Memory: rows are written out in parallel chunks, and a group that
    /// ends inside a chunk gets its total row written there and then, so
    /// only the first and last group of each chunk keep running totals.
    /// Every byte written, every cell and each chunk's own totals are
    /// charged against the new-table caps while the chunks run.
    pub fn subtotals(
        &self,
        spec: &SubtotalSpec,
        progress: &Progress,
    ) -> Result<crate::reshape::Table, Error> {
        self.subtotals_within(spec, progress, MAX_TABLE_BYTES)
    }

    /// [`Document::subtotals`] holding at most `cap` bytes while building.
    fn subtotals_within(
        &self,
        spec: &SubtotalSpec,
        progress: &Progress,
        cap: usize,
    ) -> Result<crate::reshape::Table, Error> {
        self.ready_for_bulk()?;
        if spec.cols.is_empty() {
            return Err(Error::Invalid("choose the columns to total".into()));
        }
        let width = self.col_count().min(crate::import::MAX_IMPORT_COLS as u32);
        if spec.by >= width || spec.cols.iter().any(|&c| c >= width) {
            return Err(Error::OutOfRange);
        }
        let mut seen = vec![false; width as usize];
        for &c in &spec.cols {
            if std::mem::replace(&mut seen[c as usize], true) {
                return Err(Error::Invalid("each column can be totalled once".into()));
            }
        }
        let all: Vec<u32> = (0..width).collect();
        let reader = Cols::new(self, &all);
        let word = match spec.total {
            Total::Sum => "Total",
            Total::Count => "Count",
            Total::Average => "Average",
            Total::Min => "Min",
            Total::Max => "Max",
        };
        #[derive(Clone)]
        struct T {
            sum: Kahan,
            n: u64,
            filled: u64,
            min: f64,
            max: f64,
        }
        impl T {
            fn take(&mut self, v: &str, rd: Reader) {
                let v = v.trim();
                if !v.is_empty() {
                    self.filled += 1;
                }
                if let Some(x) = rd.number(v) {
                    self.sum.add(x);
                    self.n += 1;
                    self.min = self.min.min(x);
                    self.max = self.max.max(x);
                }
            }
            fn add(&mut self, o: &T) {
                self.sum.merge(&o.sum);
                self.n += o.n;
                self.filled += o.filled;
                self.min = self.min.min(o.min);
                self.max = self.max.max(o.max);
            }
        }
        /// Lines written: how many, their cells, the widest.
        #[derive(Default, Clone, Copy)]
        struct Seg {
            rows: u64,
            cells: u64,
            cols: u32,
        }
        impl Seg {
            fn line(&mut self, n: usize) {
                self.rows += 1;
                self.cells += n.max(1) as u64;
                self.cols = self.cols.max(n as u32);
            }
            fn add(&mut self, o: Seg) {
                self.rows += o.rows;
                self.cells += o.cells;
                self.cols = self.cols.max(o.cols);
            }
        }
        /// A group still open at a chunk's edge (it may go on in the next
        /// chunk or have started in the one before).
        struct Open {
            key: String,
            seg: Seg,
            totals: Vec<T>,
        }
        /// One chunk: `bytes[..first_end]` are the first group's rows,
        /// `bytes[first_end..mid_end]` whole groups with their total rows,
        /// `bytes[mid_end..]` the last group's rows (when it is not the
        /// first).
        struct Part {
            bytes: Vec<u8>,
            first: Option<Open>,
            first_end: usize,
            mid: Seg,
            mid_end: usize,
            last: Option<Open>,
            grand: Vec<T>,
        }
        let fresh = T {
            sum: Kahan::default(),
            n: 0,
            filled: 0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        };
        let fresh_all = || vec![fresh.clone(); spec.cols.len()];
        let shown = |t: &T| -> String {
            match spec.total {
                Total::Count => t.filled.to_string(),
                _ if t.n == 0 => String::new(),
                Total::Sum => crate::fill::format_number(t.sum.value(), spec.read.decimal_comma),
                Total::Average => {
                    crate::fill::format_number(t.sum.value() / t.n as f64, spec.read.decimal_comma)
                }
                Total::Min => crate::fill::format_number(t.min, spec.read.decimal_comma),
                Total::Max => crate::fill::format_number(t.max, spec.read.decimal_comma),
            }
        };
        let total_row = |name: String, ts: &[T]| -> Vec<String> {
            let mut line = vec![String::new(); width as usize];
            line[spec.by as usize] = name;
            for (k, &c) in spec.cols.iter().enumerate() {
                line[c as usize] = shown(&ts[k]);
            }
            let n = line
                .iter()
                .rposition(|c| !c.is_empty())
                .map_or(0, |i| i + 1);
            line.truncate(n);
            line
        };
        let mut sink = Sink::new(None);
        let mut buf = Vec::new();
        let rows = self.data_rows(spec.keep_first);
        progress.start(rows.end - rows.start);
        if spec.keep_first && self.row_count() > 0 {
            let id = self.id_at(0).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let line: Vec<&str> = (0..width as usize).map(|c| reader.get(&buf, c)).collect();
            sink.row(&line)?;
        }
        // Charged while the chunks run: bytes held (text, keys, totals) and
        // cells written.
        let used = AtomicUsize::new(0);
        let cells = AtomicU64::new(0);
        let charge = |bytes: usize, n: u64| -> Result<(), Error> {
            if used.fetch_add(bytes, Ordering::Relaxed) + bytes > cap
                || cells.fetch_add(n, Ordering::Relaxed) + n > MAX_TABLE_CELLS
            {
                return Err(too_big());
            }
            Ok(())
        };
        let col_readers: Vec<Reader> = spec
            .cols
            .iter()
            .map(|&c| self.read_ctx(spec.read).reader(c))
            .collect();
        let totals_bytes = spec.cols.len() * std::mem::size_of::<T>();
        let chunks = (rows.end - rows.start).div_ceil(CHUNK);
        let parts: Vec<Part> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Part, Error> {
                let s = rows.start + c * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                // This chunk's own totals: first, current and grand.
                charge(std::mem::size_of::<Part>() + 3 * totals_bytes, 0)?;
                let mut part = Part {
                    bytes: Vec::new(),
                    first: None,
                    first_end: 0,
                    mid: Seg::default(),
                    mid_end: 0,
                    last: None,
                    grand: fresh_all(),
                };
                let mut cur: Option<Open> = None;
                let mut cur_start = 0usize;
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let key = reader.get(&buf, spec.by as usize);
                    if cur.as_ref().is_none_or(|o| o.key != key) {
                        if let Some(o) = cur.take() {
                            for (g, t) in part.grand.iter_mut().zip(&o.totals) {
                                g.add(t);
                            }
                            if part.first.is_none() {
                                part.first_end = part.bytes.len();
                                charge(o.key.len(), 0)?;
                                part.first = Some(o);
                            } else {
                                // A whole group inside this chunk: its
                                // total row goes right after it.
                                part.mid.add(o.seg);
                                let line = total_row(format!("{} {word}", o.key), &o.totals);
                                let before = part.bytes.len();
                                push_line(&mut part.bytes, &line);
                                charge(part.bytes.len() - before, line.len().max(1) as u64)?;
                                part.mid.line(line.len());
                            }
                        }
                        cur_start = part.bytes.len();
                        cur = Some(Open {
                            key: key.to_string(),
                            seg: Seg::default(),
                            totals: fresh_all(),
                        });
                    }
                    let open = cur.as_mut().expect("set above");
                    let line: Vec<&str> =
                        (0..width as usize).map(|c| reader.get(&buf, c)).collect();
                    let n = line
                        .iter()
                        .rposition(|c| !c.is_empty())
                        .map_or(0, |i| i + 1);
                    let before = part.bytes.len();
                    push_line(&mut part.bytes, &line[..n]);
                    charge(part.bytes.len() - before, n.max(1) as u64)?;
                    open.seg.line(n);
                    for (k, &c) in spec.cols.iter().enumerate() {
                        open.totals[k].take(reader.get(&buf, c as usize), col_readers[k]);
                    }
                }
                if let Some(o) = cur {
                    for (g, t) in part.grand.iter_mut().zip(&o.totals) {
                        g.add(t);
                    }
                    charge(o.key.len(), 0)?;
                    if part.first.is_none() {
                        part.first_end = part.bytes.len();
                        part.mid_end = part.bytes.len();
                        part.first = Some(o);
                    } else {
                        part.mid_end = cur_start;
                        part.last = Some(o);
                    }
                }
                progress.step(e - s)?;
                Ok(part)
            })
            .collect::<Result<_, _>>()?;
        sink.reserve(used.load(Ordering::Relaxed));
        let mut open: Option<(String, Vec<T>)> = None;
        let mut grand = fresh_all();
        for part in parts {
            let Some(first) = part.first else { continue };
            for (g, t) in grand.iter_mut().zip(&part.grand) {
                g.add(t);
            }
            match &mut open {
                Some((g, sub)) if *g == first.key => {
                    for (s, t) in sub.iter_mut().zip(&first.totals) {
                        s.add(t);
                    }
                }
                _ => {
                    if let Some((g, sub)) = open.take() {
                        sink.row(&total_row(format!("{g} {word}"), &sub))?;
                    }
                    open = Some((first.key, first.totals));
                }
            }
            let f = first.seg;
            sink.lines(&part.bytes[..part.first_end], f.rows, f.cells, f.cols)?;
            if let Some(last) = part.last {
                if let Some((g, sub)) = open.take() {
                    sink.row(&total_row(format!("{g} {word}"), &sub))?;
                }
                let m = part.mid;
                sink.lines(
                    &part.bytes[part.first_end..part.mid_end],
                    m.rows,
                    m.cells,
                    m.cols,
                )?;
                let l = last.seg;
                sink.lines(&part.bytes[part.mid_end..], l.rows, l.cells, l.cols)?;
                open = Some((last.key, last.totals));
            }
        }
        if let Some((g, sub)) = open {
            sink.row(&total_row(format!("{g} {word}"), &sub))?;
            sink.row(&total_row(format!("Grand {}", word.to_lowercase()), &grand))?;
        }
        Ok(sink.finish())
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
    fn types_and_summary() {
        let d = doc(
            "n,d,t,yn,e\r\n1,2026-01-02,a,yes,\r\n2.5,2026-02-03,b,no,\r\n-3,2026-03-04,a,yes,\r\n",
        );
        let t = d.column_types(true, ReadOptions::default()).unwrap();
        assert_eq!(
            t,
            [
                ColType::Number,
                ColType::Date,
                ColType::Text,
                ColType::YesNo,
                ColType::Empty
            ]
        );
        let s = d
            .summary(0, true, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert_eq!(
            (s.rows, s.distinct, s.sum, s.min, s.max),
            (3, 3, Some(0.5), Some(-3.0), Some(2.5))
        );
        let s = d
            .summary(2, true, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert_eq!(
            s.top[0],
            ValueCount {
                value: "a".into(),
                cut: false,
                count: 2
            }
        );
        assert_eq!(s.sum, None);
    }

    #[test]
    fn many_distinct_values_are_capped() {
        let mut text = String::from("v\r\n");
        for i in 0..(MAX_DISTINCT + 500) {
            text.push_str(&format!("{i}\r\n"));
        }
        let d = doc(&text);
        let s = d
            .summary(0, true, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert!(s.distinct_capped);
        assert_eq!(s.distinct, MAX_DISTINCT as u64);
        assert_eq!(s.rows, (MAX_DISTINCT + 500) as u64);
    }

    #[test]
    fn charts_are_bucketed() {
        let mut text = String::from("x,y\r\n");
        for i in 0..10_000 {
            text.push_str(&format!("r{i},{}\r\n", i % 10));
        }
        let d = doc(&text);
        let c = d
            .chart_data(
                &ChartSpec {
                    kind: ChartKind::Line,
                    label: Some(0),
                    values: vec![1],
                    skip_first: true,
                    read: ReadOptions::default(),
                },
                &Progress::new(),
            )
            .unwrap();
        assert!(c.labels.len() <= MAX_POINTS);
        assert_eq!(c.per_point, 5);
        assert_eq!(c.series[0][0], Some(2.0));
        let c = d
            .chart_data(
                &ChartSpec {
                    kind: ChartKind::Count,
                    label: Some(1),
                    values: vec![],
                    skip_first: true,
                    read: ReadOptions::default(),
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(c.labels.len(), 10);
        assert_eq!(c.series[0][0], Some(1000.0));
        let c = d
            .chart_data(
                &ChartSpec {
                    kind: ChartKind::Pie,
                    label: Some(0),
                    values: vec![1],
                    skip_first: true,
                    read: ReadOptions::default(),
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(c.labels.len(), MAX_SLICES + 1);
        assert!(c.other);
        let bad = ChartSpec {
            kind: ChartKind::Bar,
            label: None,
            values: (0..9).collect(),
            skip_first: true,
            read: ReadOptions::default(),
        };
        assert!(d.chart_data(&bad, &Progress::new()).is_err());
    }

    #[test]
    fn subtotals_join_groups_across_chunks() {
        // Groups of 20k, 1 and 30k rows (chunks are 16,384), with a blank
        // row inside the first group.
        let mut text = String::from(
            "g,n
",
        );
        for i in 0..20_000 {
            text.push_str(if i == 7 {
                ",
"
            } else {
                "a,1
"
            });
        }
        text.push_str(
            "b,2
",
        );
        for _ in 0..30_000 {
            text.push_str(
                "c,3
",
            );
        }
        let d = doc(&text);
        let t = d
            .subtotals(
                &SubtotalSpec {
                    by: 0,
                    cols: vec![1],
                    total: Total::Sum,
                    keep_first: true,
                    read: ReadOptions::default(),
                },
                &Progress::new(),
            )
            .unwrap();
        let d = t.into_document().unwrap();
        // The blank row is a group of its own, so it splits group a; its
        // total row has no value (nothing to sum).
        let totals: Vec<Vec<String>> = (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .filter(|v| v[0].ends_with("Total") || v[0].ends_with("total"))
            .collect();
        assert_eq!(
            totals,
            [
                vec!["a Total", "7"],
                vec![" Total"],
                vec!["a Total", "19992"],
                vec!["b Total", "2"],
                vec!["c Total", "90000"],
                vec!["Grand total", "110001"],
            ]
        );
        assert_eq!(d.row_count(), 1 + 50_001 + 6);
    }

    #[test]
    fn subtotal_rows() {
        let d = doc("city,n\r\nA,1\r\nA,2\r\nB,5\r\n");
        let t = d
            .subtotals(
                &SubtotalSpec {
                    by: 0,
                    cols: vec![1],
                    total: Total::Sum,
                    keep_first: true,
                    read: ReadOptions::default(),
                },
                &Progress::new(),
            )
            .unwrap();
        let d = t.into_document().unwrap();
        let rows: Vec<Vec<String>> = (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect();
        assert_eq!(
            rows,
            [
                vec!["city", "n"],
                vec!["A", "1"],
                vec!["A", "2"],
                vec!["A Total", "3"],
                vec!["B", "5"],
                vec!["B Total", "5"],
                vec!["Grand total", "8"],
            ]
        );
    }

    fn sub_spec(cols: Vec<u32>, total: Total) -> SubtotalSpec {
        SubtotalSpec {
            by: 0,
            cols,
            total,
            keep_first: true,
            read: ReadOptions::default(),
        }
    }

    #[test]
    fn subtotal_columns_are_checked() {
        let d = doc("g,a,b\r\nx,1,2\r\n");
        let p = Progress::new();
        // Repeats would multiply the work; columns past the table are out.
        assert!(matches!(
            d.subtotals(&sub_spec(vec![1, 2, 1], Total::Sum), &p),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            d.subtotals(&sub_spec(vec![1, 3], Total::Sum), &p),
            Err(Error::OutOfRange)
        ));
        assert!(d.subtotals(&sub_spec(vec![1, 2], Total::Sum), &p).is_ok());
    }

    #[test]
    fn subtotal_memory_does_not_grow_per_group() {
        // 3,000 one-row groups, 99 columns totalled: before, each group kept
        // 99 running totals (40 bytes each, about 12 MB here) outside the
        // size cap. Now a group ending inside a chunk is written out at
        // once, so a cap near the output size is enough.
        let mut text = String::from("g");
        for c in 1..100 {
            text.push_str(&format!(",c{c}"));
        }
        text.push_str("\r\n");
        for r in 0..3_000 {
            text.push_str(&format!("k{r}"));
            for _ in 1..100 {
                text.push_str(",1");
            }
            text.push_str("\r\n");
        }
        let d = doc(&text);
        let spec = sub_spec((1..100).collect(), Total::Sum);
        let t = d
            .subtotals_within(&spec, &Progress::new(), 2 << 20)
            .unwrap();
        assert!(t.bytes.len() < 2 << 20);
        let out = t.into_document().unwrap();
        assert_eq!(out.row_count(), 1 + 3_000 * 2 + 1);
        assert_eq!(out.row_values(2).unwrap()[0], "k0 Total");
        assert_eq!(out.row_values(2).unwrap()[99], "1");
        let last = out.row_values(out.row_count() - 1).unwrap();
        assert_eq!(
            (last[0].as_str(), last[1].as_str()),
            ("Grand total", "3000")
        );
        // The cap is checked while the chunks run.
        assert!(matches!(
            d.subtotals_within(&spec, &Progress::new(), 200_000),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn subtotals_match_a_simple_reference() {
        // Random run lengths (many one-row groups, some long runs across
        // the 16,384-row chunk edges), every kind of total.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut rows: Vec<(String, String)> = Vec::new();
        let mut g = 0;
        while rows.len() < 60_000 {
            let len = if rnd(10) == 0 {
                1 + rnd(40_000)
            } else {
                1 + rnd(3)
            };
            let key = format!("g{}", g % 7);
            g += 1;
            for _ in 0..len {
                let v = match rnd(5) {
                    0 => String::new(),
                    1 => "x".to_string(),
                    _ => format!("{}", rnd(1000) as i64 - 500),
                };
                rows.push((key.clone(), v));
            }
        }
        let mut text = String::from("g,n\r\n");
        for (k, v) in &rows {
            text.push_str(&format!("{k},{v}\r\n"));
        }
        let d = doc(&text);
        for total in [
            Total::Sum,
            Total::Count,
            Total::Average,
            Total::Min,
            Total::Max,
        ] {
            let t = d
                .subtotals(&sub_spec(vec![1], total), &Progress::new())
                .unwrap();
            let out = t.into_document().unwrap();
            let got: Vec<Vec<String>> = (0..out.row_count())
                .map(|r| out.row_values(r).unwrap())
                .collect();
            // Reference: one pass, in order.
            let word = match total {
                Total::Sum => "Total",
                Total::Count => "Count",
                Total::Average => "Average",
                Total::Min => "Min",
                Total::Max => "Max",
            };
            let show = |vals: &[&str]| -> String {
                let nums: Vec<f64> = vals.iter().filter_map(|v| v.parse().ok()).collect();
                match total {
                    Total::Count => vals.iter().filter(|v| !v.is_empty()).count().to_string(),
                    _ if nums.is_empty() => String::new(),
                    Total::Sum => crate::fill::format_number(nums.iter().sum(), false),
                    Total::Average => crate::fill::format_number(
                        nums.iter().sum::<f64>() / nums.len() as f64,
                        false,
                    ),
                    Total::Min => crate::fill::format_number(
                        nums.iter().cloned().fold(f64::INFINITY, f64::min),
                        false,
                    ),
                    Total::Max => crate::fill::format_number(
                        nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                        false,
                    ),
                }
            };
            let trim = |mut v: Vec<String>| {
                while v.last().is_some_and(|c| c.is_empty()) {
                    v.pop();
                }
                v
            };
            let mut want = vec![vec!["g".to_string(), "n".to_string()]];
            let mut i = 0;
            while i < rows.len() {
                let mut j = i;
                while j < rows.len() && rows[j].0 == rows[i].0 {
                    want.push(trim(vec![rows[j].0.clone(), rows[j].1.clone()]));
                    j += 1;
                }
                let vals: Vec<&str> = rows[i..j].iter().map(|r| r.1.as_str()).collect();
                want.push(trim(vec![format!("{} {word}", rows[i].0), show(&vals)]));
                i = j;
            }
            let vals: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
            want.push(trim(vec![
                format!("Grand {}", word.to_lowercase()),
                show(&vals),
            ]));
            assert_eq!(got.len(), want.len(), "{total:?}");
            assert!(got == want, "{total:?} differs");
        }
    }

    #[test]
    fn top_values_survive_many_distinct_values() {
        // 200k values seen once, then one value making up 83% of rows: the
        // capped count used to drop it.
        let mut text = String::from("v\r\n");
        for i in 0..200_000 {
            text.push_str(&format!("u{i}\r\n"));
        }
        for _ in 0..1_000_000 {
            text.push_str("Z\r\n");
        }
        // And one spread thin among new values: every 50th row of a block.
        for i in 0..100_000 {
            text.push_str(if i % 50 == 0 { "Y\r\n" } else { "w\r\n" });
            if i % 50 == 1 {
                text.push_str(&format!("q{i}\r\n"));
            }
        }
        let d = doc(&text);
        let s = d
            .summary(0, true, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert!(s.distinct_capped && s.top_partial);
        assert_eq!(
            s.top[0],
            ValueCount {
                value: "Z".into(),
                cut: false,
                count: 1_000_000
            }
        );
        assert_eq!((s.top[1].value.as_str(), s.top[1].count), ("w", 98_000));
        assert_eq!((s.top[2].value.as_str(), s.top[2].count), ("Y", 2_000));
        let c = d
            .chart_data(
                &ChartSpec {
                    kind: ChartKind::Count,
                    label: Some(0),
                    values: vec![],
                    skip_first: true,
                    read: ReadOptions::default(),
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(c.labels[0], "Z");
        assert_eq!(c.series[0][0], Some(1_000_000.0));
        assert_eq!(c.labels.last().map(String::as_str), Some("Other"));
        let rows = (200_000 + 1_000_000 + 100_000 + 2_000) as f64;
        let shown: f64 = c.series[0].iter().map(|x| x.unwrap()).sum();
        assert_eq!(shown, rows);
    }

    #[test]
    fn long_top_values_are_marked_cut() {
        let long = "x".repeat(300);
        let d = doc(&format!("v\r\n{long}\r\n{long}\r\nb\r\n"));
        let s = d
            .summary(0, true, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert!(s.top[0].cut && s.top[0].value.ends_with('…'));
        assert!(!s.top[1].cut);
    }

    #[test]
    fn sums_have_no_rounding_noise() {
        let d = doc(&format!("v\r\n{}", "0.1\r\n".repeat(1_000_000)));
        let s = d
            .summary(0, true, ReadOptions::default(), &Progress::new())
            .unwrap();
        assert_eq!(s.sum, Some(100_000.0));
        let mut k = Kahan::default();
        k.add(f64::INFINITY);
        k.add(1.0);
        assert_eq!(k.value(), f64::INFINITY);
    }
}
