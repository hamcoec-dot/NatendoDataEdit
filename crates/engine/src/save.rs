//! Saving: stream rows to a temp file, flush it to disk, read it back and
//! check every row, and only then atomically replace the target.
//!
//! Unedited rows, and unedited cells of edited rows, are copied
//! byte-for-byte when the encoding and dialect stay the same. Rows whose
//! quote was never closed are rewritten properly closed, so the saved file
//! reads back the same way. Nothing that could not be decoded is ever
//! replaced silently: that needs `SaveOptions::allow_lossy`. The original is
//! untouched until the final replace; on Windows the replace keeps its
//! permissions (ACL) and attributes, and if the replace fails half-way the
//! saved text is moved into place or kept, never deleted.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::dialect::{Dialect, LineBreak};
use crate::document::report::{self, Place};
use crate::document::{Document, Format, OpenOptions, RowSource, Start};
use crate::encoding::{self, Encoding};
use crate::index::RowIndex;
use crate::scan::{self, Mode, Quoting};
use crate::source::Source;
use crate::Error;

use rayon::prelude::*;

/// Rows serialized together in parallel before being written in order.
const SAVE_CHUNK: usize = 1 << 14;

/// A rewritten row made ahead of the write (see `Document::write_rows`).
struct Made {
    text: String,
    bytes: Vec<u8>,
    /// The original row's line break, if it had one and the save keeps them.
    own: Option<LineBreak>,
    /// The row holds one empty value.
    empty: bool,
}

/// What to save and how.
#[derive(Debug, Clone, Default)]
pub struct SaveOptions {
    /// Save to another file ("Save As"). `None` saves over the open file.
    pub path: Option<PathBuf>,
    /// New encoding/BOM/dialect/line break. `None` keeps the current format.
    pub format: Option<Format>,
    /// Give every row the format's line break, not only new and edited rows.
    pub convert_line_breaks: bool,
    /// Save even though another program changed the file since it was opened.
    pub overwrite_changed: bool,
    /// Write the file even when nothing changed.
    pub rewrite: bool,
    /// Allow bytes that could not be decoded to be written as "�" (or `?` in
    /// legacy encodings). Only after the user agrees.
    pub allow_lossy: bool,
    /// How values are quoted. Anything but `Keep` rewrites every row.
    pub quoting: Quoting,
}

/// What a save did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    pub path: PathBuf,
    /// Nothing changed, so nothing was written.
    pub skipped: bool,
    pub bytes: u64,
    pub rows: u64,
    /// Rows copied byte-for-byte from the original.
    pub raw_rows: u64,
    /// The written file was read back and every row matched before it
    /// replaced the original (always true for a successful save).
    pub verified: bool,
}

#[derive(Default)]
struct Counts {
    bytes: u64,
    rows: u64,
    raw_rows: u64,
    /// Lines written above the table (report notes).
    head_lines: u32,
    /// Bytes written below the table, as the file reads back (decoded).
    tail: usize,
    /// The lines above and below, decoded, as written.
    head_text: String,
    tail_text: String,
}

struct CountingWriter<W> {
    inner: W,
    n: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(b)?;
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Settings shared by every row of one write.
struct Plan {
    target: Format,
    allow_lossy: bool,
    /// Same direct encoding and dialect: raw bytes can be copied.
    raw_ok: bool,
    same_dialect: bool,
    /// Lossy save to a legacy encoding: U+FFFD becomes `?`.
    fffd_as_question: bool,
    quoting: Quoting,
}

fn line_break_str(b: LineBreak) -> &'static str {
    match b {
        LineBreak::CrLf => "\r\n",
        LineBreak::Lf => "\n",
        LineBreak::Cr => "\r",
    }
}

impl Document {
    /// Write the document in `target` format to `out`. Refuses to replace
    /// undecodable bytes (see [`SaveOptions::allow_lossy`]).
    pub fn write_to(
        &self,
        out: &mut impl Write,
        target: Format,
        convert_line_breaks: bool,
    ) -> Result<u64, Error> {
        self.write_quoted(out, target, convert_line_breaks, Quoting::Keep)
    }

    /// Write the document as it would be saved now, in its own format, to
    /// `out` (recovery copies). Unlike [`Document::write_to`] it also writes
    /// documents whose undecodable bytes were replaced, so unsaved work in
    /// them can still be recovered.
    pub fn write_copy(&self, out: &mut impl Write) -> Result<u64, Error> {
        let mut w = CountingWriter { inner: out, n: 0 };
        self.write_rows(&mut w, self.format, false, true, Quoting::Keep, true)?;
        Ok(w.n)
    }

    /// [`Document::write_to`] with a quoting style.
    pub fn write_quoted(
        &self,
        out: &mut impl Write,
        target: Format,
        convert_line_breaks: bool,
        quoting: Quoting,
    ) -> Result<u64, Error> {
        let mut w = CountingWriter { inner: out, n: 0 };
        self.write_rows(&mut w, target, convert_line_breaks, false, quoting, false)?;
        Ok(w.n)
    }

    fn write_rows(
        &self,
        w: &mut impl Write,
        target: Format,
        convert: bool,
        allow_lossy: bool,
        quoting: Quoting,
        // Report notes even while the switch says leave them out (recovery
        // copies keep everything).
        all_notes: bool,
    ) -> Result<Counts, Error> {
        if !self.is_indexed() {
            return Err(Error::StillLoading);
        }
        if self.lossy && !allow_lossy {
            return Err(Error::Lossy { row: None });
        }
        let src = self.format;
        // A new quoting style rewrites every row field by field.
        let keep = quoting == Quoting::Keep;
        let plan = Plan {
            target,
            allow_lossy,
            raw_ok: keep
                && src.encoding.is_direct()
                && target.encoding == src.encoding
                && target.dialect == src.dialect,
            same_dialect: keep && target.dialect == src.dialect,
            fffd_as_question: allow_lossy && matches!(target.encoding, Encoding::Legacy(_)),
            quoting,
        };
        let original_rows = self.index.rows() as u32;
        let head = self.note_bytes(Place::Head, all_notes);
        let foot = self.note_bytes(Place::Foot, all_notes);
        // The last row ends with a line break if the file's last row did, or
        // when lines follow below the table.
        let ends_with_break = self.index.stats().unterminated_rows == 0 || !foot.is_empty();
        let n = self.total_rows();
        let mut counts = Counts::default();
        let mut text = String::new();
        let mut bytes = Vec::new();
        // The previous row ended in a lone CR (see `guard_empty_row`).
        let mut prev_cr = false;
        // Pending run of consecutive raw rows, written in one call.
        let mut run: Option<(usize, usize)> = None;
        let data = self.source.bytes();

        if target.bom {
            w.write_all(target.encoding.bom())?;
        }
        // The lines above the table (report notes), unchanged (re-encoded if
        // need be).
        if !head.is_empty() {
            let mut head = head;
            if (n > 0 || !foot.is_empty()) && LineBreak::at_end_of(&head).is_none() {
                head.extend_from_slice(self.format.line_break.bytes());
            }
            let (out, _, text) = self.note_out(&head, &plan, src)?;
            counts.head_lines = report::lines_in(&head) as u32;
            counts.head_text = text;
            w.write_all(&out)?;
        }
        // Columns were inserted, deleted or moved: every row is rewritten
        // through the column map. Unedited rows in the same encoding and
        // dialect reuse their raw field bytes in map order (fast, and keeps
        // every byte of every kept cell); other rows go through their values.
        let map = self.col_map();
        let row_pos = |e: Error, pos: u64| match e {
            Error::RowTooWide { fields, .. } => Error::RowTooWide { row: pos, fields },
            e => e,
        };
        let mut spans: Vec<(usize, usize)> = Vec::new();
        // Rows rewritten from their values (edited rows, and with a column
        // map every row that cannot reuse its raw bytes) are made in
        // parallel, a chunk at a time, then written in order.
        let rewrite = |id: u32| match map {
            Some(_) => !(plan.raw_ok && matches!(self.row_source(id), RowSource::Raw { .. })),
            None => matches!(self.row_source(id), RowSource::Edited(_)),
        };
        let make = |pos: u64, id: u32| -> Result<Made, Error> {
            let values: std::borrow::Cow<[String]> = match (map, self.row_source(id)) {
                (Some(_), _) => {
                    std::borrow::Cow::Owned(self.logical_values(id).map_err(|e| row_pos(e, pos))?)
                }
                (None, RowSource::Edited(v)) => v,
                (None, RowSource::Raw { .. }) => unreachable!("chosen by rewrite"),
            };
            let orig = (id < original_rows).then(|| {
                let (a, b) = self.index.row_range(id as u64).expect("original row");
                let raw = &data[a..b];
                let own = LineBreak::at_end_of(raw);
                (
                    &raw[..raw.len() - own.map_or(0, |l| l.bytes().len())],
                    self.index.mode_of(id as u64),
                    own,
                )
            });
            let mut m = Made {
                text: String::new(),
                bytes: Vec::new(),
                own: orig.and_then(|o| o.2).filter(|_| !convert),
                // An empty row needs its line break, or it would vanish.
                empty: values.len() == 1 && values[0].is_empty(),
            };
            self.serialize(
                &values,
                orig.map(|o| (o.0, o.1)),
                map,
                &plan,
                pos,
                &mut m.text,
                &mut m.bytes,
            )?;
            Ok(m)
        };
        let mut ids = self.ids();
        let mut chunk: Vec<u32> = Vec::with_capacity(SAVE_CHUNK);
        let mut made: Vec<Option<Result<Made, Error>>> = Vec::new();
        let mut at = 0usize;
        for pos in 0..n {
            if at == chunk.len() {
                chunk.clear();
                chunk.extend(ids.by_ref().take(SAVE_CHUNK));
                if chunk.is_empty() {
                    break;
                }
                made = chunk
                    .par_iter()
                    .enumerate()
                    .map(|(k, &id)| rewrite(id).then(|| make(pos + k as u64, id)))
                    .collect();
                at = 0;
            }
            let id = chunk[at];
            let ready = made[at].take();
            at += 1;
            let last = pos + 1 == n;
            counts.rows += 1;
            text.clear();
            bytes.clear();
            let mut brk: Option<LineBreak>;
            let source = match (&ready, map) {
                (Some(_), _) => Some(RowSource::Edited(std::borrow::Cow::Borrowed(&[]))),
                (None, Some(_)) => None,
                (None, None) => Some(self.row_source(id)),
            };
            match source {
                None => {
                    let (Some(m), RowSource::Raw { bytes: raw, mode }) = (map, self.row_source(id))
                    else {
                        unreachable!("chosen above")
                    };
                    let own_break = LineBreak::at_end_of(raw);
                    let content = &raw[..raw.len() - own_break.map_or(0, |l| l.bytes().len())];
                    self.field_spans_into(id, content, mode, &mut spans)
                        .map_err(|e| row_pos(e, pos))?;
                    // A quote that never closed is the row's last field;
                    // wherever it moves, it gets its closing quote.
                    let unclosed = self.unclosed(content, mode);
                    let d = target.dialect;
                    let mut prev = None;
                    for (i, &p) in m.iter().enumerate() {
                        let p = p as usize;
                        put_gap(&mut bytes, content, &spans, prev, p, d);
                        prev = Some(p);
                        match spans.get(p) {
                            Some(&(s, e)) if s < e || !d.merges() => {
                                let raw = &content[s..e];
                                if i + 1 < m.len() && scan::joins_separator(raw, d) {
                                    // `x|` before `||` would read as `x`, `|...`.
                                    let value = scan::unquote(raw, d);
                                    if !scan::quote_raw(&value, d, &mut bytes) {
                                        return Err(Error::CannotRepresent {
                                            row: pos,
                                            col: i as u32,
                                        });
                                    }
                                    continue;
                                }
                                bytes.extend_from_slice(raw);
                                if unclosed && p + 1 == spans.len() {
                                    scan::close_unclosed(raw, d, &mut bytes);
                                }
                            }
                            // An empty cell between merged separators must be quoted.
                            _ if d.merges() => match d.quote() {
                                Some(q) => bytes.extend_from_slice(&[q, q]),
                                None => {
                                    return Err(Error::CannotRepresent {
                                        row: pos,
                                        col: i as u32,
                                    })
                                }
                            },
                            _ => {}
                        }
                    }
                    put_tail(&mut bytes, content, &spans, prev);
                    let own = own_break.filter(|_| !convert);
                    brk = if last && !ends_with_break && !bytes.is_empty() {
                        None
                    } else {
                        Some(own.unwrap_or(target.line_break))
                    };
                }
                Some(RowSource::Raw { bytes: raw, mode }) => {
                    let own = LineBreak::at_end_of(raw);
                    let content = &raw[..raw.len() - own.map_or(0, |l| l.bytes().len())];
                    brk = match (own, convert, last) {
                        (None, _, true) => None,
                        (None, _, false) | (Some(_), true, _) => Some(target.line_break),
                        (Some(l), false, _) => Some(l),
                    };
                    let unclosed = self.unclosed(content, mode);
                    if plan.raw_ok && unclosed {
                        // The unclosed field is always the row's last: its raw
                        // bytes plus a closing quote hold exactly its value,
                        // and nothing needs decoding.
                        if let Some((s, e)) = run.take() {
                            w.write_all(&data[s..e])?;
                        }
                        prev_cr = brk == Some(LineBreak::Cr);
                        w.write_all(content)?;
                        let mut close = Vec::new();
                        scan::close_unclosed(content, self.format.dialect, &mut close);
                        w.write_all(&close)?;
                        if let Some(b) = brk {
                            w.write_all(b.bytes())?;
                        }
                        continue;
                    }
                    if plan.raw_ok {
                        let pre = guard_empty_row(prev_cr, content.is_empty(), &mut brk, target);
                        prev_cr = brk == Some(LineBreak::Cr);
                        counts.raw_rows += 1;
                        if pre.is_none() && brk == own {
                            let (a, b) = self.index.row_range(id as u64).expect("original row");
                            run = match run {
                                Some((s, e)) if e == a => Some((s, b)),
                                other => {
                                    if let Some((s, e)) = other {
                                        w.write_all(&data[s..e])?;
                                    }
                                    Some((a, b))
                                }
                            };
                        } else {
                            if let Some((s, e)) = run.take() {
                                w.write_all(&data[s..e])?;
                            }
                            if let Some(q) = pre {
                                w.write_all(&[q, q])?;
                            }
                            w.write_all(content)?;
                            if let Some(b) = brk {
                                w.write_all(b.bytes())?;
                            }
                        }
                        continue;
                    }
                    if plan.same_dialect && !unclosed {
                        // Structural bytes are ASCII in every working encoding,
                        // so decoding the whole row keeps its quoting as is.
                        let (t, bad) =
                            encoding::decode_direct_checked(content, self.working_encoding());
                        if bad && !allow_lossy {
                            return Err(Error::Lossy { row: Some(pos) });
                        }
                        text.push_str(&t);
                    } else {
                        // New dialect, or a quote that never closed: rewrite
                        // field by field (streamed, so wide rows cost nothing).
                        self.stream_fields(content, mode, &plan, pos, &mut text)?;
                    }
                }
                Some(RowSource::Edited(_)) => {
                    let m = ready.expect("made above")?;
                    brk = if last && !ends_with_break && !m.empty {
                        None
                    } else {
                        Some(m.own.unwrap_or(target.line_break))
                    };
                    text = m.text;
                    bytes = m.bytes;
                }
            }
            if let Some((s, e)) = run.take() {
                w.write_all(&data[s..e])?;
            }
            let empty = text.is_empty() && bytes.is_empty();
            if empty && brk.is_none() {
                // A last row with nothing in it would vanish without its break.
                brk = Some(target.line_break);
            }
            if let Some(q) = guard_empty_row(prev_cr, empty, &mut brk, target) {
                text.push(q as char);
                text.push(q as char);
            }
            prev_cr = brk == Some(LineBreak::Cr);
            if !bytes.is_empty() {
                // Raw-reuse path: already in the target encoding.
                if let Some(b) = brk {
                    bytes.extend_from_slice(b.bytes());
                }
                w.write_all(&bytes)?;
                continue;
            }
            if let Some(b) = brk {
                text.push_str(line_break_str(b));
            }
            if plan.fffd_as_question && text.contains('\u{FFFD}') {
                text = text.replace('\u{FFFD}', "?");
            }
            encoding::encode(&text, target.encoding, &mut bytes).map_err(|u| {
                Error::Unmappable {
                    row: pos,
                    ch: u.0,
                    encoding: target.encoding.name(),
                }
            })?;
            w.write_all(&bytes)?;
        }
        if let Some((s, e)) = run.take() {
            w.write_all(&data[s..e])?;
        }
        // The lines below the table, after the last row whatever the order.
        if !foot.is_empty() {
            let (out, tail, text) = self.note_out(&foot, &plan, src)?;
            counts.tail = tail;
            counts.tail_text = text;
            w.write_all(&out)?;
        }
        Ok(counts)
    }

    /// Report-note bytes (working encoding) in the target encoding, with the
    /// length they read back as (decoded) and their text.
    fn note_out(
        &self,
        b: &[u8],
        plan: &Plan,
        src: Format,
    ) -> Result<(Vec<u8>, usize, String), Error> {
        let target = plan.target;
        let (text, bad) = encoding::decode_direct_checked(b, self.working_encoding());
        let mut text = text.into_owned();
        // The same bytes in the same encoding: written as they are (bytes
        // that do not decode included, as for the table's own rows).
        if plan.raw_ok || (src.encoding.is_direct() && target.encoding == src.encoding) {
            return Ok((b.to_vec(), b.len(), text));
        }
        if bad && !plan.allow_lossy {
            return Err(Error::Lossy { row: None });
        }
        if plan.fffd_as_question && text.contains('\u{FFFD}') {
            text = text.replace('\u{FFFD}', "?");
        }
        let mut out = Vec::new();
        encoding::encode(&text, target.encoding, &mut out).map_err(|u| Error::Unmappable {
            row: 0,
            ch: u.0,
            encoding: target.encoding.name(),
        })?;
        let back = if target.encoding.is_direct() {
            out.len()
        } else {
            text.len()
        };
        Ok((out, back, text))
    }

    /// True for a line-bound row whose quote never closed.
    fn unclosed(&self, content: &[u8], mode: Mode) -> bool {
        mode == Mode::LineBound
            && scan::scan_row(content, 0, self.format.dialect, mode, |_, _| true).repairs
                & scan::UNCLOSED_QUOTE
                != 0
    }

    /// Re-serialise a raw row's fields in the target dialect, one at a time.
    fn stream_fields(
        &self,
        content: &[u8],
        mode: Mode,
        plan: &Plan,
        pos: u64,
        text: &mut String,
    ) -> Result<(), Error> {
        let d = plan.target.dialect;
        let mut col = 0u32;
        let mut err = None;
        scan::scan_row(content, 0, self.format.dialect, mode, |s, e| {
            let v = scan::unquote(&content[s..e], self.format.dialect);
            let (v, bad) = encoding::decode_direct_checked(&v, self.working_encoding());
            if bad && !plan.allow_lossy {
                err = Some(Error::Lossy { row: Some(pos) });
                return false;
            }
            if scan::unrepresentable(&v, d) {
                err = Some(Error::CannotRepresent { row: pos, col });
                return false;
            }
            if col > 0 {
                text.push_str(d.delimiter_str());
            }
            scan::write_field_as(&v, d, plan.quoting, text);
            col += 1;
            true
        });
        err.map_or(Ok(()), Err)
    }

    /// Serialise an edited row. With raw copying allowed, unchanged cells
    /// reuse the original bytes (written to `bytes`); otherwise the row is
    /// built as text in `text` after checking the original decodes cleanly.
    #[allow(clippy::too_many_arguments)]
    fn serialize(
        &self,
        values: &[String],
        orig: Option<(&[u8], Mode)>,
        // With a column map, value `i` came from the original's field `map[i]`.
        map: Option<&[u32]>,
        plan: &Plan,
        pos: u64,
        text: &mut String,
        bytes: &mut Vec<u8>,
    ) -> Result<(), Error> {
        let d = plan.target.dialect;
        for (col, v) in values.iter().enumerate() {
            if scan::unrepresentable(v, d) {
                return Err(Error::CannotRepresent {
                    row: pos,
                    col: col as u32,
                });
            }
        }
        if values.len() == 1 && values[0].is_empty() {
            return Ok(()); // an empty row
        }
        if plan.raw_ok {
            // An unclosed quote can only be in the original's last field; its
            // raw bytes are reused with a closing quote added.
            let unclosed = orig.is_some_and(|(c, m)| self.unclosed(c, m));
            let mut spans = Vec::new();
            if let Some((content, mode)) = orig {
                scan::scan_row(content, 0, self.format.dialect, mode, |s, e| {
                    spans.push((s, e));
                    true
                });
            }
            let content = orig.map_or(&b""[..], |(c, _)| c);
            let mut cell = String::new();
            let mut prev = None;
            for (col, v) in values.iter().enumerate() {
                let p = map.map_or(col, |m| m[col] as usize);
                // The original bytes between neighbouring cells (lined-up
                // spacing) are kept; elsewhere one separator.
                put_gap(bytes, content, &spans, prev, p, d);
                prev = Some(p);
                if let Some(&(s, e)) = spans.get(p) {
                    let raw = &content[s..e];
                    // (An empty raw cell would vanish between merged
                    // separators; one ending in part of a longer separator
                    // would join it.)
                    if self.decode_field(raw) == *v
                        && !(raw.is_empty() && d.merges())
                        && !(col + 1 < values.len() && scan::joins_separator(raw, d))
                    {
                        bytes.extend_from_slice(raw);
                        if unclosed && p + 1 == spans.len() {
                            scan::close_unclosed(raw, d, bytes);
                        }
                        continue;
                    }
                }
                cell.clear();
                scan::write_field(v, d, &mut cell);
                encoding::encode(&cell, plan.target.encoding, bytes).map_err(|u| {
                    Error::Unmappable {
                        row: pos,
                        ch: u.0,
                        encoding: plan.target.encoding.name(),
                    }
                })?;
            }
            put_tail(bytes, content, &spans, prev);
            return Ok(());
        }
        if let Some((content, _)) = orig {
            if !plan.allow_lossy
                && encoding::decode_direct_checked(content, self.working_encoding()).1
            {
                return Err(Error::Lossy { row: Some(pos) });
            }
        }
        for (col, v) in values.iter().enumerate() {
            if col > 0 {
                text.push_str(d.delimiter_str());
            }
            scan::write_field_as(v, d, plan.quoting, text);
        }
        Ok(())
    }

    /// Save the document. See [`SaveOptions`].
    pub fn save(&mut self, opts: SaveOptions) -> Result<SaveReport, Error> {
        if !self.is_indexed() {
            return Err(Error::StillLoading);
        }
        let target = match opts
            .path
            .clone()
            .or_else(|| self.path().map(Path::to_path_buf))
        {
            Some(p) => p,
            None => return Err(Error::NoPath),
        };
        let format = opts.format.unwrap_or(self.format);
        let same_file = self.path().is_some_and(|p| same_path(p, &target));
        let unchanged = !self.is_modified()
            && format == self.format
            && !opts.convert_line_breaks
            && opts.quoting == Quoting::Keep;
        if same_file && unchanged && !opts.rewrite {
            return Ok(SaveReport {
                path: target,
                skipped: true,
                bytes: 0,
                rows: self.total_rows(),
                raw_rows: self.total_rows(),
                verified: true,
            });
        }
        if same_file && !opts.overwrite_changed && self.source.changed_on_disk()? {
            return Err(Error::ChangedOnDisk);
        }

        // A gzip file is saved gzipped (as is any file saved under a .gz
        // name); reading it back for the check below unpacks it again.
        let gzip = target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gz"))
            || (same_file && self.source.is_gzipped());

        // 1. Write and flush the temp file next to the target.
        let (temp, file) = create_temp(&target)?;
        let this: &Document = self;
        let write = || -> Result<Counts, Error> {
            let mut bw = BufWriter::with_capacity(1 << 20, &file);
            let c = if gzip {
                let mut gz = ParGz::new(&mut bw);
                let mut cw = CountingWriter {
                    inner: &mut gz,
                    n: 0,
                };
                let mut c = this.write_rows(
                    &mut cw,
                    format,
                    opts.convert_line_breaks,
                    opts.allow_lossy,
                    opts.quoting,
                    false,
                )?;
                c.bytes = cw.n;
                gz.finish()?;
                c
            } else {
                let mut cw = CountingWriter {
                    inner: &mut bw,
                    n: 0,
                };
                let mut c = this.write_rows(
                    &mut cw,
                    format,
                    opts.convert_line_breaks,
                    opts.allow_lossy,
                    opts.quoting,
                    false,
                )?;
                c.bytes = cw.n;
                c
            };
            bw.flush()?;
            file.sync_all()?;
            Ok(c)
        };
        // The undo history is moved onto the rows being written beside the write (it only reads; it is kept only if
        // the save then succeeds). With no history there is nothing to do.
        let (written, rebase) = if !this.has_history() {
            (write(), Err(Default::default()))
        } else {
            rayon::join(write, || this.prepare_rebase())
        };
        drop(file);
        let counts = match written {
            Ok(c) => c,
            Err(e) => {
                let _ = fs::remove_file(&temp);
                return Err(e);
            }
        };

        // 2. Read it back and compare every row before touching the original.
        let check = match self.verify(&temp, format, opts.allow_lossy, &counts) {
            Ok(c) => c,
            Err(e) => {
                let _ = fs::remove_file(&temp);
                return Err(e);
            }
        };
        let (check_src, check_start, check_index) = check.into_parts();
        let temp_hash = check_src.file_hash();
        // A mapped temp file is locked; release it so it can be moved.
        let mut kept = (!check_src.is_mapped()).then_some(check_src);

        // 3. Release our hold on the file being replaced (a mapped file is
        //    locked against writers, including us).
        let released = same_file && self.source.is_mapped();
        let my_tail = self.source.bytes().len() - self.data_end;
        if released {
            drop(self.take_source());
        }

        // 4. Atomic replace.
        if let Err(e) = replace(&temp, &target) {
            let message = e.to_string();
            if target.exists() {
                // The original is still in place.
                if !released {
                    let _ = fs::remove_file(&temp);
                    return Err(Error::SaveIncomplete {
                        message,
                        temp: None,
                    });
                }
                if let Ok(s) = Source::open(&target) {
                    if s.bytes().len() == self.index.reached() + my_tail || self.index.rows() == 0 {
                        self.put_source(s);
                        let _ = fs::remove_file(&temp);
                        return Err(Error::SaveIncomplete {
                            message,
                            temp: None,
                        });
                    }
                }
                // The original cannot be re-read as it was: continue from the
                // saved text instead, which holds everything.
                self.adopt_or_empty(kept.take(), &temp, check_start, format, check_index);
                return Err(Error::SaveIncomplete {
                    message,
                    temp: Some(temp),
                });
            }
            // The original is gone (a half-finished replace): put the saved
            // text in its place, or keep it and say where it is.
            if let Err(e2) = fs::rename(&temp, &target) {
                self.adopt_or_empty(kept.take(), &temp, check_start, format, check_index);
                return Err(Error::SaveIncomplete {
                    message: format!("{message}; {e2}"),
                    temp: Some(temp),
                });
            }
        }

        // 5. Continue from the verified copy now at `target`.
        let source = match kept.take() {
            Some(mut s) => {
                // In-memory copy: it is exactly what was written.
                if let Err(e) = s.reattach(&target, temp_hash.unwrap_or_default()) {
                    return Err(self.reload_failed(&target, e.to_string()));
                }
                s
            }
            None => match Source::open(&target) {
                Ok(s) if s.bytes().len() == check_index.reached() + check_start.tail_len => s,
                Ok(_) => {
                    return Err(self.reload_failed(&target, "the file changed after saving".into()))
                }
                Err(e) => return Err(self.reload_failed(&target, e.to_string())),
            },
        };
        self.reset_keeping(source, check_start, format, check_index, rebase);
        Ok(SaveReport {
            path: target,
            skipped: false,
            bytes: counts.bytes,
            rows: counts.rows,
            raw_rows: counts.raw_rows,
            verified: true,
        })
    }

    /// Open the written file the way it will be reopened and compare every
    /// row's values with the document.
    fn verify(
        &self,
        temp: &Path,
        format: Format,
        allow_lossy: bool,
        counts: &Counts,
    ) -> Result<Document, Error> {
        let opts = OpenOptions {
            encoding: Some(format.encoding),
            dialect: Some(format.dialect),
            skip_lines: Some(counts.head_lines),
            tail_bytes: Some(counts.tail),
            ..Default::default()
        };
        let mut check = Document::from_source(Source::open(temp)?, opts, None)?;
        while !check.index_step(usize::MAX)? {}
        let fail = |row| Err(Error::VerifyFailed { row });
        if check.format.bom != format.bom || check.lossy {
            return fail(0);
        }
        // The lines above the table read back the same (a line break may
        // have been added after them, when rows follow).
        let trim = |t: String| -> String {
            let n = LineBreak::at_end_of(t.as_bytes()).map_or(0, |l| l.bytes().len());
            t[..t.len() - n].to_owned()
        };
        if check.preamble_lines != counts.head_lines
            || trim(check.preamble_text()) != trim(counts.head_text.clone())
        {
            return fail(0);
        }
        // And the lines below it.
        let tail = &check.source_ref().bytes()[check.data_end..];
        if encoding::decode_direct(tail, check.working_encoding()) != counts.tail_text.as_str() {
            return fail(check.total_rows());
        }
        if let Some(r) = check.index.line_bound_from() {
            return fail(r);
        }
        if check.total_rows() != self.total_rows() {
            return fail(check.total_rows().min(self.total_rows()));
        }
        let fffd = allow_lossy && matches!(format.encoding, Encoding::Legacy(_));
        let same_bytes = check.working_encoding() == self.working_encoding()
            && check.format.dialect == self.format.dialect;
        let strip = |r: &'_ [u8]| -> usize {
            r.len() - LineBreak::at_end_of(r).map_or(0, |l| l.bytes().len())
        };
        let map = self.col_map();
        let quote = self.format.dialect;
        // Rows are compared in parallel; the first row (in order) that
        // differs, or fails to read, decides the result.
        let ids: Vec<u32> = self.ids().collect();
        let check_row = |sa: &mut Vec<(usize, usize)>,
                         sb: &mut Vec<(usize, usize)>,
                         pos: usize,
                         id: u32|
         -> Result<bool, Error> {
            let cid = pos as u32;
            if same_bytes {
                // Compare each written field with the field it came from
                // (unquoted bytes), without allocating: identical rows at
                // once; rows written again (column changes, a new quoting
                // style) field by field. Any difference falls through to
                // the full value comparison.
                if let (
                    RowSource::Raw { bytes: a, mode: ma },
                    RowSource::Raw { bytes: b, mode: mb },
                ) = (self.row_source(id), check.row_source(cid))
                {
                    let (a, b) = (&a[..strip(a)], &b[..strip(b)]);
                    if map.is_none() && a == b {
                        return Ok(true);
                    }
                    if self.field_spans_into(id, a, ma, sa).is_ok()
                        && check.field_spans_into(cid, b, mb, sb).is_ok()
                    {
                        let same = |mine: &[u8], (cs, ce): (usize, usize)| {
                            scan::unquote(mine, quote) == scan::unquote(&b[cs..ce], quote)
                        };
                        let equal = match map {
                            Some(m) => {
                                sb.len() == m.len()
                                    && m.iter().zip(sb.iter()).all(|(&p, &span)| {
                                        same(
                                            sa.get(p as usize).map_or(&b""[..], |&(s, e)| &a[s..e]),
                                            span,
                                        )
                                    })
                            }
                            None => {
                                sa.len() == sb.len()
                                    && sa
                                        .iter()
                                        .zip(sb.iter())
                                        .all(|(&(s, e), &span)| same(&a[s..e], span))
                            }
                        };
                        if equal {
                            return Ok(true);
                        }
                    }
                }
            }
            Ok(self.values_hash(id, fffd)? == check.values_hash(cid, false)?)
        };
        let first_bad = ids
            .par_iter()
            .enumerate()
            .with_min_len(1 << 12)
            .map_init(
                || (Vec::new(), Vec::new()),
                |(sa, sb), (pos, &id)| match check_row(sa, sb, pos, id) {
                    Ok(true) => None,
                    Ok(false) => Some(Ok(pos as u64)),
                    Err(e) => Some(Err(e)),
                },
            )
            .find_map_first(|x| x);
        match first_bad {
            None => Ok(check),
            Some(Ok(pos)) => fail(pos),
            Some(Err(e)) => Err(e),
        }
    }

    /// After a failed replace, make the saved text (which holds everything)
    /// the document's content; if even that cannot be read, empty it.
    fn adopt_or_empty(
        &mut self,
        kept: Option<Source>,
        temp: &Path,
        start: Start,
        format: Format,
        index: RowIndex,
    ) {
        let src = kept.or_else(|| Source::open(temp).ok());
        match src {
            Some(s) if s.bytes().len() == index.reached() + start.tail_len => {
                self.reset_to(s, start, format, index)
            }
            _ => self.make_empty(format),
        }
    }

    fn reload_failed(&mut self, path: &Path, message: String) -> Error {
        self.make_empty(self.format);
        Error::SavedButReloadFailed {
            path: path.to_owned(),
            message,
        }
    }

    /// A consistent empty document (used only when the saved file cannot be
    /// read back; the data is safe on disk).
    pub(crate) fn make_empty(&mut self, format: Format) {
        let index = RowIndex::build(&[], format.dialect, 0).expect("empty index");
        self.reset_to(
            Source::from_vec(Vec::new()),
            Start::default(),
            format,
            index,
        );
    }
}

/// What goes before an output cell taken from the original's field `p`
/// (the previous output cell came from field `prev`): the original bytes
/// between them when they were neighbours (keeps lined-up spacing), the
/// original bytes before the first field when `p` is the first, otherwise
/// one separator. Only used when the dialect is unchanged.
fn put_gap(
    out: &mut Vec<u8>,
    content: &[u8],
    spans: &[(usize, usize)],
    prev: Option<usize>,
    p: usize,
    d: Dialect,
) {
    match prev {
        None => {
            if let (0, Some(&(s, _))) = (p, spans.first()) {
                out.extend_from_slice(&content[..s]);
            }
        }
        Some(q) => match (spans.get(q), spans.get(p)) {
            (Some(&(_, e)), Some(&(s, _))) if q + 1 == p && e <= s => {
                out.extend_from_slice(&content[e..s])
            }
            _ => out.extend_from_slice(d.delimiter()),
        },
    }
}

/// After the last output cell: the original bytes after the last field when
/// that cell came from it (trailing spaces of a lined-up row).
fn put_tail(out: &mut Vec<u8>, content: &[u8], spans: &[(usize, usize)], last: Option<usize>) {
    if let (Some(p), Some(&(_, e))) = (last, spans.last()) {
        if p + 1 == spans.len() {
            out.extend_from_slice(&content[e..]);
        }
    }
}

/// A lone CR followed by an empty row ending in LF would read back as one
/// CRLF, losing a row (possible after deletes or edits). Write the empty row
/// as `""` instead, or end it with CR when the dialect has no quotes.
fn guard_empty_row(
    prev_cr: bool,
    empty: bool,
    brk: &mut Option<LineBreak>,
    target: Format,
) -> Option<u8> {
    if !(prev_cr && empty && *brk == Some(LineBreak::Lf)) {
        return None;
    }
    match target.dialect.quote() {
        Some(q) => Some(q),
        None => {
            *brk = Some(LineBreak::Cr);
            None
        }
    }
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Create `.ndtmp-<pid>-<time>-<n>` beside `target`, never reusing a name.
/// The name says nothing about the file being saved (a private file's
/// name is not repeated). On Unix the temp file gets the target's
/// permissions from the start.
pub(crate) fn create_temp(target: &Path) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    for _ in 0..100 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = dir.join(format!("{TEMP_PREFIX}{}-{nanos:x}-{n}", std::process::id()));
        match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(f) => {
                #[cfg(not(windows))]
                if let Ok(m) = fs::metadata(target) {
                    f.set_permissions(m.permissions())?;
                }
                return Ok((p, f));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("could not create a temporary file"))
}

/// Write a new file through a temp file beside `target`, moved into place
/// only once `write` has succeeded and the bytes are on disk: an error or a
/// cancel part way leaves no file that looks complete (the temp file is
/// removed, and a file already at `target` stays as it was).
pub fn write_atomic<T>(
    target: &Path,
    write: impl FnOnce(&mut BufWriter<&File>) -> Result<T, Error>,
) -> Result<T, Error> {
    let (temp, file) = create_temp(target)?;
    let done = (|| -> Result<T, Error> {
        let mut w = BufWriter::with_capacity(1 << 20, &file);
        let r = write(&mut w)?;
        w.flush()?;
        drop(w);
        file.sync_all()?;
        Ok(r)
    })();
    drop(file);
    let done = done.and_then(|r| replace(&temp, target).map(|()| r).map_err(Error::from));
    if done.is_err() {
        let _ = fs::remove_file(&temp);
    }
    done
}

/// The start of the app's temporary file names.
pub const TEMP_PREFIX: &str = ".ndtmp-";

/// Whether a file name is one of the app's temporary files (this kind, or
/// the older `.<name>.ndtmp-<pid>-...` and `.<name>.<pid>-<n>.ndtmp`), and
/// the process that made it when the name says.
pub fn own_temp(name: &str) -> Option<Option<u32>> {
    let pid_at = |s: &str| s.split('-').next().and_then(|d| d.parse::<u32>().ok());
    if let Some(rest) = name.strip_prefix(TEMP_PREFIX) {
        return Some(pid_at(rest));
    }
    if !name.starts_with('.') {
        return None;
    }
    if let Some(i) = name.find(".ndtmp-") {
        return Some(pid_at(&name[i + 7..]));
    }
    let stem = name.strip_suffix(".ndtmp")?;
    let last = stem.rsplit('.').next()?;
    Some(pid_at(last))
}

/// Remove the app's temporary files left in `dir` (not its subfolders) by
/// a save that was cut off: made by another process run (never this one)
/// and not changed for `older_than`. Returns how many were removed.
pub fn sweep_temps(dir: &Path, older_than: std::time::Duration) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let me = std::process::id();
    let mut n = 0;
    for e in entries.flatten().take(100_000) {
        let name = e.file_name();
        let Some(pid) = own_temp(&name.to_string_lossy()) else {
            continue;
        };
        if pid == Some(me) {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let old = m
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= older_than);
        if old && fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// Gzip in parallel: the output is cut into blocks of a few MB, each
/// compressed on its own core (fast level) as a gzip member of its own and
/// written in order. Every gzip reader reads such a file (members one after
/// another make one stream); the app's own reader does too.
struct ParGz<W: Write> {
    out: W,
    blocks: Vec<Vec<u8>>,
    cur: Vec<u8>,
}

const GZ_BLOCK: usize = 4 << 20;

impl<W: Write> ParGz<W> {
    fn new(out: W) -> Self {
        ParGz {
            out,
            blocks: Vec::new(),
            cur: Vec::with_capacity(GZ_BLOCK),
        }
    }

    /// Compress the full blocks held (in parallel) and write them in order.
    fn drain(&mut self) -> std::io::Result<()> {
        use rayon::prelude::*;
        let packed: Vec<std::io::Result<Vec<u8>>> = std::mem::take(&mut self.blocks)
            .into_par_iter()
            .map(|b| {
                let mut e = flate2::write::GzEncoder::new(
                    Vec::with_capacity(b.len() / 3),
                    flate2::Compression::fast(),
                );
                e.write_all(&b)?;
                e.finish()
            })
            .collect();
        for p in packed {
            self.out.write_all(&p?)?;
        }
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<()> {
        if !self.cur.is_empty() || self.blocks.is_empty() {
            let b = std::mem::take(&mut self.cur);
            self.blocks.push(b);
        }
        self.drain()?;
        self.out.flush()
    }
}

impl<W: Write> Write for ParGz<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = buf.len().min(GZ_BLOCK - self.cur.len());
        self.cur.extend_from_slice(&buf[..n]);
        if self.cur.len() >= GZ_BLOCK {
            let full = std::mem::replace(&mut self.cur, Vec::with_capacity(GZ_BLOCK));
            self.blocks.push(full);
            if self.blocks.len() >= rayon::current_num_threads().max(1) * 2 {
                self.drain()?;
            }
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod temp_tests {
    use super::*;

    #[test]
    fn temp_names_and_sweep() {
        assert_eq!(own_temp(".ndtmp-123-ab-0"), Some(Some(123)));
        assert_eq!(own_temp(".voters.csv.ndtmp-77-1f-2"), Some(Some(77)));
        assert_eq!(own_temp(".recent.json.88-1.ndtmp"), Some(Some(88)));
        assert_eq!(own_temp("voters.csv"), None);
        assert_eq!(own_temp("notes.ndtmp.txt"), None);
        let dir = std::env::temp_dir().join(format!("nd-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("private voters.csv");
        let (mine, f) = create_temp(&target).unwrap();
        drop(f);
        let name = mine.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(TEMP_PREFIX) && !name.contains("voters"));
        let left = dir.join(".ndtmp-1-0-0");
        fs::write(&left, b"x").unwrap();
        let keep = dir.join("data.csv");
        fs::write(&keep, b"x").unwrap();
        assert_eq!(sweep_temps(&dir, std::time::Duration::ZERO), 1);
        assert!(!left.exists() && mine.exists() && keep.exists());
        // Recent ones stay (a save may still be running in another window).
        fs::write(&left, b"x").unwrap();
        assert_eq!(sweep_temps(&dir, std::time::Duration::from_secs(3600)), 0);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
thread_local! {
    /// Test hook: 1 = replace fails leaving both files, 2 = replace fails
    /// after removing the target (Windows error 1176).
    static FAIL_REPLACE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn injected_failure(target: &Path) -> Option<io::Error> {
    match FAIL_REPLACE.with(|f| f.get()) {
        1 => Some(io::Error::other("injected replace failure")),
        2 => {
            let _ = fs::remove_file(target);
            Some(io::Error::other("injected failure after removing target"))
        }
        _ => None,
    }
}

/// Rename `from` to `to`, failing (never replacing) when `to` already
/// exists as another file. A change of letter case only is allowed.
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
        let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
        let (f, t) = (wide(from), wide(to));
        // SAFETY: both pointers are valid NUL-terminated UTF-16 strings that
        // outlive the call. Flags 0: no MOVEFILE_REPLACE_EXISTING, so an
        // existing target makes the call fail instead of being replaced.
        #[allow(unsafe_code)]
        let ok = unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), 0) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if to.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the file exists",
            ));
        }
        fs::rename(from, to)
    }
}

/// Move `temp` over `target` atomically.
#[cfg(windows)]
pub(crate) fn replace(temp: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    #[cfg(test)]
    if let Some(e) = injected_failure(target) {
        return Err(e);
    }
    if !target.exists() {
        return fs::rename(temp, target);
    }
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (t, r) = (wide(target), wide(temp));
    // SAFETY: both pointers are valid NUL-terminated UTF-16 strings that
    // outlive the call; the optional arguments are null as documented.
    #[allow(unsafe_code)]
    let ok = unsafe {
        ReplaceFileW(
            t.as_ptr(),
            r.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub(crate) fn replace(temp: &Path, target: &Path) -> io::Result<()> {
    #[cfg(test)]
    if let Some(e) = injected_failure(target) {
        return Err(e);
    }
    fs::rename(temp, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::Dialect;
    use crate::document::OpenOptions;
    use crate::encoding::Encoding;
    use proptest::prelude::*;

    fn doc(b: &[u8]) -> Document {
        Document::from_bytes(b.to_vec(), OpenOptions::default(), None).unwrap()
    }

    fn row_by_id(d: &Document, id: u32) -> Vec<String> {
        (0..d.col_count())
            .map(|c| d.cell_by_id(id, c).unwrap_or_default())
            .collect()
    }

    fn rewrite(d: &Document) -> Vec<u8> {
        let mut out = Vec::new();
        d.write_to(&mut out, d.format(), false).unwrap();
        out
    }

    fn same_values(a: &Document, b: &Document) -> Result<(), TestCaseError> {
        prop_assert_eq!(b.index().line_bound_from(), None);
        prop_assert_eq!(a.row_count(), b.row_count());
        for r in 0..a.row_count() {
            prop_assert_eq!(
                a.row_values(r).unwrap(),
                b.row_values(r).unwrap(),
                "row {}",
                r
            );
        }
        Ok(())
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("nde-save-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// A .gz file opens unpacked in memory and saves back gzipped; a crafted
    /// file that unpacks to far more than its size is refused.
    #[test]
    fn gzip_files_open_and_save_back_gzipped() {
        use std::io::{Read, Write};
        let dir = tmpdir("gz");
        let p = dir.join("voters.csv.gz");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(b"name,zip\r\nAnn,01234\r\n").unwrap();
        fs::write(&p, enc.finish().unwrap()).unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        assert_eq!(d.row_values(1).unwrap(), ["Ann", "01234"]);
        d.set_cell(1, 0, "Bea".into()).unwrap();
        assert!(d.save(SaveOptions::default()).unwrap().verified);
        let mut text = String::new();
        flate2::read::GzDecoder::new(&fs::read(&p).unwrap()[..])
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "name,zip\r\nBea,01234\r\n");
        assert!(!d.is_modified());
        // Saved again after another change: still gzipped, nothing left over.
        d.set_cell(1, 1, "99999".into()).unwrap();
        d.save(SaveOptions::default()).unwrap();
        assert!(crate::source::is_gzip(&fs::read(&p).unwrap()));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        // Save As a plain name writes plain text.
        let plain = dir.join("out.csv");
        d.save(SaveOptions {
            path: Some(plain.clone()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(fs::read(&plain).unwrap(), b"name,zip\r\nBea,99999\r\n");
        // A bomb: a megabyte-plus of zeros per few bytes is refused.
        let bomb = dir.join("bomb.csv.gz");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        enc.write_all(&vec![b'a'; 64 << 20]).unwrap();
        fs::write(&bomb, enc.finish().unwrap()).unwrap();
        assert!(Document::open(&bomb, OpenOptions::default()).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    /// A blank template (titles over rows of separators only), comma and
    /// tab, opened as it is: typing into a row and saving writes that row
    /// with the file's own separator, and the titles stay as they were.
    #[test]
    fn typing_into_a_blank_template_saves_its_own_separator() {
        let dir = tmpdir("template");
        for (name, text, want) in [
            ("t.csv", "x,y,z\n,,\n,,\n,,\n", "ann,,lee"),
            ("t.txt", "x,y,z\n,,\n,,\n,,\n", "ann,,lee"),
            ("t.tsv", "a\tb\tc\n\t\t\n\t\t\n\t\t\n", "ann\t\tlee"),
            ("tab.txt", "a\tb\tc\n\t\t\n\t\t\n\t\t\n", "ann\t\tlee"),
        ] {
            let p = dir.join(name);
            fs::write(&p, text).unwrap();
            let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
            assert_eq!(d.col_count(), 3, "{name}");
            d.set_cell(2, 0, "ann".into()).unwrap();
            d.set_cell(2, 2, "lee".into()).unwrap();
            d.save(SaveOptions::default()).unwrap();
            let saved = fs::read_to_string(&p).unwrap();
            let lines: Vec<&str> = saved.lines().collect();
            assert_eq!(lines[0], text.lines().next().unwrap(), "{name}");
            assert_eq!(lines[2], want, "{name}");
            assert_eq!(lines.len(), 4, "{name}");
            // Any other program splits the edited row into 3 fields.
            let sep = if want.contains('\t') { '\t' } else { ',' };
            assert_eq!(lines[2].split(sep).collect::<Vec<_>>(), ["ann", "", "lee"]);
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// A save keeps the undo history: marks made before it still work, and the saved state is not modified.
    #[test]
    fn marks_survive_a_save() {
        let dir = tmpdir("marks");
        let p = dir.join("m.csv");
        fs::write(&p, b"a,b\n1,2\n").unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        let start = d.step_mark();
        d.set_cell(1, 0, "x".into()).unwrap();
        let edited = d.step_mark();
        d.save(SaveOptions::default()).unwrap();
        // The history follows the saved file.
        assert_eq!(d.step_reach(edited), Some(0));
        assert_eq!(d.step_reach(start), Some(-1));
        assert!(!d.is_modified());
        assert!(d.go_to_mark(start));
        assert_eq!(d.cell(1, 0).unwrap(), "1");
        assert!(d.is_modified());
        assert!(d.go_to_mark(edited));
        assert!(!d.is_modified());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn edited_rows_rewritten_others_raw() {
        let src = b"a , b\r\n\"q\",x\n1,2";
        let mut d = doc(src);
        assert_eq!(rewrite(&d), src);
        d.set_cell(1, 1, "y,z".into()).unwrap();
        // The unchanged cell keeps its original quotes.
        assert_eq!(rewrite(&d), b"a , b\r\n\"q\",\"y,z\"\n1,2");
        d.insert_rows(3, vec![vec!["3".into(), "4".into()]])
            .unwrap();
        // The old last row gains the file's line break; the new last row
        // follows the file in having none.
        assert_eq!(rewrite(&d), b"a , b\r\n\"q\",\"y,z\"\n1,2\r\n3,4");
    }

    #[test]
    fn column_changes_rewrite_and_verify_on_disk() {
        let dir = tmpdir("cols");
        let p = dir.join("t.csv");
        fs::write(&p, b"a,b,c\r\n1,2,3\r\n").unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        d.move_cols(2, 1, 0).unwrap();
        d.insert_cols(3, 1).unwrap();
        d.delete_cols(1, 1).unwrap();
        assert!(d.has_column_changes());
        let r = d.save(SaveOptions::default()).unwrap();
        assert!(r.verified);
        assert_eq!(fs::read(&p).unwrap(), b"c,b,\r\n3,2,\r\n");
        assert!(!d.has_column_changes());
        assert!(!d.is_modified());
        let _ = fs::remove_dir_all(dir);
    }

    /// Column changes keep undecodable bytes byte-for-byte (no consent
    /// needed, no prompt for a bad byte in a deleted column) and keep every
    /// kept cell's original quoting; converting the encoding still asks.
    #[test]
    fn column_changes_keep_raw_cells() {
        let opts = OpenOptions {
            encoding: Some(Encoding::Utf8),
            quote: Some(Some(b'"')),
            ..Default::default()
        };
        let src = b"a,\"b\",c\nx\xFF,y,\"z\xFE\"\n\"open\n";
        let mut d = Document::from_bytes(src.to_vec(), opts, None).unwrap();
        d.move_cols(1, 1, 0).unwrap();
        let mut out = Vec::new();
        d.write_to(&mut out, d.format(), false).unwrap();
        assert_eq!(out, b"\"b\",a,c\ny,x\xFF,\"z\xFE\"\n,\"open\",\n");
        d.delete_cols(2, 1).unwrap();
        d.set_cell(1, 0, "Y".into()).unwrap();
        let mut out = Vec::new();
        d.write_to(&mut out, d.format(), false).unwrap();
        assert_eq!(out, b"\"b\",a\nY,x\xFF\n,\"open\"\n");
        let mut f = d.format();
        f.encoding = Encoding::Utf16Le;
        assert!(matches!(
            d.write_to(&mut Vec::new(), f, false),
            Err(Error::Lossy { .. })
        ));
    }

    #[test]
    fn quoting_styles() {
        let mut d = doc(b"name,n,note\r\n\"Ann\",1.5,\r\nBob,-2e3,\"x,y\"\r\n");
        let f = d.format();
        let out = |d: &Document, q| {
            let mut o = Vec::new();
            d.write_quoted(&mut o, f, false, q).unwrap();
            String::from_utf8(o).unwrap()
        };
        assert_eq!(
            out(&d, Quoting::Keep),
            "name,n,note\r\n\"Ann\",1.5,\r\nBob,-2e3,\"x,y\"\r\n"
        );
        assert_eq!(
            out(&d, Quoting::Needed),
            "name,n,note\r\nAnn,1.5,\r\nBob,-2e3,\"x,y\"\r\n"
        );
        assert_eq!(
            out(&d, Quoting::All),
            "\"name\",\"n\",\"note\"\r\n\"Ann\",\"1.5\",\"\"\r\n\"Bob\",\"-2e3\",\"x,y\"\r\n"
        );
        assert_eq!(
            out(&d, Quoting::Text),
            "\"name\",\"n\",\"note\"\r\n\"Ann\",1.5,\r\n\"Bob\",-2e3,\"x,y\"\r\n"
        );
        // A decimal comma is a number, but still needs quotes with commas.
        d.set_cell(1, 1, "2,5".into()).unwrap();
        assert!(out(&d, Quoting::Text).contains("\"Ann\",\"2,5\","));
        // Edited rows and column changes go through the same rules.
        d.insert_cols(1, 1).unwrap();
        assert!(out(&d, Quoting::All).starts_with("\"name\",\"\",\"n\""));
        for (v, num) in [
            ("12", true),
            ("-0.5", true),
            ("+3,25", true),
            ("1e9", true),
            ("1e", false),
            ("1.2.3", false),
            ("12 ", false),
            ("", false),
            ("-", false),
            ("1,000.5", false),
        ] {
            assert_eq!(scan::plain_number(v), num, "{v}");
        }
    }

    #[test]
    fn quoting_style_saves_verified() {
        let dir = tmpdir("quoting");
        let path = dir.join("q.csv");
        fs::write(&path, b"a,b\r\n1,x\r\n").unwrap();
        let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let r = d
            .save(SaveOptions {
                quoting: Quoting::Text,
                ..Default::default()
            })
            .unwrap();
        assert!(!r.skipped && r.verified);
        assert_eq!(fs::read(&path).unwrap(), b"\"a\",\"b\"\r\n1,\"x\"\r\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn convert_dialect_and_breaks() {
        let d = doc(b"a,\"b;c\"\r\n1,2\n");
        let mut f = d.format();
        f.dialect = Dialect::new(b';', Some(b'"')).unwrap();
        f.line_break = LineBreak::Lf;
        let mut out = Vec::new();
        d.write_to(&mut out, f, true).unwrap();
        assert_eq!(out, b"a;\"b;c\"\n1;2\n");
        f.dialect = Dialect::new(b';', None).unwrap();
        assert!(matches!(
            d.write_to(&mut Vec::new(), f, true),
            Err(Error::CannotRepresent { row: 0, col: 1 })
        ));
    }

    /// B2 / R9a: after the file is read in another encoding, saving to a third one converts every row (the raw bytes
    /// are no longer copied as they are), so the output holds no byte of the old reading.
    #[test]
    fn switching_the_encoding_then_saving_as_utf8_transcodes_every_row() {
        // The bytes of "caf\u{e9}" in UTF-8, opened as UTF-8.
        let src = "id,name\n1,caf\u{e9}\n2,plain\n";
        let opts = OpenOptions {
            encoding: Some(Encoding::Utf8),
            ..Default::default()
        };
        let mut d = Document::from_bytes(src.as_bytes().to_vec(), opts, None).unwrap();
        let rev = d.data_revision();
        let latin = Encoding::from_label("windows-1252").unwrap();
        d.set_encoding(latin).unwrap();
        assert!(d.data_revision() > rev, "Apply bumps the data revision");
        assert_eq!(
            d.row_values(1).unwrap(),
            ["1", "caf\u{c3}\u{a9}"],
            "now read as Windows-1252"
        );
        // Save as UTF-8: every row converted, not copied raw.
        let mut f = d.format();
        f.encoding = Encoding::Utf8;
        let mut out = Vec::new();
        d.write_to(&mut out, f, false).unwrap();
        assert_eq!(out, "id,name\n1,caf\u{c3}\u{a9}\n2,plain\n".as_bytes());
        assert!(std::str::from_utf8(&out).is_ok());
        // Saved in the encoding it is read as: the bytes stay exactly as they were.
        let mut same = Vec::new();
        d.write_to(&mut same, d.format(), false).unwrap();
        assert_eq!(same, src.as_bytes());
        // One undo reads it as UTF-8 again.
        assert!(d.undo());
        assert_eq!(d.row_values(1).unwrap(), ["1", "caf\u{e9}"]);
        assert!(d.redo());
        assert_eq!(d.format().encoding, latin);
    }

    #[test]
    fn convert_encoding_strictly() {
        let mut d = doc("x,é\n".as_bytes());
        let mut f = d.format();
        f.encoding = Encoding::from_label("windows-1252").unwrap();
        let mut out = Vec::new();
        d.write_to(&mut out, f, false).unwrap();
        assert_eq!(out, b"x,\xE9\n");
        d.set_cell(0, 0, "π".into()).unwrap();
        assert!(matches!(
            d.write_to(&mut Vec::new(), f, false),
            Err(Error::Unmappable { ch: 'π', .. })
        ));
        f.encoding = Encoding::Utf16Le;
        f.bom = true;
        let mut out = Vec::new();
        d.write_to(&mut out, f, false).unwrap();
        let back = Document::from_bytes(out, OpenOptions::default(), None).unwrap();
        assert_eq!(back.row_values(0).unwrap(), ["π", "é"]);
    }

    #[test]
    fn save_to_disk_and_reload() {
        let dir = tmpdir("disk");
        let p = dir.join("t.csv");
        fs::write(&p, b"id,name\r\n1,ann\r\n2,bob\r\n").unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        let r = d.save(SaveOptions::default()).unwrap();
        assert!(r.skipped);
        d.set_cell(2, 1, "bea".into()).unwrap();
        let r = d.save(SaveOptions::default()).unwrap();
        assert!(!r.skipped && r.verified);
        assert_eq!((r.rows, r.raw_rows), (3, 2));
        assert_eq!(fs::read(&p).unwrap(), b"id,name\r\n1,ann\r\n2,bea\r\n");
        assert!(!d.is_modified());
        assert_eq!(d.cell(2, 1).unwrap(), "bea");
        // No temp files left behind.
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

        // Another program changes the file: saving must refuse.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&p, b"id,name\r\n9,zed\r\n9,zed\r\n9,zed\r\n").unwrap();
        d.set_cell(1, 1, "x".into()).unwrap();
        assert!(matches!(
            d.save(SaveOptions::default()),
            Err(Error::ChangedOnDisk)
        ));
        let r = d
            .save(SaveOptions {
                overwrite_changed: true,
                ..Default::default()
            })
            .unwrap();
        assert!(r.verified);

        // Save As leaves the original alone.
        let q = dir.join("copy.tsv");
        let mut f = d.format();
        f.dialect = Dialect::TSV;
        d.set_cell(0, 0, "ID".into()).unwrap();
        d.save(SaveOptions {
            path: Some(q.clone()),
            format: Some(f),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(fs::read(&q).unwrap(), b"ID\tname\r\n1\tx\r\n2\tbea\r\n");
        assert_eq!(d.path(), Some(q.as_path()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_save_keeps_original_and_edits() {
        let dir = tmpdir("fail");
        let p = dir.join("t.csv");
        fs::write(&p, b"a\n").unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        d.set_cell(0, 0, "π".into()).unwrap();
        let mut f = d.format();
        f.encoding = Encoding::from_label("windows-1252").unwrap();
        assert!(d
            .save(SaveOptions {
                format: Some(f),
                ..Default::default()
            })
            .is_err());
        assert_eq!(fs::read(&p).unwrap(), b"a\n");
        assert_eq!(d.cell(0, 0).unwrap(), "π");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn mapped_file_saves_in_place() {
        let dir = tmpdir("map");
        let p = dir.join("big.csv");
        let mut body = Vec::new();
        let mut i = 0;
        while body.len() < crate::source::MAP_THRESHOLD as usize {
            body.extend_from_slice(format!("{i},row {i}\r\n").as_bytes());
            i += 1;
        }
        fs::write(&p, &body).unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        assert!(d.source.is_mapped());
        d.set_cell(1, 1, "changed".into()).unwrap();
        let r = d.save(SaveOptions::default()).unwrap();
        assert!(r.verified);
        assert_eq!(d.cell(1, 1).unwrap(), "changed");
        assert_eq!(d.cell(i - 1, 0).unwrap(), (i - 1).to_string());
        drop(d);
        fs::remove_dir_all(&dir).unwrap();
    }

    fn any_csvish() -> impl Strategy<Value = Vec<u8>> {
        proptest::collection::vec(
            prop_oneof![
                Just(b','),
                Just(b'"'),
                Just(b'\r'),
                Just(b'\n'),
                Just(b'x'),
                any::<u8>()
            ],
            0..300,
        )
    }

    // ---- Milestone 3b review ----

    fn with(dialect: Dialect, src: &[u8]) -> (Document, OpenOptions) {
        let opts = OpenOptions {
            encoding: Some(Encoding::Utf8),
            dialect: Some(dialect),
            skip_lines: Some(0),
            ..Default::default()
        };
        (
            Document::from_bytes(src.to_vec(), opts, None).unwrap(),
            opts,
        )
    }

    /// Finding 4: a reused cell ending in part of a longer separator is
    /// quoted (or refused without quotes) instead of joining it.
    #[test]
    fn cell_ending_in_part_of_separator_is_quoted() {
        for (sep, row) in [
            (&b"||"[..], &b"a||b|\n"[..]),
            (b"~|~", b"x~|~y~|\n"),
            (b"::", b"k::v:\n"),
            (b"||", b"a||\"\"|\n"),
        ] {
            let dl = Dialect::custom(sep, Some(b'"'), Some(b'\\'), false).unwrap();
            let (mut d, opts) = with(dl, row);
            let before = d.row_values(0).unwrap();
            d.insert_cols(2, 1).unwrap();
            let out = rewrite(&d);
            let back = Document::from_bytes(out.clone(), opts, None).unwrap();
            let mut want = before.clone();
            want.push(String::new());
            assert_eq!(
                back.row_values(0).unwrap(),
                want,
                "{:?}",
                String::from_utf8_lossy(&out)
            );
            // An edit elsewhere in the row takes the same care.
            let (mut d, _) = with(dl, row);
            d.set_cell(0, 1, "z".into()).unwrap();
            let back = Document::from_bytes(rewrite(&d), opts, None).unwrap();
            assert_eq!(
                back.row_values(0).unwrap(),
                vec![before[0].clone(), "z".into()]
            );
        }
        let bare = Dialect::custom(b"||", None, None, false).unwrap();
        let (mut d, _) = with(bare, b"a||b|\n");
        d.insert_cols(2, 1).unwrap();
        let mut out = Vec::new();
        assert!(matches!(
            d.write_to(&mut out, d.format(), false),
            Err(Error::CannotRepresent { row: 0, col: 1 })
        ));
    }

    /// Finding 5: editing a lined-up row keeps its spacing.
    #[test]
    fn editing_keeps_lined_up_spacing() {
        let spaces = Dialect::custom(b" ", Some(b'"'), None, true).unwrap();
        let src = b"name    age  city\nalice   30   Paris\nbob     41   Oslo  \n";
        let (mut d, _) = with(spaces, src);
        d.set_cell(1, 1, "31".into()).unwrap();
        d.set_cell(2, 0, "robert".into()).unwrap();
        assert_eq!(
            rewrite(&d),
            b"name    age  city\nalice   31   Paris\nrobert     41   Oslo  \n"
        );
        // Moving columns keeps the spacing between cells that stay neighbours.
        let (mut d, _) = with(spaces, b"  a   b  c\n");
        d.move_cols(2, 1, 0).unwrap();
        assert_eq!(rewrite(&d), b"c a   b\n");
    }

    // ---- Regression tests for the independent review's findings ----

    /// Finding 1: an edit below an unclosed quote must not scramble rows.
    #[test]
    fn save_after_unclosed_quote_keeps_rows() {
        let dir = tmpdir("unclosed");
        let p = dir.join("t.csv");
        fs::write(&p, b"id,v\n1,\"oops\n2,x\n3,y\n").unwrap();
        let quoted = OpenOptions {
            quote: Some(Some(b'"')),
            ..Default::default()
        };
        let mut d = Document::open_indexed(&p, quoted, None).unwrap();
        assert_eq!(d.index().line_bound_from(), Some(1));
        assert_eq!(d.row_count(), 4);
        d.set_cell(3, 1, "has, comma".into()).unwrap();
        let want: Vec<_> = (0..4).map(|r| d.row_values(r).unwrap()).collect();
        d.save(SaveOptions::default()).unwrap();
        assert_eq!(
            fs::read(&p).unwrap(),
            b"id,v\n1,\"oops\"\n2,x\n3,\"has, comma\"\n"
        );
        let back = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        let got: Vec<_> = (0..back.row_count())
            .map(|r| back.row_values(r).unwrap())
            .collect();
        assert_eq!(got, want);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Finding 1 (second half): the written file is checked before it
    /// replaces anything; a mismatch keeps the original.
    #[test]
    fn verification_failure_keeps_original() {
        let dir = tmpdir("verify");
        let p = dir.join("t.csv");
        fs::write(&p, b"a,b\n1,2\n").unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        // A UTF-8 file without BOM whose first cell starts with U+FEFF would
        // reopen without it: verification must catch that.
        d.set_cell(0, 0, "\u{FEFF}a".into()).unwrap();
        let r = d.save(SaveOptions::default());
        assert!(matches!(r, Err(Error::VerifyFailed { row: 0 })), "{r:?}");
        assert_eq!(fs::read(&p).unwrap(), b"a,b\n1,2\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Finding 2a: a transcoded file with invalid sequences is not rewritten
    /// silently.
    #[test]
    fn lossy_utf16_needs_consent() {
        let mut bytes = b"\xFF\xFE".to_vec();
        encoding::encode("a,b\r\n", Encoding::Utf16Le, &mut bytes).unwrap();
        bytes.extend_from_slice(b"\x00\xD8,\x00x\x00\r\x00\n\x00"); // lone surrogate
        let dir = tmpdir("lossy16");
        let p = dir.join("t.csv");
        fs::write(&p, &bytes).unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        assert!(d.is_lossy());
        d.set_cell(0, 0, "A".into()).unwrap();
        assert!(matches!(
            d.save(SaveOptions::default()),
            Err(Error::Lossy { row: None })
        ));
        assert_eq!(fs::read(&p).unwrap(), bytes);
        d.save(SaveOptions {
            allow_lossy: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.cell(1, 0).unwrap(), "\u{FFFD}");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Finding 2b: a bad byte in an unedited cell of an edited row survives.
    #[test]
    fn invalid_utf8_in_other_cell_survives_edit() {
        let src = b"a,b\nbad\xFFbyte,x\n";
        let opts = OpenOptions {
            encoding: Some(Encoding::Utf8),
            ..Default::default()
        };
        let mut d = Document::from_bytes(src.to_vec(), opts, None).unwrap();
        d.set_cell(1, 1, "y".into()).unwrap();
        assert_eq!(rewrite(&d), b"a,b\nbad\xFFbyte,y\n");
        // Converting it to another encoding would lose the byte: refused.
        let mut f = d.format();
        f.encoding = Encoding::Utf16Le;
        assert!(matches!(
            d.write_to(&mut Vec::new(), f, false),
            Err(Error::Lossy { row: Some(1) })
        ));
    }

    /// Finding 2c: Shift_JIS with a bad byte can be saved after consent.
    #[test]
    fn lossy_shift_jis_saves_with_consent() {
        let sjis = Encoding::from_label("shift_jis").unwrap();
        let mut bytes = Vec::new();
        encoding::encode("名前,町\r\n", sjis, &mut bytes).unwrap();
        bytes.extend_from_slice(b"\x81,x\r\n"); // truncated double-byte char
        let dir = tmpdir("sjis");
        let p = dir.join("t.csv");
        fs::write(&p, &bytes).unwrap();
        let opts = OpenOptions {
            encoding: Some(sjis),
            ..Default::default()
        };
        let mut d = Document::open_indexed(&p, opts, None).unwrap();
        assert!(d.is_lossy());
        d.set_cell(1, 1, "y".into()).unwrap();
        assert!(d.save(SaveOptions::default()).is_err());
        d.save(SaveOptions {
            allow_lossy: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.row_values(1).unwrap(), ["?", "y"]);
        assert_eq!(d.row_values(0).unwrap(), ["名前", "町"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Finding 3: editing a crafted, enormously wide row is refused cleanly.
    #[test]
    fn wide_row_edit_is_refused() {
        let mut row = vec![b','; crate::document::MAX_ROW_FIELDS as usize + 10];
        row.push(b'\n');
        let mut d = Document::from_bytes(row, OpenOptions::default(), Some("csv")).unwrap();
        assert!(matches!(
            d.set_cell(0, 0, "x".into()),
            Err(Error::RowTooWide { .. })
        ));
        // Viewing it is still fine.
        assert_eq!(d.viewport(0..1, 0..3, 10).unwrap().cells.len(), 3);
    }

    /// Finding 4: a document that could not reload stays usable.
    #[test]
    fn reload_failure_leaves_consistent_document() {
        let mut d = doc(b"a,b\n1,2\n");
        let e = d.reload_failed(Path::new("x.csv"), "gone".into());
        assert!(matches!(e, Error::SavedButReloadFailed { .. }));
        assert_eq!(d.row_count(), 0);
        assert_eq!(d.viewport(0..10, 0..10, 10).unwrap().rows, 0);
        assert!(d.cell(0, 0).is_none());
    }

    /// Finding 5: a replace that fails after removing the target must put
    /// the saved text in place, not delete it.
    #[test]
    fn half_failed_replace_never_loses_data() {
        let dir = tmpdir("half");
        let p = dir.join("t.csv");
        fs::write(&p, b"a\n1\n").unwrap();
        let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        d.set_cell(1, 0, "2".into()).unwrap();
        FAIL_REPLACE.with(|f| f.set(2));
        let r = d.save(SaveOptions::default());
        FAIL_REPLACE.with(|f| f.set(0));
        r.unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"a\n2\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

        // A plain failure keeps the original and the edits.
        d.set_cell(1, 0, "3".into()).unwrap();
        FAIL_REPLACE.with(|f| f.set(1));
        let r = d.save(SaveOptions::default());
        FAIL_REPLACE.with(|f| f.set(0));
        assert!(matches!(r, Err(Error::SaveIncomplete { temp: None, .. })));
        assert_eq!(fs::read(&p).unwrap(), b"a\n2\n");
        assert_eq!(d.cell(1, 0).unwrap(), "3");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Finding 12: runs of raw rows are written in one piece and still exact.
    #[test]
    fn raw_runs_after_edits_stay_exact() {
        let src = b"h1,h2\r\nr1,a\r\nr2,b\r\nr3,c\r\n";
        let mut d = doc(src);
        d.set_cell(2, 1, "B".into()).unwrap();
        d.delete_rows(0, 1).unwrap();
        d.insert_rows(0, vec![vec!["n".into()]]).unwrap();
        assert_eq!(rewrite(&d), b"n\r\nr1,a\r\nr2,B\r\nr3,c\r\n");
    }

    proptest! {
        #[test]
        fn nothing_panics_on_any_input(bytes in proptest::collection::vec(any::<u8>(), 0..400), ext in proptest::option::of("csv|tsv|txt")) {
            // Full detection path, every read, an edit and conversions.
            let Ok(mut d) = Document::from_bytes(bytes, OpenOptions::default(), ext.as_deref()) else {
                return Ok(());
            };
            let _ = d.viewport(0..d.row_count(), 0..d.col_count().min(1000), 5).unwrap();
            for r in 0..d.row_count() {
                let _ = d.row_values(r).unwrap();
            }
            if d.row_count() > 0 {
                d.set_cell(0, 0, "é,\"\n".into()).unwrap();
                d.delete_rows(0, 1).unwrap();
                d.undo();
            }
            let _ = d.write_to(&mut Vec::new(), d.format(), false);
            for (enc, dia) in [(Encoding::Utf16Be, Dialect::TSV), (Encoding::Utf7, Dialect::CSV), (Encoding::from_label("latin1").unwrap(), Dialect::new(b'|', None).unwrap())] {
                let mut f = d.format();
                f.encoding = enc;
                f.dialect = dia;
                let _ = d.write_to(&mut Vec::new(), f, true);
            }
        }

        /// Every quoting style, separator and qualifier reads back the same
        /// values (edits and column changes included).
        #[test]
        fn quoting_styles_read_back_the_same(
            bytes in any_csvish(),
            edits in proptest::collection::vec((0u64..20, 0u32..4, "[a-z0-9.,;'\"\r\n -]{0,6}"), 0..5),
            style in 0usize..4,
            sep in 0usize..3,
            quote in 0usize..2,
            insert in any::<bool>(),
        ) {
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), delimiter: Some(b','), ..Default::default() };
            let Ok(mut d) = Document::from_bytes(bytes, opts, None) else { return Ok(()) };
            if d.index().line_bound_from().is_some() || d.is_lossy() {
                return Ok(());
            }
            for (r, c, v) in edits {
                if d.row_count() > 0 {
                    d.set_cell(r % d.row_count(), c, v).unwrap();
                }
            }
            if insert && d.col_count() > 0 {
                d.insert_cols(0, 1).unwrap();
            }
            let q = [Quoting::Keep, Quoting::Needed, Quoting::All, Quoting::Text][style];
            let dialect = Dialect::new(b",;\t"[sep], Some(b"\"'"[quote])).unwrap();
            let mut f = d.format();
            f.dialect = dialect;
            let mut out = Vec::new();
            if d.write_quoted(&mut out, f, true, q).is_err() {
                return Ok(()); // e.g. a value the format cannot hold
            }
            let (back, _) = with(dialect, &out);
            same_values(&d, &back)?;
        }

        #[test]
        fn unedited_rewrite_is_byte_exact(bytes in any_csvish()) {
            // Force UTF-8 so arbitrary bytes stay direct; any input at all.
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), ..Default::default() };
            let d = Document::from_bytes(bytes.clone(), opts, None).unwrap();
            let out = rewrite(&d);
            if d.index().line_bound_from().is_none() {
                prop_assert_eq!(out, bytes);
            } else {
                // Unclosed quotes are closed; values must still match.
                let again = OpenOptions {
                    dialect: Some(d.format().dialect),
                    skip_lines: Some(d.preamble_lines()),
                    ..opts
                };
                same_values(&d, &Document::from_bytes(out, again, None).unwrap())?;
            }
        }

        #[test]
        fn wide_encodings_round_trip(text in "[a-z,\"\r\n é😀]{0,60}", which in 0usize..5, bom: bool) {
            let enc = [Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be, Encoding::Utf7][which];
            let mut bytes = if bom { enc.bom().to_vec() } else { Vec::new() };
            encoding::encode(&text, enc, &mut bytes).unwrap();
            let opts = OpenOptions { encoding: Some(enc), delimiter: Some(b','), ..Default::default() };
            let d = Document::from_bytes(bytes.clone(), opts, None).unwrap();
            prop_assert_eq!(d.format().bom, bom);
            let out = rewrite(&d);
            if enc == Encoding::Utf7 {
                // UTF-7 has several spellings of the same text; compare text.
                let opts = OpenOptions { encoding: Some(enc), delimiter: Some(b','), ..Default::default() };
                let back = Document::from_bytes(out, opts, None).unwrap();
                if d.index().line_bound_from().is_none() {
                    prop_assert_eq!(back.source.bytes(), d.source.bytes());
                } else {
                    same_values(&d, &back)?;
                }
            } else if d.index().line_bound_from().is_none() {
                prop_assert_eq!(out, bytes);
            } else {
                same_values(&d, &Document::from_bytes(out, opts, None).unwrap())?;
            }
        }

        #[test]
        fn edits_survive_save_and_reload(
            bytes in any_csvish(),
            edits in proptest::collection::vec((0u64..20, 0u32..6, "[a-z,\"\r\n]{0,5}"), 0..8),
            inserts in proptest::collection::vec((0u64..20, proptest::collection::vec("[a-z,\"\n]{0,4}", 1..4)), 0..3),
            delete in proptest::option::of((0u64..20, 0u64..3)),
        ) {
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), delimiter: Some(b','), ..Default::default() };
            let mut d = Document::from_bytes(bytes, opts, None).unwrap();
            for (at, row) in inserts {
                let at = at.min(d.row_count());
                d.insert_rows(at, vec![row]).unwrap();
            }
            for (r, c, v) in edits {
                if d.row_count() > 0 {
                    d.set_cell(r % d.row_count(), c, v).unwrap();
                }
            }
            if let Some((at, n)) = delete {
                if d.row_count() > 0 {
                    let at = at % d.row_count();
                    d.delete_rows(at, n.min(d.row_count() - at)).unwrap();
                }
            }
            // Everything reads back identically, including after an
            // unclosed quote (review finding 1).
            let out = rewrite(&d);
            same_values(&d, &Document::from_bytes(out, opts, None).unwrap())?;
        }

        /// Column inserts, deletes and moves (plus edits and row moves)
        /// rewrite every row through the column map and read back the same.
        #[test]
        fn column_changes_survive_save(
            bytes in any_csvish(),
            ops in proptest::collection::vec((0u8..5, 0u32..8, 0u32..3, 0u32..8, "[a-z,\"\r\n]{0,4}"), 0..10),
        ) {
            let bytes_copy = bytes.clone();
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), delimiter: Some(b','), ..Default::default() };
            let mut d = Document::from_bytes(bytes, opts, None).unwrap();
            for (kind, a, n, b, v) in ops {
                let cols = d.col_count();
                let rows = d.row_count();
                let _ = match kind {
                    0 => d.insert_cols(a.min(cols), n),
                    1 if cols > 0 => d.delete_cols(a % cols, n.min(cols - a % cols)),
                    2 if cols > 0 => {
                        let at = a % cols;
                        let n = n.min(cols - at);
                        d.move_cols(at, n, b.min(cols - n))
                    }
                    3 if rows > 0 => d.set_cell(a as u64 % rows, b, v),
                    4 if rows > 1 => d.move_rows(a as u64 % rows, 1, b as u64 % rows),
                    _ => Ok(()),
                };
            }
            let _ = &bytes_copy;
            let out = rewrite(&d);
            same_values(&d, &Document::from_bytes(out, opts, None).unwrap())?;
        }

        /// Milestone 4: sorts, filters, replace-all, edits under a filter and
        /// undo, in any order. Saving with a filter on writes every row
        /// (hidden ones too) and reads back the same; undoing everything
        /// gives back the original values.
        #[test]
        fn data_ops_survive_save_and_undo(
            bytes in any_csvish(),
            ops in proptest::collection::vec((0u8..8, 0u32..6, 0u32..4, "[a-z,\"
    ]{0,3}"), 0..14),
        ) {
            use crate::filter::{ColumnFilter, Condition, FilterSpec, Rule, TextOp};
            use crate::find::{FindSpec, Pattern, Scope};
            use crate::sort::{SortKey, SortKind, SortSpec};
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), delimiter: Some(b','), ..Default::default() };
            let mut d = Document::from_bytes(bytes.clone(), opts, None).unwrap();
            let original = Document::from_bytes(bytes, opts, None).unwrap();
            for (kind, a, b, v) in ops {
                let cols = d.col_count().max(1);
                let rows = d.row_count();
                let _ = match kind {
                    0 => d.sort(&SortSpec { scope: Default::default(),
                        keys: vec![SortKey {
                            col: a % cols,
                            descending: b % 2 == 1,
                            kind: [SortKind::Auto, SortKind::Text, SortKind::Natural, SortKind::Number][b as usize],
                            match_case: b == 3,
                        }],
                        keep_first: a % 2 == 0,
                        read: Default::default(),
                        lists: Vec::new(),
                    }).map(|_| ()),
                    1 => d.set_filter(&FilterSpec { combine: Default::default(), invert: false,
                        columns: vec![ColumnFilter { enabled: true, negate: false, name: None,
                            col: a % cols,
                            rule: Rule::Conditions {
                                first: Condition::Text { op: TextOp::Contains, text: v.clone(), match_case: false },
                                second: None,
                                or: false,
                            },
                        }],
                        keep_first: b % 2 == 0,
                        read: Default::default(),
                    }).map(|_| ()),
                    2 => {
                        d.clear_filter();
                        Ok(())
                    }
                    3 if !v.is_empty() => d.replace_all(&FindSpec {
                        pattern: Pattern { text: v.clone(), ..Default::default() },
                        scope: Scope::All,
                        skip_first: b % 2 == 0,
                        skip_cols: Vec::new(),
                        ..Default::default()
                    }, "R").map(|_| ()),
                    4 if rows > 0 => d.set_cell(a as u64 % rows, b, v),
                    5 if rows > 0 => d.delete_rows(a as u64 % rows, 1),
                    6 => d.insert_rows((a as u64).min(rows), vec![vec![v]]),
                    7 => {
                        d.undo();
                        Ok(())
                    }
                    _ => Ok(()),
                };
            }
            let out = rewrite(&d);
            d.clear_filter();
            same_values(&d, &Document::from_bytes(out, opts, None).unwrap())?;
            while d.undo() {}
            prop_assert_eq!(d.row_count(), original.row_count());
            for r in 0..d.row_count() {
                prop_assert_eq!(d.row_values(r).unwrap(), original.row_values(r).unwrap());
            }
            prop_assert!(!d.is_modified());
        }

        /// Milestone 4b: fills, pastes, duplicates, blanks, split and merge
        /// of rows and columns, sorted columns, column changes, filters and
        /// undo, in any order. Saving reads back the same; hidden rows are
        /// never lost; undoing everything gives back the original.
        #[test]
        fn reshaping_survives_save_and_undo(
            bytes in any_csvish(),
            ops in proptest::collection::vec((0u8..14, 0u32..6, 0u32..4, "[a-z;, ]{0,3}"), 0..14),
        ) {
            use crate::filter::{ColumnFilter, Condition, FilterSpec, Rule, TextOp};
            use crate::fill::FillMode;
            use crate::ops::{Dir, FillSpec, MergeColumns, PasteSpec, SplitColumn};
            use crate::reshape::{Blanks, Duplicates, MergeRows, SplitRows};
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), delimiter: Some(b','), ..Default::default() };
            let mut d = Document::from_bytes(bytes.clone(), opts, None).unwrap();
            let original = Document::from_bytes(bytes, opts, None).unwrap();
            let p = crate::Progress::new();
            for (kind, a, b, v) in ops {
                let cols = d.col_count().max(1);
                let rows = d.row_count();
                let sep = if v.contains(';') { ";".to_string() } else { " ".to_string() };
                let _ = match kind {
                    0 => d.set_filter(&FilterSpec { combine: Default::default(), invert: false,
                        columns: vec![ColumnFilter { enabled: true, negate: false, name: None,
                            col: a % cols,
                            rule: Rule::Conditions {
                                first: Condition::Text { op: TextOp::Contains, text: v.clone(), match_case: false },
                                second: None,
                                or: false,
                            },
                        }],
                        keep_first: b % 2 == 0,
                        read: Default::default(),
                    }).map(|_| ()),
                    1 => {
                        d.clear_filter();
                        Ok(())
                    }
                    2 if rows > 0 => d
                        .plan_fill(&FillSpec {
                            row: a as u64 % rows,
                            col: a % cols.min(d.col_count().max(1)),
                            rows: 1,
                            cols: 1,
                            dir: [Dir::Down, Dir::Up, Dir::Right, Dir::Left][b as usize],
                            count: 3,
                            mode: if b % 2 == 0 { FillMode::Auto } else { FillMode::AutoSwap },
                            read: Default::default(),
                            skip_cols: Vec::new(),
                        }, &p)
                        .and_then(|plan| d.apply_cells(plan)),
                    3 => d
                        .plan_paste(&PasteSpec { row: (a as u64).min(rows), col: b, transpose: a % 2 == 0, ..Default::default() },
                            vec![vec![v.clone(), "p".into()], vec![String::new()]], &p)
                        .and_then(|plan| d.apply_cells(plan)),
                    4 => d
                        .plan_remove_duplicates(&Duplicates { cols: if b % 2 == 0 { vec![] } else { vec![a % cols] }, keep_first: a % 2 == 0, ..Default::default() }, &p)
                        .and_then(|plan| d.apply_rows(plan)),
                    5 => d
                        .plan_remove_blank_rows(&Blanks { spaces: b % 2 == 0, keep_first: a % 2 == 0 }, &p)
                        .and_then(|plan| d.apply_rows(plan)),
                    6 => d
                        .plan_remove_blank_cols(&Blanks { spaces: b % 2 == 0, keep_first: a % 2 == 0 }, &p)
                        .and_then(|plan| d.apply_cols(plan)),
                    7 => d
                        .plan_split_column(&SplitColumn { col: a % cols, separator: if v.len() > 1 { v.clone() } else { sep }, keep_original: b % 2 == 0, keep_first: a % 2 == 0, mode: [crate::ops::SplitMode::Text, crate::ops::SplitMode::Last, crate::ops::SplitMode::Any, crate::ops::SplitMode::Positions, crate::ops::SplitMode::LettersDigits][(a as usize + b as usize) % 5], positions: vec![b + 1, a], ..Default::default() }, &p)
                        .and_then(|plan| d.apply_cols(plan)),
                    8 if cols > 1 => d
                        .plan_merge_columns(&MergeColumns { col: a % (cols - 1), count: 2, separator: v.clone(), skip_blanks: b % 2 == 0 }, &p)
                        .and_then(|plan| d.apply_cols(plan)),
                    9 => d
                        .plan_split_rows(&SplitRows { col: a % cols, separator: sep, keep_first: b % 2 == 0, ..Default::default() }, &p)
                        .and_then(|plan| d.apply_rows(plan)),
                    10 => d
                        .plan_merge_rows(&MergeRows { keys: vec![a % cols], separator: "|".into(), distinct: b % 2 == 0, ..Default::default() }, &p)
                        .and_then(|plan| d.apply_rows(plan)),
                    11 => d.plan_sort_columns(0, 0, b % 2 == 0).and_then(|plan| d.apply_cols(plan)),
                    12 => d.insert_cols((a).min(d.col_count()), 1),
                    _ => {
                        d.undo();
                        Ok(())
                    }
                };
            }
            let out = rewrite(&d);
            d.clear_filter();
            same_values(&d, &Document::from_bytes(out, opts, None).unwrap())?;
            while d.undo() {}
            prop_assert_eq!(d.row_count(), original.row_count());
            for r in 0..d.row_count() {
                prop_assert_eq!(d.row_values(r).unwrap(), original.row_values(r).unwrap());
            }
            prop_assert!(!d.is_modified());
        }

        /// Milestone 11: every kind of join (new or updated columns, kept
        /// or dropped rows, repeated keys, loose and range keys, placement)
        /// after column moves and with a filter on: hidden rows never
        /// change, saving reads back the same, undo gives the original.
        #[test]
        fn joins_survive_save_and_undo(
            mine in any_csvish(),
            other in any_csvish(),
            choice in proptest::collection::vec(0u32..8, 12),
            filter in proptest::option::of("[a-z0-9]{0,2}"),
        ) {
            use crate::filter::{ColumnFilter, Condition, FilterSpec, Rule, TextOp};
            use crate::join::{JoinSource, JoinSpec, JoinWrite, KeepRows, KeyMatch, Place, Repeats};
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), delimiter: Some(b','), ..Default::default() };
            let mut d = Document::from_bytes(mine.clone(), opts, None).unwrap();
            let o = Document::from_bytes(other, opts, None).unwrap();
            let p = crate::Progress::new();
            if d.col_count() > 1 && choice[0] % 2 == 0 {
                d.move_cols(0, 1, d.col_count() - 1).unwrap();
            }
            let original: Vec<Vec<String>> = (0..d.total_rows()).map(|r| d.row_values(r).unwrap()).collect();
            if let Some(f) = &filter {
                let _ = d.set_filter(&FilterSpec {
                    columns: vec![ColumnFilter { enabled: true, negate: false, name: None, col: 0, rule: Rule::Conditions { first: Condition::Text { op: TextOp::Contains, text: f.clone(), match_case: false }, second: None, or: false } }],
                    ..Default::default()
                });
            }
            let cols = d.col_count().max(1);
            let ocols = o.col_count().max(1);
            let key = choice[1] % cols;
            let okey = choice[2] % ocols;
            let write = [JoinWrite::New, JoinWrite::Overwrite, JoinWrite::FillBlanks, JoinWrite::Nothing][(choice[3] % 4) as usize];
            let repeats = [Repeats::First, Repeats::Last, Repeats::Every, Repeats::Joined, Repeats::Count, Repeats::Sum, Repeats::Min, Repeats::Max][choice[4] as usize];
            let key_match = [KeyMatch::Exact, KeyMatch::Exact, KeyMatch::Below, KeyMatch::Above][(choice[5] % 4) as usize];
            let columns: Vec<u32> = (0..ocols).filter(|&c| c != okey).take(1 + (choice[6] % 2) as usize).collect();
            let width = if repeats == Repeats::Count { 1 } else { columns.len().max(1) };
            let targets: Vec<u32> = (0..cols).filter(|&c| c != key).take(width).collect();
            let spec = JoinSpec {
                keys: vec![key],
                other_keys: vec![okey],
                columns,
                keep_first: choice[7] % 2 == 0,
                other_header: choice[7] % 3 == 0,
                match_case: choice[8] % 2 == 0,
                trim: choice[8] % 3 == 0,
                add_unmatched: choice[9] % 3 == 0 && key_match == KeyMatch::Exact,
                write,
                targets,
                keep_rows: [KeepRows::All, KeepRows::Matched, KeepRows::Unmatched][(choice[10] % 3) as usize],
                repeats,
                separator: "|".into(),
                loose: choice[11] % 2 == 0,
                not_found: if choice[11] % 3 == 0 { "-".into() } else { String::new() },
                place: if choice[9] % 2 == 0 { Place::End } else { Place::AfterKey },
                prefix: "j ".into(),
                key_match,
            };
            let hidden_before: Vec<(u32, Vec<String>)> = (0..d.total_rows())
                .filter_map(|r| {
                    let id = d.doc_id_at(r)?;
                    (!d.view().is_none_or(|v| v.shows(id))).then(|| (id, row_by_id(&d, id)))
                })
                .collect();
            let joined = JoinSource::build(&o, &spec, &p)
                .and_then(|src| d.plan_join(&src, &spec, &p))
                .and_then(|(plan, _)| d.apply_join(plan));
            if let Ok(()) = &joined {
                // Hidden rows keep their values; with new columns, those
                // columns are blank in hidden rows.
                let added = d.col_count() as usize - cols as usize;
                let at = match spec.place {
                    Place::End => cols as usize,
                    Place::AfterKey => key as usize + 1,
                };
                for (id, v) in &hidden_before {
                    let mut want = v.clone();
                    if spec.write == JoinWrite::New && added > 0 {
                        let at = at.min(want.len());
                        want.splice(at..at, std::iter::repeat_n(String::new(), added));
                    }
                    prop_assert_eq!(&row_by_id(&d, *id), &want);
                }
            }
            let out = rewrite(&d);
            d.clear_filter();
            same_values(&d, &Document::from_bytes(out, opts, None).unwrap())?;
            while d.undo() {}
            let back: Vec<Vec<String>> = (0..d.total_rows()).map(|r| d.row_values(r).unwrap()).collect();
            if d.col_count() > 1 && choice[0] % 2 == 0 {
                // The column move is undone too: compare with the file.
                let first = Document::from_bytes(mine, opts, None).unwrap();
                let file: Vec<Vec<String>> = (0..first.total_rows()).map(|r| first.row_values(r).unwrap()).collect();
                prop_assert_eq!(back, file);
            } else {
                prop_assert_eq!(back, original);
            }
        }

        /// Milestone 3b review: the new dialects (several-byte separators,
        /// escapes, merged spaces) survive edits and column changes; a
        /// value that cannot be written is refused, never changed.
        #[test]
        fn new_dialects_survive_changes(
            bytes in proptest::collection::vec(
                prop_oneof![Just(b'|'), Just(b'~'), Just(b':'), Just(b' '), Just(b'"'), Just(b'\\'), Just(b'\n'), Just(b'x'), Just(b'1')],
                0..200,
            ),
            which in 0usize..5,
            ops in proptest::collection::vec((0u8..5, 0u32..6, 0u32..3, 0u32..6, "[a-z|~: \"\\\\]{0,4}"), 0..8),
        ) {
            let dialect = [
                Dialect::custom(b"||", Some(b'"'), None, false),
                Dialect::custom(b"~|~", Some(b'"'), None, false),
                Dialect::custom(b"::", Some(b'"'), Some(b'\\'), false),
                Dialect::custom(b" ", Some(b'"'), None, true),
                Dialect::custom(b"||", None, None, false),
            ]
            .into_iter()
            .nth(which)
            .unwrap()
            .unwrap();
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), dialect: Some(dialect), skip_lines: Some(0), ..Default::default() };
            let mut d = Document::from_bytes(bytes, opts, None).unwrap();
            for (kind, a, n, b, v) in ops {
                let cols = d.col_count();
                let rows = d.row_count();
                let _ = match kind {
                    0 => d.insert_cols(a.min(cols), n),
                    1 if cols > 1 => d.delete_cols(a % cols, n.min(cols - a % cols).min(cols - 1)),
                    2 if cols > 0 => {
                        let at = a % cols;
                        let n = n.min(cols - at);
                        d.move_cols(at, n, b.min(cols - n))
                    }
                    3 if rows > 0 => d.set_cell(a as u64 % rows, b, v),
                    4 if rows > 1 => d.move_rows(a as u64 % rows, 1, b as u64 % rows),
                    _ => Ok(()),
                };
            }
            let mut out = Vec::new();
            match d.write_to(&mut out, d.format(), false) {
                Ok(_) => {
                    let back = Document::from_bytes(out, opts, None).unwrap();
                    if d.index().line_bound_from().is_none() {
                        same_values(&d, &back)?;
                    }
                }
                Err(Error::CannotRepresent { .. }) => {}
                Err(e) => prop_assert!(false, "{e}"),
            }
        }

        /// Review finding 1: any mix of edits, row and column changes, all
        /// undone, gives back the original document and byte-exact saving.
        #[test]
        fn undoing_everything_restores_the_original(
            bytes in any_csvish(),
            ops in proptest::collection::vec((0u8..8, 0u32..8, 0u32..3, 0u32..8, "[a-z,\"\r\n]{0,4}"), 0..14),
            redo_some in 0usize..4,
        ) {
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), delimiter: Some(b','), ..Default::default() };
            let mut d = Document::from_bytes(bytes.clone(), opts, None).unwrap();
            let original = Document::from_bytes(bytes.clone(), opts, None).unwrap();
            for (kind, a, n, b, v) in ops {
                let cols = d.col_count();
                let rows = d.row_count();
                let _ = match kind {
                    0 => d.insert_cols(a.min(cols), n),
                    1 if cols > 0 => d.delete_cols(a % cols, n.min(cols - a % cols)),
                    2 if cols > 0 => {
                        let at = a % cols;
                        let n = n.min(cols - at);
                        d.move_cols(at, n, b.min(cols - n))
                    }
                    3 if rows > 0 => d.set_cell(a as u64 % rows, b, v),
                    4 if rows > 1 => d.move_rows(a as u64 % rows, 1, b as u64 % rows),
                    5 => {
                        let row: Vec<String> = (0..=b).map(|i| format!("{v}{i}")).collect();
                        d.insert_rows((a as u64).min(rows), vec![row])
                    }
                    6 if rows > 0 => d.delete_rows(a as u64 % rows, 1),
                    7 => { d.undo(); Ok(()) }
                    _ => Ok(()),
                };
                // Whatever the state, it saves and reads back the same.
                let out = rewrite(&d);
                same_values(&d, &Document::from_bytes(out, opts, None).unwrap())?;
            }
            for _ in 0..redo_some { d.redo(); }
            while d.undo() {}
            prop_assert!(!d.has_column_changes());
            prop_assert_eq!(d.col_count(), original.col_count());
            prop_assert_eq!(original.row_count(), d.row_count());
            for r in 0..d.row_count() {
                prop_assert_eq!(original.row_values(r).unwrap(), d.row_values(r).unwrap());
            }
            prop_assert_eq!(rewrite(&d), rewrite(&original));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// The full save path (write, verify, replace, reload) on disk.
        #[test]
        fn save_on_disk_round_trips(
            bytes in any_csvish(),
            edits in proptest::collection::vec((0u64..20, 0u32..4, "[a-z,\"\r\n]{0,5}"), 0..5),
        ) {
            let dir = tmpdir("prop");
            let p = dir.join("t.csv");
            fs::write(&p, &bytes).unwrap();
            let opts = OpenOptions { encoding: Some(Encoding::Utf8), delimiter: Some(b','), ..Default::default() };
            let mut d = Document::open_indexed(&p, opts, None).unwrap();
            for (r, c, v) in edits {
                if d.row_count() > 0 {
                    d.set_cell(r % d.row_count(), c, v).unwrap();
                }
            }
            let want: Vec<_> = (0..d.row_count()).map(|r| d.row_values(r).unwrap()).collect();
            let r = d.save(SaveOptions { rewrite: true, ..Default::default() });
            prop_assert!(r.is_ok(), "{:?}", r);
            let back = Document::open_indexed(&p, opts, None).unwrap();
            let got: Vec<_> = (0..back.row_count()).map(|r| back.row_values(r).unwrap()).collect();
            prop_assert_eq!(got, want);
            prop_assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
            drop(d);
            drop(back);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod write_atomic_tests {
    use super::*;

    fn temps(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .is_ok_and(|e| own_temp(&e.file_name().to_string_lossy()).is_some())
            })
            .count()
    }

    #[test]
    fn a_failure_part_way_leaves_no_file_and_no_temp() {
        let dir = std::env::temp_dir().join(format!("nd-atomic-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("out.csv");
        let r: Result<(), Error> = write_atomic(&target, |w| {
            w.write_all(&[b'x'; 3 << 20])?;
            Err(Error::Cancelled)
        });
        assert!(r.is_err());
        assert!(!target.exists());
        assert_eq!(temps(&dir), 0);
        // A file already there is left as it was.
        fs::write(&target, b"before").unwrap();
        let r: Result<(), Error> = write_atomic(&target, |w| {
            w.write_all(b"half")?;
            Err(Error::Invalid("stop".into()))
        });
        assert!(r.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"before");
        assert_eq!(temps(&dir), 0);
        // And a good write replaces it.
        write_atomic(&target, |w| Ok(w.write_all(b"after")?)).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"after");
        assert_eq!(temps(&dir), 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
