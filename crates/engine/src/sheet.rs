//! Spreadsheet files (`.xlsx`, `.ods`) read into tables.
//!
//! Only cell values are read (formulas give their last calculated value;
//! styles, comments and pictures are left out). Everything is read through
//! [`crate::zipread`] (capped, never extracted) and a streaming XML reader
//! (no entity expansion, nesting capped), so a hostile file costs bounded
//! time and memory.

mod odsstyle;

use std::collections::HashMap;
use std::rc::Rc;

use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::document::Progress;
use crate::reshape::{too_big, Table};
use crate::table::{
    attr, local, push_ref, push_text, text_of, xml_err, xml_reader, Sink, MAX_DEPTH,
};
use crate::zipread::{damaged, resolve, Zip};
use crate::Error;

/// Excel's row limit; also the most rows read from one `.xlsx` sheet.
pub const MAX_ROWS: u64 = 1_048_576;
/// Excel's column limit; cells further right are refused.
pub const MAX_COLS: u32 = 16_384;
/// Most shared strings in a workbook.
pub const MAX_SHARED: usize = 20_000_000;
/// Small package parts (relationships, workbook, styles) at most this big.
const MAX_SMALL_PART: u64 = 64 << 20;

/// A sheet's name and whether it is hidden in the workbook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetInfo {
    pub name: String,
    pub hidden: bool,
}

/// How values are read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetOptions {
    /// Keep date cells as the numbers they are stored as.
    pub raw_dates: bool,
    /// Numbers shown with a zero-padding format (`00000`, `000-00-0000`): `Some(true)` writes them padded (the
    /// way the spreadsheet displays them), `Some(false)` keeps the number, `None` keeps the number and counts
    /// them (so the import screen can ask).
    pub pad_zeros: Option<bool>,
}

/// What the guards did while a sheet was read (counts and column numbers only, never cell text).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetStats {
    /// Whole numbers of more than 15 digits kept exactly as the file stores them.
    pub kept_big: u64,
    /// Numbers of more than 15 digits whose digits the file no longer has (stored in exponent form, or
    /// fractions rounded to the 15 digits a spreadsheet keeps): shown as before.
    pub lost_digits: u64,
    /// Columns with cells in a zero-padding number format, and how many cells.
    pub padded: Vec<(u32, u64)>,
    /// Padded cells written padded / left as numbers because they are not a whole number that fits.
    pub padded_applied: u64,
    pub padded_left: u64,
    /// Columns where a guard kept a cell exactly (big whole numbers or padded cells), sorted.
    pub kept_columns: Vec<u32>,
}

impl SheetStats {
    fn note_col(&mut self, col: u32, padded: bool) {
        if padded {
            match self.padded.iter_mut().find(|p| p.0 == col) {
                Some(p) => p.1 += 1,
                None => self.padded.push((col, 1)),
            }
        }
    }

    fn kept_col(&mut self, col: u32) {
        if let Err(at) = self.kept_columns.binary_search(&col) {
            self.kept_columns.insert(at, col);
        }
    }
}

/// A zero-padding number format: zeros with literal `-` and spaces between them (`00000`, `000-00-0000`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pad {
    /// `Some(())` for a digit place, a literal character otherwise.
    parts: Vec<Option<char>>,
    digits: usize,
}

impl Pad {
    /// `n` written into the pattern, or `None` when it has more digits than the pattern has places.
    fn apply(&self, n: u64) -> Option<String> {
        let d = n.to_string();
        let zeros = self.digits.checked_sub(d.len())?;
        // Zeros first, then the number, with the literals between them.
        let mut it = std::iter::repeat_n('0', zeros).chain(d.chars());
        let mut out = String::with_capacity(self.parts.len());
        for p in &self.parts {
            match p {
                None => out.push(it.next()?),
                Some(c) => out.push(*c),
            }
        }
        Some(out)
    }
}

/// A format made only of zeros (and `-` or space between them, written plainly, escaped or in quotes) pads:
/// `00000`, `0000`, `000-00-0000`, `00"-"000`. `0.00`, `#,##0`, `0%` and other quoted text do not.
pub(crate) fn pad_by_code(code: &str) -> Option<Pad> {
    let first = code.split(';').next().unwrap_or("");
    let mut parts = Vec::new();
    let mut digits = 0;
    let mut it = first.chars();
    while let Some(c) = it.next() {
        match c {
            '0' => {
                parts.push(None);
                digits += 1;
            }
            '-' | ' ' => parts.push(Some(c)),
            '"' => loop {
                match it.next() {
                    Some('"') => break,
                    Some(l @ ('-' | ' ')) => parts.push(Some(l)),
                    _ => return None,
                }
            },
            '\\' => match it.next() {
                Some(l @ ('-' | ' ')) => parts.push(Some(l)),
                _ => return None,
            },
            _ => return None,
        }
    }
    (digits >= 2).then_some(Pad { parts, digits })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Xlsx,
    Ods,
}

/// An opened workbook.
pub struct Book<'a> {
    zip: Zip<'a>,
    kind: Kind,
    sheets: Vec<SheetInfo>,
    /// XLSX: part name of each sheet.
    parts: Vec<String>,
    workbook: String,
    date1904: bool,
    /// What the guards did in the last sheet read.
    stats: std::cell::RefCell<SheetStats>,
}

/// True when `data` looks like a spreadsheet package.
pub fn is_spreadsheet(data: &[u8]) -> bool {
    crate::zipread::is_zip(data)
}

fn rels_of(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/_rels/{name}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Relationship id → (type, resolved target) of a part.
fn relationships(zip: &Zip, part: &str) -> Result<HashMap<String, (String, String)>, Error> {
    let name = rels_of(part);
    let mut out = HashMap::new();
    if !zip.has(&name) {
        return Ok(out);
    }
    let text = zip.read_string(&name, MAX_SMALL_PART)?;
    let mut r = crate::table::xml_reader_str(&text);
    loop {
        match r.read_event().map_err(xml_err)? {
            Event::Start(e) | Event::Empty(e) if local(&e) == "Relationship" => {
                if attr(&e, "TargetMode").as_deref() == Some("External") {
                    continue;
                }
                if let (Some(id), Some(t)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    let ty = attr(&e, "Type").unwrap_or_default();
                    out.insert(id, (ty, resolve(part, &t)));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

impl<'a> Book<'a> {
    pub fn open(data: &'a [u8]) -> Result<Book<'a>, Error> {
        let zip = Zip::open(data)?;
        if zip.has("content.xml") && !zip.has("xl/workbook.xml") {
            let mut b = Book {
                zip,
                kind: Kind::Ods,
                sheets: Vec::new(),
                parts: Vec::new(),
                workbook: "content.xml".into(),
                date1904: false,
                stats: Default::default(),
            };
            b.sheets = b.ods_sheets()?;
            return Ok(b);
        }
        let workbook = relationships(&zip, "")?
            .into_values()
            .find(|(t, _)| t.ends_with("/officeDocument"))
            .map(|(_, p)| p)
            .unwrap_or_else(|| "xl/workbook.xml".into());
        if !zip.has(&workbook) {
            return Err(damaged("no workbook inside"));
        }
        let rels = relationships(&zip, &workbook)?;
        let text = zip.read_string(&workbook, MAX_SMALL_PART)?;
        let mut r = crate::table::xml_reader_str(&text);
        let mut sheets = Vec::new();
        let mut parts = Vec::new();
        let mut date1904 = false;
        loop {
            match r.read_event().map_err(xml_err)? {
                Event::Start(e) | Event::Empty(e) => match local(&e).as_str() {
                    "workbookPr" => {
                        date1904 = matches!(attr(&e, "date1904").as_deref(), Some("1" | "true"));
                    }
                    "sheet" => {
                        let name = attr(&e, "name").unwrap_or_default();
                        let hidden = attr(&e, "state").is_some_and(|s| s != "visible");
                        let target = attr(&e, "id").and_then(|id| rels.get(&id).cloned());
                        // Chart sheets and dialogs have no cells.
                        if let Some((ty, part)) = target {
                            if ty.ends_with("/worksheet") && zip.has(&part) {
                                sheets.push(SheetInfo { name, hidden });
                                parts.push(part);
                            }
                        }
                    }
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(Book {
            zip,
            kind: Kind::Xlsx,
            sheets,
            parts,
            workbook,
            date1904,
            stats: Default::default(),
        })
    }

    pub fn sheets(&self) -> &[SheetInfo] {
        &self.sheets
    }

    /// What the guards did while the last sheet was read.
    pub fn stats(&self) -> SheetStats {
        self.stats.borrow().clone()
    }

    /// Read sheet `index` (at most `limit` rows when given).
    pub fn read(
        &self,
        index: usize,
        opts: SheetOptions,
        limit: Option<u64>,
        progress: &Progress,
    ) -> Result<Table, Error> {
        *self.stats.borrow_mut() = SheetStats::default();
        if index >= self.sheets.len() {
            return Err(Error::Invalid("that sheet is not in the file".into()));
        }
        let mut sink = Sink::new(limit);
        match self.kind {
            Kind::Xlsx => self.read_xlsx(index, opts, &mut sink, progress)?,
            Kind::Ods => self.read_ods(index, opts, &mut sink, progress)?,
        }
        Ok(sink.finish())
    }

    // ---- XLSX ----

    fn shared_strings(
        &self,
        rels: &HashMap<String, (String, String)>,
    ) -> Result<Vec<Rc<str>>, Error> {
        let Some((_, part)) = rels.values().find(|(t, _)| t.ends_with("/sharedStrings")) else {
            return Ok(Vec::new());
        };
        if !self.zip.has(part) {
            return Ok(Vec::new());
        }
        let mut r = xml_reader(self.zip.reader(part)?);
        let mut buf = Vec::new();
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut total = 0usize;
        let (mut in_t, mut skip) = (false, 0u32);
        loop {
            match r.read_event_into(&mut buf).map_err(xml_err)? {
                Event::Start(e) => match local(&e).as_str() {
                    "si" => cur.clear(),
                    "t" => in_t = true,
                    "rPh" => skip += 1,
                    _ => {}
                },
                Event::Empty(e) if local(&e) == "si" => out.push(Rc::from("")),
                Event::End(e) => match e.local_name().as_ref() {
                    "si" => {
                        total += cur.len();
                        if out.len() >= MAX_SHARED || total > crate::reshape::MAX_TABLE_BYTES {
                            return Err(Error::Invalid(
                                "the workbook's text is too large to import".into(),
                            ));
                        }
                        out.push(Rc::from(unescape_x(std::mem::take(&mut cur))));
                    }
                    "t" => in_t = false,
                    "rPh" => skip = skip.saturating_sub(1),
                    _ => {}
                },
                Event::Text(t) if in_t && skip == 0 => push_text(&mut cur, &text_of(&t))?,
                Event::CData(t) if in_t && skip == 0 => push_text(&mut cur, &t)?,
                Event::GeneralRef(g) if in_t && skip == 0 => push_ref(&g, &mut cur),
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(out)
    }

    /// Per cell style: how a number shows (date, time, both, or a number).
    fn styles(
        &self,
        rels: &HashMap<String, (String, String)>,
    ) -> Result<(Vec<Shown>, Vec<Pad>), Error> {
        let Some((_, part)) = rels.values().find(|(t, _)| t.ends_with("/styles")) else {
            return Ok((Vec::new(), Vec::new()));
        };
        if !self.zip.has(part) {
            return Ok((Vec::new(), Vec::new()));
        }
        let text = self.zip.read_string(part, MAX_SMALL_PART)?;
        let mut r = crate::table::xml_reader_str(&text);
        let mut custom: HashMap<u32, Shown> = HashMap::new();
        let mut pads: Vec<Pad> = Vec::new();
        let mut xfs = Vec::new();
        let mut in_xfs = false;
        loop {
            match r.read_event().map_err(xml_err)? {
                Event::Start(e) | Event::Empty(e) => match local(&e).as_str() {
                    "numFmt" => {
                        if let (Some(id), Some(code)) = (
                            attr(&e, "numFmtId").and_then(|v| v.parse().ok()),
                            attr(&e, "formatCode"),
                        ) {
                            let shown = match pad_by_code(&code) {
                                Some(p) if pads.len() < u16::MAX as usize => {
                                    pads.push(p);
                                    Shown::Padded(pads.len() as u16 - 1)
                                }
                                _ => shown_by_code(&code),
                            };
                            custom.insert(id, shown);
                        }
                    }
                    "cellXfs" => in_xfs = true,
                    "xf" if in_xfs => {
                        let id: u32 = attr(&e, "numFmtId")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        xfs.push(custom.get(&id).copied().unwrap_or(builtin_shown(id)));
                    }
                    _ => {}
                },
                Event::End(e) if e.local_name().as_ref() == "cellXfs" => in_xfs = false,
                Event::Eof => break,
                _ => {}
            }
        }
        Ok((xfs, pads))
    }

    fn read_xlsx(
        &self,
        index: usize,
        opts: SheetOptions,
        sink: &mut Sink,
        progress: &Progress,
    ) -> Result<(), Error> {
        let rels = relationships(&self.zip, &self.workbook)?;
        let shared = self.shared_strings(&rels)?;
        let (styles, pads) = if opts.raw_dates {
            (Vec::new(), Vec::new())
        } else {
            self.styles(&rels)?
        };
        let part = &self.parts[index];
        progress.start(self.zip.declared_size(part));
        let mut r = xml_reader(self.zip.reader(part)?);
        let mut buf = Vec::new();
        let mut row: Vec<(u32, Rc<str>)> = Vec::new();
        let mut next_row = 0u64; // the next row to write
        let mut this_row = 0u64;
        let mut next_col = 0u32;
        // The cell being read.
        let mut cell = Cell::default();
        let mut row_bytes = 0usize;
        let mut depth = 0usize;
        let mut last_pos = 0u64;
        loop {
            let ev = r.read_event_into(&mut buf).map_err(xml_err)?;
            match ev {
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let empty = matches!(ev, Event::Empty(_));
                    if !empty {
                        depth += 1;
                        if depth > MAX_DEPTH {
                            return Err(damaged("nested too deeply"));
                        }
                    }
                    match local(e).as_str() {
                        "row" => {
                            this_row = match attr(e, "r").and_then(|v| v.parse::<u64>().ok()) {
                                Some(n) if n > next_row => n - 1,
                                _ => next_row,
                            };
                            if this_row >= MAX_ROWS {
                                return Err(Error::Invalid(
                                    "the sheet has more rows than a spreadsheet can hold".into(),
                                ));
                            }
                            row.clear();
                            row_bytes = 0;
                            next_col = 0;
                            if empty {
                                // A row with no cells.
                            }
                        }
                        "c" => {
                            let col = match attr(e, "r").and_then(|v| cell_col(&v)) {
                                Some(c) => c,
                                None => next_col,
                            };
                            if col >= MAX_COLS {
                                return Err(Error::Invalid(
                                    "the sheet is wider than a spreadsheet can be".into(),
                                ));
                            }
                            next_col = col + 1;
                            cell = Cell {
                                col,
                                kind: attr(e, "t").unwrap_or_default(),
                                style: attr(e, "s").and_then(|v| v.parse().ok()).unwrap_or(0),
                                ..Default::default()
                            };
                            if empty {
                                cell.done = true;
                            }
                        }
                        "v" if !empty => cell.in_v = true,
                        "t" if !empty => cell.in_t = true,
                        "rPh" if !empty => cell.skip += 1,
                        _ => {}
                    }
                }
                Event::End(ref e) => {
                    depth = depth.saturating_sub(1);
                    match e.local_name().as_ref() {
                        "v" => cell.in_v = false,
                        "t" => cell.in_t = false,
                        "rPh" => cell.skip = cell.skip.saturating_sub(1),
                        "c" => cell.done = true,
                        "row" => {
                            if this_row > next_row {
                                sink.empty_rows(this_row - next_row);
                            }
                            emit(sink, &mut row)?;
                            next_row = this_row + 1;
                            let pos = r.buffer_position();
                            progress.step(pos - last_pos)?;
                            last_pos = pos;
                            if sink.full() {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
                Event::Text(ref t) if cell.wants_text() => push_text(&mut cell.text, &text_of(t))?,
                Event::CData(ref t) if cell.wants_text() => push_text(&mut cell.text, t)?,
                Event::GeneralRef(ref g) if cell.wants_text() => push_ref(g, &mut cell.text),
                Event::Eof => break,
                _ => {}
            }
            if cell.done {
                cell.done = false;
                let v = std::mem::take(&mut cell.text);
                // Shared text is shared, not copied, into the row.
                let value: Rc<str> = match cell.kind.as_str() {
                    "s" => {
                        let hit = v.trim().parse::<usize>().ok().and_then(|i| shared.get(i));
                        let Some(text) = hit else {
                            return Err(damaged("a cell names text that is not in the file"));
                        };
                        // Checked before copying: one long text may be
                        // named by every cell of a row.
                        if row_bytes + text.len() > sink.room() {
                            return Err(too_big());
                        }
                        Rc::clone(text)
                    }
                    "b" => Rc::from(match v.trim() {
                        "1" | "true" => "TRUE".into(),
                        "0" | "false" => "FALSE".into(),
                        _ => v,
                    }),
                    "str" | "inlineStr" => Rc::from(unescape_x(v)),
                    "e" | "d" => Rc::from(v),
                    _ => {
                        let shown = styles.get(cell.style as usize).copied();
                        let mut stats = self.stats.borrow_mut();
                        Rc::from(number_text(
                            &v,
                            shown,
                            &pads,
                            opts.pad_zeros,
                            self.date1904,
                            cell.col,
                            &mut stats,
                        ))
                    }
                };
                if !value.is_empty() {
                    row_bytes += value.len() + 1;
                    if row_bytes > sink.room() {
                        return Err(too_big());
                    }
                    row.push((cell.col, value));
                }
            }
            buf.clear();
        }
        Ok(())
    }

    // ---- ODS ----

    fn ods_sheets(&self) -> Result<Vec<SheetInfo>, Error> {
        let mut r = xml_reader(self.zip.reader("content.xml")?);
        let mut buf = Vec::new();
        let mut out = Vec::new();
        let mut depth = 0usize;
        let mut table_depth = None;
        loop {
            match r.read_event_into(&mut buf).map_err(xml_err)? {
                Event::Start(e) => {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return Err(damaged("nested too deeply"));
                    }
                    if table_depth.is_none() && local(&e) == "table" {
                        table_depth = Some(depth);
                        out.push(SheetInfo {
                            name: attr(&e, "name").unwrap_or_default(),
                            hidden: false,
                        });
                    }
                }
                Event::End(_) => {
                    if table_depth == Some(depth) {
                        table_depth = None;
                    }
                    depth = depth.saturating_sub(1);
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(out)
    }

    fn read_ods(
        &self,
        index: usize,
        opts: SheetOptions,
        sink: &mut Sink,
        progress: &Progress,
    ) -> Result<(), Error> {
        progress.start(self.zip.declared_size("content.xml"));
        let mut r = xml_reader(self.zip.reader("content.xml")?);
        let mut buf = Vec::new();
        let mut depth = 0usize;
        let mut seen = 0usize;
        // Depth of the wanted table while inside it.
        let mut inside: Option<usize> = None;
        let mut other_table: Option<usize> = None;
        let mut row: Vec<(u32, Rc<str>)> = Vec::new();
        let mut row_repeat = 0u64;
        let mut col = 0u32;
        let mut cell = OdsCell::default();
        let mut row_bytes = 0usize;
        // Zero-padding formats, and the default cell styles of the columns
        // (as runs) and of the row being read.
        let mut formats = OdsFormats::default();
        if !opts.raw_dates {
            if self.zip.has("styles.xml") {
                formats.styles.read(self.zip.reader("styles.xml")?);
            }
            formats.styles.read(self.zip.reader("content.xml")?);
        }
        formats.pad_zeros = opts.pad_zeros;
        let mut next_column = 0u32;
        let mut last_pos = 0u64;
        loop {
            let ev = r.read_event_into(&mut buf).map_err(xml_err)?;
            let empty = matches!(ev, Event::Empty(_));
            match ev {
                Event::Start(ref e) | Event::Empty(ref e) => {
                    if !empty {
                        depth += 1;
                        if depth > MAX_DEPTH {
                            return Err(damaged("nested too deeply"));
                        }
                    }
                    let name = local(e);
                    if inside.is_none() {
                        if name == "table" && other_table.is_none() && !empty {
                            if seen == index {
                                inside = Some(depth);
                            } else {
                                other_table = Some(depth);
                            }
                            seen += 1;
                        }
                    } else if cell.active && !cell.text_done {
                        // Inside a cell: its text.
                        match name.as_str() {
                            "annotation" | "note" if !empty => cell.skip_depth.push(depth),
                            _ if !cell.skip_depth.is_empty() => {}
                            "p" | "h" => {
                                if cell.paragraphs > 0 {
                                    push_text(&mut cell.text, "\n")?;
                                }
                                cell.paragraphs += 1;
                            }
                            "s" => {
                                let n: usize = attr(e, "c")
                                    .and_then(|v| v.parse().ok())
                                    .unwrap_or(1)
                                    .clamp(1, 1024);
                                push_text(&mut cell.text, &" ".repeat(n))?;
                            }
                            "tab" => push_text(&mut cell.text, "\t")?,
                            "line-break" => push_text(&mut cell.text, "\n")?,
                            _ => {}
                        }
                    } else {
                        match name.as_str() {
                            "table-column" if next_column < MAX_COLS => {
                                let n = repeat(e, "number-columns-repeated")
                                    .min(u64::from(MAX_COLS - next_column))
                                    as u32;
                                if let Some(style) = attr(e, "default-cell-style-name") {
                                    let pad = formats.styles.pad_of(&style);
                                    formats.columns.push((next_column, next_column + n, pad));
                                }
                                next_column += n;
                            }
                            "table-row" => {
                                formats.row = attr(e, "default-cell-style-name")
                                    .map(|s| formats.styles.pad_of(&s));
                                formats.copies =
                                    sink.take(repeat(e, "number-rows-repeated")).max(1);
                                row.clear();
                                row_bytes = 0;
                                col = 0;
                                row_repeat = repeat(e, "number-rows-repeated");
                                if empty {
                                    sink.empty_rows(row_repeat);
                                    row_repeat = 0;
                                }
                            }
                            "table-cell" | "covered-table-cell" => {
                                let found = ods_cell_attrs(e);
                                let style = found
                                    .style
                                    .filter(|_| !formats.styles.pads.is_empty())
                                    .map(|s| formats.styles.pad_of(&s));
                                let (value, float) = (found.value, found.float);
                                cell = OdsCell {
                                    active: true,
                                    repeat: repeat(e, "number-columns-repeated"),
                                    value,
                                    float,
                                    style,
                                    depth,
                                    ..Default::default()
                                };
                                if empty {
                                    cell.finish(
                                        &mut row,
                                        &mut col,
                                        &mut row_bytes,
                                        row_room(sink, row_repeat),
                                        &formats,
                                        &mut self.stats.borrow_mut(),
                                    )?;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::End(_) => {
                    if inside == Some(depth) {
                        // The end of the wanted table.
                        return Ok(());
                    }
                    if other_table == Some(depth) {
                        other_table = None;
                    } else if inside.is_some() {
                        if cell.active && cell.skip_depth.last() == Some(&depth) {
                            cell.skip_depth.pop();
                        } else if cell.active && cell.depth == depth {
                            cell.finish(
                                &mut row,
                                &mut col,
                                &mut row_bytes,
                                row_room(sink, row_repeat),
                                &formats,
                                &mut self.stats.borrow_mut(),
                            )?;
                        } else if !cell.active {
                            // A row ends when its element closes; rows sit
                            // directly in the table or in row groups.
                            if let Some(first) = row_end(&mut row, &mut row_repeat) {
                                if first.is_empty() {
                                    sink.empty_rows(row_repeat);
                                } else {
                                    let copies = row_repeat.min(MAX_ROWS);
                                    // All copies are checked before the first is written.
                                    let bytes: u64 =
                                        first.iter().map(|v| v.len() as u64 + 1).sum::<u64>() + 1;
                                    if bytes.saturating_mul(sink.take(copies)) > sink.room() as u64
                                    {
                                        return Err(too_big());
                                    }
                                    for _ in 0..copies {
                                        sink.row(&first)?;
                                        if sink.full() {
                                            return Ok(());
                                        }
                                    }
                                }
                                row_repeat = 0;
                                let pos = r.buffer_position();
                                progress.step(pos - last_pos)?;
                                last_pos = pos;
                                if sink.full() {
                                    return Ok(());
                                }
                            }
                        }
                    }
                    depth = depth.saturating_sub(1);
                }
                Event::Text(ref t) if cell.wants_text() => push_text(&mut cell.text, &text_of(t))?,
                Event::CData(ref t) if cell.wants_text() => push_text(&mut cell.text, t)?,
                Event::GeneralRef(ref g) if cell.wants_text() => push_ref(g, &mut cell.text),
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }
}

/// Write a row's cells (column, value) as one table row.
fn emit(sink: &mut Sink, row: &mut Vec<(u32, Rc<str>)>) -> Result<(), Error> {
    if row.is_empty() {
        sink.empty_rows(1);
        return Ok(());
    }
    row.sort_by_key(|c| c.0);
    let width = row.last().map_or(0, |c| c.0 + 1) as usize;
    let mut dense: Vec<Rc<str>> = vec![Rc::from(""); width];
    for (c, v) in row.drain(..) {
        dense[c as usize] = v;
    }
    sink.row(&dense)
}

/// Room for one row that is written `repeat` times: the table's room
/// shared by the copies a preview or import will take.
fn row_room(sink: &Sink, repeat: u64) -> usize {
    let copies = sink.take(repeat.max(1)).max(1);
    (sink.room() as u64 / copies) as usize
}

/// The finished row of an ODS table as dense cells, once per row end.
fn row_end(row: &mut Vec<(u32, Rc<str>)>, repeat: &mut u64) -> Option<Vec<Rc<str>>> {
    if *repeat == 0 {
        return None; // not a row's end (a row group or header-rows end)
    }
    let width = row.last().map_or(0, |c| c.0 + 1) as usize;
    let mut dense: Vec<Rc<str>> = vec![Rc::from(""); width];
    for (c, v) in row.drain(..) {
        dense[c as usize] = v;
    }
    let all_empty = dense.iter().all(|v| v.is_empty());
    Some(if all_empty { Vec::new() } else { dense })
}

fn repeat(e: &quick_xml::events::BytesStart, name: &str) -> u64 {
    attr(e, name)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1)
        .max(1)
}

/// What a cell's start tag says, read in one pass over its attributes.
struct OdsAttrs {
    /// The typed value (`None`: use the cell's text).
    value: Option<String>,
    /// A plain number (`office:value-type="float"`).
    float: bool,
    /// The cell's own style.
    style: Option<String>,
}

fn ods_cell_attrs(e: &quick_xml::events::BytesStart) -> OdsAttrs {
    // The first of each name, as `attr` takes it.
    let mut kept: [Option<quick_xml::events::attributes::Attribute>; 7] = Default::default();
    const NAMES: [&str; 7] = [
        "value-type",
        "value",
        "date-value",
        "time-value",
        "boolean-value",
        "string-value",
        "style-name",
    ];
    let mut it = e.attributes();
    it.with_checks(false);
    for a in it.flatten() {
        let name = a.key.local_name();
        if let Some(i) = NAMES.iter().position(|n| name.as_ref() == *n) {
            if kept[i].is_none() {
                kept[i] = Some(a);
            }
        }
    }
    let get = |i: usize| {
        kept[i].as_ref().map(
            |a| match a.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
                Ok(v) => v.into_owned(),
                Err(_) => a.value.clone().into_owned(),
            },
        )
    };
    let ty = get(0);
    let value = match ty.as_deref() {
        Some("float" | "percentage" | "currency") => get(1),
        Some("date") => get(2).map(|v| v.replacen('T', " ", 1)),
        Some("time") => get(3).map(|v| ods_time(&v)),
        Some("boolean") => get(4).map(|v| {
            if v == "true" {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }),
        Some("string") => get(5),
        _ => None,
    };
    OdsAttrs {
        value,
        float: ty.as_deref() == Some("float"),
        style: get(6),
    }
}

/// `PT13H05M00S` → `13:05:00` (left alone if it reads differently).
fn ods_time(v: &str) -> String {
    let Some(rest) = v.strip_prefix("PT") else {
        return v.into();
    };
    let mut parts = [String::new(), String::new(), String::new()];
    let mut num = String::new();
    for c in rest.chars() {
        match c {
            'H' => parts[0] = std::mem::take(&mut num),
            'M' => parts[1] = std::mem::take(&mut num),
            'S' => parts[2] = std::mem::take(&mut num),
            c => num.push(c),
        }
    }
    if !num.is_empty() {
        return v.into();
    }
    let two = |s: &str| {
        if s.is_empty() {
            "00".to_string()
        } else if s.len() == 1
            || s.starts_with(|c: char| c.is_ascii_digit()) && s.find('.') == Some(1)
        {
            format!("0{s}")
        } else {
            s.to_string()
        }
    };
    format!("{}:{}:{}", two(&parts[0]), two(&parts[1]), two(&parts[2]))
}

/// What decides the padding of a cell in an `.ods` sheet.
#[derive(Default)]
struct OdsFormats {
    styles: odsstyle::OdsStyles,
    /// Columns with a default cell style: (first, past the last, pattern).
    columns: Vec<(u32, u32, Option<u16>)>,
    /// The row's default cell style, when it has one.
    row: Option<Option<u16>>,
    /// How many times the row is taken.
    copies: u64,
    pad_zeros: Option<bool>,
}

impl OdsFormats {
    /// The pattern of a cell in column `col` that has no style of its own.
    fn inherited(&self, col: u32) -> Option<u16> {
        if let Some(row) = self.row {
            return row;
        }
        let at = self.columns.partition_point(|c| c.1 <= col);
        match self.columns.get(at) {
            Some(c) if c.0 <= col => c.2,
            _ => self.styles.default_pad(),
        }
    }
}

#[derive(Default)]
struct OdsCell {
    /// A number (`office:value-type="float"`).
    float: bool,
    /// The cell's own style, when it names one: its pattern.
    style: Option<Option<u16>>,
    active: bool,
    text_done: bool,
    repeat: u64,
    value: Option<String>,
    text: String,
    paragraphs: u32,
    depth: usize,
    skip_depth: Vec<usize>,
}

impl OdsCell {
    fn wants_text(&self) -> bool {
        self.active && self.skip_depth.is_empty() && self.paragraphs > 0
    }

    /// Put the cell (repeated) into the row. Its size times the repeat
    /// count is checked against `room` before anything is copied.
    fn finish(
        &mut self,
        row: &mut Vec<(u32, Rc<str>)>,
        col: &mut u32,
        row_bytes: &mut usize,
        room: usize,
        formats: &OdsFormats,
        stats: &mut SheetStats,
    ) -> Result<(), Error> {
        self.active = false;
        let stored = self.value.take();
        // Only a stored number can be padded; its displayed text is kept to check against.
        let number = self.float && stored.is_some() && !formats.styles.pads.is_empty();
        let value = stored.unwrap_or_else(|| std::mem::take(&mut self.text));
        let n = self.repeat.max(1);
        if !value.is_empty() {
            if u64::from(*col) + n > u64::from(MAX_COLS) {
                return Err(Error::Invalid(
                    "the sheet is wider than a spreadsheet can be".into(),
                ));
            }
            // (A padded number is at most a pattern long.)
            let longest = if number {
                value.len().max(odsstyle::LONGEST)
            } else {
                value.len()
            };
            let need = (longest as u64 + 1).saturating_mul(n);
            if (*row_bytes as u64).saturating_add(need) > room as u64 {
                return Err(too_big());
            }
            *row_bytes += need as usize;
            // One copy of the text, shared by every repeat.
            let value: Rc<str> = Rc::from(value);
            // The last padded text, shared by neighbours with the same pattern.
            let mut last: Option<(u16, Rc<str>)> = None;
            for i in 0..n {
                let c = *col + i as u32;
                let mut text = Rc::clone(&value);
                if number {
                    if let Some(pad) = self.style.unwrap_or_else(|| formats.inherited(c)) {
                        let padded = padding_of(
                            value.trim(),
                            formats.styles.pads.get(pad as usize),
                            Some(&self.text),
                        );
                        if let Some(p) =
                            count_padded(padded, formats.pad_zeros, c, formats.copies, stats)
                        {
                            text = match &last {
                                Some((at, t)) if *at == pad => Rc::clone(t),
                                _ => {
                                    let t: Rc<str> = Rc::from(p);
                                    last = Some((pad, Rc::clone(&t)));
                                    t
                                }
                            };
                        }
                    }
                }
                row.push((c, text));
            }
        }
        *col = (u64::from(*col) + n).min(u64::from(MAX_COLS)) as u32;
        Ok(())
    }
}

#[derive(Default)]
struct Cell {
    col: u32,
    kind: String,
    style: u32,
    text: String,
    in_v: bool,
    in_t: bool,
    skip: u32,
    done: bool,
}

impl Cell {
    fn wants_text(&self) -> bool {
        (self.in_v || (self.in_t && self.kind == "inlineStr")) && self.skip == 0
    }
}

/// Spreadsheet text stores characters XML cannot hold as `_xHHHH_`
/// (and a literal `_x` sequence as `_x005F_xHHHH_`).
fn unescape_x(s: String) -> String {
    if !s.contains("_x") {
        return s;
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut from = 0;
    while i + 7 <= b.len() {
        if b[i] == b'_'
            && b[i + 1] == b'x'
            && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit)
            && b[i + 6] == b'_'
        {
            let code = u32::from_str_radix(&s[i + 2..i + 6], 16).unwrap_or(0xFFFD);
            out.push_str(&s[from..i]);
            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
            i += 7;
            from = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[from..]);
    out
}

/// `"AB12"` → column 27 (0-based).
fn cell_col(r: &str) -> Option<u32> {
    let mut col: u64 = 0;
    let mut any = false;
    for b in r.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' => {
                col = col * 26 + u64::from(b.to_ascii_uppercase() - b'A' + 1);
                any = true;
                if col > u64::from(u32::MAX) {
                    return None;
                }
            }
            b'0'..=b'9' => break,
            _ => return None,
        }
    }
    any.then(|| (col - 1) as u32)
}

/// How a number is shown, from its format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    Number,
    Date,
    Time,
    DateTime,
    /// A zero-padding format: the index of its pattern.
    Padded(u16),
}

fn builtin_shown(id: u32) -> Shown {
    match id {
        14..=17 | 27..=36 | 50..=58 => Shown::Date,
        18..=21 | 45..=47 => Shown::Time,
        22 => Shown::DateTime,
        _ => Shown::Number,
    }
}

/// Read a custom number format: dates have y or d (or m without h/s),
/// times have h or s. Quoted text, escapes and `[...]` are skipped;
/// elapsed times (`[h]`) stay numbers.
fn shown_by_code(code: &str) -> Shown {
    let lower = code.to_ascii_lowercase();
    if lower.contains("[h") || lower.contains("[m") || lower.contains("[s") {
        return Shown::Number;
    }
    let mut plain = String::new();
    let mut it = lower.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                for d in it.by_ref() {
                    if d == '"' {
                        break;
                    }
                }
            }
            '[' => {
                for d in it.by_ref() {
                    if d == ']' {
                        break;
                    }
                }
            }
            '\\' => {
                it.next();
            }
            ';' => break, // only the first section decides
            c => plain.push(c),
        }
    }
    let date = plain.contains('y') || plain.contains('d');
    let time = plain.contains('h') || plain.contains('s');
    match (date || (plain.contains('m') && !time), time) {
        (true, true) => Shown::DateTime,
        (true, false) => Shown::Date,
        (false, true) => Shown::Time,
        _ => Shown::Number,
    }
}

/// A number cell's text: a date or time when its format says so, otherwise
/// the stored number (cut to 15 significant digits, as spreadsheets show).
fn number_text(
    v: &str,
    shown: Option<Shown>,
    pads: &[Pad],
    pad_zeros: Option<bool>,
    date1904: bool,
    col: u32,
    stats: &mut SheetStats,
) -> String {
    let t = v.trim();
    let Ok(x) = t.parse::<f64>() else {
        return v.to_string();
    };
    match shown {
        Some(s @ (Shown::Date | Shown::Time | Shown::DateTime)) => {
            serial_to_text(x, s, date1904).unwrap_or_else(|| short_number(t, x, col, stats))
        }
        Some(Shown::Padded(i)) => {
            let padded = padding_of(t, pads.get(i as usize), None);
            count_padded(padded, pad_zeros, col, 1, stats)
                .unwrap_or_else(|| short_number(t, x, col, stats))
        }
        _ => short_number(t, x, col, stats),
    }
}

/// The text a cell in a zero-padding format shows for stored number `t`: only a whole, non-negative number that
/// fits the pattern is padded. With `shown` (the text the file itself says it displays), the two must agree.
fn padding_of(t: &str, pad: Option<&Pad>, shown: Option<&str>) -> Option<String> {
    let whole = t
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| t.parse::<u64>().ok())
        .flatten();
    whole
        .and_then(|n| pad.and_then(|p| p.apply(n)))
        // (A space may be shown as a non-breaking one.)
        .filter(|p| {
            shown.is_none_or(|s| {
                s.chars()
                    .map(|c| if c == '\u{a0}' { ' ' } else { c })
                    .eq(p.chars())
            })
        })
}

/// Count `cells` cells of a padding format in column `col`, and give the padded text when it is to be written.
fn count_padded(
    padded: Option<String>,
    pad_zeros: Option<bool>,
    col: u32,
    cells: u64,
    stats: &mut SheetStats,
) -> Option<String> {
    for _ in 0..cells {
        stats.note_col(col, true);
    }
    if padded.is_none() {
        stats.padded_left += cells;
    }
    match (pad_zeros, padded) {
        (Some(true), Some(p)) => {
            stats.padded_applied += cells;
            stats.kept_col(col);
            Some(p)
        }
        _ => None,
    }
}

/// Keep the stored text unless it has more than 15 significant digits. A whole number written out in full
/// (an ID, an account number: `1234567890123456789`) is kept exactly as the file stores it; one stored in
/// exponent form, or a fraction, has no more digits than a spreadsheet keeps (15) and is shown as before.
fn short_number(t: &str, x: f64, col: u32, stats: &mut SheetStats) -> String {
    let digits = t
        .split(['e', 'E'])
        .next()
        .unwrap_or("")
        .trim_start_matches(['-', '+'])
        .trim_start_matches(['0', '.'])
        .chars()
        .filter(char::is_ascii_digit)
        .count();
    if digits <= 15 || !x.is_finite() {
        return t.to_string();
    }
    if t.trim_start_matches(['-', '+'])
        .bytes()
        .all(|b| b.is_ascii_digit())
    {
        stats.kept_big += 1;
        stats.kept_col(col);
        return t.to_string();
    }
    stats.lost_digits += 1;
    let s = format!("{:.14e}", x);
    let (m, e) = s.split_once('e').expect("exponent form");
    let e: i32 = e.parse().unwrap_or(0);
    if (-5..15).contains(&e) {
        let decimals = (14 - e).max(0) as usize;
        let f = format!("{x:.decimals$}");
        if f.contains('.') {
            f.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            f
        }
    } else {
        let m = if m.contains('.') {
            m.trim_end_matches('0').trim_end_matches('.')
        } else {
            m
        };
        format!("{m}E{}{}", if e < 0 { "-" } else { "+" }, e.abs())
    }
}

/// Days since 1970-01-01 → (year, month, day). (Proleptic Gregorian.)
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A spreadsheet date serial as text (`None` when out of range).
fn serial_to_text(x: f64, shown: Shown, date1904: bool) -> Option<String> {
    if !x.is_finite() || !(0.0..2_958_466.0).contains(&x) {
        return None;
    }
    let mut days = x.floor() as i64;
    let mut secs = ((x - x.floor()) * 86_400.0).round() as i64;
    if secs >= 86_400 {
        days += 1;
        secs -= 86_400;
    }
    let time = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    if shown == Shown::Time && days == 0 {
        return Some(time);
    }
    let date = if date1904 {
        let (y, m, d) = civil(days - 24_107);
        format!("{y:04}-{m:02}-{d:02}")
    } else if days == 60 {
        "1900-02-29".to_string() // the 1900 system's famous non-day
    } else if days == 0 {
        "1900-01-00".to_string()
    } else {
        let (y, m, d) = civil(days - if days < 60 { 25_568 } else { 25_569 });
        format!("{y:04}-{m:02}-{d:02}")
    };
    Some(match shown {
        Shown::Date if secs == 0 => date,
        Shown::Time => time,
        _ => format!("{date} {time}"),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::zipread::tests::build;

    pub(crate) fn xlsx(sheet: &str, shared: &str, styles: &str) -> Vec<u8> {
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
        let wb = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr/><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Secret" sheetId="2" state="hidden" r:id="rId2"/></sheets></workbook>"#;
        let wbrels = r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="/xl/worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
        build(&[
            ("_rels/.rels", rels.as_bytes(), false),
            ("xl/workbook.xml", wb.as_bytes(), true),
            ("xl/_rels/workbook.xml.rels", wbrels.as_bytes(), true),
            ("xl/worksheets/sheet1.xml", sheet.as_bytes(), true),
            (
                "xl/worksheets/sheet2.xml",
                b"<worksheet><sheetData><row><c t=\"inlineStr\"><is><t>two</t></is></c></row></sheetData></worksheet>",
                true,
            ),
            ("xl/sharedStrings.xml", shared.as_bytes(), true),
            ("xl/styles.xml", styles.as_bytes(), true),
        ])
    }

    fn rows(t: &Table) -> Vec<Vec<String>> {
        let d = t_doc(t);
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    fn t_doc(t: &Table) -> crate::Document {
        crate::reshape::Table {
            bytes: t.bytes.clone(),
            rows: t.rows,
            cols: t.cols,
        }
        .into_document()
        .unwrap()
    }

    #[test]
    fn reads_xlsx_values() {
        let sheet = r#"<worksheet><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c></row>
<row r="2"><c r="A2"><v>1.5</v></c><c r="C2" s="1"><v>45306.5</v></c><c r="D2" t="b"><v>1</v></c><c r="E2" t="e"><v>#DIV/0!</v></c></row>
<row r="4"><c r="B4" t="inlineStr"><is><t>in &amp; line</t></is></c><c r="A4" t="str"><f>A1</f><v>calc</v></c><c r="F4"><v>0.30000000000000004</v></c><c r="G4" s="2"><v>0.75</v></c></row>
<row r="9"/>
</sheetData></worksheet>"#;
        let shared = r#"<sst><si><t>name</t></si><si><r><t>ri</t></r><r><t>ch</t></r><rPh><t>skip</t></rPh></si><si><t xml:space="preserve"> a,"b" </t></si></sst>"#;
        let styles = r#"<styleSheet><numFmts><numFmt numFmtId="164" formatCode="yyyy-mm-dd hh:mm"/></numFmts><cellStyleXfs><xf numFmtId="14"/></cellStyleXfs><cellXfs><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="20"/></cellXfs></styleSheet>"#;
        let data = xlsx(sheet, shared, styles);
        let b = Book::open(&data).unwrap();
        assert_eq!(
            b.sheets(),
            &[
                SheetInfo {
                    name: "Data".into(),
                    hidden: false
                },
                SheetInfo {
                    name: "Secret".into(),
                    hidden: true
                }
            ]
        );
        let t = b
            .read(0, SheetOptions::default(), None, &Progress::new())
            .unwrap();
        assert_eq!(
            rows(&t),
            vec![
                vec!["name", "rich", " a,\"b\" "],
                vec!["1.5", "", "2024-01-15 12:00:00", "TRUE", "#DIV/0!"],
                vec![""],
                vec!["calc", "in & line", "", "", "", "0.3", "18:00:00"],
            ]
        );
        let raw = b
            .read(
                0,
                SheetOptions {
                    raw_dates: true,
                    ..Default::default()
                },
                None,
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(rows(&raw)[1][2], "45306.5");
        let t = b
            .read(1, SheetOptions::default(), None, &Progress::new())
            .unwrap();
        assert_eq!(rows(&t), vec![vec!["two"]]);
        let t = b
            .read(0, SheetOptions::default(), Some(2), &Progress::new())
            .unwrap();
        assert_eq!(t.rows, 2);
        assert!(b
            .read(5, SheetOptions::default(), None, &Progress::new())
            .is_err());
    }

    /// A1a: whole numbers of more than 15 digits are kept exactly; exponent forms and fractions are
    /// counted as digits already lost; zero-padding formats are counted and padded only on request.
    #[test]
    fn long_numbers_and_zero_padding_are_guarded() {
        let sheet = r#"<worksheet><sheetData>
<row r="1"><c r="A1"><v>1234567890123456789</v></c><c r="B1"><v>1.2345678901234567E+18</v></c><c r="C1"><v>0.30000000000000004</v></c><c r="D1" s="1"><v>2134</v></c><c r="E1" s="2"><v>123456789</v></c><c r="F1" s="1"><v>1.5</v></c><c r="G1" s="3"><v>2.5</v></c></row>
</sheetData></worksheet>"#;
        let styles = r#"<styleSheet><numFmts><numFmt numFmtId="164" formatCode="00000"/><numFmt numFmtId="165" formatCode="000-00-0000"/></numFmts><cellXfs><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="165"/><xf numFmtId="2"/></cellXfs></styleSheet>"#;
        let data = xlsx(sheet, "<sst/>", styles);
        let b = Book::open(&data).unwrap();
        let read = |pad| {
            let t = b
                .read(
                    0,
                    SheetOptions {
                        raw_dates: false,
                        pad_zeros: pad,
                    },
                    None,
                    &Progress::new(),
                )
                .unwrap();
            (rows(&t).remove(0), b.stats())
        };
        let (row, st) = read(None);
        assert_eq!(row[0], "1234567890123456789");
        assert_eq!(row[1], "1.23456789012346E+18");
        assert_eq!(row[2], "0.3");
        assert_eq!(&row[3..], ["2134", "123456789", "1.5", "2.5"]);
        assert_eq!((st.kept_big, st.lost_digits), (1, 2));
        assert_eq!(st.padded, vec![(3, 1), (4, 1), (5, 1)]);
        assert_eq!((st.padded_applied, st.padded_left), (0, 1));
        assert_eq!(st.kept_columns, vec![0]);
        let (row, st) = read(Some(true));
        assert_eq!(&row[3..], ["02134", "123-45-6789", "1.5", "2.5"]);
        assert_eq!((st.padded_applied, st.padded_left), (2, 1));
        let (row, _) = read(Some(false));
        assert_eq!(&row[3..], ["2134", "123456789", "1.5", "2.5"]);
        // Formats that only look similar do not pad.
        assert!(pad_by_code("0.00").is_none());
        assert!(pad_by_code("#,##0").is_none());
        assert!(pad_by_code("0%").is_none());
        assert!(pad_by_code("0").is_none());
        assert!(pad_by_code("000-00-0000").is_some());
    }

    #[test]
    fn refuses_hostile_sheets() {
        let wide =
            r#"<worksheet><sheetData><row><c r="ZZZZ1"><v>1</v></c></row></sheetData></worksheet>"#;
        let b = xlsx(wide, "<sst/>", "<styleSheet/>");
        assert!(Book::open(&b)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .is_err());
        let tall = r#"<worksheet><sheetData><row r="99999999"><c><v>1</v></c></row></sheetData></worksheet>"#;
        let b = xlsx(tall, "<sst/>", "<styleSheet/>");
        assert!(Book::open(&b)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .is_err());
        let deep = format!("<worksheet>{}</worksheet>", "<x>".repeat(MAX_DEPTH + 10));
        let b = xlsx(&deep, "<sst/>", "<styleSheet/>");
        assert!(Book::open(&b)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .is_err());
        // Entities declared in a DTD are never expanded.
        let laughs = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;">]><worksheet><sheetData><row><c t="inlineStr"><is><t>&lol2;</t></is></c></row></sheetData></worksheet>"#;
        let b = xlsx(laughs, "<sst/>", "<styleSheet/>");
        let t = Book::open(&b)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .unwrap();
        assert_eq!(rows(&t), vec![vec!["&lol2;"]]);
        // Every truncation of a real file fails cleanly.
        let good = xlsx(
            r#"<worksheet><sheetData><row><c t="s"><v>0</v></c></row></sheetData></worksheet>"#,
            "<sst><si><t>x</t></si></sst>",
            "<styleSheet/>",
        );
        for cut in (0..good.len()).step_by(7) {
            if let Ok(b) = Book::open(&good[..cut]) {
                let _ = b.read(0, SheetOptions::default(), None, &Progress::new());
            }
        }
    }

    /// Review finding 1: short files that name one long value many times
    /// are refused before the copies are made.
    #[test]
    fn long_values_repeated_are_refused() {
        // .xlsx: one 2 MB shared string used by 16,384 cells of a row.
        // Varied letters, so the file itself is not a zip bomb.
        let mut seed = 1u32;
        let big: String = (0..2 << 20)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (b'a' + (seed >> 24) as u8 % 26) as char
            })
            .collect();
        let cells: String = (0..16_384).map(|_| "<c t=\"s\"><v>0</v></c>").collect();
        let sheet =
            format!("<worksheet><sheetData><row r=\"1\">{cells}</row></sheetData></worksheet>");
        let data = xlsx(
            &sheet,
            &format!("<sst><si><t>{big}</t></si></sst>"),
            "<styleSheet/>",
        );
        let t = std::time::Instant::now();
        let e = Book::open(&data)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .unwrap_err();
        assert!(e.to_string().contains("too large"), "{e}");
        assert!(t.elapsed().as_secs() < 20);
        // A shared string that is not there is an error, not a blank.
        let sheet =
            "<worksheet><sheetData><row><c t=\"s\"><v>7</v></c></row></sheetData></worksheet>";
        let data = xlsx(sheet, "<sst><si><t>a</t></si></sst>", "<styleSheet/>");
        assert!(Book::open(&data)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .is_err());
        // .ods: one 2 MB cell repeated 16,384 times: refused before copying.
        let content = format!(
            "<office:document-content><office:body><office:spreadsheet><table:table table:name=\"S\"><table:table-row><table:table-cell table:number-columns-repeated=\"16384\"><text:p>{big}</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"
        );
        let data = ods(&content);
        let t = std::time::Instant::now();
        let e = Book::open(&data)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .unwrap_err();
        assert!(e.to_string().contains("too large"), "{e}");
        assert!(t.elapsed().as_secs() < 5);
        // .ods: a 2 MB cell across 400 columns, the row repeated 1,000
        // times: refused while the row is built, for an import and for a
        // preview.
        let content = format!(
            "<office:document-content><office:body><office:spreadsheet><table:table table:name=\"S\"><table:table-row table:number-rows-repeated=\"1000\"><table:table-cell table:number-columns-repeated=\"400\"><text:p>{big}</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"
        );
        let data = ods(&content);
        for limit in [None, Some(10)] {
            let t = std::time::Instant::now();
            let e = Book::open(&data)
                .unwrap()
                .read(0, SheetOptions::default(), limit, &Progress::new())
                .unwrap_err();
            assert!(e.to_string().contains("too large"), "{e}");
            assert!(t.elapsed().as_secs() < 5);
        }
        // A short repeated row is still written out each time.
        let content = "<office:document-content><office:body><office:spreadsheet><table:table table:name=\"S\"><table:table-row table:number-rows-repeated=\"3\"><table:table-cell table:number-columns-repeated=\"2\"><text:p>ab</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>";
        let t = Book::open(&ods(content))
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .unwrap();
        assert_eq!(t.into_document().unwrap().row_count(), 3);
        // .xlsx: the 2 MB shared string in 400 cells of each of 3 rows.
        let row: String = (0..400).map(|_| "<c t=\"s\"><v>0</v></c>").collect();
        let sheet = format!("<worksheet><sheetData><row>{row}</row><row>{row}</row><row>{row}</row></sheetData></worksheet>");
        let data = xlsx(
            &sheet,
            &format!("<sst><si><t>{big}</t></si></sst>"),
            "<styleSheet/>",
        );
        let e = Book::open(&data)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .unwrap_err();
        assert!(e.to_string().contains("too large"), "{e}");
        // .ods: a value pushed past column 16,384 is refused, not dropped.
        let content = "<office:document-content><office:body><office:spreadsheet><table:table table:name=\"S\"><table:table-row><table:table-cell table:number-columns-repeated=\"16383\"/><table:table-cell table:number-columns-repeated=\"2\"><text:p>v</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>";
        let data = ods(content);
        assert!(Book::open(&data)
            .unwrap()
            .read(0, SheetOptions::default(), None, &Progress::new())
            .is_err());
    }

    pub(crate) fn ods(content: &str) -> Vec<u8> {
        build(&[
            (
                "mimetype",
                b"application/vnd.oasis.opendocument.spreadsheet",
                false,
            ),
            ("content.xml", content.as_bytes(), true),
        ])
    }

    /// A made-up `.ods` file: `styles` goes in styles.xml, `auto` in the automatic styles of content.xml, `table`
    /// inside the sheet. Hand-written from the ODF form.
    fn ods_with(styles: &str, auto: &str, table: &str) -> Vec<u8> {
        let content = format!(
            r#"<office:document-content xmlns:office="o" xmlns:table="t" xmlns:text="x" xmlns:style="s" xmlns:number="n"><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:spreadsheet><table:table table:name="S">{table}</table:table></office:spreadsheet></office:body></office:document-content>"#
        );
        let styles = format!(
            r#"<office:document-styles xmlns:office="o" xmlns:style="s" xmlns:number="n"><office:styles>{styles}</office:styles></office:document-styles>"#
        );
        build(&[
            (
                "mimetype",
                b"application/vnd.oasis.opendocument.spreadsheet",
                false,
            ),
            ("content.xml", content.as_bytes(), true),
            ("styles.xml", styles.as_bytes(), true),
        ])
    }

    fn number_style(name: &str, inner: &str) -> String {
        format!(r#"<number:number-style style:name="{name}">{inner}</number:number-style>"#)
    }

    fn digits(n: u32) -> String {
        format!(r#"<number:number number:decimal-places="0" number:min-integer-digits="{n}"/>"#)
    }

    fn cell_style(name: &str, data: &str) -> String {
        format!(
            r#"<style:style style:name="{name}" style:family="table-cell" style:data-style-name="{data}"/>"#
        )
    }

    fn float(style: &str, value: &str, shown: &str) -> String {
        let style = if style.is_empty() {
            String::new()
        } else {
            format!(r#" table:style-name="{style}""#)
        };
        format!(
            r#"<table:table-cell{style} office:value-type="float" office:value="{value}"><text:p>{shown}</text:p></table:table-cell>"#
        )
    }

    fn ods_read(data: &[u8], pad_zeros: Option<bool>) -> (Vec<Vec<String>>, SheetStats) {
        let b = Book::open(data).unwrap();
        let t = b
            .read(
                0,
                SheetOptions {
                    raw_dates: false,
                    pad_zeros,
                },
                None,
                &Progress::new(),
            )
            .unwrap();
        (rows(&t), b.stats())
    }

    #[test]
    fn ods_zero_padding_formats_are_counted_and_applied_when_asked() {
        let ssn = r#"<number:number number:decimal-places="0" number:min-decimal-places="0" number:min-integer-digits="9"><number:embedded-text number:position="6">-</number:embedded-text><number:embedded-text number:position="4">-</number:embedded-text></number:number>"#;
        let spaced = r#"<number:number number:decimal-places="0" number:min-integer-digits="5"><number:embedded-text number:position="3"> </number:embedded-text></number:number>"#;
        let styles = number_style("N5", &digits(5))
            + &number_style("NS", ssn)
            + &number_style("NW", spaced)
            + &cell_style("base", "N5");
        let auto = cell_style("zip", "N5")
            + &cell_style("ssn", "NS")
            + &cell_style("sp", "NW")
            + r#"<style:style style:name="child" style:family="table-cell" style:parent-style-name="base"/>"#;
        let table = format!(
            "<table:table-row>{}{}{}{}{}{}{}</table:table-row>",
            float("zip", "123", "00123"),
            float("ssn", "12345678", "012-34-5678"),
            float("sp", "1234", "01 234"),
            float("child", "7", "00007"),
            // Left as they are: negative, a fraction, too long.
            float("zip", "-5", "-00005"),
            float("zip", "1.5", "00002"),
            float("zip", "123456", "123456"),
        );
        let data = ods_with(&styles, &auto, &table);
        let (rows_asked, st) = ods_read(&data, None);
        assert_eq!(
            rows_asked[0],
            ["123", "12345678", "1234", "7", "-5", "1.5", "123456"]
        );
        assert_eq!(st.padded.len(), 7);
        assert_eq!((st.padded_applied, st.padded_left), (0, 3));
        let (yes, st) = ods_read(&data, Some(true));
        assert_eq!(
            yes[0],
            [
                "00123",
                "012-34-5678",
                "01 234",
                "00007",
                "-5",
                "1.5",
                "123456"
            ]
        );
        assert_eq!((st.padded_applied, st.padded_left), (4, 3));
        assert_eq!(st.kept_columns, [0, 1, 2, 3]);
        let (no, st) = ods_read(&data, Some(false));
        assert_eq!(no[0][0], "123");
        assert_eq!(st.padded_applied, 0);
    }

    #[test]
    fn ods_a_pattern_that_disagrees_with_the_shown_text_does_not_pad() {
        let styles = number_style("N5", &digits(5));
        let auto = cell_style("zip", "N5");
        let table = format!(
            "<table:table-row>{}{}</table:table-row>",
            float("zip", "123", "000123"),
            float("zip", "45", "00045"),
        );
        let (rows, st) = ods_read(&ods_with(&styles, &auto, &table), Some(true));
        assert_eq!(rows[0], ["123", "00045"]);
        assert_eq!((st.padded_applied, st.padded_left), (1, 1));
    }

    #[test]
    fn ods_default_styles_go_cell_then_row_then_column_then_family() {
        let styles = number_style("N3", &digits(3))
            + &number_style("N4", &digits(4))
            + &number_style("N6", &digits(6))
            + r#"<style:default-style style:family="table-cell" style:data-style-name="N6"/>"#;
        let auto = cell_style("c3", "N3")
            + &cell_style("r4", "N4")
            + r#"<style:style style:name="plain" style:family="table-cell" style:data-style-name="none"/>"#;
        let table = format!(
            r#"<table:table-column table:default-cell-style-name="c3" table:number-columns-repeated="2"/><table:table-column table:default-cell-style-name="plain"/><table:table-column/><table:table-row>{}{}{}{}</table:table-row><table:table-row table:default-cell-style-name="r4">{}{}{}</table:table-row>"#,
            // Column default, the cell's own style over it, a column default that does not pad, the family's.
            float("", "1", "001"),
            float("r4", "1", "0001"),
            float("", "1", "1"),
            float("", "1", "000001"),
            // The row's default wins over the columns'.
            float("", "2", "0002"),
            float("c3", "2", "002"),
            float("", "2", "0002"),
        );
        let (rows, _) = ods_read(&ods_with(&styles, &auto, &table), Some(true));
        assert_eq!(rows[0], ["001", "0001", "1", "000001"]);
        assert_eq!(rows[1][..3], ["0002", "002", "0002"]);
    }

    #[test]
    fn ods_repeats_are_walked_as_runs() {
        let styles = number_style("N3", &digits(3));
        let auto = cell_style("c3", "N3");
        // A huge run of columns and of empty cells; one repeated value spanning two column defaults.
        let table = format!(
            r#"<table:table-column table:number-columns-repeated="2"/><table:table-column table:default-cell-style-name="c3" table:number-columns-repeated="1000000"/><table:table-row><table:table-cell table:number-columns-repeated="1"/><table:table-cell table:number-columns-repeated="3" office:value-type="float" office:value="7"><text:p>007</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1000000"/></table:table-row><table:table-row table:number-rows-repeated="3">{}</table:table-row>"#,
            float("c3", "5", "005"),
        );
        let started = std::time::Instant::now();
        let (rows, st) = ods_read(&ods_with(&styles, &auto, &table), Some(true));
        assert!(started.elapsed().as_secs() < 5);
        // The shown text is "007" for all three, so only the padded columns take it.
        assert_eq!(rows[0][1..4], ["7", "007", "007"]);
        assert_eq!(rows[1][0], "005");
        assert_eq!(rows.len(), 4);
        assert_eq!(st.padded_applied, 5);
    }

    #[test]
    fn ods_other_number_formats_never_pad() {
        let bad = [
            r#"<number:number number:decimal-places="2" number:min-integer-digits="5"/>"#.to_string(),
            r#"<number:number number:min-integer-digits="5"/>"#.to_string(),
            r#"<number:number number:decimal-places="0" number:min-integer-digits="5" number:grouping="true"/>"#.to_string(),
            r#"<number:number number:decimal-places="0" number:min-integer-digits="1"/>"#.to_string(),
            format!("<number:text>ID </number:text>{}", digits(5)),
            format!("{}<number:text>%</number:text>", digits(5)),
            format!("<number:currency-symbol>$</number:currency-symbol>{}", digits(5)),
            r#"<number:scientific-number number:decimal-places="0" number:min-integer-digits="5"/>"#.to_string(),
        ];
        for (k, inner) in bad.iter().enumerate() {
            let data = ods_with(
                &number_style("N", inner),
                &cell_style("c", "N"),
                &format!(
                    "<table:table-row>{}</table:table-row>",
                    float("c", "12", "00012")
                ),
            );
            let (rows, st) = ods_read(&data, Some(true));
            assert_eq!(rows[0], ["12"], "format {k}");
            assert!(st.padded.is_empty(), "format {k}");
        }
        // A parent cycle ends; damaged styles never fail the import.
        let cycle = r#"<style:style style:name="c" style:family="table-cell" style:parent-style-name="d"/><style:style style:name="d" style:family="table-cell" style:parent-style-name="c"/>"#;
        let table = format!(
            "<table:table-row>{}</table:table-row>",
            float("c", "12", "12")
        );
        assert_eq!(
            ods_read(&ods_with("", cycle, &table), Some(true)).0[0],
            ["12"]
        );
        let broken = ods_with(
            "<number:number-style style:name=\"N\"><number:number <<",
            "",
            &table,
        );
        assert_eq!(ods_read(&broken, Some(true)).0[0], ["12"]);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(300))]
        /// Styles with characters knocked out or swapped never panic the reader.
        #[test]
        fn damaged_ods_styles_never_panic(cuts in proptest::collection::vec((0usize..900, 0usize..6), 1..8)) {
            let mut styles = number_style("N5", &digits(5))
                + r#"<number:number-style style:name="NS"><number:number number:decimal-places="0" number:min-integer-digits="9"><number:embedded-text number:position="4">-</number:embedded-text></number:number></number:number-style>"#
                + &cell_style("c", "N5")
                + &cell_style("d", "NS");
            for (at, with) in cuts {
                let mut at = at.min(styles.len().saturating_sub(1));
                while !styles.is_char_boundary(at) {
                    at -= 1;
                }
                if at < styles.len() {
                    styles.remove(at);
                    let add = ["", "<", ">", "\"", "/", "99999999999999999999"][with];
                    styles.insert_str(at, add);
                }
            }
            let table = format!("<table:table-row>{}{}</table:table-row>", float("c", "12", "00012"), float("d", "12", "000-00-0012"));
            if let Ok(b) = Book::open(&ods_with(&styles, "", &table)) {
                let opts = SheetOptions { raw_dates: false, pad_zeros: Some(true) };
                let _ = b.read(0, opts, None, &Progress::new());
            }
        }
    }

    /// A sheet saved by LibreOffice 24.2 itself (made-up values): its padding styles sit on the column defaults.
    #[test]
    fn a_file_saved_by_libreoffice_pads_as_it_shows() {
        let data: &[u8] = include_bytes!("testdata/libreoffice_zeros.ods");
        let stored = [
            ["123", "123456789", "1234", "5", "-12", "1.5", "3"],
            ["7", "12", "99", "42", "-1", "2.25", "10"],
        ];
        for ask in [None, Some(false)] {
            let (rows, st) = ods_read(data, ask);
            assert_eq!(rows[1..], stored, "{ask:?}");
            assert_eq!(st.padded_applied, 0);
            // Ten cells carry a padding format: six can be padded, the two negatives and two fractions cannot.
            assert_eq!(st.padded.iter().map(|p| p.1).sum::<u64>(), 10, "{ask:?}");
            assert_eq!(st.padded_left, 4, "{ask:?}");
        }
        let (rows, st) = ods_read(data, Some(true));
        assert_eq!(
            rows[1],
            ["00123", "123-45-6789", "01 234", "5", "-12", "1.5", "3"]
        );
        assert_eq!(
            rows[2],
            ["00007", "000-00-0012", "00 099", "42", "-1", "2.25", "10"]
        );
        assert_eq!((st.padded_applied, st.padded_left), (6, 4));
    }

    #[test]
    fn xlsx_quoted_literals_pad_only_dashes_and_spaces() {
        assert_eq!(
            pad_by_code("00\"-\"000").and_then(|p| p.apply(1234)),
            Some("01-234".into())
        );
        assert_eq!(
            pad_by_code("000\" \"00").and_then(|p| p.apply(7)),
            Some("000 07".into())
        );
        for other in ["00\"x\"0", "000\"ID\"", "00\"-", "\"Zip \"00000"] {
            assert!(pad_by_code(other).is_none(), "{other}");
        }
    }

    #[test]
    fn reads_ods_values() {
        let content = r#"<office:document-content xmlns:office="o" xmlns:table="t" xmlns:text="x"><office:body><office:spreadsheet>
<table:table table:name="First">
<table:table-column table:number-columns-repeated="3"/>
<table:table-header-rows><table:table-row><table:table-cell office:value-type="string"><text:p>Name</text:p></table:table-cell><table:table-cell><text:p>Two<text:s text:c="2"/>words</text:p><text:p>line</text:p></table:table-cell></table:table-row></table:table-header-rows>
<table:table-row table:number-rows-repeated="2"><table:table-cell office:value-type="float" office:value="1.25"><text:p>1.3</text:p></table:table-cell><table:table-cell table:number-columns-repeated="2" office:value-type="boolean" office:boolean-value="true"><text:p>TRUE</text:p></table:table-cell></table:table-row>
<table:table-row table:number-rows-repeated="1048000"><table:table-cell table:number-columns-repeated="16384"/></table:table-row>
<table:table-row><table:table-cell office:value-type="date" office:date-value="2024-01-05T13:00:00"/><table:table-cell office:value-type="time" office:time-value="PT01H05M30S"/><table:table-cell><office:annotation><text:p>note</text:p></office:annotation><text:p>a &amp; b</text:p></table:table-cell></table:table-row>
<table:table-row table:number-rows-repeated="1048000"><table:table-cell table:number-columns-repeated="16384"/></table:table-row>
</table:table>
<table:table table:name="Second"><table:table-row><table:table-cell><text:p>x</text:p></table:table-cell></table:table-row></table:table>
</office:spreadsheet></office:body></office:document-content>"#;
        let data = ods(content);
        let b = Book::open(&data).unwrap();
        let names: Vec<_> = b.sheets().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["First", "Second"]);
        let t = b
            .read(1, SheetOptions::default(), None, &Progress::new())
            .unwrap();
        assert_eq!(rows(&t), vec![vec!["x"]]);
        // Blank rows between data are kept; the trailing million are not.
        let t = b
            .read(0, SheetOptions::default(), None, &Progress::new())
            .unwrap();
        assert_eq!(t.rows, 1 + 2 + 1_048_000 + 1);
        let small = content.replace("1048000", "3");
        let data = ods(&small);
        let b = Book::open(&data).unwrap();
        let t = b
            .read(0, SheetOptions::default(), None, &Progress::new())
            .unwrap();
        assert_eq!(
            rows(&t),
            vec![
                vec!["Name", "Two  words\nline"],
                vec!["1.25", "TRUE", "TRUE"],
                vec!["1.25", "TRUE", "TRUE"],
                vec![""],
                vec![""],
                vec![""],
                vec!["2024-01-05 13:00:00", "01:05:30", "a & b"],
            ]
        );
    }

    #[test]
    fn dates_and_numbers() {
        assert_eq!(
            serial_to_text(1.0, Shown::Date, false).unwrap(),
            "1900-01-01"
        );
        assert_eq!(
            serial_to_text(59.0, Shown::Date, false).unwrap(),
            "1900-02-28"
        );
        assert_eq!(
            serial_to_text(61.0, Shown::Date, false).unwrap(),
            "1900-03-01"
        );
        assert_eq!(
            serial_to_text(45306.0, Shown::Date, false).unwrap(),
            "2024-01-15"
        );
        assert_eq!(
            serial_to_text(0.0, Shown::Date, true).unwrap(),
            "1904-01-01"
        );
        assert_eq!(
            serial_to_text(0.999999, Shown::Time, false).unwrap(),
            "00:00:00"
        );
        assert!(serial_to_text(-1.0, Shown::Date, false).is_none());
        assert!(serial_to_text(f64::NAN, Shown::Date, false).is_none());
        assert_eq!(shown_by_code("yyyy-mm-dd"), Shown::Date);
        assert_eq!(shown_by_code("mmm"), Shown::Date);
        assert_eq!(shown_by_code("h:mm AM/PM"), Shown::Time);
        assert_eq!(shown_by_code("[h]:mm"), Shown::Number);
        assert_eq!(shown_by_code("0.00\"days\""), Shown::Number);
        assert_eq!(shown_by_code("#,##0.00;[Red]-#,##0.00"), Shown::Number);
        assert_eq!(
            short_number(
                "0.30000000000000004",
                0.30000000000000004,
                0,
                &mut SheetStats::default()
            ),
            "0.3"
        );
        assert_eq!(
            short_number("123", 123.0, 0, &mut SheetStats::default()),
            "123"
        );
        assert_eq!(
            short_number(
                "1.2345678901234567E+30",
                1.2345678901234567e30,
                0,
                &mut SheetStats::default()
            ),
            "1.23456789012346E+30"
        );
        assert_eq!(cell_col("A1"), Some(0));
        assert_eq!(cell_col("AB12"), Some(27));
        assert_eq!(cell_col("12"), None);
        assert_eq!(ods_time("PT1H5M3.5S"), "01:05:03.5");
    }
}
