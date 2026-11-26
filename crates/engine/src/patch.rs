//! Portable patches (`.ndpatch`): the differences between two copies of a
//! table, by key, saved so they can be applied to another copy.
//!
//! A patch **holds data**: old and new values and whole rows. It is sealed
//! with a SHA-256 (tamper-evident against accidents and edits, not a
//! signature), may be encrypted with a phrase, and never overwrites a value
//! that is neither the old one nor already the new one.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::hashing::{check_seal, parse_strict, seal, SealKind};
use crate::join::{norm_key, JoinSpec};
use crate::privacy::Secret;
use crate::reshape::{push_line, too_big, RowPlan, Table, MAX_TABLE_BYTES};
use crate::updates::pair_columns;
use crate::Error;

pub const PATCH_VERSION: u32 = 1;
/// Largest patch file read or written.
pub const MAX_PATCH_BYTES: usize = 256 << 20;
/// Changes, additions and removals together.
pub const MAX_PATCH_ENTRIES: usize = 1_000_000;
const MAX_PATCH_COLS: usize = 16_384;
const MAX_PATCH_KEYS: usize = 16;
/// Longest value, in bytes.
const MAX_VALUE_BYTES: usize = 4 << 20;
/// Report lines kept (the counts stay exact).
pub const MAX_PATCH_LINES: usize = 10_000;
const SEAL: &str = "patch_sha256";
/// Rows between looks at Cancel.
const STEP: usize = 1 << 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub key: Vec<String>,
    /// (column of the patch, old value, new value).
    pub cells: Vec<(u32, String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remove {
    pub key: Vec<String>,
    /// The old row, in the patch's columns.
    pub row: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub version: u32,
    /// The app that made it.
    pub tool: String,
    /// The titles of the columns that identify a row (each in `columns`).
    pub keys: Vec<String>,
    /// The titles of the compared columns.
    pub columns: Vec<String>,
    pub match_case: bool,
    pub trim: bool,
    pub changes: Vec<Change>,
    /// New rows, in the patch's columns.
    pub adds: Vec<Vec<String>>,
    pub removes: Vec<Remove>,
}

/// How a patch is made.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatchSpec {
    /// Key columns of the older table.
    pub keys: Vec<u32>,
    pub match_case: bool,
    pub trim: bool,
    /// Leave columns only one table has out of the patch (else refused).
    pub leave_out: bool,
}

fn bad(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

fn rules(match_case: bool, trim: bool) -> JoinSpec {
    JoinSpec {
        match_case,
        trim,
        ..Default::default()
    }
}

/// A key under the patch's rules; `None` when every part is blank.
fn key_of<S: AsRef<str>>(parts: &[S], rules: &JoinSpec) -> Option<String> {
    let parts: Vec<&str> = parts.iter().map(AsRef::as_ref).collect();
    let mut out = String::new();
    norm_key(&parts, rules, &[], &mut out).then_some(out)
}

impl Patch {
    /// The columns of `columns` that are the keys, in key order.
    fn key_columns(&self) -> Result<Vec<usize>, Error> {
        let mut used = Vec::new();
        for k in &self.keys {
            let at = self
                .columns
                .iter()
                .enumerate()
                .position(|(i, t)| t == k && !used.contains(&i))
                .ok_or_else(|| bad("a key column of the patch is not among its columns"))?;
            used.push(at);
        }
        Ok(used)
    }

    pub fn entries(&self) -> usize {
        self.changes.len() + self.adds.len() + self.removes.len()
    }

    /// Check sizes, places and that no key is in the patch twice (a patch
    /// file is untrusted).
    pub fn check(&self) -> Result<(), Error> {
        if self.version != PATCH_VERSION {
            return Err(bad(
                "this patch was made by a newer version of Natendo Data Edit",
            ));
        }
        let width = self.columns.len();
        if self.keys.is_empty() || self.keys.len() > MAX_PATCH_KEYS {
            return Err(bad("a patch has 1 to 16 key columns"));
        }
        if width == 0 || width > MAX_PATCH_COLS {
            return Err(bad("the patch has no columns, or too many"));
        }
        if self.entries() > MAX_PATCH_ENTRIES {
            return Err(bad("the patch holds too many changes"));
        }
        let long = |v: &[String]| v.iter().any(|s| s.len() > MAX_VALUE_BYTES);
        if long(&self.columns) || self.tool.len() > 200 {
            return Err(bad("a text in the patch is too long"));
        }
        let keys = self.key_columns()?;
        let rules = rules(self.match_case, self.trim);
        let mut seen = std::collections::HashSet::with_capacity(self.entries());
        let mut one = |parts: &[String]| -> Result<(), Error> {
            let key = key_of(parts, &rules).ok_or_else(|| bad("the patch has a blank key"))?;
            if !seen.insert(key) {
                return Err(bad("the patch holds the same key twice"));
            }
            Ok(())
        };
        let broken = || bad("an entry of the patch does not fit its columns");
        for c in &self.changes {
            if c.key.len() != keys.len() || c.cells.is_empty() || long(&c.key) {
                return Err(broken());
            }
            let mut cols = std::collections::HashSet::new();
            for (i, old, new) in &c.cells {
                let i = *i as usize;
                if i >= width
                    || keys.contains(&i)
                    || !cols.insert(i)
                    || old.len() > MAX_VALUE_BYTES
                    || new.len() > MAX_VALUE_BYTES
                {
                    return Err(broken());
                }
            }
            one(&c.key)?;
        }
        for r in &self.removes {
            if r.key.len() != keys.len() || r.row.len() != width || long(&r.key) || long(&r.row) {
                return Err(broken());
            }
            one(&r.key)?;
        }
        for row in &self.adds {
            if row.len() != width || long(row) {
                return Err(broken());
            }
            let parts: Vec<&String> = keys.iter().map(|&k| &row[k]).collect();
            one(&parts.into_iter().cloned().collect::<Vec<_>>())?;
        }
        Ok(())
    }

    /// The patch as file bytes: sealed, and with `phrase` encrypted.
    pub fn to_bytes(&self, phrase: Option<&Secret>) -> Result<Vec<u8>, Error> {
        self.check()?;
        let mut value = serde_json::to_value(self).map_err(|e| bad(e.to_string()))?;
        let map = value.as_object_mut().expect("a struct");
        seal(map, SealKind::Patch, SEAL)?;
        let bytes = serde_json::to_vec(&value).map_err(|e| bad(e.to_string()))?;
        if bytes.len() > MAX_PATCH_BYTES {
            return Err(bad("the differences are too large for a patch file"));
        }
        match phrase {
            Some(p) => crate::crypt::encrypt(&bytes, p),
            None => Ok(bytes),
        }
    }

    /// Read a patch file. Its seal is checked before any entry is read.
    pub fn from_bytes(data: &[u8], phrase: Option<&Secret>) -> Result<Patch, Error> {
        let plain;
        let data = if crate::crypt::is_encrypted(data) {
            let p = phrase.ok_or_else(|| bad("this patch is protected: type its phrase"))?;
            plain = crate::crypt::decrypt(data, p)?;
            &plain[..]
        } else {
            data
        };
        let mut value = parse_strict(data, MAX_PATCH_BYTES)?;
        let map = value
            .as_object_mut()
            .ok_or_else(|| bad("this is not a patch file"))?;
        if !check_seal(map, SealKind::Patch, SEAL) {
            return Err(bad(
                "this is not a patch file, or it was changed or damaged after it was made",
            ));
        }
        map.remove(SEAL);
        let patch: Patch = serde_json::from_value(value)
            .map_err(|e| bad(format!("this is not a patch file Natendo can read: {e}")))?;
        patch.check()?;
        Ok(patch)
    }
}

/// A made patch, and the columns it had to leave out.
#[derive(Debug, Clone)]
pub struct Made {
    pub patch: Patch,
    pub left_out: Vec<String>,
}

/// (key, row id) in order, and each key's place in that list.
type Index = (Vec<(String, u32)>, HashMap<String, usize>);

/// Every data row of a table by key, refusing blanks and repeats.
fn index(
    d: &Document,
    cols: &[u32],
    rules: &JoinSpec,
    which: &str,
    progress: &Progress,
) -> Result<Index, Error> {
    let reader = crate::ops::Cols::new(d, cols);
    let mut rows = Vec::new();
    let mut at = HashMap::new();
    let mut buf = Vec::new();
    for (n, id) in d.ids().enumerate().skip(1) {
        reader.read(d, id, &mut buf);
        let parts: Vec<&str> = (0..cols.len()).map(|i| reader.get(&buf, i)).collect();
        // The row is named by its place, never by its key.
        let key = key_of(&parts, rules).ok_or_else(|| {
            bad(format!(
                "row {} of the {which} table has a blank key",
                n + 1
            ))
        })?;
        if at.insert(key.clone(), rows.len()).is_some() {
            return Err(bad(format!(
                "row {} of the {which} table repeats an earlier key: a patch needs each key once",
                n + 1
            )));
        }
        rows.push((key, id));
        if n % STEP == 0 {
            progress.step(STEP as u64)?;
        }
    }
    Ok((rows, at))
}

impl Document {
    /// The differences from this (older) table to `newer`, over every row
    /// (hidden ones too). Both tables need a title row.
    pub fn make_patch(
        &self,
        newer: &Document,
        spec: &PatchSpec,
        progress: &Progress,
    ) -> Result<Made, Error> {
        self.ready_for_bulk()?;
        newer.ready_for_bulk()?;
        if spec.keys.is_empty() || spec.keys.len() > MAX_PATCH_KEYS {
            return Err(bad("choose 1 to 16 columns that identify a row"));
        }
        if self.row_count() == 0 || newer.row_count() == 0 {
            return Err(bad("both tables need a title row"));
        }
        let titles = |d: &Document| d.row_values(0).unwrap_or_default();
        let (old_titles, new_titles) = (titles(self), titles(newer));
        let pairs = pair_columns(&old_titles, &new_titles, true);
        // The patch's columns: those both have, in this table's order.
        let mut here = Vec::new();
        let mut there = Vec::new();
        let mut left_out = Vec::new();
        for (c, p) in pairs.iter().enumerate() {
            match p {
                Some(n) => {
                    here.push(c as u32);
                    there.push(*n);
                }
                None => left_out.push(old_titles[c].clone()),
            }
        }
        left_out.extend(
            (0..new_titles.len() as u32)
                .filter(|n| !there.contains(n))
                .map(|n| new_titles[n as usize].clone()),
        );
        if !left_out.is_empty() && !spec.leave_out {
            return Err(bad(format!(
                "only one table has these columns: {}. Leave them out of the patch, or add them to the other table first",
                left_out.join(", ")
            )));
        }
        let key_at: Vec<usize> = spec
            .keys
            .iter()
            .map(|k| {
                here.iter()
                    .position(|c| c == k)
                    .ok_or_else(|| bad("a key column is not in both tables"))
            })
            .collect::<Result<_, _>>()?;
        let rules = rules(spec.match_case, spec.trim);
        progress.start(2 * (self.row_count() + newer.row_count()));
        let pick = |cols: &[u32]| key_at.iter().map(|&k| cols[k]).collect::<Vec<_>>();
        let (old_rows, old_at) = index(self, &pick(&here), &rules, "older", progress)?;
        let (new_rows, new_at) = index(newer, &pick(&there), &rules, "newer", progress)?;
        let mut patch = Patch {
            version: PATCH_VERSION,
            tool: format!("Natendo Data Edit {}", crate::version()),
            keys: key_at
                .iter()
                .map(|&k| old_titles[here[k] as usize].clone())
                .collect(),
            columns: here
                .iter()
                .map(|&c| old_titles[c as usize].clone())
                .collect(),
            match_case: spec.match_case,
            trim: spec.trim,
            changes: Vec::new(),
            adds: Vec::new(),
            removes: Vec::new(),
        };
        let old_read = crate::ops::Cols::new(self, &here);
        let new_read = crate::ops::Cols::new(newer, &there);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let mut bytes = 0usize;
        let mut room = |patch: &Patch, more: usize| -> Result<(), Error> {
            bytes += more;
            if patch.entries() >= MAX_PATCH_ENTRIES || bytes > MAX_PATCH_BYTES {
                return Err(bad("there are too many differences for one patch file"));
            }
            Ok(())
        };
        let row = |read: &crate::ops::Cols, buf: &[std::borrow::Cow<'_, str>]| -> Vec<String> {
            (0..here.len())
                .map(|i| read.get(buf, i).to_string())
                .collect()
        };
        for (n, (key, id)) in old_rows.iter().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            old_read.read(self, *id, &mut a);
            let text: Vec<String> = key_at
                .iter()
                .map(|&k| old_read.get(&a, k).to_string())
                .collect();
            match new_at.get(key) {
                None => {
                    let old = row(&old_read, &a);
                    room(&patch, old.iter().map(String::len).sum::<usize>() + 32)?;
                    patch.removes.push(Remove {
                        key: text,
                        row: old,
                    });
                }
                Some(&j) => {
                    new_read.read(newer, new_rows[j].1, &mut b);
                    let cells: Vec<(u32, String, String)> = (0..here.len())
                        .filter(|i| !key_at.contains(i))
                        .filter(|&i| old_read.get(&a, i) != new_read.get(&b, i))
                        .map(|i| {
                            (
                                i as u32,
                                old_read.get(&a, i).to_string(),
                                new_read.get(&b, i).to_string(),
                            )
                        })
                        .collect();
                    if !cells.is_empty() {
                        let size: usize = cells.iter().map(|c| c.1.len() + c.2.len() + 16).sum();
                        room(&patch, size)?;
                        patch.changes.push(Change { key: text, cells });
                    }
                }
            }
        }
        for (n, (key, id)) in new_rows.iter().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            if !old_at.contains_key(key) {
                new_read.read(newer, *id, &mut b);
                let new = row(&new_read, &b);
                room(&patch, new.iter().map(String::len).sum::<usize>() + 8)?;
                patch.adds.push(new);
            }
        }
        Ok(Made { patch, left_out })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    WillApply,
    AlreadyApplied,
    /// Neither the old value nor the new one is there: left alone.
    Conflict,
    KeyNotFound,
    KeyTwice,
    /// A protected column: left alone.
    Protected,
    MissingColumn,
}

impl Outcome {
    pub fn words(self) -> &'static str {
        match self {
            Outcome::WillApply => "will apply",
            Outcome::AlreadyApplied => "already applied",
            Outcome::Conflict => "conflict: left alone",
            Outcome::KeyNotFound => "key not found",
            Outcome::KeyTwice => "key found more than once",
            Outcome::Protected => "protected column: left alone",
            Outcome::MissingColumn => "column not in this table",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Change,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PatchLine {
    pub outcome: Outcome,
    pub kind: EntryKind,
    pub key: Vec<String>,
    /// The changed column's title (changes only).
    pub column: String,
}

/// What applying a patch would do: counts of changed cells, added rows and
/// removed rows by outcome, and the first lines.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PatchReport {
    pub will_apply: u64,
    pub already: u64,
    pub conflicts: u64,
    pub not_found: u64,
    pub twice: u64,
    pub protected: u64,
    pub missing_column: u64,
    pub cells: u64,
    pub added: u64,
    pub removed: u64,
    /// Columns of the patch this table does not have.
    pub missing: Vec<String>,
    /// Columns here the patch does not look at (added rows get them blank).
    pub extra: Vec<String>,
    pub keys: Vec<String>,
    pub lines: Vec<PatchLine>,
    pub lines_total: u64,
}

impl PatchReport {
    fn line(&mut self, outcome: Outcome, kind: EntryKind, key: &[String], column: &str) {
        match outcome {
            Outcome::WillApply => {
                self.will_apply += 1;
                match kind {
                    EntryKind::Change => self.cells += 1,
                    EntryKind::Add => self.added += 1,
                    EntryKind::Remove => self.removed += 1,
                }
            }
            Outcome::AlreadyApplied => self.already += 1,
            Outcome::Conflict => self.conflicts += 1,
            Outcome::KeyNotFound => self.not_found += 1,
            Outcome::KeyTwice => self.twice += 1,
            Outcome::Protected => self.protected += 1,
            Outcome::MissingColumn => self.missing_column += 1,
        }
        self.lines_total += 1;
        if self.lines.len() < MAX_PATCH_LINES {
            self.lines.push(PatchLine {
                outcome,
                kind,
                key: key.to_vec(),
                column: column.to_string(),
            });
        }
    }

    /// Entries that could not be applied.
    pub fn unapplied(&self) -> u64 {
        self.conflicts + self.not_found + self.twice + self.protected + self.missing_column
    }

    /// The lines as a table: what happens, which kind, the key, the column.
    pub fn table(&self) -> Result<Table, Error> {
        let mut head = vec!["Outcome".to_string(), "Kind".to_string()];
        head.extend(self.keys.iter().cloned());
        head.push("Column".into());
        let cols = head.len();
        let mut out = Vec::new();
        push_line(&mut out, &head);
        for l in &self.lines {
            let kind = match l.kind {
                EntryKind::Change => "change",
                EntryKind::Add => "add",
                EntryKind::Remove => "remove",
            };
            let mut cells = vec![l.outcome.words().to_string(), kind.to_string()];
            cells.extend(l.key.iter().cloned());
            cells.push(l.column.clone());
            push_line(&mut out, &cells);
            if out.len() > MAX_TABLE_BYTES {
                return Err(too_big());
            }
        }
        Ok(Table {
            bytes: out,
            rows: self.lines.len() as u64 + 1,
            cols: cols as u32,
        })
    }
}

impl Document {
    /// What applying `patch` to every row (hidden ones too) would do, and
    /// the plan that does it as one undo step ([`Document::apply_rows`]).
    pub fn plan_patch(
        &self,
        patch: &Patch,
        progress: &Progress,
    ) -> Result<(RowPlan, PatchReport), Error> {
        self.ready_for_bulk()?;
        patch.check()?;
        if self.row_count() == 0 {
            return Err(bad("this table has no title row to find the columns by"));
        }
        let titles = self.row_values(0).unwrap_or_default();
        // For each patch column, this table's column.
        let place = pair_columns(&patch.columns, &titles, true);
        let keys = patch.key_columns()?;
        let key_cols: Vec<u32> = keys
            .iter()
            .map(|&k| {
                place[k].ok_or_else(|| {
                    bad(format!(
                        "this table has no column titled \"{}\", which the patch finds rows by",
                        patch.columns[k]
                    ))
                })
            })
            .collect::<Result<_, _>>()?;
        let mut report = PatchReport {
            keys: patch.keys.clone(),
            missing: (0..place.len())
                .filter(|&i| place[i].is_none())
                .map(|i| patch.columns[i].clone())
                .collect(),
            extra: (0..titles.len() as u32)
                .filter(|c| !place.contains(&Some(*c)))
                .map(|c| titles[c as usize].clone())
                .collect(),
            ..Default::default()
        };
        let rules = rules(patch.match_case, patch.trim);
        // The keys the patch asks about, then where each is in this table:
        // (rows found, the first of them).
        let added_key =
            |row: &[String]| -> Vec<String> { keys.iter().map(|&k| row[k].clone()).collect() };
        let mut wanted: HashMap<String, (u32, u32)> = HashMap::with_capacity(patch.entries());
        let all = patch
            .changes
            .iter()
            .map(|c| c.key.clone())
            .chain(patch.removes.iter().map(|r| r.key.clone()))
            .chain(patch.adds.iter().map(|r| added_key(r)));
        for k in all {
            wanted.insert(key_of(&k, &rules).expect("checked"), (0, 0));
        }
        let reader = crate::ops::Cols::new(self, &key_cols);
        let mut buf = Vec::new();
        let mut key = String::new();
        progress.start(self.row_count() + patch.entries() as u64);
        for (n, id) in self.ids().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            if n == 0 {
                continue;
            }
            reader.read(self, id, &mut buf);
            let parts: Vec<&str> = (0..key_cols.len()).map(|i| reader.get(&buf, i)).collect();
            if norm_key(&parts, &rules, &[], &mut key) {
                if let Some(found) = wanted.get_mut(&key) {
                    if found.0 == 0 {
                        found.1 = id;
                    }
                    found.0 = found.0.saturating_add(1);
                }
            }
        }
        // The patch's columns that are here, to compare whole rows.
        let present: Vec<(usize, u32)> = (0..place.len())
            .filter_map(|i| place[i].map(|c| (i, c)))
            .collect();
        let cols: Vec<u32> = present.iter().map(|p| p.1).collect();
        let whole = crate::ops::Cols::new(self, &cols);
        let same_row = |id: u32, row: &[String], buf: &mut Vec<_>| {
            whole.read(self, id, buf);
            present
                .iter()
                .enumerate()
                .all(|(n, &(i, _))| whole.get(buf, n) == row[i])
        };
        let all_here = present.len() == patch.columns.len();
        let find = |key: &[String]| wanted[&key_of(key, &rules).expect("checked")];
        let mut plan = RowPlan::new(self);
        for (n, c) in patch.changes.iter().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            let (times, id) = find(&c.key);
            let mut cells = Vec::new();
            for (i, old, new) in &c.cells {
                let i = *i as usize;
                let title = &patch.columns[i];
                let outcome = match (place[i], times) {
                    (None, _) => Outcome::MissingColumn,
                    (_, 0) => Outcome::KeyNotFound,
                    (_, 2..) => Outcome::KeyTwice,
                    (Some(col), 1) => {
                        let one = crate::ops::Cols::new(self, &[col]);
                        one.read(self, id, &mut buf);
                        let now = one.get(&buf, 0);
                        if now == new {
                            Outcome::AlreadyApplied
                        } else if now != old {
                            Outcome::Conflict
                        } else if self.is_protected(col) {
                            Outcome::Protected
                        } else {
                            cells.push((col, new.clone()));
                            Outcome::WillApply
                        }
                    }
                };
                report.line(outcome, EntryKind::Change, &c.key, title);
            }
            if !cells.is_empty() {
                plan.changes.push((id, cells));
            }
        }
        for (n, r) in patch.removes.iter().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            let (times, id) = find(&r.key);
            let outcome = match times {
                // Gone already.
                0 => Outcome::AlreadyApplied,
                2.. => Outcome::KeyTwice,
                _ if !all_here => Outcome::MissingColumn,
                _ if !same_row(id, &r.row, &mut buf) => Outcome::Conflict,
                _ => {
                    plan.remove.push(id);
                    Outcome::WillApply
                }
            };
            report.line(outcome, EntryKind::Remove, &r.key, "");
        }
        let mut new_rows = Vec::new();
        for (n, row) in patch.adds.iter().enumerate() {
            if n % STEP == 0 {
                progress.step(STEP as u64)?;
            }
            let k = added_key(row);
            let (times, id) = find(&k);
            let outcome = match times {
                2.. => Outcome::KeyTwice,
                _ if !all_here => Outcome::MissingColumn,
                1 if same_row(id, row, &mut buf) => Outcome::AlreadyApplied,
                1 => Outcome::Conflict,
                0 => {
                    // This table's columns; those the patch lacks are blank.
                    let mut out = vec![String::new(); self.col_count() as usize];
                    for &(i, col) in &present {
                        out[col as usize] = row[i].clone();
                    }
                    new_rows.push(out);
                    Outcome::WillApply
                }
            };
            report.line(outcome, EntryKind::Add, &k, "");
        }
        if !new_rows.is_empty() {
            // After the table's last row that stays (the title row at least).
            let gone: HashSet<u32> = plan.remove.iter().copied().collect();
            let last = self.ids().filter(|id| !gone.contains(id)).last();
            plan.after.push((last.ok_or(Error::OutOfRange)?, new_rows));
        }
        plan.rows = report.added + report.removed + plan.changes.len() as u64;
        Ok((plan, report))
    }
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod tests;
