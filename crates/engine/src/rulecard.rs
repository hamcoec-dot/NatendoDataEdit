//! "Show me the rule" (C10): fill by example's rule as a card the person can read and change. The
//! learned [`Program`] is the editable form; this module checks a program sent back, previews it,
//! finds the cells it would fill that look odd, names the example that breaks the rule when the
//! examples disagree, and writes the rule as a formula where every piece has one.

use std::collections::HashSet;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::coltype::ColumnType;
use crate::document::{Document, Progress};
use crate::example::{
    learn_until, Case, Cut, Example, ExampleSpec, Piece, Program, MAX_EXAMPLES, MAX_SOURCES,
    MAX_TIME,
};
use crate::Error;

/// Most pieces a rule may have.
pub const MAX_PIECES: usize = 32;
/// Longest fixed text in a rule, in characters.
pub const MAX_TEXT_CHARS: usize = 1_000;
/// Rows the preview looks through.
pub const PREVIEW_LOOK: u64 = 1_000;
/// Rows the preview shows at most.
pub const PREVIEW_ROWS: usize = 20;
/// Odd results listed at most.
pub const MAX_OUTLIERS: usize = 200;
/// Highest piece number with a formula form (the formula repeats a pattern this many times).
const MAX_FORMULA_INDEX: u32 = 16;

/// The examples of a column: the first filled cells of the shown rows (those left out are not).
pub(crate) struct ExampleRows {
    pub sources: Vec<u32>,
    pub owned: Vec<(Vec<String>, String)>,
    /// Their row ids and shown positions.
    pub ids: Vec<u32>,
    pub rows: Vec<u64>,
}

impl ExampleRows {
    pub(crate) fn examples(&self) -> Vec<Example<'_>> {
        self.owned
            .iter()
            .map(|(row, out)| Example {
                row: row.iter().map(String::as_str).collect(),
                out,
            })
            .collect()
    }
}

impl Program {
    /// Check a rule sent back from the card: columns inside the table (and not the one filled),
    /// at most [`MAX_PIECES`] pieces, fixed text at most [`MAX_TEXT_CHARS`], a split character
    /// that is one character that is not a letter, digit or space.
    pub fn validate(&self, ncols: u32, target: u32) -> Result<(), Error> {
        let bad = |w: String| Err(Error::Invalid(w));
        if self.pieces.is_empty() {
            return bad("the rule has no pieces".into());
        }
        if self.pieces.len() > MAX_PIECES {
            return bad(format!("a rule can have at most {MAX_PIECES} pieces"));
        }
        for p in &self.pieces {
            match p {
                Piece::Text { text } => {
                    if text.is_empty() {
                        return bad("a fixed-text piece is empty".into());
                    }
                    if text.chars().count() > MAX_TEXT_CHARS {
                        return bad(format!(
                            "fixed text can be at most {MAX_TEXT_CHARS} characters"
                        ));
                    }
                }
                Piece::From {
                    col, cut, index, ..
                } => {
                    if *col >= ncols {
                        return bad(format!(
                            "the rule reads column {}, which is not in the table any more: learn it again",
                            col + 1
                        ));
                    }
                    if *col == target {
                        return bad("the rule cannot read the column it fills".into());
                    }
                    if index.unsigned_abs() > 256 || (*cut == Cut::Whole && *index != 0) {
                        return bad("that piece number does not fit".into());
                    }
                    if let Cut::Split { by } = cut {
                        if by.is_alphanumeric() || by.is_whitespace() || by.is_control() {
                            return bad(
                                "split at a character that is not a letter, digit or space".into(),
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The result for a row, and whether a piece came out empty (`None`: a piece it needs is
    /// missing, so the cell is not filled).
    pub fn run_parts(&self, row: &dyn Fn(u32) -> Option<String>) -> Option<(String, bool)> {
        let mut out = String::new();
        let mut empty = false;
        for p in &self.pieces {
            let t = crate::example::piece_text(p, row)?;
            empty |= t.is_empty();
            out.push_str(&t);
            if out.len() > crate::find::MAX_RESULT_CELL_BYTES {
                return None;
            }
        }
        Some((out, empty))
    }

    /// The rule as a formula for a computed column (`names`: the columns' titles; `typed`: a
    /// column read as numbers or dates, whose text a formula would change). `Err`: why a piece
    /// has no formula form. A cell the rule leaves empty (a piece missing) is empty in the formula
    /// too.
    pub fn to_expression(
        &self,
        names: &dyn Fn(u32) -> String,
        typed: &dyn Fn(u32) -> bool,
    ) -> Result<String, String> {
        let mut parts = Vec::with_capacity(self.pieces.len());
        for p in &self.pieces {
            parts.push(match p {
                Piece::Text { text } => quote(text),
                Piece::From {
                    col,
                    cut,
                    index,
                    first,
                    case,
                } => {
                    let name = names(*col);
                    if typed(*col) {
                        return Err(format!(
                            "{name} is read as numbers or dates, so a formula would change its text"
                        ));
                    }
                    if index.unsigned_abs() > MAX_FORMULA_INDEX {
                        return Err(format!(
                            "piece {} of {name} is too far along for a formula",
                            index.unsigned_abs()
                        ));
                    }
                    let col_ref = format!("[{}]", name.replace(']', "]]"));
                    let mut x = format!(
                        "REGEXEXTRACT({col_ref}, {}, 1)",
                        quote(&piece_pattern(*cut, *index, *first))
                    );
                    if *first {
                        x = format!("LEFT({x}, 1)");
                    }
                    match case {
                        Case::Keep => x,
                        Case::Upper => format!("UPPER({x})"),
                        Case::Lower => format!("LOWER({x})"),
                        Case::Title => format!("PROPER({x})"),
                    }
                }
            });
        }
        let body = if parts.len() == 1 {
            parts.pop().expect("one")
        } else {
            parts.join(" & ")
        };
        Ok(format!("IFERROR({body}, \"\")"))
    }
}

fn quote(t: &str) -> String {
    format!("\"{}\"", t.replace('"', "\"\""))
}

/// A regular expression whose group 1 is exactly the piece `tokens` and `pick` give (in
/// `example.rs`), and that does not match when the piece is missing, or with `nonempty` when it
/// is empty (its first letter is then missing too).
fn piece_pattern(cut: Cut, index: i32, nonempty: bool) -> String {
    // The characters a piece is made of, as a class body.
    let class = match cut {
        Cut::Whole => return "^(.+)$".into(),
        Cut::Part => r"\S".to_string(),
        Cut::Word => r"\p{Alphabetic}".to_string(),
        Cut::Number => "0-9".to_string(),
        Cut::Run => r"\p{Alphabetic}\p{N}".to_string(),
        Cut::Split { by } => {
            let e = regex::escape(&by.to_string());
            let rep = if nonempty { "+" } else { "*" };
            // Pieces between the character; the value must hold it at least once.
            return if index >= 0 {
                let k = index as u32;
                if k == 0 {
                    format!("^([^{e}]{rep}){e}")
                } else {
                    format!("^(?:[^{e}]*{e}){{{k}}}([^{e}]{rep})")
                }
            } else {
                let k = index.unsigned_abs();
                if k == 1 {
                    format!("{e}([^{e}]{rep})$")
                } else {
                    format!("(?:^|{e})([^{e}]{rep})(?:{e}[^{e}]*){{{}}}$", k - 1)
                }
            };
        }
    };
    let (yes, no) = if cut == Cut::Part {
        (r"\S".to_string(), r"\s".to_string())
    } else {
        (format!("[{class}]"), format!("[^{class}]"))
    };
    if index >= 0 {
        let k = index as u32;
        if k == 0 {
            format!("^{no}*({yes}+)")
        } else {
            format!("^{no}*(?:{yes}+{no}+){{{k}}}({yes}+)")
        }
    } else {
        let k = index.unsigned_abs();
        if k == 1 {
            format!("({yes}+){no}*$")
        } else {
            format!("({yes}+)(?:{no}+{yes}+){{{}}}{no}*$", k - 1)
        }
    }
}

/// What learning found: the rule, or the example that breaks it.
pub(crate) struct Learned {
    pub program: Option<Program>,
    /// The example (its place among them) that, left out, lets every other one fit one rule.
    pub conflict: Option<usize>,
}

/// Learn from every example; when no rule fits them all, look for the one example that breaks
/// it, all within one shared time budget. An example is named only when the rule learned without
/// it fits every other example; with two bad examples, or when time runs out, none is named.
pub(crate) fn learn_or_conflict(
    examples: &[Example],
    sources: &[u32],
    budget: std::time::Duration,
) -> Result<Learned, Error> {
    let deadline = Instant::now() + budget;
    let none = Learned {
        program: None,
        conflict: None,
    };
    if let Some(p) = learn_until(examples, sources, deadline)? {
        return Ok(Learned {
            program: Some(p),
            conflict: None,
        });
    }
    let n = examples.len().min(MAX_EXAMPLES);
    if n < 2 || Instant::now() > deadline {
        return Ok(none);
    }
    let fits = |k: usize| learn_until(&examples[..k], sources, deadline).map(|p| p.is_some());
    if !fits(1)? {
        return Ok(none);
    }
    // Learning is monotone: if a prefix has no rule, no longer prefix has one.
    let (mut lo, mut hi) = (1usize, n);
    while hi - lo > 1 {
        if Instant::now() > deadline {
            return Ok(none);
        }
        let mid = (lo + hi) / 2;
        if fits(mid)? {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // The example at `hi - 1` breaks the rule of those before it; it, or one before it, is the odd one.
    for c in (0..hi).rev() {
        if Instant::now() > deadline {
            break;
        }
        let others: Vec<Example> = examples[..n]
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != c)
            .map(|(_, e)| Example {
                row: e.row.clone(),
                out: e.out,
            })
            .collect();
        if let Some(p) = learn_until(&others, sources, deadline)? {
            let all_fit = others.iter().all(|e| {
                p.run(&|col| e.row.get(col as usize).map(|s| s.to_string()))
                    .as_deref()
                    == Some(e.out)
            });
            if all_fit {
                return Ok(Learned {
                    program: Some(p),
                    conflict: Some(c),
                });
            }
        }
    }
    Ok(none)
}

/// A result's shape for the odd-results count: each run of letters one `A`, each run of digits
/// one `9`, every other character itself ("A.Lee" and "C.Hart" have one shape; lengths are
/// counted apart). The first 1,000 characters.
fn result_shape(v: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut last = '\0';
    for c in v.chars().take(MAX_TEXT_CHARS) {
        let k = if c.is_numeric() {
            '9'
        } else if c.is_alphabetic() {
            'A'
        } else {
            c
        };
        if (k == 'A' || k == '9') && k == last {
            continue;
        }
        last = k;
        h ^= u64::from(k as u32);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The example a card names when the examples disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    /// Its shown position, its row id and what was typed.
    pub row: u64,
    pub id: u32,
    pub value: String,
}

/// The rule card's start: the rule, in words, and the example that breaks it, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LearnReport {
    /// `None`: no rule fits (see `conflict`).
    pub program: Option<Program>,
    pub words: String,
    pub examples: u64,
    /// When the examples disagree and one of them is the odd one: that one (the rule is then
    /// the one learned without it).
    pub conflict: Option<Conflict>,
    /// The data revision it was worked out at (Apply carries it).
    pub revision: u64,
}

/// One row of the preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewRow {
    /// Shown position.
    pub row: u64,
    /// The cells the rule reads: (column, text).
    pub reads: Vec<(u32, String)>,
    /// `None`: a piece it needs is missing (the cell stays empty).
    pub result: Option<String>,
}

/// The first rows the rule would fill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub rows: Vec<PreviewRow>,
    /// Shown rows looked through (at most [`PREVIEW_LOOK`]).
    pub looked: u64,
}

/// Why a result looks odd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OddKind {
    /// A piece came out empty.
    EmptyPiece,
    /// Its shape (letters A, digits 9) is none of the examples' shapes.
    Shape,
    /// Under half the shortest example's length, or over twice the longest.
    Length,
}

/// One odd result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outlier {
    pub row: u64,
    pub kinds: Vec<OddKind>,
    pub value: String,
}

/// The cells a rule fills (or filled) and the odd ones among them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Outliers {
    /// Cells it fills, and empty cells it cannot fill (a piece missing).
    pub fills: u64,
    pub missed: u64,
    pub empty_piece: u64,
    pub odd_shape: u64,
    pub odd_length: u64,
    /// Rows with at least one: the first [`MAX_OUTLIERS`], in order.
    pub rows: Vec<Outlier>,
    /// Rows with at least one, in all.
    pub odd_rows: u64,
}

impl Document {
    /// The examples of column `spec.col`: its first filled cells among the shown rows (those
    /// in `spec.leave_out` are not), with their rows' values.
    pub(crate) fn example_rows(&self, spec: &ExampleSpec) -> Result<ExampleRows, Error> {
        let ncols = self.col_count();
        if spec.col >= ncols {
            return Err(Error::Invalid("choose a column inside the table".into()));
        }
        let sources: Vec<u32> = (0..ncols)
            .filter(|&c| c != spec.col)
            .take(MAX_SOURCES)
            .collect();
        let mut out = ExampleRows {
            sources,
            owned: Vec::new(),
            ids: Vec::new(),
            rows: Vec::new(),
        };
        for r in self.data_rows(spec.skip_first) {
            if out.owned.len() >= MAX_EXAMPLES {
                break;
            }
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            if spec.leave_out.contains(&id) {
                continue;
            }
            let value = self.cell_by_id(id, spec.col).unwrap_or_default();
            if value.is_empty() {
                continue;
            }
            let row: Vec<String> = (0..ncols.min(MAX_SOURCES as u32 + 1))
                .map(|c| {
                    if c == spec.col {
                        String::new()
                    } else {
                        self.cell_by_id(id, c).unwrap_or_default()
                    }
                })
                .collect();
            out.owned.push((row, value));
            out.ids.push(id);
            out.rows.push(r);
        }
        Ok(out)
    }

    /// The rule as a formula for a computed column of this table (Save as formula). `Err`: why
    /// it has none (a column read as numbers or dates, a piece too far along, two columns with
    /// the same title).
    pub fn example_formula(
        &self,
        spec: &ExampleSpec,
        program: &Program,
        read: crate::value::ReadOptions,
    ) -> Result<String, Error> {
        program.validate(self.col_count(), spec.col)?;
        let names = self.expr_names(spec.skip_first);
        let ctx = self.read_ctx(read);
        let f = program
            .to_expression(
                &|c| names.get(c as usize).cloned().unwrap_or_default(),
                &|c| {
                    !matches!(
                        ctx.type_of(c),
                        ColumnType::Auto | ColumnType::Text | ColumnType::Id
                    )
                },
            )
            .map_err(Error::Invalid)?;
        let e = self.parse_expr(&f, spec.skip_first)?;
        let mut got = e.columns().to_vec();
        let mut want = program.cols();
        got.sort_unstable();
        got.dedup();
        want.sort_unstable();
        want.dedup();
        if got != want {
            return Err(Error::Invalid(
                "two columns have the same title, so a formula cannot tell them apart".into(),
            ));
        }
        Ok(f)
    }

    /// A column's titles for a rule's words ("column 3" with no title).
    pub fn rule_names(&self, skip_first: bool) -> impl Fn(u32) -> String + '_ {
        let header = if skip_first { self.id_at(0) } else { None };
        move |c: u32| match header.and_then(|h| self.cell_by_id(h, c)) {
            Some(n) if !n.is_empty() => n,
            _ => format!("column {}", c + 1),
        }
    }

    /// Learn the rule for the card: from `spec.program` when given (checked), else from the
    /// column's examples; when they disagree, the one example that breaks the rule, if there is
    /// exactly one (all within one 2 s budget).
    pub fn example_learn(&self, spec: &ExampleSpec) -> Result<LearnReport, Error> {
        self.ready_for_bulk()?;
        let ex = self.example_rows(spec)?;
        let names = self.rule_names(spec.skip_first);
        let revision = self.data_revision();
        if let Some(p) = &spec.program {
            p.validate(self.col_count(), spec.col)?;
            return Ok(LearnReport {
                words: p.describe(&names),
                program: Some(p.clone()),
                examples: ex.owned.len() as u64,
                conflict: None,
                revision,
            });
        }
        if ex.owned.is_empty() {
            return Err(Error::Invalid(
                "type a few results in the column first, then fill by example".into(),
            ));
        }
        let learned = learn_or_conflict(&ex.examples(), &ex.sources, MAX_TIME)?;
        let conflict = learned.conflict.map(|i| Conflict {
            row: ex.rows[i],
            id: ex.ids[i],
            value: ex.owned[i].1.clone(),
        });
        Ok(LearnReport {
            words: learned
                .program
                .as_ref()
                .map_or(String::new(), |p| p.describe(&names)),
            program: learned.program,
            examples: ex.owned.len() as u64 - u64::from(conflict.is_some()),
            conflict,
            revision,
        })
    }

    /// The first rows `program` would fill (empty cells of the column with something to read),
    /// among the first [`PREVIEW_LOOK`] shown rows.
    pub fn example_preview(&self, spec: &ExampleSpec, program: &Program) -> Result<Preview, Error> {
        self.ready_for_bulk()?;
        program.validate(self.col_count(), spec.col)?;
        let reads = program.cols();
        let rows = self.data_rows(spec.skip_first);
        let end = rows.end.min(rows.start + PREVIEW_LOOK);
        let mut out = Preview {
            rows: Vec::new(),
            looked: end.saturating_sub(rows.start),
        };
        for r in rows.start..end {
            if out.rows.len() >= PREVIEW_ROWS {
                break;
            }
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            if self.cell_by_id(id, spec.col).is_some_and(|v| !v.is_empty()) {
                continue;
            }
            let vals: Vec<(u32, String)> = reads
                .iter()
                .map(|&c| (c, self.cell_by_id(id, c).unwrap_or_default()))
                .collect();
            if vals.iter().all(|(_, v)| v.is_empty()) {
                continue;
            }
            let get = |c: u32| self.cell_by_id(id, c);
            out.rows.push(PreviewRow {
                row: r,
                reads: vals,
                result: program.run(&get).filter(|v| !v.is_empty()),
            });
        }
        Ok(out)
    }

    /// The cells `program` fills and the odd results among them: a piece that came out empty, a
    /// shape none of the examples has, a length far from theirs. `after`: the example row ids
    /// of a fill that has run; look at the cells it filled (those holding exactly its result,
    /// the examples left out), judged against those examples, instead of the empty ones it
    /// would fill. One pass, cancellable through `progress`.
    pub fn example_outliers(
        &self,
        spec: &ExampleSpec,
        program: &Program,
        after: Option<&[u32]>,
        progress: &Progress,
    ) -> Result<Outliers, Error> {
        self.ready_for_bulk()?;
        program.validate(self.col_count(), spec.col)?;
        let col = spec.col;
        let (ids, values): (Vec<u32>, Vec<String>) = match after {
            Some(ids) => ids
                .iter()
                .map(|&id| (id, self.cell_by_id(id, col).unwrap_or_default()))
                .filter(|(_, v)| !v.is_empty())
                .unzip(),
            None => {
                let ex = self.example_rows(spec)?;
                (ex.ids, ex.owned.into_iter().map(|(_, v)| v).collect())
            }
        };
        let shapes: HashSet<u64> = values.iter().map(|v| result_shape(v)).collect();
        let lens: Vec<usize> = values.iter().map(|v| v.chars().count()).collect();
        let (short, long) = match (lens.iter().min(), lens.iter().max()) {
            (Some(&a), Some(&b)) => (a, b),
            _ => (0, usize::MAX / 4),
        };
        let examples: HashSet<u32> = ids.into_iter().collect();
        let after = after.is_some();
        let reads = program.cols();
        // (row, filled, odd kinds, value)
        let found = self.par_rows(self.data_rows(spec.skip_first), progress, |r, id, _| {
            let have = self.cell_by_id(id, col).unwrap_or_default();
            if after && (have.is_empty() || examples.contains(&id)) {
                return Ok(None);
            }
            if !after && !have.is_empty() {
                return Ok(None);
            }
            if reads
                .iter()
                .all(|&c| self.cell_by_id(id, c).is_none_or(|v| v.is_empty()))
            {
                return Ok(None);
            }
            let get = |c: u32| self.cell_by_id(id, c);
            let Some((v, empty)) = program.run_parts(&get).filter(|(v, _)| !v.is_empty()) else {
                return Ok((!after).then_some((r, false, Vec::new(), String::new())));
            };
            if after && v != have {
                return Ok(None);
            }
            let mut kinds = Vec::new();
            if empty {
                kinds.push(OddKind::EmptyPiece);
            }
            if !shapes.is_empty() && !shapes.contains(&result_shape(&v)) {
                kinds.push(OddKind::Shape);
            }
            let n = v.chars().count();
            if !lens.is_empty() && (n * 2 < short || n > long * 2) {
                kinds.push(OddKind::Length);
            }
            Ok(Some((r, true, kinds, v)))
        })?;
        let mut out = Outliers::default();
        for (row, filled, kinds, value) in found {
            if !filled {
                out.missed += 1;
                continue;
            }
            out.fills += 1;
            if kinds.is_empty() {
                continue;
            }
            for k in &kinds {
                match k {
                    OddKind::EmptyPiece => out.empty_piece += 1,
                    OddKind::Shape => out.odd_shape += 1,
                    OddKind::Length => out.odd_length += 1,
                }
            }
            out.odd_rows += 1;
            if out.rows.len() < MAX_OUTLIERS {
                out.rows.push(Outlier { row, kinds, value });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "rulecard_tests.rs"]
mod tests;
