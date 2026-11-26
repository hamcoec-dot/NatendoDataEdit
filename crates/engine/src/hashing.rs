//! SHA-256 of files and bytes, and a canonical form of JSON whose hash
//! "seals" a receipt or a patch file.
//!
//! The canonical form is this app's own (it is not RFC 8785, "JCS"):
//!
//! 1. UTF-8, no spaces or line breaks.
//! 2. Object keys sorted by their UTF-8 bytes.
//! 3. Arrays keep their order.
//! 4. Strings: `"` and `\` escaped, control characters as `\b` `\t` `\n` `\f`
//!    `\r` or `\u00xx` (lower-case hex), everything else as it is (no other
//!    escapes, `/` unescaped). No Unicode normalisation: two spellings of the
//!    same letter are different text.
//! 5. Numbers: whole numbers within a signed or unsigned 64 bits, in decimal.
//!    Anything written with a fraction or an exponent is refused.
//! 6. `true`, `false` and `null`.
//!
//! A seal is tamper-evidence, not a signature: whoever rewrites a file can
//! compute a new seal.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

use crate::{Error, Progress};

/// Bytes read at a time.
const PIECE: usize = 1 << 20;
/// Deepest nesting read or written.
const MAX_DEPTH: usize = 64;

/// What a seal is for. The kind is part of the hash, so a seal made for one
/// kind of file never passes for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealKind {
    Receipt,
    Patch,
}

impl SealKind {
    fn name(self) -> &'static str {
        match self {
            SealKind::Receipt => "receipt",
            SealKind::Patch => "ndpatch",
        }
    }
}

/// Bytes as lower-case hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    s
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A reader that hashes every byte passing through it, so the bytes hashed
/// are the bytes parsed.
pub struct HashingReader<R> {
    inner: R,
    hash: Sha256,
    length: u64,
}

impl<R: Read> HashingReader<R> {
    pub fn new(inner: R) -> HashingReader<R> {
        HashingReader {
            inner,
            hash: Sha256::new(),
            length: 0,
        }
    }

    /// The hash and the number of bytes read, when that number is `expected`
    /// (the file's length); `None` otherwise: a partly read or changing file
    /// is not hashed.
    pub fn finish(self, expected: u64) -> Option<(String, u64)> {
        (self.length == expected).then(|| (hex(&self.hash.finalize()), self.length))
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hash.update(&buf[..n]);
        self.length += n as u64;
        Ok(n)
    }
}

/// The SHA-256 of a file in hex, and the number of bytes hashed. Read in
/// pieces; cancelling returns [`Error::Cancelled`], never a hash of part of
/// the file.
pub fn file_sha256(path: &Path, progress: &Progress) -> Result<(String, u64), Error> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; PIECE];
    let mut length = 0u64;
    loop {
        progress.step(0)?;
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        length += n as u64;
    }
    progress.step(0)?;
    Ok((hex(&h.finalize()), length))
}

fn invalid(what: &str) -> Error {
    Error::Invalid(format!("the data cannot be sealed: {what}"))
}

// ---- the canonical form ----

fn write_string(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for &b in s.as_bytes() {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            0x0C => out.extend_from_slice(b"\\f"),
            b'\r' => out.extend_from_slice(b"\\r"),
            0..=0x1F => {
                out.extend_from_slice(b"\\u00");
                out.extend_from_slice(hex(&[b]).as_bytes());
            }
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

fn write_object(
    m: &Map<String, Value>,
    skip: Option<&str>,
    depth: usize,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    if depth >= MAX_DEPTH {
        return Err(invalid("it is nested too deeply"));
    }
    let mut keys: Vec<&String> = m.keys().filter(|k| Some(k.as_str()) != skip).collect();
    keys.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    out.push(b'{');
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        write_string(k, out);
        out.push(b':');
        write_value(&m[k.as_str()], depth + 1, out)?;
    }
    out.push(b'}');
    Ok(())
}

fn write_value(v: &Value, depth: usize, out: &mut Vec<u8>) -> Result<(), Error> {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.extend_from_slice(i.to_string().as_bytes());
            } else if let Some(u) = n.as_u64() {
                out.extend_from_slice(u.to_string().as_bytes());
            } else {
                return Err(invalid("only whole numbers can be used"));
            }
        }
        Value::String(s) => write_string(s, out),
        Value::Array(a) => {
            if depth >= MAX_DEPTH {
                return Err(invalid("it is nested too deeply"));
            }
            out.push(b'[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(x, depth + 1, out)?;
            }
            out.push(b']');
        }
        Value::Object(m) => write_object(m, None, depth, out)?,
    }
    Ok(())
}

/// The canonical bytes of a value (see the module's description).
pub fn canonical_json(v: &Value) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    write_value(v, 0, &mut out)?;
    Ok(out)
}

// ---- strict reading ----

struct Strict {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Strict {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Strict {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON with whole numbers and no repeated keys")
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(Number::from(v)))
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Value, E> {
        Err(E::custom("only whole numbers can be used"))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }
    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        if self.depth >= MAX_DEPTH {
            return Err(A::Error::custom("nested too deeply"));
        }
        let mut out = Vec::new();
        while let Some(v) = a.next_element_seed(Strict {
            depth: self.depth + 1,
        })? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        if self.depth >= MAX_DEPTH {
            return Err(A::Error::custom("nested too deeply"));
        }
        let mut out = Map::new();
        let mut seen = HashSet::new();
        while let Some(k) = a.next_key::<String>()? {
            if !seen.insert(k.clone()) {
                // The key is not quoted: it may be text from a table.
                return Err(A::Error::custom("a key appears twice in one object"));
            }
            let v = a.next_value_seed(Strict {
                depth: self.depth + 1,
            })?;
            out.insert(k, v);
        }
        Ok(Value::Object(out))
    }
}

/// Read untrusted JSON for a sealed file: at most `max_bytes`, no object
/// with a key twice, whole numbers only, at most 64 levels deep.
pub fn parse_strict(bytes: &[u8], max_bytes: usize) -> Result<Value, Error> {
    if bytes.len() > max_bytes {
        return Err(Error::Invalid("the file is too large".into()));
    }
    let bad = |e: serde_json::Error| Error::Invalid(format!("the file cannot be read: {e}"));
    let mut d = serde_json::Deserializer::from_slice(bytes);
    let v = Strict { depth: 0 }.deserialize(&mut d).map_err(bad)?;
    d.end().map_err(bad)?;
    // A number too big for 64 bits arrives as a fraction and was refused.
    Ok(v)
}

// ---- seals ----

fn seal_of(map: &Map<String, Value>, kind: SealKind, field: &str) -> Result<String, Error> {
    let mut bytes = format!("natendo-seal-v1\n{}\n", kind.name()).into_bytes();
    // Only the top-level `field` is left out.
    write_object(map, Some(field), 0, &mut bytes)?;
    Ok(sha256_hex(&bytes))
}

/// Set `map[field]` to the SHA-256 of `"natendo-seal-v1\n" + kind + "\n"` and
/// the canonical form of `map` without that field.
pub fn seal(map: &mut Map<String, Value>, kind: SealKind, field: &str) -> Result<(), Error> {
    let s = seal_of(map, kind, field)?;
    map.insert(field.to_string(), Value::String(s));
    Ok(())
}

/// Whether `map[field]` is the seal of the rest of `map` for this kind:
/// exactly 64 lower-case hexadecimal digits that match.
pub fn check_seal(map: &Map<String, Value>, kind: SealKind, field: &str) -> bool {
    let Some(Value::String(have)) = map.get(field) else {
        return false;
    };
    have.len() == 64
        && have
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && seal_of(map, kind, field).is_ok_and(|want| &want == have)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::Ordering;

    fn strict(text: &str) -> Result<Value, Error> {
        parse_strict(text.as_bytes(), 1 << 20)
    }

    #[test]
    fn the_canonical_bytes_are_pinned() {
        let v = json!({
            "b": [1, -2, 18446744073709551615u64, true, false, null],
            "a": "\u{1}\u{8}\t\n\u{c}\r \"q\" \\ / \u{2028} \u{1F600} é",
            "é": {"z": 0, "Z": 0},
            "z": ""
        });
        let want = "{\"a\":\"\\u0001\\b\\t\\n\\f\\r \\\"q\\\" \\\\ / \u{2028} \u{1F600} é\",\
\"b\":[1,-2,18446744073709551615,true,false,null],\"z\":\"\",\"é\":{\"Z\":0,\"z\":0}}";
        let bytes = canonical_json(&v).unwrap();
        assert_eq!(String::from_utf8(bytes.clone()).unwrap(), want);
        assert_eq!(strict(want).unwrap(), v);
        assert_eq!(sha256_hex(&bytes), PINNED);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    const PINNED: &str = "4ef29a76aa6a1aeb2f02b1bfaf5839555ab8a360f9c0f6387a4c97f77861526c";

    #[test]
    fn spelling_differences_are_kept() {
        // No Unicode normalisation: composed and decomposed é differ.
        let a = canonical_json(&json!("\u{e9}")).unwrap();
        let b = canonical_json(&json!("e\u{301}")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn only_whole_numbers() {
        for text in [
            "1.0",
            "1e2",
            "1E2",
            "-1.5",
            "18446744073709551616",
            "-9223372036854775809",
            "[1,{\"a\":2.0}]",
        ] {
            assert!(strict(text).is_err(), "{text}");
        }
        assert!(canonical_json(&json!(1.5)).is_err());
        // "-0" reads as the fraction -0.0, so it is refused too.
        assert!(strict("-0").is_err());
        assert_eq!(strict("0").unwrap(), json!(0));
        assert_eq!(strict("-9223372036854775808").unwrap(), json!(i64::MIN));
    }

    #[test]
    fn strict_reading_refuses_repeats_depth_size_and_tails() {
        assert!(strict("{\"a\":1,\"a\":2}").is_err());
        assert!(strict("{\"x\":[{\"a\":1,\"b\":{},\"a\":1}]}").is_err());
        let e = strict("{\"secret\":1,\"secret\":2}").unwrap_err();
        assert!(!e.to_string().contains("secret"), "{e}");
        assert!(strict("{} x").is_err());
        assert!(parse_strict(b"[1,2,3]", 3).is_err());
        let deep = |n: usize| "[".repeat(n) + &"]".repeat(n);
        assert!(strict(&deep(64)).is_ok());
        assert!(strict(&deep(65)).is_err());
        assert!(canonical_json(&strict(&deep(64)).unwrap()).is_ok());
        assert!(strict(&deep(100_000)).is_err());
    }

    #[test]
    fn seals() {
        let mut m = strict("{\"b\":{\"seal\":\"inner\"},\"a\":1}")
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        assert!(!check_seal(&m, SealKind::Receipt, "seal"));
        seal(&mut m, SealKind::Receipt, "seal").unwrap();
        assert!(check_seal(&m, SealKind::Receipt, "seal"));
        // Another kind of file never accepts it.
        assert!(!check_seal(&m, SealKind::Patch, "seal"));
        // Key order does not matter.
        let text = format!(
            "{{\"seal\":{},\"a\":1,\"b\":{{\"seal\":\"inner\"}}}}",
            m["seal"]
        );
        let again = strict(&text).unwrap();
        assert!(check_seal(
            again.as_object().unwrap(),
            SealKind::Receipt,
            "seal"
        ));
        // A changed value fails, and so does a nested field of the same name.
        for (k, v) in [
            ("a", json!(2)),
            ("b", json!({"seal": "other"})),
            ("c", json!(0)),
        ] {
            let mut t = m.clone();
            t.insert(k.into(), v);
            assert!(!check_seal(&t, SealKind::Receipt, "seal"), "{k}");
        }
        let good = m["seal"].as_str().unwrap().to_string();
        for bad in [
            json!(good.to_uppercase()),
            json!(format!("{good} ")),
            json!(&good[..63]),
            json!(""),
            json!(5),
        ] {
            let mut t = m.clone();
            t.insert("seal".into(), bad);
            assert!(!check_seal(&t, SealKind::Receipt, "seal"));
        }
        // The domain line is part of the hash.
        let mut bytes = b"natendo-seal-v1\nreceipt\n".to_vec();
        bytes.extend(canonical_json(&json!({"a": 1, "b": {"seal": "inner"}})).unwrap());
        assert_eq!(good, sha256_hex(&bytes));
    }

    #[test]
    fn files_are_hashed_whole_or_not_at_all() {
        let path = std::env::temp_dir().join(format!("nd-hash-{}", std::process::id()));
        let data: Vec<u8> = (0..3 * PIECE + 7).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let p = Progress::new();
        let (h, n) = file_sha256(&path, &p).unwrap();
        assert_eq!(
            (h.as_str(), n),
            (sha256_hex(&data).as_str(), data.len() as u64)
        );
        let mut r = HashingReader::new(std::fs::File::open(&path).unwrap());
        let mut all = Vec::new();
        r.read_to_end(&mut all).unwrap();
        assert_eq!(r.finish(n), Some((h, n)));
        // Read only in part, or not the length the file has: no hash.
        let mut r = HashingReader::new(&data[..]);
        r.read_exact(&mut [0; 10]).unwrap();
        assert_eq!(r.finish(n), None);
        p.cancel.store(true, Ordering::Relaxed);
        assert!(matches!(file_sha256(&path, &p), Err(Error::Cancelled)));
        let _ = std::fs::remove_file(&path);
    }

    proptest::proptest! {
        /// Canonical text reads back as the same value and does not change.
        #[test]
        fn canonical_text_is_stable(
            entries in proptest::collection::vec((".{0,8}", ".{0,12}", proptest::num::i64::ANY), 0..12),
        ) {
            let mut m = Map::new();
            for (k, s, n) in entries {
                m.insert(k, json!([s, n, {"n": n}]));
            }
            let v = Value::Object(m);
            let once = canonical_json(&v).unwrap();
            let back = parse_strict(&once, once.len()).unwrap();
            proptest::prop_assert_eq!(&back, &v);
            proptest::prop_assert_eq!(canonical_json(&back).unwrap(), once);
        }
    }
}
