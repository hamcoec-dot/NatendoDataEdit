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

use crate::document::Document;
use crate::expr::{to_text_owned, Ctx, Expr, Lookups, Value};
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

/// A text literal for an expression.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// What a formula does to one cell, worked out.
enum Rewritten {
    /// The expression to run, and the formula as this cell keeps it.
    Ok(String, String),
    /// A reference moved off the table.
    Off,
}

impl Document {
    /// A cell's text by row id and logical column.
    fn cell_text_by_id(&self, id: u32, col: u32) -> String {
        let reader = Cols::new(self, &[col]);
        let mut buf = Vec::new();
        reader.read(self, id, &mut buf);
        reader.get(&buf, 0).to_string()
    }

    /// The formula's text for the cell `(pos, col)` (document position):
    /// references moved by its distance from where it was typed, then read.
    fn rewrite_formula(
        &self,
        src: &str,
        from: (i64, i64),
        to: (i64, i64),
        reads: &mut u64,
    ) -> Result<Rewritten, Error> {
        let chars: Vec<char> = src.chars().collect();
        let (dr, dc) = (to.0 - from.0, to.1 - from.1);
        let rows = self.total_rows() as i64;
        let cols = i64::from(self.col_count());
        let mut run = String::with_capacity(src.len());
        let mut keep = String::with_capacity(src.len());
        // Function names open, to skip text cells inside SUM-like ranges.
        let mut open: Vec<String> = Vec::new();
        let mut word = String::new();
        let mut i = 0;
        let start_reads = *reads;
        let moved = |r: Ref| Ref {
            col: if r.col_fixed { r.col } else { r.col + dc },
            row: if r.row_fixed { r.row } else { r.row + dr },
            ..r
        };
        let show = |r: Ref| {
            format!(
                "{}{}{}{}",
                if r.col_fixed { "$" } else { "" },
                col_letters(r.col.max(0) as u32),
                if r.row_fixed { "$" } else { "" },
                r.row + 1
            )
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
                let t: String = chars[s..i].iter().collect();
                run.push_str(&t);
                keep.push_str(&t);
                word.clear();
                continue;
            }
            let boundary = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
            if boundary && (c == '$' || c.is_ascii_alphabetic()) {
                if let Some((a, n)) = parse_ref(&chars, i) {
                    let mut end = i + n;
                    let mut b = None;
                    if chars.get(end) == Some(&':') {
                        if let Some((r2, n2)) = parse_ref(&chars, end + 1) {
                            b = Some(r2);
                            end += 1 + n2;
                        }
                    }
                    let (a, b) = (moved(a), b.map(moved));
                    let fits = |r: Ref| r.col >= 0 && r.col < cols && r.row >= 0 && r.row < rows;
                    if !fits(a) || b.is_some_and(|b| !fits(b)) {
                        return Ok(Rewritten::Off);
                    }
                    match b {
                        None => {
                            *reads += 1;
                            let t = self
                                .doc_id_at(a.row as u64)
                                .map(|id| self.cell_text_by_id(id, a.col as u32))
                                .unwrap_or_default();
                            run.push_str(&if t.is_empty() {
                                "BLANK()".into()
                            } else {
                                quote(&t)
                            });
                            keep.push_str(&show(a));
                        }
                        Some(b) => {
                            let (r1, r2) = (a.row.min(b.row), a.row.max(b.row));
                            let (c1, c2) = (a.col.min(b.col), a.col.max(b.col));
                            let n = ((r2 - r1 + 1) * (c2 - c1 + 1)) as u64;
                            *reads += n;
                            if *reads - start_reads > MAX_RANGE_CELLS {
                                return Err(Error::Invalid(format!(
                                    "a formula reads at most {MAX_RANGE_CELLS} cells; use a smaller range, or Data › Computed column"
                                )));
                            }
                            if *reads > MAX_FORMULA_READS {
                                return Err(Error::Invalid(
                                    "this formula reads too many cells; use a smaller range or select fewer cells".into(),
                                ));
                            }
                            let numbers_only = open.last().is_some_and(|f| {
                                matches!(f.as_str(), "SUM" | "AVERAGE" | "MIN" | "MAX")
                            });
                            let ids: Vec<u32> =
                                (r1..=r2).filter_map(|r| self.doc_id_at(r as u64)).collect();
                            let cs: Vec<u32> = (c1..=c2).map(|c| c as u32).collect();
                            let reader = Cols::new(self, &cs);
                            let mut buf = Vec::new();
                            let mut parts: Vec<String> = Vec::new();
                            for id in ids {
                                reader.read(self, id, &mut buf);
                                for k in 0..cs.len() {
                                    let t = reader.get(&buf, k);
                                    if t.is_empty() {
                                        continue;
                                    }
                                    if numbers_only
                                        && crate::value::parse_number(t, ReadOptions::default())
                                            .is_none()
                                    {
                                        continue;
                                    }
                                    parts.push(quote(t));
                                }
                            }
                            if parts.is_empty() {
                                run.push_str("BLANK()");
                            } else {
                                run.push_str(&parts.join(","));
                            }
                            keep.push_str(&show(a));
                            keep.push(':');
                            keep.push_str(&show(b));
                        }
                    }
                    i = end;
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
            run.push(c);
            keep.push(c);
            i += 1;
        }
        Ok(Rewritten::Ok(run, keep))
    }

    /// The formula as an expression for the cell it was typed in (its
    /// references read), e.g. to find the tabs its LOOKUPs read.
    pub fn cell_formula_expr(&self, spec: &CellFormula) -> Result<String, Error> {
        let src = spec.formula.trim_start();
        let src = src.strip_prefix('=').unwrap_or(src);
        let p = self.row_number(spec.row).ok_or(Error::OutOfRange)? as i64;
        let at = (p, i64::from(spec.col));
        let mut reads = 0;
        match self.rewrite_formula(src, at, at, &mut reads)? {
            Rewritten::Ok(run, _) => Ok(run),
            Rewritten::Off => Err(Error::Invalid(
                "a cell reference in the formula is outside the table".into(),
            )),
        }
    }

    /// A worked-out formula (its ranges written out) as an expression.
    fn parse_run(run: &str, names: &[String]) -> Result<Expr, Error> {
        Expr::parse_within(
            run,
            names,
            usize::MAX,
            MAX_RANGE_CELLS as usize + crate::expr::MAX_NODES,
        )
    }

    /// Run a cell formula over its cells. Nothing changes here: the caller
    /// writes `cells` (one undo step) and remembers `formulas`.
    pub fn run_cell_formula(&self, spec: &CellFormula) -> Result<FormulaResult, Error> {
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
        let pos = |r: u64| -> Result<i64, Error> {
            self.row_number(r)
                .map(|p| p as i64)
                .ok_or(Error::OutOfRange)
        };
        let from = (pos(spec.row)?, i64::from(spec.col));
        // Check it reads as an expression once, so a typo is one message
        // (references read as blank here).
        let mut reads = 0u64;
        let probe = match self.rewrite_formula(src, from, from, &mut reads)? {
            Rewritten::Ok(run, _) => run,
            Rewritten::Off => {
                return Err(Error::Invalid(
                    "a cell reference in the formula is outside the table".into(),
                ))
            }
        };
        let names = self.expr_names(spec.keep_first);
        let mut e = Self::parse_run(&probe, &names)?;
        check_cols(self, e.columns())?;
        let total = self.total_rows();
        let first = u64::from(spec.keep_first && total > 0);
        let mut out = FormulaResult::default();
        let mut buf = Vec::new();
        let (mut pv, mut nv) = (Vec::new(), Vec::new());
        reads = 0;
        for &(r, c) in &targets {
            let p = pos(r)?;
            let (run, keep) =
                match self.rewrite_formula(src, from, (p, i64::from(c)), &mut reads)? {
                    Rewritten::Ok(run, keep) => (run, keep),
                    Rewritten::Off => {
                        out.errors += 1;
                        out.first_error.get_or_insert_with(|| "#REF!".into());
                        continue;
                    }
                };
            e = Self::parse_run(&run, &names)?;
            e.bind(&spec.tables);
            let reader = Cols::new(self, e.columns());
            let id = self.doc_id_at(p as u64).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let near = e.uses_near();
            let (mut has_p, mut has_n) = (false, false);
            if near {
                let p = p as u64;
                if p > first {
                    if let Some(pid) = self.doc_id_at(p - 1) {
                        reader.read(self, pid, &mut pv);
                        has_p = true;
                    }
                }
                if p + 1 < total {
                    if let Some(nid) = self.doc_id_at(p + 1) {
                        reader.read(self, nid, &mut nv);
                        has_n = true;
                    }
                }
            }
            let ctx = Ctx {
                cells: &buf,
                slots: reader.slots(),
                row: p as u64 + 1,
                now: spec.now,
                read: spec.read,
                prev: has_p.then_some(pv.as_slice()),
                next: has_n.then_some(nv.as_slice()),
            };
            let v = e.eval(&ctx);
            if let Value::Err(_) = v {
                out.errors += 1;
                out.first_error.get_or_insert_with(|| to_text_owned(&v));
                continue;
            }
            out.cells.push((r, c, to_text_owned(&v)));
            out.formulas.push(format!("={keep}"));
        }
        Ok(out)
    }
}

impl Document {
    /// The results as a change of cells (one undo step, the path a paste
    /// takes).
    pub fn formula_plan(&self, res: &FormulaResult) -> Result<crate::ops::CellPlan, Error> {
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
        d.run_cell_formula(&CellFormula {
            formula: f.into(),
            row,
            col,
            keep_first: true,
            ..Default::default()
        })
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
        let r = d.run_cell_formula(&spec).unwrap();
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
        let r = d.run_cell_formula(&spec).unwrap();
        assert_eq!(r.cells, vec![(1, 3, "Price".to_string())]);
        assert_eq!(r.first_error.as_deref(), Some("#REF!"));
    }

    #[test]
    fn bad_formulas_are_refused() {
        let d = doc();
        let bad = |f: &str| {
            d.run_cell_formula(&CellFormula {
                formula: f.into(),
                keep_first: true,
                row: 1,
                ..Default::default()
            })
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
}
