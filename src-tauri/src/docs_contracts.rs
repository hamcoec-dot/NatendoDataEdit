//! File contracts (C9): the contracts folder in the app's data folder, recognising the file a tab
//! holds, the drift check (a count any change stops), Fix what I can, and making a contract.
//!
//! The webview names a contract by its name only, never by a path: every path is the folder plus
//! a checked name, except the one file an Import or Export dialog picked. A contract is never made
//! or changed from a private tab or one opened from an encrypted file (it would keep its titles
//! and values), and such a tab is never matched by itself.

use super::*;
use natendo_engine::checkfile::LoadedRules;
use natendo_engine::contract::{
    self, check_name, name_taken, pair, pair_in_order, recognise, Contract, ContractFix, Draft,
    DraftSpec, DriftOptions, DriftReport, DriftRows, Pairing, MAX_FILE_BYTES,
};

/// Contracts kept in the folder at most.
pub const MAX_CONTRACTS: usize = 1_000;

/// What making or changing a contract from a private tab is refused with.
pub const PRIVATE_WHY: &str =
    "This tab is private (or from an encrypted file): a contract would keep its titles and values";

/// The contracts folder as last read.
pub struct Store {
    folder: PathBuf,
    items: Vec<Stored>,
}

struct Stored {
    file: String,
    contract: Result<Contract, String>,
}

/// A contract in the list.
#[derive(Debug, Clone, Serialize)]
pub struct ContractItem {
    pub name: String,
    /// The file's own name (a broken one shows by it).
    pub file: String,
    pub columns: usize,
    pub pattern: Option<String>,
    /// The file could not be read: why (the others still load).
    pub problem: Option<String>,
}

/// The contract a tab looks like.
#[derive(Debug, Clone, Serialize)]
pub struct ContractMatch {
    pub name: String,
    pub by_pattern: bool,
    pub title_share: f64,
    /// Other contracts it also looks like, best first.
    pub others: Vec<String>,
    /// Those that score exactly as well as the best.
    pub ties: Vec<String>,
}

/// What Fix what I can did, and the contract's checks as they fit the fixed table.
#[derive(Debug, Clone, Serialize)]
pub struct ContractFixed {
    pub info: DocInfo,
    pub fix: ContractFix,
    pub checks: Option<LoadedRules>,
}

#[cfg(test)]
pub static MATCH_READS: AtomicU64 = AtomicU64::new(0);

fn io(what: &str) -> CmdError {
    CmdError::new("io", what)
}

fn read_folder(folder: &Path) -> Store {
    let mut items = Vec::new();
    if let Ok(dir) = std::fs::read_dir(folder) {
        for e in dir.flatten() {
            let path = e.path();
            let is_contract = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case(contract::EXTENSION));
            if !is_contract || !path.is_file() || items.len() >= MAX_CONTRACTS {
                continue;
            }
            let file = e.file_name().to_string_lossy().into_owned();
            let contract = match e.metadata() {
                Ok(m) if m.len() > MAX_FILE_BYTES as u64 => {
                    Err("the contract file is too large (over 4 MB)".to_string())
                }
                Ok(_) => match std::fs::read(&path) {
                    Ok(b) => Contract::from_bytes(&b).map_err(|e| e.to_string()),
                    Err(_) => Err("the contract file could not be read".to_string()),
                },
                Err(_) => Err("the contract file could not be read".to_string()),
            };
            items.push(Stored { file, contract });
        }
    }
    items.sort_by_key(|s| match &s.contract {
        Ok(c) => c.name.to_lowercase(),
        Err(_) => s.file.to_lowercase(),
    });
    Store {
        folder: folder.to_path_buf(),
        items,
    }
}

impl Store {
    fn names(&self) -> impl Iterator<Item = &str> {
        self.items
            .iter()
            .filter_map(|s| s.contract.as_ref().ok().map(|c| c.name.as_str()))
    }

    fn named(&self, name: &str) -> CmdResult<&Contract> {
        self.items
            .iter()
            .filter_map(|s| s.contract.as_ref().ok())
            .find(|c| c.name == name)
            .ok_or_else(|| CmdError::new("not_found", "that contract is not in the list any more"))
    }

    fn list(&self) -> Vec<ContractItem> {
        self.items
            .iter()
            .map(|s| match &s.contract {
                Ok(c) => ContractItem {
                    name: c.name.clone(),
                    file: s.file.clone(),
                    columns: c.columns.len(),
                    pattern: c.recognise.pattern.clone(),
                    problem: None,
                },
                Err(why) => ContractItem {
                    name: s.file.clone(),
                    file: s.file.clone(),
                    columns: 0,
                    pattern: None,
                    problem: Some(why.clone()),
                },
            })
            .collect()
    }
}

/// Write a contract into the folder: a temporary file moved into place.
fn write_contract(folder: &Path, c: &Contract) -> CmdResult<()> {
    let bytes = c.to_bytes()?;
    let path = folder.join(contract::file_name(&c.name));
    let tmp = folder.join(format!(".{}.tmp", contract::file_name(&c.name)));
    std::fs::write(&tmp, &bytes).map_err(|_| io("the contract could not be saved"))?;
    std::fs::rename(&tmp, &path).map_err(|_| {
        let _ = std::fs::remove_file(&tmp);
        io("the contract could not be saved")
    })
}

impl Docs {
    /// The folder's contracts, read now if they were not yet (or the folder changed).
    fn contract_store(&self, folder: &Path) -> MutexGuard<'_, Option<Store>> {
        let mut g = self.contracts.lock().unwrap_or_else(|p| p.into_inner());
        if g.as_ref().is_none_or(|s| s.folder != folder) {
            *g = Some(read_folder(folder));
        }
        g
    }

    fn contracts_reload(&self, folder: &Path) -> Vec<ContractItem> {
        let store = read_folder(folder);
        let list = store.list();
        *self.contracts.lock().unwrap_or_else(|p| p.into_inner()) = Some(store);
        list
    }

    /// One contract by name (a copy).
    pub fn contract_get(&self, folder: &Path, name: &str) -> CmdResult<Contract> {
        let g = self.contract_store(folder);
        Ok(g.as_ref().expect("read above").named(name)?.clone())
    }

    /// The contracts, read again from the folder.
    pub fn contracts_list(&self, folder: &Path) -> CmdResult<Vec<ContractItem>> {
        Ok(self.contracts_reload(folder))
    }

    /// A private tab, or one from an encrypted file.
    fn tab_is_private(&self, id: u32) -> CmdResult<bool> {
        let entry = self.get(id)?;
        let enc = entry
            .encrypted_from
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some();
        Ok(enc || entry.private.load(Ordering::SeqCst))
    }

    fn refuse_private(&self, id: u32) -> CmdResult<()> {
        if self.tab_is_private(id)? {
            return Err(CmdError::new("private", PRIVATE_WHY));
        }
        Ok(())
    }

    /// The contract tab `id` looks like, from its file name and titles only (`keep_first`: it has
    /// a title row). Nothing is read when there are no contracts, and a private or encrypted tab is
    /// never matched by itself.
    pub fn contract_match(
        &self,
        folder: &Path,
        id: u32,
        keep_first: bool,
    ) -> CmdResult<Option<ContractMatch>> {
        if self.tab_is_private(id)? {
            return Ok(None);
        }
        let g = self.contract_store(folder);
        let store = g.as_ref().expect("read above");
        let list: Vec<&Contract> = store
            .items
            .iter()
            .filter_map(|s| s.contract.as_ref().ok())
            .collect();
        if list.is_empty() {
            return Ok(None);
        }
        #[cfg(test)]
        MATCH_READS.fetch_add(1, Ordering::Relaxed);
        let entry = self.get(id)?;
        let d = entry.read();
        let titles = keep_first.then(|| d.rule_titles(true));
        let file = d
            .path()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().into_owned());
        drop(d);
        let m = recognise(list.iter().copied(), file.as_deref(), titles.as_deref());
        let Some(best) = m.first() else {
            return Ok(None);
        };
        let score = |x: &contract::Match| (x.by_pattern, x.title_share);
        Ok(Some(ContractMatch {
            name: list[best.index].name.clone(),
            by_pattern: best.by_pattern,
            title_share: best.title_share,
            others: m[1..].iter().map(|x| list[x.index].name.clone()).collect(),
            ties: m[1..]
                .iter()
                .filter(|x| score(x) == score(best))
                .map(|x| list[x.index].name.clone())
                .collect(),
        }))
    }

    /// How tab `id`'s columns pair with the contract now.
    fn pairing_of(d: &Document, c: &Contract, keep_first: bool) -> Pairing {
        if keep_first {
            pair(c, &d.rule_titles(true))
        } else {
            pair_in_order(c, d.col_count())
        }
    }

    /// Check tab `id` against a contract: a count that a newer check, Cancel or any change stops
    /// (it never waits for a change).
    pub fn contract_drift(
        &self,
        folder: &Path,
        id: u32,
        name: &str,
        opts: &DriftOptions,
    ) -> CmdResult<DriftReport> {
        let c = self.contract_get(folder, name)?;
        let entry = self.get(id)?;
        let progress = Arc::new(Progress::new());
        {
            let mut slot = entry
                .contract_count
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(old) = slot.replace(progress.clone()) {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
        let d = match entry.doc.try_read() {
            Ok(d) => d,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(CmdError::new("busy", "the file is being changed"))
            }
        };
        if entry.change_waiting() {
            drop(d);
            return Err(CmdError::new("busy", "the file is being changed"));
        }
        let p = Self::pairing_of(&d, &c, opts.keep_first);
        let r = d.drift(&c, &p, opts, &progress);
        drop(d);
        let mut slot = entry
            .contract_count
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &progress)) {
            *slot = None;
        }
        Ok(r?)
    }

    /// Stop a contract check (Cancel, or the banner closed).
    pub fn contract_drift_stop(&self, id: u32) -> CmdResult<()> {
        let entry = self.get(id)?;
        if let Some(p) = entry
            .contract_count
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            p.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Show the rows behind one count of a drift report (a filter chip).
    pub fn contract_show_rows(
        &self,
        folder: &Path,
        id: u32,
        name: &str,
        what: DriftRows,
        opts: DriftOptions,
    ) -> CmdResult<OpReport> {
        let contract = Box::new(self.contract_get(folder, name)?);
        self.data_op(
            id,
            DataOp::ShowContractRows {
                contract,
                what,
                opts,
            },
        )
    }

    /// Fix what I can: titles and column order as one undo step. The types to set and the
    /// contract's checks (fitted to the fixed table) are returned for the window, which joins them
    /// to the same step.
    pub fn contract_fix(
        &self,
        folder: &Path,
        id: u32,
        name: &str,
        keep_first: bool,
    ) -> CmdResult<ContractFixed> {
        let c = self.contract_get(folder, name)?;
        let mut fix = None;
        let info = self.recorded(
            id,
            Logged::LeftOut("fixing columns to a file contract".into()),
            || {
                self.edit(id, |d| {
                    let p = Self::pairing_of(d, &c, keep_first);
                    fix = Some(d.contract_fix(&c, &p, keep_first)?);
                    Ok(())
                })
            },
        )?;
        let entry = self.get(id)?;
        let checks = keep_first.then(|| c.checks_for(&entry.read())).flatten();
        Ok(ContractFixed {
            info,
            fix: fix.unwrap_or_default(),
            checks,
        })
    }

    /// A contract's draft from tab `id` (a job: progress and Cancel). Refused for a private tab.
    pub fn contract_draft(&self, id: u32, spec: &DraftSpec) -> CmdResult<Draft> {
        self.refuse_private(id)?;
        natendo_engine::validate::check_spec(&CheckSpec {
            checks: spec.checks.clone(),
            table_checks: spec.table_checks.clone(),
            ..Default::default()
        })?;
        let entry = self.get(id)?;
        let job = entry.start("Reading the columns")?;
        let d = entry.read();
        Ok(d.contract_draft(spec, &job.progress)?)
    }

    /// Save a contract into the folder (`was`: the name it had, when it is being changed or
    /// renamed; `from_tab`: the tab its contents came from, refused when private).
    pub fn contract_save(
        &self,
        folder: &Path,
        c: &Contract,
        was: Option<&str>,
        from_tab: Option<u32>,
    ) -> CmdResult<Vec<ContractItem>> {
        if let Some(t) = from_tab {
            self.refuse_private(t)?;
        }
        check_name(&c.name)?;
        c.validate()?;
        {
            let g = self.contract_store(folder);
            let store = g.as_ref().expect("read above");
            let others = store.names().filter(|n| Some(*n) != was);
            if name_taken(others, &c.name) {
                return Err(CmdError::new(
                    "exists",
                    "a contract of that name already exists",
                ));
            }
            if was.is_none() && store.items.len() >= MAX_CONTRACTS {
                return Err(CmdError::new(
                    "too_large",
                    "the contracts folder is full (1,000 contracts)",
                ));
            }
        }
        match was.filter(|o| *o != c.name) {
            // Capitals only: rename the file first (on Windows writing over it would keep the old
            // capitals; on a case-sensitive disk it would leave both), then write it.
            Some(old) if old.eq_ignore_ascii_case(&c.name) => {
                check_name(old)?;
                std::fs::rename(
                    folder.join(contract::file_name(old)),
                    folder.join(contract::file_name(&c.name)),
                )
                .map_err(|_| io("the contract could not be renamed"))?;
                write_contract(folder, c)?;
            }
            Some(old) => {
                check_name(old)?;
                write_contract(folder, c)?;
                let _ = std::fs::remove_file(folder.join(contract::file_name(old)));
            }
            None => write_contract(folder, c)?,
        }
        Ok(self.contracts_reload(folder))
    }

    /// Delete a contract (by name; a broken one by its file name).
    pub fn contract_delete(&self, folder: &Path, name: &str) -> CmdResult<Vec<ContractItem>> {
        let file = {
            let g = self.contract_store(folder);
            let store = g.as_ref().expect("read above");
            store
                .items
                .iter()
                .find(|s| match &s.contract {
                    Ok(c) => c.name == name,
                    Err(_) => s.file == name,
                })
                .map(|s| s.file.clone())
                .ok_or_else(|| {
                    CmdError::new("not_found", "that contract is not in the list any more")
                })?
        };
        // Only a file the folder listing found, by its own name.
        if file.contains(['/', '\\']) || file.starts_with('.') {
            return Err(CmdError::new("bad_request", "that is not a contract file"));
        }
        std::fs::remove_file(folder.join(&file))
            .map_err(|_| io("the contract could not be deleted"))?;
        Ok(self.contracts_reload(folder))
    }

    /// Write a contract to the file an Export dialog picked.
    pub fn contract_export(&self, folder: &Path, name: &str, dest: &Path) -> CmdResult<()> {
        let c = self.contract_get(folder, name)?;
        std::fs::write(dest, c.to_bytes()?).map_err(|_| io("the contract could not be written"))
    }

    /// Bring in the contract file an Import dialog picked (checked in full first).
    pub fn contract_import(&self, folder: &Path, src: &Path) -> CmdResult<Vec<ContractItem>> {
        let len = std::fs::metadata(src)
            .map_err(|_| io("the file could not be read"))?
            .len();
        if len > MAX_FILE_BYTES as u64 {
            return Err(CmdError::new(
                "too_large",
                "the contract file is too large (over 4 MB)",
            ));
        }
        let bytes = std::fs::read(src).map_err(|_| io("the file could not be read"))?;
        let c = Contract::from_bytes(&bytes)?;
        self.contract_save(folder, &c, None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use natendo_engine::contract::RowRange;

    fn folder(name: &str) -> PathBuf {
        let dir = crate::testdir::path(&format!("nde-contracts-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Contracts")).unwrap();
        dir
    }

    fn open(dir: &Path, name: &str, body: &str) -> (Docs, u32) {
        let docs = Docs::default();
        let id = open_more(&docs, dir, name, body);
        (docs, id)
    }

    fn open_more(docs: &Docs, dir: &Path, name: &str, body: &str) -> u32 {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        let id = docs.open_path(&p, false).unwrap().id;
        for _ in 0..1000 {
            if docs.info(id).unwrap().indexed {
                return id;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("indexing did not finish");
    }

    const JAN: &str = "id,name,party\r\n1,Ann,DEM\r\n2,Bo,REP\r\n";

    fn draft(docs: &Docs, id: u32, name: &str) -> Contract {
        docs.contract_draft(
            id,
            &DraftSpec {
                name: name.into(),
                keep_first: true,
                allowed_cols: vec![2],
                ..Default::default()
            },
        )
        .unwrap()
        .contract
    }

    fn opts() -> DriftOptions {
        DriftOptions {
            keep_first: true,
            ..Default::default()
        }
    }

    #[test]
    fn with_no_contracts_matching_reads_nothing() {
        let dir = folder("none");
        let store = dir.join("Contracts");
        let (docs, id) = open(&dir, "voters_jan.csv", JAN);
        let before = MATCH_READS.load(Ordering::Relaxed);
        for _ in 0..50 {
            assert!(docs.contract_match(&store, id, true).unwrap().is_none());
        }
        // Other tests may match at the same time, but never 50 times.
        assert!(MATCH_READS.load(Ordering::Relaxed) - before < 50);
    }

    #[test]
    fn make_save_match_drift_fix_and_undo() {
        let dir = folder("flow");
        let store = dir.join("Contracts");
        let (docs, jan) = open(&dir, "voters_jan.csv", JAN);
        let mut c = draft(&docs, jan, "Monthly voters");
        c.recognise.pattern = Some("voters_*.csv".into());
        c.rows = Some(RowRange { min: 2, max: 2 });
        let list = docs.contract_save(&store, &c, None, Some(jan)).unwrap();
        assert_eq!(list.len(), 1);
        assert!(store.join("Monthly voters.ndcontract").is_file());
        // The next month: a new first column, name renamed, a new party, one more row.
        let feb = open_more(
            &docs,
            &dir,
            "voters_feb.csv",
            "new,id,full_name,party\r\nx,1,Ann,DEM\r\ny,2,Bo,GRN\r\nz,3,Cy,REP\r\n",
        );
        let m = docs.contract_match(&store, feb, true).unwrap().unwrap();
        assert_eq!(m.name, "Monthly voters");
        assert!(m.by_pattern);
        let rep = docs
            .contract_drift(&store, feb, "Monthly voters", &opts())
            .unwrap();
        assert!(rep.drifted);
        assert!(rep.rows_out);
        assert_eq!(rep.added, ["new"]);
        assert_eq!(rep.columns[2].first_new, ["GRN"]);
        let fixed = docs
            .contract_fix(&store, feb, "Monthly voters", true)
            .unwrap();
        assert!(fixed.fix.moved);
        assert_eq!(
            docs.first_row_names(feb, 10, 100).unwrap(),
            ["id", "name", "party", "new"]
        );
        docs.undo(feb).unwrap();
        assert_eq!(
            docs.first_row_names(feb, 10, 100).unwrap(),
            ["new", "id", "full_name", "party"]
        );
        // Show these rows makes a filter chip.
        docs.contract_show_rows(
            &store,
            feb,
            "Monthly voters",
            DriftRows::NewValues { col: 2 },
            opts(),
        )
        .unwrap();
        let chips = docs.filter_chips(feb).unwrap();
        assert_eq!(chips.chips[0].label, "New values in party");
        assert_eq!(chips.shown, 1);
    }

    #[test]
    fn a_private_tab_never_makes_or_changes_a_contract() {
        let dir = folder("private");
        let store = dir.join("Contracts");
        let (docs, id) = open(&dir, "voters_jan.csv", JAN);
        let c = draft(&docs, id, "Monthly voters");
        docs.contract_save(&store, &c, None, Some(id)).unwrap();
        docs.set_private(id, true).unwrap();
        let err = docs
            .contract_draft(
                id,
                &DraftSpec {
                    name: "Other".into(),
                    keep_first: true,
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(err.message, PRIVATE_WHY);
        assert!(docs
            .contract_save(&store, &c, Some("Monthly voters"), Some(id))
            .is_err());
        // Never matched by itself; still checked by hand, writing nothing.
        assert!(docs.contract_match(&store, id, true).unwrap().is_none());
        let before = std::fs::read(store.join("Monthly voters.ndcontract")).unwrap();
        docs.contract_drift(&store, id, "Monthly voters", &opts())
            .unwrap();
        assert_eq!(
            std::fs::read(store.join("Monthly voters.ndcontract")).unwrap(),
            before
        );
    }

    #[test]
    fn names_stay_inside_the_folder() {
        let dir = folder("names");
        let store = dir.join("Contracts");
        let (docs, id) = open(&dir, "voters_jan.csv", JAN);
        let mut c = draft(&docs, id, "Monthly voters");
        docs.contract_save(&store, &c, None, Some(id)).unwrap();
        for bad in ["..\\escape", "../escape", "C:evil", "CON", ".."] {
            c.name = bad.into();
            assert!(
                docs.contract_save(&store, &c, None, Some(id)).is_err(),
                "{bad}"
            );
        }
        assert!(!dir.join("escape.ndcontract").exists());
        c.name = "MONTHLY VOTERS".into();
        let err = docs.contract_save(&store, &c, None, Some(id)).unwrap_err();
        assert!(err.message.contains("already exists"), "{}", err.message);
        // Renaming keeps one file.
        let list = docs
            .contract_save(&store, &c, Some("Monthly voters"), Some(id))
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "MONTHLY VOTERS");
        // One file, with the new capitals in its name on disk (every disk).
        let on_disk: Vec<String> = std::fs::read_dir(&store)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(on_disk, [contract::file_name("MONTHLY VOTERS")]);
        // A broken file is listed with its reason; the others still load.
        std::fs::write(store.join("broken.ndcontract"), b"{oops").unwrap();
        let list = docs.contracts_list(&store).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|i| i.problem.is_some()));
        assert!(list.iter().any(|i| i.problem.is_none()));
        // Export, delete, import.
        let out = dir.join("exported.ndcontract");
        docs.contract_export(&store, "MONTHLY VOTERS", &out)
            .unwrap();
        docs.contract_delete(&store, "MONTHLY VOTERS").unwrap();
        docs.contract_delete(&store, "broken.ndcontract").unwrap();
        assert!(docs.contracts_list(&store).unwrap().is_empty());
        let list = docs.contract_import(&store, &out).unwrap();
        assert_eq!(list[0].name, "MONTHLY VOTERS");
        assert!(docs.contract_import(&store, &out).is_err(), "a second copy");
    }

    #[test]
    fn an_edit_stops_a_contract_check() {
        let dir = folder("cancel");
        let (docs, id) = open(&dir, "voters_jan.csv", JAN);
        let p = Arc::new(Progress::new());
        *docs.get(id).unwrap().contract_count.lock().unwrap() = Some(p.clone());
        docs.set_first_row_cell(id, 0, "ID".into()).unwrap();
        assert!(p.cancel.load(Ordering::Relaxed));
        let p2 = Arc::new(Progress::new());
        *docs.get(id).unwrap().contract_count.lock().unwrap() = Some(p2.clone());
        docs.contract_drift_stop(id).unwrap();
        assert!(p2.cancel.load(Ordering::Relaxed));
    }
}
