//! A small, careful ZIP reader for spreadsheet files (`.xlsx`, `.ods`).
//!
//! Spreadsheet files are untrusted input. This reader only ever reads parts
//! into memory by exact name (nothing is written to disk, names are never
//! used as paths), supports only "stored" and "deflate", and caps everything:
//! entry count, each part's decompressed size, the total decompressed while
//! reading one file, and the compression ratio (zip bombs). Declared sizes
//! are never trusted: reading stops at the caps, and each part's CRC must
//! match once it has been read to the end.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, Read};

use crate::Error;

/// Largest spreadsheet file read.
pub const MAX_INPUT: u64 = 1 << 30;
/// Most entries in one archive.
pub const MAX_ENTRIES: usize = 100_000;
/// Largest decompressed part.
pub const MAX_PART: u64 = 2 << 30;
/// Most bytes decompressed in total while reading one archive.
pub const MAX_TOTAL: u64 = 4 << 30;
/// A part may inflate at most this many times its compressed size...
pub const MAX_RATIO: u64 = 250;
/// ...once it is past this size (small parts compress very well).
pub const RATIO_FREE: u64 = 1 << 20;

const EOCD: u32 = 0x0605_4b50;
const EOCD64_LOCATOR: u32 = 0x0706_4b50;
const EOCD64: u32 = 0x0606_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

#[derive(Debug, Clone)]
struct Entry {
    method: u16,
    crc: u32,
    csize: u64,
    /// Declared; only used as a hint.
    usize: u64,
    local: u64,
}

/// An opened archive over bytes in memory.
pub struct Zip<'a> {
    data: &'a [u8],
    entries: HashMap<String, Entry>,
    /// Lower-cased name → name, for writers that change case.
    folded: HashMap<String, String>,
    used: Cell<u64>,
}

pub(crate) fn damaged(why: &str) -> Error {
    Error::Invalid(format!("the file is damaged or not a spreadsheet ({why})"))
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    d.get(at..at + 8).map(|b| {
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        u64::from_le_bytes(a)
    })
}

/// True when `data` starts like a ZIP archive.
pub fn is_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04")
}

impl<'a> Zip<'a> {
    pub fn open(data: &'a [u8]) -> Result<Zip<'a>, Error> {
        if data.len() as u64 > MAX_INPUT {
            return Err(Error::Invalid(
                "the file is larger than 1 GB, too large to import".into(),
            ));
        }
        // End of central directory: the last signature in the final 64 KiB.
        let from = data.len().saturating_sub(22 + 65_535);
        let eocd = (from..data.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(data, i) == Some(EOCD))
            .ok_or_else(|| damaged("no directory"))?;
        let bad = || damaged("bad directory");
        let mut count = u16_at(data, eocd + 10).ok_or_else(bad)? as u64;
        let mut size = u32_at(data, eocd + 12).ok_or_else(bad)? as u64;
        let mut offset = u32_at(data, eocd + 16).ok_or_else(bad)? as u64;
        if u16_at(data, eocd + 4) != Some(0) || u16_at(data, eocd + 6) != Some(0) {
            return Err(damaged("split archives are not supported"));
        }
        if count == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
            let loc = eocd.checked_sub(20).ok_or_else(bad)?;
            if u32_at(data, loc) != Some(EOCD64_LOCATOR) {
                return Err(bad());
            }
            let at = u64_at(data, loc + 8).ok_or_else(bad)?;
            let at = usize::try_from(at).map_err(|_| bad())?;
            if at >= loc || u32_at(data, at) != Some(EOCD64) {
                return Err(bad());
            }
            count = u64_at(data, at + 32).ok_or_else(bad)?;
            size = u64_at(data, at + 40).ok_or_else(bad)?;
            offset = u64_at(data, at + 48).ok_or_else(bad)?;
        }
        if count > MAX_ENTRIES as u64 {
            return Err(damaged("too many parts"));
        }
        let end = offset.checked_add(size).ok_or_else(bad)?;
        if end > eocd as u64 {
            return Err(bad());
        }
        let mut entries = HashMap::new();
        let mut folded = HashMap::new();
        let mut p = offset as usize;
        for _ in 0..count {
            if u32_at(data, p) != Some(CENTRAL) || p as u64 + 46 > end {
                return Err(bad());
            }
            let flags = u16_at(data, p + 8).ok_or_else(bad)?;
            let method = u16_at(data, p + 10).ok_or_else(bad)?;
            let crc = u32_at(data, p + 16).ok_or_else(bad)?;
            let mut csize = u32_at(data, p + 20).ok_or_else(bad)? as u64;
            let mut usize_ = u32_at(data, p + 24).ok_or_else(bad)? as u64;
            let nlen = u16_at(data, p + 28).ok_or_else(bad)? as usize;
            let xlen = u16_at(data, p + 30).ok_or_else(bad)? as usize;
            let clen = u16_at(data, p + 32).ok_or_else(bad)? as usize;
            let mut local = u32_at(data, p + 42).ok_or_else(bad)? as u64;
            let name = data.get(p + 46..p + 46 + nlen).ok_or_else(bad)?;
            let extra = data
                .get(p + 46 + nlen..p + 46 + nlen + xlen)
                .ok_or_else(bad)?;
            // Zip64 sizes and offset, in that order, for fields set to all ones.
            let mut x = 0;
            while x + 4 <= extra.len() {
                let id = u16_at(extra, x).unwrap_or(0);
                let len = u16_at(extra, x + 2).unwrap_or(0) as usize;
                let body = extra.get(x + 4..x + 4 + len).ok_or_else(bad)?;
                if id == 1 {
                    let mut k = 0;
                    let mut take = |v: &mut u64, wide: bool| -> Result<(), Error> {
                        if wide {
                            *v = u64_at(body, k).ok_or_else(bad)?;
                            k += 8;
                        }
                        Ok(())
                    };
                    let wide = [usize_, csize, local].map(|v| v == 0xFFFF_FFFF);
                    take(&mut usize_, wide[0])?;
                    take(&mut csize, wide[1])?;
                    take(&mut local, wide[2])?;
                }
                x += 4 + len;
            }
            p += 46 + nlen + xlen + clen;
            if p as u64 > end {
                return Err(bad());
            }
            if flags & 1 != 0 {
                return Err(Error::Invalid(
                    "the file is password protected; save it without a password first".into(),
                ));
            }
            let name = String::from_utf8_lossy(name).into_owned();
            if name.ends_with('/') {
                continue; // a folder
            }
            let fold = name.to_lowercase();
            if entries.contains_key(&name) || folded.contains_key(&fold) {
                // Two parts with one name: refuse rather than guess which.
                return Err(damaged("a part appears twice"));
            }
            folded.insert(fold, name.clone());
            entries.insert(
                name,
                Entry {
                    method,
                    crc,
                    csize,
                    usize: usize_,
                    local,
                },
            );
        }
        Ok(Zip {
            data,
            entries,
            folded,
            used: Cell::new(0),
        })
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        let name = name.trim_start_matches('/');
        self.entries.get(name).or_else(|| {
            self.folded
                .get(&name.to_lowercase())
                .and_then(|n| self.entries.get(n))
        })
    }

    /// Every part's name and declared (untrusted) unpacked size, by name.
    pub fn names(&self) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = self.entries.iter().map(|(n, e)| (n.clone(), e.usize)).collect();
        v.sort();
        v
    }

    /// A whole part, unpacked (at most `cap` bytes; the archive's own caps
    /// and checks apply too).
    pub fn read_bytes(&self, name: &str, cap: u64) -> Result<Vec<u8>, Error> {
        let mut r = self.reader(name)?;
        let mut v = Vec::new();
        (&mut r).take(cap + 1).read_to_end(&mut v).map_err(io_err)?;
        if v.len() as u64 > cap {
            return Err(Error::Invalid(format!("{name} is too large to open")));
        }
        Ok(v)
    }

    pub fn has(&self, name: &str) -> bool {
        self.entry(name).is_some()
    }

    /// Declared size of a part (untrusted; for progress only).
    pub fn declared_size(&self, name: &str) -> u64 {
        self.entry(name).map_or(0, |e| e.usize)
    }

    /// A reader of one part, decompressed, capped and checked.
    pub fn reader(&self, name: &str) -> Result<PartReader<'_>, Error> {
        let e = self
            .entry(name)
            .ok_or_else(|| damaged(&format!("{name} is missing")))?
            .clone();
        let bad = || damaged("bad part header");
        let at = usize::try_from(e.local).map_err(|_| bad())?;
        if u32_at(self.data, at) != Some(LOCAL) {
            return Err(bad());
        }
        let nlen = u16_at(self.data, at + 26).ok_or_else(bad)? as usize;
        let xlen = u16_at(self.data, at + 28).ok_or_else(bad)? as usize;
        let start = at + 30 + nlen + xlen;
        let csize = usize::try_from(e.csize).map_err(|_| bad())?;
        let raw = self
            .data
            .get(start..start.checked_add(csize).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        let inner = match e.method {
            0 => Inner::Stored(raw),
            8 => Inner::Deflate(Box::new(flate2::read::DeflateDecoder::new(raw))),
            _ => {
                return Err(Error::Invalid(
                    "the file uses a compression method that is not supported".into(),
                ))
            }
        };
        Ok(PartReader {
            inner,
            crc: crc32fast::Hasher::new(),
            want_crc: e.crc,
            csize: e.csize,
            produced: 0,
            used: &self.used,
            done: false,
        })
    }

    /// A whole part as text (for small parts: relationships, workbook).
    pub fn read_string(&self, name: &str, cap: u64) -> Result<String, Error> {
        let mut r = self.reader(name)?;
        let mut v = Vec::new();
        (&mut r).take(cap + 1).read_to_end(&mut v).map_err(io_err)?;
        if v.len() as u64 > cap {
            return Err(damaged(&format!("{name} is too large")));
        }
        String::from_utf8(v).map_err(|_| damaged(&format!("{name} is not UTF-8 text")))
    }
}

enum Inner<'a> {
    Stored(&'a [u8]),
    Deflate(Box<flate2::read::DeflateDecoder<&'a [u8]>>),
}

/// Reads one decompressed part; errors past the caps or on a bad CRC.
pub struct PartReader<'a> {
    inner: Inner<'a>,
    crc: crc32fast::Hasher,
    want_crc: u32,
    csize: u64,
    produced: u64,
    used: &'a Cell<u64>,
    done: bool,
}

/// Errors from a part reader carry an engine error inside.
pub(crate) fn io_err(e: io::Error) -> Error {
    io_err_ref(&e)
}

pub(crate) fn io_err_ref(e: &io::Error) -> Error {
    match e.get_ref().and_then(|i| i.downcast_ref::<Limit>()) {
        Some(Limit(m)) => Error::Invalid((*m).to_string()),
        None => damaged(&e.to_string()),
    }
}

#[derive(Debug)]
struct Limit(&'static str);

impl std::fmt::Display for Limit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Limit {}

fn limit(m: &'static str) -> io::Error {
    io::Error::other(Limit(m))
}

impl Read for PartReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        let n = match &mut self.inner {
            Inner::Stored(s) => s.read(buf)?,
            Inner::Deflate(d) => d.read(buf)?,
        };
        if n == 0 {
            self.done = true;
            if self.crc.clone().finalize() != self.want_crc {
                return Err(limit(
                    "the file is damaged (a part does not match its checksum)",
                ));
            }
            return Ok(0);
        }
        self.produced += n as u64;
        let used = self.used.get() + n as u64;
        self.used.set(used);
        if self.produced > MAX_PART || used > MAX_TOTAL {
            return Err(limit("the file unpacks to more data than can be imported"));
        }
        if self.produced > RATIO_FREE && self.produced / self.csize.max(1) > MAX_RATIO {
            return Err(limit(
                "the file unpacks to far more data than it holds (a zip bomb); not imported",
            ));
        }
        self.crc.update(&buf[..n]);
        Ok(n)
    }
}

/// Resolve a relationship target against the folder of the part that names
/// it, the way the package format does (no file system involved).
pub fn resolve(base_part: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        let mut b: Vec<&str> = base_part.split('/').collect();
        b.pop();
        b
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// Build an archive: (name, data, deflate?).
    pub(crate) fn build(parts: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in parts {
            let body = if *deflate {
                let mut e =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let crc = crc32fast::hash(data);
            let off = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            out.extend_from_slice(&LOCAL.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&body);
            central.extend_from_slice(&CENTRAL.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&off.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
        out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn reads_stored_and_deflated_parts() {
        let z = build(&[("a.txt", b"hello", false), ("dir/B.xml", b"<x/>", true)]);
        let zip = Zip::open(&z).unwrap();
        assert_eq!(zip.read_string("a.txt", 100).unwrap(), "hello");
        assert_eq!(zip.read_string("/dir/b.xml", 100).unwrap(), "<x/>");
        assert!(zip.reader("missing").is_err());
        assert!(zip.read_string("a.txt", 2).is_err());
    }

    #[test]
    fn refuses_bad_archives() {
        assert!(Zip::open(b"").is_err());
        assert!(Zip::open(b"PK\x03\x04garbage").is_err());
        // Duplicate names.
        let z = build(&[("a", b"1", false), ("A", b"2", false)]);
        assert!(Zip::open(&z).is_err());
        // Corrupted data fails its checksum.
        let mut z = build(&[("a", b"hello", false)]);
        let at = z.windows(5).position(|w| w == b"hello").unwrap();
        z[at] = b'j';
        let zip = Zip::open(&z).unwrap();
        assert!(zip.read_string("a", 100).is_err());
        // Truncated anywhere: never panics.
        let z = build(&[("a.xml", b"<a>text</a>", true), ("b", b"x", false)]);
        for cut in 0..z.len() {
            if let Ok(zip) = Zip::open(&z[..cut]) {
                let _ = zip.read_string("a.xml", 100);
            }
        }
        // Flipped bytes: never panics.
        for i in 0..z.len() {
            let mut y = z.clone();
            y[i] ^= 0xFF;
            if let Ok(zip) = Zip::open(&y) {
                let _ = zip.read_string("a.xml", 100);
                let _ = zip.read_string("b", 100);
            }
        }
    }

    #[test]
    fn stops_zip_bombs() {
        // 64 MB of zeros compress to about 64 KB: over the ratio cap.
        let zeros = vec![0u8; 64 << 20];
        let z = build(&[("bomb", &zeros, true)]);
        let zip = Zip::open(&z).unwrap();
        let mut r = zip.reader("bomb").unwrap();
        let e = io::copy(&mut r, &mut io::sink()).unwrap_err();
        assert!(io_err(e).to_string().contains("zip bomb"));
        // Ordinary text of the same size reads fine.
        let mut seed = 7u32;
        let text: Vec<u8> = (0..4u32 << 20)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                b"abc,123\n"[(seed >> 28) as usize & 7]
            })
            .collect();
        let z = build(&[("x", &text, true)]);
        let zip = Zip::open(&z).unwrap();
        let mut r = zip.reader("x").unwrap();
        assert_eq!(
            io::copy(&mut r, &mut io::sink()).unwrap(),
            text.len() as u64
        );
    }

    #[test]
    fn resolves_targets() {
        assert_eq!(
            resolve("xl/workbook.xml", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(resolve("xl/workbook.xml", "/xl/s.xml"), "xl/s.xml");
        assert_eq!(resolve("xl/workbook.xml", "../../../x"), "x");
        // The package's own relationships resolve from its root.
        assert_eq!(resolve("", "xl/workbook.xml"), "xl/workbook.xml");
    }
}
