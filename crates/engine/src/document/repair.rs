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
}

const MERGED: &str =
    "this file treats runs of separators as one, so the original separator between two fields is not known";

impl Document {
    /// Where row `id` is in the whole table, shown or not. Looked for outward
    /// from its own number: rows rarely move far.
    fn position_of(&self, id: u32) -> Option<u64> {
        let total = self.total_rows();
        let at = u64::from(id).min(total.saturating_sub(1));
        let is = |p: u64| self.doc_id_at(p) == Some(id);
        for off in 0..total {
            let below = at.checked_sub(off);
            let above = at + off;
            if below.is_none() && above >= total {
                break;
            }
            if let Some(p) = below.filter(|&p| is(p)) {
                return Some(p);
            }
            if above < total && is(above) {
                return Some(above);
            }
        }
        None
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

    /// Where row `id` is on screen, looked for outward from `near` (a row
    /// is never below its place in the whole table).
    fn shown_at(&self, id: u32, near: u64) -> Option<u64> {
        let total = self.row_count();
        if total == self.total_rows() {
            return Some(near);
        }
        let from = near.min(total.saturating_sub(1));
        (0..=from)
            .rev()
            .chain(from + 1..total)
            .find(|&r| self.id_at(r) == Some(id))
    }

    /// The id of shown row `row` (kept by the row through sorts and
    /// deletes).
    pub fn row_id(&self, row: u64) -> Option<u32> {
        self.id_at(row)
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

    /// What the repair bar shows for row `id`.
    pub fn row_text(&self, id: u32) -> Result<RowText, Error> {
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
            Ok((n, next_at)) => {
                let mut fields = self.logical_values(n)?;
                let mut used = 0usize;
                fields.retain(|v| {
                    used += v.len() + 1;
                    used <= MAX_SHOWN
                });
                (
                    Some(NextRow {
                        id: n,
                        file_row: u64::from(n) + 1,
                        edited: self.overlay.get(n).is_some(),
                        hidden: self.shown_at(n, next_at).is_none(),
                        fields,
                    }),
                    String::new(),
                )
            }
            Err(why) => (None, why.to_string()),
        };
        Ok(RowText {
            id,
            row: self.shown_at(id, at),
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
                let from = width.saturating_sub(1);
                self.unprotected_from(from, width + after.len())?;
                let mut out = values;
                let mut after = after.into_iter();
                let head = after.next().unwrap_or_default();
                match out.last_mut() {
                    Some(last) => {
                        last.push_str(between.text());
                        last.push_str(&head);
                    }
                    None => out.push(head),
                }
                out.extend(after);
                if out.len() as u64 > MAX_ROW_FIELDS {
                    return Err(Error::Invalid("the joined row would be too wide".into()));
                }
                let state = self.state_for(id, out)?;
                // The row first: its position does not matter to a cell state.
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
        let t = d.row_text(1).unwrap();
        assert_eq!(t.raw, "1,Smith, John,Rome");
        assert_eq!(t.fields, ["1", "Smith", " John", "Rome"]);
        assert_eq!((t.width, t.usual, t.edited, t.cut), (4, 3, false, false));
        let next = t.next.unwrap();
        assert_eq!((next.id, next.file_row, next.hidden), (2, 3, false));
        assert_eq!(
            d.row_text(2).unwrap().no_next,
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
        let t = d.row_text(1).unwrap();
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
            d.row_text(1).unwrap().no_next,
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
        assert!(d.row_text(id).unwrap().no_next.contains("added by hand"));
    }

    #[test]
    fn a_quote_treated_as_text_saves_and_reopens_the_same() {
        let body = "a,b,c\n1,\"x, y\",3\n4,5,6\n";
        let (mut d, p) = open(body);
        let t = d.row_text(1).unwrap();
        assert_eq!(t.quoted_as_text, ["1", "\"x", " y\"", "3"]);
        apply(&mut d, 1, Repair::QuoteAsText).unwrap();
        assert_eq!(all(&d)[1], t.quoted_as_text);
        saved(&mut d, &p);
        let again = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        assert_eq!(all(&again), all(&d));
        // Not offered once the row was changed.
        assert!(d.row_text(2).unwrap().quoted_as_text.is_empty());
    }

    #[test]
    fn a_very_long_row_is_cut_before_it_is_shown() {
        let long = "x".repeat(MAX_SHOWN);
        let body = format!("a,b\n1,{long},tail\n2,3\n");
        let (mut d, _) = open(&body);
        let t = d.row_text(1).unwrap();
        assert!(t.cut && t.raw.len() <= MAX_SHOWN && t.fields.len() == 1 && t.width == 3);
        assert!(apply(&mut d, 1, join(&[3])).is_err());
        // Fields before the cut can still be joined.
        apply(&mut d, 1, join(&[1])).unwrap();
    }
}
