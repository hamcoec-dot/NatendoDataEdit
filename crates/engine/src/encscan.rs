//! Encoding tools (B2): look for rows whose bytes are not in the encoding the file is read as, preview a file read
//! in another encoding, and undo the "mojibake" a wrong guess makes (UTF-8 text read as Windows-1252, then saved).
//!
//! Nothing here changes a document: the scan only counts and marks rows, the preview reads a throwaway copy of the
//! head of the file, and the fix is a cleaning step (see `clean.rs`).

use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress, RowSource};
use crate::encoding::Encoding;
use crate::ops::CHUNK;
use crate::{Error, OpenOptions};
use rayon::prelude::*;

/// What a scan of the raw row bytes found. Counts are of shown data rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodingScan {
    /// Rows looked at.
    pub looked: u64,
    /// Rows of plain ASCII (the same in every encoding).
    pub ascii: u64,
    /// Rows that are valid UTF-8 and hold at least one multi-byte character.
    pub utf8: u64,
    /// Rows that are not valid UTF-8 (probably Windows-1252 or another legacy encoding).
    pub invalid: u64,
    /// Valid UTF-8 rows that look double-encoded (`Ã©` for `é`): the Fix mojibake clean-up step repairs them.
    pub double_encoded: u64,
    /// Valid multi-byte UTF-8 rows and invalid rows in one file: two encodings are mixed.
    pub mixed: bool,
    /// Row ids that are invalid or double-encoded (the rows "show only these rows" keeps), as a bit set.
    #[serde(skip)]
    pub failing: Vec<u64>,
    /// How many rows are in `failing`.
    pub rows: u64,
}

/// A raw row that holds a byte pair UTF-8 text makes when it was read as Windows-1252 and written again:
/// `Ã` + something (C3 83), `Â` (C3 82), or `â€` (C3 A2 E2 82 AC). A cheap test before the real one.
fn may_be_double_encoded(bytes: &[u8]) -> bool {
    bytes
        .windows(2)
        .any(|w| w == [0xC3, 0x83] || w == [0xC3, 0x82])
        || bytes
            .windows(5)
            .any(|w| w == [0xC3, 0xA2, 0xE2, 0x82, 0xAC])
}

/// How one raw row classifies.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Ascii,
    Utf8,
    Invalid,
    Double,
}

fn classify(bytes: &[u8]) -> Class {
    if bytes.is_ascii() {
        return Class::Ascii;
    }
    match std::str::from_utf8(bytes) {
        Err(_) => Class::Invalid,
        Ok(text) => {
            if may_be_double_encoded(bytes)
                && text
                    .split(',')
                    .any(|c| crate::encoding::fix_mojibake(c).is_some())
            {
                Class::Double
            } else {
                Class::Utf8
            }
        }
    }
}

impl Document {
    /// Classify the shown data rows by their raw bytes (one parallel pass, with progress and Cancel). Only for a
    /// file read in place (UTF-8 and the single-byte encodings): a file read as UTF-16 or Shift_JIS has been
    /// decoded already, so every row would be valid.
    pub fn scan_encodings(
        &self,
        keep_first: bool,
        progress: &Progress,
    ) -> Result<EncodingScan, Error> {
        self.ready_for_bulk()?;
        if !self.format().encoding.is_direct() {
            return Err(Error::Invalid(format!(
                "this file was read as {}, so its bytes were decoded when it opened and every row is valid. Reopen it as another encoding to compare.",
                self.format().encoding
            )));
        }
        let rows = self.data_rows(keep_first);
        let (r0, r1) = (rows.start, rows.end);
        progress.start(r1.saturating_sub(r0));
        let chunks = (r1.saturating_sub(r0) as usize).div_ceil(CHUNK);
        type Part = ([u64; 4], Vec<u32>);
        let parts: Vec<Part> = (0..chunks)
            .into_par_iter()
            .map(|c| -> Result<Part, Error> {
                let s = r0 + (c * CHUNK) as u64;
                let e = (s + CHUNK as u64).min(r1);
                let mut n = [0u64; 4];
                let mut ids = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    // An edited row holds text, which is valid by construction.
                    let class = match self.row_source(id) {
                        RowSource::Raw { bytes, .. } => classify(bytes),
                        RowSource::Edited(_) => Class::Ascii,
                    };
                    match class {
                        Class::Ascii => n[0] += 1,
                        Class::Utf8 => n[1] += 1,
                        Class::Invalid => {
                            n[2] += 1;
                            ids.push(id);
                        }
                        Class::Double => {
                            n[3] += 1;
                            ids.push(id);
                        }
                    }
                }
                progress.step(e - s)?;
                Ok((n, ids))
            })
            .collect::<Result<_, _>>()?;
        let base = self.next_row_id();
        let mut scan = EncodingScan {
            looked: r1.saturating_sub(r0),
            failing: vec![0u64; (base as usize).div_ceil(64)],
            ..Default::default()
        };
        for (n, ids) in parts {
            scan.ascii += n[0];
            scan.utf8 += n[1];
            scan.invalid += n[2];
            scan.double_encoded += n[3];
            for id in ids {
                scan.failing[(id >> 6) as usize] |= 1 << (id & 63);
                scan.rows += 1;
            }
        }
        scan.mixed = scan.utf8 + scan.double_encoded > 0 && scan.invalid > 0;
        Ok(scan)
    }

    /// A plan that shows only the rows a scan flagged (the title row stays with `keep_first`): the same row-set
    /// route as "show rows with problems".
    pub fn plan_show_encoding_rows(
        &self,
        scan: &EncodingScan,
        keep_first: bool,
    ) -> Result<crate::reshape::RowPlan, Error> {
        let rep = crate::validate::CheckReport {
            rows: scan.rows,
            failing: scan.failing.clone(),
            ..Default::default()
        };
        self.plan_show_problems(&rep, keep_first, false)
    }

    /// The first `rows` rows of `head` (the first bytes of the file as they are on disk) read as `enc`, laid out like
    /// this document. A throwaway copy: this document does not change. The last row of a cut-off head can be short.
    pub fn preview_decoded(
        &self,
        head: &[u8],
        enc: Encoding,
        rows: usize,
    ) -> Result<Vec<Vec<String>>, Error> {
        let opts = OpenOptions {
            encoding: Some(enc),
            dialect: Some(self.format().dialect),
            skip_lines: Some(self.preamble_lines()),
            ..Default::default()
        };
        let d = Document::from_bytes(head.to_vec(), opts, None)?;
        let n = d.row_count().min(rows as u64);
        (0..n).map(|r| d.row_values(r)).collect()
    }
}

/// The encodings the picker offers, in this order: (label that `Encoding::from_label` knows, name shown).
pub fn picker_encodings() -> Vec<(&'static str, &'static str)> {
    vec![
        ("utf-8", "UTF-8"),
        ("windows-1252", "Windows-1252 / Latin-1"),
        ("utf-16le", "UTF-16 LE"),
        ("utf-16be", "UTF-16 BE"),
        ("utf-32le", "UTF-32 LE"),
        ("utf-32be", "UTF-32 BE"),
        ("shift_jis", "Shift_JIS"),
        ("gbk", "GBK"),
        ("big5", "Big5"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(bytes: &[u8]) -> Document {
        Document::from_bytes(bytes.to_vec(), OpenOptions::default(), None).unwrap()
    }

    fn pr() -> Progress {
        Progress::new()
    }

    #[test]
    fn classifies_ascii_utf8_invalid_and_double_encoded_rows() {
        // Rows: ascii, valid UTF-8 (é), a Windows-1252 byte (0xE9 alone), double-encoded (Ã©), ascii.
        let mut b = b"name,city\r\n".to_vec();
        b.extend_from_slice(b"Ann,Oslo\r\n");
        b.extend_from_slice("Zo\u{e9},Li\u{e8}ge\r\n".as_bytes());
        b.extend_from_slice(b"Jos\xe9,Lyon\r\n");
        b.extend_from_slice("Ren\u{c3}\u{a9}e,Nice\r\n".as_bytes());
        b.extend_from_slice(b"Bob,Rome\r\n");
        // Detection would call a file with invalid UTF-8 Windows-1252: read it as UTF-8, as a person who sees mixed
        // rows would.
        let opts = OpenOptions {
            encoding: Some(Encoding::Utf8),
            ..Default::default()
        };
        let d = Document::from_bytes(b.clone(), opts, None).unwrap();
        let s = d.scan_encodings(true, &pr()).unwrap();
        assert_eq!(
            (s.looked, s.ascii, s.utf8, s.invalid, s.double_encoded),
            (5, 2, 1, 1, 1)
        );
        assert!(
            s.mixed,
            "valid multi-byte rows and invalid rows are two encodings"
        );
        assert_eq!(s.rows, 2);
    }

    #[test]
    fn a_plain_utf8_file_is_not_mixed_and_a_utf16_file_says_why_it_cannot_scan() {
        let d = doc("a,b\r\nz\u{e9},1\r\n".as_bytes());
        let s = d.scan_encodings(true, &pr()).unwrap();
        assert_eq!((s.invalid, s.double_encoded, s.mixed), (0, 0, false));
        let mut bytes = Vec::new();
        crate::encoding::encode("a;b\r\n\u{e9};1\r\n", Encoding::Utf16Le, &mut bytes).unwrap();
        let mut with_bom = b"\xFF\xFE".to_vec();
        with_bom.extend_from_slice(&bytes);
        let d = doc(&with_bom);
        let e = d.scan_encodings(true, &pr()).unwrap_err().to_string();
        assert!(e.contains("decoded when it opened"), "{e}");
    }

    #[test]
    fn the_rows_found_can_be_shown_by_the_row_set_route() {
        let mut b = b"name\r\n".to_vec();
        b.extend_from_slice(b"ok\r\n");
        b.extend_from_slice(b"Jos\xe9\r\n");
        b.extend_from_slice(b"ok too\r\n");
        let opts = OpenOptions {
            encoding: Some(Encoding::Utf8),
            ..Default::default()
        };
        let mut d = Document::from_bytes(b, opts, None).unwrap();
        let s = d.scan_encodings(true, &pr()).unwrap();
        let plan = d.plan_show_encoding_rows(&s, true).unwrap();
        d.apply_rows(plan).unwrap();
        assert_eq!(d.row_count(), 2, "the title row and the one bad row");
    }

    /// Fixtures of made-up text in several encodings: the preview reads the head of the file as each one.
    #[test]
    fn preview_reads_the_same_file_as_each_encoding() {
        let text = "name;city\r\nJos\u{e9};Z\u{fc}rich\r\n\u{e9}t\u{e9};Li\u{e8}ge\r\n";
        let d = doc(text.as_bytes());
        let latin = Encoding::from_label("windows-1252").unwrap();
        for enc in [Encoding::Utf8, latin] {
            let mut bytes = Vec::new();
            crate::encoding::encode(text, enc, &mut bytes).unwrap();
            let rows = d.preview_decoded(&bytes, enc, 5).unwrap();
            assert_eq!(rows[1], ["Jos\u{e9}", "Z\u{fc}rich"], "{enc}");
            assert_eq!(rows[2], ["\u{e9}t\u{e9}", "Li\u{e8}ge"], "{enc}");
        }
        // UTF-16 LE and BE, UTF-32.
        for enc in [
            Encoding::Utf16Le,
            Encoding::Utf16Be,
            Encoding::Utf32Le,
            Encoding::Utf32Be,
        ] {
            let mut bytes = Vec::new();
            crate::encoding::encode(text, enc, &mut bytes).unwrap();
            let rows = d.preview_decoded(&bytes, enc, 5).unwrap();
            assert_eq!(rows[1], ["Jos\u{e9}", "Z\u{fc}rich"], "{enc}");
        }
        // Shift_JIS, GBK and Big5 are exact for their own text (a decode that was not would be listed as unsupported).
        let sjis = "name;city\r\n\u{6771}\u{4eac};\u{5927}\u{962a}\r\n";
        let gbk = "name;city\r\n\u{4e2d}\u{6587};\u{5317}\u{4eac}\r\n";
        let big5 = "name;city\r\n\u{4e2d}\u{6587};\u{81fa}\u{5317}\r\n";
        for (label, text) in [("shift_jis", sjis), ("gbk", gbk), ("big5", big5)] {
            let enc = Encoding::from_label(label).unwrap();
            let mut bytes = Vec::new();
            crate::encoding::encode(text, enc, &mut bytes).unwrap();
            let rows = d.preview_decoded(&bytes, enc, 5).unwrap();
            let want: Vec<&str> = text.lines().nth(1).unwrap().split(';').collect();
            assert_eq!(rows[1], want, "{label} round trip");
        }
        // The same Windows-1252 bytes read as UTF-8 show the replacement character, not the text.
        let mut bytes = Vec::new();
        crate::encoding::encode(text, latin, &mut bytes).unwrap();
        let as_utf8 = d.preview_decoded(&bytes, Encoding::Utf8, 5).unwrap();
        assert!(as_utf8[1][0].contains('\u{fffd}'));
    }

    #[test]
    fn the_picker_list_is_all_known_labels() {
        for (label, name) in picker_encodings() {
            assert!(Encoding::from_label(label).is_some(), "{label}");
            assert!(!name.is_empty());
        }
    }
}
