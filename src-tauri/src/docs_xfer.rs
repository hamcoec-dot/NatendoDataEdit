//! Whole columns and whole rows dragged from one open file onto another's tab (or sent there
//! with Edit › Copy/Move selection to another file). The source is copied out under its own
//! lock, the target is written under its own (never both at once), and a move deletes from the
//! source only after the target's step is in, and only if the source is as it was.

use super::*;
use natendo_engine::xfer::{Wider, XferReport};

/// Cells the clipboard can hold for an insert (the same limit as Copy).
const MAX_COPY_CELLS: u64 = super::MAX_COPY_CELLS;

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

    /// Insert copied rows: the clipboard's rows (as Paste would read them) right above the shown row
    /// `before`, by position, as one undo step. `src`: the tab they were copied from, for the private
    /// rule (the values are always what was copied).
    pub fn insert_copied_rows(
        &self,
        dst: u32,
        before: u64,
        text: &str,
        src: Option<u32>,
        wider: Wider,
    ) -> CmdResult<XferDone> {
        self.insert_copied_checks(dst, src)?;
        let grid = natendo_engine::ops::parse_tsv(text, MAX_COPY_CELLS)?;
        if grid.is_empty() {
            return Err(CmdError::new("invalid", "the clipboard holds no rows"));
        }
        let data = natendo_engine::xfer::RowsOut::from_grid(&grid)?;
        let target = self.get(dst)?;
        let job = target.start("Inserting copied rows")?;
        let mut report = XferReport::default();
        let info = self.recorded(dst, Logged::LeftOut("copied rows inserted".into()), || {
            self.edit(dst, |d| {
                let (plan, rep) = d.plan_insert_rows_above(before, &data, wider, &job.progress)?;
                report = rep;
                d.apply_join(plan)
            })
        })?;
        drop(job);
        if let Some(s) = src.filter(|&s| s != dst) {
            self.inherit_private(dst, &[s], &[]);
        }
        Ok(XferDone {
            report,
            dst: info,
            src: None,
            kept: None,
        })
    }

    /// Insert copied columns: the clipboard's columns at logical column `at`, by position over the
    /// rows, with `titles` in the title row when this tab has one (`keep_first`). Refused while a
    /// filter is on or rows are hidden here, as a drop of columns is.
    pub fn insert_copied_cols(
        &self,
        dst: u32,
        at: u32,
        text: &str,
        titles: Vec<String>,
        keep_first: bool,
        src: Option<u32>,
    ) -> CmdResult<XferDone> {
        self.insert_copied_checks(dst, src)?;
        let target = self.get(dst)?;
        if target.read().is_filtered() {
            return Err(CmdError::new(
                "filtered",
                "this file has a filter on or rows hidden: show every row first, so you can see where the values go",
            ));
        }
        let grid = natendo_engine::ops::parse_tsv(text, MAX_COPY_CELLS)?;
        if grid.is_empty() {
            return Err(CmdError::new("invalid", "the clipboard holds no columns"));
        }
        let data = natendo_engine::xfer::ColsOut::from_grid(&grid, titles)?;
        let job = target.start("Inserting copied columns")?;
        let mut report = XferReport::default();
        let info = self.recorded(
            dst,
            Logged::LeftOut("copied columns inserted".into()),
            || {
                self.edit(dst, |d| {
                    let (plan, rep) = d.plan_insert_cols(at, &data, keep_first, &job.progress)?;
                    report = rep;
                    d.apply_join(plan)
                })
            },
        )?;
        drop(job);
        if let Some(s) = src.filter(|&s| s != dst) {
            self.inherit_private(dst, &[s], &[]);
        }
        Ok(XferDone {
            report,
            dst: info,
            src: None,
            kept: None,
        })
    }

    fn insert_copied_checks(&self, dst: u32, src: Option<u32>) -> CmdResult<()> {
        if let Some(s) = src.filter(|&s| s != dst) {
            if self.private_or_encrypted(s)? && !self.private_or_encrypted(dst)? {
                return Err(CmdError::new("private", PRIVATE_INTO));
            }
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
    #[test]
    fn insert_copied_rows_go_above_as_one_step_and_equal_inserting_by_hand() {
        let docs = Docs::default();
        let a = open(&docs, "ins.csv", "id,city\r\n1,Ames\r\n2,Bly\r\n3,Cody\r\n");
        let h = docs.info(a).unwrap().history_pos;
        let done = docs
            .insert_copied_rows(
                a,
                2,
                "8\tGil\r\n9\t\"Hy\tHo\"\r\n",
                Some(a),
                Wider::AddColumns,
            )
            .unwrap();
        assert_eq!(done.dst.history_pos, h + 1, "one undo step");
        let rows: Vec<String> = (0..6)
            .map(|r| {
                let d = docs.get(a).unwrap();
                let d = d.read();
                format!(
                    "{},{}",
                    d.cell(r, 0).unwrap_or_default(),
                    d.cell(r, 1).unwrap_or_default()
                )
            })
            .collect();
        assert_eq!(
            rows,
            ["id,city", "1,Ames", "8,Gil", "9,Hy\tHo", "2,Bly", "3,Cody"]
        );
        // One Undo takes it all out.
        docs.get(a).unwrap().write().undo();
        assert_eq!(docs.get(a).unwrap().read().row_count(), 4);
    }

    #[test]
    fn insert_copied_rows_at_the_top_of_a_file_without_titles_and_wider_rows() {
        let docs = Docs::default();
        let a = open(&docs, "top.csv", "1\r\n2\r\n");
        docs.insert_copied_rows(a, 0, "x\ty\r\n", None, Wider::AddColumns)
            .unwrap();
        let d = docs.get(a).unwrap();
        let d = d.read();
        assert_eq!(d.cell(0, 0).as_deref(), Some("x"));
        assert_eq!(d.cell(0, 1).as_deref(), Some("y"));
        assert_eq!(d.cell(1, 0).as_deref(), Some("1"));
        assert_eq!(d.col_count(), 2);
    }

    #[test]
    fn insert_copied_rows_above_a_row_keep_hidden_rows_before_it_before_it() {
        let docs = Docs::default();
        let a = open(&docs, "hid.csv", "n\r\na\r\nb\r\nc\r\n");
        docs.get(a).unwrap().write().hide_rows(2, 1).unwrap(); // b hidden: shown are n, a, c
        docs.insert_copied_rows(a, 2, "new\r\n", None, Wider::Cut)
            .unwrap(); // above c
        let d = docs.get(a).unwrap();
        let mut d = d.write();
        assert_eq!(d.row_count(), 4, "the new row shows; b stays hidden");
        assert_eq!(d.cell(2, 0).as_deref(), Some("new"));
        d.show_rows(natendo_engine::rows::ShowRows::All).unwrap();
        let all: Vec<String> = (0..5).map(|r| d.cell(r, 0).unwrap_or_default()).collect();
        assert_eq!(all, ["n", "a", "b", "new", "c"]);
    }

    #[test]
    fn insert_copied_cols_with_titles_one_step_and_refusals() {
        let docs = Docs::default();
        let a = open(&docs, "c.csv", "id,zip\r\n1,50010\r\n2,97622\r\n");
        let h = docs.info(a).unwrap().history_pos;
        let done = docs
            .insert_copied_cols(a, 1, "Ames\r\nBly\r\n", vec!["city".into()], true, Some(a))
            .unwrap();
        assert_eq!(done.dst.history_pos, h + 1);
        {
            let d = docs.get(a).unwrap();
            let d = d.read();
            let row = |r| {
                (0..3)
                    .map(|c| d.cell(r, c).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            assert_eq!(
                [row(0), row(1), row(2)],
                ["id,city,zip", "1,Ames,50010", "2,Bly,97622"]
            );
        }
        // Private into not private, and a filtered target, are refused.
        let p = open(&docs, "p.csv", "x\r\n1\r\n");
        docs.get(p).unwrap().private.store(true, Ordering::SeqCst);
        assert_eq!(
            docs.insert_copied_cols(a, 0, "v\r\n", vec![], true, Some(p))
                .unwrap_err()
                .kind,
            "private"
        );
        assert_eq!(
            docs.insert_copied_rows(a, 1, "v\r\n", Some(p), Wider::Cut)
                .unwrap_err()
                .kind,
            "private"
        );
        docs.get(a).unwrap().write().hide_rows(1, 1).unwrap();
        assert_eq!(
            docs.insert_copied_cols(a, 0, "v\r\n", vec![], true, None)
                .unwrap_err()
                .kind,
            "filtered"
        );
    }
}
