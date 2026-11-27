//! Whole columns and whole rows dragged from one open file onto another's tab (or sent there
//! with Edit › Copy/Move selection to another file). The source is copied out under its own
//! lock, the target is written under its own (never both at once), and a move deletes from the
//! source only after the target's step is in, and only if the source is as it was.

use super::*;
use natendo_engine::xfer::{Wider, XferReport};

/// What a cross-tab copy or move did.
#[derive(Debug, Clone, Serialize)]
pub struct XferDone {
    pub report: XferReport,
    /// The target's facts after the insert.
    pub dst: DocInfo,
    /// The source's facts after a move (`None`: nothing was taken out).
    pub src: Option<DocInfo>,
    /// A move that copied but did not take out, and why (in words).
    pub kept: Option<String>,
}

/// Why a private or encrypted tab's cells don't go into one that is not (no names, no values).
pub const PRIVATE_INTO: &str =
    "the cells come from a private or encrypted file and this one is not private: make it private first";

impl Docs {
    fn private_or_encrypted(&self, id: u32) -> CmdResult<bool> {
        let entry = self.get(id)?;
        let enc = entry
            .encrypted_from
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some();
        Ok(enc || entry.private.load(Ordering::SeqCst))
    }

    fn xfer_checks(&self, src: u32, dst: u32) -> CmdResult<()> {
        if src == dst {
            return Err(CmdError::new("bad_request", "that is this file's tab"));
        }
        if self.private_or_encrypted(src)? && !self.private_or_encrypted(dst)? {
            return Err(CmdError::new("private", PRIVATE_INTO));
        }
        let e = self.get(dst)?;
        if e.peeking.load(Ordering::Relaxed) {
            return Err(peeking());
        }
        if e.locked.load(Ordering::Relaxed) {
            return Err(read_only());
        }
        Ok(())
    }

    /// Copy (or with `mv`, move) the source's logical columns `at..at + count` into `dst`,
    /// inserted at its logical column `to`, lined up by position over every row. Refused while a
    /// filter is on in the target (the values would land where nobody can see them).
    #[allow(clippy::too_many_arguments)]
    pub fn xfer_cols(
        &self,
        src: u32,
        at: u32,
        count: u32,
        src_keep_first: bool,
        dst: u32,
        to: u32,
        dst_keep_first: bool,
        mv: bool,
    ) -> CmdResult<XferDone> {
        self.xfer_checks(src, dst)?;
        let target = self.get(dst)?;
        if target.read().is_filtered() {
            return Err(CmdError::new(
                "filtered",
                "that file has a filter on or rows hidden: show every row there first, so you can see where the values go",
            ));
        }
        let job = target.start("Copying columns")?;
        let cols: Vec<u32> = (at..at.saturating_add(count)).collect();
        let (data, revision) = {
            let s = self.get(src)?;
            let d = s.read();
            (
                d.take_cols(&cols, src_keep_first, &job.progress)?,
                d.revision(),
            )
        };
        let mut report = XferReport::default();
        let info = self.recorded(
            dst,
            Logged::LeftOut("columns from another file".into()),
            || {
                self.edit(dst, |d| {
                    let (plan, rep) =
                        d.plan_insert_cols(to, &data, dst_keep_first, &job.progress)?;
                    report = rep;
                    d.apply_join(plan)
                })
            },
        )?;
        drop(job);
        self.inherit_private(dst, &[src], &[]);
        let mut done = XferDone {
            report,
            dst: info,
            src: None,
            kept: None,
        };
        if mv {
            self.take_out(src, revision, &mut done, |d| d.delete_cols(at, count));
        }
        Ok(done)
    }

    /// Copy (or move) the source's shown rows `at..at + count` into `dst`, after its shown row
    /// `after` (`None`: at the end); by title when the two title sets match cleanly.
    #[allow(clippy::too_many_arguments)]
    pub fn xfer_rows(
        &self,
        src: u32,
        at: u64,
        count: u64,
        src_keep_first: bool,
        dst: u32,
        after: Option<u64>,
        dst_keep_first: bool,
        wider: Wider,
        mv: bool,
    ) -> CmdResult<XferDone> {
        self.xfer_checks(src, dst)?;
        let target = self.get(dst)?;
        let job = target.start("Copying rows")?;
        let rows: Vec<u64> = (at..at.saturating_add(count)).collect();
        let (data, revision) = {
            let s = self.get(src)?;
            let d = s.read();
            (
                d.take_rows(&rows, src_keep_first, &job.progress)?,
                d.revision(),
            )
        };
        let by_title = src_keep_first && dst_keep_first;
        let mut report = XferReport::default();
        let info = self.recorded(
            dst,
            Logged::LeftOut("rows from another file".into()),
            || {
                self.edit(dst, |d| {
                    let (plan, rep) =
                        d.plan_insert_rows(after, &data, by_title, wider, &job.progress)?;
                    report = rep;
                    d.apply_join(plan)
                })
            },
        )?;
        drop(job);
        self.inherit_private(dst, &[src], &[]);
        let mut done = XferDone {
            report,
            dst: info,
            src: None,
            kept: None,
        };
        if mv {
            self.take_out(src, revision, &mut done, |d| d.delete_rows(at, count));
        }
        Ok(done)
    }

    /// The move's second half: only when the source is as it was copied, and never by force.
    fn take_out(
        &self,
        src: u32,
        revision: u64,
        done: &mut XferDone,
        f: impl FnOnce(&mut Document) -> Result<(), Error>,
    ) {
        let Ok(entry) = self.get(src) else {
            done.kept = Some("the file it came from was closed".into());
            return;
        };
        if entry.read().revision() != revision {
            done.kept = Some("the file it came from changed meanwhile".into());
            return;
        }
        let res = self.recorded(src, Logged::LeftOut("moved to another file".into()), || {
            self.edit(src, |d| {
                if d.revision() != revision {
                    return Err(Error::Changed);
                }
                f(d)
            })
        });
        match res {
            Ok(info) => done.src = Some(info),
            Err(e) => done.kept = Some(e.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(docs: &Docs, name: &str, text: &str) -> u32 {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("nde-xfer-{}-{n}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        let id = docs.open_path(&p, false).unwrap().id;
        for _ in 0..1000 {
            if docs.info(id).unwrap().indexed {
                return id;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("indexing did not finish")
    }

    fn cells(docs: &Docs, id: u32) -> Vec<String> {
        let e = docs.get(id).unwrap();
        let d = e.read();
        (0..d.row_count())
            .map(|r| {
                (0..d.col_count())
                    .map(|c| d.cell(r, c).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect()
    }

    #[test]
    fn a_column_moves_and_each_file_undoes_its_half() {
        let docs = Docs::default();
        let a = open(&docs, "a.csv", "id,city\r\n1,Ames\r\n2,Bly\r\n");
        let b = open(&docs, "b.csv", "name\r\nAnn\r\nBo\r\n");
        let done = docs.xfer_cols(a, 1, 1, true, b, 1, true, true).unwrap();
        assert!(done.kept.is_none() && done.src.is_some());
        assert_eq!(cells(&docs, b), ["name,city", "Ann,Ames", "Bo,Bly"]);
        assert_eq!(cells(&docs, a), ["id", "1", "2"]);
        docs.undo(a).unwrap();
        assert_eq!(cells(&docs, a), ["id,city", "1,Ames", "2,Bly"]);
        docs.undo(b).unwrap();
        assert_eq!(cells(&docs, b), ["name", "Ann", "Bo"]);
    }

    #[test]
    fn refused_on_its_own_tab_into_a_filtered_tab_and_private_into_not_private() {
        let docs = Docs::default();
        let a = open(&docs, "a.csv", "id,city\r\n1,Ames\r\n2,Bly\r\n");
        let b = open(&docs, "b.csv", "name\r\nAnn\r\nBo\r\n");
        let e = docs
            .xfer_cols(a, 1, 1, true, a, 0, true, false)
            .unwrap_err();
        assert!(e.message.contains("this file's tab"));
        docs.hide_rows(b, 1, 1).unwrap();
        let e = docs
            .xfer_cols(a, 1, 1, true, b, 1, true, false)
            .unwrap_err();
        assert_eq!(e.kind, "filtered");
        assert_eq!(cells(&docs, b), ["name", "Bo"], "nothing written");
        docs.get(a).unwrap().private.store(true, Ordering::SeqCst);
        let e = docs
            .xfer_rows(a, 1, 1, true, b, None, true, Wider::Cut, false)
            .unwrap_err();
        assert_eq!(e.kind, "private");
        assert!(
            !e.message.contains("Ames") && !e.message.contains("a.csv"),
            "{}",
            e.message
        );
    }

    #[test]
    fn a_source_changed_meanwhile_keeps_its_rows() {
        let docs = Docs::default();
        let a = open(&docs, "a.csv", "id,city\r\n1,Ames\r\n2,Bly\r\n");
        let b = open(&docs, "b.csv", "id,city\r\n9,Nyc\r\n");
        let mut done = XferDone {
            report: XferReport::default(),
            dst: docs.info(b).unwrap(),
            src: None,
            kept: None,
        };
        // As a move's second half sees it after another change to the source.
        let rev = docs.get(a).unwrap().read().revision();
        docs.set_cells(a, vec![(1, 1, "X".into())]).unwrap();
        docs.take_out(a, rev, &mut done, |d| d.delete_rows(1, 1));
        assert_eq!(
            done.kept.as_deref(),
            Some("the file it came from changed meanwhile")
        );
        assert_eq!(cells(&docs, a).len(), 3);
        // Rows by title, moved.
        let done = docs
            .xfer_rows(a, 2, 1, true, b, None, true, Wider::Cut, true)
            .unwrap();
        assert!(done.kept.is_none());
        assert_eq!(cells(&docs, b), ["id,city", "9,Nyc", "2,Bly"]);
        assert_eq!(cells(&docs, a), ["id,city", "1,X"]);
    }
}
