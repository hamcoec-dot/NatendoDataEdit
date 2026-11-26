//! Combine with a column map: how each file's columns line up in the combined
//! table, shown before combining and changeable by hand, with suggestions and a
//! mapping that can be saved (titles only) for next month's files.
//!
//! [`auto_plan`] is exactly what Combine did before (columns by title through
//! [`TitleUnion`], or by place), so nothing changes for anyone who does not
//! change the map. Suggestions are never applied by themselves.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::document::{Document, Progress};
use crate::tools::TitleUnion;
use crate::{Error, OpenOptions};

/// Bytes of each file read for its head (the title row and the preview).
pub const HEAD_BYTES: usize = 1 << 20;
/// Data rows of each file shown in the preview.
pub const PREVIEW_ROWS: usize = 20;
/// Largest saved mapping read.
pub const MAX_MAPPING_BYTES: usize = 1 << 20;
/// Most output columns (as the combined table may have).
pub const MAX_OUTPUT_COLS: usize = crate::reshape::MAX_PIVOT_COLS;
/// Sample values shown beside a suggestion, per side.
pub const SAMPLES: usize = 3;

fn bad(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

/// The first rows of a file, read without opening it fully.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHead {
    /// The file name (never the folder).
    pub name: String,
    /// The title row (blank titles when the file has none), as wide as `cols`.
    pub titles: Vec<String>,
    /// The widest row of the sample.
    pub cols: u32,
    /// About how many rows the file has (from the sample's bytes per row).
    pub rows_est: u64,
    /// The whole file was in the sample (so `rows_est` is exact).
    pub whole: bool,
    /// The first data rows.
    pub preview: Vec<Vec<String>>,
}

/// Read the head of a file: at most [`HEAD_BYTES`], cut at the last full line,
/// detected as the file will be opened.
pub fn head_of(path: &Path, header: bool) -> Result<FileHead, Error> {
    let size = std::fs::metadata(path)?.len();
    let mut bytes = Vec::with_capacity(HEAD_BYTES.min(size as usize));
    std::fs::File::open(path)?
        .take(HEAD_BYTES as u64)
        .read_to_end(&mut bytes)?;
    if crate::crypt::is_encrypted(&bytes) {
        return Err(bad("this is an encrypted file: open it in a tab first"));
    }
    let whole = bytes.len() as u64 >= size;
    if !whole {
        if let Some(i) = bytes.iter().rposition(|&b| b == b'\n') {
            bytes.truncate(i + 1);
        }
    }
    let sampled = bytes.len().max(1) as u64;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    let d = Document::from_bytes(bytes, OpenOptions::default(), ext.as_deref())?;
    let cols = d.col_count();
    let rows = d.row_count();
    let mut titles = if header && rows > 0 {
        d.row_values(0)?
    } else {
        Vec::new()
    };
    titles.resize(cols as usize, String::new());
    let first = u64::from(header);
    let preview = (first..rows.min(first + PREVIEW_ROWS as u64))
        .map(|r| {
            let mut v = d.row_values(r)?;
            v.resize(cols as usize, String::new());
            Ok(v)
        })
        .collect::<Result<_, Error>>()?;
    let rows_est = if whole {
        rows
    } else {
        (rows as u128 * size as u128 / sampled as u128) as u64
    };
    Ok(FileHead {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        titles,
        cols,
        rows_est,
        whole,
        preview,
    })
}

/// Where each column of each file goes in the combined table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombinePlan {
    /// The output columns' titles.
    pub titles: Vec<String>,
    /// For each file, for each of its columns: the output column, or `None`
    /// (left out).
    pub files: Vec<Vec<Option<u32>>>,
}

impl CombinePlan {
    /// Refuse a plan that does not fit files of these widths, or that puts two
    /// columns of one file on one output column (one would overwrite the other).
    pub fn check(&self, widths: &[usize]) -> Result<(), Error> {
        if self.files.len() != widths.len() {
            return Err(bad("the column map is for another set of files"));
        }
        if self.titles.is_empty() || self.titles.len() > MAX_OUTPUT_COLS {
            return Err(bad(format!(
                "the combined table needs 1 to {MAX_OUTPUT_COLS} columns"
            )));
        }
        let n = self.titles.len() as u32;
        for (f, (map, &w)) in self.files.iter().zip(widths).enumerate() {
            if map.len() != w {
                return Err(bad(format!(
                    "file {} has {w} columns, not the {} the column map was made for",
                    f + 1,
                    map.len()
                )));
            }
            let mut used = HashSet::with_capacity(map.len());
            for to in map.iter().flatten() {
                if *to >= n {
                    return Err(bad("the column map names a column that is not there"));
                }
                if !used.insert(*to) {
                    return Err(bad(format!(
                        "two columns of file {} go to \"{}\": one would overwrite the other",
                        f + 1,
                        self.titles[*to as usize]
                    )));
                }
            }
        }
        Ok(())
    }

    /// Output columns no file feeds are taken out (the others renumbered).
    pub fn compact(&mut self) {
        let mut fed = vec![false; self.titles.len()];
        for to in self.files.iter().flatten().flatten() {
            if let Some(f) = fed.get_mut(*to as usize) {
                *f = true;
            }
        }
        if fed.iter().all(|&f| f) {
            return;
        }
        let mut new = vec![None; self.titles.len()];
        let mut titles = Vec::new();
        for (i, t) in self.titles.drain(..).enumerate() {
            if fed[i] {
                new[i] = Some(titles.len() as u32);
                titles.push(t);
            }
        }
        self.titles = titles;
        for to in self.files.iter_mut().flatten() {
            *to = to.and_then(|t| new.get(t as usize).copied().flatten());
        }
    }

    /// Move every column feeding `merge` onto `keep` (files that feed both are
    /// refused), then take out the empty column.
    pub fn merge(&mut self, keep: u32, merge: u32) -> Result<(), Error> {
        if keep == merge
            || keep as usize >= self.titles.len()
            || merge as usize >= self.titles.len()
        {
            return Err(bad("choose two columns to merge"));
        }
        for map in &self.files {
            if map.contains(&Some(keep)) && map.contains(&Some(merge)) {
                return Err(bad(
                    "a file has both columns: merging them would put two columns of one file together",
                ));
            }
        }
        for to in self.files.iter_mut().flatten() {
            if *to == Some(merge) {
                *to = Some(keep);
            }
        }
        self.compact();
        Ok(())
    }
}

/// Today's Combine: by title ([`TitleUnion`]) or by place.
pub fn auto_plan(title_rows: &[Vec<String>], by_title: bool) -> CombinePlan {
    let mut union = TitleUnion::default();
    let mut titles: Vec<String> = Vec::new();
    let mut files = Vec::with_capacity(title_rows.len());
    for row0 in title_rows {
        if by_title {
            files.push(
                union
                    .add(row0)
                    .into_iter()
                    .map(|i| Some(i as u32))
                    .collect(),
            );
        } else {
            for (c, t) in row0.iter().enumerate().skip(titles.len()) {
                titles.push(if t.is_empty() {
                    format!("Column {}", c + 1)
                } else {
                    t.clone()
                });
            }
            files.push((0..row0.len() as u32).map(Some).collect());
        }
    }
    if by_title {
        titles = union.into_titles();
    }
    CombinePlan { titles, files }
}

/// A title with case, spaces and `_ - .` left out: `Cust_ID`, `cust id` and
/// `CustID` are the same after tidying.
pub fn tidy(t: &str) -> String {
    t.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-' | '.'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// A tidied title with every run of digits as one `#`: titles equal this way
/// differ only in their numbers (`Address Line 1` / `Address Line 2`).
fn numbers_out(t: &str) -> String {
    let mut s = String::new();
    let mut digit = false;
    for c in t.chars() {
        if c.is_ascii_digit() {
            if !digit {
                s.push('#');
            }
            digit = true;
        } else {
            s.push(c);
            digit = false;
        }
    }
    s
}

/// Why two output columns may be one column under two titles.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Why {
    /// The same after tidying (case, spaces, `_ - .`).
    Tidied,
    /// A similar title in about the same place.
    Similar { score: f32 },
}

/// Two output columns that are probably one: merge `merge` into `keep`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub keep: u32,
    pub merge: u32,
    pub why: Why,
    /// A few values from each side (from the files' first rows), to see
    /// whether they look like the same thing.
    pub keep_samples: Vec<String>,
    pub merge_samples: Vec<String>,
}

/// Similar-title suggestions are looked for among at most this many columns.
const MAX_SIMILAR_COLS: usize = 1_000;

/// Suggested merges for `plan` over files with these heads. Only pairs no
/// file feeds both of, so a merge can never put two columns of a file on one.
pub fn suggest(plan: &CombinePlan, heads: &[FileHead]) -> Vec<Suggestion> {
    let n = plan.titles.len();
    // Which files feed each output, and from which place.
    let mut feeds: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (f, map) in plan.files.iter().enumerate() {
        for (c, to) in map.iter().enumerate() {
            if let Some(to) = to {
                if let Some(v) = feeds.get_mut(*to as usize) {
                    v.push((f, c));
                }
            }
        }
    }
    let apart = |a: usize, b: usize| {
        !feeds[a]
            .iter()
            .any(|(fa, _)| feeds[b].iter().any(|(fb, _)| fa == fb))
    };
    let samples = |o: usize| -> Vec<String> {
        let mut out = Vec::new();
        for &(f, c) in &feeds[o] {
            if let Some(h) = heads.get(f) {
                for row in &h.preview {
                    if let Some(v) = row.get(c).filter(|v| !v.trim().is_empty()) {
                        if !out.contains(v) {
                            out.push(v.clone());
                        }
                    }
                    if out.len() >= SAMPLES {
                        return out;
                    }
                }
            }
        }
        out
    };
    let tidied: Vec<String> = plan.titles.iter().map(|t| tidy(t)).collect();
    let mut out = Vec::new();
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    // Same after tidying.
    let mut by_tidy: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, t) in tidied.iter().enumerate() {
        if !t.is_empty() {
            by_tidy.entry(t.as_str()).or_default().push(i);
        }
    }
    let mut groups: Vec<&Vec<usize>> = by_tidy.values().filter(|v| v.len() > 1).collect();
    groups.sort();
    for g in groups {
        for (k, &a) in g.iter().enumerate() {
            for &b in &g[k + 1..] {
                if apart(a, b) && seen.insert((a, b)) {
                    out.push((a, b, Why::Tidied));
                }
            }
        }
    }
    // A similar title in about the same place, never one that differs only in
    // its numbers.
    if n <= MAX_SIMILAR_COLS {
        for a in 0..n {
            for b in a + 1..n {
                let (ta, tb) = (&tidied[a], &tidied[b]);
                if ta.is_empty() || tb.is_empty() || ta == tb || seen.contains(&(a, b)) {
                    continue;
                }
                if numbers_out(ta) == numbers_out(tb) {
                    continue;
                }
                let near = feeds[a]
                    .iter()
                    .any(|(_, ca)| feeds[b].iter().any(|(_, cb)| ca.abs_diff(*cb) <= 2));
                if !near || !apart(a, b) {
                    continue;
                }
                let score = crate::fuzzy::jaro_winkler(ta, tb);
                if score >= 0.9 {
                    seen.insert((a, b));
                    out.push((a, b, Why::Similar { score }));
                }
            }
        }
    }
    out.into_iter()
        .map(|(a, b, why)| Suggestion {
            keep: a as u32,
            merge: b as u32,
            why,
            keep_samples: samples(a),
            merge_samples: samples(b),
        })
        .collect()
}

/// What a file's columns are called for matching: the trimmed, lower-case
/// title, with a repeated title numbered in order as [`TitleUnion`] names it
/// (`note`, `note (2)`); `None` for a blank title.
fn keys_of(titles: &[String]) -> Vec<Option<String>> {
    names_of(titles)
        .into_iter()
        .map(|n| n.map(|n| n.to_lowercase()))
        .collect()
}

/// The same, keeping the title's own case: `Note`, `Note (2)`.
fn names_of(titles: &[String]) -> Vec<Option<String>> {
    let mut used: HashMap<String, u32> = HashMap::new();
    titles
        .iter()
        .map(|t| {
            let name = t.trim();
            if name.is_empty() {
                return None;
            }
            let n = used.entry(name.to_lowercase()).or_insert(0);
            *n += 1;
            Some(if *n == 1 {
                name.to_string()
            } else {
                format!("{name} ({n})")
            })
        })
        .collect()
}

/// A saved column map (`.ndcombine`): titles only, never paths, file names or
/// values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedMapping {
    pub kind: String,
    pub version: u32,
    /// The program that wrote it (name and version).
    pub tool: String,
    /// The files start with a title row (a mapping is only for those).
    pub header: bool,
    /// The combined table starts with a column holding each row's file name.
    #[serde(default)]
    pub file_name_column: bool,
    /// What happens to a file's column the mapping does not name.
    pub extras: Extras,
    pub columns: Vec<SavedColumn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extras {
    /// Added as a column of its own, at the end.
    Add,
    /// Left out.
    LeaveOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedColumn {
    pub title: String,
    /// Other titles that go to this column.
    #[serde(default)]
    pub from: Vec<String>,
}

pub const MAPPING_KIND: &str = "natendo-combine";
pub const MAPPING_VERSION: u32 = 1;
/// Longest title kept in a mapping, and most other titles per column.
const MAX_TITLE_CHARS: usize = 1_000;
const MAX_FROM: usize = 64;

impl SavedMapping {
    /// The mapping of `plan` over files with these title rows: each output's
    /// title, and the other titles mapped onto it.
    pub fn of_plan(
        plan: &CombinePlan,
        title_rows: &[Vec<String>],
        extras: Extras,
        file_name_column: bool,
    ) -> SavedMapping {
        let mut from: Vec<Vec<String>> = vec![Vec::new(); plan.titles.len()];
        for (map, titles) in plan.files.iter().zip(title_rows) {
            let names = names_of(titles);
            for (c, to) in map.iter().enumerate() {
                let (Some(to), Some(Some(t))) = (to, names.get(c)) else {
                    continue;
                };
                let own = plan.titles[*to as usize].trim().to_lowercase();
                let key = t.to_lowercase();
                if key != own && !from[*to as usize].iter().any(|x| x.to_lowercase() == key) {
                    from[*to as usize].push(t.clone());
                }
            }
        }
        SavedMapping {
            kind: MAPPING_KIND.into(),
            version: MAPPING_VERSION,
            tool: format!("Natendo Data Edit {}", crate::version()),
            header: true,
            file_name_column,
            extras,
            columns: plan
                .titles
                .iter()
                .zip(from)
                .map(|(t, mut f)| {
                    f.sort();
                    SavedColumn {
                        title: t.clone(),
                        from: f,
                    }
                })
                .collect(),
        }
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    /// Read and check a saved mapping.
    pub fn from_json(bytes: &[u8]) -> Result<SavedMapping, Error> {
        let v: Value = crate::hashing::parse_strict(bytes, MAX_MAPPING_BYTES)
            .map_err(|_| bad("this is not a column map Natendo can read"))?;
        if v.get("kind").and_then(Value::as_str) != Some(MAPPING_KIND) {
            return Err(bad("this is not a column map Natendo can read"));
        }
        let m: SavedMapping = serde_json::from_value(v)
            .map_err(|e| bad(format!("this column map cannot be read: {e}")))?;
        if m.version > MAPPING_VERSION {
            return Err(bad(
                "this column map was saved by a newer version of the app",
            ));
        }
        if !m.header {
            return Err(bad(
                "a column map is for files that start with a row of titles",
            ));
        }
        if m.columns.is_empty() || m.columns.len() > MAX_OUTPUT_COLS {
            return Err(bad(format!(
                "a column map holds 1 to {MAX_OUTPUT_COLS} columns"
            )));
        }
        for c in &m.columns {
            if c.title.trim().is_empty()
                || c.from.len() > MAX_FROM
                || std::iter::once(&c.title)
                    .chain(&c.from)
                    .any(|t| t.chars().count() > MAX_TITLE_CHARS)
            {
                return Err(bad("this column map has a column it cannot use"));
            }
        }
        Ok(m)
    }

    /// The plan this mapping gives for files with these title rows, and what
    /// did not fit: [`Fit::problems`] (a file column that matches two outputs,
    /// or two columns of one file that match one output) are never guessed.
    pub fn apply(&self, title_rows: &[Vec<String>]) -> Fit {
        let mut titles: Vec<String> = self.columns.iter().map(|c| c.title.clone()).collect();
        let mut by_key: HashMap<String, Vec<u32>> = HashMap::new();
        for (i, c) in self.columns.iter().enumerate() {
            let mut keys: Vec<String> = std::iter::once(&c.title)
                .chain(&c.from)
                .map(|t| t.trim().to_lowercase())
                .collect();
            keys.dedup();
            for k in keys {
                let v = by_key.entry(k).or_default();
                if !v.contains(&(i as u32)) {
                    v.push(i as u32);
                }
            }
        }
        let mut fit = Fit::default();
        let mut extra_at: HashMap<String, u32> = HashMap::new();
        for (f, row) in title_rows.iter().enumerate() {
            let keys = keys_of(row);
            let mut map: Vec<Option<u32>> = vec![None; row.len()];
            let mut taken: HashMap<u32, usize> = HashMap::new();
            for (c, key) in keys.iter().enumerate() {
                let found = key.as_ref().and_then(|k| by_key.get(k));
                match found.map(Vec::as_slice) {
                    Some([to]) => {
                        if let Some(&other) = taken.get(to) {
                            fit.problems.push(Problem {
                                file: f,
                                columns: vec![other as u32, c as u32],
                                what: format!(
                                    "two of its columns, \"{}\" and \"{}\", both go to \"{}\"",
                                    row[other], row[c], titles[*to as usize]
                                ),
                            });
                            map[other] = None;
                        } else {
                            taken.insert(*to, c);
                            map[c] = Some(*to);
                        }
                    }
                    Some(many) if many.len() > 1 => fit.problems.push(Problem {
                        file: f,
                        columns: vec![c as u32],
                        what: format!(
                            "its column \"{}\" matches {} columns of the map",
                            row[c],
                            many.len()
                        ),
                    }),
                    _ => {
                        fit.extras.push((f, c as u32));
                        if self.extras == Extras::Add {
                            let name = if row[c].trim().is_empty() {
                                format!("Column {}", c + 1)
                            } else {
                                row[c].clone()
                            };
                            let k = key.clone().unwrap_or_else(|| format!("\u{0}{c}"));
                            let to = *extra_at.entry(k).or_insert_with(|| {
                                titles.push(name);
                                (titles.len() - 1) as u32
                            });
                            if taken.insert(to, c).is_none() {
                                map[c] = Some(to);
                            }
                        }
                    }
                }
            }
            for (i, _) in self.columns.iter().enumerate() {
                if !map.contains(&Some(i as u32)) {
                    fit.missing.push((f, i as u32));
                }
            }
            fit.plan.files.push(map);
        }
        fit.plan.titles = titles;
        fit
    }
}

/// What applying a saved mapping gave.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fit {
    pub plan: CombinePlan,
    /// (file, output column) the file has nothing for: blank there.
    pub missing: Vec<(usize, u32)>,
    /// (file, column) the mapping does not name.
    pub extras: Vec<(usize, u32)>,
    pub problems: Vec<Problem>,
}

/// Something a saved mapping cannot decide for a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub file: usize,
    pub columns: Vec<u32>,
    pub what: String,
}

/// Check, after a file is opened fully, that it still is what its head
/// showed (M1): the same title row, and no row wider than the head's.
pub fn still_fits(
    head: &FileHead,
    d: &Document,
    header: bool,
    progress: &Progress,
) -> Result<(), Error> {
    let _ = progress;
    if d.col_count() > head.cols {
        return Err(bad(format!(
            "{} has rows wider ({} columns) than its first rows showed ({}): look at the column map again",
            head.name,
            d.col_count(),
            head.cols
        )));
    }
    let mut titles = if header && d.row_count() > 0 {
        d.row_values(0)?
    } else {
        Vec::new()
    };
    titles.resize(head.cols as usize, String::new());
    if header && titles != head.titles {
        return Err(bad(format!(
            "{}'s titles are not the ones the column map was made from: look at the column map again",
            head.name
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "combine_tests.rs"]
mod tests;
