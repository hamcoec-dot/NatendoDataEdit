//! Mailing labels and letters (Milestone 14): one label or one letter per
//! shown row, filled from a text template with `{Column title}` fields, and
//! written as a PDF with the same writer as PDF export.

use std::io::{BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::Cols;
use crate::pdf::{finish, fit, load_face, num, Face, Out, Paper, MAX_PAGES};
use crate::Error;

/// Longest template, in characters.
pub const MAX_TEMPLATE: usize = 20_000;
/// Most fields in a template.
const MAX_FIELDS: usize = 200;
/// Points per inch.
const PT: f64 = 72.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailKind {
    #[default]
    Labels,
    /// One letter per row, starting on a new page.
    Letters,
}

/// A sheet of labels, in inches.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LabelSheet {
    pub across: u32,
    pub down: u32,
    pub width: f64,
    pub height: f64,
    /// From the top edge of the paper to the first label.
    pub top: f64,
    /// From the left edge of the paper to the first label.
    pub left: f64,
    /// Space between labels side by side.
    pub gap_across: f64,
    /// Space between labels one above the other.
    pub gap_down: f64,
    pub paper: Paper,
}

impl Default for LabelSheet {
    fn default() -> Self {
        LABEL_SHEETS[0].1
    }
}

/// Common US letter label sheets, by size.
pub const LABEL_SHEETS: &[(&str, LabelSheet)] = &[
    (
        "Address labels, 1 x 2 5/8 in, 30 per sheet",
        LabelSheet {
            across: 3,
            down: 10,
            width: 2.625,
            height: 1.0,
            top: 0.5,
            left: 0.1875,
            gap_across: 0.125,
            gap_down: 0.0,
            paper: Paper::Letter,
        },
    ),
    (
        "Address labels, 1 x 4 in, 20 per sheet",
        LabelSheet {
            across: 2,
            down: 10,
            width: 4.0,
            height: 1.0,
            top: 0.5,
            left: 0.15625,
            gap_across: 0.1875,
            gap_down: 0.0,
            paper: Paper::Letter,
        },
    ),
    (
        "Address labels, 1 1/3 x 4 in, 14 per sheet",
        LabelSheet {
            across: 2,
            down: 7,
            width: 4.0,
            height: 4.0 / 3.0,
            top: 0.8333,
            left: 0.15625,
            gap_across: 0.1875,
            gap_down: 0.0,
            paper: Paper::Letter,
        },
    ),
    (
        "Shipping labels, 2 x 4 in, 10 per sheet",
        LabelSheet {
            across: 2,
            down: 5,
            width: 4.0,
            height: 2.0,
            top: 0.5,
            left: 0.15625,
            gap_across: 0.1875,
            gap_down: 0.0,
            paper: Paper::Letter,
        },
    ),
    (
        "Shipping labels, 3 1/3 x 4 in, 6 per sheet",
        LabelSheet {
            across: 2,
            down: 3,
            width: 4.0,
            height: 10.0 / 3.0,
            top: 0.5,
            left: 0.15625,
            gap_across: 0.1875,
            gap_down: 0.0,
            paper: Paper::Letter,
        },
    ),
    (
        "Return address labels, 1/2 x 1 3/4 in, 80 per sheet",
        LabelSheet {
            across: 4,
            down: 20,
            width: 1.75,
            height: 0.5,
            top: 0.5,
            left: 0.28125,
            gap_across: 0.3125,
            gap_down: 0.0,
            paper: Paper::Letter,
        },
    ),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MailSpec {
    pub kind: MailKind,
    /// Text with `{Column title}` fields (or `{3}` for column 3 when no
    /// column is called "3"); `{{` and `}}` for braces.
    pub template: String,
    /// The first row holds the column titles (it gets no label).
    pub header: bool,
    pub sheet: LabelSheet,
    /// Labels already used on the first sheet (start further on).
    pub skip: u32,
    /// Letters: paper and margin.
    pub paper: Paper,
    pub margin_mm: f64,
    /// Text size in points (labels shrink it to fit when needed).
    pub font_size: f64,
    /// Leave out lines whose fields are all blank (a second address line).
    pub skip_blank_lines: bool,
    /// Shown data rows to use: (first, count); `None`: all.
    pub rows: Option<(u64, u64)>,
}

impl Default for MailSpec {
    fn default() -> Self {
        MailSpec {
            kind: MailKind::Labels,
            template: String::new(),
            header: true,
            sheet: LabelSheet::default(),
            skip: 0,
            paper: Paper::Letter,
            margin_mm: 25.4,
            font_size: 10.0,
            skip_blank_lines: true,
            rows: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailReport {
    /// Labels or letters made.
    pub rows: u64,
    pub pages: u64,
    pub bytes: u64,
    /// Lines cut short because they did not fit.
    pub cut: u64,
}

/// A template split into text and fields (column positions in `cols`).
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Text(String),
    Field(usize),
    Break,
}

#[derive(Debug)]
struct Template {
    pieces: Vec<Piece>,
    /// Logical columns the fields read.
    cols: Vec<u32>,
}

fn bad(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

/// Read a template against the column titles.
fn parse(t: &str, titles: &[String]) -> Result<Template, Error> {
    if t.trim().is_empty() {
        return Err(bad("write the text for each label or letter first"));
    }
    if t.chars().count() > MAX_TEMPLATE {
        return Err(bad(format!(
            "the text can be at most {MAX_TEMPLATE} characters"
        )));
    }
    let mut pieces = Vec::new();
    let mut cols: Vec<u32> = Vec::new();
    let mut text = String::new();
    let mut it = t.chars().peekable();
    let flush = |text: &mut String, pieces: &mut Vec<Piece>| {
        if !text.is_empty() {
            pieces.push(Piece::Text(std::mem::take(text)));
        }
    };
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                text.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                text.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for c in it.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    if c == '\n' || c == '{' {
                        break;
                    }
                    name.push(c);
                }
                if !closed {
                    return Err(bad("a { has no } after it (write {{ for a brace)"));
                }
                let key = name.trim();
                let col = titles
                    .iter()
                    .position(|t| t.trim().eq_ignore_ascii_case(key))
                    .or_else(|| {
                        key.parse::<usize>()
                            .ok()
                            .filter(|&n| n >= 1 && n <= titles.len())
                            .map(|n| n - 1)
                    })
                    .ok_or_else(|| bad(format!("there is no column called \"{key}\"")))?;
                let at = match cols.iter().position(|&c| c as usize == col) {
                    Some(at) => at,
                    None => {
                        if cols.len() >= MAX_FIELDS {
                            return Err(bad(format!("use at most {MAX_FIELDS} different columns")));
                        }
                        cols.push(col as u32);
                        cols.len() - 1
                    }
                };
                flush(&mut text, &mut pieces);
                pieces.push(Piece::Field(at));
            }
            '\r' => {}
            '\n' => {
                flush(&mut text, &mut pieces);
                pieces.push(Piece::Break);
            }
            c => text.push(c),
        }
    }
    flush(&mut text, &mut pieces);
    Ok(Template { pieces, cols })
}

impl Template {
    /// The lines for one row. With `skip_blank`, a line with fields that
    /// are all blank is left out, and spaces left by blank fields are
    /// tidied.
    fn lines(&self, values: &[&str], skip_blank: bool) -> Vec<String> {
        let mut out = Vec::new();
        let mut line = String::new();
        let mut fields = 0;
        let mut filled = 0;
        let mut end = |line: &mut String, fields: &mut u32, filled: &mut u32| {
            let l = std::mem::take(line);
            if *fields > 0 && skip_blank {
                if *filled > 0 {
                    let tidy: Vec<&str> = l.split(' ').filter(|w| !w.is_empty()).collect();
                    let mut t = tidy.join(" ");
                    // "Springfield, " when the state was blank.
                    while t.ends_with(',') {
                        t.pop();
                    }
                    out.push(t);
                }
            } else {
                out.push(l);
            }
            *fields = 0;
            *filled = 0;
        };
        for p in &self.pieces {
            match p {
                Piece::Text(t) => line.push_str(t),
                Piece::Field(k) => {
                    let v = values.get(*k).copied().unwrap_or("");
                    // Line breaks inside a cell become new lines.
                    let v = v.replace("\r\n", "\n").replace('\r', "\n");
                    fields += 1;
                    if !v.trim().is_empty() {
                        filled += 1;
                    }
                    line.push_str(&v);
                }
                Piece::Break => end(&mut line, &mut fields, &mut filled),
            }
        }
        end(&mut line, &mut fields, &mut filled);
        // Cell line breaks.
        out.into_iter()
            .flat_map(|l| l.split('\n').map(str::to_string).collect::<Vec<_>>())
            .map(|l| l.replace('\t', " "))
            .collect()
    }
}

/// Words of `text` wrapped to `room` points; a word longer than the line is
/// broken.
fn wrap(face: &Face, text: &str, room: f64, size: f64) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if face.width(&candidate, size) <= room || line.is_empty() && word.is_empty() {
            line = candidate;
            continue;
        }
        if !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        // A word wider than the line: break it.
        let mut part = String::new();
        for c in word.chars() {
            part.push(c);
            if face.width(&part, size) > room && part.chars().count() > 1 {
                let last = part.pop().expect("one char");
                out.push(std::mem::take(&mut part));
                part.push(last);
            }
        }
        line = part;
    }
    out.push(line);
    out
}

impl Document {
    /// Write labels or letters for the shown rows as a PDF.
    pub fn write_mail(
        &self,
        out: &mut impl Write,
        spec: &MailSpec,
        progress: &Progress,
    ) -> Result<MailReport, Error> {
        check(spec)?;
        let all: Vec<u32> = (0..self.col_count()).collect();
        let titles = self.titles(&all, spec.header);
        let tpl = parse(&spec.template, &titles)?;
        let data = self.data_rows(spec.header);
        let rows = match spec.rows {
            Some((a, n)) => {
                (data.start + a).min(data.end)..(data.start + a).saturating_add(n).min(data.end)
            }
            None => data,
        };
        let n = rows.end - rows.start;
        if n == 0 {
            return Err(bad("there are no rows to print"));
        }
        let face = load_face();
        let reader = Cols::new(self, &tpl.cols);
        let (mut o, mut used) = Out::start(out, &face)?;
        let mut kids = String::new();
        let mut next = 9usize;
        let mut pages = 0u64;
        let mut cut = 0u64;
        let mut buf = Vec::new();
        let mut c: Vec<u8> = Vec::with_capacity(64 << 10);
        let mut page = |o: &mut Out<'_, _>,
                        c: &[u8],
                        pw: f64,
                        ph: f64,
                        kids: &mut String|
         -> Result<(), Error> {
            pages += 1;
            if pages > MAX_PAGES {
                return Err(bad(format!("that would be more than {MAX_PAGES} pages")));
            }
            let (pid, cid) = (next, next + 1);
            next += 2;
            o.stream(cid, c, "")?;
            o.obj(
                pid,
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 4 0 R >> >> /Contents {cid} 0 R >>",
                    num(pw),
                    num(ph)
                )
                .as_bytes(),
            )?;
            kids.push_str(&format!("{pid} 0 R "));
            Ok(())
        };
        let read_lines = |r: u64, buf: &mut Vec<_>| -> Result<Vec<String>, Error> {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, buf);
            let values: Vec<&str> = (0..tpl.cols.len()).map(|k| reader.get(buf, k)).collect();
            Ok(tpl.lines(&values, spec.skip_blank_lines))
        };
        progress.start(n);
        match spec.kind {
            MailKind::Labels => {
                let s = &spec.sheet;
                let (pw, ph) = s.paper.size();
                let per = (s.across * s.down) as u64;
                let pad_x = 0.1 * PT;
                let pad_y = 0.06 * PT;
                let (w, h) = (s.width * PT, s.height * PT);
                let mut slot = u64::from(spec.skip.min(s.across * s.down - 1));
                c.clear();
                for r in rows.clone() {
                    let lines = read_lines(r, &mut buf)?;
                    let k = slot % per;
                    let (col, row) = (k % s.across as u64, k / s.across as u64);
                    let x = s.left * PT + col as f64 * (w + s.gap_across * PT) + pad_x;
                    let top = ph - s.top * PT - row as f64 * (h + s.gap_down * PT) - pad_y;
                    let room_h = h - 2.0 * pad_y;
                    let count = lines.len().max(1) as f64;
                    let size = spec
                        .font_size
                        .min(room_h / (count * 1.15))
                        .max(crate::pdf::MIN_FONT);
                    let lh = size * 1.15;
                    let mut y = top - size;
                    let _ = writeln!(c, "BT /F1 {} Tf", num(size));
                    for l in &lines {
                        if y < top - room_h {
                            cut += 1;
                            break;
                        }
                        let t = fit(&face, l, w - 2.0 * pad_x, size);
                        if t.len() < l.len() {
                            cut += 1;
                        }
                        if !t.is_empty() {
                            let _ = write!(c, "1 0 0 1 {} {} Tm ", num(x), num(y));
                            face.encode(&t, &mut used, &mut c);
                            c.extend_from_slice(b" Tj\n");
                        }
                        y -= lh;
                    }
                    let _ = writeln!(c, "ET");
                    slot += 1;
                    if slot % per == 0 {
                        page(&mut o, &c, pw, ph, &mut kids)?;
                        c.clear();
                    }
                    progress.step(1)?;
                }
                if slot % per != 0 {
                    page(&mut o, &c, pw, ph, &mut kids)?;
                }
            }
            MailKind::Letters => {
                let (pw, ph) = spec.paper.size();
                let m = spec.margin_mm / 25.4 * PT;
                let size = spec.font_size;
                let lh = size * 1.3;
                let room = pw - 2.0 * m;
                for r in rows.clone() {
                    let lines = read_lines(r, &mut buf)?;
                    c.clear();
                    let mut y = ph - m - size;
                    let _ = writeln!(c, "BT /F1 {} Tf", num(size));
                    for l in &lines {
                        for part in wrap(&face, l, room, size) {
                            if y < m {
                                let _ = writeln!(c, "ET");
                                page(&mut o, &c, pw, ph, &mut kids)?;
                                c.clear();
                                y = ph - m - size;
                                let _ = writeln!(c, "BT /F1 {} Tf", num(size));
                            }
                            if !part.is_empty() {
                                let _ = write!(c, "1 0 0 1 {} {} Tm ", num(m), num(y));
                                face.encode(&part, &mut used, &mut c);
                                c.extend_from_slice(b" Tj\n");
                            }
                            y -= lh;
                        }
                    }
                    let _ = writeln!(c, "ET");
                    page(&mut o, &c, pw, ph, &mut kids)?;
                    progress.step(1)?;
                }
            }
        }
        let title = match spec.kind {
            MailKind::Labels => "Labels",
            MailKind::Letters => "Letters",
        };
        finish(&mut o, &face, &used, &kids, pages, title)?;
        Ok(MailReport {
            rows: n,
            pages,
            bytes: o.len(),
            cut,
        })
    }

    /// Write labels or letters to a PDF file (a temp file moved into place).
    pub fn save_mail(
        &self,
        path: &Path,
        spec: &MailSpec,
        progress: &Progress,
    ) -> Result<MailReport, Error> {
        if self.path().is_some_and(|p| crate::save::same_path(p, path)) {
            return Err(bad("choose another name: that is the open file"));
        }
        let (temp, file) = crate::save::create_temp(path)?;
        let done = (|| -> Result<MailReport, Error> {
            let mut w = BufWriter::with_capacity(1 << 20, &file);
            let r = self.write_mail(&mut w, spec, progress)?;
            w.flush()?;
            file.sync_all()?;
            Ok(r)
        })();
        drop(file);
        match done {
            Ok(r) => {
                if let Err(e) = crate::save::replace(&temp, path) {
                    let _ = std::fs::remove_file(&temp);
                    return Err(e.into());
                }
                Ok(r)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                Err(e)
            }
        }
    }

    /// The first `n` labels or letters as text, for the preview.
    pub fn mail_preview(&self, spec: &MailSpec, n: usize) -> Result<Vec<Vec<String>>, Error> {
        let all: Vec<u32> = (0..self.col_count()).collect();
        let titles = self.titles(&all, spec.header);
        let tpl = parse(&spec.template, &titles)?;
        let reader = Cols::new(self, &tpl.cols);
        let mut buf = Vec::new();
        let rows = self.data_rows(spec.header);
        let start = rows.start + spec.rows.map_or(0, |r| r.0);
        let mut out = Vec::new();
        for r in start..rows.end.min(start + n as u64) {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let values: Vec<&str> = (0..tpl.cols.len()).map(|k| reader.get(&buf, k)).collect();
            out.push(tpl.lines(&values, spec.skip_blank_lines));
        }
        Ok(out)
    }
}

/// Sizes that make sense.
fn check(spec: &MailSpec) -> Result<(), Error> {
    if !(4.0..=72.0).contains(&spec.font_size) {
        return Err(bad("choose a text size from 4 to 72 points"));
    }
    match spec.kind {
        MailKind::Labels => {
            let s = &spec.sheet;
            let (pw, ph) = s.paper.size();
            let ok = |v: f64| v.is_finite() && v >= 0.0;
            if s.across == 0 || s.down == 0 || s.across > 20 || s.down > 50 {
                return Err(bad("a sheet holds 1 to 20 labels across and 1 to 50 down"));
            }
            if !(ok(s.top) && ok(s.left) && ok(s.gap_across) && ok(s.gap_down))
                || !(s.width >= 0.25 && s.height >= 0.25)
            {
                return Err(bad("labels must be at least 1/4 inch wide and high"));
            }
            let right = s.left + s.across as f64 * s.width + (s.across - 1) as f64 * s.gap_across;
            let bottom = s.top + s.down as f64 * s.height + (s.down - 1) as f64 * s.gap_down;
            if right * PT > pw + 0.5 || bottom * PT > ph + 0.5 {
                return Err(bad("those labels do not fit on the paper"));
            }
        }
        MailKind::Letters => {
            let (pw, _) = spec.paper.size();
            if !(spec.margin_mm.is_finite()
                && spec.margin_mm >= 0.0
                && spec.margin_mm / 25.4 * PT * 2.0 < pw - 72.0)
            {
                return Err(bad("the margins leave no room for the text"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Progress;
    use crate::OpenOptions;

    fn from_text(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn doc() -> Document {
        from_text(
            "first,last,street,street2,city,state,zip\n\
             Ann,Lee,1 Main St,,Springfield,IL,62701\n\
             Bo,Wu,2 Oak Ave,Apt 4,Dayton,OH,45402\n\
             Cy,Diaz,3 Elm Rd,,Reno,,89501\n",
        )
    }

    fn tpl() -> String {
        "{first} {last}\n{street}\n{Street2}\n{city}, {state} {zip}".to_string()
    }

    #[test]
    fn fields_and_blank_lines() {
        let d = doc();
        let spec = MailSpec {
            template: tpl(),
            ..MailSpec::default()
        };
        let p = d.mail_preview(&spec, 5).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], ["Ann Lee", "1 Main St", "Springfield, IL 62701"]);
        assert_eq!(p[1], ["Bo Wu", "2 Oak Ave", "Apt 4", "Dayton, OH 45402"]);
        assert_eq!(p[2][2], "Reno, 89501");
        let keep = MailSpec {
            skip_blank_lines: false,
            ..spec.clone()
        };
        assert_eq!(d.mail_preview(&keep, 1).unwrap()[0].len(), 4);
        // Column numbers, braces.
        let n = MailSpec {
            template: "{{{1}}} {7}".into(),
            ..spec.clone()
        };
        assert_eq!(d.mail_preview(&n, 1).unwrap()[0], ["{Ann} 62701"]);
        let bad = MailSpec {
            template: "{nope}".into(),
            ..spec.clone()
        };
        assert!(d
            .mail_preview(&bad, 1)
            .unwrap_err()
            .to_string()
            .contains("nope"));
        let open = MailSpec {
            template: "{first".into(),
            ..spec
        };
        assert!(d.mail_preview(&open, 1).is_err());
    }

    fn pdf(d: &Document, spec: &MailSpec) -> (Vec<u8>, MailReport) {
        let mut out = Vec::new();
        let r = d.write_mail(&mut out, spec, &Progress::default()).unwrap();
        (out, r)
    }

    #[test]
    fn labels_fill_sheets() {
        let mut text = String::from("name,city\n");
        for i in 0..65 {
            text.push_str(&format!("Person {i},Town {i}\n"));
        }
        let d = from_text(&text);
        let spec = MailSpec {
            template: "{name}\n{city}".into(),
            ..MailSpec::default()
        };
        let (b, r) = pdf(&d, &spec);
        assert_eq!((r.rows, r.pages), (65, 3));
        assert!(b.starts_with(b"%PDF-1.7"));
        assert!(b.ends_with(b"%%EOF\n"));
        // Starting on a used sheet moves everything along.
        let skip = MailSpec {
            skip: 29,
            ..spec.clone()
        };
        assert_eq!(pdf(&d, &skip).1.pages, 4);
        // Only some rows.
        let some = MailSpec {
            rows: Some((0, 30)),
            ..spec
        };
        assert_eq!(pdf(&d, &some).1.pages, 1);
    }

    #[test]
    fn letters_one_per_row_and_long_ones_flow_on() {
        let d = doc();
        let spec = MailSpec {
            kind: MailKind::Letters,
            template: "Dear {first},\n\nThank you.".into(),
            ..MailSpec::default()
        };
        assert_eq!(pdf(&d, &spec).1.pages, 3);
        let long = MailSpec {
            template: format!("Dear {{first}},\n{}", "word ".repeat(3000)),
            ..spec
        };
        let r = pdf(&d, &long).1;
        assert!(r.pages > 3, "{r:?}");
    }

    #[test]
    fn bad_setups_are_refused() {
        let d = doc();
        let base = MailSpec {
            template: tpl(),
            ..MailSpec::default()
        };
        let mut out = Vec::new();
        let p = Progress::default();
        let cases = [
            MailSpec {
                template: " ".into(),
                ..base.clone()
            },
            MailSpec {
                font_size: 1.0,
                ..base.clone()
            },
            MailSpec {
                sheet: LabelSheet {
                    across: 9,
                    ..LabelSheet::default()
                },
                ..base.clone()
            },
            MailSpec {
                sheet: LabelSheet {
                    width: 0.0,
                    ..LabelSheet::default()
                },
                ..base.clone()
            },
            MailSpec {
                kind: MailKind::Letters,
                margin_mm: 200.0,
                ..base.clone()
            },
            MailSpec {
                rows: Some((10, 5)),
                ..base
            },
        ];
        for s in cases {
            assert!(d.write_mail(&mut out, &s, &p).is_err(), "{s:?}");
        }
    }
}
