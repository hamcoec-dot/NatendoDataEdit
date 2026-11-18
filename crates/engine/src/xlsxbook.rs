//! Excel workbooks that open finished: a bold, frozen title row with filter
//! buttons, columns as wide as their values, dates, currency and percents as
//! real Excel values with matching formats, and IDs, ZIP codes and phone
//! numbers kept as text (no lost leading zeros, no "number stored as text"
//! warnings). A table longer than an Excel sheet goes on in parts, each with
//! the title row, on more sheets or in more files.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write;
use std::ops::Range;

use rayon::prelude::*;

use crate::document::{Document, Progress};
use crate::export::{
    col_letters, excel_number, xlsx_escape, xml_escape, Counter, ExportOptions, ExportReport,
    SHEET_PART_LIMIT, XLSX_MAX_CELL_CHARS, XLSX_MAX_COLS, XLSX_MAX_ROWS,
};
use crate::ops::Cols;
use crate::value::{self, ReadOptions};
use crate::zipwrite::{Block, ZipWriter};

/// Rows turned into XML and compressed together.
const ROWS_PER_BLOCK: u64 = 8192;
use crate::Error;

/// How a number column is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Fmt {
    /// Fixed decimals, with or without thousands separators.
    Fixed { dec: u8, group: bool },
    Currency { sym: char, dec: u8 },
    Percent { dec: u8 },
}

/// How a column's cells are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    /// Numbers Excel shows exactly as written are numbers, the rest text.
    General,
    /// Every value text, shown as text (IDs, ZIP codes, phone numbers).
    Text,
    /// Every value a number shown as Excel would show it as written.
    Number(Option<Fmt>),
    /// Every value a date, a date and time, or a time of day.
    Date { time: bool, day: bool, seconds: bool },
}

/// What a column holds, counted over every row.
#[derive(Debug, Clone, Default)]
struct Stat {
    filled: u64,
    /// Numbers (any way of writing them), and those with more than 15
    /// significant digits (Excel would change them).
    numbers: u64,
    too_long: u64,
    percents: u64,
    currency: Option<char>,
    group: bool,
    /// Decimals written (most).
    dec: u8,
    /// Written so Excel would show them differently ("1.50", "1,000").
    reshaped: u64,
    dates: u64,
    times: u64,
    with_time: u64,
    with_seconds: u64,
    early: u64,
    /// Only digits, those starting with 0, and those over 15 digits.
    digits: u64,
    zero_led: u64,
    long_digits: u64,
    /// Characters of the widest value (capped).
    width: u32,
}

impl Stat {
    fn merge(mut self, b: &Stat) -> Stat {
        self.filled += b.filled;
        self.numbers += b.numbers;
        self.too_long += b.too_long;
        self.percents += b.percents;
        self.currency = self.currency.or(b.currency);
        self.group |= b.group;
        self.dec = self.dec.max(b.dec);
        self.reshaped += b.reshaped;
        self.dates += b.dates;
        self.times += b.times;
        self.with_time += b.with_time;
        self.with_seconds += b.with_seconds;
        self.early += b.early;
        self.digits += b.digits;
        self.zero_led += b.zero_led;
        self.long_digits += b.long_digits;
        self.width = self.width.max(b.width);
        self
    }

    fn add(&mut self, v: &str, read: ReadOptions) {
        let t = v.trim();
        if t.is_empty() {
            return;
        }
        self.filled += 1;
        self.width = self.width.max(t.chars().count().min(255) as u32);
        if t.bytes().all(|c| c.is_ascii_digit()) {
            self.digits += 1;
            if t.len() > 1 && t.starts_with('0') {
                self.zero_led += 1;
            }
            if t.len() > 15 {
                self.long_digits += 1;
            }
        }
        if value::parse_number(t, read).is_some() {
            self.numbers += 1;
            let (dec_c, group_c) = if read.decimal_comma {
                (',', '.')
            } else {
                ('.', ',')
            };
            let sig = t.bytes().filter(u8::is_ascii_digit).count()
                - t
                    .bytes()
                    .skip_while(|c| !c.is_ascii_digit())
                    .take_while(|&c| c == b'0')
                    .count();
            if sig > 15 || t.contains(['e', 'E']) {
                self.too_long += 1;
            }
            if t.ends_with('%') {
                self.percents += 1;
            }
            if let Some(c) = t.chars().find(|c| CURRENCY.contains(c)) {
                self.currency.get_or_insert(c);
            }
            if t.contains(group_c) {
                self.group = true;
            }
            let dec = t
                .split_once(dec_c)
                .map_or(0, |(_, f)| f.chars().take_while(char::is_ascii_digit).count());
            self.dec = self.dec.max(dec.min(10) as u8);
            if !excel_number(t) {
                self.reshaped += 1;
            }
        } else if let Some(d) = value::parse_date(t, read) {
            let time_only = !t.contains(['-', '/', '.']) && t.contains(':') && (0.0..1.0).contains(&d);
            if time_only {
                self.times += 1;
            } else {
                self.dates += 1;
                if d < -25_509.0 {
                    // Before 1 March 1900: Excel can't show it as a date.
                    self.early += 1;
                }
            }
            if t.contains(':') {
                self.with_time += 1;
                if t.matches(':').count() > 1 {
                    self.with_seconds += 1;
                }
            }
        }
    }

    fn kind(&self, title: &str, numbers: bool) -> Kind {
        if !numbers || self.filled == 0 {
            return if numbers { Kind::General } else { Kind::Text };
        }
        // Codes: a value with a leading zero or too many digits for a
        // number, or a column titled as an ID, ZIP code or phone number
        // whose numbers are all plain digits ("37402-0042" beside "37421").
        if self.zero_led > 0
            || self.long_digits > 0
            || (self.digits > 0 && self.numbers == self.digits && id_title(title))
        {
            return Kind::Text;
        }
        if self.dates + self.times == self.filled && self.early == 0 {
            return Kind::Date {
                time: self.with_time > 0,
                day: self.dates > 0,
                seconds: self.with_seconds > 0,
            };
        }
        if self.numbers == self.filled && self.too_long == 0 {
            let fmt = if self.percents == self.filled {
                Some(Fmt::Percent { dec: self.dec })
            } else if self.percents > 0 {
                return Kind::General;
            } else if let Some(sym) = self.currency {
                Some(Fmt::Currency { sym, dec: self.dec })
            } else if self.reshaped > 0 || self.group {
                Some(Fmt::Fixed {
                    dec: self.dec,
                    group: self.group,
                })
            } else {
                None
            };
            return Kind::Number(fmt);
        }
        Kind::General
    }
}

const CURRENCY: &[char] = &['$', '€', '£', '¥', '₹', '₩', '₽', '¢'];

/// A title naming an identifier or a code, kept as text when its values are
/// all digits (a voter ID, a ZIP code, a phone number).
fn id_title(title: &str) -> bool {
    let t = title.to_lowercase();
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    const WORDS: &[&str] = &[
        "id", "ids", "zip", "zipcode", "postal", "postcode", "phone", "tel", "telephone", "mobile",
        "cell", "fax", "ssn", "acct", "account", "code", "pin", "upc", "isbn", "ein", "npi",
    ];
    words.iter().any(|w| WORDS.contains(w))
        || words
            .last()
            .is_some_and(|w| w.len() > 2 && (w.ends_with("id") || w.ends_with("zip")))
}

/// The Excel number format of a kind: a built-in id, or a format string.
fn number_format(k: Kind) -> Option<(u32, Option<String>)> {
    let dec = |d: u8| {
        if d == 0 {
            String::new()
        } else {
            format!(".{}", "0".repeat(d as usize))
        }
    };
    Some(match k {
        Kind::General => return None,
        Kind::Text => (49, None),
        Kind::Number(None) => return None,
        Kind::Number(Some(Fmt::Fixed { dec: 0, group: true })) => (3, None),
        Kind::Number(Some(Fmt::Fixed { dec: 2, group: true })) => (4, None),
        Kind::Number(Some(Fmt::Fixed { dec: 2, group: false })) => (2, None),
        Kind::Number(Some(Fmt::Fixed { dec: 0, group: false })) => (1, None),
        Kind::Number(Some(Fmt::Fixed { dec: d, group })) => (
            0,
            Some(format!("{}0{}", if group { "#,##" } else { "" }, dec(d))),
        ),
        Kind::Number(Some(Fmt::Percent { dec: 0 })) => (9, None),
        Kind::Number(Some(Fmt::Percent { dec: 2 })) => (10, None),
        Kind::Number(Some(Fmt::Percent { dec: d })) => (0, Some(format!("0{}%", dec(d)))),
        Kind::Number(Some(Fmt::Currency { sym, dec: d })) => {
            let body = format!("\"{sym}\"#,##0{}", dec(d));
            (0, Some(format!("{body};-{body}")))
        }
        Kind::Date { day: true, time: false, .. } => (14, None),
        Kind::Date { day: true, time: true, seconds: false } => (22, None),
        Kind::Date { day: true, time: true, seconds: true } => {
            (0, Some("m/d/yyyy h:mm:ss".into()))
        }
        Kind::Date { day: false, seconds, .. } => (if seconds { 21 } else { 20 }, None),
    })
}

/// One sheet of a workbook: rows `rows` of `doc` (data rows; the title row
/// is written first when `opts.header`).
pub struct BookSheet<'a> {
    pub doc: &'a Document,
    pub opts: &'a ExportOptions,
    pub name: String,
    pub rows: Range<u64>,
}

/// What a document's columns are, worked out once for all its sheets.
struct Plan {
    kinds: Vec<Kind>,
    /// Column widths (Excel characters).
    widths: Vec<f64>,
}

impl Document {
    /// Count what each column holds, over the data rows (in parallel).
    fn xlsx_plan(&self, opts: &ExportOptions, names: &[String]) -> Result<Plan, Error> {
        let n = self.col_count() as usize;
        let first = u64::from(opts.header && self.row_count() > 0);
        let rows = first..self.row_count();
        let cols: Vec<u32> = (0..n as u32).collect();
        let reader = Cols::new(self, &cols);
        let read = opts.read;
        const CHUNK: u64 = 1 << 15;
        let chunks = (rows.end - rows.start).div_ceil(CHUNK);
        let stats = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Vec<Stat>, Error> {
                let s = rows.start + c * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut st = vec![Stat::default(); n];
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for (k, s) in st.iter_mut().enumerate() {
                        s.add(reader.get(&buf, k), read);
                    }
                }
                Ok(st)
            })
            .try_reduce(
                || vec![Stat::default(); n],
                |a, b| Ok(a.into_iter().zip(&b).map(|(x, y)| x.merge(y)).collect()),
            )?;
        let kinds: Vec<Kind> = stats
            .iter()
            .enumerate()
            .map(|(k, s)| {
                if opts.look {
                    s.kind(names.get(k).map_or("", String::as_str), opts.numbers)
                } else if opts.numbers {
                    Kind::General
                } else {
                    Kind::Text
                }
            })
            .collect();
        let widths = (0..n)
            .map(|k| {
                if let Some(&px) = opts.widths.get(k) {
                    if px > 0 {
                        return (f64::from(px) / 7.0).clamp(2.0, 120.0);
                    }
                }
                let chars = match kinds[k] {
                    Kind::Date { day: true, time, .. } => {
                        if time {
                            16
                        } else {
                            10
                        }
                    }
                    _ => stats[k].width,
                } as f64;
                // The title is bold and has a filter button beside it.
                let title = names.get(k).map_or(0, |t| t.chars().count()) as f64 * 1.1 + 3.0;
                chars.max(if opts.header { title } else { 0.0 }).clamp(6.0, 60.0) + 1.0
            })
            .collect();
        Ok(Plan { kinds, widths })
    }

    /// The data rows of an Excel export, cut into parts that each fit a
    /// sheet with the title row: `part_rows` each (0: as many as fit).
    pub fn xlsx_parts(&self, opts: &ExportOptions) -> Vec<Range<u64>> {
        let first = u64::from(opts.header && self.row_count() > 0);
        let end = self.row_count();
        let most = XLSX_MAX_ROWS - first;
        let per = if opts.part_rows == 0 {
            most
        } else {
            opts.part_rows.min(most)
        };
        let mut out = Vec::new();
        let mut s = first;
        while s < end {
            out.push(s..(s + per).min(end));
            s += per;
        }
        if out.is_empty() {
            out.push(first..first);
        }
        out
    }
}

/// The sheet name for part `i` of `n` (31 characters at most, no
/// characters Excel refuses).
pub fn part_sheet_name(base: &str, i: usize, n: usize) -> String {
    let clean: String = base
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .collect();
    let clean = clean.trim().trim_matches('\'');
    let clean = if clean.is_empty() { "Sheet1" } else { clean };
    let tail = if n > 1 {
        format!(" ({i} of {n})")
    } else {
        String::new()
    };
    let head: String = clean
        .chars()
        .take(31 - tail.chars().count())
        .collect();
    format!("{}{tail}", head.trim_end())
}

/// Sheet names made unique (any case), as Excel requires.
fn unique_names(names: &mut [String]) {
    let mut seen = std::collections::HashSet::new();
    for n in names.iter_mut() {
        let mut k = 2;
        let base = n.clone();
        while !seen.insert(n.to_lowercase()) {
            let tail = format!(" {k}");
            let head: String = base.chars().take(31 - tail.len()).collect();
            *n = format!("{head}{tail}");
            k += 1;
        }
    }
}

/// Write a workbook with these sheets.
pub fn write_book(
    out: &mut impl Write,
    sheets: &[BookSheet<'_>],
    progress: &Progress,
) -> Result<ExportReport, Error> {
    progress.start(sheets.iter().map(|s| s.rows.end - s.rows.start).sum());
    write_book_on(out, sheets, progress)
}

/// As [`write_book`], counting on in `progress` (one of several files).
pub(crate) fn write_book_on(
    out: &mut impl Write,
    sheets: &[BookSheet<'_>],
    progress: &Progress,
) -> Result<ExportReport, Error> {
    if sheets.is_empty() {
        return Err(Error::Invalid("nothing to write".into()));
    }
    let mut names: Vec<String> = sheets.iter().map(|s| s.name.clone()).collect();
    unique_names(&mut names);
    // Each document's plan, once.
    let mut plans: HashMap<*const Document, (Plan, Vec<String>)> = HashMap::new();
    for s in sheets {
        if s.doc.col_count() > XLSX_MAX_COLS {
            return Err(Error::Invalid(format!(
                "Excel files hold at most {XLSX_MAX_COLS} columns; this table has {}",
                s.doc.col_count()
            )));
        }
        let key = s.doc as *const Document;
        if let std::collections::hash_map::Entry::Vacant(e) = plans.entry(key) {
            s.doc.ready_for_bulk()?;
            let titles = s.doc.export_names(s.opts.header);
            e.insert((s.doc.xlsx_plan(s.opts, &titles)?, titles));
        }
    }
    // Styles: 0 plain, 1 title, then one per number format used.
    let mut formats: Vec<(u32, Option<String>)> = Vec::new();
    let mut style_of: HashMap<Kind, u32> = HashMap::new();
    for (plan, _) in plans.values() {
        for &k in &plan.kinds {
            if let Some(f) = number_format(k) {
                if !style_of.contains_key(&k) {
                    let i = match formats.iter().position(|x| *x == f) {
                        Some(i) => i,
                        None => {
                            formats.push(f);
                            formats.len() - 1
                        }
                    };
                    style_of.insert(k, 2 + i as u32);
                }
            }
        }
    }
    let mut report = ExportReport::default();
    let mut counter = Counter { w: out, n: 0 };
    let mut zip = ZipWriter::new(&mut counter);
    let mut filters: Vec<Option<String>> = Vec::new();
    let mut s = String::new();
    for (i, sh) in sheets.iter().enumerate() {
        let (plan, titles) = &plans[&(sh.doc as *const Document)];
        let opts = sh.opts;
        let d = sh.doc;
        let n = d.col_count() as usize;
        let letters: Vec<String> = (0..n.max(1) as u32).map(col_letters).collect();
        let styles: Vec<Option<u32>> = plan.kinds.iter().map(|k| style_of.get(k).copied()).collect();
        let fancy = opts.look && opts.header;
        let mut part = zip.raw_part(&format!("xl/worksheets/sheet{}.xml", i + 1))?;
        s.clear();
        s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">");
        s.push_str("<sheetViews><sheetView workbookViewId=\"0\"");
        if i == 0 {
            s.push_str(" tabSelected=\"1\"");
        }
        if fancy {
            s.push_str("><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/><selection pane=\"bottomLeft\" activeCell=\"A2\" sqref=\"A2\"/></sheetView></sheetViews>");
        } else {
            s.push_str("/></sheetViews>");
        }
        s.push_str("<sheetFormatPr defaultRowHeight=\"15\"/>");
        if opts.look && n > 0 {
            s.push_str("<cols>");
            for (k, w) in plan.widths.iter().enumerate() {
                let _ = write!(s, "<col min=\"{0}\" max=\"{0}\" width=\"{1:.2}\" customWidth=\"1\"", k + 1, w);
                if let Some(st) = styles[k] {
                    let _ = write!(s, " style=\"{st}\"");
                }
                s.push_str("/>");
            }
            s.push_str("</cols>");
        }
        s.push_str("<sheetData>");
        part.write_block(Block::new(s.as_bytes())?)?;
        let mut row_no = 0u64;
        if opts.header {
            row_no += 1;
            s.clear();
            let _ = write!(s, "<row r=\"{row_no}\">");
            for (k, v) in titles.iter().enumerate() {
                if v.is_empty() {
                    continue;
                }
                let _ = write!(s, "<c r=\"{}{row_no}\" t=\"inlineStr\" s=\"1\"><is><t xml:space=\"preserve\">", letters[k]);
                xlsx_escape(v, &mut s);
                s.push_str("</t></is></c>");
            }
            s.push_str("</row>");
            part.write_block(Block::new(s.as_bytes())?)?;
        }
        let reader = Cols::new(d, &(0..n as u32).collect::<Vec<_>>());
        let read = opts.read;
        // Rows to XML and compressed in parallel, a batch of blocks at a
        // time, written in order.
        let first_row = row_no + 1;
        let start = sh.rows.start;
        let total = sh.rows.end - start;
        let blocks = total.div_ceil(ROWS_PER_BLOCK);
        let batch = (rayon::current_num_threads() as u64 * 2).max(4);
        let mut b0 = 0;
        while b0 < blocks {
            let b1 = (b0 + batch).min(blocks);
            let done: Vec<Block> = (b0..b1)
                .into_par_iter()
                .map(|b| -> Result<Block, Error> {
                    let s0 = start + b * ROWS_PER_BLOCK;
                    let s1 = (s0 + ROWS_PER_BLOCK).min(sh.rows.end);
                    let mut s = String::with_capacity(((s1 - s0) as usize) * (n * 24 + 16));
                    let mut buf = Vec::new();
                    for r in s0..s1 {
                        let row_no = first_row + (r - start);
                        let id = d.id_at(r).ok_or(Error::OutOfRange)?;
                        reader.read(d, id, &mut buf);
                        let _ = write!(s, "<row r=\"{row_no}\">");
                        for k in 0..n {
                            let v = reader.get(&buf, k);
                            if v.is_empty() {
                                continue;
                            }
                            if v.len() > XLSX_MAX_CELL_CHARS
                                && v.chars().count() > XLSX_MAX_CELL_CHARS
                            {
                                return Err(Error::Invalid(format!(
                                    "row {row_no}, column {} holds more than {XLSX_MAX_CELL_CHARS} characters, more than an Excel cell can hold",
                                    k + 1
                                )));
                            }
                            let _ = write!(s, "<c r=\"{}{row_no}\"", letters[k]);
                            if let Some(st) = styles[k] {
                                let _ = write!(s, " s=\"{st}\"");
                            }
                            let number = match plan.kinds[k] {
                                Kind::Text => false,
                                Kind::General => {
                                    if excel_number(v) {
                                        let _ = write!(s, "><v>{v}</v></c>");
                                        true
                                    } else {
                                        false
                                    }
                                }
                                Kind::Number(_) => match value::parse_number(v, read)
                                    .filter(|x| x.is_finite())
                                {
                                    Some(x) => {
                                        let _ = write!(s, "><v>{x}</v></c>");
                                        true
                                    }
                                    None => false,
                                },
                                Kind::Date { day, .. } => match value::parse_date(v, read) {
                                    Some(x) => {
                                        let serial = if day { x + 25_569.0 } else { x };
                                        let _ = write!(s, "><v>{serial}</v></c>");
                                        true
                                    }
                                    None => false,
                                },
                            };
                            if !number {
                                s.push_str(" t=\"inlineStr\"><is><t xml:space=\"preserve\">");
                                xlsx_escape(v, &mut s);
                                s.push_str("</t></is></c>");
                            }
                        }
                        s.push_str("</row>");
                    }
                    progress.step(s1 - s0)?;
                    Block::new(s.as_bytes())
                })
                .collect::<Result<_, _>>()?;
            for blk in done {
                part.write_block(blk)?;
            }
            if part.written() > SHEET_PART_LIMIT {
                return Err(Error::Invalid(
                    "a sheet would be too big for Excel; choose fewer rows per part".into(),
                ));
            }
            b0 = b1;
        }
        row_no += total;
        report.rows += total;
        s.clear();
        s.push_str("</sheetData>");
        let filter = (fancy && n > 0).then(|| {
            format!("A1:{}{}", letters[n - 1], row_no.max(1))
        });
        if let Some(f) = &filter {
            let _ = write!(s, "<autoFilter ref=\"{f}\"/>");
        }
        s.push_str("<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>");
        if opts.look {
            // Text that looks like a number is text on purpose (IDs, ZIP codes).
            s.push_str("<ignoredErrors><ignoredError sqref=\"A1:XFD1048576\" numberStoredAsText=\"1\"/></ignoredErrors>");
        }
        s.push_str("</worksheet>");
        part.write_block(Block::new(s.as_bytes())?)?;
        part.finish()?;
        filters.push(filter);
    }
    let k = sheets.len();
    let mut ct = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>");
    let mut wb = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><bookViews><workbookView activeTab=\"0\"/></bookViews><sheets>");
    let mut rels = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    let mut defined = String::new();
    for (i, name) in names.iter().enumerate() {
        let j = i + 1;
        let _ = write!(ct, "<Override PartName=\"/xl/worksheets/sheet{j}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>");
        wb.push_str("<sheet name=\"");
        xml_escape(name, &mut wb);
        let _ = write!(wb, "\" sheetId=\"{j}\" r:id=\"rId{j}\"/>");
        let _ = write!(rels, "<Relationship Id=\"rId{j}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{j}.xml\"/>");
        if let Some(f) = &filters[i] {
            let (a, b) = f.split_once(':').expect("a range");
            let abs = |c: &str| {
                let split = c.find(|ch: char| ch.is_ascii_digit()).unwrap_or(c.len());
                format!("${}${}", &c[..split], &c[split..])
            };
            let _ = write!(defined, "<definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"{i}\" hidden=\"1\">");
            let mut q = String::from("'");
            q.push_str(&name.replace('\'', "''"));
            q.push('\'');
            xml_escape(&q, &mut defined);
            let _ = write!(defined, "!{}:{}</definedName>", abs(a), abs(b));
        }
    }
    ct.push_str("</Types>");
    wb.push_str("</sheets>");
    if !defined.is_empty() {
        let _ = write!(wb, "<definedNames>{defined}</definedNames>");
    }
    wb.push_str("</workbook>");
    let _ = write!(rels, "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>", k + 1);
    // Styles.
    let mut st = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">");
    let custom: Vec<(u32, &String)> = formats
        .iter()
        .filter_map(|(_, c)| c.as_ref())
        .enumerate()
        .map(|(i, c)| (164 + i as u32, c))
        .collect();
    if !custom.is_empty() {
        let _ = write!(st, "<numFmts count=\"{}\">", custom.len());
        for (id, code) in &custom {
            let _ = write!(st, "<numFmt numFmtId=\"{id}\" formatCode=\"");
            xml_escape(code, &mut st);
            st.push_str("\"/>");
        }
        st.push_str("</numFmts>");
    }
    st.push_str("<fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/><family val=\"2\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/><family val=\"2\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>");
    let _ = write!(st, "<cellXfs count=\"{}\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/>", 2 + formats.len());
    let mut next_custom = 164;
    for (id, code) in &formats {
        let id = if code.is_some() {
            next_custom += 1;
            next_custom - 1
        } else {
            *id
        };
        let _ = write!(st, "<xf numFmtId=\"{id}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>");
    }
    st.push_str("</cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>");
    let root_rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>";
    // No author, company or other personal details.
    let app = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Natendo Data Edit</Application></Properties>";
    for (name, body) in [
        ("[Content_Types].xml", ct.as_str()),
        ("_rels/.rels", root_rels),
        ("docProps/app.xml", app),
        ("xl/workbook.xml", wb.as_str()),
        ("xl/_rels/workbook.xml.rels", rels.as_str()),
        ("xl/styles.xml", st.as_str()),
    ] {
        let mut p = zip.part(name)?;
        p.write_all(body.as_bytes())?;
        p.finish()?;
    }
    zip.finish()?;
    report.sheets = k as u32;
    report.bytes = counter.n;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_of(title: &str, vals: &[&str]) -> Kind {
        let mut s = Stat::default();
        for v in vals {
            s.add(v, ReadOptions::default());
        }
        s.kind(title, true)
    }

    #[test]
    fn columns_are_read_as_excel_users_expect() {
        assert_eq!(kind_of("zip", &["37421", "02134"]), Kind::Text);
        assert_eq!(kind_of("zip", &["37421", "37402"]), Kind::Text, "by its title");
        assert_eq!(kind_of("VoterID", &["2001", "2002"]), Kind::Text);
        assert_eq!(kind_of("votes", &["2001", "2002"]), Kind::Number(None));
        assert_eq!(kind_of("card", &["4111111111111111"]), Kind::Text);
        assert_eq!(kind_of("n", &["007", "abc"]), Kind::Text);
        assert_eq!(kind_of("zip", &["37421", "37402-0042", ""]), Kind::Text);
        assert_eq!(kind_of("zip", &["37421", "n/a", "1.5"]), Kind::General);
        assert_eq!(
            kind_of("amount", &["$1,234.50", "$5.00"]),
            Kind::Number(Some(Fmt::Currency { sym: '$', dec: 2 }))
        );
        assert_eq!(
            kind_of("share", &["12.5%", "3.0%"]),
            Kind::Number(Some(Fmt::Percent { dec: 1 }))
        );
        assert_eq!(kind_of("mix", &["12%", "0.5"]), Kind::General);
        assert_eq!(
            kind_of("price", &["1.50", "2.25"]),
            Kind::Number(Some(Fmt::Fixed { dec: 2, group: false }))
        );
        assert_eq!(
            kind_of("born", &["2024-01-31", "1/2/2023"]),
            Kind::Date {
                time: false,
                day: true,
                seconds: false
            }
        );
        assert_eq!(
            kind_of("at", &["2024-01-31 08:30", "2024-02-01 17:05"]),
            Kind::Date {
                time: true,
                day: true,
                seconds: false
            }
        );
        assert_eq!(
            kind_of("t", &["08:30", "17:05:10"]),
            Kind::Date {
                time: true,
                day: false,
                seconds: true
            }
        );
        assert_eq!(kind_of("old", &["1850-01-01"]), Kind::General);
        assert_eq!(kind_of("phone", &["423-555-0100"]), Kind::General);
        assert_eq!(kind_of("phone", &["4235550100"]), Kind::Text);
    }

    #[test]
    fn part_names_fit_excel() {
        assert_eq!(part_sheet_name("voters", 1, 1), "voters");
        assert_eq!(part_sheet_name("voters", 2, 10), "voters (2 of 10)");
        let long = part_sheet_name(&"x".repeat(40), 10, 10);
        assert_eq!(long.chars().count(), 31);
        assert!(long.ends_with("(10 of 10)"));
        assert_eq!(part_sheet_name("a/b:c", 1, 1), "abc");
        let mut n = vec!["Data".to_string(), "data".to_string(), "Data".to_string()];
        unique_names(&mut n);
        assert_eq!(n, ["Data", "data 2", "Data 3"]);
    }
}
