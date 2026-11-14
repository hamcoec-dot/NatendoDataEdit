//! Just enough TrueType reading to embed a font in a PDF: character to
//! glyph lookup (cmap formats 4 and 12), glyph widths, the font's box and
//! whether its licence allows embedding. Every offset is checked against
//! the file; a damaged font is refused, never trusted.

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
}

impl Font {
    /// Read a font file's tables. Fonts whose licence does not allow
    /// embedding, or that are not TrueType outlines, are refused.
    pub fn parse(data: Vec<u8>) -> Result<Font, Error> {
        if data.len() > MAX_FONT_BYTES {
            return Err(Error::Invalid("the font file is too large".into()));
        }
        let d = &data;
        let version = u32_at(d, 0)?;
        if version != 0x0001_0000 && version != u32::from_be_bytes(*b"true") {
            return Err(Error::Invalid("not a TrueType font".into()));
        }
        let n = u16_at(d, 4)? as usize;
        let table = |tag: &[u8; 4]| -> Result<Option<(usize, usize)>, Error> {
            for i in 0..n {
                let rec = 12 + i * 16;
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
        if let Some((os2, len)) = table(b"OS/2")? {
            if len >= 10 {
                let fs_type = u16_at(d, os2 + 8)?;
                // Restricted licence, or bitmap embedding only.
                if fs_type & 0x000F == 0x0002 || fs_type & 0x0200 != 0 {
                    return Err(Error::Invalid(
                        "the font's licence does not allow embedding".into(),
                    ));
                }
            }
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
            data,
        })
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

#[cfg(test)]
mod tests {
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
}
