//! Cleaning tools and cleaners.
//!
//! Every cleaning tool is a [`Step`]: a plain description of one change to
//! one cell's text. A [`Cleaner`] is a list of steps run in order on each
//! cell; a tool run from the menu is a cleaner of one step. Cleaners are
//! saved as JSON and are untrusted when loaded ([`Cleaner::from_json`]):
//! every size is capped and every pattern is compiled with the find/replace
//! limits (the `regex` crate runs in linear time, so no pattern can take
//! exponential time).
//!
//! Each step checks the size of its result before building it, so no cell
//! grows past [`MAX_RESULT_CELL_BYTES`], and a run's plan (new text plus
//! about 64 bytes per changed cell) stays within [`MAX_REPLACE_BYTES`].

use std::sync::atomic::AtomicUsize;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::find::{
    short, Area, Change, FindSpec, Matcher, Pattern, Scope, MAX_PATTERN_CHARS,
    MAX_REPLACEMENT_CHARS, MAX_RESULT_CELL_BYTES,
};
use crate::ops::CellPlan;
use crate::value::ReadOptions;
use crate::Error;

/// Largest cleaner file, in bytes.
pub const MAX_CLEANER_BYTES: usize = 1 << 20;
/// Most steps in one cleaner.
pub const MAX_STEPS: usize = 100;
/// Longest cleaner name, in characters.
pub const MAX_NAME_CHARS: usize = 200;
/// Widest pad, longest random text, in characters.
pub const MAX_WIDTH: u32 = 10_000;
/// Most characters in a random-text character set.
pub const MAX_CHARSET: usize = 1_000;
/// Cleaner files carry this version.
pub const CLEANER_VERSION: u32 = 1;
/// Rows planned per parallel task.
const BLOCK: u64 = 2048;

// ---- steps ----

/// Where text is added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum At {
    Start,
    End,
    /// After this many characters (past the end: at the end).
    Position {
        n: u32,
    },
    /// Before the first occurrence of `text` (not found: unchanged).
    Before {
        text: String,
    },
    /// After the first occurrence of `text` (not found: unchanged).
    After {
        text: String,
    },
}

/// Characters of a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Letters,
    Digits,
    /// Punctuation and symbols.
    Punctuation,
    /// Spaces, tabs and line breaks.
    Spaces,
    /// Characters outside plain ASCII.
    NonAscii,
    /// Control characters (not tab or line breaks).
    Control,
    /// Characters that show nothing: zero-width spaces and joiners, the
    /// byte-order mark, soft hyphens, direction marks.
    Invisible,
}

/// A character that shows nothing (Unicode format characters and a few
/// fillers), the kind that sneaks into pasted or exported data.
fn invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}'
        | '\u{180B}'..='\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF9}'..='\u{FFFB}')
}

impl Kind {
    fn has(self, c: char) -> bool {
        match self {
            Kind::Letters => c.is_alphabetic(),
            Kind::Digits => c.is_numeric(),
            Kind::Punctuation => !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control(),
            Kind::Spaces => c.is_whitespace(),
            Kind::NonAscii => !c.is_ascii(),
            Kind::Control => c.is_control() && !matches!(c, '\t' | '\n' | '\r'),
            Kind::Invisible => invisible(c),
        }
    }
}

/// What text is removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Remove {
    /// The first `n` characters.
    First { n: u32 },
    /// The last `n` characters.
    Last { n: u32 },
    /// `count` characters starting after `from` characters.
    Range { from: u32, count: u32 },
    /// Everything before the first `text` (with it, if `inclusive`).
    Before {
        text: String,
        #[serde(default)]
        inclusive: bool,
    },
    /// Everything after the first `text` (with it, if `inclusive`).
    After {
        text: String,
        #[serde(default)]
        inclusive: bool,
    },
    /// Every match of a pattern.
    Matching { find: Pattern },
    /// Every character of a kind.
    Kind { of: Kind },
    /// Every one of these characters.
    Characters { chars: String },
}

/// What happens to spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Spaces {
    /// Leading and trailing spaces removed.
    Trim,
    TrimStart,
    TrimEnd,
    /// Trimmed, and every run of spaces (tabs, line breaks) inside made one space.
    Collapse,
    /// Every space removed.
    RemoveAll,
    /// Line breaks made spaces.
    LineBreaks,
    /// A space put between letters and digits (`A12` -> `A 12`).
    LettersDigits,
    /// A space put before capitals inside words (`firstName` -> `first Name`).
    BeforeCapitals,
    /// Odd spaces (no-break, thin, wide, ...) made plain spaces.
    Plain,
}

/// A case change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseTo {
    Upper,
    Lower,
    /// Each word's first letter capital, the rest small.
    Title,
    /// The first letter of each sentence capital, the rest small.
    Sentence,
    /// Capitals small and small capitals.
    Toggle,
    /// Only the first letter made a capital; the rest left as it is.
    First,
    /// Title Case, but small words (of, and, the, ...) small inside.
    TitleSmall,
    /// Names: Title Case plus McDonald, O'Brien, Smith-Jones, and Roman
    /// numerals (Henry VIII) in capitals.
    Names,
}

/// Words left small inside a title (not first or last).
const SMALL_WORDS: [&str; 18] = [
    "a", "an", "and", "as", "at", "but", "by", "for", "from", "in", "nor", "of", "on", "or", "the",
    "to", "vs", "with",
];

/// Characters for random text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Charset {
    Letters,
    Upper,
    Lower,
    Digits,
    LettersDigits,
    Hex,
    /// These characters.
    Custom {
        chars: String,
    },
}

/// One part of a person's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamePart {
    Title,
    First,
    Middle,
    Last,
    Suffix,
    /// The name as "First Middle Last".
    FirstLast,
    /// The name as "Last, First Middle".
    LastFirst,
}

/// One cleaning tool with its options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum Step {
    /// Replace text found by a pattern.
    Replace {
        find: Pattern,
        #[serde(default)]
        with: String,
    },
    /// Rewrite dates and times in a format (`yyyy-mm-dd hh:mm`). With
    /// `from`, cells are read in that pattern (`yyyymmdd`, `dd.mm.yy`,
    /// `mmm d, yyyy`) instead of being guessed: for dates nothing else
    /// recognises, or to say which way round day and month are.
    Dates {
        format: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        from: String,
        /// Two-digit years are in the past (birth dates): `26` is 1926 when
        /// 2026 would be after today.
        #[serde(default, skip_serializing_if = "is_false")]
        past: bool,
        /// What is done with a date both day orders can read. Left out only
        /// by recipes saved before this was recorded: they read it by the
        /// file's day order, as they always did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ambiguous: Option<Ambiguous>,
        /// Made on the date review screen: nothing else is guessed either
        /// (eight-digit values and two-digit years only when ticked).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        review: Option<DateReview>,
    },
    /// Rewrite numbers: decimals (`None` keeps them), thousands
    /// separators, decimal comma.
    Numbers {
        #[serde(default)]
        decimals: Option<u8>,
        #[serde(default)]
        thousands: bool,
        #[serde(default)]
        decimal_comma: bool,
    },
    /// Add text.
    Add {
        text: String,
        at: At,
    },
    /// Replace the cell with random characters.
    Random {
        length: u32,
        chars: Charset,
    },
    /// Pad to a width with a character, on the left or the right.
    Pad {
        width: u32,
        #[serde(default = "space")]
        with: char,
        #[serde(default)]
        right: bool,
    },
    Remove {
        what: Remove,
    },
    /// Reverse the characters.
    Reverse,
    Spaces {
        how: Spaces,
    },
    /// Fix what looks right but is not: zero-width characters, odd spaces, smart quotes, tabs and line breaks,
    /// and look-alike letters inside mixed-script words (see [`crate::xray`]).
    Sanitize {
        #[serde(flatten)]
        options: crate::xray::SanitizeOptions,
    },
    /// Undo mojibake: UTF-8 text that was read as Windows-1252 and saved again (`cafÃ©` becomes `café`). A cell is
    /// changed only when every character maps back to a Windows-1252 byte, those bytes are valid UTF-8, and the
    /// result has fewer odd characters; real text is left alone (see [`crate::encoding::fix_mojibake`]).
    FixMojibake,
    /// Number the cells: `start`, `start + step`, ... with at least `digits`
    /// digits, added at the start or end with `separator`.
    Number {
        #[serde(default = "one")]
        start: f64,
        #[serde(default = "one")]
        step: f64,
        #[serde(default)]
        digits: u8,
        #[serde(default)]
        at_end: bool,
        #[serde(default)]
        separator: String,
        /// Replace the cell instead of adding to it.
        #[serde(default)]
        replace: bool,
    },
    /// Remove digits, or keep only them.
    Digits {
        #[serde(default)]
        keep: bool,
    },
    /// Add the date and time of the run, in a format.
    Now {
        format: String,
        #[serde(default)]
        at_end: bool,
        #[serde(default)]
        separator: String,
        #[serde(default)]
        replace: bool,
    },
    Case {
        to: CaseTo,
    },
    /// Keep one part of a person's name, or reorder it.
    Name {
        part: NamePart,
    },
    /// `&`, `<`, `>`, quotes as HTML entities.
    HtmlEncode,
    /// HTML entities as characters.
    HtmlDecode,
    /// The text of HTML: tags dropped, entities decoded, breaks kept.
    HtmlText,
    /// Letters without accents (é → e, Æ → AE), case kept.
    Accents,
    /// Rewrite how a number is written.
    FixNumber {
        how: NumberFix,
    },
    /// Arithmetic on numbers; other cells are left.
    Calc {
        op: CalcOp,
        #[serde(default)]
        value: f64,
    },
    /// Split at a separator, sort the parts, join them again.
    SortParts {
        separator: String,
        #[serde(default)]
        descending: bool,
        /// Each part once.
        #[serde(default)]
        distinct: bool,
        /// Spaces around parts dropped; empty parts left out.
        #[serde(default)]
        trim: bool,
    },
    /// `%20` as a space (UTF-8); cells with bad escapes are left.
    UrlDecode,
    /// Everything but letters, digits and `-._~` as `%XX`.
    UrlEncode,
    /// A number written in English words (`123` -> `one hundred
    /// twenty-three`); as an amount, with dollars and cents.
    Words {
        #[serde(default)]
        money: bool,
    },
    /// A random whole number from `min` to `max`; with `unique`, no two
    /// cells of a column get the same one.
    RandomNumber {
        min: i64,
        max: i64,
        #[serde(default)]
        unique: bool,
    },
    /// Hide letters and digits behind `with`, all but the last `keep_last`
    /// (`123-45-6789` -> `***-**-6789`); with `all`, punctuation too.
    Mask {
        #[serde(default)]
        keep_last: u32,
        #[serde(default = "star")]
        with: char,
        #[serde(default)]
        all: bool,
    },
    /// A date made less exact: the year, the year and month, or an age band
    /// on the day of the run (`30-39`, with `top` and over as `90+`).
    DateBand {
        to: DateTo,
    },
    /// Replace the value with 16 characters made from it and a secret
    /// phrase: the same value always gives the same text (so columns still
    /// join and count). Anyone with the phrase can work the originals out
    /// by trying values, so the phrase must be long and not reused. The
    /// phrase is never saved with the step. Values are always trimmed;
    /// with `loose`, spaces, punctuation and case are ignored too
    /// (`123-45-6789` and `123456789` give the same text).
    Scramble {
        #[serde(default, skip_serializing)]
        phrase: crate::privacy::Secret,
        #[serde(default)]
        loose: bool,
    },
    /// Street type, directions and unit words abbreviated or written out
    /// (`123 North Main Street` -> `123 N Main St`).
    Address {
        #[serde(default)]
        style: crate::tidy::AddressStyle,
    },
    /// ZIP codes: leading zeros put back, `12345-6789`, or five digits.
    Zip {
        #[serde(default)]
        style: crate::tidy::ZipStyle,
    },
    /// US phone numbers written one way (`423-555-0123`), extensions kept.
    Phone {
        #[serde(default)]
        style: crate::tidy::PhoneStyle,
    },
    /// E-mail addresses trimmed and in small letters; with `fix_domains`,
    /// common misspellings of big providers fixed (`gmial.com`). Cells that
    /// are not an address are left and counted.
    Email {
        #[serde(default)]
        fix_domains: bool,
    },
}

/// How [`Step::DateBand`] coarsens a date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DateTo {
    Year,
    YearMonth,
    AgeBand {
        /// Years a band covers (1 gives the age itself).
        size: u32,
        /// Ages from this up share one band (`90+`); 0 for none.
        #[serde(default)]
        top: u32,
    },
}

fn star() -> char {
    '*'
}

/// How "fix numbers" rewrites a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberFix {
    /// `50-` and `(50)` as `-50`.
    TrailingMinus,
    /// `-5` ↔ `5`.
    FlipSign,
    /// Drop the minus.
    Positive,
    /// Add a minus.
    Negative,
    /// `12.5%` as `0.125`.
    Percent,
    /// A Julian date (`yyddd` or `yyyyddd`) as `yyyy-mm-dd`.
    Julian,
}

/// A calculation on numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalcOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    /// Round to `value` decimals (half away from zero).
    Round,
    /// Round away from zero.
    RoundUp,
    /// Round towards zero.
    RoundDown,
    Abs,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn space() -> char {
    ' '
}

fn one() -> f64 {
    1.0
}

/// A saved list of steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cleaner {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    pub steps: Vec<Step>,
}

fn version() -> u32 {
    CLEANER_VERSION
}

fn bad(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

fn check_text(what: &str, s: &str) -> Result<(), Error> {
    if s.chars().count() > MAX_REPLACEMENT_CHARS {
        return Err(bad(format!(
            "{what} can be at most {MAX_REPLACEMENT_CHARS} characters"
        )));
    }
    Ok(())
}

fn check_pattern(p: &Pattern) -> Result<(), Error> {
    if p.text.chars().count() > MAX_PATTERN_CHARS {
        return Err(Error::BadPattern(format!(
            "a pattern can be at most {MAX_PATTERN_CHARS} characters"
        )));
    }
    Ok(())
}

impl Cleaner {
    /// A cleaner from a saved file's bytes. The file is untrusted: size,
    /// step count, texts and patterns are all checked, and unknown tools
    /// are refused.
    pub fn from_json(data: &[u8]) -> Result<Cleaner, Error> {
        if data.len() > MAX_CLEANER_BYTES {
            return Err(bad(format!(
                "a cleaner file can be at most {} MB",
                MAX_CLEANER_BYTES >> 20
            )));
        }
        let c: Cleaner = serde_json::from_slice(data)
            .map_err(|e| bad(format!("this is not a cleaner file Natendo can read: {e}")))?;
        if c.version > CLEANER_VERSION {
            return Err(bad(
                "this cleaner was saved by a newer version of Natendo Data Edit",
            ));
        }
        c.check()?;
        Ok(c)
    }

    /// The cleaner as file bytes (pretty JSON).
    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        self.check()?;
        serde_json::to_vec_pretty(self).map_err(|e| bad(e.to_string()))
    }

    /// Check every size; [`Compiled::new`] also compiles the patterns.
    pub fn check(&self) -> Result<(), Error> {
        if self.steps.len() > MAX_STEPS {
            return Err(bad(format!("a cleaner can have at most {MAX_STEPS} steps")));
        }
        if self.name.chars().count() > MAX_NAME_CHARS {
            return Err(bad(format!(
                "a cleaner's name can be at most {MAX_NAME_CHARS} characters"
            )));
        }
        for s in &self.steps {
            s.check()?;
        }
        Ok(())
    }
}

impl Step {
    fn check(&self) -> Result<(), Error> {
        let at = |a: &At| match a {
            At::Before { text } | At::After { text } => check_text("the text to look for", text),
            _ => Ok(()),
        };
        match self {
            Step::Replace { find, with } => {
                check_pattern(find)?;
                check_text("the replacement", with)
            }
            Step::Dates { format, .. } | Step::Now { format, .. } => {
                if format.chars().count() > 200 {
                    return Err(bad("a date format can be at most 200 characters"));
                }
                if let Step::Dates { from, .. } = self {
                    if from.chars().count() > 200 {
                        return Err(bad("a date pattern can be at most 200 characters"));
                    }
                }
                if let Step::Now { separator, .. } = self {
                    check_text("the separator", separator)?;
                }
                Ok(())
            }
            Step::Numbers { decimals, .. } => {
                if decimals.is_some_and(|d| d > 15) {
                    return Err(bad("numbers can have at most 15 decimals"));
                }
                Ok(())
            }
            Step::Add { text, at: a } => {
                check_text("the text to add", text)?;
                at(a)
            }
            Step::Random { length, chars } => {
                if *length > MAX_WIDTH {
                    return Err(bad(format!(
                        "random text can be at most {MAX_WIDTH} characters"
                    )));
                }
                if let Charset::Custom { chars } = chars {
                    let n = chars.chars().count();
                    if n == 0 || n > MAX_CHARSET {
                        return Err(bad(format!(
                            "give between 1 and {MAX_CHARSET} characters for random text"
                        )));
                    }
                }
                Ok(())
            }
            Step::Pad { width, .. } => {
                if *width > MAX_WIDTH {
                    return Err(bad(format!("pad to at most {MAX_WIDTH} characters")));
                }
                Ok(())
            }
            Step::Remove { what } => match what {
                Remove::Before { text, .. } | Remove::After { text, .. } => {
                    check_text("the text to look for", text)
                }
                Remove::Matching { find } => check_pattern(find),
                Remove::Characters { chars } => check_text("the characters", chars),
                _ => Ok(()),
            },
            Step::Number {
                start,
                step,
                digits,
                separator,
                ..
            } => {
                if !start.is_finite()
                    || !step.is_finite()
                    || start.abs() > 1e15
                    || step.abs() > 1e15
                {
                    return Err(bad("the numbers must be between -10^15 and 10^15"));
                }
                if *digits > 30 {
                    return Err(bad("numbers can be padded to at most 30 digits"));
                }
                check_text("the separator", separator)
            }
            Step::Calc { op, value } => {
                if !value.is_finite() || value.abs() > 1e15 {
                    return Err(bad("the number must be between -10^15 and 10^15"));
                }
                match op {
                    CalcOp::Divide if *value == 0.0 => Err(bad("cannot divide by zero")),
                    CalcOp::Round | CalcOp::RoundUp | CalcOp::RoundDown
                        if !(0.0..=15.0).contains(value) || value.fract() != 0.0 =>
                    {
                        Err(bad("round to between 0 and 15 decimals"))
                    }
                    CalcOp::Power if value.abs() > 1000.0 => {
                        Err(bad("the power must be between -1000 and 1000"))
                    }
                    _ => Ok(()),
                }
            }
            Step::RandomNumber { min, max, .. } => {
                if min > max {
                    return Err(bad("the smallest number must not be above the largest"));
                }
                if min.abs() > 1_000_000_000_000_000 || max.abs() > 1_000_000_000_000_000 {
                    return Err(bad("choose numbers between -10^18 and 10^18"));
                }
                Ok(())
            }
            Step::Mask { keep_last, .. } => {
                if *keep_last > MAX_WIDTH {
                    return Err(bad(format!("keep at most {MAX_WIDTH} characters")));
                }
                Ok(())
            }
            Step::DateBand {
                to: DateTo::AgeBand { size, top },
            } => {
                if *size == 0 || *size > 150 || *top > 150 {
                    return Err(bad("choose bands of 1 to 150 years"));
                }
                Ok(())
            }
            Step::SortParts { separator, .. } => {
                if separator.is_empty() {
                    return Err(bad("choose the separator between the parts"));
                }
                check_text("the separator", separator)
            }
            _ => Ok(()),
        }
    }

    /// A short description for lists and status lines.
    pub fn describe(&self) -> String {
        match self {
            Step::Replace { find, with } => format!("Replace \"{}\" with \"{}\"", find.text, with),
            Step::Dates {
                format,
                from,
                ambiguous,
                ..
            } if from.is_empty() => {
                format!("Dates as {format} ({})", Ambiguous::words(*ambiguous))
            }
            Step::Dates { format, from, .. } => format!("Dates from {from} as {format}"),
            Step::Numbers { .. } => "Rewrite numbers".into(),
            Step::Add { text, .. } => format!("Add \"{text}\""),
            Step::Random { length, .. } => format!("Random text ({length} characters)"),
            Step::Pad { width, .. } => format!("Pad to {width} characters"),
            Step::Remove { .. } => "Remove text".into(),
            Step::Reverse => "Reverse text".into(),
            Step::Spaces { .. } => "Spaces".into(),
            Step::Sanitize { .. } => "Sanitize look-alike characters".into(),
            Step::FixMojibake => "Fix mojibake (cafÃ© to café)".into(),
            Step::Number { .. } => "Number the cells".into(),
            Step::Digits { keep: true } => "Keep only digits".into(),
            Step::Digits { keep: false } => "Remove digits".into(),
            Step::Now { .. } => "Add date and time".into(),
            Step::Case { .. } => "Change case".into(),
            Step::Name { .. } => "Name part".into(),
            Step::HtmlEncode => "HTML encode".into(),
            Step::HtmlDecode => "HTML decode".into(),
            Step::HtmlText => "Text from HTML".into(),
            Step::Accents => "Remove accents".into(),
            Step::FixNumber { .. } => "Fix numbers".into(),
            Step::Calc { .. } => "Calculate".into(),
            Step::SortParts { .. } => "Sort inside the cell".into(),
            Step::UrlDecode => "URL decode".into(),
            Step::UrlEncode => "URL encode".into(),
            Step::Words { money: false } => "Number in words".into(),
            Step::Words { money: true } => "Amount in words".into(),
            Step::RandomNumber { min, max, .. } => format!("Random number {min} to {max}"),
            Step::Mask { keep_last, .. } => format!("Mask all but the last {keep_last}"),
            Step::DateBand { to: DateTo::Year } => "Date to year".into(),
            Step::DateBand {
                to: DateTo::YearMonth,
            } => "Date to year and month".into(),
            Step::DateBand { .. } => "Date to age band".into(),
            Step::Scramble { .. } => "Scramble with a secret phrase".into(),
            Step::Address {
                style: crate::tidy::AddressStyle::Short,
            } => "Abbreviate street words".into(),
            Step::Address { .. } => "Write out street words".into(),
            Step::Zip { .. } => "Tidy ZIP codes".into(),
            Step::Phone { .. } => "Tidy phone numbers".into(),
            Step::Email { .. } => "Tidy e-mail addresses".into(),
        }
    }
}

// ---- compiled ----

/// Where a cell sits in a run, for steps that depend on it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ctx {
    /// The cell's row within the run (0 for the first row cleaned).
    pub index: u64,
    /// Random seed for this cell.
    pub seed: u64,
    /// Random seed shared by the cells of one column in a run.
    pub run: u64,
}

/// What one cell's run gave.
#[derive(Debug, Default)]
pub struct Outcome {
    /// The new text, if it changed.
    pub text: Option<String>,
    /// A step could not read the cell (not a date, not a number) and left it.
    pub left: bool,
    /// A date could only be read the other way round from the chosen day
    /// order (13/04/2026 when reading month first).
    pub other_order: bool,
    /// A date both day orders can read, left as typed because a step was
    /// told to leave such dates.
    pub ambiguous_left: bool,
}

/// What to do with a date both day orders can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ambiguous {
    /// Leave the cell exactly as typed.
    #[default]
    Leave,
    Mdy,
    Dmy,
}

impl Ambiguous {
    /// How a step's description names the choice.
    pub fn words(choice: Option<Ambiguous>) -> &'static str {
        match choice {
            None => "ambiguous dates: as in older versions (read by the file's day order)",
            Some(Ambiguous::Leave) => "ambiguous dates: left as typed",
            Some(Ambiguous::Mdy) => "ambiguous dates: read month first",
            Some(Ambiguous::Dmy) => "ambiguous dates: read day first",
        }
    }
}

/// The date review screen's other choices. Everything off means: change only
/// the dates that can be read one way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DateReview {
    /// Also change eight-digit values that are calendar dates (`20260425`).
    pub compact: bool,
    /// Also change dates whose year has two digits.
    pub two_digit: bool,
}

/// A cell's date under the review screen's choices (`None`: leave it).
fn reviewed_date(t: &str, ambiguous: Ambiguous, review: DateReview) -> Option<f64> {
    use crate::value::{classify_date, DateKind, DateOrder};
    let class = classify_date(t);
    if class.two_digit_year && !review.two_digit {
        return None;
    }
    let read = |date_order| {
        crate::coltype::Reader::plain(ReadOptions {
            date_order,
            ..Default::default()
        })
        .date(t)
    };
    match class.kind {
        DateKind::Ambiguous => match ambiguous {
            Ambiguous::Leave => None,
            Ambiguous::Mdy => read(DateOrder::Mdy),
            Ambiguous::Dmy => read(DateOrder::Dmy),
        },
        // One reading only: the order given does not matter.
        DateKind::Iso
        | DateKind::Written
        | DateKind::MonthFirstOnly
        | DateKind::DayFirstOnly
        | DateKind::SameEither => read(DateOrder::Mdy),
        DateKind::Compact if review.compact => compact_date(t),
        DateKind::Compact | DateKind::NotDate => None,
    }
}

enum CStep {
    Plain(Step),
    Replace(Matcher, String),
    RemoveMatching(Matcher),
    /// Format, the pattern cells are in (if given), two-digit years in the past.
    Dates(Vec<DatePart>, Option<Vec<DatePart>>, bool),
    /// Format and the review screen's choices.
    Reviewed(Vec<DatePart>, Ambiguous, DateReview),
    /// Outside the review screen, with the ambiguous dates' choice: the
    /// step it wraps, run in that order or leaving them.
    Ambiguous(Box<CStep>, Ambiguous),
    /// The stretched key, and whether spaces, punctuation and case are ignored.
    Scramble(crate::privacy::Keyed, bool),
    DateBand(DateTo),
    Now {
        when: String,
        at_end: bool,
        separator: String,
        replace: bool,
    },
}

/// A cleaner ready to run: patterns compiled, the moment of the run fixed.
pub struct Compiled {
    steps: Vec<CStep>,
    read: ReadOptions,
    /// The run's moment (days since 1970-01-01), for age bands.
    now: f64,
}

impl Compiled {
    /// Compile `cleaner`. `now` is the run's moment, as days since
    /// 1970-01-01 (local time) with the time as a fraction.
    pub fn new(cleaner: &Cleaner, read: ReadOptions, now: f64) -> Result<Compiled, Error> {
        cleaner.check()?;
        let mut steps = Vec::with_capacity(cleaner.steps.len());
        for s in &cleaner.steps {
            steps.push(match s {
                Step::Replace { find, with } => CStep::Replace(Matcher::new(find)?, with.clone()),
                Step::Remove {
                    what: Remove::Matching { find },
                } => CStep::RemoveMatching(Matcher::new(find)?),
                Step::Dates {
                    format,
                    ambiguous,
                    review: Some(review),
                    ..
                } => CStep::Reviewed(date_format(format)?, ambiguous.unwrap_or_default(), *review),
                Step::Dates {
                    format,
                    from,
                    past,
                    ambiguous,
                    ..
                } => {
                    let plain = CStep::Dates(
                        date_format(format)?,
                        if from.trim().is_empty() {
                            None
                        } else {
                            Some(date_format(from)?)
                        },
                        *past,
                    );
                    match ambiguous {
                        // A given input pattern says the order itself.
                        Some(choice) if from.trim().is_empty() => {
                            CStep::Ambiguous(Box::new(plain), *choice)
                        }
                        _ => plain,
                    }
                }
                Step::Now {
                    format,
                    at_end,
                    separator,
                    replace,
                } => CStep::Now {
                    when: write_date(now, &date_format(format)?),
                    at_end: *at_end,
                    separator: separator.clone(),
                    replace: *replace,
                },
                Step::Scramble { phrase, loose } => {
                    if phrase.chars() < crate::privacy::MIN_PHRASE_CHARS {
                        return Err(bad(format!(
                            "type a secret phrase of at least {} characters for the scramble",
                            crate::privacy::MIN_PHRASE_CHARS
                        )));
                    }
                    CStep::Scramble(crate::privacy::Keyed::from_secret(phrase), *loose)
                }
                Step::DateBand { to } => CStep::DateBand(*to),
                other => CStep::Plain(other.clone()),
            });
        }
        Ok(Compiled { steps, read, now })
    }

    /// Run every step on one cell.
    pub fn run(&self, cell: &str, ctx: Ctx) -> Result<Outcome, Error> {
        let mut cur: Option<String> = None;
        let mut left = false;
        let mut other_order = false;
        for (i, s) in self.steps.iter().enumerate() {
            let text = cur.as_deref().unwrap_or(cell);
            let ctx = Ctx {
                seed: ctx.seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                ..ctx
            };
            let guessing = match s {
                CStep::Dates(_, None, _) => Some(self.read.date_order),
                CStep::Ambiguous(_, Ambiguous::Mdy) => Some(crate::value::DateOrder::Mdy),
                CStep::Ambiguous(_, Ambiguous::Dmy) => Some(crate::value::DateOrder::Dmy),
                _ => None,
            };
            if guessing.is_some_and(|order| forced_order(text, order)) {
                other_order = true;
            }
            if let Some(new) = self.step(s, text, ctx, &mut left)? {
                if new.len() > MAX_RESULT_CELL_BYTES {
                    return Err(too_long());
                }
                cur = Some(new);
            }
        }
        let ambiguous_left = left
            && cur.is_none()
            && self.steps.iter().any(|s| {
                matches!(
                    s,
                    CStep::Ambiguous(_, Ambiguous::Leave) | CStep::Reviewed(_, Ambiguous::Leave, _)
                )
            })
            && crate::value::classify_date(cell).kind == crate::value::DateKind::Ambiguous;
        Ok(Outcome {
            text: cur.filter(|t| t != cell),
            left,
            other_order,
            ambiguous_left,
        })
    }

    fn step(&self, s: &CStep, t: &str, ctx: Ctx, left: &mut bool) -> Result<Option<String>, Error> {
        self.step_read(s, t, ctx, left, self.read)
    }

    /// [`Compiled::step`] with the way dates and numbers are read.
    fn step_read(
        &self,
        s: &CStep,
        t: &str,
        ctx: Ctx,
        left: &mut bool,
        options: ReadOptions,
    ) -> Result<Option<String>, Error> {
        Ok(match s {
            CStep::Replace(m, with) => m.replace(t, with)?.map(|(s, _)| s),
            CStep::RemoveMatching(m) => m.replace(t, "")?.map(|(s, _)| s),
            CStep::Dates(f, from, past) => {
                if t.trim().is_empty() {
                    return Ok(None);
                }
                let read = match from {
                    Some(p) => read_date(t, p),
                    None => crate::coltype::Reader::plain(options)
                        .date(t)
                        .filter(|v| !time_only(t, *v)),
                };
                // A two-digit year that lands after today goes back a century.
                let two_digit = match from {
                    Some(p) => p.contains(&DatePart::Year2),
                    None => !t.split(|c: char| !c.is_ascii_digit()).any(|r| r.len() >= 3),
                };
                let read = read.map(|v| {
                    if *past && two_digit && v.floor() > self.now.floor() {
                        century_back(v)
                    } else {
                        v
                    }
                });
                match read {
                    Some(v) => Some(write_date(v, f)),
                    _ => {
                        *left = true;
                        None
                    }
                }
            }
            CStep::Ambiguous(step, choice) => {
                use crate::value::{classify_date, DateKind, DateOrder};
                let order = match choice {
                    Ambiguous::Leave => {
                        if classify_date(t).kind == DateKind::Ambiguous {
                            *left = true;
                            return Ok(None);
                        }
                        self.read.date_order
                    }
                    Ambiguous::Mdy => DateOrder::Mdy,
                    Ambiguous::Dmy => DateOrder::Dmy,
                };
                let read = ReadOptions {
                    date_order: order,
                    ..self.read
                };
                return self.step_read(step, t, ctx, left, read);
            }
            CStep::Reviewed(format, ambiguous, review) => {
                if t.trim().is_empty() {
                    return Ok(None);
                }
                match reviewed_date(t, *ambiguous, *review) {
                    Some(v) => Some(write_date(v, format)),
                    None => {
                        *left = true;
                        None
                    }
                }
            }
            CStep::Now {
                when,
                at_end,
                separator,
                replace,
            } => Some(join_text(t, when, separator, *at_end, *replace)?),
            CStep::Scramble(k, loose) => {
                let v = t.trim();
                if v.is_empty() {
                    return Ok(None);
                }
                if *loose {
                    let squashed: String = v
                        .chars()
                        .filter(|c| c.is_alphanumeric())
                        .flat_map(char::to_lowercase)
                        .collect();
                    // Only punctuation: scramble it as written.
                    if squashed.is_empty() {
                        Some(k.scramble(v))
                    } else {
                        Some(k.scramble(&squashed))
                    }
                } else {
                    Some(k.scramble(v))
                }
            }
            CStep::DateBand(to) => {
                if t.trim().is_empty() {
                    return Ok(None);
                }
                let r = crate::coltype::Reader::plain(self.read)
                    .date(t)
                    .filter(|v| !time_only(t, *v))
                    .or_else(|| compact_date(t))
                    .and_then(|v| date_band(v, *to, self.now))
                    .or_else(|| year_band(t, *to, self.now));
                if r.is_none() {
                    *left = true;
                }
                r
            }
            CStep::Plain(p) => plain(p, t, ctx, self.read, left)?,
        })
    }
}

/// A tidy step's answer: blank cells stay blank, unreadable ones are left
/// and counted.
fn tidied(t: &str, left: &mut bool, f: impl Fn(&str) -> Option<String>) -> Option<String> {
    if t.trim().is_empty() {
        return None;
    }
    let r = f(t);
    if r.is_none() {
        *left = true;
    }
    r
}

fn too_long() -> Error {
    Error::Invalid(format!(
        "a cleaned cell would be over {} MB",
        MAX_RESULT_CELL_BYTES >> 20
    ))
}

/// `t` with `add` at the start or end (with `sep` between when both are
/// there), or `add` alone.
fn join_text(t: &str, add: &str, sep: &str, at_end: bool, replace: bool) -> Result<String, Error> {
    if replace {
        return Ok(add.to_string());
    }
    if t.len() + add.len() + sep.len() > MAX_RESULT_CELL_BYTES {
        return Err(too_long());
    }
    let sep = if t.is_empty() { "" } else { sep };
    Ok(if at_end {
        format!("{t}{sep}{add}")
    } else {
        format!("{add}{sep}{t}")
    })
}

fn char_index(t: &str, n: u32) -> usize {
    t.char_indices().nth(n as usize).map_or(t.len(), |(i, _)| i)
}

fn plain(
    p: &Step,
    t: &str,
    ctx: Ctx,
    read: ReadOptions,
    left: &mut bool,
) -> Result<Option<String>, Error> {
    Ok(match p {
        Step::Numbers {
            decimals,
            thousands,
            decimal_comma,
        } => {
            let s = t.trim();
            if s.is_empty() {
                return Ok(None);
            }
            match number_value(s, read) {
                Some(v) => Some(write_number(v, *decimals, *thousands, *decimal_comma)),
                None => {
                    *left = true;
                    None
                }
            }
        }
        Step::Add { text, at } => {
            if t.len() + text.len() > MAX_RESULT_CELL_BYTES {
                return Err(too_long());
            }
            let i = match at {
                At::Start => 0,
                At::End => t.len(),
                At::Position { n } => char_index(t, *n),
                At::Before { text: f } => {
                    match (!f.is_empty()).then(|| t.find(f.as_str())).flatten() {
                        Some(i) => i,
                        None => return Ok(None),
                    }
                }
                At::After { text: f } => {
                    match (!f.is_empty()).then(|| t.find(f.as_str())).flatten() {
                        Some(i) => i + f.len(),
                        None => return Ok(None),
                    }
                }
            };
            let mut s = String::with_capacity(t.len() + text.len());
            s.push_str(&t[..i]);
            s.push_str(text);
            s.push_str(&t[i..]);
            Some(s)
        }
        Step::Random { length, chars } => Some(random_text(*length, chars, ctx.seed)),
        Step::Pad { width, with, right } => {
            let have = t.chars().count();
            let w = *width as usize;
            if have >= w {
                return Ok(None);
            }
            let need = (w - have) * with.len_utf8();
            if t.len() + need > MAX_RESULT_CELL_BYTES {
                return Err(too_long());
            }
            let pad: String = std::iter::repeat_n(*with, w - have).collect();
            Some(if *right {
                format!("{t}{pad}")
            } else {
                format!("{pad}{t}")
            })
        }
        Step::Remove { what } => remove(what, t),
        Step::Reverse => Some(t.chars().rev().collect()),
        Step::Spaces { how } => Some(spaces(*how, t)),
        Step::Sanitize { options } => crate::xray::sanitize(t, options),
        Step::FixMojibake => crate::encoding::fix_mojibake(t),
        Step::Number {
            start,
            step,
            digits,
            at_end,
            separator,
            replace,
        } => {
            let v = start + step * ctx.index as f64;
            let mut n = crate::fill::format_number(v, read.decimal_comma);
            let (neg, body) = match n.strip_prefix('-') {
                Some(b) => (true, b.to_string()),
                None => (false, n.clone()),
            };
            let int_len = body.find([',', '.']).unwrap_or(body.len());
            if int_len < *digits as usize {
                n = format!(
                    "{}{}{body}",
                    if neg { "-" } else { "" },
                    "0".repeat(*digits as usize - int_len)
                );
            }
            Some(join_text(t, &n, separator, *at_end, *replace)?)
        }
        Step::Digits { keep } => Some(t.chars().filter(|c| c.is_ascii_digit() == *keep).collect()),
        Step::Case { to } => Some(case(*to, t)),
        Step::Name { part } => {
            if t.trim().is_empty() {
                return Ok(None);
            }
            Some(name_part(&split_name(t), *part))
        }
        Step::HtmlEncode => Some(html_encode(t)),
        Step::HtmlDecode => {
            if !t.contains('&') {
                return Ok(None);
            }
            let mut out = String::with_capacity(t.len());
            crate::import::html_unescape(t, &mut out);
            Some(out)
        }
        Step::HtmlText => Some(html_text(t)),
        Step::Accents => accents(t),
        Step::FixNumber { how } => {
            if t.trim().is_empty() {
                return Ok(None);
            }
            let r = fix_number(*how, t.trim(), read);
            if r.is_none() {
                *left = true;
            }
            r
        }
        Step::Calc { op, value } => {
            let s = t.trim();
            if s.is_empty() {
                return Ok(None);
            }
            match number_value(s, read).and_then(|v| calc(*op, v, *value, read)) {
                Some(r) => Some(r),
                None => {
                    *left = true;
                    None
                }
            }
        }
        Step::SortParts {
            separator,
            descending,
            distinct,
            trim,
        } => sort_parts(t, separator, *descending, *distinct, *trim),
        Step::UrlDecode => {
            if !t.contains('%') {
                return Ok(None);
            }
            let r = url_decode(t);
            if r.is_none() {
                *left = true;
            }
            r
        }
        Step::UrlEncode => Some(url_encode(t)),
        Step::Mask {
            keep_last,
            with,
            all,
        } => {
            let n = t.chars().filter(|c| *all || c.is_alphanumeric()).count();
            // Never show more than half of a value, and a value no longer
            // than the part kept is hidden whole (`6789` with "keep 4").
            let keep = if n <= *keep_last as usize {
                0
            } else {
                (*keep_last as usize).min(n / 2)
            };
            let hide = n - keep;
            let mut seen = 0;
            let out: String = t
                .chars()
                .map(|c| {
                    if *all || c.is_alphanumeric() {
                        seen += 1;
                        if seen <= hide {
                            return *with;
                        }
                    }
                    c
                })
                .collect();
            Some(out)
        }
        Step::Address { style } => {
            let r = crate::tidy::address(t, *style);
            if r.is_none() && !t.trim().is_empty() {
                *left = true;
            }
            r
        }
        Step::Zip { style } => tidied(t, left, |t| crate::tidy::zip(t, *style)),
        Step::Phone { style } => tidied(t, left, |t| crate::tidy::phone(t, *style)),
        Step::Email { fix_domains } => tidied(t, left, |t| crate::tidy::email(t, *fix_domains)),
        Step::Words { money } => {
            let s = t.trim();
            if s.is_empty() {
                return Ok(None);
            }
            let r = number_value(s, read).and_then(|v| in_words(v, *money));
            if r.is_none() {
                *left = true;
            }
            r
        }
        Step::RandomNumber { min, max, unique } => {
            let n = (*max as i128 - *min as i128 + 1) as u128;
            let k = if *unique {
                if u128::from(ctx.index) >= n {
                    // More cells than numbers: these stay as they are.
                    *left = true;
                    return Ok(None);
                }
                permute(ctx.index as u128, n, ctx.run)
            } else {
                u128::from(mix(ctx.seed)) % n
            };
            Some((*min as i128 + k as i128).to_string())
        }
        // Compiled separately.
        Step::Replace { .. }
        | Step::Dates { .. }
        | Step::Now { .. }
        | Step::Scramble { .. }
        | Step::DateBand { .. } => None,
    })
}

fn remove(what: &Remove, t: &str) -> Option<String> {
    Some(match what {
        Remove::First { n } => t[char_index(t, *n)..].to_string(),
        Remove::Last { n } => {
            let count = t.chars().count();
            t[..char_index(t, count.saturating_sub(*n as usize) as u32)].to_string()
        }
        Remove::Range { from, count } => {
            let a = char_index(t, *from);
            let b = a + char_index(&t[a..], *count);
            format!("{}{}", &t[..a], &t[b..])
        }
        Remove::Before { text, inclusive } => {
            let i = (!text.is_empty())
                .then(|| t.find(text.as_str()))
                .flatten()?;
            t[if *inclusive { i + text.len() } else { i }..].to_string()
        }
        Remove::After { text, inclusive } => {
            let i = (!text.is_empty())
                .then(|| t.find(text.as_str()))
                .flatten()?;
            t[..if *inclusive { i } else { i + text.len() }].to_string()
        }
        Remove::Kind { of } => t.chars().filter(|c| !of.has(*c)).collect(),
        Remove::Characters { chars } => t.chars().filter(|c| !chars.contains(*c)).collect(),
        Remove::Matching { .. } => return None,
    })
}

/// Byte spans of the words (letters, digits and inner apostrophes).
fn word_spans(t: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let cs: Vec<(usize, char)> = t.char_indices().collect();
    for (k, &(i, c)) in cs.iter().enumerate() {
        let inner = (c == '\'' || c == '’')
            && start.is_some()
            && cs.get(k + 1).is_some_and(|x| x.1.is_alphanumeric());
        if c.is_alphanumeric() || inner {
            start.get_or_insert(i);
        } else if let Some(a) = start.take() {
            out.push((a, i));
        }
    }
    if let Some(a) = start {
        out.push((a, t.len()));
    }
    out
}

/// Roman numerals used after names (II to XIII).
fn roman(w: &str) -> bool {
    matches!(
        w.to_ascii_lowercase().as_str(),
        "ii" | "iii"
            | "iv"
            | "vi"
            | "vii"
            | "viii"
            | "ix"
            | "xi"
            | "xii"
            | "xiii"
            | "xiv"
            | "xv"
            | "xvi"
            | "xvii"
            | "xviii"
            | "xix"
            | "xx"
    )
}

/// Numerals that are also given names or surnames (Vi, Xi, Ix): written
/// as numerals only where a numeral is expected (see `names_case`).
fn roman_name(w: &str) -> bool {
    matches!(w.to_ascii_lowercase().as_str(), "vi" | "xi" | "ix")
}

/// Title Case for people's names: McDonald, O'Brien, D'Angelo, Smith-Jones,
/// Henry VIII.
///
/// A numeral that is also a name (Vi, Xi, Ix) stays a name unless it is the
/// last word after at least two others (John Paul XI), or the text has mixed
/// case and it alone was in capitals (Pope Pius XI). Other numerals (II,
/// III, IV, VIII...) are capitals anywhere but the first word, so two-word
/// "Henry VIII" works.
fn names_case(t: &str) -> String {
    let title = case(CaseTo::Title, t);
    let spans = word_spans(&title);
    let originals: Vec<&str> = word_spans(t).iter().map(|&(a, b)| &t[a..b]).collect();
    let mixed = t.chars().any(char::is_lowercase) && t.chars().any(char::is_uppercase);
    let mut out = String::with_capacity(title.len());
    let mut last = 0;
    let n = spans.len();
    for (k, &(a, b)) in spans.iter().enumerate() {
        out.push_str(&title[last..a]);
        let w = &title[a..b];
        let chars: Vec<char> = w.chars().collect();
        let caps = originals
            .get(k)
            .is_some_and(|o| o.chars().all(|c| !c.is_lowercase()));
        let numeral = roman(w)
            && k > 0
            && if roman_name(w) {
                (k + 1 == n && k >= 2) || (mixed && caps)
            } else {
                true
            };
        if numeral {
            out.push_str(&w.to_uppercase());
        } else if chars.len() > 2 && chars[0] == 'M' && chars[1] == 'c' && chars[2].is_alphabetic()
        {
            out.push_str("Mc");
            out.extend(chars[2].to_uppercase());
            out.extend(chars[3..].iter());
        } else if chars.len() > 2
            && matches!(chars[0], 'O' | 'D')
            && matches!(chars[1], '\'' | '’')
            && chars[2].is_alphabetic()
        {
            out.push(chars[0]);
            out.push(chars[1]);
            out.extend(chars[2].to_uppercase());
            out.extend(chars[3..].iter());
        } else {
            out.push_str(w);
        }
        last = b;
    }
    out.push_str(&title[last..]);
    out
}

fn spaces(how: Spaces, t: &str) -> String {
    match how {
        Spaces::Trim => t.trim().to_string(),
        Spaces::TrimStart => t.trim_start().to_string(),
        Spaces::TrimEnd => t.trim_end().to_string(),
        Spaces::Collapse => t.split_whitespace().collect::<Vec<_>>().join(" "),
        Spaces::RemoveAll => t.chars().filter(|c| !c.is_whitespace()).collect(),
        Spaces::LineBreaks => {
            let s = t.replace("\r\n", " ");
            s.replace(['\r', '\n'], " ")
        }
        Spaces::LettersDigits => {
            let mut out = String::with_capacity(t.len() + 8);
            let mut prev: Option<char> = None;
            for c in t.chars() {
                if let Some(p) = prev {
                    if (p.is_alphabetic() && c.is_ascii_digit())
                        || (p.is_ascii_digit() && c.is_alphabetic())
                    {
                        out.push(' ');
                    }
                }
                out.push(c);
                prev = Some(c);
            }
            out
        }
        Spaces::Plain => t
            .chars()
            .map(|c| {
                if c.is_whitespace() && !matches!(c, ' ' | '\t' | '\n' | '\r') {
                    ' '
                } else {
                    c
                }
            })
            .collect(),
        Spaces::BeforeCapitals => {
            let mut out = String::with_capacity(t.len() + 8);
            let mut prev: Option<char> = None;
            for c in t.chars() {
                if let Some(p) = prev {
                    if c.is_uppercase() && (p.is_lowercase() || p.is_ascii_digit()) {
                        out.push(' ');
                    }
                }
                out.push(c);
                prev = Some(c);
            }
            out
        }
    }
}

fn case(to: CaseTo, t: &str) -> String {
    match to {
        CaseTo::First => {
            let Some((i, c)) = t.char_indices().find(|(_, c)| c.is_alphabetic()) else {
                return t.to_string();
            };
            let mut out = String::with_capacity(t.len());
            out.push_str(&t[..i]);
            out.extend(c.to_uppercase());
            out.push_str(&t[i + c.len_utf8()..]);
            out
        }
        CaseTo::TitleSmall => {
            let title = case(CaseTo::Title, t);
            let words: Vec<(usize, usize)> = word_spans(&title);
            let mut out = title.clone();
            for (n, &(a, b)) in words.iter().enumerate() {
                if n == 0 || n + 1 == words.len() {
                    continue;
                }
                let w = title[a..b].to_lowercase();
                // Not right after a colon or a full stop: a new part starts.
                let before = title[..a].trim_end();
                if SMALL_WORDS.contains(&w.as_str()) && !before.ends_with([':', '.', '?', '!']) {
                    out.replace_range(a..b, &w);
                }
            }
            out
        }
        CaseTo::Names => names_case(t),
        CaseTo::Upper => t.to_uppercase(),
        CaseTo::Lower => t.to_lowercase(),
        CaseTo::Toggle => t
            .chars()
            .flat_map(|c| {
                if c.is_uppercase() {
                    c.to_lowercase().collect::<Vec<_>>()
                } else {
                    c.to_uppercase().collect::<Vec<_>>()
                }
            })
            .collect(),
        CaseTo::Title => {
            let mut out = String::with_capacity(t.len());
            let mut start = true;
            for c in t.chars() {
                if c.is_alphanumeric() {
                    if start && c.to_uppercase().count() == 1 {
                        out.extend(c.to_uppercase());
                    } else if start {
                        out.push(c);
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(c);
                    // An apostrophe inside a word does not start a new one
                    // (don't), but a hyphen does (Smith-Jones).
                    start = c != '\'' && c != '’';
                }
            }
            out
        }
        CaseTo::Sentence => {
            // A sentence starts the text, a line, or follows `.`, `!` or `?`
            // and a space ("e.g. this" stays one sentence: "E.g. this").
            let mut out = String::with_capacity(t.len());
            let mut start = true;
            let mut stop = false;
            for c in t.chars() {
                if c.is_alphabetic() {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                    stop = false;
                } else {
                    out.push(c);
                    if c == '\n' || (stop && c.is_whitespace()) {
                        start = true;
                    }
                    stop = matches!(c, '.' | '!' | '?');
                }
            }
            out
        }
    }
}

// ---- random ----

/// SplitMix64: small, fast, good enough for test data (not for secrets).
const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

/// A whole number below 10^15 in words.
fn whole_words(mut n: u64) -> String {
    if n == 0 {
        return "zero".into();
    }
    const SCALES: [&str; 5] = ["", " thousand", " million", " billion", " trillion"];
    let mut groups = Vec::new();
    let mut k = 0;
    while n > 0 {
        let g = n % 1000;
        if g > 0 {
            let mut w = String::new();
            let (h, r) = (g / 100, g % 100);
            if h > 0 {
                w.push_str(ONES[h as usize]);
                w.push_str(" hundred");
            }
            if r > 0 {
                if !w.is_empty() {
                    w.push(' ');
                }
                if r < 20 {
                    w.push_str(ONES[r as usize]);
                } else {
                    w.push_str(TENS[(r / 10) as usize]);
                    if r % 10 > 0 {
                        w.push('-');
                        w.push_str(ONES[(r % 10) as usize]);
                    }
                }
            }
            w.push_str(SCALES[k]);
            groups.push(w);
        }
        n /= 1000;
        k += 1;
    }
    groups.reverse();
    groups.join(" ")
}

/// A number in English words; as money, dollars and cents.
fn in_words(v: f64, money: bool) -> Option<String> {
    if !v.is_finite() || v.abs() >= 1e15 {
        return None;
    }
    let neg = v < 0.0;
    let a = v.abs();
    let mut out = String::new();
    if neg {
        out.push_str("minus ");
    }
    if money {
        // Round on the written digits (half away from zero), not in binary:
        // 1.005 is a dollar and one cent.
        let text = format!("{a}");
        let (whole, frac) = text.split_once('.').unwrap_or((&text, ""));
        let mut d: u64 = whole.parse().ok()?;
        let f: Vec<u32> = frac.chars().map(|c| c.to_digit(10).unwrap_or(0)).collect();
        let mut c = f.first().copied().unwrap_or(0) * 10 + f.get(1).copied().unwrap_or(0);
        if f.get(2).is_some_and(|&x| x >= 5) {
            c += 1;
        }
        if c == 100 {
            d += 1;
            c = 0;
        }
        let c = u64::from(c);
        out.push_str(&whole_words(d));
        out.push_str(if d == 1 { " dollar" } else { " dollars" });
        if c > 0 {
            out.push_str(" and ");
            out.push_str(&whole_words(c));
            out.push_str(if c == 1 { " cent" } else { " cents" });
        }
        return Some(out);
    }
    let whole = a.trunc() as u64;
    out.push_str(&whole_words(whole));
    // Decimals digit by digit, as written by the shortest form.
    let text = format!("{a}");
    if let Some((_, dec)) = text.split_once('.') {
        out.push_str(" point");
        for c in dec.chars() {
            out.push(' ');
            out.push_str(ONES[c.to_digit(10)? as usize]);
        }
    }
    Some(out)
}

/// Where `i` (below `n`) goes in a shuffled order of 0..n, the same for a
/// given `key`: a small Feistel network over the next power of four,
/// walking until it lands below `n`. Each `i` gets its own place.
fn permute(i: u128, n: u128, key: u64) -> u128 {
    let mut bits = 128 - (n.max(2) - 1).leading_zeros();
    bits += bits % 2;
    let half = bits / 2;
    let mask = (1u128 << half) - 1;
    let mut x = i;
    loop {
        let (mut l, mut r) = (x >> half, x & mask);
        for round in 0..4u64 {
            let f = u128::from(mix(key ^ mix(round ^ (r as u64) ^ ((r >> 64) as u64)))) & mask;
            (l, r) = (r, l ^ f);
        }
        x = (l << half) | r;
        if x < n {
            return x;
        }
    }
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn random_text(length: u32, chars: &Charset, seed: u64) -> String {
    const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
    const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const DIGITS: &str = "0123456789";
    let set: Vec<char> = match chars {
        Charset::Letters => format!("{LOWER}{UPPER}").chars().collect(),
        Charset::Upper => UPPER.chars().collect(),
        Charset::Lower => LOWER.chars().collect(),
        Charset::Digits => DIGITS.chars().collect(),
        Charset::LettersDigits => format!("{LOWER}{UPPER}{DIGITS}").chars().collect(),
        Charset::Hex => "0123456789abcdef".chars().collect(),
        Charset::Custom { chars } => {
            let mut v: Vec<char> = Vec::new();
            for c in chars.chars() {
                if !v.contains(&c) {
                    v.push(c);
                }
            }
            v
        }
    };
    if set.is_empty() {
        return String::new();
    }
    let mut s = seed;
    (0..length)
        .map(|_| {
            s = mix(s);
            set[(s % set.len() as u64) as usize]
        })
        .collect()
}

// ---- numbers ----

/// The number a cell holds, for rewriting: percentages and values with more
/// digits than a double keeps exactly are left alone.
fn number_value(s: &str, read: ReadOptions) -> Option<f64> {
    if s.contains('%') || s.chars().filter(char::is_ascii_digit).count() > 15 {
        return None;
    }
    if !groups_ok(s, read.decimal_comma) {
        return None;
    }
    crate::coltype::Reader::plain(read).number(s)
}

/// Thousands separators, if any, sit between groups of three digits after a
/// first group of one to three that does not start with 0 (`1,234`), so
/// `0.125` read with a decimal comma is not taken for 125.
fn groups_ok(s: &str, comma: bool) -> bool {
    let (sep, point) = if comma { ('.', ',') } else { (',', '.') };
    let digits_start = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
    let body = &s[digits_start..];
    let int = body.split(point).next().unwrap_or("");
    let int: &str = int
        .find(|c: char| !c.is_ascii_digit() && c != sep)
        .map_or(int, |i| &int[..i]);
    if !int.contains(sep) {
        return true;
    }
    let mut parts = int.split(sep);
    let first = parts.next().unwrap_or("");
    (1..=3).contains(&first.len())
        && !first.starts_with('0')
        && parts.all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
}

/// `v` with `d` decimals, rounded half away from zero on its shortest
/// decimal form (2.675 -> 2.68, as spreadsheets show it).
fn round_text(v: f64, d: usize) -> String {
    let s = format!("{}", v.abs());
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (s.clone(), String::new()),
    };
    let mut digits: Vec<u8> = int
        .bytes()
        .chain(frac.bytes().chain(std::iter::repeat(b'0')).take(d))
        .collect();
    if frac.len() > d && frac.as_bytes()[d] >= b'5' {
        // Add one at the last kept digit, carrying.
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, b'1');
                break;
            }
            i -= 1;
            if digits[i] == b'9' {
                digits[i] = b'0';
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let n = digits.len() - d;
    let mut out = String::with_capacity(digits.len() + 2);
    let zero = digits.iter().all(|&b| b == b'0');
    if v < 0.0 && !zero {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&digits[..n]).unwrap_or("0"));
    if d > 0 {
        out.push('.');
        out.push_str(std::str::from_utf8(&digits[n..]).unwrap_or(""));
    }
    out
}

fn write_number(v: f64, decimals: Option<u8>, thousands: bool, comma: bool) -> String {
    let mut s = match decimals {
        Some(d) => round_text(v, d as usize),
        None => crate::fill::format_number(v, false),
    };
    if s == "-0" || s.starts_with("-0.") && s[3..].bytes().all(|b| b == b'0') {
        s.remove(0);
    }
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s.as_str()),
    };
    let (int, frac) = match body.find('.') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let mut out = String::with_capacity(s.len() + 8);
    if neg {
        out.push('-');
    }
    let sep = if comma { '.' } else { ',' };
    for (i, ch) in int.chars().enumerate() {
        if thousands && i > 0 && (int.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push(if comma { ',' } else { '.' });
        out.push_str(f);
    }
    out
}

// ---- dates ----

#[derive(Debug, Clone, PartialEq)]
enum DatePart {
    Text(String),
    Year4,
    Year2,
    MonthName,
    MonthShort,
    Month2,
    Month,
    DayName,
    DayShort,
    Day2,
    Day,
    Hour2,
    Hour,
    Minute2,
    Minute,
    Second2,
    Second,
    /// AM/PM (or am/pm).
    AmPm {
        lower: bool,
    },
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
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
];

/// Read a date format such as `yyyy-mm-dd hh:mm:ss` or `d mmm yyyy h:mm
/// AM/PM`. `m`/`mm` right after an hour or before seconds are minutes, as in
/// spreadsheets. Text in quotes, or after a backslash, is written as is.
fn date_format(f: &str) -> Result<Vec<DatePart>, Error> {
    let chars: Vec<char> = f.chars().collect();
    let mut out: Vec<DatePart> = Vec::new();
    let mut i = 0;
    let lit = |out: &mut Vec<DatePart>, c: char| match out.last_mut() {
        Some(DatePart::Text(t)) => t.push(c),
        _ => out.push(DatePart::Text(c.to_string())),
    };
    while i < chars.len() {
        let c = chars[i];
        let run = |ch: char| {
            chars[i..]
                .iter()
                .take_while(|x| x.eq_ignore_ascii_case(&ch))
                .count()
        };
        let rest: String = chars[i..]
            .iter()
            .take(5)
            .collect::<String>()
            .to_ascii_lowercase();
        if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                lit(&mut out, chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '\\' {
            if let Some(&n) = chars.get(i + 1) {
                lit(&mut out, n);
            }
            i += 2;
            continue;
        }
        if rest.starts_with("am/pm") {
            out.push(DatePart::AmPm {
                lower: chars[i] == 'a',
            });
            i += 5;
            continue;
        }
        match c.to_ascii_lowercase() {
            'y' => {
                let n = run('y');
                out.push(if n >= 3 {
                    DatePart::Year4
                } else {
                    DatePart::Year2
                });
                i += n;
            }
            'm' => {
                let n = run('m');
                out.push(match n {
                    1 => DatePart::Month,
                    2 => DatePart::Month2,
                    3 => DatePart::MonthShort,
                    _ => DatePart::MonthName,
                });
                i += n;
            }
            'd' => {
                let n = run('d');
                out.push(match n {
                    1 => DatePart::Day,
                    2 => DatePart::Day2,
                    3 => DatePart::DayShort,
                    _ => DatePart::DayName,
                });
                i += n;
            }
            'h' => {
                let n = run('h');
                out.push(if n >= 2 {
                    DatePart::Hour2
                } else {
                    DatePart::Hour
                });
                i += n;
            }
            's' => {
                let n = run('s');
                out.push(if n >= 2 {
                    DatePart::Second2
                } else {
                    DatePart::Second
                });
                i += n;
            }
            _ => {
                lit(&mut out, c);
                i += 1;
            }
        }
    }
    // Months next to hours or seconds are minutes.
    for k in 0..out.len() {
        if !matches!(out[k], DatePart::Month | DatePart::Month2) {
            continue;
        }
        let before = out[..k]
            .iter()
            .rev()
            .find(|p| !matches!(p, DatePart::Text(_)));
        let after = out[k + 1..]
            .iter()
            .find(|p| !matches!(p, DatePart::Text(_)));
        let hour_before = before.is_some_and(|p| matches!(p, DatePart::Hour | DatePart::Hour2));
        let sec_after = after.is_some_and(|p| matches!(p, DatePart::Second | DatePart::Second2));
        if hour_before || sec_after {
            out[k] = if out[k] == DatePart::Month {
                DatePart::Minute
            } else {
                DatePart::Minute2
            };
        }
    }
    if !out.iter().any(|p| !matches!(p, DatePart::Text(_))) {
        return Err(bad(
            "the date format needs at least one of yyyy, mm, dd, hh, mm, ss",
        ));
    }
    Ok(out)
}

/// Read a date written in the pattern `f` (from [`date_format`]). Digits
/// for `yyyy`, `mm`, `dd`, `hh`, `ss` are taken exactly (so `yyyymmdd`
/// reads `20261113`); `m`, `d`, `h` take one or two. A separator in the
/// pattern (`-`, `/`, `.`, space, comma) matches any run of separators.
fn read_date(t: &str, f: &[DatePart]) -> Option<f64> {
    let s: Vec<char> = t.trim().chars().collect();
    let mut i = 0;
    let sep = |c: char| matches!(c, '-' | '/' | '.' | ' ' | ',');
    let num = |i: &mut usize, min: usize, max: usize| -> Option<u32> {
        let n = s[*i..]
            .iter()
            .take(max)
            .take_while(|c| c.is_ascii_digit())
            .count();
        if n < min {
            return None;
        }
        let v: String = s[*i..*i + n].iter().collect();
        *i += n;
        v.parse().ok()
    };
    let word = |i: &mut usize| -> String {
        let n = s[*i..]
            .iter()
            .take_while(|c| c.is_alphabetic() || **c == '.')
            .count();
        let w: String = s[*i..*i + n].iter().collect::<String>().to_lowercase();
        *i += n;
        w
    };
    let (mut y, mut mo, mut d) = (None::<i64>, None::<u32>, None::<u32>);
    let (mut h, mut mi, mut se, mut pm) = (None::<u32>, 0u32, 0u32, None::<bool>);
    for p in f {
        match p {
            DatePart::Text(x) => {
                for c in x.chars() {
                    if sep(c) {
                        while i < s.len() && sep(s[i]) {
                            i += 1;
                        }
                    } else if s
                        .get(i)
                        .is_some_and(|x| x.to_lowercase().eq(c.to_lowercase()))
                    {
                        i += 1;
                    } else {
                        return None;
                    }
                }
            }
            DatePart::Year4 => y = Some(i64::from(num(&mut i, 4, 4)?)),
            DatePart::Year2 => {
                let v = i64::from(num(&mut i, 2, 2)?);
                y = Some(if v < 30 { 2000 + v } else { 1900 + v });
            }
            DatePart::Month2 => mo = Some(num(&mut i, 2, 2)?),
            DatePart::Month => mo = Some(num(&mut i, 1, 2)?),
            DatePart::MonthName | DatePart::MonthShort => {
                mo = Some(crate::value::month_of(&word(&mut i))?);
            }
            DatePart::Day2 => d = Some(num(&mut i, 2, 2)?),
            DatePart::Day => d = Some(num(&mut i, 1, 2)?),
            DatePart::DayName | DatePart::DayShort => {
                if word(&mut i).is_empty() {
                    return None;
                }
            }
            DatePart::Hour2 => h = Some(num(&mut i, 2, 2)?),
            DatePart::Hour => h = Some(num(&mut i, 1, 2)?),
            DatePart::Minute2 => mi = num(&mut i, 2, 2)?,
            DatePart::Minute => mi = num(&mut i, 1, 2)?,
            DatePart::Second2 => se = num(&mut i, 2, 2)?,
            DatePart::Second => se = num(&mut i, 1, 2)?,
            DatePart::AmPm { .. } => {
                pm = match word(&mut i).as_str() {
                    "am" | "a.m." => Some(false),
                    "pm" | "p.m." => Some(true),
                    _ => return None,
                }
            }
        }
    }
    if i != s.len() {
        return None;
    }
    let hour = match (h, pm) {
        (Some(h), Some(p)) if (1..=12).contains(&h) => h % 12 + if p { 12 } else { 0 },
        (Some(_), Some(_)) => return None,
        (Some(h), None) => h,
        (None, _) => 0,
    };
    if hour > 23 || mi > 59 || se > 59 {
        return None;
    }
    let secs = f64::from(hour * 3600 + mi * 60 + se) / 86_400.0;
    let days = match (y, mo) {
        (Some(y), Some(m)) => crate::value::date_days(y, m, d.unwrap_or(1))?,
        (None, None) if d.is_none() && h.is_some() => 0,
        _ => return None,
    };
    Some(days as f64 + secs)
}

/// A date `v` (days since 1970-01-01) as its year, year and month, or age
/// band on the day `now`; none for a date after `now` in an age band.
fn date_band(v: f64, to: DateTo, now: f64) -> Option<String> {
    let (y, m, d) = crate::fill::civil_from_days(v.floor() as i64);
    match to {
        DateTo::Year => Some(format!("{y:04}")),
        DateTo::YearMonth => Some(format!("{y:04}-{m:02}")),
        DateTo::AgeBand { size, top } => {
            let (ny, nm, nd) = crate::fill::civil_from_days(now.floor() as i64);
            let mut age = ny - y;
            if (nm, nd) < (m, d) {
                age -= 1;
            }
            if age < 0 {
                return None;
            }
            let age = age as u32;
            if top > 0 && age >= top {
                return Some(format!("{top}+"));
            }
            if size == 1 {
                return Some(age.to_string());
            }
            let lo = age / size * size;
            let hi = lo + size - 1;
            Some(if top > 0 {
                format!("{lo}-{}", hi.min(top - 1))
            } else {
                format!("{lo}-{hi}")
            })
        }
    }
}

/// A date written `yyyymmdd` with no separators (as many voter and
/// payroll exports do), between the years 1800 and 2199.
fn compact_date(t: &str) -> Option<f64> {
    let t = t.trim();
    if t.len() != 8 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i64 = t[..4].parse().ok()?;
    let m: u32 = t[4..6].parse().ok()?;
    let d: u32 = t[6..].parse().ok()?;
    if !(1800..2200).contains(&y) || !(1..=12).contains(&m) || d == 0 {
        return None;
    }
    let days = crate::value::days_from_civil(y, m, d);
    // 31 February and the like.
    (crate::fill::civil_from_days(days) == (y, m, d)).then_some(days as f64)
}

/// A year on its own (`1980`) as a year or an age band (the age it gives
/// may be a year too high before the birthday).
fn year_band(t: &str, to: DateTo, now: f64) -> Option<String> {
    let t = t.trim();
    if t.len() != 4 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i64 = t.parse().ok()?;
    if !(1800..2200).contains(&y) {
        return None;
    }
    match to {
        DateTo::Year => Some(t.to_string()),
        DateTo::YearMonth => None,
        DateTo::AgeBand { .. } => date_band(crate::value::days_from_civil(y, 1, 1) as f64, to, now),
    }
}

/// The same day and time a hundred years earlier (29 February: the 28th).
fn century_back(v: f64) -> f64 {
    let days = v.floor();
    let (y, m, d) = crate::fill::civil_from_days(days as i64);
    let d = if m == 2 && d == 29 { 28 } else { d };
    crate::value::days_from_civil(y - 100, m, d) as f64 + (v - days)
}

/// A numeric date like `13/04/2026` that can only be read the other way
/// round from `order` (its first number is over 12 when reading month
/// first, its second when reading day first).
fn forced_order(t: &str, order: crate::value::DateOrder) -> bool {
    let parts: Vec<&str> = t.trim().split(['/', '-', '.']).collect();
    if parts.len() != 3 || parts[0].len() > 2 || parts[1].len() > 2 {
        return false;
    }
    let num = |p: &str| -> Option<u32> {
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse().ok())?
    };
    let (Some(x), Some(y)) = (num(parts[0]), num(parts[1])) else {
        return false;
    };
    if !parts[2].trim().chars().take(4).all(|c| c.is_ascii_digit()) {
        return false;
    }
    match order {
        crate::value::DateOrder::Mdy => x > 12 && y <= 12,
        crate::value::DateOrder::Dmy => y > 12 && x <= 12,
    }
}

/// A cell that holds a time but no date (day 0 with no date written).
fn time_only(t: &str, v: f64) -> bool {
    v.floor() == 0.0
        && t.contains(':')
        && !t.contains(['/', '-', '.', ','])
        && !t.split(|c: char| !c.is_alphabetic()).any(|w| {
            !w.is_empty() && !w.eq_ignore_ascii_case("am") && !w.eq_ignore_ascii_case("pm")
        })
}

fn write_date(v: f64, f: &[DatePart]) -> String {
    let days = v.floor();
    let mut secs = ((v - days) * 86_400.0).round() as i64;
    let mut days = days as i64;
    if secs >= 86_400 {
        secs -= 86_400;
        days += 1;
    }
    let (y, m, d) = crate::fill::civil_from_days(days);
    let (h, mi, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let twelve = f.iter().any(|p| matches!(p, DatePart::AmPm { .. }));
    let hh = if twelve {
        match h % 12 {
            0 => 12,
            x => x,
        }
    } else {
        h
    };
    let wd = days.rem_euclid(7) as usize;
    let mut out = String::new();
    for p in f {
        match p {
            DatePart::Text(t) => out.push_str(t),
            DatePart::Year4 => out.push_str(&format!("{y:04}")),
            DatePart::Year2 => out.push_str(&format!("{:02}", y.rem_euclid(100))),
            DatePart::MonthName => out.push_str(MONTHS[m as usize - 1]),
            DatePart::MonthShort => out.push_str(&MONTHS[m as usize - 1][..3]),
            DatePart::Month2 => out.push_str(&format!("{m:02}")),
            DatePart::Month => out.push_str(&m.to_string()),
            DatePart::DayName => out.push_str(DAYS[wd]),
            DatePart::DayShort => out.push_str(&DAYS[wd][..3]),
            DatePart::Day2 => out.push_str(&format!("{d:02}")),
            DatePart::Day => out.push_str(&d.to_string()),
            DatePart::Hour2 => out.push_str(&format!("{hh:02}")),
            DatePart::Hour => out.push_str(&hh.to_string()),
            DatePart::Minute2 => out.push_str(&format!("{mi:02}")),
            DatePart::Minute => out.push_str(&mi.to_string()),
            DatePart::Second2 => out.push_str(&format!("{s:02}")),
            DatePart::Second => out.push_str(&s.to_string()),
            DatePart::AmPm { lower } => out.push_str(match (h < 12, lower) {
                (true, false) => "AM",
                (false, false) => "PM",
                (true, true) => "am",
                (false, true) => "pm",
            }),
        }
    }
    out
}

/// The current moment as days since 1970-01-01 (UTC) with the time as a
/// fraction; the shell passes local time.
pub fn now_utc() -> f64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    t / 86_400.0
}

// ---- names ----

/// A person's name in parts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Name {
    pub title: String,
    pub first: String,
    pub middle: String,
    pub last: String,
    pub suffix: String,
}

const TITLES: &[&str] = &[
    "mr", "mrs", "ms", "miss", "mx", "dr", "prof", "rev", "sir", "dame", "lord", "lady", "fr",
    "hon", "capt", "col", "gen", "lt", "sgt",
];
const SUFFIXES: &[&str] = &[
    "jr", "sr", "ii", "iii", "iv", "v", "phd", "md", "dds", "esq", "cpa", "rn", "dvm", "jd",
];
const PARTICLES: &[&str] = &[
    "van", "von", "de", "da", "del", "della", "der", "di", "du", "la", "le", "st", "bin", "ibn",
    "al", "ben", "dos", "das", "ter", "ten",
];

fn word_key(w: &str) -> String {
    w.trim_matches(|c: char| c == '.' || c == ',')
        .to_lowercase()
}

/// Split a person's name: "Dr. Mary Ann van der Berg Jr." or
/// "van der Berg, Mary Ann".
pub fn split_name(s: &str) -> Name {
    let s = s.trim();
    let mut n = Name::default();
    let (front, back): (String, Option<String>) = match s.split_once(',') {
        // "Last, First Middle" (unless what follows the comma is a suffix:
        // "Mary Smith, Jr.").
        Some((a, b))
            if !b.trim().is_empty()
                && !b
                    .split_whitespace()
                    .all(|w| SUFFIXES.contains(&word_key(w).as_str())) =>
        {
            (b.trim().to_string(), Some(a.trim().to_string()))
        }
        _ => (s.replace(',', " "), None),
    };
    let mut words: Vec<&str> = front.split_whitespace().collect();
    while let Some(w) = words.first() {
        if TITLES.contains(&word_key(w).as_str()) && words.len() > 1 {
            n.title = join(&n.title, w);
            words.remove(0);
        } else {
            break;
        }
    }
    let mut back_words: Vec<&str> = back
        .as_deref()
        .map(|b| b.split_whitespace().collect())
        .unwrap_or_default();
    // Suffixes at the end of the last name or the whole name.
    let tail = if back.is_some() {
        &mut back_words
    } else {
        &mut words
    };
    let mut suffix: Vec<&str> = Vec::new();
    while tail.len() > 1 {
        let w = tail[tail.len() - 1];
        if SUFFIXES.contains(&word_key(w).as_str()) {
            suffix.insert(0, w);
            tail.pop();
        } else {
            break;
        }
    }
    if back.is_some() {
        // Suffixes may also follow the first names ("Smith, John Jr.").
        while words.len() > 1 && SUFFIXES.contains(&word_key(words[words.len() - 1]).as_str()) {
            suffix.insert(0, words.pop().unwrap_or_default());
        }
    }
    n.suffix = suffix.join(" ");
    if back.is_some() {
        n.last = back_words.join(" ");
        if let Some((f, rest)) = words.split_first() {
            n.first = f.to_string();
            n.middle = rest.join(" ");
        }
        return n;
    }
    match words.len() {
        0 => {}
        1 => n.first = words[0].to_string(),
        _ => {
            // The last name starts at its first particle ("van der Berg");
            // a name that starts with one is all last name ("de la Cruz").
            let mut start = words.len() - 1;
            while start > 0 && PARTICLES.contains(&word_key(words[start - 1]).as_str()) {
                start -= 1;
            }
            if start == 0 {
                n.last = words.join(" ");
                return n;
            }
            n.first = words[0].to_string();
            n.last = words[start..].join(" ");
            n.middle = words[1..start].join(" ");
        }
    }
    n
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a} {b}"),
    }
}

fn name_part(n: &Name, part: NamePart) -> String {
    match part {
        NamePart::Title => n.title.clone(),
        NamePart::First => n.first.clone(),
        NamePart::Middle => n.middle.clone(),
        NamePart::Last => n.last.clone(),
        NamePart::Suffix => n.suffix.clone(),
        NamePart::FirstLast => [&n.first, &n.middle, &n.last]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        NamePart::LastFirst => {
            let given = join(&n.first, &n.middle);
            match (n.last.is_empty(), given.is_empty()) {
                (true, _) => given,
                (false, true) => n.last.clone(),
                _ => format!("{}, {given}", n.last),
            }
        }
    }
}

// ---- HTML ----

// ---- M10 steps ----

fn is_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// `t` without accents, or `None` when it has none.
/// A value with accents taken off (`Seán` -> `Sean`).
pub(crate) fn fold_accents(t: &str) -> std::borrow::Cow<'_, str> {
    match accents(t) {
        Some(s) => std::borrow::Cow::Owned(s),
        None => std::borrow::Cow::Borrowed(t),
    }
}

fn accents(t: &str) -> Option<String> {
    if t.is_ascii() {
        return None;
    }
    let mut out = String::with_capacity(t.len());
    let mut changed = false;
    let mut tmp = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if is_mark(c) {
            changed = true;
            continue;
        }
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        // Capital I with a dot (Turkish) lowers to two characters.
        if c == '\u{130}' {
            out.push('I');
            changed = true;
            continue;
        }
        let mut lower = c.to_lowercase();
        let l = match (lower.next(), lower.next()) {
            (Some(l), None) => l,
            _ => {
                out.push(c);
                continue;
            }
        };
        tmp.clear();
        crate::value::fold_char(l, &mut tmp);
        if tmp.len() == l.len_utf8() && tmp.starts_with(l) {
            out.push(c);
            continue;
        }
        changed = true;
        if c.is_uppercase() {
            // Þór → Thor, ÞÓR → THOR: a capital that becomes several letters
            // is capitalized only at the front when lower case follows.
            let title = chars.peek().is_some_and(|n| n.is_lowercase());
            for (i, ch) in tmp.chars().enumerate() {
                if i > 0 && title {
                    out.push(ch);
                } else {
                    out.extend(ch.to_uppercase());
                }
            }
        } else {
            out.push_str(&tmp);
        }
    }
    changed.then_some(out)
}

/// Days since 1970 of a Julian date (`yyddd`, `yyyyddd`).
fn julian(s: &str) -> Option<i64> {
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (y, d): (i64, i64) = match s.len() {
        5 => {
            let y: i64 = s[..2].parse().ok()?;
            (
                if y < 50 { 2000 + y } else { 1900 + y },
                s[2..].parse().ok()?,
            )
        }
        // cyyddd: century digit 0 for the 1900s, 1 for the 2000s.
        6 => {
            let c: i64 = s[..1].parse().ok()?;
            if c > 2 {
                return None;
            }
            let y: i64 = s[1..3].parse().ok()?;
            (1900 + c * 100 + y, s[3..].parse().ok()?)
        }
        7 => (s[..4].parse().ok()?, s[4..].parse().ok()?),
        _ => return None,
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    if !(1..=(365 + i64::from(leap))).contains(&d) || !(1000..=9999).contains(&y) {
        return None;
    }
    Some(crate::value::days_from_civil(y, 1, 1) + d - 1)
}

fn fix_number(how: NumberFix, s: &str, read: ReadOptions) -> Option<String> {
    let num = |x: &str| number_value(x, read).is_some();
    match how {
        NumberFix::TrailingMinus => {
            if let Some(b) = s.strip_suffix('-') {
                let b = b.trim_end();
                (num(b) && !b.starts_with('-')).then(|| format!("-{b}"))
            } else if let Some(b) = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
                let b = b.trim();
                (num(b) && !b.starts_with('-')).then(|| format!("-{b}"))
            } else {
                // Already written the usual way: no change, not "left".
                num(s).then(|| s.to_string())
            }
        }
        NumberFix::FlipSign | NumberFix::Positive | NumberFix::Negative => {
            if !num(s) {
                return None;
            }
            // (5) is a negative 5, written the accounting way.
            let (neg, body) = match s.strip_prefix('-') {
                Some(b) => (true, b.trim_start()),
                None => match s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
                    Some(b) => (true, b.trim()),
                    None => (false, s.strip_prefix('+').unwrap_or(s)),
                },
            };
            let zero = number_value(body, read) == Some(0.0);
            let want_neg = match how {
                NumberFix::FlipSign => !neg,
                NumberFix::Positive => false,
                _ => true,
            };
            Some(if want_neg && !zero {
                format!("-{body}")
            } else {
                body.to_string()
            })
        }
        NumberFix::Percent => {
            let b = s.strip_suffix('%')?.trim_end();
            let v = number_value(b, read)?;
            Some(exact_number(v / 100.0, read.decimal_comma))
        }
        NumberFix::Julian => {
            let days = julian(s)?;
            Some(write_date(
                days as f64,
                &[
                    DatePart::Year4,
                    DatePart::Text("-".into()),
                    DatePart::Month2,
                    DatePart::Text("-".into()),
                    DatePart::Day2,
                ],
            ))
        }
    }
}

fn calc(op: CalcOp, v: f64, x: f64, read: ReadOptions) -> Option<String> {
    let comma = |s: String| {
        if read.decimal_comma {
            s.replace('.', ",")
        } else {
            s
        }
    };
    let r = match op {
        CalcOp::Add => v + x,
        CalcOp::Subtract => v - x,
        CalcOp::Multiply => v * x,
        CalcOp::Divide => v / x,
        CalcOp::Power => v.powf(x),
        CalcOp::Abs => v.abs(),
        CalcOp::Round => return Some(comma(no_trailing_zeros(round_text(v, x as usize)))),
        CalcOp::RoundUp | CalcOp::RoundDown => {
            let d = x as i32;
            // Work on the shortest decimal form so 2.1 * 10 is 21, not 21.000000000000004.
            let m = 10f64.powi(d);
            let y: f64 = exact_number(v * m, false).parse().ok()?;
            let z = match op {
                CalcOp::RoundUp if y >= 0.0 => y.ceil(),
                CalcOp::RoundUp => y.floor(),
                _ => y.trunc(),
            };
            return Some(comma(no_trailing_zeros(round_text(z / m, d as usize))));
        }
    };
    if !r.is_finite() || r.abs() > 1e300 {
        return None;
    }
    Some(exact_number(r, read.decimal_comma))
}

/// A calculated number written in full: float noise past 15 significant
/// digits removed (0.1 + 0.2 is 0.3), every real digit kept
/// (1234567890.12345 + 0 stays 1234567890.12345), never in e-notation.
pub(crate) fn exact_number(v: f64, comma: bool) -> String {
    let r: f64 = format!("{v:.14e}").parse().unwrap_or(v);
    let mut s = format!("{r}");
    if s == "-0" {
        s = "0".into();
    }
    if comma {
        s.replace('.', ",")
    } else {
        s
    }
}

/// 8.20 → 8.2, 8.00 → 8: rounding never adds digits a value did not have.
fn no_trailing_zeros(mut s: String) -> String {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

fn sort_parts(t: &str, sep: &str, descending: bool, distinct: bool, trim: bool) -> Option<String> {
    if !t.contains(sep) {
        return None;
    }
    let mut parts: Vec<&str> = t.split(sep).collect();
    if trim {
        parts = parts
            .into_iter()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
    }
    let mut keyed: Vec<(String, &str)> = parts
        .into_iter()
        .map(|p| {
            let mut k = String::new();
            crate::value::text_key(p, false, &mut k);
            (k, p)
        })
        .collect();
    keyed.sort_by(|a, b| {
        crate::value::natural_cmp(a.0.as_bytes(), b.0.as_bytes()).then_with(|| a.1.cmp(b.1))
    });
    if descending {
        keyed.reverse();
    }
    if distinct {
        let mut seen = std::collections::HashSet::new();
        keyed.retain(|(_, p)| seen.insert(*p));
    }
    let out: Vec<&str> = keyed.into_iter().map(|(_, p)| p).collect();
    Some(out.join(sep))
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `%XX` escapes as bytes, read as UTF-8; `None` for a bad escape or bytes
/// that are not UTF-8.
fn url_decode(t: &str) -> Option<String> {
    let b = t.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = hex_digit(*b.get(i + 1)?)?;
            let l = hex_digit(*b.get(i + 2)?)?;
            out.push((h << 4) | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn url_encode(t: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(t.len() * 3);
    for &b in t.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn html_encode(t: &str) -> String {
    let mut out = String::with_capacity(t.len() + 8);
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// The text of an HTML fragment: tags dropped, scripts and styles skipped,
/// `<br>` and block ends kept as line breaks, entities decoded, spaces
/// collapsed as a browser shows them. Linear in the input.
pub fn html_text(t: &str) -> String {
    let b = t.as_bytes();
    let mut raw = String::with_capacity(t.len());
    let mut i = 0;
    let mut from = 0;
    while let Some(e) = memchr::memchr(b'<', &b[i..]) {
        let k = i + e;
        raw.push_str(&t[from..k]);
        if b[k..].starts_with(b"<!--") {
            i = t[k..].find("-->").map_or(b.len(), |e| k + e + 3);
            from = i;
            continue;
        }
        let end = memchr::memchr(b'>', &b[k..]).map_or(b.len(), |e| k + e + 1);
        let mut tag_end = end.saturating_sub(1).max(k + 1).min(k + 1 + 64);
        while !t.is_char_boundary(tag_end) {
            tag_end -= 1;
        }
        let tag = t[k + 1..tag_end].to_ascii_lowercase();
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if name.is_empty() && !closing {
            // A lone `<` is text.
            raw.push('<');
            i = k + 1;
            from = i;
            continue;
        }
        i = end;
        from = end;
        if !closing && (name == "script" || name == "style") {
            i = crate::import::find_ci(b, i, format!("</{name}").as_bytes());
            i = memchr::memchr(b'>', &b[i..]).map_or(b.len(), |e| i + e + 1);
            from = i;
            continue;
        }
        let block = matches!(
            name.as_str(),
            "br" | "p"
                | "div"
                | "li"
                | "tr"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "ul"
                | "ol"
                | "table"
        );
        if block && (name == "br" || closing) {
            raw.push('\u{1}');
        } else if matches!(name.as_str(), "td" | "th") && closing {
            raw.push(' ');
        }
    }
    raw.push_str(&t[from..]);
    let mut dec = String::with_capacity(raw.len());
    crate::import::html_unescape(&raw, &mut dec);
    // Spaces collapsed within each line; empty lines dropped.
    dec.split('\u{1}')
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- running on a document ----

/// A cleaner run: what, where, and how dates and numbers are read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanSpec {
    pub cleaner: Cleaner,
    pub scope: Scope,
    /// Leave the first row (column titles) out of whole-column scopes.
    #[serde(default)]
    pub skip_first: bool,
    #[serde(default)]
    pub read: ReadOptions,
}

impl CleanSpec {
    /// The step as a recipe records it: every dates step says what it does
    /// with a date both day orders can read. One that did not say reads it by
    /// the day order it ran with.
    pub fn recorded(&self) -> CleanSpec {
        let mut spec = self.clone();
        record_ambiguous(&mut spec.cleaner.steps, spec.read.date_order);
        spec
    }
}

impl CleanMatrix {
    /// As a recipe records it (see [`CleanSpec::recorded`]).
    pub fn recorded(&self) -> CleanMatrix {
        let mut out = self.clone();
        for e in &mut out.entries {
            record_ambiguous(&mut e.steps, self.read.date_order);
        }
        out
    }
}

/// Every dates step that does not say what it does with ambiguous dates
/// reads them by `order`: written down.
fn record_ambiguous(steps: &mut [Step], order: crate::value::DateOrder) {
    let order = match order {
        crate::value::DateOrder::Mdy => Ambiguous::Mdy,
        crate::value::DateOrder::Dmy => Ambiguous::Dmy,
    };
    for step in steps {
        if let Step::Dates { ambiguous, .. } = step {
            ambiguous.get_or_insert(order);
        }
    }
}

/// The first changes of a run, for the cleaning screen.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct CleanPreview {
    pub changes: Vec<Change>,
    /// Rows looked at.
    pub rows: u64,
    /// Cells changed among them.
    pub changed: u64,
    /// Cells left as they were (not a date or number) among them.
    pub left: u64,
    /// Dates read the other way round from the chosen order among them.
    pub other_order: u64,
    /// Rows past those looked at.
    pub more: bool,
    /// Sanitize only: changed cells that hold a look-alike letter fixed to its Latin twin (names on voter
    /// lists matter, so the count is shown before Apply).
    pub homoglyph_cells: u64,
}

/// A computed run, applied with [`Document::apply_clean`].
#[derive(Debug)]
pub struct CleanPlan {
    pub(crate) cells: CellPlan,
    /// Cells that change.
    pub changed: u64,
    /// Cells a step could not read (not a date or number) and left alone.
    pub left: u64,
    /// Dates that could only be read the other way round from the chosen
    /// day order: the column mixes day-first and month-first dates.
    pub other_order: u64,
    /// Dates both day orders can read, left as typed because a step said so.
    pub ambiguous_left: u64,
    /// The first changes, for showing before applying.
    pub preview: Vec<Change>,
}

/// Most columns in a matrix.
pub const MAX_MATRIX_COLS: usize = 10_000;
/// A matrix saved in a recipe is at most this large.
pub const MAX_MATRIX_BYTES: usize = 4 << 20;
/// Example changes shown for each column of a matrix.
pub const MATRIX_EXAMPLES: usize = 3;

/// One column of a clean matrix: its steps, run in the order stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixEntry {
    pub col: u32,
    /// The column's title when the matrix was made.
    #[serde(default)]
    pub title: String,
    pub steps: Vec<Step>,
    /// The name of the cleaner file whose steps were copied in.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from_file: String,
}

impl MatrixEntry {
    fn cleaner(&self) -> Cleaner {
        Cleaner {
            version: version(),
            name: String::new(),
            steps: self.steps.clone(),
        }
    }
}

/// Columns by cleaners: each column with its own steps, run in one pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanMatrix {
    pub entries: Vec<MatrixEntry>,
    /// Leave the first row (column titles) out.
    #[serde(default)]
    pub skip_first: bool,
    #[serde(default)]
    pub read: ReadOptions,
}

impl CleanMatrix {
    /// Check sizes and steps (a recipe file is untrusted).
    pub fn check(&self) -> Result<(), Error> {
        if self.entries.is_empty() || self.entries.len() > MAX_MATRIX_COLS {
            return Err(bad(format!(
                "tick at least one column and at most {MAX_MATRIX_COLS}"
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for e in &self.entries {
            if !seen.insert(e.col) {
                return Err(bad("a column is in the clean matrix twice"));
            }
            if e.steps.is_empty() {
                return Err(bad("a column of the clean matrix has nothing ticked"));
            }
            if e.from_file.chars().count() > MAX_NAME_CHARS
                || e.title.chars().count() > MAX_REPLACEMENT_CHARS
            {
                return Err(bad("a name in the clean matrix is too long"));
            }
            if e.steps.iter().any(|s| matches!(s, Step::Scramble { .. })) {
                return Err(bad(
                    "the clean matrix cannot scramble (the phrase is never saved): use Data > Clean",
                ));
            }
            if e.steps.len() > MAX_STEPS {
                return Err(bad(format!("a cleaner can have at most {MAX_STEPS} steps")));
            }
            for s in &e.steps {
                s.check()?;
            }
        }
        let size = serde_json::to_vec(self)
            .map_err(|e| bad(e.to_string()))?
            .len();
        if size > MAX_MATRIX_BYTES {
            return Err(bad("the clean matrix is too large"));
        }
        Ok(())
    }
}

/// What a matrix does to one column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatrixColumn {
    pub col: u32,
    /// Cells that change.
    pub changed: u64,
    /// Cells that would change in a protected column: left alone.
    pub protected: u64,
    /// Cells a step could not read.
    pub left: u64,
    pub other_order: u64,
    pub homoglyph_cells: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct MatrixPreview {
    /// The document revision counted: Apply is refused at any other.
    pub revision: u64,
    pub columns: Vec<MatrixColumn>,
    /// The first changes of each column.
    pub examples: Vec<Change>,
}

/// A computed matrix: apply `plan` with [`Document::apply_clean`].
#[derive(Debug)]
pub struct MatrixPlan {
    pub plan: CleanPlan,
    pub columns: Vec<MatrixColumn>,
}

/// How [`Document::run_cells`] runs.
struct Cells<'a> {
    /// The cleaner of the area's `k`th column.
    compiled: &'a (dyn Fn(usize) -> &'a Compiled + Sync),
    seed: u64,
    /// Keep the new texts (a plan), or only count (a preview).
    keep: bool,
    examples: usize,
    /// `examples` of each column, not in all.
    per_column: bool,
    /// Columns whose changed cells are checked for look-alike letters.
    homoglyphs: &'a [bool],
    /// Columns that give no examples (per column only).
    no_examples: &'a [bool],
}

#[derive(Debug, Clone, Copy, Default)]
struct Tally {
    changed: u64,
    left: u64,
    other_order: u64,
    ambiguous_left: u64,
    homoglyphs: u64,
}

struct CellRun {
    changes: Vec<(u32, Vec<(u32, String)>)>,
    examples: Vec<Change>,
    columns: Vec<Tally>,
}

impl CellRun {
    fn total(&self) -> Tally {
        self.columns.iter().fold(Tally::default(), |mut a, b| {
            a.changed += b.changed;
            a.left += b.left;
            a.other_order += b.other_order;
            a.ambiguous_left += b.ambiguous_left;
            a
        })
    }
}

/// Whether a Sanitize step is set to fix look-alike letters.
fn fixes_homoglyphs(steps: &[Step]) -> bool {
    steps.iter().any(|s| {
        matches!(
            s,
            Step::Sanitize {
                options: crate::xray::SanitizeOptions {
                    homoglyphs: true,
                    ..
                }
            }
        )
    })
}

fn has_homoglyph(t: &str) -> bool {
    let mut found = Vec::new();
    crate::xray::scan(t, &mut found);
    found
        .iter()
        .any(|g| g.kind == crate::xray::GremlinKind::Homoglyph)
}

impl CleanPlan {
    /// Put cells the person typed themselves (shown row, column, text) into
    /// the run, over whatever it would have written there: one undo step
    /// takes back both.
    pub fn set_typed(&mut self, doc: &Document, typed: &[(u64, u32, String)]) -> Result<(), Error> {
        let mut at: std::collections::HashMap<u32, usize> = self
            .cells
            .changes
            .iter()
            .enumerate()
            .map(|(i, c)| (c.0, i))
            .collect();
        for (row, col, text) in typed {
            let id = doc.id_at(*row).ok_or(Error::OutOfRange)?;
            if *col >= doc.col_count() {
                return Err(Error::OutOfRange);
            }
            let i = *at.entry(id).or_insert_with(|| {
                self.cells.changes.push((id, Vec::new()));
                self.cells.changes.len() - 1
            });
            let cells = &mut self.cells.changes[i].1;
            match cells.binary_search_by_key(col, |c| c.0) {
                Ok(k) => cells[k].1 = text.clone(),
                Err(k) => {
                    cells.insert(k, (*col, text.clone()));
                    self.changed += 1;
                    self.cells.cells += 1;
                }
            }
        }
        Ok(())
    }
}

impl Document {
    /// Plan running a cleaner. `now` is the run's moment (see
    /// [`Compiled::new`]); `seed` makes random text differ between runs.
    pub fn plan_clean(
        &self,
        spec: &CleanSpec,
        now: f64,
        seed: u64,
        preview: usize,
        progress: &Progress,
    ) -> Result<CleanPlan, Error> {
        self.ready_for_bulk()?;
        let c = Compiled::new(&spec.cleaner, spec.read, now)?;
        let area = self.area(&FindSpec {
            pattern: Pattern::default(),
            scope: spec.scope.clone(),
            skip_first: spec.skip_first,
            skip_cols: Vec::new(),
            ..Default::default()
        })?;
        let (r0, r1) = (area.rows.start, area.rows.end);
        progress.start(r1.saturating_sub(r0));
        let run = self.run_cells(
            &area,
            &Cells {
                compiled: &|_| &c,
                seed,
                keep: true,
                examples: preview,
                per_column: false,
                homoglyphs: &[],
                no_examples: &[],
            },
            progress,
        )?;
        let total = run.total();
        let mut plan = CellPlan::new(self);
        plan.changes = run.changes;
        plan.cells = total.changed;
        Ok(CleanPlan {
            cells: plan,
            changed: total.changed,
            left: total.left,
            other_order: total.other_order,
            ambiguous_left: total.ambiguous_left,
            preview: run.examples,
        })
    }

    /// Run a compiled cleaner (one per searched column) over `area` in one
    /// parallel pass: the loop under every cleaning run.
    fn run_cells(
        &self,
        area: &Area,
        how: &Cells<'_>,
        progress: &Progress,
    ) -> Result<CellRun, Error> {
        let (r0, r1) = (area.rows.start, area.rows.end);
        let created = AtomicUsize::new(0);
        let width = area.cols.len();
        let blocks = r1.saturating_sub(r0).div_ceil(BLOCK);
        let parts: Vec<CellRun> = (0..blocks)
            .into_par_iter()
            .map(|b| -> Result<CellRun, Error> {
                let s = r0 + b * BLOCK;
                let e = (s + BLOCK).min(r1);
                let mut part = CellRun {
                    changes: Vec::new(),
                    examples: Vec::new(),
                    columns: vec![Tally::default(); width],
                };
                let mut buf = Vec::new();
                let seed = how.seed;
                for r in s..e {
                    let Some(id) = self.id_at(r) else { continue };
                    let mut row: Vec<(u32, String)> = Vec::new();
                    let mut err = None;
                    self.each_cell(area, r, false, &mut buf, |k, t| {
                        let col = area.cols[k];
                        let ctx = Ctx {
                            index: r - r0,
                            seed: mix(seed ^ mix(r ^ (u64::from(col) << 40))),
                            run: mix(seed ^ (u64::from(col) << 40)),
                        };
                        match (how.compiled)(k).run(t, ctx) {
                            Ok(out) => {
                                let tally = &mut part.columns[k];
                                tally.left += u64::from(out.left);
                                tally.other_order += u64::from(out.other_order);
                                tally.ambiguous_left += u64::from(out.ambiguous_left);
                                let new = out.text?;
                                if how.keep && !crate::find::charge(&created, new.len()) {
                                    err = Some(Error::OutOfMemory(
                                        "this would change too much at once; clean a smaller selection",
                                    ));
                                    return Some(());
                                }
                                tally.changed += 1;
                                if how.homoglyphs.get(k) == Some(&true) && has_homoglyph(t) {
                                    tally.homoglyphs += 1;
                                }
                                let room = if how.per_column {
                                    tally.changed <= how.examples as u64
                                        && how.no_examples.get(k) != Some(&true)
                                } else {
                                    part.examples.len() < how.examples
                                };
                                if room {
                                    part.examples.push(Change {
                                        row: r,
                                        number: self.row_number(r).unwrap_or(r),
                                        col,
                                        before: short(t),
                                        after: short(&new),
                                        hidden: false,
                                    });
                                }
                                if how.keep {
                                    row.push((col, new));
                                }
                                None
                            }
                            Err(e) => {
                                err = Some(e);
                                Some(())
                            }
                        }
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                    if !row.is_empty() {
                        part.changes.push((id, row));
                    }
                }
                progress.step(e - s)?;
                Ok(part)
            })
            .collect::<Result<_, _>>()?;
        let mut out = CellRun {
            changes: Vec::new(),
            examples: Vec::new(),
            columns: vec![Tally::default(); width],
        };
        // Examples wanted so far, per column or in all.
        let mut had = vec![0usize; width];
        for p in parts {
            out.changes.extend(p.changes);
            for x in p.examples {
                let k = area.cols.binary_search(&x.col).unwrap_or(0);
                let room = if how.per_column {
                    had[k] < how.examples
                } else {
                    out.examples.len() < how.examples
                };
                if room {
                    had[k] += 1;
                    out.examples.push(x);
                }
            }
            for (a, b) in out.columns.iter_mut().zip(p.columns) {
                a.changed += b.changed;
                a.left += b.left;
                a.other_order += b.other_order;
                a.ambiguous_left += b.ambiguous_left;
                a.homoglyphs += b.homoglyphs;
            }
        }
        Ok(out)
    }

    /// Plan a matrix: every column's steps in one pass over the shown rows.
    /// Cells of protected columns are counted apart and left alone when the
    /// plan is applied.
    pub fn plan_clean_matrix(
        &self,
        matrix: &CleanMatrix,
        now: f64,
        seed: u64,
        progress: &Progress,
    ) -> Result<MatrixPlan, Error> {
        let (run, columns) = self.run_matrix(matrix, now, seed, true, progress)?;
        let sum = |f: fn(&MatrixColumn) -> u64| columns.iter().map(f).sum::<u64>();
        let changed = sum(|c| c.changed);
        let mut cells = CellPlan::new(self);
        cells.changes = run.changes;
        cells.cells = changed;
        Ok(MatrixPlan {
            plan: CleanPlan {
                cells,
                changed,
                left: sum(|c| c.left),
                other_order: sum(|c| c.other_order),
                ambiguous_left: run.columns.iter().map(|t| t.ambiguous_left).sum(),
                preview: Vec::new(),
            },
            columns,
        })
    }

    /// What a matrix would change, column by column, with exact counts and
    /// the first [`MATRIX_EXAMPLES`] changes of each. Nothing new is stored.
    pub fn preview_clean_matrix(
        &self,
        matrix: &CleanMatrix,
        now: f64,
        seed: u64,
        progress: &Progress,
    ) -> Result<MatrixPreview, Error> {
        let (run, columns) = self.run_matrix(matrix, now, seed, false, progress)?;
        Ok(MatrixPreview {
            revision: self.revision(),
            columns,
            examples: run.examples,
        })
    }

    fn run_matrix(
        &self,
        matrix: &CleanMatrix,
        now: f64,
        seed: u64,
        keep: bool,
        progress: &Progress,
    ) -> Result<(CellRun, Vec<MatrixColumn>), Error> {
        self.ready_for_bulk()?;
        matrix.check()?;
        let mut entries: Vec<&MatrixEntry> = matrix.entries.iter().collect();
        entries.sort_by_key(|e| e.col);
        if entries.iter().any(|e| e.col >= self.col_count()) {
            return Err(Error::OutOfRange);
        }
        let compiled = entries
            .iter()
            .map(|e| Compiled::new(&e.cleaner(), matrix.read, now))
            .collect::<Result<Vec<_>, _>>()?;
        let homoglyphs: Vec<bool> = entries.iter().map(|e| fixes_homoglyphs(&e.steps)).collect();
        // A protected column will not change: it shows no example of one.
        let protected: Vec<bool> = entries.iter().map(|e| self.is_protected(e.col)).collect();
        let area = self.area(&FindSpec {
            pattern: Pattern::default(),
            scope: Scope::Columns {
                cols: entries.iter().map(|e| e.col).collect(),
            },
            skip_first: matrix.skip_first,
            skip_cols: Vec::new(),
            ..Default::default()
        })?;
        progress.start(area.rows.end.saturating_sub(area.rows.start));
        let run = self.run_cells(
            &area,
            &Cells {
                compiled: &|k| &compiled[k],
                seed,
                keep,
                examples: if keep { 0 } else { MATRIX_EXAMPLES },
                per_column: true,
                homoglyphs: &homoglyphs,
                no_examples: &protected,
            },
            progress,
        )?;
        let columns = area
            .cols
            .iter()
            .zip(&run.columns)
            .map(|(&col, t)| {
                let protected = self.is_protected(col);
                MatrixColumn {
                    col,
                    changed: if protected { 0 } else { t.changed },
                    protected: if protected { t.changed } else { 0 },
                    left: t.left,
                    other_order: t.other_order,
                    homoglyph_cells: t.homoglyphs,
                }
            })
            .collect();
        Ok((run, columns))
    }

    /// `matrix` with each column found by its title in this table (case and
    /// spaces around it ignored). A title the table does not have, or has
    /// twice and not at the saved place, is an error.
    pub fn matrix_by_title(&self, matrix: &CleanMatrix) -> Result<CleanMatrix, Error> {
        let titles = if matrix.skip_first {
            self.row_values(0).unwrap_or_default()
        } else {
            Vec::new()
        };
        let norm = |t: &str| t.trim().to_lowercase();
        let mut out = matrix.clone();
        for e in &mut out.entries {
            let want = norm(&e.title);
            if want.is_empty() || !matrix.skip_first {
                if e.col >= self.col_count() {
                    return Err(bad(format!("there is no column {}", e.col + 1)));
                }
                continue;
            }
            let found: Vec<u32> = (0..titles.len() as u32)
                .filter(|&c| norm(&titles[c as usize]) == want)
                .collect();
            e.col = match found.as_slice() {
                [one] => *one,
                [] => return Err(bad(format!("no column titled \"{}\"", e.title))),
                many if many.contains(&e.col) => e.col,
                _ => {
                    return Err(bad(format!(
                        "more than one column is titled \"{}\"",
                        e.title
                    )))
                }
            };
        }
        out.check()?;
        Ok(out)
    }

    /// The first changes a run would make, looking at no more than
    /// `max_rows` rows: quick enough to redo on every option change.
    pub fn preview_clean(
        &self,
        spec: &CleanSpec,
        now: f64,
        seed: u64,
        want: usize,
        max_rows: u64,
    ) -> Result<CleanPreview, Error> {
        self.preview_clean_page(spec, now, seed, 0, want, max_rows, false)
    }

    /// A page of the changes a cleaner would make: `want` changes after the first `offset`. With `exact` the
    /// whole area is looked at (up to `max_rows`) so every count is exact, however many changes there are.
    #[allow(clippy::too_many_arguments)]
    pub fn preview_clean_page(
        &self,
        spec: &CleanSpec,
        now: f64,
        seed: u64,
        offset: u64,
        want: usize,
        max_rows: u64,
        exact: bool,
    ) -> Result<CleanPreview, Error> {
        // Look-alike cells are counted only when a Sanitize step is set to fix look-alike letters.
        let sanitizes = fixes_homoglyphs(&spec.cleaner.steps);
        let c = Compiled::new(&spec.cleaner, spec.read, now)?;
        let area = self.area(&FindSpec {
            pattern: Pattern::default(),
            scope: spec.scope.clone(),
            skip_first: spec.skip_first,
            skip_cols: Vec::new(),
            ..Default::default()
        })?;
        let mut out = CleanPreview::default();
        let mut buf = Vec::new();
        let r0 = area.rows.start;
        for r in area.rows.clone() {
            if (!exact && out.changes.len() >= want) || out.rows >= max_rows {
                break;
            }
            out.rows += 1;
            let mut err = None;
            self.each_cell(&area, r, false, &mut buf, |k, t| {
                let col = area.cols[k];
                let ctx = Ctx {
                    index: r - r0,
                    seed: mix(seed ^ mix(r ^ (u64::from(col) << 40))),
                    run: mix(seed ^ (u64::from(col) << 40)),
                };
                match c.run(t, ctx) {
                    Ok(o) => {
                        out.left += u64::from(o.left);
                        out.other_order += u64::from(o.other_order);
                        if let Some(new) = o.text {
                            out.changed += 1;
                            if sanitizes && has_homoglyph(t) {
                                out.homoglyph_cells += 1;
                            }
                            if out.changed > offset && out.changes.len() < want {
                                out.changes.push(Change {
                                    row: r,
                                    number: self.row_number(r).unwrap_or(r),
                                    col,
                                    before: short(t),
                                    after: short(&new),
                                    hidden: false,
                                });
                            }
                        }
                        None
                    }
                    Err(e) => {
                        err = Some(e);
                        Some(())
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
        }
        out.more = area.rows.end > r0 + out.rows;
        Ok(out)
    }

    /// Apply a planned cleaner run as one undo step.
    pub fn apply_clean(&mut self, plan: CleanPlan) -> Result<(), Error> {
        self.apply_cells(plan.cells)
    }
}

#[cfg(test)]
#[path = "clean_matrix_tests.rs"]
mod matrix_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{ColumnFilter, FilterSpec, Rule};
    use crate::find::PatternKind;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn rows(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    fn cleaner(steps: Vec<Step>) -> Cleaner {
        Cleaner {
            version: 1,
            name: String::new(),
            steps,
        }
    }

    fn compiled(steps: Vec<Step>) -> Compiled {
        // 2026-11-12 14:05:09
        let now = 20_769.0 + (14.0 * 3600.0 + 5.0 * 60.0 + 9.0) / 86_400.0;
        Compiled::new(&cleaner(steps), ReadOptions::default(), now).unwrap()
    }

    fn one(step: Step, cell: &str) -> String {
        compiled(vec![step])
            .run(cell, Ctx::default())
            .unwrap()
            .text
            .unwrap_or_else(|| cell.to_string())
    }

    fn left(step: Step, cell: &str) -> bool {
        compiled(vec![step]).run(cell, Ctx::default()).unwrap().left
    }

    /// The ASAP Utilities clean-up check (M12): case options, invisible and
    /// odd characters, dates read in a pattern, Julian cyyddd, numbers in
    /// words.
    /// B1: the Sanitize step, paged preview with exact counts and the homoglyph count (R8), one undo step.
    #[test]
    fn homoglyph_count_is_zero_when_look_alikes_are_not_fixed() {
        let mut text = String::from("name\r\n");
        for i in 0..10 {
            text.push_str(&format!("P\u{430}ul\u{2019}s{i}\r\n"));
        }
        let d = doc(&text);
        let mut o = crate::xray::SanitizeOptions::usual();
        o.homoglyphs = false;
        let spec = |options| CleanSpec {
            cleaner: cleaner(vec![Step::Sanitize { options }]),
            scope: Scope::Columns { cols: vec![0] },
            skip_first: true,
            read: Default::default(),
        };
        // Smart quotes are fixed in all 10 cells, but no look-alike letter is: the count says 0.
        let p = d
            .preview_clean_page(&spec(o), 0.0, 1, 0, 5, 1_000_000, true)
            .unwrap();
        assert_eq!((p.changed, p.homoglyph_cells), (10, 0));
        let on = d
            .preview_clean_page(
                &spec(crate::xray::SanitizeOptions::usual()),
                0.0,
                1,
                0,
                5,
                1_000_000,
                true,
            )
            .unwrap();
        assert_eq!((on.changed, on.homoglyph_cells), (10, 10));
    }

    /// B2: the Fix mojibake step changes only cells that are really double-encoded, lists them in the preview, is
    /// one undo step and a second run changes nothing.
    #[test]
    fn fix_mojibake_changes_only_the_double_encoded_cells() {
        let mut d =
            doc("name\r\ncaf\u{c3}\u{a9}\r\ncaf\u{e9}\r\nO\u{e2}\u{20ac}\u{2122}Neil\r\nplain\r\n");
        let spec = CleanSpec {
            cleaner: cleaner(vec![Step::FixMojibake]),
            scope: Scope::Columns { cols: vec![0] },
            skip_first: true,
            read: Default::default(),
        };
        let p = d.preview_clean(&spec, 0.0, 1, 100, 1_000_000).unwrap();
        assert_eq!(p.changed, 2, "the two double-encoded cells, not the real é");
        assert_eq!(p.changes[0].before, "caf\u{c3}\u{a9}");
        assert_eq!(p.changes[0].after, "caf\u{e9}");
        let plan = d
            .plan_clean(&spec, 0.0, 1, 0, &crate::Progress::new())
            .unwrap();
        d.apply_clean(plan).unwrap();
        assert_eq!(d.cell(1, 0).unwrap(), "caf\u{e9}");
        assert_eq!(d.cell(2, 0).unwrap(), "caf\u{e9}");
        assert_eq!(d.cell(3, 0).unwrap(), "O\u{2019}Neil");
        assert_eq!(d.cell(4, 0).unwrap(), "plain");
        assert_eq!(
            d.preview_clean(&spec, 0.0, 1, 100, 1_000_000)
                .unwrap()
                .changed,
            0
        );
        assert!(d.undo());
        assert_eq!(d.cell(1, 0).unwrap(), "caf\u{c3}\u{a9}");
        // It round trips in a saved cleaner (a recipe step).
        let json = serde_json::to_string(&spec.cleaner).unwrap();
        assert!(json.contains("fix_mojibake"), "{json}");
        let back: Cleaner = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec.cleaner);
    }

    #[test]
    fn sanitize_preview_pages_with_exact_counts() {
        let mut text = String::from("name\r\n");
        for i in 0..50 {
            match i % 3 {
                0 => text.push_str(&format!("P\u{430}ul{i}\r\n")),
                1 => text.push_str(&format!("O\u{2019}Brien{i}\r\n")),
                _ => text.push_str(&format!("plain{i}\r\n")),
            }
        }
        let mut d = doc(&text);
        let spec = CleanSpec {
            cleaner: cleaner(vec![Step::Sanitize {
                options: crate::xray::SanitizeOptions::usual(),
            }]),
            scope: Scope::Columns { cols: vec![0] },
            skip_first: true,
            read: Default::default(),
        };
        // 17 cells have a Cyrillic а, 17 a curly quote, 16 are plain.
        let p = d
            .preview_clean_page(&spec, 0.0, 1, 0, 10, 1_000_000, true)
            .unwrap();
        assert_eq!((p.changed, p.homoglyph_cells), (34, 17));
        assert_eq!(p.changes.len(), 10);
        let second = d
            .preview_clean_page(&spec, 0.0, 1, 10, 10, 1_000_000, true)
            .unwrap();
        assert_eq!(second.changed, 34, "the counts are exact on every page");
        assert_ne!(p.changes[0].row, second.changes[0].row);
        assert!(second.changes[0].row > p.changes.last().unwrap().row);
        // Apply: one undo step; a second run changes nothing.
        let plan = d
            .plan_clean(&spec, 0.0, 1, 0, &crate::Progress::new())
            .unwrap();
        d.apply_clean(plan).unwrap();
        assert_eq!(d.cell(1, 0).unwrap(), "Paul0");
        assert_eq!(d.cell(2, 0).unwrap(), "O'Brien1");
        let again = d
            .preview_clean_page(&spec, 0.0, 1, 0, 10, 1_000_000, true)
            .unwrap();
        assert_eq!(again.changed, 0);
        assert!(d.undo());
        assert_eq!(d.cell(1, 0).unwrap(), "P\u{430}ul0");
    }

    #[test]
    fn m12_clean_up_steps() {
        let case = |to| Step::Case { to };
        assert_eq!(one(case(CaseTo::First), "  hello WORLD"), "  Hello WORLD");
        assert_eq!(one(case(CaseTo::First), "123"), "123");
        assert_eq!(
            one(
                case(CaseTo::TitleSmall),
                "the lord of the rings: the return of the king"
            ),
            "The Lord of the Rings: The Return of the King"
        );
        assert_eq!(
            one(case(CaseTo::TitleSmall), "what it is for"),
            "What It Is For"
        );
        assert_eq!(
            one(case(CaseTo::Names), "MARY o'brien-mcdonald iii"),
            "Mary O'Brien-McDonald III"
        );
        assert_eq!(
            one(case(CaseTo::Names), "d'angelo, don't"),
            "D'Angelo, Don't"
        );
        assert_eq!(one(case(CaseTo::Names), "mcd mc"), "McD Mc");
        // Review M3: Xi, Vi and Ix are names; numerals where numerals go.
        let names = |t| one(case(CaseTo::Names), t);
        assert_eq!(names("XI JINPING"), "Xi Jinping");
        assert_eq!(names("jinping xi"), "Jinping Xi");
        assert_eq!(names("vi nguyen"), "Vi Nguyen");
        assert_eq!(names("NGUYEN VI"), "Nguyen Vi");
        assert_eq!(names("ix smith"), "Ix Smith");
        assert_eq!(names("henry viii"), "Henry VIII");
        assert_eq!(names("HENRY IV"), "Henry IV");
        assert_eq!(names("LOUIS XIV"), "Louis XIV");
        assert_eq!(names("pope john xx"), "Pope John XX");
        assert_eq!(names("john smith iii"), "John Smith III");
        assert_eq!(names("john paul xi"), "John Paul XI");
        assert_eq!(names("Pope Pius XI"), "Pope Pius XI");
        assert_eq!(names("Pope Pius Xi"), "Pope Pius XI");
        assert_eq!(names("Ann Xi"), "Ann Xi");
        let rm = |of| Step::Remove {
            what: Remove::Kind { of },
        };
        assert_eq!(
            one(rm(Kind::Invisible), "\u{feff}a\u{200b}b\u{00ad}c\u{2060}"),
            "abc"
        );
        assert_eq!(one(rm(Kind::Invisible), "e\u{301} 😀"), "e\u{301} 😀");
        assert_eq!(
            one(
                Step::Spaces { how: Spaces::Plain },
                "1\u{a0}234\u{2009}5\u{3000}x\ty"
            ),
            "1 234 5 x\ty"
        );
        // Dates read in a pattern.
        let from = |from: &str, format: &str| Step::Dates {
            ambiguous: None,
            review: None,
            format: format.into(),
            from: from.into(),
            past: false,
        };
        let iso = |f: &str| from(f, "yyyy-mm-dd");
        assert_eq!(one(iso("yyyymmdd"), "20261113"), "2026-11-13");
        assert_eq!(one(iso("mmddyyyy"), "11132026"), "2026-11-13");
        assert_eq!(one(iso("ddmmyy"), "131126"), "2026-11-13");
        assert_eq!(one(iso("dd/mm/yyyy"), "13.11.2026"), "2026-11-13");
        assert_eq!(one(iso("d/m/yyyy"), "3/1/2026"), "2026-01-03");
        assert_eq!(one(iso("mmm d, yyyy"), "Nov 13, 2026"), "2026-11-13");
        assert_eq!(one(iso("d mmmm yyyy"), "13 november 2026"), "2026-11-13");
        assert_eq!(one(iso("yyyy.mm"), "2026.11"), "2026-11-01");
        assert_eq!(
            one(
                from("mm/dd/yyyy h:mm AM/PM", "yyyy-mm-dd hh:mm"),
                "11/13/2026 2:05 PM"
            ),
            "2026-11-13 14:05"
        );
        assert_eq!(one(from("yyyymmdd", "dd.mm.yyyy"), "20260229"), "20260229");
        assert!(left(iso("yyyymmdd"), "20261332"));
        assert!(left(iso("yyyymmdd"), "2026111"));
        assert!(left(iso("dd/mm/yyyy"), "13/11/2026 extra"));
        // Review L2: two-digit years in the past (now is 2026-11-12).
        let past = |from: &str| Step::Dates {
            ambiguous: None,
            review: None,
            format: "yyyy-mm-dd".into(),
            from: from.into(),
            past: true,
        };
        assert_eq!(one(past("dd.mm.yy"), "13.11.26"), "1926-11-13");
        assert_eq!(one(past("dd.mm.yy"), "12.11.26"), "2026-11-12");
        assert_eq!(one(past("dd.mm.yy"), "01.01.05"), "2005-01-01");
        assert_eq!(one(past("dd.mm.yyyy"), "13.11.2030"), "2030-11-13");
        assert_eq!(one(past(""), "11/13/26"), "1926-11-13");
        assert_eq!(one(past(""), "11/13/2026"), "2026-11-13");
        assert_eq!(one(iso("dd.mm.yy"), "13.11.26"), "2026-11-13");
        // Review L3: a date that can only be read the other way is flagged.
        let guess = compiled(vec![Step::Dates {
            ambiguous: None,
            review: None,
            format: "yyyy-mm-dd".into(),
            from: String::new(),
            past: false,
        }]);
        let o = guess.run("13/04/2026", Ctx::default()).unwrap();
        assert_eq!(
            (o.text.as_deref(), o.other_order),
            (Some("2026-04-13"), true)
        );
        let o = guess.run("03/04/2026", Ctx::default()).unwrap();
        assert_eq!(
            (o.text.as_deref(), o.other_order),
            (Some("2026-03-04"), false)
        );
        assert!(!guess.run("2026-04-13", Ctx::default()).unwrap().other_order);
        // Julian cyyddd.
        let jul = Step::FixNumber {
            how: NumberFix::Julian,
        };
        assert_eq!(one(jul.clone(), "126317"), "2026-11-13");
        assert_eq!(one(jul.clone(), "099001"), "1999-01-01");
        // Review L4: century digit 0 to 2 only.
        assert_eq!(one(jul, "926001"), "926001");
        // Numbers in words.
        let w = |money| Step::Words { money };
        assert_eq!(one(w(false), "0"), "zero");
        assert_eq!(one(w(false), "123"), "one hundred twenty-three");
        assert_eq!(one(w(false), "1,000,017"), "one million seventeen");
        assert_eq!(one(w(false), "-12.05"), "minus twelve point zero five");
        assert_eq!(
            one(w(true), "$1,234.50"),
            "one thousand two hundred thirty-four dollars and fifty cents"
        );
        assert_eq!(one(w(true), "1.01"), "one dollar and one cent");
        // Review L1: rounded on the written digits.
        assert_eq!(one(w(true), "1.005"), "one dollar and one cent");
        assert_eq!(one(w(true), "2.675"), "two dollars and sixty-eight cents");
        assert_eq!(one(w(true), "0.995"), "one dollar");
        assert_eq!(
            one(w(false), "999999999999999"),
            "nine hundred ninety-nine trillion nine hundred ninety-nine billion nine hundred ninety-nine million nine hundred ninety-nine thousand nine hundred ninety-nine"
        );
        assert!(left(w(false), "abc"));
        assert!(left(w(false), "1e16"));
    }

    /// Unique random numbers: every cell of a column gets its own; past
    /// the range the rest are left.
    #[test]
    fn unique_random_numbers() {
        for (min, max, cells) in [
            (1i64, 10, 10u64),
            (5, 5, 1),
            (-3, 1000, 600),
            (0, 1 << 40, 2000),
        ] {
            let c = compiled(vec![Step::RandomNumber {
                min,
                max,
                unique: true,
            }]);
            let mut seen = std::collections::HashSet::new();
            for index in 0..cells {
                let v: i64 = c
                    .run(
                        "x",
                        Ctx {
                            index,
                            seed: index,
                            run: 99,
                        },
                    )
                    .unwrap()
                    .text
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!((min..=max).contains(&v));
                assert!(seen.insert(v), "{v} twice");
            }
        }
        let c = compiled(vec![Step::RandomNumber {
            min: 1,
            max: 3,
            unique: true,
        }]);
        assert!(
            c.run(
                "x",
                Ctx {
                    index: 3,
                    seed: 3,
                    run: 1
                }
            )
            .unwrap()
            .left
        );
        // Another run gives another order.
        let order = |run| {
            (0..20)
                .map(|index| {
                    c_text(
                        &compiled(vec![Step::RandomNumber {
                            min: 1,
                            max: 1000,
                            unique: true,
                        }]),
                        index,
                        run,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_ne!(order(1), order(2));
        let r = compiled(vec![Step::RandomNumber {
            min: -2,
            max: 2,
            unique: false,
        }]);
        for seed in 0..50 {
            let v: i64 = r
                .run(
                    "",
                    Ctx {
                        index: 0,
                        seed,
                        run: 0,
                    },
                )
                .unwrap()
                .text
                .unwrap()
                .parse()
                .unwrap();
            assert!((-2..=2).contains(&v));
        }
        assert!(Cleaner {
            version: 1,
            name: String::new(),
            steps: vec![Step::RandomNumber {
                min: 2,
                max: 1,
                unique: false
            }],
        }
        .check()
        .is_err());
    }

    fn c_text(c: &Compiled, index: u64, run: u64) -> String {
        c.run(
            "x",
            Ctx {
                index,
                seed: index,
                run,
            },
        )
        .unwrap()
        .text
        .unwrap()
    }

    #[test]
    fn m10_steps() {
        // Accents.
        assert_eq!(one(Step::Accents, "Crème Brûlée"), "Creme Brulee");
        assert_eq!(one(Step::Accents, "ÆRØ Straße"), "AERO Strasse");
        assert_eq!(one(Step::Accents, "Zoë Łódź"), "Zoe Lodz");
        assert_eq!(one(Step::Accents, "e\u{301}cole"), "ecole");
        assert_eq!(one(Step::Accents, "日本 ok"), "日本 ok");
        // Fix numbers.
        let fix = |how, c| one(Step::FixNumber { how }, c);
        assert_eq!(fix(NumberFix::TrailingMinus, "50-"), "-50");
        assert_eq!(fix(NumberFix::TrailingMinus, "1,234.50 -"), "-1,234.50");
        assert_eq!(fix(NumberFix::TrailingMinus, "(7.5)"), "-7.5");
        assert_eq!(fix(NumberFix::TrailingMinus, "12"), "12");
        assert!(!left(
            Step::FixNumber {
                how: NumberFix::TrailingMinus
            },
            "12"
        ));
        assert!(left(
            Step::FixNumber {
                how: NumberFix::TrailingMinus
            },
            "a-"
        ));
        assert_eq!(fix(NumberFix::FlipSign, "-5"), "5");
        assert_eq!(fix(NumberFix::FlipSign, "5.25"), "-5.25");
        assert_eq!(fix(NumberFix::FlipSign, "0"), "0");
        assert_eq!(fix(NumberFix::Positive, "-3"), "3");
        assert_eq!(fix(NumberFix::Negative, "+3"), "-3");
        assert_eq!(fix(NumberFix::Negative, "-3"), "-3");
        assert_eq!(fix(NumberFix::Percent, "12.5%"), "0.125");
        assert_eq!(fix(NumberFix::Percent, "100 %"), "1");
        assert!(left(
            Step::FixNumber {
                how: NumberFix::Percent
            },
            "12.5"
        ));
        assert_eq!(fix(NumberFix::Julian, "24060"), "2024-02-29");
        assert_eq!(fix(NumberFix::Julian, "2023365"), "2023-12-31");
        assert_eq!(fix(NumberFix::Julian, "99001"), "1999-01-01");
        assert!(left(
            Step::FixNumber {
                how: NumberFix::Julian
            },
            "2023366"
        ));
        // Calculate.
        let c = |op, value, cell| one(Step::Calc { op, value }, cell);
        assert_eq!(c(CalcOp::Add, 0.2, "0.1"), "0.3");
        assert_eq!(c(CalcOp::Multiply, 1.1, "1,000"), "1100");
        assert_eq!(c(CalcOp::Divide, 3.0, "10"), "3.33333333333333");
        // Every real digit kept (review M3), float noise dropped.
        assert_eq!(c(CalcOp::Add, 0.0, "1234567890.12345"), "1234567890.12345");
        assert_eq!(
            c(CalcOp::Multiply, 1.0, "3.14159265358979"),
            "3.14159265358979"
        );
        assert_eq!(c(CalcOp::Divide, 3.0, "1"), "0.333333333333333");
        assert_eq!(
            fix(NumberFix::Percent, "123456789.123456%"),
            "1234567.89123456"
        );
        // Rounding never pads: 8.2 stays 8.2 (no change).
        assert_eq!(c(CalcOp::Round, 2.0, "8.256"), "8.26");
        assert!(
            !left(
                Step::Calc {
                    op: CalcOp::Round,
                    value: 2.0
                },
                "8.2"
            ) && one(
                Step::Calc {
                    op: CalcOp::Round,
                    value: 2.0
                },
                "8.2"
            ) == "8.2"
        );
        // Brackets are a negative number (review L2).
        assert_eq!(fix(NumberFix::FlipSign, "(5)"), "5");
        assert_eq!(fix(NumberFix::Positive, "(5)"), "5");
        assert_eq!(fix(NumberFix::Negative, "(5)"), "-5");
        // Review L7.
        assert_eq!(one(Step::Accents, "Þór"), "Thor");
        assert_eq!(one(Step::Accents, "ÞÓR"), "THOR");
        assert_eq!(one(Step::Accents, "İstanbul"), "Istanbul");
        assert_eq!(c(CalcOp::Subtract, 5.0, "2"), "-3");
        assert_eq!(c(CalcOp::Power, 2.0, "12"), "144");
        assert_eq!(c(CalcOp::Abs, 0.0, "-4.5"), "4.5");
        assert_eq!(c(CalcOp::Round, 2.0, "2.675"), "2.68");
        assert_eq!(c(CalcOp::Round, 0.0, "-2.5"), "-3");
        assert_eq!(c(CalcOp::RoundUp, 1.0, "2.11"), "2.2");
        assert_eq!(c(CalcOp::RoundUp, 1.0, "-2.11"), "-2.2");
        assert_eq!(c(CalcOp::RoundUp, 1.0, "2.1"), "2.1");
        assert_eq!(c(CalcOp::RoundDown, 0.0, "-2.9"), "-2");
        assert_eq!(c(CalcOp::RoundDown, 2.0, "2.119"), "2.11");
        assert!(left(
            Step::Calc {
                op: CalcOp::Add,
                value: 1.0
            },
            "abc"
        ));
        assert_eq!(c(CalcOp::Add, 1.0, ""), "");
        assert!(Step::Calc {
            op: CalcOp::Divide,
            value: 0.0
        }
        .check()
        .is_err());
        assert!(Step::Calc {
            op: CalcOp::Round,
            value: 1.5
        }
        .check()
        .is_err());
        assert!(Step::Calc {
            op: CalcOp::Add,
            value: f64::NAN
        }
        .check()
        .is_err());
        assert_eq!(c(CalcOp::Power, 1000.0, "10"), "10");
        // Sort parts.
        let sp = |d, u, t, cell| {
            one(
                Step::SortParts {
                    separator: ";".into(),
                    descending: d,
                    distinct: u,
                    trim: t,
                },
                cell,
            )
        };
        assert_eq!(sp(false, false, false, "b;a10;a2"), "a2;a10;b");
        assert_eq!(sp(true, false, false, "b;a;c"), "c;b;a");
        assert_eq!(sp(false, true, true, " b ; a;;b"), "a;b");
        assert_eq!(sp(false, false, false, "single"), "single");
        assert!(Step::SortParts {
            separator: String::new(),
            descending: false,
            distinct: false,
            trim: false
        }
        .check()
        .is_err());
        // URL codes.
        assert_eq!(one(Step::UrlDecode, "a%20b%2Fc%C3%A9"), "a b/cé");
        assert!(left(Step::UrlDecode, "100%"));
        assert!(left(Step::UrlDecode, "%FF"));
        assert_eq!(one(Step::UrlEncode, "a b/é~"), "a%20b%2F%C3%A9~");
        assert_eq!(
            one(Step::UrlDecode, &one(Step::UrlEncode, "x?y=1&z=ü")),
            "x?y=1&z=ü"
        );
        // Saved and read back.
        let all = cleaner(vec![
            Step::Accents,
            Step::FixNumber {
                how: NumberFix::Julian,
            },
            Step::Calc {
                op: CalcOp::RoundUp,
                value: 2.0,
            },
            Step::SortParts {
                separator: ", ".into(),
                descending: false,
                distinct: true,
                trim: true,
            },
            Step::UrlDecode,
            Step::UrlEncode,
            Step::Words { money: true },
            Step::RandomNumber {
                min: -5,
                max: 5,
                unique: true,
            },
            Step::Case { to: CaseTo::Names },
            Step::Spaces { how: Spaces::Plain },
            Step::Remove {
                what: Remove::Kind {
                    of: Kind::Invisible,
                },
            },
            Step::Dates {
                ambiguous: None,
                review: None,
                format: "yyyy-mm-dd".into(),
                from: "yyyymmdd".into(),
                past: false,
            },
        ]);
        assert_eq!(Cleaner::from_json(&all.to_json().unwrap()).unwrap(), all);
        // Saved cleaners from before M12 (no "from") still load.
        let old = Cleaner::from_json(
            br#"{"version":1,"name":"","steps":[{"tool":"dates","format":"yyyy"}]}"#,
        )
        .unwrap();
        assert_eq!(
            old.steps[0],
            Step::Dates {
                ambiguous: None,
                review: None,
                format: "yyyy".into(),
                from: String::new(),
                past: false
            }
        );
    }

    fn pat(t: &str, kind: PatternKind) -> Pattern {
        Pattern {
            text: t.into(),
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn replace_dates_numbers() {
        let r = |find: &str, kind, with: &str| Step::Replace {
            find: pat(find, kind),
            with: with.into(),
        };
        assert_eq!(one(r("o", PatternKind::Plain, "0"), "foo"), "f00");
        assert_eq!(
            one(r(r"(\d+)-(\d+)", PatternKind::Regex, "$2/$1"), "12-34"),
            "34/12"
        );
        let dates = |f: &str| Step::Dates {
            ambiguous: None,
            review: None,
            format: f.into(),
            from: String::new(),
            past: false,
        };
        assert_eq!(one(dates("yyyy-mm-dd"), "3/4/2026"), "2026-03-04");
        assert_eq!(one(dates("d mmmm yyyy"), "2026-03-04"), "4 March 2026");
        assert_eq!(
            one(dates("ddd dd/mm/yy h:mm AM/PM"), "2026-03-04 15:07"),
            "Wed 04/03/26 3:07 PM"
        );
        assert_eq!(one(dates("hh:mm:ss"), "2026-03-04 15:07:09"), "15:07:09");
        assert_eq!(one(dates("\"on\" d"), "2026-03-04"), "on 4");
        assert_eq!(one(dates("yyyy"), "not a date"), "not a date");
        assert_eq!(one(dates("yyyy"), "10:30"), "10:30", "a time alone is left");
        let num = |d, t, c| Step::Numbers {
            decimals: d,
            thousands: t,
            decimal_comma: c,
        };
        assert_eq!(
            one(num(Some(2), true, false), "1234567.891"),
            "1,234,567.89"
        );
        assert_eq!(one(num(None, true, true), "-1234.5"), "-1.234,5");
        assert_eq!(one(num(Some(0), false, false), "$1,200.4"), "1200");
        assert_eq!(one(num(Some(2), false, false), "50%"), "50%");
        assert_eq!(one(num(Some(2), false, false), "-0.001"), "0.00");
        assert_eq!(
            one(num(Some(2), false, false), "12345678901234567"),
            "12345678901234567"
        );
    }

    #[test]
    fn add_pad_remove_spaces() {
        let add = |text: &str, at| Step::Add {
            text: text.into(),
            at,
        };
        assert_eq!(one(add(">", At::Start), "ab"), ">ab");
        assert_eq!(one(add("<", At::End), "ab"), "ab<");
        assert_eq!(one(add("-", At::Position { n: 1 }), "ébc"), "é-bc");
        assert_eq!(one(add("-", At::Position { n: 9 }), "ab"), "ab-");
        assert_eq!(
            one(add("(", At::Before { text: "b".into() }), "abc"),
            "a(bc"
        );
        assert_eq!(one(add(")", At::After { text: "b".into() }), "abc"), "ab)c");
        assert_eq!(one(add(")", At::After { text: "z".into() }), "abc"), "abc");
        let rnd = one(
            Step::Random {
                length: 12,
                chars: Charset::Hex,
            },
            "x",
        );
        assert_eq!(rnd.len(), 12);
        assert!(rnd.chars().all(|c| c.is_ascii_hexdigit()));
        let pad = |w, c, right| Step::Pad {
            width: w,
            with: c,
            right,
        };
        assert_eq!(one(pad(5, '0', false), "42"), "00042");
        assert_eq!(one(pad(4, '·', true), "ab"), "ab··");
        assert_eq!(one(pad(1, '0', false), "42"), "42");
        let rm = |what| Step::Remove { what };
        assert_eq!(one(rm(Remove::First { n: 2 }), "abcd"), "cd");
        assert_eq!(one(rm(Remove::Last { n: 9 }), "abcd"), "");
        assert_eq!(one(rm(Remove::Range { from: 1, count: 2 }), "abcd"), "ad");
        let before = Remove::Before {
            text: "@".into(),
            inclusive: true,
        };
        assert_eq!(one(rm(before), "me@x.org"), "x.org");
        let after = Remove::After {
            text: "@".into(),
            inclusive: false,
        };
        assert_eq!(one(rm(after), "me@x.org"), "me@");
        let m = Remove::Matching {
            find: pat(r"\s*\(.*?\)", PatternKind::Regex),
        };
        assert_eq!(one(rm(m), "a (b) c"), "a c");
        assert_eq!(
            one(
                rm(Remove::Kind {
                    of: Kind::Punctuation
                }),
                "a-b.c!"
            ),
            "abc"
        );
        assert_eq!(one(rm(Remove::Kind { of: Kind::NonAscii }), "café"), "caf");
        let chars = Remove::Characters {
            chars: "()-".into(),
        };
        assert_eq!(one(rm(chars), "(555) 123-4"), "555 1234");
        assert_eq!(one(Step::Reverse, "abc"), "cba");
        let sp = |how| Step::Spaces { how };
        assert_eq!(one(sp(Spaces::Trim), "  a b  "), "a b");
        assert_eq!(one(sp(Spaces::Collapse), " a \t\n b "), "a b");
        assert_eq!(one(sp(Spaces::RemoveAll), " a b "), "ab");
        assert_eq!(one(sp(Spaces::LineBreaks), "a\r\nb\nc"), "a b c");
        assert_eq!(one(sp(Spaces::LettersDigits), "AB12cd"), "AB 12 cd");
        assert_eq!(
            one(sp(Spaces::BeforeCapitals), "firstNameHTML"),
            "first Name HTML"
        );
    }

    #[test]
    fn numbers_dates_case_html() {
        let numbered = Step::Number {
            start: 1.0,
            step: 1.0,
            digits: 3,
            at_end: false,
            separator: ". ".into(),
            replace: false,
        };
        assert_eq!(one(numbered, "x"), "001. x");
        assert_eq!(
            one(Step::Digits { keep: true }, "(555) 123-4567"),
            "5551234567"
        );
        assert_eq!(one(Step::Digits { keep: false }, "ab12c3"), "abc");
        let now = Step::Now {
            format: "yyyy-mm-dd hh:mm".into(),
            at_end: true,
            separator: " @ ".into(),
            replace: false,
        };
        assert_eq!(one(now, "seen"), "seen @ 2026-11-12 14:05");
        let case = |to| Step::Case { to };
        assert_eq!(one(case(CaseTo::Upper), "straße"), "STRASSE");
        assert_eq!(
            one(case(CaseTo::Title), "o'neil smith-jones don't"),
            "O'neil Smith-Jones Don't"
        );
        assert_eq!(
            one(case(CaseTo::Sentence), "HELLO THERE. how ARE you?"),
            "Hello there. How are you?"
        );
        assert_eq!(one(case(CaseTo::Toggle), "aBc"), "AbC");
        assert_eq!(
            one(Step::HtmlEncode, "a<b & \"c\""),
            "a&lt;b &amp; &quot;c&quot;"
        );
        assert_eq!(one(Step::HtmlDecode, "a&lt;b &amp; &#x41;"), "a<b & A");
        assert_eq!(
            one(
                Step::HtmlText,
                "<p>Hi <b>there</b></p><script>x<1</script><STYLE>p{}</STYLE>a &amp; b<br>c<!-- no --> 1 < 2"
            ),
            "Hi there\na & b\nc 1 < 2"
        );
    }

    #[test]
    fn names() {
        let n = split_name("Dr. Mary Ann van der Berg Jr.");
        assert_eq!(
            n,
            Name {
                title: "Dr.".into(),
                first: "Mary".into(),
                middle: "Ann".into(),
                last: "van der Berg".into(),
                suffix: "Jr.".into(),
            }
        );
        let n = split_name("Smith, John Q.");
        assert_eq!(
            (n.first.as_str(), n.middle.as_str(), n.last.as_str()),
            ("John", "Q.", "Smith")
        );
        let n = split_name("John Smith, Jr.");
        assert_eq!(
            (n.first.as_str(), n.last.as_str(), n.suffix.as_str()),
            ("John", "Smith", "Jr.")
        );
        assert_eq!(split_name("Cher").first, "Cher");
        assert_eq!(split_name("Mr").first, "Mr", "a title alone stays a name");
        let part = |p, s| one(Step::Name { part: p }, s);
        assert_eq!(
            part(NamePart::LastFirst, "Mary Ann Smith"),
            "Smith, Mary Ann"
        );
        assert_eq!(
            part(NamePart::FirstLast, "Smith, Mary Ann"),
            "Mary Ann Smith"
        );
        assert_eq!(part(NamePart::Last, "Ms Jo"), "");
    }

    fn hide_c3(d: &mut Document) {
        d.set_filter(&FilterSpec {
            columns: vec![ColumnFilter {
                col: 1,
                rule: Rule::Values {
                    values: vec!["a1".into(), "b22".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
    }

    #[test]
    fn steps_run_in_order_on_a_document() {
        let text = "name,code\r\n  ann  lee ,a1\r\nBOB  RAY,b22\r\nhidden one,c3\r\n";
        let before = rows(&doc(text));
        let mut d = doc(text);
        hide_c3(&mut d);
        let spec = CleanSpec {
            cleaner: cleaner(vec![
                Step::Spaces {
                    how: Spaces::Collapse,
                },
                Step::Case { to: CaseTo::Title },
                Step::Name {
                    part: NamePart::LastFirst,
                },
            ]),
            scope: Scope::Columns { cols: vec![0] },
            skip_first: true,
            read: ReadOptions::default(),
        };
        let plan = d.plan_clean(&spec, 0.0, 1, 10, &Progress::new()).unwrap();
        assert_eq!(plan.changed, 2);
        assert_eq!(plan.preview[0].after, "Lee, Ann");
        d.apply_clean(plan).unwrap();
        assert_eq!(
            rows(&d)[1..],
            [vec!["Lee, Ann", "a1"], vec!["Ray, Bob", "b22"]]
        );
        d.set_filter(&FilterSpec::default()).unwrap();
        assert_eq!(
            rows(&d)[3],
            ["hidden one", "c3"],
            "hidden rows are not cleaned"
        );
        assert!(d.undo(), "the filter is an undo step of its own");
        assert!(d.undo());
        assert!(d.undo(), "and so is the first one");
        assert_eq!(rows(&d), before);
        // Numbering counts the cleaned rows; empty cells get it too.
        let mut d = doc("a,b\r\nx,\r\ny,\r\n");
        let spec = CleanSpec {
            cleaner: cleaner(vec![Step::Number {
                start: 10.0,
                step: 5.0,
                digits: 0,
                at_end: false,
                separator: String::new(),
                replace: true,
            }]),
            scope: Scope::Columns { cols: vec![1] },
            skip_first: true,
            read: ReadOptions::default(),
        };
        let plan = d.plan_clean(&spec, 0.0, 1, 0, &Progress::new()).unwrap();
        d.apply_clean(plan).unwrap();
        assert_eq!(rows(&d)[1..], [vec!["x", "10"], vec!["y", "15"]]);
        // Dates that cannot be read are counted; blanks are not.
        let d = doc("d\r\n2026-01-02\r\nsoon\r\n\"\"\r\n");
        let spec = CleanSpec {
            cleaner: cleaner(vec![Step::Dates {
                ambiguous: None,
                review: None,
                format: "dd.mm.yyyy".into(),
                from: String::new(),
                past: false,
            }]),
            scope: Scope::All,
            skip_first: true,
            read: ReadOptions::default(),
        };
        let plan = d.plan_clean(&spec, 0.0, 1, 5, &Progress::new()).unwrap();
        assert_eq!((plan.changed, plan.left), (1, 1));
    }

    #[test]
    fn cleaner_files_round_trip() {
        let c = Cleaner {
            version: 1,
            name: "Phone numbers".into(),
            steps: vec![
                Step::Digits { keep: true },
                Step::Replace {
                    find: pat(r"^(\d{3})(\d{3})(\d{4})$", PatternKind::Regex),
                    with: "($1) $2-$3".into(),
                },
            ],
        };
        let bytes = c.to_json().unwrap();
        assert_eq!(Cleaner::from_json(&bytes).unwrap(), c);
        let run = Compiled::new(&c, ReadOptions::default(), 0.0).unwrap();
        let out = run.run("555.123.4567", Ctx::default()).unwrap();
        assert_eq!(out.text.unwrap(), "(555) 123-4567");
        // A short hand-written file, defaults filled in.
        let c = Cleaner::from_json(
            br#"{"steps":[{"tool":"pad","width":3,"right":true},{"tool":"reverse"}]}"#,
        )
        .unwrap();
        assert_eq!(c.version, CLEANER_VERSION);
        let run = Compiled::new(&c, ReadOptions::default(), 0.0).unwrap();
        assert_eq!(run.run("ab", Ctx::default()).unwrap().text.unwrap(), " ba");
    }

    /// Cleaner files are untrusted: every bound is checked when loaded.
    #[test]
    fn hostile_cleaner_files_are_refused() {
        let refused = |json: &str, word: &str| {
            let e = Cleaner::from_json(json.as_bytes())
                .and_then(|c| Compiled::new(&c, ReadOptions::default(), 0.0).map(|_| ()))
                .unwrap_err();
            assert!(e.to_string().contains(word), "{e} (wanted {word})");
        };
        let big = format!(
            "{{\"steps\":[],\"name\":\"{}\"}}",
            "x".repeat(MAX_CLEANER_BYTES)
        );
        refused(&big, "at most 1 MB");
        refused(
            "{\"steps\":[{\"tool\":\"format_disk\"}]}",
            "unknown variant",
        );
        refused("{\"version\":99,\"steps\":[]}", "newer version");
        refused("not json", "not a cleaner file");
        refused(
            "{\"steps\":[{\"tool\":\"pad\",\"width\":4294967295}]}",
            "pad to at most",
        );
        refused(
            "{\"steps\":[{\"tool\":\"random\",\"length\":100000,\"chars\":{\"kind\":\"hex\"}}]}",
            "random text",
        );
        refused(
            "{\"steps\":[{\"tool\":\"random\",\"length\":3,\"chars\":{\"kind\":\"custom\",\"chars\":\"\"}}]}",
            "between 1",
        );
        refused(
            "{\"steps\":[{\"tool\":\"numbers\",\"decimals\":200}]}",
            "15 decimals",
        );
        refused(
            "{\"steps\":[{\"tool\":\"number\",\"start\":1e300}]}",
            "10^15",
        );
        refused(
            "{\"steps\":[{\"tool\":\"number\",\"digits\":255}]}",
            "30 digits",
        );
        refused(
            "{\"steps\":[{\"tool\":\"dates\",\"format\":\"-\"}]}",
            "date format needs",
        );
        let name = format!(
            "{{\"name\":\"{}\",\"steps\":[]}}",
            "n".repeat(MAX_NAME_CHARS + 1)
        );
        refused(&name, "name can be at most");
        let many = format!(
            "{{\"steps\":[{}{{\"tool\":\"reverse\"}}]}}",
            "{\"tool\":\"reverse\"},".repeat(MAX_STEPS)
        );
        refused(&many, "at most 100 steps");
        let long = format!(
            "{{\"steps\":[{{\"tool\":\"replace\",\"find\":{{\"text\":\"{}\"}}}}]}}",
            "a".repeat(MAX_PATTERN_CHARS + 1)
        );
        refused(&long, "pattern can be at most");
        let long = format!(
            "{{\"steps\":[{{\"tool\":\"add\",\"text\":\"{}\",\"at\":{{\"kind\":\"start\"}}}}]}}",
            "a".repeat(MAX_REPLACEMENT_CHARS + 1)
        );
        refused(&long, "text to add");
        // A pattern whose compiled form is too big is refused, not built.
        refused(
            r#"{"steps":[{"tool":"replace","find":{"text":"(\\w{100}){100}{100}","kind":"regex"}}]}"#,
            "",
        );
        // Deeply nested JSON is refused by the parser's depth limit.
        let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
        refused(&format!("{{\"steps\":{deep}}}"), "not a cleaner file");
    }

    /// Patterns that backtrack badly elsewhere run in linear time here, and
    /// results are measured before they are built.
    #[test]
    fn hostile_cells_stay_bounded() {
        let c = compiled(vec![Step::Replace {
            find: pat("(a+)+$", PatternKind::Regex),
            with: "x".into(),
        }]);
        let cell = format!("{}b", "a".repeat(100_000));
        let t = std::time::Instant::now();
        assert!(c.run(&cell, Ctx::default()).unwrap().text.is_none());
        assert!(t.elapsed().as_secs() < 2);
        // Growing a cell past 32 MB is refused.
        let big = "x".repeat(MAX_RESULT_CELL_BYTES - 10);
        let grow = |step: Step| compiled(vec![step]).run(&big, Ctx::default()).unwrap_err();
        grow(Step::Add {
            text: "y".repeat(100),
            at: At::Start,
        });
        grow(Step::Replace {
            find: pat("x", PatternKind::Plain),
            with: "yy".into(),
        });
        grow(Step::Number {
            start: 1.0,
            step: 1.0,
            digits: 30,
            at_end: true,
            separator: String::new(),
            replace: false,
        });
        // Padding never multiplies: each step pads to the same width.
        let c = compiled(
            (0..MAX_STEPS)
                .map(|_| Step::Pad {
                    width: MAX_WIDTH,
                    with: '😀',
                    right: true,
                })
                .collect(),
        );
        let out = c
            .run(&"é".repeat(10), Ctx::default())
            .unwrap()
            .text
            .unwrap();
        assert_eq!(out.chars().count(), MAX_WIDTH as usize);
    }
}

// ---- split names into columns ----

/// Split a name column into new columns to its right.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitName {
    pub col: u32,
    /// The parts wanted, in order (at most the five parts, each once).
    pub parts: Vec<NamePart>,
    /// Keep the name column (otherwise the first part replaces it).
    #[serde(default)]
    pub keep_original: bool,
    /// The first row holds titles: the new columns get part names.
    #[serde(default)]
    pub keep_first: bool,
}

fn part_title(p: NamePart) -> &'static str {
    match p {
        NamePart::Title => "Title",
        NamePart::First => "First name",
        NamePart::Middle => "Middle name",
        NamePart::Last => "Last name",
        NamePart::Suffix => "Suffix",
        NamePart::FirstLast => "Name",
        NamePart::LastFirst => "Name (last first)",
    }
}

impl Document {
    /// Plan splitting a name column into its parts. Every row is split,
    /// hidden ones too (the new columns exist for all rows).
    pub fn plan_split_name(
        &self,
        spec: &SplitName,
        progress: &Progress,
    ) -> Result<crate::ops::ColPlan, Error> {
        self.ready_for_bulk()?;
        crate::ops::check_cols(self, &[spec.col])?;
        let mut parts = spec.parts.clone();
        parts.dedup();
        if parts.is_empty() || parts.len() > 5 {
            return Err(Error::Invalid(
                "choose between one and five name parts".into(),
            ));
        }
        let new = if spec.keep_original {
            parts.len() as u32
        } else {
            parts.len() as u32 - 1
        };
        let at = spec.col + 1;
        let slot = |k: usize| -> u32 {
            if spec.keep_original {
                at + k as u32
            } else if k == 0 {
                spec.col
            } else {
                at + k as u32 - 1
            }
        };
        let first = u64::from(spec.keep_first && self.total_rows() > 0);
        let col = spec.col;
        let spent = std::sync::atomic::AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        let split: Vec<(u32, Vec<(u32, String)>)> =
            self.par_all_rows(first, progress, |_, id, _| {
                let v = self.cell_by_id(id, col).unwrap_or_default();
                if v.trim().is_empty() {
                    return Ok(None);
                }
                let n = split_name(&v);
                let cells: Vec<(u32, String)> = parts
                    .iter()
                    .enumerate()
                    .map(|(k, p)| (slot(k), name_part(&n, *p)))
                    .filter(|(s, t)| *s == col || !t.is_empty())
                    .collect();
                for (_, t) in &cells {
                    if !crate::find::charge_within(&spent, t.len(), budget) {
                        return Err(Error::OutOfMemory(
                            "splitting these names would make too many new cells at once",
                        ));
                    }
                }
                Ok(Some((id, cells)))
            })?;
        let mut plan = crate::ops::ColPlan::new(self);
        if new > 0 {
            plan.insert = Some((at, new));
            plan.cols = new;
        }
        if first == 1 {
            let id = self.doc_id_at(0).ok_or(Error::OutOfRange)?;
            let cells: Vec<(u32, String)> = parts
                .iter()
                .enumerate()
                .filter(|(k, _)| slot(*k) != col)
                .map(|(k, p)| (slot(k), part_title(*p).to_string()))
                .collect();
            plan.cells += cells.len() as u64;
            plan.changes.push((id, cells));
        }
        for (id, cells) in split {
            plan.cells += cells.len() as u64;
            plan.changes.push((id, cells));
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;
    use crate::OpenOptions;

    /// Review of M6: splits count each new cell against the run budget.
    #[test]
    fn big_splits_are_bounded() {
        let rows = crate::find::RUN_BUDGET / 64 / 3 + 1_000;
        let mut text = String::from("name\r\n");
        for _ in 0..rows {
            text.push_str("\"Ann B Lee\"\r\n");
        }
        let d =
            Document::from_bytes(text.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let e = d
            .plan_split_name(
                &SplitName {
                    col: 0,
                    parts: vec![NamePart::First, NamePart::Middle, NamePart::Last],
                    keep_original: true,
                    keep_first: true,
                },
                &Progress::new(),
            )
            .unwrap_err();
        assert!(e.to_string().contains("too many new cells"), "{e}");
        let mut text = String::from("v\r\n");
        for _ in 0..rows {
            text.push_str("\"a b c\"\r\n");
        }
        let d =
            Document::from_bytes(text.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let e = d
            .plan_split_column(
                &crate::ops::SplitColumn {
                    col: 0,
                    separator: " ".into(),
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap_err();
        assert!(e.to_string().contains("too many new cells"), "{e}");
    }

    #[test]
    fn split_names_into_columns() {
        let mut d = Document::from_bytes(
            b"id,name,x\r\n1,Dr. Ann Lee,a\r\n2,\"Smith, John Q.\",b\r\n3,,c\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let plan = d
            .plan_split_name(
                &SplitName {
                    col: 1,
                    parts: vec![NamePart::First, NamePart::Middle, NamePart::Last],
                    keep_original: true,
                    keep_first: true,
                },
                &Progress::new(),
            )
            .unwrap();
        d.apply_cols(plan).unwrap();
        let rows: Vec<Vec<String>> = (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect();
        assert_eq!(
            rows[0],
            ["id", "name", "First name", "Middle name", "Last name", "x"]
        );
        assert_eq!(rows[1], ["1", "Dr. Ann Lee", "Ann", "", "Lee", "a"]);
        assert_eq!(rows[2], ["2", "Smith, John Q.", "John", "Q.", "Smith", "b"]);
        assert_eq!(rows[3], ["3", "", "", "", "", "c"]);
        assert!(d.undo());
        assert_eq!(d.col_count(), 3);
        let e = d
            .plan_split_name(
                &SplitName {
                    col: 9,
                    parts: vec![NamePart::First],
                    keep_original: false,
                    keep_first: false,
                },
                &Progress::new(),
            )
            .unwrap_err();
        let _ = e;
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use crate::find::PatternKind;
    use proptest::prelude::*;

    fn steps() -> Vec<Step> {
        let pat = |t: &str, kind| Pattern {
            text: t.into(),
            kind,
            ..Default::default()
        };
        vec![
            Step::Replace {
                find: pat("a", PatternKind::Plain),
                with: "xyz".into(),
            },
            Step::Replace {
                find: pat(r"(\w)(\w)", PatternKind::Regex),
                with: "$2$1".into(),
            },
            Step::Replace {
                find: pat("*b?", PatternKind::Wildcard),
                with: String::new(),
            },
            Step::Dates {
                ambiguous: None,
                review: None,
                format: "dddd d mmmm yyyy hh:mm:ss AM/PM".into(),
                from: String::new(),
                past: false,
            },
            Step::Numbers {
                decimals: Some(3),
                thousands: true,
                decimal_comma: true,
            },
            Step::Add {
                text: "é".into(),
                at: At::Position { n: 3 },
            },
            Step::Add {
                text: "[".into(),
                at: At::Before { text: "b".into() },
            },
            Step::Random {
                length: 5,
                chars: Charset::Custom {
                    chars: "😀ab".into(),
                },
            },
            Step::Pad {
                width: 9,
                with: '·',
                right: false,
            },
            Step::Remove {
                what: Remove::First { n: 2 },
            },
            Step::Remove {
                what: Remove::Last { n: 3 },
            },
            Step::Remove {
                what: Remove::Range { from: 1, count: 4 },
            },
            Step::Remove {
                what: Remove::Before {
                    text: "a".into(),
                    inclusive: true,
                },
            },
            Step::Remove {
                what: Remove::After {
                    text: "b".into(),
                    inclusive: false,
                },
            },
            Step::Remove {
                what: Remove::Kind {
                    of: Kind::Punctuation,
                },
            },
            Step::Reverse,
            Step::Spaces {
                how: Spaces::Collapse,
            },
            Step::Spaces {
                how: Spaces::LettersDigits,
            },
            Step::Spaces {
                how: Spaces::BeforeCapitals,
            },
            Step::Number {
                start: -3.5,
                step: 0.25,
                digits: 4,
                at_end: true,
                separator: "/".into(),
                replace: false,
            },
            Step::Digits { keep: true },
            Step::Now {
                format: "yy-m-d h:m".into(),
                at_end: false,
                separator: " ".into(),
                replace: false,
            },
            Step::Case { to: CaseTo::Title },
            Step::Case {
                to: CaseTo::Sentence,
            },
            Step::Case { to: CaseTo::Toggle },
            Step::Name {
                part: NamePart::LastFirst,
            },
            Step::Name {
                part: NamePart::Middle,
            },
            Step::HtmlEncode,
            Step::HtmlDecode,
            Step::HtmlText,
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        /// Any list of steps on any cell: no panic, and the result is
        /// valid text no larger than a cell may be.
        #[test]
        fn any_steps_on_any_cell(picks in prop::collection::vec(0usize..30, 1..6), cell in "\\PC{0,40}|[ab,.<>&;#x0-9 ]{0,40}", index in 0u64..1000) {
            let all = steps();
            let c = Cleaner { version: 1, name: String::new(), steps: picks.iter().map(|&i| all[i].clone()).collect() };
            let run = Compiled::new(&c, ReadOptions::default(), 20_000.5).unwrap();
            let out = run.run(&cell, Ctx { index, seed: index, run: 7 }).unwrap();
            if let Some(t) = out.text {
                prop_assert!(t.len() <= MAX_RESULT_CELL_BYTES);
                prop_assert!(t != cell);
            }
            // Every cleaner survives a save and load unchanged.
            prop_assert_eq!(Cleaner::from_json(&c.to_json().unwrap()).unwrap(), c);
        }

        /// Names never panic, and reordering keeps every word.
        #[test]
        fn any_name(s in "\\PC{0,30}|([A-Za-z.]{1,6}[ ,]{1,2}){0,6}") {
            let n = split_name(&s);
            let joined = name_part(&n, NamePart::FirstLast);
            for w in [&n.first, &n.middle, &n.last] {
                prop_assert!(w.is_empty() || joined.contains(w.as_str()));
            }
        }
    }
}

/// Independent review of M6, round 1.
#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::find::{PatternKind, RUN_BUDGET};
    use crate::OpenOptions;

    fn doc(b: &[u8]) -> Document {
        Document::from_bytes(b.to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn one(step: Step, cell: &str, read: ReadOptions) -> String {
        let c = Compiled::new(
            &Cleaner {
                version: 1,
                name: String::new(),
                steps: vec![step],
            },
            read,
            0.0,
        )
        .unwrap();
        c.run(cell, Ctx::default())
            .unwrap()
            .text
            .unwrap_or_else(|| cell.to_string())
    }

    /// Item 1: many tiny changes are bounded, not just their text.
    #[test]
    fn many_tiny_changes_are_refused() {
        // Enough one-character cells that their bookkeeping alone passes the
        // (test-size) budget, while their text is far below it.
        let cells = RUN_BUDGET / 64 + 1_000;
        let rows = cells / 20 + 1;
        let mut text = Vec::with_capacity(rows * 40);
        for _ in 0..rows {
            text.extend_from_slice(b"a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q,r,s,t\r\n");
        }
        assert!(rows * 20 < RUN_BUDGET, "the text alone fits");
        let d = doc(&text);
        let spec = CleanSpec {
            cleaner: Cleaner {
                version: 1,
                name: String::new(),
                steps: vec![Step::Case { to: CaseTo::Upper }],
            },
            scope: Scope::All,
            skip_first: false,
            read: ReadOptions::default(),
        };
        let e = d
            .plan_clean(&spec, 0.0, 1, 0, &Progress::new())
            .unwrap_err();
        assert!(e.to_string().contains("too much at once"), "{e}");
        // Replace-all has the same bound.
        let find = crate::find::FindSpec {
            pattern: Pattern {
                text: "[a-t]".into(),
                kind: PatternKind::Regex,
                ..Default::default()
            },
            scope: Scope::All,
            skip_first: false,
            skip_cols: Vec::new(),
            ..Default::default()
        };
        let e = d.plan_replace(&find, "x", 0, &Progress::new()).unwrap_err();
        assert!(e.to_string().contains("too much at once"), "{e}");
        // A smaller selection works.
        let spec = CleanSpec {
            scope: Scope::Columns { cols: vec![0] },
            ..spec
        };
        let plan = d.plan_clean(&spec, 0.0, 1, 0, &Progress::new()).unwrap();
        assert_eq!(plan.changed, rows as u64);
    }

    /// Item 3: with a decimal comma, badly grouped values are left alone.
    #[test]
    fn badly_grouped_numbers_are_left() {
        let comma = ReadOptions {
            decimal_comma: true,
            ..Default::default()
        };
        let num = Step::Numbers {
            decimals: Some(2),
            thousands: false,
            decimal_comma: true,
        };
        assert_eq!(one(num.clone(), "0.125", comma), "0.125");
        assert_eq!(one(num.clone(), "-0.001", comma), "-0.001");
        assert_eq!(one(num.clone(), "1.2345", comma), "1.2345");
        assert_eq!(one(num.clone(), "1.234,5", comma), "1234,50");
        assert_eq!(one(num.clone(), "12,5", comma), "12,50");
        let point = Step::Numbers {
            decimals: Some(2),
            thousands: false,
            decimal_comma: false,
        };
        assert_eq!(one(point.clone(), "0,125", ReadOptions::default()), "0,125");
        assert_eq!(
            one(point.clone(), "1,234.5", ReadOptions::default()),
            "1234.50"
        );
        assert_eq!(
            one(point, "$12,345,678", ReadOptions::default()),
            "12345678.00"
        );
        // Counted as left alone in a run.
        let d = doc(b"n\r\n0.125\r\n\"1,5\"\r\n");
        let spec = CleanSpec {
            cleaner: Cleaner {
                version: 1,
                name: String::new(),
                steps: vec![num],
            },
            scope: Scope::All,
            skip_first: true,
            read: comma,
        };
        let plan = d.plan_clean(&spec, 0.0, 1, 5, &Progress::new()).unwrap();
        assert_eq!((plan.changed, plan.left), (1, 1));
    }

    /// Item 4: smaller accuracy notes.
    #[test]
    fn accuracy_notes() {
        let r = ReadOptions::default();
        let dec = |d| Step::Numbers {
            decimals: Some(d),
            thousands: false,
            decimal_comma: false,
        };
        assert_eq!(one(dec(2), "2.675", r), "2.68");
        assert_eq!(one(dec(2), "0.125", r), "0.13");
        assert_eq!(one(dec(2), "-0.125", r), "-0.13");
        assert_eq!(one(dec(0), "9.5", r), "10");
        assert_eq!(one(dec(1), "-0.04", r), "0.0");
        assert_eq!(one(dec(3), "1", r), "1.000");
        let p = |s| split_name(s);
        assert_eq!(
            (p("de la Cruz").first, p("de la Cruz").last),
            (String::new(), "de la Cruz".into())
        );
        assert_eq!(p("Maria de la Cruz").last, "de la Cruz");
        let case = |to, s| one(Step::Case { to }, s, r);
        assert_eq!(case(CaseTo::Title, "ßtraße weg"), "ßtraße Weg");
        assert_eq!(case(CaseTo::Sentence, "use E.G. now"), "Use e.g. Now");
        assert_eq!(case(CaseTo::Sentence, "one!two? three"), "One!two? Three");
        // Random text from "aba": both letters equally likely.
        let s = random_text(
            10_000,
            &Charset::Custom {
                chars: "aba".into(),
            },
            7,
        );
        let a = s.chars().filter(|&c| c == 'a').count();
        assert!((4_500..5_500).contains(&a), "{a}");
    }
}

#[cfg(test)]
mod date_review_tests {
    use super::*;

    fn run(chosen: Option<(Ambiguous, Option<DateReview>)>, cell: &str) -> (Option<String>, bool) {
        let step = Step::Dates {
            format: "yyyy-mm-dd".into(),
            from: String::new(),
            past: false,
            ambiguous: chosen.map(|c| c.0),
            review: chosen.and_then(|c| c.1),
        };
        let cleaner = Cleaner {
            version: 1,
            name: String::new(),
            steps: vec![step],
        };
        let c = Compiled::new(&cleaner, ReadOptions::default(), 20000.0).unwrap();
        let ctx = Ctx {
            index: 0,
            seed: 1,
            run: 1,
        };
        let o = c.run(cell, ctx).unwrap();
        (o.text, o.left)
    }

    #[test]
    fn nothing_is_guessed_unless_chosen() {
        let leave = Some((Ambiguous::Leave, Some(DateReview::default())));
        for (cell, want) in [
            ("25/04/2026", Some("2026-04-25")),
            ("04/25/2026", Some("2026-04-25")),
            ("05/05/2026", Some("2026-05-05")),
            ("Jan 5 2024", Some("2024-01-05")),
        ] {
            assert_eq!(run(leave, cell).0.as_deref(), want, "{cell}");
        }
        for cell in ["03/04/2026", "20260425", "4/25/26", "soon", "12345678"] {
            assert_eq!(run(leave, cell), (None, true), "{cell}");
        }
        let with = |ambiguous, compact, two_digit| {
            Some((ambiguous, Some(DateReview { compact, two_digit })))
        };
        assert_eq!(
            run(with(Ambiguous::Mdy, false, false), "03/04/2026")
                .0
                .as_deref(),
            Some("2026-03-04")
        );
        assert_eq!(
            run(with(Ambiguous::Dmy, false, false), "03/04/2026")
                .0
                .as_deref(),
            Some("2026-04-03")
        );
        assert_eq!(
            run(with(Ambiguous::Leave, true, false), "20260425")
                .0
                .as_deref(),
            Some("2026-04-25")
        );
        assert_eq!(
            run(with(Ambiguous::Leave, false, true), "4/25/29")
                .0
                .as_deref(),
            Some("2029-04-25")
        );
        assert_eq!(
            run(with(Ambiguous::Leave, false, true), "4/25/30")
                .0
                .as_deref(),
            Some("1930-04-25")
        );
        // A two-digit ambiguous date needs both choices.
        assert_eq!(
            run(with(Ambiguous::Mdy, false, false), "4/7/26"),
            (None, true)
        );
    }

    #[test]
    fn an_older_recipe_step_reads_as_it_did() {
        // The shape 0.6.0 saved: no review field.
        let old: Step = serde_json::from_str(r#"{"tool":"dates","format":"yyyy-mm-dd"}"#).unwrap();
        assert!(matches!(old, Step::Dates { review: None, .. }));
        assert_eq!(run(None, "03/04/2026").0.as_deref(), Some("2026-03-04"));
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("review"), "{json}");
        // Recorded today: the choice is written, and does what it says.
        assert_eq!(
            run(Some((Ambiguous::Dmy, None)), "03/04/2026").0.as_deref(),
            Some("2026-04-03")
        );
        assert_eq!(
            run(Some((Ambiguous::Leave, None)), "03/04/2026"),
            (None, true)
        );
        assert_eq!(
            run(Some((Ambiguous::Leave, None)), "25/04/2026")
                .0
                .as_deref(),
            Some("2026-04-25")
        );
        assert!(old.describe().contains("as in older versions"));
        let new = Step::Dates {
            format: "yyyy-mm-dd".into(),
            from: String::new(),
            past: false,
            ambiguous: Some(Ambiguous::Leave),
            review: Some(DateReview::default()),
        };
        assert!(new.describe().contains("left as typed"));
        assert!(serde_json::to_string(&new)
            .unwrap()
            .contains("\"ambiguous\":\"leave\""));
        let back: Step = serde_json::from_str(&serde_json::to_string(&new).unwrap()).unwrap();
        assert_eq!(back, new);
        // A recipe records the choice: the one made, or the order run with.
        let spec = |step: &Step| CleanSpec {
            cleaner: Cleaner {
                version: 1,
                name: String::new(),
                steps: vec![step.clone()],
            },
            scope: Scope::Columns { cols: vec![0] },
            skip_first: false,
            read: ReadOptions {
                date_order: crate::value::DateOrder::Dmy,
                ..Default::default()
            },
        };
        assert!(matches!(
            spec(&old).recorded().cleaner.steps[0],
            Step::Dates {
                ambiguous: Some(Ambiguous::Dmy),
                ..
            }
        ));
        assert_eq!(spec(&new).recorded().cleaner.steps[0], new);
    }

    /// Dates left because they could be read two ways are counted apart from
    /// cells that are no date; typed cells go in with the same undo step.
    #[test]
    fn typed_cells_and_the_clear_dates_are_one_undo_step() {
        let body = "d\n03/04/2026\n25/04/2026\nsoon\n07/08/2026\n";
        let mut d = Document::from_bytes(
            body.as_bytes().to_vec(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let spec = CleanSpec {
            cleaner: Cleaner {
                version: 1,
                name: String::new(),
                steps: vec![Step::Dates {
                    format: "yyyy-mm-dd".into(),
                    from: String::new(),
                    past: false,
                    ambiguous: Some(Ambiguous::Leave),
                    review: Some(DateReview::default()),
                }],
            },
            scope: Scope::Columns { cols: vec![0] },
            skip_first: true,
            read: ReadOptions::default(),
        };
        let mut plan = d
            .plan_clean(&spec, 20_000.0, 1, 0, &Progress::new())
            .unwrap();
        assert_eq!((plan.left, plan.ambiguous_left), (3, 2));
        plan.set_typed(&d, &[(1, 0, "2026-04-03".into())]).unwrap();
        assert_eq!(plan.changed, 2);
        let column =
            |d: &Document| -> Vec<String> { (1..5).map(|r| d.cell(r, 0).unwrap()).collect() };
        let before = column(&d);
        d.apply_clean(plan).unwrap();
        assert_eq!(
            column(&d),
            ["2026-04-03", "2026-04-25", "soon", "07/08/2026"]
        );
        d.undo();
        assert_eq!(column(&d), before);
    }
}
