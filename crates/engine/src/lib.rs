//! Natendo Data Edit engine.
//!
//! Pure Rust, no UI or Tauri dependencies. Opens delimited text files of any
//! size, reads windows of cells, holds edits without touching the original
//! bytes, and saves safely.
//!
//! ```no_run
//! use natendo_engine::{Document, OpenOptions, SaveOptions};
//! let mut doc = Document::open_indexed("data.csv".as_ref(), OpenOptions::default(), None)?;
//! let view = doc.viewport(0..50, 0..10, 4096)?;
//! doc.set_cell(1, 2, "new value".into())?;
//! doc.save(SaveOptions::default())?;
//! # Ok::<(), natendo_engine::Error>(())
//! ```

#![deny(unsafe_code)]

pub mod abbrev;
pub mod areas;
pub mod cellformula;
pub mod changes;
pub mod checkfile;
pub mod chips;
pub mod clean;
pub mod colcompare;
pub mod coltotals;
pub mod coltype;
pub mod combine;
pub mod contract;
pub mod counts;
pub mod crypt;
pub mod datescan;
pub mod dbf;
pub mod detect;
pub mod dialect;
pub mod dictionary;
pub mod diff;
pub mod disk;
pub mod document;
pub mod editing;
pub mod encoding;
pub mod encscan;
pub mod example;
pub mod explode;
pub mod export;
pub mod expr;
pub mod fill;
pub mod filter;
pub mod find;
pub mod fixed;
pub mod folder;
pub mod fuzzy;
pub mod gaps;
pub mod golden;
pub mod hashing;
pub mod highlight;
pub mod import;
pub mod index;
pub mod insight;
pub mod join;
pub mod jsonshape;
pub mod keyset;
pub mod lintsuggest;
pub mod logs;
pub mod lookalike;
pub mod mail;
pub mod notes;
pub mod ops;
pub mod patch;
pub mod pdf;
pub mod pivot;
pub mod privacy;
pub mod profile;
pub mod receipt;
pub mod recipe;
pub mod relations;
pub mod reshape;
pub mod rows;
pub mod rulecard;
pub mod save;
pub mod scan;
pub mod sets;
pub mod sheet;
pub mod shift;
pub mod sort;
pub mod source;
pub mod special;
pub mod sqlite;
pub mod stats;
pub mod streets;
pub mod suggest;
pub mod sysmem;
pub mod table;
pub mod tablecheck;
pub mod tidy;
pub mod titles;
pub mod tools;
pub mod ttf;
pub mod updates;
pub mod validate;
pub mod value;
pub mod viewport;
pub mod xlsxbook;
pub mod xray;
pub mod zipread;
pub mod zipwrite;

use std::fmt;
use std::path::PathBuf;

pub use detect::{Choice, Detection};
pub use dialect::{Dialect, LineBreak};
pub use document::Progress;
pub use document::{Document, Format, OpenOptions};
pub use encoding::Encoding;
pub use save::{SaveOptions, SaveReport};
pub use scan::Quoting;
pub use viewport::Viewport;

/// Engine version, shown in the app's About box.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Everything that can go wrong. Row and column numbers are 0-based here and
/// shown 1-based in messages.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidDialect(&'static str),
    /// More than 4,294,967,295 rows.
    TooManyRows,
    /// The file is still being indexed.
    StillLoading,
    OutOfRange,
    ViewportTooLarge,
    Cancelled,
    /// A new document has no file yet; use Save As.
    NoPath,
    /// Another program changed the file since it was opened.
    ChangedOnDisk,
    /// The target encoding cannot store a character.
    Unmappable {
        row: u64,
        ch: char,
        encoding: &'static str,
    },
    /// A value contains the delimiter or a line break and the dialect has no quotes.
    CannotRepresent {
        row: u64,
        col: u32,
    },
    /// The new file was written but could not replace the old one.
    SaveIncomplete {
        message: String,
        /// Where the complete saved text is, if it was kept.
        temp: Option<PathBuf>,
    },
    /// The file was saved, but reopening it failed; the document is now empty.
    SavedButReloadFailed {
        path: PathBuf,
        message: String,
    },
    /// A memory limit was reached (crafted or enormous file).
    OutOfMemory(&'static str),
    /// A row has too many fields to edit or list in full.
    RowTooWide {
        row: u64,
        fields: u64,
    },
    /// Saving would replace bytes that could not be decoded; pass
    /// `SaveOptions::allow_lossy` after the user agrees.
    Lossy {
        row: Option<u64>,
    },
    /// Reading back the written file gave different values; nothing was replaced.
    VerifyFailed {
        row: u64,
    },
    /// A find or filter pattern is not valid (message says why).
    BadPattern(String),
    /// The operation needs the filter off (for example moving rows).
    FilterActive,
    /// The document changed while a long operation was working; nothing was
    /// changed, try again.
    Changed,
    /// The request does not fit the document (message says why, in words
    /// meant for the user).
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::InvalidDialect(m) => write!(f, "invalid format: {m}"),
            Error::TooManyRows => f.write_str("the file has more rows than supported (4,294,967,295)"),
            Error::StillLoading => f.write_str("the file is still loading"),
            Error::OutOfRange => f.write_str("row or column out of range"),
            Error::ViewportTooLarge => f.write_str("too many cells requested at once"),
            Error::Cancelled => f.write_str("cancelled"),
            Error::NoPath => f.write_str("choose where to save this file"),
            Error::ChangedOnDisk => f.write_str("another program changed the file since it was opened"),
            Error::Unmappable { row, ch, encoding } => write!(
                f,
                "row {} contains '{ch}' (U+{:04X}), which {encoding} cannot store",
                row + 1,
                *ch as u32
            ),
            Error::CannotRepresent { row, col } => write!(
                f,
                "row {}, column {} contains the delimiter or a line break, which this format cannot store without quotes",
                row + 1,
                col + 1
            ),
            Error::SaveIncomplete { message, temp } => {
                write!(f, "could not replace the file: {message}")?;
                if let Some(t) = temp {
                    write!(f, " (the saved text is in {})", t.display())?;
                }
                Ok(())
            }
            Error::SavedButReloadFailed { path, message } => write!(
                f,
                "saved {}, but reopening it failed ({message}); please open it again",
                path.display()
            ),
            Error::OutOfMemory(m) => f.write_str(m),
            Error::RowTooWide { row, fields } => write!(
                f,
                "row {} has {fields} fields, too many to edit as a whole row",
                row + 1
            ),
            Error::Lossy { row } => {
                f.write_str("saving would replace characters that could not be read")?;
                if let Some(r) = row {
                    write!(f, " (first in row {})", r + 1)?;
                }
                Ok(())
            }
            Error::VerifyFailed { row } => write!(
                f,
                "the saved file did not read back the same (row {}); the original was kept",
                row + 1
            ),
            Error::BadPattern(m) => write!(f, "the pattern is not valid: {m}"),
            Error::FilterActive => f.write_str(
                "turn the filter off and show hidden rows to move rows",
            ),
            Error::Changed => f.write_str(
                "the file changed while this was working; nothing was changed, please try again",
            ),
            Error::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::version().is_empty());
    }
}
