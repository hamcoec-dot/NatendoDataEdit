//! Rows with a different number of fields than most: the count known as soon
//! as the file is read, the job that finds the rows, and the view that shows
//! only them. A row's width is what a save would write: its stored record, or
//! its fields in the file.

use super::*;

const CHUNK: usize = 1 << 14;

/// Why nothing is called ragged while columns have been added, removed or
/// moved.
pub const COLUMNS_CHANGED: &str = "columns were added, removed or moved: saving writes every row at the same width; undo that change to repair shifted rows first";

/// Which of the ragged rows to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaggedPick {
    #[default]
    All,
    Short,
    Long,
}

/// The rows whose width differs from the usual one.
#[derive(Debug, Clone, PartialEq)]
pub struct Ragged {
    /// The number of fields most rows of the file have.
    pub usual: u32,
    pub short: u64,
    pub long: u64,
    /// Row ids (below `base`), one bit each.
    short_bits: Vec<u64>,
    long_bits: Vec<u64>,
    base: u32,
    revision: u64,
}

impl Ragged {
    pub fn is_short(&self, id: u32) -> bool {
        bit(&self.short_bits, id)
    }

    pub fn is_long(&self, id: u32) -> bool {
        bit(&self.long_bits, id)
    }
}

fn bit(bits: &[u64], id: u32) -> bool {
    bits.get((id >> 6) as usize)
        .is_some_and(|w| w & (1 << (id & 63)) != 0)
}

impl Document {
    /// Rows of the file that had another number of fields than most when it
    /// was read, from the counts kept at index time (nothing is read). `None`
    /// until the whole file is indexed. With `header`, the first row is left
    /// out.
    pub fn ragged_at_open(&self, header: bool) -> Option<(u32, u64)> {
        if !self.index.is_complete() {
            return None;
        }
        let stats = self.index.stats();
        let usual = stats.usual_fields();
        let rows = self.index.rows();
        let mut n = rows.saturating_sub(stats.field_counts.get(&usual).copied().unwrap_or(0));
        if header && rows > 0 && self.file_fields(0) != u64::from(usual) {
            n = n.saturating_sub(1);
        }
        Some((usual, n))
    }

    /// True when the file's first row had another number of fields than most.
    pub fn first_row_ragged(&self) -> bool {
        self.ragged_at_open(false).is_some_and(|(usual, _)| {
            self.index.rows() > 0 && self.file_fields(0) != u64::from(usual)
        })
    }

    /// Find every ragged row, hidden ones too. With `header`, the first row
    /// is never one.
    pub fn ragged_rows(&self, header: bool, progress: &Progress) -> Result<Ragged, Error> {
        self.ready_for_bulk()?;
        if self.col_map().is_some() {
            return Err(Error::Invalid(COLUMNS_CHANGED.into()));
        }
        let usual = self.index.stats().usual_fields();
        let base = self.next_row_id();
        let words = (base as usize).div_ceil(64);
        let first = u64::from(header);
        let total = self.total_rows();
        let n = total.saturating_sub(first);
        progress.start(n);
        let found: Vec<Vec<(u32, bool)>> = (0..(n as usize).div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<Vec<(u32, bool)>, Error> {
                let s = first + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(total);
                let mut out = Vec::new();
                for pos in s..e {
                    let id = self.doc_id_at(pos).ok_or(Error::OutOfRange)?;
                    let width = match self.overlay.get(id) {
                        Some(edit) => u64::from(edit.width()),
                        None => self.file_fields(id),
                    };
                    if width != u64::from(usual) {
                        out.push((id, width < u64::from(usual)));
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let mut r = Ragged {
            usual,
            short: 0,
            long: 0,
            short_bits: vec![0; words],
            long_bits: vec![0; words],
            base,
            revision: self.revision(),
        };
        for (id, short) in found.into_iter().flatten() {
            let (bits, count) = if short {
                (&mut r.short_bits, &mut r.short)
            } else {
                (&mut r.long_bits, &mut r.long)
            };
            bits[(id >> 6) as usize] |= 1 << (id & 63);
            *count += 1;
        }
        Ok(r)
    }

    /// The ids of the ragged rows in table order, at most `most`, and how
    /// many there are.
    pub fn ragged_ids(&self, found: &Ragged, most: usize) -> (Vec<u32>, u64) {
        let ids = (0..self.total_rows())
            .filter_map(|p| self.doc_id_at(p))
            .filter(|&id| found.is_short(id) || found.is_long(id))
            .take(most)
            .collect();
        (ids, found.short + found.long)
    }

    /// Show only the ragged rows (and the title row with `header`). Refused
    /// when the table changed since they were found.
    pub fn plan_show_ragged(
        &self,
        found: &Ragged,
        header: bool,
        pick: RaggedPick,
    ) -> Result<crate::reshape::RowPlan, Error> {
        if found.revision != self.revision() {
            return Err(Error::Invalid(
                "the table changed since the rows were found; look again".into(),
            ));
        }
        let mut bits = match pick {
            RaggedPick::Short => found.short_bits.clone(),
            RaggedPick::Long => found.long_bits.clone(),
            RaggedPick::All => found
                .short_bits
                .iter()
                .zip(&found.long_bits)
                .map(|(a, b)| a | b)
                .collect(),
        };
        let shown = match pick {
            RaggedPick::Short => found.short,
            RaggedPick::Long => found.long,
            RaggedPick::All => found.short + found.long,
        };
        if header {
            if let Some(id) = self.doc_id_at(0) {
                bits[(id >> 6) as usize] |= 1 << (id & 63);
            }
        }
        let mut plan = crate::reshape::RowPlan::new(self);
        plan.rows = shown;
        plan.view = Some(View::new(
            FilterSpec {
                keep_first: header,
                ..Default::default()
            },
            bits,
            found.base,
        ));
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(body: &str) -> Document {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("nde-ragged-{}-{n}.csv", std::process::id()));
        std::fs::write(&p, body).unwrap();
        Document::open_indexed(&p, OpenOptions::default(), None).unwrap()
    }

    /// The same answer worked out the slow way, from each row's stored width.
    fn brute(d: &Document, header: bool, usual: u32) -> (Vec<u32>, Vec<u32>) {
        let mut short = Vec::new();
        let mut long = Vec::new();
        for pos in u64::from(header)..d.total_rows() {
            let id = d.doc_id_at(pos).unwrap();
            let w = d.logical_values(id).unwrap().len() as u32;
            if w < usual {
                short.push(id);
            } else if w > usual {
                long.push(id);
            }
        }
        short.sort_unstable();
        long.sort_unstable();
        (short, long)
    }

    fn check(d: &Document) -> Ragged {
        let r = d.ragged_rows(true, &Progress::new()).unwrap();
        let (short, long) = brute(d, true, r.usual);
        assert_eq!((r.short, r.long), (short.len() as u64, long.len() as u64));
        for id in 0..d.next_row_id() {
            assert_eq!(r.is_short(id), short.contains(&id), "row {id}");
            assert_eq!(r.is_long(id), long.contains(&id), "row {id}");
        }
        r
    }

    const BODY: &str = "a,b,c\n1,2,3\n4,Smith, John,6\n7\n\"x,y\",2,3\n8,9,10\n\n";

    #[test]
    fn finds_short_and_long_rows_and_counts_them_at_open() {
        let d = doc(BODY);
        let r = check(&d);
        assert_eq!(r.usual, 3);
        assert_eq!(r.long, 1);
        assert_eq!(d.ragged_at_open(true), Some((3, r.short + r.long)));
        assert!(
            doc("a,b\n1,2\n")
                .ragged_rows(true, &Progress::new())
                .unwrap()
                .short
                == 0
        );
    }

    #[test]
    fn follows_edits_sorts_deletes_and_new_rows() {
        let mut d = doc(BODY);
        d.set_cell(3, 2, "x".into()).unwrap();
        check(&d);
        d.delete_rows(2, 1).unwrap();
        check(&d);
        d.insert_rows(1, vec![vec!["only".into()]]).unwrap();
        check(&d);
        assert!(d.undo());
        assert!(d.undo());
        check(&d);
    }

    #[test]
    fn nothing_is_ragged_while_columns_have_changed() {
        let mut d = doc(BODY);
        d.insert_cols(1, 1).unwrap();
        let e = d.ragged_rows(true, &Progress::new()).unwrap_err();
        assert!(e.to_string().contains("added, removed or moved"), "{e}");
        assert!(d.undo());
        check(&d);
    }

    #[test]
    fn the_view_shows_the_asked_rows_and_refuses_a_stale_list() {
        let mut d = doc(BODY);
        let r = check(&d);
        let plan = d.plan_show_ragged(&r, true, RaggedPick::Long).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 2);
        assert_eq!(d.cell(1, 0).as_deref(), Some("4"));
        d.set_cell(1, 0, "changed".into()).unwrap();
        assert!(d.plan_show_ragged(&r, true, RaggedPick::All).is_err());
    }
}
