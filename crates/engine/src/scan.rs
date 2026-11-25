//! The one row/field state machine.
//!
//! Both the row indexer and the field reader call [`scan_row`], so where a row
//! ends can never disagree with how its fields are read. Rules:
//!
//! * A quote opens a quoted field only at the start of a field.
//! * Inside quotes, two quotes are one literal quote; any other quote closes.
//! * Text between a closing quote and the next delimiter or line break is kept
//!   literally and the row is marked [`TEXT_AFTER_QUOTE`].
//! * A row ends at CRLF, LF or a lone CR outside quotes.
//! * In [`Mode::Normal`] line breaks inside quotes belong to the cell. In
//!   [`Mode::LineBound`] (used after an unterminated quote at end of file) a
//!   line break always ends the row and the open quote is marked
//!   [`UNCLOSED_QUOTE`].
//!
//! The scanner jumps between interesting bytes with `memchr`, so its cost is
//! close to a memory scan and it never looks back, which keeps total work
//! linear in the file size whatever the input.

use std::borrow::Cow;

use memchr::{memchr, memchr2, memchr3};

use crate::dialect::Dialect;

/// Repair flag: text followed a closing quote.
pub const TEXT_AFTER_QUOTE: u8 = 1;
/// Repair flag: a quoted field was cut by a line break or end of file.
pub const UNCLOSED_QUOTE: u8 = 2;
/// Review flag: a quoted cell has [`LONG_CELL_BREAKS`] or more line breaks,
/// which often means a stray quote swallowed the rows after it.
pub const LONG_QUOTED_CELL: u8 = 4;
/// Line breaks inside one quoted cell from which it is flagged.
pub const LONG_CELL_BREAKS: usize = 8;

/// Line breaks in s (CRLF counts once).
pub fn line_breaks_in(s: &[u8]) -> usize {
    let lf = memchr::memchr_iter(b'\n', s).count();
    let lone_cr = memchr::memchr_iter(b'\r', s)
        .filter(|&i| s.get(i + 1) != Some(&b'\n'))
        .count();
    lf + lone_cr
}

/// Whether quoted fields may span line breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    LineBound,
}

/// Where a scanned row ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowEnd {
    /// End of the row's fields (start of its line break, or end of buffer).
    pub content_end: usize,
    /// Start of the next row.
    pub next: usize,
    /// Number of fields (at least 1, saturating).
    pub fields: u32,
    /// Repair flags (`TEXT_AFTER_QUOTE`, `UNCLOSED_QUOTE`, `LONG_QUOTED_CELL`).
    pub repairs: u8,
    /// Normal mode only: a quoted field ran to the end of the buffer. The
    /// other values are then meaningless and the caller must rescan in
    /// line-bound mode.
    pub unterminated: bool,
}

/// Offset of the next separator, CR or LF at or after `pos` (or `buf.len()`).
/// The one-byte case (nearly every file) is a single `memchr3`.
#[inline(always)]
fn next_stop(buf: &[u8], pos: usize, dl: &[u8]) -> usize {
    match memchr3(dl[0], b'\r', b'\n', &buf[pos..]) {
        None => buf.len(),
        Some(i) if dl.len() == 1 => pos + i,
        Some(i) => next_stop_multi(buf, pos + i, dl),
    }
}

/// `next_stop` for separators of several bytes, from a candidate at `pos`.
#[inline(never)]
fn next_stop_multi(buf: &[u8], mut pos: usize, dl: &[u8]) -> usize {
    loop {
        if buf[pos] != dl[0] || buf[pos..].starts_with(dl) {
            return pos;
        }
        pos += 1;
        match memchr3(dl[0], b'\r', b'\n', &buf[pos..]) {
            None => return buf.len(),
            Some(i) => pos += i,
        }
    }
}

/// Inside a quoted field of a dialect with an escape byte: the end of the
/// field (just past its closing quote) or `Err(end)` when it never closes
/// (end of buffer, or a line break in line-bound mode).
fn quoted_with_escape(
    buf: &[u8],
    mut pos: usize,
    q: u8,
    e: u8,
    mode: Mode,
) -> Result<usize, usize> {
    let len = buf.len();
    let line_bound = mode == Mode::LineBound;
    loop {
        let hit = if line_bound {
            buf[pos..]
                .iter()
                .position(|&b| b == q || b == e || b == b'\r' || b == b'\n')
        } else {
            memchr2(q, e, &buf[pos..])
        };
        let Some(i) = hit else {
            return Err(len);
        };
        pos += i;
        let b = buf[pos];
        if b == e {
            match buf.get(pos + 1) {
                None => return Err(len),
                // A line break always ends a line-bound row.
                Some(b'\r' | b'\n') if line_bound => pos += 1,
                Some(_) => pos += 2,
            }
            continue;
        }
        if b != q {
            return Err(pos); // line break, line-bound mode
        }
        pos += 1;
        if pos < len && buf[pos] == q {
            pos += 1; // doubled quote
            continue;
        }
        return Ok(pos);
    }
}

/// Scan one row of `buf` starting at `start`.
///
/// `on_field(start, end)` receives each field's raw byte range (quotes
/// included); returning `false` stops the scan early, in which case the
/// returned `RowEnd` only describes the part scanned.
#[inline]
pub fn scan_row(
    buf: &[u8],
    start: usize,
    d: Dialect,
    mode: Mode,
    mut on_field: impl FnMut(usize, usize) -> bool,
) -> RowEnd {
    let len = buf.len();
    let dl = d.delimiter();
    let d0 = dl[0];
    let mut pos = start.min(len);
    if d.merges() {
        while pos < len && buf[pos] == d0 {
            pos += 1;
        }
    }
    let mut fields: u32 = 1;
    let mut repairs = 0u8;
    let end = |content_end, next, fields, repairs| RowEnd {
        content_end,
        next,
        fields,
        repairs,
        unterminated: false,
    };
    loop {
        let field_start = pos;
        match d.quote() {
            Some(q) if pos < len && buf[pos] == q => {
                pos += 1;
                let closed = match d.escape() {
                    Some(e) => match quoted_with_escape(buf, pos, q, e, mode) {
                        Ok(p) => {
                            pos = p;
                            true
                        }
                        Err(p) => {
                            pos = p;
                            false
                        }
                    },
                    None => loop {
                        let hit = match mode {
                            Mode::Normal => memchr(q, &buf[pos..]),
                            Mode::LineBound => memchr3(q, b'\r', b'\n', &buf[pos..]),
                        };
                        let Some(i) = hit else {
                            pos = len;
                            break false;
                        };
                        pos += i;
                        if buf[pos] != q {
                            break false; // line break, line-bound mode
                        }
                        pos += 1;
                        if pos < len && buf[pos] == q {
                            pos += 1; // escaped quote
                            continue;
                        }
                        break true;
                    },
                };
                if closed {
                    let body = &buf[field_start + 1..pos - 1];
                    if mode == Mode::Normal
                        && memchr::memchr2(b'\r', b'\n', body).is_some()
                        && line_breaks_in(body) >= LONG_CELL_BREAKS
                    {
                        repairs |= LONG_QUOTED_CELL;
                    }
                    let rest = next_stop(buf, pos, dl);
                    if rest > pos {
                        repairs |= TEXT_AFTER_QUOTE;
                    }
                    pos = rest;
                } else if mode == Mode::Normal {
                    return RowEnd {
                        content_end: len,
                        next: len,
                        fields,
                        repairs,
                        unterminated: true,
                    };
                } else {
                    repairs |= UNCLOSED_QUOTE;
                }
            }
            _ => pos = next_stop(buf, pos, dl),
        }
        if !on_field(field_start, pos) {
            return end(pos, pos, fields, repairs);
        }
        if pos == len {
            return end(len, len, fields, repairs);
        }
        if buf[pos] == d0 {
            // next_stop and the quote path only stop at a whole separator.
            pos += dl.len();
            if !d.merges() {
                fields = fields.saturating_add(1);
                continue;
            }
            while pos < len && buf[pos] == d0 {
                pos += 1;
            }
            if pos == len {
                return end(len, len, fields, repairs);
            }
            if !matches!(buf[pos], b'\r' | b'\n') {
                fields = fields.saturating_add(1);
                continue;
            }
            // A run just before the line break ends the row: fall through.
        }
        if buf[pos] == b'\r' {
            let next = if buf.get(pos + 1) == Some(&b'\n') {
                pos + 2
            } else {
                pos + 1
            };
            return end(pos, next, fields, repairs);
        }
        return end(pos, pos + 1, fields, repairs); // LF
    }
}

/// The value of a raw field: quotes removed, doubled quotes collapsed and
/// escapes applied.
#[inline]
pub fn unquote(raw: &[u8], d: Dialect) -> Cow<'_, [u8]> {
    match d.quote() {
        Some(q) if raw.first() == Some(&q) => unquote_quoted(raw, q, d.escape()),
        _ => Cow::Borrowed(raw),
    }
}

/// [`unquote`] for a field that starts with the quote `q`.
fn unquote_quoted(raw: &[u8], q: u8, e: Option<u8>) -> Cow<'_, [u8]> {
    let body = &raw[1..];
    // Common case: the only other quote is the closing one at the end.
    let special = match e {
        Some(e) => memchr2(q, e, body),
        None => memchr(q, body),
    };
    match special {
        None => return Cow::Borrowed(body),
        Some(i) if i + 1 == body.len() && body[i] == q => return Cow::Borrowed(&body[..i]),
        _ => {}
    }
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        let b = body[i];
        if Some(b) == e {
            match body.get(i + 1) {
                Some(&n) => {
                    out.push(n);
                    i += 2;
                }
                None => {
                    out.push(b); // a lone escape at the end of an unclosed cell
                    i += 1;
                }
            }
            continue;
        }
        if b == q {
            if body.get(i + 1) == Some(&q) {
                out.push(q);
                i += 2;
                continue;
            }
            // Closing quote: anything after it is literal.
            out.extend_from_slice(&body[i + 1..]);
            break;
        }
        out.push(b);
        i += 1;
    }
    Cow::Owned(out)
}

/// Append what closes a quoted cell that never closed, given its raw bytes
/// (or the row's, which end with that cell): the quote, preceded by an
/// escape when the cell ends with a lone escape byte (which would otherwise
/// escape the closing quote).
pub fn close_unclosed(raw: &[u8], d: Dialect, out: &mut Vec<u8>) {
    let Some(q) = d.quote() else { return };
    if let Some(e) = d.escape() {
        // Escapes pair up left to right inside a run, so an odd run at the
        // end leaves one lone escape. The run cannot reach the opening
        // quote, which is not an escape byte.
        let run = raw.iter().rev().take_while(|&&b| b == e).count();
        if run % 2 == 1 {
            out.push(e);
        }
    }
    out.push(q);
}

/// True when a raw cell followed by the separator would read
/// differently: with a separator of several bytes, the cell's end and the
/// separator's start can form an earlier separator (`x|` + `||`, `a~|` +
/// `~|~`).
pub fn joins_separator(raw: &[u8], d: Dialect) -> bool {
    let dl = d.delimiter();
    // A cleanly quoted cell ends with its quote, which no separator holds
    // (text after a closing quote does not count as clean).
    let clean = d
        .quote()
        .is_some_and(|q| raw.len() >= 2 && raw[0] == q && raw[raw.len() - 1] == q);
    if dl.len() < 2 || raw.is_empty() || clean {
        return false;
    }
    let k = (dl.len() - 1).min(raw.len());
    let mut joined = [0u8; 2 * crate::dialect::MAX_DELIM];
    joined[..k].copy_from_slice(&raw[raw.len() - k..]);
    joined[k..k + dl.len()].copy_from_slice(dl);
    memchr::memmem::find(&joined[..k + dl.len()], dl).is_some_and(|i| i < k)
}

/// Append a cell's value (unquoted bytes) as a quoted cell. False when the
/// dialect has no quote character.
pub fn quote_raw(raw: &[u8], d: Dialect, out: &mut Vec<u8>) -> bool {
    let Some(q) = d.quote() else {
        return false;
    };
    out.push(q);
    for &b in raw {
        if b == q {
            out.push(d.escape().unwrap_or(q));
        } else if Some(b) == d.escape() {
            out.push(b);
        }
        out.push(b);
    }
    out.push(q);
    true
}

/// True when `value` must be quoted to survive a round trip in `d`.
pub fn needs_quotes(value: &str, d: Dialect) -> bool {
    let v = value.as_bytes();
    let dl = d.delimiter();
    match d.quote() {
        Some(q) => {
            (d.merges() && v.is_empty())
                || v.iter().any(|&b| {
                    dl.contains(&b) || b == q || Some(b) == d.escape() || b == b'\r' || b == b'\n'
                })
        }
        None => false,
    }
}

/// True when `value` cannot be written at all in a dialect without quotes.
pub fn unrepresentable(value: &str, d: Dialect) -> bool {
    let dl = d.delimiter();
    d.quote().is_none()
        && ((d.merges() && value.is_empty())
            || value
                .as_bytes()
                .iter()
                .any(|&b| dl.contains(&b) || b == b'\r' || b == b'\n'))
}

/// Append `value` as one field to `out`, quoting only when needed.
pub fn write_field(value: &str, d: Dialect, out: &mut String) {
    write_field_as(value, d, Quoting::Needed, out)
}

/// A value whose whole text is a signed number: digits, with `.` or `,` only
/// between digits (grouping or decimal mark), an optional exponent and `%`.
fn signed_number(v: &str) -> bool {
    let Some(rest) = v.strip_prefix(['+', '-']) else {
        return false;
    };
    let rest = rest.strip_suffix('%').unwrap_or(rest);
    let (body, exponent) = match rest.split_once(['e', 'E']) {
        Some((b, e)) => (b, Some(e)),
        None => (rest, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if exponent.is_some_and(|e| !digits(e.strip_prefix(['+', '-']).unwrap_or(e))) {
        return false;
    }
    // A leading mark (".5") is a number; two marks together or a trailing one
    // after a grouped number are not.
    let body = body.strip_prefix(['.', ',']).unwrap_or(body);
    body.split(['.', ',']).all(digits)
}

/// True when a spreadsheet could run this cell's text as a formula: it starts
/// (after any spaces) with `=`, `+`, `-`, `@`, their full-width forms, a tab
/// or a carriage return. A signed number such as `-1,234.50` is left alone;
/// `=` never is.
pub fn formula_risk(value: &str) -> bool {
    let t = value.trim_start_matches([' ', '\u{a0}', '\u{3000}']);
    match t.chars().next() {
        Some('=' | '@' | '\t' | '\r' | '\u{ff1d}' | '\u{ff0b}' | '\u{ff0d}' | '\u{ff20}') => true,
        Some('+' | '-') => !signed_number(t),
        _ => false,
    }
}

/// `value` as it goes into a guarded CSV: with one apostrophe in front when
/// [`formula_risk`] says so, so a spreadsheet shows it as text.
pub fn guard_formula(value: &str) -> std::borrow::Cow<'_, str> {
    if formula_risk(value) {
        std::borrow::Cow::Owned(format!("'{value}"))
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

/// [`write_field`] with the formula guard when `guard` is on; true when the
/// value got an apostrophe.
pub fn write_field_guarded(value: &str, d: Dialect, guard: bool, out: &mut String) -> bool {
    if guard && formula_risk(value) {
        write_field(&guard_formula(value), d, out);
        true
    } else {
        write_field(value, d, out);
        false
    }
}

/// How values are put in quotes when a file is written out again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quoting {
    /// Rows nobody changed keep their bytes; changed cells are quoted only
    /// where needed.
    #[default]
    Keep,
    /// Quote only values that need it (separator, quote, line break).
    Needed,
    /// Quote every value, empty ones too.
    All,
    /// Quote every value except plain numbers (and empty cells).
    Text,
}

/// True for a plain number: optional sign, digits with at most one decimal
/// point or comma, optional exponent. No spaces, no grouping.
pub fn plain_number(v: &str) -> bool {
    let b = v.as_bytes();
    let mut i = usize::from(matches!(b.first(), Some(b'-' | b'+')));
    let mut digits = 0;
    let mut point = false;
    while i < b.len() {
        match b[i] {
            b'0'..=b'9' => digits += 1,
            b'.' | b',' if !point => point = true,
            b'e' | b'E' if digits > 0 => {
                i += 1;
                if matches!(b.get(i), Some(b'-' | b'+')) {
                    i += 1;
                }
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                return i == b.len() && i > start;
            }
            _ => return false,
        }
        i += 1;
    }
    digits > 0
}

/// Append `value` as one field, quoted as `quoting` says (and always when
/// the value needs it).
pub fn write_field_as(value: &str, d: Dialect, quoting: Quoting, out: &mut String) {
    let quote = match quoting {
        Quoting::Keep | Quoting::Needed => needs_quotes(value, d),
        Quoting::All => true,
        Quoting::Text => needs_quotes(value, d) || !(value.is_empty() || plain_number(value)),
    };
    match d.quote() {
        Some(q) if quote => {
            let qc = q as char;
            let ec = d.escape().map(|e| e as char);
            out.push(qc);
            let special = match d.escape() {
                Some(e) => memchr2(q, e, value.as_bytes()),
                None => memchr(q, value.as_bytes()),
            };
            match special {
                // Most values hold no quote: copy them whole.
                None => out.push_str(value),
                Some(first) => {
                    out.push_str(&value[..first]);
                    for c in value[first..].chars() {
                        if c == qc {
                            out.push(ec.unwrap_or(qc));
                        } else if Some(c) == ec {
                            out.push(c);
                        }
                        out.push(c);
                    }
                }
            }
            out.push(qc);
        }
        _ => out.push_str(value),
    }
}

#[cfg(test)]
mod formula_guard_tests {
    use super::*;

    #[test]
    fn risky_starts_get_one_apostrophe() {
        for v in [
            "=1+1",
            "=",
            "=-5",
            "+1 555 0100",
            "-1+1",
            "-Smith",
            "@SUM(A1)",
            "+",
            "-",
            "--5",
            "\tx",
            "\t5",
            "\rx",
            " =1",
            "   @a",
            "\u{a0}=1",
            "\u{ff1d}1+1",
            "\u{ff0b}a",
            "\u{ff0d}a",
            "\u{ff20}a",
            "\"=1+1\"x".trim_start_matches('"'),
            "-1 000",
            "-5 ",
            "+1..2",
            "-1,",
            "-e5",
            "-1e",
            "-%",
            "=5",
            "+%5",
        ] {
            assert!(formula_risk(v), "{v:?}");
            assert_eq!(guard_formula(v), format!("'{v}"), "{v:?}");
            assert!(!formula_risk(&guard_formula(v)), "never twice: {v:?}");
        }
    }

    #[test]
    fn numbers_and_plain_text_are_left_alone() {
        for v in [
            "",
            " ",
            "a",
            "a=b",
            "1-1",
            "5",
            "'=1",
            "-5",
            "+1.5",
            "-1,234.50",
            "-1.234,50",
            "-1,5",
            "1e5",
            "-1e5",
            "+2.5E-3",
            "12%",
            "-12%",
            "+.5",
            "-,5",
            " -5",
            "email@example.com",
            "\"=1+1\"",
        ] {
            assert!(!formula_risk(v), "{v:?}");
            assert_eq!(guard_formula(v), v);
        }
    }

    #[test]
    fn the_test_runs_on_the_text_and_quoting_comes_after() {
        for d in [Dialect::CSV, Dialect::TSV] {
            let mut on = String::new();
            let mut off = String::new();
            let mut plain = String::new();
            assert!(write_field_guarded("=a,\"b\"", d, true, &mut on));
            assert!(!write_field_guarded("=a,\"b\"", d, false, &mut off));
            write_field("=a,\"b\"", d, &mut plain);
            assert_eq!(off, plain);
            assert!(on.trim_start_matches('"').starts_with("'="), "{on}");
        }
        let mut semi = String::new();
        let d = Dialect::new(b';', Some(b'"')).unwrap();
        assert!(write_field_guarded("@a;b", d, true, &mut semi));
        assert!(semi.contains("'@a;b"), "{semi}");
    }
}

#[cfg(test)]
pub(crate) mod reference {
    //! Byte-at-a-time parser written straight from the rules, used only to
    //! check the fast scanner.
    use super::*;

    pub struct Row {
        pub start: usize,
        pub next: usize,
        pub fields: Vec<Vec<u8>>,
        pub repairs: u8,
    }

    pub fn parse(buf: &[u8], d: Dialect) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut mode = Mode::Normal;
        let mut pos = 0;
        while pos < buf.len() {
            match parse_row(buf, pos, d, mode) {
                Some(r) => {
                    pos = r.next;
                    rows.push(r);
                }
                None => mode = Mode::LineBound,
            }
        }
        rows
    }

    fn parse_row(buf: &[u8], start: usize, d: Dialect, mode: Mode) -> Option<Row> {
        #[derive(PartialEq)]
        enum S {
            FieldStart,
            Unquoted,
            Quoted,
            AfterQuote,
        }
        let mut s = S::FieldStart;
        let mut qstart = 0;
        let mut fields = vec![Vec::new()];
        let mut repairs = 0;
        let mut i = start;
        let dl = d.delimiter();
        if d.merges() {
            while buf.get(i) == Some(&dl[0]) {
                i += 1;
            }
        }
        loop {
            let Some(&b) = buf.get(i) else {
                if s == S::Quoted {
                    if mode == Mode::Normal {
                        return None;
                    }
                    repairs |= UNCLOSED_QUOTE;
                }
                return Some(Row {
                    start,
                    next: i,
                    fields,
                    repairs,
                });
            };
            let cur = fields.last_mut().unwrap();
            let brk = b == b'\r' || b == b'\n';
            let next_of = |i: usize| {
                if b == b'\r' && buf.get(i + 1) == Some(&b'\n') {
                    i + 2
                } else {
                    i + 1
                }
            };
            match s {
                S::Quoted => {
                    if Some(b) == d.escape() {
                        match buf.get(i + 1) {
                            Some(&n)
                                if !(mode == Mode::LineBound && (n == b'\r' || n == b'\n')) =>
                            {
                                cur.push(n);
                                i += 2;
                            }
                            _ => {
                                cur.push(b);
                                i += 1;
                            }
                        }
                        continue;
                    }
                    if Some(b) == d.quote() {
                        if buf.get(i + 1).copied() == d.quote() {
                            cur.push(b);
                            i += 2;
                            continue;
                        }
                        s = S::AfterQuote;
                        if mode == Mode::Normal
                            && line_breaks_in(&buf[qstart + 1..i]) >= LONG_CELL_BREAKS
                        {
                            repairs |= LONG_QUOTED_CELL;
                        }
                    } else if brk && mode == Mode::LineBound {
                        repairs |= UNCLOSED_QUOTE;
                        return Some(Row {
                            start,
                            next: next_of(i),
                            fields,
                            repairs,
                        });
                    } else {
                        cur.push(b);
                    }
                }
                _ if buf[i..].starts_with(dl) => {
                    i += dl.len();
                    if d.merges() {
                        while buf.get(i) == Some(&dl[0]) {
                            i += 1;
                        }
                        if matches!(buf.get(i), None | Some(b'\r' | b'\n')) {
                            continue;
                        }
                    }
                    fields.push(Vec::new());
                    s = S::FieldStart;
                    continue;
                }
                _ if brk => {
                    return Some(Row {
                        start,
                        next: next_of(i),
                        fields,
                        repairs,
                    })
                }
                S::FieldStart if Some(b) == d.quote() => {
                    s = S::Quoted;
                    qstart = i;
                }
                S::FieldStart | S::Unquoted => {
                    cur.push(b);
                    s = S::Unquoted;
                }
                S::AfterQuote => {
                    repairs |= TEXT_AFTER_QUOTE;
                    cur.push(b);
                }
            }
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rows(buf: &[u8], d: Dialect) -> Vec<(Vec<Vec<u8>>, u8)> {
        let mut out = Vec::new();
        let mut pos = 0;
        let mut mode = Mode::Normal;
        while pos < buf.len() {
            let mut f = Vec::new();
            let e = scan_row(buf, pos, d, mode, |s, e| {
                f.push(unquote(&buf[s..e], d).into_owned());
                true
            });
            if e.unterminated {
                mode = Mode::LineBound;
                continue;
            }
            out.push((f, e.repairs));
            pos = e.next;
        }
        out
    }

    fn s(v: &[&str]) -> Vec<Vec<u8>> {
        v.iter().map(|x| x.as_bytes().to_vec()).collect()
    }

    #[test]
    fn basic_rows_and_breaks() {
        let r = rows(b"a,b\r\nc,d\ne\rf,", Dialect::CSV);
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].0, s(&["a", "b"]));
        assert_eq!(r[2].0, s(&["e"]));
        assert_eq!(r[3].0, s(&["f", ""]));
    }

    #[test]
    fn quoted_fields() {
        let r = rows(
            b"\"a,b\",\"say \"\"hi\"\"\"\n\"multi\r\nline\",x\n",
            Dialect::CSV,
        );
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].0, s(&["a,b", "say \"hi\""]));
        assert_eq!(r[1].0, s(&["multi\r\nline", "x"]));
        assert_eq!(r[1].1, 0);
    }

    #[test]
    fn quote_mid_field_is_literal() {
        let r = rows(b"ab\"c,d\n", Dialect::CSV);
        assert_eq!(r[0].0, s(&["ab\"c", "d"]));
        assert_eq!(r[0].1, 0);
    }

    #[test]
    fn text_after_closing_quote() {
        let r = rows(b"\"ab\"cd,e\n", Dialect::CSV);
        assert_eq!(r[0].0, s(&["abcd", "e"]));
        assert_eq!(r[0].1, TEXT_AFTER_QUOTE);
    }

    #[test]
    fn unterminated_quote_falls_back_to_line_bound() {
        let r = rows(b"a,b\n\"oops,c\nd,e\n", Dialect::CSV);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1].0, s(&["oops,c"]));
        assert_eq!(r[1].1, UNCLOSED_QUOTE);
        assert_eq!(r[2].0, s(&["d", "e"]));
    }

    #[test]
    fn stray_quote_swallowing_rows_is_flagged() {
        let mut text = b"id,v\n1,\"stray\n".to_vec();
        for i in 2..12 {
            text.extend_from_slice(format!("{i},x\n").as_bytes());
        }
        text.extend_from_slice(b"\"\n13,y\n");
        let r = rows(&text, Dialect::CSV);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1].1, LONG_QUOTED_CELL);
        // Exactly 8 swallowed lines (the reviewer's case) is flagged too.
        let eight = b"\"a\n1\n2\n3\n4\n5\n6\n7\n\",x\n";
        assert_eq!(rows(eight, Dialect::CSV)[0].1, LONG_QUOTED_CELL);
        // A short multi-line note is not flagged.
        assert_eq!(rows(b"\"a\nb\nc\",d\n", Dialect::CSV)[0].1, 0);
        assert_eq!(line_breaks_in(b"a\r\nb\rc\n"), 3);
    }

    #[test]
    fn no_quote_dialect() {
        let d = Dialect::new(b'\t', None).unwrap();
        let r = rows(b"\"a\t\"b\n", d);
        assert_eq!(r[0].0, s(&["\"a", "\"b"]));
    }

    #[test]
    fn multi_character_separators() {
        let d = Dialect::custom(b"||", Some(b'"'), None, false).unwrap();
        let r = rows(b"a||b|c||\"x||y\"\n|||z\n", d);
        assert_eq!(r[0].0, s(&["a", "b|c", "x||y"]));
        assert_eq!(r[0].1, 0);
        // The first whole separator wins, left to right.
        assert_eq!(r[1].0, s(&["", "|z"]));
        let d = Dialect::custom(b"~|~", None, None, false).unwrap();
        assert_eq!(rows(b"a~|~b~|c~|~~|~\n", d)[0].0, s(&["a", "b~|c", "", ""]));
    }

    #[test]
    fn control_character_separator() {
        let d = Dialect::new(0x1F, Some(b'"')).unwrap();
        assert_eq!(rows(b"a\x1fb,c\x1f\n", d)[0].0, s(&["a", "b,c", ""]));
    }

    #[test]
    fn backslash_escapes() {
        let d = Dialect::custom(b",", Some(b'"'), Some(b'\\'), false).unwrap();
        let r = rows(br#""say \"hi\"","c:\\x","q""q""#, d);
        assert_eq!(r[0].0, s(&["say \"hi\"", "c:\\x", "q\"q"]));
        let mut out = String::new();
        write_field("a\"b\\c", d, &mut out);
        assert_eq!(out, r#""a\"b\\c""#);
        // Outside quotes a backslash is plain text.
        assert_eq!(rows(b"a\\,b\n", d)[0].0, s(&["a\\", "b"]));
    }

    #[test]
    fn merged_space_runs() {
        let d = Dialect::custom(b" ", Some(b'"'), None, true).unwrap();
        let r = rows(b"  id   name    score  \n   7  \"Ann Lee\"  9.5\n\n", d);
        assert_eq!(r[0].0, s(&["id", "name", "score"]));
        assert_eq!(r[1].0, s(&["7", "Ann Lee", "9.5"]));
        assert_eq!(r[2].0, s(&[""]));
        let mut out = String::new();
        write_field("", d, &mut out);
        assert_eq!(out, "\"\"");
        assert!(unrepresentable(
            "",
            Dialect::custom(b" ", None, None, true).unwrap()
        ));
    }

    #[test]
    fn early_stop() {
        let mut n = 0;
        scan_row(b"a,b,c,d\n", 0, Dialect::CSV, Mode::Normal, |_, _| {
            n += 1;
            n < 2
        });
        assert_eq!(n, 2);
    }

    #[test]
    fn writer_quotes_only_when_needed() {
        let mut out = String::new();
        for v in ["plain", "a,b", "q\"x", "line\nbreak", " pad "] {
            write_field(v, Dialect::CSV, &mut out);
            out.push('|');
        }
        assert_eq!(out, "plain|\"a,b\"|\"q\"\"x\"|\"line\nbreak\"| pad |");
        let none = Dialect::new(b',', None).unwrap();
        assert!(unrepresentable("a,b", none));
        assert!(!unrepresentable("a\"b", none));
    }

    fn dialects() -> impl Strategy<Value = Dialect> {
        prop_oneof![
            Just(Dialect::CSV),
            Just(Dialect::TSV),
            Just(Dialect::new(b';', Some(b'\'')).unwrap()),
            Just(Dialect::new(b'|', None).unwrap()),
            Just(Dialect::new(0x1F, Some(b'"')).unwrap()),
            Just(Dialect::custom(b"||", Some(b'"'), None, false).unwrap()),
            Just(Dialect::custom(b"~|~", None, None, false).unwrap()),
            Just(Dialect::custom(b",", Some(b'"'), Some(b'\\'), false).unwrap()),
            Just(Dialect::custom(b" ", Some(b'"'), None, true).unwrap()),
            Just(Dialect::custom(b" ", None, None, true).unwrap()),
        ]
    }

    /// Bytes biased towards the structural characters.
    fn tricky_bytes() -> impl Strategy<Value = Vec<u8>> {
        proptest::collection::vec(
            prop_oneof![
                Just(b','),
                Just(b'"'),
                Just(b'\''),
                Just(b'\t'),
                Just(b';'),
                Just(b'|'),
                Just(b'\r'),
                Just(b'\n'),
                Just(b'a'),
                Just(b'\\'),
                Just(b' '),
                Just(b'~'),
                Just(0x1F),
                any::<u8>(),
            ],
            0..200,
        )
    }

    proptest! {
        #[test]
        fn fast_scanner_matches_reference(buf in tricky_bytes(), d in dialects()) {
            let reference = reference::parse(&buf, d);
            let fast = rows(&buf, d);
            prop_assert_eq!(reference.len(), fast.len());
            for (r, (f, rep)) in reference.iter().zip(&fast) {
                prop_assert_eq!(&r.fields, f);
                prop_assert_eq!(r.repairs, *rep);
            }
            // Rows tile the buffer exactly.
            let mut pos = 0;
            for r in &reference {
                prop_assert_eq!(r.start, pos);
                pos = r.next;
            }
            prop_assert_eq!(pos, buf.len());
        }

        #[test]
        fn written_fields_read_back(values in proptest::collection::vec("[a-z,\"'\t;|\r\n ~\\\\\u{1f}]{0,8}", 1..6), d in dialects()) {
            // Without quotes some characters cannot be stored at all; drop them.
            let delim: Vec<char> = d.delimiter().iter().map(|&b| b as char).collect();
            let values: Vec<String> = values
                .into_iter()
                .map(|v| if d.quote().is_none() { v.replace(|c: char| delim.contains(&c) || c == '\r' || c == '\n', "") } else { v })
                .map(|v| if unrepresentable(&v, d) { "x".into() } else { v })
                .collect();
            // A single empty field would be a blank line; still one field.
            let mut line = String::new();
            for (i, v) in values.iter().enumerate() {
                if i > 0 { line.push_str(d.delimiter_str()); }
                write_field(v, d, &mut line);
            }
            line.push('\n');
            let r = rows(line.as_bytes(), d);
            prop_assert_eq!(r.len(), 1);
            let want: Vec<Vec<u8>> = values.iter().map(|v| v.as_bytes().to_vec()).collect();
            prop_assert_eq!(&r[0].0, &want);
            prop_assert_eq!(r[0].1, 0);
        }
    }
}
