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
    Embedded { font: Font, name: String },
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
                font,
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
pub(crate) fn fit<'a>(face: &Face, s: &'a str, room: f64, size: f64) -> Cow<'a, str> {
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
    if face.width(&t, size) <= room + 0.01 {
        return t;
    }
    let ell = face.ellipsis();
    let room = room - face.width(ell, size);
    let mut w = 0.0;
    let mut out = String::new();
    for c in t.chars() {
        w += face.char_width(c, size);
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

/// Text as a PDF string (UTF-16 with a byte-order mark, in hex).
/// The rest of a PDF after its pages (objects 9 and up): fonts (objects
/// 4 to 8), the page list (2), catalog (1), info (3), and the cross
/// reference table. `used` maps glyphs to the characters drawn with them.
pub(crate) fn finish<W: Write>(
    o: &mut Out<'_, W>,
    face: &Face,
    used: &[u32],
    kids: &str,
    total_pages: u64,
    title_text: &str,
) -> Result<(), Error> {
    match face {
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
        let face = load_face();
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
        let l = self.pdf_layout(page, &face, opts.header, &titles, &reader, rows.clone())?;
        let total_pages = (l.groups.len() * l.row_pages.len()) as u64;
        progress.start(total_pages);

        let (mut o, mut used) = Out::start(out, &face)?;
        let mut kids = String::new();
        let mut buf = Vec::new();
        let mut c: Vec<u8> = Vec::with_capacity(64 << 10);
        let mut next = 9usize;
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
                        let t = fit(&face, text, room, l.size);
                        if t.is_empty() {
                            return;
                        }
                        let tx = if right {
                            x + l.widths[k] - l.pad - face.width(&t, l.size)
                        } else {
                            x + l.pad
                        };
                        let base = y - (l.row_h + l.size * 0.7) / 2.0;
                        let _ = write!(c, "1 0 0 1 {} {} Tm ", num(tx), num(base));
                        face.encode(&t, &mut used, c);
                        c.extend_from_slice(b" Tj\n");
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
                    let t = fit(&face, &page.title, l.pw - 2.0 * l.margin, l.title_size);
                    let _ = write!(
                        c,
                        "BT /F1 {} Tf 1 0 0 1 {} {} Tm ",
                        num(l.title_size),
                        num(l.margin),
                        num(l.ph - l.margin - l.title_size)
                    );
                    face.encode(&t, &mut used, &mut c);
                    c.extend_from_slice(b" Tj ET\n");
                }
                if l.footer_h > 0.0 {
                    let t = format!("Page {page_no} of {total_pages}");
                    let w = face.width(&t, l.size);
                    let _ = write!(
                        c,
                        "BT /F1 {} Tf 1 0 0 1 {} {} Tm ",
                        num(l.size),
                        num((l.pw - w) / 2.0),
                        num(l.margin)
                    );
                    face.encode(&t, &mut used, &mut c);
                    c.extend_from_slice(b" Tj ET\n");
                }
                let (pid, cid) = (next, next + 1);
                next += 2;
                o.stream(cid, &c, "")?;
                o.obj(
                    pid,
                    format!(
                        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 4 0 R >> >> /Contents {cid} 0 R >>",
                        num(l.pw),
                        num(l.ph)
                    )
                    .as_bytes(),
                )?;
                kids.push_str(&format!("{pid} 0 R "));
                progress.step(1)?;
            }
        }
        finish(&mut o, &face, &used, &kids, total_pages, &page.title)?;
        Ok(ExportReport {
            rows: rows.end - rows.start,
            sheets: u32::try_from(total_pages).unwrap_or(u32::MAX),
            bytes: o.n,
            files: Vec::new(),
        })
    }

    fn pdf_layout(
        &self,
        page: &PageSetup,
        face: &Face,
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
            .map(|t| if header { face.width(t, fs) } else { 0.0 })
            .collect();
        let mut buf = Vec::new();
        for r in rows.start..rows.end.min(rows.start + SAMPLE_ROWS) {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            for (k, w) in nat.iter_mut().enumerate() {
                let v = reader.get(&buf, k);
                if !v.is_empty() {
                    let v: String = v.chars().take(200).collect();
                    *w = w.max(face.width(&v, fs));
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
        let row_h = size * 1.4;
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
mod tests {
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
            &load_face(),
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
        let face = load_face();
        let l = d
            .pdf_layout(
                &page,
                &face,
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
            let shown = fit(&face, &widest, l.widths[k] - 2.0 * pad, l.size);
            assert_eq!(shown, widest.as_str(), "column {k} cut");
        }
        // Only with the font the record was made with (Segoe UI, Windows).
        if matches!(&face, Face::Embedded { name, .. } if name.to_lowercase().contains("segoe")) {
            let want = rec["pdf_size"].as_f64().unwrap();
            assert!((l.size - want).abs() < 0.05, "{} against {want}", l.size);
        }
    }

    #[test]
    fn cells_are_cut_to_fit() {
        let face = Face::Helvetica;
        let long = "x".repeat(500);
        let t = fit(&face, &long, 50.0, 9.0);
        assert!(t.ends_with('…'));
        assert!(face.width(&t, 9.0) <= 50.0 + 0.01);
        assert_eq!(fit(&face, "a\nb", 100.0, 9.0), "a b");
        let mut out = Vec::new();
        face.encode("(a)\\€日", &mut [], &mut out);
        assert_eq!(out, b"(\\(a\\)\\\\\x80?)");
    }
}
