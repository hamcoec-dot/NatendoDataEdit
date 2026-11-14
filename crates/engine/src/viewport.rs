//! Reading a rectangular window of cells for display.

use std::ops::Range;

use crate::document::{Document, RowSource};
use crate::scan;
use crate::Error;

/// Largest window one request may ask for.
pub const MAX_CELLS: u64 = 200_000;
/// Hard cap on characters per cell in a viewport (keeps every cell's byte
/// length far below the 2 GiB the binary form can describe).
pub const MAX_CELL_CHARS: usize = 1 << 20;
/// Default per-cell character cap for display.
pub const DEFAULT_CELL_CHARS: usize = 4096;

/// A window of cells, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewport {
    pub first_row: u64,
    pub first_col: u32,
    pub rows: u32,
    pub cols: u32,
    pub cells: Vec<String>,
    /// True where the cell was cut to the character cap.
    pub truncated: Vec<bool>,
    /// With a filter on: each row's number in the whole document (0-based).
    pub row_numbers: Option<Vec<u64>>,
}

impl Viewport {
    pub fn cell(&self, row: u32, col: u32) -> Option<&str> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.cells
            .get(row as usize * self.cols as usize + col as usize)
            .map(String::as_str)
    }

    /// Compact binary form for IPC, little-endian:
    /// `u64 first_row, u32 first_col, u32 rows, u32 cols`, then per cell
    /// `u32 (byte_len << 1 | truncated)` followed by the UTF-8 bytes; with a
    /// filter on, then `u32 rows` and that many `u64` document row numbers.
    pub fn to_bytes(&self) -> Vec<u8> {
        let text: usize = self.cells.iter().map(String::len).sum();
        let mut out = Vec::with_capacity(20 + self.cells.len() * 4 + text);
        out.extend_from_slice(&self.first_row.to_le_bytes());
        out.extend_from_slice(&self.first_col.to_le_bytes());
        out.extend_from_slice(&self.rows.to_le_bytes());
        out.extend_from_slice(&self.cols.to_le_bytes());
        for (c, &t) in self.cells.iter().zip(&self.truncated) {
            // Cells are capped far below 2^31 bytes.
            let head = ((c.len() as u32) << 1) | t as u32;
            out.extend_from_slice(&head.to_le_bytes());
            out.extend_from_slice(c.as_bytes());
        }
        if let Some(nums) = &self.row_numbers {
            out.extend_from_slice(&(nums.len() as u32).to_le_bytes());
            for n in nums {
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
        out
    }
}

/// Cut `s` to at most `max` characters.
fn cap(mut s: String, max: usize) -> (String, bool) {
    match s.char_indices().nth(max) {
        Some((i, _)) => {
            s.truncate(i);
            (s, true)
        }
        None => (s, false),
    }
}

impl Document {
    /// Read rows `rows` × columns `cols` (clipped to the document), each cell
    /// cut to `max_cell_chars` characters.
    pub fn viewport(
        &self,
        rows: Range<u64>,
        cols: Range<u32>,
        max_cell_chars: usize,
    ) -> Result<Viewport, Error> {
        let max_cell_chars = max_cell_chars.min(MAX_CELL_CHARS);
        let r0 = rows.start.min(self.row_count());
        let r1 = rows.end.clamp(r0, self.row_count());
        let c0 = cols.start;
        let c1 = cols.end.max(c0);
        let (nr, nc) = (r1 - r0, (c1 - c0) as u64);
        if nr.saturating_mul(nc) > MAX_CELLS {
            return Err(Error::ViewportTooLarge);
        }
        let mut cells = Vec::with_capacity((nr * nc) as usize);
        let mut truncated = Vec::with_capacity(cells.capacity());
        // Raw bytes decoded per cell: enough for the cap even at 4 bytes per
        // character plus doubled quotes, so a huge cell is never fully copied.
        let byte_cap = max_cell_chars.saturating_mul(8).saturating_add(8);
        // With a column map, the physical columns wanted, sorted, with the
        // slot each fills: one scan of the row serves the whole window.
        let mut wanted: Vec<(u32, usize)> = Vec::new();
        if self.col_map().is_some() {
            wanted = (c0..c1)
                .enumerate()
                .filter_map(|(slot, c)| self.phys(c).map(|p| (p, slot)))
                .collect();
            wanted.sort_unstable();
        }
        let mapped = self.col_map().is_some();
        let decode = |raw: &[u8]| -> (String, bool) {
            let cut = raw.len() > byte_cap;
            let (text, t) = cap(
                self.decode_field(&raw[..raw.len().min(byte_cap)]),
                max_cell_chars,
            );
            (text, t || cut)
        };
        for r in r0..r1 {
            let id = self.id_at(r).expect("row in range");
            let start = cells.len();
            match self.row_source(id) {
                RowSource::Edited(v) => {
                    for c in c0..c1 {
                        let s = self
                            .phys(c)
                            .and_then(|p| v.get(p as usize))
                            .cloned()
                            .unwrap_or_default();
                        let (s, t) = cap(s, max_cell_chars);
                        cells.push(s);
                        truncated.push(t);
                    }
                }
                RowSource::Raw { bytes, mode } if mapped => {
                    cells.resize(start + nc as usize, String::new());
                    truncated.resize(start + nc as usize, false);
                    let mut i = 0u32;
                    let mut k = 0usize;
                    scan::scan_row(bytes, 0, self.format.dialect, mode, |s, e| {
                        if k >= wanted.len() {
                            return false;
                        }
                        if wanted[k].0 == i {
                            let (text, t) = decode(&bytes[s..e]);
                            cells[start + wanted[k].1] = text;
                            truncated[start + wanted[k].1] = t;
                            k += 1;
                        }
                        i += 1;
                        true
                    });
                }
                RowSource::Raw { bytes, mode } => {
                    let mut i = 0u32;
                    scan::scan_row(bytes, 0, self.format.dialect, mode, |s, e| {
                        if i >= c1 {
                            return false;
                        }
                        if i >= c0 {
                            let (text, t) = decode(&bytes[s..e]);
                            cells.push(text);
                            truncated.push(t);
                        }
                        i += 1;
                        true
                    });
                }
            }
            // Short rows: pad with empty cells.
            while cells.len() < start + nc as usize {
                cells.push(String::new());
                truncated.push(false);
            }
        }
        Ok(Viewport {
            first_row: r0,
            first_col: c0,
            rows: nr as u32,
            cols: nc as u32,
            cells,
            truncated,
            row_numbers: self
                .is_filtered()
                .then(|| (r0..r1).filter_map(|r| self.row_number(r)).collect()),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::document::{Document, OpenOptions};

    fn doc(b: &[u8]) -> Document {
        Document::from_bytes(b.to_vec(), OpenOptions::default(), None).unwrap()
    }

    #[test]
    fn window_and_padding() {
        let d = doc(b"a,b,c\n1,2\n\"x\"\"y\",,z\n");
        let v = d.viewport(1..10, 1..3, 100).unwrap();
        assert_eq!((v.rows, v.cols), (2, 2));
        assert_eq!(v.cells, ["2", "", "", "z"]);
        let v = d.viewport(2..3, 0..1, 100).unwrap();
        assert_eq!(v.cell(0, 0), Some("x\"y"));
    }

    #[test]
    fn truncates_long_cells() {
        let big = format!("\"{}\",b\n", "é".repeat(10_000));
        let d = doc(big.as_bytes());
        let v = d.viewport(0..1, 0..2, 10).unwrap();
        assert_eq!(v.cells[0], "é".repeat(10));
        assert!(v.truncated[0]);
        assert_eq!(v.cells[1], "b");
        assert!(!v.truncated[1]);
        assert_eq!(d.cell(0, 0).unwrap().chars().count(), 10_000);
    }

    #[test]
    fn rejects_huge_requests_and_clips() {
        let d = doc(b"a\n");
        assert!(d.viewport(0..1, 0..1_000_000, 10).is_err());
        // Rows are clipped to the document before the size check.
        assert!(d.viewport(0..1_000_000, 0..1_000, 10).is_ok());
        let v = d.viewport(5..9, 0..1, 10).unwrap();
        assert_eq!(v.rows, 0);
    }

    #[test]
    fn window_through_column_map() {
        let mut d = doc(b"a,b,c,d\n1,2,3,4\n\"x\"\"y\"\n");
        d.move_cols(3, 1, 0).unwrap();
        d.insert_cols(2, 1).unwrap();
        d.set_cell(2, 3, "e".into()).unwrap();
        let v = d.viewport(0..3, 0..6, 100).unwrap();
        assert_eq!(
            v.cells,
            [
                "d", "a", "", "b", "c", "", //
                "4", "1", "", "2", "3", "", //
                "", "x\"y", "", "e", "", "",
            ]
        );
        let v = d.viewport(0..2, 3..5, 100).unwrap();
        assert_eq!(v.cells, ["b", "c", "2", "3"]);
    }

    #[test]
    fn binary_form() {
        let d = doc(b"ab,c\n");
        let b = d.viewport(0..1, 0..2, 10).unwrap().to_bytes();
        assert_eq!(&b[..8], &0u64.to_le_bytes());
        assert_eq!(&b[12..20], &[1, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(&b[20..24], &(2u32 << 1).to_le_bytes());
        assert_eq!(&b[24..26], b"ab");
    }
}
