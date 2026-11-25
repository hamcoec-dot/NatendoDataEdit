//! Importing other formats as a new table: spreadsheets (see
//! [`crate::sheet`]), JSON, XML, HTML tables, Markdown tables, and
//! delimited text (for the clipboard; files use Open).
//!
//! Every importer builds through [`Sink`], which holds the output to the
//! same caps as pivot (50M cells, 1 GiB of text).

use std::collections::HashMap;

use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::document::{Document, OpenOptions, Progress};
use crate::encoding;
use crate::reshape::Table;
use crate::sheet::{Book, SheetInfo, SheetOptions, SheetStats};
use crate::table::{
    attrs, local, push_ref, push_text, text_of, xml_err, xml_reader_str, Sink, MAX_DEPTH,
};
use crate::zipread::damaged;
use crate::Error;

/// Largest text file (JSON, XML, HTML, Markdown) imported. JSON is held
/// whole while it is read, so it has a lower cap.
pub const MAX_TEXT_INPUT: usize = 1 << 30;
pub const MAX_JSON_INPUT: usize = 256 << 20;
/// Bytes of delimited text read for a preview.
pub const PREVIEW_BYTES: usize = 4 << 20;
/// Most columns an import creates.
pub const MAX_IMPORT_COLS: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    Spreadsheet,
    Json,
    Xml,
    Html,
    Markdown,
    Delimited,
    /// Columns at fixed character positions.
    FixedWidth,
    /// Lines cut by a pattern (log files).
    Pattern,
}

/// What the import screen can choose from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportOptions {
    /// `None`: guessed.
    pub format: Option<ImportFormat>,
    /// Spreadsheet sheet, or HTML/Markdown table, by number.
    pub part: u32,
    /// Spreadsheets: keep dates as stored numbers.
    pub raw_dates: bool,
    /// Spreadsheets: numbers shown with a zero-padding format (`00123`): `Some(true)` writes them padded,
    /// `Some(false)` or `None` keeps the number (`None` also counts them, so the screen can ask).
    pub pad_zeros: Option<bool>,
    /// XML: the record element's path (`a/b/item`); empty picks the most
    /// repeated one.
    pub record: String,
    /// Fixed width: where columns start (characters from 0); empty: guessed.
    pub starts: Vec<u32>,
    /// Fixed width: keep the spaces around values.
    pub keep_spaces: bool,
    /// Pattern: the regular expression for a line; empty: guessed preset.
    pub pattern: String,
    /// Pattern: lines that do not fit.
    pub unmatched: crate::logs::Unmatched,
}

/// What a source holds, for the import screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportInfo {
    pub format: Option<ImportFormat>,
    /// Sheets (spreadsheets) or tables (HTML, Markdown) to choose from.
    pub parts: Vec<String>,
    /// Hidden sheets (same order as `parts`).
    pub hidden: Vec<bool>,
    /// XML record paths, most repeated first.
    pub records: Vec<String>,
    /// Fixed width: guessed column starts.
    pub starts: Vec<u32>,
    /// Fixed width and pattern: the first lines, for the ruler and preview.
    pub lines: Vec<String>,
    /// Pattern: the preset that fits best (its pattern).
    pub pattern: String,
}

/// Guess the format from a file extension and the first bytes.
pub fn guess(ext: Option<&str>, data: &[u8]) -> ImportFormat {
    let ext = ext.unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "xlsx" | "xlsm" | "ods" => return ImportFormat::Spreadsheet,
        "json" | "jsonl" | "ndjson" | "geojson" => return ImportFormat::Json,
        "xml" => return ImportFormat::Xml,
        "html" | "htm" | "xhtml" => return ImportFormat::Html,
        "md" | "markdown" => return ImportFormat::Markdown,
        "prn" | "fwf" => return ImportFormat::FixedWidth,
        "log" => return ImportFormat::Pattern,
        _ => {}
    }
    if crate::zipread::is_zip(data) {
        return ImportFormat::Spreadsheet;
    }
    let text = decode(&data[..data.len().min(64 << 10)]);
    let t = text.trim_start();
    let mut cut = t.len().min(4096);
    while !t.is_char_boundary(cut) {
        cut -= 1;
    }
    let lower = t[..cut].to_ascii_lowercase();
    if t.starts_with('{') || t.starts_with('[') {
        ImportFormat::Json
    } else if lower.contains("<table")
        || lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
    {
        ImportFormat::Html
    } else if t.starts_with('<') {
        ImportFormat::Xml
    } else if t.lines().take(50).any(is_md_rule) {
        ImportFormat::Markdown
    } else {
        ImportFormat::Delimited
    }
}

fn decode(data: &[u8]) -> String {
    let d = encoding::detect(data);
    encoding::decode_all(&data[d.bom_len.min(data.len())..], d.encoding).0
}

/// The start of the text (up to the preview size, cut at a line end).
fn text_head(data: &[u8]) -> String {
    let cut = if data.len() > PREVIEW_BYTES {
        data[..PREVIEW_BYTES]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(PREVIEW_BYTES, |i| i + 1)
    } else {
        data.len()
    };
    decode(&data[..cut])
}

fn text_input(data: &[u8], cap: usize) -> Result<String, Error> {
    if data.len() > cap {
        return Err(Error::Invalid(format!(
            "the file is larger than {} MB, too large to import in this format",
            cap >> 20
        )));
    }
    Ok(decode(data))
}

/// What the source holds (sheets, tables, record paths).
pub fn inspect(
    data: &[u8],
    ext: Option<&str>,
    format: Option<ImportFormat>,
) -> Result<ImportInfo, Error> {
    let format = format.unwrap_or_else(|| guess(ext, data));
    let mut info = ImportInfo {
        format: Some(format),
        ..Default::default()
    };
    match format {
        ImportFormat::Spreadsheet => {
            let b = Book::open(data)?;
            for SheetInfo { name, hidden } in b.sheets() {
                info.parts.push(name.clone());
                info.hidden.push(*hidden);
            }
            if info.parts.is_empty() {
                return Err(Error::Invalid(
                    "the workbook has no sheets with cells".into(),
                ));
            }
        }
        ImportFormat::Html => {
            let text = text_input(data, MAX_TEXT_INPUT)?;
            info.parts = html_tables(&text, None, &mut |_| Ok(true))?
                .iter()
                .enumerate()
                .map(|(i, n)| format!("Table {} ({} rows)", i + 1, n))
                .collect();
        }
        ImportFormat::Markdown => {
            let text = text_input(data, MAX_TEXT_INPUT)?;
            info.parts = md_tables(&text, None, &mut |_| Ok(true))?
                .iter()
                .enumerate()
                .map(|(i, n)| format!("Table {} ({} rows)", i + 1, n))
                .collect();
        }
        ImportFormat::Xml => {
            let text = text_input(data, MAX_TEXT_INPUT)?;
            info.records = xml_records(&text)?;
        }
        ImportFormat::FixedWidth => {
            let text = text_head(data);
            info.starts = crate::fixed::guess_starts(crate::fixed::lines(&text));
            info.lines = crate::fixed::ruler_lines(&text);
        }
        ImportFormat::Pattern => {
            let text = text_head(data);
            info.pattern = crate::logs::PRESETS[crate::logs::guess(&text)]
                .1
                .to_string();
            info.lines = crate::fixed::ruler_lines(&text);
        }
        ImportFormat::Json | ImportFormat::Delimited => {}
    }
    Ok(info)
}

/// Import as a table (at most `limit` rows when given, for previews).
pub fn import(
    data: &[u8],
    ext: Option<&str>,
    opts: &ImportOptions,
    limit: Option<u64>,
    progress: &Progress,
) -> Result<Table, Error> {
    import_report(data, ext, opts, limit, progress).map(|r| r.0)
}

/// [`import`], and what the "exactly as written" guards did (counts and column numbers only): whole
/// numbers of more than 15 digits kept in full, digits a spreadsheet had already lost, and numbers shown
/// with a zero-padding format.
pub fn import_report(
    data: &[u8],
    ext: Option<&str>,
    opts: &ImportOptions,
    limit: Option<u64>,
    progress: &Progress,
) -> Result<(Table, SheetStats), Error> {
    let format = opts.format.unwrap_or_else(|| guess(ext, data));
    let mut sink = Sink::new(limit);
    match format {
        ImportFormat::Spreadsheet => {
            let b = Book::open(data)?;
            let t = b.read(
                opts.part as usize,
                SheetOptions {
                    raw_dates: opts.raw_dates,
                    pad_zeros: opts.pad_zeros,
                },
                limit,
                progress,
            )?;
            return Ok((t, b.stats()));
        }
        ImportFormat::Json => json(&text_input(data, MAX_JSON_INPUT)?, limit, &mut sink)?,
        ImportFormat::Xml => xml(
            &text_input(data, MAX_TEXT_INPUT)?,
            &opts.record,
            limit,
            &mut sink,
        )?,
        ImportFormat::Html | ImportFormat::Markdown => {
            let text = text_input(data, MAX_TEXT_INPUT)?;
            let want = opts.part as usize;
            let mut on = |r: Vec<String>| -> Result<bool, Error> {
                sink.row(&r)?;
                Ok(!sink.full())
            };
            let counts = if format == ImportFormat::Html {
                html_tables(&text, Some(want), &mut on)?
            } else {
                md_tables(&text, Some(want), &mut on)?
            };
            if counts.len() <= want {
                return Err(Error::Invalid("that table is not in the file".into()));
            }
        }
        ImportFormat::FixedWidth | ImportFormat::Pattern => {
            // A preview reads only the start of the text.
            let text = match limit {
                Some(_) => text_head(data),
                None => text_input(data, MAX_TEXT_INPUT)?,
            };
            if format == ImportFormat::FixedWidth {
                crate::fixed::read(&text, &opts.starts, opts.keep_spaces, &mut sink)?;
            } else {
                let pattern = if opts.pattern.trim().is_empty() {
                    crate::logs::PRESETS[crate::logs::guess(&text)].1
                } else {
                    opts.pattern.as_str()
                };
                crate::logs::read(&text, pattern, opts.unmatched, &mut sink)?;
            }
        }
        ImportFormat::Delimited => {
            // A preview reads only the start of the text.
            let data = match limit {
                Some(_) if data.len() > PREVIEW_BYTES => {
                    let cut = data[..PREVIEW_BYTES]
                        .iter()
                        .rposition(|&b| b == b'\n')
                        .map_or(PREVIEW_BYTES, |i| i + 1);
                    &data[..cut]
                }
                _ => data,
            };
            let d = delimited(data, ext)?;
            for r in 0..d.row_count() {
                if sink.full() {
                    break;
                }
                sink.row(&d.row_values(r)?)?;
            }
        }
    }
    Ok((sink.finish(), SheetStats::default()))
}

/// Delimited text as a document (layout detected as on open).
pub fn delimited(data: &[u8], ext: Option<&str>) -> Result<Document, Error> {
    let mut d = Document::from_bytes(data.to_vec(), OpenOptions::default(), ext.or(Some("txt")))?;
    d.mark_modified();
    Ok(d)
}

// ---- JSON ----

/// A JSON file that is one object (not an array of rows, not JSON Lines) is
/// held whole while it is read, so it has a lower cap.
pub const MAX_JSON_WHOLE: usize = 32 << 20;

/// Most text one row of a JSON array or JSON Lines may take. Each row is
/// first read as raw text and measured, then parsed: a parsed value takes
/// many times the room of its text.
pub const MAX_JSON_ROW: usize = 16 << 20;

/// Parse one row after checking the size of its text. The row's own cells keep a number exactly as it is
/// written (`1.50`, `12345678901234567890123`, `1E5`): a number token becomes the text of the token.
/// (Values nested inside a cell are read as before.)
fn json_row(raw: &serde_json::value::RawValue) -> Result<Value, Error> {
    if raw.get().len() > MAX_JSON_ROW {
        return Err(Error::Invalid(format!(
            "a row of the JSON is over {} MB and cannot be imported",
            MAX_JSON_ROW >> 20
        )));
    }
    exact_row(raw.get())
}

/// The members of a JSON object in file order, each value as its raw text.
struct Pairs(Vec<(String, Box<serde_json::value::RawValue>)>);

impl<'de> serde::Deserialize<'de> for Pairs {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Pairs;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Pairs, A::Error> {
                let mut v = Vec::new();
                while let Some(kv) = m.next_entry::<String, Box<serde_json::value::RawValue>>()? {
                    v.push(kv);
                }
                Ok(Pairs(v))
            }
        }
        d.deserialize_map(V)
    }
}

impl Pairs {
    fn iter(&self) -> impl Iterator<Item = &(String, Box<serde_json::value::RawValue>)> {
        self.0.iter()
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One cell's value from its text: a number as the text of its token, anything else as a value.
fn exact_cell(text: &str) -> Result<Value, Error> {
    let t = text.trim();
    if t.starts_with(|c: char| c == '-' || c.is_ascii_digit()) {
        return Ok(Value::String(t.to_string()));
    }
    // The common cases without a second parse: a plain string, true, false, null.
    if let Some(inner) = t.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        if !inner.contains(['\\', '"']) {
            return Ok(Value::String(inner.to_string()));
        }
    }
    match t {
        "null" => return Ok(Value::Null),
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        _ => {}
    }
    serde_json::from_str(t).map_err(bad_json)
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while matches!(b.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        i += 1;
    }
    i
}

/// The end (after the closing quote) of the string starting at `i`, and whether it has escapes.
fn scan_string(b: &[u8], i: usize) -> Option<(usize, bool)> {
    if b.get(i) != Some(&b'"') {
        return None;
    }
    let mut j = i + 1;
    let mut esc = false;
    loop {
        match b.get(j)? {
            b'\\' => {
                esc = true;
                j += 2;
            }
            b'"' => return Some((j + 1, esc)),
            _ => j += 1,
        }
    }
}

/// The text of a string token (quotes included) as a string.
fn string_of(t: &str, start: usize, end: usize, esc: bool) -> Option<String> {
    if esc {
        serde_json::from_str::<String>(&t[start..end]).ok()
    } else {
        Some(t[start + 1..end - 1].to_string())
    }
}

/// The end of the object or array starting at `i` (strings skipped properly).
fn scan_container(b: &[u8], i: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut j = i;
    loop {
        match b.get(j)? {
            b'{' | b'[' => {
                depth += 1;
                j += 1;
            }
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                j += 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            b'"' => j = scan_string(b, j)?.0,
            _ => j += 1,
        }
    }
}

/// One cell's value starting at `i` and where it ends: a number as its own text, a string unescaped, a
/// nested container read as before. `None` for anything unexpected (the caller falls back to serde).
fn fast_value(t: &str, i: usize) -> Option<(Value, usize)> {
    let b = t.as_bytes();
    match b.get(i)? {
        b'"' => {
            let (end, esc) = scan_string(b, i)?;
            Some((Value::String(string_of(t, i, end, esc)?), end))
        }
        c if *c == b'-' || c.is_ascii_digit() => {
            let mut j = i + 1;
            while matches!(
                b.get(j),
                Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            ) {
                j += 1;
            }
            Some((Value::String(t[i..j].to_string()), j))
        }
        b't' if t[i..].starts_with("true") => Some((Value::Bool(true), i + 4)),
        b'f' if t[i..].starts_with("false") => Some((Value::Bool(false), i + 5)),
        b'n' if t[i..].starts_with("null") => Some((Value::Null, i + 4)),
        b'{' | b'[' => {
            let end = scan_container(b, i)?;
            Some((serde_json::from_str::<Value>(&t[i..end]).ok()?, end))
        }
        _ => None,
    }
}

/// A whole row (an object or an array of cells) read by hand, much quicker than a general parse; `None`
/// when anything is not as expected.
fn fast_row(t: &str) -> Option<Value> {
    let b = t.as_bytes();
    let mut i = skip_ws(b, 0);
    let open = *b.get(i)?;
    if open != b'{' && open != b'[' {
        return None;
    }
    let close = if open == b'{' { b'}' } else { b']' };
    i = skip_ws(b, i + 1);
    let mut map = serde_json::Map::new();
    let mut list = Vec::new();
    if b.get(i) != Some(&close) {
        loop {
            if open == b'{' {
                let (ke, kesc) = scan_string(b, i)?;
                let key = string_of(t, i, ke, kesc)?;
                i = skip_ws(b, ke);
                if b.get(i) != Some(&b':') {
                    return None;
                }
                i = skip_ws(b, i + 1);
                let (v, end) = fast_value(t, i)?;
                map.insert(key, v);
                i = end;
            } else {
                let (v, end) = fast_value(t, i)?;
                list.push(v);
                i = end;
            }
            i = skip_ws(b, i);
            match b.get(i)? {
                b',' => i = skip_ws(b, i + 1),
                c if *c == close => break,
                _ => return None,
            }
        }
    }
    if skip_ws(b, i + 1) != b.len() {
        return None;
    }
    Some(if open == b'{' {
        Value::Object(map)
    } else {
        Value::Array(list)
    })
}

/// A row's text as a value whose own cells keep their numbers exactly.
fn exact_row(text: &str) -> Result<Value, Error> {
    if let Some(v) = fast_row(text) {
        return Ok(v);
    }
    let t = text.trim_start();
    if t.starts_with('{') {
        let members: Pairs = serde_json::from_str(text).map_err(bad_json)?;
        let mut out = serde_json::Map::new();
        for (k, v) in members.0 {
            out.insert(k, exact_cell(v.get())?);
        }
        return Ok(Value::Object(out));
    }
    if t.starts_with('[') {
        let items: Vec<&serde_json::value::RawValue> =
            serde_json::from_str(text).map_err(bad_json)?;
        return Ok(Value::Array(
            items
                .into_iter()
                .map(|v| exact_cell(v.get()))
                .collect::<Result<_, _>>()?,
        ));
    }
    exact_cell(text)
}

fn json_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

const STOP: &str = "natendo: stop";

/// Hands each element of a top-level array to a callback as it is parsed,
/// so only one row is in memory at a time.
struct Elements<'a> {
    f: &'a mut dyn FnMut(&Value) -> Result<bool, Error>,
    err: &'a mut Option<Error>,
}

impl<'de> serde::de::Visitor<'de> for Elements<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("an array")
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        use serde::de::Error as _;
        while let Some(raw) = seq.next_element::<&serde_json::value::RawValue>()? {
            match json_row(raw).and_then(|v| (self.f)(&v)) {
                Ok(true) => {}
                Ok(false) => return Err(A::Error::custom(STOP)),
                Err(e) => {
                    *self.err = Some(e);
                    return Err(A::Error::custom(STOP));
                }
            }
        }
        Ok(())
    }
}

fn bad_json(e: serde_json::Error) -> Error {
    Error::Invalid(format!("the JSON is not valid: {e}"))
}

/// How a JSON text holds its rows.
enum JsonRows {
    /// One array: each element is a row (streamed).
    Array,
    /// JSON Lines: each value is a row (streamed).
    Lines,
    /// One object (or value), small enough to hold: its rows.
    Whole(Vec<Value>),
}

impl JsonRows {
    fn detect(text: &str) -> Result<JsonRows, Error> {
        let t = text.trim_start();
        let first = t.split('\n').next().unwrap_or("");
        let more = t.len() > first.len() && !t[first.len()..].trim().is_empty();
        if more && serde_json::from_str::<serde::de::IgnoredAny>(first).is_ok() {
            return Ok(JsonRows::Lines);
        }
        if t.starts_with('[') {
            return Ok(JsonRows::Array);
        }
        if text.len() > MAX_JSON_WHOLE {
            return Err(Error::Invalid(format!(
                "a JSON object over {} MB cannot be imported; arrays of rows and JSON Lines can be up to {} MB",
                MAX_JSON_WHOLE >> 20,
                MAX_JSON_INPUT >> 20
            )));
        }
        // Read from the raw text (not parsed to numbers first) so the rows' cells keep their numbers exactly.
        let raw: &serde_json::value::RawValue = serde_json::from_str(text).map_err(bad_json)?;
        let top = raw.get().trim_start();
        let raw_rows = |s: &str| -> Result<Vec<Value>, Error> {
            let items: Vec<&serde_json::value::RawValue> =
                serde_json::from_str(s).map_err(bad_json)?;
            items.into_iter().map(|v| exact_row(v.get())).collect()
        };
        Ok(JsonRows::Whole(if top.starts_with('[') {
            raw_rows(top)?
        } else if top.starts_with('{') {
            let members: Pairs = serde_json::from_str(top).map_err(bad_json)?;
            // The first array of rows inside, else each property a row.
            let is_rows = |s: &str| -> bool {
                s.trim_start().starts_with('[')
                    && serde_json::from_str::<Vec<&serde_json::value::RawValue>>(s).is_ok_and(|a| {
                        a.iter()
                            .any(|x| x.get().trim_start().starts_with(['{', '[']))
                    })
            };
            let found = members
                .iter()
                .find(|(_, v)| is_rows(v.get()))
                .map(|(_, v)| v.get().to_string());
            if let Some(rows) = found {
                raw_rows(&rows)?
            } else if !members.is_empty()
                && members
                    .iter()
                    .all(|(_, v)| v.get().trim_start().starts_with('{'))
            {
                let mut out = Vec::new();
                for (k, v) in members.0 {
                    let mut m = serde_json::Map::new();
                    m.insert("key".into(), Value::String(k));
                    if let Value::Object(inner) = exact_row(v.get())? {
                        for (a, b) in inner {
                            m.entry(a).or_insert(b);
                        }
                    }
                    out.push(Value::Object(m));
                }
                out
            } else {
                vec![exact_row(top)?]
            }
        } else {
            vec![exact_cell(top)?]
        }))
    }

    /// Call `f` with each row until it returns `false`.
    fn each(
        &self,
        text: &str,
        f: &mut dyn FnMut(&Value) -> Result<bool, Error>,
    ) -> Result<(), Error> {
        use serde::Deserializer as _;
        match self {
            JsonRows::Whole(rows) => {
                for r in rows {
                    if !f(r)? {
                        break;
                    }
                }
                Ok(())
            }
            JsonRows::Lines => {
                let rows = serde_json::Deserializer::from_str(text)
                    .into_iter::<&serde_json::value::RawValue>();
                for raw in rows {
                    if !f(&json_row(raw.map_err(bad_json)?)?)? {
                        break;
                    }
                }
                Ok(())
            }
            JsonRows::Array => {
                let mut err = None;
                let mut de = serde_json::Deserializer::from_str(text);
                let r = (&mut de).deserialize_seq(Elements { f, err: &mut err });
                if let Some(e) = err {
                    return Err(e);
                }
                match r {
                    Ok(()) => de.end().map_err(bad_json),
                    Err(e) if e.to_string().starts_with(STOP) => Ok(()),
                    Err(e) => Err(bad_json(e)),
                }
            }
        }
    }
}

/// Rows of JSON: two passes over the text (names first, then rows), so
/// no more than one row is held at a time. A preview's first pass stops at
/// its row limit too.
fn json(text: &str, limit: Option<u64>, sink: &mut Sink) -> Result<(), Error> {
    let rows = JsonRows::detect(text)?;
    let mut cols: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut objects = false;
    let mut seen = 0u64;
    rows.each(text, &mut |r| {
        if let Value::Object(o) = r {
            objects = true;
            for k in o.keys() {
                if !index.contains_key(k) {
                    if cols.len() >= MAX_IMPORT_COLS {
                        return Err(Error::Invalid(
                            "the JSON has more than 16,384 different names".into(),
                        ));
                    }
                    index.insert(k.clone(), cols.len());
                    cols.push(k.clone());
                }
            }
        }
        seen += 1;
        Ok(limit.is_none_or(|l| seen < l))
    })?;
    if objects {
        if cols.is_empty() {
            cols.push("value".into());
        }
        sink.row(&cols)?;
        let mut line = vec![String::new(); cols.len()];
        rows.each(text, &mut |r| {
            line.iter_mut().for_each(String::clear);
            match r {
                Value::Object(o) => {
                    for (k, v) in o {
                        // A name past a preview's first rows is left out.
                        if let Some(&i) = index.get(k) {
                            line[i] = json_cell(v);
                        }
                    }
                }
                other => line[0] = json_cell(other),
            }
            sink.row(&line)?;
            Ok(!sink.full())
        })?;
    } else {
        rows.each(text, &mut |r| {
            match r {
                Value::Array(a) => {
                    if a.len() > MAX_IMPORT_COLS {
                        return Err(Error::Invalid("a JSON row has too many values".into()));
                    }
                    let line: Vec<String> = a.iter().map(json_cell).collect();
                    sink.row(&line)?;
                }
                other => sink.row(&[json_cell(other)])?,
            }
            Ok(!sink.full())
        })?;
    }
    Ok(())
}

// ---- XML ----

/// Paths of elements that look like records (have attributes or child
/// elements), most repeated first.
fn xml_records(text: &str) -> Result<Vec<String>, Error> {
    let mut r = xml_reader_str(text);
    let mut stack: Vec<String> = Vec::new();
    // Per open element: whether it had children or attributes.
    let mut rich: Vec<bool> = Vec::new();
    let mut counts: HashMap<String, (u64, usize)> = HashMap::new();
    let mut order = 0usize;
    loop {
        match r.read_event().map_err(xml_err)? {
            Event::Start(e) => {
                if let Some(last) = rich.last_mut() {
                    *last = true;
                }
                stack.push(local(&e));
                if stack.len() > MAX_DEPTH {
                    return Err(damaged("nested too deeply"));
                }
                rich.push(e.attributes().next().is_some());
            }
            Event::Empty(e) => {
                if let Some(last) = rich.last_mut() {
                    *last = true;
                }
                if e.attributes().next().is_some() {
                    stack.push(local(&e));
                    let p = stack.join("/");
                    stack.pop();
                    let c = counts.entry(p).or_insert((0, order));
                    c.0 += 1;
                    order += 1;
                }
            }
            Event::End(_) => {
                if let Some(was_rich) = rich.pop() {
                    if was_rich && stack.len() > 1 {
                        let c = counts.entry(stack.join("/")).or_insert((0, order));
                        c.0 += 1;
                        order += 1;
                    }
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
        if counts.len() > 100_000 {
            // Pathological variety; the most common paths are known by now.
            break;
        }
    }
    let mut v: Vec<(String, (u64, usize))> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.1 .1.cmp(&b.1 .1)));
    Ok(v.into_iter().take(20).map(|(p, _)| p).collect())
}

/// Field names of XML records, numbered in the order first seen.
#[derive(Default)]
struct XmlCols {
    cols: Vec<String>,
    index: HashMap<String, usize>,
}

impl XmlCols {
    fn col(&mut self, name: String) -> Result<usize, Error> {
        if let Some(&i) = self.index.get(&name) {
            return Ok(i);
        }
        if self.cols.len() >= MAX_IMPORT_COLS {
            return Err(Error::Invalid(
                "the XML records have too many different fields".into(),
            ));
        }
        self.index.insert(name.clone(), self.cols.len());
        self.cols.push(name);
        Ok(self.cols.len() - 1)
    }
}

/// One record's fields while it is read: values by column, with an index
/// so a record with many fields costs no more per field than a small one.
#[derive(Default)]
struct XmlRow {
    cells: Vec<(usize, String)>,
    at: HashMap<usize, usize>,
    bytes: usize,
}

impl XmlRow {
    fn put(&mut self, c: usize, v: String) -> Result<(), Error> {
        self.bytes += v.len();
        if self.bytes > crate::table::MAX_CELL * 4 {
            return Err(crate::reshape::too_big());
        }
        match self.at.get(&c) {
            Some(&i) => {
                if !v.is_empty() {
                    let x = &mut self.cells[i].1;
                    if !x.is_empty() {
                        x.push_str("; ");
                    }
                    x.push_str(&v);
                }
            }
            None => {
                self.at.insert(c, self.cells.len());
                self.cells.push((c, v));
            }
        }
        Ok(())
    }
}

/// Text inside an element: kept as written, except that whitespace alone
/// (indentation between elements) counts as empty.
fn xml_value(t: String) -> String {
    if t.trim().is_empty() {
        String::new()
    } else {
        t
    }
}

/// Read every record (element at path `record`), handing each to `on` as
/// it ends; `on` returns `false` to stop.
fn xml_walk(
    text: &str,
    record: &str,
    names: &mut XmlCols,
    on: &mut dyn FnMut(XmlRow) -> Result<bool, Error>,
) -> Result<(), Error> {
    let depth_of_record = record.split('/').count();
    let mut r = xml_reader_str(text);
    let mut stack: Vec<String> = Vec::new();
    let mut cur: Option<XmlRow> = None;
    // Text of each open element inside a record.
    let mut texts: Vec<String> = Vec::new();
    loop {
        let ev = r.read_event().map_err(xml_err)?;
        let empty = matches!(ev, Event::Empty(_));
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                stack.push(local(e));
                if stack.len() > MAX_DEPTH {
                    return Err(damaged("nested too deeply"));
                }
                if cur.is_none() && stack.len() == depth_of_record && stack.join("/") == record {
                    let mut row = XmlRow::default();
                    for (k, v) in attrs(e) {
                        let c = names.col(k)?;
                        row.put(c, v)?;
                    }
                    cur = Some(row);
                    texts.clear();
                    texts.push(String::new());
                } else if let Some(row) = cur.as_mut() {
                    let sub = stack[depth_of_record..].join("/");
                    for (k, v) in attrs(e) {
                        let c = names.col(format!("{sub}/@{k}"))?;
                        row.put(c, v)?;
                    }
                    texts.push(String::new());
                }
                if empty {
                    if let Some(row) = cur.as_mut() {
                        let t = texts.pop().unwrap_or_default();
                        if stack.len() > depth_of_record {
                            let sub = stack[depth_of_record..].join("/");
                            let c = names.col(sub)?;
                            row.put(c, t)?;
                        } else if let Some(done) = cur.take() {
                            stack.pop();
                            if !on(done)? {
                                return Ok(());
                            }
                            continue;
                        }
                    }
                    stack.pop();
                }
            }
            Event::End(_) => {
                if let Some(row) = cur.as_mut() {
                    let t = xml_value(texts.pop().unwrap_or_default());
                    if stack.len() > depth_of_record {
                        let sub = stack[depth_of_record..].join("/");
                        if !t.is_empty() || !names.index.contains_key(&sub) {
                            let c = names.col(sub)?;
                            row.put(c, t)?;
                        }
                    } else {
                        if !t.is_empty() {
                            let name = stack.last().cloned().unwrap_or_default();
                            let c = names.col(name)?;
                            row.put(c, t)?;
                        }
                        if let Some(done) = cur.take() {
                            stack.pop();
                            if !on(done)? {
                                return Ok(());
                            }
                            continue;
                        }
                    }
                }
                stack.pop();
            }
            Event::Text(ref t) => {
                if let Some(s) = texts.last_mut() {
                    push_text(s, &text_of(t))?;
                }
            }
            Event::CData(ref t) => {
                if let Some(s) = texts.last_mut() {
                    push_text(s, t)?;
                }
            }
            Event::GeneralRef(ref g) => {
                if let Some(s) = texts.last_mut() {
                    push_ref(g, s);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(())
}

/// XML records as rows: a first pass finds the fields that hold values
/// (a preview's stops at its row limit), a second writes each record to
/// the table as it ends.
fn xml(text: &str, record: &str, limit: Option<u64>, sink: &mut Sink) -> Result<(), Error> {
    let record = if record.is_empty() {
        xml_records(text)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Invalid("no repeated records were found in the XML".into()))?
    } else {
        record.to_string()
    };
    let mut names = XmlCols::default();
    let mut used: Vec<bool> = Vec::new();
    let mut seen = 0u64;
    xml_walk(text, &record, &mut names, &mut |row| {
        for (c, v) in &row.cells {
            if used.len() <= *c {
                used.resize(c + 1, false);
            }
            used[*c] |= !v.is_empty();
        }
        seen += 1;
        Ok(limit.is_none_or(|l| seen < l))
    })?;
    let keep: Vec<usize> = (0..names.cols.len())
        .filter(|&c| used.get(c) == Some(&true))
        .collect();
    let mut at = vec![usize::MAX; names.cols.len()];
    for (i, &c) in keep.iter().enumerate() {
        at[c] = i;
    }
    let header: Vec<&str> = keep.iter().map(|&c| names.cols[c].as_str()).collect();
    sink.row(&header)?;
    let mut line = vec![String::new(); keep.len()];
    // Same text, same order: the second pass numbers fields the same way.
    let mut again = XmlCols::default();
    xml_walk(text, &record, &mut again, &mut |row| {
        line.iter_mut().for_each(String::clear);
        for (c, v) in row.cells {
            if let Some(&i) = at.get(c) {
                if i != usize::MAX {
                    line[i] = v;
                }
            }
        }
        sink.row(&line)?;
        Ok(!sink.full())
    })?;
    Ok(())
}

// ---- HTML ----

/// Decode HTML character references (named ones a table usually holds).
pub(crate) fn html_unescape(s: &str, out: &mut String) {
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.as_bytes()[..rest.len().min(12)]
            .iter()
            .position(|&b| b == b';');
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "euro" => Some('€'),
            "pound" => Some('£'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16)
                .ok()
                .and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) if c != '\0' => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
}

/// Walk every table (nested tables count as their own). Rows of table
/// `want` go to `on_row` one at a time as they finish (it returns `false`
/// to stop early); nothing else is kept. Returns each table's row count.
pub(crate) fn html_tables(
    text: &str,
    want: Option<usize>,
    on_row: &mut dyn FnMut(Vec<String>) -> Result<bool, Error>,
) -> Result<Vec<usize>, Error> {
    // Tags are matched without regard to case, without a lowercase copy
    // of the whole text (only each tag's start is lowercased).
    let b = text.as_bytes();
    let mut counts: Vec<usize> = Vec::new();
    // Open tables: index, current row, current cell text, and the blank
    // cells a column span still owes (written only if another cell follows).
    struct Open {
        t: usize,
        row: Option<Vec<String>>,
        cell: Option<String>,
        span: usize,
        owed: usize,
    }
    let mut open: Vec<Open> = Vec::new();
    // Finished rows of the wanted table, handed on after each tag.
    let mut ready: Vec<Vec<String>> = Vec::new();
    let mut i = 0;
    let mut text_from = 0;
    // A cell's text is measured before it is copied, so one huge cell
    // is refused rather than copied several times over.
    let flush = |open: &mut Vec<Open>, from: usize, to: usize| -> Result<(), Error> {
        if let Some(o) = open.last_mut() {
            if let Some(c) = o.cell.as_mut() {
                if c.len() + (to - from) > crate::table::MAX_CELL {
                    return Err(Error::Invalid(format!(
                        "a cell of the HTML holds over {} MB of text and cannot be imported",
                        crate::table::MAX_CELL >> 20
                    )));
                }
                let part = &text[from..to];
                if part.contains(BREAK) {
                    html_unescape(&part.replace(BREAK, ""), c);
                } else {
                    html_unescape(part, c);
                }
            }
        }
        Ok(())
    };
    let finish_cell = |o: &mut Open| {
        if let Some(c) = o.cell.take() {
            let row = o.row.get_or_insert_with(Vec::new);
            for _ in 0..o.owed.min(MAX_IMPORT_COLS.saturating_sub(row.len())) {
                row.push(String::new());
            }
            if row.len() < MAX_IMPORT_COLS {
                row.push(cell_text(&c));
            }
            o.owed = o.span - 1;
        }
    };
    let finish_row = |o: &mut Open, counts: &mut Vec<usize>, ready: &mut Vec<Vec<String>>| {
        finish_cell(o);
        o.owed = 0;
        if let Some(r) = o.row.take() {
            counts[o.t] += 1;
            if want == Some(o.t) && !r.is_empty() {
                ready.push(r);
            }
        }
    };
    let mut stop = false;
    while i < b.len() && !stop {
        if b[i] != b'<' {
            i = memchr::memchr(b'<', &b[i..]).map_or(b.len(), |e| i + e);
            continue;
        }
        flush(&mut open, text_from, i)?;
        // Comments, scripts and styles are skipped whole.
        if b[i..].starts_with(b"<!--") {
            i = text[i..].find("-->").map_or(b.len(), |e| i + e + 3);
            text_from = i;
            continue;
        }
        let end = memchr::memchr(b'>', &b[i..]).map_or(b.len(), |e| i + e + 1);
        let mut tag_end = end.saturating_sub(1).max(i + 1).min(i + 1 + MAX_TAG);
        while !text.is_char_boundary(tag_end) {
            tag_end -= 1;
        }
        let tag_l = text[i + 1..tag_end].to_ascii_lowercase();
        let tag = tag_l.as_str();
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        i = end;
        text_from = end;
        if !closing && (name == "script" || name == "style") {
            let close = format!("</{name}");
            i = find_ci(b, i, close.as_bytes());
            text_from = i;
            continue;
        }
        match (closing, name.as_str()) {
            (false, "table") => {
                counts.push(0);
                open.push(Open {
                    t: counts.len() - 1,
                    row: None,
                    cell: None,
                    span: 1,
                    owed: 0,
                });
                if open.len() > MAX_DEPTH {
                    break;
                }
            }
            (true, "table") => {
                if let Some(mut o) = open.pop() {
                    finish_row(&mut o, &mut counts, &mut ready);
                }
            }
            (false, "tr") => {
                if let Some(o) = open.last_mut() {
                    finish_row(o, &mut counts, &mut ready);
                    o.row = Some(Vec::new());
                }
            }
            (true, "tr") => {
                if let Some(o) = open.last_mut() {
                    finish_row(o, &mut counts, &mut ready);
                }
            }
            (false, "td" | "th") => {
                if let Some(o) = open.last_mut() {
                    finish_cell(o);
                    o.row.get_or_insert_with(Vec::new);
                    o.cell = Some(String::new());
                    o.span = tag
                        .split("colspan=")
                        .nth(1)
                        .map(|s| s.trim_start_matches(['"', '\'']))
                        .and_then(|s| {
                            s.chars()
                                .take_while(char::is_ascii_digit)
                                .collect::<String>()
                                .parse()
                                .ok()
                        })
                        .unwrap_or(1usize)
                        .clamp(1, 1000);
                }
            }
            (true, "td" | "th") => {
                if let Some(o) = open.last_mut() {
                    finish_cell(o);
                }
            }
            (false, "br") | (true, "p" | "div" | "li") => {
                if let Some(c) = open.last_mut().and_then(|o| o.cell.as_mut()) {
                    c.push(BREAK);
                }
            }
            _ => {}
        }
        for r in ready.drain(..) {
            if !on_row(r)? {
                stop = true;
                break;
            }
        }
    }
    if !stop {
        flush(&mut open, text_from, b.len())?;
        while let Some(mut o) = open.pop() {
            finish_row(&mut o, &mut counts, &mut ready);
        }
        for r in ready.drain(..) {
            if !on_row(r)? {
                break;
            }
        }
    }
    Ok(counts)
}

/// Most of a tag read for its name and column span.
const MAX_TAG: usize = 4096;

/// The first `pat` (lowercase ASCII starting with `<`) at or after `from`,
/// in any case; the end of `b` if there is none.
pub(crate) fn find_ci(b: &[u8], mut from: usize, pat: &[u8]) -> usize {
    while let Some(e) = memchr::memchr(b'<', &b[from..]) {
        let k = from + e;
        if b.len() - k >= pat.len() && b[k..k + pat.len()].eq_ignore_ascii_case(pat) {
            return k;
        }
        from = k + 1;
    }
    b.len()
}

/// Marks a line break inside a cell while spaces are collapsed.
const BREAK: char = '\u{1}';

/// A cell's text: spaces collapsed as a browser shows them, `<br>` and
/// paragraph ends as line breaks.
fn cell_text(c: &str) -> String {
    let v = c.split_whitespace().collect::<Vec<_>>().join(" ");
    let v = v.replace(&format!(" {BREAK}"), &BREAK.to_string());
    let v = v.replace(&format!("{BREAK} "), &BREAK.to_string());
    v.trim_matches(BREAK).replace(BREAK, "\n")
}

// ---- Markdown ----

fn is_md_rule(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('-') || (!t.contains('|') && !t.starts_with('-')) {
        return false;
    }
    let cells = split_md(t, MAX_IMPORT_COLS);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim();
            let c = c.strip_prefix(':').unwrap_or(c);
            let c = c.strip_suffix(':').unwrap_or(c);
            !c.is_empty() && c.bytes().all(|b| b == b'-')
        })
}

/// A table line's cells, at most `max` (the rest of the line is left out).
/// `<br>` in a cell is a line break (as Markdown tables write them).
fn split_md(line: &str, max: usize) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") {
        &t[..t.len() - 1]
    } else {
        t
    };
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    let finish = |cur: &mut String| {
        let v = std::mem::take(cur);
        let v = v.trim();
        if v.contains("<br") {
            v.replace("<br>", "\n")
                .replace("<br/>", "\n")
                .replace("<br />", "\n")
        } else {
            v.to_string()
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => {
                cells.push(finish(&mut cur));
                if cells.len() >= max {
                    return cells;
                }
            }
            c => cur.push(c),
        }
    }
    cells.push(finish(&mut cur));
    cells
}

/// Walk every pipe table; rows (header first) of table `want` go to `on`
/// one at a time (it returns `false` to stop). Returns each table's rows.
pub(crate) fn md_tables(
    text: &str,
    want: Option<usize>,
    on: &mut dyn FnMut(Vec<String>) -> Result<bool, Error>,
) -> Result<Vec<usize>, Error> {
    let mut counts = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(next) = lines.peek() else { break };
        if !(line.contains('|') && is_md_rule(next)) {
            continue;
        }
        lines.next();
        let t = counts.len();
        counts.push(1);
        let header = split_md(line, MAX_IMPORT_COLS);
        let width = header.len().max(1);
        let mut give = want == Some(t);
        if give && !on(header)? {
            give = false;
        }
        while let Some(l) = lines.peek() {
            if !l.contains('|') || l.trim().is_empty() {
                break;
            }
            let l = lines.next().expect("peeked");
            counts[t] += 1;
            if give && !on(split_md(l, width))? {
                give = false;
            }
        }
        if want == Some(t) {
            break;
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(t: Table) -> Vec<Vec<String>> {
        let d = t.into_document().unwrap();
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    fn imp(data: &str, ext: &str, opts: ImportOptions) -> Vec<Vec<String>> {
        rows(import(data.as_bytes(), Some(ext), &opts, None, &Progress::new()).unwrap())
    }

    /// A1a: numbers keep the text they are written with, in every JSON shape.
    #[test]
    fn json_numbers_are_exact() {
        let o = ImportOptions::default();
        let want = vec![
            vec!["id", "price", "e"],
            vec!["12345678901234567890123", "1.50", "1E5"],
            vec!["-0.10", "2.0", "text"],
        ];
        let body = r#"[{"id":12345678901234567890123,"price":1.50,"e":1E5},{"id":-0.10,"price":2.0,"e":"text"}]"#;
        assert_eq!(imp(body, "json", o.clone()), want);
        // Streamed lines and an array inside an object.
        assert_eq!(
            imp(
                "{\"id\":12345678901234567890123,\"price\":1.50,\"e\":1E5}
{\"id\":-0.10,\"price\":2.0,\"e\":\"text\"}
",
                "jsonl",
                o.clone()
            ),
            want
        );
        assert_eq!(
            imp(
                &format!("{{\"meta\":1,\"data\":{body}}}"),
                "json",
                o.clone()
            ),
            want
        );
        assert_eq!(
            imp(r#"{"x":{"n":1.10},"y":{"n":2}}"#, "json", o.clone()),
            vec![vec!["key", "n"], vec!["x", "1.10"], vec!["y", "2"]]
        );
        assert_eq!(
            imp(r#"[[1.0,"a"],[12345678901234567890,true]]"#, "json", o),
            vec![vec!["1.0", "a"], vec!["12345678901234567890", "true"]]
        );
    }

    /// The hand-written row reader gives what the general one gives, on odd but valid rows.
    #[test]
    fn fast_rows_match_the_general_reader() {
        let rows = [
            r#"{"a":1,"b":"x"}"#,
            "  { \"a\" : 1.50 , \"b\" :\n[1, 2.0, {\"z\":\"}\"}] ,\"c\":null,\"d\":true }  ",
            r#"{"k\"ey":"va\\lue \u00e9 \ud83d\ude00","é":"ü","n":-0.10,"e":1E5,"m":{"q":[1.0,"a]b"]}}"#,
            r#"[1,"two",3.50,false,null,[1,[2]],{"a":"b"}]"#,
            "{}",
            "[ ]",
            r#"{"a":"","a":2}"#,
        ];
        for r in rows {
            let fast = fast_row(r).unwrap_or_else(|| panic!("fast path refused {r}"));
            // The general reader, with the same cell rule.
            let slow = if r.trim_start().starts_with('{') {
                let m: Pairs = serde_json::from_str(r).unwrap();
                let mut out = serde_json::Map::new();
                for (k, v) in m.0 {
                    out.insert(k, exact_cell(v.get()).unwrap());
                }
                Value::Object(out)
            } else {
                let items: Vec<&serde_json::value::RawValue> = serde_json::from_str(r).unwrap();
                Value::Array(
                    items
                        .into_iter()
                        .map(|v| exact_cell(v.get()).unwrap())
                        .collect(),
                )
            };
            assert_eq!(fast, slow, "{r}");
        }
        // Not as expected: left to the general reader (which reports the error).
        for r in ["{\"a\":}", "[1,]", "{\"a\" 1}", "[1] x", "{\"a\":\"x"] {
            assert_eq!(fast_row(r), None, "{r}");
        }
    }

    #[test]
    fn json_shapes() {
        let o = ImportOptions::default();
        assert_eq!(
            imp(
                r#"[{"a":1,"b":"x"},{"b":null,"c":[1,2],"a":2.5}]"#,
                "json",
                o.clone()
            ),
            vec![
                vec!["a", "b", "c"],
                vec!["1", "x"],
                vec!["2.5", "", "[1,2]"],
            ]
        );
        assert_eq!(
            imp(
                r#"{"meta":1,"data":[[1,"a"],[true,false]]}"#,
                "json",
                o.clone()
            ),
            vec![vec!["1", "a"], vec!["true", "false"]]
        );
        assert_eq!(
            imp("{\"a\":1}\n{\"a\":2}\n", "jsonl", o.clone()),
            vec![vec!["a"], vec!["1"], vec!["2"]]
        );
        assert_eq!(
            imp(r#"{"x":{"n":1},"y":{"n":2}}"#, "json", o.clone()),
            vec![vec!["key", "n"], vec!["x", "1"], vec!["y", "2"]]
        );
        // Too deep: the parser's own limit, a clean error.
        let deep = "[".repeat(10_000);
        assert!(import(deep.as_bytes(), Some("json"), &o, None, &Progress::new()).is_err());
    }

    #[test]
    fn xml_records_and_fields() {
        let x = r#"<?xml version="1.0"?>
<!DOCTYPE r [<!ENTITY e "boom">]>
<root><meta><title>t</title></meta>
<items>
 <item id="1"><name>Ann &amp; Co</name><tag>a</tag><tag>b</tag><addr><city>X</city></addr></item>
 <item id="2"><name><![CDATA[<Bob>]]></name><note>&e;</note></item>
 <item id="3"/>
</items></root>"#;
        let info = inspect(x.as_bytes(), Some("xml"), None).unwrap();
        assert_eq!(info.records[0], "root/items/item");
        assert_eq!(
            imp(x, "xml", ImportOptions::default()),
            vec![
                vec!["id", "name", "tag", "addr/city", "note"],
                vec!["1", "Ann & Co", "a; b", "X"],
                vec!["2", "<Bob>", "", "", "&e;"],
                vec!["3"],
            ]
        );
    }

    #[test]
    fn html_and_markdown_tables() {
        let h = r#"<html><script>var t="<table>";</script><table><tr><th>A</th><th colspan=2>B</th></tr>
<tr><td> x &amp; <b>y</b> </td><td>1<br>2</td><td>&#x41;</td></tr><!-- <table> --></table>
<table><tr><td>second</td></tr></table>"#;
        let info = inspect(h.as_bytes(), Some("html"), None).unwrap();
        assert_eq!(info.parts.len(), 2);
        assert_eq!(
            imp(h, "html", ImportOptions::default()),
            vec![vec!["A", "B"], vec!["x & y", "1\n2", "A"]]
        );
        // Tags in any case, scripts closed in any case.
        let u = "<HTML><SCRIPT>x='<table>'</Script><TABLE><TR><TD COLSPAN=\"2\">a</TD><Td>b</tD></TR></TABLE>";
        assert_eq!(
            imp(u, "html", ImportOptions::default()),
            vec![vec!["a", "", "b"]]
        );
        assert_eq!(
            imp(
                h,
                "html",
                ImportOptions {
                    part: 1,
                    ..Default::default()
                }
            ),
            vec![vec!["second"]]
        );
        let m = "Intro\n\n| Name | Note |\n|:--|--:|\n| a \\| b | 1 |\n| c |\n\nafter";
        assert_eq!(guess(None, m.as_bytes()), ImportFormat::Markdown);
        assert_eq!(
            imp(m, "md", ImportOptions::default()),
            vec![vec!["Name", "Note"], vec!["a | b", "1"], vec!["c"]]
        );
    }

    #[test]
    fn guess_cuts_at_a_character() {
        // Review: a multi-byte character across byte 4096 must not panic.
        for pad in 0..12 {
            let text = format!("{}{}", "a".repeat(4090 + pad), "漢字テキスト".repeat(10));
            let _ = guess(None, text.as_bytes());
            let _ = guess(Some("txt"), text.as_bytes());
        }
    }

    #[test]
    fn guesses_and_delimited() {
        assert_eq!(guess(Some("XLSX"), b""), ImportFormat::Spreadsheet);
        assert_eq!(guess(None, b"PK\x03\x04..."), ImportFormat::Spreadsheet);
        assert_eq!(guess(None, b"  [1]"), ImportFormat::Json);
        assert_eq!(guess(None, b"<rows><r/></rows>"), ImportFormat::Xml);
        assert_eq!(guess(None, b"<p>x</p><TABLE>"), ImportFormat::Html);
        assert_eq!(guess(None, b"a\tb\n1\t2\n"), ImportFormat::Delimited);
        assert_eq!(
            imp("a\tb\n1\t2\n", "txt", ImportOptions::default()),
            vec![vec!["a", "b"], vec!["1", "2"]]
        );
        // Previews stop early.
        let t = import(
            b"[[1],[2],[3]]",
            None,
            &ImportOptions::default(),
            Some(2),
            &Progress::new(),
        )
        .unwrap();
        assert_eq!(t.rows, 2);
    }

    /// Review findings 1 and 2: inputs that would multiply in memory stay
    /// bounded and quick.
    #[test]
    fn big_inputs_stay_bounded() {
        let quick = |what: &str, t: std::time::Instant| {
            assert!(t.elapsed().as_secs() < 15, "{what} took {:?}", t.elapsed());
        };
        // HTML: 20,000 rows of one wide-spanning cell keep no padding.
        let rows: String = (0..20_000)
            .map(|i| format!("<tr><td colspan=1000>{i}</td></tr>"))
            .collect();
        let h = format!("<table>{rows}</table>");
        let t = std::time::Instant::now();
        let out = import(
            h.as_bytes(),
            Some("html"),
            &ImportOptions::default(),
            None,
            &Progress::new(),
        )
        .unwrap();
        assert_eq!((out.rows, out.cols), (20_000, 1));
        let p = import(
            h.as_bytes(),
            Some("html"),
            &ImportOptions::default(),
            Some(100),
            &Progress::new(),
        )
        .unwrap();
        assert_eq!(p.rows, 100);
        quick("html", t);
        // Padding is kept only between cells.
        let h = "<table><tr><td colspan=3>a</td><td>b</td></tr></table>";
        assert_eq!(
            imp(h, "html", ImportOptions::default()),
            vec![vec!["a", "", "", "b"]]
        );
        // Markdown: a 20 MB line of pipes is cut at 16,384 cells.
        let m = format!("{}\n|---|\n|x|\n", "|".repeat(20 << 20));
        let t = std::time::Instant::now();
        let out = import(
            m.as_bytes(),
            Some("md"),
            &ImportOptions::default(),
            None,
            &Progress::new(),
        )
        .unwrap();
        assert!(out.cols as usize <= MAX_IMPORT_COLS);
        quick("markdown", t);
        assert_eq!(
            imp(
                "| a | b |\n|---|---|\n| one<br>two | x |\n",
                "md",
                ImportOptions::default()
            )[1],
            vec!["one\ntwo", "x"]
        );
        // JSON: an array is read row by row; a preview stops early.
        let arr = format!("[{}{{\"a\":1}}]", "{\"a\":1},".repeat(2_000_000));
        let t = std::time::Instant::now();
        let p = import(
            arr.as_bytes(),
            Some("json"),
            &ImportOptions::default(),
            Some(100),
            &Progress::new(),
        )
        .unwrap();
        assert_eq!(p.rows, 100);
        quick("json preview", t);
        // A single object over the whole-object cap is refused up front.
        let obj = format!("{{\"data\": [{}1]}}", "1,".repeat(MAX_JSON_WHOLE / 2 + 10));
        let e = import(
            obj.as_bytes(),
            Some("json"),
            &ImportOptions::default(),
            None,
            &Progress::new(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("cannot be imported"), "{e}");
        // XML: self-closing records go straight to the table; a preview stops.
        let x = format!("<rows>{}</rows>", "<r a=\"1\" b=\"2\"/>".repeat(500_000));
        let t = std::time::Instant::now();
        let p = import(
            x.as_bytes(),
            Some("xml"),
            &ImportOptions::default(),
            Some(100),
            &Progress::new(),
        )
        .unwrap();
        assert_eq!(p.rows, 100);
        quick("xml preview", t);
        // Records with many fields cost no more per field.
        let fields: String = (0..5_000).map(|i| format!("<f{i}>{i}</f{i}>")).collect();
        let x = format!("<rows>{}</rows>", format!("<r>{fields}</r>").repeat(40));
        let t = std::time::Instant::now();
        let out = import(
            x.as_bytes(),
            Some("xml"),
            &ImportOptions::default(),
            None,
            &Progress::new(),
        )
        .unwrap();
        assert_eq!((out.rows, out.cols), (41, 5_000));
        quick("xml wide", t);
        // XML values keep their own spaces; indentation alone is empty.
        assert_eq!(
            imp(
                "<rs><r><a>  padded </a><b>\n   </b></r><r><a>x</a><b>y</b></r></rs>",
                "xml",
                ImportOptions::default()
            )[1],
            vec!["  padded "]
        );
        // A delimited preview reads only the start of the text.
        let big = "a,b\n".repeat(3_000_000);
        let t = std::time::Instant::now();
        let p = import(
            big.as_bytes(),
            None,
            &ImportOptions {
                format: Some(ImportFormat::Delimited),
                ..Default::default()
            },
            Some(100),
            &Progress::new(),
        )
        .unwrap();
        assert_eq!(p.rows, 100);
        quick("delimited preview", t);
    }

    #[test]
    fn one_huge_row_or_cell_is_refused() {
        let opts = |format| ImportOptions {
            format,
            ..Default::default()
        };
        let refused = |data: &str, ext: &str, format, word: &str| {
            for limit in [None, Some(10)] {
                let t = std::time::Instant::now();
                let e = import(
                    data.as_bytes(),
                    Some(ext),
                    &opts(format),
                    limit,
                    &Progress::new(),
                )
                .unwrap_err();
                assert!(e.to_string().contains(word), "{e}");
                assert!(t.elapsed().as_secs() < 10);
            }
        };
        // JSON: one row holding a huge array, as an object or an array,
        // in an array of rows or as JSON Lines.
        let zeros = "0,".repeat(MAX_JSON_ROW / 2 + 10);
        refused(
            &format!("[{{\"a\":[{zeros}0]}}]"),
            "json",
            Some(ImportFormat::Json),
            "row of the JSON",
        );
        refused(
            &format!("[[{zeros}0]]"),
            "json",
            Some(ImportFormat::Json),
            "row of the JSON",
        );
        refused(
            &format!(
                "{{\"a\":1}}
[{zeros}0]
"
            ),
            "jsonl",
            Some(ImportFormat::Json),
            "row of the JSON",
        );
        // Rows under the cap still import.
        let ok = format!("[[1,2],[{}3]]", "1,".repeat(1000));
        assert_eq!(
            imp(&ok, "json", opts(Some(ImportFormat::Json)))[1].len(),
            1001
        );
        // HTML: one cell over 32 MB, refused before it is copied.
        let html = format!(
            "<table><tr><td>a</td></tr><tr><td>{}</td></tr></table>",
            "x ".repeat(crate::table::MAX_CELL / 2 + 10)
        );
        refused(&html, "html", Some(ImportFormat::Html), "cell of the HTML");
    }

    #[test]
    fn hostile_text_never_panics() {
        let samples = [
            "<table><tr><td colspan=999999999>x",
            "<table><td>&#xFFFFFFFF;&#0;&;&amp</td>",
            "<table><td>&abcdefghij\u{bcd};&\u{1F600};</td>",
            "<<<>>>&&&",
            "| a |\n|---|\n|",
            "<a><b x='1'><c/></b></a>",
            "<a>\u{0}</a>",
            "[{\"a\":",
        ];
        for s in samples {
            for f in [
                ImportFormat::Html,
                ImportFormat::Markdown,
                ImportFormat::Xml,
                ImportFormat::Json,
            ] {
                let o = ImportOptions {
                    format: Some(f),
                    ..Default::default()
                };
                let _ = inspect(s.as_bytes(), None, Some(f));
                let _ = import(s.as_bytes(), None, &o, None, &Progress::new());
            }
        }
    }
}
