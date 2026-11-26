//! A read-only reader for dBase (`.dbf`) tables, as GIS programs write them
//! beside a shapefile. Reads dBase III, IV and 7 and Visual FoxPro tables,
//! with memo text from a `.dbt` file. Numbers come through as the text the
//! file holds. Nothing here ever writes a `.dbf`.
//!
//! The file is untrusted: every offset and length is checked. Warnings name
//! fields and counts only, never cell values.

use std::fmt::Write;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::Progress;
use crate::reshape::{push_line, too_big};
use crate::table::{Sink, MAX_CELL};
use crate::Error;

/// Most fields a table may have.
pub const MAX_FIELDS: usize = 2_046;
/// Longest single memo read.
const MAX_MEMO: usize = 1 << 20;
/// Memo bytes looked through in one read (a damaged memo file cannot make
/// every record scan to its end).
const MAX_MEMO_SCAN: u64 = 4 << 30;
/// Largest `.dbt` memo file read.
const MAX_DBT: u64 = 1 << 30;
/// Largest `.cpg` or `.prj` file read.
const MAX_SMALL: u64 = 64 << 10;
/// Records one parallel task converts.
const CHUNK: usize = 2_048;
/// Tasks between progress steps (and chances to cancel).
const BATCH: usize = 64;

/// The record number column.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fid {
    Off,
    /// Counted from 0, as QGIS and GDAL number shapes.
    #[default]
    From0,
    From1,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DbfOptions {
    /// An encoding label; empty: from the `.cpg` file, the table's own code,
    /// or a guess.
    pub encoding: String,
    pub fid: Fid,
    /// Include records marked deleted, with a "Deleted" column.
    pub show_deleted: bool,
}

/// The files beside a `.dbf`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Sidecars {
    pub cpg: Option<Vec<u8>>,
    pub dbt: Option<Vec<u8>>,
    pub prj: Option<Vec<u8>>,
    /// Why a file beside the table was not read.
    pub problems: Vec<String>,
}

impl Sidecars {
    /// Read the `.cpg`, `.dbt` and `.prj` files with the table's name.
    pub fn beside(table: &Path) -> Sidecars {
        let mut s = Sidecars::default();
        let mut read = |ext: &str, cap: u64| -> Option<Vec<u8>> {
            let p = table.with_extension(ext);
            let f = std::fs::File::open(&p).ok()?;
            let mut v = Vec::new();
            match f.take(cap + 1).read_to_end(&mut v) {
                Ok(_) if v.len() as u64 <= cap => Some(v),
                Ok(_) => {
                    s.problems
                        .push(format!("the .{ext} file is too large and was not read"));
                    None
                }
                Err(e) => {
                    s.problems
                        .push(format!("the .{ext} file could not be read: {e}"));
                    None
                }
            }
        };
        let (cpg, dbt, prj) = (
            read("cpg", MAX_SMALL),
            read("dbt", MAX_DBT),
            read("prj", MAX_SMALL),
        );
        s.cpg = cpg;
        s.dbt = dbt;
        s.prj = prj;
        s
    }
}

/// The `.dbf` holding the attributes of a shapefile part (`.shp`, `.shx`,
/// `.prj`, `.cpg`, `.sbn`, `.sbx`), when there is one.
pub fn shapefile_table(path: &Path) -> Option<PathBuf> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "shp" | "shx" | "prj" | "cpg" | "sbn" | "sbx") {
        return None;
    }
    let dbf = path.with_extension("dbf");
    dbf.is_file().then_some(dbf)
}

/// What a table holds, for the import screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbfInfo {
    /// Records that can be read, deleted ones included.
    pub records: u64,
    /// Records among them marked as deleted.
    pub deleted: u64,
    pub fields: u32,
    /// The encoding text is read with, and which rule chose it.
    pub encoding: String,
    pub rule: String,
    /// The first line of the `.prj` file (the coordinate system).
    pub projection: String,
    pub warnings: Vec<String>,
}

// ---- code pages ----

const CP437: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";
const CP850: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜø£Ø×ƒáíóúñÑªº¿®¬½¼¡«»░▒▓│┤ÁÂÀ©╣║╗╝¢¥┐└┴┬├─┼ãÃ╚╔╩╦╠═╬¤ðÐÊËÈıÍÎÏ┘┌█▄¦Ì▀ÓßÔÒõÕµþÞÚÛÙýÝ¯´\u{ad}±‗¾¶§÷¸°¨·¹³²■\u{a0}";

#[derive(Clone)]
enum Decoder {
    Rs(&'static encoding_rs::Encoding),
    /// A DOS code page: the characters of bytes 0x80 to 0xFF.
    Page(&'static str, Box<[char; 128]>),
}

impl Decoder {
    fn page(name: &'static str, high: &str) -> Decoder {
        let mut t = Box::new(['\u{fffd}'; 128]);
        for (slot, c) in t.iter_mut().zip(high.chars()) {
            *slot = c;
        }
        Decoder::Page(name, t)
    }

    fn name(&self) -> String {
        match self {
            Decoder::Rs(e) => e.name().to_string(),
            Decoder::Page(n, _) => n.to_string(),
        }
    }

    /// Append the text of `b` to `out`; false when bytes had to be replaced.
    fn push(&self, b: &[u8], out: &mut String) -> bool {
        if b.is_ascii() {
            // Every encoding used here keeps ASCII as it is.
            out.push_str(std::str::from_utf8(b).unwrap_or_default());
            return true;
        }
        match self {
            Decoder::Rs(e) => {
                let (s, lossy) = e.decode_without_bom_handling(b);
                out.push_str(&s);
                !lossy
            }
            Decoder::Page(_, t) => {
                out.extend(b.iter().map(|&c| {
                    if c < 0x80 {
                        c as char
                    } else {
                        t[(c - 0x80) as usize]
                    }
                }));
                true
            }
        }
    }
}

/// An encoding by name, for `.cpg` files and the picker. Only encodings that
/// keep ASCII as it is can be used for a `.dbf`.
fn by_label(label: &str) -> Option<Decoder> {
    let l = label.trim().trim_start_matches('\u{feff}').trim();
    let up = l.to_ascii_uppercase();
    let up = up.strip_prefix("ANSI ").unwrap_or(&up);
    let own = match up {
        "65001" => Some("utf-8".to_string()),
        "88591" => Some("iso-8859-1".to_string()),
        "437" | "CP437" | "IBM437" | "OEM 437" => return Some(Decoder::page("CP437", CP437)),
        "850" | "CP850" | "IBM850" | "OEM 850" => return Some(Decoder::page("CP850", CP850)),
        "932" => Some("shift_jis".to_string()),
        "936" => Some("gbk".to_string()),
        "949" => Some("euc-kr".to_string()),
        "950" => Some("big5".to_string()),
        n if n.len() == 4 && n.starts_with("125") => Some(format!("windows-{n}")),
        "874" => Some("windows-874".to_string()),
        "866" => Some("ibm866".to_string()),
        n => n
            .strip_prefix("8859")
            .map(|k| format!("iso-8859-{}", k.trim_start_matches('-'))),
    };
    let label = own.unwrap_or_else(|| l.to_ascii_lowercase());
    let e = encoding_rs::Encoding::for_label(label.as_bytes())?;
    let usable = e.is_ascii_compatible()
        && e != encoding_rs::REPLACEMENT
        && e != encoding_rs::X_USER_DEFINED;
    usable.then_some(Decoder::Rs(e))
}

/// What the table's language byte (offset 29) says.
enum Language {
    None,
    Known(Decoder),
    /// A code page this app cannot read.
    Unsupported(u16),
}

fn language(b: u8) -> Language {
    use encoding_rs as rs;
    let k = |e| Language::Known(Decoder::Rs(e));
    match b {
        0x01 | 0x09 | 0x0B | 0x0D | 0x0F | 0x11 | 0x15 | 0x18 | 0x19 | 0x1B => {
            Language::Known(Decoder::page("CP437", CP437))
        }
        0x02 | 0x0A | 0x0E | 0x10 | 0x12 | 0x14 | 0x16 | 0x1A | 0x1D | 0x25 | 0x37 => {
            Language::Known(Decoder::page("CP850", CP850))
        }
        0x03 | 0x57 | 0x58 | 0x59 => k(rs::WINDOWS_1252),
        0x04 => k(rs::MACINTOSH),
        0x13 | 0x7B => k(rs::SHIFT_JIS),
        0x26 | 0x65 => k(rs::IBM866),
        0x1F | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => Language::Unsupported(852),
        0x4D | 0x7A => k(rs::GBK),
        0x4E | 0x79 => k(rs::EUC_KR),
        0x4F | 0x78 => k(rs::BIG5),
        0x50 | 0x7C => k(rs::WINDOWS_874),
        0x7D => k(rs::WINDOWS_1255),
        0x7E => k(rs::WINDOWS_1256),
        0xC8 => k(rs::WINDOWS_1250),
        0xC9 => k(rs::WINDOWS_1251),
        0xCA => k(rs::WINDOWS_1254),
        0xCB => k(rs::WINDOWS_1253),
        0xCC => k(rs::WINDOWS_1257),
        _ => Language::None,
    }
}

// ---- header ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Dbase,
    Dbase7,
    FoxPro,
}

#[derive(Debug)]
struct Field {
    name: Vec<u8>,
    kind: u8,
    /// Offset in the record (the deletion mark is at 0).
    at: usize,
    len: usize,
    /// Visual FoxPro: bits of `_NullFlags` for "shorter than the field"
    /// (varchar) and "is null".
    short_bit: Option<usize>,
    null_bit: Option<usize>,
    hidden: bool,
}

struct Header {
    flavor: Flavor,
    version: u8,
    /// Where records start.
    start: usize,
    record_len: usize,
    records: u64,
    language: u8,
    fields: Vec<Field>,
    /// The hidden `_NullFlags` field.
    flags: Option<usize>,
    warnings: Vec<String>,
}

fn bad(m: &str) -> Error {
    Error::Invalid(format!("the dBase file: {m}"))
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

fn header(data: &[u8]) -> Result<Header, Error> {
    if data.len() < 32 {
        return Err(bad("too short to hold a table"));
    }
    let version = data[0];
    let flavor = match version {
        0x03 | 0x83 | 0x8B | 0xF5 => Flavor::Dbase,
        0x04 | 0x8C => Flavor::Dbase7,
        0x30..=0x32 => Flavor::FoxPro,
        v => {
            return Err(bad(&format!(
                "not a dBase file this app can read (version byte 0x{v:02X})"
            )))
        }
    };
    let claimed = u64::from(u32::from_le_bytes([data[4], data[5], data[6], data[7]]));
    let start = usize::from(u16_at(data, 8));
    let record_len = usize::from(u16_at(data, 10));
    let (first, size) = match flavor {
        Flavor::Dbase7 => (68, 48),
        _ => (32, 32),
    };
    if start <= first || start > data.len() {
        return Err(bad("its header length does not fit the file"));
    }
    if record_len < 2 {
        return Err(bad("its record length is zero"));
    }
    let mut warnings = Vec::new();
    let mut fields: Vec<Field> = Vec::new();
    let mut at = first;
    let mut offset = 1usize;
    let mut bits = 0usize;
    let mut flags = None;
    let mut ended = false;
    while at < start {
        if data[at] == 0x0D {
            ended = true;
            break;
        }
        if at + size > start {
            break;
        }
        if fields.len() >= MAX_FIELDS {
            return Err(bad(&format!("more than {MAX_FIELDS} fields")));
        }
        let d = &data[at..at + size];
        let name_len = if flavor == Flavor::Dbase7 { 32 } else { 11 };
        let name = &d[..name_len];
        let name = name[..name.iter().position(|&b| b == 0).unwrap_or(name_len)].to_vec();
        let (kind, low, high, bits_byte) = match flavor {
            Flavor::Dbase7 => (d[32], d[33], d[34], 0),
            _ => (d[11], d[16], d[17], d[18]),
        };
        // A long character field keeps the high byte of its length where a
        // number keeps its decimals.
        let len = if kind == b'C' {
            usize::from(low) | (usize::from(high) << 8)
        } else {
            usize::from(low)
        };
        let mut f = Field {
            name,
            kind,
            at: offset,
            len,
            short_bit: None,
            null_bit: None,
            hidden: false,
        };
        if flavor == Flavor::FoxPro {
            if kind == b'0' {
                f.hidden = true;
                flags = Some(fields.len());
            } else {
                if matches!(kind, b'V' | b'Q') {
                    f.short_bit = Some(bits);
                    bits += 1;
                }
                if bits_byte & 0x02 != 0 {
                    f.null_bit = Some(bits);
                    bits += 1;
                }
            }
        }
        offset += len;
        fields.push(f);
        at += size;
    }
    if fields.is_empty() {
        return Err(bad("it has no fields"));
    }
    // The terminator sits at the end of the header; Visual FoxPro keeps 263
    // more bytes after it, and dBase 7 may keep field properties there.
    let expected = match flavor {
        Flavor::Dbase => ended && (start - at == 1 || start - at == 2),
        Flavor::FoxPro => ended && (start - at == 264 || start - at == 1),
        Flavor::Dbase7 => ended,
    };
    if !expected {
        warnings
            .push("the field list and the header length disagree; check the columns".to_string());
    }
    if offset != record_len {
        warnings.push(format!(
            "the field widths add up to {} bytes but a record holds {}",
            offset - 1,
            record_len - 1
        ));
    }
    // Records that are really there.
    let body = (data.len() - start) as u64;
    let present = body / record_len as u64;
    let records = claimed.min(present);
    if claimed > present {
        warnings.push(format!(
            "the file is shorter than its header says: {present} of {claimed} records are there"
        ));
    } else {
        let used = records
            .checked_mul(record_len as u64)
            .ok_or_else(|| bad("its size does not add up"))?;
        // One byte (the end mark) may follow the last record.
        if body - used > 1 {
            warnings.push(format!(
                "the file holds more than the {claimed} records its header counts; the rest was not read"
            ));
        }
    }
    Ok(Header {
        flavor,
        version,
        start,
        record_len,
        records,
        language: data[29],
        fields,
        flags,
        warnings,
    })
}

/// The bytes of field `f` in record `rec`, if the record holds them.
fn slice<'a>(rec: &'a [u8], f: &Field) -> Option<&'a [u8]> {
    rec.get(f.at..f.at.checked_add(f.len)?)
}

/// True when every character field is UTF-8 (a character cut off at the end
/// of a field is allowed).
fn all_utf8(data: &[u8], h: &Header) -> bool {
    let ok = |b: &[u8]| match std::str::from_utf8(b) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    let text: Vec<&Field> = h
        .fields
        .iter()
        .filter(|f| matches!(f.kind, b'C' | b'V'))
        .collect();
    h.fields.iter().all(|f| ok(&f.name))
        && data[h.start..h.start + h.records as usize * h.record_len]
            .par_chunks(h.record_len * CHUNK)
            .all(|part| {
                part.is_ascii()
                    || part.chunks_exact(h.record_len).all(|rec| {
                        rec.is_ascii() || text.iter().all(|f| slice(rec, f).is_none_or(ok))
                    })
            })
}

/// The decoder, the rule that chose it, and what was passed over on the way.
fn choose(
    data: &[u8],
    h: &Header,
    side: &Sidecars,
    opts: &DbfOptions,
) -> Result<(Decoder, String, Vec<String>), Error> {
    let mut notes = Vec::new();
    if !opts.encoding.trim().is_empty() {
        let d = by_label(&opts.encoding).ok_or_else(|| {
            Error::Invalid("that encoding cannot be used for a dBase file".into())
        })?;
        return Ok((d, "your choice".into(), notes));
    }
    if let Some(cpg) = &side.cpg {
        let text = String::from_utf8_lossy(cpg);
        let line = text.lines().next().unwrap_or("").trim();
        match by_label(line) {
            Some(d) => return Ok((d, "the .cpg file".into(), notes)),
            None => notes.push(
                "the .cpg file names an encoding this app cannot use here, so it was passed over"
                    .to_string(),
            ),
        }
    }
    match language(h.language) {
        Language::Known(d) => return Ok((d, "the code page stored in the table".into(), notes)),
        Language::Unsupported(page) => notes.push(format!(
            "the table says code page {page}, which is not supported: pick an encoding if text looks wrong"
        )),
        Language::None => {}
    }
    Ok(if all_utf8(data, h) {
        (
            Decoder::Rs(encoding_rs::UTF_8),
            "a guess: all text is valid UTF-8".into(),
            notes,
        )
    } else {
        (
            Decoder::Rs(encoding_rs::WINDOWS_1252),
            "a guess: the text is not UTF-8".into(),
            notes,
        )
    })
}

fn projection(side: &Sidecars) -> String {
    side.prj
        .as_deref()
        .map(|p| {
            String::from_utf8_lossy(p)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .chars()
                .take(200)
                .collect()
        })
        .unwrap_or_default()
}

/// What the table holds.
pub fn inspect(data: &[u8], side: &Sidecars, opts: &DbfOptions) -> Result<DbfInfo, Error> {
    let h = header(data)?;
    let (decoder, rule, notes) = choose(data, &h, side, opts)?;
    let mut warnings = h.warnings.clone();
    warnings.extend(notes);
    warnings.extend(side.problems.iter().cloned());
    let deleted = data[h.start..h.start + h.records as usize * h.record_len]
        .chunks_exact(h.record_len)
        .filter(|r| r[0] == b'*')
        .count() as u64;
    Ok(DbfInfo {
        records: h.records,
        deleted,
        fields: h.fields.iter().filter(|f| !f.hidden).count() as u32,
        encoding: decoder.name(),
        rule,
        projection: projection(side),
        warnings,
    })
}

// ---- values ----

/// A `.dbt` memo file.
struct Memos<'a> {
    data: &'a [u8],
    block: u64,
    /// dBase IV: each memo starts with its length.
    counted: bool,
    scanned: AtomicU64,
}

enum Memo<'a> {
    Text(&'a [u8]),
    /// Points outside the file, has no end, or is too long.
    Bad,
}

impl<'a> Memos<'a> {
    fn open(data: &'a [u8], version: u8) -> Result<Memos<'a>, &'static str> {
        let counted = matches!(version, 0x8B | 0x8C | 0x04);
        let block = if counted {
            if data.len() < 22 {
                return Err("the .dbt file is too short");
            }
            let b = u64::from(u16_at(data, 20));
            if !(64..=32_768).contains(&b) {
                return Err("the .dbt file has a block size that makes no sense");
            }
            b
        } else {
            512
        };
        Ok(Memos {
            data,
            block,
            counted,
            scanned: AtomicU64::new(0),
        })
    }

    fn get(&self, number: u64) -> Result<Memo<'a>, Error> {
        let data = self.data;
        let Some(at) = number
            .checked_mul(self.block)
            .filter(|&a| a < data.len() as u64)
        else {
            return Ok(Memo::Bad);
        };
        let rest = &data[at as usize..];
        if self.counted && rest.starts_with(&[0xFF, 0xFF, 0x08, 0x00]) {
            let Some(len) = rest.get(4..8) else {
                return Ok(Memo::Bad);
            };
            let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
            if !(8..=MAX_MEMO + 8).contains(&len) {
                return Ok(Memo::Bad);
            }
            return Ok(rest.get(8..len).map_or(Memo::Bad, Memo::Text));
        }
        let window = &rest[..rest.len().min(MAX_MEMO + 1)];
        let end = memchr::memchr(0x1A, window);
        let looked = end.unwrap_or(window.len()) as u64 + 1;
        if self.scanned.fetch_add(looked, Ordering::Relaxed) + looked > MAX_MEMO_SCAN {
            return Err(bad("its memo file is damaged (memos without an end)"));
        }
        Ok(match end {
            Some(e) => Memo::Text(&window[..e]),
            None => Memo::Bad,
        })
    }
}

/// `YYYY-MM-DD` of a day counted from 1970-01-01.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A Visual FoxPro date and time: a Julian day and milliseconds.
fn date_time(b: &[u8; 8]) -> Option<String> {
    let day = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let ms = i32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    if day == 0 {
        return Some(String::new());
    }
    if !(0..86_400_000).contains(&ms) {
        return None;
    }
    let (y, m, d) = civil(i64::from(day) - 2_440_588);
    if !(0..=9999).contains(&y) {
        return None;
    }
    let s = ms / 1000;
    let mut out = format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        s / 3600,
        s / 60 % 60,
        s % 60
    );
    if ms % 1000 != 0 {
        out.push_str(&format!(".{:03}", ms % 1000));
    }
    Some(out)
}

/// A currency value: a whole number of ten-thousandths.
fn currency(v: i64) -> String {
    let a = v.unsigned_abs();
    let sign = if v < 0 { "-" } else { "" };
    format!("{sign}{}.{:04}", a / 10_000, a % 10_000)
}

/// What went wrong in a field, counted for one warning each.
#[derive(Default, Clone, Copy)]
struct Troubles {
    overflow: u64,
    unreadable: u64,
    memo: u64,
    fox_memo: u64,
    unsupported: u64,
    bytes: u64,
}

impl Troubles {
    fn add(&mut self, o: &Troubles) {
        self.overflow += o.overflow;
        self.unreadable += o.unreadable;
        self.memo += o.memo;
        self.fox_memo += o.fox_memo;
        self.unsupported += o.unsupported;
        self.bytes += o.bytes;
    }
}

fn trim_end(b: &[u8]) -> &[u8] {
    let n = b
        .iter()
        .rposition(|&c| c != b' ' && c != 0)
        .map_or(0, |i| i + 1);
    &b[..n]
}

fn trim_both(b: &[u8]) -> &[u8] {
    let b = trim_end(b);
    let n = b
        .iter()
        .position(|&c| c != b' ' && c != 0)
        .unwrap_or(b.len());
    &b[n..]
}

struct Reader<'a> {
    h: Header,
    decoder: Decoder,
    memos: Option<Memos<'a>>,
}

impl Reader<'_> {
    fn text(&self, b: &[u8], out: &mut String, t: &mut Troubles) {
        if !self.decoder.push(b, out) {
            t.bytes += 1;
        }
    }

    /// The value of field `i` of `rec`, appended to `out`.
    fn value(&self, i: usize, rec: &[u8], out: &mut String, t: &mut Troubles) -> Result<(), Error> {
        let f = &self.h.fields[i];
        let (kind, short_bit, null_bit) = (f.kind, f.short_bit, f.null_bit);
        let Some(raw) = slice(rec, f) else {
            t.unreadable += 1;
            return Ok(());
        };
        let flag = |bit: usize| {
            self.h
                .flags
                .and_then(|k| slice(rec, &self.h.fields[k]))
                .and_then(|b| b.get(bit / 8))
                .is_some_and(|&b| b & (1 << (bit % 8)) != 0)
        };
        if null_bit.is_some_and(flag) {
            return Ok(());
        }
        let fox = self.h.flavor == Flavor::FoxPro;
        let seven = self.h.flavor == Flavor::Dbase7;
        match kind {
            b'C' => self.text(trim_end(raw), out, t),
            b'V' if fox => {
                let b = if short_bit.is_some_and(flag) {
                    let n = raw.last().map_or(0, |&n| usize::from(n));
                    &raw[..n.min(raw.len().saturating_sub(1))]
                } else {
                    raw
                };
                self.text(b, out, t);
            }
            b'N' | b'F' => {
                let b = trim_both(raw);
                if !b.is_empty() && b.iter().all(|&c| c == b'*') {
                    t.overflow += 1;
                }
                self.text(b, out, t);
            }
            b'D' => {
                let b = trim_both(raw);
                if b.len() == 8 && b.iter().all(u8::is_ascii_digit) {
                    if b != b"00000000" {
                        let s = std::str::from_utf8(b).unwrap_or("");
                        out.push_str(&s[..4]);
                        out.push('-');
                        out.push_str(&s[4..6]);
                        out.push('-');
                        out.push_str(&s[6..]);
                    }
                } else {
                    self.text(b, out, t);
                }
            }
            b'L' => match raw.first() {
                Some(b'T' | b't' | b'Y' | b'y') => out.push_str("TRUE"),
                Some(b'F' | b'f' | b'N' | b'n') => out.push_str("FALSE"),
                _ => {}
            },
            b'M' => {
                let number = if raw.len() == 4 {
                    Some(u64::from(u32::from_le_bytes([
                        raw[0], raw[1], raw[2], raw[3],
                    ])))
                } else {
                    let b = trim_both(raw);
                    if b.is_empty() {
                        Some(0)
                    } else {
                        std::str::from_utf8(b).ok().and_then(|s| s.parse().ok())
                    }
                };
                match number {
                    Some(0) => {}
                    // Left blank: a placeholder would be saved, and joined
                    // back into the GIS data, as if it were the value.
                    Some(_) if fox || self.h.version == 0xF5 => t.fox_memo += 1,
                    Some(n) => match &self.memos {
                        Some(m) => match m.get(n)? {
                            Memo::Text(b) => {
                                let ok = self.decoder.push(trim_end(b), out);
                                t.bytes += u64::from(!ok);
                            }
                            Memo::Bad => t.memo += 1,
                        },
                        None => t.memo += 1,
                    },
                    None => t.unreadable += 1,
                }
            }
            b'I' if fox => match <[u8; 4]>::try_from(raw) {
                Ok(b) => out.push_str(&i32::from_le_bytes(b).to_string()),
                Err(_) => t.unreadable += 1,
            },
            // dBase 7 whole numbers: big end first, the sign bit flipped.
            b'I' | b'+' if seven => match <[u8; 4]>::try_from(raw) {
                Ok(b) if b != [0; 4] => {
                    let v = (u32::from_be_bytes(b) ^ 0x8000_0000) as i32;
                    out.push_str(&v.to_string());
                }
                Ok(_) => {}
                Err(_) => t.unreadable += 1,
            },
            b'O' if seven => match <[u8; 8]>::try_from(raw) {
                Ok(b) if b != [0; 8] => {
                    let v = u64::from_be_bytes(b);
                    let v = if v >> 63 == 1 { v ^ (1 << 63) } else { !v };
                    out.push_str(&f64::from_bits(v).to_string());
                }
                Ok(_) => {}
                Err(_) => t.unreadable += 1,
            },
            b'B' if fox => match <[u8; 8]>::try_from(raw) {
                Ok(b) => out.push_str(&f64::from_le_bytes(b).to_string()),
                Err(_) => t.unreadable += 1,
            },
            b'Y' if fox => match <[u8; 8]>::try_from(raw) {
                Ok(b) => out.push_str(&currency(i64::from_le_bytes(b))),
                Err(_) => t.unreadable += 1,
            },
            b'T' if fox => match <[u8; 8]>::try_from(raw).ok().and_then(|b| date_time(&b)) {
                Some(s) => out.push_str(&s),
                None => t.unreadable += 1,
            },
            _ => t.unsupported += 1,
        }
        if out.len() > MAX_CELL {
            return Err(bad("a cell holds more than 32 MB of text"));
        }
        Ok(())
    }
}

/// Which columns a record becomes.
struct Layout {
    opts: DbfOptions,
    /// Columns before the fields (FID, Deleted).
    lead: usize,
    shown: Vec<usize>,
}

/// The rows of one run of records, as table lines.
struct Part {
    bytes: Vec<u8>,
    rows: u64,
    cells: u64,
    cols: u32,
    troubles: Vec<Troubles>,
}

impl Reader<'_> {
    /// Record number `n` into `row`; false when it is deleted and not shown.
    fn record(
        &self,
        layout: &Layout,
        n: usize,
        rec: &[u8],
        row: &mut [String],
        troubles: &mut [Troubles],
        room: usize,
    ) -> Result<bool, Error> {
        let deleted = rec[0] == b'*';
        if deleted && !layout.opts.show_deleted {
            return Ok(false);
        }
        for c in row.iter_mut() {
            c.clear();
        }
        let mut at = 0;
        if layout.opts.fid != Fid::Off {
            // The number counts every record, deleted ones too, so it still
            // matches the shape in the .shp file.
            let base = usize::from(layout.opts.fid == Fid::From1);
            let _ = write!(row[0], "{}", n + base);
            at = 1;
        }
        if layout.opts.show_deleted {
            row[at].push_str(if deleted { "TRUE" } else { "FALSE" });
        }
        let mut size = 0usize;
        for (k, &i) in layout.shown.iter().enumerate() {
            self.value(i, rec, &mut row[layout.lead + k], &mut troubles[i])?;
            size = size.saturating_add(row[layout.lead + k].len());
            if size > room {
                return Err(too_big());
            }
        }
        Ok(true)
    }

    fn part(
        &self,
        layout: &Layout,
        first: usize,
        records: &[u8],
        record_len: usize,
        room: usize,
    ) -> Result<Part, Error> {
        let mut p = Part {
            bytes: Vec::with_capacity(records.len().min(room)),
            rows: 0,
            cells: 0,
            cols: 0,
            troubles: vec![Troubles::default(); self.h.fields.len()],
        };
        let mut row = vec![String::new(); layout.lead + layout.shown.len()];
        for (k, rec) in records.chunks_exact(record_len).enumerate() {
            if !self.record(layout, first + k, rec, &mut row, &mut p.troubles, room)? {
                continue;
            }
            // Trailing empty cells are left off, as the importers do.
            let n = row.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1);
            if n == 0 {
                p.bytes.extend_from_slice(b"\r\n");
            } else {
                push_line(&mut p.bytes, &row[..n]);
            }
            if p.bytes.len() > room {
                return Err(too_big());
            }
            p.rows += 1;
            p.cells += n.max(1) as u64;
            p.cols = p.cols.max(n as u32);
        }
        Ok(p)
    }
}

/// Put converted runs into the table, in order.
fn add(
    sink: &mut Sink,
    troubles: &mut [Troubles],
    parts: Vec<Result<Part, Error>>,
) -> Result<(), Error> {
    for part in parts {
        let part = part?;
        sink.lines(&part.bytes, part.rows, part.cells, part.cols)?;
        for (all, one) in troubles.iter_mut().zip(&part.troubles) {
            all.add(one);
        }
    }
    Ok(())
}

/// Read the records into `sink` (a title row first). Returns the warnings.
pub fn read(
    data: &[u8],
    side: &Sidecars,
    opts: &DbfOptions,
    sink: &mut Sink,
    progress: &Progress,
) -> Result<Vec<String>, Error> {
    let h = header(data)?;
    let (decoder, _, notes) = choose(data, &h, side, opts)?;
    let mut warnings = h.warnings.clone();
    warnings.extend(notes);
    warnings.extend(side.problems.iter().cloned());
    let has_memo = h.fields.iter().any(|f| f.kind == b'M');
    let memos = match &side.dbt {
        Some(d) if has_memo && h.flavor != Flavor::FoxPro && h.version != 0xF5 => {
            match Memos::open(d, h.version) {
                Ok(m) => Some(m),
                Err(e) => {
                    warnings.push(e.to_string());
                    None
                }
            }
        }
        _ => None,
    };
    let shown: Vec<usize> = (0..h.fields.len())
        .filter(|&i| !h.fields[i].hidden)
        .collect();
    let mut troubles = vec![Troubles::default(); h.fields.len()];
    let (records, start, record_len) = (h.records, h.start, h.record_len);
    let r = Reader { h, decoder, memos };
    let layout = Layout {
        opts: opts.clone(),
        lead: usize::from(opts.fid != Fid::Off) + usize::from(opts.show_deleted),
        shown,
    };
    let width = layout.lead + layout.shown.len();
    let mut names: Vec<String> = Vec::with_capacity(width);
    if opts.fid != Fid::Off {
        names.push("FID".into());
    }
    if opts.show_deleted {
        names.push("Deleted".into());
    }
    for &i in &layout.shown {
        let mut name = String::new();
        r.decoder.push(&r.h.fields[i].name, &mut name);
        if name.trim().is_empty() {
            name = format!("Column {}", names.len() + 1);
        }
        names.push(name);
    }
    sink.row(&names)?;

    progress.start(records.max(1));
    let end = start + records as usize * record_len;
    let body = &data[start..end];
    if sink.is_preview() {
        // A preview: one record at a time until it has enough rows.
        let mut row = vec![String::new(); width];
        for (n, rec) in body.chunks_exact(record_len).enumerate() {
            if sink.full() {
                break;
            }
            if n % CHUNK == CHUNK - 1 {
                progress.step(CHUNK as u64)?;
            }
            if !r.record(&layout, n, rec, &mut row, &mut troubles, sink.room())? {
                continue;
            }
            if row.iter().all(String::is_empty) {
                // A blank record is a record: not dropped as padding.
                sink.blank_row()?;
            } else {
                sink.row(&row)?;
            }
        }
    } else {
        // The whole table: records are converted in parallel, in order.
        // Its text is no longer than the records, plus numbers and commas.
        sink.reserve(body.len().saturating_add(records as usize * 8));
        let chunk = CHUNK * record_len;
        // Each run is added to the table while the next one is converted.
        let mut ready: Vec<Result<Part, Error>> = Vec::new();
        let mut done = 0;
        for (b, batch) in body.chunks(chunk * BATCH).enumerate() {
            let room = sink.room();
            let (next, added) = rayon::join(
                || {
                    batch
                        .par_chunks(chunk)
                        .enumerate()
                        .map(|(c, recs)| {
                            r.part(&layout, (b * BATCH + c) * CHUNK, recs, record_len, room)
                        })
                        .collect::<Vec<_>>()
                },
                || add(sink, &mut troubles, std::mem::take(&mut ready)),
            );
            added?;
            ready = next;
            progress.step(done)?;
            done = (batch.len() / record_len) as u64;
        }
        add(sink, &mut troubles, ready)?;
        progress.step(done)?;
    }

    for (i, t) in troubles.iter().enumerate() {
        let f = &r.h.fields[i];
        let mut name = String::new();
        r.decoder.push(&f.name, &mut name);
        let mut say = |n: u64, what: &str| {
            if n > 0 {
                warnings.push(format!("field {name}: {what} ({n} records)"));
            }
        };
        say(t.overflow, "numbers too wide for the field, kept as ****");
        say(
            t.unreadable,
            "values that could not be read were left blank",
        );
        say(t.memo, "memos missing or damaged were left blank");
        say(t.fox_memo, "FoxPro memos are not read");
        say(
            t.unsupported,
            &format!(
                "field type {} is not supported, left blank",
                char::from(f.kind).escape_default()
            ),
        );
        say(t.bytes, "bytes that do not fit the encoding were replaced");
    }
    Ok(warnings)
}
