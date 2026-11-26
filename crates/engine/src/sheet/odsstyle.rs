//! Zero-padding number formats of an `.ods` file: which cell styles show a
//! number as `00123` or `000-00-0000`.
//!
//! A cell's style is looked up in the order of OpenDocument 1.3 part 3,
//! `table:default-cell-style-name` (see also OASIS issue OFFICE-4004): the
//! cell's own `table:style-name`, else the row's default cell style, else the
//! column's; `style:default-style` of family `table-cell` is the last
//! fallback, also for a style chain that names no data style.

use super::*;
use std::cell::RefCell;

/// Styles kept at most (as many patterns as an `.xlsx` can have).
const MAX_STYLES: usize = u16::MAX as usize;
const MAX_NAME: usize = 256;
/// How far a chain of parent styles is followed (a cycle ends here too).
const MAX_PARENTS: usize = 16;
/// A pattern longer than this is not a padding format.
const MAX_DIGITS: usize = 30;
/// The longest padded text: the digits and the literals a number style can hold.
pub(super) const LONGEST: usize = MAX_DIGITS * 4;

/// A table-cell style: its data style and its parent.
#[derive(Default, Clone)]
struct CellStyle {
    data: Option<String>,
    parent: Option<String>,
}

#[derive(Default)]
pub(super) struct OdsStyles {
    pub pads: Vec<Pad>,
    /// Data style name → pattern, for the padding formats only.
    numbers: HashMap<String, u16>,
    cells: HashMap<String, CellStyle>,
    default_data: Option<String>,
    found: RefCell<HashMap<String, Option<u16>>>,
}

/// A number style being read.
#[derive(Default)]
struct NumberStyle {
    name: String,
    ok: bool,
    prefix: String,
    suffix: String,
    digits: usize,
    /// (digits to the right of it, text)
    embedded: Vec<(usize, String)>,
}

fn literal(s: &str) -> bool {
    s.chars().all(|c| matches!(c, '-' | ' ' | '\u{a0}'))
}

impl NumberStyle {
    fn pad(&self) -> Option<Pad> {
        if !self.ok || !(2..=MAX_DIGITS).contains(&self.digits) {
            return None;
        }
        let mut parts: Vec<Option<char>> = Vec::new();
        let chars = |s: &str| {
            s.chars()
                .map(|c| Some(if c == '\u{a0}' { ' ' } else { c }))
                .collect::<Vec<_>>()
        };
        let literals: usize = self.prefix.len()
            + self.suffix.len()
            + self.embedded.iter().map(|e| e.1.len()).sum::<usize>();
        if !literal(&self.prefix) || !literal(&self.suffix) || literals > MAX_DIGITS * 2 {
            return None;
        }
        parts.extend(chars(&self.prefix));
        // Positions count digits leftward from the decimal point.
        for right in (0..=self.digits).rev() {
            for (at, text) in &self.embedded {
                if *at == right {
                    parts.extend(chars(text));
                }
            }
            if right > 0 {
                parts.push(None);
            }
        }
        if self
            .embedded
            .iter()
            .any(|(at, t)| *at > self.digits || !literal(t))
        {
            return None;
        }
        parts.extend(chars(&self.suffix));
        Some(Pad {
            parts,
            digits: self.digits,
        })
    }
}

impl OdsStyles {
    /// Read the styles of one XML part (`styles.xml`, or `content.xml` up to
    /// its body). Damaged XML ends the reading: what was found stays.
    pub fn read<R: std::io::Read>(&mut self, part: R) {
        let mut r = xml_reader(part);
        let mut buf = Vec::new();
        let mut depth = 0usize;
        let mut number: Option<(usize, NumberStyle)> = None;
        // Inside number:text / number:embedded-text: where the text goes.
        let mut text_at: Option<(usize, Option<usize>)> = None;
        loop {
            let Ok(ev) = r.read_event_into(&mut buf) else {
                return;
            };
            let empty = matches!(ev, Event::Empty(_));
            match ev {
                Event::Start(ref e) | Event::Empty(ref e) => {
                    if !empty {
                        depth += 1;
                        if depth > MAX_DEPTH {
                            return;
                        }
                    }
                    let name = local(e);
                    if let Some((_, n)) = number.as_mut() {
                        match name.as_str() {
                            "number" => {
                                let no = |a: &str, want: &str| attr(e, a).is_none_or(|v| v == want);
                                let digits = attr(e, "min-integer-digits")
                                    .and_then(|v| v.parse::<usize>().ok());
                                n.ok = n.digits == 0
                                    && attr(e, "decimal-places").as_deref() == Some("0")
                                    && no("min-decimal-places", "0")
                                    && no("grouping", "false")
                                    && attr(e, "display-factor").is_none()
                                    && attr(e, "decimal-replacement").is_none();
                                n.digits = digits.unwrap_or(0).min(MAX_DIGITS + 1);
                            }
                            "embedded-text" => {
                                match attr(e, "position").and_then(|v| v.parse().ok()) {
                                    Some(at) if n.embedded.len() < MAX_DIGITS => {
                                        n.embedded.push((at, String::new()));
                                        if !empty {
                                            text_at = Some((depth, Some(n.embedded.len() - 1)));
                                        }
                                    }
                                    _ => n.ok = false,
                                }
                            }
                            "text" => {
                                if !empty {
                                    text_at = Some((depth, None));
                                }
                            }
                            // Colours and conditions: the base format only.
                            "text-properties" | "map" => {}
                            // Anything else (a currency sign, a fraction, an
                            // exponent) is not a padding format.
                            _ => n.digits = MAX_DIGITS + 1,
                        }
                    } else {
                        match name.as_str() {
                            "body" => return,
                            "number-style" if !empty => {
                                let n = NumberStyle {
                                    name: attr(e, "name").unwrap_or_default(),
                                    ..Default::default()
                                };
                                number = Some((depth, n));
                            }
                            "style" | "default-style"
                                if attr(e, "family").as_deref() == Some("table-cell") =>
                            {
                                let data = attr(e, "data-style-name");
                                if name == "default-style" {
                                    self.default_data = data;
                                } else if let Some(n) = attr(e, "name") {
                                    if n.len() <= MAX_NAME && self.cells.len() < MAX_STYLES {
                                        let parent = attr(e, "parent-style-name");
                                        self.cells.insert(n, CellStyle { data, parent });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Text(ref t) => {
                    if let (Some((_, n)), Some((_, at))) = (number.as_mut(), text_at) {
                        let text = text_of(t);
                        let to = match at {
                            Some(i) => &mut n.embedded[i].1,
                            None if n.digits == 0 => &mut n.prefix,
                            None => &mut n.suffix,
                        };
                        if to.len() + text.len() <= MAX_NAME {
                            to.push_str(&text);
                        } else {
                            n.ok = false;
                            n.digits = MAX_DIGITS + 1;
                        }
                    }
                }
                Event::End(_) => {
                    if text_at.is_some_and(|(d, _)| d == depth) {
                        text_at = None;
                    }
                    if number.as_ref().is_some_and(|(d, _)| *d == depth) {
                        if let Some((_, n)) = number.take() {
                            if let Some(p) = n.pad() {
                                if self.pads.len() < MAX_STYLES && n.name.len() <= MAX_NAME {
                                    self.numbers.insert(n.name, self.pads.len() as u16);
                                    self.pads.push(p);
                                }
                            }
                        }
                    }
                    depth = depth.saturating_sub(1);
                }
                Event::Eof => return,
                _ => {}
            }
            buf.clear();
        }
    }

    /// The padding pattern of cell style `name`, through its parents.
    pub fn pad_of(&self, name: &str) -> Option<u16> {
        if let Some(found) = self.found.borrow().get(name) {
            return *found;
        }
        let mut data = None;
        let mut at = name;
        for _ in 0..MAX_PARENTS {
            let Some(style) = self.cells.get(at) else {
                break;
            };
            if style.data.is_some() {
                data = style.data.as_deref();
                break;
            }
            let Some(parent) = style.parent.as_deref() else {
                break;
            };
            at = parent;
        }
        let found = data
            .or(self.default_data.as_deref())
            .and_then(|d| self.numbers.get(d).copied());
        if self.cells.contains_key(name) {
            self.found.borrow_mut().insert(name.to_string(), found);
        }
        found
    }

    /// The pattern of a cell no style names at all.
    pub fn default_pad(&self) -> Option<u16> {
        self.default_data
            .as_deref()
            .and_then(|d| self.numbers.get(d).copied())
    }
}
