//! Duplicate tab: an unsaved copy of a tab's file as it is now, in a new tab.
//!
//! The copy is made in memory (see `Document::duplicate`) and is private when
//! the original is: a private tab's copy never gets a recovery copy on disk,
//! and a copy of a tab from an encrypted file offers encryption when it is
//! saved.

use super::*;

/// Memory left free after the copy, so the rest of the app keeps working.
const KEEP_FREE: u64 = 256 << 20;

impl Docs {
    /// Copy tab `id` into a new tab called `title`. Runs as a job on `id`
    /// (its Cancel stops it, and nothing is kept).
    pub fn duplicate(&self, id: u32, title: &str) -> CmdResult<DocInfo> {
        let title: String = title.trim().chars().take(200).collect();
        if title.is_empty() {
            return Err(CmdError::new("invalid", "the copy needs a name"));
        }
        let entry = self.get(id)?;
        if entry.peeking.load(Ordering::Relaxed) {
            return Err(CmdError::new(
                "peeking",
                "this tab is showing an earlier step: go back to now first, then duplicate it",
            ));
        }
        let copy = {
            let job = entry.start("Duplicating")?;
            let d = entry.read();
            if !d.is_indexed() {
                return Err(Error::StillLoading.into());
            }
            let need = d.duplicate_bytes();
            if let Some(free) = natendo_engine::sysmem::available_bytes() {
                if need.saturating_add(KEEP_FREE) > free {
                    return Err(CmdError::new(
                        "too_large",
                        format!(
                            "a copy needs about {} MB of memory and {} MB are free: close other files or programs first",
                            need.div_ceil(1 << 20),
                            free >> 20
                        ),
                    ));
                }
            }
            d.duplicate(&job.progress)?
        };
        let private = entry.private.load(Ordering::SeqCst);
        let encrypted = entry
            .encrypted_from
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
            || entry.encrypted_copy.load(Ordering::SeqCst);
        drop(entry);
        // Private from the moment it is listed (nothing, not a recovery copy or another window, sees it otherwise).
        let (nid, _) = self.insert_with(copy, false, private || encrypted, encrypted)?;
        self.inherit_private(nid, &[id], &[]);
        self.set_title_of(nid, title);
        crate::recovery::forget(nid);
        self.info(nid)
    }

    /// Whether tab `id` is a Duplicate of a tab from an encrypted file.
    pub fn encrypted_copy(&self, id: u32) -> CmdResult<bool> {
        Ok(self.get(id)?.encrypted_copy.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    static N: AtomicU32 = AtomicU32::new(0);

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "nde-dupapp-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn open(docs: &Docs, p: &Path) -> u32 {
        let id = docs.open_path(p, false).unwrap().id;
        for _ in 0..500 {
            if docs.info(id).unwrap().indexed {
                return id;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("not indexed");
    }

    #[test]
    fn the_copy_is_unsaved_named_and_has_the_edits() {
        let docs = Docs::default();
        let p = dir().join("v.csv");
        std::fs::write(&p, "id,city\r\n1,Ames\r\n2,Bly\r\n").unwrap();
        let id = open(&docs, &p);
        docs.get(id)
            .unwrap()
            .write()
            .set_cell(1, 1, "Boise".into())
            .unwrap();
        let c = docs.duplicate(id, "v.csv (copy)").unwrap();
        assert_ne!(c.id, id);
        assert_eq!(c.name, "v.csv (copy)");
        assert!(c.modified, "closing it asks to save");
        assert!(c.path.is_none());
        assert_eq!(
            docs.get(c.id).unwrap().read().cell(1, 1).as_deref(),
            Some("Boise")
        );
        assert!(!docs.is_private(c.id).unwrap());
        assert!(!docs.encrypted_copy(c.id).unwrap());
    }

    #[test]
    fn a_private_files_copy_is_never_listed_as_not_private() {
        // Review note 1: watch the list of open files while copies are made; each copy must be private the first
        // time it is seen there.
        let docs = Docs::default();
        let p = dir().join("p.csv");
        std::fs::write(&p, "ssn\r\n000-00-0000\r\n").unwrap();
        let id = open(&docs, &p);
        docs.get(id).unwrap().private.store(true, Ordering::SeqCst);
        let done = AtomicBool::new(false);
        let seen_open = std::thread::scope(|s| {
            let w = s.spawn(|| {
                let mut bad = 0u32;
                let mut looks = 0u32;
                while !done.load(Ordering::SeqCst) {
                    for (k, e) in docs.map().iter() {
                        if *k != id {
                            looks += 1;
                            if !e.private.load(Ordering::SeqCst) {
                                bad += 1;
                            }
                        }
                    }
                }
                (bad, looks)
            });
            for _ in 0..200 {
                let c = docs.duplicate(id, "copy").unwrap();
                assert!(docs.is_private(c.id).unwrap());
                docs.close(c.id);
            }
            done.store(true, Ordering::SeqCst);
            w.join().unwrap()
        });
        assert_eq!(
            seen_open.0, 0,
            "a copy was listed as not private ({} looks)",
            seen_open.1
        );
    }

    #[test]
    fn a_private_copy_stays_private_and_never_gets_a_recovery_copy() {
        let docs = Docs::default();
        let p = dir().join("p.csv");
        std::fs::write(&p, "a\r\n1\r\n").unwrap();
        let id = open(&docs, &p);
        docs.get(id).unwrap().private.store(true, Ordering::SeqCst);
        let c = docs.duplicate(id, "p.csv (copy)").unwrap();
        assert!(docs.is_private(c.id).unwrap());
        docs.get(c.id)
            .unwrap()
            .write()
            .set_cell(1, 0, "x".into())
            .unwrap();
        assert_eq!(docs.autosave(c.id).unwrap(), Autosaved::Skipped);
    }

    #[test]
    fn a_copy_of_an_encrypted_tab_is_private_and_offers_encryption() {
        let docs = Docs::default();
        let p = dir().join("e.csv");
        std::fs::write(&p, "a\r\n1\r\n").unwrap();
        let id = open(&docs, &p);
        *docs.get(id).unwrap().encrypted_from.lock().unwrap() = Some(dir().join("e.ndenc"));
        let c = docs.duplicate(id, "e (copy)").unwrap();
        assert!(docs.is_private(c.id).unwrap());
        assert!(docs.encrypted_copy(c.id).unwrap());
        // Save writes a new file, never over the encrypted original.
        assert_eq!(docs.encrypted_from(c.id).unwrap(), None);
        assert_eq!(docs.autosave(c.id).unwrap(), Autosaved::Skipped);
        // A copy of the copy offers it too.
        let cc = docs.duplicate(c.id, "e (copy 2)").unwrap();
        assert!(docs.encrypted_copy(cc.id).unwrap());
    }

    #[test]
    fn refused_while_peeking_or_busy_or_unnamed() {
        let docs = Docs::default();
        let p = dir().join("b.csv");
        std::fs::write(&p, "a\r\n1\r\n").unwrap();
        let id = open(&docs, &p);
        assert_eq!(docs.duplicate(id, "  ").unwrap_err().kind, "invalid");
        let e = docs.get(id).unwrap();
        e.peeking.store(true, Ordering::Relaxed);
        assert_eq!(docs.duplicate(id, "x").unwrap_err().kind, "peeking");
        e.peeking.store(false, Ordering::Relaxed);
        let _job = e.start("Something else").unwrap();
        assert_eq!(docs.duplicate(id, "x").unwrap_err().kind, "busy");
    }
}
