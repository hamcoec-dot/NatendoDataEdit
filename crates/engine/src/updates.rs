//! Accept or reject changes from a newer file (Milestone 15): rows of this
//! table and of a newer copy are paired by key columns, and columns by
//! their titles; every changed cell, new row and missing row is listed,
//! and the ones the user accepts are applied as one undo step.
//!
//! Built for full 10M-row files: the newer file is first indexed by hashes
//! only (key and row), so unchanged rows cost a few bytes each; only the
//! rows that differ have their values copied, within a memory budget.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::Cols;
use crate::reshape::RowPlan;
use crate::Error;

/// Changed cells listed with their values (all are counted and can be
/// accepted together).
pub const MAX_LISTED: usize = 100_000;
/// New rows listed with their values.
pub const MAX_ADDED_LISTED: usize = 1_000;
const CHUNK: u64 = 1 << 15;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSpec {
    /// Columns here that identify a row (found in the newer file by title
    /// when both have a title row, else by position).
    pub keys: Vec<u32>,
    /// Both files start with a title row (columns are then paired by
    /// title).
    pub keep_first: bool,
    pub match_case: bool,
    pub trim: bool,
}

/// One changed cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellUpdate {
    /// Its row here, counted over every row (hidden ones too).
    pub row: u64,
    pub col: u32,
    pub old: String,
    pub new: String,
    /// A filter or the person hid the row.
    pub hidden: bool,
}

/// What a newer file changes, for the review screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateList {
    /// The first [`MAX_LISTED`] changed cells, in table order.
    pub cells: Vec<CellUpdate>,
    pub cells_total: u64,
    /// Rows of the newer file not here: the first 1,000, as values (in
    /// this table's columns).
    pub added: Vec<Vec<String>>,
    pub added_total: u64,
    /// Rows here not in the newer file: their rows counted over every row
    /// (first 100,000).
    pub removed: Vec<u64>,
    pub removed_total: u64,
    /// Which of the listed rows in `removed` are hidden here.
    pub removed_hidden: Vec<bool>,
    /// All changed cells, and all missing rows, that are in hidden rows.
    pub cells_hidden_total: u64,
    pub removed_hidden_total: u64,
    /// Keys found more than once (paired in order).
    pub repeated_keys: u64,
    /// Columns here the newer file does not have (never compared or
    /// changed), by title.
    pub here_only: Vec<String>,
    /// Columns of the newer file not here (not compared; add them here
    /// first to take them in), by title.
    pub newer_only: Vec<String>,
    /// Columns found under the same title at another position.
    pub moved: u32,
}

/// Which of a kind of change to apply: every one but `picked`, or only
/// `picked` (numbers in listing order).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub all: bool,
    pub picked: Vec<u64>,
}

impl Choice {
    fn takes(&self, i: u64, picked: &std::collections::HashSet<u64>) -> bool {
        self.all != picked.contains(&i)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateChoice {
    pub cells: Choice,
    pub added: Choice,
    pub removed: Choice,
    /// A changed cell, or a missing row, in a hidden row is taken only when
    /// its number is in `picked`, whatever `all` says.
    pub cells_skip_hidden: bool,
    pub removed_skip_hidden: bool,
}

/// Document positions of every data row, shown or not.
fn every_row(d: &Document, keep_first: bool) -> std::ops::Range<u64> {
    let n = d.total_rows();
    u64::from(keep_first && n > 0).min(n)..n
}

/// Normalized title for pairing columns.
pub(crate) fn title_key(t: &str) -> String {
    t.trim().to_lowercase()
}

/// For each column here, the newer file's column with the same title
/// (`by_title`; repeated titles pair in order) or position.
pub(crate) fn pair_columns(here: &[String], there: &[String], by_title: bool) -> Vec<Option<u32>> {
    if !by_title {
        return (0..here.len())
            .map(|c| (c < there.len()).then_some(c as u32))
            .collect();
    }
    let mut queue: HashMap<String, VecDeque<u32>> = HashMap::new();
    for (j, t) in there.iter().enumerate() {
        queue.entry(title_key(t)).or_default().push_back(j as u32);
    }
    here.iter()
        .map(|t| queue.get_mut(&title_key(t)).and_then(|q| q.pop_front()))
        .collect()
}

/// A 128-bit key hash (two keyed SipHashes): different keys pairing by
/// accident is not a practical risk.
pub(crate) fn key_hash(v: &[&str], match_case: bool, trim: bool) -> u128 {
    let mut a = std::collections::hash_map::DefaultHasher::new();
    let mut b = std::collections::hash_map::DefaultHasher::new();
    0x5a_u8.hash(&mut b);
    for c in v {
        let c = if trim { c.trim() } else { c };
        if match_case {
            a.write(c.as_bytes());
            b.write(c.as_bytes());
        } else {
            let l = c.to_lowercase();
            a.write(l.as_bytes());
            b.write(l.as_bytes());
        }
        a.write_u8(0xff);
        b.write_u8(0xff);
    }
    (u128::from(a.finish()) << 64) | u128::from(b.finish())
}

/// The newer file indexed by hashes: for each data row, its key, a hash of
/// its compared cells, and its row id.
pub struct NewerIndex {
    pub(crate) revision: u64,
    /// Here column -> newer column (`None`: not in the newer file).
    pub(crate) map: Vec<Option<u32>>,
    pub(crate) newer_cols: u32,
    pub(crate) here_only: Vec<String>,
    pub(crate) newer_only: Vec<String>,
    pub(crate) moved: u32,
    /// (key, row hash, newer row id), in the newer file's order.
    pub(crate) rows: Vec<(u128, u64, u32)>,
}

impl NewerIndex {
    /// Index `newer` for comparing with a table titled `here_titles`.
    pub fn build(
        newer: &Document,
        here_titles: &[String],
        spec: &UpdateSpec,
        progress: &Progress,
    ) -> Result<NewerIndex, Error> {
        newer.ready_for_bulk()?;
        if spec.keys.is_empty() || spec.keys.len() > 16 {
            return Err(Error::Invalid(
                "choose 1 to 16 columns that identify a row".into(),
            ));
        }
        let newer_cols = newer.col_count();
        let all: Vec<u32> = (0..newer_cols).collect();
        let there_titles = newer.titles(&all, spec.keep_first);
        let map = pair_columns(here_titles, &there_titles, spec.keep_first);
        if spec.keep_first && map.iter().all(Option::is_none) {
            return Err(Error::Invalid(
                "no column title of this table is in the newer file; if the files have no title row, untick \"first row is titles\" to pair columns by position".into(),
            ));
        }
        let mut keys_there = Vec::with_capacity(spec.keys.len());
        for &k in &spec.keys {
            match map.get(k as usize).copied().flatten() {
                Some(j) => keys_there.push(j),
                None => {
                    return Err(Error::Invalid(format!(
                        "the key column {} is not in the newer file",
                        here_titles.get(k as usize).map_or("?", String::as_str)
                    )))
                }
            }
        }
        let mut used = vec![false; newer_cols as usize];
        for j in map.iter().flatten() {
            used[*j as usize] = true;
        }
        let here_only = map
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_none())
            .map(|(c, _)| here_titles[c].clone())
            .collect();
        let newer_only = used
            .iter()
            .enumerate()
            .filter(|(_, u)| !**u)
            .map(|(j, _)| there_titles[j].clone())
            .collect();
        let moved = map
            .iter()
            .enumerate()
            .filter(|(c, m)| m.is_some_and(|j| j as usize != *c))
            .count() as u32;
        // Compared columns, in this table's order.
        let compared: Vec<u32> = map.iter().flatten().copied().collect();
        let reader = Cols::new(newer, &all);
        // Every row of the newer file: its filter and hidden rows say what
        // the person is looking at, not what the file holds.
        let rows = every_row(newer, spec.keep_first);
        progress.start(rows.end - rows.start);
        let parts: Vec<Vec<(u128, u64, u32)>> = (0..(rows.end - rows.start).div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<_, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut out = Vec::with_capacity((e - s) as usize);
                let mut buf = Vec::new();
                for r in s..e {
                    let id = newer.doc_id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(newer, id, &mut buf);
                    let parts: Vec<&str> = keys_there
                        .iter()
                        .map(|&j| reader.get(&buf, j as usize))
                        .collect();
                    let key = key_hash(&parts, spec.match_case, spec.trim);
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    for &j in &compared {
                        h.write(reader.get(&buf, j as usize).as_bytes());
                        h.write_u8(0xff);
                    }
                    out.push((key, h.finish(), id));
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        Ok(NewerIndex {
            revision: newer.revision(),
            map,
            newer_cols,
            here_only,
            newer_only,
            moved,
            rows: parts.into_iter().flatten().collect(),
        })
    }
}

/// Rows paired by key: which differ, which are new, which are gone.
pub struct Pairing {
    pub(crate) revision: u64,
    /// (row id here, shown row here, newer row id) of paired rows whose
    /// compared cells differ.
    pub(crate) changed: Vec<(u32, u64, u32)>,
    /// (row id, shown row) here with no newer row.
    pub(crate) removed: Vec<(u32, u64)>,
    /// Newer row ids not paired, in the newer file's order.
    pub(crate) added: Vec<u32>,
    pub(crate) repeated: u64,
    /// Rows here whose key is empty (all key cells blank).
    pub(crate) empty_here: u64,
}

impl Pairing {
    /// The same pairing keeping at most `n` of each kind (for listing; the full counts are read before).
    pub(crate) fn capped(&self, n: usize) -> Pairing {
        Pairing {
            revision: self.revision,
            changed: self.changed.iter().take(n).copied().collect(),
            removed: self.removed.iter().take(n).copied().collect(),
            added: self.added.iter().take(n).copied().collect(),
            repeated: self.repeated,
            empty_here: self.empty_here,
        }
    }
}

/// The newer values of changed and new rows, in this table's columns
/// (end to end in one buffer).
pub struct NewerValues {
    width: usize,
    text: String,
    offs: Vec<u64>,
    /// Rows: first those of `changed`, then those of `added`.
    changed_rows: usize,
}

impl NewerValues {
    pub(crate) fn cell(&self, row: usize, c: usize) -> &str {
        let k = row * self.width + c;
        &self.text[self.offs[k] as usize..self.offs[k + 1] as usize]
    }

    pub(crate) fn row(&self, row: usize) -> Vec<String> {
        (0..self.width)
            .map(|c| self.cell(row, c).to_string())
            .collect()
    }

    /// Copy out the newer values of `pairing`'s changed and new rows.
    pub fn fetch(
        newer: &Document,
        index: &NewerIndex,
        pairing: &Pairing,
        progress: &Progress,
    ) -> Result<NewerValues, Error> {
        newer.ready_for_bulk()?;
        if newer.revision() != index.revision {
            return Err(Error::Changed);
        }
        let width = index.map.len();
        let all: Vec<u32> = (0..index.newer_cols).collect();
        let reader = Cols::new(newer, &all);
        let ids: Vec<u32> = pairing
            .changed
            .iter()
            .map(|x| x.2)
            .chain(pairing.added.iter().copied())
            .collect();
        let budget = crate::find::split_budget(newer);
        let used = AtomicUsize::new(0);
        progress.start(ids.len() as u64);
        let parts: Vec<(String, Vec<u64>)> = ids
            .par_chunks(CHUNK as usize)
            .map(|part| -> Result<_, Error> {
                let mut text = String::new();
                let mut offs = Vec::with_capacity(part.len() * width);
                let mut buf = Vec::new();
                for &id in part {
                    reader.read(newer, id, &mut buf);
                    let before = text.len();
                    for m in &index.map {
                        if let Some(j) = m {
                            text.push_str(reader.get(&buf, *j as usize));
                        }
                        offs.push(text.len() as u64);
                    }
                    let n = text.len() - before + 8 * width;
                    if used.fetch_add(n, Ordering::Relaxed) + n > budget {
                        return Err(Error::OutOfMemory(
                            "too many rows differ to take the changes at once; compare part of the file (filter it first)",
                        ));
                    }
                }
                progress.step(part.len() as u64)?;
                Ok((text, offs))
            })
            .collect::<Result<_, _>>()?;
        let mut text = String::new();
        let mut offs = vec![0u64];
        for (t, o) in parts {
            let base = text.len() as u64;
            text.push_str(&t);
            offs.extend(o.iter().map(|x| x + base));
        }
        Ok(NewerValues {
            width,
            text,
            offs,
            changed_rows: pairing.changed.len(),
        })
    }
}

/// Everything found, kept for applying.
pub struct Updates {
    revision: u64,
    /// (row id here, column, row of `values`), all of them, in table order.
    cells: Vec<(u32, u32, u32)>,
    /// Rows of `values` that are new rows.
    added: Vec<u32>,
    removed: Vec<u32>,
    values: NewerValues,
}

impl Document {
    /// Pair this table's shown rows with the newer file's by key; rows whose
    /// compared cells hash the same are unchanged.
    pub fn pair_newer(
        &self,
        index: &NewerIndex,
        spec: &UpdateSpec,
        progress: &Progress,
    ) -> Result<Pairing, Error> {
        self.ready_for_bulk()?;
        let cols = self.col_count();
        if index.map.len() != cols as usize || spec.keys.iter().any(|&k| k >= cols) {
            return Err(Error::Changed);
        }
        let compared: Vec<u32> = (0..cols)
            .filter(|&c| index.map[c as usize].is_some())
            .collect();
        // The newer keys sorted (stable: repeats keep the file's order).
        let mut sorted: Vec<(u128, u32)> = index
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.0, i as u32))
            .collect();
        sorted.par_sort_unstable();
        let all: Vec<u32> = (0..cols).collect();
        let reader = Cols::new(self, &all);
        // Every row here too: a hidden row is still in the table.
        let rows = every_row(self, spec.keep_first);
        progress.start(rows.end - rows.start);
        let empty_key = key_hash(&vec![""; spec.keys.len()], spec.match_case, spec.trim);
        let mut empty_here = 0u64;
        let mut repeated = 0u64;
        for w in sorted.windows(2) {
            if w[0].0 == w[1].0 {
                repeated += 1;
            }
        }
        // How many of a repeated key's newer rows are used so far.
        let mut used: HashMap<u128, u32> = HashMap::new();
        let mut taken = vec![false; index.rows.len()];
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        // The rows here are read a batch of chunks at a time and paired as they come (in order, which repeated
        // keys need), so the whole table's (id, row, key, hash) list is never held at once.
        const BATCH: u64 = 64;
        let n_chunks = (rows.end - rows.start).div_ceil(CHUNK);
        let mut kb = 0u64;
        while kb < n_chunks {
            let ke = (kb + BATCH).min(n_chunks);
            let mine: Vec<Vec<(u32, u64, u128, u64)>> = (kb..ke)
                .into_par_iter()
                .map(|k| -> Result<_, Error> {
                    let s = rows.start + k * CHUNK;
                    let e = (s + CHUNK).min(rows.end);
                    let mut out = Vec::with_capacity((e - s) as usize);
                    let mut buf = Vec::new();
                    for r in s..e {
                        let id = self.doc_id_at(r).ok_or(Error::OutOfRange)?;
                        reader.read(self, id, &mut buf);
                        let parts: Vec<&str> = spec
                            .keys
                            .iter()
                            .map(|&k| reader.get(&buf, k as usize))
                            .collect();
                        let key = key_hash(&parts, spec.match_case, spec.trim);
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        for &c in &compared {
                            h.write(reader.get(&buf, c as usize).as_bytes());
                            h.write_u8(0xff);
                        }
                        out.push((id, r, key, h.finish()));
                    }
                    progress.step(e - s)?;
                    Ok(out)
                })
                .collect::<Result<_, _>>()?;
            for &(id, r, key, hash) in mine.iter().flatten() {
                if key == empty_key {
                    empty_here += 1;
                }
                let first = sorted.partition_point(|x| x.0 < key);
                if sorted.get(first).is_none_or(|x| x.0 != key) {
                    removed.push((id, r));
                    continue;
                }
                let repeats = sorted.get(first + 1).is_some_and(|x| x.0 == key);
                let n = if repeats {
                    let n = used.entry(key).or_insert(0);
                    let at = *n;
                    *n += 1;
                    at as usize
                } else if taken[sorted[first].1 as usize] {
                    1
                } else {
                    0
                };
                match sorted.get(first + n).filter(|x| x.0 == key) {
                    Some(&(_, j)) => {
                        taken[j as usize] = true;
                        let (_, h, nid) = index.rows[j as usize];
                        if h != hash {
                            changed.push((id, r, nid));
                        }
                    }
                    None => {
                        repeated += 1;
                        removed.push((id, r));
                    }
                }
            }
            kb = ke;
        }
        let added = taken
            .iter()
            .enumerate()
            .filter(|(_, t)| !**t)
            .map(|(j, _)| index.rows[j].2)
            .collect();
        Ok(Pairing {
            revision: self.revision(),
            changed,
            removed,
            added,
            repeated,
            empty_here,
        })
    }

    /// List what differs: changed cells of the changed rows (compared
    /// columns only), new rows and missing rows.
    pub fn diff_newer(
        &self,
        index: &NewerIndex,
        pairing: &Pairing,
        values: NewerValues,
        progress: &Progress,
    ) -> Result<(Updates, UpdateList), Error> {
        self.ready_for_bulk()?;
        if self.revision() != pairing.revision {
            return Err(Error::Changed);
        }
        let cols = self.col_count();
        let all: Vec<u32> = (0..cols).collect();
        let reader = Cols::new(self, &all);
        progress.start(pairing.changed.len() as u64);
        type Found = Vec<(u32, u64, u32, u32, String)>;
        let found: Vec<Found> = pairing
            .changed
            .par_chunks(4096)
            .enumerate()
            .map(|(k, part)| -> Result<_, Error> {
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for (i, &(id, r, _)) in part.iter().enumerate() {
                    let vr = k * 4096 + i;
                    reader.read(self, id, &mut buf);
                    #[allow(clippy::needless_range_loop)] // c also indexes the row and values
                    for c in 0..cols as usize {
                        if index.map[c].is_none() {
                            continue;
                        }
                        let old = reader.get(&buf, c);
                        if old != values.cell(vr, c) {
                            out.push((id, r, c as u32, vr as u32, old.to_string()));
                        }
                    }
                }
                progress.step(part.len() as u64)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let mut list = UpdateList {
            repeated_keys: pairing.repeated,
            here_only: index.here_only.clone(),
            newer_only: index.newer_only.clone(),
            moved: index.moved,
            ..Default::default()
        };
        let mut cells = Vec::new();
        for (id, r, c, vr, old) in found.into_iter().flatten() {
            let hidden = !self.is_shown(id);
            list.cells_hidden_total += u64::from(hidden);
            if list.cells.len() < MAX_LISTED {
                list.cells.push(CellUpdate {
                    hidden,
                    row: r,
                    col: c,
                    old,
                    new: values.cell(vr as usize, c as usize).to_string(),
                });
            }
            cells.push((id, c, vr));
        }
        list.cells_total = cells.len() as u64;
        let added: Vec<u32> = (0..pairing.added.len())
            .map(|i| (values.changed_rows + i) as u32)
            .collect();
        list.added = added
            .iter()
            .take(MAX_ADDED_LISTED)
            .map(|&v| values.row(v as usize))
            .collect();
        list.added_total = added.len() as u64;
        list.removed = pairing
            .removed
            .iter()
            .take(MAX_LISTED)
            .map(|x| x.1)
            .collect();
        list.removed_total = pairing.removed.len() as u64;
        list.removed_hidden = pairing
            .removed
            .iter()
            .take(MAX_LISTED)
            .map(|x| !self.is_shown(x.0))
            .collect();
        list.removed_hidden_total = pairing
            .removed
            .iter()
            .filter(|x| !self.is_shown(x.0))
            .count() as u64;
        Ok((
            Updates {
                revision: self.revision(),
                cells,
                added,
                removed: pairing.removed.iter().map(|x| x.0).collect(),
                values,
            },
            list,
        ))
    }

    /// The accepted changes as one undo step: cells set, new rows added at
    /// the end, missing rows taken out.
    pub fn plan_updates(&self, ups: &Updates, choice: &UpdateChoice) -> Result<RowPlan, Error> {
        self.ready_for_bulk()?;
        if ups.revision != self.revision() {
            return Err(Error::Changed);
        }
        let set = |c: &Choice| {
            c.picked
                .iter()
                .copied()
                .collect::<std::collections::HashSet<u64>>()
        };
        let (pc, pa, pr) = (set(&choice.cells), set(&choice.added), set(&choice.removed));
        let mut plan = RowPlan::new(self);
        let width = self.col_count();
        let mut by_row: Vec<(u32, Vec<(u32, String)>)> = Vec::new();
        for (i, &(id, c, vr)) in ups.cells.iter().enumerate() {
            let held = choice.cells_skip_hidden && !self.is_shown(id);
            let take = if held {
                pc.contains(&(i as u64))
            } else {
                choice.cells.takes(i as u64, &pc)
            };
            if !take || c >= width {
                continue;
            }
            let text = ups.values.cell(vr as usize, c as usize).to_string();
            match by_row.last_mut() {
                Some((last, cells)) if *last == id => cells.push((c, text)),
                _ => by_row.push((id, vec![(c, text)])),
            }
        }
        plan.changes = by_row;
        let mut rows_added: Vec<Vec<String>> = Vec::new();
        for (i, &v) in ups.added.iter().enumerate() {
            if choice.added.takes(i as u64, &pa) {
                rows_added.push(ups.values.row(v as usize));
            }
        }
        for (i, &id) in ups.removed.iter().enumerate() {
            let held = choice.removed_skip_hidden && !self.is_shown(id);
            let take = if held {
                pr.contains(&(i as u64))
            } else {
                choice.removed.takes(i as u64, &pr)
            };
            if take {
                plan.remove.push(id);
            }
        }
        plan.rows = (rows_added.len() + plan.remove.len()) as u64;
        if !rows_added.is_empty() {
            let gone: std::collections::HashSet<u32> = plan.remove.iter().copied().collect();
            let total = self.total_rows();
            let last = (0..total)
                .rev()
                .filter_map(|r| self.doc_id_at(r))
                .find(|id| !gone.contains(id))
                .ok_or_else(|| Error::Invalid("the table has no row to add after".into()))?;
            plan.after.push((last, rows_added));
        }
        Ok(plan)
    }
}

/// All steps at once, for two documents both at hand (tests, the perf
/// suite and the command line).
pub fn compare_newer(
    here: &Document,
    newer: &Document,
    spec: &UpdateSpec,
    progress: &Progress,
) -> Result<(Updates, UpdateList), Error> {
    let all: Vec<u32> = (0..here.col_count()).collect();
    let titles = here.titles(&all, spec.keep_first);
    let index = NewerIndex::build(newer, &titles, spec, progress)?;
    let pairing = here.pair_newer(&index, spec, progress)?;
    let values = NewerValues::fetch(newer, &index, &pairing, progress)?;
    here.diff_newer(&index, &pairing, values, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn spec() -> UpdateSpec {
        UpdateSpec {
            keys: vec![0],
            keep_first: true,
            ..Default::default()
        }
    }

    #[test]
    fn accept_some_changes() {
        let mut old = doc("id,name,ward\r\n1,Ann,4\r\n2,Bo,5\r\n3,Cy,6\r\n");
        let new = doc("id,name,ward\r\n2,Bob,5\r\n1,Ann,7\r\n4,Dee,1\r\n");
        let p = Progress::new();
        let (ups, list) = compare_newer(&old, &new, &spec(), &p).unwrap();
        assert_eq!(list.cells_total, 2);
        assert_eq!(
            (
                list.cells[0].row,
                list.cells[0].col,
                list.cells[0].new.as_str()
            ),
            (1, 2, "7")
        );
        assert_eq!((list.added_total, list.removed_total), (1, 1));
        assert_eq!(list.added[0], vec!["4", "Dee", "1"]);
        assert_eq!(list.removed, vec![3]);
        assert!(list.here_only.is_empty() && list.newer_only.is_empty());
        // Accept every cell but the ward, the new row, not the removal.
        let choice = UpdateChoice {
            cells: Choice {
                all: true,
                picked: vec![0],
            },
            added: Choice {
                all: true,
                picked: vec![],
            },
            removed: Choice {
                all: false,
                picked: vec![],
            },
            ..Default::default()
        };
        let plan = old.plan_updates(&ups, &choice).unwrap();
        old.apply_rows(plan).unwrap();
        let rows: Vec<Vec<String>> = (0..old.row_count())
            .map(|r| old.row_values(r).unwrap())
            .collect();
        assert_eq!(rows[1], vec!["1", "Ann", "4"]);
        assert_eq!(rows[2], vec!["2", "Bob", "5"]);
        assert_eq!(rows[3], vec!["3", "Cy", "6"]);
        assert_eq!(rows[4], vec!["4", "Dee", "1"]);
        // One undo step.
        assert!(old.undo());
        assert_eq!(old.row_count(), 4);
        assert_eq!(old.cell(2, 1).unwrap(), "Bo");
        // A changed table is refused.
        old.set_cell(1, 1, "x".into()).unwrap();
        assert!(old.plan_updates(&ups, &choice).is_err());
    }

    const THREE: &str = "id,name\n1,Ann\n2,Bo\n3,Cy\n";

    fn take_all() -> UpdateChoice {
        let all = Choice {
            all: true,
            picked: vec![],
        };
        UpdateChoice {
            cells: all.clone(),
            added: all.clone(),
            removed: all,
            cells_skip_hidden: false,
            removed_skip_hidden: true,
        }
    }

    /// A row hidden here is not offered as new again, and a row hidden in the
    /// newer table is not offered for deletion.
    #[test]
    fn hidden_rows_of_both_tables_are_compared() {
        let p = Progress::new();
        let mut old = doc(THREE);
        old.hide_rows(2, 1).unwrap();
        let (_, list) = compare_newer(&old, &doc(THREE), &spec(), &p).unwrap();
        assert_eq!((list.added_total, list.removed_total), (0, 0));
        let mut newer = doc(THREE);
        newer.hide_rows(2, 1).unwrap();
        let (_, list) = compare_newer(&doc(THREE), &newer, &spec(), &p).unwrap();
        assert_eq!((list.added_total, list.removed_total), (0, 0));
    }

    #[test]
    fn a_change_to_a_hidden_row_is_marked_and_can_be_left_out() {
        let p = Progress::new();
        let newer = doc("id,name\n1,Ann\n2,Bob\n3,Cyd\n");
        for (skip, want) in [(false, ["Ann", "Bob", "Cyd"]), (true, ["Ann", "Bo", "Cyd"])] {
            let mut old = doc(THREE);
            old.hide_rows(2, 1).unwrap();
            let (ups, list) = compare_newer(&old, &newer, &spec(), &p).unwrap();
            assert_eq!(list.cells_hidden_total, 1);
            assert_eq!(
                list.cells
                    .iter()
                    .map(|c| (c.row, c.hidden))
                    .collect::<Vec<_>>(),
                [(2, true), (3, false)]
            );
            let choice = UpdateChoice {
                cells_skip_hidden: skip,
                ..take_all()
            };
            let plan = old.plan_updates(&ups, &choice).unwrap();
            old.apply_rows(plan).unwrap();
            old.show_rows(crate::rows::ShowRows::All).unwrap();
            let got: Vec<String> = (1..4).map(|r| old.cell(r, 1).unwrap()).collect();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn deleting_a_hidden_row_takes_an_explicit_tick() {
        let p = Progress::new();
        let newer = doc("id,name\n1,Ann\n");
        for (picked, left) in [(vec![], vec!["Ann", "Bo"]), (vec![0], vec!["Ann"])] {
            let mut old = doc(THREE);
            old.hide_rows(2, 1).unwrap(); // Bo: the first missing row
            let (ups, list) = compare_newer(&old, &newer, &spec(), &p).unwrap();
            assert_eq!(list.removed_hidden, [true, false]);
            assert_eq!(list.removed_hidden_total, 1);
            let mut choice = take_all();
            choice.removed.picked = picked;
            // "Take all" with the hidden ones held back: a number in `picked`
            // takes the hidden row and nothing else changes.
            let plan = old.plan_updates(&ups, &choice).unwrap();
            old.apply_rows(plan).unwrap();
            old.show_rows(crate::rows::ShowRows::All).unwrap();
            let got: Vec<String> = (1..old.row_count())
                .map(|r| old.cell(r, 1).unwrap())
                .collect();
            assert_eq!(got, left);
        }
    }

    proptest::proptest! {
        /// Hiding rows in either table changes nothing about the result.
        #[test]
        fn filters_do_not_change_the_result(
            old in proptest::collection::vec((0u8..6, "[ab]{0,2}", proptest::bool::ANY), 0..12),
            new in proptest::collection::vec((0u8..6, "[ab]{0,2}", proptest::bool::ANY), 0..12),
        ) {
            let make = |rows: &[(u8, String, bool)], hide: bool| {
                let mut text = String::from("id,name\n");
                for r in rows {
                    text.push_str(&format!("{},{}\n", r.0, r.1));
                }
                let mut d = doc(&text);
                if hide {
                    for (i, r) in rows.iter().enumerate().rev() {
                        if r.2 {
                            d.hide_rows(i as u64 + 1, 1).unwrap();
                        }
                    }
                }
                d
            };
            let run = |hide: bool| {
                let mut here = make(&old, hide);
                let p = Progress::new();
                let (ups, _) = compare_newer(&here, &make(&new, hide), &spec(), &p).unwrap();
                let mut choice = take_all();
                choice.removed_skip_hidden = false;
                let plan = here.plan_updates(&ups, &choice).unwrap();
                here.apply_rows(plan).unwrap();
                let _ = here.show_rows(crate::rows::ShowRows::All);
                (0..here.total_rows())
                    .map(|r| here.row_values(r).unwrap())
                    .collect::<Vec<_>>()
            };
            proptest::prop_assert_eq!(run(true), run(false));
        }
    }

    #[test]
    fn columns_pair_by_title() {
        // The newer export has a column inserted, one moved, one dropped
        // and one added: values still land in the right columns.
        let mut old = doc("id,name,ward,phone\r\n1,Ann,4,555\r\n2,Bo,5,556\r\n");
        let new = doc(
            "id,precinct,ward,name,email\r\n1,P1,4,Anne,a@x\r\n2,P2,9,Bo,b@x\r\n3,P3,1,Cy,c@x\r\n",
        );
        let p = Progress::new();
        let (ups, list) = compare_newer(&old, &new, &spec(), &p).unwrap();
        assert_eq!(list.here_only, vec!["phone"]);
        assert_eq!(list.newer_only, vec!["precinct", "email"]);
        assert_eq!(list.moved, 1, "name moved");
        assert_eq!(list.cells_total, 2, "{:?}", list.cells);
        assert_eq!((list.cells[0].col, list.cells[0].new.as_str()), (1, "Anne"));
        assert_eq!((list.cells[1].col, list.cells[1].new.as_str()), (2, "9"));
        assert_eq!(list.added[0], vec!["3", "Cy", "1", ""]);
        let all = Choice {
            all: true,
            picked: vec![],
        };
        let plan = old
            .plan_updates(
                &ups,
                &UpdateChoice {
                    cells: all.clone(),
                    added: all.clone(),
                    removed: all,
                    ..Default::default()
                },
            )
            .unwrap();
        old.apply_rows(plan).unwrap();
        assert_eq!(old.row_values(1).unwrap(), vec!["1", "Anne", "4", "555"]);
        assert_eq!(old.row_values(2).unwrap(), vec!["2", "Bo", "9", "556"]);
        assert_eq!(old.row_values(3).unwrap(), vec!["3", "Cy", "1", ""]);
        // No titles in common: refused with a plain reason.
        let other = doc("a,b\r\n1,2\r\n");
        assert!(compare_newer(&old, &other, &spec(), &p).is_err());
        // The key column must be in the newer file.
        let nokey = doc("name,ward\r\nAnn,4\r\n");
        assert!(compare_newer(&old, &nokey, &spec(), &p).is_err());
    }

    #[test]
    fn repeated_keys_pair_in_order() {
        let old = doc("id,v\r\n1,a\r\n1,b\r\n2,c\r\n");
        let new = doc("id,v\r\n1,a\r\n1,B\r\n1,extra\r\n");
        let (_, list) = compare_newer(&old, &new, &spec(), &Progress::new()).unwrap();
        assert_eq!(list.cells_total, 1);
        assert_eq!((list.cells[0].row, list.cells[0].new.as_str()), (2, "B"));
        assert_eq!(list.added, vec![vec!["1", "extra"]]);
        assert_eq!(list.removed, vec![3]);
        assert_eq!(list.repeated_keys, 2);
    }
}
