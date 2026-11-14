//! Autosave and recovery, and the list of earlier saves.
//!
//! **Recovery copies.** While a document has unsaved changes, the UI asks
//! every few minutes for a recovery copy. The copy is the document as it
//! would be saved now, written to the app's private per-user folder
//! (`%LOCALAPPDATA%\<app>\recovery`), never next to the user's file and
//! never over it. Each copy is written to a new temporary file, flushed to
//! disk and only then renamed into place, and its small description file is
//! written the same way afterwards, so a crash mid-write leaves the last
//! good copy. Copies are bounded: [`MAX_COPY_BYTES`] each and
//! [`MAX_TOTAL_BYTES`] for the folder (bigger documents get no copy and the
//! UI says so), at most [`MAX_ENTRIES`] are kept, and copies older than
//! [`MAX_AGE_DAYS`] days are removed at start. Writing holds only the
//! document's read lock, and any change to the document cancels a copy in
//! progress, so autosave never makes the user wait. A copy is removed when
//! its document is saved or closed, and every copy of the session is
//! removed when the app is closed normally; what is left after a crash is
//! offered back at the next start, opened as a new unsaved tab (the user
//! decides where to save it).
//!
//! **Earlier saves.** When the Options switch is on, before a save replaces
//! a file of at most [`MAX_VERSION_BYTES`], the file as it was is copied
//! into `...\versions\<key>\`, so earlier saves can be opened again (as
//! new tabs). At most [`MAX_VERSIONS`] per file and [`MAX_VERSIONS_TOTAL`]
//! bytes in all are kept, oldest first out, and copies older than
//! [`MAX_AGE_DAYS`] days are removed at start. Options can delete them all,
//! and uninstalling removes both folders.
//!
//! The webview names entries by their place in the last listing, never by
//! path.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use natendo_engine::Document;
use serde::{Deserialize, Serialize};

use crate::docs::{CmdError, CmdResult, Layout};

/// Largest recovery copy.
pub const MAX_COPY_BYTES: u64 = 200 << 20;
/// All recovery copies together.
pub const MAX_TOTAL_BYTES: u64 = 1 << 30;
/// Recovery copies kept (and listed).
pub const MAX_ENTRIES: usize = 64;
/// Copies older than this are removed at start.
pub const MAX_AGE_DAYS: u64 = 30;
/// Largest file kept as an earlier save.
pub const MAX_VERSION_BYTES: u64 = 64 << 20;
/// Earlier saves kept per file.
pub const MAX_VERSIONS: usize = 20;
/// Earlier saves of all files together.
pub const MAX_VERSIONS_TOTAL: u64 = 512 << 20;
/// Largest description file read back.
const MAX_META_BYTES: u64 = 64 << 10;

const DATA_EXT: &str = "ndrec";
const META_EXT: &str = "json";
const VERSION_EXT: &str = "ndver";

struct Store {
    recovery: PathBuf,
    versions: PathBuf,
    session: String,
    /// The last listing of recovery copies (data file, description file).
    listed: Mutex<Vec<(PathBuf, PathBuf)>>,
    /// The last listing of earlier saves: (document, files).
    versions_listed: Mutex<(u32, Vec<PathBuf>)>,
}

static STORE: OnceLock<Store> = OnceLock::new();

/// Whether earlier saves are kept (the Options switch; the UI sets it at
/// start). Off until told, so nothing is kept against the user's choice.
static KEEP_VERSIONS: AtomicBool = AtomicBool::new(false);

/// Turn keeping earlier saves on or off.
pub fn set_keep_versions(on: bool) {
    KEEP_VERSIONS.store(on, Ordering::SeqCst);
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Start: folders under `base` (the app's per-user local data folder).
/// Old copies are removed. Without a folder, autosave is off.
pub fn init(base: PathBuf) {
    let recovery = base.join("recovery");
    let versions = base.join("versions");
    if fs::create_dir_all(&recovery).is_err() || fs::create_dir_all(&versions).is_err() {
        return;
    }
    let session = format!("{:x}-{:x}", std::process::id(), now_millis());
    prune_old(&recovery);
    prune_old_versions(&versions);
    let _ = STORE.set(Store {
        recovery,
        versions,
        session,
        listed: Mutex::new(Vec::new()),
        versions_listed: Mutex::new((0, Vec::new())),
    });
}

#[cfg(test)]
fn init_at(base: &Path) -> &'static Store {
    init(base.to_path_buf());
    STORE.get().expect("test store")
}

fn store() -> CmdResult<&'static Store> {
    STORE
        .get()
        .ok_or_else(|| CmdError::new("no_recovery", "there is no folder for recovery copies"))
}

/// What is kept about a recovery copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    version: u32,
    session: String,
    doc: u32,
    title: String,
    path: Option<String>,
    ext: Option<String>,
    layout: Layout,
    saved: u64,
    rows: u64,
    bytes: u64,
}

/// A recovery copy, for the UI.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecoveryItem {
    pub index: u32,
    pub title: String,
    /// The file it came from (shown only), if it had one.
    pub path: Option<String>,
    /// When it was written (seconds since 1970).
    pub saved: u64,
    pub rows: u64,
    pub bytes: u64,
}

/// What an autosave did.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Autosaved {
    Written {
        at: u64,
        bytes: u64,
    },
    /// Nothing changed since the last copy (or nothing is unsaved).
    Unchanged,
    /// Too big for a recovery copy.
    TooLarge,
    /// The recovery folder is full of copies left by earlier runs: the
    /// user opens or discards them (File > Recover unsaved work).
    Full,
    /// Busy (being changed or saved); asked again later.
    Skipped,
    /// No folder for copies.
    Off,
}

fn names(s: &Store, session: &str, doc: u32) -> (PathBuf, PathBuf) {
    let stem = format!("{session}-{doc}");
    (
        s.recovery.join(format!("{stem}.{DATA_EXT}")),
        s.recovery.join(format!("{stem}.{META_EXT}")),
    )
}

/// A new temporary file in `dir`, never reusing a name.
fn temp_in(dir: &Path) -> io::Result<(PathBuf, fs::File)> {
    for n in 0..100u32 {
        let p = dir.join(format!(
            ".{:x}-{:x}-{n}.tmp",
            std::process::id(),
            now_millis()
        ));
        match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(f) => return Ok((p, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("could not create a temporary file"))
}

/// Write `bytes` to `target` atomically (temporary file, flushed, renamed).
fn write_small(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    let (tmp, mut f) = temp_in(dir)?;
    let r = f.write_all(bytes).and_then(|_| f.sync_all());
    drop(f);
    match r.and_then(|_| fs::rename(&tmp, target)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// A writer that stops at a size cap or when cancelled.
struct Capped<'a, W: Write> {
    inner: W,
    left: u64,
    cancel: &'a dyn Fn() -> bool,
    too_large: bool,
}

impl<W: Write> Write for Capped<'_, W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if (self.cancel)() {
            // Not `Interrupted`: `write_all` would retry that forever.
            return Err(io::Error::other("cancelled"));
        }
        if b.len() as u64 > self.left {
            self.too_large = true;
            return Err(io::Error::other("too large"));
        }
        let n = self.inner.write(b)?;
        self.left -= n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Bytes held by recovery copies other than `except`.
fn used_bytes(s: &Store, except: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(&s.recovery) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| e.path() != except)
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// Write a recovery copy of `d` (open as document `doc` and called
/// `title`); stops as soon as `cancel` returns true.
pub fn write_copy(
    d: &Document,
    doc: u32,
    title: &str,
    cancel: &dyn Fn() -> bool,
) -> CmdResult<Autosaved> {
    let Ok(s) = store() else {
        return Ok(Autosaved::Off);
    };
    // A file far past the cap would only be read to be thrown away.
    if d.source_bytes() > MAX_COPY_BYTES + MAX_COPY_BYTES / 4 {
        return Ok(Autosaved::TooLarge);
    }
    let (data, meta) = names(s, &s.session, doc);
    let room = MAX_TOTAL_BYTES.saturating_sub(used_bytes(s, &data));
    let budget = MAX_COPY_BYTES.min(room);
    if (count_entries(s) >= MAX_ENTRIES && !data.exists()) || room == 0 {
        return Ok(Autosaved::Full);
    }
    let (tmp, file) = temp_in(&s.recovery).map_err(io_err)?;
    let mut w = Capped {
        inner: io::BufWriter::with_capacity(1 << 20, &file),
        left: budget,
        cancel,
        too_large: false,
    };
    let written = d.write_copy(&mut w).and_then(|n| {
        w.flush()?;
        Ok(n)
    });
    let too_large = w.too_large;
    drop(w);
    let result = written.map_err(CmdError::from).and_then(|n| {
        file.sync_all().map_err(io_err)?;
        Ok(n)
    });
    drop(file);
    let bytes = match result {
        Ok(n) => n,
        Err(_) => {
            let _ = fs::remove_file(&tmp);
            return Ok(if too_large && budget < MAX_COPY_BYTES {
                // It would have fitted the per-copy limit: the folder is
                // what is full.
                Autosaved::Full
            } else if too_large {
                Autosaved::TooLarge
            } else {
                Autosaved::Skipped
            });
        }
    };
    if let Err(e) = fs::rename(&tmp, &data) {
        let _ = fs::remove_file(&tmp);
        return Err(io_err(e));
    }
    let at = now_secs();
    let m = Meta {
        version: 1,
        session: s.session.clone(),
        doc,
        title: title.chars().take(260).collect(),
        path: d.path().map(|p| p.display().to_string()),
        ext: d
            .path()
            .and_then(|p| p.extension())
            .map(|e| e.to_string_lossy().chars().take(16).collect()),
        layout: Layout::of(&d.format().dialect, d.preamble_lines()),
        saved: at,
        rows: d.total_rows(),
        bytes,
    };
    let json = serde_json::to_vec(&m).map_err(|e| CmdError::new("error", e.to_string()))?;
    write_small(&meta, &json).map_err(io_err)?;
    Ok(Autosaved::Written { at, bytes })
}

fn io_err(e: io::Error) -> CmdError {
    CmdError::new("error", e.to_string())
}

fn count_entries(s: &Store) -> usize {
    fs::read_dir(&s.recovery)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == DATA_EXT))
                .count()
        })
        .unwrap_or(0)
}

/// Remove this session's copy of document `doc` (saved or closed).
pub fn forget(doc: u32) {
    if let Ok(s) = store() {
        let (data, meta) = names(s, &s.session, doc);
        let _ = fs::remove_file(meta);
        let _ = fs::remove_file(data);
    }
}

/// Whether `p` is `target` or inside it (both compared as full paths, in
/// the same form `path_key` uses: links, junctions and mapped drives are
/// resolved where the path still exists). A drive root holds everything
/// on the drive.
fn under(p: &Path, target: &Path) -> bool {
    let (p, t) = (full_text(p), full_text(target));
    if p == t {
        return true;
    }
    match p.strip_prefix(&t) {
        Some(rest) => t.ends_with(['\\', '/']) || rest.starts_with(['\\', '/']),
        None => false,
    }
}

/// A path as comparable text: resolved where it (or its folder) still
/// exists, without the `\\?\` long-path prefix, a network path as
/// `\\server\share`, small letters on Windows.
fn full_text(p: &Path) -> String {
    let full = std::fs::canonicalize(p)
        .or_else(|e| match (p.parent(), p.file_name()) {
            (Some(dir), Some(name)) => std::fs::canonicalize(dir).map(|d| d.join(name)),
            _ => Err(e),
        })
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf());
    let mut text = full.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        text = format!(r"\\{rest}");
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        text = rest.to_string();
    }
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    text
}

/// Earlier saves and recovery copies kept for files under `target`:
/// (folders of earlier saves, files in them, recovery copies), each with
/// what deleting them would remove.
struct Leftovers {
    versions: Vec<(PathBuf, u64)>,
    recovery: Vec<PathBuf>,
}

fn leftovers(target: &Path) -> Leftovers {
    let mut out = Leftovers {
        versions: Vec::new(),
        recovery: Vec::new(),
    };
    let Ok(s) = store() else { return out };
    if let Ok(rd) = fs::read_dir(&s.versions) {
        for e in rd.flatten() {
            let info = e.path().join("info.json");
            let Ok(f) = fs::File::open(&info) else {
                continue;
            };
            let mut b = Vec::new();
            if f.take(MAX_META_BYTES).read_to_end(&mut b).is_err() {
                continue;
            }
            if let Ok(v) = serde_json::from_slice::<VersionInfo>(&b) {
                if under(Path::new(&v.path), target) {
                    let n = fs::read_dir(e.path()).map_or(0, |rd| {
                        rd.flatten()
                            .filter(|f| f.path().extension().is_some_and(|x| x == VERSION_EXT))
                            .count() as u64
                    });
                    out.versions.push((e.path(), n));
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir(&s.recovery) {
        for e in rd.flatten() {
            let meta = e.path();
            if meta.extension().is_none_or(|x| x != META_EXT) {
                continue;
            }
            let Some(m) = read_meta(&meta) else { continue };
            if m.path
                .as_deref()
                .is_some_and(|p| under(Path::new(p), target))
            {
                out.recovery.push(meta);
            }
        }
    }
    out
}

/// How many earlier saves and recovery copies are kept for files under
/// `target` (asked before [`forget_path`] deletes them).
pub fn count_path(target: &Path) -> (u64, u64) {
    let l = leftovers(target);
    (
        l.versions.iter().map(|(_, n)| n).sum(),
        l.recovery.len() as u64,
    )
}

/// Delete every earlier save and recovery copy (from any run) of files
/// under `target`; only after the user said yes. Returns how many went.
pub fn forget_path(target: &Path) -> u64 {
    let l = leftovers(target);
    let mut n = 0;
    for (dir, files) in l.versions {
        if fs::remove_dir_all(dir).is_ok() {
            n += files;
        }
    }
    for meta in l.recovery {
        let _ = fs::remove_file(meta.with_extension(DATA_EXT));
        if fs::remove_file(&meta).is_ok() {
            n += 1;
        }
    }
    n
}

/// Remove every copy of this session (the app closed normally).
pub fn forget_session() {
    let Ok(s) = store() else { return };
    let Ok(rd) = fs::read_dir(&s.recovery) else {
        return;
    };
    let prefix = format!("{}-", s.session);
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Remove copies older than [`MAX_AGE_DAYS`], left-over temporary files,
/// description files without data and the other way round, and the oldest
/// copies past [`MAX_ENTRIES`].
fn prune_old(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let cutoff = SystemTime::now() - std::time::Duration::from_secs(MAX_AGE_DAYS * 86_400);
    let mut datas: Vec<(SystemTime, PathBuf)> = Vec::new();
    for e in rd.flatten().take(10_000) {
        let p = e.path();
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let modified = m.modified().unwrap_or(UNIX_EPOCH);
        let ext = p.extension().map(|x| x.to_string_lossy().into_owned());
        let old = modified < cutoff;
        match ext.as_deref() {
            Some(DATA_EXT) if !old && p.with_extension(META_EXT).exists() => {
                datas.push((modified, p))
            }
            Some(META_EXT) if !old && p.with_extension(DATA_EXT).exists() => {}
            // Temporary files from a crash mid-write, orphans and old copies.
            _ => {
                let _ = fs::remove_file(&p);
            }
        }
    }
    if datas.len() > MAX_ENTRIES {
        datas.sort();
        for (_, p) in &datas[..datas.len() - MAX_ENTRIES] {
            let _ = fs::remove_file(p.with_extension(META_EXT));
            let _ = fs::remove_file(p);
        }
    }
}

fn read_meta(p: &Path) -> Option<Meta> {
    let f = fs::File::open(p).ok()?;
    let mut buf = Vec::new();
    f.take(MAX_META_BYTES).read_to_end(&mut buf).ok()?;
    let m: Meta = serde_json::from_slice(&buf).ok()?;
    (m.version == 1).then_some(m)
}

/// Copies left by earlier runs (newest first), remembered for open/discard.
pub fn list() -> CmdResult<Vec<RecoveryItem>> {
    let s = store()?;
    let mut found: Vec<(Meta, PathBuf, PathBuf)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&s.recovery) {
        for e in rd.flatten().take(10_000) {
            let p = e.path();
            if p.extension().is_none_or(|x| x != META_EXT) {
                continue;
            }
            let data = p.with_extension(DATA_EXT);
            let Some(m) = read_meta(&p) else { continue };
            if m.session == s.session || !data.is_file() {
                continue;
            }
            found.push((m, data, p));
        }
    }
    found.sort_by_key(|x| std::cmp::Reverse(x.0.saved));
    found.truncate(MAX_ENTRIES);
    let items = found
        .iter()
        .enumerate()
        .map(|(i, (m, _, _))| RecoveryItem {
            index: i as u32,
            title: m.title.clone(),
            path: m.path.clone(),
            saved: m.saved,
            rows: m.rows,
            bytes: m.bytes,
        })
        .collect();
    *s.listed.lock().unwrap_or_else(|p| p.into_inner()) =
        found.into_iter().map(|(_, d, m)| (d, m)).collect();
    Ok(items)
}

fn listed(index: u32) -> CmdResult<(PathBuf, PathBuf)> {
    store()?
        .listed
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(index as usize)
        .cloned()
        .ok_or_else(|| CmdError::new("bad_request", "list the recovery copies again"))
}

/// Read a listed copy: the document (unsaved, no file) and its title.
pub fn open(index: u32) -> CmdResult<(Document, String)> {
    let (data, meta) = listed(index)?;
    let m = read_meta(&meta)
        .ok_or_else(|| CmdError::new("error", "the recovery copy cannot be read"))?;
    let bytes = read_capped(&data, MAX_COPY_BYTES)?;
    let doc = document_from(bytes, &m.layout, m.ext.as_deref())?;
    Ok((doc, format!("{} (recovered)", m.title)))
}

/// After a recovered copy was opened as document `doc`: the copy becomes
/// that document's own recovery copy (so it is still there if the app
/// stops again before the next autosave).
pub fn adopt(index: u32, doc: u32) {
    let (Ok(s), Ok((data, meta))) = (store(), listed(index)) else {
        return;
    };
    let (to_data, to_meta) = names(s, &s.session, doc);
    if fs::rename(&data, &to_data).is_ok() {
        if let Some(mut m) = read_meta(&meta) {
            m.session = s.session.clone();
            m.doc = doc;
            if let Ok(json) = serde_json::to_vec(&m) {
                let _ = write_small(&to_meta, &json);
            }
        }
    }
    let _ = fs::remove_file(meta);
    let _ = fs::remove_file(data);
}

/// Throw a listed copy away.
pub fn discard(index: u32) -> CmdResult<()> {
    let (data, meta) = listed(index)?;
    let _ = fs::remove_file(meta);
    let _ = fs::remove_file(data);
    Ok(())
}

fn read_capped(p: &Path, cap: u64) -> CmdResult<Vec<u8>> {
    let f = fs::File::open(p).map_err(io_err)?;
    let mut buf = Vec::new();
    f.take(cap + 1).read_to_end(&mut buf).map_err(io_err)?;
    if buf.len() as u64 > cap {
        return Err(CmdError::new("too_large", "the copy is too large to open"));
    }
    Ok(buf)
}

fn document_from(bytes: Vec<u8>, layout: &Layout, ext: Option<&str>) -> CmdResult<Document> {
    let opts = natendo_engine::OpenOptions {
        dialect: Some(layout.dialect()?),
        skip_lines: Some(layout.skip_lines),
        ..Default::default()
    };
    let mut d = Document::from_bytes(bytes, opts, ext)?;
    while !d.index_step(64 << 20)? {}
    Ok(d)
}

// ---- earlier saves ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionInfo {
    path: String,
}

/// An earlier save, for the UI.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VersionItem {
    pub index: u32,
    /// When that version was saved (the file's time then; seconds since 1970).
    pub saved: u64,
    pub bytes: u64,
}

fn path_key(p: &Path) -> (String, String) {
    let full = std::fs::canonicalize(p)
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf());
    let mut text = full.display().to_string();
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    // FNV-1a: the same key in every version of the app (the standard
    // hasher may change between Rust releases). The full path is stored
    // and compared too, so a clash only means no earlier saves are shown.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (format!("{h:016x}"), text)
}

/// The folder of `path`'s earlier saves, if it is really that file's.
fn version_dir(s: &Store, path: &Path, create: bool) -> Option<PathBuf> {
    let (key, text) = path_key(path);
    let dir = s.versions.join(key);
    let info = dir.join("info.json");
    let read = fs::File::open(&info).ok().and_then(|f| {
        let mut b = Vec::new();
        f.take(MAX_META_BYTES).read_to_end(&mut b).ok()?;
        Some(b)
    });
    match read.and_then(|b| serde_json::from_slice::<VersionInfo>(&b).ok()) {
        Some(v) if v.path == text => Some(dir),
        Some(_) => None,
        None if create => {
            fs::create_dir_all(&dir).ok()?;
            let json = serde_json::to_vec(&VersionInfo { path: text }).ok()?;
            write_small(&info, &json).ok()?;
            Some(dir)
        }
        None => None,
    }
}

/// Keep `path` as it is now (before a save replaces it) as an earlier save.
/// Never fails the save: problems only mean no copy.
pub fn keep_version(path: &Path) {
    if !KEEP_VERSIONS.load(Ordering::SeqCst) {
        return;
    }
    let Ok(s) = store() else { return };
    let Ok(m) = fs::metadata(path) else { return };
    if !m.is_file() || m.len() > MAX_VERSION_BYTES || m.len() == 0 {
        return;
    }
    let Some(dir) = version_dir(s, path, true) else {
        return;
    };
    let when = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    let target = dir.join(format!("{when:013}.{VERSION_EXT}"));
    if target.exists() {
        return;
    }
    let Ok((tmp, mut f)) = temp_in(&dir) else {
        return;
    };
    let ok = fs::File::open(path)
        .and_then(|mut src| io::copy(&mut (&mut src).take(MAX_VERSION_BYTES), &mut f))
        .and_then(|_| f.sync_all())
        .is_ok();
    drop(f);
    if !ok || fs::rename(&tmp, &target).is_err() {
        let _ = fs::remove_file(&tmp);
        return;
    }
    prune_versions(s, &dir);
}

fn version_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut out: Vec<(PathBuf, u64)> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .take(1_000)
                .filter(|e| e.path().extension().is_some_and(|x| x == VERSION_EXT))
                .filter_map(|e| Some((e.path(), e.metadata().ok()?.len())))
                .collect()
        })
        .unwrap_or_default();
    // Names are zero-padded times: newest first.
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

fn prune_versions(s: &Store, dir: &Path) {
    for (p, _) in version_files(dir).iter().skip(MAX_VERSIONS) {
        let _ = fs::remove_file(p);
    }
    // All files together: drop the oldest anywhere until under the cap.
    let mut all: Vec<(PathBuf, u64)> = fs::read_dir(&s.versions)
        .map(|rd| {
            rd.flatten()
                .take(10_000)
                .filter(|e| e.path().is_dir())
                .flat_map(|e| version_files(&e.path()))
                .collect()
        })
        .unwrap_or_default();
    let mut total: u64 = all.iter().map(|x| x.1).sum();
    all.sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));
    for (p, n) in all {
        if total <= MAX_VERSIONS_TOTAL {
            break;
        }
        if fs::remove_file(&p).is_ok() {
            total -= n;
        }
    }
}

/// Remove earlier saves kept more than [`MAX_AGE_DAYS`] days ago, and
/// folders left with none.
fn prune_old_versions(root: &Path) {
    let Ok(rd) = fs::read_dir(root) else { return };
    let cutoff = SystemTime::now() - std::time::Duration::from_secs(MAX_AGE_DAYS * 86_400);
    for e in rd.flatten().take(10_000) {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let mut left = 0usize;
        for (p, _) in version_files(&dir) {
            let old = fs::metadata(&p)
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < cutoff);
            if !(old && fs::remove_file(&p).is_ok()) {
                left += 1;
            }
        }
        if left == 0 {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

/// Delete every earlier save of every file. Returns how many were removed.
pub fn clear_versions() -> CmdResult<u64> {
    let s = store()?;
    let mut n = 0u64;
    if let Ok(rd) = fs::read_dir(&s.versions) {
        for e in rd.flatten().take(10_000) {
            let dir = e.path();
            if dir.is_dir() {
                n += version_files(&dir).len() as u64;
                fs::remove_dir_all(&dir).map_err(io_err)?;
            }
        }
    }
    s.versions_listed
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .1
        .clear();
    Ok(n)
}

/// Earlier saves of `path` (newest first), remembered for opening.
pub fn versions(doc: u32, path: &Path) -> CmdResult<Vec<VersionItem>> {
    let s = store()?;
    let files = version_dir(s, path, false)
        .map(|d| version_files(&d))
        .unwrap_or_default();
    let items = files
        .iter()
        .enumerate()
        .map(|(i, (p, n))| VersionItem {
            index: i as u32,
            saved: p
                .file_stem()
                .and_then(|x| x.to_str())
                .and_then(|x| x.parse::<u64>().ok())
                .map_or(0, |ms| ms / 1000),
            bytes: *n,
        })
        .collect();
    *s.versions_listed.lock().unwrap_or_else(|p| p.into_inner()) =
        (doc, files.into_iter().map(|x| x.0).collect());
    Ok(items)
}

/// Read a listed earlier save of document `doc`'s file with `layout`.
pub fn open_version(
    doc: u32,
    index: u32,
    layout: &Layout,
    ext: Option<&str>,
) -> CmdResult<Document> {
    let s = store()?;
    let path = {
        let l = s.versions_listed.lock().unwrap_or_else(|p| p.into_inner());
        if l.0 != doc {
            return Err(CmdError::new("bad_request", "list the earlier saves again"));
        }
        l.1.get(index as usize)
            .cloned()
            .ok_or_else(|| CmdError::new("bad_request", "list the earlier saves again"))?
    };
    let bytes = read_capped(&path, MAX_VERSION_BYTES)?;
    document_from(bytes, layout, ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use natendo_engine::OpenOptions;

    fn doc(text: &str) -> Document {
        let mut d = Document::from_bytes(
            text.as_bytes().to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        while !d.index_step(1 << 20).unwrap() {}
        d
    }

    // One test: the store is set once per process.
    #[test]
    fn copies_versions_and_cleanup() {
        let base = std::env::temp_dir().join(format!("nde-recovery-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("recovery")).unwrap();
        // Left-overs from an earlier run: a temporary file and an orphan.
        fs::write(base.join("recovery").join(".x.tmp"), b"junk").unwrap();
        fs::write(base.join("recovery").join("orphan.json"), b"{}").unwrap();
        let s = init_at(&base);
        assert!(!base.join("recovery").join(".x.tmp").exists());
        assert!(!base.join("recovery").join("orphan.json").exists());

        // A copy holds the unsaved edit, in the document's format.
        let mut d = doc("a,b\n1,2\n");
        d.set_cell(1, 1, "two".into()).unwrap();
        let cancel = AtomicBool::new(false);
        let stop = || cancel.load(Ordering::Relaxed);
        let r = write_copy(&d, 7, "t.csv", &stop).unwrap();
        assert!(matches!(r, Autosaved::Written { bytes: 10, .. }), "{r:?}");
        let (data, meta) = names(s, &s.session, 7);
        assert_eq!(fs::read_to_string(&data).unwrap(), "a,b\n1,two\n");
        assert!(meta.exists());
        // Cancelled: nothing changes and no temporary file stays.
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            write_copy(&d, 7, "t.csv", &stop).unwrap(),
            Autosaved::Skipped
        );
        assert_eq!(fs::read_to_string(&data).unwrap(), "a,b\n1,two\n");
        assert!(fs::read_dir(&s.recovery)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
        // This session's copies are not offered back...
        assert!(list().unwrap().is_empty());
        // ...a copy from an earlier session is, and opens as unsaved work.
        let (od, om) = names(s, "dead-1", 3);
        fs::copy(&data, &od).unwrap();
        let mut m = read_meta(&meta).unwrap();
        m.session = "dead-1".into();
        m.doc = 3;
        fs::write(&om, serde_json::to_vec(&m).unwrap()).unwrap();
        let items = list().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "t.csv");
        let (rd, title) = open(0).unwrap();
        assert_eq!(title, "t.csv (recovered)");
        assert_eq!(rd.cell(1, 1).as_deref(), Some("two"));
        assert!(rd.path().is_none());
        adopt(0, 9);
        assert!(!od.exists() && !om.exists());
        assert!(names(s, &s.session, 9).0.exists());
        // Closing and quitting remove the session's copies.
        forget(7);
        assert!(!data.exists() && !meta.exists());
        forget_session();
        assert!(!names(s, &s.session, 9).0.exists());

        // Earlier saves: none while the switch is off...
        let file = base.join("data.csv");
        fs::write(&file, "x\n1\n").unwrap();
        keep_version(&file);
        assert!(versions(1, &file).unwrap().is_empty());
        // ...and the file as it was before each save once it is on.
        set_keep_versions(true);
        keep_version(&file);
        let v = versions(1, &file).unwrap();
        assert_eq!(v.len(), 1);
        let layout = Layout::of(&natendo_engine::Dialect::CSV, 0);
        let vd = open_version(1, 0, &layout, Some("csv")).unwrap();
        assert_eq!(vd.cell(1, 0).as_deref(), Some("1"));
        assert!(open_version(2, 0, &layout, Some("csv")).is_err());
        // Another file never sees them.
        assert!(versions(1, &base.join("other.csv")).unwrap().is_empty());
        // Deleting them all empties the folder.
        assert_eq!(clear_versions().unwrap(), 1);
        assert!(versions(1, &file).unwrap().is_empty());
        assert_eq!(fs::read_dir(&s.versions).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn capped_writer_stops() {
        let never = || false;
        let mut w = Capped {
            inner: Vec::new(),
            left: 4,
            cancel: &never,
            too_large: false,
        };
        assert!(w.write_all(b"abc").is_ok());
        assert!(w.write_all(b"de").is_err());
        assert!(w.too_large);
    }
}

#[cfg(test)]
mod under_tests {
    use super::under;
    use std::path::Path;

    #[test]
    fn drive_roots_and_separators() {
        let dir = std::env::temp_dir().join("natendo-under");
        let root: std::path::PathBuf = dir.ancestors().last().unwrap().to_path_buf();
        assert!(under(&dir.join("a.csv"), &root), "{root:?}");
        assert!(under(&dir.join("a.csv"), &dir));
        assert!(!under(Path::new(&format!("{}x", dir.display())), &dir));
    }
}
