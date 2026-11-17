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

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
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
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Matcher {
    re: Regex,
    /// Replacement text is taken literally (plain and wildcard patterns).
    literal: bool,
    /// `\n`, `\t` ... in the replacement stand for those characters.
    rep_escapes: bool,
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
                    }
                }
            }
            '[' => {
                class_depth += 1;
                out.push(c);
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

/// `\t` `\n` `\r` `\\` `\u{XXXX}` in text as the characters they name;
/// any other backslash stays as it is.
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
        let text = if p.escapes && p.kind != PatternKind::Regex {
            unescape(&p.text)
        } else {
            Cow::Borrowed(p.text.as_str())
        };
        let accents = p.ignore_accents;
        let body = match p.kind {
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
            re: build(&source, p.match_case)?,
            literal: p.kind != PatternKind::Regex,
            rep_escapes: p.escapes || p.kind == PatternKind::Regex,
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
        })
    }

    pub fn is_match(&self, s: &str) -> bool {
        self.re.is_match(s)
    }

    /// Byte range of the first match.
    pub fn find(&self, s: &str) -> Option<Range<usize>> {
        self.re.find(s).map(|m| m.range())
    }

    /// `s` with every match replaced, and the number of matches replaced, or
    /// `None` when nothing matched. Empty matches are skipped unless the
    /// cell is empty (so `^$` fills blank cells, but `x*` never inserts text
    /// between every character).
    pub fn replace(&self, s: &str, rep: &str) -> Result<Option<(String, u64)>, Error> {
        let rep: Cow<'_, str> = if self.rep_escapes {
            unescape(rep)
        } else {
            Cow::Borrowed(rep)
        };
        let rep = rep.as_ref();
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
                push(&mut out, rep)?;
                last = m.end();
                hits += 1;
            }
        } else {
            let mut piece = String::new();
            for caps in self.re.captures_iter(s) {
                let m = caps.get(0).expect("group 0 always matches");
                if m.is_empty() && !s.is_empty() {
                    continue;
                }
                push(&mut out, &s[last..m.start()])?;
                piece.clear();
                caps.expand(rep, &mut piece);
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
        if self.literal {
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
                    for (_, m) in chars.by_ref() {
                        if m == '}' {
                            break;
                        }
                        n.push(m);
                    }
                    n
                }
                _ => {
                    let mut n = String::new();
                    while let Some(&(_, m)) = chars.peek() {
                        if m.is_alphanumeric() || m == '_' {
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Every column of every shown row.
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
}

/// A search: what and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// Result of find-all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindAll {
    pub hits: Vec<Hit>,
    /// Cells that match (all of them, not just those listed).
    pub cells: u64,
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
}

/// A computed replace-all, applied with [`Document::apply_replace`].
#[derive(Debug, Clone)]
pub struct ReplacePlan {
    revision: u64,
    changes: Vec<(u32, Vec<(u32, String)>)>,
    /// Cells that change.
    pub cells: u64,
    /// Matches replaced.
    pub hits: u64,
    /// The first changes, for showing before applying.
    pub preview: Vec<Change>,
}

/// The cells a search covers, resolved against the document.
pub(crate) struct Area {
    pub(crate) rows: Range<u64>,
    /// Logical columns in search order.
    pub(crate) cols: Vec<u32>,
    /// Physical columns to read, sorted, and for each logical column the
    /// slot in that list (`None`: past the column map, always empty).
    wanted: Vec<u32>,
    slot: Vec<Option<usize>>,
}

fn snippet(s: &str, hit: Range<usize>) -> (String, u32, u32) {
    let before: Vec<char> = s[..hit.start].chars().collect();
    let skip = before.len().saturating_sub(SNIPPET_CHARS);
    let mut text: String = before[skip..].iter().collect();
    let start = text.chars().count() as u32;
    if skip > 0 {
        text.insert(0, '…');
    }
    let m: String = s[hit.clone()].chars().take(SNIPPET_CHARS * 2).collect();
    let len = m.chars().count() as u32;
    text.push_str(&m);
    let after = &s[hit.end..];
    let tail: String = after.chars().take(SNIPPET_CHARS).collect();
    let more = tail.len() < after.len();
    text.push_str(&tail);
    if more {
        text.push('…');
    }
    let text = text.replace(['\r', '\n', '\t'], " ");
    (text, start + (skip > 0) as u32, len)
}

pub(crate) fn short(s: &str) -> String {
    let mut t: String = s.chars().take(SNIPPET_CHARS * 2).collect();
    if t.len() < s.len() {
        t.push('…');
    }
    t.replace(['\r', '\n', '\t'], " ")
}

impl Document {
    pub(crate) fn area(&self, spec: &FindSpec) -> Result<Area, Error> {
        let n = self.row_count();
        let first = u64::from(spec.skip_first && n > 0);
        let (rows, cols): (Range<u64>, Vec<u32>) = match &spec.scope {
            Scope::All => (first..n, (0..self.col_count()).collect()),
            Scope::Columns { cols } => {
                let mut c = cols.clone();
                c.sort_unstable();
                c.dedup();
                c.retain(|&x| x < self.col_count());
                (first..n, c)
            }
            Scope::Range {
                row,
                rows,
                col,
                cols,
            } => {
                let r0 = (*row).min(n);
                let r1 = row.saturating_add(*rows).min(n);
                let c1 = col.saturating_add(*cols).min(self.col_count());
                (r0..r1, (*col..c1).collect())
            }
        };
        let mut cols = cols;
        if !spec.skip_cols.is_empty() {
            cols.retain(|c| !spec.skip_cols.contains(c));
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
        Ok(Area {
            rows,
            cols,
            wanted,
            slot,
        })
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
        let id = self.id_at(row)?;
        self.read_cells(id, &area.wanted, buf);
        let n = area.cols.len();
        for i in 0..n {
            let k = if rev { n - 1 - i } else { i };
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
        let area = self.area(spec)?;
        let (r0, r1) = (area.rows.start, area.rows.end);
        if r0 >= r1 || area.cols.is_empty() {
            return Ok(None);
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
            });
        }
        let blocks = (r1 - r0).div_ceil(BLOCK);
        let parts: Vec<(Vec<Hit>, u64)> = (0..blocks)
            .into_par_iter()
            .map(|b| -> Result<(Vec<Hit>, u64), Error> {
                let s = r0 + b * BLOCK;
                let e = (s + BLOCK).min(r1);
                let mut hits = Vec::new();
                let mut count = 0u64;
                let mut buf = Vec::new();
                for r in s..e {
                    self.each_cell(&area, r, false, &mut buf, |k, t| {
                        if let Some(range) = m.find(t) {
                            count += 1;
                            if hits.len() < limit {
                                let (text, start, len) = snippet(t, range);
                                hits.push(Hit {
                                    row: r,
                                    number: self.row_number(r).unwrap_or(r),
                                    col: area.cols[k],
                                    text,
                                    start,
                                    len,
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
        let mut cells = 0;
        for (h, c) in parts {
            cells += c;
            if hits.len() < limit {
                hits.extend(h.into_iter().take(limit - hits.len()));
            }
        }
        Ok(FindAll { hits, cells })
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
        let area = self.area(spec)?;
        let (r0, r1) = (area.rows.start, area.rows.end);
        progress.start(r1.saturating_sub(r0));
        let created = AtomicUsize::new(0);
        type Part = (Vec<(u32, Vec<(u32, String)>)>, Vec<Change>, u64, u64);
        let blocks = r1.saturating_sub(r0).div_ceil(BLOCK);
        let parts: Vec<Part> = (0..blocks)
            .into_par_iter()
            .map(|b| -> Result<Part, Error> {
                let s = r0 + b * BLOCK;
                let e = (s + BLOCK).min(r1);
                let mut changes = Vec::new();
                let mut shown = Vec::new();
                let (mut cells, mut hits) = (0u64, 0u64);
                let mut buf = Vec::new();
                for r in s..e {
                    let Some(id) = self.id_at(r) else { continue };
                    let mut row: Vec<(u32, String)> = Vec::new();
                    let mut err = None;
                    self.each_cell(&area, r, false, &mut buf, |k, t| {
                        let phys = self.phys(area.cols[k])?;
                        match m.replace(t, replacement) {
                            Ok(Some((new, n))) if new != t => {
                                if !charge(&created, new.len()) {
                                    err = Some(Error::OutOfMemory(
                                        "this replace would change too much at once; replace in a smaller selection",
                                    ));
                                    return Some(());
                                }
                                if shown.len() < preview {
                                    shown.push(Change {
                                        row: r,
                                        number: self.row_number(r).unwrap_or(r),
                                        col: area.cols[k],
                                        before: short(t),
                                        after: short(&new),
                                    });
                                }
                                cells += 1;
                                hits += n;
                                row.push((phys, new));
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
                        changes.push((id, row));
                    }
                }
                progress.step(e - s)?;
                Ok((changes, shown, cells, hits))
            })
            .collect::<Result<_, _>>()?;
        let mut plan = ReplacePlan {
            revision: self.revision(),
            changes: Vec::new(),
            cells: 0,
            hits: 0,
            preview: Vec::new(),
        };
        for (c, p, cells, hits) in parts {
            plan.changes.extend(c);
            if plan.preview.len() < preview {
                plan.preview
                    .extend(p.into_iter().take(preview - plan.preview.len()));
            }
            plan.cells += cells;
            plan.hits += hits;
        }
        Ok(plan)
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
        }
    }

    fn with(text: &str, kind: PatternKind, f: impl FnOnce(&mut Pattern)) -> Matcher {
        let mut p = pat(text, kind);
        f(&mut p);
        Matcher::new(&p).unwrap()
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
        let m = with("caf?", PatternKind::Wildcard, |p| p.ignore_accents = true);
        assert!(m.is_match("Càfe"));
        // Off, accents count.
        assert!(!with("Jose", PatternKind::Plain, |_| {}).is_match("José"));
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
    fn replacement_names_missing_groups() {
        let m = with(r"(\d+)", PatternKind::Regex, |_| {});
        assert!(m.check_replacement("$1 and ${1}a and $$2").is_ok());
        let e = m.check_replacement("$1a").unwrap_err().to_string();
        assert!(e.contains("${1}a"), "{e}");
        assert!(m.check_replacement("$2").is_err());
        let m = with(r"(?P<zip>\d{5})", PatternKind::Regex, |_| {});
        assert!(m.check_replacement("$zip").is_ok() && m.check_replacement("$zap").is_err());
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
