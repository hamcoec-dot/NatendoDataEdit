//! Password-protected files (Milestone 15): a table saved as an encrypted
//! file (`.ndenc`) that only opens with its password.
//!
//! Format: `NDENC\0` + version (2 bytes) + rounds (4, little-endian) +
//! salt (16) + nonce prefix (7), then the contents in 1 MiB chunks, each
//! sealed with ChaCha20-Poly1305 under a key stretched from
//! the password (PBKDF2-HMAC-SHA-256 with the file's own random salt). Each
//! chunk's nonce is the prefix, the chunk number and a last-chunk flag, so
//! chunks cannot be reordered, dropped or cut off unnoticed. Any change to
//! the file, or a wrong password, fails the same way.

use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, Key, Nonce};

use crate::privacy::Secret;
use crate::Error;

const MAGIC: &[u8; 6] = b"NDENC\0";
const VERSION: u16 = 1;
const HEADER: usize = 6 + 2 + 4 + 16 + 7;
const CHUNK: usize = 1 << 20;
const TAG: usize = 16;
/// Password stretch rounds for new files.
pub const ROUNDS: u32 = 600_000;
/// Fewest characters in a password.
pub const MIN_PASSWORD_CHARS: usize = 12;
/// Largest contents (decrypted files are held in memory).
pub const MAX_PLAIN: usize = 8 << 30;

/// The file extension of encrypted files.
pub const EXTENSION: &str = "ndenc";

/// Whether bytes look like an encrypted file (its first bytes).
pub fn is_encrypted(head: &[u8]) -> bool {
    head.starts_with(MAGIC)
}

fn key_for(password: &Secret, salt: &[u8], rounds: u32) -> Key {
    let k = crate::privacy::pbkdf2(password.bytes(), salt, rounds);
    Key::clone_from_slice(&k)
}

fn nonce(prefix: &[u8], n: u32, last: bool) -> Nonce {
    let mut b = [0u8; 12];
    b[..7].copy_from_slice(prefix);
    b[7..11].copy_from_slice(&n.to_be_bytes());
    b[11] = u8::from(last);
    *Nonce::from_slice(&b)
}

fn rounds_for_new() -> u32 {
    if cfg!(test) {
        1_000
    } else {
        ROUNDS
    }
}

/// Encrypt `plain` with a password.
pub fn encrypt(plain: &[u8], password: &Secret) -> Result<Vec<u8>, Error> {
    if password.chars() < MIN_PASSWORD_CHARS {
        return Err(Error::Invalid(format!(
            "the password must be at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    if plain.len() > MAX_PLAIN {
        return Err(Error::Invalid("the file is too large to encrypt".into()));
    }
    let rounds = rounds_for_new();
    let salt = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let salt2 = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut salt16 = [0u8; 16];
    salt16[..12].copy_from_slice(&salt);
    salt16[12..].copy_from_slice(&salt2[..4]);
    let prefix: [u8; 7] = salt2[4..11].try_into().expect("seven bytes");
    let cipher = ChaCha20Poly1305::new(&key_for(password, &salt16, rounds));
    let mut out = Vec::with_capacity(HEADER + plain.len() + plain.len() / CHUNK * TAG + TAG);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&rounds.to_le_bytes());
    out.extend_from_slice(&salt16);
    out.extend_from_slice(&prefix);
    let chunks: Vec<&[u8]> = if plain.is_empty() {
        vec![&[][..]]
    } else {
        plain.chunks(CHUNK).collect()
    };
    let n = chunks.len();
    if n > u32::MAX as usize {
        return Err(Error::Invalid("the file is too large to encrypt".into()));
    }
    for (i, c) in chunks.into_iter().enumerate() {
        let sealed = cipher
            .encrypt(&nonce(&prefix, i as u32, i + 1 == n), c)
            .map_err(|_| Error::Invalid("could not encrypt".into()))?;
        out.extend_from_slice(&sealed);
    }
    Ok(out)
}

/// Decrypt an encrypted file's bytes. A wrong password and a damaged or
/// changed file give the same answer.
pub fn decrypt(data: &[u8], password: &Secret) -> Result<Vec<u8>, Error> {
    let wrong = || Error::Invalid("wrong password, or the file is damaged or was changed".into());
    if !is_encrypted(data) || data.len() < HEADER + TAG {
        return Err(Error::Invalid(
            "this is not an encrypted file the app can read".into(),
        ));
    }
    let version = u16::from_le_bytes([data[6], data[7]]);
    if version != VERSION {
        return Err(Error::Invalid(
            "this encrypted file was saved by a newer version of the app".into(),
        ));
    }
    let rounds = u32::from_le_bytes(data[8..12].try_into().expect("four bytes"));
    if !(1_000..=10_000_000).contains(&rounds) {
        return Err(wrong());
    }
    let salt = &data[12..28];
    let prefix = &data[28..35];
    let body = &data[HEADER..];
    if body.len() > MAX_PLAIN + MAX_PLAIN / CHUNK * TAG + TAG {
        return Err(Error::Invalid("the file is too large to open".into()));
    }
    let cipher = ChaCha20Poly1305::new(&key_for(password, salt, rounds));
    let sealed: Vec<&[u8]> = body.chunks(CHUNK + TAG).collect();
    let n = sealed.len();
    let mut out = Vec::with_capacity(body.len());
    for (i, c) in sealed.into_iter().enumerate() {
        let plain = cipher
            .decrypt(&nonce(prefix, i as u32, i + 1 == n), c)
            .map_err(|_| wrong())?;
        out.extend_from_slice(&plain);
    }
    Ok(out)
}

impl crate::Document {
    /// Save the shown rows as an encrypted CSV file (a temp file moved
    /// into place). Never over the open file.
    pub fn save_encrypted(
        &self,
        path: &std::path::Path,
        password: &Secret,
        progress: &crate::Progress,
    ) -> Result<u64, Error> {
        if self.path().is_some_and(|p| crate::save::same_path(p, path)) {
            return Err(Error::Invalid(
                "choose another name: that is the open file".into(),
            ));
        }
        self.ready_for_bulk()?;
        // Every shown row as CSV (UTF-8, CRLF).
        let cols: Vec<u32> = (0..self.col_count()).collect();
        let reader = crate::ops::Cols::new(self, &cols);
        let mut plain: Vec<u8> = Vec::new();
        let mut line = String::new();
        let mut buf = Vec::new();
        let n = self.row_count();
        progress.start(n);
        for r in 0..n {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            line.clear();
            for c in 0..cols.len() {
                if c > 0 {
                    line.push(',');
                }
                crate::scan::write_field(
                    reader.get(&buf, c),
                    crate::dialect::Dialect::CSV,
                    &mut line,
                );
            }
            line.push_str("\r\n");
            plain.extend_from_slice(line.as_bytes());
            if plain.len() > MAX_PLAIN {
                crate::privacy::wipe(&mut plain);
                return Err(Error::Invalid("the table is too large to encrypt".into()));
            }
            if r % 65_536 == 65_535 {
                progress.step(65_536)?;
            }
        }
        let sealed = encrypt(&plain, password);
        crate::privacy::wipe(&mut plain);
        let sealed = sealed?;
        let (temp, mut file) = crate::save::create_temp(path)?;
        let done = (|| -> std::io::Result<()> {
            use std::io::Write;
            file.write_all(&sealed)?;
            file.sync_all()
        })();
        drop(file);
        if let Err(e) = done.and_then(|_| crate::save::replace(&temp, path)) {
            let _ = std::fs::remove_file(&temp);
            return Err(e.into());
        }
        Ok(self.row_count())
    }

    /// Open an encrypted file: decrypted in memory only (never written
    /// out), read as CSV.
    pub fn open_encrypted(
        path: &std::path::Path,
        password: &Secret,
    ) -> Result<crate::Document, Error> {
        let size = std::fs::metadata(path)?.len();
        if size > (MAX_PLAIN + MAX_PLAIN / CHUNK * TAG + HEADER + TAG) as u64 {
            return Err(Error::Invalid("the file is too large to open".into()));
        }
        let data = std::fs::read(path)?;
        let plain = decrypt(&data, password)?;
        crate::Document::from_bytes(plain, crate::OpenOptions::default(), Some("csv"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper() {
        let pw = Secret::new("correct horse battery");
        for size in [0usize, 5, CHUNK, CHUNK + 7, 3 * CHUNK] {
            let plain: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let enc = encrypt(&plain, &pw).unwrap();
            assert!(is_encrypted(&enc));
            assert_eq!(decrypt(&enc, &pw).unwrap(), plain);
            // Wrong password.
            assert!(decrypt(&enc, &Secret::new("correct horse battery!")).is_err());
            // A flipped byte, a cut-off last chunk, a dropped chunk.
            let mut bad = enc.clone();
            let at = bad.len() - 1;
            bad[at] ^= 1;
            assert!(decrypt(&bad, &pw).is_err());
            if size > CHUNK {
                let cut = &enc[..HEADER + CHUNK + TAG];
                assert!(decrypt(cut, &pw).is_err(), "truncated at a chunk edge");
            }
        }
        // Two files of the same text differ (random salt).
        let a = encrypt(b"same", &pw).unwrap();
        let b = encrypt(b"same", &pw).unwrap();
        assert_ne!(a, b);
        assert!(encrypt(b"x", &Secret::new("short")).is_err());
    }

    #[test]
    fn save_and_open_a_table() {
        let d = crate::Document::from_bytes(
            b"name,ssn\r\nAnn,123-45-6789\r\n".to_vec(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!("nd-enc-{}.ndenc", std::process::id()));
        let pw = Secret::new("a long enough password");
        d.save_encrypted(&path, &pw, &crate::Progress::new())
            .unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(11).any(|w| w == b"123-45-6789"));
        let back = crate::Document::open_encrypted(&path, &pw).unwrap();
        assert_eq!(back.cell(1, 1).unwrap(), "123-45-6789");
        assert!(crate::Document::open_encrypted(&path, &Secret::new("not the password!")).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
