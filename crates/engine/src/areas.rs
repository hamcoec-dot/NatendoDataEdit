//! Several selected blocks (Ctrl+click): the same cells as blocks that do not
//! overlap, so a count or a write never touches a cell twice. The window
//! splits them too, but the engine never trusts that.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// A block of shown rows and columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Area {
    pub row: u64,
    pub rows: u64,
    pub col: u32,
    pub cols: u32,
}

impl Area {
    pub fn cells(&self) -> u64 {
        self.rows.saturating_mul(self.cols as u64)
    }
}

/// The same cells as non-overlapping blocks, sweeping down the rows: each band
/// of rows between two block edges merges the column ranges of the blocks it
/// crosses, and bands that give the same ranges one below the other are
/// joined. Empty blocks are dropped. Blocks that reach past `u64`/`u32` are
/// clamped (the caller checks the table's size).
pub fn disjoint(areas: &[Area]) -> Vec<Area> {
    let mut starts: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    let mut ends: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    let mut ys: BTreeSet<u64> = BTreeSet::new();
    for (i, a) in areas.iter().enumerate() {
        if a.rows == 0 || a.cols == 0 {
            continue;
        }
        let e = a.row.saturating_add(a.rows);
        starts.entry(a.row).or_default().push(i);
        ends.entry(e).or_default().push(i);
        ys.insert(a.row);
        ys.insert(e);
    }
    let bounds: Vec<u64> = ys.into_iter().collect();
    let mut active: BTreeSet<usize> = BTreeSet::new();
    let mut out: Vec<Area> = Vec::new();
    // (col start, col end) -> index in `out` of the block still growing down.
    let mut open: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for w in bounds.windows(2) {
        let (y, next_y) = (w[0], w[1]);
        if let Some(v) = ends.get(&y) {
            for i in v {
                active.remove(i);
            }
        }
        if let Some(v) = starts.get(&y) {
            for &i in v {
                active.insert(i);
            }
        }
        let height = next_y - y;
        let mut next_open: BTreeMap<(u32, u32), usize> = BTreeMap::new();
        if !active.is_empty() {
            let mut spans: Vec<(u32, u32)> = active
                .iter()
                .map(|&i| {
                    let a = &areas[i];
                    (a.col, a.col.saturating_add(a.cols))
                })
                .collect();
            spans.sort_unstable();
            let mut emit = |c0: u32, c1: u32, out: &mut Vec<Area>| {
                let at = match open.get(&(c0, c1)) {
                    Some(&k) => {
                        out[k].rows += height;
                        k
                    }
                    None => {
                        out.push(Area {
                            row: y,
                            rows: height,
                            col: c0,
                            cols: c1 - c0,
                        });
                        out.len() - 1
                    }
                };
                next_open.insert((c0, c1), at);
            };
            let (mut c0, mut c1) = spans[0];
            for &(a, b) in &spans[1..] {
                if a <= c1 {
                    c1 = c1.max(b);
                } else {
                    emit(c0, c1, &mut out);
                    c0 = a;
                    c1 = b;
                }
            }
            emit(c0, c1, &mut out);
        }
        open = next_open;
    }
    out
}

/// Cells covered by the blocks, each counted once.
pub fn cell_count(areas: &[Area]) -> u64 {
    disjoint(areas)
        .iter()
        .fold(0u64, |n, a| n.saturating_add(a.cells()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn cells(areas: &[Area]) -> HashSet<(u64, u32)> {
        let mut s = HashSet::new();
        for a in areas {
            for r in a.row..a.row + a.rows {
                for c in a.col..a.col + a.cols {
                    s.insert((r, c));
                }
            }
        }
        s
    }

    fn a(row: u64, rows: u64, col: u32, cols: u32) -> Area {
        Area {
            row,
            rows,
            col,
            cols,
        }
    }

    #[test]
    fn overlapping_nested_touching_and_identical_blocks_split_into_the_same_cells() {
        let cases = vec![
            vec![a(0, 4, 0, 4), a(2, 4, 2, 4)],
            vec![a(0, 10, 0, 10), a(3, 2, 3, 2)],
            vec![a(0, 2, 0, 2), a(2, 2, 0, 2)],
            vec![a(1, 2, 1, 2), a(1, 2, 1, 2)],
            vec![a(5, 1, 5, 1), a(5, 1, 6, 1), a(5, 1, 5, 2)],
            vec![],
            vec![a(3, 0, 1, 1), a(1, 1, 0, 0)],
        ];
        for c in cases {
            let d = disjoint(&c);
            let want = cells(&c);
            assert_eq!(cells(&d), want);
            let total: u64 = d.iter().map(Area::cells).sum();
            assert_eq!(total, want.len() as u64, "no cell twice for {c:?}");
            assert_eq!(cell_count(&c), want.len() as u64);
        }
    }

    #[test]
    fn random_small_cases_agree_with_a_cell_set() {
        let mut x: u32 = 12345;
        let mut rnd = |n: u32| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 8) % n
        };
        for _ in 0..500 {
            let n = 1 + rnd(8);
            let areas: Vec<Area> = (0..n)
                .map(|_| a(rnd(8) as u64, 1 + rnd(5) as u64, rnd(8), 1 + rnd(5)))
                .collect();
            let d = disjoint(&areas);
            let want = cells(&areas);
            assert_eq!(cells(&d), want);
            assert_eq!(d.iter().map(Area::cells).sum::<u64>(), want.len() as u64);
        }
    }

    #[test]
    fn ten_thousand_blocks_split_fast() {
        let areas: Vec<Area> = (0..10_000u64)
            .map(|i| a((i * 7919) % 1_000_000, 1, (i % 20) as u32, 1))
            .collect();
        let t = std::time::Instant::now();
        let d = disjoint(&areas);
        // 100 ms in a release build (about 6 ms here); a debug build on a slow runner is far slower.
        let limit = if cfg!(debug_assertions) { 1_500 } else { 100 };
        assert!(t.elapsed().as_millis() < limit, "{:?}", t.elapsed());
        assert!(!d.is_empty());
    }
}
