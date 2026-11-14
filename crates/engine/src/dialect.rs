//! Separator, quote character, escape style and line breaks.
//!
//! Guessing them from a file lives in [`crate::detect`]; the extension map
//! is here because it is part of what a dialect means for a file name.

use crate::Error;

/// Longest separator, in bytes (for example `~|~` or `||`).
pub const MAX_DELIM: usize = 4;

/// How fields and rows are separated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dialect {
    delim: [u8; MAX_DELIM],
    delim_len: u8,
    quote: Option<u8>,
    escape: Option<u8>,
    merge: bool,
}

/// A byte allowed in a separator, quote or escape: ASCII, not a letter or
/// digit, not CR, LF or NUL. That keeps them unambiguous in every encoding
/// the parser reads in place.
fn allowed(b: u8) -> bool {
    b.is_ascii() && !b.is_ascii_alphanumeric() && !matches!(b, b'\r' | b'\n' | 0)
}

impl Dialect {
    /// Comma-separated with double quotes.
    pub const CSV: Dialect = Dialect::raw(b',', Some(b'"'));
    /// Tab-separated with double quotes.
    pub const TSV: Dialect = Dialect::raw(b'\t', Some(b'"'));

    const fn raw(delimiter: u8, quote: Option<u8>) -> Dialect {
        Dialect {
            delim: [delimiter, 0, 0, 0],
            delim_len: 1,
            quote,
            escape: None,
            merge: false,
        }
    }

    /// A one-byte separator with an optional quote character (quotes inside
    /// a quoted field are doubled).
    pub fn new(delimiter: u8, quote: Option<u8>) -> Result<Dialect, Error> {
        Dialect::custom(&[delimiter], quote, None, false)
    }

    /// Any dialect the reader supports:
    /// * `delimiter`: 1 to [`MAX_DELIM`] allowed bytes (`||`, `~|~`, ...);
    /// * `quote`: opens a quoted field at the start of a field;
    /// * `escape`: inside quotes, makes the next byte literal (for example a
    ///   backslash before a quote); needs a quote; doubled quotes still work;
    /// * `merge`: a run of the (one-byte) separator counts as one, and runs
    ///   at the start and end of a row are ignored (columns lined up with
    ///   spaces).
    pub fn custom(
        delimiter: &[u8],
        quote: Option<u8>,
        escape: Option<u8>,
        merge: bool,
    ) -> Result<Dialect, Error> {
        if delimiter.is_empty() || delimiter.len() > MAX_DELIM {
            return Err(Error::InvalidDialect("separator must be 1 to 4 characters"));
        }
        if !delimiter.iter().all(|&b| allowed(b)) {
            return Err(Error::InvalidDialect(
                "separator must be ASCII punctuation, space, tab or control characters",
            ));
        }
        if let Some(q) = quote {
            if !allowed(q) || delimiter.contains(&q) {
                return Err(Error::InvalidDialect(
                    "quote must be ASCII punctuation and differ from the separator",
                ));
            }
        }
        if let Some(e) = escape {
            if quote.is_none() || !allowed(e) || Some(e) == quote || delimiter.contains(&e) {
                return Err(Error::InvalidDialect(
                    "escape needs a quote and must differ from the quote and separator",
                ));
            }
        }
        if merge && delimiter.len() != 1 {
            return Err(Error::InvalidDialect(
                "only a one-character separator can merge repeats",
            ));
        }
        let mut delim = [0; MAX_DELIM];
        delim[..delimiter.len()].copy_from_slice(delimiter);
        Ok(Dialect {
            delim,
            delim_len: delimiter.len() as u8,
            quote,
            escape,
            merge,
        })
    }

    /// The separator's bytes.
    pub fn delimiter(&self) -> &[u8] {
        &self.delim[..self.delim_len as usize]
    }

    /// The separator as text (always ASCII).
    pub fn delimiter_str(&self) -> &str {
        std::str::from_utf8(self.delimiter()).unwrap_or(",")
    }

    pub fn quote(&self) -> Option<u8> {
        self.quote
    }

    pub fn escape(&self) -> Option<u8> {
        self.escape
    }

    /// Runs of the separator count as one.
    pub fn merges(&self) -> bool {
        self.merge
    }

    /// The same dialect with another quote (and no escape when unquoted).
    pub fn with_quote(&self, quote: Option<u8>) -> Result<Dialect, Error> {
        let escape = quote.and(self.escape);
        Dialect::custom(self.delimiter(), quote, escape, self.merge)
    }
}

/// Row terminator style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineBreak {
    CrLf,
    Lf,
    Cr,
}

impl LineBreak {
    pub fn bytes(&self) -> &'static [u8] {
        match self {
            LineBreak::CrLf => b"\r\n",
            LineBreak::Lf => b"\n",
            LineBreak::Cr => b"\r",
        }
    }

    /// The terminator at the end of a raw row, if any.
    pub fn at_end_of(row: &[u8]) -> Option<LineBreak> {
        if row.ends_with(b"\r\n") {
            Some(LineBreak::CrLf)
        } else if row.ends_with(b"\n") {
            Some(LineBreak::Lf)
        } else if row.ends_with(b"\r") {
            Some(LineBreak::Cr)
        } else {
            None
        }
    }
}

/// Delimiter suggested by a file extension (`csv`, `tsv`, ...).
pub fn delimiter_for_extension(ext: &str) -> Option<u8> {
    match ext.to_ascii_lowercase().as_str() {
        "csv" => Some(b','),
        "tsv" | "tab" => Some(b'\t'),
        "psv" => Some(b'|'),
        "ssv" => Some(b';'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_validation() {
        assert!(Dialect::new(b',', Some(b'"')).is_ok());
        assert!(Dialect::new(b'\t', None).is_ok());
        assert!(Dialect::new(0x1F, Some(b'"')).is_ok());
        assert!(Dialect::new(b'a', None).is_err());
        assert!(Dialect::new(b'\n', None).is_err());
        assert!(Dialect::new(0xE9, None).is_err());
        assert!(Dialect::new(b',', Some(b',')).is_err());
        assert!(Dialect::custom(b"||", Some(b'"'), None, false).is_ok());
        assert!(Dialect::custom(b"~|~", None, None, false).is_ok());
        assert!(Dialect::custom(b"", None, None, false).is_err());
        assert!(Dialect::custom(b"|||||", None, None, false).is_err());
        assert!(Dialect::custom(b"|\"|", Some(b'"'), None, false).is_err());
        assert!(Dialect::custom(b",", Some(b'"'), Some(b'\\'), false).is_ok());
        assert!(Dialect::custom(b",", None, Some(b'\\'), false).is_err());
        assert!(Dialect::custom(b",", Some(b'"'), Some(b'"'), false).is_err());
        assert!(Dialect::custom(b" ", Some(b'"'), None, true).is_ok());
        assert!(Dialect::custom(b"||", None, None, true).is_err());
        let d = Dialect::custom(b"~|~", Some(b'"'), None, false).unwrap();
        assert_eq!(d.delimiter(), b"~|~");
        assert_eq!(d.delimiter_str(), "~|~");
    }

    #[test]
    fn extension_map() {
        assert_eq!(delimiter_for_extension("TSV"), Some(b'\t'));
        assert_eq!(delimiter_for_extension("txt"), None);
    }

    #[test]
    fn line_break_at_end() {
        assert_eq!(LineBreak::at_end_of(b"a\r\n"), Some(LineBreak::CrLf));
        assert_eq!(LineBreak::at_end_of(b"a\n"), Some(LineBreak::Lf));
        assert_eq!(LineBreak::at_end_of(b"a\r"), Some(LineBreak::Cr));
        assert_eq!(LineBreak::at_end_of(b"a"), None);
    }
}
