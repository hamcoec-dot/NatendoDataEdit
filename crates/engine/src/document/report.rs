//! Report notes (C4): lines above and below the table that are not rows of
//! it, such as a report's title, its date and a total line. The lines above
//! the table when the file was opened are kept as they are; rows can be set
//! aside later ("Headers start here", "Set aside as footer notes") and put
//! back. Saving writes them where they were: the lines above, the table, the
//! lines below (whatever the sort). One switch leaves them all out.
//!
//! Set-aside rows keep their ids and overlay states while the history can
//! reach them, so putting them back (or undoing) is exact; an unedited row is
//! held as its file bytes, an edited one written in the file's own format.

use super::*;

/// Most lines above, or below, the table that rows can be set aside into.
pub const MAX_NOTE_LINES: usize = 1_000;
/// Most bytes above, or below, the table that rows can be set aside into.
pub const MAX_NOTE_BYTES: usize = 1 << 20;

/// Which end of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    /// Above the table (the first rows of the file).
    Head,
    /// Below the table (the last rows of the file).
    Foot,
}

/// Where a block's text is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NoteText {
    /// The lines above the table when the file was opened (in the source).
    Preamble,
    /// The lines below the table after a save (in the source).
    Tail,
    /// Set-aside rows, in the working encoding, each ending in its break.
    Lines(Vec<u8>),
}

/// Some lines of report notes.
#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub text: NoteText,
    /// The rows they were made from (ids and overlay states), so putting
    /// them back is exact. `None` for lines read from the file.
    pub rows: Option<Vec<(u32, Option<Edit>)>>,
}

/// The report notes of a document (one undo step replaces them whole).
#[derive(Debug, Clone)]
pub(crate) struct Notes {
    /// In file order.
    pub head: Vec<Block>,
    /// In file order.
    pub foot: Vec<Block>,
    /// Saving writes them (on by default).
    pub keep: bool,
}

impl Default for Notes {
    fn default() -> Notes {
        Notes {
            head: Vec::new(),
            foot: Vec::new(),
            keep: true,
        }
    }
}

impl Notes {
    /// Notes of a freshly opened (or saved) source.
    pub(crate) fn of_source(preamble_len: usize, tail_len: usize) -> Notes {
        let mut n = Notes::default();
        if preamble_len > 0 {
            n.head.push(Block {
                text: NoteText::Preamble,
                rows: None,
            });
        }
        if tail_len > 0 {
            n.foot.push(Block {
                text: NoteText::Tail,
                rows: None,
            });
        }
        n
    }

    /// Approximate bytes held (for the undo memory budget).
    pub(crate) fn cost(&self) -> usize {
        self.head
            .iter()
            .chain(&self.foot)
            .map(|b| {
                64 + match &b.text {
                    NoteText::Lines(l) => l.len(),
                    _ => 0,
                } + b.rows.as_ref().map_or(0, |r| {
                    r.iter()
                        .map(|(_, e)| 32 + e.as_ref().map_or(0, Edit::cost))
                        .sum()
                })
            })
            .sum()
    }
}

/// What the Report notes panel shows.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct ReportNotes {
    /// Lines above the table as they were in the file.
    pub above_file: String,
    pub above_file_lines: u32,
    /// Lines set aside above the table since.
    pub above_set: String,
    pub above_set_lines: u32,
    /// Lines below the table as they were in the file (after a save).
    pub below_file: String,
    pub below_file_lines: u32,
    /// Lines set aside below the table since.
    pub below_set: String,
    pub below_set_lines: u32,
    /// Saving writes them.
    pub keep: bool,
    /// `Label: value` and two-field lines (at most 50).
    pub info: Vec<(String, String)>,
    /// Text cut to this many characters per part for showing.
    pub cut: bool,
}

/// How many lines of report notes there are (cheap: nothing is copied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct NoteCounts {
    /// Lines above the table that were in the file.
    pub above_file: u32,
    /// Lines set aside above the table since.
    pub above_set: u32,
    pub below_file: u32,
    pub below_set: u32,
    /// Saving writes them.
    pub keep: bool,
}

/// Lines in `b`: its breaks, and a last line without one.
pub(crate) fn lines_in(b: &[u8]) -> usize {
    scan::line_breaks_in(b) + usize::from(!b.is_empty() && LineBreak::at_end_of(b).is_none())
}

/// Characters of each part the panel shows.
const SHOW_CHARS: usize = 200_000;

impl Document {
    /// The text of one block, in the working encoding.
    pub(crate) fn block_bytes<'a>(&'a self, b: &'a Block) -> &'a [u8] {
        match &b.text {
            NoteText::Preamble => self.preamble(),
            NoteText::Tail => &self.source.bytes()[self.data_end..],
            NoteText::Lines(l) => l,
        }
    }

    /// Everything above (`Head`) or below the table, in the working encoding,
    /// as a save writes it (an empty `Vec` when the switch is off unless
    /// `all`). A break goes between blocks where one lacks it.
    pub(crate) fn note_bytes(&self, place: Place, all: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.notes.keep && !all {
            return out;
        }
        let blocks = match place {
            Place::Head => &self.notes.head,
            Place::Foot => &self.notes.foot,
        };
        for b in blocks {
            if !out.is_empty() && LineBreak::at_end_of(&out).is_none() {
                out.extend_from_slice(self.format.line_break.bytes());
            }
            out.extend_from_slice(self.block_bytes(b));
        }
        out
    }

    /// Whether there are report notes (lines above or below the table).
    pub fn has_report_notes(&self) -> bool {
        !self.notes.head.is_empty() || !self.notes.foot.is_empty()
    }

    /// How many lines of report notes there are, by kind.
    pub fn note_counts(&self) -> NoteCounts {
        let mut c = NoteCounts {
            keep: self.notes.keep,
            ..Default::default()
        };
        for b in &self.notes.head {
            let n = lines_in(self.block_bytes(b)) as u32;
            match b.rows {
                None => c.above_file += n,
                Some(_) => c.above_set += n,
            }
        }
        for b in &self.notes.foot {
            let n = lines_in(self.block_bytes(b)) as u32;
            match b.rows {
                None => c.below_file += n,
                Some(_) => c.below_set += n,
            }
        }
        c
    }

    /// Whether a save writes them.
    pub fn keeps_report_notes(&self) -> bool {
        self.notes.keep
    }

    /// Lines above the table, and below it, that a save writes (or would
    /// write with the switch on, `all`).
    pub fn note_lines(&self, all: bool) -> (u32, u32) {
        (
            lines_in(&self.note_bytes(Place::Head, all)) as u32,
            lines_in(&self.note_bytes(Place::Foot, all)) as u32,
        )
    }

    /// How a recovery copy ([`Document::write_copy`], which writes every
    /// note) is read back: the lines to skip above the table and the bytes
    /// below it (see [`OpenOptions::tail_bytes`]).
    pub fn copy_layout(&self) -> (u32, usize) {
        let head = self.note_bytes(Place::Head, true);
        let foot = self.note_bytes(Place::Foot, true);
        (lines_in(&head) as u32, foot.len())
    }

    /// The panel's view of the notes.
    pub fn report_notes(&self) -> ReportNotes {
        let mut r = ReportNotes {
            keep: self.notes.keep,
            ..Default::default()
        };
        let enc = self.working_encoding();
        let put = |b: &Block, text: &mut String, lines: &mut u32, cut: &mut bool| {
            let bytes = self.block_bytes(b);
            *lines += lines_in(bytes) as u32;
            if text.len() < SHOW_CHARS {
                let t = encoding::decode_direct(bytes, enc);
                let room = SHOW_CHARS - text.len();
                if t.len() > room {
                    let mut end = room;
                    while !t.is_char_boundary(end) {
                        end -= 1;
                    }
                    text.push_str(&t[..end]);
                    *cut = true;
                } else {
                    text.push_str(&t);
                }
            } else {
                *cut = true;
            }
        };
        let mut cut = false;
        for b in &self.notes.head {
            match b.rows {
                None => put(b, &mut r.above_file, &mut r.above_file_lines, &mut cut),
                Some(_) => put(b, &mut r.above_set, &mut r.above_set_lines, &mut cut),
            }
        }
        for b in &self.notes.foot {
            match b.rows {
                None => put(b, &mut r.below_file, &mut r.below_file_lines, &mut cut),
                Some(_) => put(b, &mut r.below_set, &mut r.below_set_lines, &mut cut),
            }
        }
        r.cut = cut;
        let all = [&r.above_file, &r.above_set, &r.below_set, &r.below_file];
        'lines: for text in all {
            for line in text.lines() {
                if r.info.len() >= 50 {
                    break 'lines;
                }
                if let Some(pair) = info_of(line, self.format.dialect) {
                    r.info.push(pair);
                }
            }
        }
        r
    }

    /// Why rows `at..at + count` (shown rows) cannot be set aside at `place`,
    /// or `Ok` when they can.
    pub fn set_aside_check(&self, at: u64, count: u64, place: Place) -> Result<(), Error> {
        self.ready()?;
        if self.view.is_some() {
            return Err(Error::Invalid(
                "show all rows first: some rows are hidden or filtered out, so the first and last rows of the file may not be on screen".into(),
            ));
        }
        let total = self.total_rows();
        let end = at.checked_add(count).ok_or(Error::OutOfRange)?;
        if count == 0 || end > total {
            return Err(Error::OutOfRange);
        }
        if count >= total {
            return Err(Error::Invalid(
                "at least one row has to stay in the table".into(),
            ));
        }
        let what = match place {
            Place::Head => "first",
            Place::Foot => "last",
        };
        let edge = match place {
            Place::Head => at == 0,
            Place::Foot => end == total,
        };
        if !edge {
            return Err(Error::Invalid(format!(
                "only the {what} rows of the table can be set aside {}: select rows {}",
                match place {
                    Place::Head => "above it",
                    Place::Foot => "below it",
                },
                span(
                    match place {
                        Place::Head => 0,
                        Place::Foot => total - count,
                    },
                    count
                )
            )));
        }
        if count as usize > MAX_NOTE_LINES {
            return Err(too_many(count as usize));
        }
        self.in_file_order(at, count, place)?;
        let side = match place {
            Place::Head => &self.notes.head,
            Place::Foot => &self.notes.foot,
        };
        let have: usize = side.iter().map(|b| self.block_bytes(b).len()).sum();
        let have_lines: usize = side.iter().map(|b| lines_in(self.block_bytes(b))).sum();
        let mut bytes = have;
        let mut lines = have_lines;
        let mut made = Vec::new();
        for pos in at..end {
            let id = self.doc_id_at(pos).ok_or(Error::OutOfRange)?;
            let n = match self.row_source(id) {
                RowSource::Raw { bytes, .. } if self.col_map().is_none() => {
                    lines += lines_in(bytes).max(1);
                    bytes.len()
                }
                // An edited (or added) row counts as it will be written.
                _ => {
                    made.clear();
                    self.row_bytes(id, pos, &mut made)?;
                    lines += lines_in(&made).max(1);
                    made.len()
                }
            };
            bytes += n;
            if bytes > MAX_NOTE_BYTES || lines > MAX_NOTE_LINES {
                break;
            }
        }
        if lines > MAX_NOTE_LINES {
            return Err(too_many(lines));
        }
        if bytes > MAX_NOTE_BYTES {
            return Err(Error::Invalid(format!(
                "the report notes {} the table would hold more than 1 MB of text",
                match place {
                    Place::Head => "above",
                    Place::Foot => "below",
                }
            )));
        }
        Ok(())
    }

    /// The rows at the top (bottom) of the table are the first (last) rows
    /// of the file: a sort or a move may have put other rows there. Rows
    /// added by hand have no place in the file and may be anywhere.
    fn in_file_order(&self, at: u64, count: u64, place: Place) -> Result<(), Error> {
        let Some(order) = &self.order else {
            return Ok(());
        };
        let original = self.index.rows() as u32;
        let picked = &order[at as usize..(at + count) as usize];
        let picked_orig = || picked.iter().copied().filter(|&id| id < original);
        let rest = order[..at as usize]
            .iter()
            .chain(&order[(at + count) as usize..])
            .copied()
            .filter(|&id| id < original);
        let ok = match place {
            Place::Head => match picked_orig().max() {
                None => true,
                Some(m) => rest.into_iter().all(|id| id > m),
            },
            Place::Foot => match picked_orig().min() {
                None => true,
                Some(m) => rest.into_iter().all(|id| id < m),
            },
        };
        if ok {
            return Ok(());
        }
        // File row numbers: places among the file's rows still in the table.
        let mut all: Vec<u32> = order.iter().copied().filter(|&id| id < original).collect();
        all.sort_unstable();
        let k = picked_orig().count() as u64;
        let n = all.len() as u64;
        let (file_from, what) = match place {
            Place::Head => (0, "first"),
            Place::Foot => (n - k, "last"),
        };
        let mut nums: Vec<u64> = picked_orig()
            .map(|id| all.binary_search(&id).map_or(0, |i| i as u64))
            .collect();
        nums.sort_unstable();
        let list = numbers(&nums);
        Err(Error::Invalid(format!(
            "the table is sorted or rows were moved: rows {} of the file are its {what} rows, but your selection (rows {} on screen) holds {} of the file; undo the sort or move first",
            span(file_from, k),
            span(at, count),
            list,
        )))
    }

    /// Set rows `at..at + count` aside above (`Head`) or below (`Foot`) the
    /// table as report notes: one undo step that takes them out of the table
    /// the way deleting rows does. See [`Document::set_aside_check`].
    pub fn set_aside_rows(&mut self, at: u64, count: u64, place: Place) -> Result<(), Error> {
        self.set_aside_check(at, count, place)?;
        let mut notes = self.notes.clone();
        let take = self.take_aside(at, count, place, &mut notes)?;
        self.push(Op::Batch(vec![take, Op::SetNotes(Box::new(notes))]));
        Ok(())
    }

    /// The rows `at..at + count` as a block of `notes`, and the op that takes
    /// them out of the table.
    fn take_aside(
        &self,
        at: u64,
        count: u64,
        place: Place,
        notes: &mut Notes,
    ) -> Result<Op, Error> {
        let mut bytes = Vec::new();
        let mut rows = Vec::with_capacity(count as usize);
        for pos in at..at + count {
            let id = self.doc_id_at(pos).ok_or(Error::OutOfRange)?;
            self.row_bytes(id, pos, &mut bytes)?;
            rows.push((id, self.overlay.get(id).cloned()));
        }
        let block = Block {
            text: NoteText::Lines(bytes),
            rows: Some(rows),
        };
        match place {
            Place::Head => notes.head.push(block),
            Place::Foot => notes.foot.insert(0, block),
        }
        Ok(Op::Remove {
            at: at as usize,
            count: count as usize,
        })
    }

    /// A recipe's "set aside": the first `head` and last `foot` rows, in one
    /// undo step. Refused when the table has too few rows, or when a row to
    /// set aside holds more fields than the title row (`head` rows down):
    /// then it looks like data, not a report line.
    pub fn set_aside_ends(&mut self, head: u64, foot: u64) -> Result<(), Error> {
        self.ready()?;
        let total = self.total_rows();
        if head + foot == 0 {
            return Ok(());
        }
        if head + foot >= total {
            return Err(Error::Invalid(format!(
                "the table has {} rows: too few to set aside the first {head} and last {foot}",
                group(total)
            )));
        }
        if foot > 0 {
            self.set_aside_check(total - foot, foot, Place::Foot)?;
        }
        if head > 0 {
            self.set_aside_check(0, head, Place::Head)?;
        }
        let width = |d: &Document, pos: u64| -> Result<usize, Error> {
            let id = d.doc_id_at(pos).ok_or(Error::OutOfRange)?;
            Ok(d.logical_values(id)?.len())
        };
        let title = width(self, head)?;
        for pos in (0..head).chain(total - foot..total) {
            let w = width(self, pos)?;
            if w > title {
                return Err(Error::Invalid(format!(
                    "row {} has {w} fields, more than the {title} of the title row (row {}): it looks like data, not a report line",
                    group(pos + 1),
                    group(head + 1)
                )));
            }
        }
        let mut notes = self.notes.clone();
        let mut ops = Vec::new();
        if foot > 0 {
            ops.push(self.take_aside(total - foot, foot, Place::Foot, &mut notes)?);
        }
        if head > 0 {
            ops.push(self.take_aside(0, head, Place::Head, &mut notes)?);
        }
        ops.push(Op::SetNotes(Box::new(notes)));
        self.push(Op::Batch(ops));
        Ok(())
    }

    /// One row as the file holds it: its own bytes when it is a file row
    /// unchanged (and no column moved), else its values written in the
    /// file's separator, quoting and line break. Always ends in a break.
    fn row_bytes(&self, id: u32, pos: u64, out: &mut Vec<u8>) -> Result<(), Error> {
        if self.col_map().is_none() {
            if let RowSource::Raw { bytes, .. } = self.row_source(id) {
                out.extend_from_slice(bytes);
                if LineBreak::at_end_of(bytes).is_none() {
                    out.extend_from_slice(self.format.line_break.bytes());
                }
                return Ok(());
            }
        }
        let values = self.logical_values(id)?;
        let d = self.format.dialect;
        let mut text = String::new();
        for (col, v) in values.iter().enumerate() {
            if scan::unrepresentable(v, d) {
                return Err(Error::CannotRepresent {
                    row: pos,
                    col: col as u32,
                });
            }
            if col > 0 {
                text.push_str(d.delimiter_str());
            }
            scan::write_field(v, d, &mut text);
        }
        let own = (u64::from(id) < self.index.rows())
            .then(|| LineBreak::at_end_of(self.raw_row(id).0))
            .flatten();
        text.push_str(match own.unwrap_or(self.format.line_break) {
            LineBreak::CrLf => "\r\n",
            LineBreak::Lf => "\n",
            LineBreak::Cr => "\r",
        });
        encoding::encode(&text, self.working_encoding(), out).map_err(|u| Error::Unmappable {
            row: pos,
            ch: u.0,
            encoding: self.working_encoding().name(),
        })?;
        Ok(())
    }

    /// Put every report note back into the table (one undo step): set-aside
    /// rows exactly as they were, lines from the file read with its
    /// separator (lines with another number of fields come back as ragged
    /// rows). The rows go at the top (lines above) and the bottom (lines
    /// below). Returns how many rows came back.
    pub fn put_back_notes(&mut self) -> Result<u64, Error> {
        self.ready()?;
        if self.view.is_some() {
            return Err(Error::Invalid(
                "show all rows first: some rows are hidden or filtered out".into(),
            ));
        }
        if !self.has_report_notes() {
            return Err(Error::Invalid("there are no report notes".into()));
        }
        let mut head: Vec<(u32, Option<Edit>)> = Vec::new();
        for b in &self.notes.head {
            self.block_rows(b, &mut head)?;
        }
        let mut foot: Vec<(u32, Option<Edit>)> = Vec::new();
        for b in &self.notes.foot {
            self.block_rows(b, &mut foot)?;
        }
        let n = (head.len() + foot.len()) as u64;
        if self.total_rows().saturating_add(n) > u64::from(u32::MAX) {
            return Err(Error::TooManyRows);
        }
        // New ids for rows read from text.
        let fresh = head
            .iter()
            .chain(&foot)
            .filter(|(id, _)| *id == u32::MAX)
            .count() as u32;
        let mut next = self.new_row_ids(fresh)?;
        for (id, _) in head.iter_mut().chain(foot.iter_mut()) {
            if *id == u32::MAX {
                *id = next;
                next += 1;
            }
        }
        let end = self.total_rows() as usize + head.len();
        let notes = Notes {
            head: Vec::new(),
            foot: Vec::new(),
            keep: self.notes.keep,
        };
        let mut ops = Vec::new();
        if !head.is_empty() {
            ops.push(Op::Insert { at: 0, rows: head });
        }
        if !foot.is_empty() {
            ops.push(Op::Insert {
                at: end,
                rows: foot,
            });
        }
        ops.push(Op::SetNotes(Box::new(notes)));
        self.push(Op::Batch(ops));
        Ok(n)
    }

    /// The rows of one block: its own rows, or its lines read as rows (id
    /// `u32::MAX` until given a new one).
    fn block_rows(&self, b: &Block, out: &mut Vec<(u32, Option<Edit>)>) -> Result<(), Error> {
        if let Some(rows) = &b.rows {
            out.extend(rows.iter().cloned());
            return Ok(());
        }
        let bytes = self.block_bytes(b);
        if bytes.len() > MAX_NOTE_BYTES * 4 || lines_in(bytes) > MAX_NOTE_LINES * 4 {
            return Err(Error::Invalid(
                "the lines above or below the table are too long to put back into it".into(),
            ));
        }
        let opts = OpenOptions {
            encoding: Some(self.working_encoding()),
            dialect: Some(self.format.dialect),
            skip_lines: Some(0),
            ..Default::default()
        };
        let lines = Document::from_bytes(bytes.to_vec(), opts, None)?;
        for r in 0..lines.total_rows() {
            out.push((u32::MAX, Some(Edit::Full(lines.row_values(r)?))));
        }
        Ok(())
    }

    /// Save the report notes, or leave them out (one undo step; nothing is
    /// lost either way).
    pub fn set_keep_notes(&mut self, keep: bool) {
        if self.notes.keep == keep {
            return;
        }
        let mut notes = self.notes.clone();
        notes.keep = keep;
        self.push(Op::SetNotes(Box::new(notes)));
    }
}

/// A row near the end that looks like a total line (piece b).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TotalRow {
    /// Its place among the rows (0-based).
    pub row: u64,
    /// Rows from it to the end (blank lines after a total go with it).
    pub count: u64,
    /// Its first filled cell ("Total", "Grand total:").
    pub label: String,
    /// The number cells and what the rows above add up to.
    pub checks: Vec<TotalCheck>,
    /// Every number cell equals its column's sum exactly.
    pub adds_up: bool,
    /// Not exactly, but every cell equals its sum rounded to the cell's own
    /// decimals (whole numbers for a cell with none): never deleted.
    pub rounded: bool,
    /// Rows above that are themselves totals (first filled cell Total,
    /// Subtotal...): left out of the sums.
    pub subtotals: u64,
}

/// One number cell of a total row.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TotalCheck {
    pub col: u32,
    /// The cell as written.
    pub total: String,
    /// The rows above, added up and written with as many decimals as the
    /// cell (`None`: no numbers to add).
    pub sum: Option<String>,
    /// Exactly equal (up to the far smaller error of adding binary numbers).
    pub matches: bool,
    /// Equal only once the sum is rounded to the cell's decimals.
    pub rounded: bool,
}

/// Rows at the end looked at for a total line.
pub const TOTAL_ROWS_LOOKED_AT: u64 = 5;

/// The words that start a total line (whole cell, any case, a colon after
/// allowed).
fn total_word(cell: &str) -> bool {
    let c = cell.trim().trim_end_matches(':').trim().to_lowercase();
    matches!(c.as_str(), "total" | "grand total" | "subtotal" | "sum")
}

impl Document {
    /// A total line among the last rows: its first filled cell is Total,
    /// Grand total, Subtotal or Sum and it has at least one number cell.
    /// Only then are the rows above added up (one pass, the column readers'
    /// numbers). `None` while rows are hidden or filtered, or when there is
    /// none. With `header`, the first row is titles and is not added.
    pub fn find_total_row(
        &self,
        header: bool,
        read: crate::value::ReadOptions,
        progress: &Progress,
    ) -> Result<Option<TotalRow>, Error> {
        self.ready()?;
        if self.view.is_some() {
            return Ok(None);
        }
        let n = self.total_rows();
        let first = u64::from(header);
        if n <= first + 1 {
            return Ok(None);
        }
        let ctx = self.read_ctx(read);
        let from = n.saturating_sub(TOTAL_ROWS_LOOKED_AT).max(first + 1);
        for pos in (from..n).rev() {
            let values = self.row_values(pos)?;
            let Some(label_col) = values.iter().position(|v| !v.trim().is_empty()) else {
                continue;
            };
            if !total_word(&values[label_col]) {
                continue;
            }
            let cols: Vec<u32> = values
                .iter()
                .enumerate()
                .filter(|&(c, v)| {
                    c != label_col
                        && (c as u32) < self.col_count()
                        && !v.trim().is_empty()
                        && ctx.reader(c as u32).number(v.trim()).is_some()
                })
                .map(|(c, _)| c as u32)
                .take(crate::coltotals::MAX_COLUMN_TOTALS)
                .collect();
            if cols.is_empty() {
                // A word like "Total" with no number (a last name, say).
                continue;
            }
            let sums = self.column_totals_in(&cols, first..pos, header, read, progress)?;
            // Subtotal rows above (their first filled cell a total word) are
            // taken out of the sums again.
            let sub = self.subtotal_rows(first..pos, label_col as u32, progress)?;
            let mut less = vec![(0.0f64, 0u64); cols.len()];
            if !sub.is_empty() {
                let reader = crate::ops::Cols::new(self, &cols);
                let mut buf = Vec::new();
                for &id in &sub {
                    reader.read(self, id, &mut buf);
                    for (k, &c) in cols.iter().enumerate() {
                        if let Some(x) = ctx.reader(c).number(reader.get(&buf, k).trim()) {
                            less[k].0 += x;
                            less[k].1 += 1;
                        }
                    }
                }
            }
            let mut checks = Vec::with_capacity(cols.len());
            for ((s, &c), (minus, gone)) in sums.iter().zip(&cols).zip(&less) {
                let text = values[c as usize].trim().to_string();
                let want = ctx.reader(c).number(&text).unwrap_or(f64::NAN);
                let dec = decimals(&text, read.decimal_comma);
                let sum = s
                    .totals
                    .sum
                    .filter(|_| s.totals.numbers > *gone)
                    .map(|x| x - minus);
                // Exact: far below any decimal place (binary sums of decimal
                // values are off by about 1e-16 of the total per row).
                let matches = sum.is_some_and(|x| (x - want).abs() <= want.abs().max(1.0) * 1e-9);
                let rounded = !matches
                    && sum.is_some_and(|x| {
                        let f = 10f64.powi(dec as i32);
                        ((x * f).round() - (want * f).round()).abs() < 0.5
                    });
                checks.push(TotalCheck {
                    col: c,
                    total: text,
                    sum: sum.map(|x| {
                        let t = format!("{x:.dec$}");
                        if read.decimal_comma {
                            t.replace('.', ",")
                        } else {
                            t
                        }
                    }),
                    matches,
                    rounded,
                });
            }
            let adds_up = checks.iter().all(|c| c.matches);
            return Ok(Some(TotalRow {
                row: pos,
                count: n - pos,
                label: values[label_col].trim().to_string(),
                adds_up,
                rounded: !adds_up && checks.iter().all(|c| c.matches || c.rounded),
                subtotals: sub.len() as u64,
                checks,
            }));
        }
        Ok(None)
    }

    /// Ids of rows in `rows` whose first filled cell, up to column `upto`,
    /// is a total word (subtotals above a grand total). At most 1,000,000.
    fn subtotal_rows(
        &self,
        rows: std::ops::Range<u64>,
        upto: u32,
        progress: &Progress,
    ) -> Result<Vec<u32>, Error> {
        use rayon::prelude::*;
        const CHUNK: u64 = 8_192;
        let lead: Vec<u32> = (0..=upto.min(self.col_count().saturating_sub(1)).min(15)).collect();
        let reader = crate::ops::Cols::new(self, &lead);
        let (s0, e0) = (rows.start, rows.end);
        let parts = (0..(e0.saturating_sub(s0)).div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<Vec<u32>, Error> {
                progress.step(0)?;
                let s = s0 + k * CHUNK;
                let e = (s + CHUNK).min(e0);
                let mut buf = Vec::new();
                let mut out = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let first = (0..lead.len())
                        .map(|i| reader.get(&buf, i))
                        .find(|v| !v.trim().is_empty());
                    if first.is_some_and(total_word) {
                        out.push(id);
                    }
                }
                Ok(out)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(parts.into_iter().flatten().take(1_000_000).collect())
    }
}

/// Digits after the decimal mark of a number as written ("1,234.50" -> 2;
/// with a decimal comma "3.700,50" -> 2).
fn decimals(text: &str, decimal_comma: bool) -> usize {
    let t = text.trim().trim_end_matches('%');
    let mark = if decimal_comma { ',' } else { '.' };
    match t.rfind(mark) {
        Some(i) => t[i + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .count()
            .min(10),
        None => 0,
    }
}

fn too_many(lines: usize) -> Error {
    Error::Invalid(format!(
        "report notes can hold at most {MAX_NOTE_LINES} lines above and {MAX_NOTE_LINES} below the table ({} asked for)",
        lines
    ))
}

/// "5-8" (1-based), or "5" for one row.
fn span(from: u64, count: u64) -> String {
    if count <= 1 {
        group(from + 1)
    } else {
        format!("{}-{}", group(from + 1), group(from + count))
    }
}

/// "rows 3, 9 and 12" style list (1-based), at most 6 named.
fn numbers(nums: &[u64]) -> String {
    let names: Vec<String> = nums.iter().take(6).map(|n| group(n + 1)).collect();
    let more = nums.len().saturating_sub(6);
    let mut s = match names.len() {
        0 => "no rows".to_string(),
        1 => format!("row {}", names[0]),
        _ => {
            let (last, rest) = names.split_last().expect("two or more");
            format!("rows {} and {last}", rest.join(", "))
        }
    };
    if more > 0 {
        s = format!("{s} (and {more} more)");
    }
    s
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A `Label: value` line, or a line of two fields, as report info.
fn info_of(line: &str, d: Dialect) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.len() > 300 {
        return None;
    }
    let fields: Vec<String> = {
        let mut v = Vec::new();
        scan::scan_row(line.as_bytes(), 0, d, Mode::LineBound, |s, e| {
            v.push(scan::unquote(&line.as_bytes()[s..e], d).into_owned());
            true
        });
        v.into_iter()
            .map(|b| String::from_utf8_lossy(&b).trim().to_string())
            .collect()
    };
    let filled: Vec<&String> = fields.iter().filter(|f| !f.is_empty()).collect();
    if filled.len() == 2 {
        let label = filled[0].trim_end_matches(':').trim();
        if !label.is_empty() {
            return Some((label.to_string(), filled[1].clone()));
        }
    }
    if filled.len() == 1 {
        let (label, value) = filled[0].split_once(':')?;
        let (label, value) = (label.trim(), value.trim());
        if !label.is_empty() && !value.is_empty() && label.len() <= 60 && !label.contains("//") {
            return Some((label.to_string(), value.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(b: &[u8]) -> Document {
        Document::from_bytes(b.to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn rows(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    fn saved(d: &Document) -> Vec<u8> {
        let mut out = Vec::new();
        d.write_to(&mut out, d.format(), false).unwrap();
        out
    }

    const REPORT: &[u8] = b"name,amount\r\nann,1\r\nbo,2\r\ncy,3\r\nTotal,6\r\n";

    #[test]
    fn foot_set_aside_saves_below_and_undo_restores() {
        let mut d = doc(REPORT);
        d.set_aside_rows(4, 1, Place::Foot).unwrap();
        assert_eq!(d.row_count(), 4);
        assert_eq!(saved(&d), REPORT);
        assert_eq!(d.note_lines(false), (0, 1));
        // Sorted: the total line still goes last.
        let spec = crate::sort::SortSpec {
            keys: vec![crate::sort::SortKey {
                col: 0,
                descending: true,
                kind: Default::default(),
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        };
        let p = d.plan_sort(&spec, &Progress::new()).unwrap();
        d.apply_sort(p).unwrap();
        assert_eq!(
            saved(&d),
            b"name,amount\r\ncy,3\r\nbo,2\r\nann,1\r\nTotal,6\r\n"
        );
        assert!(d.undo());
        assert!(d.undo());
        assert_eq!(rows(&d).len(), 5);
        assert!(!d.has_report_notes());
        assert_eq!(saved(&d), REPORT);
        assert!(d.redo());
        assert_eq!(saved(&d), REPORT);
        assert_eq!(d.row_count(), 4);
    }

    #[test]
    fn head_set_aside_keeps_odd_quoting_byte_for_byte() {
        let src = b"\"a\"\"b\",c\r\n\"x\"\r\nReport for May\r\nname,n\r\nann,1\r\n";
        let opts = OpenOptions {
            delimiter: Some(b','),
            skip_lines: Some(0),
            ..Default::default()
        };
        let mut d = Document::from_bytes(src.to_vec(), opts, Some("csv")).unwrap();
        assert_eq!(rows(&d)[0], ["a\"b", "c"]);
        assert_eq!(rows(&d)[1], ["x"]);
        d.set_aside_rows(0, 3, Place::Head).unwrap();
        assert_eq!(rows(&d)[0], ["name", "n"]);
        assert_eq!(saved(&d), src);
        assert_eq!(d.note_lines(false), (3, 0));
        // Put back is the exact reverse: the same rows, still unedited.
        assert_eq!(d.put_back_notes().unwrap(), 3);
        assert!(!d.has_report_notes());
        assert_eq!(rows(&d)[0], ["a\"b", "c"]);
        assert_eq!(d.edited_rows(), 0);
        assert_eq!(saved(&d), src);
        // And undo of the put back sets them aside again.
        assert!(d.undo());
        assert_eq!(d.note_lines(false), (3, 0));
        assert_eq!(saved(&d), src);
    }

    #[test]
    fn edited_row_is_written_in_the_file_format() {
        let mut d = doc(b"a;b\nx;1\ny;2\n");
        d.set_cell(2, 0, "Total; all".into()).unwrap();
        d.set_aside_rows(2, 1, Place::Foot).unwrap();
        assert_eq!(saved(&d), b"a;b\nx;1\n\"Total; all\";2\n");
    }

    #[test]
    fn refused_while_hidden_and_away_from_the_ends() {
        let mut d = doc(REPORT);
        let e = d
            .set_aside_check(1, 2, Place::Head)
            .unwrap_err()
            .to_string();
        assert!(e.contains("select rows 1-2"), "{e}");
        let e = d
            .set_aside_check(1, 2, Place::Foot)
            .unwrap_err()
            .to_string();
        assert!(e.contains("select rows 4-5"), "{e}");
        d.hide_rows(2, 1).unwrap();
        let e = d
            .set_aside_check(0, 1, Place::Head)
            .unwrap_err()
            .to_string();
        assert!(e.contains("show all rows first"), "{e}");
    }

    #[test]
    fn sorted_bottom_rows_are_not_the_file_last_rows() {
        let mut d = doc(REPORT);
        let spec = crate::sort::SortSpec {
            keys: vec![crate::sort::SortKey {
                col: 1,
                descending: true,
                kind: Default::default(),
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        };
        let p = d.plan_sort(&spec, &Progress::new()).unwrap();
        d.apply_sort(p).unwrap();
        // amount sorted high to low as text: Total,6 first, ann,1 last.
        let last = d.total_rows() - 1;
        let e = d
            .set_aside_check(last, 1, Place::Foot)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("rows 5 of the file") || e.contains("row 5"),
            "{e}"
        );
        assert!(e.contains("your selection (rows 5 on screen)"), "{e}");
    }

    #[test]
    fn keep_switch_off_and_on_loses_nothing() {
        let mut d = doc(REPORT);
        d.set_aside_rows(4, 1, Place::Foot).unwrap();
        d.set_keep_notes(false);
        assert_eq!(saved(&d), b"name,amount\r\nann,1\r\nbo,2\r\ncy,3\r\n");
        d.set_keep_notes(true);
        assert_eq!(saved(&d), REPORT);
        d.undo();
        d.undo();
        assert_eq!(saved(&d), REPORT);
    }

    #[test]
    fn put_back_reads_file_lines_as_rows_ragged_too() {
        let src = b"Sales report\nMay 2026, all shops\nname,n\nann,1\n";
        let opts = OpenOptions {
            skip_lines: Some(2),
            ..Default::default()
        };
        let mut d = Document::from_bytes(src.to_vec(), opts, Some("csv")).unwrap();
        assert_eq!(d.note_lines(false), (2, 0));
        let r = d.report_notes();
        assert_eq!(r.above_file_lines, 2);
        assert_eq!(d.put_back_notes().unwrap(), 2);
        let all = rows(&d);
        assert_eq!(all[0], ["Sales report"]);
        assert_eq!(all[1], ["May 2026", " all shops"]);
        assert_eq!(all[2], ["name", "n"]);
        // The lines are rows now; saving writes them once.
        assert_eq!(saved(&d), src);
        assert!(d.undo());
        assert_eq!(saved(&d), src);
        assert_eq!(rows(&d)[0], ["name", "n"]);
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("nde-notes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn save_over(d: &mut Document) {
        d.save(crate::save::SaveOptions::default()).unwrap();
    }

    /// A report file: 4 lines above (title, date, blank, a label line), the
    /// table with a quoted two-line cell, and a total row.
    fn report_file(lb: &str) -> String {
        [
            "Sales by shop",
            "Printed: 2026-05-01",
            "",
            "Region: North",
            "name,amount,note",
            "ann,1,\"two",
            "lines\"",
            "bo,2,",
            "Total,3,",
            "",
        ]
        .join(lb)
    }

    #[test]
    fn saved_to_disk_continues_with_the_notes_and_saves_the_same_bytes() {
        for lb in ["\r\n", "\n"] {
            let dir = tmp(if lb == "\n" { "lf" } else { "crlf" });
            let path = dir.join("report.csv");
            let src = report_file(lb);
            std::fs::write(&path, &src).unwrap();
            let opts = OpenOptions {
                skip_lines: Some(4),
                ..Default::default()
            };
            let mut d = Document::open_indexed(&path, opts, None).unwrap();
            assert_eq!(d.total_rows(), 4);
            d.set_aside_rows(3, 1, Place::Foot).unwrap();
            save_over(&mut d);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), src);
            // After the save the total line is still below the table.
            assert_eq!(d.total_rows(), 3);
            assert_eq!(d.note_lines(false), (4, 1));
            let r = d.report_notes();
            assert_eq!(r.below_file_lines, 1);
            assert!(r.below_file.starts_with("Total,3,"));
            // An edit, then a second save: the notes stay where they are.
            d.set_cell(2, 1, "9".into()).unwrap();
            save_over(&mut d);
            let want = src.replace(&format!("bo,2,{lb}"), &format!("bo,9,{lb}"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), want);
            assert_eq!(d.total_rows(), 3);
            // Leaving them out writes the table only.
            d.set_keep_notes(false);
            save_over(&mut d);
            let table = format!("name,amount,note{lb}ann,1,\"two{lb}lines\"{lb}bo,9,{lb}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), table);
            assert!(!d.has_report_notes());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_recovery_copy_holds_the_notes_and_saves_the_original_bytes() {
        let src = report_file("\r\n");
        let opts = OpenOptions {
            skip_lines: Some(4),
            ..Default::default()
        };
        let mut d = Document::from_bytes(src.clone().into_bytes(), opts, Some("csv")).unwrap();
        d.set_aside_rows(3, 1, Place::Foot).unwrap();
        // The switch off: the copy still holds every line.
        d.set_keep_notes(false);
        let mut copy = Vec::new();
        d.write_copy(&mut copy).unwrap();
        assert_eq!(copy, src.as_bytes());
        let (head, tail) = d.copy_layout();
        let back = OpenOptions {
            dialect: Some(d.format().dialect),
            skip_lines: Some(head),
            tail_bytes: Some(tail),
            ..Default::default()
        };
        let mut r = Document::from_bytes(copy, back, Some("csv")).unwrap();
        assert_eq!(r.total_rows(), 3);
        assert_eq!(r.note_lines(false), (4, 1));
        let dir = tmp("recover");
        let path = dir.join("again.csv");
        r.save(crate::save::SaveOptions {
            path: Some(path.clone()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), src);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wide_and_legacy_encodings_keep_the_notes() {
        let text = "Ventes \u{e9}t\u{e9}\r\nname;n\r\nJos\u{e9};1\r\nTotal;1\r\n";
        let cases: Vec<(Encoding, Vec<u8>)> = vec![
            (Encoding::Utf16Le, {
                let mut b = vec![0xFF, 0xFE];
                for u in text.encode_utf16() {
                    b.extend_from_slice(&u.to_le_bytes());
                }
                b
            }),
            (
                Encoding::Legacy(encoding_rs::WINDOWS_1252),
                encoding_rs::WINDOWS_1252.encode(text).0.into_owned(),
            ),
        ];
        for (enc, bytes) in cases {
            let dir = tmp(enc.name());
            let path = dir.join("r.csv");
            std::fs::write(&path, &bytes).unwrap();
            let opts = OpenOptions {
                encoding: Some(enc),
                delimiter: Some(b';'),
                skip_lines: Some(1),
                ..Default::default()
            };
            let mut d = Document::open_indexed(&path, opts, None).unwrap();
            d.set_aside_rows(2, 1, Place::Foot).unwrap();
            save_over(&mut d);
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{}", enc.name());
            assert_eq!(d.total_rows(), 2);
            assert_eq!(d.note_lines(false), (1, 1));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn headers_start_here_then_save_reads_back() {
        let src = b"Sales\r\n,\r\nname,n\r\nann,1\r\n";
        let dir = tmp("head");
        let path = dir.join("h.csv");
        std::fs::write(&path, src).unwrap();
        let opts = OpenOptions {
            skip_lines: Some(0),
            delimiter: Some(b','),
            ..Default::default()
        };
        let mut d = Document::open_indexed(&path, opts, None).unwrap();
        d.set_aside_rows(0, 2, Place::Head).unwrap();
        assert_eq!(rows(&d)[0], ["name", "n"]);
        save_over(&mut d);
        assert_eq!(std::fs::read(&path).unwrap(), src);
        assert_eq!(rows(&d)[0], ["name", "n"]);
        assert_eq!(d.note_lines(false), (2, 0));
        // Lines read from the file go back as rows (ragged ones too).
        assert_eq!(d.put_back_notes().unwrap(), 2);
        assert_eq!(rows(&d)[0], ["Sales"]);
        assert_eq!(rows(&d)[1], ["", ""]);
        save_over(&mut d);
        assert_eq!(std::fs::read(&path).unwrap(), src);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Random small files: setting aside the first and last rows, with or
    /// without edits elsewhere and a sort, always saves the set-aside lines
    /// byte for byte, and undoing everything gives the file back.
    #[test]
    fn random_files_keep_set_aside_bytes() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n.max(1)
        };
        let cells = ["a", "\"q,\"", "", "\"x\"\"y\"", "12", "\"two\nlines\""];
        for _ in 0..300 {
            let lb = if next(2) == 0 { "\r\n" } else { "\n" };
            let rows_n = 3 + next(6) as usize;
            let mut src = String::new();
            for _ in 0..rows_n {
                let w = 1 + next(3) as usize;
                let row: Vec<&str> = (0..w)
                    .map(|_| cells[next(cells.len() as u64) as usize])
                    .collect();
                src.push_str(&row.join(","));
                src.push_str(lb);
            }
            let opts = OpenOptions {
                delimiter: Some(b','),
                skip_lines: Some(0),
                ..Default::default()
            };
            let mut d = Document::from_bytes(src.clone().into_bytes(), opts, Some("csv")).unwrap();
            let total = d.total_rows();
            let h = next(2).min(total - 2);
            let f = 1 + next(1);
            let head: Vec<u8> = (0..h)
                .flat_map(|i| {
                    let (a, b) = d.index.row_range(i).unwrap();
                    d.source.bytes()[a..b].to_vec()
                })
                .collect();
            let foot: Vec<u8> = (total - f..total)
                .flat_map(|i| {
                    let (a, b) = d.index.row_range(i).unwrap();
                    d.source.bytes()[a..b].to_vec()
                })
                .collect();
            d.set_aside_rows(total - f, f, Place::Foot).unwrap();
            if h > 0 {
                d.set_aside_rows(0, h, Place::Head).unwrap();
            }
            if d.total_rows() > 0 && next(2) == 0 {
                d.set_cell(0, 0, "edited".into()).unwrap();
            }
            if next(2) == 0 {
                let p = d.plan_flip(0, d.total_rows()).unwrap();
                d.apply_shuffle(p).unwrap();
            }
            let out = saved(&d);
            assert!(out.starts_with(&head), "{src:?}");
            assert!(out.ends_with(&foot), "{src:?}");
            while d.undo() {}
            assert_eq!(saved(&d), src.as_bytes(), "{src:?}");
        }
    }

    #[test]
    fn recipe_step_counts_only_and_refuses_data_rows() {
        use crate::recipe::{Recipe, RecipeOp, RECIPE_VERSION};
        let step = RecipeOp::SetAside { head: 2, foot: 1 };
        assert_eq!(
            step.describe(),
            "Set aside the first 2 rows and the last row as report notes"
        );
        assert_eq!(
            RecipeOp::SetAside { head: 4, foot: 0 }.describe(),
            "Set aside the first 4 rows as report notes"
        );
        let r = Recipe {
            version: RECIPE_VERSION,
            name: String::new(),
            columns: vec![],
            header: true,
            steps: vec![step],
            left_out: vec![],
            questions: vec![],
        };
        let json = String::from_utf8(r.to_json().unwrap()).unwrap();
        assert!(json.contains("\"set_aside\""), "{json}");
        assert!(!json.contains("Sales") && !json.contains("Total"), "{json}");
        let src = b"Sales\nMay\nname,n\nann,1\nTotal,1\n";
        let opts = OpenOptions {
            delimiter: Some(b','),
            skip_lines: Some(0),
            ..Default::default()
        };
        let mut d = Document::from_bytes(src.to_vec(), opts, Some("csv")).unwrap();
        d.run_recipe(&r, 0.0, 1, &Progress::new()).unwrap();
        assert_eq!(rows(&d), [vec!["name", "n"], vec!["ann", "1"]]);
        assert_eq!(saved(&d), src);
        // One undo step.
        assert!(d.undo());
        assert_eq!(d.total_rows(), 5);
        assert!(!d.undo());
        // Too few rows.
        let mut small = Document::from_bytes(b"a,b\n1,2\n".to_vec(), opts, None).unwrap();
        let e = small
            .run_recipe(&r, 0.0, 1, &Progress::new())
            .unwrap_err()
            .to_string();
        assert!(e.contains("too few"), "{e}");
        // A row to set aside wider than the title row: data, not a note.
        let mut wide =
            Document::from_bytes(b"x,y,z\nq\nname,n\nann,1\nTotal,1\n".to_vec(), opts, None)
                .unwrap();
        let e = wide
            .run_recipe(&r, 0.0, 1, &Progress::new())
            .unwrap_err()
            .to_string();
        assert!(e.contains("looks like data"), "{e}");
        assert_eq!(wide.total_rows(), 5);
    }

    /// The 1 MB limit counts an edited row at the length it will be written.
    #[test]
    fn edited_rows_count_toward_the_size_limit() {
        let mut d = doc(b"a\nb\nc\nd\n");
        let big = "x".repeat(600 * 1024);
        d.set_cell(2, 0, big.clone()).unwrap();
        d.set_cell(3, 0, big).unwrap();
        let e = d
            .set_aside_check(2, 2, Place::Foot)
            .unwrap_err()
            .to_string();
        assert!(e.contains("more than 1 MB"), "{e}");
        assert!(d.set_aside_check(3, 1, Place::Foot).is_ok());
        // An edited cell with a line break counts its lines.
        let mut d = doc(b"a\nb\n");
        d.set_cell(1, 0, "two\nlines".into()).unwrap();
        d.set_aside_rows(1, 1, Place::Foot).unwrap();
        assert_eq!(d.note_lines(false), (0, 2));
    }

    /// Saved with the switch off, the tab carries on without the notes (the
    /// question says so).
    #[test]
    fn saved_with_the_switch_off_the_tab_has_no_notes() {
        let dir = tmp("off");
        let path = dir.join("r.csv");
        std::fs::write(&path, REPORT).unwrap();
        let mut d = Document::open_indexed(
            &path,
            OpenOptions {
                skip_lines: Some(0),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        d.set_aside_rows(4, 1, Place::Foot).unwrap();
        d.set_keep_notes(false);
        save_over(&mut d);
        assert!(!d.has_report_notes());
        d.set_keep_notes(true);
        assert!(!d.is_modified());
        d.set_cell(1, 1, "5".into()).unwrap();
        save_over(&mut d);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"name,amount\r\nann,5\r\nbo,2\r\ncy,3\r\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn total_rows_found_checked_and_not_mistaken() {
        let read = crate::value::ReadOptions::default();
        let p = Progress::new();
        // A true total.
        let d = doc(b"name,amount,n\nann,1.25,2\nbo,2.50,3\nTotal:,3.75,5\n");
        let t = d.find_total_row(true, read, &p).unwrap().unwrap();
        assert_eq!(
            (t.row, t.count, t.label.as_str(), t.adds_up),
            (3, 1, "Total:", true)
        );
        assert_eq!(t.checks.len(), 2);
        assert_eq!(t.checks[0].sum.as_deref(), Some("3.75"));
        // Wrong numbers: says which column does not add up.
        let d = doc(b"name,amount,n\nann,1.25,2\nbo,2.50,3\nGrand total,3.80,5\n,,\n");
        let t = d.find_total_row(true, read, &p).unwrap().unwrap();
        assert_eq!((t.row, t.count), (3, 2));
        assert!(!t.adds_up);
        assert!(!t.checks[0].matches && t.checks[1].matches);
        assert_eq!(t.checks[0].sum.as_deref(), Some("3.75"));
        // A person whose last name is Total, numbers elsewhere blank: no.
        let d = doc(b"first,last,amount\nann,Lee,1\n,Total,\n");
        assert_eq!(d.find_total_row(true, read, &p).unwrap(), None);
        // A first cell "Total" over text only: no number cell, no.
        let d = doc(b"name,city\nann,Oslo\nTotal,all\n");
        assert_eq!(d.find_total_row(true, read, &p).unwrap(), None);
        // A non-numeric column beside a numeric one: only number cells are checked.
        let d = doc(b"name,city,n\nann,Oslo,1\nbo,Rome,2\nSUM,-,3\n");
        let t = d.find_total_row(true, read, &p).unwrap().unwrap();
        assert!(t.adds_up && t.checks.len() == 1 && t.checks[0].col == 2);
        // Further up than the last 5 rows: not looked at.
        let d = doc(b"n\n1\nTotal,1\n2\n3\n4\n5\n6\n");
        assert_eq!(d.find_total_row(true, read, &p).unwrap(), None);
        // Not while rows are hidden.
        let mut d = doc(b"name,amount\nann,1\nbo,2\nTotal,3\n");
        d.hide_rows(1, 1).unwrap();
        assert_eq!(d.find_total_row(true, read, &p).unwrap(), None);
    }

    /// Review b1: a whole-number total is not "adds up" for 3699.60; it adds
    /// up only when rounded, and that is said apart (no Delete).
    #[test]
    fn adds_up_means_exactly() {
        let read = crate::value::ReadOptions::default();
        let p = Progress::new();
        let d = doc(b"name,amount\nann,1849.80\nbo,1849.80\nTotal,3700\n");
        let t = d.find_total_row(true, read, &p).unwrap().unwrap();
        assert!(!t.adds_up && t.rounded, "{t:?}");
        assert!(!t.checks[0].matches && t.checks[0].rounded);
        assert_eq!(t.checks[0].sum.as_deref(), Some("3700"));
        // Far off is neither.
        let d = doc(b"name,amount\nann,1\nbo,2\nTotal,4\n");
        let t = d.find_total_row(true, read, &p).unwrap().unwrap();
        assert!(!t.adds_up && !t.rounded);
        // Many cents add up exactly.
        let mut src = String::from("name,amount\n");
        for i in 0..10_000 {
            src.push_str(&format!("r{i},0.10\n"));
        }
        src.push_str("Total,1000.00\n");
        let d = doc(src.as_bytes());
        assert!(d.find_total_row(true, read, &p).unwrap().unwrap().adds_up);
    }

    /// Review b2: a decimal comma counts the decimals after the comma.
    #[test]
    fn decimal_comma_totals() {
        let read = crate::value::ReadOptions {
            decimal_comma: true,
            ..Default::default()
        };
        let p = Progress::new();
        let d = Document::from_bytes(
            b"name;amount\nann;1.850,25\nbo;1.850,25\nTotal;3.700,50\n".to_vec(),
            OpenOptions {
                delimiter: Some(b';'),
                ..Default::default()
            },
            Some("csv"),
        )
        .unwrap();
        assert_eq!(decimals("3.700,50", true), 2);
        let t = d.find_total_row(true, read, &p).unwrap().unwrap();
        assert!(t.adds_up, "{t:?}");
        assert_eq!(t.checks[0].sum.as_deref(), Some("3700,50"));
        let d = Document::from_bytes(
            b"name;amount\nann;1.850,25\nbo;1.850,25\nTotal;3.700,40\n".to_vec(),
            OpenOptions {
                delimiter: Some(b';'),
                ..Default::default()
            },
            Some("csv"),
        )
        .unwrap();
        let t = d.find_total_row(true, read, &p).unwrap().unwrap();
        assert!(!t.adds_up && !t.rounded);
    }

    /// Review b3: subtotal rows above a grand total are left out, and counted.
    #[test]
    fn subtotals_are_left_out_of_a_grand_total() {
        let read = crate::value::ReadOptions::default();
        let p = Progress::new();
        let d = doc(
            b"shop,amount\nnorth,10\nnorth,5\nSubtotal,15\nsouth,7\nSubtotal:,7\nGrand total,22\n",
        );
        let t = d.find_total_row(true, read, &p).unwrap().unwrap();
        assert!(t.adds_up, "{t:?}");
        assert_eq!((t.subtotals, t.label.as_str()), (2, "Grand total"));
    }

    /// Review b4: a voter list where a person's first name is "Total" and the
    /// ID column is all numbers never adds up, so Delete is never offered.
    #[test]
    fn a_voter_named_total_never_adds_up() {
        let read = crate::value::ReadOptions::default();
        let p = Progress::new();
        let d = doc(
            b"first_name,last_name,voter_id,precinct\nAnn,Lee,100231,12\nBo,Wu,100232,12\nCy,Ray,100233,7\nTotal,Smith,100234,7\n",
        );
        if let Some(t) = d.find_total_row(true, read, &p).unwrap() {
            assert!(!t.adds_up && !t.rounded, "{t:?}");
        }
    }

    #[test]
    fn info_lines() {
        let d = Document::from_bytes(
            b"Report: May sales\nShop,North\nname,n\nann,1\n".to_vec(),
            OpenOptions {
                skip_lines: Some(2),
                ..Default::default()
            },
            Some("csv"),
        )
        .unwrap();
        let r = d.report_notes();
        assert_eq!(
            r.info,
            [
                ("Report".to_string(), "May sales".to_string()),
                ("Shop".to_string(), "North".to_string())
            ]
        );
    }
}
