//! Privacy tools (Milestone 13): the keyed scramble behind the "scramble"
//! cleaning step, the private data finder, and share-safe copies (a copy of
//! the table with columns dropped or masked; the open file never changes).

use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::clean::{Cleaner, Compiled, Ctx};
use crate::document::{Document, Progress};
use crate::ops::Cols;
use crate::value::ReadOptions;
use crate::Error;

/// A secret phrase. Never written to a file (cleaners and recipes save the
/// step without it) and never shown in logs.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into())
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
    pub(crate) fn chars(&self) -> usize {
        self.0.chars().count()
    }
}

impl Drop for Secret {
    /// The phrase is overwritten before its memory is given back.
    fn drop(&mut self) {
        let mut v = std::mem::take(&mut self.0).into_bytes();
        wipe(&mut v);
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// Shortest secret phrase the scramble accepts.
pub const MIN_PHRASE_CHARS: usize = 16;
/// PBKDF2-HMAC-SHA-256 rounds that stretch the phrase into the key.
pub const PHRASE_ROUNDS: u32 = 600_000;
/// Fixed salt for the stretch, so the same phrase gives the same codes in
/// every file and every run.
const PHRASE_SALT: &[u8] = b"Natendo Data Edit scramble key v1";

/// The last stretched key, kept while a screen previews the same phrase
/// again and again; overwritten by [`forget_keys`] (when the cleaning or
/// share-safe screen closes) or after [`KEY_CACHE_TIME`].
struct CachedKey {
    check: [u8; 32],
    key: [u8; 32],
    at: std::time::Instant,
}

impl CachedKey {
    fn wipe(&mut self) {
        wipe(&mut self.check);
        wipe(&mut self.key);
    }
}

static KEY_CACHE: std::sync::Mutex<Option<CachedKey>> = std::sync::Mutex::new(None);
const KEY_CACHE_TIME: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Overwrite bytes that held a secret.
pub(crate) fn wipe(b: &mut [u8]) {
    b.iter_mut().for_each(|x| *x = 0);
    std::hint::black_box(&b);
}

/// Forget the kept scramble key (its bytes are overwritten).
pub fn forget_keys() {
    if let Some(mut c) = KEY_CACHE.lock().unwrap_or_else(|e| e.into_inner()).take() {
        c.wipe();
    }
}

/// HMAC-SHA-256 (RFC 2104) with the key's inner and outer states made once,
/// so each cell costs two short hashes.
#[derive(Clone)]
pub(crate) struct Keyed {
    inner: Sha256,
    outer: Sha256,
}

impl Keyed {
    pub(crate) fn new(key: &[u8]) -> Keyed {
        let mut block = [0u8; 64];
        if key.len() > 64 {
            block[..32].copy_from_slice(&Sha256::digest(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let pad = |x: u8| -> Sha256 {
            let mut h = Sha256::new();
            let b: Vec<u8> = block.iter().map(|b| b ^ x).collect();
            h.update(&b);
            h
        };
        Keyed {
            inner: pad(0x36),
            outer: pad(0x5c),
        }
    }

    /// The key for a phrase: the phrase stretched with PBKDF2-HMAC-SHA-256
    /// ([`PHRASE_ROUNDS`] rounds, fixed salt), so each guess at the phrase
    /// costs as much as 600,000 scrambles. The last phrase's key is kept
    /// for the next run (previews run again and again).
    pub(crate) fn from_secret(s: &Secret) -> Keyed {
        let mut salted = [PHRASE_SALT, s.0.as_bytes()].concat();
        let check: [u8; 32] = Sha256::digest(&salted).into();
        wipe(&mut salted);
        let mut last = KEY_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = last.as_ref() {
            if c.check == check && c.at.elapsed() < KEY_CACHE_TIME {
                return Keyed::new(&c.key);
            }
        }
        if let Some(mut old) = last.take() {
            old.wipe();
        }
        // Unit tests (unoptimised builds) use fewer rounds; the stretch
        // itself is checked against the standard's vectors.
        let rounds = if cfg!(test) { 1_000 } else { PHRASE_ROUNDS };
        let key = pbkdf2(s.0.as_bytes(), PHRASE_SALT, rounds);
        let k = Keyed::new(&key);
        *last = Some(CachedKey {
            check,
            key,
            at: std::time::Instant::now(),
        });
        k
    }

    pub(crate) fn mac(&self, msg: &[u8]) -> [u8; 32] {
        let mut i = self.inner.clone();
        i.update(msg);
        let mut o = self.outer.clone();
        o.update(i.finalize());
        o.finalize().into()
    }

    /// The same value always gives the same 16 hexadecimal characters for
    /// one phrase; without the phrase they cannot be traced back.
    pub(crate) fn scramble(&self, v: &str) -> String {
        let m = self.mac(v.as_bytes());
        let mut out = String::with_capacity(16);
        for b in &m[..8] {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}

/// PBKDF2 (RFC 8018) with HMAC-SHA-256, one 32-byte block.
pub(crate) fn pbkdf2(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let prf = Keyed::new(password);
    let mut u = prf.mac(&[salt, &1u32.to_be_bytes()].concat());
    let mut t = u;
    for _ in 1..rounds {
        u = prf.mac(&u);
        for (a, b) in t.iter_mut().zip(u) {
            *a ^= b;
        }
    }
    t
}

/// A kind of private data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateKind {
    /// US Social Security numbers.
    Ssn,
    BirthDate,
    Phone,
    Email,
    Address,
    /// Driver's licence and ID card numbers.
    Licence,
}

/// A column that looks like it holds private data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub col: u32,
    pub kind: PrivateKind,
    /// The column's title (or "Column N").
    pub title: String,
    /// Non-blank values looked at, and how many looked like this kind.
    pub looked: u64,
    pub hits: u64,
    /// Found from the column's title (as well as or instead of its values).
    pub by_title: bool,
}

/// Rows the finder reads (the first shown ones).
pub const FINDER_ROWS: u64 = 20_000;
/// Columns the finder looks at.
pub const FINDER_COLS: u32 = 500;

fn squash(t: &str) -> String {
    t.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The kind a column's title names, if any.
fn kind_of_title(title: &str) -> Option<PrivateKind> {
    let t = squash(title);
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    if t == "ssn"
        || t == "ss"
        || t == "socsec"
        || t == "ssno"
        || title.contains("SS#")
        || title.to_ascii_lowercase().contains("ss #")
        || has(&[
            "socialsecurity",
            "ssnumber",
            "ssn4",
            "lastfourssn",
            "ssnlast",
        ])
        || t.starts_with("ssn")
    {
        Some(PrivateKind::Ssn)
    } else if t == "dob" || has(&["birth", "dateofbirth", "birthdate", "birthday"]) {
        Some(PrivateKind::BirthDate)
    } else if has(&["email", "emailaddress"]) || t == "mail" {
        Some(PrivateKind::Email)
    } else if has(&[
        "phone",
        "telephone",
        "mobile",
        "cellnumber",
        "cellphone",
        "cellno",
    ]) || t == "tel"
        || t == "cell"
        || t == "fax"
        || t.starts_with("telno")
        || t.starts_with("tel") && t[3..].chars().all(|c| c.is_ascii_digit())
        || t.starts_with("cell") && t[4..].chars().all(|c| c.is_ascii_digit())
    {
        Some(PrivateKind::Phone)
    } else if has(&[
        "licen",
        "driversid",
        "dlnumber",
        "stateid",
        "idcard",
        "idnumber",
        "passport",
        "driverslic",
    ]) || t == "dl"
        || t == "dln"
        || t == "dlno"
        || t == "dlnum"
    {
        Some(PrivateKind::Licence)
    } else if has(&[
        "address",
        "street",
        "residence",
        "mailingaddr",
        "resaddr",
        "homeaddr",
    ]) || t == "addr"
    {
        Some(PrivateKind::Address)
    } else {
        None
    }
}

const STREET_WORDS: [&str; 24] = [
    "st",
    "street",
    "ave",
    "avenue",
    "rd",
    "road",
    "dr",
    "drive",
    "ln",
    "lane",
    "blvd",
    "boulevard",
    "ct",
    "court",
    "way",
    "pl",
    "place",
    "hwy",
    "highway",
    "pkwy",
    "cir",
    "circle",
    "ter",
    "trl",
];

/// The kind one value looks like, if any (cheap tests, most specific first).
fn kind_of_value(v: &str, read: ReadOptions, today: f64) -> Option<PrivateKind> {
    let t = v.trim();
    let b = t.as_bytes();
    if b.len() == 11
        && b[3] == b'-'
        && b[6] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 3 || i == 6 || c.is_ascii_digit())
    {
        return Some(PrivateKind::Ssn);
    }
    // 123456789, 123 45 6789
    let ssn_spaced = b.len() == 11
        && b[3] == b' '
        && b[6] == b' '
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 3 || i == 6 || c.is_ascii_digit());
    if ssn_spaced || (b.len() == 9 && b.iter().all(u8::is_ascii_digit) && b[0] != b'0') {
        return Some(PrivateKind::Ssn);
    }
    if let Some((user, host)) = t.split_once('@') {
        if !user.is_empty()
            && host.contains('.')
            && !host.starts_with('.')
            && !host.ends_with('.')
            && !t.contains(char::is_whitespace)
            && !host.contains('@')
        {
            return Some(PrivateKind::Email);
        }
    }
    let digits = t.chars().filter(char::is_ascii_digit).count();
    let phone_chars = t
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '(' | ')' | '-' | '.' | ' ' | '+'));
    // Ten plain digits count only when they cannot be a date or a year
    // run; the area code never starts with 0 or 1.
    let plain_ok = t.chars().any(|c| !c.is_ascii_digit()) || matches!(b.first(), Some(b'2'..=b'9'));
    if phone_chars
        && (digits == 10 || (digits == 11 && t.trim_start_matches('+').starts_with('1')))
        && plain_ok
    {
        return Some(PrivateKind::Phone);
    }
    // "12 Main St", "4410 N Oak Avenue Apt 2"
    let mut words = t.split_whitespace();
    if words
        .next()
        .is_some_and(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        && t.split_whitespace().skip(1).any(|w| {
            let w = w.trim_end_matches(['.', ',']).to_ascii_lowercase();
            STREET_WORDS.contains(&w.as_str())
        })
    {
        return Some(PrivateKind::Address);
    }
    // Licence and ID numbers: one or two letters then 6 to 12 digits
    // (A1234567, DL12345678).
    let letters = b.iter().take_while(|c| c.is_ascii_alphabetic()).count();
    if (1..=2).contains(&letters)
        && (6..=12).contains(&(b.len() - letters))
        && b[letters..].iter().all(u8::is_ascii_digit)
    {
        return Some(PrivateKind::Licence);
    }
    if t.len() >= 6 && t.len() <= 40 {
        if let Some(d) = crate::coltype::Reader::plain(read).date(t) {
            let age = (today - d) / 365.25;
            if d.floor() != 0.0 && (0.0..=120.0).contains(&age) {
                return Some(PrivateKind::BirthDate);
            }
        }
    }
    None
}

impl Document {
    /// Columns that look like they hold private data: by their title, or by
    /// most of the first [`FINDER_ROWS`] shown values. `today` is days since
    /// 1970-01-01 (birth dates are dates in the last 120 years, and need a
    /// title that says so unless nothing else fits).
    pub fn find_private(
        &self,
        header: bool,
        read: ReadOptions,
        today: f64,
        progress: &Progress,
    ) -> Result<Vec<Finding>, Error> {
        self.ready_for_bulk()?;
        let ncols = self.col_count().min(FINDER_COLS);
        let cols: Vec<u32> = (0..ncols).collect();
        let titles = self.titles(&cols, header);
        let rows = self.data_rows(header);
        let end = rows.end.min(rows.start + FINDER_ROWS);
        let reader = Cols::new(self, &cols);
        // Per column: non-blank count and hits per kind.
        let mut looked = vec![0u64; ncols as usize];
        let mut hits = vec![[0u64; 6]; ncols as usize];
        let mut buf = Vec::new();
        progress.start(end - rows.start);
        for r in rows.start..end {
            let Some(id) = self.id_at(r) else { continue };
            reader.read(self, id, &mut buf);
            for c in 0..ncols as usize {
                let v = reader.get(&buf, c);
                if v.trim().is_empty() || v.len() > 200 {
                    continue;
                }
                looked[c] += 1;
                if let Some(k) = kind_of_value(v, read, today) {
                    hits[c][k as usize] += 1;
                }
            }
            if (r - rows.start) % 4096 == 4095 {
                progress.step(4096)?;
            }
        }
        const KINDS: [PrivateKind; 6] = [
            PrivateKind::Ssn,
            PrivateKind::BirthDate,
            PrivateKind::Phone,
            PrivateKind::Email,
            PrivateKind::Address,
            PrivateKind::Licence,
        ];
        let mut out = Vec::new();
        for c in 0..ncols as usize {
            let by_title = if header {
                kind_of_title(&titles[c])
            } else {
                None
            };
            let n = looked[c];
            let best = (0..6)
                .max_by_key(|&k| hits[c][k])
                .filter(|&k| hits[c][k] > 0);
            // Most values (60%) look alike; a birth date needs the title too,
            // since plain date columns are common.
            let by_values = best
                .filter(|&k| hits[c][k] * 10 >= n * 6)
                .map(|k| KINDS[k])
                .filter(|&k| k != PrivateKind::BirthDate || by_title == Some(k));
            let kind = match (by_title, by_values) {
                (Some(t), _) => Some(t),
                (None, v) => v,
            };
            if let Some(kind) = kind {
                out.push(Finding {
                    col: c as u32,
                    kind,
                    title: titles[c].clone(),
                    looked: n,
                    hits: hits[c][kind as usize],
                    by_title: by_title.is_some(),
                });
            }
        }
        Ok(out)
    }
}

/// What a share-safe copy does with one column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ColumnAction {
    Keep,
    Drop,
    /// Run these cleaning steps on its values (mask, year only, scramble…).
    Clean {
        cleaner: Cleaner,
    },
}

/// A share-safe copy: every shown row, each column kept, dropped or
/// cleaned; the column titles (with `header`) are kept as they are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafeCopy {
    /// One action per column, in order; columns past the end are left out.
    pub columns: Vec<ColumnAction>,
    pub header: bool,
    #[serde(default)]
    pub read: ReadOptions,
}

/// What a share-safe copy wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeCopyReport {
    pub rows: u64,
    pub cols: u32,
    pub changed: u64,
    pub bytes: u64,
    /// Values a step could not change (a birth date it could not read, say)
    /// and that were therefore written blank, never as they were.
    pub blanked: u64,
    /// The same per column: (column, values blanked).
    pub blanked_cols: Vec<(u32, u64)>,
}

impl Document {
    /// Write a share-safe copy to `out` in this file's separator (UTF-8,
    /// CRLF). `now` is the run's moment (days since 1970-01-01).
    pub fn write_safe_copy(
        &self,
        out: &mut impl Write,
        spec: &SafeCopy,
        now: f64,
        progress: &Progress,
    ) -> Result<SafeCopyReport, Error> {
        self.ready_for_bulk()?;
        let n = self.col_count();
        let action = |c: u32| spec.columns.get(c as usize).unwrap_or(&ColumnAction::Drop);
        let mut runs: Vec<Option<Compiled>> = Vec::with_capacity(n as usize);
        for c in 0..n {
            runs.push(match action(c) {
                ColumnAction::Clean { cleaner } => Some(Compiled::new(cleaner, spec.read, now)?),
                _ => None,
            });
        }
        let kept: Vec<u32> = (0..n)
            .filter(|&c| *action(c) != ColumnAction::Drop)
            .collect();
        if kept.is_empty() {
            return Err(Error::Invalid("keep at least one column".into()));
        }
        let reader = Cols::new(self, &kept);
        let d = self.format().dialect;
        let total = self.row_count();
        let first = u64::from(spec.header && total > 0);
        let mut report = SafeCopyReport {
            cols: kept.len() as u32,
            ..Default::default()
        };
        let seed = (now.to_bits() ^ 0x5eed_c0de).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        // One row's text, how many of its values changed, and the kept
        // columns (by position) whose value had to be blanked.
        type Line = (String, u64, Vec<u32>);
        let line_of = |r: u64| -> Result<Option<Line>, Error> {
            let Some(id) = self.id_at(r) else {
                return Ok(None);
            };
            let mut buf = Vec::new();
            reader.read(self, id, &mut buf);
            let mut line = String::new();
            let mut changed = 0;
            let mut blank = Vec::new();
            for (k, &c) in kept.iter().enumerate() {
                if k > 0 {
                    line.push_str(d.delimiter_str());
                }
                let v = reader.get(&buf, k);
                let run = runs[c as usize].as_ref().filter(|_| r >= first);
                let cleaned;
                let v = match run {
                    Some(run) => {
                        let ctx = Ctx {
                            index: r - first,
                            seed: seed ^ r.wrapping_mul(0xD1B5_4A32_D192_ED03) ^ u64::from(c) << 40,
                            run: seed ^ u64::from(c) << 40,
                        };
                        let o = run.run(v, ctx)?;
                        match o.text {
                            // A value no step could read is never written
                            // as it was.
                            _ if o.left => {
                                blank.push(k as u32);
                                ""
                            }
                            Some(t) => {
                                changed += 1;
                                cleaned = t;
                                cleaned.as_str()
                            }
                            None => v,
                        }
                    }
                    None => v,
                };
                crate::scan::write_field(v, d, &mut line);
            }
            line.push_str("\r\n");
            Ok(Some((line, changed, blank)))
        };
        let mut blanked = vec![0u64; kept.len()];
        progress.start(total);
        // Rows are made in parallel a chunk at a time, written in order.
        const CHUNK: u64 = 1 << 14;
        let mut start = 0;
        while start < total {
            let end = (start + CHUNK).min(total);
            let lines: Vec<Option<Line>> = (start..end)
                .into_par_iter()
                .map(line_of)
                .collect::<Result<_, _>>()?;
            for (line, changed, blank) in lines.into_iter().flatten() {
                for k in blank {
                    blanked[k as usize] += 1;
                }
                out.write_all(line.as_bytes())?;
                report.bytes += line.len() as u64;
                report.rows += 1;
                report.changed += changed;
            }
            progress.step(end - start)?;
            start = end;
        }
        report.rows -= first.min(report.rows);
        report.blanked = blanked.iter().sum();
        report.blanked_cols = kept
            .iter()
            .zip(&blanked)
            .filter(|(_, &n)| n > 0)
            .map(|(&c, &n)| (c, n))
            .collect();
        Ok(report)
    }

    /// [`Document::write_safe_copy`] to a file: written beside it, then
    /// moved into place (never over this document's own file).
    pub fn save_safe_copy(
        &self,
        path: &Path,
        spec: &SafeCopy,
        now: f64,
        progress: &Progress,
    ) -> Result<SafeCopyReport, Error> {
        if self.path().is_some_and(|p| crate::save::same_path(p, path)) {
            return Err(Error::Invalid(
                "a share-safe copy cannot replace the file itself; choose another name".into(),
            ));
        }
        let (temp, file) = crate::save::create_temp(path)?;
        let done = (|| -> Result<SafeCopyReport, Error> {
            let mut w = BufWriter::with_capacity(1 << 20, &file);
            let r = self.write_safe_copy(&mut w, spec, now, progress)?;
            w.flush()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::{DateTo, Step};
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn cleaner(steps: Vec<Step>) -> Cleaner {
        Cleaner {
            version: 1,
            name: String::new(),
            steps,
        }
    }

    fn one(step: Step, cell: &str, now: f64) -> String {
        Compiled::new(&cleaner(vec![step]), ReadOptions::default(), now)
            .unwrap()
            .run(cell, Ctx::default())
            .unwrap()
            .text
            .unwrap_or_else(|| cell.to_string())
    }

    /// RFC 4231 test cases 1, 2 and 6 (a key longer than a block).
    #[test]
    fn hmac_matches_the_standard() {
        let k = Keyed::new(&[0x0b; 20]);
        assert_eq!(
            hex(&k.mac(b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        let k = Keyed::new(b"Jefe");
        assert_eq!(
            hex(&k.mac(b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let k = Keyed::new(&[0xaa; 131]);
        assert_eq!(
            hex(&k.mac(b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn masking_steps() {
        let mask = |keep_last, all| Step::Mask {
            keep_last,
            with: '*',
            all,
        };
        assert_eq!(one(mask(4, false), "123-45-6789", 0.0), "***-**-6789");
        assert_eq!(one(mask(0, false), "Ann Lee", 0.0), "*** ***");
        assert_eq!(one(mask(2, true), "a-b c", 0.0), "*** c");
        assert_eq!(one(mask(10, false), "ab", 0.0), "**");
        // 2026-11-13 is day 20_770.
        let now = 20_770.0;
        let band = |to| Step::DateBand { to };
        assert_eq!(one(band(DateTo::Year), "03/14/1980", now), "1980");
        assert_eq!(one(band(DateTo::YearMonth), "1980-03-14", now), "1980-03");
        let ages = DateTo::AgeBand { size: 10, top: 90 };
        assert_eq!(one(band(ages), "1980-11-13", now), "40-49");
        assert_eq!(one(band(ages), "1986-11-14", now), "30-39");
        assert_eq!(one(band(ages), "1930-01-01", now), "90+");
        assert_eq!(
            one(band(DateTo::AgeBand { size: 1, top: 0 }), "2000-11-13", now),
            "26"
        );
        assert_eq!(
            one(
                band(DateTo::AgeBand { size: 10, top: 85 }),
                "1945-01-01",
                now
            ),
            "80-84"
        );
        let c = Compiled::new(&cleaner(vec![band(ages)]), ReadOptions::default(), now).unwrap();
        assert!(c.run("not a date", Ctx::default()).unwrap().left);
        assert!(c.run("2030-01-01", Ctx::default()).unwrap().left);
    }

    #[test]
    fn scramble_is_keyed_and_never_saved() {
        let s = |p: &str| Step::Scramble {
            phrase: Secret::new(p),
            loose: false,
        };
        let a = one(s("correct horse battery"), "123-45-6789", 0.0);
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, one(s("correct horse battery"), "123-45-6789", 0.0));
        assert_ne!(a, one(s("correct horse batterz"), "123-45-6789", 0.0));
        assert_ne!(a, one(s("correct horse battery"), "123-45-6788", 0.0));
        assert_eq!(one(s("correct horse battery"), "", 0.0), "");
        // Too short, or missing (a cleaner loaded from a file).
        assert!(Compiled::new(&cleaner(vec![s("short")]), ReadOptions::default(), 0.0).is_err());
        let c = cleaner(vec![s("correct horse battery")]);
        let json = String::from_utf8(c.to_json().unwrap()).unwrap();
        assert!(!json.contains("correct"), "{json}");
        assert!(!format!("{c:?}").contains("correct"));
        let back = Cleaner::from_json(json.as_bytes()).unwrap();
        assert!(Compiled::new(&back, ReadOptions::default(), 0.0).is_err());
    }

    #[test]
    fn finder_marks_private_columns() {
        let d = doc(concat!(
            "Voter ID,Name,SSN,Birth date,Phone,E-mail,Street,Notes,DL number,Joined\r\n",
            "1,Ann Lee,123-45-6789,03/14/1980,(423) 555-0101,ann@x.org,12 Main St,ok,A123,2020-01-02\r\n",
            "2,Bob Ray,987-65-4321,1975-07-01,423-555-0102,bob@y.com,4410 N Oak Avenue,,B456,2021-05-06\r\n",
            "3,Cy Fox,,,423.555.0103,cy@z.net,9 Elm Rd.,hi,C789,2022-09-10\r\n",
        ));
        let f = d
            .find_private(true, ReadOptions::default(), 20_770.0, &Progress::new())
            .unwrap();
        let got: Vec<(u32, PrivateKind)> = f.iter().map(|x| (x.col, x.kind)).collect();
        assert_eq!(
            got,
            vec![
                (2, PrivateKind::Ssn),
                (3, PrivateKind::BirthDate),
                (4, PrivateKind::Phone),
                (5, PrivateKind::Email),
                (6, PrivateKind::Address),
                (8, PrivateKind::Licence),
            ]
        );
        assert_eq!((f[0].looked, f[0].hits), (2, 2));
        // Without titles, values alone (a plain date column is not a birth date).
        let f = d
            .find_private(false, ReadOptions::default(), 20_770.0, &Progress::new())
            .unwrap();
        let kinds: Vec<PrivateKind> = f.iter().map(|x| x.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PrivateKind::Ssn,
                PrivateKind::Phone,
                PrivateKind::Email,
                PrivateKind::Address
            ]
        );
    }

    #[test]
    fn share_safe_copy() {
        let mut d = doc(
            "name,ssn,born,city\r\nAnn Lee,123-45-6789,1980-11-13,\"Oslo, NO\"\r\nBob,,1975-07-01,Rome\r\nCy,1,2,3\r\n",
        );
        d.set_filter(&crate::filter::FilterSpec {
            columns: vec![crate::filter::ColumnFilter {
                col: 0,
                rule: crate::filter::Rule::Values {
                    values: vec!["Ann Lee".into(), "Bob".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        let spec = SafeCopy {
            columns: vec![
                ColumnAction::Drop,
                ColumnAction::Clean {
                    cleaner: cleaner(vec![Step::Mask {
                        keep_last: 4,
                        with: '#',
                        all: false,
                    }]),
                },
                ColumnAction::Clean {
                    cleaner: cleaner(vec![Step::DateBand { to: DateTo::Year }]),
                },
                ColumnAction::Keep,
            ],
            header: true,
            read: ReadOptions::default(),
        };
        let mut out = Vec::new();
        let rep = d
            .write_safe_copy(&mut out, &spec, 20_770.0, &Progress::new())
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "ssn,born,city\r\n###-##-6789,1980,\"Oslo, NO\"\r\n,1975,Rome\r\n"
        );
        assert_eq!((rep.rows, rep.cols, rep.changed), (2, 3, 3));
        // The open file is unchanged and cannot be replaced by the copy.
        assert_eq!(d.row_values(1).unwrap()[1], "123-45-6789");
        let dir = std::env::temp_dir().join(format!("nde-safe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("list.csv");
        std::fs::write(&src, "a,b\r\n1,2\r\n").unwrap();
        let opened = Document::open_indexed(&src, OpenOptions::default(), None).unwrap();
        let keep = SafeCopy {
            columns: vec![ColumnAction::Keep, ColumnAction::Keep],
            header: true,
            read: ReadOptions::default(),
        };
        assert!(opened
            .save_safe_copy(&src, &keep, 0.0, &Progress::new())
            .is_err());
        let copy = dir.join("copy.csv");
        opened
            .save_safe_copy(&copy, &keep, 0.0, &Progress::new())
            .unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "a,b\r\n1,2\r\n");
        let all_dropped = SafeCopy {
            columns: vec![ColumnAction::Drop, ColumnAction::Drop],
            header: true,
            read: ReadOptions::default(),
        };
        assert!(opened
            .write_safe_copy(&mut Vec::new(), &all_dropped, 0.0, &Progress::new())
            .is_err());
        drop(opened);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pbkdf2_matches_the_standard() {
        // RFC 7914 section 11 / common PBKDF2-HMAC-SHA-256 test vectors.
        assert_eq!(
            hex(&pbkdf2(b"password", b"salt", 1)),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            hex(&pbkdf2(b"password", b"salt", 2)),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
        assert_eq!(
            hex(&pbkdf2(b"password", b"salt", 4096)),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
    }

    #[test]
    fn review_m13_scramble_and_mask() {
        let phrase = "a long enough phrase";
        let s = |loose| Step::Scramble {
            phrase: Secret::new(phrase),
            loose,
        };
        // M2: trimmed always; loose ignores spaces, punctuation and case.
        let a = one(s(false), "123-45-6789", 0.0);
        assert_eq!(a, one(s(false), "  123-45-6789 ", 0.0));
        assert_ne!(a, one(s(false), "123456789", 0.0));
        let l = one(s(true), "123-45-6789", 0.0);
        for v in ["123456789", "123 45 6789", " 123-45-6789"] {
            assert_eq!(l, one(s(true), v, 0.0), "{v}");
        }
        assert_eq!(
            one(s(true), "Ann@X.org", 0.0),
            one(s(true), "ann@x.org", 0.0)
        );
        // M1: the minimum is 16 characters.
        let short = Step::Scramble {
            phrase: Secret::new("fifteen chars!!"),
            loose: false,
        };
        assert!(Compiled::new(&cleaner(vec![short]), ReadOptions::default(), 0.0).is_err());
        // M3: short values are hidden whole; never more than half shown.
        let m = |v: &str| {
            one(
                Step::Mask {
                    keep_last: 4,
                    with: '*',
                    all: false,
                },
                v,
                0.0,
            )
        };
        assert_eq!(m("6789"), "****");
        assert_eq!(m("12"), "**");
        assert_eq!(m("José Ñ"), "***é Ñ");
        assert_eq!(m("123456"), "***456");
        assert_eq!(m("123-45-6789"), "***-**-6789");
    }

    #[test]
    fn review_m13_unreadable_values_are_blanked() {
        let d = doc(
            "name,born,extra\r\nA,19800105,x\r\nB,2030-01-01,x\r\nC,13/31/1980,x\r\nD,N/A,x\r\nE,1980-01-05,x\r\nF,1980,x\r\nG,13/05/1980,x\r\nH,,x,EXTRA-SECRET\r\n",
        );
        let band = || ColumnAction::Clean {
            cleaner: cleaner(vec![Step::DateBand {
                to: DateTo::AgeBand { size: 10, top: 90 },
            }]),
        };
        // 2026-11-13
        let now = 20_770.0;
        let spec = SafeCopy {
            columns: vec![ColumnAction::Keep, band()],
            header: true,
            read: ReadOptions::default(),
        };
        let mut out = Vec::new();
        let rep = d
            .write_safe_copy(&mut out, &spec, now, &Progress::new())
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text,
            "name,born\r\nA,40-49\r\nB,\r\nC,\r\nD,\r\nE,40-49\r\nF,40-49\r\nG,40-49\r\nH,\r\n"
        );
        assert!(!text.contains("EXTRA") && !text.contains("1980"));
        assert_eq!(rep.blanked, 3);
        assert_eq!(rep.blanked_cols, vec![(1, 3)]);
        // Day first: 13/05/1980 is still read.
        let dmy = SafeCopy {
            read: ReadOptions {
                date_order: crate::value::DateOrder::Dmy,
                ..ReadOptions::default()
            },
            ..spec
        };
        let mut out = Vec::new();
        let rep = d
            .write_safe_copy(&mut out, &dmy, now, &Progress::new())
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("G,40-49") && !text.contains("1980"), "{text}");
        assert!(rep.blanked >= 3);
    }

    #[test]
    fn review_m13_finder_plain_formats() {
        let today = 20_770.0;
        let k = |v: &str| kind_of_value(v, ReadOptions::default(), today);
        assert_eq!(k("123456789"), Some(PrivateKind::Ssn));
        assert_eq!(k("123 45 6789"), Some(PrivateKind::Ssn));
        assert_eq!(k("4235551234"), Some(PrivateKind::Phone));
        assert_eq!(k("A1234567"), Some(PrivateKind::Licence));
        assert_eq!(k("DL12345678"), Some(PrivateKind::Licence));
        assert_eq!(k("1234567890"), None);
        for (t, want) in [
            ("SS#", PrivateKind::Ssn),
            ("SS #", PrivateKind::Ssn),
            ("Tel No", PrivateKind::Phone),
            ("Cell #", PrivateKind::Phone),
            ("Phone1", PrivateKind::Phone),
            ("Tel2", PrivateKind::Phone),
            ("DL No", PrivateKind::Licence),
        ] {
            assert_eq!(kind_of_title(t), Some(want), "{t}");
        }
        assert_eq!(kind_of_title("Telluride"), None);
        assert_eq!(kind_of_title("Cellar"), None);
    }
}
