//! Key-aware semantic diff of two tables (Tools > Compare with another file, "By key"): rows are
//! paired by key columns picked by title, columns by title, the same way "take changes from a newer file"
//! does it (`updates.rs`: `NewerIndex`, `pair_newer`), so there is only one keying code. The result is a
//! table of what differs, exact counts, warnings (repeated and empty keys, columns in only one file) and an
//! index that draws the differences on the source tabs and on the result tab: each shown row's key is looked
//! up in the index when a window is drawn, so the marks follow any sort, filter or delete, with no list of
//! row numbers to keep right.

use std::collections::HashMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::Cols;
use crate::reshape::{push_line, Table, MAX_TABLE_BYTES};
use crate::updates::{key_hash, NewerIndex, NewerValues, Pairing, UpdateSpec};
use crate::Error;

/// Rows listed in the result table (the counts are always exact).
pub const MAX_LISTED_ROWS: usize = 1_000_000;
/// Rows of a window `diff_marks` looks at (the grid's own limit).
const MAX_WINDOW: u64 = 1_000;
/// Rows per parallel task.
const CHUNK: usize = 4096;
/// Repeated or empty keys named in the warning.
const WARN_KEYS: usize = 10;

/// What to compare.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffSpec {
    /// Key columns of the first table (found in the second by title when both have a title row, else by
    /// position).
    pub keys: Vec<u32>,
    /// Both tables start with a title row.
    pub keep_first: bool,
    pub match_case: bool,
    pub trim: bool,
}

impl DiffSpec {
    fn updates(&self) -> UpdateSpec {
        UpdateSpec {
            keys: self.keys.clone(),
            keep_first: self.keep_first,
            match_case: self.match_case,
            trim: self.trim,
        }
    }
}

/// Counts, warnings and the column report of a diff.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiffReport {
    pub same: u64,
    pub changed: u64,
    /// In the second table only.
    pub added: u64,
    /// In the first table only.
    pub removed: u64,
    /// Changed cells (in the listed rows).
    pub cells: u64,
    /// Rows in the result table.
    pub listed: u64,
    /// More rows differ than are listed.
    pub truncated: bool,
    /// Columns (by title) only in the first / second table.
    pub only_a: Vec<String>,
    pub only_b: Vec<String>,
    /// Columns found under the same title at another place.
    pub moved: u32,
    /// Keys that appear more than once in either table (paired in order).
    pub repeated: u64,
    /// The first repeated keys of each table, as written.
    pub dup_a: Vec<String>,
    pub dup_b: Vec<String>,
    /// Rows with an empty key (never paired).
    pub empty_a: u64,
    pub empty_b: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Changed,
    Added,
    Removed,
}

#[derive(Debug, Clone)]
struct Entry {
    kind: Kind,
    /// The changed columns, in the numbering of the table the entry is kept for.
    cols: Box<[u32]>,
}

/// Which table a window of marks is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    A,
    B,
    Result,
}

/// A difference on screen: `col` is `None` for a whole row. `kind`: 0 changed cell, 1 added row,
/// 2 removed row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DiffMark {
    pub row: u64,
    pub col: Option<u32>,
    pub kind: u8,
}

/// The index that draws a diff on its tables. Kept (by the shell) while the tabs are open.
pub struct DiffIndex {
    spec: DiffSpec,
    keys_a: Vec<u32>,
    keys_b: Vec<u32>,
    a_side: HashMap<u128, Entry>,
    b_side: HashMap<u128, Entry>,
    a_rev: u64,
    b_rev: u64,
    /// The result table's data revision, set once it exists.
    pub result_rev: Option<u64>,
}

/// A row of the first table read for the result (values, key and its number in the sheet).
pub struct RowA {
    number: u64,
    vals: Vec<String>,
    keys: Vec<String>,
}

/// What step 2 (the first table) leaves for step 3 (the second).
pub struct DiffFirst {
    pairing: Pairing,
    titles_a: Vec<String>,
    cols_a: u32,
    total_a: u64,
    removed: Vec<RowA>,
    changed: Vec<RowA>,
    empty_a: u64,
    dup_a: Vec<String>,
    rev: u64,
}

/// What a diff gives back.
pub struct DiffOutcome {
    pub table: Table,
    pub report: DiffReport,
    pub index: DiffIndex,
}

/// The result table's first columns before the first table's own.
pub const RESULT_LEAD: u32 = 3;

impl DiffIndex {
    /// Whether the table's rows are as they were when the diff was made (a sort only reorders them, and
    /// the shell moves the number along for it; any other change makes the marks out of date).
    pub fn current(&self, side: Side, data_revision: u64) -> bool {
        match side {
            Side::A => self.a_rev == data_revision,
            Side::B => self.b_rev == data_revision,
            Side::Result => self.result_rev.is_none_or(|r| r == data_revision),
        }
    }

    /// The table's rows were only put in another order: the index follows.
    pub fn follow_order(&mut self, side: Side, data_revision: u64) {
        match side {
            Side::A => self.a_rev = data_revision,
            Side::B => self.b_rev = data_revision,
            Side::Result => self.result_rev = Some(data_revision),
        }
    }

    /// The key columns of a table of `side`.
    fn key_cols(&self, side: Side) -> Vec<u32> {
        match side {
            Side::A => self.keys_a.clone(),
            Side::B => self.keys_b.clone(),
            Side::Result => self.keys_a.iter().map(|k| k + RESULT_LEAD).collect(),
        }
    }
}

fn key_parts(ra: &RowA) -> Vec<&str> {
    ra.keys.iter().map(String::as_str).collect()
}

fn is_blank(parts: &[&str], trim: bool) -> bool {
    parts.iter().all(|p| if trim { p.trim().is_empty() } else { p.is_empty() })
}

/// Empty keys, repeated keys and the first few repeated keys (as written) of one table. Reads only the key
/// columns; the repeated keys are looked for only when `look_for_repeats`.
fn scan_keys(
    doc: &Document,
    keys: &[u32],
    spec: &DiffSpec,
    look_for_repeats: bool,
    progress: &Progress,
) -> Result<(u64, Vec<String>), Error> {
    let rows = doc.data_rows(spec.keep_first);
    let reader = Cols::new(doc, keys);
    let n = rows.end.saturating_sub(rows.start);
    progress.start(n);
    type Part = (u64, Vec<(u128, u32)>);
    let parts: Vec<Part> = (0..n.div_ceil(CHUNK as u64))
        .into_par_iter()
        .map(|k| -> Result<Part, Error> {
            let s = rows.start + k * CHUNK as u64;
            let e = (s + CHUNK as u64).min(rows.end);
            let mut empty = 0u64;
            let mut hashes = Vec::new();
            let mut buf = Vec::new();
            for r in s..e {
                let Some(id) = doc.id_at(r) else { continue };
                reader.read(doc, id, &mut buf);
                let parts: Vec<&str> = (0..keys.len()).map(|i| reader.get(&buf, i)).collect();
                if is_blank(&parts, spec.trim) {
                    empty += 1;
                } else if look_for_repeats {
                    hashes.push((key_hash(&parts, spec.match_case, spec.trim), id));
                }
            }
            progress.step(e - s)?;
            Ok((empty, hashes))
        })
        .collect::<Result<_, _>>()?;
    let empty = parts.iter().map(|p| p.0).sum();
    let mut dups = Vec::new();
    if look_for_repeats {
        let mut all: Vec<(u128, u32)> = parts.into_iter().flat_map(|p| p.1).collect();
        all.par_sort_unstable();
        let mut at = 0;
        let mut buf = Vec::new();
        while at + 1 < all.len() && dups.len() < WARN_KEYS {
            if all[at].0 == all[at + 1].0 {
                reader.read(doc, all[at].1, &mut buf);
                let text: Vec<&str> = (0..keys.len()).map(|i| reader.get(&buf, i)).collect();
                dups.push(text.join(" | "));
                while at + 1 < all.len() && all[at].0 == all[at + 1].0 {
                    at += 1;
                }
            }
            at += 1;
        }
    }
    Ok((empty, dups))
}

impl Document {
    /// Compare this table (the first) with `other` (the second) by key, in one call (both readable here).
    /// The shell runs the same three steps one table at a time ([`Document::diff_index`] on the second,
    /// [`Document::diff_first`] on the first, [`Document::diff_second`] on the second), so no call ever
    /// holds both tables' locks; comparing a table with itself works either way.
    pub fn diff_files(
        &self,
        other: &Document,
        spec: &DiffSpec,
        progress: &Progress,
    ) -> Result<DiffOutcome, Error> {
        let titles = self.diff_titles(spec);
        let index = Document::diff_index(other, &titles, spec, progress)?;
        let first = self.diff_first(&index, spec, progress)?;
        other.diff_second(index, first, spec, progress)
    }

    /// The first table's column titles, for pairing the second's columns with them.
    pub fn diff_titles(&self, spec: &DiffSpec) -> Vec<String> {
        let all: Vec<u32> = (0..self.col_count()).collect();
        self.titles(&all, spec.keep_first)
    }

    /// Step 1, on the SECOND table: index it by key and row hashes.
    pub fn diff_index(
        other: &Document,
        titles_a: &[String],
        spec: &DiffSpec,
        progress: &Progress,
    ) -> Result<NewerIndex, Error> {
        other.ready_for_bulk()?;
        if spec.keys.is_empty() {
            return Err(Error::Invalid("choose the key columns".into()));
        }
        NewerIndex::build(other, titles_a, &spec.updates(), progress)
    }

    /// Step 2, on the FIRST table: pair its rows with the index, and read what the result table needs of
    /// the rows that are removed or changed (their values and keys), plus its empty and repeated keys.
    pub fn diff_first(
        &self,
        index: &NewerIndex,
        spec: &DiffSpec,
        progress: &Progress,
    ) -> Result<DiffFirst, Error> {
        self.ready_for_bulk()?;
        let cols_a = self.col_count();
        if spec.keys.is_empty() || spec.keys.iter().any(|&k| k >= cols_a) {
            return Err(Error::Invalid("choose the key columns".into()));
        }
        let us = spec.updates();
        let pairing = self.pair_newer(index, &us, progress)?;
        let titles_a = self.diff_titles(spec);
        let rows_a = self.data_rows(spec.keep_first);
        let total_a = rows_a.end.saturating_sub(rows_a.start);
        let look = pairing.repeated > 0;
        let (empty_a, dup_a) = scan_keys(self, &spec.keys, spec, look, progress)?;
        let all_a: Vec<u32> = (0..cols_a).collect();
        let reader = Cols::new(self, &all_a);
        let listed = pairing.capped(MAX_LISTED_ROWS);
        let mut buf = Vec::new();
        let width = cols_a as usize;
        let mut read = |id: u32, r: u64| -> RowA {
            reader.read(self, id, &mut buf);
            let vals: Vec<String> = (0..width).map(|c| reader.get(&buf, c).to_string()).collect();
            let keys: Vec<String> = spec.keys.iter().map(|&k| vals[k as usize].clone()).collect();
            RowA {
                number: self.row_number(r).unwrap_or(r) + 1,
                vals,
                keys,
            }
        };
        let removed: Vec<RowA> = listed.removed.iter().map(|&(id, r)| read(id, r)).collect();
        let changed: Vec<RowA> = listed.changed.iter().map(|&(id, r, _)| read(id, r)).collect();
        Ok(DiffFirst {
            pairing,
            titles_a,
            cols_a,
            total_a,
            removed,
            changed,
            empty_a,
            dup_a,
            rev: self.data_revision(),
        })
    }

    /// Step 3, on the SECOND table: read the new values, build the result table, the counts and the index
    /// that draws the differences.
    pub fn diff_second(
        &self,
        index: NewerIndex,
        first: DiffFirst,
        spec: &DiffSpec,
        progress: &Progress,
    ) -> Result<DiffOutcome, Error> {
        self.ready_for_bulk()?;
        let other = self;
        let DiffFirst {
            pairing,
            titles_a,
            cols_a,
            total_a,
            removed,
            changed,
            empty_a,
            dup_a,
            rev: a_rev,
        } = first;
        let keys_b: Vec<u32> = spec
            .keys
            .iter()
            .map(|&k| index.map[k as usize].ok_or(Error::Changed))
            .collect::<Result<_, _>>()?;
        let mut report = DiffReport {
            changed: pairing.changed.len() as u64,
            added: pairing.added.len() as u64,
            removed: pairing.removed.len() as u64,
            repeated: pairing.repeated,
            only_a: index.here_only.clone(),
            only_b: index.newer_only.clone(),
            moved: index.moved,
            empty_a,
            dup_a,
            ..Default::default()
        };
        report.same = total_a.saturating_sub(report.changed + report.removed);
        report.truncated = pairing.changed.len() > MAX_LISTED_ROWS
            || pairing.removed.len() > MAX_LISTED_ROWS
            || pairing.added.len() > MAX_LISTED_ROWS;
        let (empty_b, dup_b) = scan_keys(other, &keys_b, spec, pairing.repeated > 0, progress)?;
        report.empty_b = empty_b;
        report.dup_b = dup_b;

        // The values of the rows listed (the rest are counted only).
        let listed = pairing.capped(MAX_LISTED_ROWS);
        let values = NewerValues::fetch(other, &index, &listed, progress)?;
        let all_b: Vec<u32> = (0..index.newer_cols).collect();
        let titles_b = other.titles(&all_b, spec.keep_first);
        let mut used = vec![false; index.newer_cols as usize];
        for j in index.map.iter().flatten() {
            used[*j as usize] = true;
        }
        let only_b_cols: Vec<u32> = (0..index.newer_cols).filter(|j| !used[*j as usize]).collect();
        let reader_b = Cols::new(other, &only_b_cols);
        let reader_bk = Cols::new(other, &keys_b);

        let mut header: Vec<String> = vec!["Change".into(), "Row (first)".into(), "Row (second)".into()];
        header.extend(titles_a.iter().cloned());
        header.extend(only_b_cols.iter().map(|&j| titles_b[j as usize].clone()));
        header.push("What changed".into());
        let width_a = cols_a as usize;
        let mut out = Vec::new();
        push_line(&mut out, &header);
        let mut rows_out = 1u64;
        let mut a_side: HashMap<u128, Entry> = HashMap::new();
        let mut b_side: HashMap<u128, Entry> = HashMap::new();
        let mut line: Vec<String> = Vec::with_capacity(header.len());
        let mut bbuf = Vec::new();
        let mut kbuf = Vec::new();
        let blank_b: Vec<String> = vec![String::new(); only_b_cols.len()];
        let mut cells = 0u64;
        progress.start((removed.len() + changed.len() + listed.added.len()) as u64);
        let check = |out: &Vec<u8>| -> Result<(), Error> {
            if out.len() > MAX_TABLE_BYTES {
                return Err(Error::Invalid("the diff table would be too large; compare part of the file (filter it first)".into()));
            }
            Ok(())
        };
        // Removed rows: in the first table only.
        for ra in &removed {
            line.clear();
            line.extend(["removed".to_string(), ra.number.to_string(), String::new()]);
            line.extend(ra.vals.iter().cloned());
            line.extend(blank_b.iter().cloned());
            line.push(String::new());
            push_line(&mut out, &line);
            rows_out += 1;
            let parts = key_parts(ra);
            if !is_blank(&parts, spec.trim) {
                a_side
                    .entry(key_hash(&parts, spec.match_case, spec.trim))
                    .or_insert(Entry { kind: Kind::Removed, cols: Box::new([]) });
            }
            check(&out)?;
        }
        // Changed rows: the second table's values, and what changed.
        for (vr, ra) in changed.iter().enumerate() {
            let nid = listed.changed[vr].2;
            let newer = values.row(vr);
            let mut changed_cols: Vec<u32> = Vec::new();
            let mut what: Vec<String> = Vec::new();
            for c in 0..width_a {
                if index.map[c].is_none() {
                    continue;
                }
                if ra.vals[c] != newer[c] {
                    changed_cols.push(c as u32);
                    what.push(format!("{}: {} → {}", titles_a[c], ra.vals[c], newer[c]));
                }
            }
            let ka = key_parts(ra);
            if is_blank(&ka, spec.trim) {
                // Rows with an empty key are never paired with each other: one removed, one added.
                line.clear();
                line.extend(["removed".to_string(), ra.number.to_string(), String::new()]);
                line.extend(ra.vals.iter().cloned());
                line.extend(blank_b.iter().cloned());
                line.push(String::new());
                push_line(&mut out, &line);
                rows_out += 1;
                report.changed -= 1;
                report.removed += 1;
                continue;
            }
            if changed_cols.is_empty() {
                // Only the fingerprints differed (e.g. spaces at the ends with trim off): the same.
                report.changed -= 1;
                report.same += 1;
                continue;
            }
            cells += changed_cols.len() as u64;
            reader_b.read(other, nid, &mut bbuf);
            line.clear();
            line.extend(["changed".to_string(), ra.number.to_string(), String::new()]);
            line.extend(newer.iter().cloned());
            line.extend((0..only_b_cols.len()).map(|i| reader_b.get(&bbuf, i).to_string()));
            line.push(what.join("; "));
            push_line(&mut out, &line);
            rows_out += 1;
            let key = key_hash(&ka, spec.match_case, spec.trim);
            let bcols: Box<[u32]> = changed_cols.iter().filter_map(|&c| index.map[c as usize]).collect();
            a_side.insert(key, Entry { kind: Kind::Changed, cols: changed_cols.into_boxed_slice() });
            b_side.insert(key, Entry { kind: Kind::Changed, cols: bcols });
            check(&out)?;
            progress.step(1)?;
        }
        // Added rows: in the second table only.
        for (i, &nid) in listed.added.iter().enumerate() {
            let vals = values.row(listed.changed.len() + i);
            reader_b.read(other, nid, &mut bbuf);
            line.clear();
            line.extend(["added".to_string(), String::new(), String::new()]);
            line.extend(vals);
            line.extend((0..only_b_cols.len()).map(|i| reader_b.get(&bbuf, i).to_string()));
            line.push(String::new());
            push_line(&mut out, &line);
            rows_out += 1;
            reader_bk.read(other, nid, &mut kbuf);
            let parts: Vec<&str> = (0..keys_b.len()).map(|i| reader_bk.get(&kbuf, i)).collect();
            if !is_blank(&parts, spec.trim) {
                b_side
                    .entry(key_hash(&parts, spec.match_case, spec.trim))
                    .or_insert(Entry { kind: Kind::Added, cols: Box::new([]) });
            }
            check(&out)?;
        }
        report.cells = cells;
        report.listed = rows_out - 1;
        let index = DiffIndex {
            spec: spec.clone(),
            keys_a: spec.keys.clone(),
            keys_b,
            a_side,
            b_side,
            a_rev,
            b_rev: other.data_revision(),
            result_rev: None,
        };
        Ok(DiffOutcome {
            table: Table {
                bytes: out,
                rows: rows_out,
                cols: RESULT_LEAD + cols_a + only_b_cols.len() as u32 + 1,
            },
            report,
            index,
        })
    }

    /// The differences in a window of this table's shown rows (a table of `side`): a row is looked up by its
    /// key, so the marks are right after any sort, filter, insert or delete. Nothing is marked for a row
    /// with an empty key. `Error::Changed` when the rows are no longer the ones the diff was made from.
    pub fn diff_marks(
        &self,
        index: &DiffIndex,
        side: Side,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
    ) -> Result<Vec<DiffMark>, Error> {
        if !index.current(side, self.data_revision()) {
            return Err(Error::Changed);
        }
        let keys = index.key_cols(side);
        if keys.iter().any(|&k| k >= self.col_count()) {
            return Err(Error::Changed);
        }
        let reader = Cols::new(self, &keys);
        let first = u64::from(index.spec.keep_first && side != Side::Result) | u64::from(side == Side::Result);
        let r0 = row.max(first);
        let r1 = row.saturating_add(rows.min(MAX_WINDOW)).min(self.row_count());
        let c1 = col.saturating_add(cols);
        let mut out = Vec::new();
        let mut buf = Vec::new();
        for r in r0..r1 {
            let Some(id) = self.id_at(r) else { continue };
            reader.read(self, id, &mut buf);
            let parts: Vec<&str> = (0..keys.len()).map(|i| reader.get(&buf, i)).collect();
            if is_blank(&parts, index.spec.trim) {
                continue;
            }
            let key = key_hash(&parts, index.spec.match_case, index.spec.trim);
            let entry = match side {
                Side::A => index.a_side.get(&key),
                Side::B => index.b_side.get(&key),
                // The result table lists a key once: removed and changed rows by the first table's entry,
                // added rows by the second's.
                Side::Result => index.a_side.get(&key).or_else(|| index.b_side.get(&key)),
            };
            let Some(e) = entry else { continue };
            match e.kind {
                Kind::Added => out.push(DiffMark { row: r, col: None, kind: 1 }),
                Kind::Removed => out.push(DiffMark { row: r, col: None, kind: 2 }),
                Kind::Changed => {
                    for &c in e.cols.iter() {
                        let shown = if side == Side::Result { c + RESULT_LEAD } else { c };
                        if shown >= col && shown < c1 {
                            out.push(DiffMark { row: r, col: Some(shown), kind: 0 });
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(csv: &str) -> Document {
        Document::from_bytes(csv.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn spec(keys: &[u32]) -> DiffSpec {
        DiffSpec {
            keys: keys.to_vec(),
            keep_first: true,
            ..Default::default()
        }
    }

    fn lines(o: &DiffOutcome) -> Vec<String> {
        String::from_utf8(o.table.bytes.clone())
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    const A: &str = "id,name,city\r\n1,Ann,Dayton\r\n2,Bo,Lisbon\r\n3,Cy,Perth\r\n4,Di,Quito\r\n";
    const B: &str = "city,id,name,zip\r\nDayton,1,Ann,45402\r\nOsaka,2,Bo,100\r\nPerth,3,Cyd,6000\r\nNew,5,Eve,1\r\n";

    #[test]
    fn rows_are_paired_by_key_and_columns_by_title() {
        let (a, b) = (doc(A), doc(B));
        let o = a.diff_files(&b, &spec(&[0]), &Progress::new()).unwrap();
        assert_eq!((o.report.same, o.report.changed, o.report.added, o.report.removed), (1, 2, 1, 1));
        assert_eq!(o.report.cells, 2, "Bo's city and Cy's name");
        assert_eq!(o.report.only_b, vec!["zip".to_string()]);
        assert!(o.report.only_a.is_empty());
        assert_eq!(o.report.moved, 3);
        let l = lines(&o);
        assert_eq!(l[0], "Change,Row (first),Row (second),id,name,city,zip,What changed");
        assert!(l.iter().any(|x| x.starts_with("removed,5,,4,Di,Quito,,")), "{l:?}");
        assert!(l.iter().any(|x| x.starts_with("changed,3,,2,Bo,Osaka,100,") && x.contains("city: Lisbon → Osaka")), "{l:?}");
        assert!(l.iter().any(|x| x.starts_with("added,,,5,Eve,New,1,")), "{l:?}");
    }

    #[test]
    fn a_re_sorted_file_gives_the_same_result() {
        let a = doc(A);
        let sorted = doc("city,id,name,zip\r\nNew,5,Eve,1\r\nPerth,3,Cyd,6000\r\nOsaka,2,Bo,100\r\nDayton,1,Ann,45402\r\n");
        let x = a.diff_files(&doc(B), &spec(&[0]), &Progress::new()).unwrap();
        let y = a.diff_files(&sorted, &spec(&[0]), &Progress::new()).unwrap();
        assert_eq!(x.report, y.report);
        let mut lx = lines(&x);
        let mut ly = lines(&y);
        lx.sort();
        ly.sort();
        assert_eq!(lx, ly);
    }

    #[test]
    fn comparing_a_table_with_itself_gives_all_the_same() {
        let a = doc(A);
        let o = a.diff_files(&a, &spec(&[0]), &Progress::new()).unwrap();
        assert_eq!((o.report.same, o.report.changed, o.report.added, o.report.removed), (4, 0, 0, 0));
        assert_eq!(lines(&o).len(), 1, "only the title row");
    }

    #[test]
    fn repeated_and_empty_keys_are_reported_and_empty_keys_never_pair() {
        let a = doc("id,v\r\n1,a\r\n1,b\r\n2,c\r\n,x\r\n,y\r\n");
        let b = doc("id,v\r\n1,a\r\n1,B\r\n2,c\r\n,x\r\n");
        let o = a.diff_files(&b, &spec(&[0]), &Progress::new()).unwrap();
        assert!(o.report.repeated >= 1);
        assert_eq!(o.report.dup_a, vec!["1".to_string()]);
        assert_eq!(o.report.dup_b, vec!["1".to_string()]);
        assert_eq!((o.report.empty_a, o.report.empty_b), (2, 1));
        // The first "1" pairs with the first "1" (same), the second with the second (changed b -> B).
        assert_eq!(o.report.changed, 1);
        assert!(lines(&o).iter().any(|x| x.starts_with("changed,") && x.contains("v: b → B")));
    }

    #[test]
    fn options_ignore_case_and_trim_for_keys() {
        let a = doc("id,v\r\nAB ,1\r\n");
        let b = doc("id,v\r\nab,2\r\n");
        let plain = a.diff_files(&b, &spec(&[0]), &Progress::new()).unwrap();
        assert_eq!((plain.report.removed, plain.report.added), (1, 1));
        let mut s = spec(&[0]);
        s.trim = true;
        s.match_case = true;
        let o = a.diff_files(&b, &s, &Progress::new()).unwrap();
        assert_eq!((o.report.removed, o.report.added), (1, 1), "case still counts when asked to");
        s.match_case = false;
        let o = a.diff_files(&b, &s, &Progress::new()).unwrap();
        assert_eq!((o.report.removed, o.report.added, o.report.changed), (0, 0, 1));
    }

    #[test]
    fn a_missing_key_column_is_refused() {
        let a = doc("id,v\r\n1,a\r\n");
        let b = doc("code,v\r\n1,a\r\n");
        assert!(a.diff_files(&b, &spec(&[0]), &Progress::new()).is_err());
    }

    #[test]
    fn marks_are_looked_up_by_key_so_they_follow_a_sort() {
        let mut a = doc(A);
        let b = doc(B);
        let o = a.diff_files(&b, &spec(&[0]), &Progress::new()).unwrap();
        let mut idx = o.index;
        let marks = |d: &Document, idx: &DiffIndex| {
            let mut m = d.diff_marks(idx, Side::A, 0, 10, 0, 10).unwrap();
            m.sort_by_key(|x| (x.row, x.col));
            m
        };
        // Before: Bo (shown row 2) city changed, Cy (3) name changed, Di (4) removed.
        let m = marks(&a, &idx);
        assert_eq!(m.iter().map(|x| (x.row, x.col, x.kind)).collect::<Vec<_>>(), vec![(2, Some(2), 0), (3, Some(1), 0), (4, None, 2)]);
        // Sort by name descending: the marks stay on the same people.
        let spec_sort = crate::sort::SortSpec {
            keys: vec![crate::sort::SortKey {
                col: 1,
                descending: true,
                kind: crate::sort::SortKind::Text,
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        };
        let plan = a.plan_sort(&spec_sort, &Progress::new()).unwrap();
        a.apply_sort(plan).unwrap();
        idx.follow_order(Side::A, a.data_revision());
        let m = marks(&a, &idx);
        // Names now: Di, Cy, Bo, Ann on rows 1..4.
        assert_eq!(m.iter().map(|x| (x.row, x.col, x.kind)).collect::<Vec<_>>(), vec![(1, None, 2), (2, Some(1), 0), (3, Some(2), 0)]);
        // An edit makes the marks out of date.
        a.set_cell(1, 1, "Zed".into()).unwrap();
        assert!(matches!(a.diff_marks(&idx, Side::A, 0, 10, 0, 10), Err(Error::Changed)));
    }

    #[test]
    fn the_second_table_and_the_result_table_have_their_marks_too() {
        let a = doc(A);
        let b = doc(B);
        let o = a.diff_files(&b, &spec(&[0]), &Progress::new()).unwrap();
        let mut bm = b.diff_marks(&o.index, Side::B, 0, 10, 0, 10).unwrap();
        bm.sort_by_key(|x| (x.row, x.col));
        // B's columns: city(0), id(1), name(2), zip(3): Bo's city (row 2), Cy's name (row 3), Eve added (row 4).
        assert_eq!(bm.iter().map(|x| (x.row, x.col, x.kind)).collect::<Vec<_>>(), vec![(2, Some(0), 0), (3, Some(2), 0), (4, None, 1)]);
        let result = o.table.into_document().unwrap();
        let mut idx = o.index;
        idx.follow_order(Side::Result, result.data_revision());
        let mut rm = result.diff_marks(&idx, Side::Result, 0, 20, 0, 20).unwrap();
        rm.sort_by_key(|x| (x.row, x.col));
        // Result rows: removed(Di), changed(Bo), changed(Cy), added(Eve); columns 3.. are the first table's.
        assert_eq!(rm.iter().map(|x| (x.row, x.col, x.kind)).collect::<Vec<_>>(), vec![(1, None, 2), (2, Some(5), 0), (3, Some(4), 0), (4, None, 1)]);
    }

    #[test]
    fn a_big_diff_is_counted_exactly_and_can_be_cancelled() {
        let mut a = String::from("id,v\r\n");
        let mut b = String::from("id,v\r\n");
        for i in 0..50_000 {
            a.push_str(&format!("{i},x\r\n"));
            b.push_str(&format!("{i},{}\r\n", if i % 10 == 0 { "y" } else { "x" }));
        }
        let (a, b) = (doc(&a), doc(&b));
        let o = a.diff_files(&b, &spec(&[0]), &Progress::new()).unwrap();
        assert_eq!((o.report.same, o.report.changed), (45_000, 5_000));
        assert_eq!(o.report.cells, 5_000);
        let p = Progress::new();
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(a.diff_files(&b, &spec(&[0]), &p), Err(Error::Cancelled)));
    }
}
