//! SQLite databases (Milestone 15): open a table of a `.sqlite`/`.db` file
//! as a tab, and save a tab as a table in a database file (created when
//! missing). Uses the SQLite library itself (bundled).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::Cols;
use crate::table::Sink;
use crate::Error;

/// File extensions treated as SQLite databases.
pub const EXTENSIONS: &[&str] = &["sqlite", "sqlite3", "db", "db3"];

/// Time allowed for counting the rows of all tables when listing them;
/// tables past it are listed without a count.
const COUNT_TIME: Duration = Duration::from_secs(5);
/// Reading stops when no row came for this long (a view that computes
/// forever before its first row, say).
const STALL_TIME: Duration = Duration::from_secs(60);
/// A view is read for at most this long (a view can be endless).
const VIEW_TIME: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbTable {
    pub name: String,
    /// Row count; `None` for a view (never counted: a view can be
    /// endless) or a table not counted in time.
    pub rows: Option<u64>,
    pub view: bool,
}

fn db_err(e: rusqlite::Error) -> Error {
    if let rusqlite::Error::SqliteFailure(f, _) = &e {
        if f.code == rusqlite::ErrorCode::OperationInterrupted {
            return Error::Invalid(
                "the database took too long to answer, so reading it was stopped".into(),
            );
        }
    }
    Error::Invalid(format!("the database: {e}"))
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn open_read(path: &Path) -> Result<Connection, Error> {
    let c = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_err)?;
    // No extensions, no trusted schema (a crafted file cannot call
    // functions with side effects from its views or triggers).
    let _ = c.execute_batch("PRAGMA trusted_schema = OFF;");
    Ok(c)
}

/// Run `f` on `c` while a watcher interrupts SQLite when `stop()` says so
/// (checked every 50 ms). SQLite then fails with "interrupted".
fn guarded<T>(c: &Connection, stop: impl Fn() -> bool + Sync, f: impl FnOnce() -> T) -> T {
    let handle = c.get_interrupt_handle();
    let done = AtomicBool::new(false);
    std::thread::scope(|sc| {
        sc.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                if stop() {
                    handle.interrupt();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let out = f();
        done.store(true, Ordering::Relaxed);
        out
    })
}

/// The tables and views of a database file. Tables are counted within a
/// few seconds overall; views are never run here.
pub fn tables(path: &Path) -> Result<Vec<DbTable>, Error> {
    let c = open_read(path)?;
    let deadline = Instant::now() + COUNT_TIME;
    let found: Vec<(String, bool)> = guarded(&c, || Instant::now() > deadline, || {
        let mut st = c.prepare(
            "SELECT name, type = 'view' FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 10000",
        )?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<(String, bool)>, _>>();
        v
    })
    .map_err(db_err)?;
    let mut out = Vec::with_capacity(found.len());
    for (name, view) in found {
        let rows = if view || Instant::now() > deadline {
            None
        } else {
            guarded(
                &c,
                || Instant::now() > deadline,
                || {
                    c.query_row(&format!("SELECT COUNT(*) FROM {}", quote(&name)), [], |r| {
                        r.get::<_, i64>(0)
                    })
                },
            )
            .ok()
            .map(|n| n.max(0) as u64)
        };
        out.push(DbTable { name, rows, view });
    }
    Ok(out)
}

/// A table (or view) of a database file as a document (first row: the
/// column names). Numbers are written as SQLite gives them; binary values
/// as a note of their size. Stops when cancelled, when no row comes for a
/// minute, when a view runs past five minutes, or at the table size limits.
pub fn read_table(path: &Path, table: &str, progress: &Progress) -> Result<Document, Error> {
    read_table_report(path, table, progress).map(|r| r.0)
}

/// [`read_table`], and the numbers of the columns declared NUMERIC/DECIMAL that hold binary (REAL) numbers.
/// SQLite keeps such a value as a binary number, so a written `1.50` is `1.5` inside the database already;
/// it is shown with the 15 significant digits SQLite itself gives. Text values come through exactly.
pub fn read_table_report(
    path: &Path,
    table: &str,
    progress: &Progress,
) -> Result<(Document, Vec<u32>), Error> {
    let known = tables(path)?;
    let entry = known
        .iter()
        .find(|t| t.name == table)
        .ok_or_else(|| Error::Invalid("that table is not in the database".into()))?;
    let (total, view) = (entry.rows.unwrap_or(0), entry.view);
    let c = open_read(path)?;
    let started = Instant::now();
    // Milliseconds since `started` when the last row came.
    let last_row = AtomicU64::new(0);
    let stop = || {
        let now = started.elapsed();
        progress.cancel.load(Ordering::Relaxed)
            || now.saturating_sub(Duration::from_millis(last_row.load(Ordering::Relaxed)))
                > STALL_TIME
            || (view && now > VIEW_TIME)
    };
    let decimal_cols = std::cell::RefCell::new(Vec::<u32>::new());
    let result = guarded(&c, stop, || -> Result<Document, Error> {
        let mut st = c
            .prepare(&format!("SELECT * FROM {}", quote(table)))
            .map_err(db_err)?;
        let names: Vec<String> = st.column_names().map(|s| s.to_string()).collect();
        let n = names.len();
        // Declared types by column name (a view's columns have none).
        let mut decl: HashMap<String, String> = HashMap::new();
        if let Ok(mut info) = c.prepare(&format!("PRAGMA table_info({})", quote(table))) {
            if let Ok(rs) =
                info.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            {
                for (name, ty) in rs.flatten() {
                    decl.insert(name, ty.to_ascii_uppercase());
                }
            }
        }
        let decimal: Vec<bool> = names
            .iter()
            .map(|n| {
                decl.get(n)
                    .is_some_and(|d| d.contains("DECIMAL") || d.contains("NUMERIC"))
            })
            .collect();
        let mut sink = Sink::new(None);
        sink.row(&names)?;
        progress.start(total.max(1));
        let mut rows = st.query([]).map_err(db_err)?;
        let mut cells: Vec<String> = Vec::with_capacity(n);
        let mut k = 0u64;
        while let Some(r) = rows.next().map_err(db_err)? {
            cells.clear();
            for i in 0..n {
                let v = match r.get_ref(i).map_err(db_err)? {
                    ValueRef::Null => String::new(),
                    ValueRef::Integer(x) => x.to_string(),
                    ValueRef::Real(x) if decimal.get(i).copied().unwrap_or(false) => {
                        let mut seen = decimal_cols.borrow_mut();
                        if !seen.contains(&(i as u32)) {
                            seen.push(i as u32);
                        }
                        let s = x.to_string();
                        // The shortest text of up to 15 digits is what SQLite shows too.
                        if s.bytes().filter(u8::is_ascii_digit).count() <= 15 {
                            s
                        } else {
                            format!("{:.14e}", x)
                                .parse::<f64>()
                                .map_or(s, |y| y.to_string())
                        }
                    }
                    ValueRef::Real(x) => x.to_string(),
                    ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                    ValueRef::Blob(b) => format!("(binary, {} bytes)", b.len()),
                };
                cells.push(v);
            }
            sink.row(&cells)?;
            k += 1;
            if k % 4_096 == 0 {
                last_row.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
                progress.step(4_096)?;
            }
        }
        sink.finish().into_document()
    });
    match result {
        Err(_) if progress.cancel.load(Ordering::Relaxed) => Err(Error::Cancelled),
        r => r.map(|d| {
            let mut cols = decimal_cols.into_inner();
            cols.sort_unstable();
            (d, cols)
        }),
    }
}

impl Document {
    /// Save the shown rows as table `table` of database `path` (created
    /// when missing). An existing table is replaced only with `replace`.
    /// Columns are named from the title row (`header`) and stored as text;
    /// empty cells as NULL. All in one transaction.
    pub fn save_sqlite(
        &self,
        path: &Path,
        table: &str,
        header: bool,
        replace: bool,
        progress: &Progress,
    ) -> Result<u64, Error> {
        self.ready_for_bulk()?;
        let table = table.trim();
        if table.is_empty()
            || table.len() > 128
            || table.to_ascii_lowercase().starts_with("sqlite_")
        {
            return Err(Error::Invalid(
                "choose a table name (not starting with sqlite_)".into(),
            ));
        }
        let ncols = self.col_count();
        let titles = self.titles(&(0..ncols).collect::<Vec<_>>(), header);
        let mut seen = std::collections::HashSet::new();
        let cols: Vec<String> = titles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let base = if t.trim().is_empty() {
                    format!("column{}", i + 1)
                } else {
                    t.trim().to_string()
                };
                let mut name = base.clone();
                let mut k = 2;
                while !seen.insert(name.to_lowercase()) {
                    name = format!("{base}_{k}");
                    k += 1;
                }
                name
            })
            .collect();
        let mut c = Connection::open(path).map_err(db_err)?;
        let tx = c.transaction().map_err(db_err)?;
        let exists: bool = tx
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
                [table],
                |r| r.get::<_, i64>(0),
            )
            .map_err(db_err)?
            > 0;
        if exists && !replace {
            return Err(Error::Invalid(format!(
                "the database already has a table called {table}; choose another name or replace it"
            )));
        }
        if exists {
            tx.execute(&format!("DROP TABLE {}", quote(table)), [])
                .map_err(db_err)?;
        }
        let defs: Vec<String> = cols.iter().map(|c| format!("{} TEXT", quote(c))).collect();
        tx.execute(
            &format!("CREATE TABLE {} ({})", quote(table), defs.join(", ")),
            [],
        )
        .map_err(db_err)?;
        let marks: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
        let reader = Cols::new(self, &(0..ncols).collect::<Vec<_>>());
        let first = u64::from(header && self.row_count() > 0);
        let end = self.row_count();
        progress.start(end - first);
        let mut written = 0u64;
        {
            let mut ins = tx
                .prepare(&format!(
                    "INSERT INTO {} VALUES ({})",
                    quote(table),
                    marks.join(", ")
                ))
                .map_err(db_err)?;
            let mut buf = Vec::new();
            for r in first..end {
                let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                reader.read(self, id, &mut buf);
                let vals: Vec<Option<&str>> = (0..ncols as usize)
                    .map(|k| Some(reader.get(&buf, k)).filter(|v| !v.is_empty()))
                    .collect();
                ins.execute(rusqlite::params_from_iter(vals.iter()))
                    .map_err(db_err)?;
                written += 1;
                if written % 65_536 == 0 {
                    progress.step(65_536)?;
                }
            }
        }
        tx.commit().map_err(db_err)?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    /// A1a: a TEXT value comes through exactly; a REAL in a NUMERIC/DECIMAL column is shown with the 15
    /// digits SQLite itself gives, and the column is reported (the binary number is all the file holds).
    #[test]
    fn text_is_exact_and_decimal_columns_are_reported() {
        let path = std::env::temp_dir().join(format!("nd-a1a-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE t(id TEXT, price DECIMAL(10,2), n INTEGER, r REAL);
                 INSERT INTO t VALUES('00123456789012345678', 0.30000000000000004, 12, 0.30000000000000004);
                 INSERT INTO t VALUES('1.50', 1.50, 3, 1.5);",
            )
            .unwrap();
        }
        let (d, dec) = read_table_report(&path, "t", &Progress::new()).unwrap();
        assert_eq!(
            d.row_values(1).unwrap(),
            vec!["00123456789012345678", "0.3", "12", "0.30000000000000004"]
        );
        assert_eq!(d.row_values(2).unwrap(), vec!["1.50", "1.5", "3", "1.5"]);
        assert_eq!(dec, vec![1]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_and_open_tables() {
        let d = Document::from_bytes(
            b"id,name,name\r\n1,Ann,x\r\n2,,y\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!("nd-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = Progress::new();
        assert_eq!(d.save_sqlite(&path, "people", true, false, &p).unwrap(), 2);
        assert!(
            d.save_sqlite(&path, "people", true, false, &p).is_err(),
            "no silent replace"
        );
        d.save_sqlite(&path, "people", true, true, &p).unwrap();
        let t = tables(&path).unwrap();
        assert_eq!(
            t,
            vec![DbTable {
                name: "people".into(),
                rows: Some(2),
                view: false,
            }]
        );
        let back = read_table(&path, "people", &p).unwrap();
        assert_eq!(back.row_values(0).unwrap(), vec!["id", "name", "name_2"]);
        assert_eq!(back.row_values(2).unwrap(), vec!["2", "", "y"]);
        // Empty cells were stored as NULL.
        let c = Connection::open(&path).unwrap();
        let nulls: i64 = c
            .query_row("SELECT COUNT(*) FROM people WHERE name IS NULL", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(nulls, 1);
        drop(c);
        assert!(read_table(&path, "nope", &p).is_err());
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod endless {
    use super::*;

    /// A crafted database whose view never ends: listing returns at once,
    /// and reading it stops when cancelled.
    #[test]
    fn endless_view_is_listed_and_can_be_stopped() {
        let path = std::env::temp_dir().join(format!("nd-endless-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE VIEW forever AS WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n) SELECT x FROM n;
                 CREATE VIEW slow AS WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n) SELECT max(x) FROM n;",
            )
            .unwrap();
        }
        let t0 = Instant::now();
        let t = tables(&path).unwrap();
        assert!(t0.elapsed() < Duration::from_secs(2));
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|x| x.view && x.rows.is_none()));
        // Cancel after a moment: both the endless rows and the endless
        // first row stop.
        for name in ["forever", "slow"] {
            let p = Progress::new();
            let t0 = Instant::now();
            let r = std::thread::scope(|sc| {
                sc.spawn(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    p.cancel.store(true, Ordering::Relaxed);
                });
                read_table(&path, name, &p)
            });
            assert!(matches!(r, Err(Error::Cancelled)), "{name}");
            assert!(t0.elapsed() < Duration::from_secs(3), "{name}");
        }
        let _ = std::fs::remove_file(&path);
    }
}
