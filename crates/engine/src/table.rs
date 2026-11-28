//! Building imported tables, and small XML helpers shared by the importers.

use std::borrow::Cow;
use std::io::{BufRead, BufReader, Read};

use quick_xml::events::{BytesRef, BytesStart};
use quick_xml::{Reader, XmlVersion};

use crate::reshape::{push_line, too_big, Table, MAX_TABLE_BYTES, MAX_TABLE_CELLS};
use crate::zipread::{damaged, io_err_ref};
use crate::Error;

/// Longest single cell an importer accepts (characters as UTF-8 bytes).
pub const MAX_CELL: usize = 32 << 20;
/// Deepest XML nesting read.
pub const MAX_DEPTH: usize = 256;

/// Collects rows into a new table, within the size caps. Empty rows are
/// held back and only written when a row with data follows, so a sheet's
/// trailing blank area (often "repeated a million times") costs nothing.
pub struct Sink {
    out: Vec<u8>,
    rows: u64,
    cells: u64,
    cols: u32,
    pending: u64,
    limit: Option<u64>,
    /// Row heights the source gives (spreadsheets): (first row, rows, points),
    /// rows numbered as this table writes them.
    heights: Vec<(u64, u64, f32)>,
    /// Ranges noted (a long sheet of single rows stops being noted well past the cap).
    height_ranges: u64,
}

/// Most rows a source's own row heights are kept for (the grid's cap).
pub const MAX_ROW_HEIGHTS: u64 = 100_000;

impl Sink {
    /// `limit`: stop taking rows after this many (previews).
    pub fn new(limit: Option<u64>) -> Sink {
        Sink {
            out: Vec::new(),
            rows: 0,
            cells: 0,
            cols: 0,
            pending: 0,
            limit,
            heights: Vec::new(),
            height_ranges: 0,
        }
    }

    /// The next `n` rows (the row about to be written, and copies of it)
    /// have their own height in the source, `pt` points. Called by a reader
    /// in its own row loop, so a height always lands on the row it belongs
    /// to; rows that turn out to be the empty tail are left out at the end.
    pub fn row_height(&mut self, pt: f32, n: u64) {
        self.row_height_after(0, pt, n);
    }

    /// [`Sink::row_height`] for rows `skip` rows further on (a row with no
    /// cells whose number the source gives: the rows before it are written
    /// as empty rows when the next row with data comes).
    pub fn row_height_after(&mut self, skip: u64, pt: f32, n: u64) {
        if n == 0 || !pt.is_finite() || pt <= 0.0 {
            return;
        }
        let at = self.rows.saturating_add(self.pending).saturating_add(skip);
        if at >= MAX_TABLE_CELLS {
            return;
        }
        if let Some(last) = self.heights.last_mut() {
            if last.0 + last.1 == at && last.2 == pt {
                last.1 = last.1.saturating_add(n);
                return;
            }
        }
        // More ranges than the cap could ever use are not kept (each range is
        // at least one row; some may fall in the empty tail, hence the room).
        if self.height_ranges >= MAX_ROW_HEIGHTS * 2 {
            return;
        }
        self.height_ranges += 1;
        self.heights.push((at, n, pt));
    }

    /// The row heights within the rows written (the held-back empty tail
    /// left out), one entry per row in hundredths of a point, at most
    /// [`MAX_ROW_HEIGHTS`]; and how
    /// many rows with a height were left out past that.
    pub fn heights(&self) -> (Vec<(u64, u32)>, u64) {
        let mut out = Vec::new();
        let mut over = 0u64;
        for &(at, n, pt) in &self.heights {
            let end = at.saturating_add(n).min(self.rows);
            if at >= end {
                continue;
            }
            let room = MAX_ROW_HEIGHTS - out.len() as u64;
            let take = (end - at).min(room);
            let cpt = (pt * 100.0).round() as u32;
            out.extend((at..at + take).map(|r| (r, cpt)));
            over += end - at - take;
        }
        (out, over)
    }

    /// Make room for `n` more bytes up front (at most the size cap), so a
    /// table whose size is known does not grow by doubling.
    pub fn reserve(&mut self, n: usize) {
        self.out.reserve(n.min(self.room()));
    }

    /// This table is a preview: it takes a limited number of rows.
    pub fn is_preview(&self) -> bool {
        self.limit.is_some()
    }

    /// The preview limit is reached; readers may stop.
    pub fn full(&self) -> bool {
        self.limit.is_some_and(|l| self.rows >= l)
    }

    pub fn rows(&self) -> u64 {
        self.rows + self.pending
    }

    /// Bytes of text the table can still take. Readers check a row against
    /// this while building it, before copying values (a short file can name
    /// one long value thousands of times).
    pub fn room(&self) -> usize {
        MAX_TABLE_BYTES.saturating_sub(self.out.len())
    }

    /// How many of `n` more rows the table will take (a preview stops at
    /// its limit).
    pub fn take(&self, n: u64) -> u64 {
        match self.limit {
            Some(l) => n.min(l.saturating_sub(self.rows)),
            None => n,
        }
    }

    /// `n` empty rows.
    pub fn empty_rows(&mut self, n: u64) {
        self.pending = self.pending.saturating_add(n);
    }

    /// One empty row that is data: written now, not held back as padding.
    pub fn blank_row(&mut self) -> Result<(), Error> {
        if self.full() {
            return Ok(());
        }
        let n = self.pending.saturating_add(1);
        if self.cells.saturating_add(n) > MAX_TABLE_CELLS
            || self.out.len() as u64 + 2 * n > MAX_TABLE_BYTES as u64
        {
            return Err(too_big());
        }
        let n = self.take(n);
        for _ in 0..n {
            self.out.extend_from_slice(b"\r\n");
        }
        self.rows += n;
        self.cells += n;
        self.pending = 0;
        Ok(())
    }

    /// One row; trailing empty cells are dropped.
    pub fn row<S: AsRef<str>>(&mut self, cells: &[S]) -> Result<(), Error> {
        let n = cells
            .iter()
            .rposition(|c| !c.as_ref().is_empty())
            .map_or(0, |i| i + 1);
        if n == 0 {
            self.pending = self.pending.saturating_add(1);
            return Ok(());
        }
        if self.full() {
            return Ok(());
        }
        if self.pending > 0 {
            let mut p = self.pending;
            if let Some(l) = self.limit {
                p = p.min(l - self.rows);
            }
            if self.cells.saturating_add(p) > MAX_TABLE_CELLS {
                return Err(too_big());
            }
            if self.out.len() as u64 + 2 * p > MAX_TABLE_BYTES as u64 {
                return Err(too_big());
            }
            for _ in 0..p {
                self.out.extend_from_slice(b"\r\n");
            }
            self.rows += p;
            self.cells += p;
            self.pending = 0;
            if self.full() {
                return Ok(());
            }
        }
        if self.cells + n as u64 > MAX_TABLE_CELLS {
            return Err(too_big());
        }
        push_line(&mut self.out, &cells[..n]);
        if self.out.len() > MAX_TABLE_BYTES {
            return Err(too_big());
        }
        self.rows += 1;
        self.cells += n as u64;
        self.cols = self.cols.max(n as u32);
        Ok(())
    }

    /// Rows already written as lines (`rows` lines holding `cells` values,
    /// the widest `cols`), within the size caps. Not for previews.
    pub fn lines(&mut self, bytes: &[u8], rows: u64, cells: u64, cols: u32) -> Result<(), Error> {
        debug_assert!(self.limit.is_none());
        if rows == 0 {
            return Ok(());
        }
        let p = self.pending;
        let cells_after = self.cells.saturating_add(p).saturating_add(cells);
        if cells_after > MAX_TABLE_CELLS
            || self.out.len() as u64 + 2 * p + bytes.len() as u64 > MAX_TABLE_BYTES as u64
        {
            return Err(too_big());
        }
        for _ in 0..p {
            self.out.extend_from_slice(b"\r\n");
        }
        self.out.extend_from_slice(bytes);
        self.pending = 0;
        self.rows += p + rows;
        self.cells = cells_after;
        self.cols = self.cols.max(cols);
        Ok(())
    }

    pub fn finish(self) -> Table {
        Table {
            bytes: self.out,
            rows: self.rows,
            cols: self.cols,
        }
    }
}

/// An XML reader over untrusted input: no entity expansion (quick-xml never
/// expands DTD entities), lenient end tags, text kept as is.
pub fn xml_reader<R: Read>(r: R) -> Reader<BufReader<R>> {
    let mut x = Reader::from_reader(BufReader::with_capacity(1 << 16, r));
    let c = x.config_mut();
    c.check_end_names = false;
    c.allow_unmatched_ends = true;
    c.allow_dangling_amp = true;
    x
}

/// An XML reader over text already in memory.
pub fn xml_reader_str(s: &str) -> Reader<&[u8]> {
    let mut x = Reader::from_reader(s.as_bytes());
    let c = x.config_mut();
    c.check_end_names = false;
    c.allow_unmatched_ends = true;
    c.allow_dangling_amp = true;
    x
}

pub fn xml_err(e: quick_xml::Error) -> Error {
    match e {
        quick_xml::Error::Io(io) => io_err_ref(&io),
        e => damaged(&e.to_string()),
    }
}

/// An attribute's value by local name (prefixes ignored).
pub fn attr(e: &BytesStart, local: &str) -> Option<String> {
    let mut it = e.attributes();
    it.with_checks(false);
    for a in it.flatten() {
        if a.key.local_name().as_ref() == local {
            return Some(match a.normalized_value(XmlVersion::Implicit1_0) {
                Ok(v) => v.into_owned(),
                Err(_) => a.value.clone().into_owned(),
            });
        }
    }
    None
}

/// Every attribute as (local name, value).
pub fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    let mut it = e.attributes();
    it.with_checks(false);
    it.flatten()
        .map(|a| {
            let k = a.key.local_name().as_ref().to_string();
            let v = match a.normalized_value(XmlVersion::Implicit1_0) {
                Ok(v) => v.into_owned(),
                Err(_) => a.value.clone().into_owned(),
            };
            (k, v)
        })
        .collect()
}

pub fn local(e: &BytesStart) -> String {
    e.local_name().as_ref().to_string()
}

/// An entity or character reference as text. Only the five predefined
/// entities and character references are known; anything else stays as
/// written (`&name;`), never expanded.
pub fn push_ref(r: &BytesRef, out: &mut String) {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        out.push(c);
        return;
    }
    let name: &str = r;
    match name {
        "amp" => out.push('&'),
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "quot" => out.push('"'),
        "apos" => out.push('\''),
        _ => {
            out.push('&');
            out.push_str(name);
            out.push(';');
        }
    }
}

/// Add text to a cell, refusing cells over [`MAX_CELL`].
pub fn push_text(out: &mut String, s: &str) -> Result<(), Error> {
    if out.len() + s.len() > MAX_CELL {
        return Err(Error::Invalid(
            "a cell holds more than 32 MB of text; not imported".into(),
        ));
    }
    out.push_str(s);
    Ok(())
}

/// The text of an XML text event (line breaks normalised).
pub fn text_of<'a>(t: &'a quick_xml::events::BytesText<'_>) -> Cow<'a, str> {
    t.xml10_content()
}

/// A buffered reader's position for progress.
pub fn position<R: BufRead>(r: &Reader<R>) -> u64 {
    r.buffer_position()
}
