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
            v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default()
        };
        let mut odd = HashMap::new();
        for e in list("invisible") {
            if let (Some(c), Some(class)) = (
                e.get(0).and_then(|x| x.as_str()).and_then(|s| s.chars().next()),
                e.get(2).and_then(|x| x.as_str()),
            ) {
                odd.insert(c, if class == "zero" { GremlinKind::ZeroWidth } else { GremlinKind::OddSpace });
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
    let mut run_start: Option<usize> = None;
    let mut run: Vec<(usize, char)> = Vec::new();
    let flush = |run: &mut Vec<(usize, char)>, out: &mut Vec<Gremlin>| {
        if run.is_empty() {
            return;
        }
        let latin = run.iter().any(|(_, c)| script_of(*c) == Some(Script::Latin));
        if latin {
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
            run_start.get_or_insert(at);
            run.push((at, c));
            continue;
        }
        // A letter run ends here.
        let before = out.len();
        flush(&mut run, out);
        let _ = before;
        run_start = None;
        let g = if c == '\t' {
            Some((GremlinKind::Tab, Some(' ')))
        } else if c == '\n' || c == '\r' {
            Some((GremlinKind::LineBreak, Some(' ')))
        } else if let Some(&k) = t.odd.get(&c) {
            Some((k, if k == GremlinKind::OddSpace { Some(' ') } else { None }))
        } else if let Some(&q) = t.quote.get(&c) {
            Some((GremlinKind::SmartQuote, Some(q)))
        } else if (c < ' ' && c != '\t' && c != '\n' && c != '\r') || c == '\u{7f}' {
            Some((GremlinKind::Control, None))
        } else {
            None
        };
        if let Some((kind, twin)) = g {
            flagged.push(Gremlin { kind, at, ch: c, twin });
        }
    }
    flush(&mut run, out);
    let _ = run_start;
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
    for g in &found {
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
        changed = true;
    }
    if !changed {
        return None;
    }
    out.push_str(&text[last..]);
    // A "\r\n" pair is one break: two spaces would be wrong.
    if o.tabs_breaks && text.contains("\r\n") {
        out = out.replace("  ", " ");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
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
                .map(|f| (f[0].as_str().unwrap().to_string(), f[1].as_str().unwrap().to_string()))
                .collect();
            assert_eq!(got, want, "{text:?}");
            let twin: Vec<String> = found.iter().map(|g| g.twin.map(String::from).unwrap_or_default()).collect();
            let want_twin: Vec<String> = c["flags"].as_array().unwrap().iter().map(|f| f[2].as_str().unwrap().to_string()).collect();
            assert_eq!(twin, want_twin, "{text:?}");
            let sanitized = sanitize(text, &all).unwrap_or_else(|| text.to_string());
            assert_eq!(sanitized, c["sanitized"].as_str().unwrap(), "{text:?}");
            // Idempotent: sanitizing twice is sanitizing once.
            assert_eq!(sanitize(&sanitized, &all), None, "{text:?}");
        }
    }

    #[test]
    fn tabs_and_breaks_only_when_asked() {
        let o = SanitizeOptions { tabs_breaks: true, ..Default::default() };
        assert_eq!(sanitize("a\tb\r\nc\nd", &o).as_deref(), Some("a b c d"));
        assert_eq!(sanitize("a\tb", &SanitizeOptions::usual()), None);
    }

    #[test]
    fn every_option_works_alone() {
        let text = "S\u{43c}\u{430}rt\u{200b} \u{201c}x\u{201d}\u{a0}y";
        let only = |f: fn(&mut SanitizeOptions)| {
            let mut o = SanitizeOptions::default();
            f(&mut o);
            sanitize(text, &o)
        };
        assert!(only(|o| o.zero_width = true).unwrap().contains('м'));
        assert!(!only(|o| o.zero_width = true).unwrap().contains('\u{200b}'));
        assert!(only(|o| o.smart_quotes = true).unwrap().contains("\"x\""));
        assert!(only(|o| o.odd_spaces = true).unwrap().ends_with(" y"));
        assert!(only(|o| o.homoglyphs = true).unwrap().contains("Smart") || only(|o| o.homoglyphs = true).unwrap().contains("Sмart"));
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
