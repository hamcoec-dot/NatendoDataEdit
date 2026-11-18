//! Filters: rules per column that decide which rows show. A filter never
//! changes data; hidden rows are saved as they are. Which rows show is fixed
//! when the filter is applied (see `document::View`).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress, View};
use crate::find::{Matcher, Pattern};
use crate::value::{self, ReadOptions};
use crate::Error;

/// Most distinct values listed for a column's value list.
pub const MAX_LIST_VALUES: usize = 10_000;
/// Longest value listed (longer ones need a text rule).
pub const MAX_LIST_VALUE_BYTES: usize = 1024;
/// Most columns with rules at once.
pub const MAX_RULES: usize = 1024;
/// Rows per parallel task.
const CHUNK: usize = 1 << 15;

/// Comparison for numbers and dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl CmpOp {
    fn test(self, a: f64, b: f64) -> bool {
        match self {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
        }
    }
}

/// Comparison for text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOp {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    BeginsWith,
    EndsWith,
}

/// One condition on a cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    Text {
        op: TextOp,
        text: String,
        #[serde(default)]
        match_case: bool,
    },
    /// Cells that are not numbers never match.
    Number {
        op: CmpOp,
        value: f64,
    },
    NumberBetween {
        low: f64,
        high: f64,
    },
    /// Compared by calendar day; cells that are not dates never match.
    Date {
        op: CmpOp,
        value: String,
    },
    DateBetween {
        low: String,
        high: String,
    },
    Blank,
    NotBlank,
    /// Wildcard or regular expression (same rules as find).
    Pattern {
        pattern: Pattern,
        #[serde(default)]
        negate: bool,
    },
    /// The `count` largest numbers (or smallest, with `bottom`), or that
    /// percent of the column's numbers with `percent`. Ties are all kept.
    /// Worked out over the column's rows when the filter is applied.
    Top {
        count: f64,
        #[serde(default)]
        percent: bool,
        #[serde(default)]
        bottom: bool,
    },
}

/// What a column's cells must be for the row to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Rule {
    /// One of these exact values (or blank, with `blanks`).
    Values {
        values: Vec<String>,
        #[serde(default)]
        blanks: bool,
    },
    /// One or two conditions, joined by AND (or OR with `or`).
    Conditions {
        first: Condition,
        #[serde(default)]
        second: Option<Condition>,
        #[serde(default)]
        or: bool,
    },
}

/// A rule on one column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnFilter {
    pub col: u32,
    pub rule: Rule,
}

/// A whole filter: every column's rule must pass (AND).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterSpec {
    pub columns: Vec<ColumnFilter>,
    /// The first row (column titles) always shows.
    pub keep_first: bool,
    pub read: ReadOptions,
}

impl FilterSpec {
    /// The same rules with columns renumbered by `f` (rules whose column
    /// maps to `None` are dropped).
    pub fn map_cols(&self, f: impl Fn(u32) -> Option<u32>) -> FilterSpec {
        FilterSpec {
            columns: self
                .columns
                .iter()
                .filter_map(|c| {
                    f(c.col).map(|col| ColumnFilter {
                        col,
                        rule: c.rule.clone(),
                    })
                })
                .collect(),
            keep_first: self.keep_first,
            read: self.read,
        }
    }
}

/// A column's distinct values for the header menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueCount {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueList {
    /// Sorted (numbers by value, dates by date, other text naturally).
    pub values: Vec<ValueCount>,
    pub blanks: u64,
    /// False when there were more values than listed (counts may then be
    /// low for some values).
    pub complete: bool,
}

/// A computed filter, applied with [`Document::apply_filter`].
pub struct FilterPlan {
    revision: u64,
    view: Option<View>,
    /// Rows the filter shows (with the title row, if kept).
    pub shown: u64,
}

enum Cond {
    Match(Matcher, bool),
    Num(CmpOp, f64),
    NumBetween(f64, f64),
    Date(CmpOp, f64),
    DateBetween(f64, f64),
    Blank(bool),
}

enum Test {
    Values(HashSet<String>, bool),
    Conds(Cond, Option<Cond>, bool),
}

struct Compiled {
    /// Physical columns read, sorted; tests refer to slots in it.
    wanted: Vec<u32>,
    tests: Vec<(usize, Test)>,
    read: ReadOptions,
}

fn day_of(text: &str, read: ReadOptions) -> Result<f64, Error> {
    value::parse_date(text, read)
        .map(f64::floor)
        .ok_or_else(|| Error::BadPattern(format!("\"{text}\" is not a date")))
}

fn compile_cond(c: &Condition, read: ReadOptions) -> Result<Cond, Error> {
    Ok(match c {
        Condition::Text {
            op,
            text,
            match_case,
        } => {
            let (start, end, negate) = match op {
                TextOp::Equals => (true, true, false),
                TextOp::NotEquals => (true, true, true),
                TextOp::Contains => (false, false, false),
                TextOp::NotContains => (false, false, true),
                TextOp::BeginsWith => (true, false, false),
                TextOp::EndsWith => (false, true, false),
            };
            Cond::Match(Matcher::anchored(text, *match_case, start, end)?, negate)
        }
        Condition::Number { op, value } => Cond::Num(*op, *value),
        Condition::NumberBetween { low, high } => Cond::NumBetween(low.min(*high), high.max(*low)),
        Condition::Date { op, value } => Cond::Date(*op, day_of(value, read)?),
        Condition::DateBetween { low, high } => {
            let (a, b) = (day_of(low, read)?, day_of(high, read)?);
            Cond::DateBetween(a.min(b), a.max(b))
        }
        Condition::Blank => Cond::Blank(true),
        Condition::NotBlank => Cond::Blank(false),
        Condition::Pattern { pattern, negate } => Cond::Match(Matcher::new(pattern)?, *negate),
        Condition::Top { .. } => {
            return Err(Error::BadPattern(
                "largest and smallest values work in filters only".into(),
            ))
        }
    })
}

/// Most rows "largest / smallest" can keep.
pub const MAX_TOP: f64 = 1e9;

impl Cond {
    fn test(&self, s: &str, read: ReadOptions) -> bool {
        match self {
            Cond::Match(m, negate) => m.is_match(s) != *negate,
            Cond::Num(op, v) => value::parse_number(s, read).is_some_and(|x| op.test(x, *v)),
            Cond::NumBetween(a, b) => {
                value::parse_number(s, read).is_some_and(|x| x >= *a && x <= *b)
            }
            Cond::Date(op, v) => value::parse_date(s, read).is_some_and(|x| op.test(x.floor(), *v)),
            Cond::DateBetween(a, b) => value::parse_date(s, read)
                .map(f64::floor)
                .is_some_and(|x| x >= *a && x <= *b),
            Cond::Blank(want) => s.is_empty() == *want,
        }
    }
}

impl Test {
    fn test(&self, s: &str, read: ReadOptions) -> bool {
        match self {
            Test::Values(set, blanks) => {
                if s.is_empty() {
                    *blanks
                } else {
                    set.contains(s)
                }
            }
            Test::Conds(a, None, _) => a.test(s, read),
            Test::Conds(a, Some(b), or) => {
                if *or {
                    a.test(s, read) || b.test(s, read)
                } else {
                    a.test(s, read) && b.test(s, read)
                }
            }
        }
    }
}

/// One rule compiled to test single values (highlight rules).
pub struct RuleMatcher {
    test: Test,
    read: ReadOptions,
}

impl RuleMatcher {
    pub fn new(rule: &Rule, read: ReadOptions) -> Result<RuleMatcher, Error> {
        let test = match rule {
            Rule::Values { values, blanks } => {
                if values.len() > 100_000 {
                    return Err(Error::BadPattern(
                        "a rule can list at most 100,000 values".into(),
                    ));
                }
                Test::Values(values.iter().cloned().collect(), *blanks)
            }
            Rule::Conditions { first, second, or } => Test::Conds(
                compile_cond(first, read)?,
                second.as_ref().map(|s| compile_cond(s, read)).transpose()?,
                *or,
            ),
        };
        Ok(RuleMatcher { test, read })
    }

    pub fn matches(&self, s: &str) -> bool {
        self.test.test(s, self.read)
    }
}

impl Compiled {
    /// Rules by physical column.
    fn new(rules: &[ColumnFilter], read: ReadOptions) -> Result<Compiled, Error> {
        let mut wanted: Vec<u32> = rules.iter().map(|c| c.col).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let mut tests = Vec::with_capacity(rules.len());
        for c in rules {
            let slot = wanted.binary_search(&c.col).expect("listed");
            let t = match &c.rule {
                Rule::Values { values, blanks } => {
                    Test::Values(values.iter().cloned().collect(), *blanks)
                }
                Rule::Conditions { first, second, or } => Test::Conds(
                    compile_cond(first, read)?,
                    second.as_ref().map(|s| compile_cond(s, read)).transpose()?,
                    *or,
                ),
            };
            tests.push((slot, t));
        }
        Ok(Compiled {
            wanted,
            tests,
            read,
        })
    }

    fn passes(&self, cells: &[Cow<'_, str>]) -> bool {
        self.tests
            .iter()
            .all(|(slot, t)| t.test(&cells[*slot], self.read))
    }
}

impl Document {
    /// The spec with logical columns turned into physical ones (rules on
    /// columns past the map's end are dropped: those cells are all empty
    /// and new, so the user cannot have meant them).
    fn to_physical_spec(&self, spec: &FilterSpec) -> Result<FilterSpec, Error> {
        if spec.columns.len() > MAX_RULES {
            return Err(Error::BadPattern("too many filter rules".into()));
        }
        Ok(spec.map_cols(|c| (c < self.col_count()).then(|| self.phys(c)).flatten()))
    }

    /// The spec (physical columns) with every "largest / smallest" condition
    /// turned into a number comparison with the cut-off value, found over
    /// the column's rows (every row but a kept title row).
    fn resolve_top(&self, spec: &FilterSpec, progress: &Progress) -> Result<FilterSpec, Error> {
        let mut out = spec.clone();
        let total = self.total_rows();
        let first = u64::from(spec.keep_first && total > 0);
        for c in &mut out.columns {
            let Rule::Conditions {
                first: a,
                second: b,
                ..
            } = &mut c.rule
            else {
                continue;
            };
            for cond in std::iter::once(a).chain(b.as_mut()) {
                let Condition::Top {
                    count,
                    percent,
                    bottom,
                } = *cond
                else {
                    continue;
                };
                if !count.is_finite()
                    || count < 0.0
                    || count > MAX_TOP
                    || (percent && count > 100.0)
                {
                    return Err(Error::BadPattern(
                        "choose how many: up to 1,000,000,000 rows or 100 percent".into(),
                    ));
                }
                let col = c.col;
                let read = spec.read;
                let chunks = ((total - first) as usize).div_ceil(CHUNK);
                progress.start(total - first);
                let parts: Vec<Vec<f64>> = (0..chunks)
                    .into_par_iter()
                    .map(|k| -> Result<Vec<f64>, Error> {
                        let s = first as usize + k * CHUNK;
                        let e = (s + CHUNK).min(total as usize);
                        let mut out = Vec::new();
                        let mut buf = Vec::new();
                        for pos in s..e {
                            let id = self.doc_id_at(pos as u64).expect("in range");
                            self.read_cells(id, &[col], &mut buf);
                            if let Some(v) = value::parse_number(&buf[0], read) {
                                out.push(v);
                            }
                        }
                        progress.step((e - s) as u64)?;
                        Ok(out)
                    })
                    .collect::<Result<_, _>>()?;
                let mut nums: Vec<f64> = parts.into_iter().flatten().collect();
                let want = if percent {
                    (nums.len() as f64 * count / 100.0).ceil()
                } else {
                    count.floor()
                } as usize;
                *cond = if want == 0 || nums.is_empty() {
                    // Nothing qualifies.
                    Condition::Number {
                        op: CmpOp::Gt,
                        value: f64::INFINITY,
                    }
                } else {
                    let k = want.min(nums.len()) - 1;
                    let (_, v, _) = if bottom {
                        nums.select_nth_unstable_by(k, |x, y| x.total_cmp(y))
                    } else {
                        nums.select_nth_unstable_by(k, |x, y| y.total_cmp(x))
                    };
                    Condition::Number {
                        op: if bottom { CmpOp::Le } else { CmpOp::Ge },
                        value: *v,
                    }
                };
            }
        }
        Ok(out)
    }

    /// Work out which rows a filter shows (nothing changes yet). A spec
    /// without rules turns the filter off.
    pub fn plan_filter(&self, spec: &FilterSpec, progress: &Progress) -> Result<FilterPlan, Error> {
        self.ready_for_bulk()?;
        let spec = self.to_physical_spec(spec)?;
        let total = self.total_rows();
        if spec.columns.is_empty() {
            return Ok(FilterPlan {
                revision: self.revision(),
                view: None,
                shown: total,
            });
        }
        let resolved = self.resolve_top(&spec, progress)?;
        let compiled = Compiled::new(&resolved.columns, spec.read)?;
        progress.start(total);
        let first = usize::from(spec.keep_first && total > 0);
        let chunks = (total as usize).div_ceil(CHUNK);
        let parts: Vec<Vec<u32>> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Vec<u32>, Error> {
                let s = c * CHUNK;
                let e = (s + CHUNK).min(total as usize);
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for pos in s.max(first)..e {
                    let id = self.doc_id_at(pos as u64).expect("in range");
                    self.read_cells(id, &compiled.wanted, &mut buf);
                    if compiled.passes(&buf) {
                        out.push(id);
                    }
                }
                progress.step((e - s) as u64)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let base = self.next_row_id();
        let mut bits = vec![0u64; (base as usize).div_ceil(64)];
        let mut shown = 0u64;
        let mut set = |id: u32| {
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        };
        if first == 1 {
            set(self.doc_id_at(0).expect("a first row"));
            shown += 1;
        }
        for ids in parts {
            shown += ids.len() as u64;
            ids.into_iter().for_each(&mut set);
        }
        Ok(FilterPlan {
            revision: self.revision(),
            view: Some(View::new(spec, bits, base)),
            shown,
        })
    }

    /// Turn a computed filter on. Fails with [`Error::Changed`] if the
    /// document changed since it was computed.
    pub fn apply_filter(&mut self, plan: FilterPlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        self.set_view(plan.view);
        Ok(())
    }

    /// Compute and apply a filter.
    pub fn set_filter(&mut self, spec: &FilterSpec) -> Result<u64, Error> {
        let plan = self.plan_filter(spec, &Progress::new())?;
        let shown = plan.shown;
        self.apply_filter(plan)?;
        Ok(shown)
    }

    /// Show every row again.
    pub fn clear_filter(&mut self) {
        if self.is_filtered() {
            self.set_view(None);
        }
    }

    /// The filter's rules by logical column (rules on deleted columns left
    /// out), or `None` without a filter.
    pub fn filter_spec(&self) -> Option<FilterSpec> {
        let v = self.view().filter(|v| v.filters())?;
        Some(v.spec.map_cols(|p| match self.col_map() {
            Some(m) => m.iter().position(|&x| x == p).map(|i| i as u32),
            None => Some(p),
        }))
    }

    /// The filter has a rule on a column that was deleted since: its rows
    /// stay hidden with no column to show the rule on, so the UI must say
    /// so (undo brings the column and its funnel back).
    pub fn filter_on_deleted_column(&self) -> bool {
        let Some(v) = self.view() else { return false };
        let Some(m) = self.col_map() else {
            return false;
        };
        v.spec.columns.iter().any(|c| !m.contains(&c.col))
    }

    /// Logical columns that have a rule.
    pub fn filtered_columns(&self) -> Vec<u32> {
        self.filter_spec()
            .map(|s| s.columns.iter().map(|c| c.col).collect())
            .unwrap_or_default()
    }

    /// Distinct values of column `col` among the rows that pass every rule
    /// of `spec` except the one on `col`, for the header menu. With
    /// `search`, only values containing it (ignoring case) are listed.
    pub fn filter_values(
        &self,
        col: u32,
        spec: &FilterSpec,
        search: Option<&str>,
        progress: &Progress,
    ) -> Result<ValueList, Error> {
        self.ready_for_bulk()?;
        let mut others = spec.clone();
        others.columns.retain(|c| c.col != col);
        let others = self.to_physical_spec(&others)?;
        let Some(target) = (col < self.col_count()).then(|| self.phys(col)).flatten() else {
            return Ok(ValueList {
                values: Vec::new(),
                blanks: self.total_rows(),
                complete: true,
            });
        };
        let mut rules = others.columns.clone();
        // The target column is read too (its slot found below).
        let compiled = Compiled::new(&rules, others.read)?;
        rules.clear();
        let mut wanted = compiled.wanted.clone();
        if let Err(i) = wanted.binary_search(&target) {
            wanted.insert(i, target);
        }
        let tslot = wanted.binary_search(&target).expect("inserted");
        // Tests refer to slots of `compiled.wanted`; remap to `wanted`.
        let remap: Vec<usize> = compiled
            .wanted
            .iter()
            .map(|p| wanted.binary_search(p).expect("listed"))
            .collect();
        let needle = search.map(str::to_lowercase).filter(|s| !s.is_empty());
        let total = self.total_rows();
        progress.start(total);
        let first = usize::from(spec.keep_first && total > 0);
        let chunks = (total as usize).div_ceil(CHUNK);
        type Part = (HashMap<String, u64>, u64, bool);
        let parts: Vec<Part> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Part, Error> {
                let s = c * CHUNK;
                let e = (s + CHUNK).min(total as usize);
                let mut map: HashMap<String, u64> = HashMap::new();
                let (mut blanks, mut complete) = (0u64, true);
                let mut buf = Vec::new();
                for pos in s.max(first)..e {
                    let id = self.doc_id_at(pos as u64).expect("in range");
                    self.read_cells(id, &wanted, &mut buf);
                    let pass = compiled
                        .tests
                        .iter()
                        .all(|(slot, t)| t.test(&buf[remap[*slot]], compiled.read));
                    if !pass {
                        continue;
                    }
                    let v = &buf[tslot];
                    if v.is_empty() {
                        blanks += 1;
                        continue;
                    }
                    if let Some(n) = &needle {
                        if !v.to_lowercase().contains(n.as_str()) {
                            continue;
                        }
                    }
                    if let Some(n) = map.get_mut(v.as_ref()) {
                        *n += 1;
                    } else if map.len() < MAX_LIST_VALUES && v.len() <= MAX_LIST_VALUE_BYTES {
                        map.insert(v.to_string(), 1);
                    } else {
                        complete = false;
                    }
                }
                progress.step((e - s) as u64)?;
                Ok((map, blanks, complete))
            })
            .collect::<Result<_, _>>()?;
        let mut all: HashMap<String, u64> = HashMap::new();
        let (mut blanks, mut complete) = (0, true);
        for (m, b, c) in parts {
            blanks += b;
            complete &= c;
            for (k, n) in m {
                if let Some(x) = all.get_mut(&k) {
                    *x += n;
                } else if all.len() < MAX_LIST_VALUES {
                    all.insert(k, n);
                } else {
                    complete = false;
                }
            }
        }
        let mut values: Vec<ValueCount> = all
            .into_iter()
            .map(|(value, count)| ValueCount { value, count })
            .collect();
        sort_values(&mut values, spec.read);
        Ok(ValueList {
            values,
            blanks,
            complete,
        })
    }
}

/// What a filter-panel card shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacetKind {
    /// Each value with its count (most common first).
    #[default]
    Values,
    /// A histogram of the numbers.
    Numbers,
    /// A histogram of the dates (days since 1970).
    Dates,
}

/// One card of the filter panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FacetCard {
    pub col: u32,
    pub kind: FacetKind,
    /// Numbers and dates: the range the bars cover (the column's smallest
    /// and largest over every row, so the bars stay put as filters change).
    pub range: Option<(f64, f64)>,
    /// Values: only values containing this (any case).
    pub search: Option<String>,
}

/// Bars in a card's histogram.
pub const FACET_BINS: usize = 24;
/// Values listed on a card at most (most common first).
pub const FACET_VALUES: usize = 1_000;

/// One card's counts: over the rows every other rule lets through, so a
/// card shows what picking in it would leave.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Facet {
    pub col: u32,
    /// Values: most common first, at most [`FACET_VALUES`].
    pub values: Vec<ValueCount>,
    /// Different values seen (capped at [`MAX_LIST_VALUES`]).
    pub distinct: u64,
    pub complete: bool,
    /// Numbers and dates: counts per bar over `range`.
    pub bins: Vec<u64>,
    /// Numbers and dates: the range the bars cover (as given, or the
    /// column's smallest and largest over every row when not given).
    pub range: Option<(f64, f64)>,
    /// Cells that are not numbers (or dates), or outside the range.
    pub other: u64,
    pub blanks: u64,
    /// Rows counted for this card.
    pub rows: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Facets {
    pub cards: Vec<Facet>,
    /// Rows the whole filter lets through (not the title row).
    pub shown: u64,
}

/// One card's running counts.
#[derive(Default)]
struct FacetAcc {
    map: HashMap<String, u64>,
    over: bool,
    bins: Vec<u64>,
    other: u64,
    blanks: u64,
    rows: u64,
}

impl Document {
    /// Cards for numbers or dates without a range get the column's smallest
    /// and largest over every data row (one parallel pass for all of them).
    fn facet_ranges(
        &self,
        cards: &[FacetCard],
        keep_first: bool,
        read: ReadOptions,
    ) -> Result<Vec<FacetCard>, Error> {
        let mut out = cards.to_vec();
        let need: Vec<usize> = (0..out.len())
            .filter(|&k| out[k].kind != FacetKind::Values && out[k].range.is_none())
            .filter(|&k| out[k].col < self.col_count())
            .collect();
        if need.is_empty() {
            return Ok(out);
        }
        let cols: Vec<u32> = need.iter().map(|&k| out[k].col).collect();
        let reader = crate::ops::Cols::new(self, &cols);
        let total = self.total_rows();
        let first = u64::from(keep_first && total > 0);
        let n = need.len();
        let chunks = ((total - first) as usize).div_ceil(CHUNK);
        let lim = (0..chunks)
            .into_par_iter()
            .map(|c| {
                let s = first + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(total);
                let mut lim = vec![(f64::INFINITY, f64::NEG_INFINITY); n];
                let mut buf = Vec::new();
                for p in s..e {
                    let Some(id) = self.doc_id_at(p) else {
                        continue;
                    };
                    reader.read(self, id, &mut buf);
                    for (i, &k) in need.iter().enumerate() {
                        let v = reader.get(&buf, i);
                        let x = if out[k].kind == FacetKind::Numbers {
                            value::parse_number(v, read)
                        } else {
                            value::parse_date(v, read).map(f64::floor)
                        };
                        if let Some(x) = x {
                            lim[i].0 = lim[i].0.min(x);
                            lim[i].1 = lim[i].1.max(x);
                        }
                    }
                }
                lim
            })
            .reduce(
                || vec![(f64::INFINITY, f64::NEG_INFINITY); n],
                |mut a, b| {
                    for (x, y) in a.iter_mut().zip(b) {
                        x.0 = x.0.min(y.0);
                        x.1 = x.1.max(y.1);
                    }
                    a
                },
            );
        for (i, &k) in need.iter().enumerate() {
            let (lo, hi) = lim[i];
            out[k].range = (lo <= hi).then_some((lo, hi));
        }
        Ok(out)
    }
}

fn facet_accs(cards: &[FacetCard]) -> Vec<FacetAcc> {
    cards
        .iter()
        .map(|c| FacetAcc {
            bins: if c.kind == FacetKind::Values {
                Vec::new()
            } else {
                vec![0; FACET_BINS]
            },
            ..Default::default()
        })
        .collect()
}

impl Document {
    /// Counts for every card of the filter panel in one pass over the rows
    /// (in parallel): a row counts for a card when it passes every rule, or
    /// fails only the rule on the card's own column.
    pub fn facet_counts(
        &self,
        cards: &[FacetCard],
        spec: &FilterSpec,
        progress: &Progress,
    ) -> Result<Facets, Error> {
        self.ready_for_bulk()?;
        if cards.len() > 32 {
            return Err(Error::Invalid(
                "a filter panel holds at most 32 cards".into(),
            ));
        }
        let cards = &self.facet_ranges(cards, spec.keep_first, spec.read)?;
        let spec = self.to_physical_spec(spec)?;
        let compiled = Compiled::new(&spec.columns, spec.read)?;
        let read = spec.read;
        // Each card's physical column (None: past the map, all blank).
        let card_phys: Vec<Option<u32>> = cards
            .iter()
            .map(|c| {
                (c.col < self.col_count())
                    .then(|| self.phys(c.col))
                    .flatten()
            })
            .collect();
        let mut wanted = compiled.wanted.clone();
        for p in card_phys.iter().flatten() {
            if let Err(i) = wanted.binary_search(p) {
                wanted.insert(i, *p);
            }
        }
        let remap: Vec<usize> = compiled
            .wanted
            .iter()
            .map(|p| wanted.binary_search(p).expect("listed"))
            .collect();
        let rule_col: Vec<u32> = compiled
            .tests
            .iter()
            .map(|(slot, _)| compiled.wanted[*slot])
            .collect();
        let card_slot: Vec<Option<usize>> = card_phys
            .iter()
            .map(|p| p.map(|p| wanted.binary_search(&p).expect("listed")))
            .collect();
        let needles: Vec<Option<String>> = cards
            .iter()
            .map(|c| {
                c.search
                    .as_deref()
                    .map(str::to_lowercase)
                    .filter(|s| !s.is_empty())
            })
            .collect();
        let total = self.total_rows();
        progress.start(total);
        let first = usize::from(spec.keep_first && total > 0);
        let chunks = (total as usize).div_ceil(CHUNK);
        let bin_of = |c: &FacetCard, x: f64| -> Option<usize> {
            let (lo, hi) = c.range?;
            if !(x >= lo && x <= hi) {
                return None;
            }
            Some(if hi > lo {
                (((x - lo) / (hi - lo) * FACET_BINS as f64) as usize).min(FACET_BINS - 1)
            } else {
                0
            })
        };
        let parts: Vec<(Vec<FacetAcc>, u64)> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<(Vec<FacetAcc>, u64), Error> {
                let s = c * CHUNK;
                let e = (s + CHUNK).min(total as usize);
                let mut acc = facet_accs(cards);
                let mut shown = 0u64;
                let mut buf = Vec::new();
                for pos in s.max(first)..e {
                    let id = self.doc_id_at(pos as u64).expect("in range");
                    self.read_cells(id, &wanted, &mut buf);
                    // The rules this row fails: none, one (on which column),
                    // or more.
                    let mut fails = 0u32;
                    let mut failed_col = u32::MAX;
                    for (k, (slot, t)) in compiled.tests.iter().enumerate() {
                        if !t.test(&buf[remap[*slot]], compiled.read) {
                            fails += 1;
                            failed_col = rule_col[k];
                            if fails > 1 {
                                break;
                            }
                        }
                    }
                    if fails == 0 {
                        shown += 1;
                    }
                    if fails > 1 {
                        continue;
                    }
                    for (k, card) in cards.iter().enumerate() {
                        if fails == 1 && card_phys[k] != Some(failed_col) {
                            continue;
                        }
                        let a = &mut acc[k];
                        a.rows += 1;
                        let v: &str = card_slot[k].map_or("", |sl| buf[sl].as_ref());
                        if v.trim().is_empty() {
                            a.blanks += 1;
                            continue;
                        }
                        match card.kind {
                            FacetKind::Values => {
                                if let Some(n) = &needles[k] {
                                    if !v.to_lowercase().contains(n.as_str()) {
                                        continue;
                                    }
                                }
                                if let Some(n) = a.map.get_mut(v) {
                                    *n += 1;
                                } else if a.map.len() < MAX_LIST_VALUES
                                    && v.len() <= MAX_LIST_VALUE_BYTES
                                {
                                    a.map.insert(v.to_string(), 1);
                                } else {
                                    a.over = true;
                                }
                            }
                            FacetKind::Numbers | FacetKind::Dates => {
                                let x = if card.kind == FacetKind::Numbers {
                                    value::parse_number(v, read)
                                } else {
                                    value::parse_date(v, read).map(f64::floor)
                                };
                                match x.and_then(|x| bin_of(card, x)) {
                                    Some(b) => a.bins[b] += 1,
                                    None => a.other += 1,
                                }
                            }
                        }
                    }
                }
                progress.step((e - s) as u64)?;
                Ok((acc, shown))
            })
            .collect::<Result<_, _>>()?;
        let mut all = facet_accs(cards);
        let mut shown = 0;
        for (accs, n) in parts {
            shown += n;
            for (a, b) in all.iter_mut().zip(accs) {
                a.rows += b.rows;
                a.blanks += b.blanks;
                a.other += b.other;
                a.over |= b.over;
                a.bins.iter_mut().zip(b.bins).for_each(|(x, y)| *x += y);
                for (k, n) in b.map {
                    if let Some(x) = a.map.get_mut(&k) {
                        *x += n;
                    } else if a.map.len() < MAX_LIST_VALUES {
                        a.map.insert(k, n);
                    } else {
                        a.over = true;
                    }
                }
            }
        }
        let cards = cards
            .iter()
            .zip(all)
            .map(|(c, a)| {
                let distinct = a.map.len() as u64;
                let mut values: Vec<ValueCount> = a
                    .map
                    .into_iter()
                    .map(|(value, count)| ValueCount { value, count })
                    .collect();
                values.sort_unstable_by(|x, y| {
                    y.count
                        .cmp(&x.count)
                        .then_with(|| value::natural_cmp(x.value.as_bytes(), y.value.as_bytes()))
                });
                let complete = !a.over && values.len() <= FACET_VALUES;
                values.truncate(FACET_VALUES);
                Facet {
                    col: c.col,
                    values,
                    distinct,
                    complete,
                    bins: a.bins,
                    range: c.range,
                    other: a.other,
                    blanks: a.blanks,
                    rows: a.rows,
                }
            })
            .collect();
        Ok(Facets { cards, shown })
    }
}

/// Numbers by value if all are numbers, dates by date if all are dates,
/// else natural text order.
fn sort_values(values: &mut [ValueCount], read: ReadOptions) {
    if values
        .iter()
        .all(|v| value::parse_number(&v.value, read).is_some())
    {
        values.sort_by(|a, b| {
            let x = value::parse_number(&a.value, read).unwrap_or(0.0);
            let y = value::parse_number(&b.value, read).unwrap_or(0.0);
            x.total_cmp(&y).then_with(|| a.value.cmp(&b.value))
        });
    } else if values
        .iter()
        .all(|v| value::parse_date(&v.value, read).is_some())
    {
        values.sort_by(|a, b| {
            let x = value::parse_date(&a.value, read).unwrap_or(0.0);
            let y = value::parse_date(&b.value, read).unwrap_or(0.0);
            x.total_cmp(&y).then_with(|| a.value.cmp(&b.value))
        });
    } else {
        let mut keyed: Vec<(String, ValueCount)> = values
            .iter()
            .map(|v| {
                let mut k = String::new();
                value::text_key(&v.value, false, &mut k);
                (k, v.clone())
            })
            .collect();
        keyed.sort_by(|a, b| {
            value::natural_cmp(a.0.as_bytes(), b.0.as_bytes())
                .then_with(|| a.1.value.cmp(&b.1.value))
        });
        for (slot, (_, v)) in values.iter_mut().zip(keyed) {
            *slot = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::PatternKind;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn all(d: &Document) -> Vec<Vec<String>> {
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    fn rule(col: u32, rule: Rule) -> FilterSpec {
        FilterSpec {
            columns: vec![ColumnFilter { col, rule }],
            keep_first: true,
            read: ReadOptions::default(),
        }
    }

    fn cond(c: Condition) -> Rule {
        Rule::Conditions {
            first: c,
            second: None,
            or: false,
        }
    }

    const PEOPLE: &str = "name,age,city,born\r\nann,31,Paris,2001-05-04\r\nbob,,Rome,1999-12-31\r\ncat,7,paris,2019-01-01\r\ndan,45,Oslo,\r\n";

    #[test]
    fn filters_undo_and_redo_without_unsaving() {
        let names =
            |d: &Document| -> Vec<String> { all(d).into_iter().map(|r| r[0].clone()).collect() };
        let paris = rule(
            2,
            Rule::Values {
                values: vec!["Paris".into()],
                blanks: false,
            },
        );
        let mut d = doc(PEOPLE);
        let everyone = names(&d);
        d.set_filter(&paris).unwrap();
        assert_eq!(names(&d), ["name", "ann"]);
        assert!(!d.is_modified(), "a filter changes no data");
        assert!(d.can_undo());
        // Undo takes the filter off; redo puts it back.
        assert!(d.undo());
        assert_eq!(names(&d), everyone);
        assert!(!d.has_filter());
        assert!(d.redo());
        assert_eq!(names(&d), ["name", "ann"]);
        // Changing the filter, then clearing it: each is a step.
        d.set_filter(&rule(
            1,
            cond(Condition::Number {
                op: CmpOp::Gt,
                value: 10.0,
            }),
        ))
        .unwrap();
        assert_eq!(names(&d), ["name", "ann", "dan"]);
        d.clear_filter();
        assert_eq!(names(&d), everyone);
        assert!(d.undo());
        assert_eq!(names(&d), ["name", "ann", "dan"]);
        assert!(d.undo());
        assert_eq!(names(&d), ["name", "ann"]);
        // Clearing with no filter on is no step.
        assert!(d.undo());
        let pos = d.history_pos();
        d.clear_filter();
        assert_eq!(d.history_pos(), pos);
        // Hidden rows stay hidden through filter undo and redo.
        d.hide_rows(4, 1).unwrap();
        d.set_filter(&paris).unwrap();
        assert!(d.undo());
        assert_eq!(names(&d), ["name", "ann", "bob", "cat"]);
        assert!(!d.is_modified());
        // A real edit after a filter: undone first, then the filter.
        d.set_filter(&paris).unwrap();
        d.set_cell(1, 1, "32".into()).unwrap();
        assert!(d.is_modified());
        assert!(d.undo());
        assert!(!d.is_modified());
        assert_eq!(names(&d), ["name", "ann"]);
        assert!(d.undo());
        assert_eq!(names(&d), ["name", "ann", "bob", "cat"]);
    }

    #[test]
    fn largest_and_smallest() {
        let names =
            |d: &Document| -> Vec<String> { all(d).into_iter().map(|r| r[0].clone()).collect() };
        let top = |count, percent, bottom| {
            rule(
                1,
                cond(Condition::Top {
                    count,
                    percent,
                    bottom,
                }),
            )
        };
        let mut d = doc("n,v\r\na,5\r\nb,9\r\nc,x\r\nd,9\r\ne,1\r\nf,\r\ng,3\r\n");
        d.set_filter(&top(2.0, false, false)).unwrap();
        // Ties are kept: both 9s.
        assert_eq!(names(&d), ["n", "b", "d"]);
        d.set_filter(&top(3.0, false, false)).unwrap();
        assert_eq!(names(&d), ["n", "a", "b", "d"]);
        d.set_filter(&top(2.0, false, true)).unwrap();
        assert_eq!(names(&d), ["n", "e", "g"]);
        // 40 % of five numbers is two.
        d.set_filter(&top(40.0, true, true)).unwrap();
        assert_eq!(names(&d), ["n", "e", "g"]);
        d.set_filter(&top(100.0, false, false)).unwrap();
        assert_eq!(names(&d), ["n", "a", "b", "d", "e", "g"]);
        d.set_filter(&top(0.0, false, false)).unwrap();
        assert_eq!(names(&d), ["n"]);
        assert!(d.set_filter(&top(101.0, true, false)).is_err());
        assert!(d.set_filter(&top(f64::NAN, false, false)).is_err());
        // The saved rule keeps "top 2", so reapplying follows new values.
        d.set_filter(&top(1.0, false, false)).unwrap();
        assert!(matches!(
            d.filter_spec().unwrap().columns[0].rule,
            Rule::Conditions {
                first: Condition::Top { .. },
                ..
            }
        ));
        // Highlight rules refuse it.
        assert!(RuleMatcher::new(
            &cond(Condition::Top {
                count: 1.0,
                percent: false,
                bottom: false
            }),
            ReadOptions::default()
        )
        .is_err());
    }

    #[test]
    fn rule_kinds() {
        let mut d = doc(PEOPLE);
        let names =
            |d: &Document| -> Vec<String> { all(d).into_iter().map(|r| r[0].clone()).collect() };
        let cases: Vec<(FilterSpec, Vec<&str>)> = vec![
            (
                rule(
                    2,
                    Rule::Values {
                        values: vec!["Paris".into()],
                        blanks: false,
                    },
                ),
                vec!["name", "ann"],
            ),
            (
                rule(
                    1,
                    Rule::Values {
                        values: vec![],
                        blanks: true,
                    },
                ),
                vec!["name", "bob"],
            ),
            (
                rule(
                    2,
                    cond(Condition::Text {
                        op: TextOp::Equals,
                        text: "paris".into(),
                        match_case: false,
                    }),
                ),
                vec!["name", "ann", "cat"],
            ),
            (
                rule(
                    2,
                    cond(Condition::Text {
                        op: TextOp::NotContains,
                        text: "o".into(),
                        match_case: false,
                    }),
                ),
                vec!["name", "ann", "cat"],
            ),
            (
                rule(
                    0,
                    cond(Condition::Text {
                        op: TextOp::BeginsWith,
                        text: "b".into(),
                        match_case: true,
                    }),
                ),
                vec!["name", "bob"],
            ),
            (
                rule(
                    1,
                    cond(Condition::Number {
                        op: CmpOp::Ge,
                        value: 31.0,
                    }),
                ),
                vec!["name", "ann", "dan"],
            ),
            (
                rule(
                    1,
                    cond(Condition::NumberBetween {
                        low: 40.0,
                        high: 5.0,
                    }),
                ),
                vec!["name", "ann", "cat"],
            ),
            (
                rule(
                    3,
                    cond(Condition::Date {
                        op: CmpOp::Lt,
                        value: "2001-05-04".into(),
                    }),
                ),
                vec!["name", "bob"],
            ),
            (
                rule(
                    3,
                    cond(Condition::DateBetween {
                        low: "1/1/2000".into(),
                        high: "2020-01-01".into(),
                    }),
                ),
                vec!["name", "ann", "cat"],
            ),
            (rule(3, cond(Condition::Blank)), vec!["name", "dan"]),
            (
                rule(
                    0,
                    Rule::Conditions {
                        first: Condition::Text {
                            op: TextOp::Equals,
                            text: "ann".into(),
                            match_case: false,
                        },
                        second: Some(Condition::Text {
                            op: TextOp::EndsWith,
                            text: "n".into(),
                            match_case: false,
                        }),
                        or: true,
                    },
                ),
                vec!["name", "ann", "dan"],
            ),
            (
                rule(
                    0,
                    cond(Condition::Pattern {
                        pattern: Pattern {
                            text: "^[a-c]".into(),
                            kind: PatternKind::Regex,
                            ..Default::default()
                        },
                        negate: true,
                    }),
                ),
                vec!["name", "dan"],
            ),
        ];
        for (spec, want) in cases {
            d.set_filter(&spec).unwrap();
            assert_eq!(names(&d), want, "{spec:?}");
            assert_eq!(d.total_rows(), 5);
        }
        // Two columns combine with AND.
        let mut spec = rule(
            2,
            cond(Condition::Text {
                op: TextOp::Contains,
                text: "par".into(),
                match_case: false,
            }),
        );
        spec.columns.push(ColumnFilter {
            col: 1,
            rule: cond(Condition::Number {
                op: CmpOp::Lt,
                value: 10.0,
            }),
        });
        assert_eq!(d.set_filter(&spec).unwrap(), 2);
        assert_eq!(names(&d), ["name", "cat"]);
        assert!(d
            .set_filter(&rule(
                3,
                cond(Condition::Date {
                    op: CmpOp::Eq,
                    value: "soon".into()
                })
            ))
            .is_err());
        d.clear_filter();
        assert_eq!(d.row_count(), 5);
    }

    #[test]
    fn edits_and_saves_keep_hidden_rows() {
        let dir = std::env::temp_dir().join(format!("nd-filter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.csv");
        std::fs::write(&path, PEOPLE).unwrap();
        let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        d.set_filter(&rule(
            2,
            cond(Condition::Text {
                op: TextOp::Equals,
                text: "paris".into(),
                match_case: false,
            }),
        ))
        .unwrap();
        assert_eq!(d.row_count(), 3);
        assert_eq!(d.row_number(2), Some(3));
        // Go To by the whole file's numbers: shown rows map back, and a
        // hidden one gives the next row shown (or the last one).
        let shown: Vec<u64> = (0..3).map(|r| d.row_number(r).unwrap()).collect();
        for (i, &n) in shown.iter().enumerate() {
            assert_eq!(d.shown_row_of(n), Some((i as u64, true)));
        }
        if shown[0] > 0 {
            assert_eq!(d.shown_row_of(0), Some((0, false)));
        }
        assert_eq!(d.shown_row_of(10_000), Some((2, false)));
        // Edit a shown row so it no longer matches: it stays shown.
        d.set_cell(1, 2, "Lyon".into()).unwrap();
        assert_eq!(d.row_count(), 3);
        assert_eq!(d.cell(1, 2).unwrap(), "Lyon");
        // Insert shows the new row; delete removes only shown rows.
        d.insert_rows(3, vec![vec!["eve".into()]]).unwrap();
        assert_eq!(d.row_count(), 4);
        assert_eq!(d.cell(3, 0).unwrap(), "eve");
        d.delete_rows(1, 2).unwrap();
        assert_eq!(
            all(&d).iter().map(|r| r[0].as_str()).collect::<Vec<_>>(),
            ["name", "eve"]
        );
        assert_eq!(d.total_rows(), 4);
        assert!(matches!(d.move_rows(0, 1, 1), Err(Error::FilterActive)));
        d.save(crate::SaveOptions::default()).unwrap();
        assert!(d.is_filtered());
        assert_eq!(d.row_count(), 2);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "name,age,city,born\r\nbob,,Rome,1999-12-31\r\neve\r\ndan,45,Oslo,\r\n"
        );
        d.clear_filter();
        assert_eq!(d.row_count(), 4);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn undo_with_filter_on() {
        let mut d = doc(PEOPLE);
        d.set_filter(&rule(
            0,
            cond(Condition::Text {
                op: TextOp::Contains,
                text: "a".into(),
                match_case: false,
            }),
        ))
        .unwrap();
        assert_eq!(d.row_count(), 4); // name, ann, cat, dan
        d.delete_rows(1, 3).unwrap();
        assert_eq!(d.row_count(), 1);
        d.undo();
        assert_eq!(d.row_count(), 4);
        d.clear_filter();
        assert_eq!(all(&d), all(&doc(PEOPLE)));
        assert!(!d.is_modified());
    }

    #[test]
    fn rules_follow_moved_columns() {
        let mut d = doc(PEOPLE);
        d.set_filter(&rule(
            2,
            Rule::Values {
                values: vec!["Rome".into()],
                blanks: false,
            },
        ))
        .unwrap();
        d.clear_filter();
        d.insert_cols(0, 1).unwrap();
        d.set_filter(&rule(
            3,
            Rule::Values {
                values: vec!["Rome".into()],
                blanks: false,
            },
        ))
        .unwrap();
        assert_eq!(d.row_count(), 2);
        assert_eq!(d.filtered_columns(), [3]);
        d.move_cols(3, 1, 0).unwrap();
        assert_eq!(d.filtered_columns(), [0]);
        let again = d.filter_spec().unwrap();
        d.set_filter(&again).unwrap();
        assert_eq!(d.row_count(), 2);
    }

    #[test]
    fn facets_count_what_the_other_cards_leave() {
        let d = doc("party,precinct,age\nD,12,20\nR,12,30\nD,3,40\nD,12,\nI,3,25\n");
        let cards = vec![
            FacetCard {
                col: 0,
                ..Default::default()
            },
            FacetCard {
                col: 1,
                ..Default::default()
            },
            FacetCard {
                col: 2,
                kind: FacetKind::Numbers,
                range: Some((20.0, 40.0)),
                ..Default::default()
            },
        ];
        // Party D picked: the precinct card counts D's rows; the party card
        // still counts every party (its own rule left out).
        let spec = rule(
            0,
            Rule::Values {
                values: vec!["D".into()],
                blanks: false,
            },
        );
        let f = d.facet_counts(&cards, &spec, &Progress::new()).unwrap();
        assert_eq!(f.shown, 3);
        let party: Vec<(&str, u64)> = f.cards[0]
            .values
            .iter()
            .map(|v| (v.value.as_str(), v.count))
            .collect();
        assert_eq!(party, vec![("D", 3), ("I", 1), ("R", 1)]);
        let precinct: Vec<(&str, u64)> = f.cards[1]
            .values
            .iter()
            .map(|v| (v.value.as_str(), v.count))
            .collect();
        assert_eq!(precinct, vec![("12", 2), ("3", 1)]);
        assert_eq!(f.cards[2].bins.iter().sum::<u64>(), 2);
        assert_eq!(f.cards[2].blanks, 1);
        assert_eq!(f.cards[2].bins[0], 1);
        assert_eq!(f.cards[2].bins[FACET_BINS - 1], 1);
        // Searching a card.
        let mut c = cards.clone();
        c[0].search = Some("r".into());
        let f = d
            .facet_counts(
                &c,
                &FilterSpec {
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(f.cards[0].values.len(), 1);
        assert_eq!(f.shown, 5);
    }

    #[test]
    fn value_lists() {
        let d = doc("k,v\r\nb,1\r\na,2\r\nb,3\r\n,4\r\nA10,5\r\nA9,6\r\n");
        let p = Progress::new();
        let none = FilterSpec {
            keep_first: true,
            ..Default::default()
        };
        let l = d.filter_values(0, &none, None, &p).unwrap();
        let vals: Vec<(&str, u64)> = l
            .values
            .iter()
            .map(|v| (v.value.as_str(), v.count))
            .collect();
        assert_eq!(vals, [("a", 1), ("A9", 1), ("A10", 1), ("b", 2)]);
        assert_eq!((l.blanks, l.complete), (1, true));
        // Other columns' rules narrow the list; the column's own rule does not.
        let mut spec = rule(
            1,
            cond(Condition::Number {
                op: CmpOp::Le,
                value: 3.0,
            }),
        );
        spec.columns.push(ColumnFilter {
            col: 0,
            rule: Rule::Values {
                values: vec!["a".into()],
                blanks: false,
            },
        });
        let l = d.filter_values(0, &spec, None, &p).unwrap();
        assert_eq!(l.values.len(), 2);
        let l = d.filter_values(0, &none, Some("a"), &p).unwrap();
        assert_eq!(l.values.len(), 3);
        let nums = d.filter_values(1, &none, None, &p).unwrap();
        assert_eq!(nums.values.first().unwrap().value, "1");
    }
}
