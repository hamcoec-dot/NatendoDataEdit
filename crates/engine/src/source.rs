//! The bytes of an open file.
//!
//! Large files on Windows are memory-mapped so opening a multi-gigabyte file
//! costs no copy and the OS pages it in on demand. Everything else (small
//! files, other platforms, files that must be transcoded) is read into memory.
//!
//! Apart from `ReplaceFileW` in `save.rs`, this module holds the crate's only
//! `unsafe` code: the `Mmap::map` call and two drive-type queries.

use std::fs::File;
use std::hash::Hasher;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Files at least this large are mapped instead of read (Windows only).
pub const MAP_THRESHOLD: u64 = 32 << 20;
/// Largest gzip file read.
pub const MAX_GZ_INPUT: u64 = 2 << 30;
/// Most bytes a gzip file may unpack to.
pub const MAX_GZ_OUTPUT: u64 = 8 << 30;
/// A gzip file may unpack to at most this many times its size (once past
/// 1 MB): a "zip bomb" is refused instead of filling the memory.
pub const MAX_GZ_RATIO: u64 = 250;

/// True when `data` starts like a gzip file.
pub fn is_gzip(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b])
}

/// Unpack gzip bytes (every member, as `gzip -d` does), within the caps.
pub fn gunzip(data: &[u8]) -> io::Result<Vec<u8>> {
    let cap = (data.len() as u64)
        .saturating_mul(MAX_GZ_RATIO)
        .clamp(1 << 20, MAX_GZ_OUTPUT);
    let mut out = Vec::new();
    let mut r = flate2::read::MultiGzDecoder::new(data).take(cap + 1);
    r.read_to_end(&mut out)?;
    if out.len() as u64 > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the gzip file unpacks to far more than its size (a damaged or crafted file); it was not opened",
        ));
    }
    Ok(out)
}

/// Size and modified time, used to notice changes made by other programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl FileStamp {
    pub fn of(path: &Path) -> io::Result<FileStamp> {
        let m = std::fs::metadata(path)?;
        Ok(FileStamp {
            len: m.len(),
            modified: m.modified().ok(),
        })
    }
}

enum Bytes {
    Owned(Vec<u8>),
    #[cfg(windows)]
    Mapped {
        map: memmap2::Mmap,
        // Held open with a deny-write share mode for as long as `map` lives.
        _file: File,
    },
}

/// File bytes plus where they came from.
pub struct Source {
    bytes: Bytes,
    path: Option<PathBuf>,
    stamp: Option<FileStamp>,
    /// Hash of the file's bytes as read, for sources that do not lock the
    /// file (so a change by another program can be noticed even when its
    /// size and modified time were kept).
    hash: Option<u64>,
    /// The file is gzip-compressed: `bytes` are unpacked, `hash` is of the
    /// file as stored.
    gz: bool,
}

const HASH_CHUNK: usize = 1 << 20;

/// Content hash used only to compare a file with itself within one run.
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    for c in bytes.chunks(HASH_CHUNK) {
        h.write(c);
    }
    h.finish()
}

pub(crate) fn hash_file(path: &Path) -> io::Result<u64> {
    let mut f = File::open(path)?;
    let mut h = std::hash::DefaultHasher::new();
    let mut buf = vec![0u8; HASH_CHUNK];
    loop {
        // Fill whole chunks so the hash matches `hash_bytes`.
        let mut n = 0;
        while n < buf.len() {
            match f.read(&mut buf[n..])? {
                0 => break,
                k => n += k,
            }
        }
        if n == 0 {
            break;
        }
        h.write(&buf[..n]);
        if n < buf.len() {
            break;
        }
    }
    Ok(h.finish())
}

impl Source {
    /// Bytes not backed by any file (new documents, transcoded text, tests).
    pub fn from_vec(bytes: Vec<u8>) -> Source {
        Source {
            bytes: Bytes::Owned(bytes),
            path: None,
            stamp: None,
            hash: None,
            gz: false,
        }
    }

    /// Open `path`. Large files on local fixed drives are mapped (and locked
    /// against writers); everything else is read into memory and hashed.
    pub fn open(path: &Path) -> io::Result<Source> {
        let stamp = FileStamp::of(path)?;
        // A gzip file is read whole and unpacked in memory (no unpacked
        // copy is ever written to disk).
        let mut magic = [0u8; 2];
        let got = File::open(path)?.read(&mut magic)?;
        if is_gzip(&magic[..got]) {
            return Source::open_gz(path, stamp);
        }
        #[cfg(windows)]
        if stamp.len >= MAP_THRESHOLD && on_fixed_drive(path) {
            if let Some(bytes) = map_exclusive(path)? {
                return Ok(Source {
                    bytes,
                    path: Some(path.to_owned()),
                    stamp: Some(stamp),
                    hash: None,
                    gz: false,
                });
            }
        }
        let too_big = || io::Error::new(io::ErrorKind::OutOfMemory, "file too large for memory");
        let mut file = File::open(path)?;
        let mut v = Vec::new();
        v.try_reserve_exact(usize::try_from(stamp.len).map_err(|_| too_big())?)
            .map_err(|_| too_big())?;
        file.read_to_end(&mut v)?;
        if FileStamp::of(path)? != stamp || v.len() as u64 != stamp.len {
            return Err(io::Error::other(
                "the file changed while it was being read; try again",
            ));
        }
        let hash = Some(hash_bytes(&v));
        Ok(Source {
            bytes: Bytes::Owned(v),
            path: Some(path.to_owned()),
            stamp: Some(stamp),
            hash,
            gz: false,
        })
    }

    fn open_gz(path: &Path, stamp: FileStamp) -> io::Result<Source> {
        if stamp.len > MAX_GZ_INPUT {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "the gzip file is too large to open (at most 2 GB)",
            ));
        }
        let packed = std::fs::read(path)?;
        if FileStamp::of(path)? != stamp || packed.len() as u64 != stamp.len {
            return Err(io::Error::other(
                "the file changed while it was being read; try again",
            ));
        }
        let hash = Some(hash_bytes(&packed));
        let bytes = gunzip(&packed)?;
        Ok(Source {
            bytes: Bytes::Owned(bytes),
            path: Some(path.to_owned()),
            stamp: Some(stamp),
            hash,
            gz: true,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        match &self.bytes {
            Bytes::Owned(v) => v,
            #[cfg(windows)]
            Bytes::Mapped { map, .. } => map,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// No file stands behind these bytes any more (a mapped file stays
    /// locked until the table closes).
    pub(crate) fn forget_path(&mut self) {
        self.path = None;
        self.stamp = None;
    }

    /// The file was renamed or moved (same bytes, new name).
    pub(crate) fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    pub fn stamp(&self) -> Option<FileStamp> {
        self.stamp
    }

    pub fn is_mapped(&self) -> bool {
        !matches!(self.bytes, Bytes::Owned(_))
    }

    /// Replace the bytes but keep the file identity (after transcoding).
    /// A mapped file is unlocked by this, so its hash is taken first.
    pub fn with_bytes(self, bytes: Vec<u8>) -> Source {
        let hash = self
            .hash
            .or_else(|| self.path.as_ref().map(|_| hash_bytes(self.bytes())));
        Source {
            bytes: Bytes::Owned(bytes),
            path: self.path,
            stamp: self.stamp,
            hash,
            gz: self.gz,
        }
    }

    /// Point an in-memory source at `path`, whose bytes it now matches
    /// (after saving there).
    pub(crate) fn reattach(&mut self, path: &Path, file_hash: u64) -> io::Result<()> {
        self.stamp = Some(FileStamp::of(path)?);
        self.path = Some(path.to_owned());
        self.hash = Some(file_hash);
        Ok(())
    }

    /// Hash of the file as it was read (in-memory sources only).
    pub(crate) fn file_hash(&self) -> Option<u64> {
        self.hash
    }

    /// SHA-256 of the file this source was read from, covering the bytes that were parsed:
    /// a mapped file (locked against writers while mapped) is hashed in its mapping; a file
    /// read into memory is read again and hashed only when it still matches what was read
    /// (same stamp, same content check). `(None, length)` when it does not, or has no file.
    pub fn sha256(
        &self,
        progress: &crate::document::Progress,
    ) -> Result<(Option<String>, u64), crate::Error> {
        use sha2::{Digest, Sha256};
        let Some(path) = &self.path else {
            return Ok((None, 0));
        };
        let same_stamp = |p: &Path| FileStamp::of(p).ok() == self.stamp;
        if !same_stamp(path) {
            return Ok((None, FileStamp::of(path).map_or(0, |s| s.len)));
        }
        if self.is_mapped() && !self.gz {
            let bytes = self.bytes();
            let mut h = Sha256::new();
            for piece in bytes.chunks(HASH_CHUNK) {
                progress.step(0)?;
                h.update(piece);
            }
            let ok = same_stamp(path);
            return Ok((
                ok.then(|| crate::hashing::hex(&h.finalize())),
                bytes.len() as u64,
            ));
        }
        let Some(want) = self.hash else {
            return Ok((None, self.stamp.map_or(0, |s| s.len)));
        };
        let mut f = File::open(path)?;
        let mut sha = Sha256::new();
        let mut check = std::hash::DefaultHasher::new();
        let mut buf = vec![0u8; HASH_CHUNK];
        let mut length = 0u64;
        loop {
            progress.step(0)?;
            // Whole chunks, as `hash_file` reads them, so the content check matches.
            let mut n = 0;
            while n < buf.len() {
                match f.read(&mut buf[n..])? {
                    0 => break,
                    k => n += k,
                }
            }
            if n == 0 {
                break;
            }
            sha.update(&buf[..n]);
            check.write(&buf[..n]);
            length += n as u64;
            if n < buf.len() {
                break;
            }
        }
        let ok =
            check.finish() == want && same_stamp(path) && Some(length) == self.stamp.map(|s| s.len);
        Ok((ok.then(|| crate::hashing::hex(&sha.finalize())), length))
    }

    /// The file was gzip-compressed (its bytes here are unpacked).
    pub fn is_gzipped(&self) -> bool {
        self.gz
    }

    /// True when the file on disk no longer matches what was opened.
    pub fn changed_on_disk(&self) -> io::Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        let now = match FileStamp::of(path) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(e) => return Err(e),
        };
        if Some(now) != self.stamp {
            return Ok(true);
        }
        // A locked (mapped) file cannot have changed; others are compared
        // by content, since size and time can be preserved by a writer.
        match self.hash {
            Some(h) if !self.is_mapped() => Ok(hash_file(path)? != h),
            _ => Ok(false),
        }
    }
}

/// True for files on a local fixed disk. Network shares and removable
/// drives can vanish while mapped, which would crash the process, so files
/// there are read into memory instead.
#[cfg(windows)]
fn on_fixed_drive(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
    const DRIVE_FIXED: u32 = 3;

    let Ok(full) = std::fs::canonicalize(path) else {
        return false;
    };
    let s = full.to_string_lossy();
    // `\\?\C:\...` → `C:\...`; `\\?\UNC\...` is a network path.
    let plain = match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.starts_with("UNC\\") => return false,
        Some(rest) => rest.to_string(),
        None => s.into_owned(),
    };
    if plain.starts_with(r"\\") {
        return false;
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(&plain)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut root = [0u16; 1024];
    // SAFETY: `wide` is a NUL-terminated UTF-16 string and `root` is a
    // writable buffer whose length is passed; both outlive the call.
    #[allow(unsafe_code)]
    let ok = unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) };
    if ok == 0 {
        return false;
    }
    // SAFETY: `root` was NUL-terminated by the successful call above.
    #[allow(unsafe_code)]
    let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
    kind == DRIVE_FIXED
}

/// Map `path` while denying other writers. `Ok(None)` when another program
/// already has the file open for writing, so it cannot be locked; the caller
/// then reads it into memory instead.
#[cfg(windows)]
fn map_exclusive(path: &Path) -> io::Result<Option<Bytes>> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

    const ERROR_SHARING_VIOLATION: i32 = 32;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => return Ok(None),
        Err(e) => return Err(e),
    };
    // SAFETY: `Mmap::map` is unsafe because the mapped bytes must not change
    // while the map is alive. `file` was opened with share mode READ only, so
    // no other process can open it for writing, truncate it or delete it until
    // `file` is closed, and `file` lives in the same `Bytes::Mapped` value as
    // the map, so it is closed only after the map is dropped. This crate never
    // writes to a file it has mapped: saving drops the `Source` first. Only
    // files on local fixed drives are mapped (see `on_fixed_drive`).
    #[allow(unsafe_code)]
    let map = unsafe { memmap2::Mmap::map(&file)? };
    Ok(Some(Bytes::Mapped { map, _file: file }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_small_file_and_stamps_it() {
        let dir = std::env::temp_dir().join(format!("nde-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.csv");
        std::fs::write(&p, b"a,b\n").unwrap();
        let s = Source::open(&p).unwrap();
        assert_eq!(s.bytes(), b"a,b\n");
        assert!(!s.is_mapped());
        assert_eq!(s.stamp().unwrap().len, 4);
        drop(s);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn notices_same_size_change_with_time_restored() {
        let dir = std::env::temp_dir().join(format!("nde-chg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.csv");
        std::fs::write(&p, b"a,b\n1,2\n").unwrap();
        let s = Source::open(&p).unwrap();
        assert!(!s.changed_on_disk().unwrap());
        let when = std::fs::metadata(&p).unwrap().modified().unwrap();
        std::fs::write(&p, b"a,b\n9,9\n").unwrap();
        File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(when)
            .unwrap();
        assert_eq!(FileStamp::of(&p).unwrap(), s.stamp().unwrap());
        assert!(s.changed_on_disk().unwrap());
        drop(s);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn temp_dir_is_a_fixed_drive() {
        assert!(on_fixed_drive(&std::env::temp_dir()));
        assert!(!on_fixed_drive(Path::new(r"\\server\share\x.csv")));
    }

    #[cfg(windows)]
    #[test]
    fn mapped_file_denies_writers() {
        let dir = std::env::temp_dir().join(format!("nde-map-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("big.csv");
        std::fs::write(&p, vec![b'x'; 4096]).unwrap();
        let Some(Bytes::Mapped { map, _file }) = map_exclusive(&p).unwrap() else {
            panic!("expected a map");
        };
        assert_eq!(map.len(), 4096);
        assert!(std::fs::OpenOptions::new().write(true).open(&p).is_err());
        drop(map);
        drop(_file);
        assert!(std::fs::OpenOptions::new().write(true).open(&p).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
