//! File contracts (C9): what a regular file should look like (its columns, their types, allowed
//! values, blank rates, rules and row count), saved once by the person and checked when the next
//! file of that kind opens.
//!
//! A contract is read as untrusted: its size, version and every list are checked before anything is
//! used, and a file that breaks a limit is refused with the reason, never half-read. Recognising a
//! file reads only its name and titles. The drift pass reads every data row once (hidden and filtered
//! ones too) and keeps counts and the first few new values, never row lists: "show these rows" works
//! its rows out again when asked.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::checkfile::{ChecksFile, LoadedRules};
use crate::coltype::{ColumnType, Reader, MAX_TYPED_COLS};
use crate::document::{Document, Progress, Step, View};
use crate::filter::FilterSpec;
use crate::ops::{Cols, CHUNK};
use crate::reshape::RowPlan;
use crate::tablecheck::{TableCheck, TableResult};
use crate::updates::title_key;
use crate::validate::{Check, CheckKind, CheckSpec, NotRun};
use crate::value::ReadOptions;
use crate::Error;

/// The newest contract version this release reads.
pub const VERSION: u32 = 1;
/// Largest contract file read, in bytes.
pub const MAX_FILE_BYTES: usize = 4 << 20;
/// Longest contract name, in characters.
pub const MAX_NAME_CHARS: usize = 100;
/// Longest column title or alias, in characters.
pub const MAX_TITLE_CHARS: usize = 1_000;
/// Most other titles one column may come under.
pub const MAX_ALIASES: usize = 20;
/// Most allowed values one column may keep.
pub const MAX_ALLOWED: usize = 500;
/// Longest allowed value, in characters.
pub const MAX_VALUE_CHARS: usize = 1_000;
/// Most columns with allowed values.
pub const MAX_ALLOWED_COLS: usize = 200;
/// Longest file-name pattern, in characters.
pub const MAX_PATTERN_CHARS: usize = 200;
/// New values listed per column in a report.
pub const REPORT_VALUES: usize = 5;
/// Longest new value listed in a report, in characters.
pub const REPORT_VALUE_CHARS: usize = 200;
/// Share of the contract's titles a file must have to be recognised by its titles.
pub const TITLE_SHARE: f64 = 0.8;
/// Points a column's share of blanks may move before it is reported.
pub const DEFAULT_BLANK_POINTS: f32 = 5.0;
/// The file extension contracts are saved under.
pub const EXTENSION: &str = "ndcontract";

/// Names Windows keeps for devices: a file cannot be called this, with or without an extension.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A contract, as saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    /// The file version.
    pub contract: u32,
    pub name: String,
    pub columns: Vec<ContractColumn>,
    /// The data checks (rules and table checks), their columns by the contract's titles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<ChecksFile>,
    /// The data-row count expected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<RowRange>,
    #[serde(default)]
    pub recognise: Recognise,
    /// Points a share of blanks may move before it is reported.
    #[serde(default = "default_points")]
    pub blank_points: f32,
}

fn default_points() -> f32 {
    DEFAULT_BLANK_POINTS
}

/// One column of a contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractColumn {
    pub title: String,
    /// Other titles the column may come under.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<ColumnType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Allowed>,
    /// The share of blank cells (0 to 1) when the contract was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blank_rate: Option<f32>,
}

/// The values a column may hold (blank cells always may).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowed {
    pub values: Vec<String>,
    #[serde(default)]
    pub match_case: bool,
}

/// The data-row count expected, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowRange {
    pub min: u64,
    pub max: u64,
}

/// How the next file is recognised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recognise {
    /// A file-name pattern (`*` and `?`), case-insensitive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// A file whose titles include most of the contract's.
    #[serde(default = "yes")]
    pub titles: bool,
}

fn yes() -> bool {
    true
}

impl Default for Recognise {
    fn default() -> Self {
        Recognise {
            pattern: None,
            titles: true,
        }
    }
}

fn invalid(why: impl Into<String>) -> Error {
    Error::Invalid(why.into())
}

/// Whether `name` can be a contract's name (and so a file name in the contracts folder). Refused
/// with the reason; nothing is changed or guessed.
pub fn check_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty() {
        return Err(invalid("a contract needs a name"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(format!(
            "a contract name can be at most {MAX_NAME_CHARS} characters"
        )));
    }
    if let Some(c) = name.chars().find(|c| c.is_control()) {
        return Err(invalid(format!(
            "a contract name cannot hold a control character (U+{:04X})",
            c as u32
        )));
    }
    if let Some(c) = name.chars().find(|c| r#"/\:*?"<>|"#.contains(*c)) {
        return Err(invalid(format!("a contract name cannot hold {c}")));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(invalid("a contract name cannot end with a dot or a space"));
    }
    let stem = name.split('.').next().unwrap_or("").trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(invalid(format!(
            "{stem} is a name Windows keeps for itself: choose another"
        )));
    }
    Ok(())
}

/// The file name a contract called `name` is saved under (`name` passed [`check_name`]).
pub fn file_name(name: &str) -> String {
    format!("{name}.{EXTENSION}")
}

/// Whether `name` is one of `existing` apart from case (Windows file names ignore case).
pub fn name_taken<'a>(existing: impl IntoIterator<Item = &'a str>, name: &str) -> bool {
    let low = name.to_lowercase();
    existing.into_iter().any(|e| e.to_lowercase() == low)
}

/// Whether a file name matches a pattern of `*` (any run) and `?` (one character), ignoring case.
pub fn pattern_matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let s: Vec<char> = name.to_lowercase().chars().collect();
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < s.len() {
        if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if i < p.len() && (p[i] == '?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

fn check_text(t: &str, what: &str) -> Result<(), Error> {
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid(format!(
            "{what} can be at most {MAX_TITLE_CHARS} characters"
        )));
    }
    Ok(())
}

impl Contract {
    /// Read and check a contract file's bytes (before anything is used).
    pub fn from_bytes(bytes: &[u8]) -> Result<Contract, Error> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(invalid("the contract file is too large (over 4 MB)"));
        }
        // The version first, so a newer file is turned away by its version, not by a field this
        // build does not know.
        #[derive(Deserialize)]
        struct Version {
            contract: u32,
        }
        let v: Version = serde_json::from_slice(bytes)
            .map_err(|e| invalid(format!("the contract file cannot be read: {e}")))?;
        if v.contract > VERSION {
            return Err(invalid(
                "the contract was made by a newer version of the app",
            ));
        }
        let c: Contract = serde_json::from_slice(bytes)
            .map_err(|e| invalid(format!("the contract file cannot be read: {e}")))?;
        c.validate()?;
        Ok(c)
    }

    /// The bytes saved (checked first, so a bad contract is never written).
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| invalid(format!("the contract cannot be written: {e}")))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(invalid("the contract would be too large (over 4 MB)"));
        }
        Ok(bytes)
    }

    /// Every limit a contract must keep.
    pub fn validate(&self) -> Result<(), Error> {
        if self.contract == 0 || self.contract > VERSION {
            return Err(invalid("the contract has an unknown version"));
        }
        check_name(&self.name)?;
        if self.columns.is_empty() {
            return Err(invalid("a contract needs at least one column"));
        }
        if self.columns.len() > MAX_TYPED_COLS {
            return Err(invalid(format!(
                "a contract can have at most {MAX_TYPED_COLS} columns"
            )));
        }
        let mut with_values = 0;
        for c in &self.columns {
            check_text(&c.title, "a column title")?;
            if c.aliases.len() > MAX_ALIASES {
                return Err(invalid(format!(
                    "a column can have at most {MAX_ALIASES} other titles"
                )));
            }
            for a in &c.aliases {
                check_text(a, "another title")?;
            }
            if let Some(a) = &c.allowed {
                with_values += 1;
                if a.values.len() > MAX_ALLOWED {
                    return Err(invalid(format!(
                        "column {} has more than {MAX_ALLOWED} allowed values",
                        c.title
                    )));
                }
                if a.values.iter().any(|v| v.chars().count() > MAX_VALUE_CHARS) {
                    return Err(invalid(format!(
                        "an allowed value can be at most {MAX_VALUE_CHARS} characters"
                    )));
                }
            }
            if let Some(r) = c.blank_rate {
                if !(0.0..=1.0).contains(&r) {
                    return Err(invalid("a share of blanks must be between 0 and 1"));
                }
            }
        }
        if with_values > MAX_ALLOWED_COLS {
            return Err(invalid(format!(
                "at most {MAX_ALLOWED_COLS} columns can have allowed values"
            )));
        }
        if !(0.0..=100.0).contains(&self.blank_points) {
            return Err(invalid(
                "the blanks threshold must be between 0 and 100 points",
            ));
        }
        if let Some(r) = self.rows {
            if r.min > r.max {
                return Err(invalid("the row count range is the wrong way round"));
            }
        }
        if let Some(p) = &self.recognise.pattern {
            if p.chars().count() > MAX_PATTERN_CHARS {
                return Err(invalid(format!(
                    "a file-name pattern can be at most {MAX_PATTERN_CHARS} characters"
                )));
            }
            if p.trim().is_empty() {
                return Err(invalid("the file-name pattern is empty"));
            }
            if let Some(c) = p
                .chars()
                .find(|c| c.is_control() || r#"/\:"<>|[]"#.contains(*c))
            {
                return Err(invalid(format!(
                    "a file-name pattern cannot hold {}: only * and ? are wild",
                    if c.is_control() {
                        "a control character".to_string()
                    } else {
                        c.to_string()
                    }
                )));
            }
        }
        if let Some(cf) = &self.checks {
            // As the rules file loader checks them.
            let bytes = serde_json::to_vec(cf)
                .map_err(|e| invalid(format!("the contract's rules cannot be read: {e}")))?;
            ChecksFile::from_bytes(&bytes)?;
        }
        Ok(())
    }

    /// The contract's checks as they fit `doc` once its columns have the contract's titles (after
    /// Fix what I can). `None` when it has none.
    pub fn checks_for(&self, doc: &Document) -> Option<LoadedRules> {
        self.checks.as_ref().map(|cf| cf.resolve(doc))
    }

    /// The share of the contract's columns whose title (or another title) is among `keys`.
    fn title_share(&self, keys: &HashSet<String>) -> f64 {
        if self.columns.is_empty() {
            return 0.0;
        }
        let found = self
            .columns
            .iter()
            .filter(|c| {
                keys.contains(&title_key(&c.title))
                    || c.aliases.iter().any(|a| keys.contains(&title_key(a)))
            })
            .count();
        found as f64 / self.columns.len() as f64
    }
}

/// A contract a file looks like.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Match {
    /// Which contract (its place in the list given).
    pub index: usize,
    /// The file name matched the contract's pattern.
    pub by_pattern: bool,
    /// The share of the contract's titles the file has (0 when titles were not looked at).
    pub title_share: f64,
}

impl Match {
    fn score(&self) -> f64 {
        if self.by_pattern {
            1.0 + self.title_share
        } else {
            self.title_share
        }
    }
}

/// The contracts a file looks like, best first (a pattern match beats titles alone; ties keep the
/// list's order). Reads only the file name and the titles (`None`: the tab has no title row, so
/// only a pattern can match). Builds the file's title keys once.
pub fn recognise<'a>(
    contracts: impl IntoIterator<Item = &'a Contract>,
    file_name: Option<&str>,
    titles: Option<&[String]>,
) -> Vec<Match> {
    let keys: Option<HashSet<String>> = titles.map(|t| t.iter().map(|s| title_key(s)).collect());
    let mut out: Vec<Match> = Vec::new();
    for (index, c) in contracts.into_iter().enumerate() {
        let by_pattern = match (&c.recognise.pattern, file_name) {
            (Some(p), Some(f)) => pattern_matches(p, f),
            _ => false,
        };
        let share = keys.as_ref().map_or(0.0, |k| c.title_share(k));
        let by_titles = c.recognise.titles && keys.is_some() && share >= TITLE_SHARE;
        if by_pattern || by_titles {
            out.push(Match {
                index,
                by_pattern,
                title_share: share,
            });
        }
    }
    out.sort_by(|a, b| b.score().total_cmp(&a.score()));
    out
}

/// How a file's columns pair with a contract's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Pairing {
    /// For each contract column, the file's column.
    pub map: Vec<Option<u32>>,
    /// Contract columns paired by their place alone ("probably renamed").
    pub renamed: Vec<u32>,
    /// Contract columns paired by another title.
    pub by_alias: Vec<u32>,
    /// Contract columns the file does not have.
    pub missing: Vec<u32>,
    /// File columns the contract does not have, in the file's order.
    pub added: Vec<u32>,
    /// Contract columns out of the contract's order (the longest run in order stays put).
    pub moved: Vec<u32>,
}

/// Pair a file's columns (`titles`, in order) with a contract's: exact titles first (repeated
/// titles pair in order), then other titles, then by place. A contract column is paired by place
/// only when it is the one unpaired column between two paired neighbours and exactly one unpaired
/// file column sits between their partners: it is never a guess between two.
pub fn pair(contract: &Contract, titles: &[String]) -> Pairing {
    let n = contract.columns.len();
    let mut queue: HashMap<String, VecDeque<u32>> = HashMap::new();
    for (j, t) in titles.iter().enumerate() {
        queue.entry(title_key(t)).or_default().push_back(j as u32);
    }
    let mut map: Vec<Option<u32>> = contract
        .columns
        .iter()
        .map(|c| {
            queue
                .get_mut(&title_key(&c.title))
                .and_then(|q| q.pop_front())
        })
        .collect();
    let mut by_alias = Vec::new();
    for (k, c) in contract.columns.iter().enumerate() {
        if map[k].is_some() {
            continue;
        }
        for a in &c.aliases {
            if let Some(j) = queue.get_mut(&title_key(a)).and_then(|q| q.pop_front()) {
                map[k] = Some(j);
                by_alias.push(k as u32);
                break;
            }
        }
    }
    let mut taken = vec![false; titles.len()];
    for j in map.iter().flatten() {
        taken[*j as usize] = true;
    }
    let mut renamed = Vec::new();
    let mut k = 0;
    while k < n {
        if map[k].is_some() {
            k += 1;
            continue;
        }
        let start = k;
        while k < n && map[k].is_none() {
            k += 1;
        }
        if k - start != 1 {
            continue;
        }
        let lo = if start == 0 {
            0
        } else {
            map[start - 1].expect("the run is maximal") as usize + 1
        };
        let hi = if k == n {
            titles.len()
        } else {
            map[k].expect("the run is maximal") as usize
        };
        if lo > hi {
            continue;
        }
        let free: Vec<usize> = (lo..hi).filter(|&j| !taken[j]).collect();
        if let [j] = free[..] {
            map[start] = Some(j as u32);
            taken[j] = true;
            renamed.push(start as u32);
        }
    }
    let missing = (0..n as u32)
        .filter(|&k| map[k as usize].is_none())
        .collect();
    let added = (0..titles.len() as u32)
        .filter(|&j| !taken[j as usize])
        .collect();
    let paired: Vec<(u32, u32)> = map
        .iter()
        .enumerate()
        .filter_map(|(k, j)| j.map(|j| (k as u32, j)))
        .collect();
    let keep = longest_rising(&paired.iter().map(|p| p.1).collect::<Vec<_>>());
    let moved = paired
        .iter()
        .enumerate()
        .filter(|(i, _)| !keep[*i])
        .map(|(_, p)| p.0)
        .collect();
    Pairing {
        map,
        renamed,
        by_alias,
        missing,
        added,
        moved,
    }
}

/// Which items of `v` form its longest strictly rising run (the earliest one when several tie).
fn longest_rising(v: &[u32]) -> Vec<bool> {
    // tails[l]: index of the smallest last item of a rising run of length l + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; v.len()];
    for i in 0..v.len() {
        let at = tails.partition_point(|&t| v[t] < v[i]);
        if at > 0 {
            prev[i] = Some(tails[at - 1]);
        }
        if at == tails.len() {
            tails.push(i);
        } else {
            tails[at] = i;
        }
    }
    let mut keep = vec![false; v.len()];
    let mut at = tails.last().copied();
    while let Some(i) = at {
        keep[i] = true;
        at = prev[i];
    }
    keep
}

/// How a drift pass reads the table.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct DriftOptions {
    /// The first row holds titles and is not counted.
    pub keep_first: bool,
    pub read: ReadOptions,
    /// Local date and time as days since 1970 (TODAY, NOW in rules).
    pub now: f64,
}

/// A column paired with another title.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Renamed {
    /// The contract column.
    pub col: u32,
    /// The file's column.
    pub file_col: u32,
    /// The title in the file, and the contract's.
    pub from: String,
    pub to: String,
    /// Paired by another title the contract accepts (not drift), else by place.
    pub by_alias: bool,
}

/// What one paired column holds against the contract.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColumnDrift {
    /// The contract column, and the file's.
    pub col: u32,
    pub file_col: u32,
    pub title: String,
    pub blanks: u64,
    /// Share of blank cells now (0 to 1), and when the contract was made.
    pub blank_rate: f32,
    pub baseline: Option<f32>,
    /// The share moved by more than the contract's threshold.
    pub blank_jump: bool,
    /// Filled cells that are not of the contract's type.
    pub type_misses: u64,
    /// Filled cells whose value is not allowed, and the first few such values.
    pub new_values: u64,
    pub first_new: Vec<String>,
}

/// A rule's result against the file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleDrift {
    /// The rule's number in the contract.
    pub check: u32,
    pub name: String,
    /// Rows that break it.
    pub failing: u64,
}

/// What a file holds against a contract.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DriftReport {
    pub contract: String,
    /// Data rows counted (all of them, hidden and filtered ones too).
    pub rows: u64,
    pub row_range: Option<RowRange>,
    /// The row count is outside the range.
    pub rows_out: bool,
    pub renamed: Vec<Renamed>,
    /// Contract titles the file does not have.
    pub missing: Vec<String>,
    /// File titles the contract does not have.
    pub added: Vec<String>,
    /// Contract titles out of order.
    pub moved: Vec<String>,
    pub columns: Vec<ColumnDrift>,
    pub rules: Vec<RuleDrift>,
    /// Rules and table checks that did not run, and why (numbers: rules first, then table checks
    /// after the rules).
    pub not_run: Vec<NotRun>,
    pub table: Vec<TableResult>,
    /// Anything drifted.
    pub drifted: bool,
}

/// What "show these rows" shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriftRows {
    Blanks {
        col: u32,
    },
    TypeMisses {
        col: u32,
    },
    NewValues {
        col: u32,
    },
    /// A rule, by its number in the contract.
    Rule {
        check: u32,
    },
}

/// One paired column's settings for the pass.
struct Probe {
    reader: Option<Reader>,
    allowed: Option<(HashSet<String>, bool)>,
}

impl Probe {
    fn new(c: &ContractColumn, read: ReadOptions) -> Probe {
        let reader = match c.ty {
            Some(t @ (ColumnType::Number | ColumnType::Date(_))) => {
                Some(Reader::with_type(read, t))
            }
            _ => None,
        };
        let allowed = c.allowed.as_ref().map(|a| {
            let set = a
                .values
                .iter()
                .map(|v| fold(v.trim(), a.match_case).into_owned())
                .collect();
            (set, a.match_case)
        });
        Probe { reader, allowed }
    }

    /// A filled cell (trimmed) is not of the type.
    #[inline]
    fn misses(&self, v: &str) -> bool {
        match &self.reader {
            Some(r) => match r.column_type() {
                ColumnType::Number => r.number(v).is_none(),
                _ => r.date(v).is_none(),
            },
            None => false,
        }
    }

    /// A filled cell (trimmed) is not allowed.
    #[inline]
    fn is_new(&self, v: &str) -> bool {
        match &self.allowed {
            Some((set, case)) => !set.contains(fold(v, *case).as_ref()),
            None => false,
        }
    }
}

fn fold(v: &str, match_case: bool) -> Cow<'_, str> {
    if match_case {
        Cow::Borrowed(v)
    } else {
        Cow::Owned(v.to_lowercase())
    }
}

fn cut(v: &str) -> String {
    match v.char_indices().nth(REPORT_VALUE_CHARS) {
        Some((i, _)) => format!("{}…", &v[..i]),
        None => v.to_string(),
    }
}

/// One chunk's counts.
#[derive(Default)]
struct Part {
    blanks: Vec<u64>,
    misses: Vec<u64>,
    new: Vec<u64>,
    first: Vec<Vec<String>>,
}

impl Document {
    /// The data rows by position in the document (every row, shown or not).
    fn contract_rows(&self, keep_first: bool) -> std::ops::Range<u64> {
        let n = self.total_rows();
        u64::from(keep_first && n > 0).min(n)..n
    }

    /// Check `pairing` still fits this table (an edit may have moved or removed columns).
    fn check_pairing(&self, contract: &Contract, pairing: &Pairing) -> Result<(), Error> {
        let n = self.col_count();
        if pairing.map.len() != contract.columns.len()
            || pairing.map.iter().flatten().any(|&j| j >= n)
            || pairing.added.iter().any(|&j| j >= n)
        {
            return Err(invalid(
                "the table's columns changed: check it against the contract again",
            ));
        }
        Ok(())
    }

    /// The contract's checks for this file: each rule's columns found through the pairing (so a
    /// renamed column still gets its rules). Rules that cannot run here are returned with the
    /// reason. `only`: just that rule.
    fn contract_spec(
        &self,
        contract: &Contract,
        pairing: &Pairing,
        opt: &DriftOptions,
        only: Option<u32>,
    ) -> (CheckSpec, Vec<u32>, Vec<NotRun>) {
        let mut spec = CheckSpec {
            keep_first: opt.keep_first,
            read: opt.read,
            now: opt.now,
            ..Default::default()
        };
        let mut numbers = Vec::new();
        let mut not_run = Vec::new();
        let Some(cf) = &contract.checks else {
            return (spec, numbers, not_run);
        };
        let here = self.rule_titles(opt.keep_first);
        // Contract title -> this file's title.
        let to_file: HashMap<&str, &str> = contract
            .columns
            .iter()
            .zip(&pairing.map)
            .filter_map(|(c, j)| {
                j.and_then(|j| here.get(j as usize))
                    .map(|t| (c.title.as_str(), t.as_str()))
            })
            .collect();
        let gone = |t: &String| !to_file.contains_key(t.as_str());
        let swap = |t: &mut String| {
            if let Some(f) = to_file.get(t.as_str()) {
                *t = (*f).to_string();
            }
        };
        for (i, rule) in cf.rules.iter().enumerate() {
            if only.is_some_and(|o| o != i as u32) {
                continue;
            }
            let i = i as u32;
            let mut r = rule.clone();
            let names: Vec<&String> = r
                .title
                .iter()
                .chain(&r.also_titles)
                .chain(r.by_title.iter())
                .collect();
            if let Some(t) = names.into_iter().find(|t| gone(t)) {
                not_run.push(NotRun {
                    check: i,
                    why: format!("this file has no column {t}"),
                });
                continue;
            }
            if matches!(r.kind, CheckKind::ForeignKey { .. }) {
                not_run.push(NotRun {
                    check: i,
                    why: "it reads another table: run it from Data checks".into(),
                });
                continue;
            }
            if let Some(t) = &mut r.title {
                swap(t);
            }
            r.also_titles.iter_mut().for_each(swap);
            if let Some(t) = &mut r.by_title {
                swap(t);
            }
            let one = ChecksFile {
                version: cf.version,
                keep_first: opt.keep_first,
                rules: vec![r],
                table_checks: Vec::new(),
            };
            let got = one.resolve(self);
            match got.checks.into_iter().next() {
                Some(c) => {
                    spec.checks.push(c);
                    numbers.push(i);
                }
                None => not_run.push(NotRun {
                    check: i,
                    why: "it names a column this file does not have, or does not fit it".into(),
                }),
            }
        }
        if only.is_none() {
            let base = cf.rules.len() as u32;
            for (i, tc) in cf.table_checks.iter().enumerate() {
                let i = base + i as u32;
                let mut s = tc.clone();
                if let Some(t) = &s.title {
                    if gone(t) {
                        not_run.push(NotRun {
                            check: i,
                            why: format!("this file has no column {t}"),
                        });
                        continue;
                    }
                }
                if let Some(t) = &mut s.title {
                    swap(t);
                }
                let one = ChecksFile {
                    version: cf.version,
                    keep_first: opt.keep_first,
                    rules: Vec::new(),
                    table_checks: vec![s],
                };
                let got = one.resolve(self);
                match got.table_checks.into_iter().next() {
                    Some(t) => spec.table_checks.push(TableCheck {
                        name: t.name,
                        kind: t.kind,
                    }),
                    None => not_run.push(NotRun {
                        check: i,
                        why: "it names a column this file does not have".into(),
                    }),
                }
            }
        }
        (spec, numbers, not_run)
    }

    /// Count blanks, type misses and new values of the paired columns in one pass over every data
    /// row. `work`: (contract column, file column).
    fn contract_counts(
        &self,
        contract: &Contract,
        work: &[(u32, u32)],
        opt: &DriftOptions,
        progress: &Progress,
    ) -> Result<Part, Error> {
        let probes: Vec<Probe> = work
            .iter()
            .map(|&(k, _)| Probe::new(&contract.columns[k as usize], opt.read))
            .collect();
        let file_cols: Vec<u32> = work.iter().map(|w| w.1).collect();
        let reader = Cols::new(self, &file_cols);
        let rows = self.contract_rows(opt.keep_first);
        let (r0, r1) = (rows.start, rows.end);
        let m = work.len();
        progress.start(r1 - r0);
        let chunks = (r1 - r0).div_ceil(CHUNK as u64);
        let parts: Vec<Part> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Part, Error> {
                let s = r0 + c * CHUNK as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut p = Part {
                    blanks: vec![0; m],
                    misses: vec![0; m],
                    new: vec![0; m],
                    first: vec![Vec::new(); m],
                };
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.doc_id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for (k, pr) in probes.iter().enumerate() {
                        let v = reader.get(&buf, k).trim();
                        if v.is_empty() {
                            p.blanks[k] += 1;
                            continue;
                        }
                        if pr.misses(v) {
                            p.misses[k] += 1;
                        }
                        if pr.is_new(v) {
                            p.new[k] += 1;
                            let f = &mut p.first[k];
                            if f.len() < REPORT_VALUES && !f.iter().any(|x| x == v) {
                                f.push(v.to_string());
                            }
                        }
                    }
                }
                progress.step(e - s)?;
                Ok(p)
            })
            .collect::<Result<_, _>>()?;
        let mut all = Part {
            blanks: vec![0; m],
            misses: vec![0; m],
            new: vec![0; m],
            first: vec![Vec::new(); m],
        };
        for p in parts {
            for k in 0..m {
                all.blanks[k] += p.blanks[k];
                all.misses[k] += p.misses[k];
                all.new[k] += p.new[k];
                for v in p.first[k].iter() {
                    let f = &mut all.first[k];
                    if f.len() < REPORT_VALUES && !f.contains(v) {
                        f.push(v.clone());
                    }
                }
            }
        }
        Ok(all)
    }

    /// Check this table against a contract: one pass over every data row for the paired columns'
    /// blanks, types and values, then the contract's rules (as Data checks runs them). Keeps
    /// counts and the first few new values only. Cancellable through `progress`.
    pub fn drift(
        &self,
        contract: &Contract,
        pairing: &Pairing,
        opt: &DriftOptions,
        progress: &Progress,
    ) -> Result<DriftReport, Error> {
        self.ready_for_bulk()?;
        self.check_pairing(contract, pairing)?;
        let here = self.rule_titles(opt.keep_first);
        let file_title = |j: u32| here.get(j as usize).cloned().unwrap_or_default();
        let work: Vec<(u32, u32)> = pairing
            .map
            .iter()
            .enumerate()
            .filter_map(|(k, j)| j.map(|j| (k as u32, j)))
            .collect();
        let counts = self.contract_counts(contract, &work, opt, progress)?;
        let rows = self.contract_rows(opt.keep_first);
        let total = rows.end - rows.start;
        let mut drifted = false;
        let mut columns = Vec::with_capacity(work.len());
        for (i, &(k, j)) in work.iter().enumerate() {
            let c = &contract.columns[k as usize];
            let rate = if total == 0 {
                0.0
            } else {
                (counts.blanks[i] as f64 / total as f64) as f32
            };
            let blank_jump = c
                .blank_rate
                .is_some_and(|b| ((rate - b) * 100.0).abs() > contract.blank_points);
            drifted |= blank_jump || counts.misses[i] > 0 || counts.new[i] > 0;
            columns.push(ColumnDrift {
                col: k,
                file_col: j,
                title: c.title.clone(),
                blanks: counts.blanks[i],
                blank_rate: rate,
                baseline: c.blank_rate,
                blank_jump,
                type_misses: counts.misses[i],
                new_values: counts.new[i],
                first_new: counts.first[i].iter().map(|v| cut(v)).collect(),
            });
        }
        let renamed: Vec<Renamed> = work
            .iter()
            .filter_map(|&(k, j)| {
                let place = pairing.renamed.contains(&k);
                let alias = pairing.by_alias.contains(&k);
                (place || alias).then(|| Renamed {
                    col: k,
                    file_col: j,
                    from: file_title(j),
                    to: contract.columns[k as usize].title.clone(),
                    by_alias: alias,
                })
            })
            .collect();
        let title = |k: &u32| contract.columns[*k as usize].title.clone();
        let missing: Vec<String> = pairing.missing.iter().map(title).collect();
        let added: Vec<String> = pairing.added.iter().map(|&j| file_title(j)).collect();
        let moved: Vec<String> = pairing.moved.iter().map(title).collect();
        drifted |= renamed.iter().any(|r| !r.by_alias)
            || !missing.is_empty()
            || !added.is_empty()
            || !moved.is_empty();
        let rows_out = contract
            .rows
            .is_some_and(|r| total < r.min || total > r.max);
        drifted |= rows_out;
        let (spec, numbers, mut not_run) = self.contract_spec(contract, pairing, opt, None);
        let mut rules = Vec::new();
        let mut table = Vec::new();
        if !spec.checks.is_empty() || !spec.table_checks.is_empty() {
            let rep = self.run_checks(&spec, progress)?;
            for (i, c) in spec.checks.iter().enumerate() {
                let number = numbers[i];
                if let Some(n) = rep.not_run.iter().find(|n| n.check == i as u32) {
                    not_run.push(NotRun {
                        check: number,
                        why: n.why.clone(),
                    });
                    continue;
                }
                let failing = rep.per_check.get(i).copied().unwrap_or(0);
                drifted |= failing > 0;
                rules.push(RuleDrift {
                    check: number,
                    name: rule_name(c),
                    failing,
                });
            }
            drifted |= rep.table.iter().any(|t| t.ok == Some(false));
            table = rep.table;
        }
        not_run.sort_by_key(|n| n.check);
        Ok(DriftReport {
            contract: contract.name.clone(),
            rows: total,
            row_range: contract.rows,
            rows_out,
            renamed,
            missing,
            added,
            moved,
            columns,
            rules,
            not_run,
            table,
            drifted,
        })
    }

    /// A plan that shows the rows behind one count of a drift report (worked out again now, as
    /// one pass over that column, or a run of that rule), and the title row with `keep_first`.
    pub fn plan_drift_rows(
        &self,
        contract: &Contract,
        pairing: &Pairing,
        what: DriftRows,
        opt: &DriftOptions,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        self.check_pairing(contract, pairing)?;
        if let DriftRows::Rule { check } = what {
            let (spec, numbers, not_run) = self.contract_spec(contract, pairing, opt, Some(check));
            if let Some(n) = not_run.first() {
                return Err(invalid(format!("that rule did not run: {}", n.why)));
            }
            if numbers.is_empty() {
                return Err(Error::OutOfRange);
            }
            let rep = self.run_checks(&spec, progress)?;
            let mut plan = self.plan_show_problems(&rep, opt.keep_first, false)?;
            plan.chip.kind = "contract".into();
            plan.chip.label = format!("Rows breaking {}", rule_name(&spec.checks[0]));
            return Ok(plan);
        }
        let (DriftRows::Blanks { col }
        | DriftRows::TypeMisses { col }
        | DriftRows::NewValues { col }) = what
        else {
            unreachable!("rules are handled above")
        };
        let c = contract
            .columns
            .get(col as usize)
            .ok_or(Error::OutOfRange)?;
        let file_col = pairing.map[col as usize]
            .ok_or_else(|| invalid(format!("this file has no column {}", c.title)))?;
        let probe = Probe::new(c, opt.read);
        let reader = Cols::new(self, &[file_col]);
        let rows = self.contract_rows(opt.keep_first);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let chunks = (r1 - r0).div_ceil(CHUNK as u64);
        let hits: Vec<Vec<u32>> = (0..chunks)
            .into_par_iter()
            .map(|k| -> Result<Vec<u32>, Error> {
                let s = r0 + k * CHUNK as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.doc_id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let v = reader.get(&buf, 0).trim();
                    let hit = match what {
                        DriftRows::Blanks { .. } => v.is_empty(),
                        DriftRows::TypeMisses { .. } => !v.is_empty() && probe.misses(v),
                        _ => !v.is_empty() && probe.is_new(v),
                    };
                    if hit {
                        out.push(id);
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let base = self.next_row_id();
        let mut bits = vec![0u64; (base as usize).div_ceil(64)];
        let mut shown = 0u64;
        for id in hits.into_iter().flatten() {
            bits[(id >> 6) as usize] |= 1 << (id & 63);
            shown += 1;
        }
        if opt.keep_first && self.total_rows() > 0 {
            let id = self.doc_id_at(0).expect("a first row");
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        }
        let mut plan = RowPlan::new(self);
        plan.rows = shown;
        plan.chip.kind = "contract".into();
        plan.chip.label = match what {
            DriftRows::Blanks { .. } => format!("Blanks in {}", c.title),
            DriftRows::TypeMisses { .. } => format!("Not of the type of {}", c.title),
            _ => format!("New values in {}", c.title),
        };
        plan.view = Some(View::new(
            FilterSpec {
                keep_first: opt.keep_first,
                ..Default::default()
            },
            bits,
            base,
        ));
        Ok(plan)
    }

    /// Fix what can be fixed without touching a value, as one undo step: the titles of paired
    /// columns become the contract's, and the columns go in the contract's order (columns the
    /// contract does not have go after them, in their order). Nothing is deleted. Returns the
    /// types to set, by the column's place after the fix. The contract's checks fit the fixed
    /// table through [`Contract::checks_for`].
    pub fn contract_fix(
        &mut self,
        contract: &Contract,
        pairing: &Pairing,
        keep_first: bool,
    ) -> Result<ContractFix, Error> {
        self.ready()?;
        self.check_pairing(contract, pairing)?;
        let mut fix = ContractFix::default();
        let mut cells = Vec::new();
        if keep_first && self.total_rows() > 0 {
            let now = self.first_row_values()?;
            for (k, j) in pairing.map.iter().enumerate() {
                let Some(j) = *j else { continue };
                let want = &contract.columns[k].title;
                let have = now.get(j as usize).map_or("", String::as_str);
                // A protected column keeps its title (as every edit leaves it alone).
                if have != want && !self.is_protected(j) {
                    let phys = self.phys(j).ok_or(Error::OutOfRange)?;
                    cells.push((phys, want.clone()));
                    fix.renamed.push((have.to_string(), want.clone()));
                }
            }
        }
        let mut order: Vec<u32> = pairing.map.iter().flatten().copied().collect();
        order.extend_from_slice(&pairing.added);
        let n = self.col_count();
        if order.len() as u32 != n {
            return Err(invalid(
                "the table's columns changed: check it against the contract again",
            ));
        }
        fix.moved = order.iter().enumerate().any(|(i, &j)| i as u32 != j);
        for (place, (k, _)) in pairing
            .map
            .iter()
            .enumerate()
            .filter(|(_, j)| j.is_some())
            .enumerate()
        {
            if let Some(t) = contract.columns[k].ty {
                fix.types.push((place as u32, t));
            }
        }
        let mut step = Step::default();
        if !cells.is_empty() {
            cells.sort_by_key(|c| c.0);
            let id = self.doc_id_at(0).ok_or(Error::OutOfRange)?;
            step.states = self.patch_states(vec![(id, cells)])?;
        }
        if fix.moved {
            let map: Option<Vec<u32>> = order.iter().map(|&j| self.phys(j)).collect();
            step.map = Some(map.ok_or(Error::OutOfRange)?);
        }
        if !step.states.is_empty() || step.map.is_some() {
            self.push_step(step);
        }
        Ok(fix)
    }

    /// A contract's draft from this table: titles, the types set on the tab, blank shares from one
    /// pass, and for the columns asked, their different values (refused for a column with more
    /// than [`MAX_ALLOWED`]). The checks are stored by title.
    pub fn contract_draft(&self, spec: &DraftSpec, progress: &Progress) -> Result<Draft, Error> {
        self.ready_for_bulk()?;
        check_name(&spec.name)?;
        let n = self.col_count();
        if n as usize > MAX_TYPED_COLS {
            return Err(invalid(format!(
                "a contract can have at most {MAX_TYPED_COLS} columns"
            )));
        }
        if spec.allowed_cols.len() > MAX_ALLOWED_COLS {
            return Err(invalid(format!(
                "at most {MAX_ALLOWED_COLS} columns can have allowed values"
            )));
        }
        if spec.allowed_cols.iter().any(|&c| c >= n) {
            return Err(Error::OutOfRange);
        }
        let titles = self.rule_titles(spec.keep_first);
        let ctx = self.read_ctx(spec.read);
        let rows = self.contract_rows(spec.keep_first);
        let (r0, r1) = (rows.start, rows.end);
        let all: Vec<u32> = (0..n).collect();
        let reader = Cols::new(self, &all);
        let want: Vec<u32> = spec.allowed_cols.clone();
        let slot_of: HashMap<u32, usize> = want.iter().enumerate().map(|(i, &c)| (c, i)).collect();
        progress.start(r1 - r0);
        let chunks = (r1 - r0).div_ceil(CHUNK as u64);
        type Chunk = (Vec<u64>, Vec<HashSet<String>>);
        let parts: Vec<Chunk> = (0..chunks)
            .into_par_iter()
            .map(|k| -> Result<Chunk, Error> {
                let s = r0 + k * CHUNK as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut blanks = vec![0u64; n as usize];
                let mut sets: Vec<HashSet<String>> = vec![HashSet::new(); want.len()];
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.doc_id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for c in 0..n as usize {
                        let v = reader.get(&buf, c).trim();
                        if v.is_empty() {
                            blanks[c] += 1;
                        } else if let Some(&i) = slot_of.get(&(c as u32)) {
                            let set = &mut sets[i];
                            if set.len() <= MAX_ALLOWED && !set.contains(v) {
                                set.insert(v.to_string());
                            }
                        }
                    }
                }
                progress.step(e - s)?;
                Ok((blanks, sets))
            })
            .collect::<Result<_, _>>()?;
        let mut blanks = vec![0u64; n as usize];
        let mut sets: Vec<HashSet<String>> = vec![HashSet::new(); want.len()];
        for (b, s) in parts {
            for c in 0..n as usize {
                blanks[c] += b[c];
            }
            for (i, set) in s.into_iter().enumerate() {
                if sets[i].len() <= MAX_ALLOWED {
                    sets[i].extend(set);
                }
            }
        }
        let total = r1 - r0;
        let mut columns: Vec<ContractColumn> = (0..n)
            .map(|c| {
                let t = ctx.type_of(c);
                ContractColumn {
                    title: titles.get(c as usize).cloned().unwrap_or_default(),
                    aliases: Vec::new(),
                    ty: (t != ColumnType::Auto).then_some(t),
                    allowed: None,
                    blank_rate: Some(if total == 0 {
                        0.0
                    } else {
                        (blanks[c as usize] as f64 / total as f64) as f32
                    }),
                }
            })
            .collect();
        let mut values = Vec::new();
        for (i, &c) in want.iter().enumerate() {
            let filled = total - blanks[c as usize];
            let set = std::mem::take(&mut sets[i]);
            let distinct = set.len() as u64;
            if set.len() > MAX_ALLOWED {
                values.push(DraftValues {
                    col: c,
                    distinct,
                    filled,
                    looks_personal: true,
                    refused: Some(format!("it has more than {MAX_ALLOWED} different values")),
                });
                continue;
            }
            let mut list: Vec<String> = set.into_iter().collect();
            list.sort_by(|a, b| crate::coltype::id_order(a, b));
            let looks_personal = (filled > 0 && distinct * 2 > filled)
                || columns[c as usize].ty == Some(ColumnType::Id);
            columns[c as usize].allowed = Some(Allowed {
                values: list,
                match_case: true,
            });
            values.push(DraftValues {
                col: c,
                distinct,
                filled,
                looks_personal,
                refused: None,
            });
        }
        let checks = (!spec.checks.is_empty() || !spec.table_checks.is_empty()).then(|| {
            ChecksFile::for_checks(self, &spec.checks, &spec.table_checks, spec.keep_first)
        });
        let contract = Contract {
            contract: VERSION,
            name: spec.name.clone(),
            columns,
            checks,
            rows: None,
            recognise: Recognise::default(),
            blank_points: DEFAULT_BLANK_POINTS,
        };
        Ok(Draft {
            contract,
            rows: total,
            values,
        })
    }
}

fn rule_name(c: &Check) -> String {
    c.name.clone().unwrap_or_else(|| c.describe())
}

/// What Fix what I can did.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct ContractFix {
    /// Titles changed (from, to).
    pub renamed: Vec<(String, String)>,
    /// The columns were put in the contract's order.
    pub moved: bool,
    /// The contract's types, by the column's place after the fix.
    pub types: Vec<(u32, ColumnType)>,
}

/// What a draft is made from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DraftSpec {
    pub name: String,
    pub keep_first: bool,
    /// Columns whose different values become the allowed list.
    #[serde(default)]
    pub allowed_cols: Vec<u32>,
    /// The tab's checks, to include.
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub table_checks: Vec<TableCheck>,
    #[serde(default)]
    pub read: ReadOptions,
}

/// A column's values as the draft found them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftValues {
    pub col: u32,
    /// Different filled values (more than [`MAX_ALLOWED`] means "too many"), and filled cells.
    pub distinct: u64,
    pub filled: u64,
    /// More than half the filled values differ, or the column is an ID: its values look like
    /// personal data, so keeping them needs a confirm.
    pub looks_personal: bool,
    /// Why the column cannot have allowed values.
    pub refused: Option<String>,
}

/// A contract's draft, for the dialog.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Draft {
    pub contract: Contract,
    /// Data rows counted.
    pub rows: u64,
    pub values: Vec<DraftValues>,
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
