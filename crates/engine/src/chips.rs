//! Filter chips (C8): every rule of a filter is a chip that can be switched
//! off, inverted or renamed; chips are joined by Match all (AND) or Match
//! any (OR), and the whole result can be inverted ("show the rows that
//! failed"). The "show these rows" tools (problems, duplicates, the pattern
//! pill, a formula...) add a chip of their own instead of replacing the
//! filter.
//!
//! Each chip has PASS and FAIL rows. A column chip's FAIL is every row it
//! does not pass. A tool's chip only looked at some rows (the rows shown
//! when it ran, or every row): its FAIL is the rows it looked at and did
//! not pick; a row it never looked at is neither, so inverting never shows
//! rows nobody checked. A formula's error rows are neither too.
//!   - Invert a chip: swap its PASS and FAIL.
//!   - Match all: PASS = all PASS, FAIL = any FAIL.
//!   - Match any: PASS = any PASS, FAIL = all FAIL.
//!   - Invert all: swap the result's PASS and FAIL.
//!   - Shown = PASS (the title row and rows put in afterwards always show).
//!
//! Switching chips never reads the table again when the rules' rows are
//! known: each column rule's PASS rows are kept in a cache keyed by the
//! data revision (any edit empties it), capped by bytes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Progress, View};
use crate::filter::{ColumnFilter, Combine, FilterPlan, FilterSpec};
use crate::{Document, Error};

/// Most chips made by tools at once.
pub const MAX_SET_CHIPS: usize = 20;
/// Bytes of rule results and tool chips kept (about 50 rules at 10M rows).
pub const CHIP_BYTES: usize = 64 << 20;

/// A chip made by a "show these rows" tool: the rows it picked out of the
/// rows it looked at. Fixed when made (it is never worked out again).
#[derive(Debug, Clone)]
pub(crate) struct SetChip {
    /// Which tool ("problems", "duplicates", "formula"...).
    pub kind: String,
    /// What it shows, in words ("Problems", "Formula: [a] > 1").
    pub label: String,
    pub name: Option<String>,
    pub enabled: bool,
    pub negate: bool,
    /// Rows picked (ids below `base`).
    pub set: Arc<Vec<u64>>,
    /// Rows looked at (the picked ones too).
    pub looked: Arc<Vec<u64>>,
    /// A formula's rows that gave an error (neither picked nor not).
    pub errors: Option<Arc<Vec<u64>>>,
    pub error_count: u64,
    pub base: u32,
    /// The data revision it was made at ("made before your last edit").
    pub made_rev: u64,
    /// Seconds since 1970 (UTC) it was made.
    pub made_at: u64,
    /// Rows it looked at.
    pub made_from: u64,
}

impl SetChip {
    pub(crate) fn bytes(&self) -> usize {
        (self.set.len() + self.looked.len() + self.errors.as_ref().map_or(0, |e| e.len())) * 8
    }
}

/// Column rules' PASS rows, by the rule (its JSON with the columns read
/// physically), for one data revision.
#[derive(Default)]
pub(crate) struct RuleCache {
    rev: u64,
    map: HashMap<String, (Arc<Vec<u64>>, u64)>,
    tick: u64,
    bytes: usize,
    /// For the column chips last folded: how many rules that are on each
    /// row passes (one byte a row, by row id). A switch then reads only the
    /// rule switched, never all the others again.
    count: Option<PassCount>,
}

/// How many of `keys` (rules that are on, "!" before an inverted one) each
/// row passes, by row id.
struct PassCount {
    /// The reading options and title row the keys were read with.
    ctx: String,
    keys: Vec<String>,
    per_row: Vec<u8>,
}

/// Most column rules a filter's chips count row by row (one byte a row).
const MAX_COUNTED: usize = 255;

impl RuleCache {
    fn at(&mut self, rev: u64) {
        if self.rev != rev {
            self.map.clear();
            self.bytes = 0;
            self.count = None;
            self.rev = rev;
        }
    }

    fn get(&mut self, key: &str) -> Option<Arc<Vec<u64>>> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(key).map(|e| {
            e.1 = t;
            e.0.clone()
        })
    }

    /// Keep `bits` under `key`, dropping the least recently used to stay
    /// within `room` bytes (nothing kept when it alone does not fit).
    fn put(&mut self, key: String, bits: Arc<Vec<u64>>, room: usize) {
        let size = bits.len() * 8;
        if size > room {
            return;
        }
        if let Some((b, _)) = self.map.remove(&key) {
            self.bytes -= b.len() * 8;
        }
        self.shrink(room - size);
        self.tick += 1;
        self.map.insert(key, (bits, self.tick));
        self.bytes += size;
    }

    /// Drop the least recently used until at most `room` bytes are kept.
    fn shrink(&mut self, room: usize) {
        while self.bytes > room {
            let Some(old) = self
                .map
                .iter()
                .min_by_key(|(_, (_, t))| *t)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((b, _)) = self.map.remove(&old) {
                self.bytes -= b.len() * 8;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
}

/// One chip as the chip bar shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChipInfo {
    /// "column" for a column rule, else the tool's kind.
    pub kind: String,
    /// Logical column of a column rule (`None`: its column was deleted, or
    /// a tool's chip).
    pub col: Option<u32>,
    /// A tool chip's words ("" for a column rule: the rule says it).
    pub label: String,
    pub name: Option<String>,
    pub enabled: bool,
    pub negate: bool,
    /// Rows it lets through on its own (`None`: not known without reading
    /// the table again).
    pub rows: Option<u64>,
    /// Tool chips: rows it looked at, and rows it did not.
    pub looked: Option<u64>,
    pub not_looked: Option<u64>,
    /// Formula chips: rows that gave an error.
    pub errors: u64,
    /// Made before the last edit (tool chips are never worked out again).
    pub stale: bool,
    /// Seconds since 1970 (UTC) a tool chip was made.
    pub made_at: u64,
}

/// The chip bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct FilterChips {
    pub chips: Vec<ChipInfo>,
    pub combine: Combine,
    pub invert: bool,
    /// Data rows shown, and data rows in all (the title row not counted).
    pub shown: u64,
    pub total: u64,
    /// Every chip is off (every row shows).
    pub all_off: bool,
    /// No more tool chips can be added.
    pub sets_full: bool,
}

/// A change to the chips (one undo step, as any filter change).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ChipChange {
    /// Switch chip `index` off or on (chips are numbered column rules
    /// first, then tool chips).
    Toggle {
        index: usize,
    },
    /// Invert chip `index`.
    Negate {
        index: usize,
    },
    Remove {
        index: usize,
    },
    Rename {
        index: usize,
        name: Option<String>,
    },
    Combine {
        combine: Combine,
    },
    Invert {
        on: bool,
    },
    /// A formula chip's error rows as a chip of their own (the formula's
    /// chip switched off).
    ShowErrors {
        index: usize,
    },
}

/// Where a "show these rows" tool looked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Looked {
    /// The rows shown when it ran.
    #[default]
    Shown,
    /// Every row, hidden ones too.
    All,
}

/// What a tool's plan adds to make its chip.
#[derive(Debug, Clone, Default)]
pub struct ChipMaking {
    pub kind: String,
    pub label: String,
    pub looked: Looked,
    /// Rows that gave an error (a formula), by row id.
    pub errors: Option<Vec<u64>>,
    pub error_count: u64,
    /// The chip starts inverted (show the rows it did not pick).
    pub negate: bool,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn word(v: &[u64], w: usize) -> u64 {
    v.get(w).copied().unwrap_or(0)
}

fn set_bit(v: &mut [u64], id: u32) {
    if let Some(w) = v.get_mut((id >> 6) as usize) {
        *w |= 1 << (id & 63);
    }
}

fn has_bit(v: &[u64], id: u32) -> bool {
    v.get((id >> 6) as usize)
        .is_some_and(|w| (w >> (id & 63)) & 1 == 1)
}

fn popcount(v: &[u64]) -> u64 {
    v.iter().map(|w| u64::from(w.count_ones())).sum()
}

/// A chip's PASS and FAIL words.
enum Part<'a> {
    /// Column chips folded together: FAIL is not PASS.
    Cols(&'a [u64]),
    Set {
        set: &'a [u64],
        looked: &'a [u64],
        negate: bool,
    },
}

impl Part<'_> {
    fn words(&self, w: usize) -> (u64, u64) {
        match self {
            Part::Cols(c) => {
                let p = word(c, w);
                (p, !p)
            }
            Part::Set {
                set,
                looked,
                negate,
            } => {
                let (s, l) = (word(set, w), word(looked, w));
                let (p, f) = (s & l, l & !s);
                if *negate {
                    (f, p)
                } else {
                    (p, f)
                }
            }
        }
    }
}

/// The rows shown by the chips (PASS of the fold), `words` long.
fn fold(parts: &[Part], combine: Combine, invert: bool, words: usize) -> Vec<u64> {
    if parts.is_empty() {
        // No chip on: every row shows.
        return vec![!0u64; words];
    }
    (0..words)
        .map(|w| {
            let (mut p, mut f) = match combine {
                Combine::And => (!0u64, 0u64),
                Combine::Or => (0u64, !0u64),
            };
            for part in parts {
                let (a, b) = part.words(w);
                match combine {
                    Combine::And => {
                        p &= a;
                        f |= b;
                    }
                    Combine::Or => {
                        p |= a;
                        f &= b;
                    }
                }
            }
            if invert {
                f
            } else {
                p
            }
        })
        .collect()
}

/// The rule's cache key: the rule with its physical column, how values are
/// read, and whether the title row is left out.
fn rule_key(c: &ColumnFilter, spec: &FilterSpec) -> String {
    serde_json::to_string(&(c.col, &c.rule, spec.read, spec.keep_first)).unwrap_or_default()
}

/// Renumber the chips of `v` to the positions of `ids` (after a save):
/// rows put in after the filter count as passing every chip.
pub(crate) fn renumber(
    v: &View,
    ids: impl Iterator<Item = u32>,
) -> (Vec<SetChip>, Option<Arc<Vec<u64>>>) {
    let pairs: Vec<(u32, u32)> = ids.enumerate().map(|(p, id)| (p as u32, id)).collect();
    let n = pairs.len();
    let words = n.div_ceil(64);
    let after = |id: u32, base: u32| id >= base;
    let remap = |bits: &[u64], base: u32, newer: bool| -> Arc<Vec<u64>> {
        let mut out = vec![0u64; words];
        for &(p, id) in &pairs {
            if (newer && after(id, base)) || (id < base && has_bit(bits, id)) {
                set_bit(&mut out, p);
            }
        }
        Arc::new(out)
    };
    let view_base = v.base;
    let sets = v
        .sets
        .iter()
        .map(|s| {
            let base = s.base.min(view_base);
            SetChip {
                set: remap(&s.set, base, true),
                looked: remap(&s.looked, base, true),
                errors: s.errors.as_ref().map(|e| remap(e, base, false)),
                base: n as u32,
                ..s.clone()
            }
        })
        .collect();
    let cols = v
        .cols
        .as_ref()
        .map(|c| remap(c, v.cols_base.min(view_base), true));
    (sets, cols)
}

/// The column rules after `change`, for a recipe (`None`: the change is to
/// a tool's chip, which a recipe cannot repeat).
pub fn spec_after(spec: &FilterSpec, change: &ChipChange) -> Option<FilterSpec> {
    let mut s = spec.clone();
    let n = s.columns.len();
    match change {
        ChipChange::Toggle { index } if *index < n => s.columns[*index].enabled ^= true,
        ChipChange::Negate { index } if *index < n => s.columns[*index].negate ^= true,
        ChipChange::Remove { index } if *index < n => {
            s.columns.remove(*index);
        }
        ChipChange::Rename { index, name } if *index < n => {
            s.columns[*index].name = name
                .as_ref()
                .map(|n| n.trim().chars().take(80).collect::<String>())
                .filter(|n| !n.is_empty());
        }
        ChipChange::Combine { combine } => s.combine = *combine,
        ChipChange::Invert { on } => s.invert = *on,
        _ => return None,
    }
    Some(s)
}

/// The tool chips that are on, folded with Match all (for the filter
/// panel's counts).
pub(crate) struct SetMask {
    bits: Vec<u64>,
    base: u32,
}

impl SetMask {
    pub(crate) fn passes(&self, id: u32) -> bool {
        id >= self.base || has_bit(&self.bits, id)
    }
}

impl Document {
    /// The tool chips that are on, as one mask (`None`: none on).
    pub(crate) fn chip_set_mask(&self) -> Option<SetMask> {
        let v = self.view().filter(|v| v.filters())?;
        let on: Vec<&SetChip> = v.sets.iter().filter(|s| s.enabled).collect();
        if on.is_empty() {
            return None;
        }
        let base = on.iter().map(|s| s.base).min().unwrap_or(0);
        let parts: Vec<Part> = on
            .iter()
            .map(|s| Part::Set {
                set: &s.set,
                looked: &s.looked,
                negate: s.negate,
            })
            .collect();
        let bits = fold(&parts, Combine::And, false, (base as usize).div_ceil(64));
        Some(SetMask { bits, base })
    }

    /// Drop every cached rule result (tests).
    #[cfg(test)]
    pub(crate) fn rule_cache_len(&self) -> usize {
        self.rule_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    /// Bytes the tool chips of the current filter hold.
    fn set_bytes(&self) -> usize {
        self.view()
            .map_or(0, |v| v.sets.iter().map(SetChip::bytes).sum())
    }

    /// Each rule's PASS rows (rules physical, "largest" resolved), from the
    /// cache, the missing ones found in one pass over every row.
    pub(crate) fn rule_pass(
        &self,
        spec: &FilterSpec,
        rules: &[ColumnFilter],
        progress: &Progress,
    ) -> Result<Vec<Arc<Vec<u64>>>, Error> {
        let rules: Vec<ColumnFilter> = rules
            .iter()
            .map(|c| ColumnFilter {
                negate: false,
                enabled: true,
                name: None,
                ..c.clone()
            })
            .collect();
        let rules = &rules[..];
        let keys: Vec<String> = rules.iter().map(|c| rule_key(c, spec)).collect();
        let mut out: Vec<Option<Arc<Vec<u64>>>> = {
            let mut cache = self.rule_cache.lock().unwrap_or_else(|p| p.into_inner());
            cache.at(self.data_revision());
            keys.iter().map(|k| cache.get(k)).collect()
        };
        let missing: Vec<usize> = (0..rules.len()).filter(|&i| out[i].is_none()).collect();
        if !missing.is_empty() {
            let want: Vec<ColumnFilter> = missing.iter().map(|&i| rules[i].clone()).collect();
            let found = self.each_rule_pass(&want, spec, progress)?;
            let room = CHIP_BYTES.saturating_sub(self.set_bytes());
            let mut cache = self.rule_cache.lock().unwrap_or_else(|p| p.into_inner());
            cache.at(self.data_revision());
            for (&i, bits) in missing.iter().zip(found) {
                let bits = Arc::new(bits);
                cache.put(keys[i].clone(), bits.clone(), room);
                out[i] = Some(bits);
            }
        }
        Ok(out.into_iter().map(|b| b.expect("found")).collect())
    }

    /// One pass over every row: each rule's PASS rows, set straight into
    /// one bit set per rule (no list of rows is kept on the way).
    fn each_rule_pass(
        &self,
        rules: &[ColumnFilter],
        spec: &FilterSpec,
        progress: &Progress,
    ) -> Result<Vec<Vec<u64>>, Error> {
        use std::sync::atomic::{AtomicU64, Ordering};
        let compiled = crate::filter::Compiled::new(rules, &self.read_ctx(spec.read))?;
        let total = self.total_rows();
        let first = usize::from(spec.keep_first && total > 0);
        let n = rules.len();
        let words = (self.next_row_id() as usize).div_ceil(64);
        let bits: Vec<Vec<AtomicU64>> = (0..n)
            .map(|_| (0..words).map(|_| AtomicU64::new(0)).collect())
            .collect();
        progress.start(total);
        let chunks = (total as usize).div_ceil(crate::filter::CHUNK);
        (0..chunks)
            .into_par_iter()
            .try_for_each(|c| -> Result<(), Error> {
                let s = c * crate::filter::CHUNK;
                let e = (s + crate::filter::CHUNK).min(total as usize);
                let mut buf = Vec::new();
                for pos in s.max(first)..e {
                    let id = self.doc_id_at(pos as u64).expect("in range");
                    self.read_cells(id, compiled.wanted(), &mut buf);
                    for (i, b) in bits.iter().enumerate() {
                        if compiled.passes_one(i, &buf) {
                            b[(id >> 6) as usize].fetch_or(1 << (id & 63), Ordering::Relaxed);
                        }
                    }
                }
                progress.step((e - s) as u64)?;
                Ok(())
            })?;
        Ok(bits
            .into_iter()
            .map(|b| b.into_iter().map(AtomicU64::into_inner).collect())
            .collect())
    }

    /// The column chips folded together (`spec` physical): PASS rows, or
    /// `None` with no column chip on. Rules whose results are cached are
    /// not read again; the others are read in one pass together.
    pub(crate) fn cols_fold(
        &self,
        spec: &FilterSpec,
        progress: &Progress,
    ) -> Result<Option<Arc<Vec<u64>>>, Error> {
        let on: Vec<ColumnFilter> = spec.columns.iter().filter(|c| c.enabled).cloned().collect();
        if on.is_empty() {
            return Ok(None);
        }
        let only_on = FilterSpec {
            columns: on,
            ..spec.clone()
        };
        let resolved = self.resolve_top(&only_on, progress)?;
        if resolved.columns.len() <= MAX_COUNTED {
            return self
                .cols_counted(spec, &resolved.columns, progress)
                .map(Some);
        }
        let each = self.rule_pass(spec, &resolved.columns, progress)?;
        let words = (self.next_row_id() as usize).div_ceil(64);
        let mut out = match spec.combine {
            Combine::And => vec![!0u64; words],
            Combine::Or => vec![0u64; words],
        };
        for (c, bits) in resolved.columns.iter().zip(&each) {
            for (w, o) in out.iter_mut().enumerate() {
                let b = word(bits, w);
                let b = if c.negate { !b } else { b };
                match spec.combine {
                    Combine::And => *o &= b,
                    Combine::Or => *o |= b,
                }
            }
        }
        Ok(Some(Arc::new(out)))
    }

    /// The column chips that are on (`on`, resolved), folded through the
    /// count of rules each row passes: the count kept from the last fold is
    /// brought to these rules by reading only the rules added or taken out
    /// (all of them when that is fewer, or after an edit).
    fn cols_counted(
        &self,
        spec: &FilterSpec,
        on: &[ColumnFilter],
        progress: &Progress,
    ) -> Result<Arc<Vec<u64>>, Error> {
        let keys: Vec<String> = on
            .iter()
            .map(|c| format!("{}{}", if c.negate { "!" } else { "" }, rule_key(c, spec)))
            .collect();
        let ids = self.next_row_id() as usize;
        let rev = self.data_revision();
        let ctx = serde_json::to_string(&(spec.read, spec.keep_first)).unwrap_or_default();
        let kept = {
            let mut cache = self.rule_cache.lock().unwrap_or_else(|p| p.into_inner());
            cache.at(rev);
            cache.count.take()
        };
        // What to add and take out, as multisets of keys.
        let mut add: Vec<usize> = Vec::new();
        let mut sub: Vec<String> = Vec::new();
        let mut per_row = match kept {
            Some(k) if k.per_row.len() == ids && k.ctx == ctx => {
                let mut left = k.keys.clone();
                for (i, key) in keys.iter().enumerate() {
                    match left.iter().position(|x| x == key) {
                        Some(j) => {
                            left.swap_remove(j);
                        }
                        None => add.push(i),
                    }
                }
                sub = left;
                if add.len() + sub.len() > keys.len() {
                    add = (0..keys.len()).collect();
                    sub.clear();
                    vec![0u8; ids]
                } else {
                    k.per_row
                }
            }
            _ => {
                add = (0..keys.len()).collect();
                vec![0u8; ids]
            }
        };
        // The rules to read (their own PASS rows, inverted here).
        let strip = |k: &str| -> (bool, String) {
            match k.strip_prefix('!') {
                Some(r) => (true, r.to_string()),
                None => (false, k.to_string()),
            }
        };
        let mut want: Vec<ColumnFilter> = add.iter().map(|&i| on[i].clone()).collect();
        let mut signs: Vec<(bool, i8)> = add.iter().map(|&i| (on[i].negate, 1)).collect();
        for k in &sub {
            let (neg, rk) = strip(k);
            let Some(c) = self.rule_from_key(&rk, spec) else {
                // A rule no longer known: count again from nothing.
                return self.cols_counted_fresh(spec, on, &keys, progress);
            };
            want.push(c);
            signs.push((neg, -1));
        }
        let each = self.rule_pass(spec, &want, progress)?;
        for (bits, (neg, sign)) in each.iter().zip(&signs) {
            per_row.par_iter_mut().enumerate().for_each(|(id, c)| {
                let b = has_bit(bits, id as u32) != *neg;
                if b {
                    *c = if *sign > 0 {
                        c.saturating_add(1)
                    } else {
                        c.saturating_sub(1)
                    };
                }
            });
        }
        let n = keys.len() as u8;
        let words = ids.div_ceil(64);
        let any = spec.combine == Combine::Or;
        let out: Vec<u64> = (0..words)
            .into_par_iter()
            .map(|w| {
                let mut word = 0u64;
                for b in 0..64 {
                    let id = w * 64 + b;
                    if id >= ids {
                        break;
                    }
                    let c = per_row[id];
                    if (any && c > 0) || (!any && c == n) {
                        word |= 1 << b;
                    }
                }
                word
            })
            .collect();
        let mut cache = self.rule_cache.lock().unwrap_or_else(|p| p.into_inner());
        if cache.rev == rev {
            cache.count = Some(PassCount { ctx, keys, per_row });
        }
        Ok(Arc::new(out))
    }

    fn cols_counted_fresh(
        &self,
        spec: &FilterSpec,
        on: &[ColumnFilter],
        _keys: &[String],
        progress: &Progress,
    ) -> Result<Arc<Vec<u64>>, Error> {
        {
            let mut cache = self.rule_cache.lock().unwrap_or_else(|p| p.into_inner());
            cache.count = None;
        }
        self.cols_counted(spec, on, progress)
    }

    /// The rule a cache key names (the key is the rule's JSON).
    fn rule_from_key(&self, key: &str, _spec: &FilterSpec) -> Option<ColumnFilter> {
        let v: serde_json::Value = serde_json::from_str(key).ok()?;
        let rule = v.get(1).cloned()?;
        let col = v.get(0).and_then(|c| c.as_u64())? as u32;
        let rule: crate::filter::Rule = serde_json::from_value(rule).ok()?;
        Some(ColumnFilter::new(col, rule))
    }

    /// The view the chips make: `spec` physical, its column chips folded
    /// into `cols` (rows below `cols_base`), and the tool chips.
    pub(crate) fn chip_view(
        &self,
        spec: FilterSpec,
        sets: Vec<SetChip>,
        cols: Option<Arc<Vec<u64>>>,
        cols_base: u32,
    ) -> View {
        let mut base = self.next_row_id();
        if cols.is_some() || !spec.columns.is_empty() {
            base = base.min(cols_base);
        }
        for s in &sets {
            base = base.min(s.base);
        }
        let words = (base as usize).div_ceil(64);
        let mut parts = Vec::new();
        if let Some(c) = &cols {
            parts.push(Part::Cols(c));
        }
        for s in sets.iter().filter(|s| s.enabled) {
            parts.push(Part::Set {
                set: &s.set,
                looked: &s.looked,
                negate: s.negate,
            });
        }
        let mut bits = match (&parts[..], spec.invert) {
            // The column chips alone: their rows as they are.
            ([Part::Cols(c)], false) => {
                let mut b = c.to_vec();
                b.resize(words, 0);
                b
            }
            _ => fold(&parts, spec.combine, spec.invert, words),
        };
        if spec.keep_first && self.total_rows() > 0 {
            if let Some(id) = self.doc_id_at(0) {
                set_bit(&mut bits, id);
            }
        }
        drop(parts);
        let mut v = View::new(spec, bits, base);
        v.sets = sets;
        v.cols = cols;
        v.cols_base = cols_base;
        v
    }

    /// Rows a view lets through (the title row too, hidden ones too).
    pub(crate) fn passing(&self, v: &View) -> u64 {
        if !self.rows_reordered() {
            // Ids are 0..rows: count the bits.
            let rows = self.total_rows() as u32;
            let below = rows.min(v.base);
            let full = (below / 64) as usize;
            let mut n: u64 = v.bits[..full.min(v.bits.len())]
                .iter()
                .map(|w| u64::from(w.count_ones()))
                .sum();
            if below % 64 != 0 {
                let w = word(&v.bits, full) & ((1u64 << (below % 64)) - 1);
                n += u64::from(w.count_ones());
            }
            return n + u64::from(rows - below);
        }
        let ids: Vec<u32> = self.ids().collect();
        ids.par_iter().filter(|&&id| v.passes(id)).count() as u64
    }

    /// Turn a tool's "show these rows" view into a chip beside the chips
    /// that are on.
    pub(crate) fn join_set(&self, v: View, making: ChipMaking) -> Result<View, Error> {
        let cur = self.view().filter(|c| c.filters());
        let base = v.base;
        let words = (base as usize).div_ceil(64);
        let mut set = v.bits;
        set.resize(words, 0);
        let title = (v.spec.keep_first && self.total_rows() > 0)
            .then(|| self.doc_id_at(0))
            .flatten();
        if let Some(t) = title {
            if let Some(w) = set.get_mut((t >> 6) as usize) {
                *w &= !(1u64 << (t & 63));
            }
        }
        let mut looked = vec![0u64; words];
        for id in self.ids() {
            if id >= base || Some(id) == title {
                continue;
            }
            let shown = match making.looked {
                Looked::All => true,
                Looked::Shown => self.view().is_none_or(|c| c.shows(id)),
            };
            if shown || has_bit(&set, id) {
                set_bit(&mut looked, id);
            }
        }
        let errors = making.errors.map(|mut e| {
            e.resize(words, 0);
            for (w, l) in looked.iter_mut().enumerate() {
                *l &= !e[w];
                set[w] &= !e[w];
            }
            Arc::new(e)
        });
        let chip = SetChip {
            kind: if making.kind.is_empty() {
                "rows".into()
            } else {
                making.kind
            },
            label: if making.label.is_empty() {
                "Rows a tool picked".into()
            } else {
                making.label
            },
            name: None,
            enabled: true,
            negate: making.negate,
            made_from: popcount(&looked),
            set: Arc::new(set),
            looked: Arc::new(looked),
            errors,
            error_count: making.error_count,
            base,
            made_rev: self.data_revision(),
            made_at: now_secs(),
        };
        let (mut spec, mut sets, cols, cols_base) = match cur {
            Some(c) => (c.spec.clone(), c.sets.clone(), c.cols.clone(), c.cols_base),
            None => (FilterSpec::default(), Vec::new(), None, base),
        };
        if sets.len() >= MAX_SET_CHIPS {
            return Err(Error::Invalid(format!(
                "a filter holds at most {MAX_SET_CHIPS} chips made by tools: remove one first (right-click a chip › Remove)"
            )));
        }
        let bytes: usize = sets.iter().map(SetChip::bytes).sum::<usize>() + chip.bytes();
        if bytes > CHIP_BYTES {
            return Err(Error::OutOfMemory(
                "the filter chips would take too much memory: remove some chips first",
            ));
        }
        // Rule results give way to the new chip.
        self.rule_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .shrink(CHIP_BYTES.saturating_sub(bytes));
        spec.keep_first |= v.spec.keep_first;
        sets.push(chip);
        Ok(self.chip_view(spec, sets, cols, cols_base))
    }

    /// Plan a change to the chips: no row is read again unless a column
    /// rule's result is not known (after an edit).
    pub fn plan_filter_change(
        &self,
        change: &ChipChange,
        progress: &Progress,
    ) -> Result<FilterPlan, Error> {
        self.ready_for_bulk()?;
        let Some(cur) = self.view().filter(|v| v.filters()) else {
            return Err(Error::Invalid("no filter is on".into()));
        };
        let mut spec = cur.spec.clone();
        let mut sets = cur.sets.clone();
        let mut cols = cur.cols.clone();
        let mut cols_base = cur.cols_base;
        let n = spec.columns.len();
        let count = n + sets.len();
        let at = |i: usize| -> Result<(), Error> {
            if i < count {
                Ok(())
            } else {
                Err(Error::Invalid("that chip is gone".into()))
            }
        };
        let mut refold = false;
        match change {
            ChipChange::Toggle { index }
            | ChipChange::Negate { index }
            | ChipChange::Remove { index } => {
                at(*index)?;
                let i = *index;
                if i < n {
                    match change {
                        ChipChange::Toggle { .. } => spec.columns[i].enabled ^= true,
                        ChipChange::Negate { .. } => spec.columns[i].negate ^= true,
                        _ => {
                            spec.columns.remove(i);
                        }
                    }
                    refold = true;
                } else {
                    let s = i - n;
                    match change {
                        ChipChange::Toggle { .. } => sets[s].enabled ^= true,
                        ChipChange::Negate { .. } => sets[s].negate ^= true,
                        _ => {
                            sets.remove(s);
                        }
                    }
                }
            }
            ChipChange::Rename { index, name } => {
                at(*index)?;
                let name = name
                    .as_ref()
                    .map(|n| n.trim().chars().take(80).collect::<String>())
                    .filter(|n| !n.is_empty());
                if *index < n {
                    spec.columns[*index].name = name;
                } else {
                    sets[*index - n].name = name;
                }
            }
            ChipChange::Combine { combine } => {
                refold = spec.combine != *combine;
                spec.combine = *combine;
            }
            ChipChange::Invert { on } => spec.invert = *on,
            ChipChange::ShowErrors { index } => {
                at(*index)?;
                let s = index
                    .checked_sub(n)
                    .and_then(|s| sets.get(s).map(|c| (s, c)))
                    .filter(|(_, c)| c.errors.is_some())
                    .ok_or_else(|| Error::Invalid("that chip has no error rows".into()))?;
                if sets.len() >= MAX_SET_CHIPS {
                    return Err(Error::Invalid(format!(
                        "a filter holds at most {MAX_SET_CHIPS} chips made by tools: remove one first"
                    )));
                }
                let (k, c) = s;
                let errors = c.errors.clone().expect("checked");
                let chip = SetChip {
                    kind: "errors".into(),
                    label: format!("Errors of {}", c.label),
                    name: None,
                    enabled: true,
                    negate: false,
                    looked: errors.clone(),
                    made_from: popcount(&errors),
                    set: errors,
                    errors: None,
                    error_count: 0,
                    ..c.clone()
                };
                sets[k].enabled = false;
                sets.push(chip);
            }
        }
        if refold {
            cols = self.cols_fold(&spec, progress)?;
            cols_base = self.next_row_id();
        }
        let view = if spec.columns.is_empty() && sets.is_empty() {
            None
        } else {
            Some(self.chip_view(spec, sets, cols, cols_base))
        };
        let shown = match &view {
            Some(v) => self.passing(v),
            None => self.total_rows(),
        };
        Ok(FilterPlan::of(self.revision(), view, shown))
    }

    /// Change the chips (one undo step); the rows that now show.
    pub fn change_filter(&mut self, change: &ChipChange) -> Result<u64, Error> {
        let plan = self.plan_filter_change(change, &Progress::new())?;
        let shown = plan.shown;
        self.apply_filter(plan)?;
        Ok(shown)
    }

    /// The chips of the filter that is on (empty without one).
    pub fn filter_chips(&self) -> FilterChips {
        let Some(v) = self.view().filter(|v| v.filters()) else {
            return FilterChips::default();
        };
        let total_rows = self.total_rows();
        let title = u64::from(v.spec.keep_first && total_rows > 0);
        let data = total_rows - title;
        let logical = |p: u32| match self.col_map() {
            Some(m) => m.iter().position(|&x| x == p).map(|i| i as u32),
            None => Some(p),
        };
        // A rule read only with the others (a plain first filter) is read
        // once more on its own here, for its count, and kept for switching.
        // A failure (stopped, changed) leaves the count unknown.
        let _ = self.rule_pass(&v.spec, &v.spec.columns, &Progress::new());
        let mut chips = Vec::new();
        {
            let mut cache = self.rule_cache.lock().unwrap_or_else(|p| p.into_inner());
            cache.at(self.data_revision());
            for c in &v.spec.columns {
                let rows = cache.get(&rule_key(c, &v.spec)).map(|b| {
                    let p = popcount(&b).min(data);
                    if c.negate {
                        data - p
                    } else {
                        p
                    }
                });
                chips.push(ChipInfo {
                    kind: "column".into(),
                    col: logical(c.col),
                    label: String::new(),
                    name: c.name.clone(),
                    enabled: c.enabled,
                    negate: c.negate,
                    rows,
                    looked: None,
                    not_looked: None,
                    errors: 0,
                    stale: false,
                    made_at: 0,
                });
            }
        }
        for s in &v.sets {
            let looked = s.made_from;
            let picked: u64 = s
                .set
                .iter()
                .zip(s.looked.iter())
                .map(|(a, b)| u64::from((a & b).count_ones()))
                .sum();
            chips.push(ChipInfo {
                kind: s.kind.clone(),
                col: None,
                label: s.label.clone(),
                name: s.name.clone(),
                enabled: s.enabled,
                negate: s.negate,
                rows: Some(if s.negate { looked - picked } else { picked }),
                looked: Some(looked),
                not_looked: Some(data.saturating_sub(looked + s.error_count)),
                errors: s.error_count,
                stale: s.made_rev != self.data_revision(),
                made_at: s.made_at,
            });
        }
        let shown = self.passing(v).saturating_sub(title);
        FilterChips {
            all_off: chips.iter().all(|c| !c.enabled),
            sets_full: v.sets.len() >= MAX_SET_CHIPS,
            chips,
            combine: v.spec.combine,
            invert: v.spec.invert,
            shown,
            total: data,
        }
    }
}

#[cfg(test)]
#[path = "chips_tests.rs"]
mod tests;

/// The cache a document keeps.
pub(crate) fn new_cache() -> Mutex<RuleCache> {
    Mutex::new(RuleCache::default())
}
