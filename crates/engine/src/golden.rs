//! Golden records: fold the accepted pairs of a one-list Match people run
//! into one row per person, as a new table. The original is never changed.
//!
//! **Which rows merge** is deliberately cautious. The row with the most
//! accepted pairs is a cluster's hub; every other member was accepted with
//! the hub itself, so a chain (A like B, B like C, C like D) never pulls D
//! in. A row with pairs into two clusters, a cluster that lost its hub, and
//! a cluster larger than allowed are left unmerged for review. The result
//! depends only on which rows are paired, never on the order or number of
//! times the pairs are listed.
//!
//! **What a merged row holds** is chosen per column. Values are kept as
//! text; only sums are rewritten. Whenever a rule cannot be applied safely
//! (a date that could be read two ways, a value that is not an exact
//! number) the first value is kept and the cell is flagged.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::reshape::{push_line, too_big, Table, MAX_TABLE_BYTES};
use crate::value::ReadOptions;
use crate::Error;

/// Rows a cluster may have at most, when the person raises the limit.
pub const MAX_CLUSTER: u32 = 100;
/// Longest joined value, in bytes.
const MAX_JOINED: usize = 1 << 20;
/// Digits an exact number may have.
const MAX_DIGITS: usize = 28;
/// Other values listed for a cell of the preview.
const MAX_DIFFER: usize = 20;
/// Review lines kept (the counts stay exact).
pub const MAX_REVIEW_LISTED: usize = 10_000;
const STEP: usize = 1 << 12;

/// How a column's value is chosen. A blank never beats a value, except under
/// [`Rule::Newest`] when asked to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum Rule {
    /// The hub's value, else the first in file order.
    #[default]
    Keep,
    /// The value of the row with the latest date.
    Newest {
        #[serde(default)]
        blank_counts: bool,
    },
    /// The value of the row with the earliest date.
    Oldest,
    Longest,
    MostCommon,
    First,
    Last,
    /// Every different value (exact text), joined.
    Union {
        separator: String,
    },
    Sum,
    Min,
    Max,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnRule {
    pub col: u32,
    pub rule: Rule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoldenSpec {
    pub rules: Vec<ColumnRule>,
    /// The column whose values name merged rows (else file row numbers).
    pub id_col: Option<u32>,
    /// The date column for newest and oldest.
    pub date_col: Option<u32>,
    pub max_cluster: u32,
    /// Only merge clusters in which every pair was accepted.
    pub require_all: bool,
    /// Hubs (row ids) of clusters the person chose not to merge.
    pub skip: Vec<u32>,
    pub keep_first: bool,
    pub read: ReadOptions,
}

impl Default for GoldenSpec {
    fn default() -> GoldenSpec {
        GoldenSpec {
            rules: Vec::new(),
            id_col: None,
            date_col: None,
            max_cluster: 10,
            require_all: false,
            skip: Vec::new(),
            keep_first: true,
            read: ReadOptions::default(),
        }
    }
}

/// Why a rule's value could not be chosen as asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Flag {
    /// A date is blank, unreadable, readable two ways or has a two-digit
    /// year: the first value in file order was kept.
    Date,
    /// A value is not an exact number: the first value was kept.
    NotNumber,
    /// The sum is too large: the first value was kept.
    Overflow,
    /// The joined text is too long: the first value was kept.
    TooLong,
}

impl Flag {
    pub fn words(self) -> &'static str {
        match self {
            Flag::Date => "a date could not be read for certain: first value kept",
            Flag::NotNumber => "not an exact number: first value kept",
            Flag::Overflow => "too large to add up: first value kept",
            Flag::TooLong => "too long to join: first value kept",
        }
    }
}

/// Why rows were left unmerged for review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Accepted pairs lead into two clusters.
    TwoClusters,
    /// The row its pairs were accepted with is itself under review.
    HubUnderReview,
    TooLarge,
    /// "Only merge clusters where every pair was accepted" is ticked.
    NotAllPairs,
}

impl Reason {
    pub fn words(self) -> &'static str {
        match self {
            Reason::TwoClusters => "paired into two clusters",
            Reason::HubUnderReview => "its cluster's main row needs review",
            Reason::TooLarge => "cluster larger than the limit",
            Reason::NotAllPairs => "not every pair in the cluster was accepted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    /// The hub's row id.
    pub hub: u32,
    /// Row ids: the hub first, then the others in file order.
    pub members: Vec<u32>,
    /// Some members were never accepted with each other: joined through the
    /// hub only.
    pub hub_only: bool,
    /// The person chose not to merge it.
    pub skipped: bool,
    /// The merged row's values, in the table's columns.
    pub values: Vec<String>,
    /// (column, why the rule fell back).
    pub flags: Vec<(u32, Flag)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GoldenCounts {
    /// Clusters that will be merged.
    pub clusters: u64,
    /// Rows folded into another.
    pub folded: u64,
    pub hub_only: u64,
    pub skipped: u64,
    /// Rows left for review.
    pub review: u64,
    /// Accepted pairs whose rows end in no common cluster.
    pub unmerged_pairs: u64,
    /// Cells where a rule fell back.
    pub flagged: u64,
    /// Rows of the result, without the title row.
    pub rows: u64,
    /// Hidden rows (not matched, kept as they are).
    pub hidden: u64,
}

#[derive(Debug, Clone)]
pub struct GoldenPlan {
    revision: u64,
    spec: GoldenSpec,
    /// In the file order of each cluster's first row.
    pub clusters: Vec<Cluster>,
    /// (row id, why), in file order.
    pub review: Vec<(u32, Reason)>,
    pub counts: GoldenCounts,
}

impl GoldenPlan {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn spec(&self) -> &GoldenSpec {
        &self.spec
    }
}

fn bad(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

// ---- clusters ----

/// A cluster before values are chosen.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    /// Places (file order) of the hub, then the others in order.
    members: Vec<u32>,
    hub_only: bool,
}

#[derive(Debug, Default)]
struct Shapes {
    clusters: Vec<Shape>,
    review: Vec<(u32, Reason)>,
    unmerged_pairs: u64,
}

/// Cluster rows named by their place in the file.
fn shapes(pairs: &[(u32, u32)], max: usize, require_all: bool) -> Shapes {
    // Each pair once, whichever way round it was listed.
    let mut edges: Vec<(u32, u32)> = pairs
        .iter()
        .filter(|(a, b)| a != b)
        .map(|&(a, b)| (a.min(b), a.max(b)))
        .collect();
    edges.sort_unstable();
    edges.dedup();
    let mut partners: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(a, b) in &edges {
        partners.entry(a).or_default().push(b);
        partners.entry(b).or_default().push(a);
    }
    let mut order: Vec<u32> = partners.keys().copied().collect();
    order.sort_unstable_by_key(|r| (std::cmp::Reverse(partners[r].len()), *r));
    // First every hub claims its free partners.
    let mut of: HashMap<u32, usize> = HashMap::with_capacity(order.len());
    let mut claimed: Vec<Vec<u32>> = Vec::new();
    for &hub in &order {
        if of.contains_key(&hub) {
            continue;
        }
        let n = claimed.len();
        of.insert(hub, n);
        let mut members = vec![hub];
        let mut near = partners[&hub].clone();
        near.sort_unstable();
        for p in near {
            if let std::collections::hash_map::Entry::Vacant(e) = of.entry(p) {
                e.insert(n);
                members.push(p);
            }
        }
        claimed.push(members);
    }
    // Then, with every cluster known, the rows whose pairs lead into two.
    let torn: HashSet<u32> = order
        .iter()
        .copied()
        .filter(|r| {
            let mut seen = partners[r].iter().map(|p| of[p]);
            let first = seen.next();
            seen.any(|c| Some(c) != first)
        })
        .collect();
    let mut out = Shapes::default();
    let mut merged: HashMap<u32, usize> = HashMap::new();
    for members in claimed {
        if members.len() < 2 {
            if torn.contains(&members[0]) {
                out.review.push((members[0], Reason::TwoClusters));
            }
            continue;
        }
        if members.len() > max {
            out.review
                .extend(members.iter().map(|&m| (m, Reason::TooLarge)));
            continue;
        }
        let hub = members[0];
        let hub_torn = torn.contains(&hub);
        let mut keep = Vec::with_capacity(members.len());
        for &m in &members {
            if torn.contains(&m) {
                out.review.push((m, Reason::TwoClusters));
            } else if hub_torn {
                out.review.push((m, Reason::HubUnderReview));
            } else {
                keep.push(m);
            }
        }
        if keep.len() < 2 {
            continue;
        }
        let among = keep.iter().enumerate().all(|(i, &a)| {
            keep[i + 1..]
                .iter()
                .all(|&b| edges.binary_search(&(a.min(b), a.max(b))).is_ok())
        });
        if require_all && !among {
            out.review
                .extend(keep.iter().map(|&m| (m, Reason::NotAllPairs)));
            continue;
        }
        let n = out.clusters.len();
        merged.extend(keep.iter().map(|&m| (m, n)));
        out.clusters.push(Shape {
            members: keep,
            hub_only: !among,
        });
    }
    out.unmerged_pairs = edges
        .iter()
        .filter(|(a, b)| match (merged.get(a), merged.get(b)) {
            (Some(x), Some(y)) => x != y,
            _ => true,
        })
        .count() as u64;
    out.clusters
        .sort_by_key(|c| c.members.iter().copied().min());
    out.review.sort_unstable_by_key(|r| r.0);
    out
}

// ---- rules ----

/// An exact decimal: `value` / 10^`scale`.
#[derive(Debug, Clone, Copy)]
struct Exact {
    value: i128,
    scale: u32,
}

/// A plain number: sign, digits (thousands only in groups of three) and a
/// fraction. No exponent, currency or other mark, 28 digits at most.
fn exact(text: &str, decimal_comma: bool) -> Option<Exact> {
    let (point, group) = if decimal_comma {
        (',', '.')
    } else {
        ('.', ',')
    };
    let t = text.trim();
    let (negative, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (whole, fraction) = match t.split_once(point) {
        Some((w, f)) => (w, Some(f)),
        None => (t, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if fraction.is_some_and(|f| !digits(f)) {
        return None;
    }
    let mut all = String::new();
    if whole.contains(group) {
        for (i, g) in whole.split(group).enumerate() {
            let fits = if i == 0 {
                (1..=3).contains(&g.len())
            } else {
                g.len() == 3
            };
            if !fits || !digits(g) {
                return None;
            }
            all.push_str(g);
        }
    } else if digits(whole) {
        all.push_str(whole);
    } else if !(whole.is_empty() && fraction.is_some()) {
        return None;
    }
    let fraction = fraction.unwrap_or_default();
    all.push_str(fraction);
    if all.trim_start_matches('0').len() > MAX_DIGITS || fraction.len() > MAX_DIGITS {
        return None;
    }
    let value: i128 = all.trim_start_matches('0').parse().unwrap_or(0);
    Some(Exact {
        value: if negative { -value } else { value },
        scale: fraction.len() as u32,
    })
}

impl Exact {
    fn at(self, scale: u32) -> Option<i128> {
        self.value
            .checked_mul(10i128.checked_pow(scale - self.scale)?)
    }

    fn write(value: i128, scale: u32, decimal_comma: bool) -> String {
        let digits = value.unsigned_abs().to_string();
        let width = scale as usize + 1;
        let digits = format!("{digits:0>width$}");
        let cut = digits.len() - scale as usize;
        let mut out = String::new();
        if value < 0 {
            out.push('-');
        }
        out.push_str(&digits[..cut]);
        if scale > 0 {
            out.push(if decimal_comma { ',' } else { '.' });
            out.push_str(&digits[cut..]);
        }
        out
    }
}

fn blank(v: &str) -> bool {
    v.trim().is_empty()
}

/// One column's value for a cluster. `rows`: the members' values and dates
/// in file order; `hub`: which of them is the hub.
fn choose(
    rule: &Rule,
    rows: &[(&str, &str)],
    hub: usize,
    read: ReadOptions,
) -> (String, Option<Flag>) {
    let filled: Vec<&str> = rows.iter().map(|r| r.0).filter(|v| !blank(v)).collect();
    let first = filled.first().copied().unwrap_or(rows[hub].0).to_string();
    let plain = |v: &str| (v.to_string(), None);
    match rule {
        Rule::Keep => {
            if blank(rows[hub].0) {
                (first, None)
            } else {
                plain(rows[hub].0)
            }
        }
        Rule::First => (first, None),
        Rule::Last => plain(filled.last().copied().unwrap_or(rows[hub].0)),
        Rule::Longest => {
            let mut best = (0, None);
            for v in &filled {
                let n = v.chars().count();
                if best.1.is_none() || n > best.0 {
                    best = (n, Some(*v));
                }
            }
            plain(best.1.unwrap_or(rows[hub].0))
        }
        Rule::MostCommon => {
            let mut count: HashMap<&str, u32> = HashMap::new();
            for v in &filled {
                *count.entry(v).or_default() += 1;
            }
            let top = count.values().copied().max();
            match filled.iter().find(|v| Some(count[**v]) == top) {
                Some(v) => plain(v),
                None => (first, None),
            }
        }
        Rule::Union { separator } => {
            let mut seen = HashSet::new();
            let kept: Vec<&str> = filled.iter().copied().filter(|v| seen.insert(*v)).collect();
            let size: usize =
                kept.iter().map(|v| v.len()).sum::<usize>() + separator.len() * kept.len();
            if size > MAX_JOINED {
                (first, Some(Flag::TooLong))
            } else if kept.is_empty() {
                (first, None)
            } else {
                (kept.join(separator), None)
            }
        }
        Rule::Newest { .. } | Rule::Oldest => {
            let blank_counts = matches!(rule, Rule::Newest { blank_counts: true });
            let newest = matches!(rule, Rule::Newest { .. });
            let mut best: Option<(f64, &str)> = None;
            for &(v, date) in rows {
                if blank(v) && !blank_counts {
                    continue;
                }
                let Some(day) = crate::clean::certain_date(date) else {
                    return (first, Some(Flag::Date));
                };
                let better = best.is_none_or(|(b, _)| if newest { day > b } else { day < b });
                if better {
                    best = Some((day, v));
                }
            }
            match best {
                Some((_, v)) => plain(v),
                None => (first, None),
            }
        }
        Rule::Sum | Rule::Min | Rule::Max => {
            let mut numbers = Vec::with_capacity(filled.len());
            for v in &filled {
                match exact(v, read.decimal_comma) {
                    Some(n) => numbers.push((n, *v)),
                    None => return (first, Some(Flag::NotNumber)),
                }
            }
            let Some(scale) = numbers.iter().map(|n| n.0.scale).max() else {
                return (first, None);
            };
            let Some(scaled) = numbers
                .iter()
                .map(|(n, v)| Some((n.at(scale)?, *v)))
                .collect::<Option<Vec<_>>>()
            else {
                return (first, Some(Flag::Overflow));
            };
            match rule {
                Rule::Sum => {
                    let sum = scaled
                        .iter()
                        .try_fold(0i128, |a, n| a.checked_add(n.0))
                        .filter(|s| s.unsigned_abs() < 10u128.pow(MAX_DIGITS as u32));
                    match sum {
                        Some(s) => (Exact::write(s, scale, read.decimal_comma), None),
                        None => (first, Some(Flag::Overflow)),
                    }
                }
                _ => {
                    let mut best = scaled[0];
                    for &n in &scaled[1..] {
                        let better = if *rule == Rule::Min {
                            n.0 < best.0
                        } else {
                            n.0 > best.0
                        };
                        if better {
                            best = n;
                        }
                    }
                    plain(best.1)
                }
            }
        }
    }
}

// ---- the plan ----

/// A cell of a cluster in the preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellView {
    pub col: u32,
    pub value: String,
    /// The members' other values, where they disagree.
    pub differ: Vec<String>,
    pub flag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClusterView {
    pub hub: u32,
    /// What names each member (ID value or file row), the hub first.
    pub members: Vec<String>,
    /// "joined through <hub> only".
    pub hub_only: bool,
    pub skipped: bool,
    /// Cells whose members disagree or that are flagged.
    pub cells: Vec<CellView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewLine {
    pub name: String,
    pub reason: String,
}

impl Document {
    /// How a row is named in `Merged_IDs`: its ID value, else its row number
    /// in the file.
    fn golden_name(&self, id: u32, values: &[String], id_col: Option<u32>) -> String {
        if let Some(v) = id_col.and_then(|c| values.get(c as usize)) {
            if !blank(v) {
                return v.clone();
            }
        }
        if u64::from(id) < self.original_rows() {
            format!("row {}", u64::from(id) + 1)
        } else {
            "added by hand".to_string()
        }
    }

    /// Cluster the accepted `pairs` (row ids) and choose each merged row's
    /// values. Nothing changes.
    pub fn plan_golden(
        &self,
        pairs: &[(u32, u32)],
        spec: &GoldenSpec,
        progress: &Progress,
    ) -> Result<GoldenPlan, Error> {
        self.ready_for_bulk()?;
        let width = self.col_count();
        if !(2..=MAX_CLUSTER).contains(&spec.max_cluster) {
            return Err(bad("a cluster may hold 2 to 100 rows"));
        }
        let used = spec
            .rules
            .iter()
            .map(|r| r.col)
            .chain(spec.id_col)
            .chain(spec.date_col);
        if used.into_iter().any(|c| c >= width) {
            return Err(Error::OutOfRange);
        }
        let mut rules = vec![Rule::Keep; width as usize];
        for r in &spec.rules {
            if let Rule::Union { separator } = &r.rule {
                if separator.len() > 100 {
                    return Err(bad("the separator is too long"));
                }
            }
            if matches!(r.rule, Rule::Newest { .. } | Rule::Oldest) && spec.date_col.is_none() {
                return Err(bad("choose the date column for newest and oldest"));
            }
            rules[r.col as usize] = r.rule.clone();
        }
        if pairs.is_empty() {
            return Err(bad("accept at least one pair"));
        }
        progress.start(self.total_rows() + pairs.len() as u64);
        // Rows by their place in the file, so that nothing depends on the
        // order the pairs came in.
        let wanted: HashSet<u32> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
        let mut place: HashMap<u32, u32> = HashMap::with_capacity(wanted.len());
        let mut at: Vec<u32> = Vec::with_capacity(wanted.len());
        let title = spec.keep_first && self.total_rows() > 0;
        for (n, id) in self.ids().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            if wanted.contains(&id) && !(title && n == 0) {
                place.insert(id, at.len() as u32);
                at.push(id);
            }
        }
        if place.len() != wanted.len() {
            return Err(Error::Changed);
        }
        let placed: Vec<(u32, u32)> = pairs.iter().map(|(a, b)| (place[a], place[b])).collect();
        let found = shapes(&placed, spec.max_cluster as usize, spec.require_all);
        let skip: HashSet<u32> = spec.skip.iter().copied().collect();
        let mut counts = GoldenCounts {
            review: found.review.len() as u64,
            unmerged_pairs: found.unmerged_pairs,
            hidden: self.total_rows() - self.row_count(),
            ..Default::default()
        };
        let mut clusters = Vec::with_capacity(found.clusters.len());
        for (n, shape) in found.clusters.into_iter().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            let members: Vec<u32> = shape.members.iter().map(|&p| at[p as usize]).collect();
            let hub = members[0];
            // In file order for the rules.
            let mut in_order = shape.members.clone();
            in_order.sort_unstable();
            let hub_at = in_order
                .iter()
                .position(|&p| p == shape.members[0])
                .expect("the hub is a member");
            let rows = in_order
                .iter()
                .map(|&p| self.logical_values(at[p as usize]))
                .collect::<Result<Vec<_>, _>>()?;
            let cell = |r: usize, c: usize| rows[r].get(c).map_or("", String::as_str);
            let mut values = Vec::with_capacity(width as usize);
            let mut flags = Vec::new();
            for (c, rule) in rules.iter().enumerate() {
                let column: Vec<(&str, &str)> = (0..rows.len())
                    .map(|r| {
                        (
                            cell(r, c),
                            spec.date_col.map_or("", |d| cell(r, d as usize)),
                        )
                    })
                    .collect();
                let (value, flag) = choose(rule, &column, hub_at, spec.read);
                if let Some(f) = flag {
                    flags.push((c as u32, f));
                }
                values.push(value);
            }
            let skipped = skip.contains(&hub);
            if skipped {
                counts.skipped += 1;
            } else {
                counts.clusters += 1;
                counts.folded += members.len() as u64 - 1;
                counts.flagged += flags.len() as u64;
                counts.hub_only += u64::from(shape.hub_only);
            }
            clusters.push(Cluster {
                hub,
                members,
                hub_only: shape.hub_only,
                skipped,
                values,
                flags,
            });
        }
        counts.rows = self.total_rows() - u64::from(title) - counts.folded;
        let review = found
            .review
            .iter()
            .map(|&(p, why)| (at[p as usize], why))
            .collect();
        Ok(GoldenPlan {
            revision: self.revision(),
            spec: spec.clone(),
            clusters,
            review,
            counts,
        })
    }

    fn golden_current(&self, plan: &GoldenPlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        Ok(())
    }

    /// Clusters `start..start + count` for the preview.
    pub fn golden_page(
        &self,
        plan: &GoldenPlan,
        start: usize,
        count: usize,
    ) -> Result<Vec<ClusterView>, Error> {
        self.golden_current(plan)?;
        let mut out = Vec::new();
        for c in plan.clusters.iter().skip(start).take(count) {
            let rows = c
                .members
                .iter()
                .map(|&id| self.logical_values(id))
                .collect::<Result<Vec<_>, _>>()?;
            let cells = c
                .values
                .iter()
                .enumerate()
                .filter_map(|(col, value)| {
                    let flag = c
                        .flags
                        .iter()
                        .find(|f| f.0 == col as u32)
                        .map(|f| f.1.words().to_string());
                    let mut differ: Vec<String> = Vec::new();
                    let mut disagree = false;
                    for r in &rows {
                        let v = r.get(col).map_or("", String::as_str);
                        if v != value {
                            disagree = true;
                            if differ.len() < MAX_DIFFER && !differ.iter().any(|d| d == v) {
                                differ.push(v.to_string());
                            }
                        }
                    }
                    (disagree || flag.is_some()).then(|| CellView {
                        col: col as u32,
                        value: value.clone(),
                        differ,
                        flag,
                    })
                })
                .collect();
            out.push(ClusterView {
                hub: c.hub,
                members: c
                    .members
                    .iter()
                    .zip(&rows)
                    .map(|(&id, v)| self.golden_name(id, v, plan.spec.id_col))
                    .collect(),
                hub_only: c.hub_only,
                skipped: c.skipped,
                cells,
            });
        }
        Ok(out)
    }

    /// The rows left for review, named, with why (the first
    /// [`MAX_REVIEW_LISTED`]).
    pub fn golden_review(&self, plan: &GoldenPlan) -> Result<Vec<ReviewLine>, Error> {
        self.golden_current(plan)?;
        plan.review
            .iter()
            .take(MAX_REVIEW_LISTED)
            .map(|&(id, why)| {
                Ok(ReviewLine {
                    name: self.golden_name(id, &self.logical_values(id)?, plan.spec.id_col),
                    reason: why.words().to_string(),
                })
            })
            .collect()
    }

    /// The result: every row of the table (hidden ones too), each merged
    /// cluster as one row at its first row's place, with `Merged_IDs` and
    /// `Merged_Count`. With `away`, also the rows folded into a hub, with
    /// their cluster's number and file row.
    pub fn golden_tables(
        &self,
        plan: &GoldenPlan,
        away: bool,
        progress: &Progress,
    ) -> Result<(Table, Option<Table>), Error> {
        self.ready_for_bulk()?;
        self.golden_current(plan)?;
        if plan.counts.clusters == 0 {
            return Err(bad("there is no cluster to merge"));
        }
        let title = plan.spec.keep_first && self.total_rows() > 0;
        let width = self.col_count() as usize;
        let titles = if title {
            self.logical_values(self.ids().next().ok_or(Error::OutOfRange)?)?
        } else {
            (0..width).map(|c| format!("Column {}", c + 1)).collect()
        };
        // A name the table does not have yet.
        let free = |name: &str| {
            let taken = |n: &str| titles.iter().any(|t| t.trim().eq_ignore_ascii_case(n));
            if !taken(name) {
                return name.to_string();
            }
            (2..)
                .map(|k| format!("{name} ({k})"))
                .find(|n| !taken(n))
                .expect("some number is free")
        };
        let mut member: HashMap<u32, usize> = HashMap::new();
        for (n, c) in plan.clusters.iter().enumerate() {
            if !c.skipped {
                member.extend(c.members.iter().map(|&m| (m, n)));
            }
        }
        let mut head: Vec<String> = titles.clone();
        head.resize(width, String::new());
        head.push(free("Merged_IDs"));
        head.push(free("Merged_Count"));
        let mut out = Vec::new();
        push_line(&mut out, &head);
        let mut gone = Vec::new();
        let (mut rows, mut gone_rows) = (1u64, 1u64);
        if away {
            let mut head: Vec<String> = vec!["Cluster".into(), "File row".into()];
            head.extend(titles.iter().cloned());
            head.resize(width + 2, String::new());
            push_line(&mut gone, &head);
        }
        let mut written = vec![false; plan.clusters.len()];
        // The cluster numbers of the "merged away" rows count merged
        // clusters only.
        let mut number = vec![0u64; plan.clusters.len()];
        let mut merged = 0u64;
        progress.start(self.total_rows());
        for (n, id) in self.ids().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            if title && n == 0 {
                continue;
            }
            let Some(&k) = member.get(&id) else {
                let mut line = self.logical_values(id)?;
                line.resize(width, String::new());
                line.push(String::new());
                line.push(String::new());
                push_line(&mut out, &line);
                rows += 1;
                if out.len() > MAX_TABLE_BYTES {
                    return Err(too_big());
                }
                continue;
            };
            let c = &plan.clusters[k];
            if !written[k] {
                written[k] = true;
                merged += 1;
                number[k] = merged;
                let names = c
                    .members
                    .iter()
                    .map(|&m| Ok(self.golden_name(m, &self.logical_values(m)?, plan.spec.id_col)))
                    .collect::<Result<Vec<_>, Error>>()?;
                let mut line = c.values.clone();
                line.push(names.join("; "));
                line.push(c.members.len().to_string());
                push_line(&mut out, &line);
                rows += 1;
            }
            if away && id != c.hub {
                // A folded row's own values, as they were.
                let file_row = if u64::from(id) < self.original_rows() {
                    (u64::from(id) + 1).to_string()
                } else {
                    "added by hand".to_string()
                };
                let mut line = vec![number[k].to_string(), file_row];
                line.extend(self.logical_values(id)?);
                line.resize(width + 2, String::new());
                push_line(&mut gone, &line);
                gone_rows += 1;
            }
            if out.len() + gone.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
        }
        let table = Table {
            bytes: out,
            rows,
            cols: width as u32 + 2,
        };
        let away = away.then(|| Table {
            bytes: gone,
            rows: gone_rows,
            cols: width as u32 + 2,
        });
        Ok((table, away))
    }
}

#[cfg(test)]
#[path = "golden_tests.rs"]
mod tests;
