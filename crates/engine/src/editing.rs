//! Everyday editing tools (Milestone 12): the value editor (each distinct
//! value of a column with its count, look-alike groups, and one-step
//! renames and translation lists), group figures in a new column, choosing
//! and ordering columns in one step, and pulling a piece of text (a number,
//! an e-mail address, a ZIP code, a unit number, ...) into its own column.
//!
//! Every tool is one undo step and works on shown rows (column layout
//! changes reach every row, as elsewhere). Values are written as text.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::clean::exact_number;
use crate::document::{Document, Progress};
use crate::ops::{check_cols, CellPlan, ColPlan, Cols, CHUNK};
use crate::reshape::MAX_TABLE_BYTES;
use crate::stats::Kahan;
use crate::value::{fold_char, natural_cmp, ReadOptions};
use crate::Error;

/// Most distinct values the value editor lists.
pub const MAX_LISTED: usize = 100_000;
/// Most old → new pairs one rename or translation list holds.
pub const MAX_PAIRS: usize = 1_000_000;

const TOO_MUCH: &str =
    "this would write too much text at once; change fewer rows or use shorter values";

/// Charge `len` bytes of planned text (plus bookkeeping) against the run's
/// budget, refusing once it is spent.
fn charge(spent: &std::sync::atomic::AtomicUsize, len: usize, budget: usize) -> Result<(), Error> {
    if crate::find::charge_within(spent, len, budget) {
        Ok(())
    } else {
        Err(Error::OutOfMemory(TOO_MUCH))
    }
}

// ---- value editor ----

/// What to list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TallySpec {
    pub col: u32,
    pub keep_first: bool,
    /// Stop once more distinct values than this are found (the result is
    /// then incomplete): a pick list only needs to know "too many".
    pub limit: Option<u32>,
}

/// One distinct value and how many shown rows hold it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub value: String,
    pub count: u64,
}

/// Values that look alike (case, spaces, punctuation, accents), most
/// common first: the first is the suggested spelling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookAlike {
    /// Indexes into [`ValueTally::values`].
    pub members: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueTally {
    /// Most common first (ties in natural order). Blank cells are counted in
    /// `blanks`, not listed.
    pub values: Vec<Tally>,
    pub blanks: u64,
    /// Shown data rows looked at.
    pub rows: u64,
    /// False when the column has more than [`MAX_LISTED`] values (the
    /// counts of the listed ones are still exact).
    pub complete: bool,
    pub look_alikes: Vec<LookAlike>,
}

/// What to group: the values of a column that look alike by sound or spelling.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LookAlikeSpec {
    pub col: u32,
    pub keep_first: bool,
    pub method: crate::lookalike::Method,
    /// Abbreviations: dictionary pairs (variant, canonical) that also group,
    /// at most 100,000.
    pub extra: Vec<(String, String)>,
}

/// The groups found: each group's values with their counts, the suggested
/// spelling (most common, then properly capitalised) first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookAlikes {
    pub groups: Vec<Vec<Tally>>,
    /// Distinct values looked at.
    pub values: u32,
    /// False when the column has more values than the editor lists (only the
    /// most common were looked at).
    pub complete: bool,
    /// Values left out of spelling grouping for being too long.
    pub too_long: u32,
    /// Deleted forms skipped for being shared by too many values.
    pub crowded: u32,
    /// Abbreviations: each group's "maybe" mark (state or country codes in a
    /// column that is mostly not states or countries).
    #[serde(default)]
    pub maybe: Vec<bool>,
}

/// The look-alike key: lower case, accents folded, letters and digits only.
pub fn look_alike_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            fold_char(c, &mut out);
        }
    }
    out
}

// ---- rename / translation list ----

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapValues {
    /// Columns changed.
    pub cols: Vec<u32>,
    /// Old → new; a whole cell must equal the old value.
    pub pairs: Vec<(String, String)>,
    pub match_case: bool,
    /// Ignore spaces at the start and end when comparing.
    pub trim: bool,
    pub keep_first: bool,
}

// ---- group figures ----

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupFunc {
    /// Rows in the group (household size).
    #[default]
    Count,
    Sum,
    Average,
    Min,
    Max,
    First,
    Last,
    /// 1 for the largest value of the group, ties share a rank.
    Rank,
    /// 1 for the smallest value of the group.
    RankUp,
    /// 1, 2, 3 ... in row order within the group.
    Running,
    /// The row's share of the group's total, as a percentage.
    Percent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupCalc {
    /// The group's key columns (empty: the whole table is one group).
    pub keys: Vec<u32>,
    /// The column figured (not needed for count and running number).
    pub value: Option<u32>,
    pub func: GroupFunc,
    pub title: String,
    /// Where the new column goes (logical); `None`: at the end.
    pub at: Option<u32>,
    pub match_case: bool,
    pub trim: bool,
    pub keep_first: bool,
    pub read: ReadOptions,
}

// ---- choose and order columns ----

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Arrange {
    /// The columns kept, in their new order (the rest are deleted).
    pub order: Vec<u32>,
    /// New titles for the kept columns, same order ("" keeps the title).
    pub names: Vec<String>,
    pub keep_first: bool,
}

// ---- extract ----

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractWhat {
    /// The first number.
    #[default]
    Number,
    /// Every number, separated by spaces.
    Numbers,
    Email,
    /// A US ZIP code (12345 or 12345-6789).
    Zip,
    /// Text in round or square brackets (the first).
    Brackets,
    FirstWord,
    LastWord,
    /// A unit, suite or apartment number ("Apt 4B", "Unit 12", "#5").
    Unit,
    /// A phone number (7 to 15 digits with the usual punctuation).
    Phone,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Extract {
    pub col: u32,
    pub what: ExtractWhat,
    /// Title of the new column (with `keep_first`).
    pub title: String,
    /// Also take the piece out of the original cell.
    pub remove: bool,
    pub keep_first: bool,
}

/// What a tool did, for the message afterwards.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditReport {
    pub cells: u64,
    pub groups: u64,
    pub rows: u64,
}

fn norm<'a>(v: &'a str, match_case: bool, trim: bool) -> Cow<'a, str> {
    let v = if trim { v.trim() } else { v };
    if match_case {
        Cow::Borrowed(v)
    } else {
        Cow::Owned(v.to_lowercase())
    }
}

/// The regular expression for a piece of text.
fn extract_regex(what: ExtractWhat) -> Option<Regex> {
    let p = match what {
        // Thousands separators only in groups of three; signs and date-like
        // runs are sorted out by `numbers_in`.
        ExtractWhat::Number | ExtractWhat::Numbers => r"[-+]?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?",
        ExtractWhat::Email => r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
        ExtractWhat::Zip => r"\b\d{5}(?:-\d{4})?\b",
        ExtractWhat::Brackets => r"\(([^()]*)\)|\[([^\[\]]*)\]",
        ExtractWhat::Unit => {
            r"(?i)(?:\b(?:apt|apartment|unit|ste|suite|lot|rm|room|bldg|building|fl|floor)\b\.?|#)\s*#?\s*([A-Za-z]?-?\d[A-Za-z0-9-]*|[A-Za-z]\b)"
        }
        ExtractWhat::Phone => {
            r"(?:\+?\d{1,3}[\s.-]?)?(?:\(\d{3}\)|\d{3})[\s.-]?\d{3}[\s.-]?\d{4}\b"
        }
        ExtractWhat::FirstWord | ExtractWhat::LastWord => return None,
    };
    Some(Regex::new(p).expect("fixed pattern"))
}

/// The cell with `v[a..b]` cut out, and only the separator next to it:
/// one space where two would meet, or the `, ` / ` - ` that led to a piece
/// at the end. Line breaks and spacing elsewhere are kept.
fn cut(v: &str, a: usize, b: usize) -> String {
    let left = &v[..a];
    let right = &v[b..];
    let sep = |c: char| matches!(c, ',' | ';' | '-' | '/' | ':');
    if right.trim().is_empty() {
        // A piece at the end: drop the spaces and one separator before it.
        let l = left.trim_end_matches([' ', '\t']);
        let l = l.strip_suffix(sep).unwrap_or(l);
        return format!(
            "{}{}",
            l.trim_end_matches([' ', '\t']),
            right.trim_start_matches([' ', '\t'])
        );
    }
    if left.trim().is_empty() {
        // At the start: drop the spaces and one separator after it.
        let r = right.trim_start_matches([' ', '\t']);
        let r = r.strip_prefix(sep).unwrap_or(r);
        return format!(
            "{}{}",
            left.trim_end_matches([' ', '\t']),
            r.trim_start_matches([' ', '\t'])
        );
    }
    // Between two separators (`Chattanooga, 37402, USA`): one goes.
    let l = left.trim_end_matches([' ', '\t']);
    let r = right.trim_start_matches([' ', '\t']);
    if l.ends_with(sep) && r.starts_with(sep) {
        return format!("{l}{}", &r[1..]);
    }
    if left.ends_with([' ', '\t']) && right.starts_with([' ', '\t']) {
        return format!("{}{}", left.trim_end_matches([' ', '\t']), right);
    }
    format!("{left}{right}")
}

/// A number found in a cell: the span taken out (with its currency sign
/// and brackets) and the value written plainly (`-5.25` for `-$5.25`,
/// `$-5.25` or `($5.25)`).
struct Found {
    a: usize,
    b: usize,
    value: String,
}

const CURRENCY: [char; 4] = ['$', '€', '£', '¥'];

/// [`numbers_in`] with money signs read the way "Numbers stored as text"
/// reads them: a minus before or after the currency sign, or brackets
/// around an amount with a currency sign, make it negative.
fn amounts_in(v: &str, re: &Regex) -> Vec<Found> {
    numbers_in(v, re)
        .into_iter()
        .map(|(a, b)| {
            let digits = &v[a..b];
            let (signed, digits) = match digits.strip_prefix('-') {
                Some(d) => (true, d),
                None => (false, digits.strip_prefix('+').unwrap_or(digits)),
            };
            let before = &v[..a];
            let mut start = a;
            let mut end = b;
            let mut neg = signed;
            // `$-5.25`: the minus was left out of the span; `$5`, `-$5`.
            let (cur_end, minus_inside) = match before.strip_suffix('-') {
                Some(x) if x.ends_with(CURRENCY) => (x.len(), true),
                _ => (before.len(), false),
            };
            let pre = &v[..cur_end];
            if let Some(c) = pre.chars().next_back().filter(|c| CURRENCY.contains(c)) {
                start = cur_end - c.len_utf8();
                neg |= minus_inside;
                let lead = &v[..start];
                let edge = |t: &str| {
                    t.chars()
                        .next_back()
                        .is_none_or(|c| c.is_whitespace() || "([{".contains(c))
                };
                if let Some(x) = lead.strip_suffix('-').filter(|x| edge(x)) {
                    neg = true;
                    start = x.len();
                } else if lead.ends_with('(') && v[b..].starts_with(')') {
                    neg = true;
                    start -= 1;
                    end += 1;
                }
            }
            let value = if neg {
                format!("-{digits}")
            } else {
                v[a..b].to_string()
            };
            Found {
                a: start,
                b: end,
                value,
            }
        })
        .collect()
}

/// Spans of numbers in `v`: a sign only at the start of the cell or after a
/// space or bracket (never `A-5`, `10-20`); nothing inside a date such as
/// `2026-11-13` or `11/13/26`; not glued to a letter on the right (`5th`
/// is left).
fn numbers_in(v: &str, re: &Regex) -> Vec<(usize, usize)> {
    static DATE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let date =
        DATE.get_or_init(|| Regex::new(r"\d{1,4}[-/.]\d{1,2}[-/.]\d{1,4}").expect("fixed pattern"));
    let dates: Vec<(usize, usize)> = date.find_iter(v).map(|m| (m.start(), m.end())).collect();
    let bytes = v.as_bytes();
    let mut out = Vec::new();
    for m in re.find_iter(v) {
        let (mut a, b) = (m.start(), m.end());
        if matches!(bytes[a], b'-' | b'+')
            && a > 0
            && !matches!(bytes[a - 1], b' ' | b'\t' | b'(' | b'[' | b'{' | b'\n')
        {
            a += 1;
        }
        if dates.iter().any(|&(s, e)| a < e && b > s) {
            continue;
        }
        if v[b..].chars().next().is_some_and(char::is_alphabetic) {
            continue;
        }
        if v[..a]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_digit() || c == '.')
        {
            continue;
        }
        out.push((a, b));
    }
    out
}

/// ZIP codes: the last one in the cell, preferring one after a state code
/// (`TN 37402`) or a comma; a number at the very start followed by more
/// text is a house number, not a ZIP.
fn zip_in(v: &str, re: &Regex) -> Option<(usize, usize)> {
    let all: Vec<(usize, usize)> = re
        .find_iter(v)
        .map(|m| (m.start(), m.end()))
        .filter(|&(a, b)| !(a == v.len() - v.trim_start().len() && !v[b..].trim().is_empty()))
        .collect();
    let after_state = |a: usize| {
        let before = v[..a].trim_end();
        let tail: Vec<char> = before.chars().rev().take(3).collect();
        before.ends_with(',')
            || (tail.len() >= 2
                && tail[0].is_ascii_uppercase()
                && tail[1].is_ascii_uppercase()
                && tail.get(2).is_none_or(|c| !c.is_alphanumeric()))
    };
    all.iter()
        .rev()
        .find(|&&(a, _)| after_state(a))
        .or(all.last())
        .copied()
}

/// The piece found in `v` and the cell without it (`None`: nothing found).
pub fn extract_from(v: &str, what: ExtractWhat, re: Option<&Regex>) -> Option<(String, String)> {
    match what {
        ExtractWhat::FirstWord => {
            let w = v.split_whitespace().next()?;
            let start = v.find(w)?;
            Some((w.to_string(), cut(v, start, start + w.len())))
        }
        ExtractWhat::LastWord => {
            let w = v.split_whitespace().next_back()?;
            let start = v.rfind(w)?;
            Some((w.to_string(), cut(v, start, start + w.len())))
        }
        ExtractWhat::Numbers => {
            let spans = amounts_in(v, re?);
            if spans.is_empty() {
                return None;
            }
            let found: Vec<&str> = spans.iter().map(|f| f.value.as_str()).collect();
            let mut rest = v.to_string();
            for f in spans.iter().rev() {
                rest = cut(&rest, f.a, f.b);
            }
            Some((found.join(" "), rest))
        }
        ExtractWhat::Number => {
            let f = amounts_in(v, re?).into_iter().next()?;
            Some((f.value, cut(v, f.a, f.b)))
        }
        ExtractWhat::Zip => {
            let (a, b) = zip_in(v, re?)?;
            Some((v[a..b].to_string(), cut(v, a, b)))
        }
        ExtractWhat::Phone => {
            // Not part of a longer number or word (`Voter 4235551234567`).
            let m = re?.find_iter(v).find(|m| {
                let t = m.as_str().trim();
                let digits = t.chars().filter(char::is_ascii_digit).count();
                // Without a "+", ten digits (or eleven starting with 1).
                (t.starts_with('+') || digits == 10 || (digits == 11 && t.starts_with('1')))
                    && !v[..m.start()]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_alphanumeric)
                    && !v[m.end()..]
                        .chars()
                        .next()
                        .is_some_and(char::is_alphanumeric)
            })?;
            Some((m.as_str().trim().to_string(), cut(v, m.start(), m.end())))
        }
        _ => {
            let re = re?;
            let c = re.captures(v)?;
            let whole = c.get(0)?;
            let piece = c
                .iter()
                .skip(1)
                .flatten()
                .next()
                .map_or(whole.as_str(), |m| m.as_str());
            Some((piece.trim().to_string(), cut(v, whole.start(), whole.end())))
        }
    }
}

impl Document {
    /// Distinct values of a column among shown data rows, with counts and
    /// look-alike groups.
    pub fn value_tally(&self, spec: &TallySpec, progress: &Progress) -> Result<ValueTally, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[spec.col])?;
        let reader = Cols::new(self, &[spec.col]);
        let rows = self.data_rows(spec.keep_first);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let chunks = ((r1 - r0) as usize).div_ceil(CHUNK);
        let over = AtomicBool::new(false);
        let stop = AtomicBool::new(false);
        let blanks = AtomicU64::new(0);
        let merged: Mutex<HashMap<String, u64>> = Mutex::new(HashMap::new());
        let bytes = AtomicU64::new(0);
        (0..chunks)
            .into_par_iter()
            .try_for_each(|c| -> Result<(), Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut local: HashMap<String, u64> = HashMap::new();
                let mut buf = Vec::new();
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let v = reader.get(&buf, 0);
                    if v.is_empty() {
                        blanks.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    match local.get_mut(v) {
                        Some(n) => *n += 1,
                        None => {
                            local.insert(v.to_string(), 1);
                        }
                    }
                }
                progress.step(e - s)?;
                let mut m = merged.lock().unwrap_or_else(|p| p.into_inner());
                for (k, n) in local {
                    if let Some(x) = m.get_mut(&k) {
                        *x += n;
                    } else if m.len() < MAX_LISTED * 2
                        && bytes.fetch_add(k.len() as u64, Ordering::Relaxed)
                            < MAX_TABLE_BYTES as u64
                    {
                        m.insert(k, n);
                    } else {
                        over.store(true, Ordering::Relaxed);
                    }
                }
                if spec.limit.is_some_and(|l| m.len() > l as usize) {
                    over.store(true, Ordering::Relaxed);
                    stop.store(true, Ordering::Relaxed);
                }
                Ok(())
            })?;
        let mut values: Vec<Tally> = merged
            .into_inner()
            .unwrap_or_else(|p| p.into_inner())
            .into_iter()
            .map(|(value, count)| Tally { value, count })
            .collect();
        values.par_sort_unstable_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| natural_cmp(a.value.as_bytes(), b.value.as_bytes()))
        });
        let mut complete = !over.into_inner();
        if values.len() > MAX_LISTED {
            values.truncate(MAX_LISTED);
            complete = false;
        }
        let mut by_key: HashMap<String, Vec<u32>> = HashMap::new();
        for (i, t) in values.iter().enumerate() {
            let k = look_alike_key(&t.value);
            if !k.is_empty() {
                by_key.entry(k).or_default().push(i as u32);
            }
        }
        let mut look_alikes: Vec<LookAlike> = by_key
            .into_values()
            .filter(|m| m.len() > 1)
            .map(|members| LookAlike { members })
            .collect();
        // Groups in the order of their most common member.
        look_alikes.sort_unstable_by_key(|g| g.members[0]);
        Ok(ValueTally {
            values,
            blanks: blanks.into_inner(),
            rows: r1 - r0,
            complete,
            look_alikes,
        })
    }

    /// Group the look-alikes (by sound, spelling or both) among the values of
    /// a column. Counts the values first (as [`Document::value_tally`]), then
    /// groups the listed ones; `progress` covers both and cancels either.
    pub fn lookalike_groups(
        &self,
        spec: &LookAlikeSpec,
        progress: &Progress,
    ) -> Result<LookAlikes, Error> {
        let tally = self.value_tally(
            &TallySpec {
                col: spec.col,
                keep_first: spec.keep_first,
                limit: None,
            },
            progress,
        )?;
        if spec.extra.len() > crate::dictionary::MAX_DICTIONARY_PAIRS {
            return Err(Error::Invalid(
                "at most 100,000 dictionary pairs can be used".into(),
            ));
        }
        let grouping = crate::lookalike::group_lookalikes_with(
            &tally.values,
            spec.method,
            &spec.extra,
            progress,
        )?;
        Ok(LookAlikes {
            groups: grouping
                .groups
                .iter()
                .map(|g| {
                    g.iter()
                        .map(|&i| tally.values[i as usize].clone())
                        .collect()
                })
                .collect(),
            values: tally.values.len() as u32,
            complete: tally.complete,
            too_long: grouping.too_long,
            crowded: grouping.crowded,
            maybe: grouping.maybe,
        })
    }

    /// Plan replacing whole cells by a list of old → new values (renames,
    /// merged spellings, code translation lists), over shown rows.
    pub fn plan_map_values(
        &self,
        spec: &MapValues,
        progress: &Progress,
    ) -> Result<CellPlan, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &spec.cols)?;
        if spec.cols.is_empty() {
            return Err(Error::Invalid("choose the column(s) to change".into()));
        }
        if spec.pairs.len() > MAX_PAIRS {
            return Err(Error::Invalid(format!(
                "a list can hold at most {MAX_PAIRS} values"
            )));
        }
        let size: usize = spec.pairs.iter().map(|(a, b)| a.len() + b.len()).sum();
        if size > MAX_TABLE_BYTES {
            return Err(Error::Invalid("the list is too long".into()));
        }
        let mut map: HashMap<String, &str> = HashMap::with_capacity(spec.pairs.len());
        for (old, new) in &spec.pairs {
            // The first pair for a value wins.
            map.entry(norm(old, spec.match_case, spec.trim).into_owned())
                .or_insert(new.as_str());
        }
        if map.is_empty() {
            return Err(Error::Invalid("the list has no values".into()));
        }
        let reader = Cols::new(self, &spec.cols);
        let n = spec.cols.len();
        let budget = crate::find::split_budget(self);
        let spent = std::sync::atomic::AtomicUsize::new(0);
        let changes: Vec<(u32, Vec<(u32, String)>)> =
            self.par_rows(self.data_rows(spec.keep_first), progress, |_, id, buf| {
                reader.read(self, id, buf);
                let mut set = Vec::new();
                for k in 0..n {
                    let v = reader.get(buf, k);
                    if let Some(new) = map.get(norm(v, spec.match_case, spec.trim).as_ref()) {
                        if *new != v {
                            charge(&spent, new.len(), budget)?;
                            set.push((spec.cols[k], new.to_string()));
                        }
                    }
                }
                Ok((!set.is_empty()).then_some((id, set)))
            })?;
        let mut plan = CellPlan::new(self);
        plan.cells = changes.iter().map(|(_, c)| c.len() as u64).sum();
        if plan.cells > crate::reshape::MAX_TABLE_CELLS {
            return Err(Error::Invalid(
                "this would change too many cells at once".into(),
            ));
        }
        plan.changes = changes;
        Ok(plan)
    }

    /// Plan a new column with a figure for each row's group (shown rows;
    /// hidden rows get a blank).
    pub fn plan_group_calc(
        &self,
        spec: &GroupCalc,
        progress: &Progress,
    ) -> Result<(ColPlan, EditReport), Error> {
        self.ready_for_bulk()?;
        check_cols(self, &spec.keys)?;
        if spec.keys.len() > crate::reshape::MAX_KEY_COLS {
            return Err(Error::Invalid("too many key columns".into()));
        }
        let needs_value = !matches!(spec.func, GroupFunc::Count | GroupFunc::Running);
        let value = match (needs_value, spec.value) {
            (true, Some(v)) => {
                check_cols(self, &[v])?;
                Some(v)
            }
            (true, None) => return Err(Error::Invalid("choose the column to figure".into())),
            (false, _) => None,
        };
        if self.col_count() >= crate::document::MAX_MAPPED_COLS {
            return Err(Error::OutOfRange);
        }
        let rows = self.data_rows(spec.keep_first);
        let n = (rows.end - rows.start) as usize;
        // Group of each shown row (by offset): the offset of its first row.
        let leader = if spec.keys.is_empty() {
            vec![0u32; n]
        } else {
            self.group_rows(
                rows.clone(),
                &spec.keys,
                spec.match_case,
                spec.trim,
                progress,
            )?
        };
        let reader = value.map(|v| Cols::new(self, &[v]));
        // Per row: its value as a number (NaN: not a number) and its id.
        let read = spec.read;
        let vrd = value.map_or(crate::coltype::Reader::plain(spec.read), |v| {
            self.read_ctx(spec.read).reader(v)
        });
        let vals: Vec<(u32, f64)> = self.par_rows(rows.clone(), progress, |_, id, buf| {
            let x = match &reader {
                Some(rd) => {
                    rd.read(self, id, buf);
                    let v = rd.get(buf, 0);
                    vrd.number(v).unwrap_or(f64::NAN)
                }
                None => f64::NAN,
            };
            Ok(Some((id, x)))
        })?;
        // Group number per leader.
        let mut gid = vec![u32::MAX; n];
        let mut groups = 0u32;
        for i in 0..n {
            let l = leader[i] as usize;
            if gid[l] == u32::MAX {
                gid[l] = groups;
                groups += 1;
            }
            gid[i] = gid[l];
        }
        let g = groups as usize;
        let mut out: Vec<String> = vec![String::new(); n];
        match spec.func {
            GroupFunc::Count | GroupFunc::Running => {
                let mut count = vec![0u64; g];
                for i in 0..n {
                    count[gid[i] as usize] += 1;
                    if spec.func == GroupFunc::Running {
                        out[i] = count[gid[i] as usize].to_string();
                    }
                }
                if spec.func == GroupFunc::Count {
                    for i in 0..n {
                        out[i] = count[gid[i] as usize].to_string();
                    }
                }
            }
            GroupFunc::Sum | GroupFunc::Average | GroupFunc::Percent => {
                let mut sum = vec![Kahan::default(); g];
                let mut cnt = vec![0u64; g];
                for i in 0..n {
                    let x = vals[i].1;
                    if x.is_finite() {
                        sum[gid[i] as usize].add(x);
                        cnt[gid[i] as usize] += 1;
                    }
                }
                for i in 0..n {
                    let k = gid[i] as usize;
                    let s = sum[k].value();
                    let r = match spec.func {
                        GroupFunc::Sum => (cnt[k] > 0).then_some(s),
                        GroupFunc::Average => (cnt[k] > 0).then(|| s / cnt[k] as f64),
                        _ => {
                            let x = vals[i].1;
                            (x.is_finite() && s != 0.0).then(|| x / s * 100.0)
                        }
                    };
                    if let Some(r) = r.filter(|r| r.is_finite()) {
                        out[i] = exact_number(r, read.decimal_comma);
                    }
                }
            }
            GroupFunc::Min | GroupFunc::Max => {
                let mut best = vec![f64::NAN; g];
                for i in 0..n {
                    let x = vals[i].1;
                    if !x.is_finite() {
                        continue;
                    }
                    let b = &mut best[gid[i] as usize];
                    if b.is_nan()
                        || (spec.func == GroupFunc::Min && x < *b)
                        || (spec.func == GroupFunc::Max && x > *b)
                    {
                        *b = x;
                    }
                }
                for i in 0..n {
                    let b = best[gid[i] as usize];
                    if b.is_finite() {
                        out[i] = exact_number(b, read.decimal_comma);
                    }
                }
            }
            GroupFunc::First | GroupFunc::Last => {
                let mut pick = vec![u32::MAX; g];
                for i in 0..n {
                    let p = &mut pick[gid[i] as usize];
                    if spec.func == GroupFunc::Last || *p == u32::MAX {
                        *p = i as u32;
                    }
                }
                let col = value.expect("checked");
                let text: Vec<String> = pick
                    .par_iter()
                    .map(|&i| self.cell_by_id(vals[i as usize].0, col).unwrap_or_default())
                    .collect();
                // Each row gets its own copy of its group's value.
                let budget = crate::find::split_budget(self);
                let spent = std::sync::atomic::AtomicUsize::new(0);
                for i in 0..n {
                    let t = &text[gid[i] as usize];
                    charge(&spent, t.len(), budget)?;
                    out[i] = t.clone();
                }
            }
            GroupFunc::Rank | GroupFunc::RankUp => {
                let mut order: Vec<u32> = (0..n as u32)
                    .filter(|&i| vals[i as usize].1.is_finite())
                    .collect();
                let up = spec.func == GroupFunc::RankUp;
                order.par_sort_unstable_by(|&a, &b| {
                    let (a, b) = (a as usize, b as usize);
                    gid[a].cmp(&gid[b]).then_with(|| {
                        let o = vals[a].1.total_cmp(&vals[b].1);
                        if up {
                            o
                        } else {
                            o.reverse()
                        }
                    })
                });
                let mut k = 0;
                while k < order.len() {
                    let grp = gid[order[k] as usize];
                    let mut pos = 0u64;
                    let mut rank = 0u64;
                    let mut last = f64::NAN;
                    while k < order.len() && gid[order[k] as usize] == grp {
                        pos += 1;
                        let x = vals[order[k] as usize].1;
                        if x != last {
                            rank = pos;
                            last = x;
                        }
                        out[order[k] as usize] = rank.to_string();
                        k += 1;
                    }
                }
            }
        }
        let at = spec.at.unwrap_or(self.col_count()).min(self.col_count());
        let mut plan = ColPlan::new(self);
        plan.insert = Some((at, 1));
        plan.cols = 1;
        if spec.keep_first && self.row_count() > 0 {
            let id = self.id_at(0).ok_or(Error::OutOfRange)?;
            let title = if spec.title.trim().is_empty() {
                "Group figure".to_string()
            } else {
                spec.title.clone()
            };
            plan.changes.push((id, vec![(at, title)]));
        }
        let mut cells = 0u64;
        for (i, v) in out.into_iter().enumerate() {
            if !v.is_empty() {
                cells += 1;
                plan.changes.push((vals[i].0, vec![(at, v)]));
            }
        }
        plan.cells = cells;
        Ok((
            plan,
            EditReport {
                cells,
                groups: groups as u64,
                rows: n as u64,
            },
        ))
    }

    /// Plan keeping these columns in this order (the rest deleted) and
    /// renaming them, as one step.
    pub fn plan_arrange_columns(&self, spec: &Arrange) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &spec.order)?;
        if spec.order.is_empty() {
            return Err(Error::Invalid("keep at least one column".into()));
        }
        let mut seen = vec![false; self.col_count() as usize];
        for &c in &spec.order {
            if std::mem::replace(&mut seen[c as usize], true) {
                return Err(Error::Invalid("a column is listed twice".into()));
            }
        }
        if !spec.names.is_empty() && spec.names.len() != spec.order.len() {
            return Err(Error::Invalid("give one name for each column kept".into()));
        }
        let mut plan = ColPlan::new(self);
        let identity = spec.order.len() as u32 == self.col_count()
            && spec.order.iter().enumerate().all(|(i, &c)| i as u32 == c);
        if !identity {
            plan.order = Some(spec.order.clone());
            plan.cols = self.col_count() - spec.order.len() as u32;
        }
        if spec.keep_first && self.row_count() > 0 {
            let id = self.id_at(0).ok_or(Error::OutOfRange)?;
            let names: Vec<(u32, String)> = spec
                .order
                .iter()
                .zip(&spec.names)
                .filter(|(_, n)| !n.trim().is_empty())
                .filter(|(&c, n)| self.cell_by_id(id, c).as_deref() != Some(n.as_str()))
                .map(|(&c, n)| (c, n.clone()))
                .collect();
            plan.cells = names.len() as u64;
            if !names.is_empty() {
                plan.changes.push((id, names));
            }
        }
        Ok(plan)
    }

    /// Plan pulling a piece of each cell into a new column right after it
    /// (every row, hidden ones too, like a split).
    pub fn plan_extract(&self, spec: &Extract, progress: &Progress) -> Result<ColPlan, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[spec.col])?;
        if self.col_count() >= crate::document::MAX_MAPPED_COLS {
            return Err(Error::OutOfRange);
        }
        let re = extract_regex(spec.what);
        let reader = Cols::new(self, &[spec.col]);
        let first = u64::from(spec.keep_first && self.total_rows() > 0);
        let at = spec.col + 1;
        let spent = std::sync::atomic::AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        let found: Vec<(u32, String, Option<String>)> =
            self.par_all_rows(first, progress, |_, id, buf| {
                reader.read(self, id, buf);
                let v = reader.get(buf, 0);
                let Some((piece, rest)) = extract_from(v, spec.what, re.as_ref()) else {
                    return Ok(None);
                };
                let len = piece.len() + if spec.remove { rest.len() } else { 0 };
                charge(&spent, len, budget)?;
                Ok(Some((
                    id,
                    piece,
                    (spec.remove && rest != v).then_some(rest),
                )))
            })?;
        let mut plan = ColPlan::new(self);
        plan.insert = Some((at, 1));
        plan.cols = 1;
        if first == 1 {
            let id = self.doc_id_at(0).ok_or(Error::OutOfRange)?;
            let title = if spec.title.trim().is_empty() {
                format!("{} part", self.titles(&[spec.col], true)[0])
            } else {
                spec.title.clone()
            };
            plan.changes.push((id, vec![(at, title)]));
        }
        for (id, piece, rest) in found {
            let mut cells = Vec::with_capacity(2);
            if !piece.is_empty() {
                cells.push((at, piece));
            }
            if let Some(r) = rest {
                cells.push((spec.col, r));
            }
            plan.cells += cells.len() as u64;
            if !cells.is_empty() {
                plan.changes.push((id, cells));
            }
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn all(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    #[test]
    fn tally_counts_and_finds_look_alikes() {
        let d = doc("city\r\nRome\r\nrome\r\nRome\r\n\r\nSão Paulo\r\nSao-Paulo\r\nOslo\r\n");
        let t = d
            .value_tally(
                &TallySpec {
                    col: 0,
                    keep_first: true,
                    limit: None,
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(
            t.values[0],
            Tally {
                value: "Rome".into(),
                count: 2
            }
        );
        assert_eq!(t.blanks, 1);
        assert_eq!(t.rows, 7);
        assert!(t.complete);
        assert_eq!(t.look_alikes.len(), 2);
        // With a limit, it stops past that many values and says so.
        let capped = d
            .value_tally(
                &TallySpec {
                    col: 0,
                    keep_first: true,
                    limit: Some(2),
                },
                &Progress::new(),
            )
            .unwrap();
        assert!(!capped.complete);
        let names: Vec<Vec<&str>> = t
            .look_alikes
            .iter()
            .map(|g| {
                g.members
                    .iter()
                    .map(|&i| t.values[i as usize].value.as_str())
                    .collect()
            })
            .collect();
        assert_eq!(names[0], vec!["Rome", "rome"]);
        assert!(names[1].contains(&"São Paulo") && names[1].contains(&"Sao-Paulo"));
    }

    #[test]
    fn map_values_renames_whole_cells() {
        let mut d = doc("code,x\r\nNY,NY\r\nny ,b\r\nCA,c\r\nNYC,d\r\n");
        let spec = MapValues {
            cols: vec![0],
            pairs: vec![
                ("ny".into(), "New York".into()),
                ("ca".into(), "California".into()),
                ("ny".into(), "ignored".into()),
            ],
            trim: true,
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_map_values(&spec, &Progress::new()).unwrap();
        assert_eq!(plan.cells, 3);
        d.apply_cells(plan).unwrap();
        let col: Vec<String> = all(&d).iter().map(|r| r[0].clone()).collect();
        assert_eq!(
            col,
            vec!["code", "New York", "New York", "California", "NYC"]
        );
        assert_eq!(d.row_values(1).unwrap()[1], "NY");
        assert!(d.undo());
        assert_eq!(d.row_values(2).unwrap()[0], "ny ");
        let none = MapValues {
            pairs: vec![],
            ..spec
        };
        assert!(d.plan_map_values(&none, &Progress::new()).is_err());
    }

    #[test]
    fn group_figures() {
        let base = "house,age\r\nA,30\r\nB,5\r\nA,40\r\nA,\r\nB,7\r\n";
        let run = |func: GroupFunc| {
            let mut d = doc(base);
            let spec = GroupCalc {
                keys: vec![0],
                value: Some(1),
                func,
                title: "f".into(),
                keep_first: true,
                ..Default::default()
            };
            let (plan, rep) = d.plan_group_calc(&spec, &Progress::new()).unwrap();
            assert_eq!(rep.groups, 2);
            d.apply_cols(plan).unwrap();
            all(&d)
                .iter()
                .map(|r| r.get(2).cloned().unwrap_or_default())
                .collect::<Vec<_>>()
        };
        assert_eq!(run(GroupFunc::Count), vec!["f", "3", "2", "3", "3", "2"]);
        assert_eq!(run(GroupFunc::Running), vec!["f", "1", "1", "2", "3", "2"]);
        assert_eq!(run(GroupFunc::Sum), vec!["f", "70", "12", "70", "70", "12"]);
        assert_eq!(
            run(GroupFunc::Average),
            vec!["f", "35", "6", "35", "35", "6"]
        );
        assert_eq!(run(GroupFunc::Min), vec!["f", "30", "5", "30", "30", "5"]);
        assert_eq!(run(GroupFunc::Max), vec!["f", "40", "7", "40", "40", "7"]);
        assert_eq!(run(GroupFunc::First), vec!["f", "30", "5", "30", "30", "5"]);
        assert_eq!(run(GroupFunc::Last), vec!["f", "", "7", "", "", "7"]);
        assert_eq!(run(GroupFunc::Rank), vec!["f", "2", "2", "1", "", "1"]);
        assert_eq!(run(GroupFunc::RankUp), vec!["f", "1", "1", "2", "", "2"]);
        let pct = run(GroupFunc::Percent);
        assert_eq!(pct[2], "41.6666666666667");
        // No keys: one group; count needs no value column.
        let mut d = doc(base);
        let spec = GroupCalc {
            func: GroupFunc::Count,
            keep_first: true,
            at: Some(0),
            ..Default::default()
        };
        let (plan, _) = d.plan_group_calc(&spec, &Progress::new()).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(all(&d)[1], vec!["5", "A", "30"]);
        assert!(d.undo());
        assert_eq!(d.col_count(), 2);
        let bad = GroupCalc {
            func: GroupFunc::Sum,
            ..Default::default()
        };
        assert!(d.plan_group_calc(&bad, &Progress::new()).is_err());
    }

    #[test]
    fn arrange_keeps_orders_and_renames() {
        let mut d = doc("a,b,c,d\r\n1,2,3,4\r\n");
        let spec = Arrange {
            order: vec![2, 0],
            names: vec!["C".into(), String::new()],
            keep_first: true,
        };
        let plan = d.plan_arrange_columns(&spec).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(all(&d), vec![vec!["C", "a"], vec!["3", "1"]]);
        let mut out = Vec::new();
        d.write_copy(&mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "C,a\r\n3,1\r\n");
        assert!(d.undo());
        assert_eq!(all(&d)[0], vec!["a", "b", "c", "d"]);
        assert!(d
            .plan_arrange_columns(&Arrange {
                order: vec![],
                ..spec.clone()
            })
            .is_err());
        assert!(d
            .plan_arrange_columns(&Arrange {
                order: vec![1, 1],
                names: vec![],
                keep_first: true
            })
            .is_err());
    }

    /// Big values copied into many rows are refused early.
    #[test]
    fn big_copies_are_refused() {
        let big = "y".repeat(1 << 20);
        let mut body = String::from("k,v\r\n");
        for _ in 0..50 {
            body.push_str("a,x\r\n");
        }
        let d = doc(&body);
        let spec = MapValues {
            cols: vec![1],
            pairs: vec![("x".into(), big.clone())],
            keep_first: true,
            ..Default::default()
        };
        assert!(matches!(
            d.plan_map_values(&spec, &Progress::new()),
            Err(Error::OutOfMemory(_))
        ));
        let mut body = format!("k,v\r\na,{big}\r\n");
        for _ in 0..50 {
            body.push_str("a,\r\n");
        }
        let d = doc(&body);
        let spec = GroupCalc {
            keys: vec![0],
            value: Some(1),
            func: GroupFunc::First,
            keep_first: true,
            ..Default::default()
        };
        assert!(matches!(
            d.plan_group_calc(&spec, &Progress::new()),
            Err(Error::OutOfMemory(_))
        ));
    }

    /// Value tools never touch hidden rows; a save reads back the same and
    /// undo gives back the file.
    #[test]
    fn tools_respect_filters_save_and_undo() {
        use crate::filter::{ColumnFilter, FilterSpec, Rule};
        let src = "k,v,w\r\na,1,x 12\r\nb,2,y\r\na,3,z 7\r\nb,4,\r\n";
        let mut d = doc(src);
        d.set_filter(&FilterSpec {
            columns: vec![ColumnFilter {
                enabled: true,
                negate: false,
                name: None,
                col: 1,
                rule: Rule::Values {
                    values: vec!["1".into(), "2".into(), "3".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        let p = Progress::new();
        let plan = d
            .plan_map_values(
                &MapValues {
                    cols: vec![0],
                    pairs: vec![("b".into(), "B".into())],
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        d.apply_cells(plan).unwrap();
        let (plan, _) = d
            .plan_group_calc(
                &GroupCalc {
                    keys: vec![0],
                    func: GroupFunc::Count,
                    keep_first: true,
                    title: "n".into(),
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        d.apply_cols(plan).unwrap();
        let plan = d
            .plan_extract(
                &Extract {
                    col: 2,
                    what: ExtractWhat::Number,
                    keep_first: true,
                    remove: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        d.apply_cols(plan).unwrap();
        d.clear_filter();
        assert_eq!(
            all(&d),
            vec![
                vec!["k", "v", "w", "w part", "n"],
                vec!["a", "1", "x", "12", "2"],
                vec!["B", "2", "y", "", "1"],
                vec!["a", "3", "z", "7", "2"],
                vec!["b", "4", "", "", ""],
            ]
        );
        let mut out = Vec::new();
        d.write_copy(&mut out).unwrap();
        let back = doc(&String::from_utf8(out).unwrap());
        assert_eq!(all(&back)[1], vec!["a", "1", "x", "12", "2"]);
        while d.undo() {}
        assert_eq!(all(&d), all(&doc(src)));
    }

    #[test]
    fn extract_pieces() {
        let cases = [
            (
                ExtractWhat::Number,
                "Order 1,250.50 due",
                "1,250.50",
                "Order due",
            ),
            (ExtractWhat::Numbers, "a1 b22 c", "1 22", "a b c"),
            (
                ExtractWhat::Email,
                "Ann <ann.lee@mail.example.org>",
                "ann.lee@mail.example.org",
                "Ann <>",
            ),
            (
                ExtractWhat::Zip,
                "Chattanooga TN 37402-1234",
                "37402-1234",
                "Chattanooga TN",
            ),
            (
                ExtractWhat::Brackets,
                "Widget (blue) [x]",
                "blue",
                "Widget [x]",
            ),
            (ExtractWhat::FirstWord, "Mary Ann Lee", "Mary", "Ann Lee"),
            (ExtractWhat::LastWord, "Mary Ann Lee", "Lee", "Mary Ann"),
            (ExtractWhat::Unit, "12 Main St Apt 4B", "4B", "12 Main St"),
            (ExtractWhat::Unit, "12 Main St, #5", "5", "12 Main St"),
            (
                ExtractWhat::Unit,
                "400 Suite Rd Ste. 200",
                "200",
                "400 Suite Rd",
            ),
            (
                ExtractWhat::Phone,
                "call (423) 555-0123 now",
                "(423) 555-0123",
                "call now",
            ),
            // Review M1: the ZIP, not the house number.
            (
                ExtractWhat::Zip,
                "12345 Main St, Chattanooga TN 37402",
                "37402",
                "12345 Main St, Chattanooga TN",
            ),
            (ExtractWhat::Zip, "37402", "37402", ""),
            (
                ExtractWhat::Zip,
                "PO Box 55555, 37402",
                "37402",
                "PO Box 55555",
            ),
            // Review M2: hyphens are not minus signs; dates are not numbers;
            // commas group only in threes.
            (ExtractWhat::Number, "Unit A-5", "5", "Unit A"),
            (ExtractWhat::Number, "-5 degrees", "-5", "degrees"),
            (ExtractWhat::Number, "(-12.5)", "-12.5", "()"),
            (ExtractWhat::Numbers, "pages 10-20", "10 20", "pages"),
            (ExtractWhat::Numbers, "items 1,2,3", "1 2 3", "items"),
            (
                ExtractWhat::Number,
                "due 2026-11-13 or 7",
                "7",
                "due 2026-11-13 or",
            ),
            (
                ExtractWhat::Number,
                "1,234,567 people",
                "1,234,567",
                "people",
            ),
            // Review L5: only the piece and its separator go; line breaks stay.
            (
                ExtractWhat::LastWord,
                "a  b
c d",
                "d",
                "a  b
c",
            ),
            (ExtractWhat::Number, "Smith, 42", "42", "Smith"),
            // Review round 2, R1: money signs.
            (ExtractWhat::Number, "owed $-5.25", "-5.25", "owed"),
            (ExtractWhat::Number, "owed -$5.25", "-5.25", "owed"),
            (ExtractWhat::Number, "owed ($5.00) now", "-5.00", "owed now"),
            (ExtractWhat::Number, "fee -€3", "-3", "fee"),
            (ExtractWhat::Number, "paid $12 cash", "12", "paid cash"),
            (ExtractWhat::Number, "Unit A-$5", "5", "Unit A"),
            // R2: one separator goes when the piece sat between two.
            (
                ExtractWhat::Zip,
                "Chattanooga, 37402, USA",
                "37402",
                "Chattanooga, USA",
            ),
            // Review L8: not inside a longer number.
            (
                ExtractWhat::Phone,
                "Voter 4235551234567 id, cell 423-555-0199",
                "423-555-0199",
                "Voter 4235551234567 id, cell",
            ),
        ];
        for (what, v, piece, rest) in cases {
            let re = extract_regex(what);
            let got = extract_from(v, what, re.as_ref()).unwrap_or_else(|| panic!("{v}"));
            assert_eq!(
                (got.0.as_str(), got.1.as_str()),
                (piece, rest),
                "{what:?} {v}"
            );
        }
        assert!(extract_from(
            "no digits",
            ExtractWhat::Number,
            extract_regex(ExtractWhat::Number).as_ref()
        )
        .is_none());
        let mut d = doc("addr\r\n12 Main St Apt 4B\r\n9 Elm\r\n");
        let spec = Extract {
            col: 0,
            what: ExtractWhat::Unit,
            remove: true,
            keep_first: true,
            ..Default::default()
        };
        let plan = d.plan_extract(&spec, &Progress::new()).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(
            all(&d),
            vec![
                vec!["addr", "addr part"],
                vec!["12 Main St", "4B"],
                vec!["9 Elm"]
            ]
        );
        assert!(d.undo());
        assert_eq!(all(&d)[1], vec!["12 Main St Apt 4B"]);
    }
}
