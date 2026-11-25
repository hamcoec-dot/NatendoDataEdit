//! Pivot tables: any number of row, column and filter fields, any number of
//! value fields with many summaries, date and number grouping, item picks,
//! label and value filters, top N, sorting, subtotals and grand totals,
//! "show values as", number formats, expand / collapse and drill-down.
//!
//! The work is split in two so a live preview stays quick:
//!
//! 1. [`Document::pivot_cube`] reads the file once (in parallel chunks,
//!    cancelable, under a memory budget) and keeps one summary per
//!    distinct combination of row and column items: the "cube".
//! 2. [`Cube::layout`] arranges a cube into the grid shown or written:
//!    filters, sorting, totals, show-as and formatting. It never reads the
//!    file, so changing those settings is instant.

use std::cmp::Ordering as CmpOrdering;
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::coltype::{ReadCtx, Reader};
use crate::document::{Document, Progress};
use crate::fill::{civil_from_days, format_number};
use crate::ops::{check_cols, check_separator, Cols, CHUNK, MAX_CELL_BYTES};
use crate::reshape::{push_line, too_big, Table, MAX_TABLE_BYTES, MAX_TABLE_CELLS};
use crate::value::{natural_cmp, ReadOptions};
use crate::Error;

/// Most row plus column fields.
pub const MAX_FIELDS: usize = 16;
/// Most value fields.
pub const MAX_VALUES: usize = 64;
/// Most report filter fields.
pub const MAX_FILTERS: usize = 32;
/// Most items one item pick may list.
pub const MAX_PICK: usize = 100_000;
/// Most distinct row/column combinations a pivot may hold.
pub const MAX_LEAVES: usize = 10_000_000;
/// Memory a pivot may use while reading and arranging (estimated).
pub const MAX_PIVOT_BYTES: usize = 3 << 30;
/// Most items [`Document::pivot_items`] returns.
pub const MAX_ITEM_LIST: usize = 10_000;
/// Longest filter text.
const MAX_FILTER_TEXT: usize = 1_000;
/// Most text in item picks or collapsed paths.
const MAX_SPEC_BYTES: usize = 16 << 20;

// ---------------------------------------------------------------- spec ----

/// How values of a row, column or filter field are grouped into items.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Grouping {
    /// Each distinct value is an item.
    #[default]
    None,
    Year,
    Quarter,
    Month,
    YearQuarter,
    YearMonth,
    /// The date without its time.
    Day,
    Weekday,
    Hour,
    /// Number ranges of `size` from `start` (values from `end` up share one
    /// item).
    Bins {
        start: f64,
        size: f64,
        #[serde(default)]
        end: Option<f64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    /// By the item itself (numbers and dates in order, text A to Z).
    #[default]
    Label,
    /// By a value field's total for the item.
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldSort {
    pub by: SortBy,
    /// Value field index (sort by value).
    pub value: u32,
    pub desc: bool,
}

/// Which items of a field are used (applied while reading).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemPick {
    /// `items` are the ones left out (else the only ones kept).
    pub exclude: bool,
    /// Item keys, as [`Document::pivot_items`] gives them.
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelOp {
    #[default]
    Equals,
    NotEquals,
    BeginsWith,
    NotBeginsWith,
    EndsWith,
    NotEndsWith,
    Contains,
    NotContains,
    Greater,
    Less,
    Between,
    NotBetween,
}

/// Keeps items whose label matches (case does not matter).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelFilter {
    pub op: LabelOp,
    pub text: String,
    /// Second bound (between).
    pub text2: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmpOp {
    #[default]
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Equals,
    NotEquals,
    Between,
    NotBetween,
}

/// Keeps items whose total of a value field matches.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ValueFilter {
    pub value: u32,
    pub op: CmpOp,
    pub a: f64,
    pub b: f64,
}

/// Keeps only the `n` largest (or smallest) items by a value field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TopN {
    pub n: u32,
    pub value: u32,
    pub bottom: bool,
}

/// A row, column or report filter field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotField {
    pub col: u32,
    pub group: Grouping,
    pub sort: FieldSort,
    pub items: Option<ItemPick>,
    pub label_filter: Option<LabelFilter>,
    pub value_filter: Option<ValueFilter>,
    pub top: Option<TopN>,
    /// Show a subtotal for this field's items (when subtotals are on).
    pub subtotal: bool,
}

impl Default for PivotField {
    fn default() -> Self {
        PivotField {
            col: 0,
            group: Grouping::None,
            sort: FieldSort::default(),
            items: None,
            label_filter: None,
            value_filter: None,
            top: None,
            subtotal: true,
        }
    }
}

/// How a value field summarises the values that land in one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agg {
    /// Cells that are not blank.
    #[default]
    Count,
    /// Rows, blank or not.
    CountRows,
    /// Cells holding a number.
    CountNumbers,
    /// Distinct non-blank values.
    CountDistinct,
    /// Cells that are blank.
    CountBlank,
    Sum,
    Average,
    Min,
    Max,
    Median,
    /// Sample standard deviation.
    StdDev,
    /// Population standard deviation.
    StdDevP,
    Var,
    VarP,
    Product,
    /// Text of the first (or last) row.
    First,
    Last,
    /// All non-blank values joined.
    Join,
}

impl Agg {
    fn is_text(self) -> bool {
        matches!(self, Agg::First | Agg::Last | Agg::Join)
    }

    fn title(self) -> &'static str {
        match self {
            Agg::Count => "Count of",
            Agg::CountRows => "Rows of",
            Agg::CountNumbers => "Count numbers of",
            Agg::CountDistinct => "Distinct count of",
            Agg::CountBlank => "Blank count of",
            Agg::Sum => "Sum of",
            Agg::Average => "Average of",
            Agg::Min => "Min of",
            Agg::Max => "Max of",
            Agg::Median => "Median of",
            Agg::StdDev => "StdDev of",
            Agg::StdDevP => "StdDevP of",
            Agg::Var => "Var of",
            Agg::VarP => "VarP of",
            Agg::Product => "Product of",
            Agg::First => "First of",
            Agg::Last => "Last of",
            Agg::Join => "List of",
        }
    }
}

/// "Show values as".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowAs {
    #[default]
    Value,
    PctOfGrand,
    PctOfColumn,
    PctOfRow,
    PctOfParentRow,
    PctOfParentColumn,
    RunningTotal,
    PctRunningTotal,
    DifferenceFromPrevious,
    PctDifferenceFromPrevious,
    /// 1 = largest among its neighbours.
    RankLargest,
    RankSmallest,
    Index,
}

impl ShowAs {
    fn is_pct(self) -> bool {
        matches!(
            self,
            ShowAs::PctOfGrand
                | ShowAs::PctOfColumn
                | ShowAs::PctOfRow
                | ShowAs::PctOfParentRow
                | ShowAs::PctOfParentColumn
                | ShowAs::PctRunningTotal
                | ShowAs::PctDifferenceFromPrevious
        )
    }
}

/// Which neighbours running totals, differences and ranks go along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Along {
    /// Down the rows.
    #[default]
    Rows,
    /// Across the columns.
    Columns,
}

/// How numbers are written.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NumFormat {
    /// Digits after the decimal point (none: as many as needed).
    pub decimals: Option<u8>,
    /// Group thousands (1,234,567).
    pub thousands: bool,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ValueField {
    pub col: u32,
    pub agg: Agg,
    pub show_as: ShowAs,
    pub along: Along,
    /// Heading (empty: "Sum of <column>").
    pub name: String,
    pub format: NumFormat,
    /// Between joined values.
    pub separator: String,
}

impl Default for ValueField {
    fn default() -> Self {
        ValueField {
            col: 0,
            agg: Agg::Count,
            show_as: ShowAs::Value,
            along: Along::Rows,
            name: String::new(),
            format: NumFormat::default(),
            separator: ", ".into(),
        }
    }
}

/// Totals, labels and what is collapsed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotLayout {
    /// Grand total row at the bottom.
    pub grand_row: bool,
    /// Grand total column at the right.
    pub grand_col: bool,
    /// Subtotals for row fields.
    pub row_subtotals: bool,
    /// Subtotals for column fields.
    pub col_subtotals: bool,
    /// Row subtotals on the group's own row (else on a "Total" row below).
    pub subtotals_top: bool,
    /// Write outer row labels on every row.
    pub repeat_labels: bool,
    /// Label for blank items.
    pub blank_label: String,
    /// Text for cells with no data.
    pub empty_cells: String,
    /// Levels of row (column) items shown; deeper ones start collapsed
    /// (0: all).
    pub row_depth: u32,
    pub col_depth: u32,
    /// Item key paths collapsed (or expanded past the depth).
    pub collapsed_rows: Vec<Vec<String>>,
    pub expanded_rows: Vec<Vec<String>>,
    pub collapsed_cols: Vec<Vec<String>>,
    pub expanded_cols: Vec<Vec<String>>,
}

impl Default for PivotLayout {
    fn default() -> Self {
        PivotLayout {
            grand_row: true,
            grand_col: true,
            row_subtotals: true,
            col_subtotals: true,
            subtotals_top: true,
            repeat_labels: false,
            blank_label: "(blank)".into(),
            empty_cells: String::new(),
            row_depth: 0,
            col_depth: 0,
            collapsed_rows: Vec::new(),
            expanded_rows: Vec::new(),
            collapsed_cols: Vec::new(),
            expanded_cols: Vec::new(),
        }
    }
}

/// A whole pivot table: what to read and how to lay it out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PivotSpec {
    pub rows: Vec<PivotField>,
    pub cols: Vec<PivotField>,
    pub values: Vec<ValueField>,
    /// Report filters (fields that only pick which rows count).
    pub filters: Vec<PivotField>,
    pub keep_first: bool,
    pub read: ReadOptions,
    /// "Smith" and "SMITH" are different items.
    pub match_case: bool,
    /// Spaces at the start and end count.
    pub keep_spaces: bool,
    pub layout: PivotLayout,
}

impl Default for PivotSpec {
    fn default() -> Self {
        PivotSpec {
            rows: Vec::new(),
            cols: Vec::new(),
            values: Vec::new(),
            filters: Vec::new(),
            keep_first: true,
            read: ReadOptions::default(),
            match_case: false,
            keep_spaces: false,
            layout: PivotLayout::default(),
        }
    }
}

/// The parts of a spec that need the file read again when they change.
#[derive(Serialize)]
struct ScanKey<'a> {
    dims: Vec<(u32, Grouping, &'a Option<ItemPick>)>,
    nr: usize,
    filters: Vec<(u32, Grouping, &'a Option<ItemPick>)>,
    values: Vec<(u32, Agg, &'a str)>,
    keep_first: bool,
    read: ReadOptions,
    match_case: bool,
    keep_spaces: bool,
    sample: Option<u64>,
}

impl PivotSpec {
    /// A key that changes only when the file must be read again (for a
    /// cube cache). Add the document's revision.
    pub fn scan_key(&self, sample: Option<u64>) -> String {
        fn dim(p: &PivotField) -> (u32, Grouping, &Option<ItemPick>) {
            (p.col, p.group, &p.items)
        }
        let key = ScanKey {
            dims: self.rows.iter().chain(&self.cols).map(dim).collect(),
            nr: self.rows.len(),
            filters: self.filters.iter().map(dim).collect(),
            values: self
                .values
                .iter()
                .map(|v| (v.col, v.agg, v.separator.as_str()))
                .collect(),
            keep_first: self.keep_first,
            read: self.read,
            match_case: self.match_case,
            keep_spaces: self.keep_spaces,
            sample,
        };
        serde_json::to_string(&key).unwrap_or_default()
    }

    /// Every limit that needs no document: run before reading and again
    /// before every layout (a cached cube may meet a new spec).
    pub fn check_limits(&self) -> Result<(), Error> {
        if self.rows.len() + self.cols.len() > MAX_FIELDS {
            return Err(Error::Invalid(format!(
                "a pivot can have at most {MAX_FIELDS} row and column fields"
            )));
        }
        if self.values.len() > MAX_VALUES {
            return Err(Error::Invalid(format!(
                "a pivot can have at most {MAX_VALUES} value fields"
            )));
        }
        if self.filters.len() > MAX_FILTERS {
            return Err(Error::Invalid(format!(
                "a pivot can have at most {MAX_FILTERS} filter fields"
            )));
        }
        let nv = self.values.len() as u32;
        for f in self.rows.iter().chain(&self.cols).chain(&self.filters) {
            check_field(f, nv)?;
        }
        for v in &self.values {
            check_separator(&v.separator)?;
            if v.name.len() > MAX_FILTER_TEXT
                || v.format.prefix.len() > 64
                || v.format.suffix.len() > 64
            {
                return Err(Error::Invalid(
                    "a value field's name or format is too long".into(),
                ));
            }
            if v.format.decimals.is_some_and(|d| d > 15) {
                return Err(Error::Invalid("at most 15 decimals".into()));
            }
        }
        let l = &self.layout;
        if l.blank_label.len() > MAX_FILTER_TEXT || l.empty_cells.len() > MAX_FILTER_TEXT {
            return Err(Error::Invalid("a label is too long".into()));
        }
        let paths = l.collapsed_rows.len()
            + l.expanded_rows.len()
            + l.collapsed_cols.len()
            + l.expanded_cols.len();
        if paths > MAX_PICK {
            return Err(Error::Invalid("too many collapsed items".into()));
        }
        let path_bytes: usize = [
            &l.collapsed_rows,
            &l.expanded_rows,
            &l.collapsed_cols,
            &l.expanded_cols,
        ]
        .iter()
        .flat_map(|v| v.iter())
        .map(|p| p.len() * 24 + p.iter().map(String::len).sum::<usize>())
        .sum();
        let pick_bytes: usize = self
            .rows
            .iter()
            .chain(&self.cols)
            .chain(&self.filters)
            .filter_map(|f| f.items.as_ref())
            .map(|p| p.items.iter().map(|i| i.len() + 24).sum::<usize>())
            .sum();
        if path_bytes > MAX_SPEC_BYTES || pick_bytes > MAX_SPEC_BYTES {
            return Err(Error::Invalid("the pivot's item lists are too long".into()));
        }
        Ok(())
    }

    fn check(&self, d: &Document) -> Result<(), Error> {
        self.check_limits()?;
        let cols: Vec<u32> = self.used_cols();
        check_cols(d, &cols)
    }
}

fn check_field(f: &PivotField, nv: u32) -> Result<(), Error> {
    if let Grouping::Bins { start, size, end } = f.group {
        if !start.is_finite()
            || !size.is_finite()
            || size <= 0.0
            || end.is_some_and(|e| !e.is_finite() || e <= start)
        {
            return Err(Error::Invalid(
                "number groups need a start, a size above 0 and an end above the start".into(),
            ));
        }
    }
    if let Some(p) = &f.items {
        if p.items.len() > MAX_PICK {
            return Err(Error::Invalid(format!(
                "at most {MAX_PICK} items can be picked"
            )));
        }
    }
    if let Some(l) = &f.label_filter {
        if l.text.len() > MAX_FILTER_TEXT || l.text2.len() > MAX_FILTER_TEXT {
            return Err(Error::Invalid("the filter text is too long".into()));
        }
    }
    let bad = |i: u32| i >= nv;
    if f.sort.by == SortBy::Value && bad(f.sort.value)
        || f.value_filter.is_some_and(|v| bad(v.value))
        || f.top.is_some_and(|t| bad(t.value))
    {
        return Err(Error::Invalid(
            "a sort or filter uses a value field that is not in Values".into(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------- grouping ----

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/// Key of values a date or number grouping cannot read.
const OTHER_KEY: &str = "\u{1}other";

/// Reads one field's values into item keys.
struct Keyer {
    group: Grouping,
    rd: Reader,
    match_case: bool,
    trim: bool,
}

impl Keyer {
    fn new(f: &PivotField, spec: &PivotSpec, ctx: &ReadCtx) -> Keyer {
        Keyer {
            group: f.group,
            rd: ctx.reader(f.col),
            match_case: spec.match_case,
            trim: !spec.keep_spaces,
        }
    }

    fn text<'t>(&self, t: &'t str) -> &'t str {
        if self.trim {
            t.trim()
        } else {
            t
        }
    }

    /// The item key of a cell's text into `out` ("" = blank).
    fn key(&self, t: &str, out: &mut String) {
        out.clear();
        let t = self.text(t);
        if t.is_empty() {
            return;
        }
        match self.group {
            Grouping::None => {
                if self.match_case {
                    out.push_str(t);
                } else if t.is_ascii() {
                    out.extend(t.bytes().map(|b| b.to_ascii_lowercase() as char));
                } else {
                    out.extend(t.chars().flat_map(char::to_lowercase));
                }
            }
            Grouping::Bins { start, size, end } => {
                use std::fmt::Write;
                match self.rd.number(t) {
                    None => out.push_str(OTHER_KEY),
                    Some(x) if x < start => out.push('<'),
                    Some(x) if end.is_some_and(|e| x >= e) => out.push('>'),
                    Some(x) => {
                        let k = ((x - start) / size).floor();
                        let _ = write!(out, "b{k}");
                    }
                }
            }
            g => {
                use std::fmt::Write;
                let Some(days) = self.rd.date(t) else {
                    out.push_str(OTHER_KEY);
                    return;
                };
                let whole = days.floor();
                let (y, m, d) = civil_from_days(whole as i64);
                let _ = match g {
                    Grouping::Year => write!(out, "{y:04}"),
                    Grouping::Quarter => write!(out, "q{}", (m - 1) / 3 + 1),
                    Grouping::Month => write!(out, "m{m:02}"),
                    Grouping::YearQuarter => write!(out, "{y:04}-q{}", (m - 1) / 3 + 1),
                    Grouping::YearMonth => write!(out, "{y:04}-{m:02}"),
                    Grouping::Day => write!(out, "{y:04}-{m:02}-{d:02}"),
                    Grouping::Weekday => {
                        // 1970-01-01 was a Thursday (index 3 from Monday).
                        write!(out, "w{}", (whole as i64 + 3).rem_euclid(7) + 1)
                    }
                    Grouping::Hour => {
                        let h = ((days - whole) * 24.0 + 1e-9).floor().clamp(0.0, 23.0) as u32;
                        write!(out, "h{h:02}")
                    }
                    Grouping::None | Grouping::Bins { .. } => unreachable!(),
                };
            }
        }
    }

    /// Label and sort number (NaN: sort as text) of a new item.
    fn label(&self, t: &str, key: &str) -> (String, f64) {
        if key.is_empty() {
            return (String::new(), f64::NAN);
        }
        if key == OTHER_KEY {
            let l = if matches!(self.group, Grouping::Bins { .. }) {
                "(not a number)"
            } else {
                "(not a date)"
            };
            return (l.into(), f64::INFINITY);
        }
        let num = |s: &str| s.parse::<f64>().unwrap_or(f64::NAN);
        match self.group {
            Grouping::None => {
                let t = self.text(t);
                let sort = self
                    .rd
                    .number(t)
                    .or_else(|| self.rd.date(t))
                    .unwrap_or(f64::NAN);
                (t.to_string(), sort)
            }
            Grouping::Bins { start, size, end } => match key {
                "<" => (format!("< {}", fmt_plain(start)), f64::NEG_INFINITY),
                ">" => {
                    let e = end.unwrap_or(start);
                    (format!("{} and up", fmt_plain(e)), f64::MAX)
                }
                _ => {
                    let k = num(&key[1..]);
                    let lo = start + k * size;
                    let hi = lo + size;
                    let whole = |x: f64| x == x.trunc() && x.abs() < 1e15;
                    let label = if whole(start) && whole(size) {
                        format!("{}-{}", fmt_plain(lo), fmt_plain(hi - 1.0))
                    } else {
                        format!("{} to under {}", fmt_plain(lo), fmt_plain(hi))
                    };
                    (label, lo)
                }
            },
            Grouping::Year => (key.into(), num(key)),
            Grouping::Quarter => (format!("Q{}", &key[1..]), num(&key[1..])),
            Grouping::Month => {
                let m = num(&key[1..]);
                (MONTHS[(m as usize).clamp(1, 12) - 1].into(), m)
            }
            Grouping::YearQuarter => {
                let y = num(&key[..4]);
                let q = num(&key[6..]);
                (format!("{} Q{}", &key[..4], &key[6..]), y * 10.0 + q)
            }
            Grouping::YearMonth => {
                let y = num(&key[..4]);
                let m = num(&key[5..]);
                (key.into(), y * 100.0 + m)
            }
            Grouping::Day => {
                let sort = Reader::plain(ReadOptions::default())
                    .date(key)
                    .unwrap_or(f64::NAN);
                (key.into(), sort)
            }
            Grouping::Weekday => {
                let w = num(&key[1..]);
                (WEEKDAYS[(w as usize).clamp(1, 7) - 1].into(), w)
            }
            Grouping::Hour => (format!("{}:00", &key[1..]), num(&key[1..])),
        }
    }
}

/// A plain number (no grouping, dot decimals) for group labels.
fn fmt_plain(x: f64) -> String {
    format_number(x, false)
}

/// An item key pick as a set (keys normalised like [`Keyer::key`]).
struct Pick {
    exclude: bool,
    items: HashSet<String>,
}

impl Pick {
    fn new(p: &Option<ItemPick>) -> Option<Pick> {
        p.as_ref().map(|p| Pick {
            exclude: p.exclude,
            items: p.items.iter().cloned().collect(),
        })
    }

    fn keeps(&self, key: &str) -> bool {
        self.items.contains(key) != self.exclude
    }
}

// ------------------------------------------------------- accumulators ----

/// The running summary of one value field in one cell.
#[derive(Debug, Clone)]
enum Acc {
    N(u64),
    Sum {
        s: f64,
        c: f64,
        n: u64,
    },
    Min {
        x: f64,
        n: u64,
    },
    Max {
        x: f64,
        n: u64,
    },
    Mom {
        n: u64,
        mean: f64,
        m2: f64,
    },
    Prod {
        p: f64,
        n: u64,
    },
    Vals(Vec<f64>),
    /// Value hashes (distinct count); sorted and unique once reading ends.
    Ids(Vec<u128>),
    /// A summary already worked out (median / distinct count of a total).
    Done(Option<f64>),
    /// First / last: the row number and its text (`u64::MAX`: none yet).
    Pick {
        row: u64,
        text: Box<str>,
    },
    Join {
        row: u64,
        text: String,
    },
}

fn neumaier(s: &mut f64, c: &mut f64, x: f64) {
    let t = *s + x;
    if s.abs() >= x.abs() {
        *c += (*s - t) + x;
    } else {
        *c += (x - t) + *s;
    }
    *s = t;
}

/// Bytes one distinct-count entry holds.
/// (16 bytes, charged with room for the vector to grow.)
const ID_BYTES: usize = 24;
/// One number kept for a median (8 bytes, with room to grow).
const VAL_BYTES: usize = 12;

/// Sort and drop repeats now and then, so repeats never pile up.
fn squeeze(v: &mut Vec<u128>, force: bool) {
    if force || (v.len() >= 1024 && v.len().is_power_of_two()) {
        v.sort_unstable();
        v.dedup();
    }
}

/// Medians and distinct counts of totals are worked out from the leaves'
/// values in one pass per level, never by copying them into every total.
fn is_heavy(agg: Agg) -> bool {
    matches!(agg, Agg::Median | Agg::CountDistinct)
}

impl Acc {
    fn new(agg: Agg) -> Acc {
        match agg {
            Agg::Count | Agg::CountRows | Agg::CountNumbers | Agg::CountBlank => Acc::N(0),
            Agg::Sum | Agg::Average => Acc::Sum {
                s: 0.0,
                c: 0.0,
                n: 0,
            },
            Agg::Min => Acc::Min {
                x: f64::INFINITY,
                n: 0,
            },
            Agg::Max => Acc::Max {
                x: f64::NEG_INFINITY,
                n: 0,
            },
            Agg::StdDev | Agg::StdDevP | Agg::Var | Agg::VarP => Acc::Mom {
                n: 0,
                mean: 0.0,
                m2: 0.0,
            },
            Agg::Product => Acc::Prod { p: 1.0, n: 0 },
            Agg::Median => Acc::Vals(Vec::new()),
            Agg::CountDistinct => Acc::Ids(Vec::new()),
            Agg::First | Agg::Last => Acc::Pick {
                row: u64::MAX,
                text: "".into(),
            },
            Agg::Join => Acc::Join {
                row: u64::MAX,
                text: String::new(),
            },
        }
    }

    /// Add one cell. Returns the bytes newly held outside the accumulator.
    fn add(&mut self, v: &VSpec, t: &str, row: u64, hs: &(RandomState, RandomState)) -> usize {
        let blank = t.is_empty();
        let num = || v.rd.number(t);
        match self {
            Acc::N(n) => {
                let hit = match v.agg {
                    Agg::Count => !blank,
                    Agg::CountRows => true,
                    Agg::CountBlank => blank,
                    _ => !blank && num().is_some(),
                };
                *n += u64::from(hit);
                0
            }
            Acc::Sum { s, c, n } => {
                if let Some(x) = num().filter(|_| !blank) {
                    neumaier(s, c, x);
                    *n += 1;
                }
                0
            }
            Acc::Min { x, n } => {
                if let Some(y) = num().filter(|_| !blank) {
                    *x = x.min(y);
                    *n += 1;
                }
                0
            }
            Acc::Max { x, n } => {
                if let Some(y) = num().filter(|_| !blank) {
                    *x = x.max(y);
                    *n += 1;
                }
                0
            }
            Acc::Mom { n, mean, m2 } => {
                if let Some(x) = num().filter(|_| !blank) {
                    *n += 1;
                    let d = x - *mean;
                    *mean += d / *n as f64;
                    *m2 += d * (x - *mean);
                }
                0
            }
            Acc::Prod { p, n } => {
                if let Some(x) = num().filter(|_| !blank) {
                    *p *= x;
                    *n += 1;
                }
                0
            }
            Acc::Vals(vs) => match num().filter(|_| !blank) {
                Some(x) => {
                    vs.push(x);
                    VAL_BYTES
                }
                None => 0,
            },
            Acc::Ids(ids) => {
                if blank {
                    return 0;
                }
                // The same key as row items: case does not matter unless
                // asked (spaces at the ends are already gone unless kept).
                let lower: String;
                let t = if v.match_case
                    || (t.is_ascii() && !t.bytes().any(|b| b.is_ascii_uppercase()))
                {
                    t
                } else {
                    lower = t.chars().flat_map(char::to_lowercase).collect();
                    &lower
                };
                ids.push((u128::from(hs.0.hash_one(t)) << 64) | u128::from(hs.1.hash_one(t)));
                squeeze(ids, false);
                ID_BYTES
            }
            Acc::Done(_) => 0,
            Acc::Pick { row: r, text } => {
                if blank {
                    return 0;
                }
                let take = *r == u64::MAX
                    || (v.agg == Agg::First && row < *r)
                    || (v.agg == Agg::Last && row > *r);
                if take {
                    let grown = t.len().saturating_sub(text.len());
                    *r = row;
                    *text = t.into();
                    grown
                } else {
                    0
                }
            }
            Acc::Join { row: r, text } => {
                if blank {
                    return 0;
                }
                if *r == u64::MAX {
                    *r = row;
                } else {
                    text.push_str(&v.separator);
                }
                text.push_str(t);
                t.len() + v.separator.len()
            }
        }
    }

    /// Merge `o` (any order of rows) into `self`; returns bytes newly held.
    fn merge(&mut self, o: &Acc, v: &VSpec) -> Result<usize, Error> {
        Ok(match (self, o) {
            (Acc::N(a), Acc::N(b)) => {
                *a += b;
                0
            }
            (
                Acc::Sum { s, c, n },
                Acc::Sum {
                    s: s2,
                    c: c2,
                    n: n2,
                },
            ) => {
                neumaier(s, c, *s2);
                *c += c2;
                *n += n2;
                0
            }
            (Acc::Min { x, n }, Acc::Min { x: y, n: m }) => {
                *x = x.min(*y);
                *n += m;
                0
            }
            (Acc::Max { x, n }, Acc::Max { x: y, n: m }) => {
                *x = x.max(*y);
                *n += m;
                0
            }
            (
                Acc::Mom { n, mean, m2 },
                Acc::Mom {
                    n: nb,
                    mean: mb,
                    m2: m2b,
                },
            ) => {
                if *nb > 0 {
                    let na = *n as f64;
                    let nbf = *nb as f64;
                    let tot = na + nbf;
                    let d = mb - *mean;
                    *mean += d * nbf / tot;
                    *m2 += m2b + d * d * na * nbf / tot;
                    *n += nb;
                }
                0
            }
            (Acc::Prod { p, n }, Acc::Prod { p: q, n: m }) => {
                if *m > 0 {
                    *p *= q;
                    *n += m;
                }
                0
            }
            (Acc::Vals(a), Acc::Vals(b)) => {
                a.extend_from_slice(b);
                b.len() * VAL_BYTES
            }
            (Acc::Ids(a), Acc::Ids(b)) => {
                a.extend_from_slice(b);
                squeeze(a, a.len() > 2 * b.len() && a.len() >= 1024);
                b.len() * ID_BYTES
            }
            (Acc::Done(_), _) => 0,
            (Acc::Pick { row, text }, Acc::Pick { row: r2, text: t2 }) => {
                let take = *r2 != u64::MAX
                    && (*row == u64::MAX
                        || (v.agg == Agg::First && r2 < row)
                        || (v.agg == Agg::Last && r2 > row));
                if take {
                    *row = *r2;
                    *text = t2.clone();
                    t2.len()
                } else {
                    0
                }
            }
            (Acc::Join { row, text }, Acc::Join { row: r2, text: t2 }) => {
                if *r2 == u64::MAX {
                    return Ok(0);
                }
                let grown = t2.len() + v.separator.len();
                if text.len() + grown > MAX_CELL_BYTES {
                    return Err(Error::Invalid(
                        "a joined cell would be too long; turn off totals or pick another summary"
                            .into(),
                    ));
                }
                if *row == u64::MAX {
                    *row = *r2;
                    text.push_str(t2);
                } else if *r2 < *row {
                    let mut s = String::with_capacity(text.len() + grown);
                    s.push_str(t2);
                    s.push_str(&v.separator);
                    s.push_str(text);
                    *text = s;
                    *row = *r2;
                } else {
                    text.push_str(&v.separator);
                    text.push_str(t2);
                }
                grown
            }
            _ => unreachable!("accumulators of one value field have one kind"),
        })
    }

    /// The summary as a number (None: no data or not a number summary).
    fn number(&self, agg: Agg) -> Option<f64> {
        match self {
            Acc::N(n) => Some(*n as f64),
            Acc::Sum { s, c, n } => {
                if *n == 0 {
                    None
                } else if agg == Agg::Average {
                    Some((s + c) / *n as f64)
                } else {
                    Some(s + c)
                }
            }
            Acc::Min { x, n } | Acc::Max { x, n } => (*n > 0).then_some(*x),
            Acc::Mom { n, m2, .. } => {
                let n = *n as f64;
                let var = match agg {
                    Agg::StdDev | Agg::Var if n >= 2.0 => m2 / (n - 1.0),
                    Agg::StdDevP | Agg::VarP if n >= 1.0 => m2 / n,
                    _ => return None,
                };
                let var = var.max(0.0);
                Some(if matches!(agg, Agg::StdDev | Agg::StdDevP) {
                    var.sqrt()
                } else {
                    var
                })
            }
            Acc::Prod { p, n } => (*n > 0).then_some(*p),
            Acc::Vals(v) => median_sorted(v),
            Acc::Ids(v) => Some(v.len() as f64),
            Acc::Done(x) => *x,
            Acc::Pick { .. } | Acc::Join { .. } => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Acc::Pick { row, text } if *row != u64::MAX => Some(text),
            Acc::Join { row, text } if *row != u64::MAX => Some(text),
            _ => None,
        }
    }
}

/// Median of values sorted ascending (a cube's leaf).
fn median_sorted(v: &[f64]) -> Option<f64> {
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2]),
        _ => Some((v[n / 2 - 1] + v[n / 2]) / 2.0),
    }
}

/// Median of values in any order (taken, not copied).
fn median_owned(mut w: Vec<f64>) -> Option<f64> {
    if w.is_empty() {
        return None;
    }
    let n = w.len();
    let mid = n / 2;
    let cmp = |a: &f64, b: &f64| a.total_cmp(b);
    let (lower, m, _) = w.select_nth_unstable_by(mid, cmp);
    let m = *m;
    if n % 2 == 1 {
        Some(m)
    } else {
        let lo = lower.iter().copied().max_by(cmp).unwrap_or(m);
        Some((lo + m) / 2.0)
    }
}

/// What reading needs of a value field.
struct VSpec {
    agg: Agg,
    rd: Reader,
    separator: String,
    /// Distinct count: "Ann" and "ann" differ (like row items).
    match_case: bool,
}

// --------------------------------------------------------------- cube ----

/// One item of a row or column field.
#[derive(Debug, Clone)]
struct Item {
    key: Box<str>,
    label: Box<str>,
    /// First row it was seen on (its label is that row's spelling).
    first: u64,
    /// Sort number (NaN: text).
    sort: f64,
    /// Place in label order (set once reading ends).
    rank: u32,
}

/// What reading the file found: one summary per combination of items.
pub struct Cube {
    nr: usize,
    nc: usize,
    aggs: Vec<VSpec>,
    items: Vec<Vec<Item>>,
    /// Item ids of each leaf (`nr + nc` per leaf).
    keys: Vec<u32>,
    /// `aggs.len()` per leaf.
    accs: Vec<Acc>,
    /// Rows read and rows that passed the filters.
    pub rows_read: u64,
    pub rows_used: u64,
    /// Only the first rows were read (a quick preview).
    pub sampled: bool,
    /// Estimated bytes held by the cube itself.
    pub bytes: usize,
    /// Bytes charged now: the cube, its kept totals and any layout running.
    used: AtomicUsize,
    cap: usize,
    /// Totals of the last layout's filters (key, totals, their bytes).
    #[allow(clippy::type_complexity)]
    memo: Mutex<Option<(String, Arc<Memo>, usize)>>,
}

/// A chunk's (or a merge of chunks') partial cube.
/// A quick hasher for keys made by the pivot itself (item ids), keyed at
/// random so no file can choose colliding keys.
#[derive(Clone, Copy)]
struct Quick(u64);

impl Default for Quick {
    fn default() -> Self {
        let mut h = RandomState::new().build_hasher();
        h.write_u8(0);
        Quick(h.finish() | 1)
    }
}

struct QuickHasher(u64, u64);

impl BuildHasher for Quick {
    type Hasher = QuickHasher;
    fn build_hasher(&self) -> QuickHasher {
        QuickHasher(self.0, 0)
    }
}

impl Hasher for QuickHasher {
    fn finish(&self) -> u64 {
        let x = (self.1 ^ self.0).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let y = ((x as u128) * (u128::from(self.0) | 1)) as u64 ^ (x >> 29);
        y.wrapping_mul(0xbf58_476d_1ce4_e5b9) ^ (y >> 31)
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.1 = (self.1.rotate_left(8) ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    fn write_u128(&mut self, x: u128) {
        // Each half goes through a keyed folded multiply, so every input
        // bit reaches every output bit.
        let fold = |a: u64, b: u64| {
            let m = u128::from(a) * u128::from(b);
            (m as u64) ^ ((m >> 64) as u64)
        };
        let k = self.0;
        let lo = fold(x as u64 ^ k, 0x9e37_79b9_7f4a_7c15 ^ k.rotate_left(17));
        let hi = fold(
            ((x >> 64) as u64) ^ lo,
            0xc2b2_ae3d_27d4_eb4f ^ k.rotate_left(41),
        );
        self.1 = fold(self.1 ^ hi, 0x1656_67b1_9e37_79f9 | 1);
    }
}

/// Leaf index by item-id tuple: packed into one number for up to four
/// fields, boxed for more.
enum LeafMap {
    Packed(HashMap<u128, u32, Quick>),
    Boxed(HashMap<Box<[u32]>, u32>),
}

fn pack(t: &[u32]) -> u128 {
    t.iter().fold(0u128, |a, &x| (a << 32) | u128::from(x))
}

impl LeafMap {
    fn new(nd: usize) -> LeafMap {
        if nd <= 4 {
            LeafMap::Packed(HashMap::default())
        } else {
            LeafMap::Boxed(HashMap::new())
        }
    }

    fn get(&self, t: &[u32]) -> Option<u32> {
        match self {
            LeafMap::Packed(m) => m.get(&pack(t)).copied(),
            LeafMap::Boxed(m) => m.get(t).copied(),
        }
    }

    fn insert(&mut self, t: &[u32], l: u32) {
        match self {
            LeafMap::Packed(m) => {
                m.insert(pack(t), l);
            }
            LeafMap::Boxed(m) => {
                m.insert(t.into(), l);
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            LeafMap::Packed(m) => m.len(),
            LeafMap::Boxed(m) => m.len(),
        }
    }
}

struct Part {
    dicts: Vec<HashMap<Box<str>, u32>>,
    items: Vec<Vec<Item>>,
    map: LeafMap,
    keys: Vec<u32>,
    accs: Vec<Acc>,
    read: u64,
    used: u64,
}

impl Part {
    fn new(nd: usize) -> Part {
        Part {
            dicts: vec![HashMap::new(); nd],
            items: vec![Vec::new(); nd],
            map: LeafMap::new(nd),
            keys: Vec::new(),
            accs: Vec::new(),
            read: 0,
            used: 0,
        }
    }

    fn leaves(&self) -> usize {
        self.map.len()
    }
}

/// Bytes charged for one new item and one new leaf.
/// (Measured: dictionary entry, item, both strings' headers, and room for
/// the tables to grow.)
const ITEM_COST: usize = 160;
const LEAF_COST: usize = 64;

/// Bytes charged for a new leaf.
fn leaf_cost(nd: usize, nv: usize) -> usize {
    LEAF_COST + nd * 6 + nv * std::mem::size_of::<Acc>() * 3 / 2
}

struct Budget<'a> {
    used: &'a AtomicUsize,
    cap: usize,
}

impl Budget<'_> {
    fn release(&self, n: usize) {
        let mut cur = self.used.load(Ordering::Relaxed);
        while let Err(now) = self.used.compare_exchange_weak(
            cur,
            cur.saturating_sub(n),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            cur = now;
        }
    }

    fn charge(&self, n: usize) -> Result<(), Error> {
        if n == 0 {
            return Ok(());
        }
        let now = self.used.fetch_add(n, Ordering::Relaxed) + n;
        if now > self.cap {
            Err(Error::Invalid(
                "this pivot would need too much memory; use fewer or grouped fields, or turn off median / distinct count"
                    .into(),
            ))
        } else {
            Ok(())
        }
    }
}

/// Memory a pivot may use on this computer: [`MAX_PIVOT_BYTES`], or a
/// quarter of the installed memory if that is less.
pub fn pivot_budget() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| match crate::sysmem::installed_bytes() {
        Some(ram) => MAX_PIVOT_BYTES.min(usize::try_from(ram / 4).unwrap_or(usize::MAX)),
        None => MAX_PIVOT_BYTES,
    })
}

fn too_many_leaves() -> Error {
    Error::Invalid(format!(
        "this pivot has more than {} combinations of items; group the values or use fewer fields",
        MAX_LEAVES
    ))
}

impl Document {
    /// Read the file into a cube for `spec` (only its fields, groups, item
    /// picks and value summaries matter). `sample`: read only that many
    /// rows, for a quick first preview of a big file.
    pub fn pivot_cube(
        &self,
        spec: &PivotSpec,
        sample: Option<u64>,
        progress: &Progress,
    ) -> Result<Cube, Error> {
        self.pivot_cube_within(spec, sample, progress, pivot_budget())
    }

    /// [`Document::pivot_cube`] holding at most about `cap` bytes.
    pub fn pivot_cube_within(
        &self,
        spec: &PivotSpec,
        sample: Option<u64>,
        progress: &Progress,
        cap: usize,
    ) -> Result<Cube, Error> {
        self.ready_for_bulk()?;
        spec.check(self)?;
        let dims: Vec<&PivotField> = spec.rows.iter().chain(&spec.cols).collect();
        let nd = dims.len();
        let ctx = self.read_ctx(spec.read);
        let keyers: Vec<Keyer> = dims.iter().map(|f| Keyer::new(f, spec, &ctx)).collect();
        let picks: Vec<Option<Pick>> = dims.iter().map(|f| Pick::new(&f.items)).collect();
        let fkeyers: Vec<Keyer> = spec
            .filters
            .iter()
            .map(|f| Keyer::new(f, spec, &ctx))
            .collect();
        let fpicks: Vec<Option<Pick>> = spec.filters.iter().map(|f| Pick::new(&f.items)).collect();
        let vspecs: Vec<VSpec> = spec
            .values
            .iter()
            .map(|v| VSpec {
                agg: v.agg,
                rd: ctx.reader(v.col),
                separator: v.separator.clone(),
                match_case: spec.match_case,
            })
            .collect();
        let nv = vspecs.len();
        // Columns read: dims, then filters, then values.
        let mut cols: Vec<u32> = dims.iter().map(|f| f.col).collect();
        cols.extend(spec.filters.iter().map(|f| f.col));
        cols.extend(spec.values.iter().map(|v| v.col));
        let reader = Cols::new(self, &cols);
        let nf = spec.filters.len();
        let trim = !spec.keep_spaces;

        let all = self.data_rows(spec.keep_first);
        let end = match sample {
            Some(n) => all.end.min(all.start.saturating_add(n)),
            None => all.end,
        };
        let rows = all.start..end;
        let total = rows.end - rows.start;
        progress.start(total);
        let hs = (RandomState::new(), RandomState::new());
        let used = AtomicUsize::new(0);
        let budget = Budget { used: &used, cap };
        let chunks = (total as usize).div_ceil(CHUNK);

        let per = chunks
            .div_ceil(rayon::current_num_threads().max(1) * 2)
            .max(1);
        let part = (0..chunks)
            .into_par_iter()
            .with_min_len(per)
            .try_fold(
                || Part::new(nd),
                |mut part, c| -> Result<Part, Error> {
                    let s = rows.start + (c * CHUNK) as u64;
                    let e = (s + CHUNK as u64).min(rows.end);
                    let mut buf = Vec::new();
                    let mut key = String::new();
                    let mut tuple: Vec<u32> = vec![0; nd];
                    let mut charge = 0usize;
                    'row: for r in s..e {
                        let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                        reader.read(self, id, &mut buf);
                        part.read += 1;
                        for k in 0..nf {
                            if let Some(p) = &fpicks[k] {
                                fkeyers[k].key(reader.get(&buf, nd + k), &mut key);
                                if !p.keeps(&key) {
                                    continue 'row;
                                }
                            }
                        }
                        for k in 0..nd {
                            let t = reader.get(&buf, k);
                            keyers[k].key(t, &mut key);
                            if let Some(p) = &picks[k] {
                                if !p.keeps(&key) {
                                    continue 'row;
                                }
                            }
                            tuple[k] = match part.dicts[k].get(key.as_str()) {
                                Some(&i) => i,
                                None => {
                                    let (label, sort) = keyers[k].label(t, &key);
                                    let i = part.items[k].len() as u32;
                                    charge += ITEM_COST + key.len() + label.len();
                                    part.items[k].push(Item {
                                        key: key.as_str().into(),
                                        label: label.into(),
                                        first: r,
                                        sort,
                                        rank: 0,
                                    });
                                    part.dicts[k].insert(key.as_str().into(), i);
                                    i
                                }
                            };
                        }
                        part.used += 1;
                        let leaf = match part.map.get(&tuple) {
                            Some(l) => l as usize,
                            None => {
                                let l = part.map.len();
                                if l >= MAX_LEAVES {
                                    return Err(too_many_leaves());
                                }
                                part.map.insert(&tuple, l as u32);
                                part.keys.extend_from_slice(&tuple);
                                part.accs.extend(vspecs.iter().map(|v| Acc::new(v.agg)));
                                charge += leaf_cost(nd, nv);
                                l
                            }
                        };
                        for (v, vs) in vspecs.iter().enumerate() {
                            let raw = reader.get(&buf, nd + nf + v);
                            let t = if trim { raw.trim() } else { raw };
                            charge += part.accs[leaf * nv + v].add(vs, t, r, &hs);
                        }
                        if charge > 1 << 20 {
                            budget.charge(std::mem::take(&mut charge))?;
                        }
                    }
                    budget.charge(charge)?;
                    progress.step(e - s)?;
                    Ok(part)
                },
            )
            .try_reduce(|| Part::new(nd), |a, b| merge_parts(a, b, &vspecs, &budget))?;

        let leaves = part.leaves();
        let mut part = part;
        for its in &mut part.items {
            rank_items(its);
        }
        // Unique, sorted value lists: a leaf's median and distinct count are
        // then read without copying.
        part.accs.par_iter_mut().for_each(|a| match a {
            Acc::Ids(v) => {
                squeeze(v, true);
                v.shrink_to_fit();
            }
            Acc::Vals(v) => {
                v.sort_unstable_by(f64::total_cmp);
                v.shrink_to_fit();
            }
            _ => {}
        });
        let bytes = used.load(Ordering::Relaxed).max(leaves * LEAF_COST);
        Ok(Cube {
            nr: spec.rows.len(),
            nc: spec.cols.len(),
            aggs: vspecs,
            items: part.items,
            keys: part.keys,
            accs: part.accs,
            rows_read: part.read,
            rows_used: part.used,
            sampled: end < all.end,
            bytes,
            used: AtomicUsize::new(bytes),
            cap,
            memo: Mutex::new(None),
        })
    }
}

/// Merge two partial cubes (the bigger one keeps its ids).
fn merge_parts(a: Part, b: Part, vs: &[VSpec], budget: &Budget) -> Result<Part, Error> {
    let (mut a, b) = if a.leaves() >= b.leaves() {
        (a, b)
    } else {
        (b, a)
    };
    let nd = a.dicts.len();
    let nv = vs.len();
    // Item ids of b in a.
    let mut remap: Vec<Vec<u32>> = Vec::with_capacity(nd);
    let mut charge = 0usize;
    for k in 0..nd {
        let mut m = Vec::with_capacity(b.items[k].len());
        for it in &b.items[k] {
            let id = match a.dicts[k].get(&it.key) {
                Some(&i) => {
                    let mine = &mut a.items[k][i as usize];
                    if it.first < mine.first {
                        mine.first = it.first;
                        mine.label = it.label.clone();
                    }
                    i
                }
                None => {
                    let i = a.items[k].len() as u32;
                    charge += ITEM_COST + it.key.len() + it.label.len();
                    a.items[k].push(it.clone());
                    a.dicts[k].insert(it.key.clone(), i);
                    i
                }
            };
            m.push(id);
        }
        remap.push(m);
    }
    let mut tuple = vec![0u32; nd];
    let mut b = b;
    for bl in 0..b.leaves() {
        for k in 0..nd {
            tuple[k] = remap[k][b.keys[bl * nd + k] as usize];
        }
        match a.map.get(&tuple) {
            Some(al) => {
                let al = al as usize;
                for (v, spec) in vs.iter().enumerate() {
                    charge += a.accs[al * nv + v].merge(&b.accs[bl * nv + v], spec)?;
                }
            }
            None => {
                let al = a.map.len();
                if al >= MAX_LEAVES {
                    return Err(too_many_leaves());
                }
                a.map.insert(&tuple, al as u32);
                a.keys.extend_from_slice(&tuple);
                for acc in &mut b.accs[bl * nv..bl * nv + nv] {
                    a.accs.push(std::mem::replace(acc, Acc::Done(None)));
                }
            }
        }
    }
    budget.charge(charge)?;
    a.read += b.read;
    a.used += b.used;
    Ok(a)
}

// ------------------------------------------------------------- layout ----

/// What a grid row or column is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An item of the deepest field.
    Item,
    /// A group's own line (its subtotal, when shown at the top).
    Group,
    /// A group's "Total" line below (or column to the right).
    Subtotal,
    Grand,
}

/// A header cell over `span` value columns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeadCell {
    pub text: String,
    pub span: u32,
    /// Some(collapsed) when the column group can be expanded / collapsed.
    pub toggle: Option<bool>,
    /// Item keys of the column group.
    pub path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GridRow {
    pub kind: Kind,
    /// 0 for the grand total, 1 for the first row field's items, ...
    pub depth: u32,
    /// One per label column.
    pub labels: Vec<String>,
    /// Item keys (for drill-down and expand / collapse).
    pub path: Vec<String>,
    pub toggle: Option<bool>,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GridCol {
    pub kind: Kind,
    pub path: Vec<String>,
    /// Value field index.
    pub value: u32,
    /// Holds text (first / last / list), not numbers.
    pub text: bool,
}

/// A laid-out pivot table (perhaps only its first rows and columns).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PivotGrid {
    pub label_cols: u32,
    /// Header rows: `label_cols` corner cells, then cells spanning columns.
    pub head: Vec<Vec<HeadCell>>,
    pub cols: Vec<GridCol>,
    pub rows: Vec<GridRow>,
    /// Body rows and value columns of the whole table.
    pub total_rows: u64,
    pub total_cols: u64,
    pub truncated: bool,
    pub rows_read: u64,
    pub rows_used: u64,
    pub sampled: bool,
}

/// One axis (rows or columns) as a tree of items.
struct Tree {
    parent: Vec<u32>,
    item: Vec<u32>,
    depth: Vec<u32>,
    children: Vec<Vec<u32>>,
    kept: Vec<bool>,
}

impl Tree {
    /// Build from the cube's leaves (dims `off..off + n`); also the tree
    /// leaf of each cube leaf.
    fn build(cube: &Cube, off: usize, n: usize) -> (Tree, Vec<u32>) {
        let nd = cube.nr + cube.nc;
        let leaves = cube.leaves();
        let mut t = Tree {
            parent: vec![0],
            item: vec![u32::MAX],
            depth: vec![0],
            children: vec![Vec::new()],
            kept: vec![true],
        };
        let mut ix: HashMap<(u32, u32), u32> = HashMap::new();
        let mut at = Vec::with_capacity(leaves);
        for l in 0..leaves {
            let mut node = 0u32;
            for k in 0..n {
                let it = cube.keys[l * nd + off + k];
                node = *ix.entry((node, it)).or_insert_with(|| {
                    let id = t.parent.len() as u32;
                    t.parent.push(node);
                    t.item.push(it);
                    t.depth.push(k as u32 + 1);
                    t.children.push(Vec::new());
                    t.kept.push(true);
                    t.children[node as usize].push(id);
                    id
                });
            }
            at.push(node);
        }
        (t, at)
    }

    fn ancestors(&self, mut n: u32, out: &mut Vec<u32>) {
        out.clear();
        loop {
            out.push(n);
            if n == 0 {
                break;
            }
            n = self.parent[n as usize];
        }
    }

    fn path(&self, cube: &Cube, off: usize, mut n: u32) -> Vec<String> {
        let mut p = Vec::new();
        while n != 0 {
            let d = self.depth[n as usize] as usize;
            p.push(
                cube.items[off + d - 1][self.item[n as usize] as usize]
                    .key
                    .to_string(),
            );
            n = self.parent[n as usize];
        }
        p.reverse();
        p
    }

    /// The node at an item key path, if it is there and kept.
    fn find(&self, cube: &Cube, off: usize, path: &[String]) -> Option<u32> {
        let mut n = 0u32;
        for (d, k) in path.iter().enumerate() {
            let items = cube.items.get(off + d)?;
            n = *self.children[n as usize]
                .iter()
                .find(|&&c| *items[self.item[c as usize] as usize].key == **k)?;
        }
        self.kept[n as usize].then_some(n)
    }
}

/// Compare two items for "sort by label": numbers and dates in order and
/// before text, text A to Z; blanks last.
fn cmp_items(a: &Item, b: &Item) -> CmpOrdering {
    a.rank.cmp(&b.rank)
}

/// Label order of two items (`la`, `lb`: lower-case labels of text items).
fn cmp_labels(a: &Item, la: Option<&str>, b: &Item, lb: Option<&str>) -> CmpOrdering {
    let (ab, bb) = (a.key.is_empty(), b.key.is_empty());
    if ab || bb {
        return ab.cmp(&bb);
    }
    match (a.sort.is_nan(), b.sort.is_nan()) {
        (false, false) => a.sort.total_cmp(&b.sort),
        (false, true) => CmpOrdering::Less,
        (true, false) => CmpOrdering::Greater,
        (true, true) => natural_cmp(
            la.unwrap_or_default().as_bytes(),
            lb.unwrap_or_default().as_bytes(),
        )
        .then_with(|| a.label.cmp(&b.label)),
    }
}

/// Give each item its place in label order ([`Item::rank`]): numbers and
/// dates in order and before text, text A to Z, blanks last.
fn rank_items(items: &mut [Item]) {
    let lower: Vec<Option<String>> = items
        .par_iter()
        .map(|it| (!it.key.is_empty() && it.sort.is_nan()).then(|| it.label.to_lowercase()))
        .collect();
    let mut ix: Vec<u32> = (0..items.len() as u32).collect();
    {
        let items = &*items;
        ix.par_sort_by(|&a, &b| {
            let (a, b) = (a as usize, b as usize);
            cmp_labels(
                &items[a],
                lower[a].as_deref(),
                &items[b],
                lower[b].as_deref(),
            )
        });
    }
    for (r, &i) in ix.iter().enumerate() {
        items[i as usize].rank = r as u32;
    }
}

fn label_ok(f: &LabelFilter, label: &str) -> bool {
    let l = label.to_lowercase();
    let t = f.text.to_lowercase();
    let t2 = f.text2.to_lowercase();
    let cmp = |x: &str| natural_cmp(l.as_bytes(), x.as_bytes());
    match f.op {
        LabelOp::Equals => l == t,
        LabelOp::NotEquals => l != t,
        LabelOp::BeginsWith => l.starts_with(&t),
        LabelOp::NotBeginsWith => !l.starts_with(&t),
        LabelOp::EndsWith => l.ends_with(&t),
        LabelOp::NotEndsWith => !l.ends_with(&t),
        LabelOp::Contains => l.contains(&t),
        LabelOp::NotContains => !l.contains(&t),
        LabelOp::Greater => cmp(&t).is_gt(),
        LabelOp::Less => cmp(&t).is_lt(),
        LabelOp::Between => !cmp(&t).is_lt() && !cmp(&t2).is_gt(),
        LabelOp::NotBetween => cmp(&t).is_lt() || cmp(&t2).is_gt(),
    }
}

fn value_ok(f: &ValueFilter, x: Option<f64>) -> bool {
    let Some(x) = x else { return false };
    let (lo, hi) = (f.a.min(f.b), f.a.max(f.b));
    match f.op {
        CmpOp::Greater => x > f.a,
        CmpOp::GreaterOrEqual => x >= f.a,
        CmpOp::Less => x < f.a,
        CmpOp::LessOrEqual => x <= f.a,
        CmpOp::Equals => (x - f.a).abs() <= 1e-9 * f.a.abs().max(1.0),
        CmpOp::NotEquals => (x - f.a).abs() > 1e-9 * f.a.abs().max(1.0),
        CmpOp::Between => x >= lo && x <= hi,
        CmpOp::NotBetween => x < lo || x > hi,
    }
}

/// Totals of some value fields per node of one axis (over all leaves).
fn axis_totals(
    cube: &Cube,
    tree: &Tree,
    at: &[u32],
    wanted: &[u32],
    budget: &Budget,
    progress: &Progress,
) -> Result<HashMap<(u32, u32), Acc>, Error> {
    let nv = cube.aggs.len();
    let mut tot: HashMap<(u32, u32), Acc> = HashMap::new();
    let mut anc = Vec::new();
    let light: Vec<u32> = wanted
        .iter()
        .copied()
        .filter(|&v| !is_heavy(cube.aggs[v as usize].agg))
        .collect();
    if !light.is_empty() {
        for (l, &node) in at.iter().enumerate() {
            if l % 4096 == 0 {
                stopped(progress)?;
            }
            tree.ancestors(node, &mut anc);
            for &a in &anc {
                for &v in &light {
                    let vs = &cube.aggs[v as usize];
                    let e = tot.entry((a, v)).or_insert_with(|| Acc::new(vs.agg));
                    budget.charge(e.merge(&cube.accs[l * nv + v as usize], vs)?)?;
                }
            }
        }
    }
    let max_depth = tree.depth.iter().copied().max().unwrap_or(0);
    for &v in wanted
        .iter()
        .filter(|&&v| is_heavy(cube.aggs[v as usize].agg))
    {
        for depth in 0..=max_depth {
            let pairs: Vec<(u64, u32)> = at
                .iter()
                .enumerate()
                .filter_map(|(l, &node)| {
                    let mut n = node;
                    while tree.depth[n as usize] > depth {
                        n = tree.parent[n as usize];
                    }
                    (tree.depth[n as usize] == depth).then_some((u64::from(n), l as u32))
                })
                .collect();
            for (k, x) in heavy_runs(cube, v as usize, pairs, budget, progress)? {
                tot.insert((k as u32, v), Acc::Done(x));
            }
        }
    }
    Ok(tot)
}

/// Medians / distinct counts of groups of leaves: `pairs` = (group key,
/// leaf). One leaf alone is read as it is.
fn heavy_runs(
    cube: &Cube,
    v: usize,
    mut pairs: Vec<(u64, u32)>,
    budget: &Budget,
    progress: &Progress,
) -> Result<Vec<(u64, Option<f64>)>, Error> {
    let nv = cube.aggs.len();
    let acc = |l: u32| &cube.accs[l as usize * nv + v];
    // Groups are gathered at once (several threads): charge every value
    // and the pairs, against what is left, and give it back at the end.
    let elems: usize = pairs
        .iter()
        .map(|&(_, l)| match acc(l) {
            Acc::Ids(x) => x.len() * 16,
            Acc::Vals(x) => x.len() * 8,
            _ => 0,
        })
        .sum();
    let held = elems + pairs.len() * 16;
    budget.charge(held)?;
    let r = gather_runs(cube, v, &mut pairs, progress);
    budget.release(held);
    r
}

fn gather_runs(
    cube: &Cube,
    v: usize,
    pairs: &mut [(u64, u32)],
    progress: &Progress,
) -> Result<Vec<(u64, Option<f64>)>, Error> {
    let nv = cube.aggs.len();
    let agg = cube.aggs[v].agg;
    let acc = |l: u32| &cube.accs[l as usize * nv + v];
    pairs.par_sort_unstable();
    let pairs = &*pairs;
    let mut runs = Vec::new();
    let mut s = 0;
    for i in 1..=pairs.len() {
        if i == pairs.len() || pairs[i].0 != pairs[s].0 {
            runs.push((s, i));
            s = i;
        }
    }
    runs.par_iter()
        .map(|&(s, e)| {
            stopped(progress)?;
            let key = pairs[s].0;
            if e - s == 1 {
                return Ok((key, acc(pairs[s].1).number(agg)));
            }
            let x = match agg {
                Agg::CountDistinct => {
                    let mut all: Vec<u128> = Vec::new();
                    for &(_, l) in &pairs[s..e] {
                        if let Acc::Ids(x) = acc(l) {
                            all.extend_from_slice(x);
                        }
                    }
                    if all.len() > 1 << 16 {
                        all.par_sort_unstable();
                    } else {
                        all.sort_unstable();
                    }
                    all.dedup();
                    Some(all.len() as f64)
                }
                _ => {
                    let mut all: Vec<f64> = Vec::new();
                    for &(_, l) in &pairs[s..e] {
                        if let Acc::Vals(x) = acc(l) {
                            all.extend_from_slice(x);
                        }
                    }
                    median_owned(all)
                }
            };
            Ok((key, x))
        })
        .collect()
}

/// `Err(Cancelled)` once Stop was asked for.
fn stopped(progress: &Progress) -> Result<(), Error> {
    if progress.cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// Apply label filters, value filters and top N to one axis.
#[allow(clippy::too_many_arguments)]
fn filter_axis(
    cube: &Cube,
    tree: &mut Tree,
    at: &[u32],
    fields: &[PivotField],
    off: usize,
    blank: &str,
    budget: &Budget,
    progress: &Progress,
) -> Result<(), Error> {
    let mut wanted: Vec<u32> = fields
        .iter()
        .flat_map(|f| {
            f.value_filter
                .map(|v| v.value)
                .into_iter()
                .chain(f.top.map(|t| t.value))
        })
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() && fields.iter().all(|f| f.label_filter.is_none()) {
        return Ok(());
    }
    let tot = if wanted.is_empty() {
        HashMap::new()
    } else {
        axis_totals(cube, tree, at, &wanted, budget, progress)?
    };
    let num = |n: u32, v: u32| -> Option<f64> {
        tot.get(&(n, v))
            .and_then(|a| a.number(cube.aggs[v as usize].agg))
    };
    // Parents before children: node ids grow with depth along each path.
    for p in 0..tree.parent.len() {
        if !tree.kept[p] || tree.children[p].is_empty() {
            continue;
        }
        let d = tree.depth[p] as usize;
        let f = &fields[d];
        let items = &cube.items[off + d];
        let kids = tree.children[p].clone();
        let mut pass: Vec<u32> = Vec::new();
        for &c in &kids {
            let it = &items[tree.item[c as usize] as usize];
            let label: &str = if it.key.is_empty() { blank } else { &it.label };
            let ok = f.label_filter.as_ref().is_none_or(|lf| label_ok(lf, label))
                && f.value_filter
                    .is_none_or(|vf| value_ok(&vf, num(c, vf.value)));
            if ok {
                pass.push(c);
            }
        }
        if let Some(t) = f.top {
            pass.sort_by(|&a, &b| {
                let (x, y) = (num(a, t.value), num(b, t.value));
                match (x, y) {
                    (Some(x), Some(y)) => {
                        if t.bottom {
                            x.total_cmp(&y)
                        } else {
                            y.total_cmp(&x)
                        }
                    }
                    (Some(_), None) => CmpOrdering::Less,
                    (None, Some(_)) => CmpOrdering::Greater,
                    (None, None) => CmpOrdering::Equal,
                }
            });
            pass.truncate(t.n as usize);
        }
        let keep: HashSet<u32> = pass.into_iter().collect();
        for c in kids {
            tree.kept[c as usize] = keep.contains(&c);
        }
    }
    // A node is kept only under kept parents.
    for n in 1..tree.parent.len() {
        if !tree.kept[tree.parent[n] as usize] {
            tree.kept[n] = false;
        }
    }
    Ok(())
}

/// Both axes built and filtered.
struct Shape {
    rt: Tree,
    ct: Tree,
    rat: Vec<u32>,
    cat: Vec<u32>,
}

impl Cube {
    fn shape(
        &self,
        spec: &PivotSpec,
        budget: &Budget,
        progress: &Progress,
    ) -> Result<Shape, Error> {
        spec.check_limits()?;
        if spec.rows.len() != self.nr
            || spec.cols.len() != self.nc
            || spec.values.len() != self.aggs.len()
            || spec
                .values
                .iter()
                .zip(&self.aggs)
                .any(|(v, a)| v.agg != a.agg)
        {
            return Err(Error::Invalid("the pivot changed; refresh it".into()));
        }
        let (mut rt, rat) = Tree::build(self, 0, self.nr);
        let (mut ct, cat) = Tree::build(self, self.nr, self.nc);
        let blank = &spec.layout.blank_label;
        // Totals used to filter are let go once filtering is done.
        let before = budget.used.load(Ordering::Relaxed);
        let r = filter_axis(self, &mut rt, &rat, &spec.rows, 0, blank, budget, progress).and_then(
            |()| {
                filter_axis(
                    self, &mut ct, &cat, &spec.cols, self.nr, blank, budget, progress,
                )
            },
        );
        budget.used.store(before, Ordering::Relaxed);
        r?;
        Ok(Shape { rt, ct, rat, cat })
    }

    fn leaves(&self) -> usize {
        if self.aggs.is_empty() {
            let nd = self.nr + self.nc;
            self.keys.len() / nd.max(1)
        } else {
            self.accs.len() / self.aggs.len()
        }
    }
}

/// Cell summaries keyed by (row node, column node).
type Cells = HashMap<u64, Vec<Acc>>;

/// Every cell: a leaf's own (row leaf, column leaf) cell is the cube's
/// summary; totals are in `cells`.
struct Store<'a> {
    cube: &'a Cube,
    memo: Arc<Memo>,
}

/// Totals worked out for one set of filters (kept by the cube, so changes
/// to sorting, show-as, formats or collapsing reuse them).
struct Memo {
    cells: Cells,
    leaf: HashMap<u64, u32>,
}

impl Store<'_> {
    fn get(&self, r: u32, c: u32, v: usize) -> Option<&Acc> {
        let k = ck(r, c);
        match self.memo.leaf.get(&k) {
            Some(&l) => Some(&self.cube.accs[l as usize * self.cube.aggs.len() + v]),
            None => self.memo.cells.get(&k).map(|a| &a[v]),
        }
    }

    fn num(&self, r: u32, c: u32, v: usize) -> Option<f64> {
        self.get(r, c, v)
            .and_then(|a| a.number(self.cube.aggs[v].agg))
    }
}

fn ck(r: u32, c: u32) -> u64 {
    (u64::from(r) << 32) | u64::from(c)
}

/// One displayed row or column.
#[derive(Clone)]
struct Line {
    node: u32,
    kind: Kind,
    /// The group was collapsed by the user (or by depth).
    collapsed: bool,
}

struct Collapse {
    depth: u32,
    collapsed: HashSet<Vec<String>>,
    expanded: HashSet<Vec<String>>,
}

impl Collapse {
    fn is_collapsed(&self, depth: u32, path: &[String]) -> bool {
        if self.collapsed.contains(path) {
            return true;
        }
        self.depth > 0 && depth >= self.depth && !self.expanded.contains(path)
    }
}

/// Lay out one axis in display order.
#[allow(clippy::too_many_arguments)]
fn lines(
    cube: &Cube,
    tree: &Tree,
    off: usize,
    n_fields: usize,
    fields: &[PivotField],
    subtotals: bool,
    top: bool,
    grand: bool,
    col: &Collapse,
) -> Vec<Line> {
    let mut out = Vec::new();
    if n_fields == 0 {
        out.push(Line {
            node: 0,
            kind: Kind::Grand,
            collapsed: false,
        });
        return out;
    }
    // Iterative DFS: (node, after_children).
    let mut stack: Vec<(u32, bool)> = tree.children[0]
        .iter()
        .rev()
        .filter(|&&c| tree.kept[c as usize])
        .map(|&c| (c, false))
        .collect();
    while let Some((n, after)) = stack.pop() {
        let d = tree.depth[n as usize];
        if after {
            out.push(Line {
                node: n,
                kind: Kind::Subtotal,
                collapsed: false,
            });
            continue;
        }
        if d as usize == n_fields {
            out.push(Line {
                node: n,
                kind: Kind::Item,
                collapsed: false,
            });
            continue;
        }
        let path = tree.path(cube, off, n);
        let collapsed = col.is_collapsed(d, &path);
        let sub = subtotals && fields[d as usize - 1].subtotal;
        out.push(Line {
            node: n,
            kind: Kind::Group,
            collapsed,
        });
        if collapsed {
            continue;
        }
        if sub && !top {
            stack.push((n, true));
        }
        stack.extend(
            tree.children[n as usize]
                .iter()
                .rev()
                .filter(|&&c| tree.kept[c as usize])
                .map(|&c| (c, false)),
        );
    }
    if grand {
        out.push(Line {
            node: 0,
            kind: Kind::Grand,
            collapsed: false,
        });
    }
    out
}

/// Whether a group line shows values (else just its label).
fn group_shows(line: &Line, fields: &[PivotField], depth: u32, subtotals: bool, top: bool) -> bool {
    match line.kind {
        Kind::Group => line.collapsed || (top && subtotals && fields[depth as usize - 1].subtotal),
        _ => true,
    }
}

/// Write a number in a value field's format.
fn fmt_num(x: f64, f: &NumFormat, pct: bool, comma: bool) -> String {
    if !x.is_finite() {
        return String::new();
    }
    let (x, suffix_pct) = if pct { (x * 100.0, "%") } else { (x, "") };
    let decimals = f.decimals.or(if pct { Some(2) } else { None });
    let mut s = match decimals {
        Some(d) => {
            let s = format!("{:.*}", d as usize, x);
            if s.trim_start_matches('-')
                .bytes()
                .all(|b| b == b'0' || b == b'.')
            {
                s.trim_start_matches('-').to_string()
            } else {
                s
            }
        }
        None => format_number(x, false),
    };
    let neg = s.starts_with('-');
    if neg {
        s.remove(0);
    }
    let (int, frac) = match s.find('.') {
        Some(i) => (s[..i].to_string(), s[i + 1..].to_string()),
        None => (s.clone(), String::new()),
    };
    let int = if f.thousands && int.bytes().all(|b| b.is_ascii_digit()) {
        let sep = if comma { '.' } else { ',' };
        let mut g = String::with_capacity(int.len() + int.len() / 3);
        for (i, ch) in int.chars().enumerate() {
            if i > 0 && (int.len() - i) % 3 == 0 {
                g.push(sep);
            }
            g.push(ch);
        }
        g
    } else {
        int
    };
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push_str(&f.prefix);
    out.push_str(&int);
    if !frac.is_empty() {
        out.push(if comma { ',' } else { '.' });
        out.push_str(&frac);
    }
    out.push_str(suffix_pct);
    out.push_str(&f.suffix);
    out
}

/// Computes displayed cell values, with "show values as".
struct Calc<'a> {
    cube: &'a Cube,
    spec: &'a PivotSpec,
    cells: &'a Store<'a>,
    rt: &'a Tree,
    ct: &'a Tree,
    /// Position among kept, sorted siblings.
    rpos: Vec<u32>,
    cpos: Vec<u32>,
    /// Prefix sums and sorted values of sibling runs, by (parent, other
    /// axis node, value, axis).
    prefix: HashMap<(u32, u32, u32, bool), Vec<f64>>,
    sorted: HashMap<(u32, u32, u32, bool), Vec<f64>>,
}

impl<'a> Calc<'a> {
    fn num(&self, r: u32, c: u32, v: usize) -> Option<f64> {
        self.cells.num(r, c, v)
    }

    /// Siblings of `n` along an axis, and n's position.
    fn sibs(&self, rows: bool, n: u32) -> (&'a [u32], usize) {
        let (t, pos) = if rows {
            (self.rt, &self.rpos)
        } else {
            (self.ct, &self.cpos)
        };
        let p = t.parent[n as usize];
        (&t.children[p as usize], pos[n as usize] as usize)
    }

    fn at(&self, rows: bool, me: u32, other: u32, v: usize) -> Option<f64> {
        if rows {
            self.num(me, other, v)
        } else {
            self.num(other, me, v)
        }
    }

    fn value(&mut self, r: u32, c: u32, v: usize) -> Option<f64> {
        let vf = &self.spec.values[v];
        let x = self.num(r, c, v);
        let rows = vf.along == Along::Rows;
        let (me, other) = if rows { (r, c) } else { (c, r) };
        let div = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) if b != 0.0 => Some(a / b),
            _ => None,
        };
        match vf.show_as {
            ShowAs::Value => x,
            ShowAs::PctOfGrand => div(x, self.num(0, 0, v)),
            ShowAs::PctOfColumn => div(x, self.num(0, c, v)),
            ShowAs::PctOfRow => div(x, self.num(r, 0, v)),
            ShowAs::PctOfParentRow => div(x, self.num(self.rt.parent[r as usize], c, v)),
            ShowAs::PctOfParentColumn => div(x, self.num(r, self.ct.parent[c as usize], v)),
            ShowAs::Index => match (x, self.num(0, 0, v), self.num(r, 0, v), self.num(0, c, v)) {
                (Some(x), Some(g), Some(rt), Some(ct)) if rt != 0.0 && ct != 0.0 => {
                    Some(x * g / (rt * ct))
                }
                _ => None,
            },
            ShowAs::RunningTotal | ShowAs::PctRunningTotal => {
                if me == 0 {
                    return None;
                }
                x?;
                let (sibs, pos) = self.sibs(rows, me);
                let parent = if rows {
                    self.rt.parent[me as usize]
                } else {
                    self.ct.parent[me as usize]
                };
                let key = (parent, other, v as u32, rows);
                if !self.prefix.contains_key(&key) {
                    let mut acc = 0.0;
                    let pre: Vec<f64> = sibs
                        .iter()
                        .filter(|&&s| {
                            if rows {
                                self.rt.kept[s as usize]
                            } else {
                                self.ct.kept[s as usize]
                            }
                        })
                        .map(|&s| {
                            acc += self.at(rows, s, other, v).unwrap_or(0.0);
                            acc
                        })
                        .collect();
                    self.prefix.insert(key, pre);
                }
                let run = *self.prefix[&key].get(pos)?;
                if vf.show_as == ShowAs::RunningTotal {
                    Some(run)
                } else {
                    div(Some(run), self.prefix[&key].last().copied())
                }
            }
            ShowAs::DifferenceFromPrevious | ShowAs::PctDifferenceFromPrevious => {
                if me == 0 {
                    return None;
                }
                let (sibs, pos) = self.sibs(rows, me);
                let kept: Vec<u32> = sibs
                    .iter()
                    .copied()
                    .filter(|&s| {
                        if rows {
                            self.rt.kept[s as usize]
                        } else {
                            self.ct.kept[s as usize]
                        }
                    })
                    .collect();
                if pos == 0 {
                    return None;
                }
                let prev = self.at(rows, kept[pos - 1], other, v);
                match (x, prev) {
                    (Some(x), Some(p)) => {
                        if vf.show_as == ShowAs::DifferenceFromPrevious {
                            Some(x - p)
                        } else if p != 0.0 {
                            Some((x - p) / p)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            ShowAs::RankLargest | ShowAs::RankSmallest => {
                if me == 0 {
                    return None;
                }
                let x = x?;
                let (sibs, _) = self.sibs(rows, me);
                let parent = if rows {
                    self.rt.parent[me as usize]
                } else {
                    self.ct.parent[me as usize]
                };
                let key = (parent, other, v as u32, rows);
                if !self.sorted.contains_key(&key) {
                    let mut vals: Vec<f64> = sibs
                        .iter()
                        .filter(|&&s| {
                            if rows {
                                self.rt.kept[s as usize]
                            } else {
                                self.ct.kept[s as usize]
                            }
                        })
                        .filter_map(|&s| self.at(rows, s, other, v))
                        .collect();
                    vals.sort_by(f64::total_cmp);
                    self.sorted.insert(key, vals);
                }
                let vals = &self.sorted[&key];
                let rank = if vf.show_as == ShowAs::RankLargest {
                    vals.len() - vals.partition_point(|&y| y <= x) + 1
                } else {
                    vals.partition_point(|&y| y < x) + 1
                };
                Some(rank as f64)
            }
        }
    }

    fn text(&mut self, r: u32, c: u32, v: usize) -> Option<String> {
        let vf = &self.spec.values[v];
        let agg = self.cube.aggs[v].agg;
        if agg.is_text() {
            return self
                .cells
                .get(r, c, v)
                .and_then(Acc::text)
                .map(str::to_string);
        }
        let x = self.value(r, c, v)?;
        let pct = vf.show_as.is_pct();
        let fmt = if matches!(vf.show_as, ShowAs::RankLargest | ShowAs::RankSmallest) {
            &NumFormat::default()
        } else {
            &vf.format
        };
        Some(fmt_num(x, fmt, pct, self.spec.read.decimal_comma))
    }
}

/// Limits for a preview grid.
#[derive(Debug, Clone, Copy)]
pub struct GridLimit {
    pub rows: usize,
    pub cols: usize,
}

impl Cube {
    /// Lay the cube out for `spec`. `titles`: every column's title (by
    /// column number). `limit`: only the first rows / value columns.
    pub fn layout(
        &self,
        spec: &PivotSpec,
        titles: &[String],
        limit: Option<GridLimit>,
    ) -> Result<PivotGrid, Error> {
        self.layout_with(spec, titles, limit, &Progress::new())
    }

    /// [`Cube::layout`] that stops when `progress` is cancelled. The cube,
    /// its kept totals and the work of this layout share one memory limit.
    pub fn layout_with(
        &self,
        spec: &PivotSpec,
        titles: &[String],
        limit: Option<GridLimit>,
        progress: &Progress,
    ) -> Result<PivotGrid, Error> {
        let r = self.layout_inner(spec, titles, limit, progress);
        // Afterwards only the cube and its kept totals stay charged.
        let kept = self
            .memo
            .lock()
            .ok()
            .and_then(|m| m.as_ref().map(|x| x.2))
            .unwrap_or(0);
        self.used.store(self.bytes + kept, Ordering::Relaxed);
        r
    }

    fn layout_inner(
        &self,
        spec: &PivotSpec,
        titles: &[String],
        limit: Option<GridLimit>,
        progress: &Progress,
    ) -> Result<PivotGrid, Error> {
        let budget = Budget {
            used: &self.used,
            cap: self.cap,
        };
        let Shape {
            mut rt,
            mut ct,
            rat,
            cat,
        } = self.shape(spec, &budget, progress)?;
        let nv = self.aggs.len();
        let l = &spec.layout;

        // Cells: every kept (row ancestor, column ancestor) of every leaf.
        let sig = serde_json::to_string(&(
            spec.rows
                .iter()
                .chain(&spec.cols)
                .map(|f| (&f.label_filter, f.value_filter, f.top))
                .collect::<Vec<_>>(),
            &l.blank_label,
        ))
        .unwrap_or_default();
        let cached = self.memo.lock().ok().and_then(|m| {
            m.as_ref()
                .filter(|(k, _, _)| *k == sig)
                .map(|(_, a, _)| a.clone())
        });
        let memo: Arc<Memo> = match cached {
            Some(m) => m,
            None => {
                // The old totals go first; the new ones are charged as built.
                if let Ok(mut slot) = self.memo.lock() {
                    *slot = None;
                }
                self.used.store(self.bytes, Ordering::Relaxed);
                let heavy: Vec<bool> = self.aggs.iter().map(|a| is_heavy(a.agg)).collect();
                let mut cells: Cells = HashMap::new();
                let mut leaf_cells: HashMap<u64, u32> = HashMap::new();
                let kept: Vec<u32> = (0..self.leaves() as u32)
                    .filter(|&l| {
                        rt.kept[rat[l as usize] as usize] && ct.kept[cat[l as usize] as usize]
                    })
                    .collect();
                if nv > 0 {
                    let fresh = || -> Vec<Acc> {
                        self.aggs
                            .iter()
                            .map(|v| {
                                if is_heavy(v.agg) {
                                    Acc::Done(None)
                                } else {
                                    Acc::new(v.agg)
                                }
                            })
                            .collect()
                    };
                    let mut ra = Vec::new();
                    let mut ca = Vec::new();
                    let mut charge = 0usize;
                    for (i, &leaf) in kept.iter().enumerate() {
                        if i % 4096 == 0 {
                            stopped(progress)?;
                        }
                        let leaf = leaf as usize;
                        let (r, c) = (rat[leaf], cat[leaf]);
                        leaf_cells.insert(ck(r, c), leaf as u32);
                        rt.ancestors(r, &mut ra);
                        ct.ancestors(c, &mut ca);
                        let src = &self.accs[leaf * nv..leaf * nv + nv];
                        for &a in &ra {
                            for &b in &ca {
                                if a == r && b == c {
                                    continue;
                                }
                                let e = cells.entry(ck(a, b)).or_insert_with(|| {
                                    charge += 48 + nv * std::mem::size_of::<Acc>();
                                    fresh()
                                });
                                for v in 0..nv {
                                    if !heavy[v] {
                                        charge += e[v].merge(&src[v], &self.aggs[v])?;
                                    }
                                }
                            }
                        }
                        if charge > 1 << 20 {
                            budget.charge(std::mem::take(&mut charge))?;
                        }
                    }
                    budget.charge(charge + leaf_cells.len() * 24)?;
                    // Medians and distinct counts of totals: one pass per pair of
                    // levels.
                    for v in (0..nv).filter(|&v| heavy[v]) {
                        for i in 0..=self.nr as u32 {
                            for j in 0..=self.nc as u32 {
                                if i == self.nr as u32 && j == self.nc as u32 {
                                    continue;
                                }
                                let up = |t: &Tree, mut n: u32, d: u32| {
                                    while t.depth[n as usize] > d {
                                        n = t.parent[n as usize];
                                    }
                                    n
                                };
                                let pairs: Vec<(u64, u32)> = kept
                                    .iter()
                                    .map(|&l| {
                                        let a = up(&rt, rat[l as usize], i);
                                        let b = up(&ct, cat[l as usize], j);
                                        (ck(a, b), l)
                                    })
                                    .collect();
                                for (k, x) in heavy_runs(self, v, pairs, &budget, progress)? {
                                    if let Some(e) = cells.get_mut(&k) {
                                        e[v] = Acc::Done(x);
                                    }
                                }
                            }
                        }
                    }
                }
                let m = Arc::new(Memo {
                    cells,
                    leaf: leaf_cells,
                });
                let memo_bytes = self.used.load(Ordering::Relaxed).saturating_sub(self.bytes);
                if let Ok(mut slot) = self.memo.lock() {
                    *slot = Some((sig, m.clone(), memo_bytes));
                }
                m
            }
        };
        let store = Store { cube: self, memo };

        // Sort children.
        let sort_axis = |t: &mut Tree, fields: &[PivotField], off: usize, rows: bool| {
            for p in 0..t.parent.len() {
                if t.children[p].len() < 2 {
                    continue;
                }
                let d = t.depth[p] as usize;
                let f = &fields[d];
                let items = &self.items[off + d];
                let mut kids = std::mem::take(&mut t.children[p]);
                let item_of = |n: u32| &items[t.item[n as usize] as usize];
                match f.sort.by {
                    SortBy::Label => {
                        kids.sort_by(|&a, &b| cmp_items(item_of(a), item_of(b)));
                        if f.sort.desc {
                            kids.reverse();
                        }
                    }
                    SortBy::Value => {
                        let v = f.sort.value as usize;
                        let num = |n: u32| {
                            let k = if rows { ck(n, 0) } else { ck(0, n) };
                            let _ = k;
                            if rows {
                                store.num(n, 0, v)
                            } else {
                                store.num(0, n, v)
                            }
                        };
                        kids.sort_by(|&a, &b| {
                            let o = match (num(a), num(b)) {
                                (Some(x), Some(y)) => {
                                    if f.sort.desc {
                                        y.total_cmp(&x)
                                    } else {
                                        x.total_cmp(&y)
                                    }
                                }
                                (Some(_), None) => CmpOrdering::Less,
                                (None, Some(_)) => CmpOrdering::Greater,
                                (None, None) => CmpOrdering::Equal,
                            };
                            o.then_with(|| cmp_items(item_of(a), item_of(b)))
                        });
                    }
                }
                t.children[p] = kids;
            }
        };
        sort_axis(&mut rt, &spec.rows, 0, true);
        sort_axis(&mut ct, &spec.cols, self.nr, false);

        let positions = |t: &Tree| {
            let mut pos = vec![0u32; t.parent.len()];
            for kids in &t.children {
                let mut i = 0;
                for &k in kids {
                    if t.kept[k as usize] {
                        pos[k as usize] = i;
                        i += 1;
                    }
                }
            }
            pos
        };
        let rpos = positions(&rt);
        let cpos = positions(&ct);

        let set = |v: &[Vec<String>]| v.iter().cloned().collect::<HashSet<_>>();
        let rcol = Collapse {
            depth: l.row_depth,
            collapsed: set(&l.collapsed_rows),
            expanded: set(&l.expanded_rows),
        };
        let ccol = Collapse {
            depth: l.col_depth,
            collapsed: set(&l.collapsed_cols),
            expanded: set(&l.expanded_cols),
        };
        let rlines = lines(
            self,
            &rt,
            0,
            self.nr,
            &spec.rows,
            l.row_subtotals,
            l.subtotals_top,
            l.grand_row,
            &rcol,
        );
        // Column subtotals always come after their group.
        let clines = if nv == 0 {
            Vec::new()
        } else {
            lines(
                self,
                &ct,
                self.nr,
                self.nc,
                &spec.cols,
                l.col_subtotals,
                false,
                l.grand_col,
                &ccol,
            )
        };
        // A column group line only shows when collapsed.
        let clines: Vec<Line> = clines
            .into_iter()
            .filter(|c| c.kind != Kind::Group || c.collapsed)
            .collect();

        let total_rows = rlines.len() as u64;
        let total_cols = (clines.len() * nv) as u64;
        let (max_r, max_c) = match limit {
            Some(g) => (g.rows, g.cols),
            None => {
                let label_cols = self.nr.max(1) as u64;
                if total_rows.saturating_mul(total_cols + label_cols) > MAX_TABLE_CELLS {
                    return Err(too_big());
                }
                (usize::MAX, usize::MAX)
            }
        };
        let show_c = clines.len().min(max_c.div_ceil(nv.max(1)));
        let truncated = rlines.len() > max_r || clines.len() > show_c;
        let clines = &clines[..show_c];

        let label_cols = self.nr.max(1);
        let vname = |v: usize| -> String {
            let vf = &spec.values[v];
            if vf.name.is_empty() {
                let t = titles.get(vf.col as usize).cloned().unwrap_or_default();
                format!("{} {}", vf.agg.title(), t)
            } else {
                vf.name.clone()
            }
        };
        let blank = &l.blank_label;
        let item_label = |off: usize, t: &Tree, n: u32| -> String {
            let d = t.depth[n as usize] as usize;
            let it = &self.items[off + d - 1][t.item[n as usize] as usize];
            if it.key.is_empty() {
                blank.clone()
            } else {
                it.label.to_string()
            }
        };

        // Columns and header rows.
        let mut cols = Vec::with_capacity(show_c * nv);
        for c in clines {
            let path = ct.path(self, self.nr, c.node);
            #[allow(clippy::needless_range_loop)] // v is also the value number
            for v in 0..nv {
                cols.push(GridCol {
                    kind: c.kind,
                    path: path.clone(),
                    value: v as u32,
                    text: self.aggs[v].agg.is_text(),
                });
            }
        }
        let vrow = nv > 1 || self.nc == 0;
        // With column fields: a title row (value name, column field names),
        // one row per column field, then value names when there are several.
        let hrows = if self.nc > 0 {
            1 + self.nc + usize::from(vrow)
        } else {
            1
        };
        let col_titles = spec
            .cols
            .iter()
            .map(|f| titles.get(f.col as usize).cloned().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(" / ");
        let mut head: Vec<Vec<HeadCell>> = Vec::with_capacity(hrows);
        let mut ancs = Vec::new();
        for h in 0..hrows {
            let mut row: Vec<HeadCell> = Vec::new();
            #[allow(clippy::needless_range_loop)] // spec.rows is read on one branch only
            for lc in 0..label_cols {
                let text = if h + 1 == hrows {
                    if self.nr == 0 {
                        String::new()
                    } else {
                        titles
                            .get(spec.rows[lc].col as usize)
                            .cloned()
                            .unwrap_or_default()
                    }
                } else if h == 0 && lc == 0 && nv == 1 && self.nc > 0 {
                    vname(0)
                } else {
                    String::new()
                };
                row.push(HeadCell {
                    text,
                    span: 1,
                    toggle: None,
                    path: Vec::new(),
                });
            }
            let is_vrow = vrow && h == hrows - 1;
            if self.nc > 0 && h == 0 {
                if !clines.is_empty() {
                    row.push(HeadCell {
                        text: col_titles.clone(),
                        span: (clines.len() * nv) as u32,
                        toggle: None,
                        path: Vec::new(),
                    });
                }
                head.push(row);
                continue;
            }
            // (group node at this level, text, toggle, path) per column.
            let mut prev: Option<(u64, usize)> = None;
            for c in clines {
                let level = h as u32;
                let (gid, text, toggle, path) = if is_vrow {
                    (u64::MAX, String::new(), None, Vec::new())
                } else {
                    match c.kind {
                        Kind::Grand => {
                            let t = if level <= 1 {
                                "Grand Total".to_string()
                            } else {
                                String::new()
                            };
                            (u64::MAX - 1, t, None, Vec::new())
                        }
                        _ => {
                            let d = ct.depth[c.node as usize];
                            if d < level {
                                // Under a collapsed group or subtotal: blank.
                                (ck(c.node, 1) | (1 << 63), String::new(), None, Vec::new())
                            } else {
                                ct.ancestors(c.node, &mut ancs);
                                let a = ancs[(d - level) as usize];
                                let mut t = item_label(self.nr, &ct, a);
                                let mut toggle = None;
                                let mut gid = u64::from(a);
                                if d == level {
                                    match c.kind {
                                        Kind::Subtotal => {
                                            t.push_str(" Total");
                                            gid |= 1 << 62;
                                        }
                                        Kind::Group => toggle = Some(true),
                                        _ => {}
                                    }
                                } else {
                                    toggle = Some(false);
                                }
                                (gid, t, toggle, ct.path(self, self.nr, a))
                            }
                        }
                    }
                };
                if is_vrow {
                    for v in 0..nv {
                        row.push(HeadCell {
                            text: vname(v),
                            span: 1,
                            toggle: None,
                            path: Vec::new(),
                        });
                    }
                    continue;
                }
                match prev {
                    Some((g, i)) if g == gid && gid < (1 << 62) => {
                        row[i].span += nv as u32;
                    }
                    _ => {
                        prev = Some((gid, row.len()));
                        row.push(HeadCell {
                            text,
                            span: nv as u32,
                            toggle,
                            path,
                        });
                    }
                }
            }
            head.push(row);
        }

        // Body.
        let mut calc = Calc {
            cube: self,
            spec,
            cells: &store,
            rt: &rt,
            ct: &ct,
            rpos,
            cpos,
            prefix: HashMap::new(),
            sorted: HashMap::new(),
        };
        let mut rows = Vec::with_capacity(rlines.len().min(max_r));
        let mut bytes = 0usize;
        for (ri, rl) in rlines.iter().take(max_r).enumerate() {
            if ri % 256 == 0 {
                stopped(progress)?;
            }
            let depth = rt.depth[rl.node as usize];
            let mut labels = vec![String::new(); label_cols];
            match rl.kind {
                Kind::Grand => {
                    labels[0] = if self.nr == 0 {
                        "Total".into()
                    } else {
                        "Grand Total".into()
                    };
                }
                _ => {
                    let mut n = rl.node;
                    let mut first = true;
                    while n != 0 {
                        let d = rt.depth[n as usize] as usize;
                        if first || l.repeat_labels {
                            let mut t = item_label(0, &rt, n);
                            if first && rl.kind == Kind::Subtotal {
                                t.push_str(" Total");
                            }
                            labels[d - 1] = t;
                        }
                        first = false;
                        n = rt.parent[n as usize];
                    }
                }
            }
            let shows = group_shows(rl, &spec.rows, depth, l.row_subtotals, l.subtotals_top);
            let mut cells_out = Vec::with_capacity(cols.len());
            for c in clines {
                for v in 0..nv {
                    let t = if shows {
                        calc.text(rl.node, c.node, v)
                            .unwrap_or_else(|| l.empty_cells.clone())
                    } else {
                        String::new()
                    };
                    bytes += t.len() + 1;
                    cells_out.push(t);
                }
            }
            bytes += labels.iter().map(|s| s.len() + 1).sum::<usize>();
            if bytes > MAX_TABLE_BYTES {
                return Err(too_big());
            }
            let toggle = (rl.kind == Kind::Group).then_some(rl.collapsed);
            rows.push(GridRow {
                kind: rl.kind,
                depth,
                labels,
                path: if rl.node == 0 {
                    Vec::new()
                } else {
                    rt.path(self, 0, rl.node)
                },
                toggle,
                cells: cells_out,
            });
        }
        Ok(PivotGrid {
            label_cols: label_cols as u32,
            head,
            cols,
            rows,
            total_rows,
            total_cols,
            truncated,
            rows_read: self.rows_read,
            rows_used: self.rows_used,
            sampled: self.sampled,
        })
    }
}

impl PivotGrid {
    /// The grid as a table (CSV), for a new tab.
    pub fn to_table(&self, repeat_labels: bool) -> Result<Table, Error> {
        let width = self.label_cols as usize + self.cols.len();
        let mut out = Vec::new();
        let mut line: Vec<&str> = Vec::with_capacity(width);
        for h in &self.head {
            line.clear();
            for c in h {
                for i in 0..c.span {
                    line.push(if i == 0 || repeat_labels { &c.text } else { "" });
                }
            }
            push_line(&mut out, &line);
        }
        for r in &self.rows {
            line.clear();
            line.extend(r.labels.iter().map(String::as_str));
            line.extend(r.cells.iter().map(String::as_str));
            push_line(&mut out, &line);
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
        }
        Ok(Table {
            bytes: out,
            rows: (self.head.len() + self.rows.len()) as u64,
            cols: width as u32,
        })
    }

    /// The grid as tab-separated text (for the clipboard).
    pub fn to_tsv(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
        let mut out = String::new();
        for h in &self.head {
            let mut first = true;
            for c in h {
                for i in 0..c.span {
                    if !first {
                        out.push('\t');
                    }
                    first = false;
                    if i == 0 {
                        out.push_str(&clean(&c.text));
                    }
                }
            }
            out.push_str("\r\n");
        }
        for r in &self.rows {
            let all: Vec<String> = r.labels.iter().chain(&r.cells).map(|s| clean(s)).collect();
            out.push_str(&all.join("\t"));
            out.push_str("\r\n");
        }
        out
    }
}

// --------------------------------------------------- items, drill-down ----

/// One item of a field, for pick lists.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemInfo {
    pub key: String,
    pub label: String,
    /// Rows holding it.
    pub rows: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemList {
    pub items: Vec<ItemInfo>,
    /// Distinct items in all (the list may stop at [`MAX_ITEM_LIST`]).
    pub total: u64,
}

impl Document {
    /// The items of one field (grouped as asked), in label order, with how
    /// many rows hold each. Other fields and picks do not matter.
    pub fn pivot_items(
        &self,
        field: &PivotField,
        spec: &PivotSpec,
        progress: &Progress,
    ) -> Result<ItemList, Error> {
        let one = PivotSpec {
            rows: vec![PivotField {
                col: field.col,
                group: field.group,
                ..Default::default()
            }],
            cols: Vec::new(),
            values: vec![ValueField {
                col: field.col,
                agg: Agg::CountRows,
                ..Default::default()
            }],
            filters: Vec::new(),
            keep_first: spec.keep_first,
            read: spec.read,
            match_case: spec.match_case,
            keep_spaces: spec.keep_spaces,
            layout: PivotLayout::default(),
        };
        let cube = self.pivot_cube(&one, None, progress)?;
        let mut ix: Vec<usize> = (0..cube.items[0].len()).collect();
        ix.sort_by(|&a, &b| cmp_items(&cube.items[0][a], &cube.items[0][b]));
        let total = ix.len() as u64;
        // Leaf i holds item keys[i] (one field).
        let mut rows_of = vec![0u64; cube.items[0].len()];
        for (leaf, &it) in cube.keys.iter().enumerate() {
            rows_of[it as usize] = cube.accs[leaf].number(Agg::CountRows).unwrap_or(0.0) as u64;
        }
        let items = ix
            .into_iter()
            .take(MAX_ITEM_LIST)
            .map(|i| {
                let it = &cube.items[0][i];
                ItemInfo {
                    key: it.key.to_string(),
                    label: if it.key.is_empty() {
                        spec.layout.blank_label.clone()
                    } else {
                        it.label.to_string()
                    },
                    rows: rows_of[i],
                }
            })
            .collect();
        Ok(ItemList { items, total })
    }

    /// The source rows behind one pivot cell (item key paths of its row and
    /// column; empty = grand total) as a new table, every column. `cube`
    /// must be a full (not sampled) cube of `spec`.
    pub fn pivot_drill(
        &self,
        spec: &PivotSpec,
        cube: &Cube,
        row_path: &[String],
        col_path: &[String],
        progress: &Progress,
    ) -> Result<Table, Error> {
        self.ready_for_bulk()?;
        spec.check(self)?;
        if cube.sampled {
            return Err(Error::Invalid("refresh the pivot first".into()));
        }
        if row_path.len() > cube.nr || col_path.len() > cube.nc {
            return Err(Error::OutOfRange);
        }
        let used = AtomicUsize::new(cube.used.load(Ordering::Relaxed));
        let budget = Budget {
            used: &used,
            cap: cube.cap,
        };
        let sh = cube.shape(spec, &budget, progress)?;
        let missing =
            || Error::Invalid("that pivot cell is no longer there; refresh the pivot".into());
        let rn = sh.rt.find(cube, 0, row_path).ok_or_else(missing)?;
        let cn = sh.ct.find(cube, cube.nr, col_path).ok_or_else(missing)?;
        // Leaves under both nodes.
        let under = |t: &Tree, leaf: u32, top: u32| {
            let mut n = leaf;
            loop {
                if !t.kept[n as usize] {
                    return false;
                }
                if n == top {
                    return true;
                }
                if n == 0 {
                    return false;
                }
                n = t.parent[n as usize];
            }
        };
        let nd = cube.nr + cube.nc;
        let mut allowed: HashSet<&[u32]> = HashSet::new();
        for leaf in 0..cube.leaves() {
            if under(&sh.rt, sh.rat[leaf], rn) && under(&sh.ct, sh.cat[leaf], cn) {
                allowed.insert(&cube.keys[leaf * nd..leaf * nd + nd]);
            }
        }
        let dicts: Vec<HashMap<&str, u32>> = cube
            .items
            .iter()
            .map(|its| {
                its.iter()
                    .enumerate()
                    .map(|(i, it)| (&*it.key, i as u32))
                    .collect()
            })
            .collect();
        let dims: Vec<&PivotField> = spec.rows.iter().chain(&spec.cols).collect();
        let ctx = self.read_ctx(spec.read);
        let keyers: Vec<Keyer> = dims.iter().map(|f| Keyer::new(f, spec, &ctx)).collect();
        let fkeyers: Vec<Keyer> = spec
            .filters
            .iter()
            .map(|f| Keyer::new(f, spec, &ctx))
            .collect();
        let fpicks: Vec<Option<Pick>> = spec.filters.iter().map(|f| Pick::new(&f.items)).collect();
        let mut cols: Vec<u32> = dims.iter().map(|f| f.col).collect();
        cols.extend(spec.filters.iter().map(|f| f.col));
        let reader = Cols::new(self, &cols);
        let nf = spec.filters.len();
        let ids: Vec<u32> =
            self.par_rows(self.data_rows(spec.keep_first), progress, |_, id, buf| {
                reader.read(self, id, buf);
                let mut key = String::new();
                for k in 0..nf {
                    if let Some(p) = &fpicks[k] {
                        fkeyers[k].key(reader.get(buf, nd + k), &mut key);
                        if !p.keeps(&key) {
                            return Ok(None);
                        }
                    }
                }
                let mut tuple = Vec::with_capacity(nd);
                for k in 0..nd {
                    keyers[k].key(reader.get(buf, k), &mut key);
                    match dicts[k].get(key.as_str()) {
                        Some(&i) => tuple.push(i),
                        None => return Ok(None),
                    }
                }
                Ok(allowed.contains(tuple.as_slice()).then_some(id))
            })?;
        let all: Vec<u32> = (0..self.col_count()).collect();
        let mut out = Vec::new();
        push_line(&mut out, &self.titles(&all, spec.keep_first));
        let every = Cols::new(self, &all);
        let mut buf = Vec::new();
        for &id in &ids {
            every.read(self, id, &mut buf);
            let line: Vec<&str> = (0..all.len()).map(|k| every.get(&buf, k)).collect();
            push_line(&mut out, &line);
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
        }
        Ok(Table {
            bytes: out,
            rows: ids.len() as u64 + 1,
            cols: all.len() as u32,
        })
    }
}

// ------------------------------------------------------ saved layouts ----

/// Version of saved pivot layout files.
pub const SAVED_PIVOT_VERSION: u32 = 1;
/// Largest saved pivot layout file.
pub const MAX_SAVED_PIVOT_BYTES: usize = 4 << 20;

/// A pivot layout saved to reuse (`.ndpivot`): the spec plus the title of
/// each column it uses, so it still fits a file whose columns moved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPivot {
    pub version: u32,
    pub name: String,
    pub spec: PivotSpec,
    /// (column number, title) of every column the spec uses.
    pub columns: Vec<(u32, String)>,
}

impl PivotSpec {
    /// Every column the spec uses.
    pub fn used_cols(&self) -> Vec<u32> {
        let mut c: Vec<u32> = self
            .rows
            .iter()
            .chain(&self.cols)
            .chain(&self.filters)
            .map(|f| f.col)
            .chain(self.values.iter().map(|v| v.col))
            .collect();
        c.sort_unstable();
        c.dedup();
        c
    }

    fn map_cols(&mut self, f: impl Fn(u32) -> u32) {
        for x in self
            .rows
            .iter_mut()
            .chain(&mut self.cols)
            .chain(&mut self.filters)
        {
            x.col = f(x.col);
        }
        for v in &mut self.values {
            v.col = f(v.col);
        }
    }
}

impl SavedPivot {
    pub fn new(name: &str, spec: &PivotSpec, titles: &[String]) -> SavedPivot {
        let columns = spec
            .used_cols()
            .into_iter()
            .map(|c| (c, titles.get(c as usize).cloned().unwrap_or_default()))
            .collect();
        SavedPivot {
            version: SAVED_PIVOT_VERSION,
            name: name.chars().take(200).collect(),
            spec: spec.clone(),
            columns,
        }
    }

    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        let b = serde_json::to_vec_pretty(self)
            .map_err(|e| Error::Invalid(format!("could not save the pivot layout: {e}")))?;
        if b.len() > MAX_SAVED_PIVOT_BYTES {
            return Err(Error::Invalid(
                "the pivot layout is too large to save".into(),
            ));
        }
        Ok(b)
    }

    /// Read a saved layout (an untrusted file).
    pub fn from_json(bytes: &[u8]) -> Result<SavedPivot, Error> {
        if bytes.len() > MAX_SAVED_PIVOT_BYTES {
            return Err(Error::Invalid("that pivot layout file is too large".into()));
        }
        let s: SavedPivot = serde_json::from_slice(bytes)
            .map_err(|e| Error::Invalid(format!("that is not a pivot layout file: {e}")))?;
        if s.version == 0 || s.version > SAVED_PIVOT_VERSION {
            return Err(Error::Invalid(
                "that pivot layout was saved by a newer version of the app".into(),
            ));
        }
        if s.columns.len() > MAX_FIELDS + MAX_VALUES + MAX_FILTERS {
            return Err(Error::Invalid(
                "that pivot layout uses too many columns".into(),
            ));
        }
        Ok(s)
    }

    /// The spec fitted to a file with these column titles: each column is
    /// found by its title (exact, then ignoring case); a column whose title
    /// is not there keeps its number only if that number has the same
    /// title or no titles were saved.
    pub fn fit(&self, titles: &[String]) -> Result<PivotSpec, Error> {
        let (spec, swaps) = self.fit_by_place(titles)?;
        match swaps.first() {
            Some(s) => Err(Error::Invalid(format!(
                "this file has no column \"{}\" that the pivot layout uses",
                s.was
            ))),
            None => Ok(spec),
        }
    }

    /// `fit`, but a column whose title is not found (renamed, say) takes
    /// the column in the same place, and is listed so the person can be
    /// asked first. Fails when that place is past the file's last column.
    pub fn fit_by_place(&self, titles: &[String]) -> Result<(PivotSpec, Vec<PlaceSwap>), Error> {
        let mut map: HashMap<u32, u32> = HashMap::new();
        let mut swaps: Vec<PlaceSwap> = Vec::new();
        for (c, t) in &self.columns {
            let exact: Vec<usize> = (0..titles.len()).filter(|&i| titles[i] == *t).collect();
            let loose: Vec<usize> = (0..titles.len())
                .filter(|&i| titles[i].to_lowercase() == t.to_lowercase())
                .collect();
            let to = if titles.get(*c as usize) == Some(t) {
                *c
            } else if exact.len() == 1 {
                exact[0] as u32
            } else if loose.len() == 1 {
                loose[0] as u32
            } else if t.is_empty() && (*c as usize) < titles.len() {
                *c
            } else if (*c as usize) < titles.len() {
                swaps.push(PlaceSwap {
                    was: t.clone(),
                    col: *c,
                    now: titles[*c as usize].clone(),
                });
                *c
            } else {
                return Err(Error::Invalid(format!(
                    "this file has no column \"{t}\" that the pivot layout uses"
                )));
            };
            map.insert(*c, to);
        }
        let mut spec = self.spec.clone();
        let n = titles.len() as u32;
        spec.map_cols(|c| map.get(&c).copied().unwrap_or(c));
        if spec.used_cols().iter().any(|&c| c >= n) {
            return Err(Error::Invalid(
                "the pivot layout uses a column this file does not have".into(),
            ));
        }
        // A column taken by place must not also be another layout
        // column's match by title.
        for s in &swaps {
            if map.values().filter(|&&v| v == s.col).count() > 1 {
                return Err(Error::Invalid(format!(
                    "this file has no column \"{}\" that the pivot layout uses",
                    s.was
                )));
            }
        }
        Ok((spec, swaps))
    }
}

/// A pivot layout column found by its place, not its title.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaceSwap {
    /// The title the layout was saved with.
    pub was: String,
    /// The column it now uses (same place).
    pub col: u32,
    /// That column's title in this file.
    pub now: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn titles(d: &Document) -> Vec<String> {
        let all: Vec<u32> = (0..d.col_count()).collect();
        d.titles(&all, true)
    }

    fn field(col: u32) -> PivotField {
        PivotField {
            col,
            ..Default::default()
        }
    }

    fn val(col: u32, agg: Agg) -> ValueField {
        ValueField {
            col,
            agg,
            ..Default::default()
        }
    }

    fn grid(d: &Document, spec: &PivotSpec) -> PivotGrid {
        let cube = d.pivot_cube(spec, None, &Progress::new()).unwrap();
        cube.layout(spec, &titles(d), None).unwrap()
    }

    fn csv(d: &Document, spec: &PivotSpec) -> String {
        String::from_utf8(grid(d, spec).to_table(false).unwrap().bytes)
            .unwrap()
            .replace("\r\n", "\n")
    }

    const SALES: &str =
        "region,month,sales,rep\nN,Jan,10,ann\nS,Jan,5,bob\nN,Feb,7,ann\nN,Jan,1,cy\nS,Feb,x,bob\n";

    #[test]
    fn rows_columns_and_totals() {
        let d = doc(SALES);
        let spec = PivotSpec {
            rows: vec![field(0)],
            cols: vec![field(1)],
            values: vec![val(2, Agg::Sum)],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "Sum of sales,month,,\nregion,Feb,Jan,Grand Total\nN,7,11,18\nS,,5,5\nGrand Total,7,16,23\n"
        );
        // No totals.
        let mut s2 = spec.clone();
        s2.layout.grand_row = false;
        s2.layout.grand_col = false;
        assert_eq!(
            csv(&d, &s2),
            "Sum of sales,month,\nregion,Feb,Jan\nN,7,11\nS,,5\n"
        );
    }

    #[test]
    fn counts_of_every_kind() {
        let d = doc("k,v\na,1\na,\na,1\na,x\nb,2\n");
        let spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![
                val(1, Agg::Count),
                val(1, Agg::CountRows),
                val(1, Agg::CountNumbers),
                val(1, Agg::CountDistinct),
                val(1, Agg::CountBlank),
            ],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "k,Count of v,Rows of v,Count numbers of v,Distinct count of v,Blank count of v\n\
             a,3,4,2,2,1\nb,1,1,1,1,0\nGrand Total,4,5,3,3,1\n"
        );
    }

    #[test]
    fn number_summaries() {
        let d = doc("k,v\na,1\na,2\na,3\na,10\nb,-4\n");
        let spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![
                val(1, Agg::Average),
                val(1, Agg::Min),
                val(1, Agg::Max),
                val(1, Agg::Median),
                val(1, Agg::StdDev),
                val(1, Agg::VarP),
                val(1, Agg::Product),
            ],
            layout: PivotLayout {
                grand_row: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let g = grid(&d, &spec);
        assert_eq!(g.rows[0].cells[..4], ["4", "1", "10", "2.5"]);
        // Sample std dev of 1,2,3,10: sqrt(50 / 3) = 4.0824...
        assert!(
            g.rows[0].cells[4].starts_with("4.08248"),
            "{}",
            g.rows[0].cells[4]
        );
        assert_eq!(g.rows[0].cells[5], "12.5");
        assert_eq!(g.rows[0].cells[6], "60");
        // One value: no sample std dev.
        assert_eq!(g.rows[1].cells[4], "");
        assert_eq!(g.rows[1].cells[3], "-4");
    }

    #[test]
    fn exact_sums() {
        let mut s = String::from("k,v\n");
        for _ in 0..100_000 {
            s.push_str("a,0.1\n");
        }
        let d = doc(&s);
        let spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![val(1, Agg::Sum)],
            ..Default::default()
        };
        let g = grid(&d, &spec);
        assert_eq!(g.rows[0].cells[0], "10000");
    }

    #[test]
    fn case_and_spaces() {
        let d = doc("k,v\nSmith ,1\nsmith,1\nSMITH,1\n,1\n");
        let mut spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![val(1, Agg::Sum)],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "k,Sum of v\nSmith,3\n(blank),1\nGrand Total,4\n"
        );
        spec.match_case = true;
        assert_eq!(
            csv(&d, &spec),
            "k,Sum of v\nSMITH,1\nSmith,1\nsmith,1\n(blank),1\nGrand Total,4\n"
        );
        spec.layout.blank_label = "-".into();
        spec.keep_spaces = true;
        let out = csv(&d, &spec);
        assert!(out.contains("Smith ,1"), "{out}");
        assert!(out.contains("\n-,1\n"), "{out}");
    }

    #[test]
    fn date_groups_and_bins() {
        let d = doc(
            "when,n\n2024-01-05,1\n2024-02-10,2\n2023-12-31 13:30,4\n2024-01-20,8\nnope,16\n,32\n",
        );
        let mut spec = PivotSpec {
            rows: vec![
                PivotField {
                    col: 0,
                    group: Grouping::Year,
                    ..Default::default()
                },
                PivotField {
                    col: 0,
                    group: Grouping::Month,
                    ..Default::default()
                },
            ],
            values: vec![val(1, Agg::Sum)],
            layout: PivotLayout {
                repeat_labels: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "when,when,Sum of n\n2023,,4\n2023,Dec,4\n2024,,11\n2024,Jan,9\n2024,Feb,2\n\
             (not a date),,16\n(not a date),(not a date),16\n(blank),,32\n(blank),(blank),32\n\
             Grand Total,,63\n"
        );
        spec.rows = vec![PivotField {
            col: 0,
            group: Grouping::Hour,
            ..Default::default()
        }];
        assert!(csv(&d, &spec).contains("13:00,4"));
        let d = doc("n\n1\n9\n10\n25\n-3\n100\nx\n");
        let spec = PivotSpec {
            rows: vec![PivotField {
                col: 0,
                group: Grouping::Bins {
                    start: 0.0,
                    size: 10.0,
                    end: Some(50.0),
                },
                ..Default::default()
            }],
            values: vec![val(0, Agg::Count)],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "n,Count of n\n< 0,1\n0-9,2\n10-19,1\n20-29,1\n50 and up,1\n(not a number),1\nGrand Total,7\n"
        );
    }

    #[test]
    fn picks_and_report_filters() {
        let d = doc(SALES);
        let spec = PivotSpec {
            rows: vec![PivotField {
                col: 3,
                items: Some(ItemPick {
                    exclude: true,
                    items: vec!["cy".into()],
                }),
                ..Default::default()
            }],
            filters: vec![PivotField {
                col: 1,
                items: Some(ItemPick {
                    exclude: false,
                    items: vec!["jan".into()],
                }),
                ..Default::default()
            }],
            values: vec![val(2, Agg::Sum)],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "rep,Sum of sales\nann,10\nbob,5\nGrand Total,15\n"
        );
    }

    #[test]
    fn filters_top_and_sorting() {
        let d = doc("k,v\na,5\nb,1\nc,9\nd,3\ne,7\n");
        let mut spec = PivotSpec {
            rows: vec![PivotField {
                col: 0,
                top: Some(TopN {
                    n: 2,
                    value: 0,
                    bottom: false,
                }),
                ..Default::default()
            }],
            values: vec![val(1, Agg::Sum)],
            ..Default::default()
        };
        // Totals leave filtered items out.
        assert_eq!(csv(&d, &spec), "k,Sum of v\nc,9\ne,7\nGrand Total,16\n");
        spec.rows[0].top = None;
        spec.rows[0].value_filter = Some(ValueFilter {
            value: 0,
            op: CmpOp::Between,
            a: 3.0,
            b: 7.0,
        });
        spec.rows[0].sort = FieldSort {
            by: SortBy::Value,
            value: 0,
            desc: true,
        };
        assert_eq!(
            csv(&d, &spec),
            "k,Sum of v\ne,7\na,5\nd,3\nGrand Total,15\n"
        );
        spec.rows[0].value_filter = None;
        spec.rows[0].label_filter = Some(LabelFilter {
            op: LabelOp::NotContains,
            text: "A".into(),
            text2: String::new(),
        });
        spec.rows[0].sort = FieldSort {
            by: SortBy::Label,
            value: 0,
            desc: true,
        };
        assert_eq!(
            csv(&d, &spec),
            "k,Sum of v\ne,7\nd,3\nc,9\nb,1\nGrand Total,20\n"
        );
        // Numbers sort as numbers.
        let d = doc("k\n10\n9\n100\n");
        let spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![val(0, Agg::Count)],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "k,Count of k\n9,1\n10,1\n100,1\nGrand Total,3\n"
        );
    }

    #[test]
    fn show_values_as() {
        let d = doc(SALES);
        let mut spec = PivotSpec {
            rows: vec![field(0)],
            cols: vec![field(1)],
            values: vec![ValueField {
                col: 2,
                agg: Agg::Sum,
                show_as: ShowAs::PctOfGrand,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = csv(&d, &spec);
        assert!(out.contains("N,30.43%,47.83%,78.26%"), "{out}");
        assert!(out.contains("Grand Total,30.43%,69.57%,100.00%"), "{out}");
        spec.values[0].show_as = ShowAs::PctOfRow;
        assert!(csv(&d, &spec).contains("S,,100.00%,100.00%"));
        spec.values[0].show_as = ShowAs::RunningTotal;
        let out = csv(&d, &spec);
        assert!(
            out.contains("N,7,11,18\nS,,16,23\nGrand Total,,,\n"),
            "{out}"
        );
        spec.values[0].show_as = ShowAs::DifferenceFromPrevious;
        spec.values[0].along = Along::Columns;
        let out = csv(&d, &spec);
        assert!(out.contains("N,,4,\n"), "{out}");
        spec.values[0].show_as = ShowAs::RankLargest;
        spec.values[0].along = Along::Rows;
        let out = csv(&d, &spec);
        assert!(out.contains("N,1,1,1\nS,,2,2\n"), "{out}");
    }

    #[test]
    fn formats() {
        let f = NumFormat {
            decimals: Some(2),
            thousands: true,
            prefix: "$".into(),
            suffix: String::new(),
        };
        assert_eq!(fmt_num(-1234567.891, &f, false, false), "-$1,234,567.89");
        assert_eq!(fmt_num(1234.5, &f, false, true), "$1.234,50");
        assert_eq!(
            fmt_num(0.1234, &NumFormat::default(), true, false),
            "12.34%"
        );
        let two = NumFormat {
            decimals: Some(2),
            ..Default::default()
        };
        assert_eq!(fmt_num(-0.0001, &two, false, false), "0.00");
        let th = NumFormat {
            thousands: true,
            ..Default::default()
        };
        assert_eq!(fmt_num(999.0, &th, false, false), "999");
    }

    #[test]
    fn subtotals_and_collapse() {
        let d = doc("a,b,n\nx,p,1\nx,q,2\ny,p,4\n");
        let mut spec = PivotSpec {
            rows: vec![field(0), field(1)],
            values: vec![val(2, Agg::Sum)],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "a,b,Sum of n\nx,,3\n,p,1\n,q,2\ny,,4\n,p,4\nGrand Total,,7\n"
        );
        spec.layout.subtotals_top = false;
        assert_eq!(
            csv(&d, &spec),
            "a,b,Sum of n\nx,,\n,p,1\n,q,2\nx Total,,3\ny,,\n,p,4\ny Total,,4\nGrand Total,,7\n"
        );
        spec.layout.collapsed_rows = vec![vec!["x".into()]];
        assert_eq!(
            csv(&d, &spec),
            "a,b,Sum of n\nx,,3\ny,,\n,p,4\ny Total,,4\nGrand Total,,7\n"
        );
        spec.layout.row_subtotals = false;
        spec.layout.collapsed_rows.clear();
        spec.layout.row_depth = 1;
        spec.layout.expanded_rows = vec![vec!["y".into()]];
        assert_eq!(
            csv(&d, &spec),
            "a,b,Sum of n\nx,,3\ny,,\n,p,4\nGrand Total,,7\n"
        );
        let g = grid(&d, &spec);
        assert_eq!(g.rows[0].toggle, Some(true));
        assert_eq!(g.rows[1].toggle, Some(false));
    }

    #[test]
    fn column_groups() {
        let d = doc("a,b,c,n\nr,x,p,1\nr,x,q,2\nr,y,p,4\n");
        let mut spec = PivotSpec {
            rows: vec![field(0)],
            cols: vec![field(1), field(2)],
            values: vec![val(3, Agg::Sum), val(3, Agg::Count)],
            ..Default::default()
        };
        let g = grid(&d, &spec);
        // Title row, two item rows, value names.
        assert_eq!(g.head.len(), 4);
        let spans: Vec<(String, u32)> = g.head[1][1..]
            .iter()
            .map(|c| (c.text.clone(), c.span))
            .collect();
        assert_eq!(
            spans,
            [
                ("x".to_string(), 4),
                ("x Total".to_string(), 2),
                ("y".to_string(), 2),
                ("y Total".to_string(), 2),
                ("Grand Total".to_string(), 2)
            ]
        );
        assert_eq!(
            g.rows[0].cells,
            ["1", "1", "2", "1", "3", "2", "4", "1", "4", "1", "7", "3"]
        );
        spec.layout.collapsed_cols = vec![vec!["x".into()]];
        spec.layout.col_subtotals = false;
        let g = grid(&d, &spec);
        assert_eq!(g.rows[0].cells, ["3", "2", "4", "1", "7", "3"]);
        assert_eq!(g.head[1][1].toggle, Some(true));
        assert_eq!(g.cols[0].kind, Kind::Group);
    }

    #[test]
    fn text_summaries() {
        let d = doc("k,v\na,one\nb,two\na,three\na,\n");
        let spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![val(1, Agg::First), val(1, Agg::Last), val(1, Agg::Join)],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            "k,First of v,Last of v,List of v\na,one,three,\"one, three\"\nb,two,two,two\n\
             Grand Total,one,three,\"one, three, two\"\n"
        );
    }

    #[test]
    fn no_fields() {
        let d = doc(SALES);
        let spec = PivotSpec {
            values: vec![val(2, Agg::Sum), val(0, Agg::Count)],
            ..Default::default()
        };
        assert_eq!(
            csv(&d, &spec),
            ",Sum of sales,Count of region\nTotal,23,5\n"
        );
        let spec = PivotSpec {
            rows: vec![field(0)],
            ..Default::default()
        };
        assert_eq!(csv(&d, &spec), "region\nN\nS\nGrand Total\n");
    }

    #[test]
    fn items_and_drill() {
        let d = doc(SALES);
        let spec = PivotSpec {
            rows: vec![field(0)],
            cols: vec![field(1)],
            values: vec![val(2, Agg::CountRows)],
            ..Default::default()
        };
        let p = Progress::new();
        let items = d.pivot_items(&field(1), &spec, &p).unwrap();
        assert_eq!(items.total, 2);
        assert_eq!(items.items[1].label, "Jan");
        assert_eq!(items.items[1].rows, 3);
        let cube = d.pivot_cube(&spec, None, &p).unwrap();
        let t = d
            .pivot_drill(&spec, &cube, &["n".into()], &["jan".into()], &p)
            .unwrap();
        assert_eq!(
            String::from_utf8(t.bytes).unwrap(),
            "region,month,sales,rep\r\nN,Jan,10,ann\r\nN,Jan,1,cy\r\n"
        );
        let t = d.pivot_drill(&spec, &cube, &[], &[], &p).unwrap();
        assert_eq!(t.rows, 6);
        assert!(d
            .pivot_drill(&spec, &cube, &["zz".into()], &[], &p)
            .is_err());
    }

    #[test]
    fn cancel_budget_and_checks() {
        let mut s = String::from("k,v\n");
        for i in 0..200_000 {
            s.push_str(&format!("{i},{i}\n"));
        }
        let d = doc(&s);
        let spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![val(1, Agg::Median)],
            ..Default::default()
        };
        let p = Progress::new();
        p.cancel.store(true, Ordering::Relaxed);
        assert!(matches!(
            d.pivot_cube(&spec, None, &p),
            Err(Error::Cancelled)
        ));
        let p = Progress::new();
        assert!(d.pivot_cube_within(&spec, None, &p, 1 << 20).is_err());
        let cube = d.pivot_cube(&spec, Some(1000), &p).unwrap();
        assert!(cube.sampled);
        assert_eq!(cube.rows_read, 1000);
        let g = cube
            .layout(&spec, &titles(&d), Some(GridLimit { rows: 10, cols: 10 }))
            .unwrap();
        assert!(g.truncated);
        assert_eq!(g.rows.len(), 10);
        assert_eq!(g.total_rows, 1001);
        let bad = PivotSpec {
            rows: vec![field(9)],
            ..Default::default()
        };
        assert!(d.pivot_cube(&bad, None, &p).is_err());
        let bad = PivotSpec {
            rows: vec![PivotField {
                col: 0,
                group: Grouping::Bins {
                    start: 0.0,
                    size: 0.0,
                    end: None,
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(d.pivot_cube(&bad, None, &p).is_err());
    }

    #[test]
    fn saved_layouts_fit_moved_columns() {
        let spec = PivotSpec {
            rows: vec![field(1)],
            values: vec![val(2, Agg::Sum)],
            ..Default::default()
        };
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let saved = SavedPivot::new("by city", &spec, &t(&["id", "city", "sales"]));
        let back = SavedPivot::from_json(&saved.to_json().unwrap()).unwrap();
        assert_eq!(back, saved);
        let fitted = back.fit(&t(&["sales", "x", "City", "id"])).unwrap();
        assert_eq!((fitted.rows[0].col, fitted.values[0].col), (2, 0));
        assert!(back.fit(&t(&["id", "town", "sales"])).is_err());
        // After a rename (city -> town), the column in the same place is
        // offered, and listed so the person is asked first.
        let (spec2, swaps) = back.fit_by_place(&t(&["id", "town", "sales"])).unwrap();
        assert_eq!((spec2.rows[0].col, spec2.values[0].col), (1, 2));
        assert_eq!(
            swaps,
            vec![PlaceSwap {
                was: "city".into(),
                col: 1,
                now: "town".into()
            }]
        );
        // Nothing in that place, or that place already taken by another
        // of the layout's columns found by name: no offer.
        assert!(back.fit_by_place(&t(&["id"])).is_err());
        assert!(back.fit_by_place(&t(&["id", "sales", "x"])).is_err());
        assert_eq!(
            back.fit_by_place(&t(&["id", "city", "sales"])).unwrap().1,
            vec![]
        );
        assert!(SavedPivot::from_json(b"{}").is_err());
        let mut newer = saved.clone();
        newer.version = 99;
        assert!(SavedPivot::from_json(&newer.to_json().unwrap()).is_err());
    }

    #[test]
    fn layout_checks_specs_against_the_cube() {
        let d = doc(SALES);
        let spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![val(2, Agg::Sum)],
            ..Default::default()
        };
        let cube = d.pivot_cube(&spec, None, &Progress::new()).unwrap();
        let t = titles(&d);
        let mut bad = spec.clone();
        bad.rows[0].sort = FieldSort {
            by: SortBy::Value,
            value: 99,
            desc: false,
        };
        assert!(cube.layout(&bad, &t, None).is_err());
        let mut bad = spec.clone();
        bad.rows[0].top = Some(TopN {
            n: 1,
            value: 5,
            bottom: false,
        });
        assert!(cube.layout(&bad, &t, None).is_err());
        let mut other = spec.clone();
        other.values[0].agg = Agg::Median;
        assert!(cube.layout(&other, &t, None).is_err());
        assert!(cube.layout(&spec, &t, None).is_ok());
    }

    #[test]
    fn distinct_count_follows_item_rules() {
        let d = doc("k,v\na,Ann\na,ann\na,ANN \na,Bob\n");
        let mut spec = PivotSpec {
            rows: vec![field(0)],
            values: vec![val(1, Agg::CountDistinct)],
            layout: PivotLayout {
                grand_row: false,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(grid(&d, &spec).rows[0].cells, ["2"]);
        spec.match_case = true;
        assert_eq!(grid(&d, &spec).rows[0].cells, ["4"]);
        spec.match_case = false;
        spec.keep_spaces = true;
        assert_eq!(grid(&d, &spec).rows[0].cells, ["3"]);
    }

    #[test]
    fn layout_stops_and_shares_the_memory_limit() {
        let mut s = String::from("k,g,v\n");
        for i in 0..100_000 {
            s.push_str(&format!("{},{},{i}\n", i % 5_000, i % 7));
        }
        let d = doc(&s);
        let spec = PivotSpec {
            rows: vec![field(1), field(0)],
            values: vec![val(2, Agg::Median), val(2, Agg::CountDistinct)],
            ..Default::default()
        };
        let p = Progress::new();
        let cube = d.pivot_cube(&spec, None, &p).unwrap();
        let t = titles(&d);
        let stop = Progress::new();
        stop.cancel.store(true, Ordering::Relaxed);
        assert!(matches!(
            cube.layout_with(&spec, &t, None, &stop),
            Err(Error::Cancelled)
        ));
        assert_eq!(cube.used.load(Ordering::Relaxed), cube.bytes);
        // Totals are charged to the cube's budget and kept with it.
        cube.layout(&spec, &t, None).unwrap();
        let with_memo = cube.used.load(Ordering::Relaxed);
        assert!(with_memo > cube.bytes);
        cube.layout(&spec, &t, None).unwrap();
        assert_eq!(cube.used.load(Ordering::Relaxed), with_memo);
        // A cube built near its limit cannot add big totals.
        let tight = [101, 105, 110, 120]
            .iter()
            .find_map(|pc| {
                d.pivot_cube_within(&spec, None, &p, cube.bytes * pc / 100)
                    .ok()
            })
            .unwrap();
        assert!(tight.layout(&spec, &t, None).is_err());
        assert_eq!(tight.used.load(Ordering::Relaxed), tight.bytes);
    }

    /// Random data and fields: every cell matches a plain recount, and the
    /// parallel merge (over many chunks) changes nothing.
    #[test]
    fn oracle() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let rows = 3 * CHUNK + 777;
        let mut s = String::from("a,b,c,n\n");
        let mut data = Vec::new();
        for _ in 0..rows {
            let a = ["x", "X", "y", "z", ""][rnd(5) as usize];
            let b = rnd(7);
            let c = rnd(3);
            let n = rnd(1000) as i64 - 300;
            s.push_str(&format!("{a},{b},{c},{n}\n"));
            data.push((a.to_lowercase(), b, c, n));
        }
        let d = doc(&s);
        let spec = PivotSpec {
            rows: vec![field(0), field(1)],
            cols: vec![field(2)],
            values: vec![val(3, Agg::Sum), val(3, Agg::CountRows), val(3, Agg::Max)],
            layout: PivotLayout {
                subtotals_top: false,
                repeat_labels: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let g = grid(&d, &spec);
        let mut checked = 0;
        for r in &g.rows {
            if r.kind != Kind::Item {
                continue;
            }
            let a = r.path[0].clone();
            let b: u64 = r.path[1].parse().unwrap();
            for (i, col) in g.cols.iter().enumerate() {
                let hit: Vec<i64> = data
                    .iter()
                    .filter(|x| x.0 == a && x.1 == b)
                    .filter(|x| col.kind == Kind::Grand || x.2.to_string() == col.path[0])
                    .map(|x| x.3)
                    .collect();
                let want = match col.value {
                    0 => hit.iter().sum::<i64>().to_string(),
                    1 => hit.len().to_string(),
                    _ => hit.iter().max().unwrap().to_string(),
                };
                assert_eq!(r.cells[i], want, "row {:?} col {:?}", r.path, col);
                checked += 1;
            }
        }
        assert!(checked > 100);
        let grand = g.rows.last().unwrap();
        assert_eq!(grand.kind, Kind::Grand);
        assert_eq!(grand.cells[g.cols.len() - 2], rows.to_string());
    }
}
