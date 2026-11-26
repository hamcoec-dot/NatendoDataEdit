//! Totals of each column over the shown rows, for the totals row: one pass
//! over the rows however many columns are asked for.

use rayon::prelude::*;
use serde::Serialize;

use crate::coltype::ColumnType;
use crate::ops::Cols;
use crate::rows::{totals_of, Acc, Totals};
use crate::value::ReadOptions;
use crate::{Document, Error, Progress};

/// Rows per parallel task (and between cancel checks).
const CHUNK: u64 = 8_192;
/// Most columns totalled in one request.
pub const MAX_COLUMN_TOTALS: usize = 256;

/// What the totals row shows for a column until the person chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Suggested {
    Sum,
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ColumnTotal {
    pub col: u32,
    #[serde(flatten)]
    pub totals: Totals,
    /// Blank cells among the rows.
    pub blank: u64,
    pub suggested: Suggested,
}

/// What the filled values look like: whether they are identifiers written in
/// digits (a ZIP code, a registration number) rather than quantities.
#[derive(Clone, Copy)]
struct Shape {
    /// Every filled value so far is digits only.
    digits: bool,
    leading_zero: bool,
    shortest: usize,
    longest: usize,
}

impl Shape {
    fn new() -> Shape {
        Shape {
            digits: true,
            leading_zero: false,
            shortest: usize::MAX,
            longest: 0,
        }
    }

    fn see(&mut self, v: &str) {
        if !self.digits {
            return;
        }
        if !v.bytes().all(|b| b.is_ascii_digit()) {
            self.digits = false;
            return;
        }
        self.leading_zero |= v.len() > 1 && v.starts_with('0');
        self.shortest = self.shortest.min(v.len());
        self.longest = self.longest.max(v.len());
    }

    fn merge(&mut self, o: &Shape) {
        self.digits &= o.digits;
        self.leading_zero |= o.leading_zero;
        self.shortest = self.shortest.min(o.shortest);
        self.longest = self.longest.max(o.longest);
    }

    /// Whole numbers with leading zeros, or all of one length of 5 or more.
    /// Under a title that names a `quantity` only the zeros count: five-digit
    /// vote counts are still added.
    fn is_identifier(&self, quantity: bool) -> bool {
        self.digits
            && self.longest > 0
            && (self.leading_zero
                || (!quantity && self.shortest == self.longest && self.shortest >= 5))
    }
}

/// Title words that name a code or an identifier beyond the export's list.
const ID_WORDS: &[&str] = &[
    "zip",
    "postal",
    "phone",
    "precinct",
    "district",
    "ward",
    "code",
    "fips",
    "geoid",
    "registration",
    "voterid",
];

/// A title that names an identifier: never added up unless asked.
fn id_like(title: &str) -> bool {
    crate::xlsxbook::id_title(title)
        || crate::xlsxbook::title_words(title)
            .iter()
            .any(|w| ID_WORDS.contains(&w.as_str()))
            && !crate::xlsxbook::names_quantity(title)
}

impl Document {
    /// The totals of each of `cols` over the shown rows (without the title
    /// row when `header`). Cancelled through `progress`.
    pub fn column_totals(
        &self,
        cols: &[u32],
        header: bool,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Vec<ColumnTotal>, Error> {
        self.column_totals_in(cols, self.data_rows(header), header, read, progress)
    }

    /// [`Document::column_totals`] over shown rows `rows` only.
    pub(crate) fn column_totals_in(
        &self,
        cols: &[u32],
        rows: std::ops::Range<u64>,
        header: bool,
        read: ReadOptions,
        progress: &Progress,
    ) -> Result<Vec<ColumnTotal>, Error> {
        if cols.len() > MAX_COLUMN_TOTALS || cols.iter().any(|&c| c >= self.col_count()) {
            return Err(Error::OutOfRange);
        }
        let (first, end) = (rows.start, rows.end);
        let reader = Cols::new(self, cols);
        let ctx = self.read_ctx(read);
        let rds: Vec<crate::coltype::Reader> = cols.iter().map(|&c| ctx.reader(c)).collect();
        let fresh = || vec![(Acc::new(), Shape::new()); cols.len()];
        progress.start(end - first);
        let sums = (0..(end - first).div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<Vec<(Acc, Shape)>, Error> {
                let s = first + k * CHUNK;
                let e = (s + CHUNK).min(end);
                progress.step(0)?;
                let mut all = fresh();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    for (i, ((a, shape), rd)) in all.iter_mut().zip(&rds).enumerate() {
                        let v = reader.get(&buf, i).trim();
                        a.cells += 1;
                        if v.is_empty() {
                            continue;
                        }
                        a.filled += 1;
                        shape.see(v);
                        if let Some(x) = rd.number(v) {
                            a.number(x);
                        }
                    }
                }
                progress.step(e - s)?;
                Ok(all)
            })
            // Merged in row order, so a sum is the same on every run.
            .collect::<Result<Vec<_>, _>>()?;
        let mut all = fresh();
        for part in sums {
            for ((a, shape), (b, other)) in all.iter_mut().zip(part) {
                *a = a.merge(b);
                shape.merge(&other);
            }
        }
        let titles = self.titles(cols, header);
        Ok(cols
            .iter()
            .zip(all)
            .zip(titles)
            .map(|((&col, (a, shape)), title)| {
                let typed = matches!(ctx.type_of(col), ColumnType::Text | ColumnType::Id);
                // At least 95 filled cells in 100 are numbers.
                let numeric = a.filled > 0 && a.numbers.saturating_mul(20) >= a.filled * 19;
                let sum = numeric
                    && !typed
                    && !shape.is_identifier(header && crate::xlsxbook::names_quantity(&title))
                    && !(header && id_like(&title));
                ColumnTotal {
                    col,
                    totals: totals_of(a, a.cells),
                    blank: a.cells - a.filled,
                    suggested: if sum {
                        Suggested::Sum
                    } else {
                        Suggested::Count
                    },
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(text: &str) -> Document {
        Document::from_bytes(
            text.as_bytes().to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap()
    }

    fn totals(d: &Document, cols: &[u32]) -> Vec<ColumnTotal> {
        d.column_totals(cols, true, ReadOptions::default(), &Progress::new())
            .unwrap()
    }

    #[test]
    fn identifiers_are_counted_and_quantities_added() {
        let d = doc(
            "Voter ID,ZIP,Votes,Reg,Lead,Name,Postal votes,Precinct,Amount\r\n\
                     100234,37402,12,5550001,007,ann,3,4,10.5\r\n\
                     100235,37403,7,5550002,012,bo,4,12,\r\n\
                     100236,,20000,5550003,3,cy,5,103,2\r\n",
        );
        let t = totals(&d, &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let s: Vec<Suggested> = t.iter().map(|c| c.suggested).collect();
        use Suggested::{Count, Sum};
        assert_eq!(s, [Count, Count, Sum, Count, Count, Count, Sum, Count, Sum]);
        assert_eq!(t[2].totals.sum, Some(20019.0));
        assert_eq!((t[1].totals.filled, t[1].blank), (2, 1));
        assert_eq!((t[8].totals.sum, t[8].blank), (Some(12.5), 1));
        assert_eq!(t[5].totals.sum, None);
    }

    #[test]
    fn a_quantity_title_beats_the_shape_but_not_leading_zeros() {
        let d = doc("Votes,Total,Precinct total,Batch,Ballot count\r\n\
                     10000,52000,00101,10000,01000\r\n\
                     25000,61000,00102,25000,02000\r\n");
        let t = totals(&d, &[0, 1, 2, 3, 4]);
        use Suggested::{Count, Sum};
        let s: Vec<Suggested> = t.iter().map(|c| c.suggested).collect();
        assert_eq!(s, [Sum, Sum, Count, Count, Count]);
        assert_eq!(t[0].totals.sum, Some(35000.0));
        // A type the person set still wins.
        let mut typed = doc("Votes\r\n10000\r\n");
        let types = typed.type_overlay().with(0, ColumnType::Id);
        typed.set_column_types(types);
        assert_eq!(totals(&typed, &[0])[0].suggested, Count);
    }

    #[test]
    fn mostly_numbers_is_a_sum_and_less_is_a_count() {
        let mut text = String::from("n,m\r\n");
        for i in 0..100 {
            let n = if i < 5 { "x" } else { "2" };
            let m = if i < 6 { "x" } else { "2" };
            text.push_str(&format!("{n},{m}\r\n"));
        }
        let t = totals(&doc(&text), &[0, 1]);
        assert_eq!(t[0].suggested, Suggested::Sum);
        assert_eq!(t[1].suggested, Suggested::Count);
    }

    #[test]
    fn typed_columns_and_hidden_rows() {
        let mut d = doc("a,b\r\n1,1\r\n2,2\r\n4,4\r\n");
        let types = d.type_overlay().with(0, ColumnType::Id);
        d.set_column_types(types);
        d.hide_rows(2, 1).unwrap();
        let t = totals(&d, &[0, 1]);
        assert_eq!((t[0].suggested, t[0].totals.sum), (Suggested::Count, None));
        assert_eq!((t[1].totals.sum, t[1].totals.cells), (Some(5.0), 2));
        assert!(d
            .column_totals(&[2], true, ReadOptions::default(), &Progress::new())
            .is_err());
    }

    #[test]
    fn a_cancelled_pass_stops() {
        let d = doc("a\r\n1\r\n");
        let p = Progress::new();
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            d.column_totals(&[0], true, ReadOptions::default(), &p),
            Err(Error::Cancelled)
        ));
    }

    proptest::proptest! {
        /// Each column's total is the total of selecting that column alone.
        #[test]
        fn equals_selection_totals(
            rows in proptest::collection::vec(
                proptest::collection::vec(
                    proptest::sample::select(vec!["", " ", "1", "2.5", "-3", "x", "1e308", "0.1", "007"]),
                    3,
                ),
                0..40,
            ),
            hide in proptest::collection::vec(0u64..40, 0..5),
        ) {
            let mut text = String::from("a,b,c\r\n");
            for r in &rows {
                text.push_str(&format!("\"{}\",\"{}\",\"{}\"\r\n", r[0], r[1], r[2]));
            }
            let mut d = doc(&text);
            for h in hide {
                if h >= 1 && h < d.row_count() {
                    d.hide_rows(h, 1).unwrap();
                }
            }
            let got = totals(&d, &[2, 0, 1]);
            for t in got {
                let n = d.row_count() - 1;
                let want = d
                    .selection_totals(1, n, t.col, 1, ReadOptions::default(), &Progress::new())
                    .unwrap();
                proptest::prop_assert_eq!(t.totals.filled, want.filled);
                proptest::prop_assert_eq!(t.totals.numbers, want.numbers);
                proptest::prop_assert_eq!(t.totals.sum, want.sum);
                proptest::prop_assert_eq!(t.totals.min, want.min);
                proptest::prop_assert_eq!(t.totals.max, want.max);
            }
        }
    }
}
