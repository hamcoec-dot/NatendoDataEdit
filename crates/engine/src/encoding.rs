//! Text encodings: detection, decoding and strict encoding.
//!
//! The parser always works on an ASCII-compatible byte stream in which the
//! quote, the delimiter, CR and LF can only mean themselves. Encodings where
//! that holds for the original bytes are *direct* (parsed in place); all others
//! are *transcoded* to UTF-8 once when the file is opened.
//!
//! UTF-7 is never guessed (it has a history of being abused to smuggle
//! markup); it is used only when the file starts with a UTF-7 BOM or the
//! caller asks for it.

use std::borrow::Cow;
use std::fmt;

/// A text encoding the engine can read and write.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
    Utf7,
    /// Any other encoding known to `encoding_rs` (Windows-1252, Shift_JIS, ...).
    Legacy(&'static encoding_rs::Encoding),
}

impl fmt::Debug for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Bytes of the first `n` bytes of a sample that detection looks at.
pub const DETECT_SAMPLE: usize = 1 << 20;

impl Encoding {
    /// Display name, e.g. `UTF-8`, `windows-1252`.
    pub fn name(&self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf16Le => "UTF-16LE",
            Encoding::Utf16Be => "UTF-16BE",
            Encoding::Utf32Le => "UTF-32LE",
            Encoding::Utf32Be => "UTF-32BE",
            Encoding::Utf7 => "UTF-7",
            Encoding::Legacy(e) => e.name(),
        }
    }

    /// Look an encoding up by label (`utf-8`, `latin1`, `shift_jis`, `utf-32le` ...).
    pub fn from_label(label: &str) -> Option<Encoding> {
        let l = label.trim().to_ascii_lowercase();
        let own = match l.as_str() {
            "utf-8" | "utf8" => Some(Encoding::Utf8),
            "utf-16" | "utf-16le" | "utf16le" | "unicode" => Some(Encoding::Utf16Le),
            "utf-16be" | "utf16be" => Some(Encoding::Utf16Be),
            "utf-32" | "utf-32le" | "utf32le" => Some(Encoding::Utf32Le),
            "utf-32be" | "utf32be" => Some(Encoding::Utf32Be),
            "utf-7" | "utf7" => Some(Encoding::Utf7),
            _ => None,
        };
        own.or_else(|| encoding_rs::Encoding::for_label(l.as_bytes()).map(Encoding::from_rs))
    }

    fn from_rs(e: &'static encoding_rs::Encoding) -> Encoding {
        if e == encoding_rs::UTF_8 {
            Encoding::Utf8
        } else if e == encoding_rs::UTF_16LE {
            Encoding::Utf16Le
        } else if e == encoding_rs::UTF_16BE {
            Encoding::Utf16Be
        } else {
            Encoding::Legacy(e)
        }
    }

    /// True when the original bytes can be parsed in place.
    pub fn is_direct(&self) -> bool {
        match self {
            Encoding::Utf8 => true,
            Encoding::Legacy(e) => {
                // ASCII-compatible and every non-ASCII byte sequence is made of
                // bytes >= 0x80. Shift_JIS, Big5, GBK and GB18030 fail the second
                // test: their trail bytes include `|`, `\` and other ASCII.
                e.is_ascii_compatible()
                    && *e != encoding_rs::SHIFT_JIS
                    && *e != encoding_rs::BIG5
                    && *e != encoding_rs::GBK
                    && *e != encoding_rs::GB18030
                    && *e != encoding_rs::ISO_2022_JP
            }
            _ => false,
        }
    }

    /// Byte-order mark written when a file in this encoding is saved with one.
    pub fn bom(&self) -> &'static [u8] {
        match self {
            Encoding::Utf8 => b"\xEF\xBB\xBF",
            Encoding::Utf16Le => b"\xFF\xFE",
            Encoding::Utf16Be => b"\xFE\xFF",
            Encoding::Utf32Le => b"\xFF\xFE\x00\x00",
            Encoding::Utf32Be => b"\x00\x00\xFE\xFF",
            Encoding::Utf7 => b"+/v8-",
            Encoding::Legacy(_) => b"",
        }
    }
}

/// Result of encoding detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detected {
    pub encoding: Encoding,
    /// Length of the byte-order mark at the start of the file (0 if none).
    pub bom_len: usize,
    /// UTF-8 was chosen although the sample has a few invalid bytes.
    pub invalid_bytes: bool,
}

/// Find the BOM, if any.
pub fn bom_of(bytes: &[u8]) -> Option<Detected> {
    let d = |encoding, bom_len| {
        Some(Detected {
            encoding,
            bom_len,
            invalid_bytes: false,
        })
    };
    if bytes.starts_with(b"\xFF\xFE\x00\x00") {
        d(Encoding::Utf32Le, 4)
    } else if bytes.starts_with(b"\x00\x00\xFE\xFF") {
        d(Encoding::Utf32Be, 4)
    } else if bytes.starts_with(b"\xEF\xBB\xBF") {
        d(Encoding::Utf8, 3)
    } else if bytes.starts_with(b"\xFF\xFE") {
        d(Encoding::Utf16Le, 2)
    } else if bytes.starts_with(b"\xFE\xFF") {
        d(Encoding::Utf16Be, 2)
    } else if bytes.len() >= 4
        && bytes.starts_with(b"+/v")
        && matches!(bytes[3], b'8' | b'9' | b'+' | b'/')
    {
        // The UTF-7 BOM shares bits with the next character, so the decoder
        // drops the leading U+FEFF instead of skipping bytes here.
        d(Encoding::Utf7, 0)
    } else {
        None
    }
}

/// Detect the encoding of a file from its first bytes.
pub fn detect(sample: &[u8]) -> Detected {
    if let Some(d) = bom_of(sample) {
        return d;
    }
    let sample = &sample[..sample.len().min(DETECT_SAMPLE)];
    let found = |encoding, invalid_bytes| Detected {
        encoding,
        bom_len: 0,
        invalid_bytes,
    };
    if let Some(e) = guess_wide(sample) {
        return found(e, false);
    }
    let (valid_multibyte, invalid) = utf8_census(sample);
    if invalid == 0 {
        return found(Encoding::Utf8, false);
    }
    // A UTF-8 file with a few damaged bytes still has mostly valid multibyte
    // characters; a legacy file (é as one byte) has almost none.
    if valid_multibyte >= 8 && invalid * 20 <= valid_multibyte {
        return found(Encoding::Utf8, true);
    }
    let mut det = chardetng::EncodingDetector::new();
    det.feed(sample, true);
    found(Encoding::from_rs(det.guess(None, true)), false)
}

/// Count valid multibyte UTF-8 characters and invalid sequences in s,
/// ignoring a character cut off at the end of the sample.
fn utf8_census(mut s: &[u8]) -> (usize, usize) {
    let (mut valid, mut invalid) = (0, 0);
    loop {
        match std::str::from_utf8(s) {
            Ok(t) => {
                valid += t.chars().filter(|c| !c.is_ascii()).count();
                return (valid, invalid);
            }
            Err(e) => {
                let ok = std::str::from_utf8(&s[..e.valid_up_to()]).unwrap_or_default();
                valid += ok.chars().filter(|c| !c.is_ascii()).count();
                match e.error_len() {
                    None => return (valid, invalid),
                    Some(n) => {
                        invalid += 1;
                        s = &s[e.valid_up_to() + n..];
                    }
                }
            }
        }
    }
}

/// UTF-16/32 without a BOM, recognised by where the zero bytes fall in
/// mostly-ASCII text.
fn guess_wide(sample: &[u8]) -> Option<Encoding> {
    let s = &sample[..sample.len().min(4096) & !3];
    if s.len() < 8 {
        return None;
    }
    let mut zeros = [0usize; 4];
    for (i, &b) in s.iter().enumerate() {
        if b == 0 {
            zeros[i & 3] += 1;
        }
    }
    let quarter = s.len() / 4;
    let most = |n: usize| n * 10 >= quarter * 9;
    let few = |n: usize| n * 10 <= quarter;
    if few(zeros[0]) && most(zeros[1]) && most(zeros[2]) && most(zeros[3]) {
        return Some(Encoding::Utf32Le);
    }
    if most(zeros[0]) && most(zeros[1]) && most(zeros[2]) && few(zeros[3]) {
        return Some(Encoding::Utf32Be);
    }
    let even = zeros[0] + zeros[2];
    let odd = zeros[1] + zeros[3];
    let half = s.len() / 2;
    if odd * 10 >= half * 7 && even * 20 <= half {
        return Some(Encoding::Utf16Le);
    }
    if even * 10 >= half * 7 && odd * 20 <= half {
        return Some(Encoding::Utf16Be);
    }
    None
}

/// Decode a whole file body (BOM already removed) to UTF-8.
/// Returns the text and whether any invalid sequence was replaced.
pub fn decode_all(bytes: &[u8], enc: Encoding) -> (String, bool) {
    match enc {
        Encoding::Utf8 => match String::from_utf8_lossy(bytes) {
            Cow::Borrowed(s) => (s.to_owned(), false),
            Cow::Owned(s) => (s, true),
        },
        Encoding::Utf16Le | Encoding::Utf16Be => decode_utf16(bytes, enc == Encoding::Utf16Be),
        Encoding::Utf32Le | Encoding::Utf32Be => decode_utf32(bytes, enc == Encoding::Utf32Be),
        Encoding::Utf7 => {
            let (mut s, lossy) = decode_utf7(bytes);
            if s.starts_with('\u{FEFF}') {
                s.drain(..3);
            }
            (s, lossy)
        }
        Encoding::Legacy(e) => {
            let (s, lossy) = e.decode_without_bom_handling(bytes);
            (s.into_owned(), lossy)
        }
    }
}

/// Decode bytes of a direct-encoded file, reporting whether any invalid
/// sequence had to be replaced.
pub fn decode_direct_checked(bytes: &[u8], enc: Encoding) -> (Cow<'_, str>, bool) {
    match enc {
        Encoding::Legacy(e) => e.decode_without_bom_handling(bytes),
        _ => match String::from_utf8_lossy(bytes) {
            Cow::Borrowed(s) => (Cow::Borrowed(s), false),
            owned => (owned, true),
        },
    }
}

/// Decode one field of a direct-encoded file for display.
pub fn decode_direct(bytes: &[u8], enc: Encoding) -> Cow<'_, str> {
    match enc {
        Encoding::Legacy(e) => e.decode_without_bom_handling(bytes).0,
        // Transcoded working bytes are UTF-8 too.
        _ => String::from_utf8_lossy(bytes),
    }
}

fn decode_utf16(bytes: &[u8], be: bool) -> (String, bool) {
    let units = bytes.chunks_exact(2).map(|c| {
        if be {
            u16::from_be_bytes([c[0], c[1]])
        } else {
            u16::from_le_bytes([c[0], c[1]])
        }
    });
    let mut lossy = bytes.len() % 2 != 0;
    let mut out = String::with_capacity(bytes.len() / 2);
    for r in char::decode_utf16(units) {
        out.push(r.unwrap_or_else(|_| {
            lossy = true;
            char::REPLACEMENT_CHARACTER
        }));
    }
    if bytes.len() % 2 != 0 {
        out.push(char::REPLACEMENT_CHARACTER);
    }
    (out, lossy)
}

fn decode_utf32(bytes: &[u8], be: bool) -> (String, bool) {
    let mut lossy = bytes.len() % 4 != 0;
    let mut out = String::with_capacity(bytes.len() / 4);
    for c in bytes.chunks_exact(4) {
        let v = if be {
            u32::from_be_bytes([c[0], c[1], c[2], c[3]])
        } else {
            u32::from_le_bytes([c[0], c[1], c[2], c[3]])
        };
        out.push(char::from_u32(v).unwrap_or_else(|| {
            lossy = true;
            char::REPLACEMENT_CHARACTER
        }));
    }
    if bytes.len() % 4 != 0 {
        out.push(char::REPLACEMENT_CHARACTER);
    }
    (out, lossy)
}

fn b64_value(b: u8) -> Option<u32> {
    Some(match b {
        b'A'..=b'Z' => b - b'A',
        b'a'..=b'z' => b - b'a' + 26,
        b'0'..=b'9' => b - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    } as u32)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// RFC 2152 decoder. Strict: anything outside the rules becomes U+FFFD.
fn decode_utf7(bytes: &[u8]) -> (String, bool) {
    let mut out = String::with_capacity(bytes.len());
    let mut lossy = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'+' {
            i += 1;
            if bytes.get(i) == Some(&b'-') {
                out.push('+');
                i += 1;
                continue;
            }
            let mut acc: u32 = 0;
            let mut bits = 0u32;
            let mut units: Vec<u16> = Vec::new();
            let run_start = i;
            while let Some(v) = bytes.get(i).and_then(|&c| b64_value(c)) {
                acc = (acc << 6) | v;
                bits += 6;
                if bits >= 16 {
                    bits -= 16;
                    units.push((acc >> bits) as u16);
                    acc &= (1 << bits) - 1;
                }
                i += 1;
            }
            // Leftover bits must be fewer than 6 and all zero, and a `+` must
            // start a non-empty run (a bare `+` is not valid UTF-7).
            if bits >= 6 || acc != 0 || (i == run_start && bytes.get(i) != Some(&b'-')) {
                lossy = true;
            }
            for r in char::decode_utf16(units.iter().copied()) {
                out.push(r.unwrap_or_else(|_| {
                    lossy = true;
                    char::REPLACEMENT_CHARACTER
                }));
            }
            if bytes.get(i) == Some(&b'-') {
                i += 1;
            }
        } else if b < 0x80 {
            out.push(b as char);
            i += 1;
        } else {
            lossy = true;
            out.push(char::REPLACEMENT_CHARACTER);
            i += 1;
        }
    }
    (out, lossy)
}

fn utf7_direct(c: char) -> bool {
    matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9'
        | '\'' | '(' | ')' | ',' | '-' | '.' | '/' | ':' | '?'
        | ' ' | '\t' | '\r' | '\n'
        | '!' | '"' | '#' | '$' | '%' | '&' | '*' | ';' | '<' | '='
        | '>' | '@' | '[' | ']' | '^' | '_' | '`' | '{' | '|' | '}')
}

fn encode_utf7(text: &str, out: &mut Vec<u8>) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '+' {
            out.extend_from_slice(b"+-");
        } else if utf7_direct(c) {
            out.push(c as u8);
        } else {
            out.push(b'+');
            let mut acc: u32 = 0;
            let mut bits = 0u32;
            let push_char = |c: char, acc: &mut u32, bits: &mut u32, out: &mut Vec<u8>| {
                let mut buf = [0u16; 2];
                for &u in c.encode_utf16(&mut buf).iter() {
                    *acc = (*acc << 16) | u as u32;
                    *bits += 16;
                    while *bits >= 6 {
                        *bits -= 6;
                        out.push(B64[((*acc >> *bits) & 63) as usize]);
                    }
                    *acc &= (1 << *bits) - 1;
                }
            };
            push_char(c, &mut acc, &mut bits, out);
            while let Some(&n) = chars.peek() {
                if utf7_direct(n) || n == '+' {
                    break;
                }
                push_char(n, &mut acc, &mut bits, out);
                chars.next();
            }
            if bits > 0 {
                out.push(B64[((acc << (6 - bits)) & 63) as usize]);
            }
            // Always close explicitly: safe whatever follows.
            out.push(b'-');
        }
    }
}

/// The byte a character stands for when UTF-8 text was read as Windows-1252: its Windows-1252 byte, and for the five
/// bytes Windows-1252 leaves undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D) the Latin-1 control character, as most programs
/// read them.
fn cp1252_byte(c: char) -> Option<u8> {
    if (c as u32) < 0x80 {
        return Some(c as u8);
    }
    if matches!(c, '\u{81}' | '\u{8d}' | '\u{8f}' | '\u{90}' | '\u{9d}') {
        return Some(c as u8);
    }
    let mut buf = [0u8; 4];
    let (bytes, _, bad) = encoding_rs::WINDOWS_1252.encode(c.encode_utf8(&mut buf));
    (!bad && bytes.len() == 1).then(|| bytes[0])
}

/// How many characters of `s` look like leftovers of a wrong reading: Latin-1 supplement letters and symbols, the
/// Windows-1252 punctuation block, and the replacement character.
fn odd_chars(s: &str) -> usize {
    s.chars()
        .filter(|&c| {
            matches!(c as u32, 0x80..=0xFF | 0x152 | 0x153 | 0x160 | 0x161 | 0x178 | 0x17D | 0x17E | 0x192 | 0x2C6 | 0x2DC | 0x2013..=0x203A | 0x20AC | 0x2122 | 0xFFFD)
        })
        .count()
}

/// Undo "mojibake": text that was UTF-8, read as Windows-1252 and written out as UTF-8 again (`cafÃ©` for `café`,
/// `â€™` for a curly quote). `Some(fixed)` only when every character maps back to one Windows-1252 byte, those bytes
/// are valid UTF-8, the result differs, and it has fewer odd characters than the text had; `None` (leave it alone)
/// otherwise, so real text such as `café` is never touched.
pub fn fix_mojibake(text: &str) -> Option<String> {
    if text.is_ascii() {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len());
    for c in text.chars() {
        bytes.push(cp1252_byte(c)?);
    }
    let fixed = String::from_utf8(bytes).ok()?;
    if fixed == text
        || fixed
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
    {
        return None;
    }
    (odd_chars(&fixed) < odd_chars(text)).then_some(fixed)
}

/// A character the target encoding cannot store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmappable(pub char);

/// Append `text` encoded as `enc` to `out`. Never substitutes characters:
/// a character the encoding cannot represent is an error.
pub fn encode(text: &str, enc: Encoding, out: &mut Vec<u8>) -> Result<(), Unmappable> {
    match enc {
        Encoding::Utf8 => out.extend_from_slice(text.as_bytes()),
        Encoding::Utf16Le => text
            .encode_utf16()
            .for_each(|u| out.extend_from_slice(&u.to_le_bytes())),
        Encoding::Utf16Be => text
            .encode_utf16()
            .for_each(|u| out.extend_from_slice(&u.to_be_bytes())),
        Encoding::Utf32Le => text
            .chars()
            .for_each(|c| out.extend_from_slice(&(c as u32).to_le_bytes())),
        Encoding::Utf32Be => text
            .chars()
            .for_each(|c| out.extend_from_slice(&(c as u32).to_be_bytes())),
        Encoding::Utf7 => encode_utf7(text, out),
        Encoding::Legacy(e) => {
            if text.is_ascii() && e.is_ascii_compatible() {
                out.extend_from_slice(text.as_bytes());
                return Ok(());
            }
            let mut enc = e.new_encoder();
            let mut src = text;
            let mut buf = [0u8; 4096];
            loop {
                let (res, read, written) =
                    enc.encode_from_utf8_without_replacement(src, &mut buf, true);
                out.extend_from_slice(&buf[..written]);
                src = &src[read..];
                match res {
                    encoding_rs::EncoderResult::InputEmpty => break,
                    encoding_rs::EncoderResult::OutputFull => continue,
                    encoding_rs::EncoderResult::Unmappable(c) => return Err(Unmappable(c)),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Shared with the screen (ui/src/lib/mojibake.ts runs the same file), so the two copies cannot drift.
    #[test]
    fn fix_mojibake_corpus() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("testdata/mojibake.json")).unwrap();
        let cases = v["cases"].as_array().unwrap();
        assert!(cases.len() > 10);
        for c in cases {
            let text = c[0].as_str().unwrap();
            let want = c[1].as_str();
            assert_eq!(super::fix_mojibake(text).as_deref(), want, "{text:?}");
            // Idempotent: the fixed text is real text and is left alone.
            if let Some(w) = want {
                assert_eq!(super::fix_mojibake(w), None, "{w:?} twice");
            }
        }
    }

    /// The round trip the fix undoes: any text, written as UTF-8 and read as Windows-1252, comes back.
    #[test]
    fn fix_mojibake_undoes_a_real_wrong_reading() {
        for text in [
            "caf\u{e9} cr\u{e8}me",
            "Z\u{fc}rich \u{20ac}5",
            "O\u{2019}Neil \u{201c}x\u{201d}",
            "\u{c1}lvaro \u{cd}\u{f1}ez",
        ] {
            let (wrong, _, _) = encoding_rs::WINDOWS_1252.decode(text.as_bytes());
            let fixed =
                super::fix_mojibake(&wrong).unwrap_or_else(|| panic!("{text:?} read as {wrong:?}"));
            assert_eq!(fixed, text);
        }
    }
    use super::*;

    fn round_trip(text: &str, enc: Encoding) {
        let mut bytes = Vec::new();
        encode(text, enc, &mut bytes).unwrap();
        let (back, lossy) = decode_all(&bytes, enc);
        assert_eq!(back, text, "{enc}");
        assert!(!lossy, "{enc}");
    }

    #[test]
    fn unicode_round_trips() {
        let text = "id,name\r\n1,Zoë 🦀 \"q\"\n2,+plus ~tilde\\\n";
        for enc in [
            Encoding::Utf8,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
            Encoding::Utf32Le,
            Encoding::Utf32Be,
            Encoding::Utf7,
        ] {
            round_trip(text, enc);
        }
    }

    #[test]
    fn utf7_known_vectors() {
        assert_eq!(decode_utf7(b"Hi Mom -+Jjo--!").0, "Hi Mom -\u{263A}-!");
        assert_eq!(decode_utf7(b"A+ImIDkQ.").0, "A\u{2262}\u{0391}.");
        assert_eq!(decode_utf7(b"1 +- 1").0, "1 + 1");
        assert!(decode_utf7(b"+Jj\xFF").1);
        assert!(decode_utf7(b"a+!").1);
    }

    #[test]
    fn bom_detection() {
        assert_eq!(detect(b"\xEF\xBB\xBFa,b").encoding, Encoding::Utf8);
        assert_eq!(detect(b"\xEF\xBB\xBFa,b").bom_len, 3);
        assert_eq!(
            detect(b"\xFF\xFE\x00\x00a\0\0\0").encoding,
            Encoding::Utf32Le
        );
        assert_eq!(detect(b"\xFF\xFEa\0").encoding, Encoding::Utf16Le);
        assert_eq!(detect(b"\xFE\xFF\0a").encoding, Encoding::Utf16Be);
        assert_eq!(detect(b"+/v8-a").encoding, Encoding::Utf7);
    }

    #[test]
    fn wide_without_bom() {
        let mut le = Vec::new();
        encode("name,age\r\nann,3\r\n", Encoding::Utf16Le, &mut le).unwrap();
        assert_eq!(detect(&le).encoding, Encoding::Utf16Le);
        let mut be32 = Vec::new();
        encode("name,age\r\nann,3\r\n", Encoding::Utf32Be, &mut be32).unwrap();
        assert_eq!(detect(&be32).encoding, Encoding::Utf32Be);
    }

    #[test]
    fn utf8_and_legacy_guess() {
        assert_eq!(detect("a,é\n".as_bytes()).encoding, Encoding::Utf8);
        // Truncated multibyte character at the end of the sample is fine.
        assert_eq!(detect(&"a,é".as_bytes()[..3]).encoding, Encoding::Utf8);
        let latin = b"nom,ville\nRen\xE9,Qu\xE9bec\nJos\xE9,Montr\xE9al\n";
        let d = detect(latin);
        assert!(d.encoding.is_direct());
        assert_ne!(d.encoding, Encoding::Utf8);
        assert_eq!(decode_direct(b"Ren\xE9", d.encoding), "René");
    }

    #[test]
    fn mostly_valid_utf8_stays_utf8() {
        let mut s = "name,city\n".repeat(3).into_bytes();
        for _ in 0..20 {
            s.extend_from_slice("café,Zürich\n".as_bytes());
        }
        s.extend_from_slice(b"bad\xFFbyte,x\n");
        let d = detect(&s);
        assert_eq!(d.encoding, Encoding::Utf8);
        assert!(d.invalid_bytes);
        assert!(!detect("café\n".as_bytes()).invalid_bytes);
        assert_eq!(utf8_census("é\u{1F980}a".as_bytes()), (2, 0));
        assert_eq!(utf8_census(b"a\xFFb\xE9"), (0, 1));
    }

    #[test]
    fn utf7_is_never_guessed() {
        assert_eq!(detect(b"a+AGE-,b\n").encoding, Encoding::Utf8);
    }

    #[test]
    fn strict_legacy_encoding() {
        let w1252 = Encoding::from_label("windows-1252").unwrap();
        let mut out = Vec::new();
        encode("café", w1252, &mut out).unwrap();
        assert_eq!(out, b"caf\xE9");
        out.clear();
        assert_eq!(encode("π", w1252, &mut out), Err(Unmappable('π')));
    }

    #[test]
    fn direct_classification() {
        assert!(Encoding::Utf8.is_direct());
        assert!(Encoding::from_label("latin1").unwrap().is_direct());
        assert!(Encoding::from_label("euc-kr").unwrap().is_direct());
        assert!(!Encoding::from_label("shift_jis").unwrap().is_direct());
        assert!(!Encoding::from_label("big5").unwrap().is_direct());
        assert!(!Encoding::Utf16Le.is_direct());
        assert!(!Encoding::Utf7.is_direct());
    }
}
