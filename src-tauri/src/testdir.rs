//! Temp files for tests. Each test run gets its own folder, so a run whose
//! process id an old run also had never meets that run's leftover files.
//! Folders from runs more than a day old are removed when a run starts.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PREFIX: &str = "nde-test-";

/// Names the tests used before this folder existed (process id only).
const OLD: &[&str] = &[
    "nde-docs-",
    "nde-audit-",
    "nde-batch-",
    "nd-watch-",
    "nd-a1a-open-",
    "natendo-n1-",
    "since-report-",
    "nd-guard-",
    "since-titles-",
    "nde-clash-",
    "nde-fk-",
    "nde-sh-golden",
    "nde-sh-patch-",
    "natendo-recent-",
    "natendo-private-",
    "natendo-imp-",
    "natendo-pcopy-",
    "nde-recovery-",
    "nde-session-",
    "nde-zip-",
    "natendo-log-",
];

/// This run's folder.
pub(crate) fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let tmp = std::env::temp_dir();
        sweep(&tmp);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = tmp.join(format!("{PREFIX}{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    })
}

/// A path for one test's file or folder, inside this run's folder.
pub(crate) fn path(name: &str) -> PathBuf {
    root().join(name)
}

/// Removes test folders and files that are more than a day old.
fn sweep(tmp: &Path) {
    let Ok(entries) = std::fs::read_dir(tmp) else {
        return;
    };
    let day = Duration::from_secs(24 * 60 * 60);
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(PREFIX) && !OLD.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > day);
        if !old {
            continue;
        }
        // The entry's own type: a link is removed as a link and never
        // followed into what it points at.
        let Ok(t) = e.file_type() else {
            continue;
        };
        let p = e.path();
        if t.is_symlink() {
            // A Windows link to a folder goes with remove_dir.
            if std::fs::remove_file(&p).is_err() {
                let _ = std::fs::remove_dir(&p);
            }
        } else if t.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else if t.is_file() {
            let _ = std::fs::remove_file(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_run_has_its_own_folder() {
        let a = super::path("x");
        assert!(a.starts_with(super::root()));
        assert!(super::root().is_dir());
        let name = super::root()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(name.starts_with(&format!("{}{}-", super::PREFIX, std::process::id())));
    }
}
