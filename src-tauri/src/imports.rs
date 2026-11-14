//! Import sessions: a file the user picked in a native dialog (or clipboard
//! text) held in memory while the import screen previews it, then turned
//! into a new tab. The webview refers to a session by id only.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use natendo_engine::import::{self, ImportFormat, ImportInfo, ImportOptions};
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
    progress: Mutex<Option<Arc<Progress>>>,
}

/// What the import screen starts from.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ImportStart {
    pub id: u32,
    pub name: String,
    pub info: ImportInfo,
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
                progress: Mutex::new(None),
            }),
        );
        Ok(ImportStart { id, name, info })
    }

    /// Read a file the user picked (never a path from the webview).
    pub fn open_path(&self, path: &Path) -> CmdResult<ImportStart> {
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
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        self.start(name, ext, Some(path.to_path_buf()), data)
    }

    /// Clipboard text, sent by the UI (capped).
    pub fn open_text(&self, text: String) -> CmdResult<ImportStart> {
        if text.len() as u64 > MAX_IMPORT_BYTES {
            return Err(CmdError::new(
                "too_large",
                "the clipboard holds too much text",
            ));
        }
        self.start("Clipboard".into(), None, None, text.into_bytes())
    }

    /// What the source holds with another format chosen.
    pub fn inspect(&self, id: u32, format: Option<ImportFormat>) -> CmdResult<ImportInfo> {
        let s = self.get(id)?;
        Ok(import::inspect(&s.data, s.ext.as_deref(), format)?)
    }

    /// The first rows as they would import (cells cut for display).
    pub fn preview(&self, id: u32, opts: &ImportOptions) -> CmdResult<Vec<Vec<String>>> {
        check(opts)?;
        let s = self.get(id)?;
        let t = import::import(
            &s.data,
            s.ext.as_deref(),
            opts,
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
                import::import(&s.data, s.ext.as_deref(), opts, None, &progress)?.into_document()?
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
