//! Explode cells (C7): a column whose cells pack several fields (a JSON
//! object, a JSON list, a delimited list or key=value pairs) is turned into
//! one column per key, one column per list item, one flag column per
//! distinct item, or one row per item.
//!
//! Text is never changed on the way: numbers keep their exact text (`1.50`,
//! `12345678901234567890`), nested parts deeper than the chosen depth keep
//! their JSON text as written, and a cell that does not parse stays exactly
//! as it was (it is counted, and "show those rows" filters to it).
//!
//! Columns, fan out and flags cover every row, hidden ones too (as Split
//! column does: the new columns exist for all rows). Unnest to rows covers
//! shown rows only (as Split into rows does); hidden rows are left alone.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::document::Progress;
use crate::ops::{check_cols, check_separator, ColPlan, Cols, CHUNK, MAX_NEW_COLS};
use crate::reshape::{RowPlan, MAX_NEW_ROWS, MAX_PARTS};
use crate::{Document, Error};

/// Largest cell read as JSON (larger ones do not parse, and are counted).
pub const MAX_JSON_CELL: usize = 1 << 20;
/// Deepest nesting read; deeper input does not parse.
pub const MAX_NESTING: usize = 64;
/// Distinct keys (or tags) kept track of by a scan.
pub const MAX_TRACKED: usize = 100_000;
/// Keys (or tags) a scan lists: the most common, in first-seen order.
pub const TOP_KEYS: usize = 1_000;
/// Filled cells read to guess the kind.
pub const DETECT_SAMPLE: usize = 2_000;
/// Rows read at most to find [`DETECT_SAMPLE`] filled cells.
const DETECT_ROWS: u64 = 200_000;
/// Rows a preview shows.
pub const PREVIEW_ROWS: u64 = 20;
/// Characters of a preview's "before" text.
const PREVIEW_CHARS: usize = 200;
/// New or missing keys named in a report (the rest are counted).
pub const MAX_NAMED: usize = 50;
/// Longest flag mark.
const MAX_MARK: usize = 100;
/// Detection says "mixed" below this share of the sample.
const SURE_SHARE: f64 = 0.8;

/// What the cells hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplodeKind {
    /// `{"plan":"gold","seats":3}`
    #[default]
    Object,
    /// `["a","b"]`
    JsonList,
    /// `vip|beta|annual`
    List,
    /// `a=1;b=2`
    KeyValue,
}

impl ExplodeKind {
    /// Keys and values (objects, key=value), not items.
    pub fn has_keys(self) -> bool {
        matches!(self, ExplodeKind::Object | ExplodeKind::KeyValue)
    }

    fn words(self, sep: &str, kv: &str) -> String {
        match self {
            ExplodeKind::Object => "JSON objects".into(),
            ExplodeKind::JsonList => "JSON lists".into(),
            ExplodeKind::List => format!("lists separated by {}", show_sep(sep)),
            ExplodeKind::KeyValue => {
                format!("key{kv}value pairs separated by {}", show_sep(sep))
            }
        }
    }
}

fn show_sep(s: &str) -> String {
    match s {
        "\t" => "tabs".into(),
        " " => "spaces".into(),
        "\n" => "line breaks".into(),
        s => s.into(),
    }
}

/// What is made from them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplodeHow {
    /// One column per key (objects, key=value), or per item position (lists:
    /// fan out to "title 1", "title 2"...).
    #[default]
    Columns,
    /// Lists: one row per item, the other cells repeated (shown rows only).
    Rows,
    /// Lists: one column per distinct item, holding a mark where present.
    Flags,
}

/// How to explode a column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Explode {
    pub col: u32,
    pub keep_first: bool,
    pub kind: ExplodeKind,
    pub how: ExplodeHow,
    /// Lists: between items; key=value: between pairs.
    pub separator: String,
    /// key=value: between a key and its value (split at the first).
    pub kv: String,
    /// Objects: nested objects flattened with dots down to this depth (1-3).
    pub depth: u8,
    /// The keys (or flag tags) made into columns, in this order; `None`:
    /// every key found, in first-seen order.
    pub keys: Option<Vec<String>>,
    /// With `keys`: keys found that are not listed become columns too
    /// (after the listed ones). Off: they are left out (and named).
    pub add_new_keys: bool,
    /// Keep the column; off: the first new column takes its place.
    pub keep_original: bool,
    pub trim: bool,
    /// Flags: the text put where an item is present.
    pub mark: String,
    /// JSON `null` as a blank cell; off: as the text `null`.
    pub null_blank: bool,
    /// Rows: add a column "title (original)" holding the whole cell.
    pub original_col: bool,
    /// Flags: "VIP" and "vip" are different items.
    pub match_case: bool,
    /// Delimited lists: leave out empty items.
    pub skip_empty: bool,
}

impl Default for Explode {
    fn default() -> Self {
        Explode {
            col: 0,
            keep_first: true,
            kind: ExplodeKind::Object,
            how: ExplodeHow::Columns,
            separator: "|".into(),
            kv: "=".into(),
            depth: 1,
            keys: None,
            add_new_keys: false,
            keep_original: true,
            trim: true,
            mark: "1".into(),
            null_blank: true,
            original_col: false,
            match_case: false,
            skip_empty: true,
        }
    }
}

impl Explode {
    /// What a recipe step says it does.
    pub fn describe(&self) -> String {
        let what = match self.kind {
            ExplodeKind::Object => "JSON",
            ExplodeKind::JsonList => "JSON lists",
            ExplodeKind::List => "lists",
            ExplodeKind::KeyValue => "key=value pairs",
        };
        let col = self.col + 1;
        let named = |noun: &str| match &self.keys {
            None => format!("one column per {noun}"),
            Some(k) => {
                let mut s: Vec<&str> = k.iter().take(5).map(String::as_str).collect();
                if k.len() > 5 {
                    s.push("…");
                }
                format!("columns ({})", s.join(", "))
            }
        };
        match self.how {
            ExplodeHow::Rows => format!("Unnest {what} in column {col} to rows"),
            ExplodeHow::Flags => {
                format!("Explode {what} in column {col} to flag {}", named("item"))
            }
            ExplodeHow::Columns if self.kind.has_keys() => {
                format!("Explode {what} in column {col} to {}", named("key"))
            }
            ExplodeHow::Columns => format!("Fan out {what} in column {col} to columns"),
        }
    }

    fn check(&self, d: &Document) -> Result<(), Error> {
        d.ready_for_bulk()?;
        check_cols(d, &[self.col])?;
        check_separator(&self.separator)?;
        check_separator(&self.kv)?;
        let bad = |m: &str| Err(Error::Invalid(m.into()));
        if !(1..=3).contains(&self.depth) {
            return bad("the depth must be 1, 2 or 3");
        }
        if self.mark.len() > MAX_MARK {
            return bad("the mark is too long");
        }
        match self.kind {
            ExplodeKind::List | ExplodeKind::KeyValue if self.separator.is_empty() => {
                return bad("choose a separator");
            }
            ExplodeKind::KeyValue if self.kv.is_empty() => {
                return bad("choose what is between a key and its value");
            }
            _ => {}
        }
        if self.kind.has_keys() && self.how != ExplodeHow::Columns {
            return bad("keys and values can only become columns");
        }
        if let Some(k) = &self.keys {
            if k.len() > MAX_TRACKED {
                return bad("too many keys");
            }
        }
        Ok(())
    }

    /// Fan out: lists to "title 1", "title 2"...
    fn fan_out(&self) -> bool {
        !self.kind.has_keys() && self.how == ExplodeHow::Columns
    }

    fn null_text(&self) -> &'static str {
        if self.null_blank {
            ""
        } else {
            "null"
        }
    }
}

// ---- reading one cell ----

/// A value read from a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Val<'a> {
    Text(Cow<'a, str>),
    Null,
}

impl Val<'_> {
    fn text(&self, spec: &Explode) -> String {
        match self {
            Val::Text(t) => t.to_string(),
            Val::Null => spec.null_text().into(),
        }
    }
}

/// What one cell holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Read<'a> {
    /// Blank: nothing to do, not counted.
    Empty,
    /// Not what the kind says (or too big, too deep, a repeated key).
    Bad,
    Pairs(Vec<(String, Val<'a>)>),
    Items(Vec<Val<'a>>),
}

/// A JSON object's members in file order, each value's raw text kept.
struct Obj<'a>(Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Obj<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Obj<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Obj<'de>, A::Error> {
                let mut out = Vec::new();
                while let Some(k) = m.next_key::<String>()? {
                    let v: &'de RawValue = m.next_value()?;
                    out.push((k, v));
                }
                Ok(Obj(out))
            }
        }
        d.deserialize_map(V)
    }
}

/// Nesting deeper than [`MAX_NESTING`] (outside strings).
fn too_deep(s: &str) -> bool {
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for &b in s.as_bytes() {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_NESTING {
                    return true;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// A key as a column title part: control characters (line breaks) become
/// spaces, an empty key is "(empty)".
pub(crate) fn clean_key(k: &str) -> String {
    let s: String = k
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if s.is_empty() {
        "(empty)".into()
    } else {
        s
    }
}

/// A JSON value as cell text: strings unescaped, everything else as written.
fn json_val<'a>(raw: &'a RawValue, trim: bool) -> Option<Val<'a>> {
    let s = raw.get();
    let t: Cow<'a, str> = match s.as_bytes().first()? {
        b'"' => {
            if s.contains('\\') {
                Cow::Owned(serde_json::from_str::<String>(s).ok()?)
            } else {
                Cow::Borrowed(&s[1..s.len() - 1])
            }
        }
        b'n' => return Some(Val::Null),
        _ => Cow::Borrowed(s),
    };
    Some(Val::Text(if trim {
        match t {
            Cow::Borrowed(b) => Cow::Borrowed(b.trim()),
            Cow::Owned(o) => Cow::Owned(o.trim().to_string()),
        }
    } else {
        t
    }))
}

/// Flatten an object's members into `out`, nested objects down to `depth`.
/// `false`: a key repeated in one object.
fn flatten<'a>(
    obj: Obj<'a>,
    prefix: &str,
    level: u8,
    spec: &Explode,
    out: &mut Vec<(String, Val<'a>)>,
    seen: &mut HashSet<String>,
) -> bool {
    let mut here: HashSet<&str> = HashSet::with_capacity(obj.0.len());
    for (k, _) in &obj.0 {
        if !here.insert(k.as_str()) {
            return false;
        }
    }
    drop(here);
    for (k, raw) in obj.0 {
        let key = if prefix.is_empty() {
            clean_key(&k)
        } else {
            format!("{prefix}.{}", clean_key(&k))
        };
        let text = raw.get();
        if level < spec.depth && text.starts_with('{') && text != "{}" {
            let Ok(inner) = serde_json::from_str::<Obj>(text) else {
                return false;
            };
            if !inner.0.is_empty() {
                if !flatten(inner, &key, level + 1, spec, out, seen) {
                    return false;
                }
                continue;
            }
        }
        let Some(v) = json_val(raw, spec.trim) else {
            return false;
        };
        // A key "a.b" and a nested a -> b land on the same name: the
        // second is kept apart as "a.b (2)".
        let mut name = key.clone();
        let mut n = 2;
        while !seen.insert(name.clone()) {
            name = format!("{key} ({n})");
            n += 1;
        }
        out.push((name, v));
    }
    true
}

/// Read one cell as the spec's kind.
pub(crate) fn read_cell<'a>(v: &'a str, spec: &Explode) -> Read<'a> {
    let t = v.trim();
    if t.is_empty() {
        return Read::Empty;
    }
    match spec.kind {
        ExplodeKind::Object => {
            if !t.starts_with('{') || t.len() > MAX_JSON_CELL || too_deep(t) {
                return Read::Bad;
            }
            let Ok(obj) = serde_json::from_str::<Obj>(t) else {
                return Read::Bad;
            };
            let mut out = Vec::with_capacity(obj.0.len());
            let mut seen = HashSet::new();
            if !flatten(obj, "", 1, spec, &mut out, &mut seen) {
                return Read::Bad;
            }
            Read::Pairs(out)
        }
        ExplodeKind::JsonList => {
            if !t.starts_with('[') || t.len() > MAX_JSON_CELL || too_deep(t) {
                return Read::Bad;
            }
            let Ok(items) = serde_json::from_str::<Vec<&RawValue>>(t) else {
                return Read::Bad;
            };
            let mut out = Vec::with_capacity(items.len());
            for raw in items {
                match json_val(raw, spec.trim) {
                    Some(v) => out.push(v),
                    None => return Read::Bad,
                }
            }
            Read::Items(out)
        }
        ExplodeKind::List => {
            let mut out = Vec::new();
            for p in v.split(spec.separator.as_str()) {
                let p = if spec.trim { p.trim() } else { p };
                if spec.skip_empty && p.is_empty() {
                    continue;
                }
                out.push(Val::Text(Cow::Borrowed(p)));
            }
            Read::Items(out)
        }
        ExplodeKind::KeyValue => {
            let mut out: Vec<(String, Val<'a>)> = Vec::new();
            let mut seen = HashSet::new();
            for p in v.split(spec.separator.as_str()) {
                if p.trim().is_empty() {
                    continue;
                }
                let Some((k, val)) = p.split_once(spec.kv.as_str()) else {
                    return Read::Bad;
                };
                let (k, val) = if spec.trim {
                    (k.trim(), val.trim())
                } else {
                    (k, val)
                };
                let key = clean_key(k);
                // The same key twice in one cell: which is meant is not
                // for the app to pick.
                if !seen.insert(key.clone()) {
                    return Read::Bad;
                }
                out.push((key, Val::Text(Cow::Borrowed(val))));
            }
            Read::Pairs(out)
        }
    }
}

// ---- keys found ----

/// Distinct keys (or tags) in first-seen order, with counts.
#[derive(Default)]
struct Keys {
    names: Vec<String>,
    counts: Vec<u64>,
    ix: HashMap<String, u32>,
    /// More than [`MAX_TRACKED`] were met.
    over: bool,
}

impl Keys {
    /// Count `name` (matched as `norm`); its index, or `None` past the cap.
    fn add(&mut self, norm: Cow<str>, name: &str, count: u64) -> Option<u32> {
        if let Some(&i) = self.ix.get(norm.as_ref()) {
            self.counts[i as usize] += count;
            return Some(i);
        }
        if self.names.len() >= MAX_TRACKED {
            self.over = true;
            return None;
        }
        let i = self.names.len() as u32;
        self.ix.insert(norm.into_owned(), i);
        self.names.push(name.to_string());
        self.counts.push(count);
        Some(i)
    }

    fn find(&self, norm: &str) -> Option<u32> {
        self.ix.get(norm).copied()
    }

    /// Chunks' keys merged in order (first-seen across the whole column);
    /// each chunk's map from its own index to the merged one.
    fn merge(parts: Vec<Keys>, spec: &Explode) -> (Keys, Vec<Vec<Option<u32>>>) {
        let mut all = Keys::default();
        let mut maps = Vec::with_capacity(parts.len());
        for p in parts {
            all.over |= p.over;
            let map = p
                .names
                .iter()
                .zip(&p.counts)
                .map(|(n, &c)| all.add(norm_key(n, spec), n, c))
                .collect();
            maps.push(map);
        }
        (all, maps)
    }

    /// The [`TOP_KEYS`] most common, in first-seen order.
    fn top(&self) -> Vec<KeyCount> {
        let mut ix: Vec<usize> = (0..self.names.len()).collect();
        if ix.len() > TOP_KEYS {
            ix.sort_by(|&a, &b| self.counts[b].cmp(&self.counts[a]).then(a.cmp(&b)));
            ix.truncate(TOP_KEYS);
            ix.sort_unstable();
        }
        ix.into_iter()
            .map(|i| KeyCount {
                key: self.names[i].clone(),
                count: self.counts[i],
            })
            .collect()
    }
}

/// How keys (or flag items) are matched: keys exactly; items following
/// the spec's case setting (trim is already done).
fn norm_key<'a>(k: &'a str, spec: &Explode) -> Cow<'a, str> {
    if spec.how == ExplodeHow::Flags && !spec.match_case {
        Cow::Owned(k.to_lowercase())
    } else {
        Cow::Borrowed(k)
    }
}

/// A key (or tag) and how many cells hold it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyCount {
    pub key: String,
    pub count: u64,
}

/// What a scan of the whole column found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExplodeScan {
    /// Keys (objects, key=value) or tags (flags): the [`TOP_KEYS`] most
    /// common, in first-seen order.
    pub keys: Vec<KeyCount>,
    /// Distinct keys or tags found (up to [`MAX_TRACKED`]).
    pub distinct: u64,
    /// More than [`MAX_TRACKED`] distinct keys or tags.
    pub capped: bool,
    /// Filled cells read.
    pub filled: u64,
    /// Filled cells that did not parse (they stay as they are).
    pub bad: u64,
    /// Lists: most items in one cell.
    pub most_items: u64,
    /// Unnest: rows it would add.
    pub new_rows: u64,
    /// Rows looked at: every row (columns, flags) or shown rows (rows).
    pub rows: u64,
    pub all_rows: bool,
}

/// What a planned explode will do, for the dialog and the run report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExplodeReport {
    /// Cells that did not parse (left exactly as they were).
    pub bad: u64,
    /// Keys found that the step does not list (first [`MAX_NAMED`]).
    pub new_keys: Vec<String>,
    pub new_count: u64,
    /// Whether those new keys were added as columns.
    pub new_added: bool,
    /// Listed keys found in no cell (first [`MAX_NAMED`]).
    pub missing_keys: Vec<String>,
    pub missing_count: u64,
    /// Columns added.
    pub cols: u32,
    /// Rows added (unnest).
    pub rows: u64,
}

impl ExplodeReport {
    /// What a recipe run says about the step ("" when nothing to say).
    pub fn note(&self, title: &str) -> String {
        let names = |v: &[String], n: u64| {
            let mut s: Vec<String> = v.iter().map(|k| format!("\u{201c}{k}\u{201d}")).collect();
            if n > v.len() as u64 {
                s.push(format!("{} more", n - v.len() as u64));
            }
            s.join(", ")
        };
        let mut parts = Vec::new();
        if self.new_count > 0 {
            let word = if self.new_count == 1 { "key" } else { "keys" };
            parts.push(format!(
                "new {word} in {title}: {} ({})",
                names(&self.new_keys, self.new_count),
                if self.new_added {
                    "added as columns"
                } else {
                    "left out; tick \u{201c}add new keys\u{201d} to make them columns"
                }
            ));
        }
        if self.missing_count > 0 {
            let word = if self.missing_count == 1 {
                "key"
            } else {
                "keys"
            };
            parts.push(format!(
                "{word} found in no cell of {title}: {} (their columns are blank)",
                names(&self.missing_keys, self.missing_count)
            ));
        }
        if self.bad > 0 {
            parts.push(format!(
                "{} {} in {title} did not parse and stayed as they were",
                self.bad,
                if self.bad == 1 { "cell" } else { "cells" }
            ));
        }
        parts.join("; ")
    }
}

/// The kind a sample of the column looks like.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplodeDetect {
    pub kind: ExplodeKind,
    pub separator: String,
    pub kv: String,
    /// Sampled cells that look like it.
    pub matched: u64,
    /// Filled cells sampled.
    pub sampled: u64,
    /// In words: "JSON objects (97 of 100 sampled)", "mixed: 61 of 100 look
    /// like JSON objects".
    pub words: String,
}

/// How a preview cell came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Shown {
    /// Text.
    Value,
    /// The value is "".
    Empty,
    /// The value is JSON null (blank or "null", as set).
    Null,
    /// The cell has no such key (left blank).
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewCell {
    pub text: String,
    pub shown: Shown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewRow {
    /// Shown row.
    pub row: u64,
    /// The cell as it is (cut to 200 characters).
    pub before: String,
    pub bad: bool,
    /// After: one line (columns), or one per new row (unnest).
    pub lines: Vec<Vec<PreviewCell>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExplodePreview {
    /// Titles of the new columns.
    pub titles: Vec<String>,
    pub rows: Vec<PreviewRow>,
}

// ---- planning ----

/// One chunk's rows: (row id, (key index, text)) and its keys.
#[derive(Default)]
struct Chunk {
    keys: Keys,
    rows: Vec<(u32, Vec<(u32, String)>)>,
    bad: u64,
    filled: u64,
    most: u64,
    extra: u64,
}

fn cut(v: &str) -> String {
    match v.char_indices().nth(PREVIEW_CHARS) {
        Some((i, _)) => format!("{}…", &v[..i]),
        None => v.to_string(),
    }
}

fn new_title(base: &str, part: &str, taken: &mut HashSet<String>) -> String {
    let want = format!("{base}{part}");
    let mut t = want.clone();
    let mut n = 2;
    while !taken.insert(t.clone()) {
        t = format!("{want} ({n})");
        n += 1;
    }
    t
}

impl Document {
    /// The rows an explode looks at: (from, to, shown).
    fn explode_rows(&self, spec: &Explode) -> (u64, u64, bool) {
        if spec.how == ExplodeHow::Rows {
            let r = self.data_rows(spec.keep_first);
            (r.start, r.end, true)
        } else {
            let total = self.total_rows();
            (u64::from(spec.keep_first && total > 0), total, false)
        }
    }

    /// Run `f` over the cells of the explode's rows in parallel chunks,
    /// giving each chunk's state in row order.
    fn explode_chunks<S: Send>(
        &self,
        spec: &Explode,
        progress: &Progress,
        init: impl Fn() -> S + Sync,
        f: impl Fn(&mut S, u32, &str) -> Result<(), Error> + Sync,
    ) -> Result<Vec<S>, Error> {
        let (from, to, shown) = self.explode_rows(spec);
        let reader = Cols::new(self, &[spec.col]);
        progress.start(to.saturating_sub(from));
        let chunks = (to.saturating_sub(from) as usize).div_ceil(CHUNK);
        (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<S, Error> {
                let s = from + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(to);
                let mut st = init();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = if shown {
                        self.id_at(r)
                    } else {
                        self.doc_id_at(r)
                    }
                    .ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    f(&mut st, id, reader.get(&buf, 0))?;
                }
                progress.step(e - s)?;
                Ok(st)
            })
            .collect()
    }

    /// The column's title (or "Column N" with no title row).
    fn explode_base(&self, spec: &Explode) -> String {
        let t = if spec.keep_first {
            self.row_values(0)
                .ok()
                .and_then(|r| r.get(spec.col as usize).cloned())
                .unwrap_or_default()
        } else {
            String::new()
        };
        if t.trim().is_empty() {
            format!("Column {}", spec.col + 1)
        } else {
            clean_key(&t)
        }
    }

    /// Titles of the new columns, kept apart from the table's own titles
    /// and each other with " (2)".
    fn explode_titles(&self, spec: &Explode, names: &[String]) -> Vec<String> {
        let base = self.explode_base(spec);
        let mut taken: HashSet<String> = if spec.keep_first {
            self.row_values(0)
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .filter(|&(c, _)| c as u32 != spec.col || spec.keep_original)
                .map(|(_, t)| t)
                .collect()
        } else {
            HashSet::new()
        };
        match spec.how {
            ExplodeHow::Rows => vec![new_title(&base, " (original)", &mut taken)],
            ExplodeHow::Flags => names
                .iter()
                .map(|n| new_title(&base, &format!(": {n}"), &mut taken))
                .collect(),
            ExplodeHow::Columns if spec.kind.has_keys() => names
                .iter()
                .map(|n| new_title(&base, &format!(".{n}"), &mut taken))
                .collect(),
            ExplodeHow::Columns => names
                .iter()
                .map(|n| new_title(&base, &format!(" {n}"), &mut taken))
                .collect(),
        }
    }

    /// Guess what a column holds from up to [`DETECT_SAMPLE`] filled cells
    /// of the shown rows.
    pub fn explode_detect(&self, col: u32, keep_first: bool) -> Result<ExplodeDetect, Error> {
        self.ready_for_bulk()?;
        check_cols(self, &[col])?;
        let reader = Cols::new(self, &[col]);
        let rows = self.data_rows(keep_first);
        let mut cells: Vec<String> = Vec::new();
        let mut buf = Vec::new();
        for r in rows.start..rows.end.min(rows.start + DETECT_ROWS) {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let v = reader.get(&buf, 0);
            if !v.trim().is_empty() {
                cells.push(v.chars().take(MAX_JSON_CELL).collect());
                if cells.len() == DETECT_SAMPLE {
                    break;
                }
            }
        }
        Ok(detect(&cells))
    }

    /// Read the whole column (with progress and Cancel): the keys or tags
    /// with counts, cells that do not parse, items and rows to add.
    pub fn explode_scan(&self, spec: &Explode, progress: &Progress) -> Result<ExplodeScan, Error> {
        spec.check(self)?;
        let added = AtomicU64::new(0);
        let parts = self.explode_chunks(spec, progress, Chunk::default, |st, _, v| {
            let read = read_cell(v, spec);
            match read {
                Read::Empty => return Ok(()),
                Read::Bad => {
                    st.filled += 1;
                    st.bad += 1;
                    return Ok(());
                }
                _ => st.filled += 1,
            }
            match read {
                Read::Pairs(p) => {
                    for (k, _) in p {
                        st.keys.add(Cow::Borrowed(&k), &k, 1);
                    }
                }
                Read::Items(items) => {
                    st.most = st.most.max(items.len() as u64);
                    if spec.how == ExplodeHow::Flags {
                        let mut here = HashSet::new();
                        for it in &items {
                            let t = it.text(spec);
                            if t.is_empty() {
                                continue;
                            }
                            let n = norm_key(&t, spec).into_owned();
                            if here.insert(n.clone()) {
                                st.keys.add(Cow::Owned(n), &t, 1);
                            }
                        }
                    }
                    if items.len() > 1 {
                        st.extra += items.len() as u64 - 1;
                        added.fetch_add(items.len() as u64 - 1, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        let (from, to, shown) = self.explode_rows(spec);
        let mut out = ExplodeScan {
            rows: to - from,
            all_rows: !shown,
            ..Default::default()
        };
        let mut keys = Vec::with_capacity(parts.len());
        for p in parts {
            out.bad += p.bad;
            out.filled += p.filled;
            out.most_items = out.most_items.max(p.most);
            out.new_rows += p.extra;
            keys.push(p.keys);
        }
        let (all, _) = Keys::merge(keys, spec);
        out.distinct = all.names.len() as u64;
        out.capped = all.over;
        out.keys = all.top();
        Ok(out)
    }

    /// The rows whose cell did not parse, shown as a filter (undoable).
    pub fn plan_explode_bad(&self, spec: &Explode, progress: &Progress) -> Result<RowPlan, Error> {
        spec.check(self)?;
        let parts = self.explode_chunks(spec, progress, Vec::new, |st: &mut Vec<u32>, id, v| {
            if read_cell(v, spec) == Read::Bad {
                st.push(id);
            }
            Ok(())
        })?;
        let ids: Vec<u32> = parts.into_iter().flatten().collect();
        if ids.is_empty() {
            return Err(Error::Invalid("every filled cell could be read".into()));
        }
        let mut plan = self.plan_show_ids(&ids, spec.keep_first);
        if spec.how != ExplodeHow::Rows {
            // Columns read every row, hidden ones too.
            plan.chip.looked = crate::chips::Looked::All;
        }
        Ok(plan)
    }

    /// Before and after for the first [`PREVIEW_ROWS`] shown rows only.
    pub fn explode_preview(&self, spec: &Explode) -> Result<ExplodePreview, Error> {
        spec.check(self)?;
        let reader = Cols::new(self, &[spec.col]);
        let rows = self.data_rows(spec.keep_first);
        let mut buf = Vec::new();
        let mut cells: Vec<(u64, String)> = Vec::new();
        for r in rows.start..rows.end.min(rows.start + PREVIEW_ROWS) {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            cells.push((r, reader.get(&buf, 0).to_string()));
        }
        let reads: Vec<Read> = cells.iter().map(|(_, v)| read_cell(v, spec)).collect();
        // The columns: the listed keys, else those in these rows.
        let names: Vec<String> = if spec.how == ExplodeHow::Rows {
            Vec::new()
        } else if spec.fan_out() {
            let most = reads
                .iter()
                .map(|r| match r {
                    Read::Items(i) => i.len(),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
                .min(MAX_NEW_COLS as usize);
            (1..=most).map(|k| k.to_string()).collect()
        } else {
            let mut k = Keys::default();
            if let Some(list) = &spec.keys {
                for n in list {
                    k.add(norm_key(n, spec), n, 0);
                }
            }
            if spec.keys.is_none() || spec.add_new_keys {
                for r in &reads {
                    match r {
                        Read::Pairs(p) => {
                            for (n, _) in p {
                                k.add(Cow::Borrowed(n), n, 1);
                            }
                        }
                        Read::Items(items) => {
                            for it in items {
                                let t = it.text(spec);
                                if !t.is_empty() {
                                    k.add(norm_key(&t, spec), &t, 1);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            k.names.truncate(MAX_NEW_COLS as usize);
            k.names
        };
        let titles = self.explode_titles(spec, &names);
        let cell = |v: &Val| match v {
            Val::Null => PreviewCell {
                text: spec.null_text().into(),
                shown: Shown::Null,
            },
            Val::Text(t) if t.is_empty() => PreviewCell {
                text: String::new(),
                shown: Shown::Empty,
            },
            Val::Text(t) => PreviewCell {
                text: cut(t),
                shown: Shown::Value,
            },
        };
        let missing = || PreviewCell {
            text: String::new(),
            shown: Shown::Missing,
        };
        let mut out = ExplodePreview {
            titles,
            rows: Vec::new(),
        };
        for ((r, before), read) in cells.iter().zip(&reads) {
            let mut row = PreviewRow {
                row: *r,
                before: cut(before),
                bad: *read == Read::Bad,
                lines: Vec::new(),
            };
            match (read, spec.how) {
                (Read::Pairs(p), _) => {
                    let line = names
                        .iter()
                        .map(|n| {
                            p.iter()
                                .find(|(k, _)| k == n)
                                .map_or_else(missing, |(_, v)| cell(v))
                        })
                        .collect();
                    row.lines.push(line);
                }
                (Read::Items(items), ExplodeHow::Flags) => {
                    let here: HashSet<String> = items
                        .iter()
                        .map(|i| norm_key(&i.text(spec), spec).into_owned())
                        .collect();
                    let line = names
                        .iter()
                        .map(|n| {
                            if here.contains(norm_key(n, spec).as_ref()) {
                                PreviewCell {
                                    text: spec.mark.clone(),
                                    shown: Shown::Value,
                                }
                            } else {
                                missing()
                            }
                        })
                        .collect();
                    row.lines.push(line);
                }
                (Read::Items(items), ExplodeHow::Columns) => {
                    let line = (0..names.len())
                        .map(|k| items.get(k).map_or_else(missing, cell))
                        .collect();
                    row.lines.push(line);
                }
                (Read::Items(items), ExplodeHow::Rows) => {
                    let list_left = spec.kind == ExplodeKind::List && items.len() < 2;
                    let orig = PreviewCell {
                        text: cut(before),
                        shown: Shown::Value,
                    };
                    if list_left {
                        let mut line = vec![PreviewCell {
                            text: cut(before),
                            shown: Shown::Value,
                        }];
                        if spec.original_col {
                            line.push(orig);
                        }
                        row.lines.push(line);
                    } else if items.is_empty() {
                        let mut line = vec![PreviewCell {
                            text: String::new(),
                            shown: Shown::Empty,
                        }];
                        if spec.original_col {
                            line.push(orig);
                        }
                        row.lines.push(line);
                    } else {
                        for it in items.iter().take(MAX_PARTS) {
                            let mut line = vec![cell(it)];
                            if spec.original_col {
                                line.push(orig.clone());
                            }
                            row.lines.push(line);
                        }
                    }
                }
                _ => {}
            }
            out.rows.push(row);
        }
        Ok(out)
    }

    /// Plan one column per key, per item position (fan out) or per tag
    /// (flags), over every row (hidden ones too), as one undo step.
    pub fn plan_explode_columns(
        &self,
        spec: &Explode,
        progress: &Progress,
    ) -> Result<(ColPlan, ExplodeReport), Error> {
        spec.check(self)?;
        if spec.how == ExplodeHow::Rows {
            return Err(Error::Invalid("unnest to rows is a rows step".into()));
        }
        let spent = AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        let flags = spec.how == ExplodeHow::Flags;
        let fan = spec.fan_out();
        let charge = |len: usize| -> Result<(), Error> {
            if crate::find::charge_within(&spent, len, budget) {
                Ok(())
            } else {
                Err(Error::OutOfMemory(
                    "this would make too many new cells at once; untick some keys or explode fewer rows",
                ))
            }
        };
        let parts = self.explode_chunks(spec, progress, Chunk::default, |st, id, v| {
            let read = read_cell(v, spec);
            let mut cells: Vec<(u32, String)> = Vec::new();
            match read {
                Read::Empty => return Ok(()),
                Read::Bad => {
                    st.bad += 1;
                    return Ok(());
                }
                Read::Pairs(p) => {
                    for (k, val) in p {
                        if let Some(i) = st.keys.add(Cow::Borrowed(&k), &k, 1) {
                            let t = val.text(spec);
                            charge(t.len())?;
                            cells.push((i, t));
                        }
                    }
                }
                Read::Items(items) if fan => {
                    if items.len() > MAX_NEW_COLS as usize {
                        return Err(Error::Invalid(format!(
                            "a cell holds more than {MAX_NEW_COLS} items; at most {MAX_NEW_COLS} columns can be added"
                        )));
                    }
                    st.most = st.most.max(items.len() as u64);
                    for (k, it) in items.iter().enumerate() {
                        let t = it.text(spec);
                        charge(t.len())?;
                        cells.push((k as u32, t));
                    }
                }
                Read::Items(items) => {
                    let mut here = HashSet::new();
                    for it in &items {
                        let t = it.text(spec);
                        if t.is_empty() {
                            continue;
                        }
                        let n = norm_key(&t, spec).into_owned();
                        if !here.insert(n.clone()) {
                            continue;
                        }
                        if let Some(i) = st.keys.add(Cow::Owned(n), &t, 1) {
                            charge(spec.mark.len())?;
                            cells.push((i, String::new()));
                        }
                    }
                }
            }
            if !cells.is_empty() || !spec.keep_original {
                st.rows.push((id, cells));
            }
            Ok(())
        })?;
        let mut rep = ExplodeReport::default();
        let mut most = 0u64;
        let mut keys = Vec::with_capacity(parts.len());
        let mut rows = Vec::with_capacity(parts.len());
        for p in parts {
            rep.bad += p.bad;
            most = most.max(p.most);
            keys.push(p.keys);
            rows.push(p.rows);
        }
        if rep.bad > 0 && !spec.keep_original {
            return Err(Error::Invalid(format!(
                "{} {} did not parse; keep the original column so {} stay as {} are (or fix them first: Show those rows)",
                rep.bad,
                if rep.bad == 1 { "cell" } else { "cells" },
                if rep.bad == 1 { "it" } else { "they" },
                if rep.bad == 1 { "it" } else { "they" },
            )));
        }
        let (all, maps) = Keys::merge(keys, spec);
        // The columns: which found key goes to new column k.
        let mut names: Vec<String> = Vec::new();
        let mut slot_of: Vec<Option<u32>> = vec![None; all.names.len()];
        if fan {
            names = (1..=most).map(|k| k.to_string()).collect();
        } else {
            let too_many = |n: usize| {
                Error::Invalid(format!(
                    "this would add {n} columns; at most {MAX_NEW_COLS} can be added: untick some keys"
                ))
            };
            match &spec.keys {
                Some(list) => {
                    let mut listed = HashSet::new();
                    for n in list {
                        let norm = norm_key(n, spec);
                        if !listed.insert(norm.clone().into_owned()) {
                            continue;
                        }
                        match all.find(&norm) {
                            Some(i) => slot_of[i as usize] = Some(names.len() as u32),
                            None => {
                                rep.missing_count += 1;
                                if rep.missing_keys.len() < MAX_NAMED {
                                    rep.missing_keys.push(n.clone());
                                }
                            }
                        }
                        names.push(n.clone());
                    }
                    rep.new_added = spec.add_new_keys;
                    for (i, n) in all.names.iter().enumerate() {
                        if listed.contains(norm_key(n, spec).as_ref()) {
                            continue;
                        }
                        rep.new_count += 1;
                        if rep.new_keys.len() < MAX_NAMED {
                            rep.new_keys.push(n.clone());
                        }
                        if spec.add_new_keys {
                            slot_of[i] = Some(names.len() as u32);
                            names.push(n.clone());
                        }
                    }
                    if spec.add_new_keys && all.over {
                        return Err(too_many(MAX_TRACKED));
                    }
                }
                None => {
                    if all.over {
                        return Err(too_many(MAX_TRACKED));
                    }
                    for (i, n) in all.names.iter().enumerate() {
                        slot_of[i] = Some(i as u32);
                        names.push(n.clone());
                    }
                }
            }
            if names.len() > MAX_NEW_COLS as usize {
                return Err(too_many(names.len()));
            }
        }
        let n = names.len() as u32;
        if n == 0 {
            return Err(Error::Invalid(if spec.kind.has_keys() {
                "nothing to add: no keys were found, or none are ticked".into()
            } else {
                "nothing to add: no items were found".into()
            }));
        }
        let mut plan = ColPlan::new(self);
        let at = spec.col + 1;
        let new = if spec.keep_original { n } else { n - 1 };
        if new > 0 {
            plan.insert = Some((at, new));
        }
        plan.cols = new;
        rep.cols = new;
        let slot = |k: u32| -> u32 {
            if spec.keep_original {
                at + k
            } else if k == 0 {
                spec.col
            } else {
                at + k - 1
            }
        };
        if spec.keep_first && self.total_rows() > 0 {
            let id = self.doc_id_at(0).expect("a first row");
            let titles = self.explode_titles(spec, &names);
            let cells: Vec<(u32, String)> = titles
                .into_iter()
                .enumerate()
                .map(|(k, t)| (slot(k as u32), t))
                .collect();
            plan.cells += cells.len() as u64;
            plan.changes.push((id, cells));
        }
        for (chunk, map) in rows.into_iter().zip(&maps) {
            for (id, vals) in chunk {
                let mut cells: Vec<(u32, String)> = Vec::with_capacity(vals.len() + 1);
                let mut first = false;
                for (i, t) in vals {
                    let k = if fan {
                        Some(i)
                    } else {
                        map[i as usize].and_then(|g| slot_of[g as usize])
                    };
                    let Some(k) = k else { continue };
                    first |= k == 0;
                    let t = if flags { spec.mark.clone() } else { t };
                    if t.is_empty() && !(k == 0 && !spec.keep_original) {
                        continue;
                    }
                    cells.push((slot(k), t));
                }
                if !spec.keep_original && !first {
                    // The first new column takes the cell's place: blank it.
                    cells.push((spec.col, String::new()));
                }
                if cells.is_empty() {
                    continue;
                }
                plan.cells += cells.len() as u64;
                plan.changes.push((id, cells));
            }
        }
        Ok((plan, rep))
    }

    /// Plan one row per list item over shown rows (hidden rows are left
    /// alone), with `original_col` a column holding the whole cell, all
    /// as one undo step.
    pub fn plan_explode_rows(
        &self,
        spec: &Explode,
        progress: &Progress,
    ) -> Result<(RowPlan, ExplodeReport), Error> {
        spec.check(self)?;
        if spec.how != ExplodeHow::Rows || spec.kind.has_keys() {
            return Err(Error::Invalid("only lists can be unnested to rows".into()));
        }
        let added = AtomicUsize::new(0);
        let spent = AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        struct Out {
            id: u32,
            /// The cell's new value and the rows after it (`None`: left).
            items: Option<Vec<String>>,
            original: Option<String>,
        }
        let parts = self.explode_chunks(
            spec,
            progress,
            || (Vec::<Out>::new(), 0u64),
            |(st, bad), id, v| {
                let read = read_cell(v, spec);
                if read == Read::Empty {
                    return Ok(());
                }
                let original = spec.original_col.then(|| v.to_string());
                let items = match read {
                    Read::Bad => {
                        *bad += 1;
                        None
                    }
                    // A delimited list of one item is left (as Split into rows).
                    Read::Items(i) if spec.kind == ExplodeKind::List && i.len() < 2 => None,
                    Read::Items(i) => {
                        if i.len() > MAX_PARTS {
                            return Err(Error::Invalid(format!(
                                "a cell holds more than {MAX_PARTS} items"
                            )));
                        }
                        let more = i.len().saturating_sub(1);
                        if added.fetch_add(more, Ordering::Relaxed) + more > MAX_NEW_ROWS as usize {
                            return Err(Error::Invalid(format!(
                                "this would add more than {MAX_NEW_ROWS} rows at once"
                            )));
                        }
                        let items: Vec<String> = i.iter().map(|x| x.text(spec)).collect();
                        let len: usize = items.iter().map(String::len).sum::<usize>()
                            + original.as_ref().map_or(0, String::len);
                        if !crate::find::charge_within(&spent, len, budget) {
                            return Err(Error::OutOfMemory(
                                "this would make too many new rows at once; unnest fewer rows",
                            ));
                        }
                        Some(items)
                    }
                    _ => None,
                };
                if items.is_some() || original.is_some() {
                    st.push(Out {
                        id,
                        items,
                        original,
                    });
                }
                Ok(())
            },
        )?;
        let mut rep = ExplodeReport::default();
        let mut plan = RowPlan::new(self);
        let c = spec.col as usize;
        let oc = spec.col + 1;
        if spec.original_col {
            plan.insert_col = Some(oc);
            rep.cols = 1;
            if spec.keep_first && self.row_count() > 0 {
                let id = self.id_at(0).expect("a first row");
                let t = self.explode_titles(spec, &[]).remove(0);
                plan.changes.push((id, vec![(oc, t)]));
            }
        }
        for (chunk, bad) in parts {
            rep.bad += bad;
            for o in chunk {
                let mut cells = Vec::new();
                if let Some(items) = &o.items {
                    cells.push((spec.col, items.first().cloned().unwrap_or_default()));
                }
                if let Some(orig) = &o.original {
                    cells.push((oc, orig.clone()));
                }
                if let Some(items) = o.items {
                    if items.len() > 1 {
                        let mut values = self.logical_values(o.id)?;
                        if values.len() <= c {
                            values.resize(c + 1, String::new());
                        }
                        if let Some(orig) = &o.original {
                            values.insert(oc as usize, orig.clone());
                        }
                        let mut extra = Vec::with_capacity(items.len() - 1);
                        for it in items.into_iter().skip(1) {
                            let mut row = values.clone();
                            row[c] = it;
                            extra.push(row);
                        }
                        plan.rows += extra.len() as u64;
                        plan.groups += 1;
                        plan.after.push((o.id, extra));
                    }
                }
                plan.changes.push((o.id, cells));
            }
        }
        rep.rows = plan.rows;
        Ok((plan, rep))
    }
}

/// Guess the kind from sampled cells.
pub(crate) fn detect(cells: &[String]) -> ExplodeDetect {
    let n = cells.len() as u64;
    let mut cands: Vec<(ExplodeKind, String, String, u64)> = Vec::new();
    let probe = |kind, sep: &str, kv: &str| Explode {
        kind,
        separator: sep.into(),
        kv: kv.into(),
        trim: true,
        ..Default::default()
    };
    let objects = probe(ExplodeKind::Object, "|", "=");
    let lists = probe(ExplodeKind::JsonList, "|", "=");
    let count = |spec: &Explode| {
        cells
            .iter()
            .filter(|c| matches!(read_cell(c, spec), Read::Pairs(_) | Read::Items(_)))
            .count() as u64
    };
    cands.push((ExplodeKind::Object, "|".into(), "=".into(), count(&objects)));
    cands.push((ExplodeKind::JsonList, "|".into(), "=".into(), count(&lists)));
    // key=value: only when some cell holds two pairs, or the sign is "=".
    let mut best_kv: Option<(String, String, u64)> = None;
    for kv in ["=", ":"] {
        for sep in [";", "&", ",", "|", "\n"] {
            let spec = probe(ExplodeKind::KeyValue, sep, kv);
            let mut m = 0;
            let mut two = false;
            for c in cells {
                if c.trim_start().starts_with(['{', '[']) {
                    continue;
                }
                if let Read::Pairs(p) = read_cell(c, &spec) {
                    if !p.is_empty()
                        && p.iter()
                            .all(|(k, _)| !k.trim().is_empty() && k != "(empty)")
                    {
                        m += 1;
                        two |= p.len() > 1;
                    }
                }
            }
            if m > 0 && (two || kv == "=") && best_kv.as_ref().is_none_or(|b| m > b.2) {
                best_kv = Some((sep.into(), kv.into(), m));
            }
        }
    }
    if let Some((sep, kv, m)) = best_kv {
        cands.push((ExplodeKind::KeyValue, sep, kv, m));
    }
    let mut best_list: Option<(String, u64)> = None;
    for sep in ["|", ";", ",", "\t", "/"] {
        let m = cells
            .iter()
            .filter(|c| !c.trim_start().starts_with(['{', '[']) && c.contains(sep))
            .count() as u64;
        if m > 0 && best_list.as_ref().is_none_or(|b| m > b.1) {
            best_list = Some((sep.into(), m));
        }
    }
    let (lsep, lm) = best_list.unwrap_or_else(|| ("|".into(), 0));
    cands.push((ExplodeKind::List, lsep, "=".into(), lm));
    // Most matched; ties go to the earlier (objects, JSON lists, pairs, lists).
    let mut best = cands[0].clone();
    for c in cands.into_iter().skip(1) {
        if c.3 > best.3 {
            best = c;
        }
    }
    let (kind, separator, kv, matched) = best;
    let what = kind.words(&separator, &kv);
    let words = if n == 0 {
        "the column is empty".into()
    } else if matched as f64 >= SURE_SHARE * n as f64 {
        format!("{what} ({matched} of {n} sampled)")
    } else {
        format!("mixed: {matched} of {n} look like {what}")
    };
    ExplodeDetect {
        kind,
        separator,
        kv,
        matched,
        sampled: n,
        words,
    }
}

#[cfg(test)]
#[path = "explode_tests.rs"]
mod tests;
