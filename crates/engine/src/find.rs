//! Find and replace: plain text, wildcards and regular expressions over the
//! shown rows of a document, a selection, some columns or everything.
//!
//! Patterns come from the user and are untrusted. The `regex` crate never
//! backtracks (time is linear in the text searched), compiled patterns and
//! their lazy DFA caches are capped, and replace-all bounds the text it
//! creates, so no pattern can hang the app or exhaust memory.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::value::ReadOptions;
use crate::Error;

/// Longest pattern, in characters.
pub const MAX_PATTERN_CHARS: usize = 10_000;
/// Longest replacement text, in characters.
pub const MAX_REPLACEMENT_CHARS: usize = 65_536;
/// Largest cell a replacement may produce, in bytes.
pub const MAX_RESULT_CELL_BYTES: usize = 32 << 20;
/// Most new text one replace-all may create, in bytes.
pub const MAX_REPLACE_BYTES: usize = 1 << 30;
/// What one changed cell costs in a plan besides its text (its place in the
/// change list, the string's header, its row entry), counted against the
/// same budget so a run of many tiny changes is bounded too.
pub(crate) const CHANGE_COST: usize = 64;
/// The memory one bulk change (replace-all, a cleaner run, fill by example)
/// may plan: [`MAX_REPLACE_BYTES`], smaller under test so tests can reach it.
#[cfg(not(test))]
pub(crate) const RUN_BUDGET: usize = MAX_REPLACE_BYTES;
#[cfg(test)]
pub(crate) const RUN_BUDGET: usize = 4 << 20;

/// Count a planned change of `len` bytes; `false` once the budget is spent.
pub(crate) fn charge(spent: &AtomicUsize, len: usize) -> bool {
    charge_within(spent, len, RUN_BUDGET)
}

/// [`charge`] against another budget.
pub(crate) fn charge_within(spent: &AtomicUsize, len: usize, budget: usize) -> bool {
    let cost = len + CHANGE_COST;
    spent.fetch_add(cost, Ordering::Relaxed) + cost <= budget
}

/// The budget for a run that adds columns (split column, split names): the
/// run budget, or the file's own size when that is larger. A split's new
/// text comes from the file, so a big file may fairly need more than the
/// run budget (10M rows split in two is about 1.4 GB of plan), while a
/// small file still cannot blow up past it.
/// It never goes past [`MAX_SPLIT_BUDGET`], however big the file.
pub(crate) fn split_budget(d: &crate::Document) -> usize {
    RUN_BUDGET.max(d.source_len()).min(MAX_SPLIT_BUDGET)
}
/// Most memory a split may plan, for any file size.
pub(crate) const MAX_SPLIT_BUDGET: usize = 4 << 30;
/// Compiled pattern size and lazy-DFA cache limits.
const REGEX_SIZE_LIMIT: usize = 10 << 20;
const REGEX_DFA_LIMIT: usize = 32 << 20;
/// Characters of context kept around a hit in result lists.
pub const SNIPPET_CHARS: usize = 80;
/// Rows searched per parallel task.
const BLOCK: u64 = 2048;

/// How the pattern text is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    /// The text itself.
    #[default]
    Plain,
    /// `*` any run of characters, `?` one character, `~*`, `~?`, `~~` the
    /// characters themselves (spreadsheet style).
    Wildcard,
    /// A regular expression; the replacement may use `$1`, `${name}`, `$$`.
    Regex,
    /// Cells holding a number by its value: `5` finds 5, 5.00, $5;
    /// `10..20`, `>5`, `>=5`, `<5`, `<=5`. The whole cell is replaced.
    Number,
    /// Cells holding a date by its value: `1/5/2024` finds 2024-01-05 and
    /// Jan 5 2024; ranges and comparisons as for numbers.
    Date,
    /// Whole cells spelled nearly the same (a letter or two different,
    /// case, accents and punctuation aside): "Jon Smyth" finds "John Smith".
    Similar,
}

/// What to look for.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Pattern {
    pub text: String,
    pub kind: PatternKind,
    pub match_case: bool,
    /// The whole cell must match, not just part of it.
    pub whole_cell: bool,
    /// Only whole words match: "Ann" finds "Ann Lee", not "Annette".
    pub whole_word: bool,
    /// Letters match their accented forms: "Jose" finds "José".
    pub ignore_accents: bool,
    /// In plain and wildcard patterns, `\t` `\n` `\r` `\\` and `\u{..}`
    /// stand for a tab, a line break, a carriage return, a backslash and
    /// any character (also in the replacement).
    pub escapes: bool,
    /// A replacement takes the case of what it replaces: "smith" → "jones"
    /// turns SMITH into JONES and Smith into Jones.
    pub preserve_case: bool,
    /// How numbers and dates are read (Number and Date patterns).
    pub read: ReadOptions,
}

/// A test on the whole cell (Number, Date and Similar patterns).
#[derive(Debug, Clone)]
enum Test {
    /// A number (or a date as days) between two bounds (value, inclusive).
    Value {
        date: bool,
        lo: Option<(f64, bool)>,
        hi: Option<(f64, bool)>,
        read: ReadOptions,
    },
    /// Spelled within `max` letters of `want` (both normalized).
    Similar { want: Vec<char>, max: usize },
}

impl Test {
    fn new(p: &Pattern) -> Result<Option<Test>, Error> {
        let date = match p.kind {
            PatternKind::Number => false,
            PatternKind::Date => true,
            PatternKind::Similar => {
                let want: Vec<char> = similar_key(&p.text).chars().collect();
                if want.is_empty() {
                    return Err(Error::BadPattern("enter a name or word to look for".into()));
                }
                // One or two letters ("A", a middle initial) must match
                // exactly: one letter off would match nearly anything.
                let max = match want.len() {
                    0..=2 => 0,
                    3..=4 => 1,
                    5..=10 => 2,
                    _ => 3,
                };
                return Ok(Some(Test::Similar { want, max }));
            }
            _ => return Ok(None),
        };
        let read = p.read;
        let what = if date { "date" } else { "number" };
        // A value, and whether it is a whole day (a date with no time).
        let val = |s: &str| -> Result<(f64, bool), Error> {
            let s = s.trim();
            let v = if date {
                crate::value::parse_date(s, read)
            } else {
                crate::value::parse_number(s, read)
            };
            v.map(|v| (v, date && v.fract() == 0.0)).ok_or_else(|| {
                Error::BadPattern(if s.is_empty() {
                    format!("enter a {what}, a range such as 10..20, or >5")
                } else if date {
                    format!("\"{s}\" is not a date (try 1/5/2024 or 2024-01-05)")
                } else {
                    format!("\"{s}\" is not a number")
                })
            })
        };
        let t = p.text.trim();
        let (lo, hi) = if let Some(r) = t.strip_prefix(">=") {
            (Some((val(r)?.0, true)), None)
        } else if let Some(r) = t.strip_prefix("<=") {
            let (v, day) = val(r)?;
            (None, Some(if day { (v + 1.0, false) } else { (v, true) }))
        } else if let Some(r) = t.strip_prefix('>') {
            let (v, day) = val(r)?;
            (Some(if day { (v + 1.0, true) } else { (v, false) }), None)
        } else if let Some(r) = t.strip_prefix('<') {
            (None, Some((val(r)?.0, false)))
        } else if let Some((a, b)) = t.split_once("..") {
            let (a, _) = val(a)?;
            let (b, day) = val(b)?;
            if b < a {
                return Err(Error::BadPattern(format!(
                    "the range ends before it starts: write the smaller {what} first"
                )));
            }
            (
                Some((a, true)),
                Some(if day { (b + 1.0, false) } else { (b, true) }),
            )
        } else {
            let (v, day) = val(t)?;
            if day {
                (Some((v, true)), Some((v + 1.0, false)))
            } else {
                (Some((v, true)), Some((v, true)))
            }
        };
        Ok(Some(Test::Value { date, lo, hi, read }))
    }

    fn matches(&self, s: &str) -> bool {
        match self {
            Test::Value { date, lo, hi, read } => {
                let v = if *date {
                    crate::value::parse_date(s, *read)
                } else {
                    crate::value::parse_number(s, *read)
                };
                let Some(v) = v else { return false };
                lo.is_none_or(|(l, inc)| if inc { v >= l } else { v > l })
                    && hi.is_none_or(|(h, inc)| if inc { v <= h } else { v < h })
            }
            Test::Similar { want, max } => {
                // Cheap checks first: most cells differ a lot in length.
                let n = s.chars().count();
                if n + max < want.len() || n > (want.len() + max) * 3 + 8 {
                    return false;
                }
                let got: Vec<char> = similar_key(s).chars().collect();
                // A blank cell (or only punctuation) is never similar.
                !got.is_empty() && within(want, &got, *max)
            }
        }
    }
}

/// Text for "similar" matching: accents and case dropped, anything that is
/// not a letter or digit one space, trimmed.
fn similar_key(s: &str) -> String {
    let folded = crate::clean::fold_accents(s).to_lowercase();
    let mut out = String::with_capacity(folded.len());
    let mut space = false;
    for c in folded.chars() {
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else {
            space = true;
        }
    }
    out
}

/// `a` and `b` are at most `max` edits apart (insert, delete, change).
fn within(a: &[char], b: &[char], max: usize) -> bool {
    if a.len().abs_diff(b.len()) > max {
        return false;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &x) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut best = cur[0];
        for (j, &y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
            best = best.min(cur[j + 1]);
        }
        if best > max {
            return false;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()] <= max
}

/// A replacement in the case of the text it replaces: all capitals, all
/// small letters, or a capital first; otherwise as typed.
fn keep_case(found: &str, rep: &str) -> String {
    let letters: Vec<char> = found.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() || rep.is_empty() {
        return rep.to_string();
    }
    let upper = letters.iter().all(|c| c.is_uppercase());
    if upper && letters.len() > 1 {
        return rep.to_uppercase();
    }
    if letters.iter().all(|c| c.is_lowercase()) {
        return rep.to_lowercase();
    }
    if letters[0].is_uppercase() && letters[1..].iter().all(|c| c.is_lowercase()) {
        let mut cs = rep.chars();
        let first = cs.next().expect("not empty");
        return first.to_uppercase().collect::<String>() + cs.as_str();
    }
    rep.to_string()
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Matcher {
    re: Regex,
    /// Replacement text is taken literally (plain and wildcard patterns).
    literal: bool,
    /// `\n`, `\t` ... in the replacement stand for those characters.
    rep_escapes: bool,
    /// Number, Date and Similar patterns: a test on the whole cell.
    test: Option<Test>,
    /// The replacement takes the case of what it replaces.
    preserve: bool,
}

/// One literal character of a pattern as regex source: escaped, or with
/// "ignore accents" a class of the letter and its accented forms.
fn lit(c: char, accents: bool) -> String {
    if accents {
        if let Some(class) = accent_class(c) {
            return class;
        }
    }
    regex::escape(c.encode_utf8(&mut [0; 4]))
}

fn wildcard_to_regex(p: &str, accents: bool) -> String {
    let mut out = String::with_capacity(p.len() * 2);
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '~' if matches!(chars.peek(), Some('*' | '?' | '~')) => {
                let n = chars.next().expect("peeked");
                out.push_str(&lit(n, accents));
            }
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            _ => out.push_str(&lit(c, accents)),
        }
    }
    out
}

/// The single plain letter an accented letter stands for (é → e, Ø → O),
/// keeping its case; `None` for anything else.
fn base_letter(c: char) -> Option<char> {
    if c.is_ascii_alphabetic() {
        return Some(c);
    }
    if !c.is_alphabetic() || c.is_ascii() {
        return None;
    }
    let mut lower = c.to_lowercase();
    let l = match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => return None,
    };
    let mut tmp = String::new();
    crate::value::fold_char(l, &mut tmp);
    let mut it = tmp.chars();
    match (it.next(), it.next()) {
        (Some(b), None) if b.is_ascii_alphabetic() => Some(if c.is_uppercase() {
            b.to_ascii_uppercase()
        } else {
            b
        }),
        _ => None,
    }
}

/// Accented forms of each plain letter a-z (lower case), built once from
/// the Latin letter blocks.
fn accent_table() -> &'static [Vec<char>; 26] {
    static TABLE: std::sync::OnceLock<[Vec<char>; 26]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t: [Vec<char>; 26] = Default::default();
        let ranges = [0x00C0u32..=0x024F, 0x1E00..=0x1EFF];
        for r in ranges {
            for u in r {
                let Some(c) = char::from_u32(u) else { continue };
                if !c.is_lowercase() {
                    continue;
                }
                if let Some(b) = base_letter(c) {
                    if b.is_ascii_lowercase() && b != c {
                        t[(b as u8 - b'a') as usize].push(c);
                    }
                }
            }
        }
        t
    })
}

/// `[eéèêë…]\p{M}*` for a letter (any of its forms in the pattern), in the
/// pattern letter's case; combining marks after it are allowed too, so
/// "e" + a separate accent mark matches as well.
fn accent_class(c: char) -> Option<String> {
    let b = base_letter(c)?;
    let lower = b.to_ascii_lowercase();
    let forms = &accent_table()[(lower as u8 - b'a') as usize];
    let mut class = String::from("(?:[");
    if b.is_ascii_uppercase() {
        class.push(b);
        for f in forms {
            class.extend(f.to_uppercase());
        }
    } else {
        class.push(b);
        class.extend(forms.iter());
    }
    class.push_str("]\\p{M}*)");
    Some(class)
}

/// Literal letters of a regular expression made accent-blind; escapes,
/// classes `[...]`, repeat counts `{...}` and group headers stay as they are.
fn regex_ignore_accents(src: &str) -> String {
    let mut out = String::with_capacity(src.len() * 4);
    let mut chars = src.chars().peekable();
    let mut class_depth = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                    // \p{..}, \x{..}, \u{..}, \b{..}: copy the braces too.
                    if matches!(n, 'p' | 'P' | 'x' | 'u' | 'U' | 'b' | 'B')
                        && chars.peek() == Some(&'{')
                    {
                        for m in chars.by_ref() {
                            out.push(m);
                            if m == '}' {
                                break;
                            }
                        }
                    } else if matches!(n, 'p' | 'P') {
                        // The one-letter form, \pL: the class letter too.
                        if let Some(m) = chars.next() {
                            out.push(m);
                        }
                    }
                }
            }
            '[' => {
                class_depth += 1;
                out.push(c);
                // A "]" first in a class ("[]a]", "[^]a]") is a literal.
                if chars.peek() == Some(&'^') {
                    out.push(chars.next().expect("peeked"));
                }
                if chars.peek() == Some(&']') {
                    out.push(chars.next().expect("peeked"));
                }
            }
            ']' if class_depth > 0 => {
                class_depth -= 1;
                out.push(c);
            }
            _ if class_depth > 0 => out.push(c),
            '{' => {
                out.push(c);
                for m in chars.by_ref() {
                    out.push(m);
                    if m == '}' {
                        break;
                    }
                }
            }
            '(' if chars.peek() == Some(&'?') => {
                // (?i) (?:  (?P<name>  (?<name>: copy up to the end of the header.
                out.push(c);
                for m in chars.by_ref() {
                    out.push(m);
                    if m == ':' || m == ')' || m == '>' {
                        break;
                    }
                }
            }
            _ if base_letter(c).is_some() => out.push_str(&accent_class(c).expect("a letter")),
            _ => out.push(c),
        }
    }
    out
}

/// A regular expression replacement split at its case marks: `\U` upper
/// case and `\L` lower case until `\E`, `\u` and `\l` for the next letter
/// only (as in Notepad++ and VS Code), applied to the expanded text so a
/// group's value is never read as marks.
struct CaseTemplate {
    parts: Vec<(Case, Option<bool>, String)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Case {
    Keep,
    Upper,
    Lower,
}

impl CaseTemplate {
    /// Marks are read in the replacement as typed, before `\t` and the
    /// like become characters (`escapes`), so `\\U` stays a literal
    /// backslash and U.
    fn new(rep: &str, escapes: bool) -> Self {
        let text = |s: &str| -> String {
            if escapes {
                unescape(s).into_owned()
            } else {
                s.to_string()
            }
        };
        let mut parts = Vec::new();
        let (mut case, mut once) = (Case::Keep, None);
        let mut start = 0;
        let b = rep.as_bytes();
        let mut i = 0;
        while i + 1 < b.len() {
            if b[i] == b'\\' && b[i + 1] == b'\\' {
                // An escaped backslash: never the start of a mark.
                i += 2;
            } else if b[i] == b'\\' && matches!(b[i + 1], b'U' | b'L' | b'E' | b'u' | b'l') {
                if start < i || once.is_some() {
                    parts.push((case, once.take(), text(&rep[start..i])));
                }
                match b[i + 1] {
                    b'U' => case = Case::Upper,
                    b'L' => case = Case::Lower,
                    b'E' => case = Case::Keep,
                    b'u' => once = Some(true),
                    _ => once = Some(false),
                }
                i += 2;
                start = i;
            } else {
                i += 1;
            }
        }
        parts.push((case, once, text(&rep[start..])));
        CaseTemplate { parts }
    }

    /// The replacement uses case marks.
    fn has_marks(&self) -> bool {
        self.parts.len() > 1
            || self
                .parts
                .iter()
                .any(|p| p.0 != Case::Keep || p.1.is_some())
    }

    fn expand(&self, caps: &regex::Captures<'_>, out: &mut String) {
        if self.parts.len() == 1 && self.parts[0].0 == Case::Keep && self.parts[0].1.is_none() {
            caps.expand(&self.parts[0].2, out);
            return;
        }
        let mut pending: Option<bool> = None;
        let mut piece = String::new();
        for (case, once, text) in &self.parts {
            if once.is_some() {
                pending = *once;
            }
            piece.clear();
            caps.expand(text, &mut piece);
            let mut cased = match case {
                Case::Keep => piece.clone(),
                Case::Upper => piece.to_uppercase(),
                Case::Lower => piece.to_lowercase(),
            };
            if let Some(up) = pending {
                if let Some(first) = cased.chars().next() {
                    let rest = cased[first.len_utf8()..].to_string();
                    cased = if up {
                        first.to_uppercase().collect::<String>() + &rest
                    } else {
                        first.to_lowercase().collect::<String>() + &rest
                    };
                    pending = None;
                }
            }
            out.push_str(&cased);
        }
    }
}

/// `\t` `\n` `\r` `\\` `\u{XXXX}` in text as the characters they name;
/// any other backslash stays as it is.
/// `s` lower-cased letter by letter (the same for a pair and a cell). With
/// `map`, also where each byte of the result came from in `s`: the start of
/// a letter's lower-case form maps to that letter, the rest to `usize::MAX`,
/// and the end to `s.len()`.
fn fold(s: &str, map: Option<&mut Vec<usize>>) -> String {
    let mut out = String::with_capacity(s.len());
    match map {
        None => {
            for c in s.chars() {
                out.extend(c.to_lowercase());
            }
        }
        Some(map) => {
            map.clear();
            for (i, c) in s.char_indices() {
                let at = out.len();
                out.extend(c.to_lowercase());
                map.resize(out.len(), usize::MAX);
                map[at] = i;
            }
            map.push(s.len());
        }
    }
    out
}

pub(crate) fn unescape(t: &str) -> Cow<'_, str> {
    if !t.contains('\\') {
        return Cow::Borrowed(t);
    }
    let mut out = String::with_capacity(t.len());
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('t') => {
                chars.next();
                out.push('\t');
            }
            Some('n') => {
                chars.next();
                out.push('\n');
            }
            Some('r') => {
                chars.next();
                out.push('\r');
            }
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            Some('u') => {
                let rest: String = chars.clone().take(12).collect();
                let code = rest
                    .strip_prefix("u{")
                    .and_then(|r| r.split_once('}'))
                    .and_then(|(hex, _)| u32::from_str_radix(hex, 16).ok())
                    .and_then(char::from_u32);
                match code {
                    Some(ch) => {
                        let used = rest.find('}').expect("closed") + 1;
                        for _ in 0..used {
                            chars.next();
                        }
                        out.push(ch);
                    }
                    None => out.push('\\'),
                }
            }
            _ => out.push('\\'),
        }
    }
    Cow::Owned(out)
}

pub(crate) fn build(source: &str, match_case: bool) -> Result<Regex, Error> {
    RegexBuilder::new(source)
        .case_insensitive(!match_case)
        .dot_matches_new_line(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_DFA_LIMIT)
        .nest_limit(100)
        .build()
        .map_err(|e| match e {
            regex::Error::CompiledTooBig(_) => {
                Error::BadPattern("the pattern is too complex".into())
            }
            e => Error::BadPattern(
                e.to_string()
                    .lines()
                    .last()
                    .unwrap_or("not valid")
                    .trim()
                    .trim_start_matches("error: ")
                    .to_string(),
            ),
        })
}

impl Matcher {
    pub fn new(p: &Pattern) -> Result<Matcher, Error> {
        if p.text.is_empty() {
            return Err(Error::BadPattern("enter something to look for".into()));
        }
        if p.text.chars().count() > MAX_PATTERN_CHARS {
            return Err(Error::BadPattern(format!(
                "a pattern can be at most {MAX_PATTERN_CHARS} characters"
            )));
        }
        if let Some(test) = Test::new(p)? {
            return Ok(Matcher {
                // Never used: the test decides.
                re: build(r"[^\s\S]", true)?,
                literal: true,
                // The replacement is put in as typed.
                rep_escapes: false,
                test: Some(test),
                preserve: false,
            });
        }
        let text = if p.escapes && p.kind != PatternKind::Regex {
            unescape(&p.text)
        } else {
            Cow::Borrowed(p.text.as_str())
        };
        let accents = p.ignore_accents;
        let body = match p.kind {
            PatternKind::Number | PatternKind::Date | PatternKind::Similar => {
                unreachable!("a test")
            }
            PatternKind::Plain if accents => text.chars().map(|c| lit(c, true)).collect(),
            PatternKind::Plain => regex::escape(&text),
            PatternKind::Wildcard => wildcard_to_regex(&text, accents),
            PatternKind::Regex if accents => regex_ignore_accents(&text),
            PatternKind::Regex => text.into_owned(),
        };
        let source = if p.whole_cell {
            format!(r"\A(?:{body})\z")
        } else if p.whole_word {
            // "Half" boundaries: no letter or digit just before or after,
            // whatever the pattern's own first and last characters are.
            format!(r"\b{{start-half}}(?:{body})\b{{end-half}}")
        } else {
            body
        };
        Ok(Matcher {
            re: build(&source, p.match_case).map_err(|e| match e {
                Error::BadPattern(m) if accents && m.contains("too complex") => Error::BadPattern(
                    "the text is too long to search with Ignore accents: untick it or shorten the text".into(),
                ),
                e => e,
            })?,
            literal: p.kind != PatternKind::Regex,
            rep_escapes: p.escapes || p.kind == PatternKind::Regex,
            test: None,
            preserve: p.preserve_case,
        })
    }

    /// Text conditions of filters: `text` anchored at the start and/or end.
    pub(crate) fn anchored(
        text: &str,
        match_case: bool,
        start: bool,
        end: bool,
    ) -> Result<Matcher, Error> {
        let source = format!(
            "{}{}{}",
            if start { r"\A" } else { "" },
            regex::escape(text),
            if end { r"\z" } else { "" }
        );
        Ok(Matcher {
            re: build(&source, match_case)?,
            literal: true,
            rep_escapes: false,
            test: None,
            preserve: false,
        })
    }

    pub fn is_match(&self, s: &str) -> bool {
        match &self.test {
            Some(t) => t.matches(s),
            None => self.re.is_match(s),
        }
    }

    /// Byte range of the first match.
    pub fn find(&self, s: &str) -> Option<Range<usize>> {
        match &self.test {
            Some(t) => t.matches(s).then_some(0..s.len()),
            None => self.re.find(s).map(|m| m.range()),
        }
    }

    /// `s` with every match replaced, and the number of matches replaced, or
    /// `None` when nothing matched. Empty matches are skipped unless the
    /// cell is empty (so `^$` fills blank cells, but `x*` never inserts text
    /// between every character).
    pub fn replace(&self, s: &str, rep: &str) -> Result<Option<(String, u64)>, Error> {
        let raw = rep;
        let rep: Cow<'_, str> = if self.rep_escapes {
            unescape(rep)
        } else {
            Cow::Borrowed(rep)
        };
        let rep = rep.as_ref();
        if let Some(t) = &self.test {
            // The whole cell is replaced.
            if !t.matches(s) {
                return Ok(None);
            }
            if rep.len() > MAX_RESULT_CELL_BYTES {
                return Err(Error::OutOfMemory(
                    "a replacement would make a cell too large",
                ));
            }
            return Ok(Some((rep.to_string(), 1)));
        }
        let mut out = String::new();
        let mut last = 0;
        let mut hits = 0u64;
        let push = |out: &mut String, piece: &str| -> Result<(), Error> {
            if out.len() + piece.len() > MAX_RESULT_CELL_BYTES {
                return Err(Error::OutOfMemory(
                    "a replacement would make a cell too large",
                ));
            }
            out.push_str(piece);
            Ok(())
        };
        if self.literal {
            for m in self.re.find_iter(s) {
                if m.is_empty() && !s.is_empty() {
                    continue;
                }
                push(&mut out, &s[last..m.start()])?;
                if self.preserve {
                    push(&mut out, &keep_case(m.as_str(), rep))?;
                } else {
                    push(&mut out, rep)?;
                }
                last = m.end();
                hits += 1;
            }
        } else {
            let template = CaseTemplate::new(raw, self.rep_escapes);
            // \U \L \u \l in the replacement say the case: they win over
            // Preserve case.
            let preserve = self.preserve && !template.has_marks();
            let mut piece = String::new();
            for caps in self.re.captures_iter(s) {
                let m = caps.get(0).expect("group 0 always matches");
                if m.is_empty() && !s.is_empty() {
                    continue;
                }
                push(&mut out, &s[last..m.start()])?;
                piece.clear();
                template.expand(&caps, &mut piece);
                if preserve {
                    piece = keep_case(m.as_str(), &piece);
                }
                push(&mut out, &piece)?;
                last = m.end();
                hits += 1;
            }
        }
        if hits == 0 {
            return Ok(None);
        }
        push(&mut out, &s[last..])?;
        Ok(Some((out, hits)))
    }

    /// A replacement naming a group the pattern does not have (`$2` with one
    /// group, or `$1a`, read as a group called "1a"), which would quietly
    /// put nothing there.
    pub fn check_replacement(&self, rep: &str) -> Result<(), Error> {
        if self.literal || self.test.is_some() {
            return Ok(());
        }
        let names: Vec<&str> = self.re.capture_names().flatten().collect();
        let groups = self.re.captures_len();
        let mut chars = rep.char_indices().peekable();
        while let Some((_, c)) = chars.next() {
            if c != '$' {
                continue;
            }
            let name: String = match chars.peek() {
                Some((_, '$')) => {
                    chars.next();
                    continue;
                }
                Some((_, '{')) => {
                    chars.next();
                    let mut n = String::new();
                    let mut closed = false;
                    for (_, m) in chars.by_ref() {
                        if m == '}' {
                            closed = true;
                            break;
                        }
                        n.push(m);
                    }
                    // "${" with no "}" is plain text to the regex library.
                    if !closed {
                        continue;
                    }
                    n
                }
                _ => {
                    // The regex library reads a name as A-Z a-z 0-9 _ only.
                    let mut n = String::new();
                    while let Some(&(_, m)) = chars.peek() {
                        if m.is_ascii_alphanumeric() || m == '_' {
                            n.push(m);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    n
                }
            };
            if name.is_empty() {
                continue;
            }
            let ok = match name.parse::<usize>() {
                Ok(i) => i < groups,
                Err(_) => names.contains(&name.as_str()),
            };
            if !ok {
                let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
                let hint = if !digits.is_empty() && digits.len() < name.len() {
                    format!(
                        " (for group {digits} followed by \"{}\", write ${{{digits}}}{})",
                        &name[digits.len()..],
                        &name[digits.len()..]
                    )
                } else {
                    String::new()
                };
                return Err(Error::BadPattern(format!(
                    "the replacement uses ${name}, which is not a group in the pattern{hint}"
                )));
            }
        }
        Ok(())
    }

    /// Matches in a piece of text (the edit panel), as UTF-16 offsets
    /// `(start, end)`, at most `limit`. Empty matches are skipped.
    pub fn find_in_text(&self, text: &str, limit: usize) -> Vec<(u32, u32)> {
        if let Some(t) = &self.test {
            return if limit > 0 && !text.is_empty() && t.matches(text) {
                vec![(0, text.encode_utf16().count() as u32)]
            } else {
                Vec::new()
            };
        }
        let mut out = Vec::new();
        let mut byte = 0usize;
        let mut unit = 0u32;
        let mut advance = |to: usize| -> u32 {
            unit += text[byte..to].encode_utf16().count() as u32;
            byte = to;
            unit
        };
        for m in self.re.find_iter(text) {
            if m.is_empty() {
                continue;
            }
            if out.len() >= limit {
                break;
            }
            let s = advance(m.start());
            let e = advance(m.end());
            out.push((s, e));
        }
        out
    }
}

/// Which cells to search.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Every column of every shown row.
    #[default]
    All,
    /// These columns of every shown row.
    Columns { cols: Vec<u32> },
    /// A rectangle of shown rows and columns (a selection).
    Range {
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
    },
    /// Several selected blocks of shown rows and columns (Ctrl+click). The
    /// engine makes them disjoint itself, so a cell in two blocks is searched,
    /// counted and replaced once. At most [`MAX_FIND_BLOCKS`].
    Areas { areas: Vec<crate::areas::Area> },
}

/// Most blocks one search may be given (as for the other multi-block commands).
pub const MAX_FIND_BLOCKS: usize = 10_000;

/// A search: what and where.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FindSpec {
    pub pattern: Pattern,
    pub scope: Scope,
    /// Leave the first row (column titles) out of whole-column scopes.
    #[serde(default)]
    pub skip_first: bool,
    /// Columns left out of every scope (hidden columns, unless the user
    /// asks to include them).
    #[serde(default)]
    pub skip_cols: Vec<u32>,
    /// Count, list and replace in rows a filter or the user hid too (Find
    /// next, the lit-up matches and row actions stay on shown rows).
    #[serde(default)]
    pub hidden_rows: bool,
    /// Hidden columns that are searched (to say how many hidden cells a
    /// replace changes).
    #[serde(default)]
    pub hidden_cols: Vec<u32>,
    /// Find next goes down each column before moving right.
    #[serde(default)]
    pub by_columns: bool,
}

/// A cell found by find-next: `((row, col), wrapped)`.
pub type Found = ((u64, u32), bool);

/// One cell found by find-all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    pub row: u64,
    /// The row's number in the whole document (differs from `row` while a
    /// filter is on).
    pub number: u64,
    pub col: u32,
    /// The cell's text around the first match (line breaks shown as spaces).
    pub text: String,
    /// The match within `text`, in characters.
    pub start: u32,
    pub len: u32,
    /// In a hidden row (`row` is then the nearest shown row).
    pub hidden: bool,
}

/// Result of find-all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindAll {
    pub hits: Vec<Hit>,
    /// Cells that match (all of them, not just those listed).
    pub cells: u64,
    /// Matching cells per column (column, count), all of them, in column
    /// order; columns with none are left out.
    #[serde(default)]
    pub per_col: Vec<(u32, u64)>,
    /// The document's revision when searched (acting on the rows later
    /// refuses if the file changed since).
    #[serde(default)]
    pub revision: u64,
}

/// The longest new text kept for a ghost, in characters.
pub const MAX_GHOST_CHARS: usize = 20_000;
/// One window's time: past this the call stops and says so.
const GHOST_BUDGET: Duration = Duration::from_millis(50);

/// One cell a replace would change, as the grid shows it before the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ghost {
    /// Shown row.
    pub row: u64,
    /// The row's number in the whole document (what the preview's unticked changes are kept by).
    pub number: u64,
    pub col: u32,
    /// The new text (cut at `MAX_GHOST_CHARS`, then `cut` is set).
    pub after: String,
    pub cut: bool,
}

/// The ghosts of a window; `partial` when the time ran out first (more changes are not shown).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GhostWindow {
    pub ghosts: Vec<Ghost>,
    pub partial: bool,
}

/// The one place a cell's replacement is made, for the plan, the count and the ghosts: they cannot
/// drift apart because they all call this.
pub(crate) fn replacement_for(
    m: &Matcher,
    text: &str,
    replacement: &str,
) -> Result<Option<(String, u64)>, Error> {
    m.replace(text, replacement)
}

fn cut_ghost(s: String) -> (String, bool) {
    match s.char_indices().nth(MAX_GHOST_CHARS) {
        Some((at, _)) => (s[..at].to_string(), true),
        None => (s, false),
    }
}

/// One cell a replace would change (for the preview).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub row: u64,
    /// The row's number in the whole document.
    pub number: u64,
    pub col: u32,
    pub before: String,
    pub after: String,
    /// In a hidden row (`row` is then the nearest shown row) or a hidden
    /// column.
    pub hidden: bool,
}

/// A changed cell's logical column, its matches and whether it is hidden.
type CellAt = (u32, u32, bool);

/// A computed replace-all, applied with [`Document::apply_replace`].
#[derive(Debug, Clone)]
pub struct ReplacePlan {
    revision: u64,
    changes: Vec<(u32, Vec<(u32, String)>)>,
    /// For each entry of `changes`: its row number in the document and, for
    /// each changed cell, the logical column, its matches and whether it is
    /// hidden (for paging through the preview and unticking).
    at: Vec<(u64, Vec<CellAt>)>,
    /// Cells that change.
    pub cells: u64,
    /// Of them, cells in hidden rows or columns.
    pub hidden_cells: u64,
    /// Matches replaced.
    pub hits: u64,
    /// The first changes, for showing before applying.
    pub preview: Vec<Change>,
}

impl ReplacePlan {
    /// The rows (ids) the plan changes.
    pub fn row_ids(&self) -> Vec<u32> {
        self.changes.iter().map(|(id, _)| *id).collect()
    }

    /// Leave these cells (row number in the document, logical column) out
    /// of the plan: the changes the user unticked in the preview.
    pub fn exclude(&mut self, cells: &[(u64, u32)]) {
        if cells.is_empty() {
            return;
        }
        let skip: std::collections::HashSet<(u64, u32)> = cells.iter().copied().collect();
        let (mut removed, mut hits, mut hidden) = (0u64, 0u64, 0u64);
        for ((_, row), (r, cols)) in self.changes.iter_mut().zip(self.at.iter_mut()) {
            let mut k = 0;
            while k < cols.len() {
                if skip.contains(&(*r, cols[k].0)) {
                    hits += u64::from(cols[k].1);
                    hidden += u64::from(cols[k].2);
                    cols.remove(k);
                    row.remove(k);
                    removed += 1;
                } else {
                    k += 1;
                }
            }
        }
        let keep: Vec<bool> = self.changes.iter().map(|(_, c)| !c.is_empty()).collect();
        let mut i = 0;
        self.changes.retain(|_| {
            i += 1;
            keep[i - 1]
        });
        let mut i = 0;
        self.at.retain(|_| {
            i += 1;
            keep[i - 1]
        });
        self.cells = self.cells.saturating_sub(removed);
        self.hits = self.hits.saturating_sub(hits);
        self.hidden_cells = self.hidden_cells.saturating_sub(hidden);
    }
}

/// At most this many pairs in one batch replace, and this much text.
pub const MAX_BATCH_PAIRS: usize = 100_000;
pub const MAX_BATCH_BYTES: usize = 16 << 20;

/// Many find → replace pairs at once (a pasted two-column list, a file or
/// another tab).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchReplace {
    pub pairs: Vec<(String, String)>,
    /// A cell must equal a "find" (codes, names); otherwise every
    /// occurrence inside cells is replaced.
    pub whole_cell: bool,
    pub match_case: bool,
    pub scope: Scope,
    pub skip_first: bool,
    pub skip_cols: Vec<u32>,
}

/// How many cells match, and where a cell is among them (Find's "3 of 1,204").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct FindCount {
    /// Matching cells.
    pub cells: u64,
    /// Matching cells before the given one (in Find next order).
    pub before: u64,
    /// The given cell matches.
    pub here: bool,
}

/// The cells a search covers, resolved against the document.
pub(crate) struct Area {
    /// Shown rows, or with `all` document positions (hidden rows too).
    pub(crate) rows: Range<u64>,
    all: bool,
    /// Hidden columns that are searched, sorted (for counting).
    hidden_cols: Vec<u32>,
    /// Logical columns in search order.
    pub(crate) cols: Vec<u32>,
    /// Physical columns to read, sorted, and for each logical column the
    /// slot in that list (`None`: past the column map, always empty).
    wanted: Vec<u32>,
    slot: Vec<Option<usize>>,
    /// Several blocks: which of `cols` (by index) are searched in each band of
    /// rows. `rows` and `cols` are then the bounding box. `None`: every column
    /// of every row (one block, or a whole file), the same loop as ever.
    mask: Option<Mask>,
}

/// Bands of rows (sorted, not overlapping) with the columns searched in each.
#[derive(Clone)]
pub(crate) struct Mask {
    starts: Vec<u64>,
    ends: Vec<u64>,
    /// For each band the indexes into `Area::cols`, ascending.
    ks: Vec<Vec<usize>>,
}

impl Mask {
    /// The columns searched in `row` (none outside every band).
    fn ks_of(&self, row: u64) -> &[usize] {
        let i = self.starts.partition_point(|&s| s <= row);
        if i == 0 || row >= self.ends[i - 1] {
            return &[];
        }
        &self.ks[i - 1]
    }

    /// The same bands for the one column `k` alone (as index 0).
    fn only(&self, k: usize) -> Mask {
        let mut m = Mask {
            starts: Vec::new(),
            ends: Vec::new(),
            ks: Vec::new(),
        };
        for ((&a, &b), ks) in self.starts.iter().zip(&self.ends).zip(&self.ks) {
            if ks.binary_search(&k).is_err() {
                continue;
            }
            match m.ends.last_mut() {
                Some(e) if *e == a => *e = b,
                _ => {
                    m.starts.push(a);
                    m.ends.push(b);
                    m.ks.push(vec![0]);
                }
            }
        }
        m
    }
}

/// Text for a one-line list: a line break shows as ⏎ (one for `\r\n`), a
/// tab as →, a no-break space as ·, so they can be seen.
pub(crate) fn visible(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('⏎');
            }
            '\n' => out.push('⏎'),
            '\t' => out.push('→'),
            '\u{a0}' => out.push('·'),
            _ => out.push(c),
        }
    }
    out
}

fn snippet(s: &str, hit: Range<usize>) -> (String, u32, u32) {
    let before: Vec<char> = s[..hit.start].chars().collect();
    let skip = before.len().saturating_sub(SNIPPET_CHARS);
    let mut text = visible(&before[skip..].iter().collect::<String>());
    if skip > 0 {
        text.insert(0, '…');
    }
    let start = text.chars().count() as u32;
    let m = visible(
        &s[hit.clone()]
            .chars()
            .take(SNIPPET_CHARS * 2)
            .collect::<String>(),
    );
    let len = m.chars().count() as u32;
    text.push_str(&m);
    let after = &s[hit.end..];
    let tail: String = after.chars().take(SNIPPET_CHARS).collect();
    let more = tail.len() < after.len();
    text.push_str(&visible(&tail));
    if more {
        text.push('…');
    }
    (text, start, len)
}

pub(crate) fn short(s: &str) -> String {
    let mut t: String = s.chars().take(SNIPPET_CHARS * 2).collect();
    if t.len() < s.len() {
        t.push('…');
    }
    visible(&t)
}

/// A block in the area's row coordinates and logical columns:
/// (first row, past the last row, first column, past the last column).
type Block = (u64, u64, u32, u32);

/// The bands of rows of the blocks with, in each, the searched columns as
/// indexes into `cols` (sorted). Neighbouring bands with the same columns join.
fn band_mask(blocks: &[Block], cols: &[u32]) -> Mask {
    let mut ys: Vec<u64> = blocks.iter().flat_map(|b| [b.0, b.1]).collect();
    ys.sort_unstable();
    ys.dedup();
    // Each block as the index range of `cols` it covers.
    let mut order: Vec<(u64, u64, usize, usize)> = blocks
        .iter()
        .map(|b| {
            (
                b.0,
                b.1,
                cols.partition_point(|&c| c < b.2),
                cols.partition_point(|&c| c < b.3),
            )
        })
        .filter(|b| b.2 < b.3)
        .collect();
    order.sort_unstable_by_key(|b| b.0);
    let mut m = Mask {
        starts: Vec::new(),
        ends: Vec::new(),
        ks: Vec::new(),
    };
    let mut next = 0;
    let mut active: Vec<(u64, usize, usize)> = Vec::new();
    for w in ys.windows(2) {
        let (y, y2) = (w[0], w[1]);
        while next < order.len() && order[next].0 <= y {
            active.push((order[next].1, order[next].2, order[next].3));
            next += 1;
        }
        active.retain(|a| a.0 > y);
        if active.is_empty() {
            continue;
        }
        let mut ks: Vec<usize> = active.iter().flat_map(|a| a.1..a.2).collect();
        ks.sort_unstable();
        ks.dedup();
        match (m.ends.last_mut(), m.ks.last()) {
            (Some(e), Some(last)) if *e == y && *last == ks => *e = y2,
            _ => {
                m.starts.push(y);
                m.ends.push(y2);
                m.ks.push(ks);
            }
        }
    }
    m
}

impl Document {
    pub(crate) fn area(&self, spec: &FindSpec) -> Result<Area, Error> {
        // Hidden rows too: document positions instead of shown rows.
        let all = spec.hidden_rows && self.is_filtered();
        let n = if all {
            self.total_rows()
        } else {
            self.row_count()
        };
        let first = u64::from(spec.skip_first && n > 0);
        let (rows, cols, blocks): (Range<u64>, Vec<u32>, Option<Vec<Block>>) = match &spec.scope {
            Scope::All => (first..n, (0..self.col_count()).collect(), None),
            Scope::Columns { cols } => {
                let mut c = cols.clone();
                c.sort_unstable();
                c.dedup();
                c.retain(|&x| x < self.col_count());
                (first..n, c, None)
            }
            Scope::Range {
                row,
                rows,
                col,
                cols,
            } => {
                let c1 = col.saturating_add(*cols).min(self.col_count());
                let (r0, r1) = if all {
                    // The positions the shown rows span (hidden ones between).
                    let shown = self.row_count();
                    let a = (*row).min(shown);
                    let b = row.saturating_add(*rows).min(shown);
                    if a >= b {
                        (0, 0)
                    } else {
                        (
                            self.row_number(a).unwrap_or(a),
                            self.row_number(b - 1).map_or(b, |x| x + 1),
                        )
                    }
                } else {
                    ((*row).min(n), row.saturating_add(*rows).min(n))
                };
                (r0..r1, (*col..c1).collect(), None)
            }
            Scope::Areas { areas } => {
                if areas.len() > MAX_FIND_BLOCKS {
                    return Err(Error::Invalid(format!(
                        "Find works on up to {MAX_FIND_BLOCKS} blocks at once"
                    )));
                }
                // Cut to the table, leave out the title row when asked, make
                // the blocks disjoint (a cell in two is searched once).
                let shown = self.row_count();
                let ncols = self.col_count();
                let mut cut = Vec::with_capacity(areas.len());
                for a in areas {
                    let r0 = a.row.max(first);
                    let r1 = a.row.saturating_add(a.rows).min(shown);
                    let c1 = a.col.saturating_add(a.cols).min(ncols);
                    if r0 < r1 && a.col < c1 {
                        cut.push(crate::areas::Area {
                            row: r0,
                            rows: r1 - r0,
                            col: a.col,
                            cols: c1 - a.col,
                        });
                    }
                }
                let mut blocks: Vec<Block> = Vec::new();
                for b in crate::areas::disjoint(&cut) {
                    let (r0, r1) = (b.row, b.row + b.rows);
                    // With hidden rows included, the document positions the
                    // block's shown rows span (the hidden ones between).
                    let (p0, p1) = if all {
                        (
                            self.row_number(r0).unwrap_or(r0),
                            self.row_number(r1 - 1).map_or(r1, |x| x + 1),
                        )
                    } else {
                        (r0, r1)
                    };
                    blocks.push((p0, p1, b.col, b.col + b.cols));
                }
                if blocks.is_empty() {
                    (0..0, Vec::new(), None)
                } else {
                    let r0 = blocks.iter().map(|b| b.0).min().unwrap_or(0);
                    let r1 = blocks.iter().map(|b| b.1).max().unwrap_or(0);
                    let mut cs: Vec<u32> = Vec::new();
                    let mut spans: Vec<(u32, u32)> = blocks.iter().map(|b| (b.2, b.3)).collect();
                    spans.sort_unstable();
                    let mut next = 0u32;
                    for (c0, c1) in spans {
                        for c in c0.max(next)..c1 {
                            cs.push(c);
                        }
                        next = next.max(c1);
                    }
                    (r0..r1, cs, Some(blocks))
                }
            }
        };
        let mut cols = cols;
        if !spec.skip_cols.is_empty() {
            let mut skip = spec.skip_cols.clone();
            skip.sort_unstable();
            cols.retain(|c| skip.binary_search(c).is_err());
        }
        let mut wanted: Vec<u32> = cols.iter().filter_map(|&c| self.phys(c)).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let slot = cols
            .iter()
            .map(|&c| {
                self.phys(c)
                    .map(|p| wanted.binary_search(&p).expect("listed"))
            })
            .collect();
        let mut hidden_cols: Vec<u32> = spec
            .hidden_cols
            .iter()
            .copied()
            .filter(|c| cols.contains(c))
            .collect();
        hidden_cols.sort_unstable();
        let mask = blocks.map(|b| band_mask(&b, &cols));
        Ok(Area {
            rows,
            all,
            hidden_cols,
            cols,
            wanted,
            slot,
            mask,
        })
    }

    /// The area of shown rows only (Find next, lit-up matches, row actions).
    pub(crate) fn shown_area(&self, spec: &FindSpec) -> Result<Area, Error> {
        if !spec.hidden_rows {
            return self.area(spec);
        }
        self.area(&FindSpec {
            hidden_rows: false,
            ..spec.clone()
        })
    }

    /// Row `r` of an area: its id, its shown row (the nearest one when it
    /// is hidden), its number in the document and whether it is hidden.
    pub(crate) fn area_row(&self, area: &Area, r: u64) -> Option<(u32, u64, u64, bool)> {
        if !area.all {
            return Some((self.id_at(r)?, r, self.row_number(r).unwrap_or(r), false));
        }
        let id = self.id_of_pos(r)?;
        let (shown, exact) = self.shown_row_of(r).unwrap_or((0, false));
        Some((id, shown, r, !exact))
    }

    /// A one-column area of `area`'s `k`th column.
    fn one_col(area: &Area, k: usize) -> Area {
        Area {
            rows: area.rows.clone(),
            all: area.all,
            hidden_cols: area.hidden_cols.clone(),
            cols: vec![area.cols[k]],
            wanted: area.slot[k]
                .map(|s| vec![area.wanted[s]])
                .unwrap_or_default(),
            slot: vec![area.slot[k].map(|_| 0)],
            mask: area.mask.as_ref().map(|m| m.only(k)),
        }
    }

    /// Run `f` on every searched cell of shown row `row` (in column order,
    /// or reversed), stopping when it returns `Some`.
    pub(crate) fn each_cell<'a, T>(
        &'a self,
        area: &Area,
        row: u64,
        rev: bool,
        buf: &mut Vec<Cow<'a, str>>,
        mut f: impl FnMut(usize, &str) -> Option<T>,
    ) -> Option<T> {
        let Some(mask) = &area.mask else {
            // One block or a whole file: every searched column, the loop as it always was.
            let id = if area.all {
                self.id_of_pos(row)?
            } else {
                self.id_at(row)?
            };
            self.read_cells(id, &area.wanted, buf);
            let n = area.cols.len();
            for i in 0..n {
                let k = if rev { n - 1 - i } else { i };
                let text = area.slot[k].map_or("", |s| &buf[s]);
                if let Some(t) = f(k, text) {
                    return Some(t);
                }
            }
            return None;
        };
        // Several blocks: a row between them is not read at all.
        let ks = mask.ks_of(row);
        if ks.is_empty() {
            return None;
        }
        let id = if area.all {
            self.id_of_pos(row)?
        } else {
            self.id_at(row)?
        };
        self.read_cells(id, &area.wanted, buf);
        let n = ks.len();
        for i in 0..n {
            let k = ks[if rev { n - 1 - i } else { i }];
            let text = area.slot[k].map_or("", |s| &buf[s]);
            if let Some(t) = f(k, text) {
                return Some(t);
            }
        }
        None
    }

    /// The next cell after `from` (or before it, with `backward`) whose text
    /// matches, wrapping around the searched area. Returns the cell and
    /// whether the search wrapped. `from` outside the area starts at its
    /// beginning (or end).
    pub fn find_next(
        &self,
        spec: &FindSpec,
        from: Option<(u64, u32)>,
        backward: bool,
        progress: &Progress,
    ) -> Result<Option<Found>, Error> {
        let m = Matcher::new(&spec.pattern)?;
        let area = self.shown_area(spec)?;
        let (r0, r1) = (area.rows.start, area.rows.end);
        if r0 >= r1 || area.cols.is_empty() {
            return Ok(None);
        }
        if spec.by_columns {
            return self.find_next_by_columns(&m, &area, from, backward, progress);
        }
        progress.start(r1 - r0);
        let ncols = area.cols.len();
        // The start row and the first column index to try in it.
        let (srow, scol) = match from {
            Some((r, c)) if (r0..r1).contains(&r) => {
                let k = area.cols.iter().position(|&x| x == c);
                match (k, backward) {
                    (Some(k), false) => (r, k as i64 + 1),
                    (Some(k), true) => (r, k as i64 - 1),
                    (None, false) => (r, area.cols.partition_point(|&x| x < c) as i64),
                    (None, true) => (r, area.cols.partition_point(|&x| x < c) as i64 - 1),
                }
            }
            _ if backward => (r1 - 1, ncols as i64 - 1),
            _ => (r0, 0),
        };
        let hit_in_row = |r: u64, ks: Range<usize>, rev: bool| -> Option<u32> {
            let mut buf = Vec::new();
            self.each_cell(&area, r, rev, &mut buf, |k, t| {
                (ks.contains(&k) && m.is_match(t)).then_some(area.cols[k])
            })
        };
        // 1. The rest of the start row.
        let rest = if backward {
            0..(scol + 1).clamp(0, ncols as i64) as usize
        } else {
            scol.clamp(0, ncols as i64) as usize..ncols
        };
        if let Some(c) = hit_in_row(srow, rest, backward) {
            return Ok(Some(((srow, c), false)));
        }
        // 2. The rows after (before) it, then 3. wrap to the other end.
        let spans: [(Range<u64>, bool); 2] = if backward {
            [(r0..srow, false), (srow + 1..r1, true)]
        } else {
            [(srow + 1..r1, false), (r0..srow, true)]
        };
        for (span, wrapped) in spans {
            if let Some(hit) = self.first_row_hit(span, backward, progress, &hit_in_row)? {
                return Ok(Some((hit, wrapped)));
            }
        }
        // 4. The start row's other cells.
        let before = if backward {
            (scol + 1).clamp(0, ncols as i64) as usize..ncols
        } else {
            0..scol.clamp(0, ncols as i64) as usize
        };
        Ok(hit_in_row(srow, before, backward).map(|c| ((srow, c), true)))
    }

    /// [`Document::find_next`] going down each column before moving right
    /// (or up, and left, with `backward`).
    fn find_next_by_columns(
        &self,
        m: &Matcher,
        area: &Area,
        from: Option<(u64, u32)>,
        backward: bool,
        progress: &Progress,
    ) -> Result<Option<Found>, Error> {
        let (r0, r1) = (area.rows.start, area.rows.end);
        let ncols = area.cols.len();
        progress.start((r1 - r0).saturating_mul(ncols as u64));
        // The start: a column index and the first row to try in it.
        let (sk, srow) = match from {
            Some((r, c)) if (r0..r1).contains(&r) => {
                match (area.cols.iter().position(|&x| x == c), backward) {
                    (Some(k), false) => (k, r + 1),
                    (Some(k), true) => (k, r),
                    // Outside the searched columns: the next searched one to
                    // the right (forwards) or to the left (backwards); past
                    // the end the search starts over from the other end.
                    (None, false) => {
                        let k = area.cols.partition_point(|&x| x < c);
                        if k >= ncols {
                            return Ok(self
                                .find_next_by_columns(m, area, None, false, progress)?
                                .map(|(h, _)| (h, true)));
                        }
                        (k, r0)
                    }
                    (None, true) => {
                        let k = area.cols.partition_point(|&x| x < c);
                        if k == 0 {
                            return Ok(self
                                .find_next_by_columns(m, area, None, true, progress)?
                                .map(|(h, _)| (h, true)));
                        }
                        (k - 1, r1)
                    }
                }
            }
            _ if backward => (ncols - 1, r1),
            _ => (0, r0),
        };
        let column = |k: usize, span: Range<u64>| -> Result<Option<(u64, u32)>, Error> {
            if k >= ncols {
                return Ok(None);
            }
            let one = Self::one_col(area, k);
            let hit = |r: u64, _: Range<usize>, rev: bool| -> Option<u32> {
                let mut buf = Vec::new();
                self.each_cell(&one, r, rev, &mut buf, |_, t| {
                    m.is_match(t).then_some(one.cols[0])
                })
            };
            self.first_row_hit(span, backward, progress, &hit)
        };
        // 1. The rest of the start column.
        let rest = if backward {
            r0..srow.min(r1)
        } else {
            srow.min(r1)..r1
        };
        if let Some(h) = column(sk.min(ncols - 1), rest)? {
            return Ok(Some((h, false)));
        }
        // 2. The columns after (before) it, then 3. wrap round.
        let after: Vec<usize> = if backward {
            (0..sk.min(ncols)).rev().collect()
        } else {
            (sk + 1..ncols).collect()
        };
        for k in after {
            if let Some(h) = column(k, r0..r1)? {
                return Ok(Some((h, false)));
            }
        }
        let wrap: Vec<usize> = if backward {
            (sk + 1..ncols).rev().collect()
        } else {
            (0..sk.min(ncols)).collect()
        };
        for k in wrap {
            if let Some(h) = column(k, r0..r1)? {
                return Ok(Some((h, true)));
            }
        }
        // 4. The start column's other rows.
        let before = if backward {
            srow.min(r1)..r1
        } else {
            r0..srow.min(r1)
        };
        Ok(column(sk.min(ncols - 1), before)?.map(|h| (h, true)))
    }

    /// The first (or last) row in `span` with a hit, searched in parallel
    /// blocks, in order.
    fn first_row_hit(
        &self,
        span: Range<u64>,
        backward: bool,
        progress: &Progress,
        hit_in_row: &(dyn Fn(u64, Range<usize>, bool) -> Option<u32> + Sync),
    ) -> Result<Option<(u64, u32)>, Error> {
        if span.is_empty() {
            return Ok(None);
        }
        let blocks = (span.end - span.start).div_ceil(BLOCK);
        let batch = (rayon::current_num_threads() as u64 * 4).max(1);
        let mut b = 0;
        while b < blocks {
            let end = (b + batch).min(blocks);
            let found = (b..end).into_par_iter().find_map_first(|i| {
                let bi = if backward { blocks - 1 - i } else { i };
                let s = span.start + bi * BLOCK;
                let e = (s + BLOCK).min(span.end);
                let rows: Box<dyn Iterator<Item = u64>> = if backward {
                    Box::new((s..e).rev())
                } else {
                    Box::new(s..e)
                };
                for r in rows {
                    if let Some(c) = hit_in_row(r, 0..usize::MAX, backward) {
                        return Some((r, c));
                    }
                }
                None
            });
            if found.is_some() {
                return Ok(found);
            }
            progress.step(((end - b) * BLOCK).min(span.end - span.start))?;
            b = end;
        }
        Ok(None)
    }

    /// Every matching cell: up to `limit` listed in order, all counted.
    pub fn find_all(
        &self,
        spec: &FindSpec,
        limit: usize,
        progress: &Progress,
    ) -> Result<FindAll, Error> {
        let m = Matcher::new(&spec.pattern)?;
        let area = self.area(spec)?;
        let (r0, r1) = (area.rows.start, area.rows.end);
        progress.start(r1.saturating_sub(r0));
        if r0 >= r1 || area.cols.is_empty() {
            return Ok(FindAll {
                hits: Vec::new(),
                cells: 0,
                per_col: Vec::new(),
                revision: self.revision(),
            });
        }
        let ncols = area.cols.len();
        let blocks = (r1 - r0).div_ceil(BLOCK);
        let parts: Vec<(Vec<Hit>, Vec<u64>)> = (0..blocks)
            .into_par_iter()
            .map(|b| -> Result<(Vec<Hit>, Vec<u64>), Error> {
                let s = r0 + b * BLOCK;
                let e = (s + BLOCK).min(r1);
                let mut hits = Vec::new();
                let mut count = vec![0u64; ncols];
                let mut buf = Vec::new();
                for r in s..e {
                    let mut place = None;
                    self.each_cell(&area, r, false, &mut buf, |k, t| {
                        if let Some(range) = m.find(t) {
                            count[k] += 1;
                            if hits.len() < limit {
                                let (_, row, number, hidden) = *place.get_or_insert_with(|| {
                                    self.area_row(&area, r).unwrap_or((0, r, r, false))
                                });
                                let (text, start, len) = snippet(t, range);
                                hits.push(Hit {
                                    row,
                                    number,
                                    col: area.cols[k],
                                    text,
                                    start,
                                    len,
                                    hidden,
                                });
                            }
                        }
                        None::<()>
                    });
                }
                progress.step(e - s)?;
                Ok((hits, count))
            })
            .collect::<Result<_, _>>()?;
        let mut hits = Vec::new();
        let mut per = vec![0u64; ncols];
        for (h, c) in parts {
            for (a, b) in per.iter_mut().zip(c) {
                *a += b;
            }
            if hits.len() < limit {
                hits.extend(h.into_iter().take(limit - hits.len()));
            }
        }
        let cells = per.iter().sum();
        let per_col = area
            .cols
            .iter()
            .zip(per)
            .filter(|(_, n)| *n > 0)
            .map(|(&c, n)| (c, n))
            .collect();
        Ok(FindAll {
            hits,
            cells,
            per_col,
            revision: self.revision(),
        })
    }

    /// Count the matching cells, and how many come before `at` (row, column)
    /// in Find next order (rows, then columns), for "3 of 1,204".
    pub fn find_count(
        &self,
        spec: &FindSpec,
        at: Option<(u64, u32)>,
        progress: &Progress,
    ) -> Result<FindCount, Error> {
        self.find_count_with(spec, at, &mut None, progress)
    }

    /// [`Document::find_count`] with the matches per block of rows kept in
    /// `blocks` (for this document revision and search): when given, only
    /// the block holding `at` is read again, so Find next moving "3 of N"
    /// to "4 of N" does not read the whole file.
    pub fn find_count_with(
        &self,
        spec: &FindSpec,
        at: Option<(u64, u32)>,
        blocks: &mut Option<Vec<u64>>,
        progress: &Progress,
    ) -> Result<FindCount, Error> {
        let m = Matcher::new(&spec.pattern)?;
        let area = self.area(spec)?;
        let (r0, r1) = (area.rows.start, area.rows.end);
        progress.start(r1.saturating_sub(r0));
        if r0 >= r1 || area.cols.is_empty() {
            return Ok(FindCount::default());
        }
        // The cell's place in the column order (columns are in order); with
        // hidden rows its row is a document position.
        let at = at.map(|(r, c)| {
            if area.all {
                (self.row_number(r).unwrap_or(r), c)
            } else {
                (r, c)
            }
        });
        let cols_first = spec.by_columns;
        let (ar, ak, exact) = match at {
            Some((r, c)) => {
                let k = area.cols.partition_point(|&x| x < c);
                (r, k, area.cols.get(k) == Some(&c))
            }
            // No cell: nothing counts as before it.
            None => (0, 0, false),
        };
        let nblocks = (r1 - r0).div_ceil(BLOCK);
        let ncols = area.cols.len();
        // A block's count, and (column order) its count per column.
        let count_block = |b: u64| -> Result<(FindCount, Vec<u64>), Error> {
            let s = r0 + b * BLOCK;
            let e = (s + BLOCK).min(r1);
            let mut out = FindCount::default();
            let mut per = vec![0u64; if cols_first { ncols } else { 0 }];
            let mut buf = Vec::new();
            for r in s..e {
                self.each_cell(&area, r, false, &mut buf, |k, t| {
                    if m.is_match(t) {
                        out.cells += 1;
                        if cols_first {
                            per[k] += 1;
                        }
                        let earlier = if cols_first {
                            k < ak || (k == ak && r < ar)
                        } else {
                            r < ar || (r == ar && k < ak)
                        };
                        if earlier {
                            out.before += 1;
                        } else if r == ar && k == ak && exact {
                            out.here = true;
                        }
                    }
                    None::<()>
                });
            }
            progress.step(e - s)?;
            Ok((out, per))
        };
        // Column order keeps a count per block and column (block by block).
        if cols_first {
            if let Some(kept) = blocks
                .as_ref()
                .filter(|k| k.len() as u64 == nblocks * ncols as u64)
            {
                let col_total =
                    |k: usize| -> u64 { (0..nblocks as usize).map(|b| kept[b * ncols + k]).sum() };
                let cells = kept.iter().sum();
                let left: u64 = (0..ak.min(ncols)).map(col_total).sum();
                let (before, here) = match at {
                    Some((r, _)) if r >= r0 && r < r1 => {
                        let b = ((r - r0) / BLOCK) as usize;
                        let (mine, _) = count_block(b as u64)?;
                        // Earlier columns in the other blocks, the cell's
                        // column in the blocks above, then its own block.
                        let mut before =
                            left - (0..ak.min(ncols)).map(|k| kept[b * ncols + k]).sum::<u64>();
                        if ak < ncols {
                            before += (0..b).map(|x| kept[x * ncols + ak]).sum::<u64>();
                        }
                        (before + mine.before, mine.here)
                    }
                    Some((r, _)) if r >= r1 => {
                        (left + if ak < ncols { col_total(ak) } else { 0 }, false)
                    }
                    Some(_) => (left, false),
                    None => (0, false),
                };
                return Ok(FindCount {
                    cells,
                    before,
                    here,
                });
            }
        }
        if let Some(kept) = blocks
            .as_ref()
            .filter(|k| k.len() as u64 == nblocks && !cols_first)
        {
            let cells = kept.iter().sum();
            // Blocks wholly before the cell count in full; only the cell's
            // own block is read again.
            let (before, here) = match at {
                Some((r, _)) if r >= r0 && r < r1 => {
                    let b = (r - r0) / BLOCK;
                    let (mine, _) = count_block(b)?;
                    (
                        kept[..b as usize].iter().sum::<u64>() + mine.before,
                        mine.here,
                    )
                }
                Some((r, _)) if r >= r1 => (cells, false),
                _ => (0, false),
            };
            return Ok(FindCount {
                cells,
                before,
                here,
            });
        }
        let parts: Vec<(FindCount, Vec<u64>)> = (0..nblocks)
            .into_par_iter()
            .map(count_block)
            .collect::<Result<_, _>>()?;
        *blocks = Some(if cols_first {
            parts.iter().flat_map(|p| p.1.iter().copied()).collect()
        } else {
            parts.iter().map(|p| p.0.cells).collect()
        });
        let parts: Vec<FindCount> = parts.into_iter().map(|p| p.0).collect();
        Ok(parts
            .into_iter()
            .fold(FindCount::default(), |a, p| FindCount {
                cells: a.cells + p.cells,
                before: a.before + p.before,
                here: a.here || p.here,
            }))
    }

    /// The searched cells that match in a window of the grid (what is on
    /// screen), as (row, column): for lighting up every match.
    pub fn find_marks(
        &self,
        spec: &FindSpec,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
    ) -> Result<Vec<(u64, u32)>, Error> {
        const MAX_WINDOW: u64 = 1_000;
        let m = Matcher::new(&spec.pattern)?;
        let area = self.shown_area(spec)?;
        let r0 = row.max(area.rows.start);
        let r1 = row.saturating_add(rows.min(MAX_WINDOW)).min(area.rows.end);
        let c1 = col.saturating_add(cols.min(MAX_WINDOW as u32));
        let mut out = Vec::new();
        let mut buf = Vec::new();
        for r in r0..r1 {
            self.each_cell(&area, r, false, &mut buf, |k, t| {
                let c = area.cols[k];
                if c >= col && c < c1 && m.is_match(t) {
                    out.push((r, c));
                }
                None::<()>
            });
        }
        Ok(out)
    }

    /// What Replace all would make of the cells in a window of the grid (what is on screen), for
    /// showing the changes before they are made. The same routine, the same cells and the same
    /// protected-column rule as [`Document::plan_replace`] (`replacement_for`, `area`), only for the
    /// shown rows and columns of the window, so the ghosts cannot differ from what Replace all does.
    /// Each cell's new text is kept to `MAX_GHOST_CHARS`; the call stops after `GHOST_BUDGET` and
    /// says so (`partial`) rather than hold the window up on very long cells.
    pub fn replace_ghosts(
        &self,
        spec: &FindSpec,
        replacement: &str,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
    ) -> Result<GhostWindow, Error> {
        const MAX_WINDOW: u64 = 1_000;
        if replacement.chars().count() > MAX_REPLACEMENT_CHARS {
            return Err(Error::BadPattern(format!(
                "the replacement can be at most {MAX_REPLACEMENT_CHARS} characters"
            )));
        }
        let m = Matcher::new(&spec.pattern)?;
        m.check_replacement(replacement)?;
        let mut spec = spec.clone();
        spec.skip_cols.extend(self.protected_cols());
        let area = self.shown_area(&spec)?;
        let r0 = row.max(area.rows.start);
        let r1 = row.saturating_add(rows.min(MAX_WINDOW)).min(area.rows.end);
        let c1 = col.saturating_add(cols.min(MAX_WINDOW as u32));
        let started = Instant::now();
        let mut out = GhostWindow::default();
        let mut buf = Vec::new();
        'rows: for r in r0..r1 {
            if started.elapsed() > GHOST_BUDGET {
                out.partial = true;
                break;
            }
            let mut err = None;
            self.each_cell(&area, r, false, &mut buf, |k, t| {
                let c = area.cols[k];
                if c < col || c >= c1 || self.phys(c).is_none() {
                    return None::<()>;
                }
                match replacement_for(&m, t, replacement) {
                    Ok(Some((new, _))) if new != t => {
                        let (after, cut) = cut_ghost(new);
                        let number = self.row_number(r).unwrap_or(r);
                        out.ghosts.push(Ghost {
                            row: r,
                            number,
                            col: c,
                            after,
                            cut,
                        });
                    }
                    Ok(_) => {}
                    Err(e) => err = Some(e),
                }
                None
            });
            if let Some(e) = err {
                return Err(e);
            }
            if started.elapsed() > GHOST_BUDGET && r + 1 < r1 {
                out.partial = true;
                break 'rows;
            }
        }
        Ok(out)
    }

    /// How many cells Replace all would change, worked out the way the plan does but keeping
    /// nothing (no memory for millions of changes); cancellable.
    pub fn replace_count(
        &self,
        spec: &FindSpec,
        replacement: &str,
        progress: &Progress,
    ) -> Result<u64, Error> {
        self.ready_for_bulk()?;
        if replacement.chars().count() > MAX_REPLACEMENT_CHARS {
            return Err(Error::BadPattern(format!(
                "the replacement can be at most {MAX_REPLACEMENT_CHARS} characters"
            )));
        }
        let m = Matcher::new(&spec.pattern)?;
        m.check_replacement(replacement)?;
        let mut spec = spec.clone();
        spec.skip_cols.extend(self.protected_cols());
        let area = self.area(&spec)?;
        let (r0, r1) = (area.rows.start, area.rows.end);
        progress.start(r1.saturating_sub(r0));
        let blocks = r1.saturating_sub(r0).div_ceil(BLOCK);
        let parts: Vec<Result<u64, Error>> = (0..blocks)
            .into_par_iter()
            .map(|b| {
                let s = r0 + b * BLOCK;
                let e = (s + BLOCK).min(r1);
                let mut n = 0u64;
                let mut buf = Vec::new();
                for r in s..e {
                    if self.area_row(&area, r).is_none() {
                        continue;
                    }
                    let mut err = None;
                    self.each_cell(&area, r, false, &mut buf, |k, t| {
                        if self.phys(area.cols[k]).is_none() {
                            return None::<()>;
                        }
                        match replacement_for(&m, t, replacement) {
                            Ok(Some((new, _))) if new != t => n += 1,
                            Ok(_) => {}
                            Err(x) => err = Some(x),
                        }
                        None
                    });
                    if let Some(x) = err {
                        return Err(x);
                    }
                }
                progress.step(e - s)?;
                Ok(n)
            })
            .collect();
        let mut total = 0;
        for p in parts {
            total += p?;
        }
        Ok(total)
    }

    /// Work out a replace-all (nothing changes yet). `preview` changes are
    /// kept for showing first.
    pub fn plan_replace(
        &self,
        spec: &FindSpec,
        replacement: &str,
        preview: usize,
        progress: &Progress,
    ) -> Result<ReplacePlan, Error> {
        self.ready_for_bulk()?;
        if replacement.chars().count() > MAX_REPLACEMENT_CHARS {
            return Err(Error::BadPattern(format!(
                "the replacement can be at most {MAX_REPLACEMENT_CHARS} characters"
            )));
        }
        let m = Matcher::new(&spec.pattern)?;
        m.check_replacement(replacement)?;
        // Protected columns are left out of the plan (so the preview counts
        // only cells that change).
        let mut spec = spec.clone();
        spec.skip_cols.extend(self.protected_cols());
        let area = self.area(&spec)?;
        self.plan_cells_with(&area, preview, progress, |t| {
            replacement_for(&m, t, replacement)
        })
    }

    /// Replace many pairs at once (batch replace from a list): every pair in
    /// one pass, all at once (with A→B and B→C, an "A" becomes "B", never
    /// "C"), the longest match first. Whole cell: a cell equal to a "find"
    /// becomes its "replace". Inside cells: every occurrence.
    pub fn plan_batch_replace(
        &self,
        spec: &BatchReplace,
        preview: usize,
        progress: &Progress,
    ) -> Result<ReplacePlan, Error> {
        self.ready_for_bulk()?;
        if spec.pairs.len() > MAX_BATCH_PAIRS {
            return Err(Error::Invalid(format!(
                "at most {MAX_BATCH_PAIRS} pairs can be replaced at once"
            )));
        }
        let bytes: usize = spec.pairs.iter().map(|(a, b)| a.len() + b.len()).sum();
        if bytes > MAX_BATCH_BYTES {
            return Err(Error::Invalid("the list of pairs is too large".into()));
        }
        let pairs: Vec<&(String, String)> =
            spec.pairs.iter().filter(|(a, _)| !a.is_empty()).collect();
        if pairs.is_empty() {
            return Err(Error::Invalid("the list has nothing to look for".into()));
        }
        let find = FindSpec {
            pattern: Pattern::default(),
            scope: spec.scope.clone(),
            skip_first: spec.skip_first,
            skip_cols: spec
                .skip_cols
                .iter()
                .copied()
                .chain(self.protected_cols())
                .collect(),
            ..Default::default()
        };
        let area = self.area(&find)?;
        let key = |s: &str| -> String {
            if spec.match_case {
                s.to_string()
            } else {
                fold(s, None)
            }
        };
        if spec.whole_cell {
            // The first pair for a value wins.
            let mut map: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
            for (a, b) in &pairs {
                map.entry(key(a)).or_insert(b.as_str());
            }
            return self.plan_cells_with(&area, preview, progress, |t| {
                Ok(map.get(&key(t)).map(|b| (b.to_string(), 1)))
            });
        }
        // Case-blind: the pairs and each cell are lower-cased letter by
        // letter (any language, not just English), and matches are mapped
        // back to the cell's own text.
        let ac = aho_corasick::AhoCorasick::builder()
            .match_kind(aho_corasick::MatchKind::LeftmostLongest)
            .build(pairs.iter().map(|(a, _)| key(a)))
            .map_err(|e| Error::Invalid(format!("the list could not be used: {e}")))?;
        let blind = !spec.match_case;
        self.plan_cells_with(&area, preview, progress, |t| {
            let mut map = Vec::new();
            let folded;
            let hay: &str = if !blind {
                t
            } else if t.is_ascii() {
                folded = t.to_ascii_lowercase();
                &folded
            } else {
                folded = fold(t, Some(&mut map));
                &folded
            };
            // Where a match in `hay` starts or ends in `t` (None: inside one
            // letter's lower-case form, which is not a whole match).
            let back = |i: usize| -> Option<usize> {
                if map.is_empty() {
                    Some(i)
                } else {
                    map.get(i).copied().filter(|&p| p != usize::MAX)
                }
            };
            let mut out = String::new();
            let mut last = 0;
            let mut hits = 0u64;
            for m in ac.find_iter(hay) {
                let (Some(s), Some(e)) = (back(m.start()), back(m.end())) else {
                    continue;
                };
                out.push_str(&t[last..s]);
                out.push_str(&pairs[m.pattern().as_usize()].1);
                last = e;
                hits += 1;
                if out.len() > MAX_RESULT_CELL_BYTES {
                    return Err(Error::OutOfMemory(
                        "a replacement would make a cell too large",
                    ));
                }
            }
            if hits == 0 {
                return Ok(None);
            }
            out.push_str(&t[last..]);
            Ok(Some((out, hits)))
        })
    }

    /// Shown rows with at least one match (in order).
    pub fn matching_rows(&self, spec: &FindSpec, progress: &Progress) -> Result<Vec<u64>, Error> {
        let m = Matcher::new(&spec.pattern)?;
        let area = self.shown_area(spec)?;
        let (r0, r1) = (area.rows.start, area.rows.end);
        progress.start(r1.saturating_sub(r0));
        let blocks = r1.saturating_sub(r0).div_ceil(BLOCK);
        let parts: Vec<Vec<u64>> = (0..blocks)
            .into_par_iter()
            .map(|b| -> Result<Vec<u64>, Error> {
                let s = r0 + b * BLOCK;
                let e = (s + BLOCK).min(r1);
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for r in s..e {
                    if self
                        .each_cell(&area, r, false, &mut buf, |_, t| {
                            m.is_match(t).then_some(())
                        })
                        .is_some()
                    {
                        out.push(r);
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        Ok(parts.concat())
    }

    /// Show only the rows with a match (a view, undoable); the first row
    /// stays when it holds titles.
    pub fn plan_show_matching(
        &self,
        spec: &FindSpec,
        progress: &Progress,
    ) -> Result<crate::reshape::RowPlan, Error> {
        let rows = self.matching_rows(spec, progress)?;
        let ids: Vec<u32> = rows.iter().filter_map(|&r| self.id_at(r)).collect();
        Ok(self.plan_show_ids(&ids, spec.skip_first))
    }

    /// Show only these rows (ids; a view, undoable); the first row stays
    /// when it holds titles.
    pub fn plan_show_ids(&self, ids: &[u32], keep_first: bool) -> crate::reshape::RowPlan {
        let base = self.next_row_id();
        let mut bits = vec![0u64; (base as usize).div_ceil(64)];
        let mut set = |id: u32| {
            if id < base {
                bits[(id >> 6) as usize] |= 1 << (id & 63);
            }
        };
        if keep_first && self.row_count() > 0 {
            set(self.id_at(0).expect("a first row"));
        }
        for &id in ids {
            set(id);
        }
        let mut plan = crate::reshape::RowPlan::new(self);
        plan.rows = ids.len() as u64;
        plan.view = Some(crate::document::View::new(
            crate::filter::FilterSpec {
                keep_first,
                ..Default::default()
            },
            bits,
            base,
        ));
        plan
    }

    /// Delete the rows with a match (one undo step).
    pub fn plan_delete_matching(
        &self,
        spec: &FindSpec,
        progress: &Progress,
    ) -> Result<crate::reshape::RowPlan, Error> {
        self.ready_for_bulk()?;
        let rows = self.matching_rows(spec, progress)?;
        let mut plan = crate::reshape::RowPlan::new(self);
        plan.remove = rows.iter().filter_map(|&r| self.id_at(r)).collect();
        plan.rows = plan.remove.len() as u64;
        Ok(plan)
    }

    /// The rows with a match as a new table (with the title row first when
    /// there is one).
    pub fn matching_table(
        &self,
        spec: &FindSpec,
        progress: &Progress,
    ) -> Result<crate::reshape::Table, Error> {
        let rows = self.matching_rows(spec, progress)?;
        // Hidden columns (the search's skipped ones) are left out.
        let all: Vec<u32> = (0..self.col_count())
            .filter(|c| !spec.skip_cols.contains(c) && !spec.hidden_cols.contains(c))
            .collect();
        let cols = all.len() as u32;
        if cols == 0 {
            return Err(Error::Invalid("every column is hidden".into()));
        }
        let mut out = Vec::new();
        let mut line: Vec<String> = Vec::with_capacity(cols as usize);
        let mut buf = Vec::new();
        let mut write = |id: u32, out: &mut Vec<u8>| -> Result<(), Error> {
            let phys: Vec<u32> = all.iter().filter_map(|&c| self.phys(c)).collect();
            let mut sorted = phys.clone();
            sorted.sort_unstable();
            self.read_cells(id, &sorted, &mut buf);
            line.clear();
            for &c in &all {
                line.push(match self.phys(c) {
                    Some(p) => buf[sorted.binary_search(&p).expect("read")].to_string(),
                    None => String::new(),
                });
            }
            crate::reshape::push_line(out, &line);
            if out.len() > crate::reshape::MAX_TABLE_BYTES {
                return Err(Error::Invalid(
                    "the rows are too large for a new tab".into(),
                ));
            }
            Ok(())
        };
        let first = u64::from(spec.skip_first && self.row_count() > 0);
        if first == 1 {
            write(self.id_at(0).expect("a first row"), &mut out)?;
        }
        for &r in &rows {
            write(self.id_at(r).ok_or(Error::OutOfRange)?, &mut out)?;
        }
        Ok(crate::reshape::Table {
            bytes: out,
            rows: rows.len() as u64 + first,
            cols,
        })
    }

    /// Work out a replace over `area` with `f` giving each cell's new text
    /// and its number of matches (`None`: no match).
    fn plan_cells_with(
        &self,
        area: &Area,
        preview: usize,
        progress: &Progress,
        f: impl Fn(&str) -> Result<Option<(String, u64)>, Error> + Sync,
    ) -> Result<ReplacePlan, Error> {
        let (r0, r1) = (area.rows.start, area.rows.end);
        progress.start(r1.saturating_sub(r0));
        let created = AtomicUsize::new(0);
        type Part = (
            Vec<(u32, Vec<(u32, String)>)>,
            Vec<(u64, Vec<CellAt>)>,
            Vec<Change>,
            u64,
            u64,
            u64,
        );
        let blocks = r1.saturating_sub(r0).div_ceil(BLOCK);
        let parts: Vec<Part> = (0..blocks)
            .into_par_iter()
            .map(|b| -> Result<Part, Error> {
                let s = r0 + b * BLOCK;
                let e = (s + BLOCK).min(r1);
                let mut changes = Vec::new();
                let mut at = Vec::new();
                let mut shown = Vec::new();
                let (mut cells, mut hits, mut hidden_cells) = (0u64, 0u64, 0u64);
                let mut buf = Vec::new();
                for r in s..e {
                    let Some((id, shown_row, number, row_hidden)) = self.area_row(area, r) else {
                        continue;
                    };
                    let mut row: Vec<(u32, String, u32, u32, bool)> = Vec::new();
                    let mut err = None;
                    self.each_cell(area, r, false, &mut buf, |k, t| {
                        let phys = self.phys(area.cols[k])?;
                        match f(t) {
                            Ok(Some((new, n))) if new != t => {
                                if !charge(&created, new.len()) {
                                    err = Some(Error::OutOfMemory(
                                        "this replace would change too much at once; replace in a smaller selection",
                                    ));
                                    return Some(());
                                }
                                let hidden = row_hidden
                                    || area.hidden_cols.binary_search(&area.cols[k]).is_ok();
                                if shown.len() < preview {
                                    shown.push(Change {
                                        row: shown_row,
                                        number,
                                        col: area.cols[k],
                                        before: short(t),
                                        after: short(&new),
                                        hidden,
                                    });
                                }
                                cells += 1;
                                hits += n;
                                hidden_cells += u64::from(hidden);
                                row.push((
                                    phys,
                                    new,
                                    area.cols[k],
                                    u32::try_from(n).unwrap_or(u32::MAX),
                                    hidden,
                                ));
                                None
                            }
                            Ok(_) => None,
                            Err(e) => {
                                err = Some(e);
                                Some(())
                            }
                        }
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                    if !row.is_empty() {
                        row.sort_unstable_by_key(|c| c.0);
                        at.push((number, row.iter().map(|c| (c.2, c.3, c.4)).collect()));
                        changes.push((id, row.into_iter().map(|(p, s, _, _, _)| (p, s)).collect()));
                    }
                }
                progress.step(e - s)?;
                Ok((changes, at, shown, cells, hits, hidden_cells))
            })
            .collect::<Result<_, _>>()?;
        let mut plan = ReplacePlan {
            revision: self.revision(),
            changes: Vec::new(),
            at: Vec::new(),
            cells: 0,
            hidden_cells: 0,
            hits: 0,
            preview: Vec::new(),
        };
        for (c, a, p, cells, hits, hidden) in parts {
            plan.hidden_cells += hidden;
            plan.changes.extend(c);
            plan.at.extend(a);
            if plan.preview.len() < preview {
                plan.preview
                    .extend(p.into_iter().take(preview - plan.preview.len()));
            }
            plan.cells += cells;
            plan.hits += hits;
        }
        Ok(plan)
    }

    /// Changes `offset..offset + count` of a computed replace-all, in order,
    /// for paging through the whole preview (the document must not have
    /// changed since).
    pub fn replace_page(
        &self,
        plan: &ReplacePlan,
        offset: u64,
        count: usize,
    ) -> Result<Vec<Change>, Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        let mut out = Vec::new();
        let mut skip = offset;
        let mut buf = Vec::new();
        for ((id, cells), (number, cols)) in plan.changes.iter().zip(&plan.at) {
            let n = cells.len() as u64;
            if skip >= n {
                skip -= n;
                continue;
            }
            let wanted: Vec<u32> = cells.iter().map(|c| c.0).collect();
            self.read_cells(*id, &wanted, &mut buf);
            for (k, ((_, new), col)) in cells.iter().zip(cols).enumerate().skip(skip as usize) {
                if out.len() >= count {
                    return Ok(out);
                }
                let (row, shown) = self.shown_row_of(*number).unwrap_or((0, false));
                out.push(Change {
                    row,
                    number: *number,
                    col: col.0,
                    before: short(&buf[k]),
                    after: short(new),
                    hidden: col.2 || !shown,
                });
            }
            skip = 0;
            if out.len() >= count {
                break;
            }
        }
        Ok(out)
    }

    /// Apply a computed replace-all as one undo step. Fails with
    /// [`Error::Changed`] if the document changed since it was computed.
    pub fn apply_replace(&mut self, plan: ReplacePlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        self.patch_rows(plan.changes)
    }

    /// Replace every match in the searched cells; returns (cells, matches).
    pub fn replace_all(&mut self, spec: &FindSpec, replacement: &str) -> Result<(u64, u64), Error> {
        let plan = self.plan_replace(spec, replacement, 0, &Progress::new())?;
        let counts = (plan.cells, plan.hits);
        self.apply_replace(plan)?;
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn pat(text: &str, kind: PatternKind) -> Pattern {
        Pattern {
            text: text.into(),
            kind,
            ..Default::default()
        }
    }

    fn spec(p: Pattern) -> FindSpec {
        FindSpec {
            pattern: p,
            scope: Scope::All,
            skip_first: false,
            skip_cols: Vec::new(),
            ..Default::default()
        }
    }

    fn with(text: &str, kind: PatternKind, f: impl FnOnce(&mut Pattern)) -> Matcher {
        let mut p = pat(text, kind);
        f(&mut p);
        Matcher::new(&p).unwrap()
    }

    #[test]
    fn counts_reuse_the_blocks_kept() {
        let mut body = String::from("a\n");
        for i in 0..(BLOCK * 2 + 10) {
            body.push_str(if i % 3 == 0 { "x\n" } else { "y\n" });
        }
        let d = doc(&body);
        let s = spec(pat("x", PatternKind::Plain));
        let p = Progress::new();
        let mut kept = None;
        let at = Some((BLOCK + 4, 0));
        let full = d.find_count(&s, at, &p).unwrap();
        let first = d.find_count_with(&s, at, &mut kept, &p).unwrap();
        assert_eq!(first, full);
        assert!(kept.is_some());
        for row in [1, BLOCK + 4, BLOCK * 2 + 9, BLOCK * 2 + 10] {
            let at = Some((row, 0));
            assert_eq!(
                d.find_count_with(&s, at, &mut kept, &p).unwrap(),
                d.find_count(&s, at, &p).unwrap(),
                "row {row}"
            );
        }
    }

    #[test]
    fn find_all_counts_per_column() {
        let d = doc("a,b,c\nx,ax,b\nax,c,x\n");
        let r = d
            .find_all(&spec(pat("x", PatternKind::Plain)), 2, &Progress::new())
            .unwrap();
        assert_eq!((r.cells, r.hits.len()), (4, 2));
        assert_eq!(r.per_col, vec![(0, 2), (1, 1), (2, 1)]);
    }

    #[test]
    fn replace_preview_pages() {
        let d = doc("a,b\nx1,x2\nq,x3\nx4,q\n");
        let p = Progress::new();
        let plan = d
            .plan_replace(&spec(pat("x", PatternKind::Plain)), "y", 1, &p)
            .unwrap();
        assert_eq!(plan.preview.len(), 1);
        let all = d.replace_page(&plan, 0, 10).unwrap();
        let cells: Vec<(u64, u32, &str, &str)> = all
            .iter()
            .map(|c| (c.row, c.col, c.before.as_str(), c.after.as_str()))
            .collect();
        assert_eq!(
            cells,
            vec![
                (1, 0, "x1", "y1"),
                (1, 1, "x2", "y2"),
                (2, 1, "x3", "y3"),
                (3, 0, "x4", "y4")
            ]
        );
        let page = d.replace_page(&plan, 1, 2).unwrap();
        assert_eq!(
            page.iter().map(|c| c.after.as_str()).collect::<Vec<_>>(),
            vec!["y2", "y3"]
        );
        assert!(d.replace_page(&plan, 9, 2).unwrap().is_empty());
    }

    #[test]
    fn counts_and_marks() {
        let d = doc("a,b,c\nx,ax,b\nax,c,x\n");
        let s = spec(pat("x", PatternKind::Plain));
        let p = Progress::new();
        let c = d.find_count(&s, Some((1, 1)), &p).unwrap();
        assert_eq!(
            c,
            FindCount {
                cells: 4,
                before: 1,
                here: true
            }
        );
        let c = d.find_count(&s, Some((2, 1)), &p).unwrap();
        assert_eq!((c.before, c.here), (3, false));
        assert_eq!(d.find_count(&s, None, &p).unwrap().before, 0);
        assert_eq!(
            d.find_marks(&s, 0, 3, 0, 3).unwrap(),
            vec![(1, 0), (1, 1), (2, 0), (2, 2)]
        );
        assert_eq!(d.find_marks(&s, 2, 5, 1, 5).unwrap(), vec![(2, 2)]);
        let mut hid = s.clone();
        hid.skip_cols = vec![0];
        assert_eq!(d.find_count(&hid, None, &p).unwrap().cells, 2);
    }

    #[test]
    fn special_characters_show_in_lists() {
        assert_eq!(visible("a\r\nb\tc\u{a0}d\ne"), "a⏎b→c·d⏎e");
        let (t, s, l) = snippet("one\r\ntwo\tthree", 5..8);
        assert_eq!((t.as_str(), s, l), ("one⏎two→three", 4, 3));
    }

    #[test]
    fn whole_word() {
        let m = with("Ann", PatternKind::Plain, |p| p.whole_word = true);
        assert!(m.is_match("Ann Lee") && m.is_match("Lee, Ann") && m.is_match("ann"));
        assert!(!m.is_match("Annette") && !m.is_match("Joann"));
        // A pattern ending in a non-letter still works ("Co." in "Smith Co.").
        let m = with("Co.", PatternKind::Plain, |p| p.whole_word = true);
        assert!(m.is_match("Smith Co.") && !m.is_match("Costco."));
        let m = with("an*", PatternKind::Wildcard, |p| p.whole_word = true);
        assert!(m.is_match("Lee Anna") && !m.is_match("Joanna"));
        let m = with("[0-9]+", PatternKind::Regex, |p| p.whole_word = true);
        assert!(m.is_match("Apt 12") && !m.is_match("A12"));
    }

    #[test]
    fn ignore_accents() {
        let m = with("Jose", PatternKind::Plain, |p| p.ignore_accents = true);
        assert!(m.is_match("José") && m.is_match("JOSÉ") && m.is_match("Jose"));
        // "e" followed by a separate combining accent.
        assert!(m.is_match("Jose\u{301}"));
        let m = with("Jose", PatternKind::Plain, |p| {
            p.ignore_accents = true;
            p.match_case = true;
        });
        assert!(m.is_match("José") && !m.is_match("JOSÉ"));
        // Accented letters in the pattern match the plain ones too.
        let m = with("Zoë", PatternKind::Plain, |p| p.ignore_accents = true);
        assert!(m.is_match("Zoe") && m.is_match("ZOË"));
        // In a regular expression, classes and escapes stay as they are.
        let m = with(r"^Mu[nñ]oz\d?$", PatternKind::Regex, |p| {
            p.ignore_accents = true
        });
        assert!(m.is_match("Muñoz") && m.is_match("Múnoz1") && !m.is_match("Munozz"));
        // \pL (short class) and "]" first in a class stay as they are.
        let m = with(r"^\pL+o$", PatternKind::Regex, |p| p.ignore_accents = true);
        assert!(m.is_match("Pedró") && !m.is_match("12o"));
        let m = with(r"[]a]x", PatternKind::Regex, |p| p.ignore_accents = true);
        assert!(m.is_match("]x") && m.is_match("ax") && !m.is_match("bx"));
        let m = with("caf?", PatternKind::Wildcard, |p| p.ignore_accents = true);
        assert!(m.is_match("Càfe"));
        // Off, accents count.
        assert!(!with("Jose", PatternKind::Plain, |_| {}).is_match("José"));
        // A very long text with accents ignored is refused with a clear reason.
        let mut long = pat(&"abcdefghij".repeat(200), PatternKind::Plain);
        long.ignore_accents = true;
        if let Err(e) = Matcher::new(&long) {
            assert!(e.to_string().contains("Ignore accents"), "{e}");
        }
        // Replacing keeps the rest of the cell.
        let m = with("jose", PatternKind::Plain, |p| p.ignore_accents = true);
        assert_eq!(m.replace("Dr. José", "Joe").unwrap().unwrap().0, "Dr. Joe");
    }

    #[test]
    fn escapes() {
        let m = with(r"a\tb", PatternKind::Plain, |p| p.escapes = true);
        assert!(m.is_match("a\tb") && !m.is_match(r"a\tb"));
        let m = with(r"a\tb", PatternKind::Plain, |_| {});
        assert!(m.is_match(r"a\tb") && !m.is_match("a\tb"));
        let m = with(r"x\n*", PatternKind::Wildcard, |p| p.escapes = true);
        assert!(m.is_match("x\nmore"));
        let m = with(r"\u{e9}", PatternKind::Plain, |p| p.escapes = true);
        assert!(m.is_match("é"));
        // The replacement too: a line break becomes a space, and back.
        let m = with(r"\n", PatternKind::Plain, |p| p.escapes = true);
        assert_eq!(m.replace("a\nb", " ").unwrap().unwrap().0, "a b");
        let m = with(" ", PatternKind::Plain, |p| p.escapes = true);
        assert_eq!(m.replace("a b", r"\n").unwrap().unwrap().0, "a\nb");
        // Without the option, a replacement backslash is kept.
        let m = with(" ", PatternKind::Plain, |_| {});
        assert_eq!(m.replace("a b", r"\n").unwrap().unwrap().0, r"a\nb");
        assert_eq!(unescape(r"a\\b\q\u{zz}"), r"a\b\q\u{zz}");
    }

    #[test]
    fn replace_preview_leaves_protected_columns_out() {
        let mut d = doc("a,b
x,x
x,q
");
        d.set_protected(&[0]);
        let p = Progress::new();
        let plan = d
            .plan_replace(&spec(pat("x", PatternKind::Plain)), "y", 10, &p)
            .unwrap();
        assert_eq!(plan.cells, 1);
    }

    #[test]
    fn unticked_changes_are_left_out() {
        let mut d = doc("a,b\nx,x\nx,q\n");
        let p = Progress::new();
        let mut plan = d
            .plan_replace(&spec(pat("x", PatternKind::Plain)), "y", 0, &p)
            .unwrap();
        plan.exclude(&[(1, 1), (2, 0)]);
        assert_eq!(plan.cells, 1);
        d.apply_replace(plan).unwrap();
        assert_eq!(
            all(&d),
            vec![vec!["a", "b"], vec!["y", "x"], vec!["x", "q"]]
        );
    }

    #[test]
    fn act_on_matching_rows() {
        let mut d = doc("k,v\nx1,a\nb,c\nd,x2\n");
        let mut s = spec(pat("x", PatternKind::Plain));
        s.skip_first = true;
        let p = Progress::new();
        assert_eq!(d.matching_rows(&s, &p).unwrap(), vec![1, 3]);
        let t = d.matching_table(&s, &p).unwrap();
        assert_eq!(
            String::from_utf8(t.bytes).unwrap().replace("\r\n", "\n"),
            "k,v\nx1,a\nd,x2\n"
        );
        let plan = d.plan_show_matching(&s, &p).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 3);
        d.clear_filter();
        let plan = d.plan_delete_matching(&s, &p).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 2);
        assert!(d.undo());
        assert_eq!(d.row_count(), 4);
    }

    #[test]
    fn numbers_and_dates_by_value() {
        let m = |t: &str| with(t, PatternKind::Number, |_| {});
        let five = m("5");
        for s in ["5", "5.00", "$5", " 5 ", "5.0"] {
            assert!(five.is_match(s), "{s}");
        }
        for s in ["15", "5a", "", "five"] {
            assert!(!five.is_match(s), "{s}");
        }
        assert!(m("10..20").is_match("$12.50") && !m("10..20").is_match("20.01"));
        assert!(m(">5").is_match("5.01") && !m(">5").is_match("5"));
        assert!(m(">=5").is_match("5") && m("<=5").is_match("5") && !m("<5").is_match("5"));
        assert!(m("(3)").is_match("-3"));
        for bad in ["abc", "20..10", "", ">"] {
            let p = pat(bad, PatternKind::Number);
            assert!(Matcher::new(&p).is_err(), "{bad}");
        }
        let d = |t: &str| with(t, PatternKind::Date, |_| {});
        let jan5 = d("1/5/2024");
        for s in ["2024-01-05", "Jan 5 2024", "1/5/2024 14:30"] {
            assert!(jan5.is_match(s), "{s}");
        }
        assert!(!jan5.is_match("2024-01-06") && !jan5.is_match("x"));
        assert!(d("1/1/2024..1/31/2024").is_match("2024-01-31 23:00"));
        assert!(!d("1/1/2024..1/31/2024").is_match("2024-02-01"));
        assert!(
            d(">1/5/2024").is_match("2024-01-06") && !d(">1/5/2024").is_match("2024-01-05 18:00")
        );
        assert!(d("<=1/5/2024").is_match("2024-01-05 18:00"));
        assert!(Matcher::new(&pat("13/45/2024", PatternKind::Date)).is_err());
        // The whole cell is replaced, and only matching cells.
        assert_eq!(five.replace("$5.00", "6").unwrap().unwrap().0, "6");
        assert!(five.replace("7", "6").unwrap().is_none());
    }

    #[test]
    fn similar_names_and_preserve_case() {
        let s = with("Jon Smyth", PatternKind::Similar, |_| {});
        for y in ["John Smith", "jon smyth", "Jon-Smyth", "Jön Smyth"] {
            assert!(s.is_match(y), "{y}");
        }
        for n in ["Jane Smithers", "Smith", "Jon Smyth Jr Esq the Third", ""] {
            assert!(!s.is_match(n), "{n}");
        }
        assert!(with("Ann", PatternKind::Similar, |_| {}).is_match("Anne"));
        // Short searches match exactly (case and accents aside), and a blank
        // or punctuation-only cell never matches.
        let one = with("A", PatternKind::Similar, |_| {});
        for y in ["a", "Á", " a. "] {
            assert!(one.is_match(y), "{y}");
        }
        for n in ["", "-", "b", "ab", "  "] {
            assert!(!one.is_match(n), "{n:?}");
        }
        assert!(!with("Al", PatternKind::Similar, |_| {}).is_match("Ab"));
        assert!(!with("Jon Smyth", PatternKind::Similar, |_| {}).is_match(""));
        // Case marks win over Preserve case.
        let m = with(r"(\w+)", PatternKind::Regex, |p| p.preserve_case = true);
        assert_eq!(m.replace("SMITH", r"\L${1}x").unwrap().unwrap().0, "smithx");
        assert_eq!(m.replace("SMITH", "jones").unwrap().unwrap().0, "JONES");
        // Value patterns put the replacement in as typed.
        let v = with("5", PatternKind::Number, |p| p.escapes = true);
        assert_eq!(v.replace("5", r"a\tb").unwrap().unwrap().0, r"a\tb");
        assert!(!with("Ann", PatternKind::Similar, |_| {}).is_match("Annette"));
        let m = with("smith", PatternKind::Plain, |p| p.preserve_case = true);
        assert_eq!(
            m.replace("SMITH, Smith, smith, sMITh", "jones")
                .unwrap()
                .unwrap()
                .0,
            "JONES, Jones, jones, jones"
        );
        let m = with("smith", PatternKind::Plain, |p| p.preserve_case = false);
        assert_eq!(m.replace("SMITH", "jones").unwrap().unwrap().0, "jones");
    }

    fn filter_a(d: &mut Document) {
        use crate::filter::{ColumnFilter, FilterSpec, Rule};
        d.set_filter(&FilterSpec {
            columns: vec![ColumnFilter {
                col: 0,
                rule: Rule::Values {
                    values: vec!["a".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
    }

    #[test]
    fn hidden_rows_are_counted_listed_and_replaced_when_asked() {
        let mut d = doc("k,v\na,x\nb,x\na,x\nb,y\n");
        filter_a(&mut d);
        let p = Progress::new();
        let mut s = spec(pat("x", PatternKind::Plain));
        s.skip_first = true;
        assert_eq!(d.find_count(&s, None, &p).unwrap().cells, 2);
        s.hidden_rows = true;
        // Shown row 2 is document row 3: one hidden match (row 2) before it.
        let c = d.find_count(&s, Some((2, 1)), &p).unwrap();
        assert_eq!((c.cells, c.before, c.here), (3, 2, true));
        let all = d.find_all(&s, 10, &p).unwrap();
        let got: Vec<(u64, u64, bool)> = all
            .hits
            .iter()
            .map(|h| (h.row, h.number, h.hidden))
            .collect();
        assert_eq!(got, vec![(1, 1, false), (2, 2, true), (2, 3, false)]);
        // Find next and the lit-up matches stay on shown rows.
        assert_eq!(
            d.find_next(&s, Some((1, 1)), false, &p).unwrap(),
            Some(((2, 1), false))
        );
        assert_eq!(
            d.find_marks(&s, 0, 10, 0, 10).unwrap(),
            vec![(1, 1), (2, 1)]
        );
        let mut plan = d.plan_replace(&s, "z", 10, &p).unwrap();
        assert_eq!((plan.cells, plan.hidden_cells), (3, 1));
        assert!(plan.preview[1].hidden && plan.preview[1].number == 2);
        assert_eq!(d.replace_page(&plan, 1, 1).unwrap()[0].number, 2);
        // Unticking goes by row number in the document.
        plan.exclude(&[(2, 1)]);
        assert_eq!((plan.cells, plan.hidden_cells), (2, 0));
        d.apply_replace(plan).unwrap();
        d.clear_filter();
        assert_eq!(
            all_cells(&d),
            vec![
                vec!["k", "v"],
                vec!["a", "z"],
                vec!["b", "x"],
                vec!["a", "z"],
                vec!["b", "y"]
            ]
        );
        // Hidden columns that are searched are counted as hidden.
        let mut s = spec(pat("z", PatternKind::Plain));
        s.hidden_cols = vec![1];
        assert_eq!(d.plan_replace(&s, "q", 0, &p).unwrap().hidden_cells, 2);
    }

    #[test]
    fn find_next_by_columns() {
        let d = doc("a,b\nx,x\nx,.\n.,x\n");
        let p = Progress::new();
        let mut s = spec(pat("x", PatternKind::Plain));
        s.by_columns = true;
        let mut at = None;
        let mut seen = Vec::new();
        for _ in 0..5 {
            let ((r, c), wrapped) = d.find_next(&s, at, false, &p).unwrap().unwrap();
            seen.push((r, c, wrapped));
            at = Some((r, c));
        }
        assert_eq!(
            seen,
            vec![
                (1, 0, false),
                (2, 0, false),
                (1, 1, false),
                (3, 1, false),
                (1, 0, true)
            ]
        );
        let back = d.find_next(&s, Some((1, 1)), true, &p).unwrap().unwrap();
        assert_eq!(back, ((2, 0), false));
        // Outside the searched columns: backwards starts in the column to
        // the left; forwards past the last one starts over.
        let d3 = doc("a,b,c\nx,.,x\nx,.,x\n");
        let mut s3 = spec(pat("x", PatternKind::Plain));
        s3.by_columns = true;
        s3.scope = Scope::Columns { cols: vec![0, 2] };
        assert_eq!(
            d3.find_next(&s3, Some((1, 1)), true, &p).unwrap(),
            Some(((2, 0), false))
        );
        assert_eq!(
            d3.find_next(&s3, Some((1, 1)), false, &p).unwrap(),
            Some(((1, 2), false))
        );
        s3.scope = Scope::Columns { cols: vec![0] };
        assert_eq!(
            d3.find_next(&s3, Some((1, 2)), false, &p).unwrap(),
            Some(((1, 0), true))
        );
        assert_eq!(
            d3.find_next(&s3, Some((1, 2)), true, &p).unwrap(),
            Some(((2, 0), false))
        );
        s3.scope = Scope::Columns { cols: vec![2] };
        assert_eq!(
            d3.find_next(&s3, Some((1, 0)), true, &p).unwrap(),
            Some(((2, 2), true))
        );
        // Kept counts in column order agree with a fresh count.
        let big = doc(&format!("a,b\n{}", "x,x\n.,x\n".repeat(20_000)));
        let mut sb = spec(pat("x", PatternKind::Plain));
        sb.by_columns = true;
        let mut kept = None;
        for at in [(1, 0), (30_001, 0), (5, 1), (39_999, 1), (40_000, 1)] {
            let fresh = big.find_count(&sb, Some(at), &p).unwrap();
            let cached = big.find_count_with(&sb, Some(at), &mut kept, &p).unwrap();
            assert_eq!(fresh, cached, "{at:?}");
        }
        assert!(kept.is_some());
        // "3 of 4" in column order.
        let c = d.find_count(&s, Some((1, 1)), &p).unwrap();
        assert_eq!((c.before, c.here), (2, true));
    }

    fn all_cells(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| {
                (0..d.col_count())
                    .map(|c| d.cell(r, c).unwrap_or_default())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn batch_replace_case_blind_in_any_language() {
        let d = doc("a\nJOSÉ Ruiz\nmaría josé\nİx\nΣΟΦΙΑ\n");
        let p = Progress::new();
        let spec = |pairs: Vec<(&str, &str)>| BatchReplace {
            pairs: pairs
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            scope: Scope::All,
            skip_first: true,
            ..Default::default()
        };
        let plan = d
            .plan_batch_replace(&spec(vec![("josé", "J"), ("σοφια", "S")]), 10, &p)
            .unwrap();
        let got: Vec<&str> = plan.preview.iter().map(|c| c.after.as_str()).collect();
        assert_eq!(got, vec!["J Ruiz", "maría J", "S"]);
        // A letter whose lower-case form is longer ("İ" is "i̇") is matched
        // as a whole and the text around it kept.
        let plan = d
            .plan_batch_replace(&spec(vec![("x", "y")]), 10, &p)
            .unwrap();
        assert_eq!(plan.preview[0].after, "İy");
        let plan = d
            .plan_batch_replace(&spec(vec![("i", "Q")]), 10, &p)
            .unwrap();
        assert!(plan.preview.iter().all(|c| !c.after.contains("Q̇")));
    }

    #[test]
    fn copied_rows_leave_hidden_columns_out_and_untick_lowers_hits() {
        let d = doc("k,secret,v\nx1,111,a\nb,222,c\n");
        let p = Progress::new();
        let mut s = spec(pat("x", PatternKind::Plain));
        s.skip_first = true;
        s.skip_cols = vec![1];
        let t = d.matching_table(&s, &p).unwrap();
        assert_eq!(t.cols, 2);
        // Include hidden: the hidden column is searched but still not copied.
        s.skip_cols = Vec::new();
        s.hidden_cols = vec![1];
        assert_eq!(d.matching_table(&s, &p).unwrap().cols, 2);
        assert_eq!(
            String::from_utf8(t.bytes).unwrap().replace("\r\n", "\n"),
            "k,v\nx1,a\n"
        );
        let d = doc("a\nxx\nx\n");
        let mut plan = d
            .plan_replace(&spec(pat("x", PatternKind::Plain)), "y", 0, &p)
            .unwrap();
        assert_eq!((plan.cells, plan.hits), (2, 3));
        plan.exclude(&[(1, 0)]);
        assert_eq!((plan.cells, plan.hits), (1, 1));
    }

    #[test]
    fn batch_replace_all_at_once() {
        let d = doc("a,b\nA,x B y\nB,C\nzz,ab\n");
        let p = Progress::new();
        let pairs = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("ab".to_string(), "Q".to_string()),
        ];
        // Whole cell, case blind: A -> B, B -> C (never A -> C).
        let spec = BatchReplace {
            pairs: pairs.clone(),
            whole_cell: true,
            scope: Scope::All,
            skip_first: true,
            ..Default::default()
        };
        let plan = d.plan_batch_replace(&spec, 10, &p).unwrap();
        let got: Vec<(u64, &str, &str)> = plan
            .preview
            .iter()
            .map(|c| (c.row, c.before.as_str(), c.after.as_str()))
            .collect();
        assert_eq!(got, vec![(1, "A", "B"), (2, "B", "C"), (3, "ab", "Q")]);
        // Inside cells, match case, longest first.
        let spec = BatchReplace {
            pairs,
            match_case: true,
            scope: Scope::All,
            skip_first: true,
            ..Default::default()
        };
        let plan = d.plan_batch_replace(&spec, 10, &p).unwrap();
        let got: Vec<&str> = plan.preview.iter().map(|c| c.after.as_str()).collect();
        assert_eq!(got, vec!["B", "x C y", "C", "Q"]);
        // Limits.
        let big = BatchReplace {
            pairs: vec![("a".into(), "b".into()); MAX_BATCH_PAIRS + 1],
            ..Default::default()
        };
        assert!(d.plan_batch_replace(&big, 0, &p).is_err());
    }

    #[test]
    fn replacement_case_marks() {
        let m = with(r"(\w+) (\w+)", PatternKind::Regex, |_| {});
        let r = |rep: &str, s: &str| m.replace(s, rep).unwrap().unwrap().0;
        assert_eq!(r(r"\U$1\E $2", "ada lovelace"), "ADA lovelace");
        assert_eq!(r(r"\u$2, \u$1", "ada lovelace"), "Lovelace, Ada");
        assert_eq!(r(r"\L$0", "ADA LOVELACE"), "ada lovelace");
        assert_eq!(r(r"\l$1", "ADA X"), "aDA");
        // A group's own text is never read as a mark.
        let m = with(r"(.+)", PatternKind::Regex, |_| {});
        assert_eq!(m.replace(r"a\Ub", r"\U$1").unwrap().unwrap().0, r"A\UB");
        // A doubled backslash is a literal one, never a mark.
        assert_eq!(m.replace("ab", r"\\U$1").unwrap().unwrap().0, r"\Uab");
        assert_eq!(m.replace("ab", r"\\\U$1").unwrap().unwrap().0, r"\AB");
        // Plain text replacements are taken as they are.
        let m = with("x", PatternKind::Plain, |_| {});
        assert_eq!(m.replace("x", r"\Uy").unwrap().unwrap().0, r"\Uy");
    }

    #[test]
    fn replacement_names_missing_groups() {
        let m = with(r"(\d+)", PatternKind::Regex, |_| {});
        assert!(m.check_replacement("$1 and ${1}a and $$2").is_ok());
        let e = m.check_replacement("$1a").unwrap_err().to_string();
        assert!(e.contains("${1}a"), "{e}");
        assert!(m.check_replacement("$2").is_err());
        let m = with(r"(?P<zip>\d{5})", PatternKind::Regex, |_| {});
        assert!(m.check_replacement("$zip").is_ok() && m.check_replacement("$zap").is_err());
        // Non-ASCII letters end a name ($1é is group 1 then "é"); an
        // unclosed "${" is plain text.
        let m = with(r"(\d+)", PatternKind::Regex, |_| {});
        assert!(m.check_replacement("$1é").is_ok() && m.check_replacement("${1").is_ok());
        // Plain text never has groups to check.
        assert!(with("a", PatternKind::Plain, |_| {})
            .check_replacement("$9")
            .is_ok());
        let d = doc("a\n12\n");
        let e = d.plan_replace(
            &spec(pat(r"(\d)", PatternKind::Regex)),
            "$1x",
            0,
            &Progress::new(),
        );
        assert!(e.is_err());
    }

    fn all(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    #[test]
    fn matcher_kinds() {
        let m = Matcher::new(&pat("a.c", PatternKind::Plain)).unwrap();
        assert!(m.is_match("xa.cx") && !m.is_match("abc"));
        let m = Matcher::new(&pat("a*c?", PatternKind::Wildcard)).unwrap();
        assert!(m.is_match("abbbcd") && !m.is_match("ab"));
        let m = Matcher::new(&pat("5~*", PatternKind::Wildcard)).unwrap();
        assert!(m.is_match("5*") && !m.is_match("55"));
        let m = Matcher::new(&Pattern {
            text: "ABC".into(),
            match_case: true,
            ..Default::default()
        })
        .unwrap();
        assert!(!m.is_match("abc"));
        let m = Matcher::new(&Pattern {
            text: "ab".into(),
            whole_cell: true,
            ..Default::default()
        })
        .unwrap();
        assert!(m.is_match("AB") && !m.is_match("abc"));
        assert!(matches!(
            Matcher::new(&pat("(", PatternKind::Regex)),
            Err(Error::BadPattern(_))
        ));
        assert!(Matcher::new(&pat("", PatternKind::Plain)).is_err());
        // Huge repetition counts are refused, not compiled forever.
        assert!(Matcher::new(&pat("(a{1000}){1000}", PatternKind::Regex)).is_err());
    }

    #[test]
    fn replace_rules() {
        let m = Matcher::new(&pat(r"(\w+)@(\w+)", PatternKind::Regex)).unwrap();
        assert_eq!(
            m.replace("x a@b y c@d", "$2 at $1").unwrap(),
            Some(("x b at a y d at c".into(), 2))
        );
        let m = Matcher::new(&pat("$1", PatternKind::Plain)).unwrap();
        assert_eq!(
            m.replace("cost $1", "$2").unwrap(),
            Some(("cost $2".into(), 1))
        );
        let m = Matcher::new(&pat("x*", PatternKind::Regex)).unwrap();
        assert_eq!(m.replace("abc", "-").unwrap(), None);
        assert_eq!(m.replace("axxb", "-").unwrap(), Some(("a-b".into(), 1)));
        let m = Matcher::new(&pat("^$", PatternKind::Regex)).unwrap();
        assert_eq!(m.replace("", "N/A").unwrap(), Some(("N/A".into(), 1)));
        // Growth is bounded.
        let m = Matcher::new(&pat("a", PatternKind::Plain)).unwrap();
        let big = "a".repeat(1000);
        let rep = "b".repeat(60_000);
        assert!(m.replace(&big, &rep).is_err());
    }

    #[test]
    fn find_in_text_utf16() {
        let m = Matcher::new(&pat("b", PatternKind::Plain)).unwrap();
        assert_eq!(m.find_in_text("😀b xb", 10), vec![(2, 3), (5, 6)]);
        assert_eq!(m.find_in_text("bbb", 2).len(), 2);
    }

    #[test]
    fn find_next_wraps_and_goes_back() {
        let d = doc("a,b\r\nx,a\r\nb,a\r\n");
        let s = spec(pat("a", PatternKind::Plain));
        let p = Progress::new();
        assert_eq!(
            d.find_next(&s, None, false, &p).unwrap(),
            Some(((0, 0), false))
        );
        assert_eq!(
            d.find_next(&s, Some((0, 0)), false, &p).unwrap(),
            Some(((1, 1), false))
        );
        assert_eq!(
            d.find_next(&s, Some((2, 1)), false, &p).unwrap(),
            Some(((0, 0), true))
        );
        assert_eq!(
            d.find_next(&s, Some((1, 1)), true, &p).unwrap(),
            Some(((0, 0), false))
        );
        assert_eq!(
            d.find_next(&s, Some((0, 0)), true, &p).unwrap(),
            Some(((2, 1), true))
        );
        let one = spec(pat("x", PatternKind::Plain));
        assert_eq!(
            d.find_next(&one, Some((1, 0)), false, &p).unwrap(),
            Some(((1, 0), true))
        );
        let none = spec(pat("zzz", PatternKind::Plain));
        assert_eq!(d.find_next(&none, None, false, &p).unwrap(), None);
    }

    #[test]
    fn find_all_and_scopes() {
        let d = doc("name,city\r\nann,paris\r\nbob,Paris\r\nannie,rome\r\n");
        let p = Progress::new();
        let mut s = spec(pat("an", PatternKind::Plain));
        let r = d.find_all(&s, 10, &p).unwrap();
        assert_eq!(r.cells, 2);
        assert_eq!((r.hits[0].row, r.hits[0].col), (1, 0));
        s.pattern.text = "paris".into();
        s.scope = Scope::Columns { cols: vec![1] };
        assert_eq!(d.find_all(&s, 10, &p).unwrap().cells, 2);
        s.pattern.match_case = true;
        assert_eq!(d.find_all(&s, 10, &p).unwrap().cells, 1);
        s.scope = Scope::Range {
            row: 2,
            rows: 5,
            col: 0,
            cols: 2,
        };
        assert_eq!(d.find_all(&s, 10, &p).unwrap().cells, 0);
        let r = d
            .find_all(&spec(pat("a", PatternKind::Plain)), 1, &p)
            .unwrap();
        assert_eq!((r.hits.len(), r.cells), (1, 5));
        // Hidden columns are left out of every scope.
        let mut hid = spec(pat("a", PatternKind::Plain));
        hid.skip_cols = vec![0];
        let r = d.find_all(&hid, 10, &p).unwrap();
        assert!(r.hits.iter().all(|h| h.col != 0));
        hid.scope = Scope::Range {
            row: 0,
            rows: 9,
            col: 0,
            cols: 2,
        };
        assert!(d
            .find_all(&hid, 10, &p)
            .unwrap()
            .hits
            .iter()
            .all(|h| h.col != 0));
    }

    #[test]
    fn replace_all_is_one_undo_step_and_saves() {
        let src = "id,name\r\n1,ann\r\n2,\"bob, jr\"\r\n3,ann-marie\r\n";
        let mut d = doc(src);
        let mut s = spec(pat("ann", PatternKind::Plain));
        s.skip_first = true;
        let p = Progress::new();
        let plan = d.plan_replace(&s, "Anne", 5, &p).unwrap();
        assert_eq!((plan.cells, plan.hits, plan.preview.len()), (2, 2, 2));
        assert_eq!(plan.preview[0].after, "Anne");
        d.apply_replace(plan).unwrap();
        assert_eq!(d.cell(3, 1).unwrap(), "Anne-marie");
        assert_eq!(d.cell(2, 1).unwrap(), "bob, jr");
        // A cell edit on a patched row keeps the patch's change.
        d.set_cell(1, 0, "10".into()).unwrap();
        assert_eq!(d.row_values(1).unwrap(), ["10", "Anne"]);
        d.undo();
        d.undo();
        assert_eq!(all(&d)[1], ["1", "ann"]);
        assert!(!d.is_modified());
        d.redo();
        let dir = std::env::temp_dir().join(format!("nd-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.csv");
        std::fs::write(&path, src).unwrap();
        let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        d.replace_all(&s, "Anne").unwrap();
        d.save(crate::SaveOptions::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "id,name\r\n1,Anne\r\n2,\"bob, jr\"\r\n3,Anne-marie\r\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- Ghost diff: the preview in the grid and the real Replace all are the same routine ----

    /// A random grid of short texts as CSV (some cells have case differences and groups to capture).
    fn ghost_table(r: &mut impl FnMut() -> u64, rows: usize, cols: usize) -> String {
        const BITS: [&str; 8] = ["ab", "a", "b", "", "Ab-1", "xab xab", "12", "AB"];
        let mut csv = String::new();
        for _ in 0..rows {
            let line: Vec<&str> = (0..cols).map(|_| BITS[(r() % 8) as usize]).collect();
            csv.push_str(&line.join(","));
            csv.push_str("\r\n");
        }
        csv
    }

    #[test]
    fn ghosts_and_the_count_equal_what_replace_all_plans_for_every_cell_of_the_window() {
        let mut r = lcg(2026);
        let p = Progress::new();
        let cases: Vec<(Pattern, &str)> = vec![
            (pat("a", PatternKind::Plain), "XY"),
            (Pattern { match_case: true, ..pat("a", PatternKind::Plain) }, "Q"),
            (Pattern { whole_cell: true, ..pat("ab", PatternKind::Plain) }, "zz"),
            (pat("(a)(b)", PatternKind::Regex), "$2$1"),
            (pat("^$", PatternKind::Regex), "filled"),
            (pat("a?", PatternKind::Wildcard), "-"),
            (pat("b", PatternKind::Plain), "b"),
            (pat("[0-9]+", PatternKind::Regex), ""),
        ];
        for round in 0..40 {
            let (pattern, rep) = cases[round % cases.len()].clone();
            let rows = 6 + (r() % 30) as usize;
            let mut d = doc(&ghost_table(&mut r, rows, 5));
            // hidden rows, a protected column and skipped columns
            if round % 3 == 1 {
                d.hide_rows(2 + r() % 3, 2 + r() % 4).unwrap();
            }
            if round % 4 == 2 {
                d.set_protected(&[4]);
            }
            let mut s = spec(pattern);
            s.skip_first = round % 2 == 0;
            if round % 5 == 3 {
                s.skip_cols = vec![1];
            }
            match round % 3 {
                0 => {}
                1 => s.scope = Scope::Columns { cols: vec![0, 2, 3] },
                _ => {
                    s.scope = Scope::Areas {
                        areas: vec![
                            Block { row: 1, rows: 7, col: 0, cols: 2 },
                            Block { row: 4, rows: 9, col: 2, cols: 3 },
                        ],
                    }
                }
            }
            let plan = d.plan_replace(&s, rep, 100_000, &p).unwrap();
            let mut want: Vec<(u64, u32, String)> = plan
                .preview
                .iter()
                .filter(|c| !c.hidden)
                .map(|c| (c.row, c.col, c.after.clone()))
                .collect();
            want.sort();
            let g = d
                .replace_ghosts(&s, rep, 0, d.row_count(), 0, d.col_count())
                .unwrap();
            assert!(!g.partial);
            let mut got: Vec<(u64, u32, String)> = g
                .ghosts
                .iter()
                .map(|x| (x.row, x.col, visible(&x.after)))
                .collect();
            got.sort();
            assert_eq!(got, want, "round {round}: ghosts are the plan's changes in shown cells");
            // the count is the whole plan, hidden cells included
            assert_eq!(d.replace_count(&s, rep, &p).unwrap(), plan.cells, "round {round}: count");
            // a smaller window gives exactly the cells inside it
            let (r0, nr, c0, nc) = (1, 4, 1, 3);
            let part = d.replace_ghosts(&s, rep, r0, nr, c0, nc).unwrap();
            let inside: Vec<(u64, u32)> = got
                .iter()
                .filter(|x| x.0 >= r0 && x.0 < r0 + nr && x.1 >= c0 && x.1 < c0 + nc)
                .map(|x| (x.0, x.1))
                .collect();
            let mut win: Vec<(u64, u32)> = part.ghosts.iter().map(|x| (x.row, x.col)).collect();
            win.sort();
            assert_eq!(win, inside, "round {round}: window");
        }
    }

    #[test]
    fn a_ghost_keeps_the_whole_new_text_up_to_a_cap_and_says_so() {
        let long = "a".repeat(30_000);
        let d = doc(&format!("x\r\n{long}\r\nab\r\n"));
        let s = spec(pat("a", PatternKind::Plain));
        let g = d.replace_ghosts(&s, "bb", 0, 3, 0, 1).unwrap();
        let big = g.ghosts.iter().find(|x| x.row == 1).unwrap();
        assert!(big.cut);
        assert_eq!(big.after.chars().count(), MAX_GHOST_CHARS);
        let small = g.ghosts.iter().find(|x| x.row == 2).unwrap();
        assert_eq!((small.after.as_str(), small.cut), ("bbb", false));
    }

    #[test]
    fn a_window_of_very_long_cells_stops_at_its_time_and_the_count_can_be_cancelled() {
        let long = "ab ".repeat(33_000); // about 100 KB a cell
        let mut csv = String::new();
        for _ in 0..30 {
            csv.push_str(&format!("{long},{long},{long},{long},{long}\r\n"));
        }
        let d = doc(&csv);
        let s = spec(pat("(a)b", PatternKind::Regex));
        let t = std::time::Instant::now();
        let g = d.replace_ghosts(&s, "$1-", 0, 30, 0, 5).unwrap();
        let took = t.elapsed();
        assert!(
            took < Duration::from_millis(400),
            "a window of 150 cells of 100 KB took {took:?} (the 1,200-cell window is timed in ghost_bench)"
        );
        assert!(g.partial || g.ghosts.len() == 150);
        // a window that stopped early still gives whole rows of ghosts
        assert!(g.ghosts.len() % 5 == 0 || !g.partial);
        let p = Progress::new();
        p.cancel.store(true, Ordering::Relaxed);
        assert!(matches!(d.replace_count(&s, "x", &p), Err(Error::Cancelled)));
    }

    #[test]
    fn ghosts_refuse_a_bad_pattern_or_replacement_as_replace_all_does() {
        let d = doc("a\r\nb\r\n");
        assert!(d.replace_ghosts(&spec(pat("(", PatternKind::Regex)), "x", 0, 2, 0, 1).is_err());
        assert!(d.replace_ghosts(&spec(pat("(a)", PatternKind::Regex)), "$9", 0, 2, 0, 1).is_err());
        assert!(d.replace_count(&spec(pat("(", PatternKind::Regex)), "x", &Progress::new()).is_err());
    }

    // ---- Find in several blocks (Scope::Areas) ----

    use crate::areas::Area as Block;

    fn lcg(seed: u64) -> impl FnMut() -> u64 {
        let mut x = seed;
        move || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            x >> 33
        }
    }

    /// A random grid of "a", "b", "ab" and empty cells as CSV, and the grid.
    fn grid(
        r: &mut impl FnMut() -> u64,
        rows: usize,
        cols: usize,
    ) -> (String, Vec<Vec<&'static str>>) {
        const W: [&str; 4] = ["a", "b", "ab", ""];
        let g: Vec<Vec<&str>> = (0..rows)
            .map(|_| (0..cols).map(|_| W[(r() % 4) as usize]).collect())
            .collect();
        let csv: String = g.iter().map(|row| row.join(",") + "\r\n").collect();
        (csv, g)
    }

    fn random_blocks(r: &mut impl FnMut() -> u64, rows: u64, cols: u32) -> Vec<Block> {
        (0..1 + r() % 5)
            .map(|_| Block {
                // Some reach past the table and some are empty on purpose.
                row: r() % (rows + 2),
                rows: r() % (rows / 2 + 2),
                col: (r() % (u64::from(cols) + 1)) as u32,
                cols: (r() % (u64::from(cols) / 2 + 2)) as u32,
            })
            .collect()
    }

    /// The cells of the union of the blocks that the table has, once each.
    fn union_cells(
        blocks: &[Block],
        rows: u64,
        cols: u32,
        skip_row0: bool,
    ) -> std::collections::BTreeSet<(u64, u32)> {
        let mut out = std::collections::BTreeSet::new();
        for b in blocks {
            for row in b.row..(b.row + b.rows).min(rows) {
                if skip_row0 && row == 0 {
                    continue;
                }
                for col in b.col..(b.col.saturating_add(b.cols)).min(cols) {
                    out.insert((row, col));
                }
            }
        }
        out
    }

    #[test]
    fn areas_scope_matches_a_cell_by_cell_model_on_random_blocks() {
        let p = Progress::new();
        let mut r = lcg(11);
        for case in 0..400 {
            let (rows, cols) = (2 + (r() % 12) as usize, 1 + (r() % 5) as usize);
            let (csv, g) = grid(&mut r, rows, cols);
            let d = doc(&csv);
            let blocks = random_blocks(&mut r, rows as u64, cols as u32);
            let by_columns = case % 3 == 0;
            let skip_first = case % 4 == 0;
            let mut s = spec(pat("a", PatternKind::Plain));
            s.by_columns = by_columns;
            s.skip_first = skip_first;
            s.scope = Scope::Areas {
                areas: blocks.clone(),
            };
            let cells = union_cells(&blocks, rows as u64, cols as u32, skip_first);
            let mut hits: Vec<(u64, u32)> = cells
                .iter()
                .copied()
                .filter(|&(row, col)| g[row as usize][col as usize].contains('a'))
                .collect();
            // The list is in reading order (row by row).
            let all = d.find_all(&s, 1_000, &p).unwrap();
            assert_eq!(all.cells, hits.len() as u64, "case {case}: count");
            assert_eq!(
                all.hits.iter().map(|h| (h.row, h.col)).collect::<Vec<_>>(),
                hits,
                "case {case}: list"
            );
            // Per column counts add up.
            assert_eq!(
                all.per_col.iter().map(|x| x.1).sum::<u64>(),
                all.cells,
                "case {case}"
            );
            // Find next from every cell and from nowhere, both ways, with wrap.
            let order: Vec<(u64, u32)> = if by_columns {
                hits.sort_by_key(|&(row, col)| (col, row));
                hits.clone()
            } else {
                hits.clone()
            };
            let key = |c: (u64, u32)| {
                if by_columns {
                    (u64::from(c.1), c.0)
                } else {
                    (c.0, u64::from(c.1))
                }
            };
            let mut starts: Vec<Option<(u64, u32)>> = vec![None];
            for row in 0..rows as u64 {
                for col in 0..cols as u32 {
                    starts.push(Some((row, col)));
                }
            }
            // A start outside the searched rows (their bounding band) begins at the
            // start or the end, without a wrap, as for one block.
            let (lo, hi) = cells.iter().fold((u64::MAX, 0), |(a, b), &(row, _)| {
                (a.min(row), b.max(row + 1))
            });
            for from in starts {
                let from0 = from;
                let from = from.filter(|f| (lo..hi).contains(&f.0));
                for backward in [false, true] {
                    let got = d.find_next(&s, from0, backward, &p).unwrap();
                    let want: Option<((u64, u32), bool)> = if order.is_empty() {
                        None
                    } else {
                        match (from, backward) {
                            (None, false) => Some((order[0], false)),
                            (None, true) => Some((order[order.len() - 1], false)),
                            (Some(f), false) => order
                                .iter()
                                .find(|&&c| key(c) > key(f))
                                .map(|&c| (c, false))
                                .or(Some((order[0], true))),
                            (Some(f), true) => order
                                .iter()
                                .rev()
                                .find(|&&c| key(c) < key(f))
                                .map(|&c| (c, false))
                                .or(Some((order[order.len() - 1], true))),
                        }
                    };
                    assert_eq!(
                        got, want,
                        "case {case}: next from {from:?} backward {backward}, blocks {blocks:?}"
                    );
                }
            }
            // The count before a cell is the matches before it in that order.
            let c = d.find_count(&s, Some((0, 0)), &p).unwrap();
            assert_eq!(c.cells, hits.len() as u64, "case {case}: find_count");
            // Lit-up matches in a window.
            let marks = d.find_marks(&s, 0, rows as u64, 0, cols as u32).unwrap();
            let mut m = marks.clone();
            m.sort_unstable();
            let mut h = hits.clone();
            h.sort_unstable();
            assert_eq!(m, h, "case {case}: marks");
            // Rows that match.
            let mrows = d.matching_rows(&s, &p).unwrap();
            let mut want: Vec<u64> = hits.iter().map(|x| x.0).collect();
            want.sort_unstable();
            want.dedup();
            assert_eq!(mrows, want, "case {case}: matching rows");
            // Replace all changes only cells of the blocks, and one Undo undoes it.
            let mut d = d;
            let plan = d.plan_replace(&s, "X", 5, &p).unwrap();
            assert_eq!(plan.cells, hits.len() as u64, "case {case}: plan cells");
            d.apply_replace(plan).unwrap();
            for (row, line) in g.iter().enumerate() {
                for (col, text) in line.iter().enumerate() {
                    let now = d.cell(row as u64, col as u32).unwrap();
                    if cells.contains(&(row as u64, col as u32)) && text.contains('a') {
                        assert_eq!(
                            now,
                            text.replace('a', "X"),
                            "case {case}: changed ({row},{col})"
                        );
                    } else {
                        assert_eq!(&now, text, "case {case}: untouched ({row},{col})");
                    }
                }
            }
            if !hits.is_empty() {
                assert!(d.undo());
                for (row, line) in g.iter().enumerate() {
                    for (col, text) in line.iter().enumerate() {
                        assert_eq!(
                            &d.cell(row as u64, col as u32).unwrap(),
                            text,
                            "case {case}: undone"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn one_block_as_areas_is_the_range_scope() {
        let p = Progress::new();
        let mut r = lcg(5);
        for _ in 0..200 {
            let (csv, _) = grid(&mut r, 9, 4);
            let d = doc(&csv);
            let (row, rows, col, cols) =
                (r() % 9, r() % 9 + 1, (r() % 4) as u32, (r() % 4) as u32 + 1);
            let mut a = spec(pat("a", PatternKind::Plain));
            let mut b = a.clone();
            a.scope = Scope::Range {
                row,
                rows,
                col,
                cols,
            };
            b.scope = Scope::Areas {
                areas: vec![Block {
                    row,
                    rows,
                    col,
                    cols,
                }],
            };
            assert_eq!(
                d.find_all(&a, 100, &p).unwrap(),
                d.find_all(&b, 100, &p).unwrap()
            );
            for from in [None, Some((row, col)), Some((8, 3))] {
                assert_eq!(
                    d.find_next(&a, from, false, &p).unwrap(),
                    d.find_next(&b, from, false, &p).unwrap()
                );
            }
        }
    }

    #[test]
    fn areas_overlap_is_counted_once_and_junk_blocks_are_dropped() {
        let d = doc("a,a,a\r\na,a,a\r\na,a,a\r\n");
        let p = Progress::new();
        let mut s = spec(pat("a", PatternKind::Plain));
        let b = |row, rows, col, cols| Block {
            row,
            rows,
            col,
            cols,
        };
        // Two blocks overlapping in four cells: 9 distinct cells, not 12.
        s.scope = Scope::Areas {
            areas: vec![b(0, 2, 0, 2), b(1, 2, 1, 2), b(1, 2, 1, 2)],
        };
        assert_eq!(d.find_all(&s, 100, &p).unwrap().cells, 7);
        // Empty, out of range and huge blocks are cut or dropped, not an error.
        s.scope = Scope::Areas {
            areas: vec![
                b(0, 0, 0, 3),
                b(0, 3, 0, 0),
                b(50, 3, 0, 3),
                b(1, u64::MAX, 2, u32::MAX),
            ],
        };
        assert_eq!(d.find_all(&s, 100, &p).unwrap().cells, 2);
        s.scope = Scope::Areas { areas: Vec::new() };
        assert_eq!(d.find_all(&s, 100, &p).unwrap().cells, 0);
        assert!(d.find_next(&s, None, false, &p).unwrap().is_none());
    }

    #[test]
    fn more_than_the_most_blocks_is_refused_before_any_work() {
        let d = doc("a\r\n");
        let mut s = spec(pat("a", PatternKind::Plain));
        let blocks = vec![
            Block {
                row: 0,
                rows: 1,
                col: 0,
                cols: 1
            };
            MAX_FIND_BLOCKS + 1
        ];
        s.scope = Scope::Areas { areas: blocks };
        assert!(d.find_all(&s, 10, &Progress::new()).is_err());
        s.scope = Scope::Areas {
            areas: vec![
                Block {
                    row: 0,
                    rows: 1,
                    col: 0,
                    cols: 1
                };
                MAX_FIND_BLOCKS
            ],
        };
        assert_eq!(d.find_all(&s, 10, &Progress::new()).unwrap().cells, 1);
    }

    #[test]
    fn the_title_row_is_left_out_of_every_block_when_asked() {
        let d = doc("a,a\r\na,a\r\na,a\r\n");
        let p = Progress::new();
        let mut s = spec(pat("a", PatternKind::Plain));
        s.skip_first = true;
        // One block starts at the title row and one does not.
        s.scope = Scope::Areas {
            areas: vec![
                Block {
                    row: 0,
                    rows: 2,
                    col: 0,
                    cols: 1,
                },
                Block {
                    row: 1,
                    rows: 2,
                    col: 1,
                    cols: 1,
                },
            ],
        };
        let all = d.find_all(&s, 100, &p).unwrap();
        assert_eq!(
            all.hits.iter().map(|h| (h.row, h.col)).collect::<Vec<_>>(),
            vec![(1, 0), (1, 1), (2, 1)]
        );
        s.skip_first = false;
        assert_eq!(d.find_all(&s, 100, &p).unwrap().cells, 4);
    }

    #[test]
    fn areas_with_hidden_rows_included_follow_the_blocks_like_a_range() {
        let mut d = doc("k,v
a,x
b,x
a,x
b,y
");
        filter_a(&mut d);
        let p = Progress::new();
        let mut s = spec(pat("x", PatternKind::Plain));
        s.hidden_rows = true;
        let range = Scope::Range {
            row: 1,
            rows: 2,
            col: 1,
            cols: 1,
        };
        // One block, and the same cells as two blocks (which join).
        for areas in [
            vec![Block {
                row: 1,
                rows: 2,
                col: 1,
                cols: 1,
            }],
            vec![
                Block {
                    row: 1,
                    rows: 1,
                    col: 1,
                    cols: 1,
                },
                Block {
                    row: 2,
                    rows: 1,
                    col: 1,
                    cols: 1,
                },
            ],
        ] {
            s.scope = range.clone();
            let want = d.find_all(&s, 10, &p).unwrap();
            s.scope = Scope::Areas { areas };
            let got = d.find_all(&s, 10, &p).unwrap();
            assert_eq!(got, want);
            assert_eq!(got.cells, 3);
            assert_eq!(got.hits.iter().filter(|h| h.hidden).count(), 1);
        }
        // Find next and the lit-up matches stay on shown rows.
        assert_eq!(
            d.find_next(&s, Some((1, 1)), false, &p).unwrap(),
            Some(((2, 1), false))
        );
        assert_eq!(
            d.find_marks(&s, 0, 10, 0, 10).unwrap(),
            vec![(1, 1), (2, 1)]
        );
    }

    /// The same cases the browser stand-in (ui/src/lib/mock.ts) runs: crates/engine/src/testdata/find_blocks.json.
    #[test]
    fn find_over_blocks_gives_the_cells_in_the_shared_fixture() {
        let fx: serde_json::Value =
            serde_json::from_str(include_str!("testdata/find_blocks.json")).unwrap();
        let csv: String = fx["grid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(",")
                    + "\r\n"
            })
            .collect();
        let d = doc(&csv);
        let p = Progress::new();
        for case in fx["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let areas: Vec<crate::areas::Area> =
                serde_json::from_value(case["areas"].clone()).unwrap();
            let mut s = spec(pat(case["pattern"].as_str().unwrap(), PatternKind::Plain));
            s.scope = Scope::Areas { areas };
            s.skip_first = case["skip_first"].as_bool().unwrap();
            s.skip_cols = case["skip_cols"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_u64().unwrap() as u32)
                .collect();
            let want: Vec<(u64, u32)> = case["hits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| (h[0].as_u64().unwrap(), h[1].as_u64().unwrap() as u32))
                .collect();
            let all = d.find_all(&s, 1_000, &p).unwrap();
            let got: Vec<(u64, u32)> = all.hits.iter().map(|h| (h.row, h.col)).collect();
            assert_eq!(got, want, "{name}: cells found");
            assert_eq!(all.cells, want.len() as u64, "{name}: count");
            assert_eq!(
                d.find_count(&s, None, &p).unwrap().cells,
                want.len() as u64,
                "{name}: find_count"
            );
        }
    }

    #[test]
    fn stale_plan_is_refused() {
        let mut d = doc("a\r\nb\r\n");
        let s = spec(pat("a", PatternKind::Plain));
        let plan = d.plan_replace(&s, "z", 0, &Progress::new()).unwrap();
        d.set_cell(1, 0, "q".into()).unwrap();
        assert!(matches!(d.apply_replace(plan), Err(Error::Changed)));
        assert_eq!(d.cell(0, 0).unwrap(), "a");
    }

    #[test]
    fn cancel_stops_work() {
        let body: String = (0..10_000).map(|i| format!("{i}\r\n")).collect();
        let d = doc(&body);
        let p = Progress::new();
        p.cancel.store(true, Ordering::Relaxed);
        let s = spec(pat("zzz", PatternKind::Plain));
        assert!(matches!(d.find_all(&s, 10, &p), Err(Error::Cancelled)));
    }
}
