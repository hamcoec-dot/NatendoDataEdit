//! Table checks: rules about the table as a whole, run with the data checks over the
//! shown rows: how many rows there are (or the same as another open table), a column
//! with no blank cells, how many different values a column holds, and a formula over
//! whole-column figures (`SUM([Votes]) = SUM([Early]) + SUM([Election day])`) compared
//! within a tolerance. Figures come from one totals pass over the columns they read.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::coltotals::ColumnTotal;
use crate::document::{Document, Progress};
use crate::expr::{Agg, Expr};
use crate::ops::check_cols;
use crate::validate::CheckSpec;
use crate::Error;

/// Most table checks at once.
pub const MAX_TABLE_CHECKS: usize = 50;

/// How a count is compared with the number given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compare {
    Equals,
    AtLeast,
    AtMost,
}

impl Compare {
    fn holds(self, actual: u64, want: u64) -> bool {
        match self {
            Compare::Equals => actual == want,
            Compare::AtLeast => actual >= want,
            Compare::AtMost => actual <= want,
        }
    }

    fn words(self, want: u64) -> String {
        match self {
            Compare::Equals => format!("exactly {want}"),
            Compare::AtLeast => format!("at least {want}"),
            Compare::AtMost => format!("at most {want}"),
        }
    }
}

/// How close two figures must be to count as equal: by an amount (`absolute`, 0.005 unless
/// set: half a cent), or by a share of the larger one (`relative`: 0.0001 is 0.01%).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tolerance {
    Absolute { value: f64 },
    Relative { value: f64 },
}

impl Default for Tolerance {
    fn default() -> Self {
        Tolerance::Absolute { value: 0.005 }
    }
}

impl Tolerance {
    /// `a` and `b` are equal within the tolerance.
    pub fn close(&self, a: f64, b: f64) -> bool {
        let d = (a - b).abs();
        match *self {
            Tolerance::Absolute { value } => d <= value,
            Tolerance::Relative { value } => d <= value * a.abs().max(b.abs()),
        }
    }

    fn words(&self) -> String {
        match *self {
            Tolerance::Absolute { value } => format!("within {value}"),
            Tolerance::Relative { value } => format!("within {}%", value * 100.0),
        }
    }

    fn valid(&self) -> bool {
        let v = match *self {
            Tolerance::Absolute { value } | Tolerance::Relative { value } => value,
        };
        v.is_finite() && v >= 0.0
    }
}

/// What a table check asks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TableCheckKind {
    /// The shown data rows, counted.
    RowCount { compare: Compare, value: u64 },
    /// As many shown data rows as the open table called `table` (filled in by the app).
    RowCountSameAs { table: String },
    /// No blank cell in the column.
    NoBlanks { col: u32 },
    /// How many different values the column holds (blank left out, spaces at the ends ignored).
    Distinct {
        col: u32,
        compare: Compare,
        value: u64,
    },
    /// A formula over whole-column figures that must hold ([`Expr::parse_table`]); a
    /// top-level `=` or `<>` compares within the tolerance.
    Aggregate {
        expr: String,
        #[serde(default)]
        tolerance: Tolerance,
    },
}

/// One table check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableCheck {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: TableCheckKind,
}

/// What a table check found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableResult {
    /// The check's number.
    pub check: u32,
    /// Its name, or words made from it.
    pub name: String,
    /// Whether it holds; `None`: it did not run (`why` says why).
    pub ok: Option<bool>,
    /// What the table has, and what the check wants, in words.
    pub actual: String,
    pub expected: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl TableCheck {
    /// The check in words (its name when it has one).
    pub fn describe(&self, titles: &[String]) -> String {
        if let Some(n) = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            return n.to_string();
        }
        let title = |c: u32| {
            titles
                .get(c as usize)
                .cloned()
                .unwrap_or_else(|| format!("Column {}", c + 1))
        };
        match &self.kind {
            TableCheckKind::RowCount { compare, value } => {
                format!("Rows: {}", compare.words(*value))
            }
            TableCheckKind::RowCountSameAs { table } => format!("As many rows as {table}"),
            TableCheckKind::NoBlanks { col } => format!("No blanks in {}", title(*col)),
            TableCheckKind::Distinct {
                col,
                compare,
                value,
            } => format!(
                "{} different values in {}",
                compare.words(*value),
                title(*col)
            ),
            TableCheckKind::Aggregate { expr, tolerance } => {
                format!("{expr} ({})", tolerance.words())
            }
        }
    }
}

/// Check table checks without a document (for files and specs from outside).
pub fn check_table_checks(checks: &[TableCheck]) -> Result<(), Error> {
    if checks.len() > MAX_TABLE_CHECKS {
        return Err(Error::Invalid(format!(
            "at most {MAX_TABLE_CHECKS} table checks"
        )));
    }
    for c in checks {
        if c.name
            .as_deref()
            .is_some_and(|n| n.chars().count() > crate::validate::MAX_NAME_CHARS)
        {
            return Err(Error::Invalid("a table check's name is too long".into()));
        }
        match &c.kind {
            TableCheckKind::Aggregate { expr, tolerance } => {
                if expr.chars().count() > crate::expr::MAX_EXPR_CHARS {
                    return Err(Error::Invalid("a table check's formula is too long".into()));
                }
                if !tolerance.valid() {
                    return Err(Error::Invalid(
                        "a tolerance is a number of 0 or more".into(),
                    ));
                }
            }
            TableCheckKind::RowCountSameAs { table }
                if table.trim().is_empty()
                    || table.chars().count() > crate::validate::MAX_REF_CHARS =>
            {
                return Err(Error::Invalid(
                    "\"as many rows as\" needs the other table's name".into(),
                ))
            }
            _ => {}
        }
    }
    Ok(())
}

fn figure(t: &ColumnTotal, agg: Agg, distinct: Option<u64>) -> Option<f64> {
    let x = &t.totals;
    match agg {
        Agg::Sum => x.sum.or((x.numbers == 0 && !x.too_large).then_some(0.0)),
        Agg::Count => Some(x.numbers as f64),
        Agg::CountA => Some(x.filled as f64),
        Agg::CountBlank => Some(t.blank as f64),
        Agg::Distinct => distinct.map(|d| d as f64),
        Agg::Min => x.min,
        Agg::Max => x.max,
        Agg::Average => x.average,
    }
}

fn show(v: Option<f64>) -> String {
    match v {
        Some(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{x:.0}"),
        Some(x) => format!("{x}"),
        None => "no value".into(),
    }
}

impl Document {
    /// Run `spec.table_checks` over the shown data rows. A formula that does not parse,
    /// or a column that is not there, refuses the run; "as many rows as" another table
    /// without that table (`spec.table_rows`) does not run and says why.
    pub fn run_table_checks(
        &self,
        spec: &CheckSpec,
        progress: &Progress,
    ) -> Result<Vec<TableResult>, Error> {
        check_table_checks(&spec.table_checks)?;
        if spec.table_checks.is_empty() {
            return Ok(Vec::new());
        }
        self.ready_for_bulk()?;
        let titles = self.expr_names(spec.keep_first);
        let rows = self.data_rows(spec.keep_first);
        let n = rows.end - rows.start;
        // What every check needs: parsed formulas, the columns to total, the columns to count values in.
        let mut exprs: Vec<Option<Expr>> = Vec::with_capacity(spec.table_checks.len());
        let mut total_cols: Vec<u32> = Vec::new();
        let mut distinct_cols: Vec<u32> = Vec::new();
        let add = |v: &mut Vec<u32>, c: u32| {
            if !v.contains(&c) {
                v.push(c);
            }
        };
        for c in &spec.table_checks {
            match &c.kind {
                TableCheckKind::NoBlanks { col } => {
                    check_cols(self, &[*col])?;
                    add(&mut total_cols, *col);
                    exprs.push(None);
                }
                TableCheckKind::Distinct { col, .. } => {
                    check_cols(self, &[*col])?;
                    add(&mut distinct_cols, *col);
                    exprs.push(None);
                }
                TableCheckKind::Aggregate { expr, .. } => {
                    let e = Expr::parse_table(expr, &titles)?;
                    for &(agg, col) in e.aggregates() {
                        check_cols(self, &[col])?;
                        if agg == Agg::Distinct {
                            add(&mut distinct_cols, col);
                        } else {
                            add(&mut total_cols, col);
                        }
                    }
                    exprs.push(Some(e));
                }
                _ => exprs.push(None),
            }
        }
        let totals: HashMap<u32, ColumnTotal> = if total_cols.is_empty() {
            HashMap::new()
        } else {
            let mut out = HashMap::new();
            for part in total_cols.chunks(crate::coltotals::MAX_COLUMN_TOTALS) {
                for t in self.column_totals(part, spec.keep_first, spec.read, progress)? {
                    out.insert(t.col, t);
                }
            }
            out
        };
        let mut distinct: HashMap<u32, (u64, bool)> = HashMap::new();
        for &c in &distinct_cols {
            distinct.insert(c, self.selection_distinct(rows.start, n, c, 1, progress)?);
        }
        let mut out = Vec::with_capacity(spec.table_checks.len());
        for (i, c) in spec.table_checks.iter().enumerate() {
            let mut r = TableResult {
                check: i as u32,
                name: c.describe(&titles),
                ok: None,
                actual: String::new(),
                expected: String::new(),
                why: None,
            };
            match &c.kind {
                TableCheckKind::RowCount { compare, value } => {
                    r.ok = Some(compare.holds(n, *value));
                    r.actual = format!("{n} rows");
                    r.expected = compare.words(*value);
                }
                TableCheckKind::RowCountSameAs { table } => match spec.table_rows.get(i) {
                    Some(Some(Ok(m))) => {
                        r.ok = Some(n == *m);
                        r.actual = format!("{n} rows");
                        r.expected = format!("{m} rows, as {table} has");
                    }
                    Some(Some(Err(why))) => r.why = Some(why.clone()),
                    _ => r.why = Some(format!("open {}", table.trim())),
                },
                TableCheckKind::NoBlanks { col } => {
                    let blank = totals.get(col).map_or(0, |t| t.blank);
                    r.ok = Some(blank == 0);
                    r.actual = format!("{blank} blank cells");
                    r.expected = "no blank cells".into();
                }
                TableCheckKind::Distinct {
                    col,
                    compare,
                    value,
                } => {
                    let (d, exact) = distinct[col];
                    if exact {
                        r.ok = Some(compare.holds(d, *value));
                        r.actual = format!("{d} different values");
                    } else {
                        // Past the counting limit: only "at least" checks can still be decided.
                        r.ok = match compare {
                            Compare::AtLeast => Some(d >= *value),
                            _ if d > *value => Some(false),
                            _ => None,
                        };
                        r.actual = format!("at least {d} different values");
                        if r.ok.is_none() {
                            r.why = Some("too many different values to count exactly".into());
                        }
                    }
                    r.expected = compare.words(*value);
                }
                TableCheckKind::Aggregate { tolerance, .. } => {
                    let e = exprs[i].as_ref().expect("parsed");
                    let figures: Vec<Option<f64>> = e
                        .aggregates()
                        .iter()
                        .map(|&(agg, col)| match agg {
                            Agg::Distinct => {
                                let (d, exact) = distinct[&col];
                                exact.then_some(d as f64)
                            }
                            _ => totals.get(&col).and_then(|t| figure(t, agg, None)),
                        })
                        .collect();
                    let v = e.eval_table(&figures, |a, b| tolerance.close(a, b));
                    r.ok = Some(v.ok);
                    if v.op.is_empty() {
                        r.actual = show(v.left);
                        r.expected = "true".into();
                    } else {
                        r.actual = show(v.left);
                        r.expected = format!("{} {} ({})", v.op, show(v.right), tolerance.words());
                    }
                }
            }
            out.push(r);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{Check, CheckKind};
    use crate::OpenOptions;

    fn doc(s: &str) -> Document {
        Document::from_bytes(s.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn spec(checks: Vec<TableCheckKind>) -> CheckSpec {
        CheckSpec {
            keep_first: true,
            table_checks: checks
                .into_iter()
                .map(|kind| TableCheck { name: None, kind })
                .collect(),
            ..Default::default()
        }
    }

    fn agg(expr: &str, tolerance: Tolerance) -> TableCheckKind {
        TableCheckKind::Aggregate {
            expr: expr.into(),
            tolerance,
        }
    }

    fn run(d: &Document, s: &CheckSpec) -> Vec<TableResult> {
        d.run_table_checks(s, &Progress::default()).unwrap()
    }

    fn oks(r: &[TableResult]) -> Vec<Option<bool>> {
        r.iter().map(|r| r.ok).collect()
    }

    const VOTES: &str = "Precinct,Early,Day,Total,Party\nA1,10,5,15,R\nA2,20,7,27, D\nA3,,3,3,D\nA4,1.25,1.25,2.5,R \n";

    #[test]
    fn counts_blanks_and_distinct() {
        let d = doc(VOTES);
        let r = run(
            &d,
            &spec(vec![
                TableCheckKind::RowCount {
                    compare: Compare::Equals,
                    value: 4,
                },
                TableCheckKind::RowCount {
                    compare: Compare::AtLeast,
                    value: 5,
                },
                TableCheckKind::RowCount {
                    compare: Compare::AtMost,
                    value: 4,
                },
                TableCheckKind::NoBlanks { col: 1 },
                TableCheckKind::NoBlanks { col: 2 },
                // " D" and "R " count as D and R.
                TableCheckKind::Distinct {
                    col: 4,
                    compare: Compare::Equals,
                    value: 2,
                },
                TableCheckKind::Distinct {
                    col: 0,
                    compare: Compare::AtMost,
                    value: 3,
                },
            ]),
        );
        assert_eq!(
            oks(&r),
            vec![
                Some(true),
                Some(false),
                Some(true),
                Some(false),
                Some(true),
                Some(true),
                Some(false)
            ]
        );
        assert_eq!(r[3].actual, "1 blank cells");
        assert_eq!(r[3].name, "No blanks in Early");
        assert_eq!(r[6].actual, "4 different values");
        // The title row counts when the file has no titles.
        let mut s = spec(vec![TableCheckKind::RowCount {
            compare: Compare::Equals,
            value: 5,
        }]);
        s.keep_first = false;
        assert_eq!(oks(&run(&d, &s)), vec![Some(true)]);
    }

    #[test]
    fn same_rows_as_another_table() {
        let d = doc(VOTES);
        let k = || TableCheckKind::RowCountSameAs {
            table: "roster.csv".into(),
        };
        let mut s = spec(vec![k(), k(), k(), k()]);
        s.table_rows = vec![
            Some(Ok(4)),
            Some(Ok(9)),
            Some(Err("two open files are called roster.csv".into())),
        ];
        let r = run(&d, &s);
        assert_eq!(oks(&r), vec![Some(true), Some(false), None, None]);
        assert_eq!(r[1].expected, "9 rows, as roster.csv has");
        assert_eq!(
            r[2].why.as_deref(),
            Some("two open files are called roster.csv")
        );
        assert_eq!(r[3].why.as_deref(), Some("open roster.csv"));
    }

    #[test]
    fn totals_add_up() {
        let d = doc(VOTES);
        let t = Tolerance::default();
        let r = run(
            &d,
            &spec(vec![
                agg("SUM([Early]) + SUM([Day]) = SUM([Total])", t),
                agg("SUM([Total]) = 47", t),
                agg(
                    "AND(COUNT([Early]) = 3, COUNTA([Precinct]) = 4, COUNTBLANK([Early]) = 1)",
                    t,
                ),
                agg("DISTINCT([Party]) = 2", t),
                agg("AND(MIN([Day]) >= 1.25, MAX([Total]) <= 27)", t),
                agg("AVERAGE([Day]) = 4.0625", t),
                agg("sum([total]) <> 47.5", t),
                agg("SUM([Total]) > 0", t),
            ]),
        );
        assert_eq!(
            oks(&r),
            vec![
                Some(true),
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(true)
            ]
        );
        assert_eq!(r[1].actual, "47.5");
        assert_eq!(r[1].expected, "= 47 (within 0.005)");
        // The sum of a column with no numbers is 0; its average has no value.
        let d = doc("Name,Note\nA,x\nB,\n");
        let r = run(
            &d,
            &spec(vec![
                agg("SUM([Note]) = 0", t),
                agg("AVERAGE([Note]) = 0", t),
            ]),
        );
        assert_eq!(oks(&r), vec![Some(true), Some(false)]);
        assert_eq!(r[1].actual, "no value");
    }

    #[test]
    fn tolerance_boundaries() {
        // 100 + 0.004 is within half a cent of 100; 100 + 0.006 is not.
        let d = doc("A,B\n100,100.004\n");
        let e = doc("A,B\n100,100.006\n");
        let abs = Tolerance::default();
        let k = |t| {
            spec(vec![
                agg("SUM([A]) = SUM([B])", t),
                agg("SUM([A]) >= SUM([B])", t),
                agg("SUM([A]) <> SUM([B])", t),
            ])
        };
        assert_eq!(
            oks(&run(&d, &k(abs))),
            vec![Some(true), Some(true), Some(false)]
        );
        assert_eq!(
            oks(&run(&e, &k(abs))),
            vec![Some(false), Some(false), Some(true)]
        );
        let zero = Tolerance::Absolute { value: 0.0 };
        assert_eq!(
            oks(&run(&d, &k(zero))),
            vec![Some(false), Some(false), Some(true)]
        );
        // Relative: 0.1% of 1000.9 is just over 1.
        let rel = Tolerance::Relative { value: 0.001 };
        let near = doc("A,B\n1000,1000.9\n");
        let far = doc("A,B\n1000,1001.1\n");
        assert_eq!(
            oks(&run(&near, &k(rel))),
            vec![Some(true), Some(true), Some(false)]
        );
        assert_eq!(
            oks(&run(&far, &k(rel))),
            vec![Some(false), Some(false), Some(true)]
        );
        assert!(Tolerance::Absolute { value: 0.5 }.close(1.0, 1.5));
        assert!(!Tolerance::Absolute { value: 0.5 }.close(1.0, 1.5001));
        assert!(Tolerance::Relative { value: 0.5 }.close(-2.0, -1.0));
        assert!(!Tolerance::Relative { value: 0.5 }.close(-2.0, -0.9));
        assert!(Tolerance::Relative { value: 0.0 }.close(0.0, 0.0));
        // Saved and read with its kind; absolute 0.005 when not given.
        let j = serde_json::to_string(&rel).unwrap();
        assert_eq!(j, r#"{"kind":"relative","value":0.001}"#);
        let back: TableCheckKind =
            serde_json::from_str(r#"{"type":"aggregate","expr":"SUM([A]) = 1"}"#).unwrap();
        assert_eq!(back, agg("SUM([A]) = 1", abs));
        for bad in [
            Tolerance::Absolute { value: -1.0 },
            Tolerance::Relative { value: f64::NAN },
            Tolerance::Absolute {
                value: f64::INFINITY,
            },
        ] {
            assert!(check_table_checks(&[TableCheck {
                name: None,
                kind: agg("1", bad)
            }])
            .is_err());
        }
    }

    #[test]
    fn titles_with_brackets_quotes_and_commas() {
        // Titles that look like formula text are read as titles, never as formula parts.
        let d =
            doc("\"a]b\",\"x), SUM([y\",\"say \"\"hi\"\"\",\"1,2\",y\n1,2,3,4,100\n1,2,3,4,100\n");
        let t = Tolerance::default();
        let r = run(
            &d,
            &spec(vec![
                agg("SUM([a]]b]) = 2", t),
                agg("SUM([x), SUM([y]) = 4", t),
                agg("SUM([say \"hi\"]) = 6", t),
                agg("SUM([1,2]) = 8", t),
                agg("SUM([y]) = 200", t),
            ]),
        );
        assert_eq!(oks(&r), vec![Some(true); 5]);
        // A bracket left open, or a column outside an aggregate, is refused, not guessed at.
        for bad in [
            "SUM([a]b]) = 2",
            "SUM([y]",
            "SUM([y) = 1",
            "[y] = 100",
            "SUM([y] + 1) = 1",
            "SUM(SUM([y])) = 1",
            "SUM([nope]) = 1",
            "SUM() = 0",
            "SUM([y], [y]) = 0",
            "LEN(\"a\") + [y] = 1",
        ] {
            assert!(
                d.run_table_checks(&spec(vec![agg(bad, t)]), &Progress::default())
                    .is_err(),
                "{bad}"
            );
        }
        // Columns by number work too.
        assert_eq!(
            oks(&run(&d, &spec(vec![agg("SUM([#5]) = 200", t)]))),
            vec![Some(true)]
        );
    }

    #[test]
    fn limits_and_reports() {
        let d = doc(VOTES);
        let many = vec![TableCheckKind::NoBlanks { col: 0 }; MAX_TABLE_CHECKS + 1];
        assert!(d
            .run_table_checks(&spec(many), &Progress::default())
            .is_err());
        assert!(d
            .run_table_checks(
                &spec(vec![TableCheckKind::NoBlanks { col: 9 }]),
                &Progress::default()
            )
            .is_err());
        let blank = spec(vec![TableCheckKind::RowCountSameAs { table: "  ".into() }]);
        assert!(crate::validate::check_spec(&blank).is_err());
        // Run with the row checks, reported together.
        let mut s = spec(vec![TableCheckKind::RowCount {
            compare: Compare::Equals,
            value: 4,
        }]);
        s.checks = vec![Check::new(Some(0), CheckKind::Required)];
        let rep = d.run_checks(&s, &Progress::default()).unwrap();
        assert_eq!(rep.table.len(), 1);
        assert_eq!(rep.table[0].ok, Some(true));
        let none = d
            .run_checks(
                &CheckSpec {
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::default(),
            )
            .unwrap();
        assert!(none.table.is_empty());
        assert!(!serde_json::to_string(&none).unwrap().contains("\"table\""));
    }
}
