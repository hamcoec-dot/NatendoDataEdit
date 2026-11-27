//! Saving JSON Lines and JSON back to the file they came from (C11).
//!
//! When a file of JSON records is opened, each key's values are noted by kind: text, number, true/false,
//! a nested object or list, `null`, an empty text `""`, or no key at all. A save then writes each cell in
//! the kind it had (a number as the exact text it was written with, a nested value as its own JSON), so
//! a save with no edits gives the same records. The shape is never changed by an edit: the kind of an
//! edited cell is decided when saving, from the shape and the cell's text, so undo and redo need nothing.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::document::Progress;
use crate::{Document, Error};

/// What a JSON value was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Kind {
    Text = 0,
    Number = 1,
    Bool = 2,
    Nested = 3,
    Null = 4,
    /// The empty text `""`.
    Empty = 5,
    /// The record had no such key.
    Missing = 6,
}

const KINDS: usize = 7;
const ALL: [Kind; KINDS] = [
    Kind::Text,
    Kind::Number,
    Kind::Bool,
    Kind::Nested,
    Kind::Null,
    Kind::Empty,
    Kind::Missing,
];

impl Kind {
    /// A blank cell in the grid (`null`, `""` or no key).
    pub fn blank(self) -> bool {
        matches!(self, Kind::Null | Kind::Empty | Kind::Missing)
    }
}

/// One cell of a JSON record as read: its kind and the text the grid shows (a number's own text, a
/// nested value's JSON with the spaces outside strings taken out, nothing for `null`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    pub kind: Kind,
    pub text: String,
}

impl Cell {
    pub(crate) fn text(s: String) -> Cell {
        Cell {
            kind: if s.is_empty() {
                Kind::Empty
            } else {
                Kind::Text
            },
            text: s,
        }
    }
    pub(crate) fn of(kind: Kind, text: &str) -> Cell {
        Cell {
            kind,
            text: text.to_string(),
        }
    }
}

/// One JSON row as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Row {
    /// An object's members in file order (a key twice is kept twice).
    Object(Vec<(String, Cell)>),
    Array(Vec<Cell>),
    Value(Cell),
}

/// Rows of one kind, by data row (0 = the first record): a short list, or one bit a row once that is
/// smaller.
#[derive(Debug, Clone)]
enum RowSet {
    List(Vec<u32>),
    Bits(Vec<u64>),
}

impl RowSet {
    fn has(&self, r: u32) -> bool {
        match self {
            RowSet::List(l) => l.binary_search(&r).is_ok(),
            RowSet::Bits(b) => b
                .get((r >> 6) as usize)
                .is_some_and(|w| (w >> (r & 63)) & 1 == 1),
        }
    }
    fn push(&mut self, r: u32, rows: u32) {
        match self {
            RowSet::List(l) => {
                l.push(r);
                // A list takes 32 bits a row; past one in 32 rows bits are smaller.
                if l.len() as u64 * 32 > u64::from(rows).max(64) {
                    let mut b = vec![0u64; (rows as usize).div_ceil(64)];
                    for &x in l.iter() {
                        b[(x >> 6) as usize] |= 1 << (x & 63);
                    }
                    *self = RowSet::Bits(b);
                }
            }
            RowSet::Bits(b) => {
                if let Some(w) = b.get_mut((r >> 6) as usize) {
                    *w |= 1 << (r & 63);
                }
            }
        }
    }
    fn bytes(&self) -> usize {
        match self {
            RowSet::List(l) => l.capacity() * 4,
            RowSet::Bits(b) => b.len() * 8,
        }
    }
}

/// One key: its own text, how many values of each kind it had, and the rows whose kind is not the
/// column's usual one.
#[derive(Debug, Clone)]
pub struct ColShape {
    /// The key exactly as the file has it.
    pub key: String,
    counts: [u64; KINDS],
    /// The most common kind (the rows of every other kind are listed).
    usual: Kind,
    /// The most common kind of a value (for new rows).
    value: Kind,
    odd: Vec<(Kind, RowSet)>,
}

impl ColShape {
    /// The kind record `r` had.
    fn kind_at(&self, r: u32) -> Kind {
        for (k, s) in &self.odd {
            if s.has(r) {
                return *k;
            }
        }
        self.usual
    }

    /// The column had blanks of one kind only (or none).
    fn one_blank(&self) -> Option<Kind> {
        let present: Vec<Kind> = [Kind::Null, Kind::Empty, Kind::Missing]
            .into_iter()
            .filter(|k| self.counts[*k as usize] > 0)
            .collect();
        match present[..] {
            [k] => Some(k),
            _ => None,
        }
    }

    fn has_values(&self) -> bool {
        ALL.iter()
            .any(|k| !k.blank() && self.counts[*k as usize] > 0)
    }
}

/// How the file was laid out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonLayout {
    /// JSON Lines (one record a line), else one array of records.
    pub lines: bool,
    /// An array written over several lines: one level of indent (`"  "`, `"    "`, `"\t"`).
    pub indent: Option<String>,
    pub crlf: bool,
    pub bom: bool,
    /// The file ended with a line break.
    pub final_newline: bool,
}

/// What a file of JSON records held, key by key.
#[derive(Debug, Clone)]
pub struct JsonShape {
    pub layout: JsonLayout,
    pub cols: Vec<ColShape>,
    /// Records read.
    pub rows: u32,
    /// Why it cannot be saved back as it is (then Save is Save As), in words.
    pub refuse: Option<String>,
}

impl JsonShape {
    /// Bytes the shape holds beyond the keys.
    pub fn bytes(&self) -> usize {
        self.cols
            .iter()
            .map(|c| c.key.len() + 96 + c.odd.iter().map(|(_, s)| s.bytes()).sum::<usize>())
            .sum()
    }
}

/// Builds a shape over the two passes of the JSON reader: the first counts each key's kinds (so each
/// column's usual kind is known), the second notes the rows that differ.
pub(crate) struct ShapeBuilder {
    counts: Vec<[u64; KINDS]>,
    rows: u32,
    /// For each column, the row it was last seen in (a key twice in one record).
    seen_in: Vec<u32>,
    refuse: Option<String>,
    second: Option<Vec<ColShape>>,
    row2: u32,
}

impl ShapeBuilder {
    pub(crate) fn new() -> ShapeBuilder {
        ShapeBuilder {
            counts: Vec::new(),
            rows: 0,
            seen_in: Vec::new(),
            refuse: None,
            second: None,
            row2: 0,
        }
    }

    /// Something about the file means it cannot be saved back (the first reason is kept).
    pub(crate) fn refuse(&mut self, why: String) {
        self.refuse.get_or_insert(why);
    }

    /// First pass: one record, its members as (column, kind).
    pub(crate) fn count(
        &mut self,
        cols: usize,
        members: &[(usize, Kind)],
        key: &dyn Fn(usize) -> String,
    ) {
        if self.counts.len() < cols {
            self.counts.resize(cols, [0; KINDS]);
            self.seen_in.resize(cols, u32::MAX);
        }
        let r = self.rows;
        for &(c, k) in members {
            if self.seen_in[c] == r && self.refuse.is_none() {
                self.refuse(format!(
                    "record {} has the key '{}' twice, so a value would be lost",
                    (u64::from(r) + 1).to_string_with_commas(),
                    key(c)
                ));
            }
            self.seen_in[c] = r;
            self.counts[c][k as usize] += 1;
        }
        self.rows += 1;
    }

    /// Between the passes: the usual kinds (missing keys counted now that every key is known).
    pub(crate) fn settle(&mut self, keys: &[String]) {
        self.counts.resize(keys.len(), [0; KINDS]);
        let rows = u64::from(self.rows);
        let cols = keys
            .iter()
            .zip(self.counts.iter_mut())
            .map(|(key, counts)| {
                let present: u64 = counts.iter().sum();
                counts[Kind::Missing as usize] = rows.saturating_sub(present);
                let most = |ks: &[Kind]| {
                    ks.iter()
                        .copied()
                        .max_by_key(|k| (counts[*k as usize], std::cmp::Reverse(*k as u8)))
                        .unwrap_or(ks[0])
                };
                ColShape {
                    key: key.clone(),
                    counts: *counts,
                    usual: most(&ALL),
                    value: most(&[Kind::Text, Kind::Number, Kind::Bool, Kind::Nested]),
                    odd: Vec::new(),
                }
            })
            .collect();
        self.second = Some(cols);
    }

    /// Second pass: one record's kinds by column (`Missing` where it has no key).
    pub(crate) fn note(&mut self, kinds: &[Kind]) {
        let rows = self.rows;
        let r = self.row2;
        if let Some(cols) = self.second.as_mut() {
            for (c, &k) in cols.iter_mut().zip(kinds) {
                if k != c.usual {
                    match c.odd.iter_mut().find(|(x, _)| *x == k) {
                        Some((_, s)) => s.push(r, rows),
                        None => {
                            let mut s = RowSet::List(Vec::new());
                            s.push(r, rows);
                            c.odd.push((k, s));
                        }
                    }
                }
            }
        }
        self.row2 += 1;
    }

    pub(crate) fn finish(self, layout: JsonLayout) -> JsonShape {
        JsonShape {
            layout,
            cols: self.second.unwrap_or_default(),
            rows: self.rows,
            refuse: self.refuse,
        }
    }
}

trait Commas {
    fn to_string_with_commas(&self) -> String;
}

impl Commas for u64 {
    fn to_string_with_commas(&self) -> String {
        let s = self.to_string();
        let mut out = String::new();
        for (i, ch) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(ch);
        }
        out
    }
}

/// How the file's text was laid out, from its bytes and its decoded text.
pub(crate) fn layout_of(data: &[u8], text: &str, lines: bool) -> JsonLayout {
    let bom = data.starts_with(&[0xEF, 0xBB, 0xBF]);
    let crlf = text
        .find('\n')
        .is_some_and(|i| i > 0 && text.as_bytes()[i - 1] == b'\r');
    let final_newline = text.ends_with('\n');
    let indent = if lines {
        None
    } else {
        // "[" then a line break: the first line inside is indented one level.
        let t = text.trim_start();
        t.strip_prefix('[')
            .and_then(|r| r.strip_prefix("\r\n").or_else(|| r.strip_prefix('\n')))
            .map(|r| {
                r.chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
    };
    JsonLayout {
        lines,
        indent,
        crlf,
        bom,
        final_newline,
    }
}

/// A JSON number exactly as the grammar has it (`-?(0|[1-9][0-9]*)(.[0-9]+)?([eE][+-]?[0-9]+)?`):
/// `012`, `1,234`, ` 12`, `+1`, `NaN` and `Infinity` are not.
pub fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'-') {
        i = 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(c) if c.is_ascii_digit() => {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        let from = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == from {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let from = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == from {
            return false;
        }
    }
    i == b.len()
}

/// A JSON object or list as one line (spaces outside strings taken out), or `None` when the text is not
/// one. Numbers keep their own text.
pub fn compact_json(s: &str) -> Option<String> {
    let t = s.trim();
    if !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    serde_json::from_str::<serde::de::IgnoredAny>(t).ok()?;
    let mut out = String::with_capacity(t.len());
    let mut in_str = false;
    let mut esc = false;
    for ch in t.chars() {
        if in_str {
            out.push(ch);
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
        } else if ch == '"' {
            in_str = true;
            out.push(ch);
        } else if !ch.is_whitespace() {
            out.push(ch);
        }
    }
    Some(out)
}

/// How a newly blank cell is written (D-C11: leave the key out unless the person picks otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blanks {
    #[default]
    LeaveOut,
    EmptyText,
    Null,
}

/// The form a save writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonForm {
    pub lines: bool,
    /// An array over several lines with this indent; `None`: one line.
    pub indent: Option<String>,
    pub crlf: bool,
    pub bom: bool,
    pub final_newline: bool,
}

impl From<&JsonLayout> for JsonForm {
    fn from(l: &JsonLayout) -> JsonForm {
        JsonForm {
            lines: l.lines,
            indent: l.indent.clone(),
            crlf: l.crlf,
            bom: l.bom,
            final_newline: l.final_newline,
        }
    }
}

/// Before a save: why it cannot be saved as JSON (the titles), and whether a newly blank cell needs
/// the blank question.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct JsonCheck {
    pub refuse: Option<String>,
    pub ask_blanks: bool,
    /// The file's own layout (when it came from one).
    pub form: Option<JsonForm>,
}

/// After a save.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct JsonReport {
    pub records: u64,
    /// Cells written as text though their column holds numbers, true/false or nested values.
    pub as_text: u64,
    pub bytes: u64,
}

impl Document {
    /// The shape of the JSON file this tab came from, if any.
    pub fn json_shape(&self) -> Option<&JsonShape> {
        self.json_shape.as_deref()
    }

    pub fn set_json_shape(&mut self, shape: Option<JsonShape>) {
        self.json_shape = shape.map(std::sync::Arc::new);
    }

    /// The column titles as JSON keys (row 1), checked: none blank or used twice (a blank key the file
    /// itself had is kept).
    fn json_keys(&self) -> Result<Vec<String>, Error> {
        let n = self.col_count();
        let first = if self.total_rows() > 0 {
            self.row_values_doc(0)?
        } else {
            Vec::new()
        };
        let mut keys = Vec::with_capacity(n as usize);
        let mut seen = std::collections::HashSet::new();
        for c in 0..n {
            let title = first.get(c as usize).cloned().unwrap_or_default();
            let phys = self.physical_col(c);
            let original = self
                .json_shape
                .as_ref()
                .and_then(|s| phys.and_then(|p| s.cols.get(p as usize)))
                .is_some_and(|s| s.key == title);
            if title.is_empty() && !original {
                return Err(Error::Invalid(format!(
                    "column {} has no title, and JSON needs a name for each value: give it one",
                    c + 1
                )));
            }
            if !seen.insert(title.clone()) {
                return Err(Error::Invalid(format!(
                    "two columns are both called '{title}': rename one to save as JSON"
                )));
            }
            keys.push(title);
        }
        Ok(keys)
    }

    /// Whether this tab can be saved as JSON, and whether the blank question is needed.
    pub fn json_check(&self) -> JsonCheck {
        let mut out = JsonCheck {
            form: self.json_shape.as_ref().map(|s| JsonForm::from(&s.layout)),
            ..Default::default()
        };
        if let Some(why) = self.json_shape.as_ref().and_then(|s| s.refuse.clone()) {
            out.refuse = Some(why);
            return out;
        }
        if let Err(e) = self.json_keys() {
            out.refuse = Some(e.to_string());
            return out;
        }
        let mut ask = false;
        let _ = self.json_walk(Blanks::LeaveOut, &mut |_, _| {}, &mut ask, &mut 0);
        out.ask_blanks = ask;
        out
    }

    /// Each record in order as (key index, kind, text) members; `ask` is set when a newly blank cell
    /// had no kind of its own (the blank question), `as_text` counts cells that no longer fit their kind.
    #[allow(clippy::type_complexity, clippy::needless_range_loop)]
    fn json_walk(
        &self,
        blanks: Blanks,
        each: &mut dyn FnMut(usize, &[(usize, Kind, &str)]),
        ask: &mut bool,
        as_text: &mut u64,
    ) -> Result<(), Error> {
        let n = self.col_count();
        let cols: Vec<u32> = (0..n).collect();
        let phys: Vec<Option<u32>> = cols.iter().map(|&c| self.physical_col(c)).collect();
        let shape = self.json_shape.as_deref();
        let reader = crate::ops::Cols::new(self, &cols);
        let mut buf = Vec::new();
        let total = self.total_rows();
        let new_blank = |c: Option<&ColShape>| -> Option<Kind> { c.and_then(ColShape::one_blank) };
        for pos in 1..total {
            let id = self.doc_id_at(pos).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            // The record this row was in the file (row ids follow the file's records; new rows come after).
            let orig = shape.and_then(|s| (id >= 1 && id - 1 < s.rows).then_some(id - 1));
            let mut members: Vec<(usize, Kind, &str)> = Vec::with_capacity(n as usize);
            for k in 0..n as usize {
                let text: &str = reader.get(&buf, k);
                let col = shape.and_then(|s| phys[k].and_then(|p| s.cols.get(p as usize)));
                let was = match (col, orig) {
                    (Some(c), Some(r)) => Some(c.kind_at(r)),
                    _ => None,
                };
                let kind = if text.is_empty() {
                    match was {
                        // Unchanged blanks keep their own form.
                        Some(k) if k.blank() => k,
                        _ => match new_blank(col) {
                            Some(k) => k,
                            None => {
                                *ask = true;
                                match blanks {
                                    Blanks::LeaveOut => Kind::Missing,
                                    Blanks::EmptyText => Kind::Empty,
                                    Blanks::Null => Kind::Null,
                                }
                            }
                        },
                    }
                } else {
                    let want = match (was, col) {
                        (Some(k), _) if !k.blank() => k,
                        (_, Some(c)) if c.has_values() => c.value,
                        _ => Kind::Text,
                    };
                    let fits = match want {
                        Kind::Number => is_json_number(text),
                        Kind::Bool => text == "true" || text == "false",
                        Kind::Nested => compact_json(text).is_some(),
                        _ => true,
                    };
                    if fits {
                        want
                    } else {
                        *as_text += 1;
                        Kind::Text
                    }
                };
                members.push((k, kind, text));
            }
            each(pos as usize - 1, &members);
        }
        Ok(())
    }

    /// Save every row as JSON Lines or a JSON array of records to `path` (a temp file moved into place),
    /// each value in the kind its key had in the file it came from. The tab is marked saved at this step;
    /// its own text is not replaced (the undo history stays).
    pub fn save_json(
        &mut self,
        path: &Path,
        form: &JsonForm,
        blanks: Blanks,
        progress: &Progress,
    ) -> Result<JsonReport, Error> {
        let report = self.write_json(path, form, blanks, progress)?;
        self.mark_saved_here();
        Ok(report)
    }

    /// [`Document::save_json`] without marking the tab saved: a copy of a tab that has a file of its own
    /// (a CSV tab saved as JSON Lines).
    pub fn write_json(
        &self,
        path: &Path,
        form: &JsonForm,
        blanks: Blanks,
        progress: &Progress,
    ) -> Result<JsonReport, Error> {
        self.ready_for_bulk()?;
        if let Some(why) = self.json_shape.as_ref().and_then(|s| s.refuse.clone()) {
            return Err(Error::Invalid(format!(
                "this can only be saved as another file: {why}"
            )));
        }
        let keys = self.json_keys()?;
        let quoted: Vec<String> = keys
            .iter()
            .map(|k| serde_json::to_string(k).unwrap_or_default())
            .collect();
        let nl = if form.crlf { "\r\n" } else { "\n" };
        let (open, sep, ind1, ind2, colon) = match (&form.lines, &form.indent) {
            (false, Some(ind)) => (
                format!("[{nl}"),
                format!(",{nl}"),
                ind.clone(),
                ind.repeat(2),
                ": ",
            ),
            (false, None) => (
                "[".to_string(),
                ",".to_string(),
                String::new(),
                String::new(),
                ":",
            ),
            (true, _) => (
                String::new(),
                nl.to_string(),
                String::new(),
                String::new(),
                ":",
            ),
        };
        let pretty = !form.lines && form.indent.is_some();
        let (temp, file) = crate::save::create_temp(path)?;
        let mut w = std::io::BufWriter::with_capacity(1 << 20, file);
        let mut report = JsonReport::default();
        let mut err: Option<std::io::Error> = None;
        let mut out = String::new();
        let total = self.total_rows().saturating_sub(1);
        progress.start(total);
        let mut ask = false;
        let mut as_text = 0u64;
        let mut first = true;
        let mut written = 0u64;
        {
            use std::io::Write;
            if form.bom {
                written += 3;
                if let Err(e) = w.write_all(&[0xEF, 0xBB, 0xBF]) {
                    err = Some(e);
                }
            }
            written += open.len() as u64;
            if let Err(e) = w.write_all(open.as_bytes()) {
                err.get_or_insert(e);
            }
            let mut step = 0u64;
            let mut stopped = None;
            let walked = self.json_walk(
                blanks,
                &mut |_, members| {
                    if err.is_some() || stopped.is_some() {
                        return;
                    }
                    out.clear();
                    if !first {
                        out.push_str(&sep);
                    }
                    first = false;
                    out.push_str(&ind1);
                    out.push('{');
                    let mut any = false;
                    for &(k, kind, text) in members {
                        if kind == Kind::Missing {
                            continue;
                        }
                        if any {
                            out.push(',');
                        }
                        any = true;
                        if pretty {
                            out.push_str(nl);
                            out.push_str(&ind2);
                        }
                        out.push_str(&quoted[k]);
                        out.push_str(colon);
                        match kind {
                            Kind::Number | Kind::Bool => out.push_str(text),
                            Kind::Nested => match compact_json(text) {
                                Some(j) => out.push_str(&j),
                                None => json_string(text, &mut out),
                            },
                            Kind::Null => out.push_str("null"),
                            Kind::Empty => out.push_str("\"\""),
                            Kind::Text | Kind::Missing => json_string(text, &mut out),
                        }
                    }
                    if pretty && any {
                        out.push_str(nl);
                        out.push_str(&ind1);
                    }
                    out.push('}');
                    written += out.len() as u64;
                    if let Err(e) = w.write_all(out.as_bytes()) {
                        err = Some(e);
                    }
                    report.records += 1;
                    step += 1;
                    if step % 65_536 == 0 {
                        if let Err(e) = progress.step(65_536) {
                            stopped = Some(e);
                        }
                    }
                },
                &mut ask,
                &mut as_text,
            );
            let close = match (form.lines, report.records > 0) {
                (true, true) if form.final_newline => nl.to_string(),
                (true, _) => String::new(),
                (false, _) => {
                    let mut c = String::new();
                    if pretty && report.records > 0 {
                        c.push_str(nl);
                    }
                    c.push(']');
                    if form.final_newline {
                        c.push_str(nl);
                    }
                    c
                }
            };
            written += close.len() as u64;
            if let Err(e) = w.write_all(close.as_bytes()) {
                err.get_or_insert(e);
            }
            let flushed = w.flush().and_then(|_| w.get_ref().sync_all());
            let result = walked.and(match stopped {
                Some(e) => Err(e),
                None => Ok(()),
            });
            if let Err(e) = result {
                drop(w);
                let _ = std::fs::remove_file(&temp);
                return Err(e);
            }
            if let Some(e) = err.or(flushed.err()) {
                drop(w);
                let _ = std::fs::remove_file(&temp);
                return Err(e.into());
            }
        }
        drop(w);
        if let Err(e) = crate::save::replace(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(e.into());
        }
        report.as_text = as_text;
        report.bytes = written;
        Ok(report)
    }
}

/// `s` as a JSON string (quotes included).
fn json_string(s: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()));
}

#[cfg(test)]
#[path = "jsonshape_tests.rs"]
mod tests;
