//! X-ray gremlins: the characters in a cell that look right but are not: zero-width and odd spaces, smart
//! quotes, tabs and line breaks inside a cell, and look-alike letters from another script hiding in a word
//! ("Smаrt" with a Cyrillic а). The table of characters is data (`testdata/gremlins.json`, shared with the
//! screen's copy in `ui/src/lib/invisible.ts`; both pass the same corpus), the look-alike pairs follow the
//! idea of Unicode TR39 (confusables) and are written out by hand for the letters people actually meet.
//!
//! A homoglyph is flagged only inside a MIXED-script run: an unbroken run of letters (a hyphen, an
//! apostrophe, a digit or a space ends the run) that holds Latin letters and at least one Cyrillic or Greek
//! letter that has a Latin twin in the table. A run of one script (all Cyrillic, all Greek) is never touched,
//! and a Cyrillic letter with no twin in the table is left alone.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const DATA: &str = include_str!("testdata/gremlins.json");

/// What kind of character was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GremlinKind {
    ZeroWidth,
    OddSpace,
    Control,
    SmartQuote,
    Tab,
    LineBreak,
    Homoglyph,
}

/// One flagged character of a cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gremlin {
    pub kind: GremlinKind,
    /// Byte offset in the text and the character.
    pub at: usize,
    pub ch: char,
    /// What it becomes when sanitized (`None`: nothing, it is removed).
    pub twin: Option<char>,
}

struct Tables {
    confusable: HashMap<char, char>,
    quote: HashMap<char, char>,
    odd: HashMap<char, GremlinKind>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let v: serde_json::Value = serde_json::from_str(DATA).unwrap_or_default();
        let pair = |x: &serde_json::Value| -> Option<(char, char)> {
            let a = x.get(0)?.as_str()?.chars().next()?;
            let b = x.get(1)?.as_str()?.chars().next()?;
            Some((a, b))
        };
        let list = |k: &str| -> Vec<serde_json::Value> {
            v.get(k)
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default()
        };
        let mut odd = HashMap::new();
        for e in list("invisible") {
            if let (Some(c), Some(class)) = (
                e.get(0)
                    .and_then(|x| x.as_str())
                    .and_then(|s| s.chars().next()),
                e.get(2).and_then(|x| x.as_str()),
            ) {
                odd.insert(
                    c,
                    if class == "zero" {
                        GremlinKind::ZeroWidth
                    } else {
                        GremlinKind::OddSpace
                    },
                );
            }
        }
        Tables {
            confusable: list("confusables").iter().filter_map(pair).collect(),
            quote: list("quotes").iter().filter_map(pair).collect(),
            odd,
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
}

/// The script of a letter, for the three scripts that matter here; `None` for a letter of any other script.
fn script_of(c: char) -> Option<Script> {
    match c as u32 {
        0x41..=0x5a | 0x61..=0x7a | 0xc0..=0x24f | 0x1e00..=0x1eff => Some(Script::Latin),
        0x370..=0x3ff | 0x1f00..=0x1fff => Some(Script::Greek),
        0x400..=0x52f => Some(Script::Cyrillic),
        _ => None,
    }
}

/// A combining mark continues the letter before it.
fn is_mark(c: char) -> bool {
    matches!(c as u32, 0x300..=0x36f | 0x483..=0x489 | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff)
}

/// Every flagged character of `text`, in order.
pub fn scan(text: &str, out: &mut Vec<Gremlin>) {
    out.clear();
    let t = tables();
    // Fast exit: plain ASCII text with no control character, tab or line break has nothing to find.
    if text.is_ascii() && !text.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return;
    }
    let mut run: Vec<(usize, char)> = Vec::new();
    let flush = |run: &mut Vec<(usize, char)>, out: &mut Vec<Gremlin>| {
        if run.is_empty() {
            return;
        }
        // A run is changed only when it could become all-Latin: it holds a Latin letter, and EVERY Cyrillic or
        // Greek letter in it has a Latin twin. A word with a letter that has none (И, в, н, θ) really is Cyrillic
        // or Greek, with a stray Latin letter in it: changing the other letters would make the name more broken.
        let latin = run
            .iter()
            .any(|(_, c)| script_of(*c) == Some(Script::Latin));
        let all_twinned = run.iter().all(|(_, c)| {
            !matches!(script_of(*c), Some(Script::Cyrillic | Script::Greek))
                || t.confusable.contains_key(c)
        });
        if latin && all_twinned {
            for (at, c) in run.iter() {
                if matches!(script_of(*c), Some(Script::Cyrillic | Script::Greek)) {
                    if let Some(&twin) = t.confusable.get(c) {
                        out.push(Gremlin {
                            kind: GremlinKind::Homoglyph,
                            at: *at,
                            ch: *c,
                            twin: Some(twin),
                        });
                    }
                }
            }
        }
        run.clear();
    };
    let mut flagged: Vec<Gremlin> = Vec::new();
    for (at, c) in text.char_indices() {
        if c.is_alphabetic() || (is_mark(c) && !run.is_empty()) {
            run.push((at, c));
            continue;
        }
        // A letter run ends here.
        flush(&mut run, out);
        let g = if c == '\t' {
            Some((GremlinKind::Tab, Some(' ')))
        } else if c == '\n' || c == '\r' {
            Some((GremlinKind::LineBreak, Some(' ')))
        } else if let Some(&k) = t.odd.get(&c) {
            Some((
                k,
                if k == GremlinKind::OddSpace {
                    Some(' ')
                } else {
                    None
                },
            ))
        } else if let Some(&q) = t.quote.get(&c) {
            Some((GremlinKind::SmartQuote, Some(q)))
        } else if (c < ' ' && c != '\t' && c != '\n' && c != '\r') || c == '\u{7f}' {
            Some((GremlinKind::Control, None))
        } else {
            None
        };
        if let Some((kind, twin)) = g {
            flagged.push(Gremlin {
                kind,
                at,
                ch: c,
                twin,
            });
        }
    }
    flush(&mut run, out);
    // Everything in text order (the homoglyphs were found run by run).
    out.extend(flagged);
    out.sort_by_key(|g| g.at);
}

/// What to fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SanitizeOptions {
    /// Remove zero-width characters, the byte order mark and soft hyphens.
    pub zero_width: bool,
    /// Odd spaces (no-break, figure, wide, ...) become a plain space.
    pub odd_spaces: bool,
    /// Curly quotes and prime marks become straight quotes.
    pub smart_quotes: bool,
    /// Tabs and line breaks inside a cell become a space (off by default).
    pub tabs_breaks: bool,
    /// Look-alike letters inside a mixed-script word become their Latin twin.
    pub homoglyphs: bool,
}

impl SanitizeOptions {
    /// Everything except tabs and line breaks (the ticked options of the dialog).
    pub fn usual() -> SanitizeOptions {
        SanitizeOptions {
            zero_width: true,
            odd_spaces: true,
            smart_quotes: true,
            tabs_breaks: false,
            homoglyphs: true,
        }
    }
}

/// `text` with the chosen gremlins fixed, or `None` when nothing changes.
pub fn sanitize(text: &str, o: &SanitizeOptions) -> Option<String> {
    let mut found = Vec::new();
    scan(text, &mut found);
    if found.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut changed = false;
    // The "\n" of a "\r\n" pair that was folded into one space together with its "\r".
    let mut skip_to = 0;
    for g in &found {
        if g.at < skip_to {
            continue;
        }
        let fix = match g.kind {
            GremlinKind::ZeroWidth => o.zero_width,
            GremlinKind::OddSpace => o.odd_spaces,
            GremlinKind::SmartQuote => o.smart_quotes,
            GremlinKind::Tab | GremlinKind::LineBreak => o.tabs_breaks,
            GremlinKind::Homoglyph => o.homoglyphs,
            GremlinKind::Control => false,
        };
        if !fix {
            continue;
        }
        out.push_str(&text[last..g.at]);
        if let Some(t) = g.twin {
            out.push(t);
        }
        last = g.at + g.ch.len_utf8();
        // A "\r\n" pair is one break: one space, not two (the user's own double spaces stay).
        if g.ch == '\r' && text[last..].starts_with('\n') {
            last += 1;
            skip_to = last;
        }
        changed = true;
    }
    if !changed {
        return None;
    }
    out.push_str(&text[last..]);
    Some(out)
}

/// How many cells hold each kind of gremlin in one column.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ColumnXray {
    pub col: u32,
    /// Cells with at least one gremlin.
    pub cells: u64,
    pub kinds: Vec<(GremlinKind, u64)>,
}

/// What an X-ray run found over the shown rows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct XrayReport {
    /// Cells looked at, cells with a gremlin, rows with one.
    pub looked: u64,
    pub cells: u64,
    pub rows: u64,
    /// Counts per kind (cells), most first.
    pub kinds: Vec<(GremlinKind, u64)>,
    /// Columns with gremlins, most cells first (at most [`MAX_REPORT_COLS`]).
    pub columns: Vec<ColumnXray>,
    /// Row ids that hold one (one bit each), for "show these rows".
    #[serde(skip)]
    pub failing: Vec<u64>,
}

/// Columns listed in a report.
pub const MAX_REPORT_COLS: usize = 200;

const KINDS: [GremlinKind; 7] = [
    GremlinKind::ZeroWidth,
    GremlinKind::OddSpace,
    GremlinKind::Control,
    GremlinKind::SmartQuote,
    GremlinKind::Tab,
    GremlinKind::LineBreak,
    GremlinKind::Homoglyph,
];

fn kind_index(k: GremlinKind) -> usize {
    KINDS.iter().position(|x| *x == k).unwrap_or(0)
}

use crate::document::{Document, Progress};
use crate::ops::{check_cols, Cols, CHUNK};
use crate::Error;
use rayon::prelude::*;

impl Document {
    /// Look for gremlins in the shown data rows of `cols` (empty: every column): one parallel pass with
    /// progress and Cancel. Nothing is changed and no value leaves the engine: only counts and row ids.
    pub fn xray_report(
        &self,
        cols: &[u32],
        keep_first: bool,
        progress: &Progress,
    ) -> Result<XrayReport, Error> {
        self.ready_for_bulk()?;
        let list: Vec<u32> = if cols.is_empty() {
            (0..self.col_count().min(crate::import::MAX_IMPORT_COLS as u32)).collect()
        } else {
            check_cols(self, cols)?;
            cols.to_vec()
        };
        let reader = Cols::new(self, &list);
        let rows = self.data_rows(keep_first);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1.saturating_sub(r0));
        let n = list.len();
        let chunks = (r1.saturating_sub(r0) as usize).div_ceil(CHUNK);
        // Per chunk: [column][kind] cell counts, cells with any, row ids that hold one.
        type Part = (Vec<[u64; 7]>, Vec<u64>, Vec<u32>);
        let parts: Vec<Part> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Part, Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut counts = vec![[0u64; 7]; n];
                let mut any = vec![0u64; n];
                let mut ids = Vec::new();
                let mut buf = Vec::new();
                let mut found = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let mut hit = false;
                    for k in 0..n {
                        scan(reader.get(&buf, k), &mut found);
                        if found.is_empty() {
                            continue;
                        }
                        hit = true;
                        any[k] += 1;
                        let mut seen = [false; 7];
                        for g in &found {
                            seen[kind_index(g.kind)] = true;
                        }
                        for (i, on) in seen.iter().enumerate() {
                            counts[k][i] += u64::from(*on);
                        }
                    }
                    if hit {
                        ids.push(id);
                    }
                }
                progress.step(e - s)?;
                Ok((counts, any, ids))
            })
            .collect::<Result<_, _>>()?;
        let base = self.next_row_id();
        let mut report = XrayReport {
            looked: (r1.saturating_sub(r0)) * n as u64,
            failing: vec![0u64; (base as usize).div_ceil(64)],
            ..Default::default()
        };
        let mut per_col = vec![[0u64; 7]; n];
        let mut any = vec![0u64; n];
        for (counts, a, ids) in parts {
            for k in 0..n {
                for i in 0..7 {
                    per_col[k][i] += counts[k][i];
                }
                any[k] += a[k];
            }
            for id in ids {
                report.failing[(id >> 6) as usize] |= 1 << (id & 63);
                report.rows += 1;
            }
        }
        let mut totals = [0u64; 7];
        for k in 0..n {
            report.cells += any[k];
            for i in 0..7 {
                totals[i] += per_col[k][i];
            }
            if any[k] > 0 {
                report.columns.push(ColumnXray {
                    col: list[k],
                    cells: any[k],
                    kinds: KINDS
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| per_col[k][*i] > 0)
                        .map(|(i, kd)| (*kd, per_col[k][i]))
                        .collect(),
                });
            }
        }
        report
            .columns
            .sort_by(|a, b| b.cells.cmp(&a.cells).then(a.col.cmp(&b.col)));
        report.columns.truncate(MAX_REPORT_COLS);
        report.kinds = KINDS
            .iter()
            .enumerate()
            .filter(|(i, _)| totals[*i] > 0)
            .map(|(i, kd)| (*kd, totals[i]))
            .collect();
        report.kinds.sort_by_key(|a| std::cmp::Reverse(a.1));
        Ok(report)
    }

    /// A plan that shows only the rows an X-ray run found gremlins in (the title row stays with
    /// `keep_first`): the same row-set route as "show rows with problems".
    pub fn plan_show_xray(
        &self,
        report: &XrayReport,
        keep_first: bool,
    ) -> Result<crate::reshape::RowPlan, Error> {
        let rep = crate::validate::CheckReport {
            rows: report.rows,
            failing: report.failing.clone(),
            ..Default::default()
        };
        self.plan_show_problems(&rep, keep_first, false)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_crlf_pair_is_one_space_and_other_double_spaces_stay() {
        let o = SanitizeOptions {
            tabs_breaks: true,
            ..SanitizeOptions::usual()
        };
        assert_eq!(super::sanitize("a  b\r\nc", &o).as_deref(), Some("a  b c"));
        assert_eq!(super::sanitize("a\r\n\r\nb", &o).as_deref(), Some("a  b"));
        assert_eq!(super::sanitize("a\nb\rc", &o).as_deref(), Some("a b c"));
        // Nothing flagged: the user's own double spaces are not touched.
        assert_eq!(super::sanitize("a  b", &o), None);
    }

    #[test]
    fn a_word_with_a_letter_that_has_no_twin_is_never_changed() {
        let o = SanitizeOptions::usual();
        // Cyrillic with one Latin o typed in: И, в, н have no twin, so the word is Cyrillic and stays.
        assert_eq!(
            super::sanitize("\u{418}\u{432}\u{430}\u{43d}o\u{432}", &o),
            None
        );
        // Greek with a Latin a.
        assert_eq!(super::sanitize("\u{391}\u{3b8}\u{3b7}\u{3bd}a", &o), None);
        // The quote is fixed, the name is not.
        assert_eq!(
            super::sanitize("O\u{2019}\u{418}\u{432}\u{430}\u{43d}\u{43e}\u{432}", &o).as_deref(),
            Some("O'\u{418}\u{432}\u{430}\u{43d}\u{43e}\u{432}")
        );
        // Every letter has a twin: the whole word becomes Latin.
        assert_eq!(super::sanitize("Sm\u{430}rt", &o).as_deref(), Some("Smart"));
    }

    use super::*;

    fn corpus() -> Vec<serde_json::Value> {
        let v: serde_json::Value = serde_json::from_str(DATA).unwrap();
        v["corpus"].as_array().unwrap().clone()
    }

    fn kind_name(k: GremlinKind) -> &'static str {
        match k {
            GremlinKind::ZeroWidth => "zero_width",
            GremlinKind::OddSpace => "odd_space",
            GremlinKind::Control => "control",
            GremlinKind::SmartQuote => "smart_quote",
            GremlinKind::Tab => "tab",
            GremlinKind::LineBreak => "line_break",
            GremlinKind::Homoglyph => "homoglyph",
        }
    }

    /// The shared corpus (also run by the screen's copy): what is flagged, and what a full sanitize gives.
    #[test]
    fn the_shared_corpus() {
        let all = SanitizeOptions {
            zero_width: true,
            odd_spaces: true,
            smart_quotes: true,
            tabs_breaks: false,
            homoglyphs: true,
        };
        for c in corpus() {
            let text = c["text"].as_str().unwrap();
            let mut found = Vec::new();
            scan(text, &mut found);
            let got: Vec<(String, String)> = found
                .iter()
                .map(|g| (kind_name(g.kind).to_string(), g.ch.to_string()))
                .collect();
            let want: Vec<(String, String)> = c["flags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    (
                        f[0].as_str().unwrap().to_string(),
                        f[1].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            assert_eq!(got, want, "{text:?}");
            let twin: Vec<String> = found
                .iter()
                .map(|g| g.twin.map(String::from).unwrap_or_default())
                .collect();
            let want_twin: Vec<String> = c["flags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f[2].as_str().unwrap().to_string())
                .collect();
            assert_eq!(twin, want_twin, "{text:?}");
            let sanitized = sanitize(text, &all).unwrap_or_else(|| text.to_string());
            assert_eq!(sanitized, c["sanitized"].as_str().unwrap(), "{text:?}");
            // Idempotent: sanitizing twice is sanitizing once.
            assert_eq!(sanitize(&sanitized, &all), None, "{text:?}");
        }
    }

    #[test]
    fn a_report_counts_cells_rows_and_kinds() {
        use crate::{Document, OpenOptions, Progress};
        let csv = "name,note\r\nSm\u{430}rt,ok\r\nplain,\u{201c}hi\u{201d}\r\nnone,none\r\n";
        let mut d =
            Document::from_bytes(csv.as_bytes().to_vec(), OpenOptions::default(), Some("csv"))
                .unwrap();
        let r = d.xray_report(&[], true, &Progress::new()).unwrap();
        assert_eq!((r.cells, r.rows), (2, 2));
        assert_eq!(r.columns.len(), 2);
        assert!(r
            .kinds
            .iter()
            .any(|(k, n)| *k == GremlinKind::Homoglyph && *n == 1));
        assert!(r
            .kinds
            .iter()
            .any(|(k, n)| *k == GremlinKind::SmartQuote && *n == 1));
        // Show only those rows (and the title row).
        let plan = d.plan_show_xray(&r, true).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 3);
    }

    #[test]
    fn tabs_and_breaks_only_when_asked() {
        let o = SanitizeOptions {
            tabs_breaks: true,
            ..Default::default()
        };
        assert_eq!(sanitize("a\tb\r\nc\nd", &o).as_deref(), Some("a b c d"));
        assert_eq!(sanitize("a\tb", &SanitizeOptions::usual()), None);
    }

    #[test]
    fn every_option_works_alone() {
        let text = "Sm\u{430}rt\u{200b} \u{201c}x\u{201d}\u{a0}y";
        let only = |f: fn(&mut SanitizeOptions)| {
            let mut o = SanitizeOptions::default();
            f(&mut o);
            sanitize(text, &o)
        };
        assert!(only(|o| o.zero_width = true)
            .unwrap()
            .contains("Sm\u{430}rt"));
        assert!(!only(|o| o.zero_width = true).unwrap().contains('\u{200b}'));
        assert!(only(|o| o.smart_quotes = true).unwrap().contains("\"x\""));
        assert!(only(|o| o.odd_spaces = true).unwrap().ends_with(" y"));
        assert!(only(|o| o.homoglyphs = true).unwrap().contains("Smart"));
        assert_eq!(sanitize(text, &SanitizeOptions::default()), None);
    }

    #[test]
    fn long_text_is_linear_and_exact() {
        let text = "Sm\u{430}rt ".repeat(20_000);
        let mut found = Vec::new();
        scan(&text, &mut found);
        assert_eq!(found.len(), 20_000);
        let s = sanitize(&text, &SanitizeOptions::usual()).unwrap();
        assert_eq!(s, "Smart ".repeat(20_000));
    }
}
