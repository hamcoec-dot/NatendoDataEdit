//! "What changed since this checkpoint" (B4, part 2): the cells that differ now from the state a checkpoint names,
//! worked out from the inverse steps the undo stack already holds. Nothing is rewound and no row is copied while a
//! checkpoint sits.
//!
//! The state a row had BEFORE the oldest step after the mark is its state at the mark, and the stack holds it (as a
//! reference here). Rows are matched by id, so sorting and deleting cannot confuse the comparison, and columns by
//! their physical id, so inserting, deleting and moving columns cannot either: the column map at the mark is rebuilt by
//! applying the stored inverse column steps, newest first, to the map now. A cell is compared only in a column that
//! exists at both times; columns added or removed since are counted.

use super::*;
use serde::{Deserialize, Serialize};

/// Rows compared per batch (and per unit of progress).
const CHUNK: usize = 16_384;

/// Counts of what changed since a mark.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinceTotals {
    /// Steps taken since the mark.
    pub steps: usize,
    /// Rows of the mark that hold a different value in some cell now.
    pub rows_changed: u64,
    /// Cells that hold a different value now (in columns that exist at both times).
    pub cells_changed: u64,
    /// Rows made since the mark (and still there).
    pub rows_added: u64,
    /// Rows of the mark taken out since.
    pub rows_removed: u64,
    /// Columns made since the mark (and still there).
    pub cols_added: u32,
    /// Columns of the mark taken out since.
    pub cols_removed: u32,
    /// The columns that are in both are in another order.
    pub cols_moved: bool,
    /// The rows were sorted or moved.
    pub reordered: bool,
}

/// One changed cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinceCell {
    /// The row's id (it follows the row through sorts and deletes).
    pub id: u32,
    /// Its position now in the whole table (filter or not).
    pub row: Option<u64>,
    /// Its column now (logical).
    pub col: u32,
    pub old: String,
    pub new: String,
}

/// What a first pass found, to page through the cells afterwards: the sorted ids of the changed rows and the cells
/// each batch holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SincePlan {
    pub totals: SinceTotals,
    ids: Vec<u32>,
    /// Changed cells in each batch of `CHUNK` ids (their running total decides which batch holds a page).
    chunk_cells: Vec<u64>,
}

/// What is known about the columns when the steps touched them.
struct Cols {
    /// Physical columns that exist at both times; `None` when no column step was taken (every column does).
    common: Option<HashSet<u32>>,
    /// For each physical column now, its logical position now.
    now_pos: HashMap<u32, u32>,
}

/// Applying one stored inverse column step to a column map.
fn undo_cols(map: &mut Option<Vec<u32>>, op: &Op) -> Option<()> {
    match op {
        Op::SetMap(m) => *map = m.clone(),
        Op::ColInsert { at, ids } => {
            let m = map.as_mut()?;
            if *at > m.len() {
                return None;
            }
            for (i, id) in ids.iter().enumerate() {
                m.insert(*at + i, *id);
            }
        }
        Op::ColRemove { at, count } => {
            let m = map.as_mut()?;
            if at + count > m.len() {
                return None;
            }
            m.drain(*at..*at + *count);
        }
        Op::MoveCols { at, count, to } => {
            let m = map.as_mut()?;
            if at + count > m.len() {
                return None;
            }
            let moved: Vec<u32> = m.drain(*at..*at + *count).collect();
            if *to > m.len() {
                return None;
            }
            for (i, id) in moved.into_iter().enumerate() {
                m.insert(*to + i, id);
            }
        }
        Op::Batch(v) => {
            // A batch is applied in order, its steps' effects on columns included.
            for o in v {
                undo_cols(map, o)?;
            }
        }
        _ => {}
    }
    Some(())
}

/// Applying one stored inverse step to a row order (the ids of the whole table, in order).
fn undo_order(ord: &mut Vec<u32>, op: &Op, original_rows: u32) -> Option<()> {
    match op {
        Op::SetOrder(Some(v)) => *ord = v.clone(),
        Op::SetOrder(None) => *ord = (0..original_rows).collect(),
        Op::Insert { at, rows } => {
            if *at > ord.len() {
                return None;
            }
            let ids: Vec<u32> = rows.iter().map(|(id, _)| *id).collect();
            ord.splice(*at..*at, ids);
        }
        Op::Remove { at, count } => {
            if at + count > ord.len() {
                return None;
            }
            ord.drain(*at..*at + *count);
        }
        Op::MoveRows { at, count, to } => {
            if at + count > ord.len() {
                return None;
            }
            let moved: Vec<u32> = ord.drain(*at..*at + *count).collect();
            if *to > ord.len() {
                return None;
            }
            ord.splice(*to..*to, moved);
        }
        Op::Batch(v) => {
            for o in v {
                undo_order(ord, o, original_rows)?;
            }
        }
        _ => {}
    }
    Some(())
}

fn touches_order(op: &Op) -> bool {
    match op {
        Op::SetOrder(_) | Op::Insert { .. } | Op::Remove { .. } | Op::MoveRows { .. } => true,
        Op::Batch(v) => v.iter().any(touches_order),
        _ => false,
    }
}

fn touches_columns(op: &Op) -> bool {
    match op {
        Op::SetMap(_) | Op::ColInsert { .. } | Op::ColRemove { .. } | Op::MoveCols { .. } => true,
        Op::Batch(v) => v.iter().any(touches_columns),
        _ => false,
    }
}

/// The result of reading the steps after a mark.
struct Read<'a> {
    first: HashMap<u32, &'a Option<Edit>>,
    /// Rows to compare: there at the mark and still there now (`None`: every row that was touched, since no step
    /// moved, added or took out a row).
    both: Option<Vec<u64>>,
    totals: SinceTotals,
    cols: Cols,
}

impl Read<'_> {
    /// Ids of the rows to compare, in order: touched, there at the mark and still there now.
    fn compared_ids(&self) -> Vec<u32> {
        self.first
            .keys()
            .copied()
            .filter(|id| match &self.both {
                Some(b) => b[(*id >> 6) as usize] & (1 << (*id & 63)) != 0,
                None => true,
            })
            .collect()
    }
}

impl Document {
    /// How the history stands to `mark`: the number of steps since it, or why a comparison cannot be made.
    fn since_steps(&self, mark: u64) -> Result<usize, Error> {
        match self.step_reach(mark) {
            None => Err(Error::Invalid(
                "That checkpoint can no longer be reached: the undo history was trimmed past it, a newer change replaced it, or the file was saved.".into(),
            )),
            Some(d) if d > 0 => Err(Error::Invalid(
                "That checkpoint is ahead of the table (you went back past it): go to it first, then compare.".into(),
            )),
            Some(d) => Ok((-d) as usize),
        }
    }

    fn since_read(&self, mark: u64, watermark: u32) -> Result<Read<'_>, Error> {
        let steps = self.since_steps(mark)?;
        let ops = &self.undo[self.undo.len() - steps..];
        let mut r = Read {
            first: HashMap::new(),
            both: None,
            totals: SinceTotals {
                steps,
                ..Default::default()
            },
            cols: Cols {
                common: None,
                now_pos: HashMap::new(),
            },
        };
        fn walk<'a>(op: &'a Op, first: &mut HashMap<u32, &'a Option<Edit>>) {
            match op {
                Op::Restore(states) => {
                    for (id, st) in states {
                        first.entry(*id).or_insert(st);
                    }
                }
                // Rows a later step took out: their state at the mark is kept by the step that took them out.
                Op::Insert { rows, .. } => {
                    for (id, st) in rows {
                        first.entry(*id).or_insert(st);
                    }
                }
                Op::Batch(v) => v.iter().for_each(|o| walk(o, first)),
                _ => {}
            }
        }
        for op in ops {
            walk(op, &mut r.first);
        }
        // Rows: the order at the mark is the order now with the stored inverse steps applied, newest first; the rows
        // taken out and the rows made are the difference of the two sets, and a sort or a move shows as another order
        // of the rows in both.
        if ops.iter().any(touches_order) {
            let original = self.index.rows() as u32;
            let now: Vec<u32> = match &self.order {
                Some(o) => o.clone(),
                None => (0..original).collect(),
            };
            let mut then = now.clone();
            for op in ops.iter().rev() {
                if touches_order(op) && undo_order(&mut then, op, original).is_none() {
                    return Err(Error::Invalid(
                        "The rows were changed in a way this comparison cannot follow exactly, so it shows nothing. Going to the checkpoint and looking at the table is exact.".into(),
                    ));
                }
            }
            let bits = (self.next_id as usize).div_ceil(64).max(1);
            let mut in_now = vec![0u64; bits];
            let mut in_then = vec![0u64; bits];
            for id in &now {
                in_now[(*id >> 6) as usize] |= 1 << (*id & 63);
            }
            for id in &then {
                in_then[(*id >> 6) as usize] |= 1 << (*id & 63);
            }
            let (mut removed, mut added) = (0u64, 0u64);
            let mut both = vec![0u64; bits];
            for w in 0..bits {
                removed += (in_then[w] & !in_now[w]).count_ones() as u64;
                added += (in_now[w] & !in_then[w]).count_ones() as u64;
                both[w] = in_then[w] & in_now[w];
            }
            r.totals.rows_removed = removed;
            r.totals.rows_added = added;
            let has = |set: &[u64], id: &u32| set[(*id >> 6) as usize] & (1 << (*id & 63)) != 0;
            let a: Vec<u32> = then.iter().copied().filter(|id| has(&both, id)).collect();
            let b: Vec<u32> = now.iter().copied().filter(|id| has(&both, id)).collect();
            r.totals.reordered = a != b;
            r.both = Some(both);
        }
        let _ = watermark;
        // Columns: the map at the mark is the map now with the stored inverse steps applied, newest first.
        if ops.iter().any(touches_columns) {
            let now: Vec<u32> = self.logical_to_physical();
            let mut then = self.cols.clone();
            for op in ops.iter().rev() {
                if !touches_columns(op) {
                    continue;
                }
                // A batch's inverse steps run in the order they are stored; a lone step is itself.
                if undo_cols(&mut then, op).is_none() {
                    return Err(Error::Invalid(
                        "The columns were changed in a way this comparison cannot follow exactly, so it shows nothing. Going to the checkpoint and looking at the table is exact.".into(),
                    ));
                }
            }
            let then_map: Vec<u32> = match then {
                Some(m) => m,
                None => (0..self.original_width()).collect(),
            };
            let now_set: HashSet<u32> = now.iter().copied().collect();
            let then_set: HashSet<u32> = then_map.iter().copied().collect();
            let common: HashSet<u32> = now_set.intersection(&then_set).copied().collect();
            r.totals.cols_added = now_set.difference(&then_set).count() as u32;
            r.totals.cols_removed = then_set.difference(&now_set).count() as u32;
            let order_then: Vec<u32> = then_map
                .iter()
                .copied()
                .filter(|p| common.contains(p))
                .collect();
            let order_now: Vec<u32> = now.iter().copied().filter(|p| common.contains(p)).collect();
            r.totals.cols_moved = order_then != order_now;
            r.cols.common = Some(common);
            r.cols.now_pos = now
                .iter()
                .enumerate()
                .map(|(i, p)| (*p, i as u32))
                .collect();
        }
        Ok(r)
    }

    /// The physical cells of row `id` that differ between its state at the mark (`then`) and now.
    fn since_diff(
        &self,
        id: u32,
        then: &Option<Edit>,
        cols: &Cols,
        out: &mut Vec<(u32, String, String)>,
    ) {
        out.clear();
        let then_vals: Vec<String> = match then {
            None => {
                let (bytes, mode) = self.raw_row(id);
                self.original_fields(id, bytes, mode).unwrap_or_default()
            }
            Some(Edit::Full(v)) => v.clone(),
            Some(Edit::Patch { width, cells }) => self.patched(id, *width, cells),
        };
        let now_vals: Vec<String> = match self.row_source(id) {
            RowSource::Edited(v) => v.into_owned(),
            RowSource::Raw { bytes, mode } => {
                self.original_fields(id, bytes, mode).unwrap_or_default()
            }
        };
        let n = then_vals.len().max(now_vals.len());
        for p in 0..n {
            if let Some(common) = &cols.common {
                if !common.contains(&(p as u32)) {
                    continue;
                }
            }
            let a = then_vals.get(p).map(String::as_str).unwrap_or("");
            let b = now_vals.get(p).map(String::as_str).unwrap_or("");
            if a != b {
                out.push((p as u32, a.to_string(), b.to_string()));
            }
        }
    }

    /// First pass: every count, and the sorted ids of the changed rows with the cells in each batch (one parallel
    /// pass, with progress and Cancel). Run when asked, never while a list is drawn.
    pub fn since_plan(
        &self,
        mark: u64,
        watermark: u32,
        progress: &Progress,
    ) -> Result<SincePlan, Error> {
        self.ready_for_bulk()?;
        let r = self.since_read(mark, watermark)?;
        let mut ids: Vec<u32> = r.compared_ids();
        ids.sort_unstable();
        progress.start(ids.len() as u64);
        let per: Vec<(u64, u64)> = ids
            .par_chunks(CHUNK)
            .map(|chunk| -> Result<(u64, u64), Error> {
                let mut rows = 0u64;
                let mut cells = 0u64;
                let mut buf = Vec::new();
                for id in chunk {
                    self.since_diff(*id, r.first[id], &r.cols, &mut buf);
                    if !buf.is_empty() {
                        rows += 1;
                        cells += buf.len() as u64;
                    }
                }
                progress.step(chunk.len() as u64)?;
                Ok((rows, cells))
            })
            .collect::<Result<_, _>>()?;
        let mut totals = r.totals.clone();
        totals.rows_changed = per.iter().map(|p| p.0).sum();
        totals.cells_changed = per.iter().map(|p| p.1).sum();
        Ok(SincePlan {
            totals,
            ids,
            chunk_cells: per.into_iter().map(|p| p.1).collect(),
        })
    }

    /// A page of the changed cells, in row id order, from a plan made for the same `mark` and `watermark`.
    pub fn since_page(
        &self,
        plan: &SincePlan,
        mark: u64,
        watermark: u32,
        offset: u64,
        want: usize,
    ) -> Result<Vec<SinceCell>, Error> {
        let r = self.since_read(mark, watermark)?;
        // The batch the page starts in.
        let mut skipped = 0u64;
        let mut start_chunk = 0usize;
        for (i, c) in plan.chunk_cells.iter().enumerate() {
            if skipped + c > offset {
                start_chunk = i;
                break;
            }
            skipped += c;
            start_chunk = i + 1;
        }
        let mut found: Vec<(u32, u32, String, String)> = Vec::new();
        let mut buf = Vec::new();
        'chunks: for chunk in plan.ids.chunks(CHUNK).skip(start_chunk) {
            for id in chunk {
                self.since_diff(*id, r.first[id], &r.cols, &mut buf);
                for (p, old, new) in buf.drain(..) {
                    if skipped < offset {
                        skipped += 1;
                        continue;
                    }
                    found.push((*id, p, old, new));
                    if found.len() >= want {
                        break 'chunks;
                    }
                }
            }
        }
        let wanted: HashSet<u32> = found.iter().map(|f| f.0).collect();
        let pos = self.positions_of(&wanted);
        Ok(found
            .into_iter()
            .map(|(id, p, old, new)| SinceCell {
                id,
                row: pos.get(&id).copied(),
                col: r.cols.now_pos.get(&p).copied().unwrap_or(match &self.cols {
                    // No column steps: the logical column is the physical one (through the map, if any).
                    Some(m) => m.iter().position(|q| *q == p).map_or(p, |i| i as u32),
                    None => p,
                }),
                old,
                new,
            })
            .collect())
    }

    /// Where each of these row ids is now, in the whole table (filter or not): one scan of the row order.
    pub(crate) fn positions_of(&self, ids: &HashSet<u32>) -> HashMap<u32, u64> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return out;
        }
        match &self.order {
            Some(o) => {
                for (i, id) in o.iter().enumerate() {
                    if ids.contains(id) {
                        out.insert(*id, i as u64);
                        if out.len() == ids.len() {
                            break;
                        }
                    }
                }
            }
            None => {
                for id in ids {
                    if (*id as u64) < self.index.rows() {
                        out.insert(*id, *id as u64);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(rows: usize) -> Document {
        let mut text = String::from("a,b,c,d\n");
        for i in 0..rows {
            text.push_str(&format!("{i},n{},{},z{}\n", i % 17, i % 5, i % 3));
        }
        Document::from_bytes(text.into_bytes(), OpenOptions::default(), None).unwrap()
    }

    /// Brute force: the whole table at a moment, keyed by row id then physical column.
    struct Snap {
        rows: HashMap<u32, Vec<String>>,
        cols: Vec<u32>,
    }
    fn snap(d: &Document) -> Snap {
        Snap {
            rows: (0..d.row_count())
                .map(|r| {
                    let id = d.id_at(r).unwrap();
                    (id, d.physical_values(id).unwrap())
                })
                .collect(),
            cols: d.logical_to_physical(),
        }
    }

    fn brute(then: &Snap, now: &Snap) -> (SinceTotals, Vec<(u32, u32, String, String)>) {
        let then_cols: HashSet<u32> = then.cols.iter().copied().collect();
        let now_cols: HashSet<u32> = now.cols.iter().copied().collect();
        let common: HashSet<u32> = then_cols.intersection(&now_cols).copied().collect();
        let mut t = SinceTotals {
            cols_added: now_cols.difference(&then_cols).count() as u32,
            cols_removed: then_cols.difference(&now_cols).count() as u32,
            ..Default::default()
        };
        let a: Vec<u32> = then
            .cols
            .iter()
            .copied()
            .filter(|p| common.contains(p))
            .collect();
        let b: Vec<u32> = now
            .cols
            .iter()
            .copied()
            .filter(|p| common.contains(p))
            .collect();
        t.cols_moved = a != b;
        let mut cells = Vec::new();
        let mut ids: Vec<u32> = then.rows.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(nv) = now.rows.get(&id) else {
                t.rows_removed += 1;
                continue;
            };
            let tv = &then.rows[&id];
            let mut any = false;
            for p in 0..tv.len().max(nv.len()) {
                if !common.contains(&(p as u32)) {
                    continue;
                }
                let (x, y) = (
                    tv.get(p).map(String::as_str).unwrap_or(""),
                    nv.get(p).map(String::as_str).unwrap_or(""),
                );
                if x != y {
                    any = true;
                    t.cells_changed += 1;
                    cells.push((id, p as u32, x.to_string(), y.to_string()));
                }
            }
            t.rows_changed += u64::from(any);
        }
        t.rows_added = now
            .rows
            .keys()
            .filter(|id| !then.rows.contains_key(id))
            .count() as u64;
        (t, cells)
    }

    fn random_steps(
        d: &mut Document,
        rnd: &mut impl FnMut(u64) -> u64,
        steps: u64,
        with_cols: bool,
        tag: usize,
    ) {
        for step in 0..steps {
            match rnd(if with_cols { 11 } else { 8 }) {
                0..=3 => {
                    let r = rnd(d.row_count());
                    let c = rnd(d.col_count() as u64) as u32;
                    let _ = d.set_cell(r, c, format!("e{tag}-{step}"));
                }
                4 if d.row_count() > 5 => {
                    let _ = d.delete_rows(rnd(d.row_count()), 1);
                }
                5 => {
                    let at = rnd(d.row_count() + 1);
                    let _ = d.insert_rows(
                        at,
                        vec![vec![
                            format!("new{step}"),
                            "x".into(),
                            "y".into(),
                            "w".into(),
                        ]],
                    );
                }
                6 => {
                    let _ = d.sort(&crate::sort::SortSpec {
                        keys: vec![crate::sort::SortKey {
                            col: rnd(d.col_count() as u64) as u32,
                            descending: rnd(2) == 0,
                            kind: Default::default(),
                            match_case: false,
                        }],
                        keep_first: true,
                        ..Default::default()
                    });
                }
                8 => {
                    let _ = d.insert_cols(rnd(d.col_count() as u64 + 1) as u32, 1);
                }
                9 if d.col_count() > 2 => {
                    let _ = d.delete_cols(rnd(d.col_count() as u64) as u32, 1);
                }
                10 if d.col_count() > 2 => {
                    let n = d.col_count() as u64;
                    let _ = d.move_cols(rnd(n) as u32, 1, rnd(n) as u32);
                }
                _ => {}
            }
        }
    }

    fn check(with_cols: bool, rounds: usize) {
        let mut x: u64 = if with_cols {
            0xABCD_1234_5678_9EF1
        } else {
            0x1234_5678_9ABC_DEF1
        };
        let mut rnd = move |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        for round in 0..rounds {
            let mut d = doc(300);
            let pre = rnd(8);
            random_steps(&mut d, &mut rnd, pre, with_cols, 1000 + round);
            let (mark, wm) = (d.step_mark(), d.next_row_id());
            let then = snap(&d);
            let steps = 1 + rnd(40);
            random_steps(&mut d, &mut rnd, steps, with_cols, round);
            let now = snap(&d);
            let (want, want_cells) = brute(&then, &now);
            let plan = d.since_plan(mark, wm, &Progress::new()).unwrap();
            let t = &plan.totals;
            assert_eq!(
                (
                    t.rows_changed,
                    t.cells_changed,
                    t.rows_added,
                    t.rows_removed,
                    t.cols_added,
                    t.cols_removed,
                    t.cols_moved
                ),
                (
                    want.rows_changed,
                    want.cells_changed,
                    want.rows_added,
                    want.rows_removed,
                    want.cols_added,
                    want.cols_removed,
                    want.cols_moved
                ),
                "round {round}, {steps} steps, columns {with_cols}"
            );
            // The pages, put together, are exactly the changed cells (by id and physical column).
            let mut got = Vec::new();
            let mut offset = 0;
            loop {
                let page = d.since_page(&plan, mark, wm, offset, 7).unwrap();
                if page.is_empty() {
                    break;
                }
                offset += page.len() as u64;
                got.extend(page);
            }
            assert_eq!(
                got.len() as u64,
                t.cells_changed,
                "round {round}: every cell is on some page"
            );
            let phys_now = d.logical_to_physical();
            let mut got_cells: Vec<(u32, u32, String, String)> = got
                .iter()
                .map(|c| (c.id, phys_now[c.col as usize], c.old.clone(), c.new.clone()))
                .collect();
            got_cells.sort();
            let mut want_cells = want_cells;
            want_cells.sort();
            assert_eq!(got_cells, want_cells, "round {round}: the listed cells");
            // Each cell's row is where the row is now.
            for c in &got {
                if let Some(r) = c.row {
                    assert_eq!(d.id_at(r), Some(c.id));
                }
            }
        }
    }

    #[test]
    fn since_equals_brute_force_over_rows() {
        check(false, 40);
    }

    #[test]
    fn since_equals_brute_force_with_column_steps() {
        check(true, 60);
    }

    #[test]
    fn refuses_plainly_for_marks_ahead_or_gone() {
        let mut d = doc(20);
        let start = d.step_mark();
        d.set_cell(1, 0, "x".into()).unwrap();
        let one = d.step_mark();
        d.set_cell(2, 0, "y".into()).unwrap();
        // Behind the table: fine.
        assert!(d.since_plan(start, 0, &Progress::new()).is_ok());
        // Ahead of the table (undone): refused, saying why.
        assert!(d.undo());
        assert!(d.undo());
        let e = d
            .since_plan(one, 0, &Progress::new())
            .unwrap_err()
            .to_string();
        assert!(e.contains("ahead of the table"), "{e}");
        // Replaced by a newer change: gone.
        d.set_cell(3, 0, "z".into()).unwrap();
        let e = d
            .since_plan(one, 0, &Progress::new())
            .unwrap_err()
            .to_string();
        assert!(e.contains("no longer be reached"), "{e}");
    }

    /// B4 part 2, the case the review asked for: Split into rows makes rows; with the mark and the watermark taken
    /// inside the change they are added once, and the row they came from is a changed row (its cell now holds one
    /// part), matched by id.
    #[test]
    fn split_into_rows_adds_its_rows_once_and_changes_the_source() {
        let mut d = Document::from_bytes(
            b"name;tags\nann;a|b|c\nbob;x\ncat;p|q\n".to_vec(),
            OpenOptions {
                dialect: Some(crate::dialect::Dialect::new(b';', Some(b'"')).unwrap()),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let spec = crate::reshape::SplitRows {
            col: 1,
            separator: "|".into(),
            trim: true,
            skip_empty: true,
            keep_first: true,
        };
        crate::document::capture_mark_start(d.uid());
        let plan = d.plan_split_rows(&spec, &Progress::new()).unwrap();
        d.apply_rows(plan).unwrap();
        let (mark, wm) = crate::document::capture_mark_take().expect("the split made a step");
        let then_rows = 4;
        assert_eq!(
            d.row_count(),
            then_rows + 3,
            "a|b|c and p|q make three more rows"
        );
        let plan = d.since_plan(mark, wm, &Progress::new()).unwrap();
        let t = &plan.totals;
        assert_eq!(t.rows_added, 3, "added once, not twice");
        assert_eq!((t.rows_removed, t.cols_added, t.cols_removed), (0, 0, 0));
        // The two source rows now hold their first part: two changed cells.
        assert_eq!((t.rows_changed, t.cells_changed), (2, 2));
        let cells = d.since_page(&plan, mark, wm, 0, 10).unwrap();
        let mut seen: Vec<(String, String)> = cells
            .iter()
            .map(|c| (c.old.clone(), c.new.clone()))
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            [
                ("a|b|c".to_string(), "a".to_string()),
                ("p|q".to_string(), "p".to_string())
            ]
        );
    }

    /// The watermark is taken with the mark, inside the change: a row a bulk change adds is added, once.
    #[test]
    fn rows_a_change_adds_count_once() {
        let mut d = doc(20);
        crate::document::capture_mark_start(d.uid());
        d.insert_rows(
            5,
            vec![vec!["n".into(), "n".into(), "n".into(), "n".into()]],
        )
        .unwrap();
        let (mark, wm) = crate::document::capture_mark_take().expect("a change was made");
        let plan = d.since_plan(mark, wm, &Progress::new()).unwrap();
        assert_eq!(
            (
                plan.totals.rows_added,
                plan.totals.rows_removed,
                plan.totals.cells_changed
            ),
            (1, 0, 0)
        );
    }
}
