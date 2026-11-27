//! Exporting a document to other formats: JSON, XML, HTML table, Markdown
//! table, a template of the user's own, and Excel (`.xlsx`).
//!
//! Exports write the shown rows (a filter's hidden rows are left out) in
//! the document's column order, streamed row by row. Files are written to a
//! temp file next to the target and moved into place only when complete.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::Cols;

use crate::Error;

/// Excel's limits.
pub const XLSX_MAX_ROWS: u64 = 1_048_576;
pub const XLSX_MAX_COLS: u32 = 16_384;
pub const XLSX_MAX_CELL_CHARS: usize = 32_767;
/// A sheet part this large starts a new sheet (parts stay under 4 GiB).
pub(crate) const SHEET_PART_LIMIT: u64 = 3_500 << 20;
/// Longest template.
pub const MAX_TEMPLATE_CHARS: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[default]
    Json,
    /// JSON Lines (`.jsonl`): one JSON record per line.
    JsonLines,
    Xml,
    Html,
    Markdown,
    Template,
    Xlsx,
    Pdf,
    /// Columns padded with spaces to line up.
    FixedWidth,
    /// SQL: CREATE TABLE and INSERT statements.
    Sql,
}

/// How SQL names are quoted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlDialect {
    /// `"name"`: SQLite, PostgreSQL, Oracle, standard SQL.
    #[default]
    Standard,
    /// `` `name` ``: MySQL and MariaDB.
    Mysql,
    /// `[name]`: SQL Server and Access.
    SqlServer,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Json => "json",
            ExportFormat::JsonLines => "jsonl",
            ExportFormat::Xml => "xml",
            ExportFormat::Html => "html",
            ExportFormat::Markdown => "md",
            ExportFormat::Template => "txt",
            ExportFormat::Xlsx => "xlsx",
            ExportFormat::Pdf => "pdf",
            ExportFormat::FixedWidth => "txt",
            ExportFormat::Sql => "sql",
        }
    }
}

/// Export choices (each format reads the ones it needs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// The first shown row holds the column names.
    pub header: bool,
    /// JSON: rows as arrays instead of objects.
    pub arrays: bool,
    /// JSON/XML: indented.
    pub pretty: bool,
    /// XML element names.
    pub root: String,
    pub record: String,
    /// Template: text before, per row (`{Name}`, `{#3}`, `{{`, `}}`), after.
    pub before: String,
    pub row: String,
    pub after: String,
    /// Excel: sheet name; numbers stored as numbers.
    pub sheet: String,
    pub numbers: bool,
    /// Excel: made for Excel users (bold frozen title row with filter
    /// buttons, column widths, dates, currency and percents as Excel values
    /// with their formats, IDs and ZIP codes as text).
    pub look: bool,
    /// Excel: column widths in screen pixels (0 or missing: fit the values).
    pub widths: Vec<u32>,
    /// Excel: rows with a height of their own, as (shown row, pixels) with
    /// rows increasing (what the grid shows: none while a filter is on).
    #[serde(default)]
    pub row_heights: Vec<(u64, u32)>,
    /// Excel: data rows per part (0: as many as a sheet holds). A longer
    /// table goes on in parts, each with the title row.
    pub part_rows: u64,
    /// Excel: parts as separate files ("name (2 of 3).xlsx") instead of
    /// sheets of one workbook.
    pub split_files: bool,
    /// How numbers and dates are read.
    pub read: crate::value::ReadOptions,
    /// PDF: page setup.
    pub page: crate::pdf::PageSetup,
    /// SQL: the table's name, how names are quoted, rows per INSERT, and
    /// whether to start with CREATE TABLE.
    pub table: String,
    pub dialect: SqlDialect,
    pub batch: u32,
    pub create: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            format: ExportFormat::Json,
            header: true,
            arrays: false,
            pretty: true,
            root: "rows".into(),
            record: "row".into(),
            before: String::new(),
            row: String::new(),
            after: String::new(),
            sheet: "Sheet1".into(),
            numbers: true,
            look: true,
            widths: Vec::new(),
            row_heights: Vec::new(),
            part_rows: 0,
            split_files: false,
            read: Default::default(),
            page: Default::default(),
            table: "data".into(),
            dialect: SqlDialect::Standard,
            batch: 500,
            create: true,
        }
    }
}

/// What an export wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    pub rows: u64,
    pub sheets: u32,
    pub bytes: u64,
    /// Files written, when parts went to files of their own.
    #[serde(default)]
    pub files: Vec<String>,
}

enum Piece {
    Text(String),
    Col(usize),
}

/// Parse a template's row text against the column names.
fn parse_template(t: &str, names: &[String]) -> Result<Vec<Piece>, Error> {
    if t.chars().count() > MAX_TEMPLATE_CHARS {
        return Err(Error::Invalid("the template is too long".into()));
    }
    let by_name: HashMap<&str, usize> = names
        .iter()
        .enumerate()
        .rev()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut it = t.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                text.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                text.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for d in it.by_ref() {
                    if d == '}' {
                        closed = true;
                        break;
                    }
                    name.push(d);
                }
                if !closed {
                    return Err(Error::Invalid(
                        "the template has a { without its closing }".into(),
                    ));
                }
                let col = match name.strip_prefix('#').map(|n| n.trim().parse::<usize>()) {
                    Some(Ok(n)) if n >= 1 && n <= names.len() => n - 1,
                    _ => *by_name.get(name.as_str()).ok_or_else(|| {
                        Error::Invalid(format!(
                            "the template names a column that is not there: {{{name}}}"
                        ))
                    })?,
                };
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Col(col));
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    Ok(out)
}

pub(crate) fn xml_escape(v: &str, out: &mut String) {
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' => out.push(c),
            // A bare CR would be read back as a line feed.
            '\r' => out.push_str("&#13;"),
            // Not allowed in XML 1.0 at all.
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
}

/// Excel text: XML escaping, and control characters as `_xHHHH_` (with a
/// literal `_xHHHH_` protected as `_x005F_xHHHH_`), as spreadsheets store them.
pub(crate) fn xlsx_escape(v: &str, out: &mut String) {
    let b = v.as_bytes();
    for (i, c) in v.char_indices() {
        match c {
            '_' if b.len() >= i + 7
                && b[i + 1] == b'x'
                && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit)
                && b[i + 6] == b'_' =>
            {
                out.push_str("_x005F_")
            }
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // CR goes as _x000D_ (XML readers turn a bare CR into LF).
            '\t' | '\n' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {
                out.push_str(&format!("_x{:04X}_", c as u32))
            }
            c => out.push(c),
        }
    }
}

/// A valid XML element name made from a column name.
fn xml_name(n: &str) -> String {
    let mut s: String = n
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty()
        || !s.starts_with(|c: char| c.is_alphabetic() || c == '_')
        || s.to_ascii_lowercase().starts_with("xml")
    {
        s.insert(0, '_');
    }
    s
}

/// Text that Excel reads back as exactly the same number: no leading zeros,
/// no trailing zeros after a point, at most 15 significant digits.
pub(crate) fn excel_number(v: &str) -> bool {
    let b = v.strip_prefix('-').unwrap_or(v).as_bytes();
    if b.is_empty() || v == "-0" {
        return false;
    }
    let (int, frac) = match b.iter().position(|&c| c == b'.') {
        Some(p) => (&b[..p], Some(&b[p + 1..])),
        None => (b, None),
    };
    if int.is_empty() || !int.iter().all(u8::is_ascii_digit) || (int.len() > 1 && int[0] == b'0') {
        return false;
    }
    let mut digits = if int == b"0" { 0 } else { int.len() };
    if let Some(f) = frac {
        if f.is_empty() || !f.iter().all(u8::is_ascii_digit) || f.last() == Some(&b'0') {
            return false;
        }
        digits += if int == b"0" {
            f.len() - f.iter().take_while(|&&c| c == b'0').count()
        } else {
            f.len()
        };
    }
    digits <= 15
}

pub(crate) fn col_letters(mut c: u32) -> String {
    let mut s = Vec::new();
    c += 1;
    while c > 0 {
        c -= 1;
        s.push(b'A' + (c % 26) as u8);
        c /= 26;
    }
    s.reverse();
    String::from_utf8(s).expect("ASCII")
}

impl Document {
    /// Column names for an export: the first shown row with `header`, else
    /// "Column N". Blank and repeated names get numbered.
    pub(crate) fn export_names(&self, header: bool) -> Vec<String> {
        let cols: Vec<u32> = (0..self.col_count()).collect();
        let mut names = self.titles(&cols, header);
        let mut seen: HashMap<String, u32> = HashMap::new();
        for n in names.iter_mut() {
            let k = seen.entry(n.clone()).or_insert(0);
            *k += 1;
            if *k > 1 {
                *n = format!("{n} {k}");
            }
        }
        names
    }

    /// SQL statements: CREATE TABLE (every column as text) and INSERTs of
    /// `opts.batch` rows each. Numbers are written bare when
    /// `opts.numbers` and the value is a plain number (no leading zeros,
    /// so ZIP codes and IDs keep theirs); everything else is a quoted
    /// string, empty cells NULL.
    fn export_sql(
        &self,
        out: &mut impl Write,
        opts: &ExportOptions,
        names: &[String],
        rows: std::ops::Range<u64>,
        reader: &Cols,
        progress: &Progress,
    ) -> Result<ExportReport, Error> {
        let quote = |n: &str| -> String {
            match opts.dialect {
                SqlDialect::Standard => format!("\"{}\"", n.replace('"', "\"\"")),
                SqlDialect::Mysql => format!("`{}`", n.replace('`', "``")),
                SqlDialect::SqlServer => format!("[{}]", n.replace(']', "]]")),
            }
        };
        let table = if opts.table.trim().is_empty() {
            "data"
        } else {
            opts.table.trim()
        };
        let table = quote(table);
        // Column names: as titled, unique, never empty.
        let mut seen = std::collections::HashSet::new();
        let cols: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let base = if n.trim().is_empty() {
                    format!("column{}", i + 1)
                } else {
                    n.trim().to_string()
                };
                let mut name = base.clone();
                let mut k = 2;
                while !seen.insert(name.to_lowercase()) {
                    name = format!("{base}_{k}");
                    k += 1;
                }
                quote(&name)
            })
            .collect();
        let mut w = Counter { w: out, n: 0 };
        let mut s = String::new();
        if opts.create {
            s.push_str("CREATE TABLE ");
            s.push_str(&table);
            s.push_str(" (\n");
            for (i, c) in cols.iter().enumerate() {
                s.push_str("  ");
                s.push_str(c);
                s.push_str(if opts.dialect == SqlDialect::SqlServer {
                    " NVARCHAR(MAX)"
                } else {
                    " TEXT"
                });
                s.push_str(if i + 1 < cols.len() { ",\n" } else { "\n" });
            }
            s.push_str(");\n");
        }
        let head = format!("INSERT INTO {table} ({}) VALUES\n", cols.join(", "));
        let batch = opts.batch.clamp(1, 10_000) as u64;
        let plain_number = |v: &str| -> bool {
            let t = v.strip_prefix('-').unwrap_or(v);
            !t.is_empty()
                && t.len() <= 18
                && t.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                && t.bytes().filter(|&b| b == b'.').count() <= 1
                && !t.starts_with('.')
                && !t.ends_with('.')
                && !(t.len() > 1 && t.starts_with('0') && !t.starts_with("0."))
        };
        let mut buf: Vec<Cow<str>> = Vec::new();
        let mut written = 0u64;
        progress.start(rows.end - rows.start);
        for (i, r) in rows.clone().enumerate() {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            if i as u64 % batch == 0 {
                if i > 0 {
                    s.push_str(";\n");
                }
                s.push_str(&head);
            } else {
                s.push_str(",\n");
            }
            s.push_str("  (");
            for k in 0..names.len() {
                if k > 0 {
                    s.push_str(", ");
                }
                let v = reader.get(&buf, k);
                if v.is_empty() {
                    s.push_str("NULL");
                } else if opts.numbers && plain_number(v) {
                    s.push_str(v);
                } else {
                    sql_string(&mut s, v, opts.dialect);
                }
            }
            s.push(')');
            written += 1;
            if s.len() > 1 << 16 {
                w.write_all(s.as_bytes())?;
                s.clear();
            }
            if i % 65_536 == 65_535 {
                progress.step(65_536)?;
            }
        }
        if written > 0 {
            s.push_str(";\n");
        }
        w.write_all(s.as_bytes())?;
        Ok(ExportReport {
            rows: written,
            bytes: w.n,
            ..Default::default()
        })
    }

    /// Write the document in `opts.format` to `out`; at most `limit` data
    /// rows when given (previews).
    pub fn export(
        &self,
        out: &mut impl Write,
        opts: &ExportOptions,
        limit: Option<u64>,
        progress: &Progress,
    ) -> Result<ExportReport, Error> {
        self.ready_for_bulk()?;
        let names = self.export_names(opts.header);
        let first = u64::from(opts.header && self.row_count() > 0);
        let end = match limit {
            Some(l) => self.row_count().min(first + l),
            None => self.row_count(),
        };
        let cols: Vec<u32> = (0..self.col_count()).collect();
        let reader = Cols::new(self, &cols);
        progress.start(end - first);
        let mut report = ExportReport::default();
        if opts.format == ExportFormat::Xlsx {
            drop(reader);
            let _ = names;
            let parts = match limit {
                Some(_) => std::iter::once(first..end).collect(),
                None => self.xlsx_parts(opts),
            };
            let n = parts.len();
            let sheets: Vec<crate::xlsxbook::BookSheet<'_>> = parts
                .into_iter()
                .enumerate()
                .map(|(i, rows)| crate::xlsxbook::BookSheet {
                    doc: self,
                    opts,
                    name: crate::xlsxbook::part_sheet_name(&opts.sheet, i + 1, n),
                    rows,
                })
                .collect();
            return crate::xlsxbook::write_book(out, &sheets, progress);
        }
        if opts.format == ExportFormat::Pdf {
            return self.export_pdf(out, opts, &names, first..end, progress);
        }
        if opts.format == ExportFormat::FixedWidth {
            return self.export_fixed(out, opts, &names, first..end, &reader, progress);
        }
        if opts.format == ExportFormat::Sql {
            return self.export_sql(out, opts, &names, first..end, &reader, progress);
        }
        let template = if opts.format == ExportFormat::Template {
            Some((
                parse_template(&opts.before, &names)?,
                parse_template(&opts.row, &names)?,
                parse_template(&opts.after, &names)?,
            ))
        } else {
            None
        };
        let xml_names: Vec<String> = names.iter().map(|n| xml_name(n)).collect();
        let lines = opts.format == ExportFormat::JsonLines;
        let (nl, ind, ind2) = if opts.pretty && !lines {
            ("\n", "  ", "    ")
        } else {
            ("", "", "")
        };
        let mut buf: Vec<Cow<str>> = Vec::new();
        let mut s = String::new();
        let mut w = Counter { w: out, n: 0 };
        // Opening.
        match opts.format {
            ExportFormat::Json => s.push('['),
            ExportFormat::JsonLines => {}
            ExportFormat::Xml => {
                s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<");
                s.push_str(&xml_name(&opts.root));
                s.push('>');
            }
            ExportFormat::Html => {
                s.push_str("<table>\n  <thead>\n    <tr>");
                for n in &names {
                    s.push_str("<th>");
                    xml_escape(n, &mut s);
                    s.push_str("</th>");
                }
                s.push_str("</tr>\n  </thead>\n  <tbody>\n");
            }
            ExportFormat::Markdown => {
                s.push('|');
                for n in &names {
                    s.push(' ');
                    md_escape(n, &mut s);
                    s.push_str(" |");
                }
                s.push_str("\n|");
                for _ in &names {
                    s.push_str(" --- |");
                }
                s.push('\n');
            }
            ExportFormat::Template => {
                let (before, _, _) = template.as_ref().expect("template");
                fill(before, &[], &mut s);
            }
            ExportFormat::Xlsx
            | ExportFormat::Pdf
            | ExportFormat::FixedWidth
            | ExportFormat::Sql => {
                unreachable!("handled above")
            }
        }
        w.write_all(s.as_bytes())?;
        let record = xml_name(&opts.record);
        for (i, r) in (first..end).enumerate() {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let n = names.len();
            let cell = |k: usize| reader.get(&buf, k);
            s.clear();
            match opts.format {
                ExportFormat::Json | ExportFormat::JsonLines => {
                    if i > 0 && !lines {
                        s.push(',');
                    }
                    s.push_str(nl);
                    s.push_str(ind);
                    if opts.arrays {
                        s.push('[');
                        for k in 0..n {
                            if k > 0 {
                                s.push(',');
                            }
                            json_str(cell(k), &mut s);
                        }
                        s.push(']');
                    } else {
                        s.push('{');
                        for (k, name) in names.iter().enumerate() {
                            if k > 0 {
                                s.push(',');
                            }
                            s.push_str(nl);
                            s.push_str(ind2);
                            json_str(name, &mut s);
                            s.push(':');
                            if opts.pretty && !lines {
                                s.push(' ');
                            }
                            json_str(cell(k), &mut s);
                        }
                        s.push_str(nl);
                        s.push_str(ind);
                        s.push('}');
                    }
                    if lines {
                        s.push('\n');
                    }
                }
                ExportFormat::Xml => {
                    s.push_str(nl);
                    s.push_str(ind);
                    s.push('<');
                    s.push_str(&record);
                    s.push('>');
                    for (k, name) in xml_names.iter().enumerate() {
                        s.push_str(nl);
                        s.push_str(ind2);
                        s.push('<');
                        s.push_str(name);
                        s.push('>');
                        xml_escape(cell(k), &mut s);
                        s.push_str("</");
                        s.push_str(name);
                        s.push('>');
                    }
                    s.push_str(nl);
                    s.push_str(ind);
                    s.push_str("</");
                    s.push_str(&record);
                    s.push('>');
                }
                ExportFormat::Html => {
                    s.push_str("    <tr>");
                    for k in 0..n {
                        s.push_str("<td>");
                        let mut e = String::new();
                        xml_escape(&cell(k).replace("\r\n", "\n"), &mut e);
                        s.push_str(&e.replace("\r\n", "<br>").replace('\n', "<br>"));
                        s.push_str("</td>");
                    }
                    s.push_str("</tr>\n");
                }
                ExportFormat::Markdown => {
                    s.push('|');
                    for k in 0..n {
                        s.push(' ');
                        md_escape(cell(k), &mut s);
                        s.push_str(" |");
                    }
                    s.push('\n');
                }
                ExportFormat::Template => {
                    let (_, row, _) = template.as_ref().expect("template");
                    let vals: Vec<&str> = (0..n).map(cell).collect();
                    fill(row, &vals, &mut s);
                }
                ExportFormat::Xlsx
                | ExportFormat::Pdf
                | ExportFormat::FixedWidth
                | ExportFormat::Sql => unreachable!(),
            }
            w.write_all(s.as_bytes())?;
            report.rows += 1;
            if i % 4096 == 4095 {
                progress.step(4096)?;
            }
        }
        s.clear();
        match opts.format {
            ExportFormat::Json => {
                s.push_str(nl);
                s.push_str("]\n");
            }
            ExportFormat::JsonLines => {}
            ExportFormat::Xml => {
                s.push_str(nl);
                s.push_str("</");
                s.push_str(&xml_name(&opts.root));
                s.push_str(">\n");
            }
            ExportFormat::Html => s.push_str("  </tbody>\n</table>\n"),
            ExportFormat::Markdown => {}
            ExportFormat::Template => {
                let (_, _, after) = template.as_ref().expect("template");
                fill(after, &[], &mut s);
            }
            ExportFormat::Xlsx
            | ExportFormat::Pdf
            | ExportFormat::FixedWidth
            | ExportFormat::Sql => unreachable!(),
        }
        w.write_all(s.as_bytes())?;
        w.flush()?;
        report.bytes = w.n;
        report.sheets = 0;
        Ok(report)
    }

    /// Export to a file: written to a temp file beside it, then moved into
    /// place (an existing file is replaced only by a complete export).
    pub fn export_file(
        &self,
        path: &Path,
        opts: &ExportOptions,
        progress: &Progress,
    ) -> Result<ExportReport, Error> {
        if opts.format == ExportFormat::Xlsx && opts.split_files {
            self.ready_for_bulk()?;
            let parts = self.xlsx_parts(opts);
            if parts.len() > 1 {
                return self.export_xlsx_files(path, opts, parts, progress);
            }
        }
        write_atomic(path, |w| self.export(w, opts, None, progress))
    }

    /// Each part in a file of its own beside `path`: "name (1 of 3).xlsx".
    fn export_xlsx_files(
        &self,
        path: &Path,
        opts: &ExportOptions,
        parts: Vec<std::ops::Range<u64>>,
        progress: &Progress,
    ) -> Result<ExportReport, Error> {
        let n = parts.len();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "export".into());
        let mut report = ExportReport::default();
        progress.start(parts.iter().map(|r| r.end - r.start).sum());
        let mut plans = crate::xlsxbook::Plans::default();
        // Every part goes to a temp file first; only when all are written do
        // they take their names, so a failure or Cancel leaves the folder as
        // it was.
        let mut temps: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(n);
        let wrote = (|| -> Result<(), Error> {
            for (i, rows) in parts.into_iter().enumerate() {
                let name = format!("{stem} ({} of {n}).xlsx", i + 1);
                let p = path.with_file_name(&name);
                let sheet = crate::xlsxbook::BookSheet {
                    doc: self,
                    opts,
                    name: crate::xlsxbook::part_sheet_name(&opts.sheet, i + 1, n),
                    rows,
                };
                let (temp, file) = crate::save::create_temp(&p)?;
                temps.push((temp, p));
                let r = (|| -> Result<ExportReport, Error> {
                    let mut w = BufWriter::with_capacity(1 << 20, &file);
                    let r = crate::xlsxbook::write_book_on(
                        &mut w,
                        std::slice::from_ref(&sheet),
                        progress,
                        &mut plans,
                    )?;
                    w.flush()?;
                    drop(w);
                    file.sync_all()?;
                    Ok(r)
                })()?;
                report.rows += r.rows;
                report.sheets += r.sheets;
                report.bytes += r.bytes;
                report.files.push(name);
            }
            Ok(())
        })();
        if let Err(e) = wrote {
            for (t, _) in &temps {
                let _ = fs::remove_file(t);
            }
            return Err(e);
        }
        for (k, (t, p)) in temps.iter().enumerate() {
            if let Err(e) = crate::save::replace(t, p) {
                for (t, _) in &temps[k..] {
                    let _ = fs::remove_file(t);
                }
                return Err(Error::Io(std::io::Error::new(
                    e.kind(),
                    format!("{e}; {k} of {n} parts were saved"),
                )));
            }
        }
        Ok(report)
    }
}

/// Write a file through a temp file beside it, moved into place only when
/// complete.
pub fn write_atomic(
    path: &Path,
    f: impl FnOnce(&mut BufWriter<&fs::File>) -> Result<ExportReport, Error>,
) -> Result<ExportReport, Error> {
    {
        let (temp, file) = crate::save::create_temp(path)?;
        let done = (|| -> Result<ExportReport, Error> {
            let mut w = BufWriter::with_capacity(1 << 20, &file);
            let r = f(&mut w)?;
            w.flush()?;
            drop(w);
            file.sync_all()?;
            Ok(r)
        })();
        drop(file);
        match done {
            Ok(r) => {
                if let Err(e) = crate::save::replace(&temp, path) {
                    let _ = fs::remove_file(&temp);
                    return Err(e.into());
                }
                Ok(r)
            }
            Err(e) => {
                let _ = fs::remove_file(&temp);
                Err(e)
            }
        }
    }
}

/// Widest column a fixed-width export pads to, in characters.
pub const FIXED_MAX_WIDTH: usize = 4_000;

impl Document {
    /// Columns padded with spaces (one space between), titles first with
    /// `opts.header`. Line breaks inside cells become spaces; values longer
    /// than 4,000 characters are cut.
    fn export_fixed(
        &self,
        out: &mut impl Write,
        opts: &ExportOptions,
        names: &[String],
        rows: std::ops::Range<u64>,
        reader: &Cols,
        progress: &Progress,
    ) -> Result<ExportReport, Error> {
        use rayon::prelude::*;
        /// Characters a value takes (cut to the widest column).
        fn width_of(v: &str) -> usize {
            if v.is_ascii() {
                v.len().min(FIXED_MAX_WIDTH)
            } else {
                v.chars().take(FIXED_MAX_WIDTH).count()
            }
        }
        /// Append a value padded to `w` characters (line breaks and tabs
        /// become spaces).
        fn push_padded(line: &mut String, v: &str, w: usize) {
            let mut n = 0;
            for c in v.chars().take(FIXED_MAX_WIDTH) {
                line.push(if matches!(c, '\n' | '\r' | '\t') {
                    ' '
                } else {
                    c
                });
                n += 1;
            }
            line.extend(std::iter::repeat_n(' ', w.saturating_sub(n)));
        }
        let n = names.len();
        let first: Vec<usize> = if opts.header {
            names.iter().map(|t| width_of(t)).collect()
        } else {
            vec![0; n]
        };
        // Widths: the widest value per column, read in parallel chunks.
        const CHUNK: u64 = 1 << 15;
        let chunks = (rows.end - rows.start).div_ceil(CHUNK);
        let widths = (0..chunks)
            .into_par_iter()
            .map(|k| -> Result<Vec<usize>, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut w = vec![0usize; n];
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for (k, x) in w.iter_mut().enumerate() {
                        *x = (*x).max(width_of(reader.get(&buf, k)));
                    }
                }
                Ok(w)
            })
            .try_reduce(
                || first.clone(),
                |a, b| Ok(a.iter().zip(&b).map(|(x, y)| *x.max(y)).collect()),
            )?;
        let mut w = Counter { w: out, n: 0 };
        let mut line = String::new();
        let end_line = |line: &mut String| {
            let t = line.trim_end().len();
            line.truncate(t);
            line.push_str("\r\n");
        };
        if opts.header {
            line.clear();
            for (k, t) in names.iter().enumerate() {
                if k > 0 {
                    line.push(' ');
                }
                push_padded(&mut line, t, widths[k]);
            }
            end_line(&mut line);
            w.write_all(line.as_bytes())?;
        }
        let mut buf = Vec::new();
        let mut done = 0u64;
        for r in rows.clone() {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            line.clear();
            for (k, &wk) in widths.iter().enumerate() {
                if k > 0 {
                    line.push(' ');
                }
                push_padded(&mut line, reader.get(&buf, k), wk);
            }
            end_line(&mut line);
            w.write_all(line.as_bytes())?;
            done += 1;
            if done % 4096 == 0 {
                progress.step(4096)?;
            }
        }
        Ok(ExportReport {
            rows: rows.end - rows.start,
            sheets: 1,
            bytes: w.n,
            files: Vec::new(),
        })
    }
}

pub(crate) struct Counter<W> {
    pub(crate) w: W,
    pub(crate) n: u64,
}

impl<W: Write> Write for Counter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.w.write(b)?;
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.w.flush()
    }
}

fn json_str(v: &str, out: &mut String) {
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn md_escape(v: &str, out: &mut String) {
    for c in v.chars() {
        match c {
            '|' => out.push_str("\\|"),
            '\r' => {}
            '\n' => out.push_str("<br>"),
            c => out.push(c),
        }
    }
}

fn fill(pieces: &[Piece], vals: &[&str], out: &mut String) {
    for p in pieces {
        match p {
            Piece::Text(t) => out.push_str(t),
            Piece::Col(c) => out.push_str(vals.get(*c).copied().unwrap_or("")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, OpenOptions};

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn ex(d: &Document, opts: ExportOptions) -> String {
        let mut out = Vec::new();
        d.export(&mut out, &opts, None, &Progress::new()).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn text_formats() {
        let d = doc("name,n,name\r\n\"A \"\"q\"\"\",1,x\r\n\"b|c\nd\",<2>,\r\n");
        let o = ExportOptions {
            pretty: false,
            ..Default::default()
        };
        assert_eq!(
            ex(&d, o.clone()),
            "[{\"name\":\"A \\\"q\\\"\",\"n\":\"1\",\"name 2\":\"x\"},{\"name\":\"b|c\\nd\",\"n\":\"<2>\",\"name 2\":\"\"}]\n"
        );
        let arrays = ex(
            &d,
            ExportOptions {
                arrays: true,
                ..o.clone()
            },
        );
        assert!(arrays.starts_with("[[\"A \\\"q\\\"\",\"1\",\"x\"],"));
        let parsed: serde_json::Value =
            serde_json::from_str(&ex(&d, ExportOptions::default())).unwrap();
        assert_eq!(parsed[1]["n"], "<2>");
        // JSON Lines: one record per line, never indented, each line valid JSON.
        let jl = ex(
            &d,
            ExportOptions {
                format: ExportFormat::JsonLines,
                ..o.clone()
            },
        );
        let rows: Vec<&str> = jl.lines().collect();
        assert_eq!(rows.len(), 2);
        assert!(jl.ends_with("}\n"));
        for r in &rows {
            let v: serde_json::Value = serde_json::from_str(r).unwrap();
            assert!(v.is_object());
        }
        assert_eq!(
            rows[0],
            "{\"name\":\"A \\\"q\\\"\",\"n\":\"1\",\"name 2\":\"x\"}"
        );
        let x = ex(
            &d,
            ExportOptions {
                format: ExportFormat::Xml,
                ..o.clone()
            },
        );
        assert!(
            x.contains("<row><name>A \"q\"</name><n>1</n><name_2>x</name_2></row>")
                || x.contains("<name_2>")
        );
        assert!(x.contains("<n>&lt;2&gt;</n>"));
        let h = ex(
            &d,
            ExportOptions {
                format: ExportFormat::Html,
                ..o.clone()
            },
        );
        assert!(h.contains("<td>b|c<br>d</td><td>&lt;2&gt;</td>"));
        let m = ex(
            &d,
            ExportOptions {
                format: ExportFormat::Markdown,
                ..o.clone()
            },
        );
        assert_eq!(
            m,
            "| name | n | name 2 |\n| --- | --- | --- |\n| A \"q\" | 1 | x |\n| b\\|c<br>d | <2> |  |\n"
        );
        let t = ex(
            &d,
            ExportOptions {
                format: ExportFormat::Template,
                before: "BEGIN {{\n".into(),
                row: "{name}={#2};\n".into(),
                after: "}} END".into(),
                ..o.clone()
            },
        );
        assert_eq!(t, "BEGIN {\nA \"q\"=1;\nb|c\nd=<2>;\n} END");
        // Without a header row every row is data, named Column N.
        let nh = ex(
            &d,
            ExportOptions {
                header: false,
                arrays: true,
                ..o.clone()
            },
        );
        assert!(nh.starts_with("[[\"name\",\"n\",\"name\"]"));
        let bad = ExportOptions {
            format: ExportFormat::Template,
            row: "{nope}".into(),
            ..o.clone()
        };
        assert!(d
            .export(&mut Vec::new(), &bad, None, &Progress::new())
            .is_err());
        // Previews stop early.
        let mut out = Vec::new();
        let r = d.export(&mut out, &o, Some(1), &Progress::new()).unwrap();
        assert_eq!(r.rows, 1);
    }

    #[test]
    fn xlsx_round_trip() {
        let d = doc(
            "id,text,amount\r\n007,\"multi\nline\",1.5\r\n2,_x0041_ <&>,-3\r\n3,\u{1},1.50\r\n",
        );
        let mut out = Vec::new();
        let r = d
            .export(
                &mut out,
                &ExportOptions {
                    format: ExportFormat::Xlsx,
                    sheet: "My: data?".into(),
                    // Values exactly as written (no Excel formats).
                    look: false,
                    ..Default::default()
                },
                None,
                &Progress::new(),
            )
            .unwrap();
        assert_eq!((r.rows, r.sheets), (3, 1));
        let book = crate::sheet::Book::open(&out).unwrap();
        assert_eq!(book.sheets()[0].name, "My data");
        let t = book
            .read(0, Default::default(), None, &Progress::new())
            .unwrap();
        let back = t.into_document().unwrap();
        let rows: Vec<Vec<String>> = (0..back.row_count())
            .map(|r| back.row_values(r).unwrap())
            .collect();
        assert_eq!(
            rows,
            vec![
                vec!["id", "text", "amount"],
                vec!["007", "multi\nline", "1.5"],
                vec!["2", "_x0041_ <&>", "-3"],
                vec!["3", "\u{1}", "1.50"],
            ]
        );
        assert!(excel_number("12") && excel_number("-0.25") && excel_number("0.001"));
        assert!(!excel_number("1.50") && !excel_number("007") && !excel_number("1e5"));
        assert!(!excel_number("1234567890123456") && !excel_number("-0") && !excel_number("."));
        assert_eq!(col_letters(0), "A");
        assert_eq!(col_letters(27), "AB");
        assert_eq!(col_letters(16_383), "XFD");
        // Cells over Excel's limit are refused before anything is replaced.
        let long = doc(&format!("a\r\n{}\r\n", "y".repeat(40_000)));
        assert!(long
            .export(
                &mut Vec::new(),
                &ExportOptions {
                    format: ExportFormat::Xlsx,
                    ..Default::default()
                },
                None,
                &Progress::new()
            )
            .is_err());
    }

    #[test]
    fn xlsx_writes_a_height_for_rows_that_have_one() {
        let d = doc("a,b\n1,x\n2,y\n3,z\n");
        let mut out = Vec::new();
        d.export(
            &mut out,
            &ExportOptions {
                format: ExportFormat::Xlsx,
                // Shown rows 2 and 3 (the titles are row 0): 40 px and 400 px.
                row_heights: vec![(2, 40), (3, 400)],
                ..Default::default()
            },
            None,
            &Progress::new(),
        )
        .unwrap();
        let z = crate::zipread::Zip::open(&out).unwrap();
        let sheet = z.read_string("xl/worksheets/sheet1.xml", 1 << 20).unwrap();
        // Excel rows are one-based: shown row 2 is row 3. Pixels to points: 3 / 4.
        assert!(
            sheet.contains("<row r=\"3\" ht=\"30\" customHeight=\"1\">"),
            "{sheet}"
        );
        assert!(
            sheet.contains("<row r=\"4\" ht=\"300\" customHeight=\"1\">"),
            "{sheet}"
        );
        // The others have no height of their own.
        assert!(sheet.contains("<row r=\"1\">") && sheet.contains("<row r=\"2\">"));
        assert_eq!(sheet.matches("customHeight").count(), 2);
        // None given: no custom heights at all.
        let mut plain = Vec::new();
        d.export(
            &mut plain,
            &ExportOptions {
                format: ExportFormat::Xlsx,
                ..Default::default()
            },
            None,
            &Progress::new(),
        )
        .unwrap();
        let z = crate::zipread::Zip::open(&plain).unwrap();
        let sheet = z.read_string("xl/worksheets/sheet1.xml", 1 << 20).unwrap();
        assert!(!sheet.contains("customHeight"));
    }

    /// While filtered, the export writes only the shown rows, so the heights
    /// it is given are by shown row (what the app projects): each lands on
    /// the row holding the same data as in the app.
    #[test]
    fn xlsx_heights_while_filtered_are_by_shown_row() {
        let mut d = doc("a,b\n1,keep\n2,drop\n3,keep\n4,drop\n5,keep\n");
        let spec: crate::filter::FilterSpec = serde_json::from_str(
            r#"{"columns":[{"col":1,"rule":{"type":"values","values":["keep"],"blanks":false}}],"keep_first":true}"#,
        )
        .unwrap();
        d.set_filter(&spec).unwrap();
        // Document row 5 ("5,keep") has a height; it is shown row 3.
        let shown = d.shown_rows_at(&[5])[0].unwrap();
        assert_eq!(shown, 3);
        let mut out = Vec::new();
        d.export(
            &mut out,
            &ExportOptions {
                format: ExportFormat::Xlsx,
                row_heights: vec![(shown, 40)],
                ..Default::default()
            },
            None,
            &Progress::new(),
        )
        .unwrap();
        let z = crate::zipread::Zip::open(&out).unwrap();
        let sheet = z.read_string("xl/worksheets/sheet1.xml", 1 << 20).unwrap();
        // Only shown rows are written: 4 rows (titles and the three "keep").
        assert_eq!(sheet.matches("<row ").count(), 4, "{sheet}");
        // Excel row 4 is shown row 3, and holds the 5.
        let row4 = &sheet[sheet.find("<row r=\"4\"").unwrap()..];
        let row4 = &row4[..row4.find("</row>").unwrap()];
        assert!(row4.contains("ht=\"30\" customHeight=\"1\""), "{row4}");
        assert!(row4.contains(">5<"), "{row4}");
        assert_eq!(sheet.matches("customHeight").count(), 1);
    }

    /// Excel never runs a cell of ours as a formula: anything that is not a
    /// number or a date is an inline string, and no formula element exists.
    #[test]
    fn xlsx_writes_formula_like_text_as_text() {
        let d = doc("a,b\n=1+1,@x\n+1 555,-2+3\n");
        let mut out = Vec::new();
        d.export(
            &mut out,
            &ExportOptions {
                format: ExportFormat::Xlsx,
                ..Default::default()
            },
            None,
            &Progress::new(),
        )
        .unwrap();
        let z = crate::zipread::Zip::open(&out).unwrap();
        let sheet = z.read_string("xl/worksheets/sheet1.xml", 1 << 20).unwrap();
        assert!(!sheet.contains("<f>") && !sheet.contains("<f "), "{sheet}");
        for v in ["=1+1", "@x", "+1 555", "-2+3"] {
            let cell = format!("t=\"inlineStr\"><is><t xml:space=\"preserve\">{v}</t>");
            assert!(sheet.contains(&cell), "{v}: {sheet}");
        }
    }

    #[test]
    fn xlsx_heights_given_unsorted_repeated_or_out_of_range_are_still_right() {
        let d = doc("a,b
1,x
2,y
3,z
");
        let mut out = Vec::new();
        d.export(
            &mut out,
            &ExportOptions {
                format: ExportFormat::Xlsx,
                row_heights: vec![(3, 9999), (2, 40), (2, 60), (1, 2)],
                ..Default::default()
            },
            None,
            &Progress::new(),
        )
        .unwrap();
        let z = crate::zipread::Zip::open(&out).unwrap();
        let sheet = z.read_string("xl/worksheets/sheet1.xml", 1 << 20).unwrap();
        assert!(
            sheet.contains("<row r=\"2\" ht=\"10.5\" customHeight=\"1\">"),
            "{sheet}"
        );
        assert!(
            sheet.contains("<row r=\"3\" ht=\"45\" customHeight=\"1\">"),
            "{sheet}"
        );
        assert!(
            sheet.contains("<row r=\"4\" ht=\"300\" customHeight=\"1\">"),
            "{sheet}"
        );
        assert_eq!(sheet.matches("customHeight").count(), 3);
    }

    #[test]
    fn xlsx_made_for_excel_users() {
        let d = doc("Voter ID,zip,amount,born,share,name
2001,02134,\"$1,234.50\",2024-01-31,12.5%,Ann
2002,37421,$5.00,1/2/2023,3.0%,Bo
");
        let mut out = Vec::new();
        d.export(
            &mut out,
            &ExportOptions {
                format: ExportFormat::Xlsx,
                sheet: "voters".into(),
                ..Default::default()
            },
            None,
            &Progress::new(),
        )
        .unwrap();
        let z = crate::zipread::Zip::open(&out).unwrap();
        let sheet = z.read_string("xl/worksheets/sheet1.xml", 1 << 20).unwrap();
        let styles = z.read_string("xl/styles.xml", 1 << 20).unwrap();
        let book = z.read_string("xl/workbook.xml", 1 << 20).unwrap();
        // Frozen bold titles with filter buttons, and widths.
        assert!(
            sheet.contains("state=\"frozen\"") && sheet.contains("<autoFilter ref=\"A1:F3\"/>")
        );
        assert!(sheet.contains("<cols>") && sheet.contains("numberStoredAsText"));
        assert!(
            book.contains("_xlnm._FilterDatabase") && book.contains("&apos;voters&apos;!$A$1:$F$3")
                || book.contains("'voters'!$A$1:$F$3")
        );
        // IDs and ZIP codes as text, the rest as Excel values.
        assert!(sheet.contains("<c r=\"A2\" s=\""), "{sheet}");
        assert!(sheet.contains("<c r=\"B2\" s=\"") && sheet.contains("02134"));
        assert!(sheet.contains("<v>1234.5</v>") && sheet.contains("<v>0.125</v>"));
        assert!(sheet.contains("<v>45322</v>"), "2024-01-31 is day 45322");
        assert!(styles.contains("numFmtId=\"49\"") && styles.contains("numFmtId=\"14\""));
        assert!(styles.contains("&quot;$&quot;#,##0.00"));
        assert!(z
            .read_string("docProps/app.xml", 4096)
            .unwrap()
            .contains("Natendo Data Edit"));
        // Parts: two rows a part, each with the title row.
        let mut out = Vec::new();
        let r = d
            .export(
                &mut out,
                &ExportOptions {
                    format: ExportFormat::Xlsx,
                    sheet: "voters".into(),
                    part_rows: 1,
                    ..Default::default()
                },
                None,
                &Progress::new(),
            )
            .unwrap();
        assert_eq!((r.rows, r.sheets), (2, 2));
        let book = crate::sheet::Book::open(&out).unwrap();
        assert_eq!(book.sheets()[1].name, "voters (2 of 2)");
        // Parts as files of their own.
        let dir = std::env::temp_dir().join(format!("nde-xlsx-parts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = d
            .export_file(
                &dir.join("voters.xlsx"),
                &ExportOptions {
                    format: ExportFormat::Xlsx,
                    part_rows: 1,
                    split_files: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(r.files, ["voters (1 of 2).xlsx", "voters (2 of 2).xlsx"]);
        let second = std::fs::read(dir.join("voters (2 of 2).xlsx")).unwrap();
        let t = crate::sheet::Book::open(&second)
            .unwrap()
            .read(0, Default::default(), None, &Progress::new())
            .unwrap()
            .into_document()
            .unwrap();
        assert_eq!(
            t.row_values(0).unwrap()[0],
            "Voter ID",
            "each part has the titles"
        );
        assert_eq!(t.row_values(1).unwrap()[0], "2002");
        // A cancelled export leaves the folder as it was: no part replaced,
        // no temp file left.
        std::fs::write(dir.join("voters (1 of 2).xlsx"), b"last month").unwrap();
        let stop = Progress::new();
        stop.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let e = d.export_file(
            &dir.join("voters.xlsx"),
            &ExportOptions {
                format: ExportFormat::Xlsx,
                part_rows: 1,
                split_files: true,
                ..Default::default()
            },
            &stop,
        );
        assert!(matches!(e, Err(Error::Cancelled)), "{e:?}");
        assert_eq!(
            std::fs::read(dir.join("voters (1 of 2).xlsx")).unwrap(),
            b"last month"
        );
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 2, "{left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(200))]

        /// Any table survives a trip through Excel export and import.
        #[test]
        fn xlsx_round_trips(
            cells in proptest::collection::vec(proptest::collection::vec("[a-z0-9 .,\"_x<>&\r\n\u{1}é-]{0,8}", 1..5), 1..12),
        ) {
            let mut src = Vec::new();
            for r in &cells {
                crate::reshape::push_line(&mut src, r);
            }
            let d = Document::from_bytes(src, OpenOptions {
                dialect: Some(crate::Dialect::new(b',', Some(b'"')).unwrap()),
                encoding: Some(crate::Encoding::Utf8),
                skip_lines: Some(0),
                ..Default::default()
            }, None).unwrap();
            let mut out = Vec::new();
            d.export(&mut out, &ExportOptions { format: ExportFormat::Xlsx, header: false, look: false, ..Default::default() }, None, &Progress::new()).unwrap();
            let book = crate::sheet::Book::open(&out).unwrap();
            let back = book.read(0, Default::default(), None, &Progress::new()).unwrap().into_document().unwrap();
            // Trailing empty rows and cells are not stored by a spreadsheet.
            let trim = |v: Vec<String>| {
                let n = v.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1);
                v[..n].to_vec()
            };
            let want: Vec<Vec<String>> = (0..d.row_count()).map(|r| trim(d.row_values(r).unwrap())).collect();
            let n = want.iter().rposition(|r| !r.is_empty()).map_or(0, |i| i + 1);
            let got: Vec<Vec<String>> = (0..back.row_count()).map(|r| trim(back.row_values(r).unwrap())).collect();
            let m = got.iter().rposition(|r| !r.is_empty()).map_or(0, |i| i + 1);
            proptest::prop_assert_eq!(&want[..n], &got[..m]);
        }

        /// Damaged spreadsheet files fail cleanly, never panic or hang.
        #[test]
        fn damaged_xlsx_never_panics(flips in proptest::collection::vec((0usize..4000, proptest::prelude::any::<u8>()), 1..6)) {
            let d = doc("a,b\r\n1,x\r\n2,<y>\r\n");
            let mut out = Vec::new();
            d.export(&mut out, &ExportOptions { format: ExportFormat::Xlsx, ..Default::default() }, None, &Progress::new()).unwrap();
            for (at, b) in flips {
                let n = out.len();
                out[at % n] = b;
            }
            if let Ok(book) = crate::sheet::Book::open(&out) {
                for i in 0..book.sheets().len() {
                    let _ = book.read(i, Default::default(), None, &Progress::new());
                }
            }
        }
    }

    #[test]
    fn export_file_is_atomic() {
        let dir = std::env::temp_dir().join(format!("natendo-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.json");
        std::fs::write(&path, b"old").unwrap();
        let d = doc("a\r\n1\r\n");
        let bad = ExportOptions {
            format: ExportFormat::Template,
            row: "{missing}".into(),
            ..Default::default()
        };
        assert!(d.export_file(&path, &bad, &Progress::new()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        d.export_file(&path, &ExportOptions::default(), &Progress::new())
            .unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"a\": \"1\""));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A quoted SQL string literal for `dialect`. Quotes are doubled
/// everywhere; MySQL also treats a backslash as an escape, so backslashes
/// are doubled and NUL written as `\0` there (a cell can never end the
/// string early). SQL Server gets `N'...'` so non-Latin text survives.
fn sql_string(s: &mut String, v: &str, dialect: SqlDialect) {
    if dialect == SqlDialect::SqlServer {
        s.push('N');
    }
    s.push('\'');
    for ch in v.chars() {
        match ch {
            '\'' => s.push_str("''"),
            '\\' if dialect == SqlDialect::Mysql => s.push_str("\\\\"),
            '\0' if dialect == SqlDialect::Mysql => s.push_str("\\0"),
            c => s.push(c),
        }
    }
    s.push('\'');
}

#[cfg(test)]
mod sql_tests {
    use super::*;
    use crate::OpenOptions;

    #[test]
    fn sql_statements() {
        let d = Document::from_bytes(
            b"id,name,zip,amount\r\n1,O'Neil,02134,12.50\r\n2,,37402,-3\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let mut out = Vec::new();
        let opts = ExportOptions {
            format: ExportFormat::Sql,
            table: "people".into(),
            batch: 1,
            ..Default::default()
        };
        d.export(&mut out, &opts, None, &Progress::new()).unwrap();
        let t = String::from_utf8(out).unwrap();
        assert!(
            t.starts_with("CREATE TABLE \"people\" (\n  \"id\" TEXT,\n"),
            "{t}"
        );
        assert!(t.contains("INSERT INTO \"people\" (\"id\", \"name\", \"zip\", \"amount\") VALUES\n  (1, 'O''Neil', '02134', 12.50);\n"), "{t}");
        assert!(t.contains("(2, NULL, 37402, -3);"), "{t}");
        let mut out = Vec::new();
        let opts = ExportOptions {
            format: ExportFormat::Sql,
            dialect: SqlDialect::SqlServer,
            create: false,
            ..Default::default()
        };
        d.export(&mut out, &opts, None, &Progress::new()).unwrap();
        let t = String::from_utf8(out).unwrap();
        assert!(
            t.starts_with("INSERT INTO [data] ([id], [name], [zip], [amount]) VALUES\n  (1,"),
            "{t}"
        );
        assert!(t.ends_with("(2, NULL, 37402, -3);\n"), "{t}");
    }

    /// Reads a MySQL literal the way MySQL does (backslash escapes on):
    /// the text it stands for and where it ends.
    fn mysql_read(q: &str) -> (String, usize) {
        let b: Vec<char> = q.chars().collect();
        assert_eq!(b[0], '\'');
        let (mut i, mut out) = (1, String::new());
        while i < b.len() {
            match b[i] {
                '\\' => {
                    out.push(match b[i + 1] {
                        '0' => '\0',
                        'n' => '\n',
                        c => c,
                    });
                    i += 2;
                }
                '\'' if b.get(i + 1) == Some(&'\'') => {
                    out.push('\'');
                    i += 2;
                }
                '\'' => return (out, i + 1),
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        panic!("unterminated");
    }

    #[test]
    fn sql_strings_cannot_break_out() {
        let evil = "\\'); DROP TABLE voters; -- ";
        for v in [evil, "C:\\new", "A\\B\\tC", "x\0y", "'", "\\", "\\\\'"] {
            let mut m = String::new();
            sql_string(&mut m, v, SqlDialect::Mysql);
            let (back, end) = mysql_read(&m);
            assert_eq!(back, v, "{m}");
            assert_eq!(end, m.chars().count(), "string ends early: {m}");
        }
        let mut st = String::new();
        sql_string(&mut st, evil, SqlDialect::Standard);
        assert_eq!(st, "'\\''); DROP TABLE voters; -- '");
        let mut ss = String::new();
        sql_string(&mut ss, "José O'Neil C:\\new", SqlDialect::SqlServer);
        assert_eq!(ss, "N'José O''Neil C:\\new'");
    }
}
