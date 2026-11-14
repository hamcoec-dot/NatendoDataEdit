//! Spreadsheet files (`.xlsx`, `.ods`) read into tables.
//!
//! Only cell values are read (formulas give their last calculated value;
//! styles, comments and pictures are left out). Everything is read through
//! [`crate::zipread`] (capped, never extracted) and a streaming XML reader
//! (no entity expansion, nesting capped), so a hostile file costs bounded
//! time and memory.

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
        })
    }

    pub fn sheets(&self) -> &[SheetInfo] {
        &self.sheets
    }

    /// Read sheet `index` (at most `limit` rows when given).
    pub fn read(
        &self,
        index: usize,
        opts: SheetOptions,
        limit: Option<u64>,
        progress: &Progress,
    ) -> Result<Table, Error> {
        if index >= self.sheets.len() {
            return Err(Error::Invalid("that sheet is not in the file".into()));
        }
        let mut sink = Sink::new(limit);
        match self.kind {
            Kind::Xlsx => self.read_xlsx(index, opts, &mut sink, progress)?,
            Kind::Ods => self.read_ods(index, &mut sink, progress)?,
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
    fn styles(&self, rels: &HashMap<String, (String, String)>) -> Result<Vec<Shown>, Error> {
        let Some((_, part)) = rels.values().find(|(t, _)| t.ends_with("/styles")) else {
            return Ok(Vec::new());
        };
        if !self.zip.has(part) {
            return Ok(Vec::new());
        }
        let text = self.zip.read_string(part, MAX_SMALL_PART)?;
        let mut r = crate::table::xml_reader_str(&text);
        let mut custom: HashMap<u32, Shown> = HashMap::new();
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
                            custom.insert(id, shown_by_code(&code));
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
        Ok(xfs)
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
        let styles = if opts.raw_dates {
            Vec::new()
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
                        Rc::from(number_text(&v, shown, self.date1904))
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

    fn read_ods(&self, index: usize, sink: &mut Sink, progress: &Progress) -> Result<(), Error> {
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
                            "table-row" => {
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
                                cell = OdsCell {
                                    active: true,
                                    repeat: repeat(e, "number-columns-repeated"),
                                    value: ods_value(e),
                                    depth,
                                    ..Default::default()
                                };
                                if empty {
                                    cell.finish(
                                        &mut row,
                                        &mut col,
                                        &mut row_bytes,
                                        row_room(sink, row_repeat),
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

/// A typed ODS value from the cell's attributes (`None`: use its text).
fn ods_value(e: &quick_xml::events::BytesStart) -> Option<String> {
    let ty = attr(e, "value-type")?;
    match ty.as_str() {
        "float" | "percentage" | "currency" => attr(e, "value"),
        "date" => attr(e, "date-value").map(|v| v.replacen('T', " ", 1)),
        "time" => attr(e, "time-value").map(|v| ods_time(&v)),
        "boolean" => attr(e, "boolean-value").map(|v| {
            if v == "true" {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }),
        "string" => attr(e, "string-value"),
        _ => None,
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

#[derive(Default)]
struct OdsCell {
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
    ) -> Result<(), Error> {
        self.active = false;
        let value = self
            .value
            .take()
            .unwrap_or_else(|| std::mem::take(&mut self.text));
        let n = self.repeat.max(1);
        if !value.is_empty() {
            if u64::from(*col) + n > u64::from(MAX_COLS) {
                return Err(Error::Invalid(
                    "the sheet is wider than a spreadsheet can be".into(),
                ));
            }
            let need = (value.len() as u64 + 1).saturating_mul(n);
            if (*row_bytes as u64).saturating_add(need) > room as u64 {
                return Err(too_big());
            }
            *row_bytes += need as usize;
            // One copy of the text, shared by every repeat.
            let value: Rc<str> = Rc::from(value);
            for i in 0..n {
                row.push((*col + i as u32, Rc::clone(&value)));
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
fn number_text(v: &str, shown: Option<Shown>, date1904: bool) -> String {
    let t = v.trim();
    let Ok(x) = t.parse::<f64>() else {
        return v.to_string();
    };
    match shown {
        Some(s @ (Shown::Date | Shown::Time | Shown::DateTime)) => {
            serial_to_text(x, s, date1904).unwrap_or_else(|| short_number(t, x))
        }
        _ => short_number(t, x),
    }
}

/// Keep the stored text unless it has more than 15 significant digits.
fn short_number(t: &str, x: f64) -> String {
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
            .read(0, SheetOptions { raw_dates: true }, None, &Progress::new())
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
            short_number("0.30000000000000004", 0.30000000000000004),
            "0.3"
        );
        assert_eq!(short_number("123", 123.0), "123");
        assert_eq!(
            short_number("1.2345678901234567E+30", 1.2345678901234567e30),
            "1.23456789012346E+30"
        );
        assert_eq!(cell_col("A1"), Some(0));
        assert_eq!(cell_col("AB12"), Some(27));
        assert_eq!(cell_col("12"), None);
        assert_eq!(ods_time("PT1H5M3.5S"), "01:05:03.5");
    }
}
