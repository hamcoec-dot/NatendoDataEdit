//! Data checks: rules a column's cells (or a whole row) must meet. A run
//! counts and lists the problems and can show only the rows that have
//! them; the grid marks problem cells on screen. Blank cells pass every
//! check except "required". Checks never change data.

use std::borrow::Cow;
use std::collections::HashSet;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::coltype::{ColumnType, Reader};
use crate::document::{Document, Progress, View};
use crate::expr::{Ctx, Expr};
use crate::filter::FilterSpec;
use crate::find::{Matcher, Pattern};
use crate::ops::{check_cols, Cols};
use crate::reshape::RowPlan;
use crate::value::ReadOptions;
use crate::Error;

/// Most checks at once.
pub const MAX_CHECKS: usize = 200;
/// Problems listed by a run (all are counted).
pub const MAX_LISTED: usize = 1_000;
/// Most values in an allowed-values list.
pub const MAX_LIST: usize = 100_000;
/// Most columns in one unique rule's key (its column and the `also` columns).
pub const MAX_KEY_COLS: usize = 8;
/// Longest value shown in the problem list, in characters.
const SHOWN_CHARS: usize = 200;
/// Rows per parallel task.
const CHUNK: u64 = 1 << 15;

/// What a check asks of a cell (or, for `expression`, of a row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CheckKind {
    Required,
    Number {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default)]
        whole: bool,
    },
    Date {
        #[serde(default)]
        min: Option<String>,
        #[serde(default)]
        max: Option<String>,
    },
    /// Only a real calendar date written `YYYY-MM-DD` (`2024-02-29` yes, `2023-02-29`,
    /// `02/03/2024` and `2024-2-3` no); blank cells pass (use Required for those).
    DateIso,
    Length {
        #[serde(default)]
        min: Option<u32>,
        #[serde(default)]
        max: Option<u32>,
    },
    List {
        values: Vec<String>,
        #[serde(default)]
        match_case: bool,
    },
    Pattern {
        pattern: Pattern,
    },
    /// No two rows share a value; with `also`, no two rows share the values of the
    /// rule's column and every `also` column together (a row whose key parts are all
    /// blank passes). At most `MAX_KEY_COLS` columns in all, none twice.
    Unique {
        #[serde(default)]
        match_case: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        also: Vec<u32>,
    },
    /// The row passes when the expression is TRUE.
    Expression {
        expr: String,
    },
}

/// One check: a kind on a column (`col` is ignored for expressions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    #[serde(default)]
    pub col: Option<u32>,
    pub kind: CheckKind,
    /// What the rule is called ("5-digit ZIP"), for tooltips and reports; a name made
    /// from the kind when none is given. Files and views from before names read as None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Longest rule name, in characters.
pub const MAX_NAME_CHARS: usize = 100;

/// Checks to run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckSpec {
    pub checks: Vec<Check>,
    /// The first row holds titles and is not checked.
    pub keep_first: bool,
    pub read: ReadOptions,
    /// Local date and time as days since 1970 (TODAY, NOW in expressions).
    pub now: f64,
}

/// One problem: shown row, column (`None` for a row check), check number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub row: u64,
    pub col: Option<u32>,
    pub check: u32,
    /// The cell's text (cut to 200 characters).
    pub value: String,
}

/// What a run found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CheckReport {
    /// Rows checked.
    pub checked: u64,
    /// Rows with at least one problem.
    pub rows: u64,
    /// Problems in all.
    pub problems: u64,
    /// Problems per check.
    pub per_check: Vec<u64>,
    /// The first problems, in row order.
    pub listed: Vec<Problem>,
    /// Failing row ids (one bit each), for "show rows with problems".
    #[serde(skip)]
    pub failing: Vec<u64>,
    /// Per unique check (by number): row ids whose value repeats.
    #[serde(skip)]
    pub repeats: Vec<(u32, Vec<u64>)>,
}

/// A part's nearest problem: row, column, rule number, row rule.
type PartHit = (u64, u32, u32, bool);

/// Where a jump to the next problem landed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Jump {
    /// Shown row.
    pub row: u64,
    /// Column of the cell to select (a row rule: the row's first column not hidden).
    pub col: u32,
    /// The check's number.
    pub check: u32,
    /// The problem is a row rule's (the whole row), not one cell's.
    pub row_rule: bool,
}

/// The result of a jump: the problem found (none: the table has none in shown columns),
/// whether the search wrapped round the table, and how many problems in hidden columns
/// it passed over on the way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct JumpResult {
    pub hit: Option<Jump>,
    pub wrapped: bool,
    pub hidden_skipped: u64,
}

/// A problem cell on screen (`col` is `None` for a row check).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CheckMark {
    pub row: u64,
    pub col: Option<u32>,
    pub check: u32,
}

enum Test {
    Required,
    Number {
        min: f64,
        max: f64,
        whole: bool,
    },
    Date {
        min: f64,
        max: f64,
    },
    DateIso,
    Length {
        min: usize,
        max: usize,
    },
    List {
        values: HashSet<String>,
        match_case: bool,
    },
    Pattern(Matcher),
    /// The other key columns (`also`), marked with the rule's column when a row repeats.
    Unique(Vec<u32>),
    Expr(Expr),
}

/// A unique rule's `also` columns, checked: at most `MAX_KEY_COLS` in the key, none twice,
/// not the rule's own column.
fn key_also(col: u32, also: &[u32]) -> Result<(), Error> {
    if also.len() + 1 > MAX_KEY_COLS {
        return Err(Error::Invalid(format!(
            "a unique rule can use at most {MAX_KEY_COLS} columns together"
        )));
    }
    for (i, &k) in also.iter().enumerate() {
        if k == col || also[..i].contains(&k) {
            return Err(Error::Invalid(
                "a unique rule names the same column twice".into(),
            ));
        }
    }
    Ok(())
}

struct Compiled {
    tests: Vec<Test>,
    /// Columns read: every check's column, then each expression's columns.
    reader_cols: Vec<u32>,
    /// For each check: where its column sits in `reader_cols`, and for
    /// expressions, where their columns start.
    at: Vec<usize>,
    /// For expression checks: the type of each column the expression reads.
    types: Vec<Vec<ColumnType>>,
    /// How each check reads its column: by the column's type when it is Number or Date (a "must match this
    /// column's type" rule), else the plain rule (a Text or Id column is checked by the rule the person wrote).
    readers: Vec<Reader>,
}

fn bound_date(s: &Option<String>, read: ReadOptions, what: &str) -> Result<Option<f64>, Error> {
    match s.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(t) => Reader::plain(read)
            .date(t)
            .map(|d| Some(d.floor()))
            .ok_or_else(|| Error::Invalid(format!("the {what} date is not a date: {t}"))),
    }
}

fn compile(d: &Document, spec: &CheckSpec) -> Result<Compiled, Error> {
    if spec.checks.len() > MAX_CHECKS {
        return Err(Error::Invalid(format!("at most {MAX_CHECKS} checks")));
    }
    let names = d.expr_names(spec.keep_first);
    let mut tests = Vec::new();
    let mut reader_cols = Vec::new();
    let mut at = Vec::new();
    let mut exprs = Vec::new();
    let mut types: Vec<Vec<ColumnType>> = Vec::new();
    let mut readers: Vec<Reader> = Vec::new();
    let ctx = d.read_ctx(spec.read);
    for c in &spec.checks {
        let needs_col = !matches!(c.kind, CheckKind::Expression { .. });
        if needs_col {
            let col = c
                .col
                .ok_or_else(|| Error::Invalid("a check needs a column".into()))?;
            check_cols(d, &[col])?;
            at.push(reader_cols.len());
            reader_cols.push(col);
            let rd = ctx.reader(col);
            readers.push(if rd.column_type().is_text_like() {
                Reader::plain(spec.read)
            } else {
                rd
            });
        } else {
            at.push(usize::MAX);
            readers.push(Reader::plain(spec.read));
        }
        types.push(Vec::new());
        tests.push(match &c.kind {
            CheckKind::Required => Test::Required,
            CheckKind::Number { min, max, whole } => {
                if min.is_some_and(f64::is_nan) || max.is_some_and(f64::is_nan) {
                    return Err(Error::Invalid("a number limit is not a number".into()));
                }
                Test::Number {
                    min: min.unwrap_or(f64::NEG_INFINITY),
                    max: max.unwrap_or(f64::INFINITY),
                    whole: *whole,
                }
            }
            CheckKind::Date { min, max } => Test::Date {
                min: bound_date(min, spec.read, "earliest")?.unwrap_or(f64::NEG_INFINITY),
                max: bound_date(max, spec.read, "latest")?.unwrap_or(f64::INFINITY),
            },
            CheckKind::DateIso => Test::DateIso,
            CheckKind::Length { min, max } => Test::Length {
                min: min.unwrap_or(0) as usize,
                max: max.map_or(usize::MAX, |m| m as usize),
            },
            CheckKind::List { values, match_case } => {
                if values.len() > MAX_LIST {
                    return Err(Error::Invalid(format!(
                        "a list can hold at most {MAX_LIST} values"
                    )));
                }
                Test::List {
                    values: values
                        .iter()
                        .map(|v| {
                            if *match_case {
                                v.clone()
                            } else {
                                v.to_lowercase()
                            }
                        })
                        .collect(),
                    match_case: *match_case,
                }
            }
            CheckKind::Pattern { pattern } => {
                let mut p = pattern.clone();
                p.whole_cell = true;
                Test::Pattern(Matcher::new(&p)?)
            }
            CheckKind::Unique { also, .. } => {
                let col = c.col.expect("checked above");
                key_also(col, also)?;
                check_cols(d, also)?;
                // Read right after the rule's column (`at + 1 ..`), for the problem list.
                reader_cols.extend_from_slice(also);
                Test::Unique(also.clone())
            }
            CheckKind::Expression { expr } => {
                let e = Expr::parse(expr, &names)?;
                check_cols(d, e.columns())?;
                exprs.push((tests.len(), e.columns().to_vec()));
                if let Some(slot) = types.last_mut() {
                    *slot = e.types(d, spec.read);
                }
                Test::Expr(e)
            }
        });
    }
    for (i, cols) in exprs {
        at[i] = reader_cols.len();
        reader_cols.extend(cols);
    }
    Ok(Compiled {
        tests,
        reader_cols,
        at,
        types,
        readers,
    })
}

impl Compiled {
    /// Does check `i` pass for this row? (`Unique` always passes here.)
    fn passes(
        &self,
        i: usize,
        reader: &Cols,
        buf: &[Cow<'_, str>],
        row: u64,
        spec: &CheckSpec,
    ) -> bool {
        let read = spec.read;
        let plain = self.readers[i];
        let t = &self.tests[i];
        if let Test::Expr(e) = t {
            let a = self.at[i];
            let ctx = Ctx {
                cells: buf,
                slots: &reader.slots()[a..a + e.columns().len()],
                row,
                now: spec.now,
                read,
                prev: None,
                next: None,
                refs: None,
                types: &self.types[i],
            };
            return e.holds(&ctx);
        }
        let v = reader.get(buf, self.at[i]);
        if v.trim().is_empty() {
            return !matches!(t, Test::Required);
        }
        match t {
            Test::Required | Test::Unique(_) | Test::Expr(_) => true,
            Test::DateIso => is_iso_date(v),
            // A check the person wrote reads cells by its own rule (not by the column's type).
            Test::Number { min, max, whole } => match plain.number(v) {
                Some(n) => n >= *min && n <= *max && (!whole || n.fract() == 0.0),
                None => false,
            },
            Test::Date { min, max } => match plain.date(v) {
                Some(d) if plain.number(v).is_none() => {
                    let d = d.floor();
                    d >= *min && d <= *max
                }
                _ => false,
            },
            Test::Length { min, max } => {
                let n = v.chars().count();
                n >= *min && n <= *max
            }
            Test::List { values, match_case } => {
                if *match_case {
                    values.contains(v)
                } else {
                    values.contains(&v.to_lowercase())
                }
            }
            Test::Pattern(m) => m.is_match(v),
        }
    }
}

/// A real calendar date written `YYYY-MM-DD` (four digits, two, two; leap years right).
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let year: u32 = s[0..4].parse().unwrap_or(0);
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + u32::from(leap),
        _ => return false,
    };
    (1..=days).contains(&day)
}

impl Check {
    /// A check on a column (or, for an expression, on rows) with no name.
    pub fn new(col: Option<u32>, kind: CheckKind) -> Check {
        Check {
            col,
            kind,
            name: None,
        }
    }

    /// What to say about this rule: its name, else a sentence made from its kind
    /// ("Number from 1 to 9", "One of 12 values", "Matches ^\\d{5}$").
    pub fn describe(&self) -> String {
        if let Some(n) = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            return n.to_string();
        }
        fn range<T: std::fmt::Display>(what: &str, min: &Option<T>, max: &Option<T>) -> String {
            match (min, max) {
                (Some(a), Some(b)) => format!("{what} from {a} to {b}"),
                (Some(a), None) => format!("{what} of at least {a}"),
                (None, Some(b)) => format!("{what} of at most {b}"),
                (None, None) => what.to_string(),
            }
        }
        match &self.kind {
            CheckKind::Required => "Required (not blank)".into(),
            CheckKind::Number { min, max, whole } => {
                let r = range("Number", min, max);
                if *whole {
                    format!("Whole {}", r.to_lowercase())
                } else {
                    r
                }
            }
            CheckKind::Date { min, max } => range("Date", min, max),
            CheckKind::DateIso => "Date written YYYY-MM-DD".into(),
            CheckKind::Length { min, max } => range("Length", min, max),
            CheckKind::List { values, .. } => format!("One of {} values", values.len()),
            CheckKind::Pattern { pattern } => format!("Matches {}", pattern.text),
            CheckKind::Unique { also, .. } if !also.is_empty() => format!(
                "Unique together with {} other column{} (no repeats)",
                also.len(),
                if also.len() == 1 { "" } else { "s" }
            ),
            CheckKind::Unique { .. } => "Unique (no repeats)".into(),
            CheckKind::Expression { expr } => format!("Row rule: {expr}"),
        }
    }
}

fn cut(v: &str) -> String {
    if v.chars().count() <= SHOWN_CHARS {
        v.to_string()
    } else {
        let mut s: String = v.chars().take(SHOWN_CHARS).collect();
        s.push('…');
        s
    }
}

fn bit(bits: &[u64], id: u32) -> bool {
    (id as usize >> 6) < bits.len() && (bits[id as usize >> 6] >> (id & 63)) & 1 == 1
}

#[derive(Default)]
struct Part {
    rows: u64,
    problems: u64,
    per_check: Vec<u64>,
    listed: Vec<Problem>,
    failing: Vec<u32>,
}

impl Document {
    /// Run the checks over the shown rows.
    pub fn run_checks(&self, spec: &CheckSpec, progress: &Progress) -> Result<CheckReport, Error> {
        self.ready_for_bulk()?;
        let c = compile(self, spec)?;
        let n = spec.checks.len();
        let rows = self.data_rows(spec.keep_first);
        // Unique checks first: which rows repeat a value.
        let mut repeats: Vec<(u32, Vec<u64>)> = Vec::new();
        let base = self.next_row_id();
        for (i, ch) in spec.checks.iter().enumerate() {
            if let CheckKind::Unique { match_case, also } = &ch.kind {
                let match_case = *match_case;
                let mut key = vec![ch.col.expect("checked")];
                key.extend_from_slice(also);
                let in_group = self.repeated_keys(rows.clone(), &key, match_case, progress)?;
                let mut bits = vec![0u64; (base as usize).div_ceil(64)];
                let reader = Cols::new(self, &key);
                // Repeated rows with a value (some key part not blank), found in parallel chunks.
                let ids: Vec<Vec<u32>> = in_group
                    .par_chunks(1 << 16)
                    .enumerate()
                    .map(|(c, part)| -> Result<Vec<u32>, Error> {
                        let mut buf = Vec::new();
                        let mut out = Vec::new();
                        for (k, g) in part.iter().enumerate() {
                            if *g {
                                let at = rows.start + (c * (1 << 16) + k) as u64;
                                let id = self.id_at(at).ok_or(Error::OutOfRange)?;
                                reader.read(self, id, &mut buf);
                                if (0..key.len()).any(|k| !reader.get(&buf, k).trim().is_empty()) {
                                    out.push(id);
                                }
                            }
                        }
                        Ok(out)
                    })
                    .collect::<Result<_, _>>()?;
                for id in ids.into_iter().flatten() {
                    bits[(id >> 6) as usize] |= 1 << (id & 63);
                }
                repeats.push((i as u32, bits));
            }
        }
        let reader = Cols::new(self, &c.reader_cols);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1 - r0);
        let chunks = (r1 - r0).div_ceil(CHUNK);
        let parts: Vec<Part> = (0..chunks)
            .into_par_iter()
            .map(|k| -> Result<Part, Error> {
                let s = r0 + k * CHUNK;
                let e = (s + CHUNK).min(r1);
                let mut p = Part {
                    per_check: vec![0; n],
                    ..Default::default()
                };
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let mut bad = false;
                    for i in 0..n {
                        let ok = match &c.tests[i] {
                            Test::Unique(_) => !repeats
                                .iter()
                                .find(|x| x.0 == i as u32)
                                .is_some_and(|x| bit(&x.1, id)),
                            _ => c.passes(i, &reader, &buf, r + 1, spec),
                        };
                        if !ok {
                            bad = true;
                            p.problems += 1;
                            p.per_check[i] += 1;
                            if p.listed.len() < MAX_LISTED {
                                let col = match c.tests[i] {
                                    Test::Expr(_) => None,
                                    _ => Some(c.reader_cols[c.at[i]]),
                                };
                                let value = match (col, &c.tests[i]) {
                                    (None, _) => String::new(),
                                    (Some(_), Test::Unique(also)) if !also.is_empty() => {
                                        let a = c.at[i];
                                        cut(&(a..=a + also.len())
                                            .map(|k| reader.get(&buf, k))
                                            .collect::<Vec<_>>()
                                            .join(" | "))
                                    }
                                    (Some(_), _) => cut(reader.get(&buf, c.at[i])),
                                };
                                p.listed.push(Problem {
                                    row: r,
                                    col,
                                    check: i as u32,
                                    value,
                                });
                            }
                        }
                    }
                    if bad {
                        p.rows += 1;
                        p.failing.push(id);
                    }
                }
                progress.step(e - s)?;
                Ok(p)
            })
            .collect::<Result<_, _>>()?;
        let mut rep = CheckReport {
            checked: r1 - r0,
            per_check: vec![0; n],
            failing: vec![0u64; (base as usize).div_ceil(64)],
            repeats,
            ..Default::default()
        };
        for p in parts {
            rep.rows += p.rows;
            rep.problems += p.problems;
            for (a, b) in rep.per_check.iter_mut().zip(p.per_check) {
                *a += b;
            }
            let room = MAX_LISTED - rep.listed.len();
            rep.listed.extend(p.listed.into_iter().take(room));
            for id in p.failing {
                rep.failing[(id >> 6) as usize] |= 1 << (id & 63);
            }
        }
        Ok(rep)
    }

    /// For each shown row in `rows` (by offset), whether its key (the values of `key`
    /// together) is shared with another row. A row whose first key column is unique cannot
    /// repeat the whole key, so the other columns are read only for rows whose first
    /// column repeats: with no repeats this costs one column.
    pub(crate) fn repeated_keys(
        &self,
        rows: std::ops::Range<u64>,
        key: &[u32],
        match_case: bool,
        progress: &Progress,
    ) -> Result<Vec<bool>, Error> {
        let flags = |leader: &[u32]| {
            let mut out = vec![false; leader.len()];
            for (k, &l) in leader.iter().enumerate() {
                if l as usize != k {
                    out[l as usize] = true;
                    out[k] = true;
                }
            }
            out
        };
        let first = flags(&self.group_rows(rows.clone(), &key[..1], match_case, true, progress)?);
        if key.len() == 1 {
            return Ok(first);
        }
        let at: Vec<u64> = first
            .iter()
            .enumerate()
            .filter(|(_, &g)| g)
            .map(|(k, _)| rows.start + k as u64)
            .collect();
        let again = flags(&self.group_rows_at(&at, key, match_case, true, progress)?);
        let mut out = vec![false; first.len()];
        for (j, g) in again.into_iter().enumerate() {
            if g {
                out[(at[j] - rows.start) as usize] = true;
            }
        }
        Ok(out)
    }

    /// A plan that shows only the rows a run found problems in (or, with
    /// `passing`, only the shown rows it found none in), and the title row
    /// with `keep_first`.
    pub fn plan_show_problems(
        &self,
        report: &CheckReport,
        keep_first: bool,
        passing: bool,
    ) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        let base = self.next_row_id();
        let mut bits = report.failing.clone();
        bits.resize((base as usize).div_ceil(64), 0);
        let mut shown = report.rows;
        if passing {
            let mut ok = vec![0u64; bits.len()];
            shown = 0;
            for id in self.ids().skip(usize::from(keep_first)) {
                if !bit(&bits, id) {
                    ok[(id >> 6) as usize] |= 1 << (id & 63);
                    shown += 1;
                }
            }
            bits = ok;
        }
        if keep_first && self.row_count() > 0 {
            let id = self.id_at(0).expect("a first row");
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        }
        let mut plan = RowPlan::new(self);
        plan.rows = shown;
        plan.view = Some(View::new(
            FilterSpec {
                keep_first,
                ..Default::default()
            },
            bits,
            base,
        ));
        Ok(plan)
    }

    /// A sort that puts the rows a run found problems in first (in their
    /// order now), then the rest; the title row stays first with
    /// `keep_first`. Rows hidden by a filter were not checked and go last.
    pub fn plan_sort_problems(
        &self,
        report: &CheckReport,
        keep_first: bool,
    ) -> Result<crate::sort::SortPlan, Error> {
        self.ready_for_bulk()?;
        let total = self.total_rows();
        let first = u64::from(keep_first && total > 0);
        let mut order: Vec<u32> = Vec::with_capacity(total as usize);
        let mut rest: Vec<u32> = Vec::new();
        if first == 1 {
            order.push(self.doc_id_at(0).expect("a first row"));
        }
        for r in first..total {
            let id = self.doc_id_at(r).ok_or(Error::OutOfRange)?;
            if bit(&report.failing, id) {
                order.push(id);
            } else {
                rest.push(id);
            }
        }
        order.extend(rest);
        let same = (0..total).all(|r| self.doc_id_at(r) == Some(order[r as usize]));
        Ok(crate::sort::SortPlan::from_order(
            self,
            (!same).then_some(order),
        ))
    }

    /// A "Check result" column after the last column: `OK`, or `Fails:`
    /// and the checks the row fails (`ZIP: pattern; Email: required`), for
    /// every shown row. `report` is a run of the same checks (for unique
    /// checks).
    pub fn plan_check_column(
        &self,
        spec: &CheckSpec,
        report: &CheckReport,
        title: &str,
        progress: &Progress,
    ) -> Result<crate::ops::ColPlan, Error> {
        self.ready_for_bulk()?;
        let c = compile(self, spec)?;
        let n = spec.checks.len();
        let titles = self.titles(
            &spec
                .checks
                .iter()
                .map(|k| k.col.unwrap_or(0))
                .collect::<Vec<_>>(),
            spec.keep_first,
        );
        let names: Vec<String> = spec
            .checks
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let what = match k.kind {
                    CheckKind::Required => "required",
                    CheckKind::Number { .. } => "number",
                    CheckKind::Date { .. } => "date",
                    CheckKind::DateIso => "ISO date",
                    CheckKind::Length { .. } => "length",
                    CheckKind::List { .. } => "list",
                    CheckKind::Pattern { .. } => "pattern",
                    CheckKind::Unique { .. } => "repeated",
                    CheckKind::Expression { .. } => "rule",
                };
                match k.kind {
                    CheckKind::Expression { .. } => format!("rule {}", i + 1),
                    _ => format!("{}: {what}", titles[i]),
                }
            })
            .collect();
        let rows = self.data_rows(spec.keep_first);
        let at = self.col_count();
        let reader = Cols::new(self, &c.reader_cols);
        let spent = std::sync::atomic::AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        progress.start(rows.end - rows.start);
        let chunks = (rows.end - rows.start).div_ceil(CHUNK);
        type Changes = Vec<(u32, Vec<(u32, String)>)>;
        let parts: Vec<Changes> = (0..chunks)
            .into_par_iter()
            .map(|k| -> Result<_, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut out = Vec::with_capacity((e - s) as usize);
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let mut text = String::new();
                    for (i, name) in names.iter().enumerate().take(n) {
                        let ok = match &c.tests[i] {
                            Test::Unique(_) => !report
                                .repeats
                                .iter()
                                .find(|x| x.0 == i as u32)
                                .is_some_and(|x| bit(&x.1, id)),
                            _ => c.passes(i, &reader, &buf, r + 1, spec),
                        };
                        if !ok {
                            text.push_str(if text.is_empty() { "Fails: " } else { "; " });
                            text.push_str(name);
                        }
                    }
                    if text.is_empty() {
                        text.push_str("OK");
                    }
                    if !crate::find::charge_within(&spent, text.len() + 32, budget) {
                        return Err(Error::OutOfMemory(
                            "this would change too many cells at once; use fewer rows",
                        ));
                    }
                    out.push((id, vec![(at, text)]));
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let mut plan = crate::ops::ColPlan::new(self);
        plan.insert = Some((at, 1));
        plan.cols = 1;
        if rows.start == 1 {
            let t = if title.trim().is_empty() {
                "Check result"
            } else {
                title
            };
            plan.changes.push((
                self.id_at(0).expect("a first row"),
                vec![(at, t.chars().take(200).collect())],
            ));
        }
        for p in parts {
            plan.changes.extend(p);
        }
        plan.cells = plan.changes.len() as u64;
        Ok(plan)
    }

    /// Problem cells among shown rows `row..row+rows`, columns
    /// `col..col+cols`. Unique checks use `repeats` from the last run (rows
    /// it did not see pass). Row checks are marked with `col: None`.
    #[allow(clippy::too_many_arguments)] // an area (4 numbers) plus the checks
    pub fn check_marks(
        &self,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        spec: &CheckSpec,
        repeats: &[(u32, Vec<u64>)],
    ) -> Result<Vec<CheckMark>, Error> {
        let c = compile(self, spec)?;
        let rows = rows.min(crate::highlight::MAX_ROWS);
        let cols = cols.min(crate::highlight::MAX_COLS);
        let n = self.row_count();
        let r0 = row.max(u64::from(spec.keep_first && n > 0));
        let r1 = row.saturating_add(rows).min(n);
        Ok(self.marks_in(&c, r0..r1, col..col.saturating_add(cols), spec, repeats))
    }

    /// Problem marks of shown rows `rows` (the whole of it, in parallel), for
    /// Go To Special: row checks have `col: None`.
    pub fn problem_marks(
        &self,
        rows: std::ops::Range<u64>,
        spec: &CheckSpec,
        repeats: &[(u32, Vec<u64>)],
        progress: &Progress,
    ) -> Result<Vec<CheckMark>, Error> {
        self.ready_for_bulk()?;
        let c = compile(self, spec)?;
        let n = self.row_count();
        let r0 = rows.start.max(u64::from(spec.keep_first && n > 0));
        let r1 = rows.end.min(n);
        if r0 >= r1 {
            return Ok(Vec::new());
        }
        progress.start(r1 - r0);
        let chunk = crate::ops::CHUNK as u64;
        let parts: Vec<Vec<CheckMark>> = (0..(r1 - r0).div_ceil(chunk))
            .into_par_iter()
            .map(|k| -> Result<Vec<CheckMark>, Error> {
                let s = r0 + k * chunk;
                let e = (s + chunk).min(r1);
                let out = self.marks_in(&c, s..e, 0..u32::MAX, spec, repeats);
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        Ok(parts.into_iter().flatten().collect())
    }

    /// Whether the rules fit this table (their columns exist, their patterns and
    /// expressions compile): the same checks a run makes first, without running.
    pub fn rules_fit(&self, spec: &CheckSpec) -> Result<(), Error> {
        compile(self, spec).map(|_| ())
    }

    /// The titles the rules are stored under (see `checkfile`): "Column N" for a column with no title.
    pub fn rule_titles(&self, keep_first: bool) -> Vec<String> {
        self.expr_names(keep_first)
    }

    /// The next problem cell after (`from_row`, `from_col`) in the order the table is shown
    /// (row by row, left to right), or before it with `backward`, wrapping round the table
    /// (`wrapped`). A row rule lands on the row's first column that is not in `skip_cols`.
    /// Problems in `skip_cols` (hidden columns) are passed over, and `hidden_skipped` counts
    /// the ones passed on the way. Looks in blocks of rows, in parallel inside a block, and
    /// stops at the first block with a hit; cancellable through `progress`.
    #[allow(clippy::too_many_arguments)] // a start (2 numbers), a direction and the run's state
    pub fn next_problem(
        &self,
        from_row: u64,
        from_col: u32,
        backward: bool,
        spec: &CheckSpec,
        repeats: &[(u32, Vec<u64>)],
        skip_cols: &[u32],
        progress: &Progress,
    ) -> Result<JumpResult, Error> {
        self.ready_for_bulk()?;
        let c = compile(self, spec)?;
        let n = self.row_count();
        let first_row = u64::from(spec.keep_first && n > 0);
        let mut out = JumpResult::default();
        if n <= first_row {
            return Ok(out);
        }
        let Some(first_col) = (0..self.col_count()).find(|k| !skip_cols.contains(k)) else {
            return Ok(out);
        };
        let from_row = from_row.min(n - 1).max(first_row);
        // One row's problem cells: (column, check, row rule), hidden columns counted and left out.
        let row_cells = |r: u64, hidden: &mut u64| -> Vec<(u32, u32, bool)> {
            let mut v = Vec::new();
            for m in self.marks_in(&c, r..r + 1, 0..u32::MAX, spec, repeats) {
                match m.col {
                    Some(k) if skip_cols.contains(&k) => *hidden += 1,
                    Some(k) => v.push((k, m.check, false)),
                    None => v.push((first_col, m.check, true)),
                }
            }
            v.sort_by_key(|x| (x.0, x.1));
            v
        };
        let pick = |cells: &[(u32, u32, bool)], after: Option<u32>| -> Option<(u32, u32, bool)> {
            let ok = |k: u32| match after {
                None => true,
                Some(a) if backward => k < a,
                Some(a) => k > a,
            };
            if backward {
                cells.iter().rev().find(|x| ok(x.0)).copied()
            } else {
                cells.iter().find(|x| ok(x.0)).copied()
            }
        };
        // The start row, past the start cell.
        let mut hidden = 0u64;
        let here = row_cells(from_row, &mut hidden);
        if let Some((k, ch, rr)) = pick(&here, Some(from_col)) {
            out.hit = Some(Jump {
                row: from_row,
                col: k,
                check: ch,
                row_rule: rr,
            });
            out.hidden_skipped = hidden;
            return Ok(out);
        }
        // The rows after (before) it, then, wrapped, the rows from the other end up to it.
        let ranges: [(std::ops::Range<u64>, bool); 2] = if backward {
            [(first_row..from_row, false), (from_row + 1..n, true)]
        } else {
            [(from_row + 1..n, false), (first_row..from_row, true)]
        };
        let total: u64 = ranges
            .iter()
            .map(|r| r.0.end.saturating_sub(r.0.start))
            .sum::<u64>()
            + 1;
        progress.start(total);
        const BLOCK: u64 = 1 << 15;
        const PART: u64 = 2048;
        for (range, wrapped) in ranges {
            let (a, b) = (range.start, range.end);
            if a >= b {
                continue;
            }
            // Blocks grow as the search goes on: a hit close by is found in a small block,
            // a long miss is not cut into many small ones.
            let (mut done_rows, mut size) = (0u64, BLOCK);
            while done_rows < b - a {
                // Counted from the near end (from the start going forward, from the end going back).
                let take = size.min(b - a - done_rows);
                let (s0, e0) = if backward {
                    (b - done_rows - take, b - done_rows)
                } else {
                    (a + done_rows, a + done_rows + take)
                };
                done_rows += take;
                size = (size * 2).min(1 << 20);
                let parts = (e0 - s0).div_ceil(PART);
                let found: Vec<(Option<PartHit>, u64)> = (0..parts)
                    .into_par_iter()
                    .map(|i| {
                        let (ps, pe) = (s0 + i * PART, (s0 + (i + 1) * PART).min(e0));
                        // One pass over the part, then the row nearest the start of the search.
                        let mut hid = 0u64;
                        let mut best: Option<u64> = None;
                        let ms = self.marks_in(&c, ps..pe, 0..u32::MAX, spec, repeats);
                        for m in &ms {
                            if m.col.is_some_and(|k| skip_cols.contains(&k)) {
                                continue;
                            }
                            best = Some(match best {
                                None => m.row,
                                Some(b) if backward => b.max(m.row),
                                Some(b) => b.min(m.row),
                            });
                        }
                        for m in &ms {
                            let before =
                                best.is_none_or(|b| if backward { m.row >= b } else { m.row <= b });
                            if before && m.col.is_some_and(|k| skip_cols.contains(&k)) {
                                hid += 1;
                            }
                        }
                        let Some(r) = best else {
                            return (None, hid);
                        };
                        let mut cells: Vec<(u32, u32, bool)> = ms
                            .iter()
                            .filter(|m| m.row == r)
                            .filter_map(|m| match m.col {
                                Some(k) if skip_cols.contains(&k) => None,
                                Some(k) => Some((k, m.check, false)),
                                None => Some((first_col, m.check, true)),
                            })
                            .collect();
                        cells.sort_by_key(|x| (x.0, x.1));
                        match pick(&cells, None) {
                            Some((kk, ch, rr)) => (Some((r, kk, ch, rr)), hid),
                            None => (None, hid),
                        }
                    })
                    .collect();
                // Parts in the order they are met (the last part first going back).
                let ordered: Vec<_> = if backward {
                    found.into_iter().rev().collect()
                } else {
                    found
                };
                for (hit, hid) in ordered {
                    hidden += hid;
                    if let Some((r, kk, ch, rr)) = hit {
                        out.hit = Some(Jump {
                            row: r,
                            col: kk,
                            check: ch,
                            row_rule: rr,
                        });
                        out.wrapped = wrapped;
                        out.hidden_skipped = hidden;
                        return Ok(out);
                    }
                }
                progress.step(e0 - s0)?;
            }
        }
        // Nothing elsewhere: the start row's own cells before the start cell (wrapping onto itself).
        let mut hid = 0u64;
        let again = row_cells(from_row, &mut hid);
        if let Some((k, ch, rr)) = pick(&again, None) {
            out.hit = Some(Jump {
                row: from_row,
                col: k,
                check: ch,
                row_rule: rr,
            });
            out.wrapped = true;
        }
        out.hidden_skipped = hidden;
        Ok(out)
    }

    fn marks_in(
        &self,
        c: &Compiled,
        rows: std::ops::Range<u64>,
        cols: std::ops::Range<u32>,
        spec: &CheckSpec,
        repeats: &[(u32, Vec<u64>)],
    ) -> Vec<CheckMark> {
        let reader = Cols::new(self, &c.reader_cols);
        let mut out = Vec::new();
        let mut buf = Vec::new();
        for r in rows {
            let Some(id) = self.id_at(r) else { continue };
            reader.read(self, id, &mut buf);
            let mut done: Vec<Option<u32>> = Vec::new();
            for i in 0..spec.checks.len() {
                let mc = match c.tests[i] {
                    Test::Expr(_) => None,
                    _ => Some(c.reader_cols[c.at[i]]),
                };
                // A repeated key marks every key cell in the area (the rule's column and `also`).
                if let Test::Unique(also) = &c.tests[i] {
                    if !also.is_empty() {
                        let key =
                            std::iter::once(mc.expect("a column")).chain(also.iter().copied());
                        let todo: Vec<u32> = key
                            .filter(|k| cols.contains(k) && !done.contains(&Some(*k)))
                            .collect();
                        if !todo.is_empty()
                            && repeats
                                .iter()
                                .find(|x| x.0 == i as u32)
                                .is_some_and(|x| bit(&x.1, id))
                        {
                            for k in todo {
                                out.push(CheckMark {
                                    row: r,
                                    col: Some(k),
                                    check: i as u32,
                                });
                                done.push(Some(k));
                            }
                        }
                        continue;
                    }
                }
                if mc.is_some_and(|m| !cols.contains(&m)) || done.contains(&mc) {
                    continue;
                }
                let ok = match &c.tests[i] {
                    Test::Unique(_) => !repeats
                        .iter()
                        .find(|x| x.0 == i as u32)
                        .is_some_and(|x| bit(&x.1, id)),
                    _ => c.passes(i, &reader, &buf, r + 1, spec),
                };
                if !ok {
                    out.push(CheckMark {
                        row: r,
                        col: mc,
                        check: i as u32,
                    });
                    done.push(mc);
                }
            }
        }
        out
    }
}

/// Check a list of checks without a document (for files loaded from disk).
pub fn check_spec(spec: &CheckSpec) -> Result<(), Error> {
    if spec.checks.len() > MAX_CHECKS {
        return Err(Error::Invalid(format!("at most {MAX_CHECKS} checks")));
    }
    for c in &spec.checks {
        if c.name
            .as_deref()
            .is_some_and(|n| n.chars().count() > MAX_NAME_CHARS)
        {
            return Err(Error::Invalid(format!(
                "a rule name can be at most {MAX_NAME_CHARS} characters"
            )));
        }
        match &c.kind {
            CheckKind::List { values, .. } if values.len() > MAX_LIST => {
                return Err(Error::Invalid(format!(
                    "a list can hold at most {MAX_LIST} values"
                )))
            }
            CheckKind::Unique { also, .. } if also.len() + 1 > MAX_KEY_COLS => {
                return Err(Error::Invalid(format!(
                    "a unique rule can use at most {MAX_KEY_COLS} columns together"
                )))
            }
            CheckKind::Expression { expr }
                if expr.chars().count() > crate::expr::MAX_EXPR_CHARS =>
            {
                return Err(Error::Invalid("an expression is too long".into()))
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::PatternKind;
    use crate::OpenOptions;

    fn doc() -> Document {
        Document::from_bytes(
            b"id,age,email,start,state\r\n1,34,a@x.org,2024-01-05,CA\r\n2,-3,bad,2024-13-01,ca\r\n3,,c@y.com,,NY\r\n3,250.5,d@z.net,2023-12-31,TX\r\n"
                .to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap()
    }

    fn spec(checks: Vec<Check>) -> CheckSpec {
        CheckSpec {
            checks,
            keep_first: true,
            ..Default::default()
        }
    }

    #[test]
    fn iso_dates_are_real_calendar_dates_written_year_month_day() {
        for ok in [
            "2024-02-29",
            "2000-02-29",
            "1999-12-31",
            "2023-02-28",
            "0001-01-01",
        ] {
            assert!(is_iso_date(ok), "{ok}");
        }
        for bad in [
            "2023-02-29",
            "1900-02-29",
            "02/03/2024",
            "2024-2-3",
            "2024-13-01",
            "2024-00-10",
            "2024-04-31",
            "2024-01-32",
            "20240131",
            "2024-01-3",
            " 2024-01-31",
            "2024-01-31 ",
            "２０２４-01-31",
            "",
            "2024/01/31",
        ] {
            assert!(!is_iso_date(bad), "{bad}");
        }
    }

    #[test]
    fn the_iso_date_rule_marks_bad_cells_and_leaves_blanks() {
        let d = Document::from_bytes(
            b"when\r\n2024-02-29\r\n2023-02-29\r\n\r\n02/03/2024\r\n".to_vec(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let s = spec(vec![Check::new(Some(0), CheckKind::DateIso)]);
        let r = d.run_checks(&s, &Progress::new()).unwrap();
        assert_eq!(r.problems, 2);
        assert_eq!(
            r.listed.iter().map(|p| p.row).collect::<Vec<_>>(),
            vec![2, 4]
        );
    }

    #[test]
    fn rules_say_what_they_are_and_old_files_without_names_still_read() {
        let named = Check {
            name: Some("5-digit ZIP".into()),
            ..Check::new(Some(0), CheckKind::Required)
        };
        assert_eq!(named.describe(), "5-digit ZIP");
        assert_eq!(
            Check::new(Some(0), CheckKind::Required).describe(),
            "Required (not blank)"
        );
        assert_eq!(
            Check::new(
                Some(0),
                CheckKind::Number {
                    min: Some(1.0),
                    max: Some(9.0),
                    whole: true
                }
            )
            .describe(),
            "Whole number from 1 to 9"
        );
        assert_eq!(
            Check::new(Some(0), CheckKind::DateIso).describe(),
            "Date written YYYY-MM-DD"
        );
        // A rule written before names existed has no name field.
        let old: Check = serde_json::from_str(r#"{"col":2,"kind":{"type":"required"}}"#).unwrap();
        assert_eq!(old.name, None);
        // And a rule with no name writes none.
        assert!(!serde_json::to_string(&old).unwrap().contains("name"));
        // A name that is too long is refused.
        let long = Check {
            name: Some("x".repeat(MAX_NAME_CHARS + 1)),
            ..old
        };
        assert!(check_spec(&spec(vec![long])).is_err());
    }

    /// A table whose problems are known: the model lists every problem cell by scanning the cells.
    fn lint_table(r: &mut impl FnMut() -> u64, rows: usize) -> String {
        let mut csv = String::from("a,b,c,d\r\n");
        for i in 0..rows {
            let a = if r() % 5 == 0 {
                String::new()
            } else {
                format!("n{}", r() % 7)
            };
            let b = if r() % 4 == 0 {
                "99".to_string()
            } else {
                (r() % 5).to_string()
            };
            let c = if r() % 6 == 0 {
                "bad".to_string()
            } else {
                format!("{:03}", r() % 1000)
            };
            let d = if r() % 9 == 0 {
                "dup".to_string()
            } else {
                format!("u{i}")
            };
            csv.push_str(&format!("{a},{b},{c},{d}\r\n"));
        }
        csv
    }

    fn lint_spec() -> CheckSpec {
        spec(vec![
            Check::new(Some(0), CheckKind::Required),
            Check::new(
                Some(1),
                CheckKind::Number {
                    min: Some(0.0),
                    max: Some(5.0),
                    whole: false,
                },
            ),
            Check::new(
                Some(2),
                CheckKind::Pattern {
                    pattern: crate::find::Pattern {
                        text: "[0-9]{3}".into(),
                        kind: crate::find::PatternKind::Regex,
                        ..Default::default()
                    },
                },
            ),
            Check::new(
                Some(3),
                CheckKind::Unique {
                    match_case: false,
                    also: vec![],
                },
            ),
            // A row rule: the row's b and c must differ in length.
            Check::new(
                None,
                CheckKind::Expression {
                    expr: "LEN([b]) <> LEN([c])".into(),
                },
            ),
        ])
    }

    #[test]
    fn next_problem_equals_a_scan_of_every_cell_both_ways_with_wrap_hidden_columns_and_hidden_rows()
    {
        let mut x = 99u64;
        let mut r = move || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            x >> 33
        };
        for case in 0..12 {
            let rows = 30 + (r() % 40) as usize;
            let csv = lint_table(&mut r, rows);
            let mut d =
                Document::from_bytes(csv.into_bytes(), crate::OpenOptions::default(), Some("csv"))
                    .unwrap();
            if case % 3 == 1 {
                d.hide_rows(2 + r() % 5, 3 + r() % 6).unwrap();
            }
            let s = lint_spec();
            let p = Progress::new();
            let rep = d.run_checks(&s, &p).unwrap();
            let skip: Vec<u32> = if case % 2 == 0 { vec![] } else { vec![2] };
            let first_col = (0..4u32).find(|k| !skip.contains(k)).unwrap();
            // Every problem cell the table has, in reading order, hidden columns left out.
            let mut cells: Vec<(u64, u32)> = d
                .problem_marks(0..d.row_count(), &s, &rep.repeats, &p)
                .unwrap()
                .into_iter()
                .filter_map(|m| match m.col {
                    Some(k) if skip.contains(&k) => None,
                    Some(k) => Some((m.row, k)),
                    None => Some((m.row, first_col)),
                })
                .collect();
            cells.sort_unstable();
            cells.dedup();
            assert!(cells.len() > 3, "case {case} has problems");
            for row in 1..d.row_count() {
                for col in 0..4u32 {
                    for backward in [false, true] {
                        let got = d
                            .next_problem(row, col, backward, &s, &rep.repeats, &skip, &p)
                            .unwrap();
                        let pos = (row, col);
                        let want = if backward {
                            cells
                                .iter()
                                .rev()
                                .find(|c| **c < pos)
                                .map(|c| (*c, false))
                                .or(cells.last().map(|c| (*c, true)))
                        } else {
                            cells
                                .iter()
                                .find(|c| **c > pos)
                                .map(|c| (*c, false))
                                .or(cells.first().map(|c| (*c, true)))
                        };
                        assert_eq!(
                            got.hit.as_ref().map(|j| ((j.row, j.col), got.wrapped)),
                            want,
                            "case {case} from ({row},{col}) backward {backward}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn next_problem_reports_the_hidden_column_problems_it_passed_and_none_when_there_are_none() {
        let d = Document::from_bytes(
            b"a,b\r\nx,\r\n,\r\nx,\r\n".to_vec(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        // Column b is required but blank in every row, and hidden: nothing to jump to there.
        let s = spec(vec![Check::new(Some(1), CheckKind::Required)]);
        let p = Progress::new();
        let rep = d.run_checks(&s, &p).unwrap();
        let r = d
            .next_problem(1, 0, false, &s, &rep.repeats, &[1], &p)
            .unwrap();
        assert_eq!(r.hit, None);
        // Not hidden: found, in the next row.
        let r = d
            .next_problem(1, 0, false, &s, &rep.repeats, &[], &p)
            .unwrap();
        assert_eq!(r.hit.map(|j| (j.row, j.col)), Some((1, 1)));
        // A problem in a hidden column is passed on the way to one in a shown column.
        let s2 = spec(vec![
            Check::new(Some(1), CheckKind::Required),
            Check::new(Some(0), CheckKind::Required),
        ]);
        let rep2 = d.run_checks(&s2, &p).unwrap();
        let r = d
            .next_problem(1, 0, false, &s2, &rep2.repeats, &[1], &p)
            .unwrap();
        assert_eq!(r.hit.map(|j| (j.row, j.col)), Some((2, 0)));
        assert!(r.hidden_skipped >= 1);
    }

    #[test]
    fn next_problem_can_be_cancelled() {
        let mut csv = String::from("a\r\n");
        for _ in 0..200_000 {
            csv.push_str("x\r\n");
        }
        let d = Document::from_bytes(csv.into_bytes(), crate::OpenOptions::default(), Some("csv"))
            .unwrap();
        let s = spec(vec![Check::new(Some(0), CheckKind::Required)]);
        let p = Progress::new();
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            d.next_problem(1, 0, false, &s, &[], &[], &p),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn checks_find_the_right_cells() {
        let d = doc();
        let s = spec(vec![
            Check {
                name: None,
                col: Some(0),
                kind: CheckKind::Unique {
                    match_case: false,
                    also: vec![],
                },
            },
            Check {
                name: None,
                col: Some(1),
                kind: CheckKind::Number {
                    min: Some(0.0),
                    max: Some(120.0),
                    whole: true,
                },
            },
            Check {
                name: None,
                col: Some(1),
                kind: CheckKind::Required,
            },
            Check {
                name: None,
                col: Some(2),
                kind: CheckKind::Pattern {
                    pattern: Pattern {
                        text: r"[^@\s]+@[^@\s]+\.[a-z]+".into(),
                        kind: PatternKind::Regex,
                        ..Default::default()
                    },
                },
            },
            Check {
                name: None,
                col: Some(3),
                kind: CheckKind::Date {
                    min: Some("2024-01-01".into()),
                    max: None,
                },
            },
            Check {
                name: None,
                col: Some(4),
                kind: CheckKind::List {
                    values: vec!["CA".into(), "NY".into()],
                    match_case: true,
                },
            },
            Check {
                name: None,
                col: None,
                kind: CheckKind::Expression {
                    expr: "OR(ISBLANK([start]), [age] < 100)".into(),
                },
            },
        ]);
        let r = d.run_checks(&s, &Progress::new()).unwrap();
        assert_eq!(r.checked, 4);
        // unique: rows 3 and 4 share id 3; number: -3, 250.5; required: row 3;
        // pattern: "bad"; date: 2024-13-01, 2023-12-31; list: ca, TX;
        // expression: row 4 (age 250.5 with a start date).
        assert_eq!(r.per_check, vec![2, 2, 1, 1, 2, 2, 1]);
        assert_eq!(r.rows, 3);
        assert_eq!(r.problems, 11);
        assert_eq!(r.listed[0].row, 2);
        assert_eq!(r.listed[0].col, Some(1));
        assert_eq!(r.listed[0].value, "-3");
        let marks = d.check_marks(0, 10, 0, 10, &s, &r.repeats).unwrap();
        assert!(marks.contains(&CheckMark {
            row: 3,
            col: Some(0),
            check: 0
        }));
        assert!(marks.contains(&CheckMark {
            row: 4,
            col: None,
            check: 6
        }));
        assert!(!marks.iter().any(|m| m.row == 0));
        // Show only rows with problems: three of the four data rows.
        let mut d = d;
        let plan = d.plan_show_problems(&r, true, false).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 4);
    }

    #[test]
    fn show_problems_hides_good_rows() {
        let mut d = Document::from_bytes(
            b"n\r\n1\r\nx\r\n2\r\n\r\ny\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let s = spec(vec![Check {
            name: None,
            col: Some(0),
            kind: CheckKind::Number {
                min: None,
                max: None,
                whole: false,
            },
        }]);
        let r = d.run_checks(&s, &Progress::new()).unwrap();
        assert_eq!(r.rows, 2);
        let plan = d.plan_show_problems(&r, true, false).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 3);
        assert_eq!(d.row_values(1).unwrap(), ["x"]);
        assert_eq!(d.row_values(2).unwrap(), ["y"]);
        d.clear_filter();
        assert_eq!(d.row_count(), 6);
    }

    #[test]
    fn bad_checks_are_refused() {
        let d = doc();
        let bad = |k: CheckKind, col: Option<u32>| {
            d.run_checks(
                &spec(vec![Check {
                    name: None,
                    col,
                    kind: k,
                }]),
                &Progress::new(),
            )
            .is_err()
        };
        assert!(bad(CheckKind::Required, None));
        assert!(bad(CheckKind::Required, Some(99)));
        assert!(bad(
            CheckKind::Date {
                min: Some("soon".into()),
                max: None
            },
            Some(3)
        ));
        assert!(bad(
            CheckKind::Expression {
                expr: "[nope] > 1".into()
            },
            None
        ));
        assert!(bad(
            CheckKind::Number {
                min: Some(f64::NAN),
                max: None,
                whole: false
            },
            Some(1)
        ));
        let many = spec(
            (0..=MAX_CHECKS)
                .map(|_| Check {
                    name: None,
                    col: Some(0),
                    kind: CheckKind::Required,
                })
                .collect(),
        );
        assert!(d.run_checks(&many, &Progress::new()).is_err());
        assert!(check_spec(&many).is_err());
    }
}

#[cfg(test)]
mod field_tests {
    use super::*;
    use crate::find::PatternKind;
    use crate::OpenOptions;

    fn zip_spec() -> CheckSpec {
        CheckSpec {
            checks: vec![Check {
                name: None,
                col: Some(1),
                kind: CheckKind::Pattern {
                    pattern: Pattern {
                        text: r"^\d{5}$".into(),
                        kind: PatternKind::Regex,
                        match_case: false,
                        whole_cell: false,
                        ..Default::default()
                    },
                },
            }],
            keep_first: true,
            ..Default::default()
        }
    }

    #[test]
    fn sort_filter_and_result_column() {
        let mut d = Document::from_bytes(
            b"name,zip\r\nAnn,37402\r\nBo,3740\r\nCy,37403\r\nDee,x\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let p = Progress::new();
        let spec = zip_spec();
        let rep = d.run_checks(&spec, &p).unwrap();
        assert_eq!(rep.rows, 2);
        // Failing rows first, in their order; title stays.
        let plan = d.plan_sort_problems(&rep, true).unwrap();
        assert!(plan.changes());
        d.apply_sort(plan).unwrap();
        let names: Vec<String> = (0..5).map(|r| d.cell(r, 0).unwrap()).collect();
        assert_eq!(names, ["name", "Bo", "Dee", "Ann", "Cy"]);
        // Already first: no change.
        let rep = d.run_checks(&spec, &p).unwrap();
        assert!(!d.plan_sort_problems(&rep, true).unwrap().changes());
        // Passing rows only.
        let plan = d.plan_show_problems(&rep, true, true).unwrap();
        assert_eq!(plan.rows, 2);
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 3);
        assert_eq!(d.cell(1, 0).unwrap(), "Ann");
        d.clear_filter();
        // The result column.
        let plan = d.plan_check_column(&spec, &rep, "", &p).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(d.cell(0, 2).unwrap(), "Check result");
        assert_eq!(d.cell(1, 2).unwrap(), "Fails: zip: pattern");
        assert_eq!(d.cell(3, 2).unwrap(), "OK");
    }

    fn spec(checks: Vec<Check>) -> CheckSpec {
        CheckSpec {
            checks,
            keep_first: true,
            ..Default::default()
        }
    }

    fn uniq(col: u32, also: Vec<u32>, match_case: bool) -> Check {
        Check::new(Some(col), CheckKind::Unique { match_case, also })
    }

    fn key_doc() -> Document {
        Document::from_bytes(
            b"first,last,city,n\r\nAnn,Lee,Rome,1\r\nann,Lee,Rome,2\r\nAnn,Lee,Oslo,3\r\nAnn,Lee,Rome,4\r\n,,,5\r\n,,,6\r\nBo,,,7\r\nBo,,,8\r\n"
                .to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap()
    }

    #[test]
    fn a_unique_rule_over_several_columns_finds_rows_that_repeat_them_together() {
        let d = key_doc();
        let p = Progress::new();
        // first + last alone: Ann/ann Lee x4 (no case) and Bo x2 repeat; the all-blank rows pass.
        let r = d
            .run_checks(&spec(vec![uniq(0, vec![1], false)]), &p)
            .unwrap();
        assert_eq!(r.per_check, vec![6]);
        // With the city as well: Ann/ann Lee Rome x3 and Bo x2 (blank parts compare equal).
        let r = d
            .run_checks(&spec(vec![uniq(0, vec![1, 2], false)]), &p)
            .unwrap();
        assert_eq!(r.per_check, vec![5]);
        let rows: Vec<u64> = r.listed.iter().map(|x| x.row).collect();
        assert_eq!(rows, vec![1, 2, 4, 7, 8]);
        assert_eq!(r.listed[0].value, "Ann | Lee | Rome");
        assert_eq!(r.listed[0].col, Some(0));
        // Matching case: ann is not Ann.
        let r = d
            .run_checks(&spec(vec![uniq(0, vec![1, 2], true)]), &p)
            .unwrap();
        assert_eq!(r.per_check, vec![4]);
        // A one-column rule is unchanged: first only, no case.
        let r = d
            .run_checks(&spec(vec![uniq(0, vec![], false)]), &p)
            .unwrap();
        assert_eq!(r.per_check, vec![6]);
        assert_eq!(r.listed[0].value, "Ann");
    }

    #[test]
    fn a_repeated_key_marks_every_key_cell_and_nothing_else() {
        let d = key_doc();
        let s = spec(vec![uniq(0, vec![2], false)]);
        let r = d.run_checks(&s, &Progress::new()).unwrap();
        let marks = d.check_marks(0, 20, 0, 4, &s, &r.repeats).unwrap();
        for col in [0, 2] {
            assert!(marks.contains(&CheckMark {
                row: 1,
                col: Some(col),
                check: 0
            }));
        }
        assert!(!marks.iter().any(|m| m.col == Some(1) || m.col == Some(3)));
        assert!(!marks.iter().any(|m| m.row == 3), "Oslo is not repeated");
        // An area that holds only the other key column still marks it.
        let marks = d.check_marks(0, 20, 2, 1, &s, &r.repeats).unwrap();
        assert!(marks.iter().all(|m| m.col == Some(2)) && !marks.is_empty());
        // Go To Special sees the same cells.
        let all = d
            .problem_marks(0..d.row_count(), &s, &r.repeats, &Progress::new())
            .unwrap();
        assert_eq!(all.len(), marks.len() * 2);
    }

    #[test]
    fn a_unique_key_with_too_many_or_repeated_columns_is_refused() {
        let d = key_doc();
        let p = Progress::new();
        for also in [vec![0], vec![1, 1], vec![9]] {
            assert!(
                d.run_checks(&spec(vec![uniq(0, also.clone(), false)]), &p)
                    .is_err(),
                "{also:?}"
            );
        }
        let wide = Document::from_bytes(
            b"a,b,c,d,e,f,g,h,i\r\n1,2,3,4,5,6,7,8,9\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        assert!(wide
            .run_checks(&spec(vec![uniq(0, (1..8).collect(), false)]), &p)
            .is_ok());
        let nine = spec(vec![uniq(0, (1..9).collect(), false)]);
        assert!(wide.run_checks(&nine, &p).is_err());
        assert!(check_spec(&nine).is_err());
        assert_eq!(
            uniq(0, vec![1, 2], false).describe(),
            "Unique together with 2 other columns (no repeats)"
        );
        // Stored without `also` when there is none (older files and views read the same).
        let json = serde_json::to_string(&uniq(0, vec![], false)).unwrap();
        assert!(!json.contains("also"), "{json}");
    }
}

#[cfg(test)]
mod repeat_props {
    use super::*;
    use crate::OpenOptions;
    use proptest::prelude::*;

    const VALUES: [&str; 6] = ["", " ", "a", "A", " a", "b"];

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(300))]
        /// Grouping on the first column first gives exactly what grouping on the whole key gives.
        #[test]
        fn first_column_first_equals_the_whole_key(
            cells in proptest::collection::vec((0usize..6, 0usize..6, 0usize..6), 0..40),
            width in 1usize..4,
            match_case: bool,
        ) {
            let mut text = String::from("x,y,z\r\n");
            for (a, b, c) in &cells {
                text.push_str(&format!("{},{},{}\r\n", VALUES[*a], VALUES[*b], VALUES[*c]));
            }
            let d = Document::from_bytes(text.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
            let p = Progress::new();
            let rows = d.data_rows(true);
            let key: Vec<u32> = (0..width as u32).collect();
            let fast = d.repeated_keys(rows.clone(), &key, match_case, &p).unwrap();
            let leader = d.group_rows(rows, &key, match_case, true, &p).unwrap();
            let mut full = vec![false; leader.len()];
            for (k, &l) in leader.iter().enumerate() {
                if l as usize != k {
                    full[l as usize] = true;
                    full[k] = true;
                }
            }
            prop_assert_eq!(fast, full);
        }
    }
}
