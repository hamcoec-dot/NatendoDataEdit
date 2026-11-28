//! Just enough TrueType reading to embed a font in a PDF: character to
//! glyph lookup (cmap formats 4 and 12), glyph widths, the font's box and
//! whether its licence allows embedding; the first face of a collection
//! (`.ttc`); and a subset of the glyphs a PDF uses (glyph numbers kept).
//! Every offset is checked against the file; a damaged font is refused,
//! never trusted.

use crate::Error;

/// Largest font file read.
pub const MAX_FONT_BYTES: usize = 32 << 20;

fn damaged() -> Error {
    Error::Invalid("the font file is damaged".into())
}

fn u16_at(d: &[u8], at: usize) -> Result<u16, Error> {
    d.get(at..at + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(damaged)
}

fn i16_at(d: &[u8], at: usize) -> Result<i16, Error> {
    u16_at(d, at).map(|v| v as i16)
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, Error> {
    d.get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(damaged)
}

enum Cmap {
    /// Format 4: (end, start, delta, range offset position) per segment.
    Segments {
        segs: Vec<(u16, u16, i16, u16, usize)>,
    },
    /// Format 12: (first char, last char, first glyph).
    Groups(Vec<(u32, u32, u32)>),
}

/// A TrueType font held whole.
pub struct Font {
    pub data: Vec<u8>,
    pub units_per_em: u16,
    pub ascent: i16,
    pub descent: i16,
    pub bbox: [i16; 4],
    advances: Vec<u16>,
    num_glyphs: u16,
    cmap: Cmap,
    glyph_data: usize,
    /// The licence bits (OS/2 fsType; 0 when there is no OS/2 table).
    pub fs_type: u16,
    /// Where the tables a subset needs are: (offset, length).
    tables: Tables,
}

#[derive(Default, Clone, Copy)]
struct Tables {
    head: (usize, usize),
    hhea: (usize, usize),
    maxp: (usize, usize),
    hmtx: (usize, usize),
    glyf: (usize, usize),
    loca: Option<(usize, usize)>,
    cvt: Option<(usize, usize)>,
    fpgm: Option<(usize, usize)>,
    prep: Option<(usize, usize)>,
}

/// Deepest nesting of composite glyphs followed when subsetting.
const MAX_COMPONENT_DEPTH: usize = 16;

/// Where the table directory of a font file's first face starts: 0 for a
/// font, face 0's offset for a collection (`ttcf`, version 1 or 2). A
/// collection whose face is another collection, or does not fit, is refused.
fn first_face(d: &[u8]) -> Result<usize, Error> {
    if d.get(0..4) != Some(b"ttcf".as_slice()) {
        return Ok(0);
    }
    let major = u16_at(d, 4)?;
    if major != 1 && major != 2 {
        return Err(damaged());
    }
    let faces = u32_at(d, 8)?;
    if faces == 0 || faces as usize > d.len() / 4 {
        return Err(damaged());
    }
    let at = u32_at(d, 12)? as usize;
    // Its own header (12 bytes) and table directory must fit the file.
    let n = u16_at(d, at.checked_add(4).ok_or_else(damaged)?)? as usize;
    if at.checked_add(12 + n * 16).is_none_or(|e| e > d.len()) {
        return Err(damaged());
    }
    Ok(at)
}

impl Font {
    /// Read a font file's tables. Fonts whose licence does not allow
    /// embedding, or that are not TrueType outlines, are refused.
    pub fn parse(data: Vec<u8>) -> Result<Font, Error> {
        if data.len() > MAX_FONT_BYTES {
            return Err(Error::Invalid("the font file is too large".into()));
        }
        let d = &data;
        let face = first_face(d)?;
        let version = u32_at(d, face)?;
        if version != 0x0001_0000 && version != u32::from_be_bytes(*b"true") {
            return Err(Error::Invalid("not a TrueType font".into()));
        }
        let n = u16_at(d, face + 4)? as usize;
        let table = |tag: &[u8; 4]| -> Result<Option<(usize, usize)>, Error> {
            for i in 0..n {
                let rec = face + 12 + i * 16;
                if d.get(rec..rec + 4).ok_or_else(damaged)? == tag {
                    let off = u32_at(d, rec + 8)? as usize;
                    let len = u32_at(d, rec + 12)? as usize;
                    if off.checked_add(len).is_none_or(|e| e > d.len()) {
                        return Err(damaged());
                    }
                    return Ok(Some((off, len)));
                }
            }
            Ok(None)
        };
        let need = |t: Option<(usize, usize)>| t.ok_or_else(damaged);
        let head = need(table(b"head")?)?;
        let hhea = need(table(b"hhea")?)?;
        let maxp = need(table(b"maxp")?)?;
        let hmtx = need(table(b"hmtx")?)?;
        let cmap = need(table(b"cmap")?)?;
        let glyf = need(table(b"glyf")?)?;
        let mut fs_type = 0;
        if let Some((os2, len)) = table(b"OS/2")? {
            if len >= 10 {
                fs_type = u16_at(d, os2 + 8)?;
                // Restricted licence, or bitmap embedding only.
                if fs_type & 0x000F == 0x0002 || fs_type & 0x0200 != 0 {
                    return Err(Error::Invalid(
                        "the font's licence does not allow embedding".into(),
                    ));
                }
            }
        }
        let tables = Tables {
            head,
            hhea,
            maxp,
            hmtx,
            glyf,
            loca: table(b"loca")?,
            cvt: table(b"cvt ")?,
            fpgm: table(b"fpgm")?,
            prep: table(b"prep")?,
        };
        if head.1 < 54 || hhea.1 < 36 || maxp.1 < 6 {
            return Err(damaged());
        }
        let units_per_em = u16_at(d, head.0 + 18)?;
        if units_per_em == 0 {
            return Err(damaged());
        }
        let bbox = [
            i16_at(d, head.0 + 36)?,
            i16_at(d, head.0 + 38)?,
            i16_at(d, head.0 + 40)?,
            i16_at(d, head.0 + 42)?,
        ];
        let ascent = i16_at(d, hhea.0 + 4)?;
        let descent = i16_at(d, hhea.0 + 6)?;
        let metrics = u16_at(d, hhea.0 + 34)? as usize;
        let num_glyphs = u16_at(d, maxp.0 + 4)?;
        if metrics == 0 || metrics > num_glyphs as usize || metrics * 4 > hmtx.1 {
            return Err(damaged());
        }
        let advances = (0..metrics)
            .map(|i| u16_at(d, hmtx.0 + i * 4))
            .collect::<Result<Vec<_>, _>>()?;
        let cmap = read_cmap(d, cmap.0, cmap.1)?;
        Ok(Font {
            units_per_em,
            ascent,
            descent,
            bbox,
            advances,
            num_glyphs,
            cmap,
            glyph_data: glyf.0,
            fs_type,
            tables,
            data,
        })
    }

    /// The licence asks that the font is not subset (fsType bit 0x0100).
    pub fn no_subsetting(&self) -> bool {
        self.fs_type & 0x0100 != 0
    }

    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    /// Glyph `g`'s outline bytes within the file (offsets from loca), or
    /// `None` for an empty glyph or a loca entry that is out of order or
    /// out of the table (treated as empty, never trusted).
    fn glyph_range(&self, g: u16) -> Option<std::ops::Range<usize>> {
        let (loca, loca_len) = self.tables.loca?;
        let (glyf, glyf_len) = self.tables.glyf;
        let long = i16_at(&self.data, self.tables.head.0 + 50).ok()? != 0;
        let g = g as usize;
        let (a, b) = if long {
            if (g + 2) * 4 > loca_len {
                return None;
            }
            (
                u32_at(&self.data, loca + g * 4).ok()? as usize,
                u32_at(&self.data, loca + g * 4 + 4).ok()? as usize,
            )
        } else {
            if (g + 2) * 2 > loca_len {
                return None;
            }
            (
                u16_at(&self.data, loca + g * 2).ok()? as usize * 2,
                u16_at(&self.data, loca + g * 2 + 2).ok()? as usize * 2,
            )
        };
        if a >= b || b > glyf_len {
            return None;
        }
        Some(glyf + a..glyf + b)
    }

    /// The glyphs composite glyph `g` is made of (none for a simple or empty
    /// glyph). A record that runs past the glyph, or names a glyph the font
    /// does not have, is damage.
    fn components(&self, g: u16) -> Result<Vec<u16>, Error> {
        let Some(r) = self.glyph_range(g) else {
            return Ok(Vec::new());
        };
        let d = &self.data[r];
        if d.len() < 10 || i16_at(d, 0)? >= 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut at = 10;
        loop {
            let flags = u16_at(d, at)?;
            let id = u16_at(d, at + 2)?;
            if id >= self.num_glyphs {
                return Err(damaged());
            }
            out.push(id);
            at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
            at += if flags & 0x0008 != 0 {
                2
            } else if flags & 0x0040 != 0 {
                4
            } else if flags & 0x0080 != 0 {
                8
            } else {
                0
            };
            if at > d.len() {
                return Err(damaged());
            }
            if flags & 0x0020 == 0 {
                break;
            }
            if out.len() > 1024 {
                return Err(damaged());
            }
        }
        Ok(out)
    }

    /// Glyph 0, `used`, and every glyph their composites are made of. A
    /// composite that contains itself, or nests more than 16 deep, is
    /// refused rather than followed.
    pub fn glyph_closure(&self, used: &[u16]) -> Result<Vec<bool>, Error> {
        let n = self.num_glyphs as usize;
        let mut keep = vec![false; n];
        // A fixed limit: a font's own maxComponentDepth is often wrong.
        let limit = MAX_COMPONENT_DEPTH;
        fn walk(
            f: &Font,
            g: u16,
            keep: &mut [bool],
            path: &mut Vec<u16>,
            limit: usize,
        ) -> Result<(), Error> {
            if path.contains(&g) {
                return Err(damaged());
            }
            if keep[g as usize] {
                return Ok(());
            }
            keep[g as usize] = true;
            let parts = f.components(g)?;
            if parts.is_empty() {
                return Ok(());
            }
            if path.len() >= limit {
                return Err(damaged());
            }
            path.push(g);
            for p in parts {
                walk(f, p, keep, path, limit)?;
            }
            path.pop();
            Ok(())
        }
        // Depth first, with the glyphs on the way down (a loop is refused).
        let mut path: Vec<u16> = Vec::new();
        for &g in std::iter::once(&0u16).chain(used) {
            if (g as usize) < n {
                walk(self, g, &mut keep, &mut path, limit)?;
            }
        }
        Ok(keep)
    }

    /// A font file holding only glyph 0, `used` and their parts, with every
    /// glyph number unchanged (the others are empty): head, hhea, maxp and
    /// hmtx copied, loca (long) and glyf rebuilt, the hinting tables kept,
    /// checksums made again. Refused for a font that asks not to be subset.
    pub fn subset(&self, used: &[u16]) -> Result<Vec<u8>, Error> {
        if self.no_subsetting() {
            return Err(Error::Invalid(
                "the font's licence does not allow a subset".into(),
            ));
        }
        if self.tables.loca.is_none() {
            return Err(damaged());
        }
        let keep = self.glyph_closure(used)?;
        Ok(self.build(&keep))
    }

    /// A font file in which every glyph is empty (the box drawn as nothing):
    /// used when the glyphs a PDF needs cannot be taken safely. No glyph is
    /// read, so no damaged composite can stop it.
    pub fn empty_subset(&self) -> Result<Vec<u8>, Error> {
        if self.no_subsetting() {
            return Err(Error::Invalid(
                "the font's licence does not allow a subset".into(),
            ));
        }
        Ok(self.build(&vec![false; self.num_glyphs as usize]))
    }

    /// The font file holding the glyphs `keep` marks (the others empty).
    fn build(&self, keep: &[bool]) -> Vec<u8> {
        let d = &self.data;
        let slice = |(at, len): (usize, usize)| -> &[u8] { &d[at..at + len] };
        let mut glyf = Vec::new();
        let mut loca = Vec::with_capacity((keep.len() + 1) * 4);
        for (g, &k) in keep.iter().enumerate() {
            loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
            if k {
                if let Some(r) = self.glyph_range(g as u16) {
                    glyf.extend_from_slice(&d[r]);
                    while glyf.len() % 4 != 0 {
                        glyf.push(0);
                    }
                }
            }
        }
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        let mut head = slice(self.tables.head).to_vec();
        head[8..12].copy_from_slice(&[0; 4]); // checkSumAdjustment, set at the end
        head[50..52].copy_from_slice(&1i16.to_be_bytes()); // loca is long
                                                           // The PDF reaches glyphs by number (Identity), so the character map is
                                                           // the smallest valid one: format 4 with only its closing segment.
        let mut cmap = Vec::new();
        for v in [0u16, 1, 3, 1] {
            cmap.extend_from_slice(&v.to_be_bytes());
        }
        cmap.extend_from_slice(&12u32.to_be_bytes());
        for v in [4u16, 24, 0, 2, 2, 0, 0, 0xFFFF, 0, 0xFFFF, 1, 0] {
            cmap.extend_from_slice(&v.to_be_bytes());
        }
        let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
            (*b"cmap", cmap),
            (*b"glyf", glyf),
            (*b"head", head),
            (*b"hhea", slice(self.tables.hhea).to_vec()),
            (*b"hmtx", slice(self.tables.hmtx).to_vec()),
            (*b"loca", loca),
            (*b"maxp", slice(self.tables.maxp).to_vec()),
        ];
        for (tag, t) in [
            (*b"cvt ", self.tables.cvt),
            (*b"fpgm", self.tables.fpgm),
            (*b"prep", self.tables.prep),
        ] {
            if let Some(t) = t {
                tables.push((tag, slice(t).to_vec()));
            }
        }
        tables.sort_by_key(|t| t.0);
        write_font(&tables)
    }

    /// The glyph for a character (0, the "missing" box, when there is none).
    pub fn glyph(&self, c: char) -> u16 {
        let c = c as u32;
        let g = match &self.cmap {
            Cmap::Segments { segs } => {
                if c > 0xFFFF {
                    return 0;
                }
                let c = c as u16;
                let i = segs.partition_point(|s| s.0 < c);
                match segs.get(i) {
                    Some(&(_end, start, delta, range, pos)) if start <= c => {
                        if range == 0 {
                            (c as i32 + delta as i32) as u16
                        } else {
                            let at = pos + range as usize + 2 * (c - start) as usize;
                            match u16_at(&self.data, at) {
                                Ok(0) | Err(_) => 0,
                                Ok(g) => (g as i32 + delta as i32) as u16,
                            }
                        }
                    }
                    _ => 0,
                }
            }
            Cmap::Groups(groups) => {
                let i = groups.partition_point(|g| g.1 < c);
                match groups.get(i) {
                    Some(&(first, _last, start)) if first <= c => {
                        u16::try_from(start as u64 + (c - first) as u64).unwrap_or(0)
                    }
                    _ => 0,
                }
            }
        };
        if g < self.num_glyphs {
            g
        } else {
            0
        }
    }

    /// A glyph's advance width in font units.
    pub fn advance(&self, g: u16) -> u16 {
        let i = (g as usize).min(self.advances.len() - 1);
        self.advances[i]
    }

    /// Width of text in points at `size`.
    pub fn width(&self, s: &str, size: f64) -> f64 {
        let units: u64 = s.chars().map(|c| self.advance(self.glyph(c)) as u64).sum();
        units as f64 * size / self.units_per_em as f64
    }

    /// Where the glyph outlines start (only used to show the table exists).
    pub fn has_outlines(&self) -> bool {
        self.glyph_data > 0
    }
}

fn read_cmap(d: &[u8], at: usize, len: usize) -> Result<Cmap, Error> {
    let n = u16_at(d, at + 2)? as usize;
    let mut best: Option<(u8, usize)> = None;
    for i in 0..n {
        let rec = at + 4 + i * 8;
        let platform = u16_at(d, rec)?;
        let encoding = u16_at(d, rec + 2)?;
        let off = u32_at(d, rec + 4)? as usize;
        if off >= len {
            return Err(damaged());
        }
        let rank = match (platform, encoding) {
            (3, 10) => 3,
            (0, 4) | (0, 6) => 3,
            (3, 1) => 2,
            (0, _) => 1,
            _ => 0,
        };
        if rank > 0 && best.is_none_or(|b| rank > b.0) {
            best = Some((rank, at + off));
        }
    }
    let (_, sub) =
        best.ok_or_else(|| Error::Invalid("the font has no Unicode character map".into()))?;
    match u16_at(d, sub)? {
        4 => {
            let segx2 = u16_at(d, sub + 6)? as usize;
            let segs_n = segx2 / 2;
            let ends = sub + 14;
            let starts = ends + segx2 + 2;
            let deltas = starts + segx2;
            let ranges = deltas + segx2;
            let mut segs = Vec::with_capacity(segs_n);
            for i in 0..segs_n {
                segs.push((
                    u16_at(d, ends + 2 * i)?,
                    u16_at(d, starts + 2 * i)?,
                    i16_at(d, deltas + 2 * i)?,
                    u16_at(d, ranges + 2 * i)?,
                    ranges + 2 * i,
                ));
            }
            if segs.windows(2).any(|w| w[0].0 > w[1].0) {
                return Err(damaged());
            }
            Ok(Cmap::Segments { segs })
        }
        12 => {
            let groups_n = u32_at(d, sub + 12)? as usize;
            if groups_n > d.len() / 12 {
                return Err(damaged());
            }
            let mut groups = Vec::with_capacity(groups_n);
            for i in 0..groups_n {
                let g = sub + 16 + i * 12;
                let (a, b, s) = (u32_at(d, g)?, u32_at(d, g + 4)?, u32_at(d, g + 8)?);
                if a > b {
                    return Err(damaged());
                }
                groups.push((a, b, s));
            }
            if groups.windows(2).any(|w| w[0].1 >= w[1].0) {
                return Err(damaged());
            }
            Ok(Cmap::Groups(groups))
        }
        _ => Err(Error::Invalid(
            "the font's character map is of an unknown kind".into(),
        )),
    }
}

/// The checksum of a table (big-endian words, the last one padded).
fn checksum(b: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in b.chunks(4) {
        let mut w = [0u8; 4];
        w[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(w));
    }
    sum
}

/// A TrueType file from tables sorted by tag; head's checkSumAdjustment is
/// set so the whole file sums to 0xB1B0AFBA.
fn write_font(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let n = tables.len() as u16;
    let mut pow = 1u16;
    let mut log = 0u16;
    while pow * 2 <= n {
        pow *= 2;
        log += 1;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&(pow * 16).to_be_bytes());
    out.extend_from_slice(&log.to_be_bytes());
    out.extend_from_slice(&(n * 16 - pow * 16).to_be_bytes());
    let mut at = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, t) in tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(t).to_be_bytes());
        out.extend_from_slice(&(at as u32).to_be_bytes());
        out.extend_from_slice(&(t.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_at = Some(at);
        }
        at += t.len().div_ceil(4) * 4;
    }
    for (_, t) in tables {
        out.extend_from_slice(t);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    if let Some(h) = head_at {
        let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[h + 8..h + 12].copy_from_slice(&adj.to_be_bytes());
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn system_font() -> Option<Vec<u8>> {
        let root = std::env::var("SystemRoot").ok()?;
        std::fs::read(std::path::Path::new(&root).join("Fonts").join("arial.ttf")).ok()
    }

    #[test]
    fn reads_a_system_font() {
        let Some(data) = system_font() else { return };
        let f = Font::parse(data).unwrap();
        assert!(f.has_outlines());
        let a = f.glyph('A');
        assert_ne!(a, 0);
        assert_ne!(f.glyph('é'), 0);
        assert_eq!(f.glyph('\u{10FFFF}'), 0);
        // "i" is narrower than "W" in a proportional font.
        assert!(f.width("i", 10.0) < f.width("W", 10.0));
        assert!((f.width("", 10.0)).abs() < 1e-9);
    }

    #[test]
    fn damaged_fonts_are_refused_not_trusted() {
        assert!(Font::parse(Vec::new()).is_err());
        assert!(Font::parse(b"OTTO\0\0\0\0".to_vec()).is_err());
        assert!(Font::parse(vec![0, 1, 0, 0, 0, 200]).is_err());
        let Some(data) = system_font() else { return };
        // Cut short and bit-flipped copies never panic.
        for cut in [12, 100, 1000, data.len() / 2] {
            let _ = Font::parse(data[..cut].to_vec());
        }
        let mut x: u64 = 0x1234_5678;
        for _ in 0..200 {
            let mut d = data.clone();
            for _ in 0..8 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let i = (x as usize) % 4096.min(d.len());
                d[i] ^= (x >> 32) as u8;
            }
            if let Ok(f) = Font::parse(d) {
                for c in ['a', 'Z', '€', '日', '\u{1F600}'] {
                    let g = f.glyph(c);
                    let _ = f.advance(g);
                }
            }
        }
    }

    // ---- collections and subsets (gap #7) ----

    /// A one-face collection around a font file (its table offsets moved
    /// past the collection header).
    fn as_ttc(font: &[u8]) -> Vec<u8> {
        let mut out = b"ttcf".to_vec();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&16u32.to_be_bytes());
        let mut f = font.to_vec();
        let n = u16::from_be_bytes([f[4], f[5]]) as usize;
        for i in 0..n {
            let at = 12 + i * 16 + 8;
            let off = u32::from_be_bytes([f[at], f[at + 1], f[at + 2], f[at + 3]]) + 16;
            f[at..at + 4].copy_from_slice(&off.to_be_bytes());
        }
        out.extend_from_slice(&f);
        out
    }

    fn table_at(d: &[u8], tag: &[u8; 4]) -> (usize, usize) {
        let n = u16::from_be_bytes([d[4], d[5]]) as usize;
        (0..n)
            .map(|i| 12 + i * 16)
            .find(|&r| &d[r..r + 4] == tag)
            .map(|r| {
                (
                    u32::from_be_bytes([d[r + 8], d[r + 9], d[r + 10], d[r + 11]]) as usize,
                    u32::from_be_bytes([d[r + 12], d[r + 13], d[r + 14], d[r + 15]]) as usize,
                )
            })
            .unwrap()
    }

    #[test]
    fn a_collection_reads_as_its_first_face() {
        let Some(data) = system_font() else { return };
        let a = Font::parse(data.clone()).unwrap();
        let b = Font::parse(as_ttc(&data)).unwrap();
        for c in ['A', 'é', '€', 'z'] {
            assert_eq!(a.glyph(c), b.glyph(c));
            assert_eq!(a.advance(a.glyph(c)), b.advance(b.glyph(c)));
        }
        assert_eq!(
            (a.units_per_em, a.num_glyphs()),
            (b.units_per_em, b.num_glyphs())
        );
        // A real one, when Windows has it.
        if let Some(root) = std::env::var_os("SystemRoot") {
            if let Ok(d) = std::fs::read(std::path::Path::new(&root).join("Fonts").join("msyh.ttc"))
            {
                let f = Font::parse(d).unwrap();
                assert_ne!(f.glyph('王'), 0);
            }
        }
    }

    #[test]
    fn damaged_collection_headers_are_refused() {
        let Some(data) = system_font() else { return };
        let good = as_ttc(&data);
        let with = |at: usize, v: u32| {
            let mut d = good.clone();
            d[at..at + 4].copy_from_slice(&v.to_be_bytes());
            d
        };
        // Version 3; no faces; a face past the end; a face that is a collection.
        assert!(Font::parse(with(4, 0x0003_0000)).is_err());
        assert!(Font::parse(with(8, 0)).is_err());
        assert!(Font::parse(with(12, good.len() as u32 - 4)).is_err());
        assert!(Font::parse(with(12, u32::MAX)).is_err());
        assert!(Font::parse(with(12, 0)).is_err());
        assert!(Font::parse(b"ttcf".to_vec()).is_err());
        // Random damage in the header and the table directory never panics.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..300 {
            let mut d = good.clone();
            for _ in 0..4 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let i = (x as usize) % 400;
                d[i] ^= (x >> 40) as u8;
            }
            if let Ok(f) = Font::parse(d) {
                let g = f.glyph('A');
                let _ = f.subset(&[g]);
            }
        }
    }

    #[test]
    fn a_subset_keeps_the_used_glyphs_exactly_and_their_numbers() {
        let Some(data) = system_font() else { return };
        let f = Font::parse(data.clone()).unwrap();
        let used: Vec<u16> = "Hello, wörld! 123 é".chars().map(|c| f.glyph(c)).collect();
        let sub = f.subset(&used).unwrap();
        // The whole file sums to the magic number (checkSumAdjustment), and each table to its checksum.
        assert_eq!(checksum(&sub), 0xB1B0_AFBA);
        let n = u16::from_be_bytes([sub[4], sub[5]]) as usize;
        for i in 0..n {
            let r = 12 + i * 16;
            if &sub[r..r + 4] == b"head" {
                continue;
            }
            let (at, len) = table_at(&sub, sub[r..r + 4].try_into().unwrap());
            assert_eq!(
                checksum(&sub[at..at + len]),
                u32::from_be_bytes(sub[r + 4..r + 8].try_into().unwrap())
            );
        }
        let s = Font::parse(sub.clone()).unwrap();
        assert_eq!(s.num_glyphs(), f.num_glyphs(), "glyph numbers kept");
        let keep = f.glyph_closure(&used).unwrap();
        for g in 0..f.num_glyphs() {
            let want = if keep[g as usize] {
                f.glyph_range(g).map(|r| f.data[r].to_vec())
            } else {
                None
            };
            let got = s.glyph_range(g).map(|r| s.data[r].to_vec());
            match (want, got) {
                (None, None) => {}
                // The same outline, then at most 3 bytes of padding.
                (Some(w), Some(g2)) => {
                    assert_eq!(&g2[..w.len()], &w[..], "glyph {g}");
                    assert!(g2.len() - w.len() < 4 && g2[w.len()..].iter().all(|&b| b == 0));
                }
                (w, g2) => panic!(
                    "glyph {g}: {} kept, {} in the subset",
                    w.is_some(),
                    g2.is_some()
                ),
            }
            assert_eq!(s.advance(g), f.advance(g));
        }
        // Reached by number: the subset's character map maps nothing.
        assert_eq!(s.glyph('H'), 0);
        // Small: far under the whole font.
        assert!(
            sub.len() * 10 < data.len(),
            "{} of {}",
            sub.len(),
            data.len()
        );
    }

    #[test]
    fn a_cjk_subset_is_a_small_part_of_the_font() {
        let Some(root) = std::env::var_os("SystemRoot") else {
            return;
        };
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("Fonts").join("msyh.ttc"))
        else {
            return;
        };
        let full = d.len();
        let f = Font::parse(d).unwrap();
        let used: Vec<u16> = (0x4E00u32..)
            .filter_map(char::from_u32)
            .map(|c| f.glyph(c))
            .filter(|&g| g != 0)
            .take(200)
            .collect();
        let sub = f.subset(&used).unwrap();
        assert!(sub.len() * 20 < full, "{} of {full}", sub.len());
        let s = Font::parse(sub).unwrap();
        let g = f.glyph('一');
        assert!(
            s.glyph_range(g).map(|r| r.len()).unwrap_or(0)
                >= f.glyph_range(g).map(|r| r.len()).unwrap_or(0)
        );
        assert_eq!(s.advance(g), f.advance(g));
    }

    /// The composite records of glyph `g`, as (flags, component) byte offsets in the file.
    /// A system font with one composite glyph made of itself (a damaged font a
    /// subset must refuse), and that glyph.
    pub(crate) fn looped_font() -> Option<(Font, u16)> {
        let data = system_font()?;
        let (g, rec) = composite_of(&Font::parse(data.clone()).ok()?)?;
        let mut d = data;
        d[rec + 2..rec + 4].copy_from_slice(&g.to_be_bytes());
        Some((Font::parse(d).ok()?, g))
    }

    fn composite_of(f: &Font) -> Option<(u16, usize)> {
        (0..f.num_glyphs()).find_map(|g| {
            let r = f.glyph_range(g)?;
            (r.len() >= 14 && i16_at(&f.data, r.start).ok()? < 0).then_some((g, r.start + 10))
        })
    }

    #[test]
    fn composite_glyphs_bring_their_parts_and_loops_are_refused() {
        let Some(data) = system_font() else { return };
        let f = Font::parse(data.clone()).unwrap();
        let Some((g, rec)) = composite_of(&f) else {
            return;
        };
        let parts = f.components(g).unwrap();
        assert!(!parts.is_empty());
        let keep = f.glyph_closure(&[g]).unwrap();
        assert!(keep[0] && keep[g as usize] && parts.iter().all(|&p| keep[p as usize]));
        // A composite made of itself is refused, not followed.
        let mut looped = data.clone();
        looped[rec + 2..rec + 4].copy_from_slice(&g.to_be_bytes());
        let lf = Font::parse(looped).unwrap();
        assert!(lf.glyph_closure(&[g]).is_err());
        assert!(lf.subset(&[g]).is_err());
        // A part the font does not have is refused.
        let mut beyond = data.clone();
        beyond[rec + 2..rec + 4].copy_from_slice(&f.num_glyphs().to_be_bytes());
        assert!(Font::parse(beyond).unwrap().subset(&[g]).is_err());
        // Random damage in composite records and loca never panics.
        let (loca, loca_len) = table_at(&data, b"loca");
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        for round in 0..300 {
            let mut d = data.clone();
            for _ in 0..3 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let i = if round % 2 == 0 {
                    rec + (x as usize) % 16
                } else {
                    loca + (x as usize) % loca_len
                };
                d[i] ^= (x >> 40) as u8;
            }
            if let Ok(df) = Font::parse(d) {
                let _ = df.subset(&[g, df.glyph('A'), df.glyph('é')]);
            }
        }
    }

    #[test]
    fn a_font_that_asks_not_to_be_subset_is_not() {
        let Some(mut data) = system_font() else {
            return;
        };
        let (os2, _) = table_at(&data, b"OS/2");
        data[os2 + 8..os2 + 10].copy_from_slice(&0x0100u16.to_be_bytes());
        let f = Font::parse(data).unwrap();
        assert!(f.no_subsetting());
        assert!(f.subset(&[f.glyph('A')]).is_err());
    }
}
