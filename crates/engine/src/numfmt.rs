//! Display format codes: a closed subset of Excel's number format codes, and
//! the time and date-and-time display formats. The grid draws with the same
//! rules (`ui/src/lib/numfmt.ts`); both run `tests/data/numfmt-cases.json`,
//! so what the screen shows and what "Write the display format into the
//! cells" writes cannot disagree.
//!
//! Grammar, in short: up to four sections split by `;` (positive; negative;
//! zero; text), digit slots `0 # ?`, a decimal point, thousands and scaling
//! commas, `%`, `"text"`, `\x`, a few literal characters, `[Red]`-style
//! colours, up to two conditions like `[>100]`, the date and time letters
//! `y m d h s AM/PM A/P`, and `@` in the text section. Anything else is
//! refused with its position. Values are read from the cell's text exactly,
//! as decimal digits (no binary rounding), and rounded half away from zero.

/// Longest code, in characters.
pub const MAX_CODE: usize = 200;
const MAX_SECTIONS: usize = 4;
/// Digit slots after the point, and before it, in one section.
pub const MAX_FRAC: usize = 30;
pub const MAX_INT: usize = 30;
/// Trailing commas (each divides by 1,000).
pub const MAX_SCALE: usize = 4;
pub const MAX_PCT: usize = 2;
pub const MAX_CONDS: usize = 2;
/// Significant digits a number in a cell (or a condition) may have and still
/// be read as a number: past that, a double would change digits.
pub const MAX_DIGITS: usize = 15;

/// A section's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Red,
    Blue,
    Green,
    Black,
    White,
    Yellow,
    Magenta,
    Cyan,
}

impl Color {
    pub fn name(self) -> &'static str {
        match self {
            Color::Red => "red",
            Color::Blue => "blue",
            Color::Green => "green",
            Color::Black => "black",
            Color::White => "white",
            Color::Yellow => "yellow",
            Color::Magenta => "magenta",
            Color::Cyan => "cyan",
        }
    }
}

const COLORS: [(&str, Color); 8] = [
    ("red", Color::Red),
    ("blue", Color::Blue),
    ("green", Color::Green),
    ("black", Color::Black),
    ("white", Color::White),
    ("yellow", Color::Yellow),
    ("magenta", Color::Magenta),
    ("cyan", Color::Cyan),
];

/// Why a code is refused: the message the dialog shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeError {
    /// 1-based character position, when there is one.
    pub at: Option<usize>,
    pub reason: String,
}

impl std::fmt::Display for CodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.at {
            Some(n) => write!(f, "Not supported at character {n}: {}.", self.reason),
            None => write!(f, "Not supported: {}.", self.reason),
        }
    }
}

fn err<T>(at: Option<usize>, reason: impl Into<String>) -> Result<T, CodeError> {
    Err(CodeError {
        at,
        reason: reason.into(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Lit(String),
    /// `0`, `#` or `?`.
    Slot(char),
    Point,
    Comma,
    Group,
    Scale,
    Pct,
    Text,
    /// Two or four digits.
    Year(bool),
    Month(u8),
    Minute(u8),
    Day(u8),
    Hour(u8),
    Second(u8),
    /// AM/PM, upper case.
    AmPm(bool),
    /// A/P, upper case.
    Ap(bool),
}

impl Tok {
    fn is_date(&self) -> bool {
        matches!(
            self,
            Tok::Year(_)
                | Tok::Month(_)
                | Tok::Minute(_)
                | Tok::Day(_)
                | Tok::Hour(_)
                | Tok::Second(_)
                | Tok::AmPm(_)
                | Tok::Ap(_)
        )
    }
    fn is_number(&self) -> bool {
        matches!(self, Tok::Slot(_) | Tok::Pct)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Number,
    Date,
    Text,
    /// Only literals.
    Plain,
}

#[derive(Clone, Debug)]
struct Section {
    toks: Vec<Tok>,
    color: Option<Color>,
    cond: Option<(Op, f64)>,
    kind: Kind,
    /// Number sections: index of the decimal point in `toks`.
    point: Option<usize>,
    grouping: bool,
    /// Net power of ten the value is moved by (`%` +2, a scaling comma -3).
    shift: i32,
    frac: usize,
    /// Date sections: the hour is on a 12-hour clock.
    twelve: bool,
    needs_date: bool,
    needs_time: bool,
    /// Number sections: the integer and decimal slots (indexes into `toks`).
    int_slots: Vec<usize>,
    frac_slots: Vec<usize>,
}

/// A compiled code.
#[derive(Clone, Debug)]
pub struct Code {
    sections: Vec<Section>,
    conds: bool,
}

/// How a cell shows: its text and, for a custom code, a colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    pub color: Option<Color>,
}

fn sig_digits(s: &str) -> usize {
    // Digits after the leading zeros; with a point, trailing zeros do not count.
    let (mut n, mut zeros, mut lead, mut point) = (0, 0, true, false);
    for b in s.bytes() {
        if b == b'.' {
            point = true;
        }
        if !b.is_ascii_digit() || (lead && b == b'0') {
            continue;
        }
        lead = false;
        n += 1;
        zeros = if b == b'0' { zeros + 1 } else { 0 };
    }
    if point {
        n - zeros
    } else {
        n
    }
}

/// `-?(\d+\.?\d*|\.\d+)` with at most 15 significant digits.
fn cond_number(s: &str) -> Option<f64> {
    let body = s.strip_prefix('-').unwrap_or(s);
    let mut seen_point = false;
    let mut digits = 0;
    for c in body.chars() {
        if c == '.' && !seen_point {
            seen_point = true;
        } else if c.is_ascii_digit() {
            digits += 1;
        } else {
            return None;
        }
    }
    if digits == 0 || sig_digits(body) > MAX_DIGITS {
        return None;
    }
    let v: f64 = s.parse().ok()?;
    v.is_finite().then_some(v)
}

fn bracket(inner: &str, at: usize, sec: &mut Section, conds: &mut usize) -> Result<(), CodeError> {
    let l = inner.trim().to_lowercase();
    if l.starts_with('$') {
        return err(Some(at), "locale and currency tags ([$...])");
    }
    if matches!(l.as_str(), "h" | "hh" | "m" | "mm" | "s" | "ss") {
        return err(Some(at), "elapsed time ([h], [m], [s])");
    }
    if let Some(&(_, c)) = COLORS.iter().find(|(n, _)| *n == l) {
        if sec.color.is_some() {
            return err(Some(at), "more than one colour in a section");
        }
        sec.color = Some(c);
        return Ok(());
    }
    let ops: [(&str, Op); 6] = [
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("<>", Op::Ne),
        ("<", Op::Lt),
        (">", Op::Gt),
        ("=", Op::Eq),
    ];
    if let Some(&(p, op)) = ops.iter().find(|(p, _)| l.starts_with(p)) {
        if sec.cond.is_some() {
            return err(Some(at), "more than one condition in a section");
        }
        *conds += 1;
        if *conds > MAX_CONDS {
            return err(Some(at), "more than 2 conditions");
        }
        let Some(v) = cond_number(l[p.len()..].trim()) else {
            return err(
                Some(at),
                "a condition number (at most 15 digits, like [>100] or [<=-2.5])",
            );
        };
        sec.cond = Some((op, v));
        return Ok(());
    }
    err(Some(at), "an unknown [...] part")
}

const LITERALS: &[char] = &[
    '$', '-', '+', '(', ')', ':', '/', '\'', ' ', '€', '£', '¥', '¢',
];

fn new_section() -> Section {
    Section {
        toks: Vec::new(),
        color: None,
        cond: None,
        kind: Kind::Plain,
        point: None,
        grouping: false,
        shift: 0,
        frac: 0,
        twelve: false,
        needs_date: false,
        needs_time: false,
        int_slots: Vec::new(),
        frac_slots: Vec::new(),
    }
}

impl Code {
    /// Compile a code, or say what part of it is not supported.
    pub fn compile(code: &str) -> Result<Code, CodeError> {
        let s: Vec<char> = code.chars().collect();
        if js_trim(code).is_empty() {
            return err(None, "the code is empty");
        }
        if s.len() > MAX_CODE {
            return err(None, "the code is longer than 200 characters");
        }
        let mut sections = vec![new_section()];
        // Positions (1-based) of each section's tokens, for errors found later.
        let mut pos: Vec<Vec<usize>> = vec![Vec::new()];
        let mut conds = 0;
        let mut i = 0;
        while i < s.len() {
            let c = s[i];
            let at = i + 1;
            let sec = sections.last_mut().unwrap();
            let p = pos.last_mut().unwrap();
            let started = !sec.toks.is_empty();
            let mut push = |sec: &mut Section, t: Tok| {
                sec.toks.push(t);
                p.push(at);
            };
            match c {
                ';' => {
                    if sections.len() == MAX_SECTIONS {
                        return err(Some(at), "more than 4 sections");
                    }
                    sections.push(new_section());
                    pos.push(Vec::new());
                    i += 1;
                    continue;
                }
                '"' => {
                    let Some(j) = s[i + 1..].iter().position(|&q| q == '"') else {
                        return err(Some(at), "an unclosed quote");
                    };
                    let lit: String = s[i + 1..i + 1 + j].iter().collect();
                    push(sec, Tok::Lit(lit));
                    i += j + 2;
                    continue;
                }
                '\\' => {
                    let Some(&n) = s.get(i + 1) else {
                        return err(Some(at), "a backslash at the end");
                    };
                    push(sec, Tok::Lit(n.to_string()));
                    i += 2;
                    continue;
                }
                '[' => {
                    let Some(j) = s[i + 1..].iter().position(|&q| q == ']') else {
                        return err(Some(at), "an unclosed [");
                    };
                    if started {
                        return err(
                            Some(at),
                            "a colour or condition after the start of a section",
                        );
                    }
                    let inner: String = s[i + 1..i + 1 + j].iter().collect();
                    bracket(&inner, at, sec, &mut conds)?;
                    i += j + 2;
                    continue;
                }
                '0' | '#' | '?' => {
                    if s.get(i + 1) == Some(&'/') {
                        return err(Some(at), "fractions (?/?)");
                    }
                    push(sec, Tok::Slot(c));
                }
                '.' => push(sec, Tok::Point),
                ',' => push(sec, Tok::Comma),
                '%' => push(sec, Tok::Pct),
                '@' => push(sec, Tok::Text),
                '*' => return err(Some(at), "fill (*)"),
                '_' => return err(Some(at), "spacing (_)"),
                'E' | 'e' if matches!(s.get(i + 1), Some('+') | Some('-')) => {
                    return err(Some(at), "scientific notation (E+)");
                }
                'a' | 'A' => {
                    let rest: String = s[i..s.len().min(i + 5)]
                        .iter()
                        .collect::<String>()
                        .to_lowercase();
                    if rest == "am/pm" {
                        push(sec, Tok::AmPm(c == 'A'));
                        i += 5;
                        continue;
                    }
                    if rest.starts_with("a/p") {
                        push(sec, Tok::Ap(c == 'A'));
                        i += 3;
                        continue;
                    }
                    return err(Some(at), format!("the character '{c}'"));
                }
                'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                    let l = c.to_ascii_lowercase();
                    let n = s[i..]
                        .iter()
                        .take_while(|q| q.to_ascii_lowercase() == l)
                        .count();
                    let max = match l {
                        'y' | 'd' => 4,
                        'm' => 5,
                        _ => 2,
                    };
                    if n > max {
                        return err(Some(at), format!("a run of {n} '{l}'"));
                    }
                    let n8 = n as u8;
                    let t = match l {
                        'y' => Tok::Year(n >= 3),
                        'm' => Tok::Month(n8),
                        'd' => Tok::Day(n8),
                        'h' => Tok::Hour(n8),
                        _ => Tok::Second(n8),
                    };
                    push(sec, t);
                    i += n;
                    continue;
                }
                c if LITERALS.contains(&c) => push(sec, Tok::Lit(c.to_string())),
                c => return err(Some(at), format!("the character '{c}'")),
            }
            i += 1;
        }
        let n = sections.len();
        for (k, (sec, p)) in sections.iter_mut().zip(&pos).enumerate() {
            finish(sec, p, k, n)?;
        }
        Ok(Code {
            conds: sections.iter().any(|s| s.cond.is_some()),
            sections,
        })
    }

    /// The code has one section, of dates and times only.
    pub fn is_date_only(&self) -> bool {
        self.sections.len() == 1 && self.sections[0].kind == Kind::Date
    }

    /// How `text` shows under this code: `None` when it shows as it is.
    pub fn render(&self, text: &str) -> Option<Shown> {
        if js_trim(text).is_empty() {
            return None;
        }
        let secs = &self.sections;
        // One section of text only: every cell.
        if secs.len() == 1 && secs[0].kind == Kind::Text {
            return text_section(&secs[0], text);
        }
        if let Some(d) = read_decimal(text) {
            let (sec, minus) = self.pick(&d)?;
            return match sec.kind {
                Kind::Number | Kind::Plain => Some(Shown {
                    text: number_section(sec, &d, minus)?,
                    color: sec.color,
                }),
                _ => None,
            };
        }
        if secs[0].kind == Kind::Date {
            if let Some(st) = read_stamp(text) {
                return Some(Shown {
                    text: date_section(&secs[0], &st)?,
                    color: secs[0].color,
                });
            }
        }
        if secs.len() == 4 {
            return text_section(&secs[3], text);
        }
        None
    }

    /// The section a number uses, and whether it writes its own minus.
    fn pick(&self, d: &Dec) -> Option<(&Section, bool)> {
        let secs = &self.sections;
        let nums = if secs.len() == 4 { 3 } else { secs.len() };
        if self.conds {
            let v = d.value();
            for s in &secs[..nums] {
                if let Some((op, c)) = s.cond {
                    let hit = match op {
                        Op::Lt => v < c,
                        Op::Le => v <= c,
                        Op::Gt => v > c,
                        Op::Ge => v >= c,
                        Op::Eq => v == c,
                        Op::Ne => v != c,
                    };
                    if hit {
                        return Some((s, true));
                    }
                }
            }
            return secs[..nums]
                .iter()
                .find(|s| s.cond.is_none())
                .map(|s| (s, true));
        }
        let zero = d.digits.is_empty();
        Some(match nums {
            1 => (&secs[0], true),
            2 => (if d.neg && !zero { &secs[1] } else { &secs[0] }, false),
            _ => (
                if zero {
                    &secs[2]
                } else if d.neg {
                    &secs[1]
                } else {
                    &secs[0]
                },
                false,
            ),
        })
    }
}

/// Settle a section's kind and its commas, and check its limits.
fn finish(sec: &mut Section, p: &[usize], k: usize, n: usize) -> Result<(), CodeError> {
    let first = |f: &dyn Fn(&Tok) -> bool| sec.toks.iter().position(f).map(|i| p[i]);
    let has_num = sec.toks.iter().any(Tok::is_number);
    let has_date = sec.toks.iter().any(Tok::is_date);
    let text_at = first(&|t| *t == Tok::Text);
    let text_section = k == 3 || n == 1;
    if let Some(at) = text_at {
        if !text_section || has_num || has_date {
            return err(Some(at), "text (@) outside the text section");
        }
    }
    if k == 3 {
        if let Some(at) = first(&|t| t.is_number() || t.is_date()) {
            return err(Some(at), "number or date parts in the text section");
        }
        if sec.cond.is_some() {
            return err(None, "a condition in the text section");
        }
    }
    if has_num && has_date {
        let at = first(&Tok::is_date).unwrap_or(0);
        return err(Some(at), "number and date parts in one section");
    }
    if text_at.is_some() {
        sec.kind = Kind::Text;
        for t in &mut sec.toks {
            match t {
                Tok::Point => *t = Tok::Lit(".".into()),
                Tok::Comma => *t = Tok::Lit(",".into()),
                _ => {}
            }
        }
        return Ok(());
    }
    if has_date {
        sec.kind = Kind::Date;
        date_minutes(sec);
        sec.twelve = sec
            .toks
            .iter()
            .any(|t| matches!(t, Tok::AmPm(_) | Tok::Ap(_)));
        sec.needs_date = sec
            .toks
            .iter()
            .any(|t| matches!(t, Tok::Year(_) | Tok::Month(_) | Tok::Day(_)));
        sec.needs_time = sec.toks.iter().any(|t| {
            matches!(
                t,
                Tok::Hour(_) | Tok::Minute(_) | Tok::Second(_) | Tok::AmPm(_) | Tok::Ap(_)
            )
        });
        for t in &mut sec.toks {
            match t {
                Tok::Point => *t = Tok::Lit(".".into()),
                Tok::Comma => *t = Tok::Lit(",".into()),
                _ => {}
            }
        }
        return Ok(());
    }
    let slots = sec.toks.iter().any(|t| matches!(t, Tok::Slot(_)));
    if !slots {
        // Only literals (and perhaps %): nothing is read from the value.
        sec.kind = Kind::Plain;
        for t in &mut sec.toks {
            match t {
                Tok::Point => *t = Tok::Lit(".".into()),
                Tok::Comma => *t = Tok::Lit(",".into()),
                Tok::Pct => *t = Tok::Lit("%".into()),
                _ => {}
            }
        }
        return Ok(());
    }
    sec.kind = Kind::Number;
    let mut point = None;
    for (i, t) in sec.toks.iter().enumerate() {
        if *t == Tok::Point {
            if point.is_some() {
                return err(Some(p[i]), "two decimal points");
            }
            point = Some(i);
        }
    }
    sec.point = point;
    let end_int = point.unwrap_or(sec.toks.len());
    let is_slot = |t: &Tok| matches!(t, Tok::Slot(_));
    // Commas: between integer slots group; straight after the last slot of a part they scale.
    let toks = sec.toks.clone();
    let mut scale = 0usize;
    for i in 0..toks.len() {
        if toks[i] != Tok::Comma {
            continue;
        }
        let (lo, hi) = if i < end_int {
            (0, end_int)
        } else {
            (end_int + 1, toks.len())
        };
        let before = toks[lo..i].iter().any(is_slot);
        let after = toks[i + 1..hi].iter().any(is_slot);
        let run_from_slot = {
            let mut j = i;
            while j > lo && toks[j - 1] == Tok::Comma {
                j -= 1;
            }
            j > lo && is_slot(&toks[j - 1])
        };
        sec.toks[i] = if i < end_int && before && after {
            sec.grouping = true;
            Tok::Group
        } else if before && !after && run_from_slot {
            scale += 1;
            if scale > MAX_SCALE {
                return err(Some(p[i]), "more than 4 commas for thousands");
            }
            Tok::Scale
        } else {
            Tok::Lit(",".into())
        };
    }
    let int_slots: Vec<usize> = (0..end_int).filter(|&i| is_slot(&sec.toks[i])).collect();
    let frac_slots: Vec<usize> = (end_int..sec.toks.len())
        .filter(|&i| is_slot(&sec.toks[i]))
        .collect();
    if int_slots.len() > MAX_INT {
        return err(
            Some(p[int_slots[MAX_INT]]),
            "more than 30 digits before the point",
        );
    }
    if frac_slots.len() > MAX_FRAC {
        return err(
            Some(p[frac_slots[MAX_FRAC]]),
            "more than 30 digits after the point",
        );
    }
    let pcts: Vec<usize> = (0..sec.toks.len())
        .filter(|&i| sec.toks[i] == Tok::Pct)
        .collect();
    if pcts.len() > MAX_PCT {
        return err(Some(p[pcts[MAX_PCT]]), "more than 2 percent signs");
    }
    if sec.grouping {
        if let (Some(&a), Some(&b)) = (int_slots.first(), int_slots.last()) {
            if let Some(i) = (a..b).find(|&i| !is_slot(&sec.toks[i]) && sec.toks[i] != Tok::Group) {
                return err(Some(p[i]), "thousands commas with text between the digits");
            }
        }
    }
    sec.frac = frac_slots.len();
    sec.shift = 2 * pcts.len() as i32 - 3 * scale as i32;
    sec.int_slots = int_slots;
    sec.frac_slots = frac_slots;
    Ok(())
}

/// `m` straight after an hour or straight before a second is minutes.
fn date_minutes(sec: &mut Section) {
    let dates: Vec<usize> = (0..sec.toks.len())
        .filter(|&i| sec.toks[i].is_date())
        .collect();
    for (k, &i) in dates.iter().enumerate() {
        if let Tok::Month(n) = sec.toks[i] {
            if n > 2 {
                continue;
            }
            let prev_hour = k > 0 && matches!(sec.toks[dates[k - 1]], Tok::Hour(_));
            let next_sec = dates
                .get(k + 1)
                .is_some_and(|&j| matches!(sec.toks[j], Tok::Second(_)));
            if prev_hour || next_sec {
                sec.toks[i] = Tok::Minute(n);
            }
        }
    }
}

fn text_section(sec: &Section, text: &str) -> Option<Shown> {
    let mut out = String::new();
    for t in &sec.toks {
        match t {
            Tok::Lit(l) => out.push_str(l),
            Tok::Text => out.push_str(text),
            _ => {}
        }
    }
    Some(Shown {
        text: out,
        color: sec.color,
    })
}

/// A number written in a cell, exactly: value = 0.DIGITS × 10^exp.
#[derive(Clone, Debug, PartialEq)]
pub struct Dec {
    pub neg: bool,
    /// No leading or trailing zeros; empty for zero.
    pub digits: Vec<u8>,
    pub exp: i32,
    /// Written with a percent sign (the value is a fraction).
    pub pct: bool,
}

impl Dec {
    /// The number as a double, as the grid reads it (`Number(text) * 0.01`
    /// for a percent): for conditions only, so worked out when asked.
    pub fn value(&self) -> f64 {
        if self.digits.is_empty() {
            return 0.0;
        }
        let ds: String = self.digits.iter().map(|d| (b'0' + d) as char).collect();
        let e = self.exp + if self.pct { 2 } else { 0 };
        let v: f64 = format!("0.{ds}e{e}").parse().unwrap_or(0.0);
        let v = if self.pct { v * 0.01 } else { v };
        if self.neg {
            -v
        } else {
            v
        }
    }
}

/// JavaScript's white space (`trim`, `\s`), so the grid and the engine trim alike.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

/// The numbers the display formats read (as `readNumber` in m9.ts): plain,
/// with thousands commas, a currency sign, a percent sign (a fraction), or
/// in brackets for minus; at most 15 significant digits.
pub fn read_decimal(text: &str) -> Option<Dec> {
    let mut t = js_trim(text);
    if t.is_empty() || t.encode_utf16().count() > 64 {
        return None;
    }
    if sig_digits(t.split(['e', 'E']).next().unwrap_or("")) > MAX_DIGITS {
        return None;
    }
    let mut neg = false;
    if t.len() >= 2 && t.starts_with('(') && t.ends_with(')') {
        neg = true;
        t = js_trim(&t[1..t.len() - 1]);
    }
    let mut pct = false;
    if let Some(r) = t.strip_suffix('%') {
        pct = true;
        t = js_trim(r);
    }
    if let Some(r) = t.strip_prefix('-') {
        neg = !neg;
        t = r;
    } else if let Some(r) = t.strip_prefix('+') {
        t = r;
    }
    for sign in ['$', '€', '£', '¥'] {
        if let Some(r) = t.strip_prefix(sign) {
            t = r;
            let mut ch = t.chars();
            if ch.next().is_some_and(js_space) {
                t = ch.as_str();
            }
            break;
        }
    }
    let grouped;
    if is_grouped(t) {
        grouped = t.replace(',', "");
        t = &grouped;
    }
    // ^(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$
    let (mant, ex) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], Some(&t[i + 1..])),
        None => (t, None),
    };
    let (ip, fp) = match mant.find('.') {
        Some(i) => (&mant[..i], &mant[i + 1..]),
        None => (mant, ""),
    };
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(ip) || !all_digits(fp) || (ip.is_empty() && fp.is_empty()) {
        return None;
    }
    if ip.is_empty() && !mant.contains('.') {
        return None;
    }
    if let Some(e) = ex {
        let body = e.strip_prefix(['-', '+']).unwrap_or(e);
        if body.is_empty() || !all_digits(body) {
            return None;
        }
    }
    // With an exponent the value may be 0 or past a double: the parser says which.
    let zero = match ex {
        Some(_) => {
            let v: f64 = t.parse().ok()?;
            if !v.is_finite() {
                return None;
            }
            v == 0.0
        }
        None => false,
    };
    let mut digits: Vec<u8> = ip.bytes().chain(fp.bytes()).map(|b| b - b'0').collect();
    let mut exp = ip.len() as i64;
    let lead = digits.iter().take_while(|&&d| d == 0).count();
    digits.drain(..lead);
    exp -= lead as i64;
    while digits.last() == Some(&0) {
        digits.pop();
    }
    if digits.is_empty() || zero {
        return Some(Dec {
            neg,
            digits: Vec::new(),
            exp: 0,
            pct,
        });
    }
    if let Some(e) = ex {
        let (sign, body) = match e.as_bytes()[0] {
            b'-' => (-1i64, &e[1..]),
            b'+' => (1, &e[1..]),
            _ => (1, e),
        };
        let body = body.trim_start_matches('0');
        // A finite non-zero value keeps its exponent small.
        let n: i64 = if body.is_empty() {
            0
        } else {
            body.parse().ok()?
        };
        exp += sign * n;
    }
    if pct {
        exp -= 2;
    }
    Some(Dec {
        neg,
        digits,
        exp: i32::try_from(exp).ok()?,
        pct,
    })
}

/// `^\d{1,3}(,\d{3})+(\.\d*)?$`
fn is_grouped(t: &str) -> bool {
    let (int, rest) = match t.find('.') {
        Some(i) => (&t[..i], &t[i + 1..]),
        None => (t, ""),
    };
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let parts: Vec<&str> = int.split(',').collect();
    parts.len() >= 2
        && (1..=3).contains(&parts[0].len())
        && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
        && parts[1..].iter().all(|p| p.len() == 3)
}

/// The digits of `d` moved by `shift` and rounded half away from zero to
/// `frac` places: the integer digits (no leading zeros) and exactly `frac`
/// decimals. `None` when the value is 1e21 or more.
pub fn round_digits(d: &Dec, shift: i32, frac: usize) -> Option<(String, String)> {
    let mut digits = d.digits.clone();
    let mut exp = d.exp as i64 + shift as i64;
    if !digits.is_empty() {
        let keep = exp + frac as i64;
        if keep < 0 {
            digits.clear();
        } else if (keep as usize) < digits.len() {
            let k = keep as usize;
            let up = digits[k] >= 5;
            digits.truncate(k);
            if up {
                let mut i = k;
                loop {
                    if i == 0 {
                        digits.insert(0, 1);
                        exp += 1;
                        break;
                    }
                    i -= 1;
                    if digits[i] == 9 {
                        digits[i] = 0;
                    } else {
                        digits[i] += 1;
                        break;
                    }
                }
            }
        }
        while digits.last() == Some(&0) {
            digits.pop();
        }
        if digits.is_empty() {
            exp = 0;
        }
    }
    if !digits.is_empty() && exp >= 22 {
        return None;
    }
    let digit = |i: i64| -> char {
        if i < 0 {
            '0'
        } else {
            digits.get(i as usize).map_or('0', |&d| (b'0' + d) as char)
        }
    };
    let int: String = if digits.is_empty() || exp <= 0 {
        String::new()
    } else {
        (0..exp).map(digit).collect()
    };
    let fr: String = (0..frac as i64).map(|i| digit(exp + i)).collect();
    Some((int, fr))
}

fn number_section(sec: &Section, d: &Dec, minus: bool) -> Option<String> {
    if sec.kind == Kind::Plain {
        let mut out = String::new();
        for t in &sec.toks {
            if let Tok::Lit(l) = t {
                out.push_str(l);
            }
        }
        return Some(out);
    }
    let (int, frac) = round_digits(d, sec.shift, sec.frac)?;
    let (int, frac) = (int.as_bytes(), frac.as_bytes());
    let zero = int.is_empty() && frac.iter().all(|&b| b == b'0');
    let n_tok = sec.toks.len();
    // What each slot writes: one character, or nothing (0). The leftmost
    // integer slot also writes the digits the others had no room for.
    let mut put = vec![0u8; n_tok];
    let mut left = int.len();
    for &i in sec.int_slots.iter().rev() {
        put[i] = if left > 0 {
            left -= 1;
            int[left]
        } else {
            match sec.toks[i] {
                Tok::Slot('0') => b'0',
                Tok::Slot('?') => b' ',
                _ => 0,
            }
        };
    }
    let head = &int[..left];
    let mut trim = true;
    for (n, &i) in sec.frac_slots.iter().enumerate().rev() {
        let c = frac[n];
        put[i] = if trim && c == b'0' && sec.toks[i] != Tok::Slot('0') {
            if sec.toks[i] == Tok::Slot('?') {
                b' '
            } else {
                0
            }
        } else {
            trim = false;
            c
        };
    }
    let mut out = String::with_capacity(32);
    if minus && d.neg && !zero {
        out.push('-');
    }
    let first_int = sec.int_slots.first().copied();
    for (i, t) in sec.toks.iter().enumerate() {
        match t {
            Tok::Lit(l) => out.push_str(l),
            Tok::Point => out.push('.'),
            Tok::Pct => out.push('%'),
            Tok::Slot(_) if sec.grouping && sec.int_slots.contains(&i) => {
                if Some(i) != first_int {
                    continue;
                }
                // All the integer digits at the first slot, with thousands commas.
                let mut all: Vec<u8> = head.to_vec();
                all.extend(sec.int_slots.iter().map(|&k| put[k]).filter(|&c| c != 0));
                let spaces = all.iter().take_while(|&&c| c == b' ').count();
                let digits = &all[spaces..];
                for _ in 0..spaces {
                    out.push(' ');
                }
                for (k, &c) in digits.iter().enumerate() {
                    if k > 0 && (digits.len() - k) % 3 == 0 {
                        out.push(',');
                    }
                    out.push(c as char);
                }
            }
            Tok::Slot(_) => {
                if Some(i) == first_int {
                    out.push_str(std::str::from_utf8(head).unwrap_or(""));
                }
                if put[i] != 0 {
                    out.push(put[i] as char);
                }
            }
            _ => {}
        }
    }
    Some(out)
}

/// A date, a time, or both, as written in a cell (a time zone is dropped:
/// the clock time shows as written).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    /// Year, month, day.
    pub date: Option<(i32, u32, u32)>,
    /// Hour (0-23), minute, second (fractions cut).
    pub time: Option<(u32, u32, u32)>,
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        _ => 28,
    }
}

fn digits_at(b: &[u8], i: usize, min: usize, max: usize) -> Option<(u32, usize)> {
    let n = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    if n < min {
        return None;
    }
    let n = n.min(max);
    let v = std::str::from_utf8(&b[i..i + n]).ok()?.parse().ok()?;
    Some((v, i + n))
}

/// The date at the start: `yyyy-mm-dd` (or `/` `.`) or `m/d/yyyy` (or `.` `-`).
fn date_part(b: &[u8]) -> Option<((i32, u32, u32), usize)> {
    let sep = |i: usize| b.get(i).is_some_and(|c| matches!(c, b'-' | b'/' | b'.'));
    let four = b.len() >= 4
        && b[..4].iter().all(u8::is_ascii_digit)
        && b.get(4).is_some_and(|c| !c.is_ascii_digit());
    let (y, m, d, end) = if four {
        let y: u32 = std::str::from_utf8(&b[..4]).ok()?.parse().ok()?;
        if !sep(4) {
            return None;
        }
        let (m, i) = digits_at(b, 5, 1, 2)?;
        if b.get(i).is_some_and(u8::is_ascii_digit) || !sep(i) {
            return None;
        }
        let (d, j) = digits_at(b, i + 1, 1, 2)?;
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        (y as i32, m, d, j)
    } else {
        let (m, i) = digits_at(b, 0, 1, 2)?;
        if b.get(i).is_some_and(u8::is_ascii_digit) || !sep(i) {
            return None;
        }
        let (d, j) = digits_at(b, i + 1, 1, 2)?;
        if b.get(j).is_some_and(u8::is_ascii_digit) || !sep(j) {
            return None;
        }
        let n = b[j + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if n != 4 {
            return None;
        }
        let y: u32 = std::str::from_utf8(&b[j + 1..j + 5]).ok()?.parse().ok()?;
        (y as i32, m, d, j + 5)
    };
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(((y, m, d), end))
}

/// `h:mm`, `h:mm:ss`, `h:mm:ss.fff`, with AM/PM (`am`, `a.m.`) or on a 24-hour clock.
fn time_part(t: &str) -> Option<(u32, u32, u32)> {
    let b = t.as_bytes();
    let (h, i) = digits_at(b, 0, 1, 2)?;
    if b.get(i).is_some_and(u8::is_ascii_digit) || b.get(i) != Some(&b':') {
        return None;
    }
    let mi_n = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
    if mi_n != 2 {
        return None;
    }
    let (mi, mut j) = digits_at(b, i + 1, 2, 2)?;
    let mut s = 0;
    if b.get(j) == Some(&b':') {
        let n = b[j + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if n != 2 {
            return None;
        }
        let (sv, k) = digits_at(b, j + 1, 2, 2)?;
        s = sv;
        j = k;
        if matches!(b.get(j), Some(b'.') | Some(b',')) {
            let n = b[j + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if n == 0 {
                return None;
            }
            j += 1 + n;
        }
    }
    let rest = &t[j..];
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let mut hour = h;
    if !rest.is_empty() {
        let l = rest.to_ascii_lowercase();
        let pm = match l.as_str() {
            "am" | "a.m." | "am." | "a.m" => false,
            "pm" | "p.m." | "pm." | "p.m" => true,
            _ => return None,
        };
        if !(1..=12).contains(&h) {
            return None;
        }
        hour = h % 12 + if pm { 12 } else { 0 };
    } else if rest.len() != t.len() - j {
        // A space with nothing after it.
        return None;
    }
    if hour > 23 || mi > 59 || s > 59 {
        return None;
    }
    Some((hour, mi, s))
}

/// The time zone at the end (`Z`, `+05:30`, `-0800`) taken off.
fn strip_zone(t: &str) -> &str {
    if let Some(r) = t.strip_suffix(['Z', 'z']) {
        return r;
    }
    let b = t.as_bytes();
    let n = b.len();
    let d = |i: usize| b[i].is_ascii_digit();
    if n >= 6
        && matches!(b[n - 6], b'+' | b'-')
        && d(n - 5)
        && d(n - 4)
        && b[n - 3] == b':'
        && d(n - 2)
        && d(n - 1)
    {
        return &t[..n - 6];
    }
    if n >= 5 && matches!(b[n - 5], b'+' | b'-') && d(n - 4) && d(n - 3) && d(n - 2) && d(n - 1) {
        return &t[..n - 5];
    }
    t
}

/// A cell's date, time, or date and time; `None` for anything else.
pub fn read_stamp(text: &str) -> Option<Stamp> {
    let t = js_trim(text);
    if t.is_empty() || t.len() > 64 {
        return None;
    }
    if let Some(tm) = time_part(t) {
        return Some(Stamp {
            date: None,
            time: Some(tm),
        });
    }
    let (date, end) = date_part(t.as_bytes())?;
    let rest = &t[end..];
    if rest.is_empty() {
        return Some(Stamp {
            date: Some(date),
            time: None,
        });
    }
    let rest = rest.strip_prefix(['T', ' '])?;
    let tm = time_part(strip_zone(rest))?;
    Some(Stamp {
        date: Some(date),
        time: Some(tm),
    })
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Day of the week, 0 for Sunday (proleptic Gregorian calendar).
fn weekday(y: i32, m: u32, d: u32) -> usize {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days + 4).rem_euclid(7) as usize
}

fn date_section(sec: &Section, st: &Stamp) -> Option<String> {
    if (sec.needs_date && st.date.is_none()) || (sec.needs_time && st.time.is_none()) {
        return None;
    }
    let (y, mo, d) = st.date.unwrap_or((0, 1, 1));
    let (h, mi, s) = st.time.unwrap_or((0, 0, 0));
    let p2 = |n: u32| format!("{n:02}");
    let mut out = String::new();
    for t in &sec.toks {
        match t {
            Tok::Lit(l) => out.push_str(l),
            Tok::Year(true) => out.push_str(&format!("{y:04}")),
            Tok::Year(false) => out.push_str(&p2(y.rem_euclid(100) as u32)),
            Tok::Month(n) => {
                let name = MONTHS[mo as usize - 1];
                match n {
                    1 => out.push_str(&mo.to_string()),
                    2 => out.push_str(&p2(mo)),
                    3 => out.push_str(&name[..3]),
                    4 => out.push_str(name),
                    _ => out.push_str(&name[..1]),
                }
            }
            Tok::Day(n) => {
                let name = DAYS[weekday(y, mo, d)];
                match n {
                    1 => out.push_str(&d.to_string()),
                    2 => out.push_str(&p2(d)),
                    3 => out.push_str(&name[..3]),
                    _ => out.push_str(name),
                }
            }
            Tok::Hour(n) => {
                let hv = if sec.twelve {
                    if h % 12 == 0 {
                        12
                    } else {
                        h % 12
                    }
                } else {
                    h
                };
                out.push_str(&if *n == 1 { hv.to_string() } else { p2(hv) });
            }
            Tok::Minute(n) => out.push_str(&if *n == 1 { mi.to_string() } else { p2(mi) }),
            Tok::Second(n) => out.push_str(&if *n == 1 { s.to_string() } else { p2(s) }),
            Tok::AmPm(up) => out.push_str(match (h >= 12, up) {
                (false, true) => "AM",
                (true, true) => "PM",
                (false, false) => "am",
                (true, false) => "pm",
            }),
            Tok::Ap(up) => out.push_str(match (h >= 12, up) {
                (false, true) => "A",
                (true, true) => "P",
                (false, false) => "a",
                (true, false) => "p",
            }),
            _ => {}
        }
    }
    Some(out)
}

/// The codes the date styles and time styles of the display formats stand for.
pub fn date_style_code(style: &str) -> Option<&'static str> {
    Some(match style {
        "iso" => "yyyy-mm-dd",
        "mdy" => "mm/dd/yyyy",
        "dmy" => "dd/mm/yyyy",
        "medium" => "mmm d, yyyy",
        "long" => "dddd, mmmm d, yyyy",
        _ => return None,
    })
}

pub fn time_style_code(style: &str) -> Option<&'static str> {
    Some(match style {
        "hm12" => "h:mm AM/PM",
        "hm24" => "hh:mm",
        "hms12" => "h:mm:ss AM/PM",
        "hms24" => "hh:mm:ss",
        _ => return None,
    })
}

/// A display format ready to show cells: a custom code, or a time (with a
/// date for date and time).
#[derive(Clone, Debug)]
pub enum Display {
    Custom(Code),
    /// Time; date and time when the date code is there.
    Time {
        date: Option<Code>,
        time: Code,
    },
}

impl Display {
    /// From a step's fields: `code` for a custom code, else `time` (and `date`).
    pub fn new(code: &str, date: &str, time: &str) -> Result<Display, CodeError> {
        if !code.is_empty() {
            if !date.is_empty() || !time.is_empty() {
                return err(None, "a custom code with a date or time style");
            }
            return Ok(Display::Custom(Code::compile(code)?));
        }
        let one = |c: &str| -> Result<Code, CodeError> {
            let k = Code::compile(c)?;
            if !k.is_date_only() {
                return err(None, "a date or time style that is not one date section");
            }
            Ok(k)
        };
        if time.is_empty() {
            return err(None, "the code is empty");
        }
        Ok(Display::Time {
            date: if date.is_empty() {
                None
            } else {
                Some(one(date)?)
            },
            time: one(time)?,
        })
    }

    /// How `text` shows: `None` when it shows as it is.
    pub fn render(&self, text: &str) -> Option<Shown> {
        match self {
            Display::Custom(c) => c.render(text),
            Display::Time { date, time } => {
                let st = read_stamp(text)?;
                let t = &time.sections[0];
                let text = match (date, st.date, st.time) {
                    (None, _, Some(_)) => date_section(t, &st)?,
                    (Some(dc), Some(_), Some(_)) => {
                        format!(
                            "{} {}",
                            date_section(&dc.sections[0], &st)?,
                            date_section(t, &st)?
                        )
                    }
                    (Some(dc), Some(_), None) => date_section(&dc.sections[0], &st)?,
                    (Some(_), None, Some(_)) => date_section(t, &st)?,
                    _ => return None,
                };
                Some(Shown { text, color: None })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Case {
        code: String,
        #[serde(default)]
        date: String,
        #[serde(default)]
        time: String,
        /// Cells and what they show (null: as they are).
        #[serde(default)]
        cells: Vec<(String, Option<String>)>,
        #[serde(default)]
        color: Option<String>,
        /// The refusal message, for codes that are refused.
        #[serde(default)]
        error: Option<String>,
    }

    fn cases() -> Vec<Case> {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/numfmt-cases.json");
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn the_shared_cases() {
        let all = cases();
        assert!(all.len() >= 100, "{} cases", all.len());
        let mut cells = 0;
        for c in &all {
            let d = Display::new(&c.code, &c.date, &c.time);
            match (&c.error, d) {
                (Some(e), Err(got)) => assert_eq!(&got.to_string(), e, "code {:?}", c.code),
                (Some(e), Ok(_)) => panic!("code {:?} should be refused: {e}", c.code),
                (None, Err(got)) => panic!("code {:?} refused: {got}", c.code),
                (None, Ok(d)) => {
                    for (cell, want) in &c.cells {
                        cells += 1;
                        let got = d.render(cell);
                        assert_eq!(
                            got.as_ref().map(|s| s.text.as_str()),
                            want.as_deref(),
                            "code {:?} cell {:?}",
                            c.code,
                            cell
                        );
                        if let (Some(col), Some(g)) = (&c.color, &got) {
                            assert_eq!(
                                g.color.map(Color::name),
                                Some(col.as_str()).filter(|s| !s.is_empty()),
                                "colour of {:?} {:?}",
                                c.code,
                                cell
                            );
                        }
                    }
                }
            }
        }
        assert!(cells >= 300, "{cells} cells");
    }

    /// Random codes from the token alphabet, and long runs of one token:
    /// compile and render never panic, and the output stays small.
    #[test]
    fn random_and_runaway_codes_never_panic() {
        let alphabet: Vec<&str> = vec![
            "0", "#", "?", ".", ",", "%", "@", ";", "\"", "\\", "[", "]", "[Red]", "[>1]",
            "[<=-2]", "y", "m", "d", "h", "s", "AM/PM", "A/P", "-", "(", ")", " ", "$", "E+", "*",
            "_", "/", "x", "€", "0.00", "#,##0", "yyyy", "mmmm",
        ];
        let cells = [
            "",
            "0",
            "-0",
            "1234.5",
            "-1234.5",
            "0.125",
            "1e20",
            "1e-30",
            "99999999999999.9",
            "abc",
            "2026-03-14",
            "2026-03-14 14:30",
            "14:30:59.9",
            "2:30 PM",
            "(5)",
            "12%",
            "$1,234.56",
        ];
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut codes: Vec<String> = Vec::new();
        for _ in 0..2_000 {
            let n = (next() % 24) as usize + 1;
            codes.push(
                (0..n)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect(),
            );
        }
        for t in [
            "0", ",", "%", "\"", "\\", "#", "?", "[", "]", ";", "y", "m", "0.", ".0", "[Red]",
        ] {
            codes.push(t.repeat(200));
            codes.push(t.repeat(201));
            codes.push(format!("0{}", t.repeat(199)));
        }
        codes.push("\"abc".into());
        codes.push("0[Red".into());
        codes.push("x".repeat(10_000));
        for code in &codes {
            if let Ok(c) = Code::compile(code) {
                for cell in cells {
                    if let Some(s) = c.render(cell) {
                        assert!(
                            s.text.len() <= 4 * cell.len() + 4 * code.len() + 64,
                            "{code:?} {cell:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn exact_ties_round_half_away_from_zero() {
        let r = |cell: &str, frac: usize| {
            let d = read_decimal(cell).unwrap();
            let (i, f) = round_digits(&d, 0, frac).unwrap();
            if f.is_empty() {
                i
            } else {
                format!("{i}.{f}")
            }
        };
        assert_eq!(r("0.125", 2), ".13");
        assert_eq!(r("0.375", 2), ".38");
        assert_eq!(r("2.5", 0), "3");
        assert_eq!(r("-2.5", 0), "3");
        assert_eq!(r("1.005", 2), "1.01");
        assert_eq!(r("0.0000001", 2), ".00");
        assert_eq!(r("9.995", 2), "10.00");
        assert_eq!(r("999.5", 0), "1000");
    }

    #[test]
    fn long_numbers_stay_text() {
        assert!(read_decimal("1234567890123456").is_none());
        assert!(read_decimal("123456789012345").is_some());
        assert!(read_decimal("1e400").is_none());
        assert_eq!(read_decimal("1e-400").unwrap().digits, Vec::<u8>::new());
        assert!(read_decimal("1,23").is_none());
    }
}
