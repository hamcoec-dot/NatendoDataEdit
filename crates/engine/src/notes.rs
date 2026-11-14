//! Notes and flags on rows (Milestone 15), kept in a small file beside
//! the table (`voters.csv.ndnotes`) so the table itself is never changed.
//! A row is found again by its key columns (an ID, or a name and birth
//! date), so notes follow their rows through sorts, filters, edits to
//! other columns and newer copies of the file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress, View};
use crate::filter::FilterSpec;
use crate::ops::Cols;
use crate::reshape::RowPlan;
use crate::Error;

pub const VERSION: u32 = 1;
/// Most notes in one file.
pub const MAX_NOTES: usize = 1_000_000;
/// Longest note.
pub const MAX_NOTE_CHARS: usize = 4_000;
/// Flag colours: 1 red, 2 orange, 3 yellow, 4 green, 5 blue, 6 purple.
pub const FLAGS: u8 = 6;
const CHUNK: u64 = 1 << 15;

/// One row's note and flag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    /// 0: no flag.
    pub flag: u8,
    pub text: String,
}

/// The notes of one table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteBook {
    pub version: u32,
    /// Columns that identify a row (logical numbers).
    pub key_cols: Vec<u32>,
    /// Their titles when chosen (to warn when the columns moved).
    pub key_titles: Vec<String>,
    /// The table's first row holds titles (it gets no note).
    pub header: bool,
    /// By row key.
    pub notes: BTreeMap<String, Note>,
}

impl Default for NoteBook {
    fn default() -> Self {
        NoteBook {
            version: VERSION,
            key_cols: vec![0],
            key_titles: Vec::new(),
            header: true,
            notes: BTreeMap::new(),
        }
    }
}

/// A noted row on screen or in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteMark {
    pub row: u64,
    pub flag: u8,
    pub text: String,
}

/// The notes file beside a table.
pub fn notes_path(table: &Path) -> PathBuf {
    let mut name = table
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".ndnotes");
    table.with_file_name(name)
}

impl NoteBook {
    /// Read a notes file; `None` when there is none.
    pub fn load(path: &Path) -> Result<Option<NoteBook>, Error> {
        let Ok(meta) = std::fs::metadata(path) else {
            return Ok(None);
        };
        if meta.len() > 256 << 20 {
            return Err(Error::Invalid("the notes file is too large".into()));
        }
        let bytes = std::fs::read(path)?;
        let book: NoteBook = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Invalid(format!("the notes file cannot be read: {e}")))?;
        if book.version > VERSION {
            return Err(Error::Invalid(
                "the notes file was saved by a newer version of the app".into(),
            ));
        }
        book.check()?;
        Ok(Some(book))
    }

    pub fn check(&self) -> Result<(), Error> {
        if self.key_cols.is_empty() || self.key_cols.len() > 8 {
            return Err(Error::Invalid(
                "choose 1 to 8 columns that identify a row".into(),
            ));
        }
        if self.notes.len() > MAX_NOTES {
            return Err(Error::Invalid("too many notes".into()));
        }
        if self
            .notes
            .values()
            .any(|n| n.flag > FLAGS || n.text.chars().count() > MAX_NOTE_CHARS)
        {
            return Err(Error::Invalid(
                "a note is too long or has an unknown flag".into(),
            ));
        }
        Ok(())
    }

    /// Write the notes file (a temp file moved into place); an empty
    /// notebook removes it.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        self.check()?;
        if self.notes.is_empty() {
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => return Ok(()),
            }
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?;
        let (temp, mut file) = crate::save::create_temp(path)?;
        let done = (|| -> std::io::Result<()> {
            use std::io::Write;
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        drop(file);
        if let Err(e) = done.and_then(|_| crate::save::replace(&temp, path)) {
            let _ = std::fs::remove_file(&temp);
            return Err(e.into());
        }
        Ok(())
    }

    fn key(&self, vals: &[&str]) -> String {
        let mut k = String::new();
        for (i, v) in vals.iter().enumerate() {
            if i > 0 {
                k.push('\u{1F}');
            }
            k.push_str(v.trim());
        }
        k
    }
}

impl Document {
    fn note_reader(&self, book: &NoteBook) -> Result<Cols, Error> {
        if book.key_cols.iter().any(|&c| c >= self.col_count()) {
            return Err(Error::Invalid(
                "a column that identifies rows for the notes is missing; choose the key columns again".into(),
            ));
        }
        Ok(Cols::new(self, &book.key_cols))
    }

    /// The key of shown row `row` (empty for the title row or a row whose
    /// key cells are all blank).
    pub fn note_key(&self, book: &NoteBook, row: u64) -> Result<String, Error> {
        if book.header && row == 0 {
            return Ok(String::new());
        }
        let reader = self.note_reader(book)?;
        let id = self.id_at(row).ok_or(Error::OutOfRange)?;
        let mut buf = Vec::new();
        reader.read(self, id, &mut buf);
        let vals: Vec<&str> = (0..book.key_cols.len())
            .map(|i| reader.get(&buf, i))
            .collect();
        if vals.iter().all(|v| v.trim().is_empty()) {
            return Ok(String::new());
        }
        Ok(book.key(&vals))
    }

    /// Noted rows among shown rows `row..row+rows`.
    pub fn note_marks(&self, book: &NoteBook, row: u64, rows: u64) -> Result<Vec<NoteMark>, Error> {
        if book.notes.is_empty() {
            return Ok(Vec::new());
        }
        let reader = self.note_reader(book)?;
        let mut out = Vec::new();
        let mut buf = Vec::new();
        let end = row.saturating_add(rows.min(10_000)).min(self.row_count());
        for r in row.max(u64::from(book.header))..end {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let vals: Vec<&str> = (0..book.key_cols.len())
                .map(|i| reader.get(&buf, i))
                .collect();
            if let Some(n) = book.notes.get(&book.key(&vals)) {
                out.push(NoteMark {
                    row: r,
                    flag: n.flag,
                    text: n.text.clone(),
                });
            }
        }
        Ok(out)
    }

    /// Row ids (and shown rows) of every noted row, in table order:
    /// `flag` 0 any note or flag, else that flag only.
    fn noted(
        &self,
        book: &NoteBook,
        flag: u8,
        progress: &Progress,
    ) -> Result<Vec<(u32, u64, u8, String)>, Error> {
        self.ready_for_bulk()?;
        let reader = self.note_reader(book)?;
        let rows = self.data_rows(book.header);
        progress.start(rows.end - rows.start);
        let parts: Vec<Vec<(u32, u64, u8, String)>> = (0..(rows.end - rows.start).div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<_, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let vals: Vec<&str> = (0..book.key_cols.len())
                        .map(|i| reader.get(&buf, i))
                        .collect();
                    if let Some(n) = book.notes.get(&book.key(&vals)) {
                        if flag == 0 || n.flag == flag {
                            out.push((id, r, n.flag, n.text.clone()));
                        }
                    }
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        Ok(parts.into_iter().flatten().collect())
    }

    /// Every noted row (the first `limit`), for the notes list, and how
    /// many notes match no row (their row was deleted or its key changed).
    pub fn note_list(
        &self,
        book: &NoteBook,
        limit: usize,
        progress: &Progress,
    ) -> Result<(Vec<NoteMark>, u64), Error> {
        let all = self.noted(book, 0, progress)?;
        let matched = all.len() as u64;
        let orphans = (book.notes.len() as u64).saturating_sub(matched);
        Ok((
            all.into_iter()
                .take(limit)
                .map(|(_, row, flag, text)| NoteMark { row, flag, text })
                .collect(),
            orphans,
        ))
    }

    /// A filter showing only noted rows (`flag` 0: any; else that flag).
    pub fn plan_show_noted(
        &self,
        book: &NoteBook,
        flag: u8,
        progress: &Progress,
    ) -> Result<RowPlan, Error> {
        let all = self.noted(book, flag, progress)?;
        let base = self.next_row_id();
        let mut bits = vec![0u64; (base as usize).div_ceil(64)];
        if book.header && self.row_count() > 0 {
            let id = self.id_at(0).expect("a first row");
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        }
        for (id, ..) in &all {
            bits[(id >> 6) as usize] |= 1 << (id & 63);
        }
        let mut plan = RowPlan::new(self);
        plan.rows = all.len() as u64;
        plan.view = Some(View::new(
            FilterSpec {
                keep_first: book.header,
                ..Default::default()
            },
            bits,
            base,
        ));
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    #[test]
    fn notes_follow_rows() {
        let mut d = Document::from_bytes(
            b"id,name\r\n7,Ann\r\n3,Bo\r\n9,Cy\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let mut book = NoteBook::default();
        let k = d.note_key(&book, 2).unwrap();
        assert_eq!(k, "3");
        book.notes.insert(
            k,
            Note {
                flag: 1,
                text: "call back".into(),
            },
        );
        book.notes.insert(
            "9".into(),
            Note {
                flag: 4,
                text: String::new(),
            },
        );
        book.notes.insert(
            "42".into(),
            Note {
                flag: 2,
                text: "gone".into(),
            },
        );
        // Sorted: the note moves with its row.
        d.sort(&crate::sort::SortSpec {
            keys: vec![crate::sort::SortKey {
                col: 0,
                descending: false,
                kind: crate::sort::SortKind::Number,
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        })
        .unwrap();
        let marks = d.note_marks(&book, 0, 10).unwrap();
        assert_eq!(marks.len(), 2);
        assert_eq!(
            (marks[0].row, marks[0].flag, marks[0].text.as_str()),
            (1, 1, "call back")
        );
        let p = Progress::new();
        let (list, orphans) = d.note_list(&book, 100, &p).unwrap();
        assert_eq!((list.len(), orphans), (2, 1));
        let plan = d.plan_show_noted(&book, 4, &p).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 2);
        assert_eq!(d.cell(1, 1).unwrap(), "Cy");
        // Saved beside the table and read back; empty removes the file.
        let path = std::env::temp_dir().join(format!("nd-notes-{}.csv", std::process::id()));
        let np = notes_path(&path);
        assert!(np.to_string_lossy().ends_with(".csv.ndnotes"));
        book.save(&np).unwrap();
        assert_eq!(NoteBook::load(&np).unwrap().unwrap(), book);
        book.notes.clear();
        book.save(&np).unwrap();
        assert!(!np.exists());
    }
}
