//! Import sessions: a file the user picked in a native dialog (or clipboard
//! text) held in memory while the import screen previews it, then turned
//! into a new tab. The webview refers to a session by id only.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use natendo_engine::dbf::{self, DbfInfo, Sidecars};
use natendo_engine::import::{self, ImportFormat, ImportInfo, ImportOptions};
use natendo_engine::sheet::SheetStats;
use natendo_engine::Progress;
use serde::Serialize;

use crate::docs::{CmdError, CmdResult, DocInfo, Docs};

/// Import sessions open at once (each may hold a large file).
pub const MAX_IMPORTS: usize = 4;
/// Largest file or clipboard text imported.
pub const MAX_IMPORT_BYTES: u64 = 1 << 30;
/// Rows, columns and characters per cell in an import preview.
pub const PREVIEW_ROWS: u64 = 100;
pub const PREVIEW_COLS: usize = 50;
pub const PREVIEW_CHARS: usize = 200;
/// Longest XML record path accepted from the UI.
pub const MAX_RECORD_CHARS: usize = 4096;

struct Session {
    /// File name (or "Clipboard"), for the new tab.
    name: String,
    ext: Option<String>,
    /// The file it came from (none for the clipboard).
    path: Option<std::path::PathBuf>,
    data: Vec<u8>,
    /// dBase: the files beside the table.
    beside: Option<Arc<Sidecars>>,
    progress: Mutex<Option<Arc<Progress>>>,
}

impl Session {
    /// The options with the session's own files filled in.
    fn with(&self, opts: &ImportOptions) -> ImportOptions {
        ImportOptions {
            beside: self.beside.clone(),
            ..opts.clone()
        }
    }
}

/// What the import screen says about a dBase table: counts, the encoding and
/// why, the coordinate system and the warnings (field names and counts only).
fn dbf_summary(info: &DbfInfo, warnings: &[String]) -> String {
    let mut s = format!(
        "{}. Text is read as {} ({}).",
        count_of(u64::from(info.fields), "field", "fields"),
        info.encoding,
        info.rule
    );
    if !info.projection.is_empty() {
        s.push_str(&format!(" Coordinate system: {}", info.projection));
        s.push('.');
    }
    for w in warnings {
        let mut c = w.chars();
        if let Some(first) = c.next() {
            s.push(' ');
            s.extend(first.to_uppercase());
            s.push_str(c.as_str());
            s.push('.');
        }
    }
    s
}

/// "2 records (1 marked as deleted, left out)": the records the table will hold.
fn dbf_records(info: &DbfInfo, show_deleted: bool) -> String {
    let shown = if show_deleted {
        info.records
    } else {
        info.records - info.deleted
    };
    let mut s = count_of(shown, "record", "records");
    if info.deleted > 0 {
        s.push_str(&format!(
            " ({} marked as deleted, {})",
            info.deleted,
            if show_deleted { "shown" } else { "left out" }
        ));
    }
    s
}

/// Size and time of a file, to notice a program writing it while it is read.
fn stamps(path: &Path) -> Vec<Option<(u64, std::time::SystemTime)>> {
    [path.to_path_buf(), path.with_extension("dbt")]
        .iter()
        .map(|p| {
            let m = std::fs::metadata(p).ok()?;
            Some((m.len(), m.modified().ok()?))
        })
        .collect()
}

/// Rows looked at to count what the "exactly as written" guards did (a bigger file says "in the first N rows").
pub const NOTES_ROWS: u64 = 200_000;

/// A column with cells in a zero-padding number format.
#[derive(Debug, Serialize, Clone, PartialEq, Default)]
pub struct PaddedColumn {
    pub name: String,
    pub count: u64,
}

/// What the import guards did, as counts and column names only (never cell values).
#[derive(Debug, Serialize, Clone, PartialEq, Default)]
pub struct ImportNotes {
    pub kept_big: u64,
    pub lost_digits: u64,
    pub padded: Vec<PaddedColumn>,
    pub padded_applied: u64,
    pub padded_left: u64,
    /// Rows looked at, and whether that was the whole sheet.
    pub scanned_rows: u64,
    pub all_scanned: bool,
    /// The file to remember the zero-padding answer for; none for the clipboard or a private file.
    pub remember_path: Option<String>,
    /// One sentence for the status line and the import screen (empty when no guard did anything).
    pub summary: String,
    /// dBase: how many records there are, said under the preview (a record count is what a GIS
    /// program shows, so the title row is never in it).
    pub records: String,
}

/// "A", "B", ... "AA" for a column number counted from 0.
fn letters(mut c: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// A column named by its heading (cut short), else by its letter.
fn col_label(names: &[String], c: u32) -> String {
    match names
        .get(c as usize)
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
    {
        Some(n) if n.chars().count() <= 40 => format!("\u{201c}{n}\u{201d}"),
        Some(n) => format!(
            "\u{201c}{}\u{2026}\u{201d}",
            n.chars().take(40).collect::<String>()
        ),
        None => format!("column {}", letters(c)),
    }
}

fn count_of(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The notes of a spreadsheet read: counts and column names (the first row) only.
pub fn notes_of(st: &SheetStats, names: &[String], scanned: u64, all: bool) -> ImportNotes {
    let mut n = ImportNotes {
        kept_big: st.kept_big,
        lost_digits: st.lost_digits,
        padded: st
            .padded
            .iter()
            .map(|(c, k)| PaddedColumn {
                name: col_label(names, *c),
                count: *k,
            })
            .collect(),
        padded_applied: st.padded_applied,
        padded_left: st.padded_left,
        scanned_rows: scanned,
        all_scanned: all,
        remember_path: None,
        summary: String::new(),
        records: String::new(),
    };
    let mut parts: Vec<String> = Vec::new();
    if st.kept_big > 0 {
        let cols: Vec<String> = st
            .kept_columns
            .iter()
            .take(5)
            .map(|c| col_label(names, *c))
            .collect();
        parts.push(format!(
            "{} of more than 15 digits kept exactly as the file stores them ({})",
            count_of(st.kept_big, "number", "numbers"),
            cols.join(", ")
        ));
    }
    if st.lost_digits > 0 {
        parts.push(format!(
            "{} had digits the file no longer holds (a spreadsheet keeps 15), shown as stored",
            count_of(st.lost_digits, "number", "numbers")
        ));
    }
    if st.padded_applied > 0 {
        parts.push(format!(
            "{} written with leading zeros, as the spreadsheet shows them",
            count_of(st.padded_applied, "number", "numbers")
        ));
    }
    if st.padded_left > 0 {
        parts.push(format!(
            "{} with a leading-zero format could not be padded and stay as they are",
            count_of(st.padded_left, "cell", "cells")
        ));
    }
    if !parts.is_empty() {
        let mut s = parts.join("; ");
        if !all {
            s = format!("In the first {scanned} rows: {s}");
        }
        let first = s.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
        let up = s[..first].to_uppercase();
        s.replace_range(..first, &up);
        s.push('.');
        n.summary = s;
    }
    n
}

/// What the import screen starts from.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ImportStart {
    pub id: u32,
    pub name: String,
    pub info: ImportInfo,
    /// Said above the import screen's options (why this file opened).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Default)]
pub struct Imports {
    next: AtomicU32,
    map: Mutex<HashMap<u32, Arc<Session>>>,
}

fn check(opts: &ImportOptions) -> CmdResult<()> {
    if opts.record.len() > MAX_RECORD_CHARS {
        return Err(CmdError::new("too_large", "the record path is too long"));
    }
    Ok(())
}

impl Imports {
    fn map(&self) -> MutexGuard<'_, HashMap<u32, Arc<Session>>> {
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn get(&self, id: u32) -> CmdResult<Arc<Session>> {
        self.map()
            .get(&id)
            .cloned()
            .ok_or_else(|| CmdError::new("no_import", "that import was closed"))
    }

    fn start(
        &self,
        name: String,
        ext: Option<String>,
        path: Option<std::path::PathBuf>,
        data: Vec<u8>,
        beside: Option<Arc<Sidecars>>,
    ) -> CmdResult<ImportStart> {
        let info = import::inspect(&data, ext.as_deref(), None)?;
        let mut map = self.map();
        if map.len() >= MAX_IMPORTS {
            return Err(CmdError::new(
                "too_many_imports",
                "close another import screen first",
            ));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        map.insert(
            id,
            Arc::new(Session {
                name: name.clone(),
                ext,
                path,
                data,
                beside,
                progress: Mutex::new(None),
            }),
        );
        Ok(ImportStart {
            id,
            name,
            info,
            note: String::new(),
        })
    }

    /// Read a file the user picked (never a path from the webview).
    pub fn open_path(&self, path: &Path) -> CmdResult<ImportStart> {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        let table = ext.as_deref() == Some("dbf");
        let before = table.then(|| stamps(path));
        let f = std::fs::File::open(path).map_err(|e| CmdError::new("error", e.to_string()))?;
        let mut data = Vec::new();
        f.take(MAX_IMPORT_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| CmdError::new("error", e.to_string()))?;
        if data.len() as u64 > MAX_IMPORT_BYTES {
            return Err(CmdError::new(
                "too_large",
                "the file is larger than 1 GB, too large to import",
            ));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported".into());
        let beside = table.then(|| {
            let mut side = Sidecars::beside(path);
            if before != Some(stamps(path)) {
                side.problems.push(
                    "the file changed while it was being read: close it in the program writing it and open it again"
                        .into(),
                );
            }
            Arc::new(side)
        });
        self.start(name, ext, Some(path.to_path_buf()), data, beside)
    }

    /// Clipboard text, sent by the UI (capped).
    pub fn open_text(&self, text: String) -> CmdResult<ImportStart> {
        if text.len() as u64 > MAX_IMPORT_BYTES {
            return Err(CmdError::new(
                "too_large",
                "the clipboard holds too much text",
            ));
        }
        self.start("Clipboard".into(), None, None, text.into_bytes(), None)
    }

    /// What the source holds with another format chosen.
    pub fn inspect(&self, id: u32, format: Option<ImportFormat>) -> CmdResult<ImportInfo> {
        let s = self.get(id)?;
        Ok(import::inspect(&s.data, s.ext.as_deref(), format)?)
    }

    /// What the "exactly as written" guards did (counts and column names only), for the first
    /// [`NOTES_ROWS`] rows. Nothing about cell values leaves here, and a private file or the clipboard
    /// is never offered for remembering the zero-padding answer.
    pub fn notes(&self, id: u32, opts: &ImportOptions, docs: &Docs) -> CmdResult<ImportNotes> {
        check(opts)?;
        let s = self.get(id)?;
        let format = opts
            .format
            .unwrap_or_else(|| import::guess(s.ext.as_deref(), &s.data));
        if format == ImportFormat::Dbf {
            let opts = s.with(opts);
            let none = Sidecars::default();
            let side = opts.beside.as_deref().unwrap_or(&none);
            let info = dbf::inspect(&s.data, side, &opts.dbf)?;
            let (_, st) = import::import_report(
                &s.data,
                s.ext.as_deref(),
                &opts,
                Some(NOTES_ROWS),
                &Progress::new(),
            )?;
            return Ok(ImportNotes {
                summary: dbf_summary(&info, &st.warnings),
                records: dbf_records(&info, opts.dbf.show_deleted),
                scanned_rows: info.records.min(NOTES_ROWS),
                all_scanned: info.records < NOTES_ROWS,
                ..Default::default()
            });
        }
        if format != ImportFormat::Spreadsheet {
            return Ok(ImportNotes::default());
        }
        let (t, st) = import::import_report(
            &s.data,
            s.ext.as_deref(),
            opts,
            Some(NOTES_ROWS),
            &Progress::new(),
        )?;
        let d = t.into_document()?;
        let names = d.row_values(0).unwrap_or_default();
        let rows = d.row_count();
        let mut n = notes_of(&st, &names, rows, rows < NOTES_ROWS);
        n.remember_path = s
            .path
            .as_ref()
            .filter(|p| !docs.is_private_path(p))
            .map(|p| p.display().to_string());
        Ok(n)
    }

    /// The first rows as they would import (cells cut for display).
    pub fn preview(&self, id: u32, opts: &ImportOptions) -> CmdResult<Vec<Vec<String>>> {
        check(opts)?;
        let s = self.get(id)?;
        let t = import::import(
            &s.data,
            s.ext.as_deref(),
            &s.with(opts),
            Some(PREVIEW_ROWS),
            &Progress::new(),
        )?;
        let d = t.into_document()?;
        let mut rows = Vec::new();
        for r in 0..d.row_count().min(PREVIEW_ROWS) {
            let mut v = d.row_values(r)?;
            v.truncate(PREVIEW_COLS);
            for c in v.iter_mut() {
                if c.chars().count() > PREVIEW_CHARS {
                    *c = c.chars().take(PREVIEW_CHARS).chain(Some('…')).collect();
                }
            }
            rows.push(v);
        }
        Ok(rows)
    }

    /// Import into a new tab; the session stays open until closed.
    pub fn finish(&self, id: u32, opts: &ImportOptions, docs: &Docs) -> CmdResult<DocInfo> {
        check(opts)?;
        let s = self.get(id)?;
        let progress = Arc::new(Progress::new());
        *s.progress.lock().unwrap_or_else(|p| p.into_inner()) = Some(progress.clone());
        let result = (|| {
            let format = opts
                .format
                .unwrap_or_else(|| import::guess(s.ext.as_deref(), &s.data));
            let doc = if format == ImportFormat::Delimited {
                import::delimited(&s.data, s.ext.as_deref())?
            } else {
                import::import(&s.data, s.ext.as_deref(), &s.with(opts), None, &progress)?
                    .into_document()?
            };
            Ok::<_, CmdError>(doc)
        })();
        *s.progress.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let doc = result?;
        let base = s.name.rsplit_once('.').map_or(s.name.as_str(), |(b, _)| b);
        let info = docs.insert_titled(doc, Some(base.to_string()))?;
        // A spreadsheet from a private file or folder makes a private tab.
        if let Some(p) = &s.path {
            docs.private_if_from(info.id, p);
        }
        docs.info(info.id)
    }

    /// Progress of a running import (0 to 1), if one runs.
    pub fn progress(&self, id: u32) -> CmdResult<Option<f64>> {
        let s = self.get(id)?;
        let p = s.progress.lock().unwrap_or_else(|p| p.into_inner());
        Ok(p.as_ref().map(|p| p.fraction()))
    }

    pub fn cancel(&self, id: u32) -> CmdResult<()> {
        let s = self.get(id)?;
        if let Some(p) = s
            .progress
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn close(&self, id: u32) {
        if let Some(s) = self.map().remove(&id) {
            if let Some(p) = s
                .progress
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
            {
                p.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A1a: the summary holds counts and column names only (never a cell value), and says plainly what
    /// could not be kept.
    #[test]
    fn dbf_record_counts_leave_out_the_title_row() {
        let mut info = DbfInfo {
            records: 3,
            deleted: 1,
            ..Default::default()
        };
        assert_eq!(
            dbf_records(&info, false),
            "2 records (1 marked as deleted, left out)"
        );
        assert_eq!(
            dbf_records(&info, true),
            "3 records (1 marked as deleted, shown)"
        );
        info.deleted = 0;
        info.records = 1;
        assert_eq!(dbf_records(&info, false), "1 record");
    }

    #[test]
    fn notes_hold_counts_and_names_only() {
        let st = SheetStats {
            kept_big: 2,
            lost_digits: 1,
            padded: vec![(1, 5)],
            padded_applied: 4,
            padded_left: 1,
            kept_columns: vec![0, 1],
            warnings: Vec::new(),
        };
        let names: Vec<String> = ["Account", "ZIP", "Name"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let n = notes_of(&st, &names, 10, true);
        assert_eq!(
            n.padded,
            vec![PaddedColumn {
                name: "\u{201c}ZIP\u{201d}".into(),
                count: 5
            }]
        );
        assert!(
            n.summary
                .starts_with("2 numbers of more than 15 digits kept exactly"),
            "{}",
            n.summary
        );
        assert!(
            n.summary.contains("\u{201c}Account\u{201d}")
                && n.summary.contains("\u{201c}ZIP\u{201d}")
        );
        assert!(n.summary.contains("4 numbers written with leading zeros"));
        assert!(n
            .summary
            .contains("1 cell with a leading-zero format could not be padded"));
        assert!(n
            .summary
            .contains("1 number had digits the file no longer holds"));
        assert!(n.summary.ends_with('.'));
        // A bigger sheet says how much was looked at; a quiet one says nothing.
        assert!(notes_of(&st, &names, 200_000, false)
            .summary
            .starts_with("In the first 200000 rows: "));
        assert_eq!(
            notes_of(&SheetStats::default(), &names, 3, true).summary,
            ""
        );
        // An unnamed column is called by its letter.
        let q = notes_of(
            &SheetStats {
                kept_big: 1,
                kept_columns: vec![27],
                ..Default::default()
            },
            &[],
            1,
            true,
        );
        assert!(q.summary.contains("column AB"), "{}", q.summary);
    }

    /// R4: the clipboard has no file to remember an answer for, and a non-spreadsheet has no notes.
    #[test]
    fn clipboard_notes_offer_nothing_to_remember() {
        let imports = Imports::default();
        let docs = Docs::default();
        let s = imports.open_text("a,b\n1,2\n".into()).unwrap();
        let n = imports
            .notes(s.id, &ImportOptions::default(), &docs)
            .unwrap();
        assert_eq!(n, ImportNotes::default());
    }

    #[test]
    fn clipboard_and_file_imports() {
        let imports = Imports::default();
        let docs = Docs::default();
        let s = imports.open_text("[{\"a\":1},{\"a\":2}]".into()).unwrap();
        assert_eq!(s.info.format, Some(ImportFormat::Json));
        let p = imports.preview(s.id, &ImportOptions::default()).unwrap();
        assert_eq!(p, vec![vec!["a"], vec!["1"], vec!["2"]]);
        let info = imports
            .finish(s.id, &ImportOptions::default(), &docs)
            .unwrap();
        assert_eq!(
            (info.name.as_str(), info.rows, info.modified),
            ("Clipboard", 3, true)
        );
        assert!(info.path.is_none());
        // Text as delimited keeps the detected layout.
        let t = imports.open_text("a;b\n1;2\n".into()).unwrap();
        let opts = ImportOptions {
            format: Some(ImportFormat::Delimited),
            ..Default::default()
        };
        let info = imports.finish(t.id, &opts, &docs).unwrap();
        assert_eq!((info.cols, info.delimiter.as_str()), (2, ";"));
        // Sessions are capped and closable.
        for _ in 0..MAX_IMPORTS - 2 {
            imports.open_text("x".into()).unwrap();
        }
        assert!(imports.open_text("x".into()).is_err());
        imports.close(s.id);
        assert!(imports.preview(s.id, &ImportOptions::default()).is_err());
        assert!(imports.open_text("x".into()).is_ok());
        // Bad record paths are refused before any work.
        let bad = ImportOptions {
            record: "a/".repeat(5000),
            ..Default::default()
        };
        assert!(imports.preview(t.id, &bad).is_err());
        // A file path that is not a spreadsheet but named like one.
        let dir = std::env::temp_dir().join(format!("natendo-imp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("fake.xlsx");
        std::fs::write(&f, b"not a zip").unwrap();
        imports.close(t.id);
        assert!(imports.open_path(&f).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
