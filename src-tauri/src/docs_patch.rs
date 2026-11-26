//! Patch files (`.ndpatch`): make one from two tabs, dry run and apply one.
//! A patch holds data, so nothing here logs or keeps its values beyond the
//! dry run in memory, and no recipe repeats it.

use super::*;
use natendo_engine::patch::{Patch, PatchReport, PatchSpec};
use natendo_engine::privacy::Secret;

/// The last dry run on a tab.
#[derive(Clone)]
pub(super) struct PatchKept {
    patch: Arc<Patch>,
    report: Arc<PatchReport>,
    /// The dry run's plan, for Apply (planned again if it is gone).
    plan: Arc<Mutex<Option<RowPlan>>>,
    /// The tab's revision at the dry run (`u64::MAX`: applied since).
    revision: u64,
    /// The patch file was private, or protected with a phrase.
    private: bool,
}

/// A patch file written.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PatchSaved {
    pub changes: u64,
    pub cells: u64,
    pub adds: u64,
    pub removes: u64,
    /// Columns only one table has, left out of the patch.
    pub left_out: Vec<String>,
    pub protected: bool,
}

/// What applying a patch file would do.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PatchPreview {
    pub report: Arc<PatchReport>,
    /// Applying marks the tab private.
    pub private: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PatchApplied {
    pub info: DocInfo,
    pub cells: u64,
    pub added: u64,
    pub removed: u64,
}

fn invalid(m: &str) -> CmdError {
    CmdError::new("invalid", m)
}

fn stale() -> CmdError {
    CmdError::new(
        "changed",
        "the table changed since the dry run, so nothing was applied. Run the dry run again",
    )
}

fn secret(phrase: Option<String>) -> Option<Secret> {
    phrase.filter(|p| !p.is_empty()).map(|p| Secret::new(&p))
}

impl Docs {
    fn patch_kept(&self, id: u32) -> CmdResult<PatchKept> {
        self.get(id)?
            .patch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| invalid("choose the patch file again"))
    }

    /// A path a patch or its report may be written to.
    fn patch_target(&self, path: &Path) -> CmdResult<()> {
        self.check_writable(path)?;
        if self.find_open(path).is_some() {
            return Err(invalid("choose another name: that file is open"));
        }
        Ok(())
    }

    /// Write the differences from tab `a` to the newer tab `b` to `path`.
    /// Without a phrase, a private source needs `plain` (the user said so).
    pub fn patch_make(
        &self,
        a: u32,
        b: u32,
        spec: &PatchSpec,
        path: &Path,
        phrase: Option<String>,
        plain: bool,
    ) -> CmdResult<PatchSaved> {
        if a == b {
            return Err(invalid("choose two different tables"));
        }
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ndpatch"))
        {
            return Err(invalid("a patch file's name ends in .ndpatch"));
        }
        self.patch_target(path)?;
        let phrase = secret(phrase);
        if phrase.is_none() && !plain && (self.is_private(a)? || self.is_private(b)?) {
            return Err(invalid(
                "one of these tables is private: protect the patch with a phrase, or choose to save it unprotected",
            ));
        }
        self.refuse_peeking(a)?;
        self.refuse_peeking(b)?;
        let (ea, eb) = (self.get(a)?, self.get(b)?);
        let job = ea.start("Making a patch")?;
        let made = {
            // Always the lower tab number first, so two of these never wait
            // on each other.
            let (older, newer);
            if a < b {
                older = ea.read();
                newer = eb.read();
            } else {
                newer = eb.read();
                older = ea.read();
            }
            older.make_patch(&newer, spec, &job.progress)?
        };
        let p = &made.patch;
        if p.entries() == 0 {
            return Err(invalid("the tables have no differences to save"));
        }
        p.save(path, phrase.as_ref())?;
        Ok(PatchSaved {
            changes: p.changes.len() as u64,
            cells: p.changes.iter().map(|c| c.cells.len() as u64).sum(),
            adds: p.adds.len() as u64,
            removes: p.removes.len() as u64,
            left_out: made.left_out,
            protected: phrase.is_some(),
        })
    }

    /// The dry run: what applying the patch file to tab `id` would do.
    pub fn patch_preview(
        &self,
        id: u32,
        path: &Path,
        phrase: Option<String>,
    ) -> CmdResult<PatchPreview> {
        let entry = self.get(id)?;
        *entry.patch.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.refuse_peeking(id)?;
        let job = entry.start("Reading the patch")?;
        let protected = Patch::is_protected(path)?;
        let patch = Patch::load(path, secret(phrase).as_ref())?;
        let (plan, report, revision) = {
            let d = entry.read();
            let (plan, report) = d.plan_patch(&patch, &job.progress)?;
            (plan, Arc::new(report), d.revision())
        };
        let private = protected || self.is_private_path(path);
        *entry.patch.lock().unwrap_or_else(|p| p.into_inner()) = Some(PatchKept {
            patch: Arc::new(patch),
            report: report.clone(),
            plan: Arc::new(Mutex::new(Some(plan))),
            revision,
            private,
        });
        Ok(PatchPreview { report, private })
    }

    /// Apply the patch of the dry run as one undo step.
    pub fn patch_apply(&self, id: u32) -> CmdResult<PatchApplied> {
        let kept = self.patch_kept(id)?;
        if kept.revision == u64::MAX {
            return Err(invalid("this patch was applied: choose it again to check"));
        }
        let entry = self.get(id)?;
        if entry.read().revision() != kept.revision {
            return Err(stale());
        }
        if kept.report.will_apply == 0 {
            return Err(invalid("there is nothing to apply"));
        }
        if entry.locked.load(Ordering::Relaxed) {
            return Err(read_only());
        }
        // Before the change, so that no recovery copy ever holds the values.
        if kept.private {
            entry.private.store(true, Ordering::SeqCst);
            entry.private_origin.store(true, Ordering::SeqCst);
            crate::recovery::forget(id);
        }
        let info = self
            .recorded(id, Logged::LeftOut("applying a patch".into()), || {
                self.compute_apply(
                    id,
                    "Applying the patch",
                    |d, p| {
                        if d.revision() != kept.revision {
                            return Err(Error::Changed);
                        }
                        let held = kept.plan.lock().unwrap_or_else(|p| p.into_inner()).take();
                        match held {
                            Some(plan) => Ok(plan),
                            None => Ok(d.plan_patch(&kept.patch, p)?.0),
                        }
                    },
                    |d, plan| d.apply_rows(plan),
                )
            })
            .map_err(|e| if e.kind == "changed" { stale() } else { e })?;
        if let Some(k) = entry
            .patch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            k.revision = u64::MAX;
        }
        Ok(PatchApplied {
            info,
            cells: kept.report.cells,
            added: kept.report.added,
            removed: kept.report.removed,
        })
    }

    /// The dry run's lines as a CSV file (formulas guarded).
    pub fn patch_report_save(&self, id: u32, path: &Path) -> CmdResult<()> {
        let kept = self.patch_kept(id)?;
        self.patch_target(path)?;
        Ok(kept.report.save_csv(path)?)
    }

    pub fn patch_close(&self, id: u32) {
        if let Ok(e) = self.get(id) {
            *e.patch.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }
}

#[cfg(test)]
#[path = "docs_patch_tests.rs"]
mod tests;
