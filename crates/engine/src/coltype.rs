//! The type overlay: what a person says a column holds (Text, Number, Date, Id), kept with the document
//! and used by every read of a cell as a number or a date. It never changes a cell: the file's text and
//! the bytes saved are the same whatever the types are.
//!
//! Types are keyed by the PHYSICAL column (the column's identity in the file and the edits), so moving a
//! column keeps its type, a column put in is Auto, and a deleted column brings its type back when the
//! delete is undone.
//!
//! [`ReadCtx`] is built from a document ([`crate::Document::read_ctx`]) and is the one way to read a cell as
//! a number ([`ReadCtx::number`]) or a date ([`ReadCtx::date`]); a scan test (`tests/`-style, in this file)
//! fails when anything else calls the parsers of [`crate::value`].

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::value::{self, DateOrder, ReadOptions};

/// What a column holds, as the person says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnType {
    /// Guess from the cells, as before.
    #[default]
    Auto,
    /// Words: never a number or a date.
    Text,
    /// Numbers (a cell that is not one is skipped, and marked on screen).
    Number,
    /// Dates; `None` reads them in the file-wide date order, `Some` overrides it for this column.
    Date(Option<DateOrder>),
    /// An identifier (an account number, a ZIP code, a phone number): text that is never a number, sorted
    /// the way people sort codes, compared exactly.
    Id,
}

impl ColumnType {
    /// Never read as a number or a date.
    pub fn is_text_like(self) -> bool {
        matches!(self, ColumnType::Text | ColumnType::Id)
    }
}

/// The types of the columns of a document, by physical column. Columns past the end are Auto.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColumnTypes {
    types: Vec<ColumnType>,
}

/// Most columns a type can be set on (the same limit as every column operation).
pub const MAX_TYPED_COLS: usize = 16_384;

impl ColumnTypes {
    /// From a list by physical column (trailing Auto entries are dropped).
    pub fn from_list(mut types: Vec<ColumnType>) -> ColumnTypes {
        types.truncate(MAX_TYPED_COLS);
        while types.last() == Some(&ColumnType::Auto) {
            types.pop();
        }
        ColumnTypes { types }
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// The type of physical column `col`.
    pub fn get(&self, col: u32) -> ColumnType {
        self.types.get(col as usize).copied().unwrap_or_default()
    }

    /// A copy with physical column `col` set (Auto removes it).
    pub fn with(&self, col: u32, t: ColumnType) -> ColumnTypes {
        let mut v = self.types.clone();
        let c = col as usize;
        if c < MAX_TYPED_COLS {
            if v.len() <= c {
                v.resize(c + 1, ColumnType::Auto);
            }
            v[c] = t;
        }
        ColumnTypes::from_list(v)
    }

    /// The list by physical column.
    pub fn list(&self) -> &[ColumnType] {
        &self.types
    }
}

/// How the cells of ONE column are read: the options and the column's type. Cheap to copy; hot loops ask
/// the context for one per column and call it for every cell.
#[derive(Debug, Clone, Copy)]
pub struct Reader {
    opts: ReadOptions,
    ty: ColumnType,
}

impl Reader {
    /// A reader of a column of type `ty`.
    pub fn with_type(opts: ReadOptions, ty: ColumnType) -> Reader {
        Reader { opts, ty }
    }

    /// A reader of an untyped column (Auto).
    pub fn plain(opts: ReadOptions) -> Reader {
        Reader {
            opts,
            ty: ColumnType::Auto,
        }
    }

    pub fn column_type(&self) -> ColumnType {
        self.ty
    }

    pub fn options(&self) -> ReadOptions {
        self.opts
    }

    /// The number a cell holds (Text, Id and Date columns hold none).
    #[inline]
    pub fn number(&self, s: &str) -> Option<f64> {
        match self.ty {
            ColumnType::Auto | ColumnType::Number => value::parse_number(s, self.opts),
            ColumnType::Text | ColumnType::Id | ColumnType::Date(_) => None,
        }
    }

    /// The date a cell holds, days since 1970-01-01 (Text, Id and Number columns hold none). `Date(Some)`
    /// overrides the file-wide order for this column.
    #[inline]
    pub fn date(&self, s: &str) -> Option<f64> {
        match self.ty {
            ColumnType::Auto | ColumnType::Date(None) => value::parse_date(s, self.opts),
            ColumnType::Date(Some(order)) => value::parse_date(
                s,
                ReadOptions {
                    date_order: order,
                    ..self.opts
                },
            ),
            ColumnType::Text | ColumnType::Id | ColumnType::Number => None,
        }
    }
}

/// How one document's cells are read as numbers and dates: the read options plus its column types. Built
/// from the document, so a caller cannot leave the types out.
#[derive(Clone, Copy)]
pub struct ReadCtx<'a> {
    pub(crate) opts: ReadOptions,
    pub(crate) types: &'a ColumnTypes,
    /// Logical to physical columns (`None`: the same).
    pub(crate) map: Option<&'a [u32]>,
}

impl<'a> ReadCtx<'a> {
    /// A context with no types (every column Auto), for text that does not come from a document column.
    pub fn plain(opts: ReadOptions) -> ReadCtx<'static> {
        static NONE: ColumnTypes = ColumnTypes { types: Vec::new() };
        ReadCtx {
            opts,
            types: &NONE,
            map: None,
        }
    }

    pub fn options(&self) -> ReadOptions {
        self.opts
    }

    /// The type of logical column `col`: one array index, not one per cell (callers in a loop ask once).
    #[inline]
    pub fn type_of(&self, col: u32) -> ColumnType {
        if self.types.is_empty() {
            return ColumnType::Auto;
        }
        let phys = match self.map {
            Some(m) => match m.get(col as usize) {
                Some(p) => *p,
                None => return ColumnType::Auto,
            },
            None => col,
        };
        self.types.get(phys)
    }

    /// The number a cell of column `col` holds.
    #[inline]
    pub fn number(&self, col: u32, s: &str) -> Option<f64> {
        self.number_as(self.type_of(col), s)
    }

    /// The number a cell holds when its column has type `t`.
    #[inline]
    pub fn number_as(&self, t: ColumnType, s: &str) -> Option<f64> {
        Reader {
            opts: self.opts,
            ty: t,
        }
        .number(s)
    }

    /// The date a cell of column `col` holds (days since 1970-01-01). This is the one date entry point:
    /// the date-ambiguity work (C2) extends it, and a column signed off there is stored as `Date(order)`.
    #[inline]
    pub fn date(&self, col: u32, s: &str) -> Option<f64> {
        self.date_as(self.type_of(col), s)
    }

    /// The date a cell holds when its column has type `t`.
    #[inline]
    pub fn date_as(&self, t: ColumnType, s: &str) -> Option<f64> {
        Reader {
            opts: self.opts,
            ty: t,
        }
        .date(s)
    }

    /// The reader of logical column `col`: one lookup, then any number of cells.
    #[inline]
    pub fn reader(&self, col: u32) -> Reader {
        Reader {
            opts: self.opts,
            ty: self.type_of(col),
        }
    }

    /// The reader of physical column `phys` (for code that already works on physical columns).
    #[inline]
    pub fn reader_phys(&self, phys: u32) -> Reader {
        Reader {
            opts: self.opts,
            ty: self.types.get(phys),
        }
    }

    /// Whether cells of column `col` may be compared as the same value when only spacing or case differ
    /// and no more ("loose"): an Id or Text column is compared exactly apart from trimming and case.
    #[inline]
    pub fn compares_as_text(&self, col: u32) -> bool {
        self.type_of(col).is_text_like()
    }
}

/// The order of identifiers: runs of digits compare by value, and with equal value the run with more leading
/// zeros comes after (`123` < `0123` < `00123`); the rest compares as text. A total order (ties are equal
/// texts only), so sorting is stable and deterministic.
pub fn id_order(a: &str, b: &str) -> Ordering {
    id_order_bytes(a.as_bytes(), b.as_bytes())
}

/// [`id_order`] on the bytes of two texts.
pub fn id_order_bytes(x: &[u8], y: &[u8]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    // First difference found while the numbers had the same value: more leading zeros sorts after.
    let mut zeros: Ordering = Ordering::Equal;
    while i < x.len() && j < y.len() {
        if x[i].is_ascii_digit() && y[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < x.len() && x[i].is_ascii_digit() {
                i += 1;
            }
            while j < y.len() && y[j].is_ascii_digit() {
                j += 1;
            }
            let (ra, rb) = (&x[si..i], &y[sj..j]);
            let (za, zb) = (
                ra.iter().take_while(|c| **c == b'0').count(),
                rb.iter().take_while(|c| **c == b'0').count(),
            );
            let (va, vb) = (&ra[za.min(ra.len())..], &rb[zb.min(rb.len())..]);
            // By value: fewer digits is smaller, then digit by digit.
            let by_value = va.len().cmp(&vb.len()).then_with(|| va.cmp(vb));
            if by_value != Ordering::Equal {
                return by_value;
            }
            if zeros == Ordering::Equal {
                zeros = za.cmp(&zb);
            }
        } else {
            let c = x[i].cmp(&y[j]);
            if c != Ordering::Equal {
                return c;
            }
            i += 1;
            j += 1;
        }
    }
    // One ran out: the shorter is first; with the same text, the zeros decide, then it is a tie.
    (x.len() - i)
        .cmp(&(y.len() - j))
        .then(zeros)
        .then_with(|| x.cmp(y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_by_physical_column_default_to_auto() {
        let t = ColumnTypes::default().with(2, ColumnType::Id);
        assert_eq!(t.get(2), ColumnType::Id);
        assert_eq!(t.get(0), ColumnType::Auto);
        assert_eq!(t.get(99), ColumnType::Auto);
        assert!(t.with(2, ColumnType::Auto).is_empty());
    }

    #[test]
    fn reads_follow_the_type() {
        let t = ColumnTypes::from_list(vec![
            ColumnType::Auto,
            ColumnType::Id,
            ColumnType::Number,
            ColumnType::Date(Some(DateOrder::Dmy)),
            ColumnType::Date(None),
        ]);
        let ctx = ReadCtx {
            opts: ReadOptions::default(),
            types: &t,
            map: None,
        };
        assert_eq!(ctx.number(0, "12"), Some(12.0));
        assert_eq!(ctx.number(1, "12"), None);
        assert_eq!(ctx.number(2, "1,234"), Some(1234.0));
        assert_eq!(ctx.number(3, "12"), None);
        // 03/04/2026: day first for the overriding column, month first (the file setting) for None.
        let dmy = ctx.date(3, "03/04/2026").unwrap();
        let mdy = ctx.date(4, "03/04/2026").unwrap();
        assert!(mdy < dmy);
        assert_eq!(ctx.date(1, "2026-01-05"), None);
        // A moved column keeps its type: logical 0 is physical 1.
        let moved = ReadCtx {
            opts: ReadOptions::default(),
            types: &t,
            map: Some(&[1, 0]),
        };
        assert_eq!(moved.type_of(0), ColumnType::Id);
        assert_eq!(moved.type_of(1), ColumnType::Auto);
    }

    #[test]
    fn id_order_is_a_total_order_and_orders_zeros_after() {
        let order = |a: &str, b: &str| id_order(a, b);
        assert_eq!(order("2", "10"), Ordering::Less);
        assert_eq!(order("123", "0123"), Ordering::Less);
        assert_eq!(order("0123", "00123"), Ordering::Less);
        assert_eq!(order("a2", "a10"), Ordering::Less);
        assert_eq!(order("x", "x"), Ordering::Equal);
        // Property test on a small alphabet: reflexive, antisymmetric, transitive, and equal only for
        // equal texts.
        let alphabet = ['0', '1', '9', 'a', '-'];
        let mut all = vec![String::new()];
        for len in 1..=4 {
            let mut next = Vec::new();
            for s in all.iter().filter(|s| s.len() == len - 1) {
                for c in alphabet {
                    next.push(format!("{s}{c}"));
                }
            }
            all.extend(next);
        }
        for a in &all {
            assert_eq!(id_order(a, a), Ordering::Equal);
            for b in &all {
                let ab = id_order(a, b);
                assert_eq!(ab, id_order(b, a).reverse(), "{a:?} {b:?}");
                assert_eq!(ab == Ordering::Equal, a == b, "{a:?} {b:?}");
            }
        }
        let sample: Vec<&String> = all.iter().step_by(7).collect();
        for a in &sample {
            for b in &sample {
                for c in &sample {
                    if id_order(a, b) != Ordering::Greater && id_order(b, c) != Ordering::Greater {
                        assert_ne!(id_order(a, c), Ordering::Greater, "{a:?} {b:?} {c:?}");
                    }
                }
            }
        }
    }

    /// The scan (R3): nothing outside `value.rs` and this file calls `parse_number` or `parse_date`, except
    /// the exact list of single typed values below (each with its reason). A new direct call fails here
    /// until someone puts it through [`ReadCtx`] or adds it to the list in review.
    #[test]
    fn no_direct_number_or_date_parsers_outside_the_allow_list() {
        // (file, function it is in, how many calls, why this is a single typed value, not a column read)
        let allowed: &[(&str, &str, usize, &str)] = &[];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found: Vec<(String, usize)> = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap() {
            let path = e.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".rs") || name == "value.rs" || name == "coltype.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Test modules sit at the end of a file.
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            let n = code
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .map(|l| l.matches("parse_number(").count() + l.matches("parse_date(").count())
                .sum::<usize>();
            if n > 0 {
                found.push((name, n));
            }
        }
        found.sort();
        let want: Vec<(String, usize)> = {
            let mut per: std::collections::BTreeMap<String, usize> = Default::default();
            for (f, _, n, _) in allowed {
                *per.entry(f.to_string()).or_default() += n;
            }
            per.into_iter().collect()
        };
        assert_eq!(
            found, want,
            "direct calls to parse_number/parse_date: route them through ReadCtx"
        );
    }
}
