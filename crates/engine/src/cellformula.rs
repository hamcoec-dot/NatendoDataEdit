//! One-shot cell formulas: `=A2*2`, `=SUM(B2:B9)` or `=[Price]*[Qty]` typed
//! in a cell. The formula runs once and the cell keeps its result (the
//! formula is remembered beside it, for F2 and the cell bar, until the cell
//! changes).
//!
//! A1 references name a column by letter (A is the first column shown) and a
//! row by the number in its row heading. They are worked out before the
//! expression runs: a single cell becomes its text and a range (`B2:B9`)
//! becomes the list of its cells, so every expression function works on
//! them. Filled into several cells at once (Ctrl+Enter), references move
//! with each cell as they do in a spreadsheet, unless they carry a `$`.

use serde::{Deserialize, Serialize};

use rayon::prelude::*;

use crate::document::{Document, Progress};
use crate::expr::{to_text_owned, Ctx, Expr, Lookups, RangeAgg, RefSource, Value};
use crate::ops::{check_cols, Cols};
use crate::value::ReadOptions;
use crate::Error;

/// Cells one formula may fill at once.
pub const MAX_FORMULA_CELLS: usize = 100_000;
/// Referenced cells read for one formula, summed over the cells it fills.
pub const MAX_FORMULA_READS: u64 = 20_000_000;
/// Cells one formula may read for a single cell (its ranges together).
pub const MAX_RANGE_CELLS: u64 = 1_000_000;
/// Formulas remembered per table (beyond this, results are still written).
pub const MAX_REMEMBERED: usize = 1_000_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CellFormula {
    /// As typed, with or without its leading `=`.
    pub formula: String,
    /// The cell it was typed in: shown row, logical column.
    pub row: u64,
    pub col: u32,
    /// Every cell it fills (shown row, logical column); empty: just that one.
    #[serde(default)]
    pub targets: Vec<(u64, u32)>,
    /// The first row holds titles (`[Title]` names come from it).
    pub keep_first: bool,
    pub read: ReadOptions,
    /// Local date and time as days since 1970 (TODAY, NOW).
    pub now: f64,
    #[serde(skip)]
    pub tables: Lookups,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FormulaResult {
    /// The cells to write: shown row, logical column, value.
    pub cells: Vec<(u64, u32, String)>,
    /// Each written cell's own formula (references moved), same order.
    #[serde(skip)]
    pub formulas: Vec<String>,
    /// Cells whose result was an error (left as they were).
    pub errors: u64,
    /// The first error, as the cell would show it, with its cell.
    pub first_error: Option<String>,
    /// The table's revision the results were worked out on (they are
    /// written only to that same version).
    #[serde(skip)]
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ref {
    /// Logical column (A = 0).
    col: i64,
    col_fixed: bool,
    /// Document position (row heading number - 1).
    row: i64,
    row_fixed: bool,
}

/// Column letters to a number from 0 (A, Z, AA, XFD).
fn letters_to_col(s: &str) -> Option<i64> {
    let mut n: i64 = 0;
    for c in s.chars() {
        let d = (c.to_ascii_uppercase() as i64) - ('A' as i64) + 1;
        if !(1..=26).contains(&d) {
            return None;
        }
        n = n.checked_mul(26)?.checked_add(d)?;
    }
    (n > 0).then_some(n - 1)
}

/// A column number from 0 to letters.
pub fn col_letters(mut c: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A reference at `chars[i..]`: `$A$1`, `b12`, `AA$3`. Its length too.
fn parse_ref(chars: &[char], i: usize) -> Option<(Ref, usize)> {
    let mut j = i;
    let col_fixed = chars.get(j) == Some(&'$');
    if col_fixed {
        j += 1;
    }
    let ls = j;
    while j < chars.len() && chars[j].is_ascii_alphabetic() && j - ls < 3 {
        j += 1;
    }
    if j == ls {
        return None;
    }
    let letters: String = chars[ls..j].iter().collect();
    let row_fixed = chars.get(j) == Some(&'$');
    if row_fixed {
        j += 1;
    }
    let ds = j;
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
    }
    if j == ds || j - ds > 12 {
        return None;
    }
    // Not part of a longer name or a function (LOG10( is a function).
    if chars
        .get(j)
        .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '(' || *c == '.')
    {
        return None;
    }
    let row: i64 = chars[ds..j].iter().collect::<String>().parse().ok()?;
    if row < 1 {
        return None;
    }
    Some((
        Ref {
            col: letters_to_col(&letters)?,
            col_fixed,
            row: row - 1,
            row_fixed,
        },
        j - i,
    ))
}

impl Document {
    /// A cell's text by row id and logical column.
    fn cell_text_by_id(&self, id: u32, col: u32) -> String {
        let reader = Cols::new(self, &[col]);
        let mut buf = Vec::new();
        reader.read(self, id, &mut buf);
        reader.get(&buf, 0).to_string()
    }
}
/// One filled cell: (row, column, value, its formula), or the error.
type CellOutcome = Result<(u64, u32, String, String), String>;

/// A range in a formula: two corners, or whole columns (`C:C`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RangeRef {
    a: Ref,
    b: Ref,
    /// `C:C`: every data row (the rows do not move).
    whole: bool,
}

/// A piece of the formula as a filled cell keeps it.
#[derive(Debug, Clone)]
enum Seg {
    Text(String),
    Cell(usize),
    Range(usize),
}

/// The formula worked out once: the expression (references as
/// `CELLREF_(i)` and ranges as `RANGEREF_(j)`), its references, and the
/// pieces that show it again for each cell it fills.
struct Template {
    run: String,
    cells: Vec<Ref>,
    ranges: Vec<RangeRef>,
    segs: Vec<Seg>,
}

/// The functions a range may be given to (worked out from running totals).
const RANGE_FUNCS: [&str; 6] = ["SUM", "AVERAGE", "MIN", "MAX", "COUNT", "COUNTA"];

/// Whole columns at `chars[i..]`: `C:C`, `$A:$C`. Its length too.
fn parse_cols(chars: &[char], i: usize) -> Option<(RangeRef, usize)> {
    let col = |j: usize| -> Option<(i64, bool, usize)> {
        let mut k = j;
        let fixed = chars.get(k) == Some(&'$');
        if fixed {
            k += 1;
        }
        let s = k;
        while k < chars.len() && chars[k].is_ascii_alphabetic() && k - s < 3 {
            k += 1;
        }
        if k == s
            || chars
                .get(k)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '(')
        {
            return None;
        }
        let letters: String = chars[s..k].iter().collect();
        Some((letters_to_col(&letters)?, fixed, k))
    };
    let (c1, f1, k) = col(i)?;
    if chars.get(k) != Some(&':') {
        return None;
    }
    let (c2, f2, end) = col(k + 1)?;
    let r = |c, f| Ref {
        col: c,
        col_fixed: f,
        row: 0,
        row_fixed: true,
    };
    Some((
        RangeRef {
            a: r(c1, f1),
            b: r(c2, f2),
            whole: true,
        },
        end - i,
    ))
}

fn show_ref(r: Ref) -> String {
    format!(
        "{}{}{}{}",
        if r.col_fixed { "$" } else { "" },
        col_letters(r.col.max(0) as u32),
        if r.row_fixed { "$" } else { "" },
        r.row + 1
    )
}

impl Template {
    /// Read the formula's references (nothing is read from the table).
    fn new(src: &str) -> Result<Template, Error> {
        let chars: Vec<char> = src.chars().collect();
        let mut t = Template {
            run: String::with_capacity(src.len()),
            cells: Vec::new(),
            ranges: Vec::new(),
            segs: Vec::new(),
        };
        let mut text = String::new();
        let mut open: Vec<String> = Vec::new();
        let mut word = String::new();
        let mut i = 0;
        let flush = |text: &mut String, segs: &mut Vec<Seg>| {
            if !text.is_empty() {
                segs.push(Seg::Text(std::mem::take(text)));
            }
        };
        while i < chars.len() {
            let c = chars[i];
            if c == '"' || c == '[' {
                // Text and [column] names, as they are.
                let close = if c == '"' { '"' } else { ']' };
                let s = i;
                i += 1;
                while i < chars.len() {
                    if chars[i] == close {
                        if chars.get(i + 1) == Some(&close) {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                let piece: String = chars[s..i].iter().collect();
                t.run.push_str(&piece);
                text.push_str(&piece);
                word.clear();
                continue;
            }
            let boundary = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
            if boundary && (c == '$' || c.is_ascii_alphabetic()) {
                // A range (B2:B9 or C:C), or one cell (B2).
                let range = match parse_ref(&chars, i) {
                    Some((a, n)) if chars.get(i + n) == Some(&':') => parse_ref(&chars, i + n + 1)
                        .map(|(b, n2)| (RangeRef { a, b, whole: false }, n + 1 + n2)),
                    Some(_) => None,
                    None => parse_cols(&chars, i),
                };
                if let Some((r, n)) = range {
                    let inside = open.last().map(String::as_str);
                    if !inside.is_some_and(|f| RANGE_FUNCS.contains(&f)) {
                        return Err(Error::Invalid(
                            "a range like B2:B9 goes inside SUM, AVERAGE, MIN, MAX, COUNT or COUNTA"
                                .into(),
                        ));
                    }
                    flush(&mut text, &mut t.segs);
                    t.run.push_str(&format!("RANGEREF_({})", t.ranges.len()));
                    t.segs.push(Seg::Range(t.ranges.len()));
                    t.ranges.push(r);
                    i += n;
                    word.clear();
                    continue;
                }
                if let Some((r, n)) = parse_ref(&chars, i) {
                    flush(&mut text, &mut t.segs);
                    t.run.push_str(&format!("CELLREF_({})", t.cells.len()));
                    t.segs.push(Seg::Cell(t.cells.len()));
                    t.cells.push(r);
                    i += n;
                    word.clear();
                    continue;
                }
            }
            if c.is_alphanumeric() || c == '_' || c == '.' {
                word.push(c);
            } else {
                if c == '(' {
                    open.push(word.to_ascii_uppercase());
                } else if c == ')' {
                    open.pop();
                }
                word.clear();
            }
            t.run.push(c);
            text.push(c);
            i += 1;
        }
        flush(&mut text, &mut t.segs);
        Ok(t)
    }

    /// The formula as the cell `(dr, dc)` from where it was typed keeps it.
    fn shown(&self, dr: i64, dc: i64) -> String {
        let mut out = String::from("=");
        for s in &self.segs {
            match s {
                Seg::Text(t) => out.push_str(t),
                Seg::Cell(i) => out.push_str(&show_ref(moved(self.cells[*i], dr, dc))),
                Seg::Range(j) => {
                    let r = self.ranges[*j];
                    if r.whole {
                        let c = |x: Ref| {
                            let m = moved(x, 0, dc);
                            format!(
                                "{}{}",
                                if m.col_fixed { "$" } else { "" },
                                col_letters(m.col.max(0) as u32)
                            )
                        };
                        out.push_str(&format!("{}:{}", c(r.a), c(r.b)));
                    } else {
                        out.push_str(&show_ref(moved(r.a, dr, dc)));
                        out.push(':');
                        out.push_str(&show_ref(moved(r.b, dr, dc)));
                    }
                }
            }
        }
        out
    }
}

/// A reference moved by `(dr, dc)` unless it is fixed (`$`).
fn moved(r: Ref, dr: i64, dc: i64) -> Ref {
    Ref {
        col: if r.col_fixed { r.col } else { r.col + dc },
        row: if r.row_fixed { r.row } else { r.row + dr },
        ..r
    }
}

/// The cells a formula reads, kept by column over the rows its fills span.
/// Only what is needed is kept: the text of columns single references read,
/// and for ranges running totals (sum, numbers, filled cells) so a range's
/// SUM, AVERAGE, COUNT and COUNTA cost the same at any size (plus each
/// cell's number where MIN or MAX scan it).
struct Sheet {
    cols: std::collections::HashMap<i64, ColData>,
}

#[derive(Default)]
struct ColData {
    /// Document position of the column's first row kept (each column keeps
    /// only the rows the formula reads in it).
    lo: i64,
    /// Cell text (single references only; else empty).
    text: Vec<String>,
    /// The number in each cell, NaN when it is not one (MIN and MAX only).
    num: Vec<f64>,
    /// Running totals, one longer than the rows (index 0 is nothing);
    /// empty when no range reads the column.
    sum: Vec<f64>,
    nums: Vec<u32>,
    filled: Vec<u32>,
}

/// What a formula needs of each column it reads, over rows `lo..=hi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Need {
    text: bool,
    totals: bool,
    numbers: bool,
    lo: i64,
    hi: i64,
}

impl Default for Need {
    fn default() -> Self {
        Need {
            text: false,
            totals: false,
            numbers: false,
            lo: i64::MAX,
            hi: i64::MIN,
        }
    }
}

impl Need {
    fn span(&mut self, lo: i64, hi: i64) {
        self.lo = self.lo.min(lo);
        self.hi = self.hi.max(hi);
    }
}

impl Sheet {
    fn text(&self, row: i64, col: i64) -> Option<&str> {
        let c = self.cols.get(&col)?;
        c.text
            .get(usize::try_from(row - c.lo).ok()?)
            .map(String::as_str)
    }

    /// Rows `r1..=r2` of columns `c1..=c2` worked out.
    fn agg(&self, r1: i64, r2: i64, c1: i64, c2: i64, min_max: bool) -> Option<RangeAgg> {
        let mut g = RangeAgg {
            sum: 0.0,
            nums: 0,
            filled: 0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        };
        for c in c1..=c2 {
            let d = self.cols.get(&c)?;
            let (a, b) = (
                usize::try_from(r1 - d.lo).ok()?,
                usize::try_from(r2 - d.lo).ok()? + 1,
            );
            if b >= d.sum.len() || (min_max && b > d.num.len()) {
                return None;
            }
            g.sum += d.sum[b] - d.sum[a];
            g.nums += u64::from(d.nums[b] - d.nums[a]);
            g.filled += u64::from(d.filled[b] - d.filled[a]);
            if min_max {
                for &x in &d.num[a..b] {
                    if !x.is_nan() {
                        g.min = g.min.min(x);
                        g.max = g.max.max(x);
                    }
                }
            }
        }
        Some(g)
    }
}

/// One filled cell's view of the sheet: its references moved.
struct At<'s> {
    sheet: &'s Sheet,
    cells: Vec<Option<(i64, i64)>>,
    ranges: Vec<Option<(i64, i64, i64, i64)>>,
}

impl RefSource for At<'_> {
    fn cell(&self, i: usize) -> Option<&str> {
        let (r, c) = (*self.cells.get(i)?)?;
        self.sheet.text(r, c)
    }

    fn range(&self, j: usize, min_max: bool) -> Option<RangeAgg> {
        let (r1, r2, c1, c2) = (*self.ranges.get(j)?)?;
        self.sheet.agg(r1, r2, c1, c2, min_max)
    }
}

impl Document {
    /// The formula as an expression for the cell it was typed in, e.g. to
    /// find the tabs its LOOKUPs read (references stand in as functions).
    pub fn cell_formula_expr(&self, spec: &CellFormula) -> Result<String, Error> {
        let src = spec.formula.trim_start();
        let src = src.strip_prefix('=').unwrap_or(src);
        Ok(Template::new(src)?.run)
    }

    /// Run a cell formula over its cells, with progress and Cancel.
    /// Nothing changes here: the caller writes `cells` (one undo step,
    /// refused if the table changed since: see `revision`) and remembers
    /// `formulas`.
    pub fn run_cell_formula(
        &self,
        spec: &CellFormula,
        progress: &Progress,
    ) -> Result<FormulaResult, Error> {
        let src = spec.formula.trim_start();
        let src = src.strip_prefix('=').unwrap_or(src);
        if src.trim().is_empty() {
            return Err(Error::Invalid("the formula is empty".into()));
        }
        if src.chars().count() > crate::expr::MAX_EXPR_CHARS {
            return Err(Error::Invalid(format!(
                "a formula can be at most {} characters",
                crate::expr::MAX_EXPR_CHARS
            )));
        }
        let mut targets = spec.targets.clone();
        if targets.is_empty() {
            targets.push((spec.row, spec.col));
        }
        if targets.len() > MAX_FORMULA_CELLS {
            return Err(Error::Invalid(format!(
                "a formula fills at most {MAX_FORMULA_CELLS} cells at once; for a whole column use Data › Computed column"
            )));
        }
        let tpl = Template::new(src)?;
        let names = self.expr_names(spec.keep_first);
        let mut e = Expr::parse(&tpl.run, &names)?;
        e.bind(&spec.tables);
        check_cols(self, e.columns())?;
        let pos = |r: u64| -> Result<i64, Error> {
            self.row_number(r)
                .map(|p| p as i64)
                .ok_or(Error::OutOfRange)
        };
        let from = (pos(spec.row)?, i64::from(spec.col));
        let rows = self.total_rows() as i64;
        let ncols = i64::from(self.col_count());
        let data_lo = i64::from(spec.keep_first && rows > 0);
        // Every cell's references, moved; and what they reach in all.
        struct Placed {
            row: u64,
            col: u32,
            pos: i64,
            dr: i64,
            dc: i64,
            cells: Vec<Option<(i64, i64)>>,
            ranges: Vec<Option<(i64, i64, i64, i64)>>,
        }
        let fits = |r: i64, c: i64| r >= 0 && r < rows && c >= 0 && c < ncols;
        // Where it was typed, every reference is on the table.
        let typed_off = tpl.cells.iter().any(|x| !fits(x.row, x.col))
            || tpl.ranges.iter().any(|x| {
                let r = if x.whole {
                    data_lo
                } else {
                    x.a.row.min(x.b.row)
                };
                let r2 = if x.whole {
                    rows - 1
                } else {
                    x.a.row.max(x.b.row)
                };
                !(fits(r, x.a.col.min(x.b.col)) && fits(r2, x.a.col.max(x.b.col)))
            });
        if typed_off {
            return Err(Error::Invalid(
                "a cell reference in the formula is outside the table".into(),
            ));
        }
        let mut placed = Vec::with_capacity(targets.len());
        let (mut lo, mut hi) = (i64::MAX, i64::MIN);
        let mut want_cols: std::collections::BTreeMap<i64, Need> =
            std::collections::BTreeMap::new();
        let mut scan: u64 = 0;
        let mut min_max_ranges = vec![false; tpl.ranges.len()];
        Self::mark_min_max(&tpl.run, &mut min_max_ranges);
        for &(r, c) in &targets {
            let p = pos(r)?;
            let (dr, dc) = (p - from.0, i64::from(c) - from.1);
            let cells: Vec<Option<(i64, i64)>> = tpl
                .cells
                .iter()
                .map(|&x| {
                    let m = moved(x, dr, dc);
                    fits(m.row, m.col).then_some((m.row, m.col))
                })
                .collect();
            let ranges: Vec<Option<(i64, i64, i64, i64)>> = tpl
                .ranges
                .iter()
                .enumerate()
                .map(|(j, x)| {
                    let (a, b) = (moved(x.a, dr, dc), moved(x.b, dr, dc));
                    let (c1, c2) = (a.col.min(b.col), a.col.max(b.col));
                    let (r1, r2) = if x.whole {
                        (data_lo, rows - 1)
                    } else {
                        (a.row.min(b.row), a.row.max(b.row))
                    };
                    if !(fits(r1, c1) && fits(r2, c2)) || r1 > r2 {
                        return None;
                    }
                    if min_max_ranges[j] {
                        scan += ((r2 - r1 + 1) * (c2 - c1 + 1)) as u64;
                    }
                    Some((r1, r2, c1, c2))
                })
                .collect();
            for &(rr, cc) in cells.iter().flatten() {
                lo = lo.min(rr);
                hi = hi.max(rr);
                let n = want_cols.entry(cc).or_default();
                n.text = true;
                n.span(rr, rr);
            }
            for (j, (r1, r2, c1, c2)) in ranges
                .iter()
                .enumerate()
                .filter_map(|(j, r)| r.map(|r| (j, r)))
            {
                lo = lo.min(r1);
                hi = hi.max(r2);
                for cc in c1..=c2 {
                    let n = want_cols.entry(cc).or_default();
                    n.totals = true;
                    n.span(r1, r2);
                    n.numbers |= min_max_ranges[j];
                }
            }
            placed.push(Placed {
                row: r,
                col: c,
                pos: p,
                dr,
                dc,
                cells,
                ranges,
            });
        }
        // The cost is known before anything is read: refuse at once.
        let kept = if lo <= hi {
            want_cols
                .values()
                .map(|n| (n.hi - n.lo + 1) as u64)
                .sum::<u64>()
        } else {
            0
        };
        if kept > MAX_FORMULA_READS || scan > MAX_FORMULA_READS {
            return Err(Error::Invalid(
                "this formula reads too many cells; use a smaller range or select fewer cells"
                    .into(),
            ));
        }
        let span = if lo <= hi { (hi - lo + 1) as u64 } else { 0 };
        progress.start(span + placed.len() as u64);
        let sheet = self.formula_sheet(lo, hi, &want_cols, spec.read, progress)?;
        // Each cell, in parallel.
        let reader = Cols::new(self, e.columns());
        let tys = e.types(self, spec.read);
        let near = e.uses_near();
        let shown = self.row_count();
        let first = u64::from(spec.keep_first && shown > 0);
        let revision = self.revision();
        let parts: Vec<Vec<CellOutcome>> = placed
            .par_chunks(4096)
            .map(|chunk| -> Result<_, Error> {
                let mut out = Vec::with_capacity(chunk.len());
                let (mut buf, mut pv, mut nv) = (Vec::new(), Vec::new(), Vec::new());
                for t in chunk {
                    let id = self.doc_id_at(t.pos as u64).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    // PREV and NEXT: the shown rows above and below.
                    let (mut has_p, mut has_n) = (false, false);
                    if near {
                        if t.row > first {
                            if let Some(pid) = self.id_at(t.row - 1) {
                                reader.read(self, pid, &mut pv);
                                has_p = true;
                            }
                        }
                        if t.row + 1 < shown {
                            if let Some(nid) = self.id_at(t.row + 1) {
                                reader.read(self, nid, &mut nv);
                                has_n = true;
                            }
                        }
                    }
                    let at = At {
                        sheet: &sheet,
                        cells: t.cells.clone(),
                        ranges: t.ranges.clone(),
                    };
                    let off =
                        t.cells.iter().any(Option::is_none) || t.ranges.iter().any(Option::is_none);
                    let ctx = Ctx {
                        cells: &buf,
                        slots: reader.slots(),
                        row: t.pos as u64 + 1,
                        now: spec.now,
                        read: spec.read,
                        prev: has_p.then_some(pv.as_slice()),
                        next: has_n.then_some(nv.as_slice()),
                        refs: Some(&at),
                        types: &tys,
                    };
                    let v = if off {
                        Value::Err(crate::expr::Fault::Ref)
                    } else {
                        e.eval(&ctx)
                    };
                    out.push(match v {
                        Value::Err(_) => Err(to_text_owned(&v)),
                        v => Ok((t.row, t.col, to_text_owned(&v), tpl.shown(t.dr, t.dc))),
                    });
                }
                progress.step(chunk.len() as u64)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let mut out = FormulaResult {
            revision,
            ..Default::default()
        };
        for r in parts.into_iter().flatten() {
            match r {
                Ok((row, col, v, f)) => {
                    out.cells.push((row, col, v));
                    out.formulas.push(f);
                }
                Err(err) => {
                    out.errors += 1;
                    out.first_error.get_or_insert(err);
                }
            }
        }
        Ok(out)
    }

    /// Which ranges sit inside MIN or MAX (those are scanned, not summed).
    fn mark_min_max(run: &str, marks: &mut [bool]) {
        let mut open: Vec<String> = Vec::new();
        let mut word = String::new();
        let mut it = run.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            if c == '"' {
                // Skip text.
                while let Some((_, d)) = it.next() {
                    if d == '"' {
                        if it.peek().is_some_and(|x| x.1 == '"') {
                            it.next();
                            continue;
                        }
                        break;
                    }
                }
                word.clear();
                continue;
            }
            if c.is_alphanumeric() || c == '_' {
                word.push(c);
                continue;
            }
            if c == '(' {
                if word == "RANGEREF_" {
                    let rest = &run[i + 1..];
                    if let Some(j) = rest
                        .split(')')
                        .next()
                        .and_then(|n| n.trim().parse::<usize>().ok())
                    {
                        if open.last().is_some_and(|f| f == "MIN" || f == "MAX") {
                            if let Some(m) = marks.get_mut(j) {
                                *m = true;
                            }
                        }
                    }
                }
                open.push(word.to_ascii_uppercase());
            } else if c == ')' {
                open.pop();
            }
            word.clear();
        }
    }

    /// Read columns `cols` over document positions `lo..=hi` (in parallel),
    /// keeping only what each needs (see [`Need`]).
    fn formula_sheet(
        &self,
        lo: i64,
        hi: i64,
        cols: &std::collections::BTreeMap<i64, Need>,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Sheet, Error> {
        let mut sheet = Sheet {
            cols: std::collections::HashMap::new(),
        };
        if lo > hi || cols.is_empty() {
            return Ok(sheet);
        }
        let list: Vec<u32> = cols.keys().map(|&c| c as u32).collect();
        let ctx = self.read_ctx(read);
        let rds: Vec<crate::coltype::Reader> = list.iter().map(|&c| ctx.reader(c)).collect();
        let needs: Vec<Need> = cols.values().copied().collect();
        let reader = Cols::new(self, &list);
        let n = (hi - lo + 1) as usize;
        let chunk = 1 << 15;
        // Each part: per column, its text and numbers as needed, and running
        // totals from the part's start, over the rows that column needs.
        let parts: Vec<Vec<ColData>> = (0..n.div_ceil(chunk))
            .into_par_iter()
            .map(|k| -> Result<Vec<ColData>, Error> {
                let s = k * chunk;
                let e = (s + chunk).min(n);
                let (ps, pe) = (lo + s as i64, lo + e as i64 - 1);
                let mut out: Vec<ColData> = needs
                    .iter()
                    .map(|nd| {
                        let rows = (pe.min(nd.hi) - ps.max(nd.lo) + 1).max(0) as usize;
                        ColData {
                            lo: 0,
                            text: Vec::with_capacity(if nd.text { rows } else { 0 }),
                            num: Vec::with_capacity(if nd.numbers { rows } else { 0 }),
                            sum: Vec::with_capacity(if nd.totals { rows } else { 0 }),
                            nums: Vec::with_capacity(if nd.totals { rows } else { 0 }),
                            filled: Vec::with_capacity(if nd.totals { rows } else { 0 }),
                        }
                    })
                    .collect();
                let mut buf = Vec::new();
                for p in s..e {
                    let at = lo + p as i64;
                    if !needs.iter().any(|nd| at >= nd.lo && at <= nd.hi) {
                        continue;
                    }
                    let id = self.doc_id_at(at as u64).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for (i, o) in out.iter_mut().enumerate() {
                        let nd = needs[i];
                        if at < nd.lo || at > nd.hi {
                            continue;
                        }
                        let t = reader.get(&buf, i);
                        if nd.text {
                            o.text.push(t.to_string());
                        }
                        if nd.totals {
                            let x = rds[i].number(t);
                            if nd.numbers {
                                o.num.push(x.unwrap_or(f64::NAN));
                            }
                            let (ps, pn, pf) = (
                                o.sum.last().copied().unwrap_or(0.0),
                                o.nums.last().copied().unwrap_or(0),
                                o.filled.last().copied().unwrap_or(0),
                            );
                            o.sum.push(ps + x.unwrap_or(0.0));
                            o.nums.push(pn + u32::from(x.is_some()));
                            o.filled.push(pf + u32::from(!t.is_empty()));
                        }
                    }
                }
                progress.step((e - s) as u64)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        // Join the parts, moving their contents (each part is let go as it
        // is used).
        let mut joined: Vec<ColData> = needs
            .iter()
            .map(|nd| {
                let rows = (nd.hi - nd.lo + 1).max(0) as usize;
                let mut d = ColData {
                    lo: nd.lo,
                    text: Vec::with_capacity(if nd.text { rows } else { 0 }),
                    num: Vec::with_capacity(if nd.numbers { rows } else { 0 }),
                    ..Default::default()
                };
                if nd.totals {
                    d.sum = Vec::with_capacity(rows + 1);
                    d.nums = Vec::with_capacity(rows + 1);
                    d.filled = Vec::with_capacity(rows + 1);
                    d.sum.push(0.0);
                    d.nums.push(0);
                    d.filled.push(0);
                }
                d
            })
            .collect();
        for part in parts {
            for (d, mut pc) in joined.iter_mut().zip(part) {
                d.text.append(&mut pc.text);
                d.num.append(&mut pc.num);
                if !d.sum.is_empty() {
                    let (bs, bn, bf) = (
                        *d.sum.last().unwrap_or(&0.0),
                        *d.nums.last().unwrap_or(&0),
                        *d.filled.last().unwrap_or(&0),
                    );
                    d.sum.extend(pc.sum.iter().map(|x| bs + x));
                    d.nums.extend(pc.nums.iter().map(|x| bn + x));
                    d.filled.extend(pc.filled.iter().map(|x| bf + x));
                }
            }
        }
        for (&c, d) in list.iter().zip(joined) {
            sheet.cols.insert(i64::from(c), d);
        }
        Ok(sheet)
    }
}

impl Document {
    /// The results as a change of cells (one undo step, the path a paste
    /// takes).
    pub fn formula_plan(&self, res: &FormulaResult) -> Result<crate::ops::CellPlan, Error> {
        if res.revision != self.revision() {
            return Err(Error::Changed);
        }
        let mut cells: Vec<(u64, u32, &str)> = res
            .cells
            .iter()
            .map(|(r, c, v)| (*r, *c, v.as_str()))
            .collect();
        cells.sort_by_key(|c| (c.0, c.1));
        let mut plan = crate::ops::CellPlan::new(self);
        for (r, c, v) in cells {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            match plan.changes.last_mut() {
                Some((last, row)) if *last == id => row.push((c, v.to_string())),
                _ => plan.changes.push((id, vec![(c, v.to_string())])),
            }
            plan.cells += 1;
        }
        Ok(plan)
    }
}

/// Remembered formulas: by row id and physical column, with the value the
/// formula gave (a cell whose value changed since has lost its formula).
#[derive(Debug, Clone, Default)]
pub struct Formulas {
    map: std::collections::HashMap<(u32, u32), (String, String)>,
}

impl Formulas {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

impl Document {
    /// Remember each written cell's formula (after the write).
    pub fn remember_formulas(&mut self, res: &FormulaResult) {
        for ((r, c, v), f) in res.cells.iter().zip(&res.formulas) {
            let (Some(id), Some(p)) = (self.id_at(*r), self.phys(*c)) else {
                continue;
            };
            if self.formulas.map.len() >= MAX_REMEMBERED
                && !self.formulas.map.contains_key(&(id, p))
            {
                break;
            }
            self.formulas.map.insert((id, p), (f.clone(), v.clone()));
        }
    }

    /// The formula a cell's value came from, while the value is unchanged.
    pub fn cell_formula(&self, row: u64, col: u32) -> Option<String> {
        if self.formulas.is_empty() {
            return None;
        }
        let id = self.id_at(row)?;
        let p = self.phys(col)?;
        let (f, v) = self.formulas.map.get(&(id, p))?;
        (self.cell_text_by_id(id, col) == *v).then(|| f.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc() -> Document {
        Document::from_bytes(
            b"Item,Price,Qty\r\nPen,2,3\r\nInk,1.5,4\r\nPad,x,1\r\nCup,,5\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap()
    }

    fn one(d: &Document, f: &str, row: u64, col: u32) -> FormulaResult {
        d.run_cell_formula(
            &CellFormula {
                formula: f.into(),
                row,
                col,
                keep_first: true,
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap()
    }

    #[test]
    fn letters() {
        assert_eq!(col_letters(0), "A");
        assert_eq!(col_letters(25), "Z");
        assert_eq!(col_letters(26), "AA");
        assert_eq!(col_letters(701), "ZZ");
        assert_eq!(col_letters(702), "AAA");
        for c in [0, 5, 26, 51, 700, 16_383] {
            assert_eq!(letters_to_col(&col_letters(c)), Some(i64::from(c)));
        }
    }

    #[test]
    fn single_cells_ranges_and_names() {
        let d = doc();
        let r = one(&d, "=B2*C2", 1, 3);
        assert_eq!(r.cells, vec![(1, 3, "6".to_string())]);
        assert_eq!(r.formulas, vec!["=B2*C2".to_string()]);
        // Titles and other text are passed over by SUM, counted by COUNTA.
        assert_eq!(one(&d, "=SUM(B1:B5)", 1, 3).cells[0].2, "3.5");
        assert_eq!(one(&d, "=COUNTA(B1:B5)", 1, 3).cells[0].2, "4");
        assert_eq!(one(&d, "=AVERAGE(C2:C5, 10)", 1, 3).cells[0].2, "4.6");
        assert_eq!(one(&d, "=[Price]*[Qty]", 2, 3).cells[0].2, "6");
        assert_eq!(
            one(&d, "=A3 & \"-\" & UPPER(A4)", 1, 3).cells[0].2,
            "Ink-PAD"
        );
        // A blank cell is blank; LOG10( is a function, not a reference.
        assert_eq!(one(&d, "=ISBLANK(B5)", 1, 3).cells[0].2, "TRUE");
        assert_eq!(one(&d, "=LOG10(100)+c2", 1, 3).cells[0].2, "5");
        // Text in quotes keeps what looks like a reference.
        assert_eq!(one(&d, "=\"B2\" & B2", 1, 3).cells[0].2, "B22");
        // An error leaves the cell as it was.
        let r = one(&d, "=B4*2", 3, 3);
        assert!(r.cells.is_empty());
        assert_eq!((r.errors, r.first_error.as_deref()), (1, Some("#VALUE!")));
    }

    #[test]
    fn references_move_when_filled() {
        let d = doc();
        let spec = CellFormula {
            formula: "=B2*C2 + $C$2".into(),
            row: 1,
            col: 3,
            targets: vec![(1, 3), (2, 3), (4, 3)],
            keep_first: true,
            ..Default::default()
        };
        let r = d.run_cell_formula(&spec, &Progress::new()).unwrap();
        assert_eq!(
            r.cells,
            vec![
                (1, 3, "9".to_string()),
                (2, 3, "9".to_string()),
                (4, 3, "3".to_string())
            ]
        );
        assert_eq!(r.formulas[1], "=B3*C3 + $C$2");
        // Moved off the top of the table: #REF!, nothing written there.
        let spec = CellFormula {
            formula: "=B1".into(),
            row: 1,
            col: 3,
            targets: vec![(0, 3), (1, 3)],
            keep_first: true,
            ..Default::default()
        };
        let r = d.run_cell_formula(&spec, &Progress::new()).unwrap();
        assert_eq!(r.cells, vec![(1, 3, "Price".to_string())]);
        assert_eq!(r.first_error.as_deref(), Some("#REF!"));
    }

    #[test]
    fn bad_formulas_are_refused() {
        let d = doc();
        let bad = |f: &str| {
            d.run_cell_formula(
                &CellFormula {
                    formula: f.into(),
                    keep_first: true,
                    row: 1,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .is_err()
        };
        assert!(bad("="));
        assert!(bad("=B2 +"));
        assert!(bad("=Z99"));
        assert!(bad("=NOPE(1)"));
    }

    #[test]
    fn remembered_until_the_cell_changes() {
        let mut d = doc();
        let r = one(&d, "=B2*C2", 1, 3);
        let plan = d.formula_plan(&r).unwrap();
        d.apply_cells(plan).unwrap();
        d.remember_formulas(&r);
        assert_eq!(d.cell(1, 3).as_deref(), Some("6"));
        assert_eq!(d.cell_formula(1, 3).as_deref(), Some("=B2*C2"));
        assert_eq!(d.cell_formula(2, 3), None);
        d.set_cells(vec![(1, 3, "7".into())]).unwrap();
        assert_eq!(d.cell_formula(1, 3), None);
        // Undo brings back the value, and with it the formula.
        assert!(d.undo());
        assert_eq!(d.cell_formula(1, 3).as_deref(), Some("=B2*C2"));
    }

    #[test]
    fn ranges_are_worked_out_directly() {
        let d = doc();
        // Running totals down Price (text and blanks passed over).
        let spec = CellFormula {
            formula: "=SUM($B$2:B2)".into(),
            row: 1,
            col: 3,
            targets: vec![(1, 3), (2, 3), (3, 3), (4, 3)],
            keep_first: true,
            ..Default::default()
        };
        let r = d.run_cell_formula(&spec, &Progress::new()).unwrap();
        let v: Vec<&str> = r.cells.iter().map(|c| c.2.as_str()).collect();
        assert_eq!(v, ["2", "3.5", "3.5", "3.5"]);
        assert_eq!(r.formulas[2], "=SUM($B$2:B4)");
        // Whole columns: the data rows (titles left out).
        assert_eq!(one(&d, "=SUM(C:C)", 1, 3).cells[0].2, "13");
        assert_eq!(one(&d, "=COUNTA(B:C)", 1, 3).cells[0].2, "7");
        assert_eq!(one(&d, "=MAX(B:B)+MIN(C:C)", 1, 3).cells[0].2, "3");
        assert_eq!(one(&d, "=SUM(C:C)", 1, 3).cells.len(), 1);
        // A column read both as one cell and as a range.
        assert_eq!(one(&d, "=B3 + SUM(B2:B3) + MAX(B:B)", 1, 3).cells[0].2, "7");
        assert_eq!(one(&d, "=AVERAGE(B2:C3)", 1, 3).cells[0].2, "2.625");
        // A range anywhere else is refused.
        let bad = d.run_cell_formula(
            &CellFormula {
                formula: "=UPPER(B2:B3)".into(),
                row: 1,
                col: 3,
                keep_first: true,
                ..Default::default()
            },
            &Progress::new(),
        );
        assert!(bad.is_err());
    }

    #[test]
    fn decimal_commas_are_read_in_ranges() {
        let d = Document::from_bytes(
            b"a;b\r\n1,5;x\r\n2,5;y\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let read = ReadOptions {
            decimal_comma: true,
            ..Default::default()
        };
        let r = d
            .run_cell_formula(
                &CellFormula {
                    formula: "=SUM(A2:A3)".into(),
                    row: 1,
                    col: 1,
                    keep_first: true,
                    read,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(r.cells[0].2, "4");
    }

    #[test]
    fn a_changed_table_is_not_written() {
        let mut d = doc();
        let r = one(&d, "=B2*C2", 1, 3);
        d.set_cells(vec![(2, 0, "Pencil".into())]).unwrap();
        assert!(matches!(d.formula_plan(&r), Err(Error::Changed)));
    }

    #[test]
    fn too_many_reads_are_refused_at_once() {
        let mut csv = String::from("a\r\n");
        for i in 0..30_000 {
            csv.push_str(&format!("{i}\r\n"));
        }
        let d =
            Document::from_bytes(csv.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        // Running totals are cheap at any length.
        let targets: Vec<(u64, u32)> = (1..30_001).map(|r| (r, 0)).collect();
        let spec = CellFormula {
            formula: "=SUM($A$2:A2)".into(),
            row: 1,
            col: 0,
            targets: targets.clone(),
            keep_first: true,
            ..Default::default()
        };
        let r = d.run_cell_formula(&spec, &Progress::new()).unwrap();
        assert_eq!(
            r.cells.last().unwrap().2,
            (29_999u64 * 30_000 / 2).to_string()
        );
        // MIN over growing ranges scans each: too many, refused before work.
        let spec = CellFormula {
            formula: "=MIN($A$2:A2)".into(),
            targets,
            ..spec
        };
        assert!(d.run_cell_formula(&spec, &Progress::new()).is_err());
    }
}
