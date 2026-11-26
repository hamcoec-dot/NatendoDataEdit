//! SQLite databases (Milestone 15): open a table of a `.sqlite`/`.db` file
//! as a tab, and save a tab as a table in a database file (created when
//! missing). Also reads the attribute columns of a GeoPackage (`.gpkg`).
//! Reading never creates or changes a file beside the database. Uses the
//! SQLite library itself (bundled).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::Cols;
use crate::table::Sink;
use crate::Error;

/// File extensions treated as SQLite databases.
pub const EXTENSIONS: &[&str] = &["sqlite", "sqlite3", "db", "db3", "gpkg"];
/// The extensions opened as databases from Open, drop and double-click (`.db`
/// and `.db3` are used by too many other things).
pub const OPENED_EXTENSIONS: &[&str] = &["gpkg", "sqlite", "sqlite3"];

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
    /// In a GeoPackage: "features" or "attributes" (the layers), "tiles",
    /// "system" or "other" (shown only on request). Empty in other databases.
    #[serde(default)]
    pub kind: String,
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

/// Size and time of the database and its `-wal`, to notice a writer.
type Stamp = [Option<(u64, Option<SystemTime>)>; 2];

/// A database open for reading.
struct Reading {
    c: Connection,
    /// Set when SQLite was told the file cannot change (no locks are taken),
    /// so the app checks that it did not.
    unlocked: Option<(PathBuf, Stamp)>,
}

impl Reading {
    /// Fail when a file read without locks changed meanwhile: what was read
    /// may be torn.
    fn unchanged(&self) -> Result<(), Error> {
        match &self.unlocked {
            Some((path, before))
                if stamp(path) != *before || stamp(path)[1].is_some_and(|w| w.0 > 0) =>
            {
                Err(Error::Invalid(
                    "the database changed while it was being read: close it in the program writing it (your GIS program) and try again"
                        .into(),
                ))
            }
            _ => Ok(()),
        }
    }
}

/// `path` with `suffix` added to the file name (`a.gpkg-wal`).
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

fn stamp(path: &Path) -> Stamp {
    [path.to_path_buf(), beside(path, "-wal")].map(|p| {
        let m = std::fs::metadata(p).ok()?;
        Some((m.len(), m.modified().ok()))
    })
}

/// The `file:` address of `path`, saying the file will not change.
fn immutable_uri(path: &Path) -> Result<String, Error> {
    let full = std::path::absolute(path)?;
    let text = full
        .to_str()
        .ok_or_else(|| Error::Invalid("the database's name cannot be used".into()))?;
    let text = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!("//{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(text).to_string()
    };
    let text = if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    };
    // `C:/x` becomes `file:///C:/x`; a share `//server/x` becomes
    // `file:////server/x`.
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/:".contains(&b) {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    Ok(uri)
}

/// Open a database for reading without ever creating or changing a file. A
/// database in write-ahead mode would make SQLite create files beside it, so
/// it is read as it stands, and only when its `-wal` holds nothing.
fn open_read(path: &Path) -> Result<Reading, Error> {
    let mut head = Vec::new();
    {
        use std::io::Read;
        std::fs::File::open(path)?
            .take(100)
            .read_to_end(&mut head)?;
    }
    let filled = |suffix: &str| std::fs::metadata(beside(path, suffix)).is_ok_and(|m| m.len() > 0);
    let wal = head.starts_with(b"SQLite format 3\0")
        && head.len() >= 20
        && (head[18] == 2 || head[19] == 2);
    let busy = |what: &str| {
        Error::Invalid(format!(
            "this database has unsaved changes in its {what} file. Close it in your GIS or database program, or copy the database and the files beside it together and open the copy"
        ))
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let (c, unlocked) = if wal {
        if filled("-wal") {
            return Err(busy("-wal"));
        }
        let before = stamp(path);
        let c =
            Connection::open_with_flags(immutable_uri(path)?, flags | OpenFlags::SQLITE_OPEN_URI)
                .map_err(db_err)?;
        (c, Some((path.to_path_buf(), before)))
    } else {
        // A journal with a live header would have to be played back, which
        // writes to the database.
        let mut magic = [0u8; 8];
        let hot = std::fs::File::open(beside(path, "-journal"))
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic))
            .is_ok()
            && magic == [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7];
        if hot {
            return Err(busy("-journal"));
        }
        (
            Connection::open_with_flags(path, flags).map_err(db_err)?,
            None,
        )
    };
    // No extensions, no trusted schema (a crafted file cannot call
    // functions with side effects from its views or triggers).
    let _ = c.execute_batch("PRAGMA trusted_schema = OFF;");
    Ok(Reading { c, unlocked })
}

/// Is this database a GeoPackage?
fn is_geopackage(c: &Connection) -> bool {
    let id: i64 = c
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap_or(0);
    // "GPKG", and the older "GP10" and "GP11".
    [0x4750_4B47, 0x4750_3130, 0x4750_3131].contains(&id)
        || c.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'gpkg_contents'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .is_ok_and(|n| n > 0)
}

/// A GeoPackage's layers by lower-case table name: kind and, when stored,
/// the row count.
fn layers(c: &Connection) -> HashMap<String, (String, Option<u64>)> {
    let mut out = HashMap::new();
    let pairs = |sql: &str| -> Vec<(String, String)> {
        let Ok(mut st) = c.prepare(sql) else {
            return Vec::new();
        };
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    };
    for (name, kind) in pairs("SELECT table_name, data_type FROM gpkg_contents LIMIT 10000") {
        let kind = match kind.as_str() {
            "features" | "attributes" | "tiles" => kind,
            _ => "other".into(),
        };
        out.insert(name.to_lowercase(), (kind, None));
    }
    for (name, n) in pairs(
        "SELECT table_name, CAST(feature_count AS TEXT) FROM gpkg_ogr_contents WHERE feature_count >= 0 LIMIT 10000",
    ) {
        if let Some(e) = out.get_mut(&name.to_lowercase()) {
            e.1 = n.parse().ok();
        }
    }
    out
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
    let reading = open_read(path)?;
    let c = &reading.c;
    let deadline = Instant::now() + COUNT_TIME;
    let found: Vec<(String, bool)> = guarded(c, || Instant::now() > deadline, || {
        let mut st = c.prepare(
            "SELECT name, type = 'view' FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 10000",
        )?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<(String, bool)>, _>>();
        v
    })
    .map_err(db_err)?;
    let geo = is_geopackage(c).then(|| layers(c));
    let mut out = Vec::with_capacity(found.len());
    for (name, view) in found {
        let (kind, stored) = match &geo {
            None => (String::new(), None),
            Some(layers) => {
                let low = name.to_lowercase();
                if low.starts_with("gpkg_") || low.starts_with("rtree_") {
                    ("system".to_string(), None)
                } else {
                    layers
                        .get(&low)
                        .cloned()
                        .unwrap_or_else(|| ("other".to_string(), None))
                }
            }
        };
        let hidden = matches!(kind.as_str(), "system" | "tiles" | "other");
        let rows = if stored.is_some() {
            stored
        } else if view || hidden || Instant::now() > deadline {
            None
        } else {
            guarded(
                c,
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
        out.push(DbTable {
            name,
            rows,
            view,
            kind,
        });
    }
    reading.unchanged()?;
    Ok(out)
}

/// A table (or view) of a database file as a document (first row: the
/// column names). Numbers are written as SQLite gives them; binary values
/// as a note of their size. Stops when cancelled, when no row comes for a
/// minute, when a view runs past five minutes, or at the table size limits.
pub fn read_table(path: &Path, table: &str, progress: &Progress) -> Result<Document, Error> {
    read_table_report(path, table, progress).map(|r| r.doc)
}

/// What [`read_table_report`] returns.
pub struct Opened {
    pub doc: Document,
    /// The columns declared NUMERIC/DECIMAL that hold binary (REAL) numbers.
    pub decimal: Vec<u32>,
    /// The geometry columns left out, each described ("geom (POINT, EPSG:4326)").
    pub geometry: Vec<String>,
}

/// Column types that hold shapes.
const SHAPES: &[&str] = &[
    "GEOMETRY",
    "POINT",
    "LINESTRING",
    "POLYGON",
    "MULTIPOINT",
    "MULTILINESTRING",
    "MULTIPOLYGON",
    "GEOMETRYCOLLECTION",
    "GEOMCOLLECTION",
    "CIRCULARSTRING",
    "COMPOUNDCURVE",
    "CURVEPOLYGON",
    "MULTICURVE",
    "MULTISURFACE",
    "CURVE",
    "SURFACE",
];

/// The geometry columns of a GeoPackage table: lower-case name to description.
fn geometry_columns(c: &Connection, table: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let sql = "SELECT column_name, geometry_type_name, CAST(srs_id AS INTEGER) FROM gpkg_geometry_columns WHERE lower(table_name) = lower(?1)";
    let Ok(mut st) = c.prepare(sql) else {
        return out;
    };
    let Ok(rows) = st.query_map([table], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<i64>>(2)?,
        ))
    }) else {
        return out;
    };
    for (col, kind, id) in rows.flatten() {
        // The reference system's usual name ("EPSG:4326"), when the file
        // says it; a missing or broken list never shows the shapes.
        let srs = id.map(|id| {
            c.query_row(
                "SELECT organization || ':' || organization_coordsys_id FROM gpkg_spatial_ref_sys WHERE srs_id = ?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_else(|_| format!("SRS {id}"))
        });
        let about: Vec<String> = [kind, srs].into_iter().flatten().collect();
        out.insert(col.to_lowercase(), format!("{col} ({})", about.join(", ")));
    }
    out
}

/// The statement reading `table`: every column, except a GeoPackage's shapes,
/// which are never copied. Also the shapes left out.
fn select_of(c: &Connection, table: &str) -> Result<(String, Vec<String>), Error> {
    if !is_geopackage(c) {
        return Ok((format!("SELECT * FROM {}", quote(table)), Vec::new()));
    }
    let shapes = geometry_columns(c, table);
    let mut st = c
        .prepare(&format!("PRAGMA table_info({})", quote(table)))
        .map_err(view_err)?;
    let cols = st
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        .map_err(view_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(view_err)?;
    let mut keep = Vec::new();
    let mut left = Vec::new();
    for (name, ty) in cols {
        if let Some(d) = shapes.get(&name.to_lowercase()) {
            left.push(d.clone());
        } else if SHAPES.contains(&ty.trim().to_ascii_uppercase().as_str()) {
            left.push(format!("{name} ({})", ty.trim().to_ascii_uppercase()));
        } else {
            keep.push(quote(&name));
        }
    }
    if left.is_empty() {
        return Ok((format!("SELECT * FROM {}", quote(table)), left));
    }
    if keep.is_empty() {
        return Err(Error::Invalid(
            "this table holds only shapes, which are not shown".into(),
        ));
    }
    Ok((
        format!("SELECT {} FROM {}", keep.join(", "), quote(table)),
        left,
    ))
}

/// A view built on functions only a GIS program has says so plainly.
fn view_err(e: rusqlite::Error) -> Error {
    let text = e.to_string();
    if text.contains("no such function") || text.contains("unsafe use of") {
        Error::Invalid("this table is a view that needs a GIS program to read".into())
    } else {
        db_err(e)
    }
}

/// [`read_table`], and the numbers of the columns declared NUMERIC/DECIMAL that hold binary (REAL) numbers.
/// SQLite keeps such a value as a binary number, so a written `1.50` is `1.5` inside the database already;
/// it is shown with the 15 significant digits SQLite itself gives. Text values come through exactly.
pub fn read_table_report(path: &Path, table: &str, progress: &Progress) -> Result<Opened, Error> {
    read_checked(path, table, progress, &|| {})
}

/// `mid` runs once the rows are read, before the file is checked (tests).
fn read_checked(
    path: &Path,
    table: &str,
    progress: &Progress,
    mid: &dyn Fn(),
) -> Result<Opened, Error> {
    let known = tables(path)?;
    let entry = known
        .iter()
        .find(|t| t.name == table)
        .ok_or_else(|| Error::Invalid("that table is not in the database".into()))?;
    let (total, view) = (entry.rows.unwrap_or(0), entry.view);
    let reading = open_read(path)?;
    let c = &reading.c;
    let (select, geometry) = select_of(c, table)?;
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
    let result = guarded(c, stop, || -> Result<Document, Error> {
        let mut st = c.prepare(&select).map_err(view_err)?;
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
        while let Some(r) = rows.next().map_err(view_err)? {
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
    mid();
    if progress.cancel.load(Ordering::Relaxed) && result.is_err() {
        return Err(Error::Cancelled);
    }
    reading.unchanged()?;
    result.map(|doc| {
        let mut decimal = decimal_cols.into_inner();
        decimal.sort_unstable();
        Opened {
            doc,
            decimal,
            geometry,
        }
    })
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
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"))
        {
            return Err(Error::Invalid(
                "a GeoPackage is only read, never written: save to a .sqlite file".into(),
            ));
        }
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
        let r = read_table_report(&path, "t", &Progress::new()).unwrap();
        let (d, dec) = (r.doc, r.decimal);
        assert_eq!(
            d.row_values(1).unwrap(),
            vec!["00123456789012345678", "0.3", "12", "0.30000000000000004"]
        );
        assert_eq!(d.row_values(2).unwrap(), vec!["1.50", "1.5", "3", "1.5"]);
        assert_eq!(dec, vec![1]);
        let _ = std::fs::remove_file(&path);
    }

    /// A database read without locks that changes meanwhile is refused.
    #[test]
    fn a_writer_during_an_unlocked_read_is_noticed() {
        let path = std::env::temp_dir().join(format!("nd-mid-{}.sqlite", std::process::id()));
        let wal = beside(&path, "-wal");
        let make = || {
            for p in [&path, &wal, &beside(&path, "-shm")] {
                let _ = std::fs::remove_file(p);
            }
            let c = Connection::open(&path).unwrap();
            c.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
                .unwrap();
            c.execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES(1);")
                .unwrap();
        };
        let p = Progress::new();
        make();
        assert!(read_checked(&path, "t", &p, &|| {}).is_ok());
        // The database file grows.
        let r = read_checked(&path, "t", &p, &|| {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            std::io::Write::write_all(&mut f, &[0; 4096]).unwrap();
        });
        assert!(r.err().unwrap().to_string().contains("changed while"));
        // A writer starts a -wal.
        make();
        let r = read_checked(&path, "t", &p, &|| std::fs::write(&wal, b"x").unwrap());
        assert!(r.err().unwrap().to_string().contains("changed while"));
        for p in [&path, &wal] {
            let _ = std::fs::remove_file(p);
        }
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
                kind: String::new(),
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
