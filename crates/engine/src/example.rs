//! Fill by example: learn how a column is made from the other columns of
//! the same row, from a few typed results, and fill the rest.
//!
//! A [`Program`] is a list of pieces: fixed text, or a piece of another
//! column of the row (the whole value, or the k-th word, number, letter run
//! or space-separated part counting from the start or the end, or its first
//! character), each with a case change. [`learn`] searches all examples
//! together, position by position in their results, cheapest program first
//! (fewest fixed characters, then fewest pieces), with bounded work.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::CellPlan;
use crate::Error;

/// Most examples used.
pub const MAX_EXAMPLES: usize = 50;
/// Most columns a program reads from.
pub const MAX_SOURCES: usize = 256;
/// Values (and results) longer than this, in characters, are not used.
pub const MAX_VALUE_CHARS: usize = 1_000;
/// Most tokens of one kind looked at in a value.
const MAX_TOKENS: usize = 64;
/// Search limits.
const MAX_STATES: usize = 200_000;
const MAX_EDGES: usize = 5_000_000;
const MAX_TIME: Duration = Duration::from_secs(2);

/// How a value is cut into pieces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cut {
    /// The whole value.
    Whole,
    /// Runs of characters separated by spaces.
    Part,
    /// Runs of letters.
    Word,
    /// Runs of digits.
    Number,
    /// Runs of letters and digits.
    Run,
    /// The value split at a character (`@`, `-`, `/`, ...).
    Split { by: char },
}

const CUTS: [Cut; 5] = [Cut::Whole, Cut::Part, Cut::Word, Cut::Number, Cut::Run];
/// Most split characters tried per column.
const MAX_SPLITS: usize = 8;

/// The ways to cut a column: the fixed ones, then splitting at each
/// punctuation character the first example's value holds.
fn cuts_for(v: &str) -> Vec<Cut> {
    let mut cuts = CUTS.to_vec();
    for c in v.chars() {
        if cuts.len() >= CUTS.len() + MAX_SPLITS {
            break;
        }
        if !c.is_alphanumeric() && !c.is_whitespace() && !cuts.contains(&Cut::Split { by: c }) {
            cuts.push(Cut::Split { by: c });
        }
    }
    cuts
}

/// A case change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Case {
    Keep,
    Upper,
    Lower,
    Title,
}

const CASES: [Case; 4] = [Case::Keep, Case::Upper, Case::Lower, Case::Title];

/// One piece of a result.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Piece {
    Text {
        text: String,
    },
    From {
        /// Logical column.
        col: u32,
        cut: Cut,
        /// Which piece: 0 the first, -1 the last.
        index: i32,
        /// Its first character only.
        first: bool,
        case: Case,
    },
}

/// A learned way to make a column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Program {
    pub pieces: Vec<Piece>,
}

fn tokens(v: &str, cut: Cut) -> Vec<&str> {
    let mut out = Vec::new();
    match cut {
        Cut::Whole => {
            if !v.is_empty() {
                out.push(v);
            }
        }
        Cut::Part => out.extend(v.split_whitespace().take(MAX_TOKENS * 4)),
        // Without the character there are no pieces (not the whole value).
        Cut::Split { by } => {
            if v.contains(by) {
                out.extend(v.split(by).take(MAX_TOKENS * 4));
            }
        }
        _ => {
            let keep = |c: char| match cut {
                Cut::Word => c.is_alphabetic(),
                Cut::Number => c.is_ascii_digit(),
                _ => c.is_alphanumeric(),
            };
            let mut start = None;
            for (i, c) in v.char_indices() {
                match (keep(c), start) {
                    (true, None) => start = Some(i),
                    (false, Some(s)) => {
                        out.push(&v[s..i]);
                        start = None;
                        if out.len() >= MAX_TOKENS * 4 {
                            return out;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(s) = start {
                out.push(&v[s..]);
            }
        }
    }
    out
}

fn change_case(s: &str, case: Case) -> String {
    match case {
        Case::Keep => s.to_string(),
        Case::Upper => s.to_uppercase(),
        Case::Lower => s.to_lowercase(),
        Case::Title => {
            let mut out = String::with_capacity(s.len());
            let mut start = true;
            for c in s.chars() {
                if c.is_alphanumeric() {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(c);
                    start = c != '\'';
                }
            }
            out
        }
    }
}

fn pick<'a>(toks: &[&'a str], index: i32) -> Option<&'a str> {
    if index >= 0 {
        toks.get(index as usize).copied()
    } else {
        toks.len()
            .checked_sub(index.unsigned_abs() as usize)
            .map(|i| toks[i])
    }
}

/// One piece's text for a row (`row[c]` is logical column `c`'s value);
/// `None` if the piece is not there.
fn piece_text(p: &Piece, row: &dyn Fn(u32) -> Option<String>) -> Option<String> {
    match p {
        Piece::Text { text } => Some(text.clone()),
        Piece::From {
            col,
            cut,
            index,
            first,
            case,
        } => {
            let v = row(*col)?;
            let toks = tokens(&v, *cut);
            let t = pick(&toks, *index)?;
            let t = if *first {
                let c = t.chars().next()?;
                &t[..c.len_utf8()]
            } else {
                t
            };
            Some(change_case(t, *case))
        }
    }
}

impl Program {
    /// The result for a row, or `None` if a piece it needs is missing.
    pub fn run(&self, row: &dyn Fn(u32) -> Option<String>) -> Option<String> {
        let mut out = String::new();
        for p in &self.pieces {
            out.push_str(&piece_text(p, row)?);
            if out.len() > crate::find::MAX_RESULT_CELL_BYTES {
                return None;
            }
        }
        Some(out)
    }

    /// The columns it reads.
    fn cols(&self) -> Vec<u32> {
        let mut c: Vec<u32> = self
            .pieces
            .iter()
            .filter_map(|p| match p {
                Piece::From { col, .. } => Some(*col),
                _ => None,
            })
            .collect();
        c.sort_unstable();
        c.dedup();
        c
    }

    /// In words, with column names.
    pub fn describe(&self, names: &dyn Fn(u32) -> String) -> String {
        let parts: Vec<String> = self
            .pieces
            .iter()
            .map(|p| match p {
                Piece::Text { text } => format!("\"{text}\""),
                Piece::From {
                    col,
                    cut,
                    index,
                    first,
                    case,
                } => {
                    let which = match cut {
                        Cut::Whole => String::new(),
                        _ => {
                            let kind = match cut {
                                Cut::Part => "part",
                                Cut::Word => "word",
                                Cut::Number => "number",
                                Cut::Split { .. } => "piece",
                                _ => "letters-and-digits run",
                            };
                            let kind = match cut {
                                Cut::Split { by } => format!("{kind} (split at \"{by}\")"),
                                _ => kind.to_string(),
                            };
                            match index {
                                0 => format!("first {kind} of "),
                                -1 => format!("last {kind} of "),
                                i if *i > 0 => format!("{kind} {} of ", i + 1),
                                i => format!("{kind} {} from the end of ", -i),
                            }
                        }
                    };
                    let first = if *first { "first letter of " } else { "" };
                    let case = match case {
                        Case::Keep => "",
                        Case::Upper => ", in capitals",
                        Case::Lower => ", in small letters",
                        Case::Title => ", capitalized",
                    };
                    format!("{first}{which}{}{case}", names(*col))
                }
            })
            .collect();
        parts.join(" + ")
    }
}

/// One example: its row's values (by logical column) and the result.
pub struct Example<'a> {
    pub row: Vec<&'a str>,
    pub out: &'a str,
}

struct Prepared<'a> {
    /// Tokens by column, then by cut (in `cuts` order).
    toks: Vec<Vec<Vec<&'a str>>>,
    out: &'a str,
}

/// A value used for learning (overlong values are not).
fn value<'a>(e: &Example<'a>, c: u32) -> &'a str {
    let v = e.row.get(c as usize).copied().unwrap_or("");
    if v.chars().count() > MAX_VALUE_CHARS {
        ""
    } else {
        v
    }
}

/// Find the cheapest program that gives every example's result from its
/// row. `sources`: the columns it may read. `Ok(None)`: none fits.
pub fn learn(examples: &[Example], sources: &[u32]) -> Result<Option<Program>, Error> {
    let started = Instant::now();
    let ex: Vec<&Example> = examples
        .iter()
        .filter(|e| !e.out.is_empty() && e.out.chars().count() <= MAX_VALUE_CHARS)
        .take(MAX_EXAMPLES)
        .collect();
    if ex.is_empty() {
        return Ok(None);
    }
    let sources: Vec<u32> = sources.iter().copied().take(MAX_SOURCES).collect();
    let cuts: Vec<Vec<Cut>> = sources.iter().map(|&c| cuts_for(value(ex[0], c))).collect();
    let prep: Vec<Prepared> = ex
        .iter()
        .map(|e| Prepared {
            toks: sources
                .iter()
                .zip(&cuts)
                .map(|(&c, cuts)| cuts.iter().map(|&cut| tokens(value(e, c), cut)).collect())
                .collect(),
            out: e.out,
        })
        .collect();
    let n = prep.len();
    // Dijkstra over positions (one per example).
    type State = Vec<u32>;
    let goal: State = prep.iter().map(|p| p.out.len() as u32).collect();
    let start: State = vec![0; n];
    let mut best: HashMap<State, (u64, Option<(State, Piece)>)> = HashMap::new();
    let mut heap: BinaryHeap<Reverse<(u64, u64, State)>> = BinaryHeap::new();
    let mut tick = 0u64;
    best.insert(start.clone(), (0, None));
    heap.push(Reverse((0, 0, start)));
    let mut edges = 0usize;
    while let Some(Reverse((cost, _, state))) = heap.pop() {
        if best.get(&state).is_some_and(|b| b.0 < cost) {
            continue;
        }
        if state == goal {
            // Walk back, merging fixed characters into one text piece.
            let mut pieces: Vec<Piece> = Vec::new();
            let mut s = state;
            while let Some((_, Some((prev, p)))) = best.get(&s).cloned() {
                match (pieces.last_mut(), &p) {
                    (Some(Piece::Text { text }), Piece::Text { text: t }) => {
                        text.insert_str(0, t);
                    }
                    _ => pieces.push(p),
                }
                s = prev;
            }
            pieces.reverse();
            return Ok(Some(Program { pieces }));
        }
        if best.len() > MAX_STATES || edges > MAX_EDGES || started.elapsed() > MAX_TIME {
            return Ok(None);
        }
        let mut relax = |next: State, c: u64, piece: Piece, heap: &mut BinaryHeap<_>| {
            let total = cost + c;
            if best.get(&next).is_none_or(|b| total < b.0) {
                best.insert(next.clone(), (total, Some((state.clone(), piece))));
                tick += 1;
                heap.push(Reverse((total, tick, next)));
            }
        };
        let rest0 = &prep[0].out[state[0] as usize..];
        // A fixed character shared by every example.
        if let Some(c) = rest0.chars().next() {
            let ok = (1..n).all(|i| prep[i].out[state[i] as usize..].starts_with(c));
            if ok {
                let next: State = state.iter().map(|&p| p + c.len_utf8() as u32).collect();
                relax(
                    next,
                    1000,
                    Piece::Text {
                        text: c.to_string(),
                    },
                    &mut heap,
                );
            }
        }
        // Pieces of other columns, found in the first example and checked
        // against the others.
        let Some(next_char) = rest0.chars().next() else {
            continue;
        };
        let lead: String = next_char.to_lowercase().collect();
        for (si, &col) in sources.iter().enumerate() {
            for (ci, cut) in cuts[si].iter().enumerate() {
                let toks = &prep[0].toks[si][ci];
                let count = toks.len();
                let mut idxs: Vec<i32> = (0..count.min(MAX_TOKENS) as i32).collect();
                if *cut != Cut::Whole {
                    idxs.extend((1..=count.min(MAX_TOKENS) as i32).map(|k| -k));
                }
                for index in idxs {
                    let Some(tok) = pick(toks, index) else {
                        continue;
                    };
                    if !tok.to_lowercase().starts_with(&lead) {
                        continue;
                    }
                    if edges % 4096 == 0 && started.elapsed() > MAX_TIME {
                        return Ok(None);
                    }
                    for first in [false, true] {
                        for case in CASES {
                            edges += 1;
                            let piece = Piece::From {
                                col,
                                cut: *cut,
                                index,
                                first,
                                case,
                            };
                            let mut next = Vec::with_capacity(n);
                            let fits = (0..n).all(|i| {
                                let toks = &prep[i].toks[si][ci];
                                let Some(t) = pick(toks, index) else {
                                    return false;
                                };
                                let t = if first {
                                    match t.chars().next() {
                                        Some(c) => &t[..c.len_utf8()],
                                        None => return false,
                                    }
                                } else {
                                    t
                                };
                                let t = change_case(t, case);
                                let rest = &prep[i].out[state[i] as usize..];
                                if t.is_empty() || !rest.starts_with(&t) {
                                    return false;
                                }
                                next.push(state[i] + t.len() as u32);
                                true
                            });
                            if !fits {
                                continue;
                            }
                            // Cheaper than any fixed character; simple
                            // pieces (whole values, the first or last,
                            // no case change) first.
                            let c = 900
                                + 4 * index.unsigned_abs().min(20) as u64
                                + if index < 0 { 2 } else { 0 }
                                + ci.min(10) as u64
                                + if first { 1 } else { 0 }
                                + if case == Case::Keep { 0 } else { 3 };
                            relax(next, c, piece, &mut heap);
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

// ---- on a document ----

/// Fill one column by example.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExampleSpec {
    /// The column to fill (logical).
    pub col: u32,
    /// Leave the first row (column titles) out.
    #[serde(default)]
    pub skip_first: bool,
}

/// What fill by example did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExampleReport {
    /// Examples it learned from.
    pub examples: u64,
    /// Cells filled.
    pub filled: u64,
    /// Empty cells left empty (a piece the pattern needs is missing).
    pub missed: u64,
    /// The pattern, in words.
    pub pattern: String,
}

impl Document {
    /// Learn from the filled cells of column `spec.col` (shown rows) and
    /// fill its empty ones.
    pub fn plan_fill_example(
        &self,
        spec: &ExampleSpec,
        progress: &Progress,
    ) -> Result<(CellPlan, ExampleReport), Error> {
        self.ready_for_bulk()?;
        let ncols = self.col_count();
        if spec.col >= ncols {
            return Err(Error::Invalid("choose a column inside the table".into()));
        }
        let sources: Vec<u32> = (0..ncols)
            .filter(|&c| c != spec.col)
            .take(MAX_SOURCES)
            .collect();
        let rows = self.data_rows(spec.skip_first);
        // Examples: the first filled cells of the column.
        let mut owned: Vec<(Vec<String>, String)> = Vec::new();
        for r in rows.clone() {
            if owned.len() >= MAX_EXAMPLES {
                break;
            }
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            let out = self.cell_by_id(id, spec.col).unwrap_or_default();
            if out.is_empty() {
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
            owned.push((row, out));
        }
        if owned.is_empty() {
            return Err(Error::Invalid(
                "type a few results in the column first, then fill by example".into(),
            ));
        }
        let examples: Vec<Example> = owned
            .iter()
            .map(|(row, out)| Example {
                row: row.iter().map(String::as_str).collect(),
                out,
            })
            .collect();
        let Some(program) = learn(&examples, &sources)? else {
            return Err(Error::Invalid(
                "no single pattern fits every example; check them, or type one more".into(),
            ));
        };
        let reads = program.cols();
        let col = spec.col;
        // Each planned cell is counted as it is made, so a huge fill is
        // refused before it is all in memory.
        let spent = std::sync::atomic::AtomicUsize::new(0);
        let found = self.par_rows(rows, progress, |_, id, _| {
            if self.cell_by_id(id, col).is_some_and(|v| !v.is_empty()) {
                return Ok(None);
            }
            if reads
                .iter()
                .all(|&c| self.cell_by_id(id, c).is_none_or(|v| v.is_empty()))
            {
                return Ok(None);
            }
            let get = |c: u32| self.cell_by_id(id, c);
            let v = program.run(&get);
            if !crate::find::charge(&spent, v.as_ref().map_or(0, String::len)) {
                return Err(Error::OutOfMemory(
                    "this would fill too much at once; filter to fewer rows first",
                ));
            }
            Ok(Some((id, v)))
        })?;
        let mut plan = CellPlan::new(self);
        let (mut filled, mut missed) = (0u64, 0u64);
        for (id, v) in found {
            match v {
                Some(v) if !v.is_empty() => {
                    plan.changes.push((id, vec![(col, v)]));
                    filled += 1;
                }
                _ => missed += 1,
            }
        }
        plan.cells = filled;
        let header = if spec.skip_first { self.id_at(0) } else { None };
        let names = |c: u32| match header.and_then(|h| self.cell_by_id(h, c)) {
            Some(n) if !n.is_empty() => n,
            _ => format!("column {}", c + 1),
        };
        let report = ExampleReport {
            examples: owned.len() as u64,
            filled,
            missed,
            pattern: program.describe(&names),
        };
        Ok((plan, report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn learn_rows(rows: &[(&[&str], &str)]) -> Option<Program> {
        let ex: Vec<Example> = rows
            .iter()
            .map(|(r, o)| Example {
                row: r.to_vec(),
                out: o,
            })
            .collect();
        let cols = rows.first().map_or(0, |r| r.0.len()) as u32;
        learn(&ex, &(0..cols).collect::<Vec<_>>()).unwrap()
    }

    fn apply(p: &Program, row: &[&str]) -> Option<String> {
        p.run(&|c| row.get(c as usize).map(|s| s.to_string()))
    }

    #[test]
    fn learns_common_patterns() {
        // Initials and last name.
        let p = learn_rows(&[
            (&["John Smith"], "J. Smith"),
            (&["Mary Ann Jones"], "M. Jones"),
        ])
        .unwrap();
        assert_eq!(apply(&p, &["Ada Lovelace"]).unwrap(), "A. Lovelace");
        // Two columns joined, case changed.
        let p = learn_rows(&[(&["smith", "John"], "John SMITH")]).unwrap();
        assert_eq!(apply(&p, &["jones", "Mary"]).unwrap(), "Mary JONES");
        // Email user names.
        let p = learn_rows(&[(&["ann.lee@example.org"], "ann.lee"), (&["bo@x.com"], "bo")]);
        let p = p.unwrap();
        assert_eq!(apply(&p, &["kim.park@y.net"]).unwrap(), "kim.park");
        // A number out of text.
        let p = learn_rows(&[(&["Order #1234 shipped"], "1234")]).unwrap();
        assert_eq!(apply(&p, &["Order #77 late"]).unwrap(), "77");
        // Dates rearranged.
        let p = learn_rows(&[
            (&["2026-03-04"], "04/03/2026"),
            (&["2025-12-31"], "31/12/2025"),
        ])
        .unwrap();
        assert_eq!(apply(&p, &["2024-01-09"]).unwrap(), "09/01/2024");
        // A missing piece gives nothing.
        assert_eq!(apply(&p, &["soon"]), None);
        // Examples that disagree have no pattern.
        assert!(learn_rows(&[(&["a b"], "a"), (&["c d"], "x")]).is_none());
    }

    #[test]
    fn fills_a_document() {
        let mut d = Document::from_bytes(
            b"first,last,full\r\nJohn,Smith,\"Smith, J\"\r\nAda,Lovelace,\r\nGrace,Hopper,\r\n,,\r\n"
                .to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let (plan, rep) = d
            .plan_fill_example(
                &ExampleSpec {
                    col: 2,
                    skip_first: true,
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!((rep.examples, rep.filled, rep.missed), (1, 2, 0));
        assert_eq!(rep.pattern, "last + \", \" + first letter of first");
        d.apply_cells(plan).unwrap();
        assert_eq!(d.row_values(2).unwrap()[2], "Lovelace, A");
        assert_eq!(
            d.row_values(4).unwrap(),
            ["", "", ""],
            "empty rows stay empty"
        );
        assert!(d.undo());
        assert_eq!(d.row_values(2).unwrap()[2], "");
        // Results the other columns cannot make: a clear message.
        let e = d
            .plan_fill_example(
                &ExampleSpec {
                    col: 1,
                    skip_first: true,
                },
                &Progress::new(),
            )
            .unwrap_err();
        assert!(e.to_string().contains("no single pattern"), "{e}");
        // No examples: a clear message.
        let d = Document::from_bytes(
            b"a,b\r\nx,\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let e = d
            .plan_fill_example(
                &ExampleSpec {
                    col: 1,
                    skip_first: true,
                },
                &Progress::new(),
            )
            .unwrap_err();
        assert!(e.to_string().contains("type a few results"), "{e}");
    }

    /// Hostile inputs: long values, many columns, many tokens and examples
    /// that never agree all stay within the search limits.
    #[test]
    fn hostile_examples_stay_bounded() {
        let long = "a ".repeat(50_000);
        let words: String = (0..500).map(|i| format!("w{i} ")).collect();
        let row: Vec<&str> = std::iter::repeat_n(words.as_str(), 300).collect();
        let outs: Vec<String> = (0..60)
            .map(|i| format!("{}{i}", "w1 ".repeat(30)))
            .collect();
        let ex: Vec<Example> = outs
            .iter()
            .map(|o| Example {
                row: row.clone(),
                out: o,
            })
            .chain(std::iter::once(Example {
                row: vec![long.as_str()],
                out: &long,
            }))
            .collect();
        let t = Instant::now();
        let _ = learn(&ex, &(0..300).collect::<Vec<_>>()).unwrap();
        assert!(t.elapsed() < Duration::from_secs(6), "{:?}", t.elapsed());
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        /// Whatever the examples, learning never panics, and a learned
        /// program gives every example's result from its row.
        #[test]
        fn learned_programs_reproduce_their_examples(
            rows in prop::collection::vec(
                (prop::collection::vec("[A-Za-z0-9 ,.@-]{0,14}|\\PC{0,6}", 1..4), "[A-Za-z0-9 ,.@-]{0,10}"),
                1..4,
            )
        ) {
            let ex: Vec<Example> = rows
                .iter()
                .map(|(r, o)| Example { row: r.iter().map(String::as_str).collect(), out: o })
                .collect();
            let cols = rows.iter().map(|r| r.0.len()).max().unwrap_or(0) as u32;
            if let Some(p) = learn(&ex, &(0..cols).collect::<Vec<_>>()).unwrap() {
                for (r, o) in &rows {
                    if o.is_empty() {
                        continue;
                    }
                    let got = p.run(&|c| r.get(c as usize).cloned());
                    prop_assert_eq!(got.as_deref(), Some(o.as_str()));
                }
            }
        }
    }
}

/// Independent review of M6, round 1.
#[cfg(test)]
mod review_tests {
    use super::*;

    /// A row without the separator a pattern splits at has no piece: it is
    /// left empty, not given its whole value.
    #[test]
    fn missing_separator_is_a_missing_piece() {
        let ex = [
            Example {
                row: vec!["ann.lee@x.org"],
                out: "ann.lee",
            },
            Example {
                row: vec!["bo.ray@y.net"],
                out: "bo.ray",
            },
        ];
        let p = learn(&ex, &[0]).unwrap().unwrap();
        assert_eq!(
            p.run(&|_| Some("cy.li@z.com".into())).as_deref(),
            Some("cy.li")
        );
        assert_eq!(p.run(&|_| Some("no.atsign".into())), None);
    }

    /// The time limit is checked inside the search, not only between states.
    #[test]
    fn time_limit_holds() {
        let words: String = (0..64).map(|i| format!("w{i} ")).collect();
        let row: Vec<&str> = std::iter::repeat_n(words.as_str(), 256).collect();
        let outs: Vec<String> = (0..50)
            .map(|i| format!("{}{i}", "w1 w2 ".repeat(40)))
            .collect();
        let ex: Vec<Example> = outs
            .iter()
            .map(|o| Example {
                row: row.clone(),
                out: o,
            })
            .collect();
        let t = Instant::now();
        let _ = learn(&ex, &(0..256).collect::<Vec<_>>()).unwrap();
        assert!(
            t.elapsed() < Duration::from_millis(2_600),
            "{:?}",
            t.elapsed()
        );
    }

    /// Item 1: a fill of many rows is counted against the run budget.
    #[test]
    fn big_fills_are_bounded() {
        let rows = crate::find::RUN_BUDGET / 64 + 1_000;
        let mut text = String::from("a,b\r\nx,x\r\n");
        for _ in 0..rows {
            text.push_str("y,\r\n");
        }
        let d = Document::from_bytes(
            text.into_bytes(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let e = d
            .plan_fill_example(
                &ExampleSpec {
                    col: 1,
                    skip_first: true,
                },
                &Progress::new(),
            )
            .unwrap_err();
        assert!(e.to_string().contains("too much at once"), "{e}");
    }
}
