//! Tables inside zip files: open `export.zip` without unpacking it first.
//! Each table is unpacked in memory only (no copy is written to disk) and
//! opens as a tab with no file, so the zip itself is never rewritten: Save
//! asks where to save it. The engine's zip reader caps sizes, entry counts
//! and the unpacking ratio (zip bombs).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};

use natendo_engine::zipread::Zip;
use natendo_engine::{Document, OpenOptions};
use serde::Serialize;

use crate::docs::{CmdError, CmdResult, DocInfo, Docs};

/// Largest zip file read.
pub const MAX_ZIP_BYTES: u64 = 1 << 30;
/// Largest table unpacked from a zip.
pub const MAX_ENTRY_BYTES: u64 = 2 << 30;
/// Most tables opened from one zip at once.
pub const MAX_PICK: usize = 32;
/// Most zips waiting for a pick.
const MAX_WAITING: usize = 16;

/// Table types picked out of a zip.
const TABLE_EXTS: &[&str] = &["csv", "tsv", "tab", "txt", "psv", "dat", "ssv"];

/// One table in a zip.
#[derive(Debug, Clone, Serialize)]
pub struct ZipEntry {
    pub name: String,
    /// Unpacked size as the zip states it (a hint only).
    pub size: u64,
}

/// A zip holding several tables, waiting for the user to pick.
#[derive(Debug, Clone, Serialize)]
pub struct ZipList {
    pub token: u32,
    /// The zip's own file name.
    pub name: String,
    pub entries: Vec<ZipEntry>,
}

pub enum Listed {
    One(String),
    Many(ZipList),
}

#[derive(Default)]
pub struct Zips {
    next: AtomicU32,
    waiting: Mutex<HashMap<u32, PathBuf>>,
}

pub static ZIPS: LazyLock<Zips> = LazyLock::new(Zips::default);

pub fn is_zip_path(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

fn err(m: impl Into<String>) -> CmdError {
    CmdError::new("invalid", m)
}

fn read_zip(path: &Path) -> CmdResult<Vec<u8>> {
    let f = std::fs::File::open(path).map_err(|e| CmdError::new("error", e.to_string()))?;
    let mut data = Vec::new();
    f.take(MAX_ZIP_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| CmdError::new("error", e.to_string()))?;
    if data.len() as u64 > MAX_ZIP_BYTES {
        return Err(CmdError::new("too_large", "the zip file is larger than 1 GB, too large to open"));
    }
    if !natendo_engine::zipread::is_zip(&data) {
        return Err(err("this is not a zip file (or it is damaged)"));
    }
    Ok(data)
}

fn is_table(name: &str) -> bool {
    // Mac folders and hidden files are not tables.
    if name.starts_with("__MACOSX/") || name.rsplit('/').next().is_some_and(|n| n.starts_with('.')) {
        return false;
    }
    name.rsplit_once('.')
        .is_some_and(|(_, e)| TABLE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Zips {
    /// The tables in a zip: one, or several to pick from.
    pub fn list(&self, path: &Path) -> CmdResult<Listed> {
        let data = read_zip(path)?;
        let zip = Zip::open(&data)?;
        let entries: Vec<ZipEntry> = zip
            .names()
            .into_iter()
            .filter(|(n, _)| is_table(n))
            .map(|(name, size)| ZipEntry { name, size })
            .collect();
        match entries.len() {
            0 => Err(err("this zip file holds no tables (.csv, .tsv or .txt files)")),
            1 => Ok(Listed::One(entries.into_iter().next().map(|e| e.name).unwrap_or_default())),
            _ => {
                let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
                let mut w = self.waiting.lock().unwrap_or_else(|p| p.into_inner());
                if w.len() >= MAX_WAITING {
                    // The oldest waiting pick is dropped.
                    if let Some(old) = w.keys().min().copied() {
                        w.remove(&old);
                    }
                }
                w.insert(token, path.to_path_buf());
                Ok(Listed::Many(ZipList {
                    token,
                    name: file_name(path),
                    entries,
                }))
            }
        }
    }

    /// Open the tables picked from a listed zip.
    pub fn open_picked(&self, docs: &Docs, token: u32, names: &[String]) -> CmdResult<Vec<DocInfo>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        if names.len() > MAX_PICK {
            return Err(err(format!("open at most {MAX_PICK} tables from a zip at once")));
        }
        let path = self
            .waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&token)
            .cloned()
            .ok_or_else(|| err("that zip file is no longer waiting; open it again"))?;
        let mut out = Vec::new();
        for n in names {
            out.extend(self.open_entry(docs, &path, n)?);
        }
        Ok(out)
    }

    /// Open one table of a zip as a tab with no file ("export.zip › a.csv").
    pub fn open_entry(&self, docs: &Docs, path: &Path, name: &str) -> CmdResult<Vec<DocInfo>> {
        if !is_table(name) {
            return Err(err("that is not a table in the zip"));
        }
        let data = read_zip(path)?;
        let zip = Zip::open(&data)?;
        let bytes = zip.read_bytes(name, MAX_ENTRY_BYTES)?;
        drop(zip);
        drop(data);
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let doc = Document::from_bytes(bytes, OpenOptions::default(), ext.as_deref())?;
        let short = name.rsplit('/').next().unwrap_or(name);
        let title = format!("{} › {short}", file_name(path));
        Ok(vec![docs.insert_titled(doc, Some(title))?])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = natendo_engine::zipwrite::ZipWriter::new(Vec::new());
        for (n, b) in files {
            let mut part = w.part(n).unwrap();
            std::io::Write::write_all(&mut part, b).unwrap();
            part.finish().unwrap();
        }
        w.finish().unwrap()
    }

    #[test]
    fn lists_and_opens_tables_in_memory() {
        let dir = std::env::temp_dir().join(format!("nde-zip-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let one = dir.join("one.zip");
        std::fs::write(&one, zip_of(&[("data/voters.csv", b"a,b\r\n1,2\r\n"), ("readme.md", b"hi")])).unwrap();
        let docs = Docs::default();
        let z = Zips::default();
        match z.list(&one).unwrap() {
            Listed::One(n) => {
                assert_eq!(n, "data/voters.csv");
                let info = z.open_entry(&docs, &one, &n).unwrap();
                assert_eq!(info[0].name, "one.zip › voters.csv");
                assert!(info[0].path.is_none());
            }
            Listed::Many(_) => panic!("one table"),
        }
        let two = dir.join("two.zip");
        std::fs::write(&two, zip_of(&[("a.csv", b"x\r\n1\r\n"), ("b.tsv", b"y\r\n2\r\n"), ("__MACOSX/._a.csv", b"")])).unwrap();
        let Listed::Many(list) = z.list(&two).unwrap() else { panic!("two tables") };
        assert_eq!(list.entries.len(), 2);
        let opened = z.open_picked(&docs, list.token, &["b.tsv".into()]).unwrap();
        assert_eq!(opened.len(), 1);
        // Nothing but the zips in the folder: no unpacked copies.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let none = dir.join("none.zip");
        std::fs::write(&none, zip_of(&[("readme.md", b"hi")])).unwrap();
        assert!(z.list(&none).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
