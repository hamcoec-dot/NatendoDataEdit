//! Data > Find relationships: which columns go together. `ZIP -> State` holds
//! when rows with one ZIP nearly always have one State; two columns are
//! "blank together" when a row that lacks one lacks the other.
//!
//! Pairs are found on a sample (the first rows and then every 50th, so the
//! same table always gives the same answer), and the best are counted again
//! over every shown row.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress, View};
use crate::filter::FilterSpec;
use crate::ops::{check_cols, Cols, CHUNK};
use crate::reshape::RowPlan;
use crate::Error;

/// Columns scanned at once.
pub const MAX_RELATION_COLS: usize = 40;
/// Rows sampled from the top.
const SAMPLE_HEAD: u64 = 200_000;
/// After them, every this many rows.
const SAMPLE_EVERY: u64 = 50;
/// Findings counted again over every row.
pub const VERIFIED: usize = 25;
/// Findings listed.
pub const MAX_FINDINGS: usize = 500;
/// A column this distinct (per thousand of its filled cells) names its rows:
/// it settles everything, so it is left out.
const UNIQUE_PER_MILLE: u64 = 995;
/// A dependency must beat the dependent column's own commonest value by this
/// many points.
const LIFT: f64 = 5.0;
/// Text held for the different values at most.
const MAX_VALUE_BYTES: usize = 512 << 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelationsSpec {
    pub cols: Vec<u32>,
    /// The first row holds the column titles.
    pub header: bool,
    /// Show what holds in at least this many percent of rows (80 to 100).
    pub min_percent: f64,
    /// Stop looking for pairs after this long.
    pub seconds: u32,
}

impl Default for RelationsSpec {
    fn default() -> RelationsSpec {
        RelationsSpec {
            cols: Vec::new(),
            header: true,
            min_percent: 95.0,
            seconds: 20,
        }
    }
}

impl RelationsSpec {
    pub fn check(&self) -> Result<(), Error> {
        let mut seen = std::collections::HashSet::new();
        if !(2..=MAX_RELATION_COLS).contains(&self.cols.len())
            || !self.cols.iter().all(|c| seen.insert(c))
        {
            return Err(Error::Invalid(format!(
                "pick 2 to {MAX_RELATION_COLS} different columns"
            )));
        }
        if !(80.0..=100.0).contains(&self.min_percent) {
            return Err(Error::Invalid(
                "show what holds in 80 to 100 percent".into(),
            ));
        }
        if !(1..=600).contains(&self.seconds) {
            return Err(Error::Invalid("look for 1 to 600 seconds".into()));
        }
        Ok(())
    }
}

/// One direction of a dependency.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Holds {
    /// Rows counted: those whose first value is in two rows or more.
    pub coverage: u64,
    /// Of them, rows with their value's commonest partner.
    pub agreeing: u64,
    /// Different values counted.
    pub values: u64,
    /// Values with more than one partner.
    pub broken_values: u64,
    pub percent: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// Column `a` settles column `b`.
    Depends,
    /// Each settles the other.
    OneToOne,
    /// Blank in the same rows.
    CoBlank,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub kind: RelationKind,
    pub a: u32,
    pub b: u32,
    /// `a` to `b`; for blanks, coverage is rows blank in either and
    /// agreeing rows blank in both.
    pub there: Holds,
    /// `b` to `a`, one to one only.
    pub back: Option<Holds>,
    /// Counted over every shown row (else estimated from the sample).
    pub exact: bool,
    /// Coverage under a tenth of the rows looked at.
    pub few_rows: bool,
    /// Under 3 rows to a value on average: agreement comes easily.
    pub few_per_value: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeftOut {
    /// One value (or none).
    Constant,
    /// Nearly every row its own value.
    Unique,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RelationsReport {
    pub findings: Vec<Finding>,
    /// Columns that cannot settle or be settled.
    pub left_out: Vec<(u32, LeftOut)>,
    /// Pairs that hold only because the second column is mostly one value.
    pub mostly_one: Vec<(u32, u32)>,
    pub rows: u64,
    pub sampled: u64,
    /// Pairs not looked at when the time ran out.
    pub unchecked: u64,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    header: bool,
}

/// One column's cells as numbers: 0 blank, the same number for the same
/// trimmed text.
struct Ids {
    ids: Vec<u32>,
    /// Different filled values.
    distinct: u32,
    filled: u64,
}

/// A column's rows put together by value: `rows[from[v]..from[v + 1]]` hold
/// value `v + 1`.
struct Groups {
    rows: Vec<u32>,
    from: Vec<u32>,
}

fn groups(c: &Ids) -> Groups {
    let n = c.distinct as usize;
    let mut from = vec![0u32; n + 1];
    for &v in &c.ids {
        if v > 0 {
            from[v as usize] += 1;
        }
    }
    for i in 1..=n {
        from[i] += from[i - 1];
    }
    let mut at = from.clone();
    let mut rows = vec![0u32; c.filled as usize];
    for (r, &v) in c.ids.iter().enumerate() {
        if v > 0 {
            let p = &mut at[v as usize - 1];
            rows[*p as usize] = r as u32;
            *p += 1;
        }
    }
    Groups { rows, from }
}

/// Counting room one thread uses again and again.
#[derive(Default)]
struct Room {
    n: Vec<u32>,
    total: Vec<u32>,
    touched: Vec<u32>,
}

/// How far `a` settles `b`, and the share (percent) of the counted rows that
/// hold `b`'s commonest value. With `odd`, the rows that break it are marked,
/// each with the first row of its group that holds the commonest value.
fn holds(
    a: &Groups,
    b: &Ids,
    room: &mut Room,
    mut odd: Option<&mut Vec<(u32, u32)>>,
) -> (Holds, f64) {
    let size = b.distinct as usize + 1;
    room.n.clear();
    room.n.resize(size, 0);
    room.total.clear();
    room.total.resize(size, 0);
    let mut out = Holds::default();
    for w in a.from.windows(2) {
        let rows = &a.rows[w[0] as usize..w[1] as usize];
        if rows.len() < 2 {
            continue;
        }
        room.touched.clear();
        let mut count = 0u32;
        // The commonest partner; ties go to the one met first.
        let (mut best, mut most) = (0u32, 0u32);
        for &r in rows {
            let v = b.ids[r as usize];
            if v == 0 {
                continue;
            }
            count += 1;
            let n = &mut room.n[v as usize];
            if *n == 0 {
                room.touched.push(v);
            }
            *n += 1;
        }
        if count < 2 {
            for &v in &room.touched {
                room.n[v as usize] = 0;
            }
            continue;
        }
        for &v in &room.touched {
            if room.n[v as usize] > most {
                (best, most) = (v, room.n[v as usize]);
            }
        }
        out.coverage += u64::from(count);
        out.agreeing += u64::from(most);
        out.values += 1;
        out.broken_values += u64::from(most < count);
        for &v in &room.touched {
            room.total[v as usize] += room.n[v as usize];
            room.n[v as usize] = 0;
        }
        if let Some(odd) = odd.as_deref_mut() {
            let won = rows
                .iter()
                .copied()
                .find(|&r| b.ids[r as usize] == best)
                .unwrap_or(0);
            odd.extend(
                rows.iter()
                    .copied()
                    .filter(|&r| !matches!(b.ids[r as usize], 0) && b.ids[r as usize] != best)
                    .map(|r| (r, won)),
            );
        }
    }
    let common = room.total.iter().copied().max().unwrap_or(0);
    let share = |n: u64| {
        if out.coverage == 0 {
            0.0
        } else {
            n as f64 * 100.0 / out.coverage as f64
        }
    };
    let common = share(u64::from(common));
    out.percent = share(out.agreeing);
    (out, common)
}

/// Rows blank in either of two columns, and in both.
fn blanks(a: &Ids, b: &Ids) -> Holds {
    let (mut either, mut both) = (0u64, 0u64);
    for (&x, &y) in a.ids.iter().zip(&b.ids) {
        either += u64::from(x == 0 || y == 0);
        both += u64::from(x == 0 && y == 0);
    }
    Holds {
        coverage: either,
        agreeing: both,
        values: 0,
        broken_values: 0,
        percent: if either == 0 {
            0.0
        } else {
            both as f64 * 100.0 / either as f64
        },
    }
}

/// Whether blanks are common enough (1 row in 100) to be worth pairing.
fn has_blanks(c: &Ids) -> bool {
    let blank = c.ids.len() as u64 - c.filled;
    blank > 0 && blank * 100 >= c.ids.len() as u64
}

/// The line for a dependency, or `None` when it does not hold well enough.
/// `Err(())`: it holds only because `b` is mostly one value.
fn judge(h: Holds, common: f64, min: f64) -> Result<Option<Holds>, ()> {
    if h.coverage == 0 || h.percent < min {
        Ok(None)
    } else if h.percent - common < LIFT {
        Err(())
    } else {
        Ok(Some(h))
    }
}

impl Finding {
    fn flag(&mut self, looked: u64) {
        let h = self.there;
        self.few_rows = h.coverage.saturating_mul(10) < looked;
        self.few_per_value =
            self.kind != RelationKind::CoBlank && h.values > 0 && h.coverage < h.values * 3;
    }

    fn score(&self, looked: u64) -> f64 {
        let rows = match self.kind {
            RelationKind::CoBlank => self.there.agreeing,
            _ => self.there.coverage,
        };
        self.there.percent * rows as f64 / looked.max(1) as f64
    }
}

impl Document {
    /// Columns `cols` of the shown rows at positions `at`, as numbers.
    fn relation_ids(
        &self,
        cols: &[u32],
        at: &[u64],
        progress: &Progress,
    ) -> Result<Vec<Ids>, Error> {
        self.relation_ids_as(cols, at, false, progress)
    }

    /// [`Document::relation_ids`], with case folded when `fold` (a "settled by" rule
    /// that ignores case; the scan always compares case).
    fn relation_ids_as(
        &self,
        cols: &[u32],
        at: &[u64],
        fold: bool,
        progress: &Progress,
    ) -> Result<Vec<Ids>, Error> {
        let reader = Cols::new(self, cols);
        let k = cols.len();
        // Each part numbers its own values; the parts are then joined, one
        // column at a time.
        type Local = (Vec<u32>, Vec<String>);
        let parts: Vec<Vec<Local>> = at
            .par_chunks(CHUNK)
            .map(|rows| -> Result<Vec<Local>, Error> {
                let mut seen: Vec<HashMap<String, u32>> = (0..k).map(|_| HashMap::new()).collect();
                let mut out: Vec<Local> = (0..k)
                    .map(|_| (Vec::with_capacity(rows.len()), Vec::new()))
                    .collect();
                let mut buf = Vec::new();
                for &r in rows {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for i in 0..k {
                        let v = reader.get(&buf, i).trim();
                        let lower;
                        let v = if fold {
                            lower = v.to_lowercase();
                            lower.as_str()
                        } else {
                            v
                        };
                        let n = if v.is_empty() {
                            0
                        } else if let Some(&n) = seen[i].get(v) {
                            n
                        } else {
                            out[i].1.push(v.to_string());
                            let n = out[i].1.len() as u32;
                            seen[i].insert(v.to_string(), n);
                            n
                        };
                        out[i].0.push(n);
                    }
                }
                progress.step(rows.len() as u64)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let bytes: usize = parts
            .iter()
            .flat_map(|p| p.iter())
            .flat_map(|c| c.1.iter())
            .map(|s| s.len() + 48)
            .sum();
        if bytes > value_bytes_limit() {
            return Err(Error::OutOfMemory(
                "these columns hold too many different values to scan at once; pick fewer",
            ));
        }
        (0..k)
            .into_par_iter()
            .map(|i| -> Result<Ids, Error> {
                progress.step(0)?;
                let mut all: HashMap<&str, u32> = HashMap::new();
                let mut ids = Vec::with_capacity(at.len());
                let mut filled = 0;
                for p in &parts {
                    let (local, names) = &p[i];
                    let global: Vec<u32> = names
                        .iter()
                        .map(|n| {
                            let next = all.len() as u32 + 1;
                            *all.entry(n.as_str()).or_insert(next)
                        })
                        .collect();
                    for &n in local {
                        if n == 0 {
                            ids.push(0);
                        } else {
                            ids.push(global[n as usize - 1]);
                            filled += 1;
                        }
                    }
                }
                Ok(Ids {
                    ids,
                    distinct: all.len() as u32,
                    filled,
                })
            })
            .collect()
    }

    /// Look for relationships among the picked columns of the shown rows.
    pub fn find_relations(
        &self,
        spec: &RelationsSpec,
        progress: &Progress,
    ) -> Result<RelationsReport, Error> {
        self.ready_for_bulk()?;
        spec.check()?;
        check_cols(self, &spec.cols)?;
        let rows = self.data_rows(spec.header);
        let total = rows.end - rows.start;
        let head = rows.start.saturating_add(SAMPLE_HEAD).min(rows.end);
        let sample: Vec<u64> = (rows.start..head)
            .chain((head..rows.end).step_by(SAMPLE_EVERY as usize))
            .collect();
        let looked = sample.len() as u64;
        progress.start(looked);
        let ids = self.relation_ids(&spec.cols, &sample, progress)?;
        let mut out = RelationsReport {
            rows: total,
            sampled: looked,
            revision: self.data_revision(),
            header: spec.header,
            ..Default::default()
        };
        // Which columns can take part in a dependency.
        let useful: Vec<bool> = ids
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if c.distinct < 2 {
                    out.left_out.push((spec.cols[i], LeftOut::Constant));
                    false
                } else if u64::from(c.distinct) * 1000 > c.filled * UNIQUE_PER_MILLE {
                    out.left_out.push((spec.cols[i], LeftOut::Unique));
                    false
                } else {
                    true
                }
            })
            .collect();
        let n = ids.len();
        // (first, second, blanks): cheapest first, so a short budget covers
        // the most pairs.
        let mut pairs: Vec<(usize, usize, bool)> = Vec::new();
        for a in 0..n {
            for b in 0..n {
                if a != b
                    && useful[a]
                    && useful[b]
                    && u64::from(ids[a].distinct) * 2 <= ids[a].filled
                    && ids[b].distinct <= ids[a].distinct
                {
                    pairs.push((a, b, false));
                }
                if a < b && (has_blanks(&ids[a]) || has_blanks(&ids[b])) {
                    pairs.push((a, b, true));
                }
            }
        }
        pairs.sort_by_key(|&(a, b, blank)| (!blank, ids[a].distinct, a, b));
        let grouped: Vec<Option<Groups>> = (0..n)
            .into_par_iter()
            .map(|i| useful[i].then(|| groups(&ids[i])))
            .collect();
        progress.start(pairs.len() as u64);
        let until = Instant::now() + Duration::from_secs(u64::from(spec.seconds));
        enum Seen {
            No,
            Late,
            Found(Holds),
            MostlyOne,
        }
        let seen: Vec<Seen> = pairs
            .par_iter()
            .map_init(
                Room::default,
                |room, &(a, b, blank)| -> Result<Seen, Error> {
                    progress.step(1)?;
                    if Instant::now() >= until {
                        return Ok(Seen::Late);
                    }
                    if blank {
                        let h = blanks(&ids[a], &ids[b]);
                        return Ok(if h.coverage > 0 && h.percent >= spec.min_percent {
                            Seen::Found(h)
                        } else {
                            Seen::No
                        });
                    }
                    let g = grouped[a].as_ref().expect("useful");
                    let (h, common) = holds(g, &ids[b], room, None);
                    Ok(match judge(h, common, spec.min_percent) {
                        Ok(Some(h)) => Seen::Found(h),
                        Ok(None) => Seen::No,
                        Err(()) => Seen::MostlyOne,
                    })
                },
            )
            .collect::<Result<_, _>>()?;
        let mut at: HashMap<(usize, usize), usize> = HashMap::new();
        let exact = looked == total;
        for (&(a, b, blank), s) in pairs.iter().zip(&seen) {
            match s {
                Seen::No => {}
                Seen::Late => out.unchecked += 1,
                Seen::MostlyOne => out.mostly_one.push((spec.cols[a], spec.cols[b])),
                Seen::Found(h) => {
                    let (x, y) = (spec.cols[a], spec.cols[b]);
                    // The other direction already listed: one line for both.
                    if !blank {
                        if let Some(&i) = at.get(&(b, a)) {
                            out.findings[i].kind = RelationKind::OneToOne;
                            out.findings[i].back = Some(*h);
                            continue;
                        }
                        at.insert((a, b), out.findings.len());
                    }
                    out.findings.push(Finding {
                        kind: if blank {
                            RelationKind::CoBlank
                        } else {
                            RelationKind::Depends
                        },
                        a: x,
                        b: y,
                        there: *h,
                        back: None,
                        exact,
                        few_rows: false,
                        few_per_value: false,
                    });
                }
            }
        }
        let rank = |f: &mut Vec<Finding>, looked: u64| {
            f.sort_by(|x, y| {
                y.score(looked)
                    .total_cmp(&x.score(looked))
                    .then_with(|| (x.a, x.b, x.kind as u8).cmp(&(y.a, y.b, y.kind as u8)))
            });
        };
        rank(&mut out.findings, looked);
        out.findings.truncate(MAX_FINDINGS);
        for f in &mut out.findings {
            f.flag(looked);
        }
        if !exact {
            self.verify_relations(spec, rows, &mut out, progress)?;
        }
        Ok(out)
    }

    /// Count the best findings again over every shown row; those that no
    /// longer hold are dropped.
    fn verify_relations(
        &self,
        spec: &RelationsSpec,
        rows: std::ops::Range<u64>,
        out: &mut RelationsReport,
        progress: &Progress,
    ) -> Result<(), Error> {
        let n = out.findings.len().min(VERIFIED);
        let mut cols: Vec<u32> = out.findings[..n].iter().flat_map(|f| [f.a, f.b]).collect();
        cols.sort_unstable();
        cols.dedup();
        if cols.is_empty() {
            return Ok(());
        }
        let every: Vec<u64> = rows.collect();
        let total = every.len() as u64;
        progress.start(total);
        let ids = self.relation_ids(&cols, &every, progress)?;
        let of = |c: u32| cols.binary_search(&c).expect("listed");
        let checked: Vec<Option<Finding>> = out.findings[..n]
            .par_iter()
            .map_init(Room::default, |room, f| -> Result<Option<Finding>, Error> {
                progress.step(0)?;
                let (a, b) = (of(f.a), of(f.b));
                let mut f = f.clone();
                f.exact = true;
                if f.kind == RelationKind::CoBlank {
                    f.there = blanks(&ids[a], &ids[b]);
                    if f.there.coverage == 0 || f.there.percent < spec.min_percent {
                        return Ok(None);
                    }
                } else {
                    let mut one = |a: usize, b: usize| {
                        let (h, common) = holds(&groups(&ids[a]), &ids[b], room, None);
                        judge(h, common, spec.min_percent).ok().flatten()
                    };
                    let there = one(a, b);
                    let back = if f.kind == RelationKind::OneToOne {
                        one(b, a)
                    } else {
                        None
                    };
                    match (there, back) {
                        (Some(x), Some(y)) => (f.there, f.back) = (x, Some(y)),
                        (Some(x), None) => {
                            (f.there, f.back) = (x, None);
                            f.kind = RelationKind::Depends;
                        }
                        (None, Some(y)) => {
                            (f.a, f.b) = (f.b, f.a);
                            (f.there, f.back) = (y, None);
                            f.kind = RelationKind::Depends;
                        }
                        (None, None) => return Ok(None),
                    }
                }
                f.flag(total);
                Ok(Some(f))
            })
            .collect::<Result<_, _>>()?;
        let rest = out.findings.split_off(n);
        out.findings = checked.into_iter().flatten().chain(rest).collect();
        Ok(())
    }

    /// A view of the rows that break one finding (and the title row). For
    /// one to one, the rows that break either direction.
    pub fn plan_relation_exceptions(
        &self,
        report: &RelationsReport,
        kind: RelationKind,
        a: u32,
        b: u32,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        let rows = self.data_rows(report.header);
        if report.revision != self.data_revision() || rows.end - rows.start != report.rows {
            return Err(Error::Invalid(
                "the table changed since the scan: scan again".into(),
            ));
        }
        check_cols(self, &[a, b])?;
        let every: Vec<u64> = rows.collect();
        progress.start(every.len() as u64);
        let ids = self.relation_ids(&[a, b], &every, progress)?;
        let mut odd: Vec<(u32, u32)> = Vec::new();
        let mut room = Room::default();
        match kind {
            RelationKind::CoBlank => odd.extend(
                (0..every.len() as u32)
                    .filter(|&r| (ids[0].ids[r as usize] == 0) != (ids[1].ids[r as usize] == 0))
                    .map(|r| (r, r)),
            ),
            RelationKind::Depends => {
                holds(&groups(&ids[0]), &ids[1], &mut room, Some(&mut odd));
            }
            RelationKind::OneToOne => {
                holds(&groups(&ids[0]), &ids[1], &mut room, Some(&mut odd));
                holds(&groups(&ids[1]), &ids[0], &mut room, Some(&mut odd));
            }
        }
        let base = self.next_row_id();
        let mut bits = vec![0u64; (base as usize).div_ceil(64)];
        let mut shown = 0;
        let mut set = |id: u32| {
            let w = &mut bits[(id >> 6) as usize];
            let m = 1u64 << (id & 63);
            let new = *w & m == 0;
            *w |= m;
            new
        };
        for (r, _) in odd {
            let id = self.id_at(every[r as usize]).ok_or(Error::OutOfRange)?;
            shown += u64::from(set(id));
        }
        if shown == 0 {
            return Err(Error::Invalid("no row breaks it".into()));
        }
        if report.header && self.row_count() > 0 {
            set(self.id_at(0).expect("a first row"));
        }
        let mut plan = RowPlan::new(self);
        plan.rows = shown;
        plan.view = Some(View::new(
            FilterSpec {
                keep_first: report.header,
                ..Default::default()
            },
            bits,
            base,
        ));
        Ok(plan)
    }
}

impl Document {
    /// A "settled by" rule (`col` settled by `by`) over the shown rows `at`, in that
    /// order, counted exactly as [`Document::plan_relation_exceptions`] counts a finding:
    /// for each row that breaks it, its offset in `at` and the offset of the first row of
    /// its group holding the commonest value. `None` when the two columns hold more text
    /// of different values than a scan holds.
    pub(crate) fn settled_odd(
        &self,
        col: u32,
        by: u32,
        match_case: bool,
        at: &[u64],
        progress: &Progress,
    ) -> Result<Option<Vec<(u32, u32)>>, Error> {
        let ids = match self.relation_ids_as(&[by, col], at, !match_case, progress) {
            Ok(ids) => ids,
            Err(Error::OutOfMemory(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut odd = Vec::new();
        holds(
            &groups(&ids[0]),
            &ids[1],
            &mut Room::default(),
            Some(&mut odd),
        );
        Ok(Some(odd))
    }
}

#[cfg(test)]
thread_local! {
    /// A lower limit on the different values' text, for tests.
    pub(crate) static VALUE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_VALUE_BYTES) };
}

fn value_bytes_limit() -> usize {
    #[cfg(test)]
    {
        VALUE_BYTES.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        MAX_VALUE_BYTES
    }
}

#[cfg(test)]
#[path = "relations_tests.rs"]
mod tests;
