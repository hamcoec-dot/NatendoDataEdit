//! Reading cell text as numbers and dates, and ordering text the way people
//! expect: case and common accents folded, numbers inside text compared by
//! value ("file2" before "file10").
//!
//! Everything here works on untrusted text: inputs longer than
//! [`MAX_VALUE_LEN`] bytes are never numbers or dates, and every function is
//! linear in its input.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// Longest text read as a number or a date.
pub const MAX_VALUE_LEN: usize = 64;

/// Order of day and month in numeric dates such as `03/04/2026`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateOrder {
    /// Month first (03/04/2026 is March 4).
    #[default]
    Mdy,
    /// Day first (03/04/2026 is 3 April).
    Dmy,
}

/// How cell text is read as numbers and dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadOptions {
    /// `1.234,5` style: comma for decimals, dot for thousands.
    pub decimal_comma: bool,
    pub date_order: DateOrder,
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

const CURRENCY: &[char] = &['$', '€', '£', '¥', '₹', '₩', '₽', '¢'];

/// The number a cell holds, if it holds one: optional sign or `(123)`,
/// currency symbol, thousands separators in groups of three, decimals,
/// exponent, trailing `%` (divides by 100).
pub fn parse_number(s: &str, opts: ReadOptions) -> Option<f64> {
    let mut t = s.trim_matches(is_space);
    if t.is_empty() || t.len() > MAX_VALUE_LEN {
        return None;
    }
    let mut neg = false;
    if let Some(inner) = t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        neg = true;
        t = inner.trim_matches(is_space);
    }
    let mut percent = false;
    if let Some(x) = t.strip_suffix('%') {
        percent = true;
        t = x.trim_end_matches(is_space);
    }
    // Sign and currency in either order, currency also at the end.
    for _ in 0..2 {
        if let Some(x) = t.strip_prefix(['-', '\u{2212}']) {
            if neg {
                return None;
            }
            neg = true;
            t = x.trim_start_matches(is_space);
        } else if let Some(x) = t.strip_prefix('+') {
            t = x.trim_start_matches(is_space);
        }
        if let Some(x) = t.strip_prefix(CURRENCY) {
            t = x.trim_start_matches(is_space);
        }
    }
    if let Some(x) = t.strip_suffix(CURRENCY) {
        t = x.trim_end_matches(is_space);
    }
    let (dec, group) = if opts.decimal_comma {
        (b',', b'.')
    } else {
        (b'.', b',')
    };
    let b = t.as_bytes();
    let mut i = 0;
    let mut clean = String::with_capacity(b.len() + 1);
    if neg {
        clean.push('-');
    }
    // Integer part, with optional thousands groups.
    let mut digits = 0;
    let mut run = 0;
    let mut grouped = false;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_digit() {
            clean.push(c as char);
            digits += 1;
            run += 1;
        } else if c == group && run > 0 && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
            if (grouped && run != 3) || (!grouped && run > 3) {
                return None;
            }
            grouped = true;
            run = 0;
        } else {
            break;
        }
        i += 1;
    }
    if grouped && run != 3 {
        return None;
    }
    if i < b.len() && b[i] == dec {
        clean.push('.');
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            clean.push(b[i] as char);
            digits += 1;
            i += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        clean.push('e');
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            clean.push(b[i] as char);
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            clean.push(b[i] as char);
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    let v: f64 = clean.parse().ok()?;
    let v = if percent { v / 100.0 } else { v };
    v.is_finite().then_some(v)
}

/// Days from 1970-01-01 to a calendar date (proleptic Gregorian).
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

pub(crate) fn month_of(word: &str) -> Option<u32> {
    const NAMES: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    if word.len() < 3 {
        return None;
    }
    let w = word.strip_suffix('.').unwrap_or(word);
    if w == "sept" {
        return Some(9);
    }
    NAMES
        .iter()
        .position(|n| n.starts_with(w) && (w.len() == 3 || w.len() == n.len()))
        .map(|i| i as u32 + 1)
}

fn is_weekday(word: &str) -> bool {
    const DAYS: [&str; 7] = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    let w = word.strip_suffix('.').unwrap_or(word);
    w.len() >= 3
        && DAYS
            .iter()
            .any(|d| d.starts_with(w) && (w.len() <= 4 || w.len() == d.len()))
}

fn year_of(tok: &str) -> Option<i64> {
    let v: i64 = tok.parse().ok()?;
    match tok.len() {
        2 => Some(if v < 30 { 2000 + v } else { 1900 + v }),
        4 if v >= 1 => Some(v),
        _ => None,
    }
}

/// A day number from a token like `12`, `12th`, `1st`.
fn day_of(tok: &str) -> Option<u32> {
    let digits = tok.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &tok[digits.len()..];
    if !suffix.is_empty() && !matches!(suffix, "st" | "nd" | "rd" | "th") {
        return None;
    }
    if digits.is_empty() || digits.len() > 2 {
        return None;
    }
    digits.parse().ok()
}

pub(crate) fn date_days(y: i64, m: u32, d: u32) -> Option<i64> {
    ((1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)).then(|| days_from_civil(y, m, d))
}

/// Seconds into the day of a time like `13:45`, `1:45 pm`, `13:45:10.5`,
/// `10:00Z`, `10:00+02:00`; the offset is applied (result may leave 0..86400).
fn parse_time(t: &str) -> Option<f64> {
    let t = t.trim_matches(is_space);
    let b = t.as_bytes();
    let mut i = 0;
    let num = |i: &mut usize, max: usize| -> Option<u32> {
        let s = *i;
        while *i < b.len() && b[*i].is_ascii_digit() && *i - s < max {
            *i += 1;
        }
        if *i == s {
            None
        } else {
            t[s..*i].parse().ok()
        }
    };
    let h = num(&mut i, 2)?;
    if i >= b.len() || b[i] != b':' {
        return None;
    }
    i += 1;
    let s0 = i;
    let m = num(&mut i, 2)?;
    if i - s0 != 2 {
        return None;
    }
    let mut secs = 0.0;
    if i < b.len() && b[i] == b':' {
        i += 1;
        let s1 = i;
        let s = num(&mut i, 2)?;
        if i - s1 != 2 {
            return None;
        }
        secs = s as f64;
        if i < b.len() && (b[i] == b'.' || b[i] == b',') {
            let f0 = i;
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            secs += t[f0..i].replace(',', ".").parse::<f64>().ok()?;
        }
    }
    let rest = t[i..].trim_matches(is_space).to_ascii_lowercase();
    let (ampm, rest) = if let Some(r) = rest
        .strip_prefix("am")
        .or_else(|| rest.strip_prefix("a.m."))
    {
        (Some(false), r.trim_start().to_string())
    } else if let Some(r) = rest
        .strip_prefix("pm")
        .or_else(|| rest.strip_prefix("p.m."))
    {
        (Some(true), r.trim_start().to_string())
    } else {
        (None, rest)
    };
    let hour = match ampm {
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            (h % 12) + if pm { 12 } else { 0 }
        }
        None if h <= 24 => h,
        None => return None,
    };
    if m > 59 || secs >= 61.0 || (hour == 24 && (m > 0 || secs > 0.0)) {
        return None;
    }
    let mut offset = 0i64;
    let r = rest.as_str();
    if r == "z" || r == "utc" || r == "gmt" {
    } else if let Some(sign) = r.chars().next().filter(|c| *c == '+' || *c == '-') {
        let digits: String = r[1..].chars().filter(|c| *c != ':').collect();
        if !(digits.len() == 2 || digits.len() == 4) || !digits.bytes().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        let oh: i64 = digits[..2].parse().ok()?;
        let om: i64 = if digits.len() == 4 {
            digits[2..].parse().ok()?
        } else {
            0
        };
        if oh > 14 || om > 59 {
            return None;
        }
        offset = (oh * 3600 + om * 60) * if sign == '-' { -1 } else { 1 };
    } else if !r.is_empty() {
        return None;
    }
    Some(hour as f64 * 3600.0 + m as f64 * 60.0 + secs - offset as f64)
}

/// What a cell's date part is, before any order is chosen.
enum Form {
    /// A time alone.
    TimeOnly,
    /// A date only one reading fits: year first, or a month written out.
    Fixed { days: i64, written: bool },
    /// `a/b/year`: which of `a` and `b` is the day depends on the order.
    Slash {
        a: u32,
        b: u32,
        year: i64,
        two_digit: bool,
    },
    /// Eight digits that are a calendar date (`20260425`). Not read as a
    /// date by [`parse_date`].
    Compact,
}

/// The one splitter of a date's text: [`parse_date`] and [`classify_date`]
/// both read cells through it, so they cannot disagree. Allocates nothing.
fn date_form(s: &str) -> Option<(Form, Option<f64>)> {
    let t = s.trim_matches(is_space);
    if t.is_empty() || t.len() > MAX_VALUE_LEN {
        return None;
    }
    // Where a time starts: the digits just before the first ':'.
    let (date_part, time_part) = match t.find(':') {
        Some(c) => {
            // Digits are ASCII, so the digit run's start is a char boundary.
            let start = c - t[..c].bytes().rev().take_while(u8::is_ascii_digit).count();
            (&t[..start], Some(&t[start..]))
        }
        None => (t, None),
    };
    let secs = match time_part {
        Some(tp) => Some(parse_time(tp)?),
        None => None,
    };
    let date_part = date_part.trim_end_matches(|c: char| is_space(c) || c == 'T' || c == ',');
    if date_part.is_empty() {
        return secs.map(|secs| (Form::TimeOnly, Some(secs)));
    }
    // Month and weekday names are matched in lower case (a copy on the stack).
    let mut buf = [0u8; MAX_VALUE_LEN];
    let copy = &mut buf[..date_part.len()];
    copy.copy_from_slice(date_part.as_bytes());
    copy.make_ascii_lowercase();
    let lower = std::str::from_utf8(copy).ok()?;
    let mut parts = [""; 4];
    let mut n = 0;
    for x in lower
        .split(|c: char| is_space(c) || matches!(c, '-' | '/' | '.' | ','))
        .filter(|x| !x.is_empty())
    {
        *parts.get_mut(n)? = x;
        n += 1;
    }
    let mut toks = &parts[..n];
    if toks.first().is_some_and(|w| is_weekday(w)) {
        toks = &toks[1..];
    }
    let numeric = |x: &str| x.bytes().all(|c| c.is_ascii_digit());
    let fixed = |days: i64, written: bool| Form::Fixed { days, written };
    let form = match toks {
        [a, b, c] if numeric(a) && numeric(b) && numeric(c) => {
            if a.len() == 4 {
                fixed(
                    date_days(year_of(a)?, b.parse().ok()?, c.parse().ok()?)?,
                    false,
                )
            } else {
                if a.len() > 2 || b.len() > 2 {
                    return None;
                }
                Form::Slash {
                    a: a.parse().ok()?,
                    b: b.parse().ok()?,
                    year: year_of(c)?,
                    two_digit: c.len() == 2,
                }
            }
        }
        [a, b, c] => {
            if let Some(m) = month_of(a) {
                fixed(date_days(year_of(c)?, m, day_of(b)?)?, true)
            } else {
                let m = month_of(b)?;
                if a.len() == 4 && numeric(a) {
                    fixed(date_days(year_of(a)?, m, day_of(c)?)?, true)
                } else {
                    fixed(date_days(year_of(c)?, m, day_of(a)?)?, true)
                }
            }
        }
        [a, b] => {
            if let (Some(m), true) = (month_of(a), b.len() == 4) {
                fixed(date_days(year_of(b)?, m, 1)?, true)
            } else {
                return None;
            }
        }
        [a] if compact_days(a).is_some() => Form::Compact,
        _ => return None,
    };
    Some((form, secs))
}

/// The day of an eight-digit date (`yyyymmdd`): the one reading of that
/// form, for the classifier and for whoever is told such cells are dates.
pub(crate) fn compact_days(s: &str) -> Option<i64> {
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year = year_of(&s[..4])?;
    date_days(year, s[4..6].parse().ok()?, s[6..].parse().ok()?)
}

/// Month and day of a slash date read in `order`: a part over 12 is the day
/// whatever the order says.
fn slash_parts(a: u32, b: u32, order: DateOrder) -> (u32, u32) {
    if a > 12 {
        (b, a)
    } else if b > 12 {
        (a, b)
    } else {
        match order {
            DateOrder::Mdy => (a, b),
            DateOrder::Dmy => (b, a),
        }
    }
}

/// The date (and time) a cell holds, as days since 1970-01-01 with the time
/// as a fraction; a time alone is a fraction of day 0.
pub fn parse_date(s: &str, opts: ReadOptions) -> Option<f64> {
    let (form, secs) = date_form(s)?;
    let days = match form {
        Form::TimeOnly => return secs.map(|s| s / 86_400.0),
        Form::Fixed { days, .. } => days,
        Form::Slash { a, b, year, .. } => {
            let (m, d) = slash_parts(a, b, opts.date_order);
            date_days(year, m, d)?
        }
        Form::Compact => return None,
    };
    Some(days as f64 + secs.unwrap_or(0.0) / 86_400.0)
}

/// How a cell's date can be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateKind {
    /// Year first.
    Iso,
    /// Eight digits that are a calendar date.
    Compact,
    /// The month written as a word.
    Written,
    /// `a/b/year` only month-first can read (the second part is over 12).
    MonthFirstOnly,
    /// Only day-first can read it (the first part is over 12).
    DayFirstOnly,
    /// Both orders read it, as different days.
    Ambiguous,
    /// Both orders read it as the same day (`05/05/2026`).
    SameEither,
    NotDate,
}

/// A cell's date by its date part (a time after it changes nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateClass {
    pub kind: DateKind,
    /// The year is written with two digits (read as [`TWO_DIGIT_YEARS`]).
    pub two_digit_year: bool,
}

/// How a two-digit year is read, in words.
pub const TWO_DIGIT_YEARS: &str = "00 to 29 are 2000 to 2029, 30 to 99 are 1930 to 1999";

/// Classify a cell without choosing an order. Allocates nothing.
pub fn classify_date(s: &str) -> DateClass {
    let class = |kind, two_digit_year| DateClass {
        kind,
        two_digit_year,
    };
    let Some((form, _)) = date_form(s) else {
        return class(DateKind::NotDate, false);
    };
    match form {
        Form::TimeOnly => class(DateKind::NotDate, false),
        Form::Compact => class(DateKind::Compact, false),
        Form::Fixed { written, .. } => class(
            if written {
                DateKind::Written
            } else {
                DateKind::Iso
            },
            false,
        ),
        Form::Slash {
            a,
            b,
            year,
            two_digit,
        } => {
            let reads = |order| {
                let (m, d) = slash_parts(a, b, order);
                date_days(year, m, d)
            };
            let kind = match (reads(DateOrder::Mdy), reads(DateOrder::Dmy)) {
                _ if a > 12 && reads(DateOrder::Mdy).is_some() => DateKind::DayFirstOnly,
                _ if b > 12 && reads(DateOrder::Mdy).is_some() => DateKind::MonthFirstOnly,
                (Some(x), Some(y)) if x == y => DateKind::SameEither,
                (Some(_), Some(_)) => DateKind::Ambiguous,
                _ => DateKind::NotDate,
            };
            class(kind, two_digit && kind != DateKind::NotDate)
        }
    }
}

/// Lower-case `c` and remove common accents (Latin-1 and Latin Extended-A).
pub(crate) fn fold_char(c: char, out: &mut String) {
    let base: &str = match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ð' | 'ď' | 'đ' => "d",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' => "n",
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'þ' => "th",
        'ß' => "ss",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ĳ' => "ij",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ſ' => "s",
        'ţ' | 'ť' | 'ŧ' => "t",
        'ŵ' => "w",
        'ź' | 'ż' | 'ž' => "z",
        _ => {
            out.push(c);
            return;
        }
    };
    out.push_str(base);
}

/// The text's sort key: case and accents folded first; then, separated by a
/// NUL, the details that break ties (accents, and with `match_case` the
/// letter case, lowercase first). The tie part is left out when it equals
/// the first part and case does not matter, which orders the same.
pub fn text_key(s: &str, match_case: bool, out: &mut String) {
    out.clear();
    let mut lower = String::new();
    for c in s.chars() {
        for l in c.to_lowercase() {
            lower.push(l);
            fold_char(l, out);
        }
    }
    if match_case {
        out.push('\0');
        // Swap case so lowercase sorts before uppercase of the same letter.
        for c in s.chars() {
            if c.is_uppercase() {
                out.extend(c.to_lowercase());
            } else if c.is_lowercase() {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
        }
    } else if lower != *out {
        out.push('\0');
        out.push_str(&lower);
    }
}

/// Compare two strings, digit runs by their value (`a2` < `a10`). Ties on
/// value are broken by fewer leading zeros first.
pub fn natural_cmp(a: &[u8], b: &[u8]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let za = si + a[si..i].iter().take_while(|&&c| c == b'0').count();
            let zb = sj + b[sj..j].iter().take_while(|&&c| c == b'0').count();
            let ord = (i - za)
                .cmp(&(j - zb))
                .then_with(|| a[za..i].cmp(&b[zb..j]))
                .then_with(|| (i - si).cmp(&(j - sj)));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            match a[i].cmp(&b[j]) {
                Ordering::Equal => {
                    i += 1;
                    j += 1;
                }
                o => return o,
            }
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> Option<f64> {
        parse_number(s, ReadOptions::default())
    }

    #[test]
    fn numbers() {
        assert_eq!(n("12"), Some(12.0));
        assert_eq!(n(" -3.5 "), Some(-3.5));
        assert_eq!(n("1,234,567.25"), Some(1_234_567.25));
        assert_eq!(n("$1,200"), Some(1200.0));
        assert_eq!(n("-$5"), Some(-5.0));
        assert_eq!(n("(42)"), Some(-42.0));
        assert_eq!(n("12%"), Some(0.12));
        assert_eq!(n("1e3"), Some(1000.0));
        assert_eq!(n(".5"), Some(0.5));
        assert_eq!(n("5 €"), Some(5.0));
        for bad in [
            "", "abc", "1,23", "12,3456", "1.2.3", "inf", "NaN", "--1", "1e", "0x10", "1 2", "-",
        ] {
            assert_eq!(n(bad), None, "{bad}");
        }
        let dc = ReadOptions {
            decimal_comma: true,
            ..Default::default()
        };
        assert_eq!(parse_number("1.234,5", dc), Some(1234.5));
        assert_eq!(parse_number("3,25", dc), Some(3.25));
        assert_eq!(n(&"9".repeat(500)), None);
    }

    fn d(s: &str) -> Option<f64> {
        parse_date(s, ReadOptions::default())
    }

    #[test]
    fn dates() {
        let day = |y, m, dd| days_from_civil(y, m, dd) as f64;
        assert_eq!(day(1970, 1, 1), 0.0);
        assert_eq!(day(2000, 3, 1), 11_017.0);
        assert_eq!(d("2026-11-12"), Some(day(2026, 11, 12)));
        assert_eq!(d("2026/11/12"), Some(day(2026, 11, 12)));
        assert_eq!(d("11/12/2026"), Some(day(2026, 11, 12)));
        assert_eq!(d("13/12/2026"), Some(day(2026, 12, 13)));
        let dmy = ReadOptions {
            date_order: DateOrder::Dmy,
            ..Default::default()
        };
        assert_eq!(parse_date("11/12/2026", dmy), Some(day(2026, 12, 11)));
        assert_eq!(d("12 Nov 2026"), Some(day(2026, 11, 12)));
        assert_eq!(d("Nov 12, 2026"), Some(day(2026, 11, 12)));
        assert_eq!(d("November 12th 2026"), Some(day(2026, 11, 12)));
        assert_eq!(d("12-Nov-26"), Some(day(2026, 11, 12)));
        assert_eq!(d("Thu, 12 Nov 2026"), Some(day(2026, 11, 12)));
        assert_eq!(d("Nov 2026"), Some(day(2026, 11, 1)));
        assert_eq!(d("1/2/85"), Some(day(1985, 1, 2)));
        assert_eq!(d("2026-11-12T06:00:00"), Some(day(2026, 11, 12) + 0.25));
        assert_eq!(d("2026-11-12 08:00+02:00"), Some(day(2026, 11, 12) + 0.25));
        assert_eq!(d("2026-11-12 6:00 am"), Some(day(2026, 11, 12) + 0.25));
        assert_eq!(d("12:00"), Some(0.5));
        assert_eq!(d("12:00 PM"), Some(0.5));
        assert_eq!(d("12:00 AM"), Some(0.0));
        for bad in [
            "é1:00",
            "€:",
            "2026-02-30",
            "2025-02-29",
            "13/13/2026",
            "555-123-4567",
            "hello",
            "12",
            "1.5",
            "25:00",
            "10:61",
            "2026-11-12 10:00 xyz",
            "Nov 40, 2026",
        ] {
            assert_eq!(d(bad), None, "{bad}");
        }
        assert!(d("2024-02-29").is_some());
    }

    fn key(s: &str, case: bool) -> String {
        let mut k = String::new();
        text_key(s, case, &mut k);
        k
    }

    #[test]
    fn text_keys_order() {
        let mut v = vec!["b", "É", "a", "e", "B", "é", "f"];
        v.sort_by_key(|s| key(s, false));
        assert_eq!(v, ["a", "b", "B", "e", "É", "é", "f"]);
        let mut v = vec!["B", "b", "A", "a"];
        v.sort_by_key(|s| key(s, true));
        assert_eq!(v, ["a", "A", "b", "B"]);
        assert_eq!(key("paris", false), key("Paris", false));
        assert_ne!(key("paris", true), key("Paris", true));
    }

    proptest::proptest! {
        /// Any text: no panics, and natural order is a consistent order.
        #[test]
        fn readers_never_panic(a in "\\PC{0,40}", b in "[0-9a-z:/., -]{0,20}", c in "[0-9a-zé:]{0,12}") {
            for s in [&a, &b, &c] {
                let _ = parse_number(s, ReadOptions::default());
                let _ = parse_date(s, ReadOptions { decimal_comma: true, date_order: DateOrder::Dmy });
                let _ = parse_date(s, ReadOptions::default());
                let mut k = String::new();
                text_key(s, true, &mut k);
            }
            let (x, y) = (b.as_bytes(), c.as_bytes());
            proptest::prop_assert_eq!(natural_cmp(x, y), natural_cmp(y, x).reverse());
        }
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "file2", "file1", "file02", "a", "file", "10", "9"];
        v.sort_by(|a, b| natural_cmp(a.as_bytes(), b.as_bytes()));
        assert_eq!(
            v,
            ["9", "10", "a", "file", "file1", "file2", "file02", "file10"]
        );
        let long_a = "x".to_string() + &"9".repeat(300);
        let long_b = "x1".to_string() + &"0".repeat(300);
        assert_eq!(
            natural_cmp(long_a.as_bytes(), long_b.as_bytes()),
            Ordering::Less
        );
    }
}

#[cfg(test)]
mod date_class_tests {
    use super::*;
    use proptest::prelude::*;

    /// `parse_date` as it was before the classifier (0.6.0), to compare with.
    fn parse_date_before(s: &str, opts: ReadOptions) -> Option<f64> {
        let t = s.trim_matches(is_space);
        if t.is_empty() || t.len() > MAX_VALUE_LEN {
            return None;
        }
        // Where a time starts: the digits just before the first ':'.
        let (date_part, time_part) = match t.find(':') {
            Some(c) => {
                // Digits are ASCII, so the digit run's start is a char boundary.
                let start = c - t[..c].bytes().rev().take_while(u8::is_ascii_digit).count();
                (&t[..start], Some(&t[start..]))
            }
            None => (t, None),
        };
        let secs = match time_part {
            Some(tp) => Some(parse_time(tp)?),
            None => None,
        };
        let date_part = date_part.trim_end_matches(|c: char| is_space(c) || c == 'T' || c == ',');
        if date_part.is_empty() {
            return secs.map(|s| s / 86_400.0);
        }
        let lower = date_part.to_ascii_lowercase();
        let mut toks: Vec<&str> = lower
            .split(|c: char| is_space(c) || matches!(c, '-' | '/' | '.' | ','))
            .filter(|x| !x.is_empty())
            .collect();
        if toks.len() > 4 {
            return None;
        }
        if toks.first().is_some_and(|w| is_weekday(w)) {
            toks.remove(0);
        }
        let numeric = |x: &str| x.bytes().all(|c| c.is_ascii_digit());
        let days = match toks.as_slice() {
            [a, b, c] if numeric(a) && numeric(b) && numeric(c) => {
                if a.len() == 4 {
                    date_days(year_of(a)?, b.parse().ok()?, c.parse().ok()?)?
                } else {
                    if a.len() > 2 || b.len() > 2 {
                        return None;
                    }
                    let (x, y): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
                    let year = year_of(c)?;
                    let (m, d) = if x > 12 {
                        (y, x)
                    } else if y > 12 {
                        (x, y)
                    } else {
                        match opts.date_order {
                            DateOrder::Mdy => (x, y),
                            DateOrder::Dmy => (y, x),
                        }
                    };
                    date_days(year, m, d)?
                }
            }
            [a, b, c] => {
                if let Some(m) = month_of(a) {
                    date_days(year_of(c)?, m, day_of(b)?)?
                } else {
                    let m = month_of(b)?;
                    if a.len() == 4 && numeric(a) {
                        date_days(year_of(a)?, m, day_of(c)?)?
                    } else {
                        date_days(year_of(c)?, m, day_of(a)?)?
                    }
                }
            }
            [a, b] => {
                if let (Some(m), true) = (month_of(a), b.len() == 4) {
                    date_days(year_of(b)?, m, 1)?
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        Some(days as f64 + secs.unwrap_or(0.0) / 86_400.0)
    }

    fn same(s: &str) {
        for date_order in [DateOrder::Mdy, DateOrder::Dmy] {
            let opts = ReadOptions {
                date_order,
                ..Default::default()
            };
            assert_eq!(parse_date(s, opts), parse_date_before(s, opts), "{s:?}");
        }
    }

    #[test]
    fn the_table_of_cases() {
        use DateKind::*;
        for (text, kind, two) in [
            ("25/04/2026", DayFirstOnly, false),
            ("04/25/2026", MonthFirstOnly, false),
            ("03/04/2026", Ambiguous, false),
            ("05/05/2026", SameEither, false),
            ("13/13/2026", NotDate, false),
            ("02/30/2026", NotDate, false),
            ("20260425", Compact, false),
            ("20269999", NotDate, false),
            ("Jan 5 2024", Written, false),
            ("5th January 2024", Written, false),
            ("Mon, 5 Jan 2024", Written, false),
            ("4/7/26", Ambiguous, true),
            ("25.04.2026", DayFirstOnly, false),
            ("2026/04/03", Iso, false),
            ("2026-04-03", Iso, false),
            ("2026-04-03T14:00:00Z", Iso, false),
            ("03/04/2026 14:00", Ambiguous, false),
            ("  03-04-2026  ", Ambiguous, false),
            ("14:00", NotDate, false),
            ("123456789", NotDate, false),
            ("", NotDate, false),
            ("hello", NotDate, false),
        ] {
            let c = classify_date(text);
            assert_eq!((c.kind, c.two_digit_year), (kind, two), "{text:?}");
            same(text);
        }
    }

    #[test]
    fn the_two_digit_year_window_is_as_it_says() {
        let read = |s| parse_date(s, ReadOptions::default()).unwrap() as i64;
        assert_eq!(read("1/13/29"), date_days(2029, 1, 13).unwrap());
        assert_eq!(read("1/13/30"), date_days(1930, 1, 13).unwrap());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(6000))]
        #[test]
        fn parse_date_reads_what_it_read_before(s in "\\PC{0,40}") {
            same(&s);
        }

        #[test]
        fn date_like_text_reads_as_before_and_agrees_with_its_class(
            s in "((mon|Tue|friday),? )?([0-9]{1,4}|jan|March|sept\\.)[-/. ,]{0,2}([0-9]{1,2}(st|th)?|Feb|dec)[-/. ,]{0,2}([0-9]{2,4})?([ T][0-9]{1,2}:[0-9]{2}(:[0-9]{2})?( ?(am|PM|Z|\\+02:00))?)?",
        ) {
            same(&s);
            let read = |date_order| parse_date(&s, ReadOptions { date_order, ..Default::default() });
            let (m, d) = (read(DateOrder::Mdy), read(DateOrder::Dmy));
            match classify_date(&s).kind {
                DateKind::Ambiguous => prop_assert!(m.is_some() && d.is_some() && m != d),
                DateKind::SameEither
                | DateKind::MonthFirstOnly
                | DateKind::DayFirstOnly
                | DateKind::Iso
                | DateKind::Written => prop_assert!(m.is_some() && m == d),
                DateKind::Compact => prop_assert!(m.is_none() && d.is_none()),
                DateKind::NotDate => {}
            }
        }
    }
}
