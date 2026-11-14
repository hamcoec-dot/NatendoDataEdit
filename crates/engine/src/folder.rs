//! Search a folder of files without opening them in tabs (Milestone 14):
//! every delimited file in a folder (and, if asked, its subfolders) is read
//! the same way Open reads it, searched with the same patterns as Find, and
//! let go again. Files are searched one after another (each search is
//! parallel inside); Cancel stops at the next file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::find::{FindSpec, Pattern, Scope};
use crate::{Error, OpenOptions};

/// Most files looked at in one search.
pub const MAX_FILES: usize = 20_000;
/// Most hits kept in all, and per file.
pub const MAX_HITS: usize = 10_000;
const MAX_HITS_PER_FILE: usize = 1_000;
/// Folders deeper than this are not entered.
const MAX_DEPTH: usize = 16;

/// File types searched when none are given.
pub const DEFAULT_TYPES: &[&str] = &["csv", "tsv", "tab", "txt", "psv", "dat", "ssv"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderSearch {
    pub pattern: Pattern,
    /// Look in subfolders too.
    #[serde(default)]
    pub subfolders: bool,
    /// File name endings to search (without the dot); empty: the usual
    /// table files.
    #[serde(default)]
    pub types: Vec<String>,
}

/// A cell found in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderHit {
    pub path: PathBuf,
    /// Row in the file (0-based; the title row is row 0).
    pub row: u64,
    pub col: u32,
    /// The cell's text around the match.
    pub text: String,
    pub start: u32,
    pub len: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderReport {
    pub hits: Vec<FolderHit>,
    /// Matching cells in all (past those listed too).
    pub cells: u64,
    pub files: u64,
    pub files_with_hits: u64,
    /// Files that could not be read, with why.
    pub skipped: Vec<(PathBuf, String)>,
    /// More files were in the folder than are searched at once.
    pub too_many_files: bool,
}

/// The files to search, sorted by path.
fn files_in(dir: &Path, spec: &FolderSearch) -> Result<(Vec<PathBuf>, bool), Error> {
    let types: Vec<String> = if spec.types.is_empty() {
        DEFAULT_TYPES.iter().map(|t| (*t).to_string()).collect()
    } else {
        spec.types
            .iter()
            .map(|t| t.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    };
    let mut out = Vec::new();
    let mut more = false;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(rd) => rd,
            Err(e) if depth == 0 => return Err(e.into()),
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            // Links are not followed (no loops, nothing outside the folder).
            if ft.is_symlink() {
                continue;
            }
            let p = e.path();
            if ft.is_dir() {
                if spec.subfolders && depth < MAX_DEPTH {
                    stack.push((p, depth + 1));
                }
            } else if ft.is_file()
                && p.extension()
                    .is_some_and(|x| types.contains(&x.to_string_lossy().to_ascii_lowercase()))
            {
                if out.len() >= MAX_FILES {
                    more = true;
                    break;
                }
                out.push(p);
            }
        }
    }
    out.sort();
    Ok((out, more))
}

/// Search every table file in `dir`. `progress` counts files; its cancel
/// flag stops the search before the next file.
pub fn search_folder(
    dir: &Path,
    spec: &FolderSearch,
    progress: &Progress,
) -> Result<FolderReport, Error> {
    crate::find::Matcher::new(&spec.pattern)?;
    if spec.pattern.text.is_empty() {
        return Err(Error::Invalid("type what to look for".into()));
    }
    let (files, more) = files_in(dir, spec)?;
    let mut report = FolderReport {
        too_many_files: more,
        ..Default::default()
    };
    progress.start(files.len() as u64);
    let find = FindSpec {
        pattern: spec.pattern.clone(),
        scope: Scope::All,
        skip_first: false,
    };
    for path in files {
        progress.step(0)?;
        report.files += 1;
        let doc = Document::open(&path, OpenOptions::default()).and_then(|mut d| {
            while !d.index_step(64 << 20)? {
                if progress.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
            }
            Ok(d)
        });
        let doc = match doc {
            Ok(d) => d,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => {
                report.skipped.push((path, e.to_string()));
                progress.step(1)?;
                continue;
            }
        };
        let room = MAX_HITS
            .saturating_sub(report.hits.len())
            .min(MAX_HITS_PER_FILE);
        match doc.find_all(&find, room, &Progress::new()) {
            Ok(found) => {
                if found.cells > 0 {
                    report.files_with_hits += 1;
                }
                report.cells += found.cells;
                report
                    .hits
                    .extend(found.hits.into_iter().map(|h| FolderHit {
                        path: path.clone(),
                        row: h.number,
                        col: h.col,
                        text: h.text,
                        start: h.start,
                        len: h.len,
                    }));
            }
            Err(e) => report.skipped.push((path, e.to_string())),
        }
        progress.step(1)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::PatternKind;

    #[test]
    fn searches_files_in_a_folder() {
        let dir = std::env::temp_dir().join(format!("natendo-folder-{}", std::process::id()));
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("a.csv"), "name,city\r\nAnn,Reno\r\nBo,Reno\r\n").unwrap();
        std::fs::write(dir.join("b.tsv"), "x\ty\r\nreno\t1\r\n").unwrap();
        std::fs::write(dir.join("c.xlsx"), "not searched").unwrap();
        std::fs::write(sub.join("d.csv"), "q\r\nReno\r\n").unwrap();
        let spec = FolderSearch {
            pattern: Pattern {
                text: "reno".into(),
                kind: PatternKind::Plain,
                match_case: false,
                whole_cell: true,
            },
            subfolders: false,
            types: Vec::new(),
        };
        let p = Progress::new();
        let r = search_folder(&dir, &spec, &p).unwrap();
        assert_eq!((r.files, r.files_with_hits, r.cells), (2, 2, 3));
        assert_eq!(r.hits[0].path, dir.join("a.csv"));
        assert_eq!((r.hits[0].row, r.hits[0].col), (1, 1));
        assert_eq!((r.hits[2].row, r.hits[2].col), (1, 0));
        let deep = FolderSearch {
            subfolders: true,
            ..spec.clone()
        };
        assert_eq!(search_folder(&dir, &deep, &p).unwrap().files, 3);
        let only = FolderSearch {
            types: vec![".TSV".into()],
            ..spec.clone()
        };
        assert_eq!(search_folder(&dir, &only, &p).unwrap().cells, 1);
        // Cancel stops it.
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            search_folder(&dir, &spec, &p),
            Err(Error::Cancelled)
        ));
        // A missing folder is an error.
        assert!(search_folder(&dir.join("nope"), &spec, &Progress::new()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
