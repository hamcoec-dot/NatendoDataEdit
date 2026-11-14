//! The app's own log files: one line per event (time, level, short text),
//! in the app's log folder. `natendo.log` turns over at 1 MB and five files
//! are kept. Lines name files, never their contents; line breaks are
//! escaped and lines are cut to 2,000 characters. Logging never fails the
//! caller: a log that cannot be written is skipped.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Size at which the log turns over.
const MAX_BYTES: u64 = 1 << 20;
/// Files kept (the current one and four older).
const KEEP: u32 = 5;
/// Longest line, in characters.
pub const MAX_LINE: usize = 2_000;
const NAME: &str = "natendo";

static LOG: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<PathBuf>> {
    LOG.get_or_init(|| Mutex::new(None))
}

/// Start logging into `dir` (created when missing).
pub fn init(dir: PathBuf) {
    if fs::create_dir_all(&dir).is_ok() {
        *slot().lock().unwrap_or_else(|p| p.into_inner()) = Some(dir);
    }
}

/// The log folder, once logging started.
pub fn folder() -> Option<PathBuf> {
    slot().lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn stamp() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = natendo_engine::fill::civil_from_days((t / 86_400) as i64);
    let s = t % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// One line of text, safe to log.
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_LINE));
    for c in text.chars().take(MAX_LINE) {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn file(dir: &std::path::Path, n: u32) -> PathBuf {
    if n == 0 {
        dir.join(format!("{NAME}.log"))
    } else {
        dir.join(format!("{NAME}.{n}.log"))
    }
}

/// Write a line at `level` (info, warn, error).
pub fn write(level: &str, text: &str) {
    let guard = slot().lock().unwrap_or_else(|p| p.into_inner());
    let Some(dir) = guard.as_ref() else { return };
    let current = file(dir, 0);
    if fs::metadata(&current).is_ok_and(|m| m.len() >= MAX_BYTES) {
        let _ = fs::remove_file(file(dir, KEEP - 1));
        for n in (0..KEEP - 1).rev() {
            let _ = fs::rename(file(dir, n), file(dir, n + 1));
        }
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&current) {
        let _ = writeln!(f, "{} {:<5} {}", stamp(), level, clean(text));
    }
}

pub fn info(text: &str) {
    write("info", text);
}

pub fn warn(text: &str) {
    write("warn", text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_cleaned_and_files_turn_over() {
        assert_eq!(clean("a\nb\r\u{7}c"), "a\\nb\\r c");
        assert_eq!(clean(&"x".repeat(5_000)).len(), MAX_LINE);
        let dir = std::env::temp_dir().join(format!("natendo-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        init(dir.clone());
        let line = "y".repeat(MAX_LINE);
        // About 2 KB a line: 1 MB is ~500 lines; write enough for 6 turns.
        for _ in 0..3_200 {
            info(&line);
        }
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names.len(), KEEP as usize, "{names:?}");
        assert!(fs::metadata(file(&dir, 0)).unwrap().len() <= MAX_BYTES + 4096);
        let _ = fs::remove_dir_all(&dir);
    }
}
