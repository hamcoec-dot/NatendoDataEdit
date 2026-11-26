//! Repairing one ragged row: what the repair bar shows (the row's text as it
//! is in the file, its fields, the next row of the file) and the three small
//! changes it offers, each one undo step.

use super::*;
use crate::document::ragged::COLUMNS_CHANGED;

/// Most of a row's text sent to the screen; nothing past it can be joined.
pub const MAX_SHOWN: usize = 64 << 10;

const CHANGED: &str = "the table changed since the row was read; look again";

/// What goes between the two cells when a row is joined with the next one.
/// (Never the file's own separator.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Between {
    Space,
    LineBreak,
    Nothing,
}

impl Between {
    fn text(self) -> &'static str {
        match self {
            Between::Space => " ",
            Between::LineBreak => "\n",
            Between::Nothing => "",
        }
    }
}

/// A repair of one row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Repair {
    /// `groups`: how many neighbouring fields go into each cell, from the
    /// first; fields past the groups stay as they are.
    Join { groups: Vec<u32> },
    /// With the next row of the file.
    Next { between: Between },
    /// With the previous row of the file, which keeps the result. With
    /// `header`, the table's first row holds titles and is never joined.
    Previous { between: Between, header: bool },
    /// Split the file's text again with quote marks read as ordinary text.
    QuoteAsText,
}

/// The next row of the file, as the repair bar shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NextRow {
    pub id: u32,
    /// Its number in the file, counted from 1.
    pub file_row: u64,
    pub edited: bool,
    /// Not on screen (hidden or filtered out).
    pub hidden: bool,
    /// It has a note (filled in by the caller, who holds the notes).
    pub noted: bool,
    /// The columns its note is matched on (empty without a note).
    pub note_keys: Vec<u32>,
    pub fields: Vec<String>,
}

/// A row as the repair bar shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RowText {
    pub id: u32,
    /// Where the row is on screen (`None`: hidden or filtered out).
    pub row: Option<u64>,
    /// Its number in the file, counted from 1 (0: added by hand).
    pub file_row: u64,
    pub revision: u64,
    /// Changed or added since the file was read: there is no file text, only
    /// the cells.
    pub edited: bool,
    /// The row's text in the file (display only), at most [`MAX_SHOWN`]
    /// bytes.
    pub raw: String,
    /// The row is longer than what is shown.
    pub cut: bool,
    /// The cells, as many as are shown.
    pub fields: Vec<String>,
    /// All the row's cells, shown or not.
    pub width: u64,
    pub usual: u32,
    pub line_by_line: bool,
    /// Why fields cannot be joined ("" when they can).
    pub no_join: String,
    /// What "treat this quote as text" would give (empty: not offered).
    pub quoted_as_text: Vec<String>,
    pub next: Option<NextRow>,
    /// Why there is no next row to join ("" when there is one).
    pub no_next: String,
    /// The previous row of the file, and why there is none to join.
    pub previous: Option<NextRow>,
    pub no_previous: String,
    /// The columns this row's note is matched on (empty without a note;
    /// filled in by the caller).
    pub note_keys: Vec<u32>,
}

const MERGED: &str =
    "this file treats runs of separators as one, so the original separator between two fields is not known";

impl Document {
    /// Where row `id` is in the whole table, shown or not.
    fn position_of(&self, id: u32) -> Option<u64> {
        match &self.order {
            // One pass over the ids: well under a millisecond for a million.
            Some(order) => order.iter().position(|&i| i == id).map(|p| p as u64),
            None => (u64::from(id) < self.index.rows()).then_some(u64::from(id)),
        }
    }

    fn file_row(&self, id: u32) -> bool {
        u64::from(id) < self.index.rows()
    }

    /// The next row of the file after row `id`, or why it cannot be joined.
    fn next_of(&self, id: u32) -> Result<(u32, u64), &'static str> {
        if !self.file_row(id) {
            return Err("this row was added by hand: it has no next row in the file");
        }
        let next = id + 1;
        if !self.file_row(next) {
            return Err("this is the last row of the file");
        }
        match self.position_of(next) {
            Some(at) => Ok((next, at)),
            None => Err("the next row of the file was removed"),
        }
    }

    /// Where the row at place `at` of the whole table is on screen.
    fn shown_at(&self, at: u64) -> Option<u64> {
        match &self.view {
            None => Some(at),
            // The shown places are in order.
            Some(v) => self
                .visible(v)
                .binary_search(&u32::try_from(at).ok()?)
                .ok()
                .map(|r| r as u64),
        }
    }

    /// The id of shown row `row` (kept by the row through sorts and
    /// deletes).
    pub fn row_id(&self, row: u64) -> Option<u32> {
        self.id_at(row)
    }

    /// The previous row of the file before row `id`, or why it cannot be
    /// joined.
    fn previous_of(&self, id: u32, header: bool) -> Result<(u32, u64), &'static str> {
        if !self.file_row(id) {
            return Err("this row was added by hand: it has no previous row in the file");
        }
        let Some(previous) = id.checked_sub(1) else {
            return Err("this is the first row of the file");
        };
        match self.position_of(previous) {
            Some(0) if header => Err("the row before this one holds the column names"),
            Some(at) => Ok((previous, at)),
            None => Err("the previous row of the file was removed"),
        }
    }

    /// A neighbouring row as the bar shows it.
    fn neighbour(&self, id: u32, at: u64) -> Result<NextRow, Error> {
        let mut fields = self.logical_values(id)?;
        let mut used = 0usize;
        fields.retain(|v| {
            used += v.len() + 1;
            used <= MAX_SHOWN
        });
        Ok(NextRow {
            id,
            file_row: u64::from(id) + 1,
            edited: self.overlay.get(id).is_some(),
            hidden: self.shown_at(at).is_none(),
            noted: false,
            note_keys: Vec::new(),
            fields,
        })
    }

    /// The raw bytes of a file row that no edit has touched, without its line
    /// break.
    fn untouched(&self, id: u32) -> Option<(&[u8], Mode)> {
        if !self.file_row(id) || self.overlay.get(id).is_some() {
            return None;
        }
        let (bytes, mode) = self.raw_row(id);
        let end = scan::scan_row(bytes, 0, self.format.dialect, mode, |_, _| true).content_end;
        Some((&bytes[..end], mode))
    }

    fn split_without_quotes(&self, bytes: &[u8], mode: Mode) -> Result<Vec<String>, Error> {
        let plain = self.format.dialect.with_quote(None)?;
        let mut out = Vec::new();
        scan::scan_row(bytes, 0, plain, mode, |s, e| {
            let v = &bytes[s..e];
            out.push(encoding::decode_direct(v, self.working_encoding()).into_owned());
            out.len() as u64 <= MAX_ROW_FIELDS
        });
        Ok(out)
    }

    /// The fields of shown row `row` and the usual number, reading that row
    /// only.
    pub fn row_width(&self, row: u64) -> Result<(u64, u32), Error> {
        let id = self.id_at(row).ok_or(Error::OutOfRange)?;
        let width = match self.overlay.get(id) {
            Some(edit) => u64::from(edit.width()),
            None => self.file_fields(id),
        };
        Ok((width, self.index.stats().usual_fields()))
    }

    /// What the repair bar shows for row `id`.
    /// With `header`, the table's first row holds titles.
    pub fn row_text(&self, id: u32, header: bool) -> Result<RowText, Error> {
        self.ready_for_bulk()?;
        if self.col_map().is_some() {
            return Err(Error::Invalid(COLUMNS_CHANGED.into()));
        }
        let Some(at) = self.position_of(id) else {
            return Err(Error::Invalid(CHANGED.into()));
        };
        let mut values = self.logical_values(id)?;
        let width = values.len() as u64;
        let mut raw = String::new();
        let mut cut = false;
        let mut quoted_as_text = Vec::new();
        let untouched = self.untouched(id);
        if let Some((bytes, mode)) = untouched {
            // Cut before decoding: nothing long is built or sent.
            let end = bytes.len().min(MAX_SHOWN);
            cut = bytes.len() > MAX_SHOWN;
            raw = encoding::decode_direct(&bytes[..end], self.working_encoding()).into_owned();
            let quote = self.format.dialect.quote();
            if !cut && quote.is_some_and(|q| bytes.contains(&q)) {
                let split = self.split_without_quotes(bytes, mode)?;
                if split != values {
                    quoted_as_text = split;
                }
            }
        }
        // As many cells as fit in what is shown.
        let mut used = 0usize;
        let mut shown = 0usize;
        for v in &values {
            used += v.len() + 1;
            if used > MAX_SHOWN + 1 {
                cut = true;
                break;
            }
            shown += 1;
        }
        values.truncate(shown);
        let (next, no_next) = match self.next_of(id) {
            Ok((n, at)) => (Some(self.neighbour(n, at)?), String::new()),
            Err(why) => (None, why.to_string()),
        };
        let (previous, no_previous) = match self.previous_of(id, header) {
            Ok((n, at)) => (Some(self.neighbour(n, at)?), String::new()),
            Err(why) => (None, why.to_string()),
        };
        Ok(RowText {
            id,
            row: self.shown_at(at),
            file_row: if self.file_row(id) {
                u64::from(id) + 1
            } else {
                0
            },
            revision: self.revision(),
            edited: untouched.is_none(),
            raw,
            cut,
            fields: values,
            width,
            usual: self.index.stats().usual_fields(),
            line_by_line: untouched.is_some_and(|(_, m)| m != Mode::Normal),
            no_join: if self.format.dialect.merges() {
                MERGED.into()
            } else {
                String::new()
            },
            quoted_as_text,
            next,
            no_next,
            previous,
            no_previous,
            note_keys: Vec::new(),
        })
    }

    /// Refuse a repair that would change a protected column: every cell from
    /// `from` on may move.
    fn unprotected_from(&self, from: usize, width: usize) -> Result<(), Error> {
        if (from..width).any(|c| self.is_protected(c as u32)) {
            return Err(Error::Invalid(
                "the repair would change cells in a protected column (right-click a heading › Protect column to switch it off)".into(),
            ));
        }
        Ok(())
    }

    /// The state that makes row `id` hold `values`: none when that is what
    /// the file holds.
    fn state_for(&self, id: u32, values: Vec<String>) -> Result<(u32, Option<Edit>), Error> {
        if self.file_row(id) {
            let (bytes, mode) = self.raw_row(id);
            if self.original_fields(id, bytes, mode)? == values {
                return Ok((id, None));
            }
        }
        Ok((id, Some(Edit::Full(values))))
    }

    /// Two rows made one: refused when either is longer than the bar shows.
    fn joined(
        first: Vec<String>,
        second: Vec<String>,
        between: Between,
    ) -> Result<Vec<String>, Error> {
        for row in [&first, &second] {
            if row.iter().map(|v| v.len() + 1).sum::<usize>() > MAX_SHOWN + 1 {
                return Err(Error::Invalid(
                    "one of the two rows is longer than what the bar can show".into(),
                ));
            }
        }
        let mut out = first;
        let mut rest = second.into_iter();
        let head = rest.next().unwrap_or_default();
        match out.last_mut() {
            Some(last) => {
                last.push_str(between.text());
                last.push_str(&head);
            }
            None => out.push(head),
        }
        out.extend(rest);
        if out.len() as u64 > MAX_ROW_FIELDS {
            return Err(Error::Invalid("the joined row would be too wide".into()));
        }
        Ok(out)
    }

    /// Apply one repair to row `id`, as one undo step. Refused when the table
    /// changed since [`Document::row_text`] gave `revision`.
    pub fn repair_row(&mut self, id: u32, revision: u64, repair: &Repair) -> Result<(), Error> {
        self.ready_for_bulk()?;
        if self.col_map().is_some() {
            return Err(Error::Invalid(COLUMNS_CHANGED.into()));
        }
        if revision != self.revision() || self.position_of(id).is_none() {
            return Err(Error::Invalid(CHANGED.into()));
        }
        let values = self.logical_values(id)?;
        let width = values.len();
        match repair {
            Repair::Join { groups } => {
                if self.format.dialect.merges() {
                    return Err(Error::Invalid(MERGED.into()));
                }
                let covered: u64 = groups.iter().map(|&g| u64::from(g)).sum();
                if groups.contains(&0) || covered > width as u64 {
                    return Err(Error::Invalid("those fields are not in this row".into()));
                }
                let Some(first) = groups.iter().position(|&g| g > 1) else {
                    return Ok(());
                };
                self.unprotected_from(first, width)?;
                let sep = self.format.dialect.delimiter_str().to_string();
                let mut out = Vec::with_capacity(width);
                let mut at = 0usize;
                let mut shown = 0usize;
                for &g in groups {
                    let run = &values[at..at + g as usize];
                    let cell = run.join(&sep);
                    if g > 1 {
                        shown += cell.len();
                    }
                    out.push(cell);
                    at += g as usize;
                }
                if shown > MAX_SHOWN {
                    return Err(Error::Invalid(
                        "the joined cell would be longer than what the bar can show".into(),
                    ));
                }
                out.extend_from_slice(&values[at..]);
                let state = self.state_for(id, out)?;
                self.push(Op::Restore(vec![state]));
            }
            Repair::Next { between } => {
                let (next, at) = self.next_of(id).map_err(|why| Error::Invalid(why.into()))?;
                let after = self.logical_values(next)?;
                self.unprotected_from(width.saturating_sub(1), width + after.len())?;
                let state = self.state_for(id, Self::joined(values, after, *between)?)?;
                // (The cell state goes by id, so the removal's place holds.)
                self.push(Op::Batch(vec![
                    Op::Restore(vec![state]),
                    Op::Remove {
                        at: at as usize,
                        count: 1,
                    },
                ]));
            }
            Repair::Previous { between, header } => {
                let (previous, _) = self
                    .previous_of(id, *header)
                    .map_err(|why| Error::Invalid(why.into()))?;
                let at = self
                    .position_of(id)
                    .ok_or_else(|| Error::Invalid(CHANGED.into()))?;
                let before = self.logical_values(previous)?;
                self.unprotected_from(before.len().saturating_sub(1), before.len() + width)?;
                let state = self.state_for(previous, Self::joined(before, values, *between)?)?;
                self.push(Op::Batch(vec![
                    Op::Restore(vec![state]),
                    Op::Remove {
                        at: at as usize,
                        count: 1,
                    },
                ]));
            }
            Repair::QuoteAsText => {
                let Some((bytes, mode)) = self.untouched(id) else {
                    return Err(Error::Invalid(
                        "this row has been changed: its text in the file no longer applies".into(),
                    ));
                };
                if bytes.len() > MAX_SHOWN {
                    return Err(Error::Invalid(
                        "the row is longer than what the bar can show".into(),
                    ));
                }
                let out = self.split_without_quotes(bytes, mode)?;
                if out == values {
                    return Ok(());
                }
                self.unprotected_from(0, width.max(out.len()))?;
                let state = self.state_for(id, out)?;
                self.push(Op::Restore(vec![state]));
            }
        }
        Ok(())
    }
}

/// Rows to cut, by id, with the cells they keep.
type Cut = Vec<(u32, Vec<String>)>;

/// Changes shown before "Fix all like this" is applied.
pub const SHOWN_CHANGES: usize = 20;

/// One row "Fix all like this" would change: its cells afterwards.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ShapeRow {
    pub id: u32,
    pub file_row: u64,
    pub cells: Vec<String>,
}

/// The rows with the same number of fields as a repaired one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SameShape {
    pub revision: u64,
    pub total: u64,
    /// How many of them are not on screen.
    pub hidden: u64,
    /// The first [`SHOWN_CHANGES`].
    pub first: Vec<ShapeRow>,
}

/// A join of `groups` fits rows of `fields` fields whose usual number is
/// `usual`.
fn check_shape(groups: &[u32], fields: u32, usual: u32) -> Result<(), Error> {
    let covered: u64 = groups.iter().map(|&g| u64::from(g)).sum();
    if groups.contains(&0) || covered > u64::from(fields) || !groups.iter().any(|&g| g > 1) {
        return Err(Error::Invalid("choose the fields to join".into()));
    }
    if fields == usual {
        return Err(Error::Invalid(
            "rows with the usual number of fields are never changed together".into(),
        ));
    }
    Ok(())
}

/// `values` with its neighbouring fields joined as `groups` says.
fn join_groups(values: &[String], groups: &[u32], separator: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(values.len());
    let mut at = 0usize;
    for &g in groups {
        out.push(values[at..at + g as usize].join(separator));
        at += g as usize;
    }
    out.extend_from_slice(&values[at..]);
    out
}

impl Document {
    fn width_of(&self, id: u32) -> u64 {
        match self.overlay.get(id) {
            Some(edit) => u64::from(edit.width()),
            None => self.file_fields(id),
        }
    }

    /// Why no repair can be made at all.
    fn repairable(&self) -> Result<(), Error> {
        self.ready_for_bulk()?;
        if self.col_map().is_some() {
            return Err(Error::Invalid(COLUMNS_CHANGED.into()));
        }
        Ok(())
    }

    /// The ids, in table order, of the rows of exactly `fields` fields (the
    /// title row never, hidden rows too).
    fn rows_of_width(
        &self,
        header: bool,
        fields: u32,
        progress: &Progress,
    ) -> Result<Vec<(u64, u32)>, Error> {
        let first = u64::from(header);
        let total = self.total_rows();
        progress.start(total.saturating_sub(first));
        let mut out = Vec::new();
        for at in first..total {
            let id = self.doc_id_at(at).ok_or(Error::OutOfRange)?;
            if self.width_of(id) == u64::from(fields) {
                out.push((at, id));
            }
            if at % 8192 == 0 {
                progress.step(8192.min(total - at))?;
            }
        }
        Ok(out)
    }

    /// What "Fix all like this" would do: join the fields `groups` of every
    /// row that has exactly `fields` fields.
    pub fn same_shape(
        &self,
        header: bool,
        fields: u32,
        groups: &[u32],
        progress: &Progress,
    ) -> Result<SameShape, Error> {
        self.repairable()?;
        check_shape(groups, fields, self.index.stats().usual_fields())?;
        if self.format.dialect.merges() {
            return Err(Error::Invalid(MERGED.into()));
        }
        let rows = self.rows_of_width(header, fields, progress)?;
        let sep = self.format.dialect.delimiter_str();
        let mut first = Vec::new();
        for &(_, id) in rows.iter().take(SHOWN_CHANGES) {
            let mut cells = join_groups(&self.logical_values(id)?, groups, sep);
            let mut used = 0usize;
            cells.retain(|v| {
                used += v.len() + 1;
                used <= MAX_SHOWN
            });
            first.push(ShapeRow {
                id,
                file_row: if self.file_row(id) {
                    u64::from(id) + 1
                } else {
                    0
                },
                cells,
            });
        }
        let hidden = rows
            .iter()
            .filter(|&&(at, _)| self.shown_at(at).is_none())
            .count() as u64;
        Ok(SameShape {
            revision: self.revision(),
            total: rows.len() as u64,
            hidden,
            first,
        })
    }

    /// Join the fields `groups` of every row of exactly `fields` fields, but
    /// those in `skip`, as one undo step; the rows changed.
    pub fn fix_all(
        &mut self,
        header: bool,
        revision: u64,
        fields: u32,
        groups: &[u32],
        skip: &[u32],
        progress: &Progress,
    ) -> Result<u64, Error> {
        self.repairable()?;
        check_shape(groups, fields, self.index.stats().usual_fields())?;
        if self.format.dialect.merges() {
            return Err(Error::Invalid(MERGED.into()));
        }
        if revision != self.revision() {
            return Err(Error::Invalid(CHANGED.into()));
        }
        let first_joined = groups.iter().position(|&g| g > 1).unwrap_or(0);
        self.unprotected_from(first_joined, fields as usize)?;
        let skip: std::collections::HashSet<u32> = skip.iter().copied().collect();
        let sep = self.format.dialect.delimiter_str().to_string();
        let mut states = Vec::new();
        for (_, id) in self.rows_of_width(header, fields, progress)? {
            if skip.contains(&id) {
                continue;
            }
            let values = join_groups(&self.logical_values(id)?, groups, &sep);
            states.push(self.state_for(id, values)?);
        }
        let n = states.len() as u64;
        if n > 0 {
            self.push(Op::Restore(states));
        }
        Ok(n)
    }

    /// The columns past the usual number can be removed: none has a title or
    /// a value in any row, hidden ones too. Gives how many columns and the
    /// rows that hold such cells; refused with the reason otherwise.
    fn extra_columns(&self, header: bool, progress: &Progress) -> Result<(u32, Cut), Error> {
        self.repairable()?;
        let usual = self.index.stats().usual_fields();
        let cols = self.col_count();
        if cols <= usual {
            return Err(Error::Invalid(
                "the table has no column past the usual number".into(),
            ));
        }
        let total = self.total_rows();
        progress.start(total);
        let mut rows = Vec::new();
        for at in 0..total {
            let id = self.doc_id_at(at).ok_or(Error::OutOfRange)?;
            if self.width_of(id) > u64::from(usual) {
                let mut values = self.logical_values(id)?;
                if values[usual as usize..].iter().any(|v| !v.is_empty()) {
                    return Err(Error::Invalid(if header && at == 0 {
                        "a column past the usual number has a title".into()
                    } else {
                        "a column past the usual number holds values".into()
                    }));
                }
                values.truncate(usual as usize);
                rows.push((id, values));
            }
            if at % 8192 == 0 {
                progress.step(8192.min(total - at))?;
            }
        }
        Ok((cols - usual, rows))
    }

    /// How many empty, untitled columns past the usual number could be
    /// removed (0: none).
    pub fn empty_extra_columns(&self, header: bool, progress: &Progress) -> u32 {
        self.extra_columns(header, progress).map_or(0, |e| e.0)
    }

    /// Cut the empty cells past the usual number from every row that has
    /// them, as one undo step. Not a column delete: no column is moved, so
    /// ragged rows can still be repaired.
    pub fn remove_empty_extra_columns(
        &mut self,
        header: bool,
        revision: u64,
        progress: &Progress,
    ) -> Result<u32, Error> {
        if revision != self.revision() {
            return Err(Error::Invalid(CHANGED.into()));
        }
        let (n, rows) = self.extra_columns(header, progress)?;
        let usual = self.index.stats().usual_fields();
        self.unprotected_from(usual as usize, (usual + n) as usize)?;
        let mut states = Vec::with_capacity(rows.len());
        for (id, values) in rows {
            states.push((id, Some(Edit::Full(values))));
        }
        self.push(Op::Batch(vec![
            Op::Restore(states),
            Op::SetWidth(Some(usual)),
        ]));
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, SaveOptions};

    fn open_as(body: &[u8], ext: &str, opts: OpenOptions) -> (Document, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("nde-repair-{}-{n}.{ext}", std::process::id()));
        std::fs::write(&p, body).unwrap();
        (Document::open_indexed(&p, opts, None).unwrap(), p)
    }

    fn open(body: &str) -> (Document, std::path::PathBuf) {
        open_as(body.as_bytes(), "csv", OpenOptions::default())
    }

    fn all(d: &Document) -> Vec<Vec<String>> {
        (0..d.total_rows())
            .map(|p| d.logical_values(d.doc_id_at(p).unwrap()).unwrap())
            .collect()
    }

    fn saved(d: &mut Document, p: &std::path::Path) -> String {
        d.save(SaveOptions::default()).unwrap();
        std::fs::read_to_string(p).unwrap()
    }

    fn apply(d: &mut Document, id: u32, repair: Repair) -> Result<(), Error> {
        let revision = d.revision();
        d.repair_row(id, revision, &repair)
    }

    fn join(groups: &[u32]) -> Repair {
        Repair::Join {
            groups: groups.to_vec(),
        }
    }

    const BODY: &str = "id,name,city\n1,Smith, John,Rome\n2,ann,Oslo\n";

    #[test]
    fn shows_the_file_text_and_cells_of_a_row() {
        let (d, _) = open(BODY);
        let t = d.row_text(1, true).unwrap();
        assert_eq!(t.raw, "1,Smith, John,Rome");
        assert_eq!(t.fields, ["1", "Smith", " John", "Rome"]);
        assert_eq!((t.width, t.usual, t.edited, t.cut), (4, 3, false, false));
        assert_eq!(d.row_width(1).unwrap(), (4, 3));
        assert_eq!(d.row_width(2).unwrap(), (3, 3));
        let next = t.next.unwrap();
        assert_eq!((next.id, next.file_row, next.hidden), (2, 3, false));
        assert_eq!(
            d.row_text(2, true).unwrap().no_next,
            "this is the last row of the file"
        );
    }

    #[test]
    fn joined_fields_save_and_reopen_on_every_separator() {
        for (sep, ext) in [(",", "csv"), (";", "csv"), ("\t", "tsv"), ("|", "psv")] {
            let body = ["id", "name", "city"].join(sep)
                + "\n"
                + &["1", "Smith", " John", "Rome"].join(sep)
                + "\n"
                + &["2", "ann", "Oslo"].join(sep)
                + "\n";
            let opts = OpenOptions {
                dialect: Some(crate::Dialect::new(sep.as_bytes()[0], Some(b'"')).unwrap()),
                ..Default::default()
            };
            let (mut d, p) = open_as(body.as_bytes(), ext, opts);
            apply(&mut d, 1, join(&[1, 2])).unwrap();
            let joined = format!("Smith{sep} John");
            assert_eq!(all(&d)[1], ["1", joined.as_str(), "Rome"], "{sep:?}");
            let text = saved(&mut d, &p);
            assert!(text.ends_with(&format!("{}\n", ["2", "ann", "Oslo"].join(sep))));
            let again = Document::open_indexed(&p, opts, None).unwrap();
            assert_eq!(all(&again), all(&d), "{sep:?}");
        }
    }

    #[test]
    fn a_join_is_one_undo_step_and_refused_when_stale_protected_or_merged() {
        let (mut d, p) = open(BODY);
        let stale = d.revision();
        apply(&mut d, 1, join(&[1, 2, 1])).unwrap();
        assert!(d.undo());
        assert_eq!(saved(&mut d, &p), BODY);
        d.set_cell(2, 0, "20".into()).unwrap();
        assert!(d.repair_row(1, stale, &join(&[1, 2])).is_err());
        assert!(apply(&mut d, 1, join(&[0, 2])).is_err());
        assert!(apply(&mut d, 1, join(&[5])).is_err());
        d.set_protected(&[2]);
        let e = apply(&mut d, 1, join(&[1, 2])).unwrap_err();
        assert!(e.to_string().contains("protected"), "{e}");
        d.set_protected(&[0]);
        apply(&mut d, 1, join(&[1, 2])).unwrap();
        d.insert_cols(0, 1).unwrap();
        assert!(apply(&mut d, 1, join(&[2]))
            .unwrap_err()
            .to_string()
            .contains("removed or moved"));
    }

    #[test]
    fn joins_with_the_next_row_of_the_file_three_ways() {
        for (between, cell) in [
            (Between::Space, "Oslo North"),
            (Between::LineBreak, "Oslo\nNorth"),
            (Between::Nothing, "OsloNorth"),
        ] {
            let body = "id,name,city,x\n1,ann,Oslo\nNorth,7\n2,bo,Rome,8\n";
            let (mut d, p) = open(body);
            apply(&mut d, 1, Repair::Next { between }).unwrap();
            assert_eq!(all(&d)[1], ["1", "ann", cell, "7"], "{between:?}");
            assert_eq!(d.total_rows(), 3);
            let text = saved(&mut d, &p);
            let again = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
            assert_eq!(all(&again), all(&d), "{between:?} {text:?}");
        }
        // The break fell exactly between fields: nothing between gives them all back.
        let (mut d, p) = open("a,b,c,d\n1,2,\n3,4\n5,6,7,8\n");
        apply(
            &mut d,
            1,
            Repair::Next {
                between: Between::Nothing,
            },
        )
        .unwrap();
        assert!(d.undo());
        assert_eq!(d.total_rows(), 4);
        assert!(d.redo());
        assert_eq!(saved(&mut d, &p), "a,b,c,d\n1,2,3,4\n5,6,7,8\n");
    }

    #[test]
    fn next_means_the_next_file_row_even_when_sorted_or_edited() {
        let body = "a,b\n1\n2,x\n3,y\n";
        let (mut d, _) = open(body);
        d.set_cell(2, 1, "edited".into()).unwrap();
        let t = d.row_text(1, true).unwrap();
        assert!(t.next.as_ref().unwrap().edited);
        d.move_rows(2, 1, 3).unwrap();
        apply(
            &mut d,
            1,
            Repair::Next {
                between: Between::Space,
            },
        )
        .unwrap();
        assert_eq!(all(&d)[1], ["1 2", "edited"]);
        // The next row removed: refused, never a later one.
        let (mut d, _) = open(body);
        d.delete_rows(2, 1).unwrap();
        assert_eq!(
            d.row_text(1, true).unwrap().no_next,
            "the next row of the file was removed"
        );
        assert!(apply(
            &mut d,
            1,
            Repair::Next {
                between: Between::Space
            }
        )
        .is_err());
        // A row added by hand has no next row in the file.
        d.insert_rows(1, vec![vec!["new".into()]]).unwrap();
        let id = d.doc_id_at(1).unwrap();
        assert!(d
            .row_text(id, true)
            .unwrap()
            .no_next
            .contains("added by hand"));
    }

    #[test]
    fn a_quote_treated_as_text_saves_and_reopens_the_same() {
        let body = "a,b,c\n1,\"x, y\",3\n4,5,6\n";
        let (mut d, p) = open(body);
        let t = d.row_text(1, true).unwrap();
        assert_eq!(t.quoted_as_text, ["1", "\"x", " y\"", "3"]);
        apply(&mut d, 1, Repair::QuoteAsText).unwrap();
        assert_eq!(all(&d)[1], t.quoted_as_text);
        saved(&mut d, &p);
        let again = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        assert_eq!(all(&again), all(&d));
        // Not offered once the row was changed.
        assert!(d.row_text(2, true).unwrap().quoted_as_text.is_empty());
    }

    #[test]
    fn a_very_long_row_is_cut_before_it_is_shown() {
        let long = "x".repeat(MAX_SHOWN);
        let body = format!("a,b\n1,{long},tail\n2,3\n");
        let (mut d, _) = open(&body);
        let t = d.row_text(1, true).unwrap();
        assert!(t.cut && t.raw.len() <= MAX_SHOWN && t.fields.len() == 1 && t.width == 3);
        assert!(apply(&mut d, 1, join(&[3])).is_err());
        // Fields before the cut can still be joined.
        apply(&mut d, 1, join(&[1])).unwrap();
    }

    #[test]
    fn joins_with_the_previous_row_of_the_file_three_ways() {
        for (between, cell) in [
            (Between::Space, "Os lo"),
            (Between::LineBreak, "Os\nlo"),
            (Between::Nothing, "Oslo"),
        ] {
            // The line broke inside its last field: the first half has the usual width.
            let body = "id,name,city\n1,ann,Os\nlo\n2,bo,Rome\n";
            let (mut d, p) = open(body);
            let t = d.row_text(2, true).unwrap();
            let previous = t.previous.unwrap();
            assert_eq!((previous.id, previous.file_row), (1, 2));
            let repair = Repair::Previous {
                between,
                header: true,
            };
            apply(&mut d, 2, repair).unwrap();
            assert_eq!(all(&d)[1], ["1", "ann", cell], "{between:?}");
            assert_eq!(d.total_rows(), 3);
            assert!(d.undo());
            assert_eq!(d.total_rows(), 4);
            assert_eq!(all(&d)[1], ["1", "ann", "Os"]);
            assert!(d.redo());
            let text = saved(&mut d, &p);
            let again = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
            assert_eq!(all(&again), all(&d), "{between:?} {text:?}");
        }
    }

    #[test]
    fn the_previous_row_must_be_there_and_never_the_titles() {
        let previous = Repair::Previous {
            between: Between::Space,
            header: true,
        };
        let body = "a,b\n1\n2\n3,y\n";
        let (mut d, _) = open(body);
        assert!(d
            .row_text(1, true)
            .unwrap()
            .no_previous
            .contains("column names"));
        assert!(apply(&mut d, 1, previous.clone()).is_err());
        // Without a title row the first row is a row like any other.
        assert!(d.row_text(1, false).unwrap().previous.is_some());
        assert!(d
            .row_text(0, false)
            .unwrap()
            .no_previous
            .contains("first row"));
        d.delete_rows(1, 1).unwrap();
        assert_eq!(
            d.row_text(2, true).unwrap().no_previous,
            "the previous row of the file was removed"
        );
        assert!(apply(&mut d, 2, previous.clone()).is_err());
        d.insert_rows(2, vec![vec!["new".into()]]).unwrap();
        let id = d.doc_id_at(2).unwrap();
        assert!(d
            .row_text(id, true)
            .unwrap()
            .no_previous
            .contains("added by hand"));
        d.set_protected(&[0]);
        assert!(apply(&mut d, 3, previous)
            .unwrap_err()
            .to_string()
            .contains("protected"));
    }

    #[test]
    fn a_row_longer_than_the_bar_shows_is_not_joined_to_a_neighbour() {
        let long = "x".repeat(MAX_SHOWN + 10);
        let (mut d, _) = open(&format!("a,b\n1,{long}\n2\n3,{long}\n"));
        for repair in [
            Repair::Previous {
                between: Between::Space,
                header: true,
            },
            Repair::Next {
                between: Between::Space,
            },
        ] {
            let e = apply(&mut d, 2, repair).unwrap_err();
            assert!(e.to_string().contains("longer than"), "{e}");
        }
    }

    #[test]
    fn rows_are_found_after_a_descending_sort_and_under_a_filter() {
        let (mut d, _) = open("n,x\n1,a\n2\n3,c\n4,d\n");
        d.move_rows(1, 1, 4).unwrap();
        let t = d.row_text(2, true).unwrap();
        assert_eq!((t.row, t.previous.unwrap().id), (Some(1), 1));
        let found = d.ragged_rows(true, &Progress::new()).unwrap();
        let plan = d
            .plan_show_ragged(&found, true, crate::document::ragged::RaggedPick::All)
            .unwrap();
        d.apply_rows(plan).unwrap();
        let t = d.row_text(2, true).unwrap();
        assert_eq!(t.row, Some(1));
        assert!(t.previous.unwrap().hidden);
    }

    #[test]
    fn fix_all_changes_only_rows_of_the_same_shape() {
        let body = "id,name,city\n1,Smith, John,Rome\n2,ann,Oslo\n3,Doe, Jane,Lima\n4\n5,a,b,c,d\n6,Roe, Bo,Kiel\n7,x,y\n8,x,y\n9,x,y\n";
        let (mut d, p) = open(body);
        let progress = Progress::new();
        let same = d.same_shape(true, 4, &[1, 2], &progress).unwrap();
        assert_eq!((same.total, same.hidden), (3, 0));
        assert_eq!(same.first[0].cells, ["1", "Smith, John", "Rome"]);
        // A sort between the preview and Apply moves rows, never the change: the old reading is refused.
        d.move_rows(1, 1, 4).unwrap();
        assert!(d
            .fix_all(true, same.revision, 4, &[1, 2], &[], &progress)
            .is_err());
        let same = d.same_shape(true, 4, &[1, 2], &progress).unwrap();
        let skipped = same.first.iter().find(|r| r.file_row == 4).unwrap().id;
        let n = d
            .fix_all(true, same.revision, 4, &[1, 2], &[skipped], &progress)
            .unwrap();
        assert_eq!(n, 2);
        let rows = all(&d);
        assert!(rows.iter().any(|r| r == &["6", "Roe, Bo", "Kiel"]));
        assert!(
            rows.iter().any(|r| r == &["3", "Doe", " Jane", "Lima"]),
            "unticked"
        );
        assert!(rows.iter().any(|r| r.len() == 5) && rows.iter().any(|r| r == &["4"]));
        assert!(d.undo());
        assert!(d.undo());
        assert_eq!(saved(&mut d, &p), body);
        // Never the rows of usual width, nor without a join.
        assert!(d.same_shape(true, 3, &[2], &progress).is_err());
        assert!(d.same_shape(true, 4, &[1, 1], &progress).is_err());
        d.insert_cols(0, 1).unwrap();
        assert!(d.same_shape(true, 4, &[2], &progress).is_err());
    }

    #[test]
    fn the_empty_column_goes_by_cutting_cells() {
        let body = "a,b,c\n1,Smith, John,3\n4,5,6\n7,8,9,\n";
        let (mut d, p) = open(body);
        let progress = Progress::new();
        assert_eq!(d.col_count(), 4);
        // A value in the extra column (the unrepaired row) keeps it.
        assert_eq!(d.empty_extra_columns(true, &progress), 0);
        apply(&mut d, 1, join(&[1, 2])).unwrap();
        assert_eq!(d.empty_extra_columns(true, &progress), 1);
        // A title typed this session keeps it too, hidden rows or not.
        d.set_cell(0, 3, "extra".into()).unwrap();
        assert_eq!(d.empty_extra_columns(true, &progress), 0);
        assert!(d.undo());
        let revision = d.revision();
        assert_eq!(
            d.remove_empty_extra_columns(true, revision, &progress)
                .unwrap(),
            1
        );
        assert_eq!(d.col_count(), 3);
        assert!(
            !d.has_column_changes(),
            "no column map: repair keeps working"
        );
        assert!(d.undo());
        assert_eq!(d.col_count(), 4);
        assert!(d.redo());
        assert_eq!(d.col_count(), 3);
        assert_eq!(
            saved(&mut d, &p),
            "a,b,c\n1,\"Smith, John\",3\n4,5,6\n7,8,9\n"
        );
        assert_eq!(d.col_count(), 3);
    }
}
