//! Filters: rules per column that decide which rows show. A filter never
//! changes data; hidden rows are saved as they are. Which rows show is fixed
//! when the filter is applied (see `document::View`).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::coltype::{ReadCtx, Reader};
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
    /// How each wanted column is read (by slot).
    readers: Vec<Reader>,
}

fn day_of(text: &str, rd: Reader) -> Result<f64, Error> {
    rd.date(text)
        .map(f64::floor)
        .ok_or_else(|| Error::BadPattern(format!("\"{text}\" is not a date")))
}

fn compile_cond(c: &Condition, rd: Reader) -> Result<Cond, Error> {
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
        Condition::Date { op, value } => Cond::Date(*op, day_of(value, rd)?),
        Condition::DateBetween { low, high } => {
            let (a, b) = (day_of(low, rd)?, day_of(high, rd)?);
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
    fn test(&self, s: &str, rd: Reader) -> bool {
        match self {
            Cond::Match(m, negate) => m.is_match(s) != *negate,
            Cond::Num(op, v) => rd.number(s).is_some_and(|x| op.test(x, *v)),
            Cond::NumBetween(a, b) => rd.number(s).is_some_and(|x| x >= *a && x <= *b),
            Cond::Date(op, v) => rd.date(s).is_some_and(|x| op.test(x.floor(), *v)),
            Cond::DateBetween(a, b) => rd
                .date(s)
                .map(f64::floor)
                .is_some_and(|x| x >= *a && x <= *b),
            Cond::Blank(want) => s.is_empty() == *want,
        }
    }
}

impl Test {
    fn test(&self, s: &str, rd: Reader) -> bool {
        match self {
            Test::Values(set, blanks) => {
                if s.is_empty() {
                    *blanks
                } else {
                    set.contains(s)
                }
            }
            Test::Conds(a, None, _) => a.test(s, rd),
            Test::Conds(a, Some(b), or) => {
                if *or {
                    a.test(s, rd) || b.test(s, rd)
                } else {
                    a.test(s, rd) && b.test(s, rd)
                }
            }
        }
    }
}

/// One rule compiled to test single values (highlight rules).
pub struct RuleMatcher {
    test: Test,
    rd: Reader,
}

impl RuleMatcher {
    pub fn new(rule: &Rule, rd: Reader) -> Result<RuleMatcher, Error> {
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
                compile_cond(first, rd)?,
                second.as_ref().map(|s| compile_cond(s, rd)).transpose()?,
                *or,
            ),
        };
        Ok(RuleMatcher { test, rd })
    }

    pub fn matches(&self, s: &str) -> bool {
        self.test.test(s, self.rd)
    }
}

impl Compiled {
    /// Rules by physical column.
    fn new(rules: &[ColumnFilter], ctx: &ReadCtx) -> Result<Compiled, Error> {
        let mut wanted: Vec<u32> = rules.iter().map(|c| c.col).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let readers: Vec<Reader> = wanted.iter().map(|p| ctx.reader_phys(*p)).collect();
        let mut tests = Vec::with_capacity(rules.len());
        for c in rules {
            let slot = wanted.binary_search(&c.col).expect("listed");
            let t = match &c.rule {
                Rule::Values { values, blanks } => {
                    Test::Values(values.iter().cloned().collect(), *blanks)
                }
                Rule::Conditions { first, second, or } => Test::Conds(
                    compile_cond(first, readers[slot])?,
                    second
                        .as_ref()
                        .map(|s| compile_cond(s, readers[slot]))
                        .transpose()?,
                    *or,
                ),
            };
            tests.push((slot, t));
        }
        Ok(Compiled {
            wanted,
            tests,
            readers,
        })
    }

    fn passes(&self, cells: &[Cow<'_, str>]) -> bool {
        self.tests
            .iter()
            .all(|(slot, t)| t.test(&cells[*slot], self.readers[*slot]))
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
                let rd = self.read_ctx(spec.read).reader_phys(col);
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
                            if let Some(v) = rd.number(&buf[0]) {
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
        let compiled = Compiled::new(&resolved.columns, &self.read_ctx(spec.read))?;
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
        let compiled = Compiled::new(&rules, &self.read_ctx(others.read))?;
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
                        .all(|(slot, t)| t.test(&buf[remap[*slot]], compiled.readers[*slot]));
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
        sort_values(&mut values, self.read_ctx(spec.read).reader(col));
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
    /// Values: the column has too many different values to list (an ID or
    /// a phone number), so the card counts only what a search keeps; with a
    /// search, that search keeps too many.
    pub many: bool,
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
        let ctx = self.read_ctx(read);
        let readers: Vec<Reader> = need.iter().map(|&k| ctx.reader(out[k].col)).collect();
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
                            readers[i].number(v)
                        } else {
                            readers[i].date(v).map(f64::floor)
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
        self.facet_counts_cached(&mut FacetCache::new(), cards, spec, progress)
    }
}

// ---- the filter panel's quick recount ----

/// Different values a column may hold for its codes to be kept (more, and
/// the column is ID-like: its card is searched, not listed).
pub const FACET_CODE_VALUES: usize = 1 << 16;

/// A column's values as numbers: the value number at each document
/// position ([`NO_CODE`] for the title row, [`HIDDEN`] for a value a
/// search leaves out).
struct ColCodes {
    codes: Vec<u32>,
    values: Vec<String>,
}

const NO_CODE: u32 = u32::MAX;
const HIDDEN: u32 = u32::MAX - 1;

/// A card's smallest and largest value.
type Range = Option<(f64, f64)>;

/// What one cache entry was worked out for.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Key {
    /// A column's values.
    Col(u32),
    /// Pass (1) or fail (0) of a rule on an ID-like column.
    Rule(u32, String),
    /// Bars of a numbers or dates card on an ID-like column.
    Bars(u32, String),
    /// The values of an ID-like column that a card's search keeps.
    Search(u32, String),
}

struct Entry {
    /// The data revision the codes are for.
    rev: u64,
    /// `None`: too many different values (or search matches) for codes.
    codes: Option<ColCodes>,
}

/// What the filter panel has worked out about a table's columns, kept
/// between counts: each column's values as codes, so a recount after a
/// click is whole-number work over arrays instead of reading the file
/// again. After an edit only the changed rows are read again; entries no
/// card or rule uses any more are dropped.
#[derive(Default)]
pub struct FacetCache {
    keep_first: Option<bool>,
    entries: HashMap<Key, Entry>,
    /// Smallest and largest number (or date) of ID-like columns, by
    /// column and kind, with the data revision they are for.
    ranges: HashMap<(u32, String), (u64, Range)>,
}

impl FacetCache {
    pub fn new() -> FacetCache {
        FacetCache::default()
    }

    /// Drop everything (the panel was closed).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.ranges.clear();
        self.keep_first = None;
    }

    /// Bytes held (about).
    pub fn bytes(&self) -> usize {
        self.entries
            .values()
            .filter_map(|e| e.codes.as_ref())
            .map(|c| c.codes.len() * 4 + c.values.iter().map(|v| v.len() + 24).sum::<usize>())
            .sum()
    }
}

/// One card's running counts (by value number).
#[derive(Clone, Default)]
struct CardAcc {
    by_value: Vec<u32>,
    bins: Vec<u64>,
    other: u64,
    blanks: u64,
    rows: u64,
}

impl CardAcc {
    fn merge(mut self, b: CardAcc) -> CardAcc {
        self.by_value
            .iter_mut()
            .zip(&b.by_value)
            .for_each(|(x, y)| *x += y);
        self.bins.iter_mut().zip(&b.bins).for_each(|(x, y)| *x += y);
        self.other += b.other;
        self.blanks += b.blanks;
        self.rows += b.rows;
        self
    }
}

/// What one value of a card's column comes to.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Lands {
    Blank,
    /// Left out by the card's search.
    Hidden,
    Value,
    Bin(usize),
    Other,
}

/// A bars entry's codes: 0 blank, 1 not a number (or outside), 2.. a bar.
fn bar_lands() -> Vec<Lands> {
    let mut l = vec![Lands::Blank, Lands::Other];
    l.extend((0..FACET_BINS).map(Lands::Bin));
    l
}

fn bar_code(l: Lands) -> u32 {
    match l {
        Lands::Blank => 0,
        Lands::Bin(b) => 2 + b as u32,
        _ => 1,
    }
}

/// How an entry turns a cell into a code.
enum Coder<'a> {
    /// A value table: every value (`None`) or the values containing a
    /// search, lowercased (blanks kept, so they count as blanks).
    Table(Option<&'a str>),
    /// A fixed code per cell.
    Fixed(&'a (dyn Fn(&str) -> u32 + Sync)),
}

impl Coder<'_> {
    fn keeps(&self, v: &str) -> bool {
        match self {
            Coder::Table(Some(n)) => v.trim().is_empty() || v.to_lowercase().contains(n),
            _ => true,
        }
    }
}

impl Document {
    /// Codes for column `phys` over every data row (in parallel), written
    /// straight into one list; `None` when a table would need more than
    /// [`FACET_CODE_VALUES`] values.
    fn build_codes(
        &self,
        phys: u32,
        keep_first: bool,
        coder: &Coder<'_>,
        progress: &Progress,
    ) -> Result<Option<ColCodes>, Error> {
        let total = self.total_rows() as usize;
        let first = usize::from(keep_first && total > 0);
        let want = [phys];
        let mut codes = vec![NO_CODE; total];
        if let Coder::Fixed(f) = coder {
            codes.par_chunks_mut(CHUNK).enumerate().try_for_each(
                |(c, out)| -> Result<(), Error> {
                    let s = c * CHUNK;
                    let mut buf = Vec::new();
                    for (i, slot) in out.iter_mut().enumerate() {
                        let pos = s + i;
                        if pos < first {
                            continue;
                        }
                        let id = self.doc_id_at(pos as u64).ok_or(Error::OutOfRange)?;
                        self.read_cells(id, &want, &mut buf);
                        *slot = f(&buf[0]);
                    }
                    progress.step(out.len() as u64)
                },
            )?;
            return Ok(Some(ColCodes {
                codes,
                values: Vec::new(),
            }));
        }
        if matches!(coder, Coder::Table(None)) && total > first {
            // A quick look first: a column whose sample (start, middle and
            // end) already holds half the values allowed is ID-like; reading
            // all of it would be wasted.
            let n = total - first;
            let part = 33_000.min(n);
            let mut seen: HashSet<String> = HashSet::new();
            let mut buf = Vec::new();
            for start in [first, first + n / 2 - part.min(n / 2) / 2, total - part] {
                for pos in start..(start + part).min(total) {
                    let id = self.doc_id_at(pos as u64).ok_or(Error::OutOfRange)?;
                    self.read_cells(id, &want, &mut buf);
                    if !seen.contains(buf[0].as_ref()) {
                        seen.insert(buf[0].to_string());
                    }
                }
                if seen.len() > FACET_CODE_VALUES / 2 {
                    return Ok(None);
                }
            }
        }
        // Each part numbers its own values; the parts' tables are then
        // joined and each part's codes renumbered in place.
        let parts: Vec<Option<Vec<String>>> = codes
            .par_chunks_mut(CHUNK)
            .enumerate()
            .map(|(c, out)| -> Result<Option<Vec<String>>, Error> {
                let s = c * CHUNK;
                let mut map: HashMap<String, u32> = HashMap::new();
                let mut values: Vec<String> = Vec::new();
                let mut buf = Vec::new();
                for (i, slot) in out.iter_mut().enumerate() {
                    let pos = s + i;
                    if pos < first {
                        continue;
                    }
                    let id = self.doc_id_at(pos as u64).ok_or(Error::OutOfRange)?;
                    self.read_cells(id, &want, &mut buf);
                    let v: &str = &buf[0];
                    *slot = match map.get(v) {
                        Some(&k) => k,
                        None if !coder.keeps(v) => HIDDEN,
                        None => {
                            if values.len() >= FACET_CODE_VALUES {
                                progress.step(out.len() as u64)?;
                                return Ok(None);
                            }
                            let k = values.len() as u32;
                            values.push(v.to_string());
                            map.insert(v.to_string(), k);
                            k
                        }
                    };
                }
                progress.step(out.len() as u64)?;
                Ok(Some(values))
            })
            .collect::<Result<_, _>>()?;
        let mut index: HashMap<String, u32> = HashMap::new();
        let mut values: Vec<String> = Vec::new();
        let mut remaps: Vec<Vec<u32>> = Vec::with_capacity(parts.len());
        for part in parts {
            let Some(vals) = part else {
                return Ok(None);
            };
            let mut remap = Vec::with_capacity(vals.len());
            for v in vals {
                let k = match index.get(&v) {
                    Some(&k) => k,
                    None => {
                        if values.len() >= FACET_CODE_VALUES {
                            return Ok(None);
                        }
                        let k = values.len() as u32;
                        index.insert(v.clone(), k);
                        values.push(v);
                        k
                    }
                };
                remap.push(k);
            }
            remaps.push(remap);
        }
        codes
            .par_chunks_mut(CHUNK)
            .zip(remaps)
            .for_each(|(out, remap)| {
                for c in out {
                    if *c != NO_CODE && *c != HIDDEN {
                        *c = remap[*c as usize];
                    }
                }
            });
        Ok(Some(ColCodes { codes, values }))
    }

    /// Bring `codes` up to date by reading again only rows at `changed`;
    /// `false` when a new value does not fit (build it again instead).
    fn patch_codes(
        &self,
        cc: &mut ColCodes,
        phys: u32,
        keep_first: bool,
        coder: &Coder<'_>,
        changed: &[u64],
    ) -> bool {
        let first = u64::from(keep_first);
        let want = [phys];
        let mut buf = Vec::new();
        let mut index: Option<HashMap<String, u32>> = None;
        for &pos in changed {
            if pos < first || pos as usize >= cc.codes.len() {
                continue;
            }
            let Some(id) = self.doc_id_at(pos) else {
                return false;
            };
            self.read_cells(id, &want, &mut buf);
            let v: &str = &buf[0];
            cc.codes[pos as usize] = match coder {
                Coder::Fixed(f) => f(v),
                Coder::Table(_) => {
                    let index = index.get_or_insert_with(|| {
                        cc.values
                            .iter()
                            .enumerate()
                            .map(|(i, v)| (v.clone(), i as u32))
                            .collect()
                    });
                    match index.get(v) {
                        Some(&k) => k,
                        None if !coder.keeps(v) => HIDDEN,
                        None => {
                            if cc.values.len() >= FACET_CODE_VALUES {
                                return false;
                            }
                            let k = cc.values.len() as u32;
                            cc.values.push(v.to_string());
                            index.insert(v.to_string(), k);
                            k
                        }
                    }
                }
            };
        }
        true
    }

    /// Make the entry for `key` current: kept as it is, patched for the
    /// rows changed since, or built again.
    fn refresh_entry(
        &self,
        cache: &mut FacetCache,
        key: &Key,
        phys: u32,
        keep_first: bool,
        coder: &Coder<'_>,
        progress: &Progress,
    ) -> Result<(), Error> {
        let rev = self.data_revision();
        if let Some(e) = cache.entries.get_mut(key) {
            if e.rev == rev {
                return Ok(());
            }
            if let Some(changed) = self.changed_since(e.rev) {
                let ok = match &mut e.codes {
                    Some(cc) => self.patch_codes(cc, phys, keep_first, coder, &changed),
                    // An ID-like column stays so after a few edits.
                    None => matches!(key, Key::Col(_)),
                };
                if ok {
                    e.rev = rev;
                    return Ok(());
                }
            }
        }
        progress.grow(self.total_rows());
        let codes = self.build_codes(phys, keep_first, coder, progress)?;
        cache.entries.insert(key.clone(), Entry { rev, codes });
        Ok(())
    }

    /// The filter panel's counts, as [`Document::facet_counts`], using and
    /// filling `cache`: the first count of a column reads it once; later
    /// counts (each click) work from its codes, and after an edit only the
    /// changed rows are read. A values card on an ID-like column counts only
    /// once it has a search (`many` says so).
    pub fn facet_counts_cached(
        &self,
        cache: &mut FacetCache,
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
        if cache.keep_first != Some(spec.keep_first) {
            cache.entries.clear();
            cache.keep_first = Some(spec.keep_first);
        }
        let keep_first = spec.keep_first;
        let read = spec.read;
        let ctx = self.read_ctx(read);
        let pspec = self.to_physical_spec(spec)?;
        let compiled = Compiled::new(&pspec.columns, &self.read_ctx(read))?;
        let card_phys: Vec<Option<u32>> = cards
            .iter()
            .map(|c| {
                (c.col < self.col_count())
                    .then(|| self.phys(c.col))
                    .flatten()
            })
            .collect();
        let total = self.total_rows();
        progress.start(total);
        let mut used: HashSet<Key> = HashSet::new();
        // Every rule's and every card's column.
        let mut cols: Vec<u32> = compiled.wanted.clone();
        cols.extend(card_phys.iter().flatten());
        cols.sort_unstable();
        cols.dedup();
        for &p in &cols {
            let key = Key::Col(p);
            self.refresh_entry(cache, &key, p, keep_first, &Coder::Table(None), progress)?;
            used.insert(key);
        }
        let coded = |cache: &FacetCache, p: u32| -> bool {
            cache
                .entries
                .get(&Key::Col(p))
                .is_some_and(|e| e.codes.is_some())
        };
        // Ranges for numbers and dates cards without one: from the codes'
        // values when the column has them, else one pass as before.
        let mut cards = cards.to_vec();
        let mut slow_range = Vec::new();
        for (k, c) in cards.iter_mut().enumerate() {
            if c.kind == FacetKind::Values || c.range.is_some() {
                continue;
            }
            let codes = card_phys[k]
                .and_then(|p| cache.entries.get(&Key::Col(p)))
                .and_then(|e| e.codes.as_ref());
            match codes {
                Some(cc) => {
                    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                    let rd = card_phys[k].map_or(Reader::plain(read), |p| ctx.reader_phys(p));
                    for v in &cc.values {
                        let x = if c.kind == FacetKind::Numbers {
                            rd.number(v)
                        } else {
                            rd.date(v).map(f64::floor)
                        };
                        if let Some(x) = x {
                            lo = lo.min(x);
                            hi = hi.max(x);
                        }
                    }
                    c.range = (lo <= hi).then_some((lo, hi));
                }
                None => {
                    if let Some(p) = card_phys[k] {
                        let key = (p, format!("{:?} {read:?}", c.kind));
                        let now = self.data_revision();
                        match cache.ranges.get(&key).copied() {
                            Some((r, range)) if r == now => c.range = range,
                            // After an edit: widened by the changed cells
                            // (the bars stay put; they never shrink).
                            Some((r, range)) => match self.changed_since(r) {
                                Some(changed) => {
                                    let (mut lo, mut hi) =
                                        range.unwrap_or((f64::INFINITY, f64::NEG_INFINITY));
                                    let mut buf = Vec::new();
                                    for pos in changed {
                                        if pos < u64::from(keep_first) {
                                            continue;
                                        }
                                        let Some(id) = self.doc_id_at(pos) else {
                                            continue;
                                        };
                                        self.read_cells(id, &[p], &mut buf);
                                        let rd = ctx.reader_phys(p);
                                        let x = if c.kind == FacetKind::Numbers {
                                            rd.number(&buf[0])
                                        } else {
                                            rd.date(&buf[0]).map(f64::floor)
                                        };
                                        if let Some(x) = x {
                                            lo = lo.min(x);
                                            hi = hi.max(x);
                                        }
                                    }
                                    c.range = (lo <= hi).then_some((lo, hi));
                                    cache.ranges.insert(key, (now, c.range));
                                }
                                None => slow_range.push(k),
                            },
                            None => slow_range.push(k),
                        }
                    }
                }
            }
        }
        if !slow_range.is_empty() {
            let some: Vec<FacetCard> = slow_range.iter().map(|&k| cards[k].clone()).collect();
            let got = self.facet_ranges(&some, keep_first, read)?;
            for (&k, c) in slow_range.iter().zip(got) {
                cards[k].range = c.range;
                if let Some(p) = card_phys[k] {
                    cache.ranges.insert(
                        (p, format!("{:?} {read:?}", c.kind)),
                        (self.data_revision(), c.range),
                    );
                }
            }
        }
        if cache.ranges.len() > 64 {
            cache.ranges.clear();
        }
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
        let needles: Vec<Option<String>> = cards
            .iter()
            .map(|c| {
                c.search
                    .as_deref()
                    .map(str::to_lowercase)
                    .filter(|s| !s.is_empty())
            })
            .collect();
        let card_readers: Vec<Reader> = cards
            .iter()
            .enumerate()
            .map(|(k, _)| card_phys[k].map_or(Reader::plain(read), |p| ctx.reader_phys(p)))
            .collect();
        let lands = |k: usize, v: &str| -> Lands {
            if v.trim().is_empty() {
                return Lands::Blank;
            }
            let card = &cards[k];
            match card.kind {
                FacetKind::Values => match &needles[k] {
                    Some(n) if !v.to_lowercase().contains(n.as_str()) => Lands::Hidden,
                    _ => Lands::Value,
                },
                FacetKind::Numbers | FacetKind::Dates => {
                    let rd = card_readers[k];
                    let x = if card.kind == FacetKind::Numbers {
                        rd.number(v)
                    } else {
                        rd.date(v).map(f64::floor)
                    };
                    match x.and_then(|x| bin_of(card, x)) {
                        Some(b) => Lands::Bin(b),
                        None => Lands::Other,
                    }
                }
            }
        };
        // Rules and cards on ID-like columns get entries of their own.
        let mut rule_keys: Vec<Key> = Vec::with_capacity(compiled.tests.len());
        for (k, (slot, t)) in compiled.tests.iter().enumerate() {
            let p = compiled.wanted[*slot];
            if coded(cache, p) {
                rule_keys.push(Key::Col(p));
                continue;
            }
            let key = Key::Rule(p, format!("{:?} {read:?}", pspec.columns[k].rule));
            let rd = ctx.reader_phys(p);
            let f = |v: &str| u32::from(t.test(v, rd));
            self.refresh_entry(cache, &key, p, keep_first, &Coder::Fixed(&f), progress)?;
            used.insert(key.clone());
            rule_keys.push(key);
        }
        // Each card's entry (None: a values card on an ID-like column with
        // no search, which is not counted).
        let mut card_keys: Vec<Option<Key>> = Vec::with_capacity(cards.len());
        for (k, c) in cards.iter().enumerate() {
            let Some(p) = card_phys[k] else {
                card_keys.push(None);
                continue;
            };
            if coded(cache, p) {
                card_keys.push(Some(Key::Col(p)));
                continue;
            }
            let key = match (c.kind, &needles[k]) {
                (FacetKind::Values, None) => {
                    card_keys.push(None);
                    continue;
                }
                (FacetKind::Values, Some(n)) => Key::Search(p, n.clone()),
                _ => Key::Bars(p, format!("{:?} {:?} {read:?}", c.kind, c.range)),
            };
            let f = |v: &str| bar_code(lands(k, v));
            let coder = match &key {
                Key::Search(_, n) => Coder::Table(Some(n)),
                _ => Coder::Fixed(&f),
            };
            self.refresh_entry(cache, &key, p, keep_first, &coder, progress)?;
            used.insert(key.clone());
            card_keys.push(Some(key));
        }
        cache.entries.retain(|k, _| used.contains(k));
        let cache = &*cache;
        let codes_for = |key: &Key| cache.entries.get(key).and_then(|e| e.codes.as_ref());
        // Each rule: its codes and whether each code passes.
        struct RuleAt<'a> {
            phys: u32,
            codes: &'a ColCodes,
            pass: Vec<bool>,
        }
        let mut rules: Vec<RuleAt<'_>> = Vec::with_capacity(rule_keys.len());
        for (k, key) in rule_keys.iter().enumerate() {
            let (slot, t) = &compiled.tests[k];
            let Some(cc) = codes_for(key) else {
                return Err(Error::Invalid("the filter panel lost a column".into()));
            };
            let pass = match key {
                Key::Col(_) => {
                    let rd = compiled.readers[*slot];
                    cc.values.iter().map(|v| t.test(v, rd)).collect()
                }
                _ => vec![false, true],
            };
            rules.push(RuleAt {
                phys: compiled.wanted[*slot],
                codes: cc,
                pass,
            });
        }
        // Each card: its codes and where each code lands.
        let card_src: Vec<Option<(&ColCodes, Vec<Lands>)>> = card_keys
            .iter()
            .enumerate()
            .map(|(k, key)| {
                let key = key.as_ref()?;
                let cc = codes_for(key)?;
                let l = match key {
                    Key::Bars(..) => bar_lands(),
                    _ => cc.values.iter().map(|v| lands(k, v)).collect(),
                };
                Some((cc, l))
            })
            .collect();
        let first = usize::from(keep_first && total > 0);
        let blank_acc = || -> Vec<CardAcc> {
            cards
                .iter()
                .zip(&card_src)
                .map(|(c, src)| CardAcc {
                    by_value: match (c.kind, src) {
                        (FacetKind::Values, Some((cc, _))) => vec![0; cc.values.len()],
                        _ => Vec::new(),
                    },
                    bins: if c.kind == FacetKind::Values {
                        Vec::new()
                    } else {
                        vec![0; FACET_BINS]
                    },
                    ..Default::default()
                })
                .collect()
        };
        let chunks = (total as usize).div_ceil(CHUNK);
        let (accs, shown) = (0..chunks)
            .into_par_iter()
            .try_fold(
                || (blank_acc(), 0u64),
                |(mut acc, mut shown), c| -> Result<(Vec<CardAcc>, u64), Error> {
                    let s = c * CHUNK;
                    let e = (s + CHUNK).min(total as usize);
                    for pos in s.max(first)..e {
                        let mut fails = 0u32;
                        let mut failed_col = u32::MAX;
                        for r in &rules {
                            let code = r.codes.codes[pos];
                            let ok = code != NO_CODE && code != HIDDEN && r.pass[code as usize];
                            if !ok {
                                fails += 1;
                                failed_col = r.phys;
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
                        for (k, a) in acc.iter_mut().enumerate() {
                            let Some((cc, l)) = &card_src[k] else {
                                continue;
                            };
                            if fails == 1 && card_phys[k] != Some(failed_col) {
                                continue;
                            }
                            a.rows += 1;
                            let code = cc.codes[pos];
                            match code {
                                NO_CODE => a.blanks += 1,
                                HIDDEN => {}
                                _ => match l[code as usize] {
                                    Lands::Blank => a.blanks += 1,
                                    Lands::Hidden => {}
                                    Lands::Value => a.by_value[code as usize] += 1,
                                    Lands::Bin(b) => a.bins[b] += 1,
                                    Lands::Other => a.other += 1,
                                },
                            }
                        }
                    }
                    progress.step((e - s) as u64)?;
                    Ok((acc, shown))
                },
            )
            .try_reduce(
                || (blank_acc(), 0u64),
                |(a, n), (b, m)| {
                    Ok((
                        a.into_iter().zip(b).map(|(x, y)| x.merge(y)).collect(),
                        n + m,
                    ))
                },
            )?;
        let cards = cards
            .iter()
            .zip(accs)
            .zip(&card_src)
            .enumerate()
            .map(|(k, ((c, a), src))| {
                let mut values: Vec<ValueCount> = match src {
                    Some((cc, _)) if c.kind == FacetKind::Values => a
                        .by_value
                        .iter()
                        .enumerate()
                        .filter(|(_, &n)| n > 0)
                        .map(|(i, &n)| ValueCount {
                            value: cc.values[i].clone(),
                            count: u64::from(n),
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let distinct = values.len() as u64;
                values.sort_unstable_by(|x, y| {
                    y.count
                        .cmp(&x.count)
                        .then_with(|| value::natural_cmp(x.value.as_bytes(), y.value.as_bytes()))
                });
                let many = card_phys[k].is_some() && src.is_none();
                let complete = !many && values.len() <= FACET_VALUES;
                values.truncate(FACET_VALUES);
                Facet {
                    col: c.col,
                    values,
                    distinct,
                    complete,
                    many,
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
fn sort_values(values: &mut [ValueCount], rd: Reader) {
    if values.iter().all(|v| rd.number(&v.value).is_some()) {
        values.sort_by(|a, b| {
            let x = rd.number(&a.value).unwrap_or(0.0);
            let y = rd.number(&b.value).unwrap_or(0.0);
            x.total_cmp(&y).then_with(|| a.value.cmp(&b.value))
        });
    } else if values.iter().all(|v| rd.date(&v.value).is_some()) {
        values.sort_by(|a, b| {
            let x = rd.date(&a.value).unwrap_or(0.0);
            let y = rd.date(&b.value).unwrap_or(0.0);
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
            Reader::plain(ReadOptions::default())
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
    fn facet_cache_recounts_from_codes() {
        // An ID column with more different values than codes are kept for
        // (it is read each time), beside coded columns.
        let n = FACET_CODE_VALUES + 500;
        let mut csv = String::from("id,party,age\r\n");
        for i in 0..n {
            csv.push_str(&format!(
                "{i},{},{}\r\n",
                ["D", "R", "I"][i % 3],
                20 + i % 50
            ));
        }
        let d = Document::from_bytes(csv.into_bytes(), crate::OpenOptions::default(), Some("csv"))
            .unwrap();
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
                ..Default::default()
            },
        ];
        let mut cache = FacetCache::new();
        let all = FilterSpec {
            keep_first: true,
            ..Default::default()
        };
        let f = d
            .facet_counts_cached(&mut cache, &cards, &all, &Progress::new())
            .unwrap();
        assert!(
            cache
                .entries
                .get(&Key::Col(0))
                .is_some_and(|e| e.codes.is_none()),
            "the ID column has no codes"
        );
        assert!(cache
            .entries
            .get(&Key::Col(1))
            .is_some_and(|e| e.codes.is_some()));
        assert_eq!(f.shown as usize, n);
        assert!(f.cards[0].many, "the ID card waits for a search");
        assert!(f.cards[0].values.is_empty() && !f.cards[0].complete);
        assert_eq!(f.cards[0].rows, 0);
        assert_eq!(f.cards[1].values[0].count as usize, n.div_ceil(3));
        assert_eq!(f.cards[2].range, Some((20.0, 69.0)));
        // A click (party R): counted again from the codes, the same as a
        // fresh count.
        let spec = rule(
            1,
            Rule::Values {
                values: vec!["R".into()],
                blanks: false,
            },
        );
        let again = d
            .facet_counts_cached(&mut cache, &cards, &spec, &Progress::new())
            .unwrap();
        let fresh = d.facet_counts(&cards, &spec, &Progress::new()).unwrap();
        assert_eq!(again, fresh);
        assert_eq!(again.shown as usize, n / 3);
        assert_eq!(
            again.cards[1].values.len(),
            3,
            "the party card keeps every party"
        );
        assert_eq!(again.cards[2].bins.iter().sum::<u64>() as usize, n / 3);
        // A rule on the ID column (no codes) works too.
        let spec = rule(
            0,
            cond(Condition::Number {
                op: CmpOp::Lt,
                value: 30.0,
            }),
        );
        let f = d
            .facet_counts_cached(&mut cache, &cards, &spec, &Progress::new())
            .unwrap();
        assert_eq!(f.shown, 30);
        assert_eq!(f.cards[1].values.iter().map(|v| v.count).sum::<u64>(), 30);
        assert!(cache.entries.keys().any(|k| matches!(k, Key::Rule(0, _))));
        // A search on the ID card counts what it keeps.
        let mut searched = cards.clone();
        searched[0].search = Some("6553".into());
        let f = d
            .facet_counts_cached(&mut cache, &searched, &all, &Progress::new())
            .unwrap();
        assert!(!f.cards[0].many);
        assert_eq!(
            f.cards[0].distinct, 16,
            "6553, 65530 to 65539 and 16553 to 56553"
        );
        assert!(
            !cache.entries.keys().any(|k| matches!(k, Key::Rule(..))),
            "a rule no longer used is dropped"
        );
        // A numbers card on the ID column has bars.
        let mut bars = cards.clone();
        bars[0].kind = FacetKind::Numbers;
        let f = d
            .facet_counts_cached(&mut cache, &bars, &spec, &Progress::new())
            .unwrap();
        assert_eq!(f, d.facet_counts(&bars, &spec, &Progress::new()).unwrap());
        assert_eq!(
            f.cards[0].bins.iter().sum::<u64>() as usize,
            n,
            "its own rule does not narrow it"
        );
        // After an edit, only the changed rows are read again, and the
        // counts match a fresh count.
        let mut d = d;
        d.set_cell(1, 1, "Q".into()).unwrap();
        d.set_cell(2, 0, "x".into()).unwrap();
        d.set_cell(3, 2, "".into()).unwrap();
        let rev = d.data_revision();
        assert_eq!(d.changed_since(rev - 3).map(|v| v.len()), Some(3));
        let f = d
            .facet_counts_cached(&mut cache, &bars, &spec, &Progress::new())
            .unwrap();
        assert_eq!(f, d.facet_counts(&bars, &spec, &Progress::new()).unwrap());
        assert!(cache.entries.values().all(|e| e.rev == rev));
        let click = rule(
            1,
            Rule::Values {
                values: vec!["Q".into()],
                blanks: false,
            },
        );
        let f = d
            .facet_counts_cached(&mut cache, &bars, &click, &Progress::new())
            .unwrap();
        assert_eq!(f.shown, 1);
        assert_eq!(f, d.facet_counts(&bars, &click, &Progress::new()).unwrap());
        // A sort moves every row: the codes are built again.
        d.sort(&crate::sort::SortSpec {
            keys: vec![crate::sort::SortKey {
                col: 2,
                descending: false,
                kind: Default::default(),
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(d.changed_since(rev), None);
        let f = d
            .facet_counts_cached(&mut cache, &bars, &click, &Progress::new())
            .unwrap();
        assert_eq!(f, d.facet_counts(&bars, &click, &Progress::new()).unwrap());
        cache.clear();
        assert_eq!(cache.bytes(), 0);
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
