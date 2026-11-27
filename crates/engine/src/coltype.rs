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

    // ---- the overlay through the engine's operations ----

    use crate::filter::{CmpOp, ColumnFilter, Condition, FilterSpec, Rule};
    use crate::sort::{SortKey, SortKind, SortSpec};
    use crate::{Document, OpenOptions};

    fn doc(csv: &str) -> Document {
        Document::from_bytes(csv.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn column(d: &Document, c: u32) -> Vec<String> {
        (0..d.row_count()).map(|r| d.cell(r, c).unwrap()).collect()
    }

    fn typed(d: &mut Document, col: u32, t: ColumnType) {
        let types = d.type_overlay().with(col, t);
        d.set_column_types(types);
    }

    fn sort_by(d: &mut Document, kind: SortKind) {
        d.sort(&SortSpec {
            keys: vec![SortKey {
                col: 0,
                descending: false,
                kind,
                match_case: false,
            }],
            keep_first: false,
            ..Default::default()
        })
        .unwrap();
    }

    #[test]
    fn an_id_column_sorts_as_ids_and_a_number_column_as_numbers() {
        let body = "10
2
007
0007
7
";
        let mut d = doc(body);
        typed(&mut d, 0, ColumnType::Id);
        sort_by(&mut d, SortKind::Auto);
        // By value, more leading zeros after: 2 < 7 < 007 < 0007 < 10.
        assert_eq!(column(&d, 0), ["2", "7", "007", "0007", "10"]);
        let mut d = doc("5
-3
12
");
        typed(&mut d, 0, ColumnType::Number);
        sort_by(&mut d, SortKind::Auto);
        assert_eq!(column(&d, 0), ["-3", "5", "12"]);
    }

    #[test]
    fn a_typed_column_is_not_read_as_a_number_by_a_filter_or_the_stats() {
        let body = "id
12
7
30
";
        let over_ten = FilterSpec {
            combine: Default::default(),
            invert: false,
            columns: vec![ColumnFilter {
                enabled: true,
                negate: false,
                name: None,
                col: 0,
                rule: Rule::Conditions {
                    first: Condition::Number {
                        op: CmpOp::Gt,
                        value: 10.0,
                    },
                    second: None,
                    or: false,
                },
            }],
            keep_first: true,
            read: Default::default(),
        };
        let mut d = doc(body);
        assert_eq!(d.set_filter(&over_ten).unwrap(), 3);
        let mut d = doc(body);
        typed(&mut d, 0, ColumnType::Id);
        // Nothing matches: an Id is never a number (only the title row stays shown).
        assert_eq!(d.set_filter(&over_ten).unwrap(), 1);
        // The summary counts no numbers either.
        let d2 = {
            let mut d = doc(body);
            typed(&mut d, 0, ColumnType::Text);
            d
        };
        let s = d2
            .summary(0, true, Default::default(), &crate::Progress::new())
            .unwrap();
        assert_eq!(s.mix.number, 0);
    }

    #[test]
    fn a_type_change_is_not_an_edit_and_moves_the_data_revision() {
        let mut d = doc("a,b
1,2
");
        let (rev, pos, before) = (d.data_revision(), d.history_pos(), d.row_values(1).unwrap());
        typed(&mut d, 1, ColumnType::Id);
        assert!(
            d.data_revision() > rev,
            "results computed from numbers are out of date"
        );
        assert_eq!(d.history_pos(), pos, "not an edit: nothing to undo or save");
        assert_eq!(d.row_values(1).unwrap(), before);
        // The same types again change nothing.
        let rev = d.data_revision();
        let same = d.type_overlay().clone();
        d.set_column_types(same);
        assert_eq!(d.data_revision(), rev);
    }

    #[test]
    fn a_key_typed_id_in_either_table_joins_exactly() {
        use crate::join::{JoinSource, JoinSpec};
        let a = |t: Option<ColumnType>| {
            let mut d = doc("id,name
007,x
7,y
");
            if let Some(t) = t {
                typed(&mut d, 0, t);
            }
            d
        };
        let spec = JoinSpec {
            keys: vec![0],
            other_keys: vec![0],
            columns: vec![1],
            keep_first: true,
            other_header: true,
            loose: true,
            ..Default::default()
        };
        let matched = |first: &Document, other: &Document| {
            let src = JoinSource::build(other, &spec, &crate::Progress::new()).unwrap();
            let (_, report) = first
                .plan_join(&src, &spec, &crate::Progress::new())
                .unwrap();
            report.matched
        };
        let other = {
            let mut d = doc("id,city
7,Oslo
");
            d.set_column_types(ColumnTypes::default());
            d
        };
        // Loose, untyped: 007 and 7 are the same key, so both rows match.
        assert_eq!(matched(&a(None), &other), 2);
        // The first table's key typed Id: exact, only "7" matches.
        assert_eq!(matched(&a(Some(ColumnType::Id)), &other), 1);
        // The other table's key typed Id: exact too.
        let mut typed_other = doc("id,city
7,Oslo
");
        typed(&mut typed_other, 0, ColumnType::Id);
        assert_eq!(matched(&a(None), &typed_other), 1);
    }

    /// The second half of the scan: `Reader::plain` reads a value with NO column type, so every use of it
    /// outside tests is on this exact list, each with the reason it is a single typed value (or a value
    /// that has no column). A new use fails here until it is reviewed and listed.
    #[test]
    fn untyped_readers_are_used_only_where_a_value_has_no_column() {
        // (file, function, number of uses, reason)
        let allowed: &[(&str, &str, usize, &str)] = &[
            ("clean.rs", "step_read", 2, "Clean steps (reformat dates, date bands) are rewrites the person chose for the cells they selected"),
            ("clean.rs", "reviewed_date", 1, "the date review rewrites cells in the order the person chose on its screen, which a column set to Text or Id cannot open"),
            ("clean.rs", "number_value", 1,"Clean steps (round, calculate, in words) are rewrites the person chose for the cells they selected"),
            ("editing.rs", "plan_group_calc", 1, "a group calculation that has no value column (count, running) reads nothing"),
            ("expr.rs", "numeric", 2, "converts a value an expression already holds; column cells are typed when loaded (Node::Col)"),
            ("expr.rs", "date", 1, "converts a value an expression already holds"),
            ("expr.rs", "compare", 1, "converts a value an expression already holds"),
            ("expr.rs", "call", 7, "functions of the expression language convert the values they are given"),
            ("fill.rs", "new", 1, "a fill series reads the starting cells the person selected, as typed"),
            ("filter.rs", "facet_counts_cached", 2, "a facet card on a column that does not exist has no type"),
            ("join.rs", "from_list", 1, "a pasted list of keys has no column"),
            ("ops.rs", "combine", 2, "Paste Special arithmetic: the person chose the operation for these cells"),
            ("pivot.rs", "label", 1, "reads the program's own group key text (a month, a day), not a cell"),
            ("privacy.rs", "kind_of_value", 1, "finds private data by how a value looks, whatever its column's type (a birth date typed Id is still private)"),
            ("validate.rs", "bound_date", 1, "a limit the person typed in a check"),
            ("validate.rs", "compile", 2, "a check on a Text or Id column, or an expression check, reads cells by the rule the person wrote"),
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found: std::collections::BTreeMap<(String, String), usize> = Default::default();
        for e in std::fs::read_dir(&dir).unwrap() {
            let path = e.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".rs") || name == "value.rs" || name == "coltype.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            let mut current = String::new();
            for line in code.lines() {
                let t = line.trim_start();
                if let Some(i) = t.find("fn ") {
                    let head = &t[..i];
                    if head.is_empty()
                        || head == "pub "
                        || head.starts_with("pub(")
                        || head == "async "
                    {
                        let n: String = t[i + 3..]
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect();
                        if !n.is_empty() {
                            current = n;
                        }
                    }
                }
                if t.starts_with("//") {
                    continue;
                }
                let n = line.matches("::plain(").count();
                if n > 0 {
                    *found.entry((name.clone(), current.clone())).or_default() += n;
                }
            }
        }
        let want: std::collections::BTreeMap<(String, String), usize> = allowed
            .iter()
            .map(|(f, func, n, _)| ((f.to_string(), func.to_string()), *n))
            .collect();
        assert_eq!(
            found, want,
            "an untyped read outside the reviewed list: type it, or list it with its reason"
        );
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
