//! SPIKE (batch 3 item 1, throwaway): keep undo, redo and checkpoints across a save.
//!
//! A save writes the rows in their order, so row `pos` of the new file is the row with id `ids()[pos]` now. Every
//! step of the history refers to the OLD file: row ids, `None` ("the row's own file bytes") and `Patch` (file bytes
//! plus some cells). Before the old file goes (on Windows it is unmapped before the replace), each kept step is
//! copied with:
//! - every row id mapped: a row that is there now gets its new position; a row only the history knows (taken out,
//!   or made and taken out again) gets a fresh id past the new file's rows;
//! - every row state made `Full` (the old bytes read now, while they exist), in the new file's column order;
//! - `SetOrder(None)` ("file order") written out as the old file's ids, mapped;
//! - hidden-row and filter sets mapped id by id (a filter's "rows made later pass" becomes explicit bits).
//!
//! Steps that change the columns, the encoding or the file width are not followed: the history is cut at the newest
//! one (older steps and their checkpoints go, as when the undo budget trims them). The copy is applied only after
//! the replace succeeded; if anything cannot be followed exactly, the history is cleared as before.

use super::*;
use std::collections::VecDeque;

/// The history moved onto the file being saved (made while the old file is still there).
pub(crate) struct Rebase {
    undo: Vec<Op>,
    redo: Vec<Op>,
    undo_ids: VecDeque<u64>,
    redo_ids: Vec<u64>,
    /// The mark before the oldest kept step: kept when nothing was cut.
    base_mark: Option<u64>,
    next_id: u32,
    /// The view now, over the new ids (also the rows only the history knows): passing bits when filtered, hidden bits.
    view: Option<(Option<Vec<u64>>, Option<Vec<u64>>)>,
}

/// What the last save did with the history (for the spike's measurements).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RebaseReport {
    pub kept_undo: usize,
    pub kept_redo: usize,
    pub cut: usize,
    pub rows_materialised: usize,
    pub micros: u64,
}

thread_local! {
    pub(crate) static LAST_REBASE: std::cell::Cell<Option<RebaseReport>> = const { std::cell::Cell::new(None) };
}

thread_local! {
    static REBASE_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Turn the rebase off on this thread (spike timings: today's save).
pub fn set_rebase(on: bool) {
    REBASE_OFF.with(|c| c.set(!on));
}

/// The last save's rebase on this thread (spike measurements).
pub fn last_rebase() -> Option<RebaseReport> {
    LAST_REBASE.with(|c| c.get())
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

struct Ctx<'a> {
    d: &'a Document,
    /// New id of each old id (`u32::MAX`: not seen), indexed by old id.
    map: Vec<u32>,
    next: u32,
    /// Physical column of each new column (the column map now).
    logical: Option<Vec<u32>>,
    rows_materialised: usize,
}

impl Ctx<'_> {
    fn see(&mut self, old: u32) {
        let i = old as usize;
        if i >= self.map.len() {
            self.map.resize(i + 1, u32::MAX);
        }
        if self.map[i] == u32::MAX {
            self.map[i] = self.next;
            self.next += 1;
        }
    }

    fn id(&self, old: u32) -> Result<u32, Error> {
        match self.map.get(old as usize) {
            Some(&n) if n != u32::MAX => Ok(n),
            _ => Err(Error::Invalid(
                "rebase: a row the history names is unknown".into(),
            )),
        }
    }

    fn collect(&mut self, op: &Op) {
        match op {
            Op::Restore(s) | Op::Insert { rows: s, .. } => {
                s.iter().for_each(|(id, _)| self.see(*id))
            }
            Op::SetOrder(Some(o)) => o.iter().for_each(|&id| self.see(id)),
            Op::SetOrder(None) => (0..self.d.index.rows() as u32).for_each(|id| self.see(id)),
            Op::Batch(v) => v.iter().for_each(|o| self.collect(o)),
            _ => {}
        }
    }

    fn state(&mut self, old: u32, st: &Option<Edit>) -> Result<Option<Edit>, Error> {
        let phys = match st {
            Some(Edit::Full(v)) => v.clone(),
            Some(Edit::Patch { width, cells }) => self.d.patched(old, *width, cells),
            None => {
                if old as u64 >= self.d.index.rows() {
                    return Err(Error::Invalid("rebase: a made row has no state".into()));
                }
                let (b, m) = self.d.raw_row(old);
                let mut v = self.d.original_fields(old, b, m)?;
                let w = self.d.home_width(v.len());
                v.truncate(w);
                v
            }
        };
        self.rows_materialised += 1;
        let v = match &self.logical {
            Some(map) => map
                .iter()
                .map(|&p| phys.get(p as usize).cloned().unwrap_or_default())
                .collect(),
            None => phys,
        };
        Ok(Some(Edit::Full(v)))
    }

    fn states(&mut self, s: &[(u32, Option<Edit>)]) -> Result<Vec<(u32, Option<Edit>)>, Error> {
        s.iter()
            .map(|(id, st)| Ok((self.id(*id)?, self.state(*id, st)?)))
            .collect()
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

    fn op(&mut self, op: &Op) -> Result<Op, Error> {
        Ok(match op {
            Op::Restore(s) => Op::Restore(self.states(s)?),
            Op::Insert { at, rows } => Op::Insert {
                at: *at,
                rows: self.states(rows)?,
            },
            Op::SetOrder(o) => {
                let old: Vec<u32> = match o {
                    Some(o) => o.clone(),
                    None => (0..self.d.index.rows() as u32).collect(),
                };
                Op::SetOrder(Some(
                    old.into_iter()
                        .map(|id| self.id(id))
                        .collect::<Result<_, _>>()?,
                ))
            }
            Op::Remove { at, count } => Op::Remove {
                at: *at,
                count: *count,
            },
            Op::MoveRows { at, count, to } => Op::MoveRows {
                at: *at,
                count: *count,
                to: *to,
            },
            Op::SetHidden(h) => Op::SetHidden(h.as_ref().map(|h| {
                let has = |id: u32| {
                    h.get((id >> 6) as usize)
                        .is_some_and(|w| (w >> (id & 63)) & 1 == 1)
                };
                Arc::new(self.bits(has))
            })),
            Op::SetFilter(f) => Op::SetFilter(match f {
                None => None,
                Some(f) => {
                    let passes = |id: u32| {
                        id >= f.base
                            || (f
                                .bits
                                .get((id >> 6) as usize)
                                .is_some_and(|w| (w >> (id & 63)) & 1 == 1))
                    };
                    let bits = self.bits(passes);
                    let spec = f.spec.map_cols(|p| match &self.logical {
                        Some(m) => m.iter().position(|&x| x == p).map(|i| i as u32),
                        None => Some(p),
                    });
                    Some(Box::new(FilterPart {
                        spec,
                        bits,
                        base: self.next,
                    }))
                }
            }),
            Op::Batch(v) => Op::Batch(v.iter().map(|o| self.op(o)).collect::<Result<_, _>>()?),
            _ => return Err(Error::Invalid("rebase: a column step was not cut".into())),
        })
    }
}

impl Document {
    /// The history rebased onto the rows as they are about to be saved (`None`: nothing to keep, or it cannot be
    /// followed exactly, and the history is cleared as before). Read-only: a save that then fails changes nothing.
    pub(crate) fn prepare_rebase(&self) -> Option<Rebase> {
        if REBASE_OFF.with(|c| c.get()) {
            return None;
        }
        let t = std::time::Instant::now();
        let cut_at = self.undo.iter().rposition(structural).map_or(0, |i| i + 1);
        let redo_from = self.redo.iter().rposition(structural).map_or(0, |i| i + 1);
        let (undo, redo) = (&self.undo[cut_at..], &self.redo[redo_from..]);
        if undo.is_empty() && redo.is_empty() {
            return None;
        }
        let mut cx = Ctx {
            d: self,
            map: vec![u32::MAX; self.next_id as usize],
            next: 0,
            logical: self.cols.clone(),
            rows_materialised: 0,
        };
        for id in self.ids() {
            cx.see(id);
        }
        for op in undo.iter().chain(redo) {
            cx.collect(op);
        }
        let undo2: Result<Vec<Op>, Error> = undo.iter().map(|o| cx.op(o)).collect();
        let redo2: Result<Vec<Op>, Error> = redo.iter().map(|o| cx.op(o)).collect();
        let (Ok(undo2), Ok(redo2)) = (undo2, redo2) else {
            return None;
        };
        let view = self.view.as_ref().map(|v| {
            (
                v.filters().then(|| cx.bits(|id| v.passes(id))),
                v.hidden.is_some().then(|| cx.bits(|id| v.is_hidden(id))),
            )
        });
        LAST_REBASE.with(|c| {
            c.set(Some(RebaseReport {
                kept_undo: undo2.len(),
                kept_redo: redo2.len(),
                cut: cut_at,
                rows_materialised: cx.rows_materialised,
                micros: t.elapsed().as_micros() as u64,
            }))
        });
        Some(Rebase {
            undo: undo2,
            redo: redo2,
            undo_ids: self.undo_ids.iter().skip(cut_at).copied().collect(),
            redo_ids: self.redo_ids[redo_from..].to_vec(),
            // The state before the oldest kept step: the start, or the state the cut step left.
            base_mark: Some(match cut_at {
                0 => self.base_mark,
                n => self.undo_ids[n - 1],
            }),
            next_id: cx.next,
            view,
        })
    }

    /// Put the rebased history in place (after `reset_to` with the saved file).
    pub(crate) fn apply_rebase(&mut self, r: Rebase) {
        debug_assert!(r.next_id as u64 >= self.index.rows());
        self.undo_bytes = r.undo.iter().map(Op::cost).sum();
        self.redo_bytes = r.redo.iter().map(Op::cost).sum();
        self.undo = r.undo;
        self.redo = r.redo;
        self.undo_ids = r.undo_ids;
        self.redo_ids = r.redo_ids;
        if let Some(m) = r.base_mark {
            self.base_mark = m;
        }
        self.next_id = r.next_id;
        self.clean_depth = Some(self.undo.len());
        // The view `reset_to` rebuilt covers the saved rows only; rows only the history knows keep how they showed.
        if let (Some((passing, hidden)), Some(old)) = (r.view, self.view.take()) {
            let mut v = match passing {
                Some(bits) if old.filters() => View::new(old.spec, bits, self.next_id),
                _ => View::all(),
            };
            v.hidden = hidden.map(Arc::new);
            if v.filters() || v.hidden.is_some() {
                self.view = Some(v);
            }
        }
    }
}

#[cfg(test)]
#[path = "rebase_tests.rs"]
mod tests;
