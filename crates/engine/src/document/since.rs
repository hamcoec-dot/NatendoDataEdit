//! "What changed since this checkpoint" (B4, part 2): the cells that differ now from the state a checkpoint names,
//! worked out from the inverse steps the undo stack already holds. Nothing is rewound and no row is copied while a
//! checkpoint sits.
//!
//! The state a row had BEFORE the oldest step after the mark is its state at the mark, and the stack holds it (as a
//! reference here). Rows are matched by id, so sorting and deleting cannot confuse the comparison, and columns by
//! their physical id, so inserting, deleting and moving columns cannot either: the column map at the mark is rebuilt by
//! applying the stored inverse column steps, newest first, to the map now. A cell is compared only in a column that
//! exists at both times.
//!
//! The list of what changed has four parts, paged as one: the changed cells; the rows taken out (Was: their values at
//! the mark); the rows made since (Now: their values); and, for every column taken out, its cell in each row that is
//! there at both times. Columns added, taken out and moved are also named in the totals.

use super::*;
use serde::{Deserialize, Serialize};

/// Rows compared per batch (and per unit of progress).
const CHUNK: usize = 16_384;

/// What a row taken out, a row made since, and a cell of a column taken out show on the other side.
pub const TAKEN_OUT: &str = "(taken out)";
pub const NEW_ROW: &str = "(new)";
pub const COLUMN_TAKEN_OUT: &str = "(column taken out)";
/// Between the values of a whole row shown as one entry.
const BETWEEN: &str = " | ";

/// A column that was there at the mark and is not now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedColumn {
    /// Its position at the mark (0-based).
    pub at: u32,
    /// Its title at the mark (empty when the first row does not hold titles).
    pub title: String,
}

/// Counts of what changed since a mark, with the columns concerned.
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
    /// The columns added, by their position now.
    pub added_cols: Vec<u32>,
    /// The columns taken out.
    pub removed_cols: Vec<RemovedColumn>,
    /// The columns in another place among those in both, by their position now.
    pub moved_cols: Vec<u32>,
    /// Lines of the whole list: what the pages go through.
    pub entries: u64,
}

/// What a line of the list is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinceKind {
    /// A cell with another value.
    Cell,
    /// A whole row taken out.
    RowRemoved,
    /// A whole row made since.
    RowAdded,
    /// A cell of a column taken out.
    ColRemoved,
}

/// One line of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinceCell {
    pub kind: SinceKind,
    /// The row's id (it follows the row through sorts and deletes).
    pub id: u32,
    /// Its position now in the whole table (filter or not); none for a row taken out.
    pub row: Option<u64>,
    /// Its column now (logical), for a changed cell.
    pub col: Option<u32>,
    /// For a cell of a column taken out: which one (an index into `removed_cols`).
    pub removed_col: Option<u32>,
    pub old: String,
    pub new: String,
}

/// What a first pass found, to page through the list afterwards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SincePlan {
    pub totals: SinceTotals,
    /// Sorted ids of the rows compared cell by cell.
    ids: Vec<u32>,
    /// Changed cells in each batch of `CHUNK` ids (their running total decides which batch holds a page).
    chunk_cells: Vec<u64>,
    /// Sorted ids of the rows taken out and of the rows made since.
    removed: Vec<u32>,
    added: Vec<u32>,
    /// Sorted ids of the rows whose cells of the columns taken out are listed (empty when no column was).
    kept: Vec<u32>,
    /// The physical columns taken out, as `totals.removed_cols`.
    removed_phys: Vec<u32>,
}

/// What is known about the columns when the steps touched them.
struct Cols {
    /// Physical columns that exist at both times; `None` when no column step was taken (every column does).
    common: Option<HashSet<u32>>,
    /// The column map (logical to physical) at the mark and now.
    then_map: Vec<u32>,
    now_map: Vec<u32>,
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
    /// Rows taken out and made since (sorted ids), and the first row of the table at the mark.
    removed: Vec<u32>,
    added: Vec<u32>,
    then_first: Option<u32>,
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
                then_map: Vec::new(),
                now_map: Vec::new(),
                now_pos: HashMap::new(),
            },
            removed: Vec::new(),
            added: Vec::new(),
            then_first: self.doc_id_at(0),
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
            let mut both = vec![0u64; bits];
            for w in 0..bits {
                let (mut gone, mut made) = (in_then[w] & !in_now[w], in_now[w] & !in_then[w]);
                while gone != 0 {
                    r.removed.push((w as u32) << 6 | gone.trailing_zeros());
                    gone &= gone - 1;
                }
                while made != 0 {
                    r.added.push((w as u32) << 6 | made.trailing_zeros());
                    made &= made - 1;
                }
                both[w] = in_then[w] & in_now[w];
            }
            r.totals.rows_removed = r.removed.len() as u64;
            r.totals.rows_added = r.added.len() as u64;
            r.then_first = then.first().copied();
            let has = |set: &[u64], id: &u32| set[(*id >> 6) as usize] & (1 << (*id & 63)) != 0;
            let a: Vec<u32> = then.iter().copied().filter(|id| has(&both, id)).collect();
            let b: Vec<u32> = now.iter().copied().filter(|id| has(&both, id)).collect();
            r.totals.reordered = a != b;
            r.both = Some(both);
        }
        let _ = watermark;
        // Columns: the map at the mark is the map now with the stored inverse steps applied, newest first.
        r.cols.now_map = self.logical_to_physical();
        r.cols.then_map = r.cols.now_map.clone();
        r.cols.now_pos = r
            .cols
            .now_map
            .iter()
            .enumerate()
            .map(|(i, p)| (*p, i as u32))
            .collect();
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
            let order_then: Vec<u32> = then_map
                .iter()
                .copied()
                .filter(|p| common.contains(p))
                .collect();
            let order_now: Vec<u32> = now.iter().copied().filter(|p| common.contains(p)).collect();
            r.totals.added_cols = (0u32..)
                .zip(now.iter())
                .filter(|(_, p)| !then_set.contains(p))
                .map(|(i, _)| i)
                .collect();
            r.totals.removed_cols = (0u32..)
                .zip(then_map.iter())
                .filter(|(_, p)| !now_set.contains(p))
                .map(|(at, _)| RemovedColumn {
                    at,
                    title: String::new(),
                })
                .collect();
            r.totals.moved_cols = order_then
                .iter()
                .zip(order_now.iter())
                .filter(|(a, b)| a != b)
                .map(|(_, b)| r.cols.now_pos[b])
                .collect();
            r.totals.cols_added = r.totals.added_cols.len() as u32;
            r.totals.cols_removed = r.totals.removed_cols.len() as u32;
            r.totals.cols_moved = !r.totals.moved_cols.is_empty();
            r.cols.common = Some(common);
            r.cols.then_map = then_map;
        }
        Ok(r)
    }

    /// The physical values of row `id` in a stored state; `None`: no step since the mark touched it, so it is as it is
    /// now.
    fn since_values(&self, id: u32, state: Option<&Option<Edit>>) -> Vec<String> {
        match state {
            Some(None) => {
                let (bytes, mode) = self.raw_row(id);
                self.original_fields(id, bytes, mode).unwrap_or_default()
            }
            Some(Some(Edit::Full(v))) => v.clone(),
            Some(Some(Edit::Patch { width, cells })) => self.patched(id, *width, cells),
            None => match self.row_source(id) {
                RowSource::Edited(v) => v.into_owned(),
                RowSource::Raw { bytes, mode } => {
                    self.original_fields(id, bytes, mode).unwrap_or_default()
                }
            },
        }
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
        let then_vals = self.since_values(id, Some(then));
        let now_vals = self.since_values(id, None);
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

    /// A whole row as one line, in the column order of `map`.
    fn since_line(values: &[String], map: &[u32]) -> String {
        map.iter()
            .map(|&p| values.get(p as usize).map(String::as_str).unwrap_or(""))
            .collect::<Vec<_>>()
            .join(BETWEEN)
    }

    /// First pass: every count, and the sorted ids of the changed rows with the cells in each batch (one parallel
    /// pass, with progress and Cancel). Run when asked, never while a list is drawn.
    pub fn since_plan(
        &self,
        mark: u64,
        watermark: u32,
        header: bool,
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
        // The columns taken out: their titles at the mark, and the rows whose cells of them are listed (every row
        // there at both times; not the title row).
        let removed_phys: Vec<u32> = totals
            .removed_cols
            .iter()
            .map(|c| r.cols.then_map[c.at as usize])
            .collect();
        let mut kept = Vec::new();
        if !removed_phys.is_empty() {
            if let (true, Some(title_row)) = (header, r.then_first) {
                let titles = self.since_values(title_row, r.first.get(&title_row).copied());
                for (c, p) in totals.removed_cols.iter_mut().zip(&removed_phys) {
                    c.title = titles.get(*p as usize).cloned().unwrap_or_default();
                }
            }
            let added: HashSet<u32> = r.added.iter().copied().collect();
            let title_row = if header { r.then_first } else { None };
            kept = match &self.order {
                Some(o) => o.clone(),
                None => (0..self.index.rows() as u32).collect(),
            };
            kept.retain(|id| !added.contains(id) && Some(*id) != title_row);
            kept.sort_unstable();
        }
        totals.entries = totals.cells_changed
            + totals.rows_removed
            + totals.rows_added
            + removed_phys.len() as u64 * kept.len() as u64;
        Ok(SincePlan {
            totals,
            ids,
            chunk_cells: per.into_iter().map(|p| p.1).collect(),
            removed: r.removed,
            added: r.added,
            kept,
            removed_phys,
        })
    }

    /// A page of the list, from a plan made for the same `mark` and `watermark` against the document as it is now:
    /// the changed cells in row id order, then the rows taken out, the rows made since, and the cells of the columns
    /// taken out (column by column).
    pub fn since_page(
        &self,
        plan: &SincePlan,
        mark: u64,
        watermark: u32,
        offset: u64,
        want: usize,
    ) -> Result<Vec<SinceCell>, Error> {
        let r = self.since_read(mark, watermark)?;
        let mut found: Vec<SinceCell> = Vec::new();
        // Changed cells: start in the batch the page starts in.
        if offset < plan.totals.cells_changed {
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
            let mut buf = Vec::new();
            'chunks: for chunk in plan.ids.chunks(CHUNK).skip(start_chunk) {
                for id in chunk {
                    self.since_diff(*id, r.first[id], &r.cols, &mut buf);
                    for (p, old, new) in buf.drain(..) {
                        if skipped < offset {
                            skipped += 1;
                            continue;
                        }
                        if found.len() >= want {
                            break 'chunks;
                        }
                        found.push(SinceCell {
                            kind: SinceKind::Cell,
                            id: *id,
                            row: None,
                            col: Some(r.cols.now_pos.get(&p).copied().unwrap_or(p)),
                            removed_col: None,
                            old,
                            new,
                        });
                    }
                }
            }
        }
        // Where the page stands in the parts after the cells.
        let mut at = offset.saturating_sub(plan.totals.cells_changed);
        let mut part = |len: u64| -> std::ops::Range<u64> {
            let from = at.min(len);
            at = at.saturating_sub(len);
            from..len
        };
        for i in part(plan.removed.len() as u64) {
            if found.len() >= want {
                break;
            }
            let id = plan.removed[i as usize];
            let was = self.since_values(id, r.first.get(&id).copied());
            found.push(SinceCell {
                kind: SinceKind::RowRemoved,
                id,
                row: None,
                col: None,
                removed_col: None,
                old: Self::since_line(&was, &r.cols.then_map),
                new: TAKEN_OUT.to_string(),
            });
        }
        for i in part(plan.added.len() as u64) {
            if found.len() >= want {
                break;
            }
            let id = plan.added[i as usize];
            let now = self.since_values(id, None);
            found.push(SinceCell {
                kind: SinceKind::RowAdded,
                id,
                row: None,
                col: None,
                removed_col: None,
                old: NEW_ROW.to_string(),
                new: Self::since_line(&now, &r.cols.now_map),
            });
        }
        let kept = plan.kept.len() as u64;
        for i in part(plan.removed_phys.len() as u64 * kept) {
            if found.len() >= want {
                break;
            }
            let (c, j) = ((i / kept) as usize, (i % kept) as usize);
            let id = plan.kept[j];
            let was = self.since_values(id, r.first.get(&id).copied());
            found.push(SinceCell {
                kind: SinceKind::ColRemoved,
                id,
                row: None,
                col: None,
                removed_col: Some(c as u32),
                old: was
                    .get(plan.removed_phys[c] as usize)
                    .cloned()
                    .unwrap_or_default(),
                new: COLUMN_TAKEN_OUT.to_string(),
            });
        }
        let wanted: HashSet<u32> = found
            .iter()
            .filter(|f| f.kind != SinceKind::RowRemoved)
            .map(|f| f.id)
            .collect();
        let pos = self.positions_of(&wanted);
        for f in &mut found {
            f.row = pos.get(&f.id).copied();
        }
        Ok(found)
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

    /// The other three parts of the list by brute force, as (kind, id, column position at the mark, was, now).
    fn brute_lists(
        then: &Snap,
        now: &Snap,
        header: bool,
        first: u32,
    ) -> Vec<(u8, u32, u32, String, String)> {
        let line = |v: &[String], map: &[u32]| {
            map.iter()
                .map(|&p| v.get(p as usize).map(String::as_str).unwrap_or(""))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let mut out = Vec::new();
        for (id, v) in &then.rows {
            if !now.rows.contains_key(id) {
                out.push((1, *id, 0, line(v, &then.cols), TAKEN_OUT.to_string()));
            }
        }
        for (id, v) in &now.rows {
            if !then.rows.contains_key(id) {
                out.push((2, *id, 0, NEW_ROW.to_string(), line(v, &now.cols)));
            }
        }
        for (at, p) in then.cols.iter().enumerate() {
            if now.cols.contains(p) {
                continue;
            }
            for (id, v) in &then.rows {
                if now.rows.contains_key(id) && !(header && *id == first) {
                    out.push((
                        3,
                        *id,
                        at as u32,
                        v.get(*p as usize).cloned().unwrap_or_default(),
                        COLUMN_TAKEN_OUT.to_string(),
                    ));
                }
            }
        }
        out.sort();
        out
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
            let header = round % 2 == 0;
            let then_first = d.id_at(0).unwrap();
            let titles = d.physical_values(then_first).unwrap();
            let steps = 1 + rnd(40);
            random_steps(&mut d, &mut rnd, steps, with_cols, round);
            let now = snap(&d);
            let (want, want_cells) = brute(&then, &now);
            let plan = d.since_plan(mark, wm, header, &Progress::new()).unwrap();
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
                t.entries,
                "round {round}: every line is on some page"
            );
            let phys_now = d.logical_to_physical();
            let mut got_cells: Vec<(u32, u32, String, String)> = got
                .iter()
                .filter(|c| c.kind == SinceKind::Cell)
                .map(|c| {
                    (
                        c.id,
                        phys_now[c.col.unwrap() as usize],
                        c.old.clone(),
                        c.new.clone(),
                    )
                })
                .collect();
            got_cells.sort();
            let mut want_cells = want_cells;
            want_cells.sort();
            assert_eq!(got_cells, want_cells, "round {round}: the listed cells");
            // The rows taken out, the rows made, and the cells of the columns taken out.
            let mut got_lists: Vec<(u8, u32, u32, String, String)> = got
                .iter()
                .filter_map(|c| {
                    let (k, at) = match c.kind {
                        SinceKind::Cell => return None,
                        SinceKind::RowRemoved => (1, 0),
                        SinceKind::RowAdded => (2, 0),
                        SinceKind::ColRemoved => {
                            (3, t.removed_cols[c.removed_col.unwrap() as usize].at)
                        }
                    };
                    Some((k, c.id, at, c.old.clone(), c.new.clone()))
                })
                .collect();
            got_lists.sort();
            assert_eq!(
                got_lists,
                brute_lists(&then, &now, header, then_first),
                "round {round}: rows taken out, rows made, columns taken out"
            );
            // The columns concerned are named.
            for c in &t.removed_cols {
                let p = then.cols[c.at as usize] as usize;
                let want = if header {
                    titles.get(p).cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                assert_eq!(c.title, want, "round {round}: the title at the mark");
            }
            let added: Vec<u32> = (0..now.cols.len() as u32)
                .filter(|&i| !then.cols.contains(&now.cols[i as usize]))
                .collect();
            assert_eq!(t.added_cols, added, "round {round}");
            // Each line's row is where the row is now; a row taken out has none.
            for c in &got {
                match c.row {
                    Some(r) => assert_eq!(d.id_at(r), Some(c.id)),
                    None => assert_eq!(c.kind, SinceKind::RowRemoved),
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
        assert!(d.since_plan(start, 0, false, &Progress::new()).is_ok());
        // Ahead of the table (undone): refused, saying why.
        assert!(d.undo());
        assert!(d.undo());
        let e = d
            .since_plan(one, 0, false, &Progress::new())
            .unwrap_err()
            .to_string();
        assert!(e.contains("ahead of the table"), "{e}");
        // Replaced by a newer change: gone.
        d.set_cell(3, 0, "z".into()).unwrap();
        let e = d
            .since_plan(one, 0, false, &Progress::new())
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
        let plan = d.since_plan(mark, wm, true, &Progress::new()).unwrap();
        let t = &plan.totals;
        assert_eq!(t.rows_added, 3, "added once, not twice");
        assert_eq!((t.rows_removed, t.cols_added, t.cols_removed), (0, 0, 0));
        // The two source rows now hold their first part: two changed cells.
        assert_eq!((t.rows_changed, t.cells_changed), (2, 2));
        let cells = d.since_page(&plan, mark, wm, 0, 10).unwrap();
        assert_eq!(
            cells.len(),
            5,
            "two changed cells, then the three rows made"
        );
        let mut seen: Vec<(String, String)> = cells
            .iter()
            .filter(|c| c.kind == SinceKind::Cell)
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
        let plan = d.since_plan(mark, wm, true, &Progress::new()).unwrap();
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
