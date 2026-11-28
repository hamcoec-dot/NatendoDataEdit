//! PDF export written here, streamed page by page: a title, the column
//! titles repeated on each page, one line per cell (cut with "…" when too
//! long), numbers aligned right, grid lines and "Page N of M". Columns that
//! do not fit the width go on further pages (down, then across), or the
//! text shrinks to fit. A Windows TrueType font is embedded so every
//! language prints; without one the built-in Helvetica (Latin only) is used.

use std::borrow::Cow;
use std::io::Write;
use std::ops::Range;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::export::{ExportOptions, ExportReport};
use crate::ops::{check_cols, Cols};
use crate::ttf::{Font, MAX_FONT_BYTES};
use crate::value::ReadOptions;
use crate::Error;

/// Most pages in one PDF.
pub const MAX_PAGES: u64 = 1_000_000;
/// Rows looked at to size the columns.
const SAMPLE_ROWS: u64 = 2_000;
/// Characters of a cell considered for printing.
const CELL_CHARS: usize = 1_000;
/// Smallest text size fitting to the width may use, in points.
pub const MIN_FONT: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Paper {
    #[default]
    Letter,
    A4,
    Legal,
    A3,
}

impl Paper {
    /// Width and height in points (portrait).
    pub fn size(self) -> (f64, f64) {
        match self {
            Paper::Letter => (612.0, 792.0),
            Paper::A4 => (595.28, 841.89),
            Paper::Legal => (612.0, 1008.0),
            Paper::A3 => (841.89, 1190.55),
        }
    }
}

/// Page setup for PDF export (and the print screen).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageSetup {
    pub paper: Paper,
    pub landscape: bool,
    /// Margin on every side, in millimetres.
    pub margin_mm: f64,
    /// Text size in points.
    pub font_size: f64,
    /// Shrink the text so all columns fit the page width.
    pub fit_width: bool,
    /// Shrink the text so everything fits one page (width and height),
    /// not below [`MIN_FONT`].
    pub fit_page: bool,
    pub grid: bool,
    /// Column titles on every page (else on the first only).
    pub repeat_header: bool,
    /// Printed at the top of each page.
    pub title: String,
    pub page_numbers: bool,
    /// Shown rows to print: (first, count); `None`: all.
    pub rows: Option<(u64, u64)>,
    /// Logical columns to print, in order; `None`: all.
    pub cols: Option<Vec<u32>>,
}

impl Default for PageSetup {
    fn default() -> Self {
        PageSetup {
            paper: Paper::Letter,
            landscape: false,
            margin_mm: 12.7,
            font_size: 9.0,
            fit_width: true,
            fit_page: false,
            grid: true,
            repeat_header: true,
            title: String::new(),
            page_numbers: true,
            rows: None,
            cols: None,
        }
    }
}

// ---- fonts ----

pub(crate) enum Face {
    Embedded { font: Box<Font>, name: String },
    Helvetica,
}

/// Rough Helvetica widths (thousandths of an em), only for when no system
/// font can be read.
fn helvetica_width(c: char) -> u32 {
    match c {
        'i' | 'j' | 'l' => 222,
        ' ' | '!' | ',' | '.' | '/' | ':' | ';' | 'I' | 'f' | 't' | '[' | ']' | '\\' => 278,
        '\'' | '|' => 230,
        'r' | '(' | ')' | '-' | '`' => 333,
        'c' | 'k' | 's' | 'v' | 'x' | 'y' | 'z' | 'J' => 500,
        'm' | 'M' => 833,
        'w' | 'C' | 'D' | 'G' | 'H' | 'N' | 'O' | 'Q' | 'R' | 'U' => 722,
        'W' => 944,
        'E' | 'P' | 'S' | 'T' | 'Z' | 'F' | 'L' => 611,
        '%' => 889,
        '@' => 1015,
        'A'..='Z' => 667,
        _ => 556,
    }
}

/// The byte Helvetica (WinAnsi) uses for a character, or `?`.
fn win_ansi(c: char) -> u8 {
    match c as u32 {
        0x20..=0x7E | 0xA0..=0xFF => c as u32 as u8,
        _ => match c {
            '€' => 0x80,
            '‚' => 0x82,
            '„' => 0x84,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '™' => 0x99,
            _ => b'?',
        },
    }
}

/// Font files tried, in order.
fn font_candidates() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for var in ["SystemRoot", "WINDIR"] {
        if let Ok(root) = std::env::var(var) {
            let dir = std::path::Path::new(&root).join("Fonts");
            for f in ["segoeui.ttf", "arial.ttf", "calibri.ttf", "tahoma.ttf"] {
                out.push(dir.join(f));
            }
            break;
        }
    }
    for p in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
    ] {
        out.push(p.into());
    }
    out
}

pub(crate) fn load_face() -> Face {
    for p in font_candidates() {
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_FONT_BYTES as u64 {
            continue;
        }
        let Ok(data) = std::fs::read(&p) else {
            continue;
        };
        if let Ok(font) = Font::parse(data) {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let name: String = stem.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            return Face::Embedded {
                font: Box::new(font),
                name: if name.is_empty() {
                    "Embedded".into()
                } else {
                    name
                },
            };
        }
    }
    Face::Helvetica
}

// ---- fallback fonts for Chinese, Japanese and Korean (gap #7) ----

/// The fallback fonts, by script; each has its own PDF font slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cjk {
    Japanese = 0,
    Korean = 1,
    Chinese = 2,
}

const CJK_ALL: [Cjk; 3] = [Cjk::Japanese, Cjk::Korean, Cjk::Chinese];

/// Object numbers: the main font is 4 to 8; fallback slot `k` (/F2, /F3,
/// /F4) is `9 + 5k` to `13 + 5k`; pages start after them.
pub(crate) const FIRST_PAGE_OBJ: usize = 24;

fn slot_base(k: Cjk) -> usize {
    9 + 5 * k as usize
}

/// Which Chinese characters alone (no kana, no Hangul in the text) are
/// drawn with: the Windows display language's font, Simplified otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HanFont {
    Simplified,
    Traditional,
    Japanese,
    Korean,
}

/// Han: the unified ideographs and Extension A, the supplementary planes
/// (Extension B onwards), compatibility ideographs, CJK symbols and
/// punctuation, and fullwidth forms.
pub(crate) fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x2_0000..=0x3_FFFF | 0xF900..=0xFAFF
        | 0x3000..=0x303F | 0xFF00..=0xFFEF)
        && !is_kana(c)
}

/// Kana: hiragana, katakana, their phonetic extensions, halfwidth katakana.
pub(crate) fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF65..=0xFF9F)
}

/// Hangul: syllables, jamo, compatibility jamo, jamo extended A.
pub(crate) fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F)
}

pub(crate) fn is_cjk(c: char) -> bool {
    is_han(c) || is_kana(c) || is_hangul(c)
}

/// The fallback for a piece of text: any kana gives Japanese; otherwise any
/// Hangul gives Korean; otherwise (Han, CJK punctuation, fullwidth forms)
/// the font for the display language.
pub(crate) fn cjk_for(text: &str, han: HanFont) -> Cjk {
    if text.chars().any(is_kana) {
        return Cjk::Japanese;
    }
    if text.chars().any(is_hangul) {
        return Cjk::Korean;
    }
    match han {
        HanFont::Japanese => Cjk::Japanese,
        HanFont::Korean => Cjk::Korean,
        HanFont::Simplified | HanFont::Traditional => Cjk::Chinese,
    }
}

/// The Han font for a Windows display language (a LANGID).
pub(crate) fn han_for_langid(id: u16) -> HanFont {
    match (id & 0x3FF, id) {
        (0x11, _) => HanFont::Japanese,
        (0x12, _) => HanFont::Korean,
        // Chinese: Taiwan, Hong Kong and Macau use Traditional characters.
        (0x04, 0x0404 | 0x0C04 | 0x1404) => HanFont::Traditional,
        _ => HanFont::Simplified,
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn display_han() -> HanFont {
    // SAFETY: takes nothing and returns a number.
    han_for_langid(unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() })
}

#[cfg(not(windows))]
fn display_han() -> HanFont {
    HanFont::Simplified
}

/// The font files tried for a script, in order (in the Windows Fonts folder).
fn fallback_files(k: Cjk, han: HanFont) -> &'static [&'static str] {
    match k {
        Cjk::Japanese => &["YuGothM.ttc", "msgothic.ttc", "meiryo.ttc"],
        Cjk::Korean => &["malgun.ttf", "gulim.ttc"],
        Cjk::Chinese if han == HanFont::Traditional => &["msjh.ttc", "mingliu.ttc"],
        Cjk::Chinese => &["msyh.ttc", "simsun.ttc"],
    }
}

/// The Windows Fonts folder (the only place fallback fonts are read from:
/// never the file's folder or the working folder).
fn windows_fonts_dir() -> Option<std::path::PathBuf> {
    for var in ["SystemRoot", "WINDIR"] {
        if let Ok(root) = std::env::var(var) {
            return Some(std::path::Path::new(&root).join("Fonts"));
        }
    }
    None
}

/// A fallback font, read and checked.
pub(crate) struct Fallback {
    pub(crate) font: Font,
    name: String,
}

/// The first of `files` in `dir` that can be used: present, at most
/// [`MAX_FONT_BYTES`], TrueType outlines, embeddable and allowed to be
/// subset. Anything else moves on to the next (nothing is logged).
fn first_usable(dir: &std::path::Path, files: &[&str]) -> Option<Fallback> {
    for f in files {
        let p = dir.join(f);
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_FONT_BYTES as u64 {
            continue;
        }
        let Ok(data) = std::fs::read(&p) else {
            continue;
        };
        let Ok(font) = Font::parse(data) else {
            continue;
        };
        if font.no_subsetting() || !font.has_outlines() {
            continue;
        }
        let name: String = f
            .split('.')
            .next()
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        return Some(Fallback {
            font,
            name: if name.is_empty() { "CJK".into() } else { name },
        });
    }
    None
}

/// The fonts of one run (an export, a set of labels or letters): the main
/// font, and the fallbacks, each read only when a character first needs it
/// and then kept for the run. Which glyphs a PDF uses is kept apart, in
/// its own [`Usage`], so each PDF embeds only its own.
pub(crate) struct Fonts {
    pub(crate) main: Face,
    han: HanFont,
    dir: Option<std::path::PathBuf>,
    slots: [std::cell::OnceCell<Option<Fallback>>; 3],
    /// Fallback files read (tests: a Latin-only run reads none).
    pub(crate) reads: std::cell::Cell<u32>,
}

impl Fonts {
    pub(crate) fn load() -> Fonts {
        #[cfg(test)]
        if let Some((han, dir)) = tests::FONTS.with(|f| f.borrow().clone()) {
            return Fonts::with(load_face(), han, dir);
        }
        Fonts::with(load_face(), display_han(), windows_fonts_dir())
    }

    pub(crate) fn with(main: Face, han: HanFont, dir: Option<std::path::PathBuf>) -> Fonts {
        Fonts {
            main,
            han,
            dir,
            slots: Default::default(),
            reads: std::cell::Cell::new(0),
        }
    }

    fn fallback(&self, k: Cjk) -> Option<&Fallback> {
        self.slots[k as usize]
            .get_or_init(|| {
                let dir = self.dir.as_deref()?;
                self.reads.set(self.reads.get() + 1);
                first_usable(dir, fallback_files(k, self.han))
            })
            .as_ref()
    }

    /// The fallback read for slot `k` so far (without reading it).
    fn loaded(&self, k: Cjk) -> Option<&Fallback> {
        self.slots[k as usize].get().and_then(Option::as_ref)
    }

    /// Whether the main font draws `c` (glyph lookups only: no allocation).
    #[inline]
    fn main_has(&self, c: char) -> bool {
        match &self.main {
            Face::Embedded { font, .. } => font.glyph(c) != 0,
            Face::Helvetica => c == '?' || win_ansi(c) != b'?',
        }
    }

    /// Where `c` of `text` is drawn: `None` for the main font, else the
    /// fallback slot and its glyph. `slot` caches `text`'s script choice.
    fn place(&self, c: char, text: &str, slot: &mut Option<Cjk>) -> Option<(Cjk, u16)> {
        if self.main_has(c) || !is_cjk(c) {
            return None;
        }
        let k = *slot.get_or_insert_with(|| cjk_for(text, self.han));
        if let Some(g) = self
            .fallback(k)
            .map(|f| f.font.glyph(c))
            .filter(|&g| g != 0)
        {
            return Some((k, g));
        }
        // The text's font lacks it (Hangul in a Japanese name): the font of
        // the character's own script, then the others.
        let own = if is_kana(c) {
            Cjk::Japanese
        } else if is_hangul(c) {
            Cjk::Korean
        } else {
            k
        };
        std::iter::once(own)
            .chain(CJK_ALL)
            .filter(|&o| o != k)
            .find_map(|o| match self.fallback(o)?.font.glyph(c) {
                0 => None,
                g => Some((o, g)),
            })
    }

    pub(crate) fn char_width(&self, c: char, text: &str, slot: &mut Option<Cjk>, size: f64) -> f64 {
        match self.place(c, text, slot) {
            None => self.main.char_width(c, size),
            Some((k, g)) => {
                let f = &self.loaded(k).expect("placed in a loaded fallback").font;
                f.advance(g) as f64 * size / f.units_per_em as f64
            }
        }
    }

    /// Width of text in points at `size`, each character in the font that
    /// draws it.
    pub(crate) fn width(&self, s: &str, size: f64) -> f64 {
        // Text the main font draws: one glyph lookup a character.
        if let Face::Embedded { font, .. } = &self.main {
            let mut units = 0u64;
            let mut plain = true;
            for c in s.chars() {
                let g = font.glyph(c);
                if g == 0 && is_cjk(c) {
                    plain = false;
                    break;
                }
                units += font.advance(g) as u64;
            }
            if plain {
                return units as f64 * size / font.units_per_em as f64;
            }
        } else if !s.chars().any(is_cjk) {
            return self.main.width(s, size);
        }
        let mut slot = None;
        s.chars()
            .map(|c| self.char_width(c, s, &mut slot, size))
            .sum()
    }

    pub(crate) fn ellipsis(&self) -> &'static str {
        self.main.ellipsis()
    }

    /// The tallest line a fallback read so far needs, as a multiple of the
    /// text size (ascent minus descent), if larger than `base`.
    pub(crate) fn line_factor(&self, base: f64) -> f64 {
        CJK_ALL
            .iter()
            .filter_map(|&k| self.loaded(k))
            .map(|f| {
                (f.font.ascent as f64 - f.font.descent as f64) / f.font.units_per_em as f64 * 1.05
            })
            .fold(base, f64::max)
            .min(base * 1.5)
    }

    /// Draw `s` at the current text position: `<glyphs> Tj` runs, switching
    /// to a fallback font (`/F2`..`/F4`) and back to `/F1` as needed. Notes
    /// the glyphs used, and counts characters no font has.
    pub(crate) fn show(&self, s: &str, size: f64, u: &mut Usage, out: &mut Vec<u8>) {
        // Text the main font draws: one pass, one glyph lookup a character.
        match &self.main {
            Face::Embedded { font, .. } => {
                let start = out.len();
                let mut missing = 0u64;
                let mut plain = true;
                out.push(b'<');
                for c in s.chars() {
                    let g = font.glyph(c);
                    if g == 0 {
                        if is_cjk(c) {
                            plain = false;
                            break;
                        }
                        missing += 1;
                    } else if let Some(x) = u.main.get_mut(g as usize) {
                        if *x == 0 {
                            *x = c as u32;
                        }
                    }
                    let _ = write!(out, "{g:04X}");
                }
                if plain {
                    out.extend_from_slice(b"> Tj\n");
                    u.missing += missing;
                    return;
                }
                out.truncate(start);
            }
            Face::Helvetica => {
                if !s.chars().any(is_cjk) {
                    u.missing += s.chars().filter(|&c| !self.main_has(c)).count() as u64;
                    self.main.encode(s, &mut u.main, out);
                    out.extend_from_slice(b" Tj\n");
                    return;
                }
            }
        }
        let mut slot = None;
        // The font of the run being written: None is /F1.
        let mut cur: Option<Cjk> = None;
        let mut run = String::new();
        let flush = |cur: Option<Cjk>, run: &mut String, u: &mut Usage, out: &mut Vec<u8>| {
            if run.is_empty() {
                return;
            }
            match cur {
                None => self.main.encode(run, &mut u.main, out),
                Some(k) => {
                    let f = &self.loaded(k).expect("placed in a loaded fallback").font;
                    let used = u.fallback(k);
                    out.push(b'<');
                    for c in run.chars() {
                        let g = f.glyph(c);
                        if let Some(x) = used.get_mut(g as usize) {
                            if *x == 0 {
                                *x = c as u32;
                            }
                        }
                        let _ = write!(out, "{g:04X}");
                    }
                    out.push(b'>');
                }
            }
            out.extend_from_slice(b" Tj ");
            run.clear();
        };
        for c in s.chars() {
            let here = self.place(c, s, &mut slot).map(|(k, _)| k);
            if here.is_none() && !self.main_has(c) {
                u.missing += 1;
                if is_cjk(c) {
                    u.missing_cjk += 1;
                }
            }
            if here != cur {
                flush(cur, &mut run, u, out);
                let n = here.map_or(1, |k| k as usize + 2);
                let _ = write!(out, "/F{n} {} Tf ", num(size));
                if let Some(k) = here {
                    u.page[k as usize] = true;
                }
                cur = here;
            }
            run.push(c);
        }
        flush(cur, &mut run, u, out);
        if cur.is_some() {
            let _ = write!(out, "/F1 {} Tf", num(size));
        }
        out.push(b'\n');
    }
}

/// The glyphs one PDF uses, per font, and the characters it could not draw.
pub(crate) struct Usage {
    pub(crate) main: Vec<u32>,
    fallbacks: [Vec<u32>; 3],
    /// Fallback fonts the page being written uses.
    page: [bool; 3],
    /// Characters no font had (drawn as the empty box), and how many of
    /// them are Chinese, Japanese or Korean.
    pub(crate) missing: u64,
    pub(crate) missing_cjk: u64,
}

impl Usage {
    pub(crate) fn new(main: Vec<u32>) -> Usage {
        Usage {
            main,
            fallbacks: Default::default(),
            page: [false; 3],
            missing: 0,
            missing_cjk: 0,
        }
    }

    fn fallback(&mut self, k: Cjk) -> &mut Vec<u32> {
        let v = &mut self.fallbacks[k as usize];
        if v.is_empty() {
            *v = vec![0; u16::MAX as usize + 1];
        }
        v
    }

    /// The page's font resources (`/F1` and the fallbacks it used), and
    /// a fresh start for the next page.
    pub(crate) fn page_fonts(&mut self) -> String {
        let mut s = String::from("/F1 4 0 R");
        for k in CJK_ALL {
            if std::mem::take(&mut self.page[k as usize]) {
                let _ = std::fmt::Write::write_fmt(
                    &mut s,
                    format_args!(" /F{} {} 0 R", k as usize + 2, slot_base(k)),
                );
            }
        }
        s
    }
}

/// A six-capital tag for a subset (as the PDF spec asks: `ABCDEF+Name`),
/// from the glyphs it holds.
fn subset_tag(glyphs: &[u16]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for g in glyphs {
        h ^= u64::from(*g);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| (b'A' + ((h >> (i * 5)) % 26) as u8) as char)
        .collect()
}

/// The ToUnicode CMap for glyph → character pairs (characters past U+FFFF
/// as UTF-16 surrogate pairs).
fn to_unicode(pairs: &[(usize, u32)]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for chunk in pairs.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for &(g, ch) in chunk {
            let s: String = char::from_u32(ch)
                .unwrap_or('?')
                .encode_utf16(&mut [0; 2])
                .iter()
                .map(|u| format!("{u:04X}"))
                .collect();
            cmap.push_str(&format!("<{g:04X}> <{s}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

/// Write fallback slot `k`: a subset of its font holding the glyphs `used`
/// names, or `null` objects when the PDF never used it. A subset the font
/// refuses (damaged composites) keeps only the empty box, and the
/// characters it would have drawn count as missing.
fn write_fallback<W: Write>(
    o: &mut Out<'_, W>,
    fonts: &Fonts,
    k: Cjk,
    used: &[u32],
    missing: &mut u64,
) -> Result<(), Error> {
    let base = slot_base(k);
    let pairs: Vec<(usize, u32)> = used
        .iter()
        .enumerate()
        .filter(|(_, &c)| c != 0)
        .map(|(g, &c)| (g, c))
        .collect();
    let Some(f) = fonts.loaded(k).filter(|_| !pairs.is_empty()) else {
        for id in base..base + 5 {
            o.obj(id, b"null")?;
        }
        return Ok(());
    };
    let font = &f.font;
    let glyphs: Vec<u16> = pairs.iter().map(|&(g, _)| g as u16).collect();
    let file = match font.subset(&glyphs) {
        Ok(b) => b,
        Err(_) => {
            *missing += pairs.len() as u64;
            font.subset(&[])?
        }
    };
    let name = format!("{}+{}", subset_tag(&glyphs), f.name);
    let scale = 1000.0 / font.units_per_em as f64;
    let mut w = String::new();
    for &(g, _) in &pairs {
        let adv = (font.advance(g as u16) as f64 * scale).round();
        w.push_str(&format!("{g} [{adv}] "));
    }
    let b = font.bbox.map(|v| (v as f64 * scale).round());
    o.obj(
        base,
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>",
            base + 1,
            base + 4
        )
        .as_bytes(),
    )?;
    o.obj(
        base + 1,
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {} 0 R /DW 1000 /W [{w}] /CIDToGIDMap /Identity >>",
            base + 2
        )
        .as_bytes(),
    )?;
    o.obj(
        base + 2,
        format!(
            "<< /Type /FontDescriptor /FontName /{name} /Flags 4 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {} 0 R >>",
            b[0], b[1], b[2], b[3],
            (font.ascent as f64 * scale).round(),
            (font.descent as f64 * scale).round(),
            (font.ascent as f64 * scale * 0.7).round(),
            base + 3
        )
        .as_bytes(),
    )?;
    o.stream(base + 3, &file, &format!(" /Length1 {}", file.len()))?;
    o.stream(base + 4, to_unicode(&pairs).as_bytes(), "")?;
    Ok(())
}

impl Face {
    pub(crate) fn width(&self, s: &str, size: f64) -> f64 {
        match self {
            Face::Embedded { font, .. } => font.width(s, size),
            Face::Helvetica => {
                s.chars().map(helvetica_width).map(u64::from).sum::<u64>() as f64 * size / 1000.0
            }
        }
    }

    fn char_width(&self, c: char, size: f64) -> f64 {
        match self {
            Face::Embedded { font, .. } => {
                font.advance(font.glyph(c)) as f64 * size / font.units_per_em as f64
            }
            Face::Helvetica => helvetica_width(c) as f64 * size / 1000.0,
        }
    }

    /// Write the text as a PDF string for `Tj`, noting glyphs used.
    pub(crate) fn encode(&self, s: &str, used: &mut [u32], out: &mut Vec<u8>) {
        match self {
            Face::Embedded { font, .. } => {
                out.push(b'<');
                for c in s.chars() {
                    let g = font.glyph(c);
                    if let Some(u) = used.get_mut(g as usize) {
                        if *u == 0 {
                            *u = c as u32;
                        }
                    }
                    let _ = write!(out, "{g:04X}");
                }
                out.push(b'>');
            }
            Face::Helvetica => {
                out.push(b'(');
                for c in s.chars() {
                    match win_ansi(c) {
                        b @ (b'(' | b')' | b'\\') => {
                            out.push(b'\\');
                            out.push(b);
                        }
                        b => out.push(b),
                    }
                }
                out.push(b')');
            }
        }
    }

    fn ellipsis(&self) -> &'static str {
        match self {
            Face::Embedded { font, .. } if font.glyph('…') == 0 => "...",
            _ => "…",
        }
    }
}

/// One line of cell text that fits `room` points.
pub(crate) fn fit<'a>(fonts: &Fonts, s: &'a str, room: f64, size: f64) -> Cow<'a, str> {
    let mut t: Cow<str> = if s.chars().count() > CELL_CHARS {
        Cow::Owned(s.chars().take(CELL_CHARS).collect())
    } else {
        Cow::Borrowed(s)
    };
    if t.contains(['\n', '\r', '\t']) {
        t = Cow::Owned(t.replace(['\n', '\r', '\t'], " "));
    }
    // A hair of slack: a column sized to its widest value, then shrunk,
    // must not cut that value over a rounding error.
    if fonts.width(&t, size) <= room + 0.01 {
        return t;
    }
    let ell = fonts.ellipsis();
    let room = room - fonts.width(ell, size);
    let mut w = 0.0;
    let mut out = String::new();
    let mut slot = None;
    for c in t.chars() {
        w += fonts.char_width(c, &t, &mut slot, size);
        if w > room {
            break;
        }
        out.push(c);
    }
    out.push_str(ell);
    Cow::Owned(out)
}

// ---- writing ----

pub(crate) struct Out<'w, W: Write> {
    w: &'w mut W,
    n: u64,
    offsets: Vec<u64>,
}

impl<'w, W: Write> Out<'w, W> {
    /// A PDF's header and its font file (object 7), with room for objects
    /// 1 to 8; pages start at object 9. Also the glyph use table for
    /// [`finish`].
    pub(crate) fn start(w: &'w mut W, face: &Face) -> Result<(Self, Vec<u32>), Error> {
        let mut o = Out {
            w,
            n: 0,
            offsets: vec![0; 9],
        };
        o.raw(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")?;
        let num_glyphs = match face {
            Face::Embedded { font, .. } => {
                o.stream(7, &font.data, &format!(" /Length1 {}", font.data.len()))?;
                u16::MAX as usize + 1
            }
            Face::Helvetica => 0,
        };
        Ok((o, vec![0u32; num_glyphs]))
    }

    /// Bytes written so far.
    pub(crate) fn len(&self) -> u64 {
        self.n
    }

    pub(crate) fn raw(&mut self, b: &[u8]) -> Result<(), Error> {
        self.w.write_all(b)?;
        self.n += b.len() as u64;
        Ok(())
    }

    pub(crate) fn obj(&mut self, id: usize, body: &[u8]) -> Result<(), Error> {
        if self.offsets.len() <= id {
            self.offsets.resize(id + 1, 0);
        }
        self.offsets[id] = self.n;
        self.raw(format!("{id} 0 obj\n").as_bytes())?;
        self.raw(body)?;
        self.raw(b"\nendobj\n")
    }

    /// A Flate-compressed stream object.
    pub(crate) fn stream(&mut self, id: usize, data: &[u8], extra: &str) -> Result<(), Error> {
        let mut z = ZlibEncoder::new(Vec::with_capacity(data.len() / 3), Compression::fast());
        z.write_all(data)?;
        let z = z.finish()?;
        let mut body = format!(
            "<< /Length {} /Filter /FlateDecode{extra} >>\nstream\n",
            z.len()
        )
        .into_bytes();
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        self.obj(id, &body)
    }
}

/// The rest of a PDF after its pages: the fallback fonts it used (objects
/// 9 to 23, subsets), the main font (objects 4 to 8), the page list (2),
/// catalog (1), info (3), and the cross reference table. `u` maps glyphs
/// to the characters drawn with them.
pub(crate) fn finish<W: Write>(
    o: &mut Out<'_, W>,
    fonts: &Fonts,
    u: &mut Usage,
    kids: &str,
    total_pages: u64,
    title_text: &str,
) -> Result<(), Error> {
    for k in CJK_ALL {
        let used = std::mem::take(&mut u.fallbacks[k as usize]);
        write_fallback(o, fonts, k, &used, &mut u.missing)?;
    }
    let used = &u.main;
    match &fonts.main {
        Face::Embedded { font, name } => {
            let scale = 1000.0 / font.units_per_em as f64;
            let mut w = String::new();
            for (g, &ch) in used.iter().enumerate() {
                if ch != 0 {
                    let adv = (font.advance(g as u16) as f64 * scale).round();
                    w.push_str(&format!("{g} [{adv}] "));
                }
            }
            let b = font.bbox.map(|v| (v as f64 * scale).round());
            o.obj(
                4,
                format!(
                    "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 8 0 R >>"
                )
                .as_bytes(),
            )?;
            o.obj(
                5,
                format!(
                    "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 6 0 R /DW 1000 /W [{w}] /CIDToGIDMap /Identity >>"
                )
                .as_bytes(),
            )?;
            o.obj(
                6,
                format!(
                    "<< /Type /FontDescriptor /FontName /{name} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 7 0 R >>",
                    b[0], b[1], b[2], b[3],
                    (font.ascent as f64 * scale).round(),
                    (font.descent as f64 * scale).round(),
                    (font.ascent as f64 * scale * 0.7).round()
                )
                .as_bytes(),
            )?;
            let mut cmap = String::from(
                "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
            );
            let pairs: Vec<(usize, u32)> = used
                .iter()
                .enumerate()
                .filter(|(_, &c)| c != 0)
                .map(|(g, &c)| (g, c))
                .collect();
            for chunk in pairs.chunks(100) {
                cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
                for &(g, ch) in chunk {
                    let s: String = char::from_u32(ch)
                        .unwrap_or('?')
                        .encode_utf16(&mut [0; 2])
                        .iter()
                        .map(|u| format!("{u:04X}"))
                        .collect();
                    cmap.push_str(&format!("<{g:04X}> <{s}>\n"));
                }
                cmap.push_str("endbfchar\n");
            }
            cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
            o.stream(8, cmap.as_bytes(), "")?;
        }
        Face::Helvetica => {
            o.obj(
                4,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
            )?;
            for id in [5, 6, 7, 8] {
                o.obj(id, b"null")?;
            }
        }
    }
    o.obj(
        2,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {total_pages} >>",
            kids.trim_end()
        )
        .as_bytes(),
    )?;
    o.obj(1, b"<< /Type /Catalog /Pages 2 0 R >>")?;
    let title = if title_text.is_empty() {
        String::new()
    } else {
        format!(" /Title {}", pdf_text(title_text))
    };
    o.obj(
        3,
        format!("<< /Producer {}{title} >>", pdf_text("Natendo Data Edit")).as_bytes(),
    )?;
    let xref = o.n;
    let count = o.offsets.len();
    let mut x = format!("xref\n0 {count}\n0000000000 65535 f \n");
    for &off in &o.offsets[1..] {
        x.push_str(&format!("{off:010} 00000 n \n"));
    }
    x.push_str(&format!(
        "trailer\n<< /Size {count} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    o.raw(x.as_bytes())?;
    Ok(())
}

fn pdf_text(s: &str) -> String {
    let mut out = String::from("<FEFF");
    for u in s.encode_utf16() {
        out.push_str(&format!("{u:04X}"));
    }
    out.push('>');
    out
}

pub(crate) fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// Where everything goes on the pages.
struct Layout {
    pw: f64,
    ph: f64,
    margin: f64,
    size: f64,
    row_h: f64,
    pad: f64,
    widths: Vec<f64>,
    /// Column groups (pages across): ranges of `widths`.
    groups: Vec<Range<usize>>,
    /// Row pages: ranges of data rows (offsets from the first data row).
    row_pages: Vec<Range<u64>>,
    title_size: f64,
    title_h: f64,
    footer_h: f64,
}

impl Document {
    /// Write shown rows `rows` (data rows; the column titles are `names`
    /// when `opts.header`) as a PDF.
    pub(crate) fn export_pdf(
        &self,
        out: &mut impl Write,
        opts: &ExportOptions,
        names: &[String],
        rows: Range<u64>,
        progress: &Progress,
    ) -> Result<ExportReport, Error> {
        let page = &opts.page;
        let fonts = Fonts::load();
        let cols: Vec<u32> = match &page.cols {
            Some(c) => {
                if c.is_empty() {
                    return Err(Error::Invalid("choose at least one column to print".into()));
                }
                check_cols(self, c)?;
                c.clone()
            }
            None => (0..self.col_count()).collect(),
        };
        if cols.len() > crate::export::XLSX_MAX_COLS as usize {
            return Err(Error::Invalid("too many columns to print".into()));
        }
        let rows = match page.rows {
            Some((a, n)) => a.max(rows.start)..a.saturating_add(n).min(rows.end),
            None => rows,
        };
        let rows = rows.start..rows.end.max(rows.start);
        let titles: Vec<&str> = cols
            .iter()
            .map(|&c| names.get(c as usize).map_or("", String::as_str))
            .collect();
        let reader = Cols::new(self, &cols);
        let ctx = self.read_ctx(ReadOptions::default());
        let rds: Vec<crate::coltype::Reader> = cols.iter().map(|&c| ctx.reader(c)).collect();
        let l = self.pdf_layout(page, &fonts, opts.header, &titles, &reader, rows.clone())?;
        let total_pages = (l.groups.len() * l.row_pages.len()) as u64;
        progress.start(total_pages);

        let (mut o, used) = Out::start(out, &fonts.main)?;
        let mut u = Usage::new(used);
        let mut kids = String::new();
        let mut buf = Vec::new();
        let mut c: Vec<u8> = Vec::with_capacity(64 << 10);
        let mut next = FIRST_PAGE_OBJ;
        let mut page_no = 0u64;
        let top = l.ph - l.margin - l.title_h;
        for g in &l.groups {
            let gw: f64 = l.widths[g.clone()].iter().sum();
            for (rp, range) in l.row_pages.iter().enumerate() {
                page_no += 1;
                c.clear();
                let header = opts.header && (page.repeat_header || rp == 0);
                let n_rows = range.end - range.start + u64::from(header);
                let table_h = n_rows as f64 * l.row_h;
                let x0 = l.margin;
                // Title row shading.
                if header {
                    let _ = writeln!(
                        c,
                        "0.92 g {} {} {} {} re f",
                        num(x0),
                        num(top - l.row_h),
                        num(gw),
                        num(l.row_h)
                    );
                }
                // Grid.
                let _ = writeln!(c, "0.6 G 0.4 w");
                if page.grid {
                    for k in 0..=n_rows {
                        let y = top - k as f64 * l.row_h;
                        let _ =
                            writeln!(c, "{} {} m {} {} l", num(x0), num(y), num(x0 + gw), num(y));
                    }
                    let mut x = x0;
                    for k in g.clone() {
                        let _ = writeln!(
                            c,
                            "{} {} m {} {} l",
                            num(x),
                            num(top),
                            num(x),
                            num(top - table_h)
                        );
                        x += l.widths[k];
                    }
                    let _ = writeln!(
                        c,
                        "{} {} m {} {} l S",
                        num(x),
                        num(top),
                        num(x),
                        num(top - table_h)
                    );
                } else if header {
                    let y = top - l.row_h;
                    let _ = writeln!(
                        c,
                        "{} {} m {} {} l S",
                        num(x0),
                        num(y),
                        num(x0 + gw),
                        num(y)
                    );
                }
                let _ = writeln!(c, "0 g BT /F1 {} Tf", num(l.size));
                let mut y = top;
                let mut cell =
                    |c: &mut Vec<u8>, text: &str, k: usize, x: f64, y: f64, right: bool| {
                        let room = l.widths[k] - 2.0 * l.pad;
                        let t = fit(&fonts, text, room, l.size);
                        if t.is_empty() {
                            return;
                        }
                        let tx = if right {
                            x + l.widths[k] - l.pad - fonts.width(&t, l.size)
                        } else {
                            x + l.pad
                        };
                        let base = y - (l.row_h + l.size * 0.7) / 2.0;
                        let _ = write!(c, "1 0 0 1 {} {} Tm ", num(tx), num(base));
                        fonts.show(&t, l.size, &mut u, c);
                    };
                if header {
                    let mut x = x0;
                    for k in g.clone() {
                        cell(&mut c, titles[k], k, x, y, false);
                        x += l.widths[k];
                    }
                    y -= l.row_h;
                }
                for off in range.clone() {
                    let r = rows.start + off;
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let mut x = x0;
                    for k in g.clone() {
                        let v = reader.get(&buf, k);
                        let right = !v.is_empty() && rds[k].number(v).is_some();
                        cell(&mut c, v, k, x, y, right);
                        x += l.widths[k];
                    }
                    y -= l.row_h;
                }
                let _ = writeln!(c, "ET");
                if l.title_h > 0.0 {
                    let t = fit(&fonts, &page.title, l.pw - 2.0 * l.margin, l.title_size);
                    let _ = write!(
                        c,
                        "BT /F1 {} Tf 1 0 0 1 {} {} Tm ",
                        num(l.title_size),
                        num(l.margin),
                        num(l.ph - l.margin - l.title_size)
                    );
                    fonts.show(&t, l.title_size, &mut u, &mut c);
                    c.extend_from_slice(b"ET\n");
                }
                if l.footer_h > 0.0 {
                    let t = format!("Page {page_no} of {total_pages}");
                    let w = fonts.width(&t, l.size);
                    let _ = write!(
                        c,
                        "BT /F1 {} Tf 1 0 0 1 {} {} Tm ",
                        num(l.size),
                        num((l.pw - w) / 2.0),
                        num(l.margin)
                    );
                    fonts.show(&t, l.size, &mut u, &mut c);
                    c.extend_from_slice(b"ET\n");
                }
                let (pid, cid) = (next, next + 1);
                next += 2;
                o.stream(cid, &c, "")?;
                o.obj(
                    pid,
                    format!(
                        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << {} >> >> /Contents {cid} 0 R >>",
                        num(l.pw),
                        num(l.ph),
                        u.page_fonts()
                    )
                    .as_bytes(),
                )?;
                kids.push_str(&format!("{pid} 0 R "));
                progress.step(1)?;
            }
        }
        finish(&mut o, &fonts, &mut u, &kids, total_pages, &page.title)?;
        Ok(ExportReport {
            rows: rows.end - rows.start,
            sheets: u32::try_from(total_pages).unwrap_or(u32::MAX),
            bytes: o.n,
            files: Vec::new(),
            missing_chars: u.missing,
            missing_cjk: u.missing_cjk > 0,
        })
    }

    fn pdf_layout(
        &self,
        page: &PageSetup,
        fonts: &Fonts,
        header: bool,
        titles: &[&str],
        reader: &Cols,
        rows: Range<u64>,
    ) -> Result<Layout, Error> {
        let (mut pw, mut ph) = page.paper.size();
        if page.landscape {
            std::mem::swap(&mut pw, &mut ph);
        }
        if !(page.font_size.is_finite() && (MIN_FONT..=72.0).contains(&page.font_size)) {
            return Err(Error::Invalid(format!(
                "the text size must be from {MIN_FONT} to 72 points"
            )));
        }
        if !page.margin_mm.is_finite() || page.margin_mm < 0.0 {
            return Err(Error::Invalid("the margin must be 0 or more".into()));
        }
        let margin = (page.margin_mm * 72.0 / 25.4).min(pw.min(ph) / 4.0);
        let width = pw - 2.0 * margin;
        let fs = page.font_size;
        let pad = 0.35 * fs;
        // Natural widths at the chosen size.
        let mut nat: Vec<f64> = titles
            .iter()
            .map(|t| if header { fonts.width(t, fs) } else { 0.0 })
            .collect();
        let mut buf = Vec::new();
        for r in rows.start..rows.end.min(rows.start + SAMPLE_ROWS) {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            for (k, w) in nat.iter_mut().enumerate() {
                let v = reader.get(&buf, k);
                if !v.is_empty() {
                    let v: String = v.chars().take(200).collect();
                    *w = w.max(fonts.width(&v, fs));
                }
            }
        }
        let min_w = (fs * 2.5).max(12.0);
        let max_w = (width * 0.4).max(min_w);
        let nat: Vec<f64> = nat
            .iter()
            .map(|w| (w + 2.0 * pad).clamp(min_w, max_w))
            .collect();
        let total: f64 = nat.iter().sum();
        let n = rows.end - rows.start;
        let fit_w = page.fit_width || page.fit_page;
        let mut want = if fit_w && total > width {
            fs * width / total
        } else {
            fs
        };
        if page.fit_page {
            want = want.min(one_page_size(
                page,
                ph - 2.0 * margin,
                n + u64::from(header),
            ));
        }
        let (size, widths) = if want < fs {
            let size = want.max(MIN_FONT);
            let k = size / fs;
            (
                size,
                nat.iter().map(|w| (w * k).min(width)).collect::<Vec<_>>(),
            )
        } else {
            (fs, nat)
        };
        // Pages across.
        let mut groups = Vec::new();
        let mut start = 0;
        let mut acc = 0.0;
        for (k, w) in widths.iter().enumerate() {
            if k > start && acc + w > width + 0.01 {
                groups.push(start..k);
                start = k;
                acc = 0.0;
            }
            acc += w;
        }
        groups.push(start..widths.len());
        // A fallback font read for the rows looked at may need a taller line
        // (its ascent and descent), never a shorter one; Latin keeps 1.4.
        let row_h = size * fonts.line_factor(1.4);
        let title_size = (size + 3.0).min(16.0);
        let title_h = if page.title.trim().is_empty() {
            0.0
        } else {
            title_size * 1.8
        };
        let footer_h = if page.page_numbers { size * 2.4 } else { 0.0 };
        let avail = ph - 2.0 * margin - title_h - footer_h;
        let fits = |h: f64| ((h / row_h).floor() as u64).max(1);
        let with = fits(avail - if header { row_h } else { 0.0 });
        let without = fits(avail);
        let mut row_pages = Vec::new();
        let mut at = 0u64;
        loop {
            let per = if header && (page.repeat_header || row_pages.is_empty()) {
                with
            } else {
                without
            };
            let end = (at + per).min(n);
            row_pages.push(at..end);
            at = end;
            if at >= n {
                break;
            }
            if row_pages.len() as u64 * groups.len() as u64 > MAX_PAGES {
                return Err(Error::Invalid(format!(
                    "that would be more than {MAX_PAGES} pages; print fewer rows or columns"
                )));
            }
        }
        Ok(Layout {
            pw,
            ph,
            margin,
            size,
            row_h,
            pad: 0.35 * size,
            widths,
            groups,
            row_pages,
            title_size,
            title_h,
            footer_h,
        })
    }
}

/// The largest text size at which `lines` table rows (the titles row
/// included), the title and the page numbers fit `height` points, by the
/// same row, title and footer heights the layout uses.
fn one_page_size(page: &PageSetup, height: f64, lines: u64) -> f64 {
    let lines = lines.max(1) as f64;
    let foot = if page.page_numbers { 2.4 } else { 0.0 };
    let size = if page.title.trim().is_empty() {
        height / (lines * 1.4 + foot)
    } else {
        // The title is (size + 3) * 1.8 high, at most 16 * 1.8; each
        // formula overstates it outside its own range, so the smaller is safe.
        let small = (height - 5.4) / (lines * 1.4 + 1.8 + foot);
        let large = (height - 28.8) / (lines * 1.4 + foot);
        small.min(large)
    };
    // A hair under, so rounding never pushes the last row to a second page.
    size * 0.9999
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::export::ExportFormat;
    use crate::OpenOptions;

    fn pdf(d: &Document, page: PageSetup) -> (Vec<u8>, ExportReport) {
        let mut out = Vec::new();
        let r = d
            .export(
                &mut out,
                &ExportOptions {
                    format: ExportFormat::Pdf,
                    page,
                    ..Default::default()
                },
                None,
                &Progress::new(),
            )
            .unwrap();
        (out, r)
    }

    fn table(rows: usize, cols: usize) -> Document {
        let mut s = String::new();
        for r in 0..=rows {
            let cells: Vec<String> = (0..cols)
                .map(|c| {
                    if r == 0 {
                        format!("Title {c}")
                    } else {
                        format!("v{r}-{c} é日")
                    }
                })
                .collect();
            s.push_str(&cells.join(","));
            s.push_str("\r\n");
        }
        Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
    }

    /// Every xref offset points at its object.
    fn check_structure(b: &[u8]) {
        assert!(b.starts_with(b"%PDF-1.7"));
        let sx = b.windows(10).rposition(|w| w == b"startxref\n").unwrap();
        let tail = std::str::from_utf8(&b[sx..]).unwrap();
        let xref: usize = tail[10..].lines().next().unwrap().parse().unwrap();
        let text = std::str::from_utf8(&b[xref..]).unwrap();
        assert!(text.starts_with("xref\n0 "));
        let mut lines = text.lines().skip(1);
        let count: usize = lines.next().unwrap()[2..].parse().unwrap();
        lines.next();
        for id in 1..count {
            let off: usize = lines.next().unwrap()[..10].parse().unwrap();
            assert!(
                b[off..].starts_with(format!("{id} 0 obj").as_bytes()),
                "object {id}"
            );
        }
        assert!(text.ends_with("%%EOF\n"));
    }

    #[test]
    fn pages_count_and_structure() {
        let d = table(120, 4);
        let (b, r) = pdf(&d, PageSetup::default());
        check_structure(&b);
        assert_eq!(r.rows, 120);
        // Letter portrait, 9 pt: about 50 rows a page.
        assert!((2..=4).contains(&r.sheets), "{}", r.sheets);
        let text = String::from_utf8_lossy(&b);
        assert_eq!(text.matches("/Type /Page ").count() as u32, r.sheets);
        assert!(text.contains(&format!("/Count {}", r.sheets)));
    }

    #[test]
    fn wide_tables_go_across_or_shrink() {
        let d = table(10, 40);
        let (_, fit) = pdf(&d, PageSetup::default());
        let (_, across) = pdf(
            &d,
            PageSetup {
                fit_width: false,
                ..Default::default()
            },
        );
        assert!(across.sheets > fit.sheets);
        let (b, sel) = pdf(
            &d,
            PageSetup {
                cols: Some(vec![3, 1]),
                rows: Some((2, 3)),
                title: "Report (draft)".into(),
                ..Default::default()
            },
        );
        check_structure(&b);
        assert_eq!(sel.rows, 3);
        assert_eq!(sel.sheets, 1);
    }

    #[test]
    fn bad_setups_are_refused() {
        let d = table(3, 2);
        let mut out = Vec::new();
        for page in [
            PageSetup {
                font_size: 0.0,
                ..Default::default()
            },
            PageSetup {
                font_size: f64::NAN,
                ..Default::default()
            },
            PageSetup {
                margin_mm: -1.0,
                ..Default::default()
            },
            PageSetup {
                cols: Some(vec![]),
                ..Default::default()
            },
            PageSetup {
                cols: Some(vec![9]),
                ..Default::default()
            },
        ] {
            let o = ExportOptions {
                format: ExportFormat::Pdf,
                page,
                ..Default::default()
            };
            assert!(d.export(&mut out, &o, None, &Progress::new()).is_err());
        }
        // A huge margin is held to a quarter of the page; empty tables print.
        let (b, r) = pdf(
            &table(0, 1),
            PageSetup {
                margin_mm: 1e9,
                ..Default::default()
            },
        );
        check_structure(&b);
        assert_eq!(r.sheets, 1);
    }

    /// The layout of all of `d`'s columns and its first `rows` rows, titled.
    fn layout(d: &Document, page: &PageSetup, rows: u64) -> Layout {
        let cols: Vec<u32> = (0..d.col_count()).collect();
        let titles: Vec<String> = cols.iter().map(|c| format!("Title {c}")).collect();
        let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
        d.pdf_layout(
            page,
            &Fonts::load(),
            true,
            &titles,
            &Cols::new(d, &cols),
            0..rows,
        )
        .unwrap()
    }

    #[test]
    fn fit_on_one_page_shrinks_to_one_sheet() {
        // 40 rows at 24 pt need several pages; on one page they take one.
        let d = table(40, 6);
        let big = PageSetup {
            font_size: 24.0,
            ..Default::default()
        };
        let (_, plain) = pdf(&d, big.clone());
        assert!(plain.sheets > 1, "{}", plain.sheets);
        let one = PageSetup {
            fit_page: true,
            ..big
        };
        let (b, r) = pdf(&d, one.clone());
        check_structure(&b);
        assert_eq!((r.sheets, r.rows), (1, 40));
        let l = layout(&d, &one, 40);
        assert!(l.size < 24.0 && l.size > MIN_FONT, "{}", l.size);
        // Not shrunk when it already fits.
        let small = PageSetup {
            fit_page: true,
            ..Default::default()
        };
        assert_eq!(layout(&d, &small, 40).size, 9.0);
    }

    #[test]
    fn fit_on_one_page_holds_with_titles_and_page_numbers() {
        for n in [1u64, 5, 20, 60, 120, 140] {
            for (title, numbers) in [
                ("", false),
                ("", true),
                ("Quarterly list", true),
                ("T", false),
            ] {
                let d = table(n as usize, 3);
                let page = PageSetup {
                    font_size: 40.0,
                    fit_page: true,
                    title: title.into(),
                    page_numbers: numbers,
                    ..Default::default()
                };
                let l = layout(&d, &page, n);
                // 140 rows need more than a page even at the smallest size.
                if n < 140 {
                    assert!(l.size > MIN_FONT, "{n} rows: {}", l.size);
                    assert_eq!(
                        l.row_pages.len(),
                        1,
                        "{n} rows, {title:?}, {numbers}: {}",
                        l.size
                    );
                } else {
                    assert_eq!(l.size, MIN_FONT);
                }
            }
        }
    }

    #[test]
    fn fit_on_one_page_stops_at_the_smallest_size() {
        let d = table(2_000, 4);
        let page = PageSetup {
            fit_page: true,
            ..Default::default()
        };
        let l = layout(&d, &page, 2_000);
        assert_eq!(l.size, MIN_FONT);
        let (_, r) = pdf(&d, page);
        assert!(r.sheets > 1);
        assert_eq!(r.sheets as usize, l.row_pages.len() * l.groups.len());
    }

    #[test]
    fn printing_a_block_prints_only_its_columns() {
        let d = table(5, 6);
        let page = PageSetup {
            cols: Some(vec![2, 3, 4]),
            rows: Some((1, 3)),
            ..Default::default()
        };
        let (b, r) = pdf(&d, page);
        check_structure(&b);
        assert_eq!((r.rows, r.sheets), (3, 1));
    }

    /// The text size the PDF fits the 30-column sample to on Letter
    /// portrait; the app's Print fit (m8.ts `printFit`) must come within
    /// 0.5 pt of the same figure, kept in `thirty.json` for both.
    #[test]
    fn thirty_column_fit_matches_the_recorded_size() {
        let d = Document::from_bytes(
            include_bytes!("../tests/print-fit/thirty.csv").to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let rec: serde_json::Value =
            serde_json::from_str(include_str!("../tests/print-fit/thirty.json")).unwrap();
        let page = PageSetup {
            landscape: true,
            ..Default::default()
        };
        let cols: Vec<u32> = (0..d.col_count()).collect();
        let names = d.export_names(true);
        let titles: Vec<&str> = names.iter().map(String::as_str).collect();
        let fonts = Fonts::load();
        let face = &fonts.main;
        let l = d
            .pdf_layout(
                &page,
                &fonts,
                true,
                &titles,
                &Cols::new(&d, &cols),
                1..d.row_count(),
            )
            .unwrap();
        println!("thirty-column PDF size: {:.3}", l.size);
        // The widest value of each column is not cut once shrunk (it was,
        // by a rounding error, before the fit allowed a hair of slack).
        let pad = l.pad;
        for (k, title) in titles.iter().enumerate() {
            let mut widest = title.to_string();
            for r in 1..d.row_count() {
                let v = d.row_values(r).unwrap()[k].clone();
                if face.width(&v, l.size) > face.width(&widest, l.size) {
                    widest = v;
                }
            }
            let shown = fit(&fonts, &widest, l.widths[k] - 2.0 * pad, l.size);
            assert_eq!(shown, widest.as_str(), "column {k} cut");
        }
        // Only with the font the record was made with (Segoe UI, Windows).
        if matches!(face, Face::Embedded { name, .. } if name.to_lowercase().contains("segoe")) {
            let want = rec["pdf_size"].as_f64().unwrap();
            assert!((l.size - want).abs() < 0.05, "{} against {want}", l.size);
        }
    }

    #[test]
    fn cells_are_cut_to_fit() {
        let fonts = Fonts::with(Face::Helvetica, HanFont::Simplified, None);
        let face = &fonts.main;
        let long = "x".repeat(500);
        let t = fit(&fonts, &long, 50.0, 9.0);
        assert!(t.ends_with('…'));
        assert!(face.width(&t, 9.0) <= 50.0 + 0.01);
        assert_eq!(fit(&fonts, "a\nb", 100.0, 9.0), "a b");
        let mut out = Vec::new();
        face.encode("(a)\\€日", &mut [], &mut out);
        assert_eq!(out, b"(\\(a\\)\\\\\x80?)");
    }

    // ---- Chinese, Japanese and Korean (gap #7) ----

    thread_local! {
        /// The language and fonts folder a test's PDF runs use.
        pub(crate) static FONTS: std::cell::RefCell<Option<(HanFont, Option<std::path::PathBuf>)>> =
            const { std::cell::RefCell::new(None) };
    }

    fn use_fonts(han: HanFont, dir: Option<std::path::PathBuf>) {
        FONTS.with(|f| *f.borrow_mut() = Some((han, dir)));
    }

    /// The Windows fonts folder, when the CJK fonts the tests need are there.
    fn cjk_dir() -> Option<std::path::PathBuf> {
        let d = windows_fonts_dir()?;
        ["msyh.ttc", "YuGothM.ttc", "malgun.ttf"]
            .iter()
            .all(|f| d.join(f).is_file())
            .then_some(d)
    }

    fn doc_of(rows: &[&str]) -> Document {
        let mut s = String::from("Name\r\n");
        for r in rows {
            s.push_str(r);
            s.push_str("\r\n");
        }
        Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
    }

    /// Object `id`'s bytes (from the cross reference table).
    fn object(b: &[u8], id: usize) -> &[u8] {
        let sx = b.windows(10).rposition(|w| w == b"startxref\n").unwrap();
        let tail = std::str::from_utf8(&b[sx..]).unwrap();
        let xref: usize = tail[10..].lines().next().unwrap().parse().unwrap();
        let line = std::str::from_utf8(&b[xref..])
            .unwrap()
            .lines()
            .nth(3 + id - 1)
            .unwrap()
            .to_string();
        let off: usize = line[..10].parse().unwrap();
        let end = off + b[off..].windows(7).position(|w| w == b"endobj\n").unwrap();
        &b[off..end]
    }

    /// A stream object's data, inflated.
    fn stream_of(obj: &[u8]) -> Vec<u8> {
        use std::io::Read;
        let s = obj.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let e = obj.windows(10).rposition(|w| w == b"\nendstream").unwrap();
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(&obj[s..e])
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    /// Each page's font resources and the fonts its contents select.
    fn page_fonts(b: &[u8]) -> Vec<(Vec<String>, Vec<String>)> {
        let text = String::from_utf8_lossy(b).into_owned();
        let mut out = Vec::new();
        for (at, _) in text.match_indices("/Type /Page /Parent") {
            let obj = &text[at..at + text[at..].find("endobj").unwrap()];
            let res = &obj[obj.find("/Font << ").unwrap() + 9..];
            let res = &res[..res.find(" >>").unwrap()];
            let mut listed: Vec<String> = res
                .split(' ')
                .filter(|w| w.starts_with("/F"))
                .map(String::from)
                .collect();
            listed.sort();
            let cid: usize = obj[obj.find("/Contents ").unwrap() + 10..]
                .split(' ')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let content = String::from_utf8(stream_of(object(b, cid))).unwrap();
            let mut drawn: Vec<String> = content
                .split_whitespace()
                .filter(|w| w.starts_with("/F"))
                .map(String::from)
                .collect();
            drawn.sort();
            drawn.dedup();
            out.push((listed, drawn));
        }
        out
    }

    /// The characters a font's ToUnicode map (object `id`) holds.
    fn unicode_of(b: &[u8], id: usize) -> String {
        let cmap = String::from_utf8(stream_of(object(b, id))).unwrap();
        let mut out = String::new();
        for line in cmap
            .lines()
            .filter(|l| l.starts_with('<') && l.contains("> <"))
        {
            let hex = line.rsplit('<').next().unwrap().trim_end_matches('>');
            let units: Vec<u16> = (0..hex.len() / 4)
                .map(|i| u16::from_str_radix(&hex[i * 4..i * 4 + 4], 16).unwrap())
                .collect();
            out.push_str(&String::from_utf16(&units).unwrap());
        }
        out
    }

    #[test]
    fn scripts_are_told_apart_by_their_ranges_and_the_rule() {
        for c in [
            '中',
            '国',
            '㐀',
            '\u{20000}',
            '\u{2A6D6}',
            '豈',
            '、',
            '。',
            'Ａ',
        ] {
            assert!(
                is_han(c) && is_cjk(c) && !is_kana(c) && !is_hangul(c),
                "{c}"
            );
        }
        for c in ['あ', 'ア', 'ン', 'ㇰ', 'ｱ', 'ﾝ'] {
            assert!(is_kana(c) && !is_han(c), "{c}");
        }
        for c in ['한', '국', 'ᄀ', 'ㄱ', 'ꥠ'] {
            assert!(is_hangul(c) && !is_han(c), "{c}");
        }
        for c in ['a', 'é', 'Ж', 'ก', '😀'] {
            assert!(!is_cjk(c), "{c}");
        }
        use HanFont::*;
        // Any kana: Japanese, even with Han and Hangul; else any Hangul: Korean.
        for han in [Simplified, Traditional, Japanese, Korean] {
            assert_eq!(cjk_for("山田たろう", han), Cjk::Japanese);
            assert_eq!(cjk_for("김민수 王", han), Cjk::Korean);
            assert_eq!(cjk_for("やまだ 김", han), Cjk::Japanese);
        }
        // Han alone (and CJK punctuation alone) follows the display language.
        assert_eq!(cjk_for("王小明", Simplified), Cjk::Chinese);
        assert_eq!(cjk_for("王小明", Traditional), Cjk::Chinese);
        assert_eq!(cjk_for("王小明", Japanese), Cjk::Japanese);
        assert_eq!(cjk_for("王小明", Korean), Cjk::Korean);
        assert_eq!(cjk_for("「」", Japanese), Cjk::Japanese);
        assert_eq!(han_for_langid(0x0411), Japanese);
        assert_eq!(han_for_langid(0x0412), Korean);
        assert_eq!(han_for_langid(0x0804), Simplified);
        assert_eq!(han_for_langid(0x1004), Simplified);
        for id in [0x0404, 0x0C04, 0x1404] {
            assert_eq!(han_for_langid(id), Traditional);
        }
        assert_eq!(han_for_langid(0x0409), Simplified);
        // Traditional Han reads its own fonts; the others theirs.
        assert_eq!(fallback_files(Cjk::Chinese, Traditional)[0], "msjh.ttc");
        assert_eq!(fallback_files(Cjk::Chinese, Simplified)[0], "msyh.ttc");
    }

    #[test]
    fn each_script_gets_its_own_font_listed_only_on_the_pages_that_use_it() {
        let Some(dir) = cjk_dir() else { return };
        use_fonts(HanFont::Simplified, Some(dir));
        // 60 Latin rows (page 1), then Chinese, then Japanese, then Korean names.
        let mut rows: Vec<String> = (0..60).map(|i| format!("Person {i}")).collect();
        rows.extend((0..60).map(|i| format!("王小明{i}")));
        rows.extend((0..60).map(|i| format!("やまだ たろう{i}")));
        rows.extend((0..60).map(|i| format!("김민수{i}")));
        let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
        let (b, r) = pdf(&doc_of(&rows), PageSetup::default());
        check_structure(&b);
        assert_eq!((r.missing_chars, r.missing_cjk), (0, false));
        let pages = page_fonts(&b);
        assert!(pages.len() >= 4);
        for (i, (listed, drawn)) in pages.iter().enumerate() {
            // Every font a page draws with is in its resources, and only those (and /F1).
            let mut want = drawn.clone();
            if !want.contains(&"/F1".to_string()) {
                want.push("/F1".into());
                want.sort();
            }
            assert_eq!(listed, &want, "page {}", i + 1);
        }
        assert_eq!(pages[0].0, vec!["/F1"], "a Latin page lists no fallback");
        let all: Vec<String> = pages.iter().flat_map(|p| p.0.clone()).collect();
        for f in ["/F2", "/F3", "/F4"] {
            assert!(all.contains(&f.to_string()), "{f} used somewhere");
        }
        // Each fallback is a subset named ABCDEF+Name with its own map back to the text.
        let text = String::from_utf8_lossy(&b).into_owned();
        for (base, name, chars) in [
            (9, "YuGothM", "やまだたろう"),
            (14, "malgun", "김민수"),
            (19, "msyh", "王小明"),
        ] {
            let t0 = String::from_utf8_lossy(object(&b, base)).into_owned();
            let tag = t0
                .split("/BaseFont /")
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap()
                .to_string();
            assert_eq!(tag.len(), 7 + name.len(), "{tag}");
            assert!(
                tag[..6].chars().all(|c| c.is_ascii_uppercase())
                    && &tag[6..7] == "+"
                    && tag.ends_with(name),
                "{tag}"
            );
            let back = unicode_of(&b, base + 4);
            for c in chars.chars() {
                assert!(back.contains(c), "{c} in {name}'s map");
            }
            // A subset, not the whole font file (those are 13 to 20 MB).
            let file = object(&b, base + 3);
            assert!(file.len() < 1_500_000, "{name} subset {} bytes", file.len());
        }
        assert!(text.contains("/CIDToGIDMap /Identity"));
        // The whole PDF stays small.
        assert!(b.len() < 3_000_000, "{} bytes", b.len());
    }

    #[test]
    fn latin_text_reads_no_fallback_font_and_lists_none() {
        use_fonts(HanFont::Simplified, cjk_dir());
        let fonts = Fonts::load();
        let mut u = Usage::new(vec![0; 65536]);
        let mut out = Vec::new();
        fonts.show("Plain Latin, café, Ж", 9.0, &mut u, &mut out);
        let _ = fonts.width("Another line", 9.0);
        assert_eq!(fonts.reads.get(), 0);
        assert_eq!(u.page_fonts(), "/F1 4 0 R");
        let rows = ["Ann", "Bob", "Zoë"];
        let (b, r) = pdf(&doc_of(&rows), PageSetup::default());
        check_structure(&b);
        let text = String::from_utf8_lossy(&b);
        assert!(!text.contains("/F2") && !text.contains("/F3") && !text.contains("/F4"));
        assert_eq!(r.missing_chars, 0);
    }

    #[test]
    fn without_a_cjk_font_the_pdf_is_written_and_the_characters_are_counted_not_quoted() {
        use_fonts(HanFont::Simplified, None);
        let rows = ["王小明", "Ann", "やまだ"];
        let (b, r) = pdf(&doc_of(&rows), PageSetup::default());
        check_structure(&b);
        assert_eq!((r.missing_chars, r.missing_cjk), (6, true));
        let json = serde_json::to_string(&r).unwrap();
        for c in ['王', 'や', 'A'] {
            assert!(!json.contains(c), "{json}");
        }
        // Characters of other scripts no font has are counted too (and are not CJK).
        let Some(dir) = cjk_dir() else { return };
        use_fonts(HanFont::Simplified, Some(dir));
        let main_lacks = ['\u{1F600}', '\u{10FFFD}']
            .into_iter()
            .filter(|&c| !Fonts::load().main_has(c))
            .count() as u64;
        let (b, r) = pdf(
            &doc_of(&["smile \u{1F600}\u{10FFFD}"]),
            PageSetup::default(),
        );
        check_structure(&b);
        assert_eq!((r.missing_chars, r.missing_cjk), (main_lacks, false));
    }

    #[test]
    fn each_pdf_of_a_run_holds_only_its_own_glyphs() {
        let Some(dir) = cjk_dir() else { return };
        let fonts = Fonts::with(load_face(), HanFont::Simplified, Some(dir));
        let one = |text: &str| -> Vec<u8> {
            let mut out = Vec::new();
            let (mut o, used) = Out::start(&mut out, &fonts.main).unwrap();
            let mut u = Usage::new(used);
            let mut c = Vec::new();
            c.extend_from_slice(b"BT /F1 9 Tf ");
            fonts.show(text, 9.0, &mut u, &mut c);
            c.extend_from_slice(b"ET\n");
            o.stream(FIRST_PAGE_OBJ + 1, &c, "").unwrap();
            let res = u.page_fonts();
            o.obj(
                FIRST_PAGE_OBJ,
                format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << {res} >> >> /Contents {} 0 R >>", FIRST_PAGE_OBJ + 1).as_bytes(),
            )
            .unwrap();
            finish(
                &mut o,
                &fonts,
                &mut u,
                &format!("{FIRST_PAGE_OBJ} 0 R "),
                1,
                "",
            )
            .unwrap();
            out
        };
        let a = one("王先生");
        let b = one("李小姐");
        check_structure(&a);
        check_structure(&b);
        let (ma, mb) = (unicode_of(&a, 19 + 4), unicode_of(&b, 19 + 4));
        assert!(ma.contains('王') && !ma.contains('李'), "{ma}");
        assert!(mb.contains('李') && !mb.contains('王'), "{mb}");
        // The fallback was read once for both.
        assert_eq!(fonts.reads.get(), 1);
    }

    #[test]
    fn long_and_mixed_cjk_cells_fit_and_switch_fonts_within_a_cell() {
        let Some(dir) = cjk_dir() else { return };
        use_fonts(HanFont::Simplified, Some(dir));
        let long: String = "漢字".repeat(500);
        let mixed = "Tanaka 田中 たなか 다나카 x";
        let (b, r) = pdf(&doc_of(&[&long, mixed]), PageSetup::default());
        check_structure(&b);
        assert_eq!(r.missing_chars, 0);
        let pages = page_fonts(&b);
        // The kana make the whole mixed cell Japanese (the Hangul too, as the Japanese font has it or not).
        assert!(pages[0].1.contains(&"/F2".to_string()));
        // Text is cut to its column with "…", measured in the fonts that draw it.
        let fonts = Fonts::load();
        let t = fit(&fonts, &long, 200.0, 9.0);
        assert!(
            t.ends_with('…') && fonts.width(&t, 9.0) <= 200.01,
            "{}",
            fonts.width(&t, 9.0)
        );
        // Measured with the CJK font, not the main font's empty box.
        let w = fonts.width("王", 10.0);
        assert!((8.0..=11.0).contains(&w), "{w}");
    }

    #[test]
    fn to_unicode_writes_characters_past_the_basic_plane_as_surrogate_pairs() {
        let m = to_unicode(&[(5, 0x2000B), (6, '王' as u32)]);
        assert!(m.contains("<0005> <D840DC0B>"), "{m}");
        assert!(m.contains("<0006> <738B>"), "{m}");
        assert_eq!(subset_tag(&[1, 2, 3]).len(), 6);
        assert_ne!(subset_tag(&[1, 2, 3]), subset_tag(&[1, 2, 4]));
    }

    #[test]
    fn a_cjk_line_is_never_shorter_and_latin_keeps_its_height() {
        let Some(dir) = cjk_dir() else { return };
        let fonts = Fonts::with(load_face(), HanFont::Simplified, Some(dir));
        assert_eq!(fonts.line_factor(1.4), 1.4);
        let _ = fonts.width("王", 9.0);
        let f = fonts.line_factor(1.4);
        assert!((1.4..=2.1).contains(&f), "{f}");
    }

    #[test]
    fn fallback_candidates_that_cannot_be_used_fall_through_to_the_next() {
        let dir = std::env::temp_dir().join(format!("nde-cjk-fonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Missing, damaged, too large, not TrueType: none used.
        std::fs::write(
            dir.join("damaged.ttc"),
            b"ttcf\0\x01\0\0\0\0\0\x01\xff\xff\xff\xff",
        )
        .unwrap();
        std::fs::write(dir.join("otto.ttf"), b"OTTO\0\0\0\0").unwrap();
        let big = std::fs::File::create(dir.join("big.ttf")).unwrap();
        big.set_len(MAX_FONT_BYTES as u64 + 1).unwrap();
        drop(big);
        assert!(
            first_usable(&dir, &["missing.ttc", "damaged.ttc", "otto.ttf", "big.ttf"]).is_none()
        );
        if let Some(root) = windows_fonts_dir() {
            if let Ok(data) = std::fs::read(root.join("malgun.ttf")) {
                // A font whose licence asks not to be subset is passed over for the next one.
                let mut no = data.clone();
                set_fs_type(&mut no, 0x0100);
                std::fs::write(dir.join("nosub.ttf"), &no).unwrap();
                std::fs::write(dir.join("good.ttf"), &data).unwrap();
                let f = first_usable(&dir, &["missing.ttc", "nosub.ttf", "big.ttf", "good.ttf"])
                    .unwrap();
                assert_eq!(f.name, "good");
                assert!(first_usable(&dir, &["nosub.ttf"]).is_none());
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Write `v` into a font file's OS/2 fsType.
    fn set_fs_type(d: &mut [u8], v: u16) {
        let n = u16::from_be_bytes([d[4], d[5]]) as usize;
        for i in 0..n {
            let rec = 12 + i * 16;
            if &d[rec..rec + 4] == b"OS/2" {
                let off =
                    u32::from_be_bytes([d[rec + 8], d[rec + 9], d[rec + 10], d[rec + 11]]) as usize;
                d[off + 8..off + 10].copy_from_slice(&v.to_be_bytes());
            }
        }
    }
}
