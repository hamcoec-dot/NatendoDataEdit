//! The gap audit's date mode. Every cell is classified by the date radar's
//! own reader first; a date only one order could mean is never guessed. Each
//! date becomes the number of its step (day, business day, month, week), and
//! the walk the numbers use does the rest.

use std::collections::HashMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::{series_key, series_parts, Cell, GapReport, GapsSpec, Names, Walk};
use super::{EXAMPLE_ROWS, MAX_SERIES};
use crate::document::{Document, Progress};
use crate::fill::civil_from_days;
use crate::ops::{Cols, CHUNK};
use crate::value::{classify_date, compact_days, date_days, DateKind, DateOrder, ReadOptions};
use crate::Error;

/// Longest "every N days" step.
pub const MAX_STEP_DAYS: u32 = 3_660;
/// Added to every step number, so dates before 1970 stay above nought.
const OFFSET: i64 = 10_000_000;

/// How far apart the dates are expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum DateStep {
    Day,
    /// Monday to Friday; no holidays.
    Business,
    /// Seven days from the typed start, else the earliest date.
    Week,
    Month,
    /// Every `n` days from the typed start, else the earliest date.
    Days {
        n: u32,
    },
}

/// What a date audit left out, or read with care.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DateNotes {
    /// Dates both orders could mean, in a column with no order set: not read.
    pub ambiguous: u64,
    pub ambiguous_rows: Vec<Option<u64>>,
    /// Dates read with a two-digit year.
    pub two_digit: u64,
    pub two_digit_rows: Vec<Option<u64>>,
    /// Business days only: dates on a Saturday or Sunday.
    pub weekend: u64,
    pub weekend_rows: Vec<Option<u64>>,
    /// Weeks and every N days only: dates between the steps.
    pub off_step: u64,
    pub off_step_rows: Vec<Option<u64>>,
}

impl DateNotes {
    /// The counts to list in the result table (those that apply).
    pub(super) fn counts(&self) -> impl Iterator<Item = (&'static str, u64)> {
        [
            ("Ambiguous dates not read", self.ambiguous),
            ("Two-digit years", self.two_digit),
            ("On a weekend", self.weekend),
            ("Off the step", self.off_step),
        ]
        .into_iter()
        .filter(|(_, n)| *n > 0)
    }
}

/// Day 0 (1970-01-01) was a Thursday: Monday is 0.
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

fn iso(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

fn month_of_day(day: i64) -> i64 {
    let (y, m, _) = civil_from_days(day);
    y * 12 + i64::from(m) - 1
}

/// The steps as numbers.
#[derive(Clone, Copy)]
struct Scale {
    step: DateStep,
    /// Weeks and N days count from this day.
    anchor: i64,
}

impl Scale {
    fn days(&self) -> i64 {
        match self.step {
            DateStep::Week => 7,
            DateStep::Days { n } => i64::from(n),
            _ => 1,
        }
    }

    /// The step holding `day` (for a day between steps, the one before).
    fn floor(&self, day: i64) -> i64 {
        match self.step {
            DateStep::Day => day,
            DateStep::Business => {
                // Saturday and Sunday count as the Friday before.
                let m = day + 3;
                m.div_euclid(7) * 5 + m.rem_euclid(7).min(4)
            }
            DateStep::Month => month_of_day(day),
            DateStep::Week | DateStep::Days { .. } => (day - self.anchor).div_euclid(self.days()),
        }
    }

    fn words(&self, number: u64) -> String {
        let n = number as i64 - OFFSET;
        match self.step {
            DateStep::Day => iso(n),
            DateStep::Business => iso(n.div_euclid(5) * 7 + n.rem_euclid(5) - 3),
            DateStep::Month => format!("{:04}-{:02}", n.div_euclid(12), n.rem_euclid(12) + 1),
            DateStep::Week | DateStep::Days { .. } => iso(self.anchor + n * self.days()),
        }
    }
}

/// A typed start or end as its day: `yyyy-mm-dd` (a month step also takes
/// `yyyy-mm`).
fn typed_day(text: &str, step: DateStep, what: &str) -> Result<Option<i64>, Error> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let month = step == DateStep::Month;
    let bad = || {
        Error::Invalid(format!(
            "type the {what} as a date like 2026-03-31{}",
            if month {
                " or a month like 2026-03"
            } else {
                ""
            }
        ))
    };
    let parts: Vec<&str> = t.split('-').collect();
    let number = |s: &str, len: usize| -> Option<u32> {
        (s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (y, m, d) = match parts.as_slice() {
        [y, m, d] => (number(y, 4), number(m, 2), number(d, 2)),
        [y, m] if month => (number(y, 4), number(m, 2), Some(1)),
        _ => return Err(bad()),
    };
    let (Some(y), Some(m), Some(d)) = (y, m, d) else {
        return Err(bad());
    };
    let day = (y > 0)
        .then(|| date_days(i64::from(y), m, d))
        .flatten()
        .ok_or_else(bad)?;
    if step == DateStep::Business && weekday(day) >= 5 {
        let name = if weekday(day) == 5 {
            "Saturday"
        } else {
            "Sunday"
        };
        return Err(Error::Invalid(format!(
            "{} is a {name}: type a business day as the {what}",
            iso(day)
        )));
    }
    Ok(Some(day))
}

#[derive(Default)]
struct Found {
    /// Day (offset), row and series of every date read.
    cells: Vec<(i64, u32, u32)>,
    series: Names,
    blank: u64,
    other: u64,
    other_rows: Vec<u32>,
    notes: DateNotes,
    ambiguous_ids: Vec<u32>,
    two_digit_ids: Vec<u32>,
}

fn keep(rows: &mut Vec<u32>, id: u32) {
    if rows.len() < EXAMPLE_ROWS {
        rows.push(id);
    }
}

impl Document {
    pub(super) fn gap_dates(
        &self,
        spec: &GapsSpec,
        step: DateStep,
        progress: &Progress,
    ) -> Result<GapReport, Error> {
        if let DateStep::Days { n } = step {
            if !(1..=MAX_STEP_DAYS).contains(&n) {
                return Err(Error::Invalid(format!(
                    "a step is 1 to {MAX_STEP_DAYS} days"
                )));
            }
        }
        let start = typed_day(&spec.start, step, "start")?;
        let end = typed_day(&spec.end, step, "end")?;
        if let (Some(a), Some(b)) = (start, end) {
            if a > b {
                return Err(Error::Invalid("the start is greater than the end".into()));
            }
        }
        // The date radar's own rules for the column: whether it may hold
        // dates, which order it is set to, and whether eight digits are dates.
        let scan = self.date_scan(spec.col, spec.keep_first, None, progress)?;
        let compact = scan.compact > 0;
        let ctx = self.read_ctx(ReadOptions::default());
        let read = ctx.reader(spec.col);
        let mut cols = vec![spec.col];
        cols.extend(&spec.series);
        let reader = Cols::new(self, &cols);
        let rows = self.data_rows(spec.keep_first);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let parts: Vec<Found> = (0..((r1 - r0) as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<Found, Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut f = Found::default();
                f.cells.reserve((e - s) as usize);
                let mut buf = Vec::new();
                let mut key = String::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let v = reader.get(&buf, 0);
                    if v.trim().is_empty() {
                        f.blank += 1;
                        continue;
                    }
                    let class = classify_date(v);
                    let readable = match class.kind {
                        DateKind::Iso | DateKind::Written | DateKind::SameEither => true,
                        // The order a column is set to reads only its own.
                        DateKind::MonthFirstOnly => scan.set_order != Some(DateOrder::Dmy),
                        DateKind::DayFirstOnly => scan.set_order != Some(DateOrder::Mdy),
                        DateKind::Ambiguous => {
                            if scan.set_order.is_none() {
                                f.notes.ambiguous += 1;
                                keep(&mut f.ambiguous_ids, id);
                                continue;
                            }
                            true
                        }
                        DateKind::Compact => compact,
                        DateKind::NotDate => false,
                    };
                    let day = match class.kind {
                        _ if !readable => None,
                        // The date reader leaves eight digits as a number;
                        // the radar's scan said they are dates here.
                        // A time may follow the eight digits.
                        DateKind::Compact => {
                            v.trim().get(..8).and_then(compact_days).map(|d| d as f64)
                        }
                        _ => read.date(v),
                    };
                    let Some(day) = day else {
                        f.other += 1;
                        keep(&mut f.other_rows, id);
                        continue;
                    };
                    if class.two_digit_year {
                        f.notes.two_digit += 1;
                        keep(&mut f.two_digit_ids, id);
                    }
                    let series = if spec.series.is_empty() {
                        0
                    } else {
                        series_key(
                            (0..spec.series.len()).map(|k| reader.get(&buf, k + 1)),
                            &mut key,
                        );
                        f.series.id(&key)?
                    };
                    // A time of day is left off: the day it falls on.
                    f.cells.push((day.floor() as i64, id, series));
                }
                progress.step(e - s)?;
                Ok(f)
            })
            .collect::<Result<_, _>>()?;

        let mut out = GapReport {
            shown: r1 - r0,
            revision: self.data_revision(),
            spec: spec.clone(),
            ..Default::default()
        };
        let mut notes = DateNotes::default();
        let mut names = Names::default();
        // Each part's series in the whole table's numbering.
        let globals: Vec<Vec<u32>> = parts
            .iter()
            .map(|p| p.series.names.iter().map(|n| names.id(n)).collect())
            .collect::<Result<_, _>>()?;
        // Weeks and every N days count from the typed start, else from each
        // series' own earliest date.
        let mut earliest: HashMap<u32, i64> = HashMap::new();
        if start.is_none() && matches!(step, DateStep::Week | DateStep::Days { .. }) {
            for (p, global) in parts.iter().zip(&globals) {
                progress.step(0)?;
                for &(day, _, series) in &p.cells {
                    let series = global.get(series as usize).copied().unwrap_or(0);
                    let first = earliest.entry(series).or_insert(day);
                    *first = (*first).min(day);
                }
            }
        }
        let scale_of = |series: u32| Scale {
            step,
            anchor: start
                .or_else(|| earliest.get(&series).copied())
                .unwrap_or(0),
        };
        // Each group's way of counting.
        let mut scales: Vec<Scale> = Vec::new();
        // Groups in the order met: one for each series with a date.
        let mut groups: HashMap<u32, u32> = HashMap::new();
        let mut keys: Vec<Vec<String>> = Vec::new();
        let mut cells = Vec::with_capacity(parts.iter().map(|p| p.cells.len()).sum());
        let (mut other_ids, mut ambiguous, mut two_digit) = (Vec::new(), Vec::new(), Vec::new());
        let (mut weekend, mut off) = (Vec::new(), Vec::new());
        for (p, global) in parts.into_iter().zip(&globals) {
            progress.step(0)?;
            out.blank += p.blank;
            out.other_shape += p.other;
            other_ids.extend(p.other_rows);
            notes.ambiguous += p.notes.ambiguous;
            notes.two_digit += p.notes.two_digit;
            ambiguous.extend(p.ambiguous_ids);
            two_digit.extend(p.two_digit_ids);
            for (day, row, series) in p.cells {
                if step == DateStep::Business && weekday(day) >= 5 {
                    notes.weekend += 1;
                    keep(&mut weekend, row);
                    continue;
                }
                let series = if spec.series.is_empty() {
                    0
                } else {
                    global[series as usize]
                };
                let scale = scale_of(series);
                let n = scale.floor(day);
                if matches!(step, DateStep::Week | DateStep::Days { .. })
                    && (day - scale.anchor).rem_euclid(scale.days()) != 0
                {
                    notes.off_step += 1;
                    keep(&mut off, row);
                    continue;
                }
                let group = match groups.get(&series) {
                    Some(&g) => g,
                    None => {
                        if keys.len() >= MAX_SERIES {
                            return Err(super::too_many_series());
                        }
                        let g = keys.len() as u32;
                        groups.insert(series, g);
                        scales.push(scale);
                        keys.push(if spec.series.is_empty() {
                            Vec::new()
                        } else {
                            series_parts(&names.names[series as usize])
                        });
                        g
                    }
                };
                cells.push(Cell {
                    number: (n + OFFSET) as u64,
                    row,
                    around: 0,
                    series: group,
                    len: 0,
                    zero_led: false,
                });
            }
        }
        let first = |mut ids: Vec<u32>| {
            ids.truncate(EXAMPLE_ROWS);
            self.file_rows_of(&ids)
        };
        out.other_rows = first(other_ids);
        notes.ambiguous_rows = first(ambiguous);
        notes.two_digit_rows = first(two_digit);
        notes.weekend_rows = first(weekend);
        notes.off_step_rows = first(off);
        out.dates = Some(notes);
        // A typed range with nothing in it is still a gap.
        if keys.is_empty() && spec.series.is_empty() && start.is_some() && end.is_some() {
            keys.push(Vec::new());
            scales.push(scale_of(0));
        }
        let number = |scale: Scale, day: i64| (scale.floor(day) + OFFSET) as u64;
        // A typed start anchors every series alike, so one number serves.
        let first = start.map(|d| number(scale_of(0), d));
        let last = |g: u32| end.map(|d| number(scales[g as usize], d));
        let format = |g: u32, n: u64| scales[g as usize].words(n);
        self.walk(
            out,
            cells,
            Walk {
                keys,
                start: first,
                end: &last,
                format: &format,
            },
            progress,
        )
    }
}
