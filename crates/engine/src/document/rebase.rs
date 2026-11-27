//! Undo, redo and checkpoints that survive a save.
//!
//! A save writes the rows in their order, so row `pos` of the new file is the row with id `ids()[pos]` now. Every
//! step of the history refers to the OLD file: row ids, `None` ("the row's own file bytes") and `Patch` (file bytes
//! plus some cells). The history is moved onto the saved file in two halves:
//!
//! 1. [`Document::prepare_rebase`] (read-only, beside the write of the temp file, while the old file is still
//!    there): decides which steps are kept, gives every row id a new id (a row that is there now gets its new
//!    position; a row only the history knows gets a fresh id past the new file's rows) and reads every `None` and
//!    `Patch` row out of the old bytes, in parallel, in the new file's column order. Nothing is changed, so a save
//!    that then fails or is cancelled just drops it.
//! 2. [`Document::apply_rebase`] (after the replace succeeded and the document was reset onto the saved file):
//!    walks the old steps by value. `Full` rows are moved (not copied), the prepared rows fill the `None`/`Patch`
//!    slots, ids go through the map, `SetOrder(None)` ("file order") is written out, hidden-row and filter sets are
//!    mapped id by id.
//!
//! What is kept: steps that change the columns, the encoding or the file width are not followed, so undo is cut at
//! the newest such step and redo at the one nearest the present. Then, newest first, the steps whose rebased size
//! (worked out as `Op::cost` would count it, never below) fits the history's memory budget are kept and the older
//! ones let go. If nothing fits, or a step cannot be followed exactly, the history is cleared as before.

use super::*;
use std::collections::VecDeque;

/// What a save did with the undo history (for the status line and the checkpoints panel).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct HistoryKept {
    /// Undo and redo steps still there after the save.
    pub undo: usize,
    pub redo: usize,
    /// Undo steps let go at a column, encoding or width step (the oldest ones), and redo steps let go the same way
    /// (the ones furthest from the present).
    pub undo_cut_columns: usize,
    pub redo_cut_columns: usize,
    /// Steps let go because the history would not fit in its memory budget.
    pub undo_cut_memory: usize,
    pub redo_cut_memory: usize,
    /// Nothing could be kept (it did not fit, or could not be followed): the history is empty, as before.
    pub cleared: bool,
}

impl HistoryKept {
    /// Undo steps let go from the old end.
    pub fn undo_cut(&self) -> usize {
        self.undo_cut_columns + self.undo_cut_memory
    }
    /// Redo steps let go from the far end.
    pub fn redo_cut(&self) -> usize {
        self.redo_cut_columns + self.redo_cut_memory
    }
}

#[cfg(test)]
thread_local! {
    static REBASE_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Turn the rebase off on this thread (tests: today's save, for timings and twins).
#[cfg(test)]
pub(crate) fn set_rebase(on: bool) {
    REBASE_OFF.with(|c| c.set(!on));
}

fn rebase_off() -> bool {
    #[cfg(test)]
    {
        REBASE_OFF.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        false
    }
}

fn structural(op: &Op) -> bool {
    match op {
        Op::ColInsert { .. }
        | Op::ColRemove { .. }
        | Op::MoveCols { .. }
        | Op::SetMap(_)
        | Op::SetEncoding(_)
        | Op::SetWidth(_) => true,
        Op::Batch(v) => v.iter().any(structural),
        _ => false,
    }
}

/// The history moved onto the file being saved: everything that needs the old file, worked out before it goes.
pub(crate) struct Rebase {
    /// Old undo steps from here on are kept (older ones are let go).
    undo_from: usize,
    /// Old redo steps from here on are kept (the ones before are further from the present).
    redo_from: usize,
    kept: HistoryKept,
    /// What the kept steps will hold, worked out before (never below their real size).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) estimate: usize,
    /// New id of each old id (`u32::MAX`: not seen), indexed by old id.
    map: Vec<u32>,
    next: u32,
    /// Physical column of each new column (the column map now).
    logical: Option<Vec<u32>>,
    /// Every `None`/`Patch` row of the kept steps, read in the new column order, in walk order.
    rows: Vec<Vec<String>>,
    /// The order `SetOrder(None)` stands for (the old file's ids, mapped).
    file_order: Option<Vec<u32>>,
    undo_ids: VecDeque<u64>,
    redo_ids: Vec<u64>,
    /// The mark before the oldest kept step.
    base_mark: u64,
    /// The view now, over the new ids (also the rows only the history knows): passing bits when filtered, hidden bits.
    view: Option<RebasedView>,
}

/// Passing bits (when a filter is on) and hidden bits, over the new ids.
type RebasedView = (Option<Vec<u64>>, Option<Vec<u64>>);

/// Calls `f` on every row state of `op`, in walk order (the same order [`Moving::op`] takes them).
fn each_state<'a>(op: &'a Op, f: &mut impl FnMut(u32, &'a Option<Edit>)) {
    match op {
        Op::Restore(s) | Op::Insert { rows: s, .. } => s.iter().for_each(|(id, st)| f(*id, st)),
        Op::Batch(v) => v.iter().for_each(|o| each_state(o, f)),
        _ => {}
    }
}

fn has_file_order(op: &Op) -> bool {
    match op {
        Op::SetOrder(None) => true,
        Op::Batch(v) => v.iter().any(has_file_order),
        _ => false,
    }
}

impl Document {
    /// UTF-8 bytes per byte of the file, at most (for the memory estimate).
    fn text_factor(&self) -> usize {
        if self.format.encoding == Encoding::Utf8 {
            1
        } else {
            3
        }
    }

    /// What one row state costs once rebased (as `Edit::cost` + `Op::cost` count it; never below).
    fn rebased_row_cost(&self, id: u32, st: &Option<Edit>, cells: usize) -> usize {
        let text = |id: u32| {
            if (id as u64) < self.index.rows() {
                self.raw_row(id).0.len() * self.text_factor()
            } else {
                0
            }
        };
        32 + match st {
            Some(Edit::Full(v)) => 24 * v.len().max(cells) + v.iter().map(String::len).sum::<usize>(),
            Some(Edit::Patch { width, cells: p }) => {
                24 * (*width as usize).max(cells) + text(id) + p.iter().map(|(_, s)| s.len()).sum::<usize>()
            }
            None => 24 * cells + text(id),
        }
    }

    /// What step `op` holds once rebased (never below its real `Op::cost` afterwards).
    fn rebased_cost(&self, op: &Op, cells: usize, bit_words: usize) -> usize {
        match op {
            Op::Restore(s) | Op::Insert { rows: s, .. } => {
                32 + s.iter().map(|(id, st)| self.rebased_row_cost(*id, st, cells)).sum::<usize>()
            }
            Op::SetOrder(Some(o)) => 32 + o.len() * 4,
            Op::SetOrder(None) => 32 + self.index.rows() as usize * 4,
            Op::SetHidden(h) => 32 + h.as_ref().map_or(0, |_| bit_words * 8),
            Op::SetFilter(f) => 32 + f.as_ref().map_or(0, |f| 64 + bit_words * 8 + f.spec.columns.len() * 128),
            Op::Batch(v) => 32 + v.iter().map(|o| self.rebased_cost(o, cells, bit_words)).sum::<usize>(),
            other => other.cost(),
        }
    }

    /// The history rebased onto the rows as they are about to be saved (`None`: nothing to keep, or it cannot be
    /// followed exactly, and the history is cleared as before; `last_history` says which). Read-only: a save that
    /// then fails changes nothing.
    pub(crate) fn prepare_rebase(&self) -> Result<Rebase, HistoryKept> {
        let empty = HistoryKept {
            cleared: !(self.undo.is_empty() && self.redo.is_empty()),
            ..Default::default()
        };
        if rebase_off() || (self.undo.is_empty() && self.redo.is_empty()) {
            return Err(empty);
        }
        let mut kept = HistoryKept::default();
        // 1. Cut at the column, encoding and width steps.
        let mut undo_from = self.undo.iter().rposition(structural).map_or(0, |i| i + 1);
        let mut redo_from = self.redo.iter().rposition(structural).map_or(0, |i| i + 1);
        kept.undo_cut_columns = undo_from;
        kept.redo_cut_columns = redo_from;
        // 2. Newest first, keep what fits the memory budget.
        let cells = self.col_count().max(self.physical_width()) as usize;
        let bit_words = (self.next_id as usize).div_ceil(64).max(1);
        let budget = self.undo_budget;
        let mut left = budget;
        let mut fits = |op: &Op| {
            let c = self.rebased_cost(op, cells, bit_words);
            match left.checked_sub(c) {
                Some(l) => {
                    left = l;
                    true
                }
                None => false,
            }
        };
        let mut u = self.undo.len();
        while u > undo_from && fits(&self.undo[u - 1]) {
            u -= 1;
        }
        let mut r = self.redo.len();
        // Redo is a stack: the last step is the one nearest the present.
        while r > redo_from && fits(&self.redo[r - 1]) {
            r -= 1;
        }
        kept.undo_cut_memory = u - undo_from;
        kept.redo_cut_memory = r - redo_from;
        undo_from = u;
        redo_from = r;
        let (undo, redo) = (&self.undo[undo_from..], &self.redo[redo_from..]);
        if undo.is_empty() && redo.is_empty() {
            kept.cleared = true;
            return Err(kept);
        }
        kept.undo = undo.len();
        kept.redo = redo.len();
        let estimate = budget - left;
        // 3. New ids: the rows there now first (their new positions), then the rows only the history knows.
        let mut map = vec![u32::MAX; self.next_id as usize];
        let mut next = 0u32;
        let mut see = |old: u32, map: &mut Vec<u32>| {
            let i = old as usize;
            if i >= map.len() {
                map.resize(i + 1, u32::MAX);
            }
            if map[i] == u32::MAX {
                map[i] = next;
                next += 1;
            }
        };
        for id in self.ids() {
            see(id, &mut map);
        }
        let file_order = undo.iter().chain(redo).any(has_file_order);
        if file_order {
            for id in 0..self.index.rows() as u32 {
                see(id, &mut map);
            }
        }
        fn more(op: &Op, see: &mut impl FnMut(u32)) {
            match op {
                Op::Restore(s) | Op::Insert { rows: s, .. } => s.iter().for_each(|(id, _)| see(*id)),
                Op::SetOrder(Some(o)) => o.iter().for_each(|&id| see(id)),
                Op::Batch(v) => v.iter().for_each(|o| more(o, see)),
                _ => {}
            }
        }
        for op in undo.iter().chain(redo) {
            more(op, &mut |id| see(id, &mut map));
        }
        // 4. The `None`/`Patch` rows, read from the old bytes in parallel.
        let mut slots: Vec<(u32, &Option<Edit>)> = Vec::new();
        for op in undo.iter().chain(redo) {
            each_state(op, &mut |id, st| {
                if !matches!(st, Some(Edit::Full(_))) {
                    slots.push((id, st));
                }
            });
        }
        let logical = self.cols.clone();
        let rows: Result<Vec<Vec<String>>, Error> = slots
            .par_iter()
            .with_min_len(256)
            .map(|&(id, st)| {
                let phys = match st {
                    Some(Edit::Patch { width, cells }) => self.patched(id, *width, cells),
                    Some(Edit::Full(_)) => unreachable!("full rows are moved, not read"),
                    None => {
                        if id as u64 >= self.index.rows() {
                            return Err(Error::Invalid("rebase: a made row has no state".into()));
                        }
                        let (b, m) = self.raw_row(id);
                        let mut v = self.original_fields(id, b, m)?;
                        let w = self.home_width(v.len());
                        v.truncate(w);
                        v
                    }
                };
                Ok(match &logical {
                    Some(lm) => {
                        let mut phys = phys;
                        lm.iter()
                            .map(|&p| phys.get_mut(p as usize).map(std::mem::take).unwrap_or_default())
                            .collect()
                    }
                    None => phys,
                })
            })
            .collect();
        let Ok(rows) = rows else {
            return Err(empty);
        };
        let file_order = file_order.then(|| {
            (0..self.index.rows() as usize)
                .into_par_iter()
                .map(|id| map[id])
                .collect()
        });
        let bits = |set: &dyn Fn(u32) -> bool| -> Vec<u64> {
            let mut bits = vec![0u64; (next as usize).div_ceil(64).max(1)];
            for (old, &new) in map.iter().enumerate() {
                if new != u32::MAX && set(old as u32) {
                    bits[(new >> 6) as usize] |= 1 << (new & 63);
                }
            }
            bits
        };
        let view = self.view.as_ref().map(|v| {
            (
                v.filters().then(|| bits(&|id| v.passes(id))),
                v.hidden.is_some().then(|| bits(&|id| v.is_hidden(id))),
            )
        });
        Ok(Rebase {
            undo_from,
            redo_from,
            kept,
            estimate,
            map,
            next,
            logical,
            rows,
            file_order,
            undo_ids: self.undo_ids.iter().skip(undo_from).copied().collect(),
            redo_ids: self.redo_ids[redo_from..].to_vec(),
            // The state before the oldest kept step: the start, or the state the newest step let go left.
            base_mark: match undo_from {
                0 => self.base_mark,
                n => self.undo_ids[n - 1],
            },
            view,
        })
    }

    /// Put the rebased history in place, after `reset_to` with the saved file. `undo` and `redo` are the old steps
    /// (taken out before the reset); what is not kept is returned to be freed.
    fn apply_rebase(&mut self, r: Rebase, mut undo: Vec<Op>, mut redo: Vec<Op>) -> Vec<Op> {
        let Rebase {
            undo_from,
            redo_from,
            kept,
            map,
            next,
            logical,
            rows,
            file_order,
            undo_ids,
            redo_ids,
            base_mark,
            view,
            ..
        } = r;
        debug_assert!(next as u64 >= self.index.rows());
        let mut gone: Vec<Op> = undo.drain(..undo_from).collect();
        let keep_redo = redo.split_off(redo_from);
        gone.append(&mut redo);
        let mut mv = Moving {
            map,
            next,
            logical,
            rows: rows.into_iter(),
            file_order,
        };
        let undo: Vec<Op> = undo.into_iter().map(|o| mv.op(o)).collect();
        let redo: Vec<Op> = keep_redo.into_iter().map(|o| mv.op(o)).collect();
        debug_assert!(mv.rows.next().is_none(), "every prepared row was used");
        self.undo_bytes = undo.iter().map(Op::cost).sum();
        self.redo_bytes = redo.iter().map(Op::cost).sum();
        self.undo = undo;
        self.redo = redo;
        self.undo_ids = undo_ids;
        self.redo_ids = redo_ids;
        self.base_mark = base_mark;
        self.next_id = next;
        self.clean_depth = Some(self.undo.len());
        self.last_history = Some(kept);
        // The view `reset_to` rebuilt covers the saved rows only; rows only the history knows keep how they showed.
        if let (Some((passing, hidden)), Some(old)) = (view, self.view.take()) {
            let mut v = match passing {
                Some(bits) if old.filters() => View::new(old.spec, bits, self.next_id),
                _ => View::all(),
            };
            v.hidden = hidden.map(Arc::new);
            if v.filters() || v.hidden.is_some() {
                self.view = Some(v);
            }
        }
        gone
    }

    /// Any undo or redo steps.
    pub(crate) fn has_history(&self) -> bool {
        !(self.undo.is_empty() && self.redo.is_empty())
    }

    /// `reset_to` the saved file, keeping the history `prepare_rebase` moved onto it (or clearing it as before).
    pub(crate) fn reset_keeping(
        &mut self,
        source: Source,
        start: Start,
        format: Format,
        index: RowIndex,
        rebase: Result<Rebase, HistoryKept>,
    ) {
        match rebase {
            Ok(r) => {
                let undo = std::mem::take(&mut self.undo);
                let redo = std::mem::take(&mut self.redo);
                self.reset_to(source, start, format, index);
                let gone = self.apply_rebase(r, undo, redo);
                // Freeing many let-go rows takes a while: big leftovers go on a background thread, as in `reset_to`.
                if gone.iter().map(Op::cost).sum::<usize>() > 64 << 20 {
                    let _ = std::thread::Builder::new()
                        .name("free-edits".into())
                        .spawn(move || drop(gone));
                }
            }
            Err(kept) => {
                self.reset_to(source, start, format, index);
                self.last_history = kept.cleared.then_some(kept);
            }
        }
    }

    /// Where rows `ids` are in the order now, which is their id once this order is saved (rows not there are left
    /// out). For a list of rows kept by id beside the document ("Show changed rows") across a save.
    pub fn saved_ids_of(&self, ids: &[u32]) -> Vec<u32> {
        let mut pos = vec![u32::MAX; self.next_id as usize];
        for (p, id) in self.ids().enumerate() {
            pos[id as usize] = p as u32;
        }
        ids.iter()
            .filter_map(|&id| pos.get(id as usize).copied().filter(|&p| p != u32::MAX))
            .collect()
    }

    /// What the last save did with the undo history (`None`: there was none, or nothing was saved yet).
    pub fn last_history(&self) -> Option<HistoryKept> {
        self.last_history
    }
}

/// The old steps taken by value onto the new ids (see [`Document::apply_rebase`]).
struct Moving {
    map: Vec<u32>,
    next: u32,
    logical: Option<Vec<u32>>,
    rows: std::vec::IntoIter<Vec<String>>,
    file_order: Option<Vec<u32>>,
}

impl Moving {
    fn id(&self, old: u32) -> u32 {
        self.map[old as usize]
    }

    fn state(&mut self, st: Option<Edit>) -> Option<Edit> {
        Some(Edit::Full(match st {
            Some(Edit::Full(mut v)) => match &self.logical {
                Some(lm) => lm
                    .iter()
                    .map(|&p| v.get_mut(p as usize).map(std::mem::take).unwrap_or_default())
                    .collect(),
                None => v,
            },
            _ => self.rows.next().expect("a prepared row for every None/Patch state"),
        }))
    }

    fn states(&mut self, s: Vec<(u32, Option<Edit>)>) -> Vec<(u32, Option<Edit>)> {
        s.into_iter().map(|(id, st)| (self.id(id), self.state(st))).collect()
    }

    fn bits(&self, set: impl Fn(u32) -> bool) -> Vec<u64> {
        let mut bits = vec![0u64; (self.next as usize).div_ceil(64).max(1)];
        for (old, &new) in self.map.iter().enumerate() {
            if new != u32::MAX && set(old as u32) {
                bits[(new >> 6) as usize] |= 1 << (new & 63);
            }
        }
        bits
    }

    fn op(&mut self, op: Op) -> Op {
        match op {
            Op::Restore(s) => Op::Restore(self.states(s)),
            Op::Insert { at, rows } => Op::Insert {
                at,
                rows: self.states(rows),
            },
            Op::SetOrder(Some(mut o)) => {
                o.par_iter_mut().for_each(|id| *id = self.map[*id as usize]);
                Op::SetOrder(Some(o))
            }
            Op::SetOrder(None) => Op::SetOrder(Some(
                self.file_order.clone().expect("file order prepared"),
            )),
            Op::SetHidden(h) => Op::SetHidden(h.map(|h| {
                let has = |id: u32| h.get((id >> 6) as usize).is_some_and(|w| (w >> (id & 63)) & 1 == 1);
                Arc::new(self.bits(has))
            })),
            Op::SetFilter(f) => Op::SetFilter(f.map(|f| {
                let passes = |id: u32| {
                    id >= f.base || f.bits.get((id >> 6) as usize).is_some_and(|w| (w >> (id & 63)) & 1 == 1)
                };
                let bits = self.bits(passes);
                let logical = &self.logical;
                let spec = f.spec.map_cols(|p| match logical {
                    Some(m) => m.iter().position(|&x| x == p).map(|i| i as u32),
                    None => Some(p),
                });
                Box::new(FilterPart {
                    spec,
                    bits,
                    base: self.next,
                })
            })),
            Op::Batch(v) => Op::Batch(v.into_iter().map(|o| self.op(o)).collect()),
            // Row moves and removals hold positions only.
            other @ (Op::Remove { .. } | Op::MoveRows { .. }) => other,
            Op::ColInsert { .. }
            | Op::ColRemove { .. }
            | Op::MoveCols { .. }
            | Op::SetMap(_)
            | Op::SetEncoding(_)
            | Op::SetWidth(_) => unreachable!("the history is cut at column, encoding and width steps"),
        }
    }
}

#[cfg(test)]
#[path = "rebase_tests.rs"]
mod tests;
