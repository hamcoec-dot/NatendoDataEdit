//! Fill series: what the fill handle and the Series dialog put in each new
//! cell, from the cells the user started with. Pure text in, text out.
//!
//! Kinds, tried in this order by [`FillMode::Auto`]:
//! - plain numbers (`12`, `-3.5`): one value copies, two or more continue
//!   the straight line through them;
//! - dates (`2026-11-12`, `11/12/2026`, `12.11.26`, with the same layout
//!   written back, any time part kept): one date steps a day, two or more
//!   keep their step in days, or in months when the day of month is equal;
//! - month and weekday names (`Jan`, `Monday`, any case);
//! - text with a number (`Item 1`, `Q07`): the number counts up, keeping
//!   zero padding;
//! - the user's custom orders (`Low, Medium, High`), see [`FillOrder`];
//! - anything else repeats the starting cells.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::value::ReadOptions;
use crate::Error;

/// How to fill.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FillMode {
    /// Pick from the starting cells (the fill handle).
    #[default]
    Auto,
    /// The fill handle with Ctrl held: a series where Auto copies (a single
    /// number), a copy where Auto makes a series.
    AutoSwap,
    /// Repeat the starting cells.
    Copy,
    /// First cell, then add `step` each time; stop past `stop`.
    Linear { step: f64, stop: Option<f64> },
    /// First cell, then multiply by `step` each time; stop past `stop`.
    Growth { step: f64, stop: Option<f64> },
    /// First cell is a date; add `step` units each time.
    Date { unit: DateUnit, step: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateUnit {
    #[default]
    Day,
    /// Monday to Friday only.
    Weekday,
    Month,
    Year,
}

/// Values for positions `i` (the starting cells are `0..n`; the fill goes
/// on at `n, n+1, ...` or back at `-1, -2, ...`).
#[derive(Debug, Clone)]
pub enum Series {
    Copy(Vec<String>),
    Number {
        a: f64,
        b: f64,
        decimals: Option<usize>,
        comma: bool,
        stop: Option<f64>,
    },
    Growth {
        start: f64,
        factor: f64,
        comma: bool,
        stop: Option<f64>,
    },
    Date {
        base: i64,
        shape: DateShape,
        unit: DateUnit,
        step: i64,
    },
    Names {
        list: &'static [&'static str],
        start: i64,
        step: i64,
        case: Case,
    },
    Counted {
        prefix: String,
        suffix: String,
        a: i64,
        step: i64,
        width: usize,
    },
    /// A number series written with thousands separators ("1,000").
    Grouped(Box<Series>),
    /// Times of day ("12:30"), in seconds; wraps around midnight.
    Time {
        a: i64,
        step: i64,
        seconds: bool,
        pad: bool,
    },
    /// Along a custom order (`FillSpec::orders[order]`), in its spelling;
    /// wraps around.
    Order {
        items: Arc<[String]>,
        start: i64,
        step: i64,
        order: usize,
    },
}

/// A number written with thousands separators: `1,234,567.5` (or with a
/// decimal comma, `1.234.567,5`). Returns the plain form ("1234567.5").
fn ungroup(s: &str, comma: bool) -> Option<String> {
    let t = s.trim();
    let (sep, point) = if comma { ('.', ',') } else { (',', '.') };
    let (sign, body) = match t.strip_prefix(['-', '+']) {
        Some(b) => (&t[..1], b),
        None => ("", t),
    };
    let (int, frac) = match body.split_once(point) {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let groups: Vec<&str> = int.split(sep).collect();
    if groups.len() < 2
        || !(1..=3).contains(&groups[0].len())
        || groups[1..].iter().any(|g| g.len() != 3)
        || groups
            .iter()
            .any(|g| !g.bytes().all(|b| b.is_ascii_digit()))
        || frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let mut out = format!("{sign}{}", groups.concat());
    if let Some(f) = frac {
        out.push(point);
        out.push_str(f);
    }
    Some(out)
}

/// Put thousands separators into a written number.
fn group(s: &str, comma: bool) -> String {
    let (sep, point) = if comma { ('.', ',') } else { (',', '.') };
    let (sign, body) = match s.strip_prefix('-') {
        Some(b) => ("-", b),
        None => ("", s),
    };
    let (int, rest) = match body.find(point) {
        Some(p) => (&body[..p], &body[p..]),
        None => (body, ""),
    };
    if !int.bytes().all(|b| b.is_ascii_digit()) {
        return s.to_string(); // exponent form: left alone
    }
    let mut out = String::from(sign);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out.push_str(rest);
    out
}

/// A time of day: `H:MM`, `HH:MM` or `HH:MM:SS` → (seconds, has seconds,
/// two-digit hour).
fn parse_time(s: &str) -> Option<(i64, bool, bool)> {
    let t = s.trim();
    let parts: Vec<&str> = t.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let num = |p: &str, max: i64, width: std::ops::RangeInclusive<usize>| -> Option<i64> {
        if !width.contains(&p.len()) || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse::<i64>().ok().filter(|v| *v <= max)
    };
    let h = num(parts[0], 23, 1..=2)?;
    let m = num(parts[1], 59, 2..=2)?;
    let sec = match parts.get(2) {
        Some(p) => num(p, 59, 2..=2)?,
        None => 0,
    };
    Some((
        h * 3600 + m * 60 + sec,
        parts.len() == 3,
        parts[0].len() == 2,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    Upper,
    Lower,
    Title,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const DAYS_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const LISTS: [&[&str]; 4] = [&MONTHS, &MONTHS_SHORT, &DAYS, &DAYS_SHORT];

/// A plain number: optional sign, digits, optional decimals (`.`, or `,`
/// with decimal commas). Returns the value and its decimal places.
fn plain_number(s: &str, comma: bool) -> Option<(f64, usize)> {
    let t = s.trim();
    let body = t.strip_prefix(['-', '+']).unwrap_or(t);
    let point = if comma { ',' } else { '.' };
    let (int, frac) = match body.split_once(point) {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let digits = |x: &str| !x.is_empty() && x.len() <= 15 && x.bytes().all(|b| b.is_ascii_digit());
    if !digits(int) || frac.is_some_and(|f| !digits(f)) {
        return None;
    }
    let v: f64 = t.replace(',', ".").parse().ok()?;
    Some((v, frac.map_or(0, str::len)))
}

/// A number written for a cell: whole numbers without decimals, others to
/// 12 significant digits without trailing zeros (no `0.30000000000000004`).
pub fn format_number(v: f64, comma: bool) -> String {
    let s = if v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let mag = if v == 0.0 {
            0
        } else {
            v.abs().log10().floor() as i32
        };
        let decimals = (11 - mag).clamp(0, 15) as usize;
        let mut s = format!("{v:.decimals$}");
        if s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
        if s == "-0" {
            s = "0".into();
        }
        s
    };
    if comma {
        s.replace('.', ",")
    } else {
        s
    }
}

/// Straight line `a + b*i` through values at positions `0..n`.
fn line(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    if v.len() < 2 {
        return (v.first().copied().unwrap_or(0.0), 0.0);
    }
    let mi = (n - 1.0) / 2.0;
    let mv = v.iter().sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (i, x) in v.iter().enumerate() {
        num += (i as f64 - mi) * (x - mv);
        den += (i as f64 - mi) * (i as f64 - mi);
    }
    let b = num / den;
    (mv - b * mi, b)
}

// ---- dates ----

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date of a day count since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub(crate) fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Y,
    M,
    D,
}

/// How a date is written, so new dates are written the same way.
#[derive(Debug, Clone, PartialEq)]
pub struct DateShape {
    order: [Part; 3],
    sep: char,
    widths: [usize; 3],
    tail: String,
}

fn parse_date_shape(s: &str, read: ReadOptions) -> Option<(i64, DateShape)> {
    let s = s.trim();
    let end = s.find([' ', 'T']).unwrap_or(s.len());
    let (date, tail) = s.split_at(end);
    let sep = date.chars().find(|c| matches!(c, '-' | '/' | '.'))?;
    let groups: Vec<&str> = date.split(sep).collect();
    if groups.len() != 3
        || groups
            .iter()
            .any(|g| g.is_empty() || g.len() > 4 || !g.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let n: Vec<u32> = groups.iter().map(|g| g.parse().unwrap_or(0)).collect();
    let order = if groups[0].len() == 4 {
        [Part::Y, Part::M, Part::D]
    } else if groups[2].len() == 4 || groups[2].len() == 2 {
        let mdy = [Part::M, Part::D, Part::Y];
        let dmy = [Part::D, Part::M, Part::Y];
        if n[0] > 12 {
            dmy
        } else if n[1] > 12 {
            mdy
        } else if read.date_order == crate::value::DateOrder::Dmy {
            dmy
        } else {
            mdy
        }
    } else {
        return None;
    };
    let (mut y, mut m, mut d) = (0i64, 0u32, 0u32);
    for (k, p) in order.iter().enumerate() {
        match p {
            Part::Y => {
                y = n[k] as i64;
                if groups[k].len() == 2 {
                    y += if y < 30 { 2000 } else { 1900 };
                } else if groups[k].len() != 4 {
                    return None;
                }
            }
            Part::M => m = n[k],
            Part::D => d = n[k],
        }
    }
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    let widths = [groups[0].len(), groups[1].len(), groups[2].len()];
    let shape = DateShape {
        order,
        sep,
        widths,
        tail: tail.to_string(),
    };
    Some((days_from_civil(y, m, d), shape))
}

fn write_date(days: i64, shape: &DateShape) -> String {
    let (y, m, d) = civil_from_days(days);
    let mut out = String::new();
    for (k, p) in shape.order.iter().enumerate() {
        if k > 0 {
            out.push(shape.sep);
        }
        let w = shape.widths[k];
        match p {
            Part::Y if w == 2 => out.push_str(&format!("{:02}", y.rem_euclid(100))),
            Part::Y => out.push_str(&format!("{y:04}")),
            Part::M => out.push_str(&format!("{m:0w$}")),
            Part::D => out.push_str(&format!("{d:0w$}")),
        }
    }
    out.push_str(&shape.tail);
    out
}

fn add_months(days: i64, months: i64) -> i64 {
    let (y, m, d) = civil_from_days(days);
    let total = y * 12 + (m as i64 - 1) + months;
    let (ny, nm) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
    days_from_civil(ny, nm, d.min(days_in_month(ny, nm)))
}

/// Weekday of a day count: 0 = Monday.
fn weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

fn add_weekdays(days: i64, k: i64) -> i64 {
    let mut d = days;
    // Start from a working day.
    while weekday(d) >= 5 {
        d += if k >= 0 { 1 } else { -1 };
    }
    let weeks = k / 5;
    let mut rest = k % 5;
    d += weeks * 7;
    while rest != 0 {
        d += rest.signum();
        if weekday(d) < 5 {
            rest -= rest.signum();
        }
    }
    d
}

// ---- names and counted text ----

fn case_of(s: &str) -> Case {
    if s.chars().all(|c| !c.is_lowercase()) && s.chars().count() > 1 {
        Case::Upper
    } else if s.chars().all(|c| !c.is_uppercase()) {
        Case::Lower
    } else {
        Case::Title
    }
}

fn find_name(s: &str) -> Option<(&'static [&'static str], i64)> {
    let t = s.trim();
    LISTS.iter().find_map(|list| {
        list.iter()
            .position(|n| n.eq_ignore_ascii_case(t))
            .map(|i| (*list, i as i64))
    })
}

/// `prefix`, the last run of digits, `suffix`.
fn counted(s: &str) -> Option<(&str, &str, &str)> {
    let b = s.as_bytes();
    let end = b.iter().rposition(u8::is_ascii_digit)? + 1;
    let start = b[..end]
        .iter()
        .rposition(|c| !c.is_ascii_digit())
        .map_or(0, |p| p + 1);
    if end - start > 15 {
        return None;
    }
    Some((&s[..start], &s[start..end], &s[end..]))
}

// ---- custom orders ----

/// A custom order the fill handle can follow: the user's saved sort orders.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FillOrder {
    pub name: String,
    pub items: Vec<String>,
    /// One starting cell follows this order. Otherwise only two or more
    /// starting cells stepping evenly along it do (a one-cell drag of a
    /// name on an order made from a column of names copies it).
    pub one_cell: bool,
}

/// Distinct values looked up by scanning before every order is indexed.
const SCANS_BEFORE_INDEX: usize = 32;

/// A value as orders match it: trimmed, case and accents folded (as sort
/// matches a custom order).
fn order_key(s: &str) -> String {
    let mut out = String::new();
    crate::value::text_key(s.trim(), false, &mut out);
    if let Some(i) = out.find('\0') {
        out.truncate(i);
    }
    out
}

/// `item` matches `key` (an [`order_key`]): plain ASCII compares in place.
fn same_key(item: &str, key: &str) -> bool {
    let t = item.trim();
    if t.is_ascii() {
        t.len() == key.len() && t.eq_ignore_ascii_case(key)
    } else {
        order_key(t) == key
    }
}

/// Finds starting cells on the custom orders of one fill. Lookups scan the
/// orders (no map is built for a fill that matches nothing); a fill with
/// many different starting values indexes them once.
pub struct OrderMatch<'a> {
    orders: &'a [FillOrder],
    shared: Vec<Option<Arc<[String]>>>,
    /// Key to (order, first position), per order once looked at.
    maps: HashMap<usize, HashMap<String, u32>>,
    /// Key to the orders holding it (order, first position), in order.
    found: HashMap<String, Vec<(usize, u32)>>,
    index: Option<HashMap<String, Vec<(usize, u32)>>>,
}

impl<'a> OrderMatch<'a> {
    pub fn new(orders: &'a [FillOrder]) -> OrderMatch<'a> {
        OrderMatch {
            orders,
            shared: vec![None; orders.len()],
            maps: HashMap::new(),
            found: HashMap::new(),
            index: None,
        }
    }

    /// The orders holding `key`, first ones first, with the first position.
    fn holding(&mut self, key: &str) -> Vec<(usize, u32)> {
        if let Some(ix) = &self.index {
            return ix.get(key).cloned().unwrap_or_default();
        }
        if let Some(v) = self.found.get(key) {
            return v.clone();
        }
        if self.found.len() >= SCANS_BEFORE_INDEX {
            let mut ix: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
            for (j, o) in self.orders.iter().enumerate() {
                for (p, item) in o.items.iter().enumerate() {
                    let v = ix.entry(order_key(item)).or_default();
                    if v.last().is_none_or(|x| x.0 != j) {
                        v.push((j, p as u32));
                    }
                }
            }
            let out = ix.get(key).cloned().unwrap_or_default();
            self.index = Some(ix);
            return out;
        }
        let out: Vec<(usize, u32)> = self
            .orders
            .iter()
            .enumerate()
            .filter_map(|(j, o)| {
                o.items
                    .iter()
                    .position(|i| same_key(i, key))
                    .map(|p| (j, p as u32))
            })
            .collect();
        self.found.insert(key.to_string(), out.clone());
        out
    }

    /// Where `key` first is on order `j`.
    fn pos_in(&mut self, j: usize, key: &str) -> Option<u32> {
        let items = &self.orders[j].items;
        let map = self.maps.entry(j).or_insert_with(|| {
            let mut m = HashMap::with_capacity(items.len());
            for (p, item) in items.iter().enumerate() {
                m.entry(order_key(item)).or_insert(p as u32);
            }
            m
        });
        map.get(key).copied()
    }

    /// The series along the first order the starting cells are on: one cell
    /// only on an order marked `one_cell`; two or more stepping evenly.
    fn series(&mut self, start: &[String]) -> Option<Series> {
        if self.orders.is_empty() || start.is_empty() {
            return None;
        }
        let n = start.len();
        let k0 = order_key(&start[0]);
        if k0.is_empty() {
            return None;
        }
        let keys: Vec<String> = start[1..].iter().map(|s| order_key(s)).collect();
        for (j, p0) in self.holding(&k0) {
            if n == 1 && !self.orders[j].one_cell {
                continue;
            }
            let len = self.orders[j].items.len() as i64;
            let mut pos = vec![p0 as i64];
            for k in &keys {
                match self.pos_in(j, k) {
                    Some(p) => pos.push(p as i64),
                    None => break,
                }
            }
            if pos.len() < n {
                continue;
            }
            let step = if n >= 2 {
                (pos[1] - pos[0]).rem_euclid(len)
            } else {
                1
            };
            if step == 0
                || !pos
                    .windows(2)
                    .all(|w| (w[1] - w[0]).rem_euclid(len) == step)
            {
                continue;
            }
            let items = self.shared[j]
                .get_or_insert_with(|| self.orders[j].items.clone().into())
                .clone();
            return Some(Series::Order {
                items,
                start: pos[0],
                step,
                order: j,
            });
        }
        None
    }
}

impl Series {
    /// The series for a line of starting cells (at least one).
    pub fn new(start: &[String], mode: FillMode, read: ReadOptions) -> Result<Series, Error> {
        Series::new_with(start, mode, read, &mut OrderMatch::new(&[]))
    }

    /// The custom order this series follows (an index into the fill's
    /// orders).
    pub fn order(&self) -> Option<usize> {
        match self {
            Series::Order { order, .. } => Some(*order),
            _ => None,
        }
    }

    /// As [`Series::new`], also following the custom orders of `orders`
    /// (automatic fills only).
    pub fn new_with(
        start: &[String],
        mode: FillMode,
        read: ReadOptions,
        orders: &mut OrderMatch,
    ) -> Result<Series, Error> {
        let comma = read.decimal_comma;
        let need_number = |what: &str| -> Result<f64, Error> {
            start
                .first()
                .and_then(|s| {
                    plain_number(s, comma)
                        .map(|v| v.0)
                        .or_else(|| crate::coltype::Reader::plain(read).number(s))
                })
                .ok_or_else(|| Error::Invalid(format!("{what} needs a number in the first cell")))
        };
        match mode {
            FillMode::Copy => Ok(Series::Copy(start.to_vec())),
            FillMode::Linear { step, stop } => {
                check_finite(step)?;
                Ok(Series::Number {
                    a: need_number("a linear series")?,
                    b: step,
                    decimals: None,
                    comma,
                    stop,
                })
            }
            FillMode::Growth { step, stop } => {
                check_finite(step)?;
                Ok(Series::Growth {
                    start: need_number("a growth series")?,
                    factor: step,
                    comma,
                    stop,
                })
            }
            FillMode::Date { unit, step } => {
                let (base, shape) = start
                    .first()
                    .and_then(|s| parse_date_shape(s, read))
                    .ok_or_else(|| {
                        Error::Invalid("a date series needs a date in the first cell".into())
                    })?;
                Ok(Series::Date {
                    base,
                    shape,
                    unit,
                    step,
                })
            }
            FillMode::Auto => Ok(Series::auto(start, read, orders)),
            // Ctrl copies where the plain drag follows an order (as in Excel).
            FillMode::AutoSwap => Ok(match Series::auto(start, read, orders) {
                Series::Copy(v) => match v.as_slice() {
                    [one] if plain_number(one, comma).is_some() => Series::Number {
                        a: plain_number(one, comma).map_or(0.0, |x| x.0),
                        b: 1.0,
                        decimals: plain_number(one, comma).map(|x| x.1),
                        comma,
                        stop: None,
                    },
                    _ => Series::Copy(v),
                },
                Series::Number { .. }
                | Series::Counted { .. }
                | Series::Grouped(_)
                | Series::Time { .. }
                | Series::Order { .. } => Series::Copy(start.to_vec()),
                other => other,
            }),
        }
    }

    fn auto(start: &[String], read: ReadOptions, orders: &mut OrderMatch) -> Series {
        let comma = read.decimal_comma;
        let n = start.len();
        // Numbers.
        if let Some(nums) = start
            .iter()
            .map(|s| plain_number(s, comma))
            .collect::<Option<Vec<_>>>()
        {
            if n == 1 {
                return Series::Copy(start.to_vec());
            }
            let v: Vec<f64> = nums.iter().map(|x| x.0).collect();
            let (a, b) = line(&v);
            return Series::Number {
                a,
                b,
                decimals: nums.iter().map(|x| x.1).max(),
                comma,
                stop: None,
            };
        }
        // Numbers with thousands separators ("1,000", "2,000"); one of them
        // copies, as one plain number does.
        if n == 1 && ungroup(&start[0], comma).is_some() {
            return Series::Copy(start.to_vec());
        }
        if n >= 2 {
            if let Some(plain) = start
                .iter()
                .map(|s| {
                    plain_number(s, comma)
                        .map(|_| s.trim().to_string())
                        .or_else(|| ungroup(s, comma))
                })
                .collect::<Option<Vec<_>>>()
            {
                if let s @ Series::Number { .. } = Series::auto(&plain, read, orders) {
                    return Series::Grouped(Box::new(s));
                }
            }
        }
        // Times of day ("12:30", "13:00"); one time steps by an hour.
        if let Some(times) = start
            .iter()
            .map(|s| parse_time(s))
            .collect::<Option<Vec<_>>>()
        {
            let step = if n == 1 {
                3600
            } else {
                let v: Vec<f64> = times.iter().map(|t| t.0 as f64).collect();
                line(&v).1.round() as i64
            };
            return Series::Time {
                a: times[0].0,
                step,
                seconds: times.iter().any(|t| t.1),
                pad: times[0].2,
            };
        }
        // Dates written the same way.
        if let Some(dates) = start
            .iter()
            .map(|s| parse_date_shape(s, read))
            .collect::<Option<Vec<_>>>()
        {
            if dates.iter().all(|d| d.1 == dates[0].1) {
                let base = dates[0].0;
                let shape = dates[0].1.clone();
                if n == 1 {
                    return Series::Date {
                        base,
                        shape,
                        unit: DateUnit::Day,
                        step: 1,
                    };
                }
                let ymd: Vec<(i64, u32, u32)> =
                    dates.iter().map(|d| civil_from_days(d.0)).collect();
                let months: Vec<i64> = ymd.iter().map(|(y, m, _)| y * 12 + *m as i64).collect();
                let step_m = months[1] - months[0];
                if step_m != 0
                    && ymd.iter().all(|x| x.2 == ymd[0].2)
                    && months.windows(2).all(|w| w[1] - w[0] == step_m)
                {
                    return Series::Date {
                        base,
                        shape,
                        unit: DateUnit::Month,
                        step: step_m,
                    };
                }
                let v: Vec<f64> = dates.iter().map(|d| d.0 as f64).collect();
                let (_, b) = line(&v);
                return Series::Date {
                    base,
                    shape,
                    unit: DateUnit::Day,
                    step: b.round() as i64,
                };
            }
        }
        // Month and day names.
        if let Some(names) = start
            .iter()
            .map(|s| find_name(s))
            .collect::<Option<Vec<_>>>()
        {
            let list = names[0].0;
            if names.iter().all(|x| std::ptr::eq(x.0, list)) {
                let len = list.len() as i64;
                let step = if n >= 2 {
                    (names[1].1 - names[0].1).rem_euclid(len)
                } else {
                    1
                };
                let consistent = names
                    .windows(2)
                    .all(|w| (w[1].1 - w[0].1).rem_euclid(len) == step);
                if consistent {
                    return Series::Names {
                        list,
                        start: names[0].1,
                        step,
                        case: case_of(start[0].trim()),
                    };
                }
            }
        }
        // Text with a number.
        if let Some(parts) = start.iter().map(|s| counted(s)).collect::<Option<Vec<_>>>() {
            let (p0, d0, s0) = parts[0];
            if parts.iter().all(|(p, _, s)| *p == p0 && *s == s0) {
                let nums: Vec<i64> = parts.iter().map(|x| x.1.parse().unwrap_or(0)).collect();
                let step = if n >= 2 {
                    let total = nums[n - 1] - nums[0];
                    if total % (n as i64 - 1) == 0
                        && nums
                            .windows(2)
                            .all(|w| w[1] - w[0] == total / (n as i64 - 1))
                    {
                        total / (n as i64 - 1)
                    } else {
                        return Series::Copy(start.to_vec());
                    }
                } else {
                    1
                };
                let width = if d0.len() > 1 && d0.starts_with('0') {
                    d0.len()
                } else {
                    0
                };
                return Series::Counted {
                    prefix: p0.to_string(),
                    suffix: s0.to_string(),
                    a: nums[0],
                    step,
                    width,
                };
            }
            // Numbered text that does not make a series is copied, never
            // looked up on the custom orders.
            return Series::Copy(start.to_vec());
        }
        // The user's custom orders: only for text none of the above reads.
        if let Some(s) = orders.series(start) {
            return s;
        }
        Series::Copy(start.to_vec())
    }

    /// The value at position `i`, or `None` once a stop value is passed.
    pub fn at(&self, i: i64) -> Option<String> {
        Some(match self {
            Series::Copy(v) => v[i.rem_euclid(v.len() as i64) as usize].clone(),
            Series::Number {
                a,
                b,
                decimals,
                comma,
                stop,
            } => {
                let x = a + b * i as f64;
                if !x.is_finite() || past(x, *b, *stop) {
                    return None;
                }
                match decimals {
                    Some(d)
                        if *d > 0
                            && ((x * 10f64.powi(*d as i32)).round()
                                - x * 10f64.powi(*d as i32))
                            .abs()
                                < 1e-6 =>
                    {
                        let s = format!("{x:.d$}", d = *d);
                        if *comma {
                            s.replace('.', ",")
                        } else {
                            s
                        }
                    }
                    _ => format_number(x, *comma),
                }
            }
            Series::Growth {
                start,
                factor,
                comma,
                stop,
            } => {
                let x = start * factor.powi(i.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
                let dir = if *factor >= 1.0 { 1.0 } else { -1.0 } * start.signum();
                if !x.is_finite() || past(x, dir, *stop) {
                    return None;
                }
                format_number(x, *comma)
            }
            Series::Date {
                base,
                shape,
                unit,
                step,
            } => {
                let k = step.checked_mul(i)?;
                // Far past year 9999 either way; also keeps the sums small.
                if k.unsigned_abs() > 5_000_000 {
                    return None;
                }
                let d = match unit {
                    DateUnit::Day => base.checked_add(k)?,
                    DateUnit::Weekday => add_weekdays(*base, k),
                    DateUnit::Month => add_months(*base, k),
                    DateUnit::Year => add_months(*base, k.checked_mul(12)?),
                };
                let y = civil_from_days(d).0;
                if !(1..=9999).contains(&y) {
                    return None;
                }
                write_date(d, shape)
            }
            Series::Order {
                items, start, step, ..
            } => {
                let len = items.len() as i64;
                items[(start + step * i.rem_euclid(len)).rem_euclid(len) as usize].clone()
            }
            Series::Names {
                list,
                start,
                step,
                case,
            } => {
                let len = list.len() as i64;
                let name = list[(start + step * i.rem_euclid(len)).rem_euclid(len) as usize];
                match case {
                    Case::Upper => name.to_uppercase(),
                    Case::Lower => name.to_lowercase(),
                    Case::Title => name.to_string(),
                }
            }
            Series::Counted {
                prefix,
                suffix,
                a,
                step,
                width,
            } => {
                let v = a.checked_add(step.checked_mul(i)?)?.unsigned_abs();
                format!("{prefix}{v:0width$}{suffix}")
            }
            Series::Grouped(inner) => {
                let comma = matches!(**inner, Series::Number { comma: true, .. });
                group(&inner.at(i)?, comma)
            }
            Series::Time {
                a,
                step,
                seconds,
                pad,
            } => {
                let t = a.checked_add(step.checked_mul(i)?)?.rem_euclid(86_400);
                let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
                let mut out = if *pad {
                    format!("{h:02}:{m:02}")
                } else {
                    format!("{h}:{m:02}")
                };
                if *seconds {
                    out.push_str(&format!(":{s:02}"));
                }
                out
            }
        })
    }
}

fn check_finite(x: f64) -> Result<(), Error> {
    if x.is_finite() {
        Ok(())
    } else {
        Err(Error::Invalid("the step must be a number".into()))
    }
}

/// `x` went past `stop` in the direction of travel.
fn past(x: f64, dir: f64, stop: Option<f64>) -> bool {
    match stop {
        Some(s) if dir > 0.0 => x > s + 1e-9 * s.abs().max(1.0),
        Some(s) if dir < 0.0 => x < s - 1e-9 * s.abs().max(1.0),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start: &[&str], mode: FillMode, from: i64, n: i64) -> Vec<String> {
        let start: Vec<String> = start.iter().map(|s| s.to_string()).collect();
        let s = Series::new(&start, mode, ReadOptions::default()).unwrap();
        (from..from + n).filter_map(|i| s.at(i)).collect()
    }

    #[test]
    fn grouped_numbers_and_times() {
        assert_eq!(
            run(&["1,000", "2,000"], FillMode::Auto, 2, 2),
            ["3,000", "4,000"]
        );
        assert_eq!(run(&["999", "1,999"], FillMode::Auto, 2, 1), ["2,999"]);
        assert_eq!(
            run(&["-1,500.50", "-1,000.50"], FillMode::Auto, 2, 3),
            ["-500.50", "-0.50", "499.50"]
        );
        assert_eq!(
            run(&["1,000", "2,000"], FillMode::AutoSwap, 2, 2),
            ["1,000", "2,000"]
        );
        // One grouped number copies, as one plain number does.
        assert_eq!(run(&["1,000"], FillMode::Auto, 1, 2), ["1,000", "1,000"]);
        // Not grouping: "1,00" and "12,3456".
        assert_eq!(run(&["1,00", "2,00"], FillMode::Auto, 2, 1), ["1,00"]);
        assert_eq!(
            run(&["12:30", "13:00"], FillMode::Auto, 2, 3),
            ["13:30", "14:00", "14:30"]
        );
        assert_eq!(run(&["9:00"], FillMode::Auto, 1, 2), ["10:00", "11:00"]);
        assert_eq!(run(&["9:00"], FillMode::AutoSwap, 1, 2), ["9:00", "9:00"]);
        assert_eq!(
            run(&["23:00:30", "23:30:30"], FillMode::Auto, 2, 2),
            ["00:00:30", "00:30:30"]
        );
        assert_eq!(run(&["12:30", "13:00"], FillMode::Auto, -1, 1), ["12:00"]);
        assert!(
            parse_time("24:00").is_none()
                && parse_time("1:5").is_none()
                && parse_time("a:00").is_none()
        );
        let comma = ReadOptions {
            decimal_comma: true,
            ..Default::default()
        };
        let s = Series::new(&["1.000,5".into(), "2.000,5".into()], FillMode::Auto, comma).unwrap();
        assert_eq!(s.at(2).unwrap(), "3.000,5");
    }

    #[test]
    fn numbers() {
        assert_eq!(run(&["5"], FillMode::Auto, 1, 3), ["5", "5", "5"]);
        assert_eq!(run(&["5"], FillMode::AutoSwap, 1, 3), ["6", "7", "8"]);
        assert_eq!(run(&["1", "3"], FillMode::Auto, 2, 3), ["5", "7", "9"]);
        assert_eq!(run(&["1", "3"], FillMode::AutoSwap, 2, 2), ["1", "3"]);
        assert_eq!(run(&["0.1", "0.2"], FillMode::Auto, 2, 2), ["0.3", "0.4"]);
        assert_eq!(run(&["1.50", "1.75"], FillMode::Auto, 2, 1), ["2.00"]);
        assert_eq!(run(&["10", "8"], FillMode::Auto, -2, 2), ["14", "12"]);
        // Best straight line through uneven values.
        assert_eq!(
            run(&["1", "2", "4"], FillMode::Auto, 3, 1),
            ["5.33333333333"]
        );
        assert_eq!(
            run(
                &["2"],
                FillMode::Linear {
                    step: 3.0,
                    stop: Some(10.0)
                },
                1,
                9
            ),
            ["5", "8"]
        );
        assert_eq!(
            run(
                &["1"],
                FillMode::Growth {
                    step: 2.0,
                    stop: None
                },
                1,
                4
            ),
            ["2", "4", "8", "16"]
        );
        assert_eq!(format_number(-0.0000001, false), "-0.0000001");
        // Exponent notation is accepted as a start value (review note M4b-2).
        assert_eq!(
            run(
                &["1e3"],
                FillMode::Linear {
                    step: 1.0,
                    stop: None
                },
                1,
                1
            ),
            ["1001"]
        );
        assert_eq!(format_number(1234.5, true), "1234,5");
    }

    #[test]
    fn dates() {
        assert_eq!(
            run(&["2026-11-12"], FillMode::Auto, 1, 3),
            ["2026-11-13", "2026-11-14", "2026-11-15"]
        );
        assert_eq!(
            run(&["1/31/2026", "2/28/2026"], FillMode::Auto, 2, 1),
            ["3/28/2026"],
            "day step 28 days, not months (days of month differ)"
        );
        assert_eq!(
            run(&["01/15/2026", "02/15/2026"], FillMode::Auto, 2, 2),
            ["03/15/2026", "04/15/2026"]
        );
        assert_eq!(
            run(
                &["2024-01-31"],
                FillMode::Date {
                    unit: DateUnit::Month,
                    step: 1
                },
                1,
                2
            ),
            ["2024-02-29", "2024-03-31"]
        );
        // Friday then Monday.
        assert_eq!(
            run(
                &["2026-11-13"],
                FillMode::Date {
                    unit: DateUnit::Weekday,
                    step: 1
                },
                1,
                2
            ),
            ["2026-11-16", "2026-11-17"]
        );
        assert_eq!(
            run(&["31.12.25 10:00"], FillMode::Auto, 1, 1),
            ["01.01.26 10:00"]
        );
        assert_eq!(run(&["2026-11-12"], FillMode::Auto, -1, 1), ["2026-11-11"]);
        for z in [-800_000i64, -1, 0, 1, 19_000, 2_900_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
    }

    #[test]
    fn names_and_counted_text() {
        assert_eq!(run(&["Mon"], FillMode::Auto, 1, 3), ["Tue", "Wed", "Thu"]);
        assert_eq!(
            run(&["NOVEMBER"], FillMode::Auto, 1, 2),
            ["DECEMBER", "JANUARY"]
        );
        assert_eq!(run(&["jan", "mar"], FillMode::Auto, 2, 2), ["may", "jul"]);
        assert_eq!(run(&["Item 1"], FillMode::Auto, 1, 2), ["Item 2", "Item 3"]);
        assert_eq!(
            run(&["Q07a"], FillMode::Auto, 1, 3),
            ["Q08a", "Q09a", "Q10a"]
        );
        assert_eq!(run(&["Item 1"], FillMode::AutoSwap, 1, 1), ["Item 1"]);
        assert_eq!(run(&["a", "b"], FillMode::Auto, 2, 3), ["a", "b", "a"]);
        assert_eq!(run(&["a", "b"], FillMode::Auto, -1, 1), ["b"]);
    }

    #[test]
    fn hostile_values_never_panic() {
        let odd = [
            "",
            " ",
            "-",
            ".",
            "9999999999999999999",
            "99/99/9999",
            "0000-00-00",
            "9999-12-31",
            "1e400",
            "x99999999999999999999",
            "٣",
            "Ⅻ",
        ];
        for a in odd {
            for b in odd {
                for mode in [FillMode::Auto, FillMode::AutoSwap, FillMode::Copy] {
                    let s =
                        Series::new(&[a.into(), b.into()], mode, ReadOptions::default()).unwrap();
                    for i in [-1_000_000_000i64, -1, 2, 3, 1_000_000_000, i64::MAX / 2] {
                        let _ = s.at(i);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "fill_order_tests.rs"]
mod order_tests;
