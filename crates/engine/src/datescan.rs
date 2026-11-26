//! Looking through one column for dates that can be read two ways: counts of
//! each kind, the rows to light amber, and the next one from here.

use rayon::prelude::*;
use serde::Serialize;

use crate::coltype::ColumnType;
use crate::ops::{check_cols, Cols};
use crate::value::{classify_date, DateKind, DateOrder, ReadOptions};
use crate::{Document, Error, Progress};

/// Most cells the "type each" list holds.
const DATE_LIST_MAX: usize = 500;
const CHUNK: usize = 1 << 14;

/// Words in a title that mean eight digits are a number, not a date
/// (registration numbers, IDs).
const NOT_DATE_TITLES: [&str; 5] = ["id", "no", "number", "#", "reg"];

/// What a column holds, as dates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DateScan {
    /// Shown data rows looked at, and how many there are.
    pub looked: u64,
    pub rows: u64,
    pub blank: u64,
    pub iso: u64,
    pub written: u64,
    pub month_first_only: u64,
    pub day_first_only: u64,
    pub ambiguous: u64,
    pub same_either: u64,
    /// Eight digits that are a calendar date, counted as dates.
    pub compact: u64,
    /// Eight-digit values that could be dates but are taken for numbers.
    pub compact_numbers: u64,
    pub not_date: u64,
    /// Among the dates: years written with two digits.
    pub two_digit_year: u64,
    /// The column is set to this order: nothing is ambiguous.
    pub set_order: Option<DateOrder>,
    /// With a set order: cells only the other order can read.
    pub other_order: u64,
}

impl DateScan {
    fn add(&mut self, o: &DateScan) {
        self.blank += o.blank;
        self.iso += o.iso;
        self.written += o.written;
        self.month_first_only += o.month_first_only;
        self.day_first_only += o.day_first_only;
        self.ambiguous += o.ambiguous;
        self.same_either += o.same_either;
        self.compact += o.compact;
        self.not_date += o.not_date;
        self.two_digit_year += o.two_digit_year;
    }

    /// Cells that are dates, eight-digit ones aside.
    fn dates(&self) -> u64 {
        self.iso
            + self.written
            + self.month_first_only
            + self.day_first_only
            + self.ambiguous
            + self.same_either
    }
}

impl Document {
    /// The order a column's type already says, and why a column cannot be
    /// looked through for dates.
    fn date_column(&self, col: u32) -> Result<(Option<DateOrder>, bool), Error> {
        check_cols(self, &[col])?;
        match self.read_ctx(ReadOptions::default()).type_of(col) {
            ColumnType::Text => Err(Error::Invalid(
                "this column is set to Text: its cells are not read as dates".into(),
            )),
            ColumnType::Id => Err(Error::Invalid(
                "this column is set to Id: its cells are not read as dates".into(),
            )),
            ColumnType::Date(order) => Ok((order, false)),
            ColumnType::Auto => Ok((None, false)),
            // A number column: eight digits are numbers.
            ColumnType::Number => Ok((None, true)),
        }
    }

    /// Whether eight-digit values of this column may ever be dates: not in a
    /// Number column, nor under a title that says it holds numbers or IDs.
    fn compact_allowed(&self, col: u32, header: bool, number: bool) -> bool {
        if number {
            return false;
        }
        let title = if header {
            self.first_row_cell(col).unwrap_or_default().to_lowercase()
        } else {
            String::new()
        };
        !NOT_DATE_TITLES.iter().any(|w| title.contains(w))
    }

    /// Count the kinds of date in column `col` over the shown rows, at most
    /// `sample` of them (the first ones).
    pub fn date_scan(
        &self,
        col: u32,
        header: bool,
        sample: Option<u64>,
        progress: &Progress,
    ) -> Result<DateScan, Error> {
        self.ready_for_bulk()?;
        let (set_order, number) = self.date_column(col)?;
        let first = u64::from(header);
        let end = self.row_count();
        let rows = end.saturating_sub(first);
        let looked = sample.map_or(rows, |s| s.min(rows));
        let reader = Cols::new(self, &[col]);
        progress.start(looked);
        let parts: Vec<DateScan> = (0..(looked as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<DateScan, Error> {
                let s = first + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(first + looked);
                let mut n = DateScan::default();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let v = reader.get(&buf, 0);
                    if v.trim().is_empty() {
                        n.blank += 1;
                        continue;
                    }
                    let class = classify_date(v);
                    n.two_digit_year += u64::from(class.two_digit_year);
                    *match class.kind {
                        DateKind::Iso => &mut n.iso,
                        DateKind::Written => &mut n.written,
                        DateKind::MonthFirstOnly => &mut n.month_first_only,
                        DateKind::DayFirstOnly => &mut n.day_first_only,
                        DateKind::Ambiguous => &mut n.ambiguous,
                        DateKind::SameEither => &mut n.same_either,
                        DateKind::Compact => &mut n.compact,
                        DateKind::NotDate => &mut n.not_date,
                    } += 1;
                }
                progress.step(e - s)?;
                Ok(n)
            })
            .collect::<Result<_, _>>()?;
        let mut out = DateScan {
            looked,
            rows,
            set_order,
            ..Default::default()
        };
        for p in &parts {
            out.add(p);
        }
        // Eight digits count as dates only where they may be, and only when
        // at least half of the filled cells are dates.
        let filled = looked - out.blank;
        let counted = self.compact_allowed(col, header, number)
            && (out.dates() + out.compact) * 2 >= filled
            && out.dates() > 0;
        if !counted {
            out.compact_numbers = std::mem::take(&mut out.compact);
        }
        if let Some(order) = set_order {
            out.other_order = match order {
                DateOrder::Mdy => out.day_first_only,
                DateOrder::Dmy => out.month_first_only,
            };
            out.ambiguous = 0;
        }
        Ok(out)
    }

    /// Whether the cell of shown row `row` is one to light: ambiguous, or in
    /// a column set to an order, readable only by the other one.
    fn date_flag(&self, reader: &Cols, row: u64, set: Option<DateOrder>) -> bool {
        let Some(id) = self.id_at(row) else {
            return false;
        };
        let mut cells = Vec::new();
        reader.read(self, id, &mut cells);
        let kind = classify_date(reader.get(&cells, 0)).kind;
        match set {
            None => kind == DateKind::Ambiguous,
            Some(DateOrder::Mdy) => kind == DateKind::DayFirstOnly,
            Some(DateOrder::Dmy) => kind == DateKind::MonthFirstOnly,
        }
    }

    /// The first `limit` shown rows whose date in `col` can be read two
    /// ways, with the row's number and the cell's text (the review screen's
    /// "type each" list).
    pub fn date_list(
        &self,
        col: u32,
        header: bool,
        limit: usize,
        progress: &Progress,
    ) -> Result<Vec<(u64, u64, String)>, Error> {
        self.ready_for_bulk()?;
        self.date_column(col)?;
        let reader = Cols::new(self, &[col]);
        let end = self.row_count();
        let limit = limit.min(DATE_LIST_MAX);
        let mut out = Vec::new();
        let mut buf = Vec::new();
        progress.start(end.saturating_sub(u64::from(header)));
        for r in u64::from(header)..end {
            if out.len() >= limit {
                break;
            }
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let v = reader.get(&buf, 0);
            if classify_date(v).kind == DateKind::Ambiguous {
                out.push((r, self.row_number(r).unwrap_or(r), v.to_string()));
            }
            if r % CHUNK as u64 == 0 {
                progress.step(CHUNK as u64)?;
            }
        }
        Ok(out)
    }

    /// The shown rows among `row..row+rows` whose cell in `col` is to be lit
    /// amber (none in a column already set to an order).
    pub fn date_marks(
        &self,
        col: u32,
        header: bool,
        row: u64,
        rows: u64,
    ) -> Result<Vec<u64>, Error> {
        let (set, _) = self.date_column(col)?;
        if set.is_some() {
            return Ok(Vec::new());
        }
        let reader = Cols::new(self, &[col]);
        let end = row.saturating_add(rows.min(10_000)).min(self.row_count());
        Ok((row.max(u64::from(header))..end)
            .filter(|&r| self.date_flag(&reader, r, None))
            .collect())
    }

    /// The next (or previous) shown row after `from` whose cell is ambiguous
    /// (or, in a column set to an order, only readable the other way),
    /// wrapping around.
    pub fn date_find(
        &self,
        col: u32,
        header: bool,
        from: u64,
        forward: bool,
        progress: &Progress,
    ) -> Result<Option<u64>, Error> {
        self.ready_for_bulk()?;
        let (set, _) = self.date_column(col)?;
        let reader = Cols::new(self, &[col]);
        let first = u64::from(header);
        let total = self.row_count().saturating_sub(first);
        if total == 0 {
            return Ok(None);
        }
        let start = from.max(first).min(first + total - 1) - first;
        progress.start(total);
        for step in 1..=total {
            let at = if forward {
                (start + step) % total
            } else {
                (start + total - step) % total
            };
            if self.date_flag(&reader, first + at, set) {
                return Ok(Some(first + at));
            }
            if step % 8192 == 0 {
                progress.step(8192)?;
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coltype::ColumnType;
    use crate::OpenOptions;

    fn doc(body: &str) -> Document {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("nde-dates-{}-{n}.csv", std::process::id()));
        std::fs::write(&p, body).unwrap();
        Document::open_indexed(&p, OpenOptions::default(), None).unwrap()
    }

    fn set_type(d: &mut Document, t: ColumnType) {
        let types = d.type_overlay().with(0, t);
        d.set_column_types(types);
    }

    fn scan(d: &Document, col: u32) -> DateScan {
        d.date_scan(col, true, None, &Progress::new()).unwrap()
    }

    const BODY: &str = "born,reg_no\n25/04/2026,19750612\n03/04/2026,19800101\n04/25/2026,20010930\n05/05/2026,19991231\n,19660305\n2026-01-02,12345678\n20260425,19750101\nsoon,19750102\n07/08/2026,19750103\n";

    #[test]
    fn lists_the_ambiguous_cells_with_their_rows_up_to_the_limit() {
        let d = doc(BODY);
        let all = d.date_list(0, true, 500, &Progress::new()).unwrap();
        let text: Vec<(u64, &str)> = all.iter().map(|c| (c.0, c.2.as_str())).collect();
        assert_eq!(text, [(2, "03/04/2026"), (9, "07/08/2026")]);
        assert_eq!(d.date_list(0, true, 1, &Progress::new()).unwrap().len(), 1);
    }

    #[test]
    fn counts_each_kind_and_leaves_registration_numbers_alone() {
        let d = doc(BODY);
        let s = scan(&d, 0);
        assert_eq!(
            (
                s.day_first_only,
                s.month_first_only,
                s.ambiguous,
                s.same_either
            ),
            (1, 1, 2, 1)
        );
        assert_eq!(
            (s.iso, s.compact, s.compact_numbers, s.not_date, s.blank),
            (1, 1, 0, 1, 1)
        );
        assert_eq!((s.looked, s.rows), (9, 9));
        // Eight digits under "reg_no": numbers, however much they look like dates.
        let r = scan(&d, 1);
        assert_eq!((r.compact, r.compact_numbers, r.not_date), (0, 8, 1));
        // The first rows only.
        let few = d.date_scan(0, true, Some(2), &Progress::new()).unwrap();
        assert_eq!((few.looked, few.rows, few.ambiguous), (2, 9, 1));
    }

    #[test]
    fn eight_digits_need_half_the_cells_to_be_dates_and_a_plain_title() {
        let mostly = "when\n20260101\n20260102\n";
        assert_eq!(
            scan(&doc(mostly), 0).compact_numbers,
            2,
            "no other date at all"
        );
        let half = "when\n20260101\n2026-01-02\nx\n";
        assert_eq!(scan(&doc(half), 0).compact, 1);
        let rare = "when\n20260101\n2026-01-02\nx\ny\nz\n";
        assert_eq!(scan(&doc(rare), 0).compact_numbers, 1);
        for title in ["Voter ID", "No.", "Number", "#", "Registration"] {
            let body = format!("{title}\n20260101\n2026-01-02\n");
            assert_eq!(scan(&doc(&body), 0).compact_numbers, 1, "{title}");
        }
        let mut d = doc(half);
        set_type(&mut d, ColumnType::Number);
        assert_eq!(scan(&d, 0).compact_numbers, 1);
    }

    #[test]
    fn marks_and_next_follow_the_rows_and_a_set_order_answers() {
        let mut d = doc(BODY);
        assert_eq!(d.date_marks(0, true, 0, 100).unwrap(), [2, 9]);
        let p = Progress::new();
        assert_eq!(d.date_find(0, true, 2, true, &p).unwrap(), Some(9));
        assert_eq!(d.date_find(0, true, 9, true, &p).unwrap(), Some(2));
        assert_eq!(d.date_find(0, true, 2, false, &p).unwrap(), Some(9));
        set_type(&mut d, ColumnType::Date(Some(DateOrder::Dmy)));
        assert!(d.date_marks(0, true, 0, 100).unwrap().is_empty());
        let s = scan(&d, 0);
        assert_eq!(
            (s.ambiguous, s.other_order, s.set_order),
            (0, 1, Some(DateOrder::Dmy))
        );
        assert_eq!(
            d.date_find(0, true, 0, true, &p).unwrap(),
            Some(3),
            "04/25 needs month first"
        );
        set_type(&mut d, ColumnType::Text);
        assert!(d
            .date_marks(0, true, 0, 10)
            .unwrap_err()
            .to_string()
            .contains("Text"));
    }
}
