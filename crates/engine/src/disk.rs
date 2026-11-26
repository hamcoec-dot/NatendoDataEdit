//! Noticing that another program changed an open file.
//!
//! A [`DiskMark`] is taken from a document without touching the disk, and
//! [`DiskMark::look`] compares the file with it. The two are separate so that
//! a look at a file on a share that has stopped answering never holds the
//! document.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::source::FileStamp;
use crate::{Document, Error, OpenOptions, Progress};

/// What a document knew about its file when it was read or last saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskMark {
    pub path: PathBuf,
    stamp: Option<FileStamp>,
    /// Content hash (files held in memory only).
    hash: Option<u64>,
    /// The file is mapped, so other programs cannot change it while it is
    /// open.
    pub mapped: bool,
}

/// How the file on disk compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outside {
    /// As the document knows it, or it could not be asked.
    Same,
    /// Written again with the same bytes: nothing to tell.
    Touched(FileStamp),
    Changed(FileStamp),
    /// Deleted or moved.
    Gone,
}

impl FileStamp {
    /// A short text that changes with the stamp.
    pub fn token(&self) -> String {
        let ns = self
            .modified
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        format!("{}-{ns}", self.len)
    }

    /// The modified time in milliseconds since 1970.
    pub fn modified_ms(&self) -> Option<u64> {
        let d = self.modified?.duration_since(UNIX_EPOCH).ok()?;
        u64::try_from(d.as_millis()).ok()
    }
}

impl DiskMark {
    /// Ask the disk. This can take long on a share that does not answer.
    pub fn look(&self) -> Outside {
        let now = match FileStamp::of(&self.path) {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::NotFound => return Outside::Gone,
            Err(_) => return Outside::Same,
        };
        if Some(now) == self.stamp || self.mapped {
            return Outside::Same;
        }
        // A new time on the same bytes (a copy tool, a "touch") is no change.
        let same_length = self.stamp.is_some_and(|s| s.len == now.len);
        match self.hash {
            Some(h)
                if same_length && crate::source::hash_file(&self.path).is_ok_and(|n| n == h) =>
            {
                Outside::Touched(now)
            }
            _ => Outside::Changed(now),
        }
    }
}

impl Document {
    /// What to compare the file with, or `None` when there is no file.
    pub fn disk_mark(&self) -> Option<DiskMark> {
        let s = self.source_ref();
        Some(DiskMark {
            path: s.path()?.to_path_buf(),
            stamp: s.stamp(),
            hash: s.file_hash(),
            mapped: s.is_mapped(),
        })
    }

    /// The layout this document reads its file with.
    fn same_layout(&self, direct_bytes: bool) -> OpenOptions {
        let f = self.format();
        OpenOptions {
            // Bytes held after decoding are UTF-8 already.
            encoding: Some(if direct_bytes && !f.encoding.is_direct() {
                crate::Encoding::Utf8
            } else {
                f.encoding
            }),
            dialect: Some(f.dialect),
            skip_lines: Some(self.preamble_lines()),
            ..Default::default()
        }
    }

    /// The table as it was last opened or saved, without the edits made
    /// since. Not for mapped files (they cannot have changed).
    pub fn as_last_read(&self) -> Result<Document, Error> {
        let s = self.source_ref();
        if s.is_mapped() {
            return Err(Error::Invalid(
                "this file is locked while open, so it cannot have changed".into(),
            ));
        }
        let ext = s
            .path()
            .and_then(Path::extension)
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        Document::from_bytes(s.bytes().to_vec(), self.same_layout(true), ext.as_deref())
    }

    /// Cut the table loose from its file: it is saved only with Save As.
    pub fn forget_path(&mut self) {
        self.source_mut().forget_path();
        self.mark_modified();
    }

    /// The file as it is on disk now, read the way this document reads it.
    pub fn as_on_disk(&self, progress: &Progress) -> Result<Document, Error> {
        let path = self
            .path()
            .ok_or_else(|| Error::Invalid("this table has no file".into()))?;
        Document::open_indexed(path, self.same_layout(false), Some(&progress.cancel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn file(name: &str, text: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("nd-disk-{name}-{}.csv", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p
    }

    fn open(p: &Path) -> Document {
        Document::open_indexed(p, OpenOptions::default(), None).unwrap()
    }

    /// Give the file a time that differs from any it had.
    fn later(p: &Path, secs: u64) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn changes_deletions_and_touches() {
        let p = file("look", "a,b\r\n1,2\r\n");
        let d = open(&p);
        let mark = d.disk_mark().unwrap();
        assert!(!mark.mapped);
        assert_eq!(mark.look(), Outside::Same);
        // The same bytes again with another time.
        later(&p, 5);
        assert!(matches!(mark.look(), Outside::Touched(_)));
        // Another file of the same length.
        std::fs::write(&p, "a,b\r\n1,3\r\n").unwrap();
        later(&p, 10);
        assert!(matches!(mark.look(), Outside::Changed(_)));
        std::fs::write(&p, "a,b\r\n1,2\r\n3,4\r\n").unwrap();
        assert!(matches!(mark.look(), Outside::Changed(_)));
        std::fs::remove_file(&p).unwrap();
        assert_eq!(mark.look(), Outside::Gone);
        let none = Document::from_bytes(b"a\r\n".to_vec(), OpenOptions::default(), Some("csv"));
        assert_eq!(none.unwrap().disk_mark(), None);
    }

    #[test]
    fn the_apps_own_save_moves_the_mark() {
        let p = file("save", "a,b\r\n1,2\r\n");
        let mut d = open(&p);
        let before = d.disk_mark().unwrap();
        d.set_cell(1, 0, "9".into()).unwrap();
        d.save(Default::default()).unwrap();
        let after = d.disk_mark().unwrap();
        assert_ne!(before, after, "a look taken before the save is out of date");
        assert_eq!(after.look(), Outside::Same);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_last_read_table_holds_no_unsaved_edits() {
        for text in [
            "a;b\r\n1;é\r\n".as_bytes().to_vec(),
            b"a\tb\r\n1\t\xe9\r\n".to_vec(),
            "\u{feff}a,b\r\n1,é\r\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
        ] {
            let p = file("base", "");
            std::fs::write(&p, &text).unwrap();
            let mut d = open(&p);
            d.set_cell(1, 0, "unsaved".into()).unwrap();
            let base = d.as_last_read().unwrap();
            assert_eq!(base.row_values(1).unwrap(), ["1", "é"]);
            // The other program changes a cell.
            let mut other = open(&p);
            other.set_cell(1, 1, "theirs".into()).unwrap();
            other.save(Default::default()).unwrap();
            let disk = d.as_on_disk(&Progress::new()).unwrap();
            assert_eq!(disk.row_values(1).unwrap(), ["1", "theirs"]);
            assert_eq!(base.row_values(1).unwrap(), ["1", "é"]);
            let _ = std::fs::remove_file(&p);
        }
    }
}
