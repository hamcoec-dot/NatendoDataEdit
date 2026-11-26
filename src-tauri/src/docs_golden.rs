//! Golden records: fold the accepted pairs of a Match people run on one list
//! into a new table. The tab itself is never changed, the pairs must be those
//! of its current contents, and nothing here logs a value.

use super::*;
use natendo_engine::golden::{ClusterView, GoldenCounts, GoldenPlan, GoldenSpec, ReviewLine};

/// Clusters per page of the preview.
const PAGE: usize = 200;

/// What the preview says first.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GoldenSummary {
    pub counts: GoldenCounts,
    /// Clusters in the plan (merged and skipped).
    pub listed: u64,
    /// Pairs above the score that the match screen never listed.
    pub never_offered: u64,
    /// Rows left for review, with why (the first 10,000).
    pub review: Vec<ReviewLine>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GoldenMade {
    pub info: DocInfo,
    /// The rows folded in, when asked for.
    pub away: Option<DocInfo>,
}

fn invalid(m: &str) -> CmdError {
    CmdError::new("invalid", m)
}

fn stale() -> CmdError {
    CmdError::new(
        "changed",
        "the table changed since the match: run Match people again",
    )
}

fn fix(e: Error) -> CmdError {
    if matches!(e, Error::Changed) {
        stale()
    } else {
        e.into()
    }
}

impl Docs {
    fn golden_kept(&self, id: u32) -> CmdResult<Arc<GoldenPlan>> {
        self.get(id)?
            .golden
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| invalid("preview the golden records again"))
    }

    /// Cluster the accepted pairs (row id, the other row as shown at the
    /// match, score) and choose values. Nothing changes.
    pub fn golden_plan(
        &self,
        id: u32,
        accepted: &[(u32, u64, f32)],
        mut spec: GoldenSpec,
    ) -> CmdResult<GoldenSummary> {
        if accepted.is_empty() {
            return Err(invalid("accept at least one pair"));
        }
        if accepted.len() > natendo_engine::fuzzy::MAX_LISTED {
            return Err(invalid("too many pairs at once"));
        }
        let entry = self.get(id)?;
        *entry.golden.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.refuse_peeking(id)?;
        let (revision, never_offered) = {
            let m = entry.matched.lock().unwrap_or_else(|p| p.into_inner());
            let kept = m
                .as_ref()
                .ok_or_else(|| invalid("find the matches again"))?;
            if kept.other.is_some() {
                return Err(invalid(
                    "golden records come from matching one list with itself",
                ));
            }
            spec.keep_first = kept.keep_first;
            (kept.revision, kept.never_offered)
        };
        let job = entry.start("Making golden records")?;
        let d = entry.read();
        // The match's shown rows mean nothing once the table has changed.
        if d.revision() != revision {
            return Err(stale());
        }
        let pairs = d.golden_pairs(accepted).map_err(fix)?;
        let plan = d.plan_golden(&pairs, &spec, &job.progress).map_err(fix)?;
        let review = d.golden_review(&plan).map_err(fix)?;
        let summary = GoldenSummary {
            counts: plan.counts.clone(),
            listed: plan.clusters.len() as u64,
            never_offered,
            review,
        };
        drop(d);
        *entry.golden.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(plan));
        Ok(summary)
    }

    /// Page `page` (from 0) of the preview's clusters.
    pub fn golden_page(&self, id: u32, page: usize) -> CmdResult<Vec<ClusterView>> {
        let plan = self.golden_kept(id)?;
        let entry = self.get(id)?;
        let d = entry.read();
        d.golden_page(&plan, page.saturating_mul(PAGE), PAGE)
            .map_err(fix)
    }

    /// The result as a new tab (and the folded rows as another with `away`),
    /// private when the source is.
    pub fn golden_create(&self, id: u32, away: bool) -> CmdResult<GoldenMade> {
        let plan = self.golden_kept(id)?;
        let entry = self.get(id)?;
        let job = entry.start("Making golden records")?;
        let (table, folded) = entry
            .read()
            .golden_tables(&plan, away, &job.progress)
            .map_err(fix)?;
        drop(job);
        let stem = self.stem(id);
        let open = |table: Table, title: String| -> CmdResult<DocInfo> {
            let (new, _) = self.insert(table.into_document()?, false)?;
            self.inherit_private(new, &[id], &[]);
            self.set_title_of(new, title);
            self.info(new)
        };
        let folded = folded
            .map(|t| open(t, format!("Merged away ({stem})")))
            .transpose()?;
        let info = open(table, format!("Golden records ({stem})"))?;
        Ok(GoldenMade { info, away: folded })
    }

    pub fn golden_close(&self, id: u32) {
        if let Ok(e) = self.get(id) {
            *e.golden.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }
}

#[cfg(test)]
#[path = "docs_golden_tests.rs"]
mod tests;
