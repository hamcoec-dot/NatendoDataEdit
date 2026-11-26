//! Portable patches (`.ndpatch`): the differences between two copies of a
//! table, by key, saved so they can be applied to another copy.
//!
//! A patch **holds data**: old and new values and whole rows. It is sealed
//! with a SHA-256 (tamper-evident against accidents and edits, not a
//! signature), may be encrypted with a phrase, and never overwrites a value
//! that is neither the old one nor already the new one.

use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::export::{write_atomic, ExportReport};
use crate::hashing::{check_seal, parse_strict, seal, SealKind};
use crate::join::{norm_key, JoinSpec};
use crate::ops::{Cols, CHUNK};
use crate::privacy::Secret;
use crate::reshape::{push_line, too_big, RowPlan, Table, MAX_TABLE_BYTES};
use crate::scan::guard_formula;
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
    /// The file's seal (checked before the patch is read; empty otherwise).
    #[serde(default, rename = "patch_sha256", skip_serializing)]
    seal: String,
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

    /// Write the patch to `path` (a temp file beside it, moved into place
    /// when complete).
    pub fn save(&self, path: &Path, phrase: Option<&Secret>) -> Result<(), Error> {
        let bytes = self.to_bytes(phrase)?;
        write_atomic(path, |w| {
            w.write_all(&bytes)?;
            Ok(ExportReport::default())
        })?;
        Ok(())
    }

    /// [`Patch::from_bytes`] of a file.
    pub fn load(path: &Path, phrase: Option<&Secret>) -> Result<Patch, Error> {
        Patch::from_bytes(&read_patch(path)?, phrase)
    }

    /// Whether the file needs a phrase.
    pub fn is_protected(path: &Path) -> Result<bool, Error> {
        let mut head = Vec::new();
        std::fs::File::open(path)?.take(64).read_to_end(&mut head)?;
        Ok(crate::crypt::is_encrypted(&head))
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
        drop(value);
        // Only the place is reported: serde's own words quote the value it
        // could not use, which is somebody's data.
        let mut patch: Patch = serde_json::from_slice(data).map_err(|e| {
            bad(format!(
                "this is not a patch file Natendo can read (line {}, column {})",
                e.line(),
                e.column()
            ))
        })?;
        patch.seal.clear();
        patch.check()?;
        Ok(patch)
    }
}

/// A patch file's bytes, refused when larger than any patch.
fn read_patch(path: &Path) -> Result<Vec<u8>, Error> {
    // Room for what encrypting adds.
    let most = MAX_PATCH_BYTES as u64 + (1 << 20);
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(most + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > most {
        return Err(bad("this file is too large to be a patch"));
    }
    Ok(data)
}

/// A made patch, and the columns it had to leave out.
#[derive(Debug, Clone)]
pub struct Made {
    pub patch: Patch,
    pub left_out: Vec<String>,
}

/// One table's data rows with their keys under the patch's rules, read in
/// parallel into one text block per chunk of rows.
struct Keys {
    ids: Vec<u32>,
    /// The chunk's keys, and where each ends.
    chunks: Vec<(String, Vec<u32>)>,
}

/// Every row's key to its place in [`Keys::ids`], in shards.
struct KeyMap<'a> {
    hasher: RandomState,
    shards: Vec<HashMap<&'a str, u32>>,
}

const SHARDS: usize = 64;

fn shard(hasher: &RandomState, key: &str) -> usize {
    (hasher.hash_one(key) >> 58) as usize % SHARDS
}

impl Keys {
    fn key(&self, i: usize) -> &str {
        let (text, ends) = &self.chunks[i / CHUNK];
        let k = i % CHUNK;
        let from = if k == 0 { 0 } else { ends[k - 1] as usize };
        &text[from..ends[k] as usize]
    }

    /// Read the keys of every data row; a blank one is kept as "".
    fn read(
        d: &Document,
        cols: &[u32],
        rules: &JoinSpec,
        progress: &Progress,
    ) -> Result<Keys, Error> {
        let ids: Vec<u32> = d.ids().skip(1).collect();
        let reader = Cols::new(d, cols);
        let chunks = ids
            .par_chunks(CHUNK)
            .map(|chunk| {
                progress.step(chunk.len() as u64)?;
                let (mut text, mut ends) = (String::new(), Vec::with_capacity(chunk.len()));
                let (mut buf, mut key) = (Vec::new(), String::new());
                for &id in chunk {
                    reader.read(d, id, &mut buf);
                    let mut row: Vec<&str> = Vec::with_capacity(cols.len());
                    row.extend((0..cols.len()).map(|i| reader.get(&buf, i)));
                    if norm_key(&row, rules, &[], &mut key) {
                        text.push_str(&key);
                    }
                    ends.push(u32::try_from(text.len()).map_err(|_| too_big())?);
                }
                Ok((text, ends))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Keys { ids, chunks })
    }

    /// The map by key, refusing blank and repeated keys: the first such row
    /// is named by its place, never by its key.
    fn map(&self, which: &str, progress: &Progress) -> Result<KeyMap<'_>, Error> {
        let hasher = RandomState::new();
        // Each chunk's rows by shard, in order, and its first blank key.
        let split: Vec<(Vec<Vec<u32>>, Option<usize>)> = (0..self.chunks.len())
            .into_par_iter()
            .map(|c| {
                let first = c * CHUNK;
                let mut by = vec![Vec::new(); SHARDS];
                let mut blank = None;
                for i in first..first + self.chunks[c].1.len() {
                    let key = self.key(i);
                    if key.is_empty() {
                        blank = blank.or(Some(i));
                    } else {
                        by[shard(&hasher, key)].push(i as u32);
                    }
                }
                (by, blank)
            })
            .collect();
        progress.step(0)?;
        let built: Vec<(HashMap<&str, u32>, Option<usize>)> = (0..SHARDS)
            .into_par_iter()
            .map(|s| {
                let size = split.iter().map(|c| c.0[s].len()).sum();
                let mut map = HashMap::with_capacity(size);
                let mut repeat = None;
                for &i in split.iter().flat_map(|c| &c.0[s]) {
                    if map.insert(self.key(i as usize), i).is_some() {
                        repeat = Some(i as usize);
                        break;
                    }
                }
                (map, repeat)
            })
            .collect();
        progress.step(0)?;
        let blank = split.iter().filter_map(|c| c.1).min();
        let repeat = built.iter().filter_map(|b| b.1).min();
        // Data row `i` is row `i + 2` of the table.
        match (blank, repeat) {
            (Some(b), r) if r.is_none_or(|r| b < r) => Err(bad(format!(
                "row {} of the {which} table has a blank key",
                b + 2
            ))),
            (_, Some(r)) => Err(bad(format!(
                "row {} of the {which} table repeats an earlier key: a patch needs each key once",
                r + 2
            ))),
            _ => Ok(KeyMap {
                shards: built.into_iter().map(|b| b.0).collect(),
                hasher,
            }),
        }
    }
}

impl KeyMap<'_> {
    fn get(&self, key: &str) -> Option<u32> {
        self.shards[shard(&self.hasher, key)].get(key).copied()
    }
}

/// The rows of a table that differ, from one chunk.
#[derive(Default)]
struct Differ {
    changes: Vec<Change>,
    removes: Vec<Remove>,
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
        progress.start(2 * (self.total_rows() + newer.total_rows()));
        let pick = |cols: &[u32]| key_at.iter().map(|&k| cols[k]).collect::<Vec<_>>();
        let old_keys = Keys::read(self, &pick(&here), &rules, progress)?;
        let old_at = old_keys.map("older", progress)?;
        let new_keys = Keys::read(newer, &pick(&there), &rules, progress)?;
        let new_at = new_keys.map("newer", progress)?;
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
            seal: String::new(),
        };
        let old_read = Cols::new(self, &here);
        let new_read = Cols::new(newer, &there);
        let (entries, bytes) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let room = |more: usize| -> Result<(), Error> {
            if entries.fetch_add(1, Ordering::Relaxed) >= MAX_PATCH_ENTRIES
                || bytes.fetch_add(more, Ordering::Relaxed) + more > MAX_PATCH_BYTES
            {
                return Err(bad("there are too many differences for one patch file"));
            }
            Ok(())
        };
        let row = |read: &Cols, buf: &[std::borrow::Cow<'_, str>]| -> Vec<String> {
            (0..here.len())
                .map(|i| read.get(buf, i).to_string())
                .collect()
        };
        let compared: Vec<usize> = (0..here.len()).filter(|i| !key_at.contains(i)).collect();
        let parts = (0..old_keys.chunks.len())
            .into_par_iter()
            .map(|c| {
                let first = c * CHUNK;
                let end = first + old_keys.chunks[c].1.len();
                progress.step((end - first) as u64)?;
                let mut out = Differ::default();
                let (mut a, mut b) = (Vec::new(), Vec::new());
                for i in first..end {
                    old_read.read(self, old_keys.ids[i], &mut a);
                    let text = || -> Vec<String> {
                        key_at
                            .iter()
                            .map(|&k| old_read.get(&a, k).to_string())
                            .collect()
                    };
                    let Some(j) = new_at.get(old_keys.key(i)) else {
                        let old = row(&old_read, &a);
                        room(old.iter().map(String::len).sum::<usize>() + 32)?;
                        out.removes.push(Remove {
                            key: text(),
                            row: old,
                        });
                        continue;
                    };
                    new_read.read(newer, new_keys.ids[j as usize], &mut b);
                    let cells: Vec<(u32, String, String)> = compared
                        .iter()
                        .filter(|&&i| old_read.get(&a, i) != new_read.get(&b, i))
                        .map(|&i| {
                            (
                                i as u32,
                                old_read.get(&a, i).to_string(),
                                new_read.get(&b, i).to_string(),
                            )
                        })
                        .collect();
                    if !cells.is_empty() {
                        room(cells.iter().map(|c| c.1.len() + c.2.len() + 16).sum())?;
                        out.changes.push(Change { key: text(), cells });
                    }
                }
                Ok(out)
            })
            .collect::<Result<Vec<Differ>, Error>>()?;
        for part in parts {
            patch.changes.extend(part.changes);
            patch.removes.extend(part.removes);
        }
        let adds = (0..new_keys.chunks.len())
            .into_par_iter()
            .map(|c| {
                let first = c * CHUNK;
                let end = first + new_keys.chunks[c].1.len();
                progress.step((end - first) as u64)?;
                let mut out = Vec::new();
                let mut b = Vec::new();
                for i in first..end {
                    if old_at.get(new_keys.key(i)).is_none() {
                        new_read.read(newer, new_keys.ids[i], &mut b);
                        let new = row(&new_read, &b);
                        room(new.iter().map(String::len).sum::<usize>() + 8)?;
                        out.push(new);
                    }
                }
                Ok(out)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        patch.adds = adds.into_iter().flatten().collect();
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

    /// Write the lines as CSV, with texts a spreadsheet would run as a
    /// formula guarded.
    pub fn save_csv(&self, path: &Path) -> Result<(), Error> {
        let bytes = self.lines_table(true)?.bytes;
        write_atomic(path, |w| {
            w.write_all(&bytes)?;
            Ok(ExportReport::default())
        })?;
        Ok(())
    }

    /// The lines as a table: what happens, which kind, the key, the column.
    pub fn table(&self) -> Result<Table, Error> {
        self.lines_table(false)
    }

    fn lines_table(&self, guard: bool) -> Result<Table, Error> {
        let safe = |s: &str| -> String {
            if guard {
                guard_formula(s).into_owned()
            } else {
                s.to_string()
            }
        };
        let mut head = vec!["Outcome".to_string(), "Kind".to_string()];
        head.extend(self.keys.iter().map(|k| safe(k)));
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
            cells.extend(l.key.iter().map(|k| safe(k)));
            cells.push(safe(&l.column));
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
        let mut wanted: HashMap<String, ()> = HashMap::with_capacity(patch.entries());
        let all = patch
            .changes
            .iter()
            .map(|c| c.key.clone())
            .chain(patch.removes.iter().map(|r| r.key.clone()))
            .chain(patch.adds.iter().map(|r| added_key(r)));
        for k in all {
            wanted.insert(key_of(&k, &rules).expect("checked"), ());
        }
        let reader = Cols::new(self, &key_cols);
        progress.start(self.total_rows() + patch.entries() as u64);
        let ids: Vec<u32> = self.ids().collect();
        // Rows whose key the patch asks about, in file order.
        let seen = ids
            .get(1..)
            .unwrap_or_default()
            .par_chunks(CHUNK)
            .map(|chunk| {
                progress.step(chunk.len() as u64)?;
                let mut out = Vec::new();
                let (mut buf, mut key) = (Vec::new(), String::new());
                for &id in chunk {
                    reader.read(self, id, &mut buf);
                    let parts: Vec<&str> =
                        (0..key_cols.len()).map(|i| reader.get(&buf, i)).collect();
                    if norm_key(&parts, &rules, &[], &mut key) {
                        if let Some((k, _)) = wanted.get_key_value(&key) {
                            out.push((k.as_str(), id));
                        }
                    }
                }
                Ok(out)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut found: HashMap<&str, (u32, u32)> = HashMap::with_capacity(wanted.len());
        for (key, id) in seen.into_iter().flatten() {
            let f = found.entry(key).or_insert((0, id));
            f.0 = f.0.saturating_add(1);
        }
        // The patch's columns that are here, to compare whole rows.
        let present: Vec<(usize, u32)> = (0..place.len())
            .filter_map(|i| place[i].map(|c| (i, c)))
            .collect();
        let cols: Vec<u32> = present.iter().map(|p| p.1).collect();
        let whole = Cols::new(self, &cols);
        let same_row = |id: u32, row: &[String], buf: &mut Vec<_>| {
            whole.read(self, id, buf);
            present
                .iter()
                .enumerate()
                .all(|(n, &(i, _))| whole.get(buf, n) == row[i])
        };
        let all_here = present.len() == patch.columns.len();
        let find = |key: &[String]| -> (u32, u32) {
            let key = key_of(key, &rules).expect("checked");
            found.get(key.as_str()).copied().unwrap_or_default()
        };
        let mut plan = RowPlan::new(self);
        let mut buf = Vec::new();
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
                        let one = Cols::new(self, &[col]);
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
            let last = ids.iter().rev().find(|id| !gone.contains(id)).copied();
            plan.after.push((last.ok_or(Error::OutOfRange)?, new_rows));
        }
        plan.rows = report.added + report.removed + plan.changes.len() as u64;
        Ok((plan, report))
    }
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod tests;
