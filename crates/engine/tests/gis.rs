//! Shapefile attribute tables (`.dbf`) and GeoPackages, on made-up files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use natendo_engine::dbf::{self, DbfOptions, Fid, Sidecars};
use natendo_engine::import::{import_report, ImportOptions};
use natendo_engine::sqlite;
use natendo_engine::Progress;
use proptest::prelude::*;
use rusqlite::Connection;

// ---- building .dbf files ----

struct Dbf {
    version: u8,
    language: u8,
    fields: Vec<(&'static str, u8, u8, u8, u8)>,
    records: Vec<(u8, Vec<Vec<u8>>)>,
}

impl Dbf {
    /// Fields as (name, type, length).
    fn new(fields: &[(&'static str, u8, u8)]) -> Dbf {
        Dbf {
            version: 0x03,
            language: 0,
            fields: fields.iter().map(|&(n, t, l)| (n, t, l, 0, 0)).collect(),
            records: Vec::new(),
        }
    }
    fn row(mut self, values: &[&[u8]]) -> Dbf {
        self.records
            .push((b' ', values.iter().map(|v| v.to_vec()).collect()));
        self
    }
    fn deleted(mut self, values: &[&[u8]]) -> Dbf {
        self.records
            .push((b'*', values.iter().map(|v| v.to_vec()).collect()));
        self
    }
    fn bytes(&self) -> Vec<u8> {
        let seven = matches!(self.version, 0x04 | 0x8C);
        let fox = (0x30..=0x32).contains(&self.version);
        let (first, size) = if seven { (68, 48) } else { (32, 32) };
        let start = first + size * self.fields.len() + 1 + if fox { 263 } else { 0 };
        let record = 1 + self.fields.iter().map(|f| usize::from(f.2)).sum::<usize>();
        let mut d = vec![0u8; first];
        d[0] = self.version;
        d[4..8].copy_from_slice(&(self.records.len() as u32).to_le_bytes());
        d[8..10].copy_from_slice(&(start as u16).to_le_bytes());
        d[10..12].copy_from_slice(&(record as u16).to_le_bytes());
        d[29] = self.language;
        for &(name, kind, len, high, flags) in &self.fields {
            let mut f = vec![0u8; size];
            f[..name.len()].copy_from_slice(name.as_bytes());
            if seven {
                f[32] = kind;
                f[33] = len;
            } else {
                f[11] = kind;
                f[16] = len;
                f[17] = high;
                f[18] = flags;
            }
            d.extend(f);
        }
        d.push(0x0D);
        d.resize(start, 0);
        for (mark, values) in &self.records {
            d.push(*mark);
            for (v, f) in values.iter().zip(&self.fields) {
                let mut cell = v.clone();
                cell.resize(usize::from(f.2), b' ');
                d.extend(cell);
            }
        }
        d.push(0x1A);
        d
    }
}

fn read_with(data: &[u8], side: Sidecars, opts: DbfOptions) -> (Vec<Vec<String>>, Vec<String>) {
    let o = ImportOptions {
        dbf: opts,
        beside: Some(Arc::new(side)),
        ..Default::default()
    };
    let (t, st) = import_report(data, Some("dbf"), &o, None, &Progress::new()).unwrap();
    let d = t.into_document().unwrap();
    let width = d.row_values(0).unwrap().len();
    let rows = (0..d.row_count())
        .map(|r| {
            let mut v = d.row_values(r).unwrap();
            v.resize(width, String::new());
            v
        })
        .collect();
    (rows, st.warnings)
}

fn read(data: &[u8]) -> (Vec<Vec<String>>, Vec<String>) {
    read_with(data, Sidecars::default(), DbfOptions::default())
}

fn no_fid() -> DbfOptions {
    DbfOptions {
        fid: Fid::Off,
        ..Default::default()
    }
}

#[test]
fn every_dbase_type_comes_through_as_written() {
    let data = Dbf::new(&[
        ("NAME", b'C', 8),
        ("ZIP", b'N', 7),
        ("VAL", b'F', 6),
        ("SEEN", b'D', 8),
        ("OK", b'L', 1),
    ])
    .row(&[b"  Elm", b"  00123", b"  1.50", b"20240229", b"y"])
    .row(&[b"", b"*******", b"", b"00000000", b"?"])
    .row(&[b"Oak", b"-5", b"", b"soon", b"F"])
    .bytes();
    let (rows, warnings) = read(&data);
    assert_eq!(
        rows,
        [
            vec!["FID", "NAME", "ZIP", "VAL", "SEEN", "OK"],
            vec!["0", "  Elm", "00123", "1.50", "2024-02-29", "TRUE"],
            vec!["1", "", "*******", "", "", ""],
            vec!["2", "Oak", "-5", "", "soon", "FALSE"],
        ]
    );
    assert_eq!(
        warnings,
        ["field ZIP: numbers too wide for the field, kept as **** (1 records)"]
    );
}

#[test]
fn fid_counts_deleted_records() {
    let data = Dbf::new(&[("A", b'C', 3)])
        .row(&[b"one"])
        .deleted(&[b"two"])
        .row(&[b"six"])
        .bytes();
    assert_eq!(
        read(&data).0,
        [vec!["FID", "A"], vec!["0", "one"], vec!["2", "six"]]
    );
    let info = dbf::inspect(&data, &Sidecars::default(), &DbfOptions::default()).unwrap();
    assert_eq!((info.records, info.deleted), (3, 1));
    let all = DbfOptions {
        fid: Fid::From1,
        show_deleted: true,
        ..Default::default()
    };
    assert_eq!(
        read_with(&data, Sidecars::default(), all).0,
        [
            vec!["FID", "Deleted", "A"],
            vec!["1", "FALSE", "one"],
            vec!["2", "TRUE", "two"],
            vec!["3", "FALSE", "six"],
        ]
    );
}

#[test]
fn blank_records_are_kept_without_a_number_column() {
    let data = Dbf::new(&[("A", b'C', 2), ("B", b'C', 2)])
        .row(&[b"x", b"y"])
        .row(&[b"", b""])
        .row(&[b"", b""])
        .bytes();
    assert_eq!(
        read_with(&data, Sidecars::default(), no_fid()).0,
        [vec!["A", "B"], vec!["x", "y"], vec!["", ""], vec!["", ""]]
    );
    // An empty table is its titles.
    assert_eq!(
        read(&Dbf::new(&[("A", b'C', 2)]).bytes()).0,
        [vec!["FID", "A"]]
    );
}

#[test]
fn encoding_rules_in_order() {
    // 0x82 is é in DOS 437 and a low quote in Windows 1252.
    let make = |language: u8| {
        let mut f = Dbf::new(&[("A", b'C', 4)]).row(&[b"\x82"]);
        f.language = language;
        f.bytes()
    };
    let cpg = |text: &str| Sidecars {
        cpg: Some(text.as_bytes().to_vec()),
        ..Default::default()
    };
    let info = |data: &[u8], side: &Sidecars, encoding: &str| {
        let o = DbfOptions {
            encoding: encoding.into(),
            ..Default::default()
        };
        dbf::inspect(data, side, &o).unwrap()
    };
    let none = Sidecars::default();
    let value =
        |data: &[u8], side: Sidecars| read_with(data, side, DbfOptions::default()).0[1][1].clone();

    // No hint at all: not UTF-8, so Windows 1252.
    assert_eq!(value(&make(0), Sidecars::default()), "\u{201a}");
    assert!(info(&make(0), &none, "").rule.starts_with("a guess"));
    // The table's own code.
    assert_eq!(value(&make(0x01), Sidecars::default()), "é");
    assert_eq!(value(&make(0x02), Sidecars::default()), "é");
    assert_eq!(value(&make(0x57), Sidecars::default()), "\u{201a}");
    assert_eq!(info(&make(0x01), &none, "").encoding, "CP437");
    // The .cpg file wins over it.
    assert_eq!(value(&make(0x01), cpg("1252\r\n")), "\u{201a}");
    assert_eq!(
        info(&make(0x01), &cpg("ANSI 1252"), "").rule,
        "the .cpg file"
    );
    for (text, name) in [
        ("UTF-8", "UTF-8"),
        ("65001", "UTF-8"),
        ("ISO-8859-1", "windows-1252"),
        ("88591", "windows-1252"),
        ("ISO 8859-2", "ISO-8859-2"),
        ("cp850", "CP850"),
        ("\u{feff}utf-8", "UTF-8"),
    ] {
        let side = cpg(text);
        let got = info(&make(0), &side, "");
        if text == "ISO 8859-2" && got.rule != "the .cpg file" {
            continue;
        }
        assert_eq!(
            (got.encoding.as_str(), got.rule.as_str()),
            (name, "the .cpg file"),
            "{text}"
        );
    }
    // Labels that would destroy ASCII text are passed over, and said.
    for text in [
        "iso-2022-kr",
        "UTF-16",
        "utf-16be",
        "iso-2022-jp",
        "x-user-defined",
        "nonsense",
        "",
    ] {
        let got = info(&make(0x01), &cpg(text), "");
        assert_eq!(got.rule, "the code page stored in the table", "{text}");
        assert!(got.warnings.iter().any(|w| w.contains(".cpg")), "{text}");
    }
    // The picker wins over everything.
    assert_eq!(
        info(&make(0x01), &cpg("utf-8"), "windows-1251").rule,
        "your choice"
    );
    let bad = DbfOptions {
        encoding: "utf-16".into(),
        ..Default::default()
    };
    assert!(dbf::inspect(&make(0), &none, &bad).is_err());
    // A code page with no reader is said, and a guess follows.
    let got = info(&make(0x64), &none, "");
    assert!(got.warnings.iter().any(|w| w.contains("852")));
    // Valid UTF-8 is guessed as such; a character cut off at the field's end
    // does not spoil the guess.
    let data = Dbf::new(&[("A", b'C', 3)])
        .row(&["é".as_bytes()])
        .row(&[b"ab\xC3"])
        .bytes();
    assert_eq!(info(&data, &none, "").encoding, "UTF-8");
}

#[test]
fn dos_code_pages_are_whole() {
    let high: Vec<u8> = (0x80..=0xFF).collect();
    for (encoding, sample) in [
        ("cp437", (0x9Cusize - 0x80, '£')),
        ("cp850", (0x9D - 0x80, 'Ø')),
    ] {
        let mut f = Dbf::new(&[("A", b'C', 128)]);
        f = f.row(&[&high]);
        let o = DbfOptions {
            encoding: encoding.into(),
            fid: Fid::Off,
            ..Default::default()
        };
        let text: Vec<char> = read_with(&f.bytes(), Sidecars::default(), o).0[1][0]
            .chars()
            .collect();
        assert_eq!(text.len(), 128, "{encoding}");
        assert!(!text.contains(&'\u{fffd}'));
        assert_eq!(text[sample.0], sample.1, "{encoding}");
        assert_eq!(text[0x82 - 0x80], 'é');
        assert_eq!(text[127], '\u{a0}');
    }
}

#[test]
fn memo_text_from_dbt() {
    let data = |version: u8| {
        let mut f = Dbf::new(&[("NOTE", b'M', 10)])
            .row(&[b"         1"])
            .row(&[b""])
            .row(&[b"        99"]);
        f.version = version;
        f.bytes()
    };
    // dBase III: 512-byte blocks ending in 0x1A.
    let mut three = vec![0u8; 512];
    three.extend_from_slice(b"made up memo\r\nsecond line\x1a\x1a");
    let side = Sidecars {
        dbt: Some(three),
        ..Default::default()
    };
    let (rows, warnings) = read_with(&data(0x83), side, no_fid());
    assert_eq!(rows[1][0], "made up memo\r\nsecond line");
    assert_eq!(rows[3][0], "");
    assert_eq!(
        warnings,
        ["field NOTE: memos missing or damaged were left blank (1 records)"]
    );
    // dBase IV: a block size in the header and a length on each memo.
    let four = |block: u16, len: u32| {
        let mut d = vec![0u8; 128];
        d[20..22].copy_from_slice(&block.to_le_bytes());
        d[64..68].copy_from_slice(&[0xFF, 0xFF, 0x08, 0x00]);
        d[68..72].copy_from_slice(&len.to_le_bytes());
        d[72..76].copy_from_slice(b"four");
        Sidecars {
            dbt: Some(d),
            ..Default::default()
        }
    };
    assert_eq!(
        read_with(&data(0x8B), four(64, 12), no_fid()).0[1][0],
        "four"
    );
    for (block, len) in [(64, 4), (64, u32::MAX), (64, 2000)] {
        let (rows, w) = read_with(&data(0x8B), four(block, len), no_fid());
        assert_eq!(rows[1][0], "", "{block} {len}");
        assert!(!w.is_empty());
    }
    let (_, w) = read_with(&data(0x8B), four(0, 12), no_fid());
    assert!(w.iter().any(|w| w.contains("block size")), "{w:?}");
    // No .dbt: blank, with a warning.
    let (rows, w) = read(&data(0x83));
    assert_eq!(rows[1][1], "");
    assert!(!w.is_empty());
}

#[test]
fn visual_foxpro_types_and_nulls() {
    let mut f = Dbf::new(&[
        ("I", b'I', 4),
        ("B", b'B', 8),
        ("Y", b'Y', 8),
        ("T", b'T', 8),
        ("V", b'V', 6),
        ("N", b'N', 3),
        ("M", b'M', 4),
        ("_NullFlags", b'0', 1),
    ]);
    f.version = 0x30;
    // V and N can be null: bits are V short, V null, N null.
    f.fields[4].4 = 0x02;
    f.fields[5].4 = 0x02;
    let mut when = 2_460_000i32.to_le_bytes().to_vec();
    when.extend_from_slice(&3_723_004i32.to_le_bytes());
    let f = f
        .row(&[
            &(-7i32).to_le_bytes(),
            &1.5f64.to_le_bytes(),
            &(-12_345i64).to_le_bytes(),
            &when,
            b"ab\0\0\0\x02",
            b" 42",
            &5u32.to_le_bytes(),
            &[0b001],
        ])
        .row(&[
            &0i32.to_le_bytes(),
            &0f64.to_le_bytes(),
            &0i64.to_le_bytes(),
            &[0; 8],
            b"sixsix",
            b" 42",
            &0u32.to_le_bytes(),
            &[0b100],
        ]);
    let (rows, warnings) = read_with(&f.bytes(), Sidecars::default(), no_fid());
    assert_eq!(
        rows,
        [
            vec!["I", "B", "Y", "T", "V", "N", "M"],
            vec![
                "-7",
                "1.5",
                "-1.2345",
                "2023-02-24 01:02:03.004",
                "ab",
                "42",
                ""
            ],
            vec!["0", "0", "0.0000", "", "sixsix", "", ""],
        ]
    );
    assert_eq!(warnings, ["field M: FoxPro memos are not read (1 records)"]);
}

#[test]
fn dbase7_and_unknown_types() {
    let mut f = Dbf::new(&[("A_LONG_FIELD_NAME_7", b'I', 4), ("G", b'G', 2)]);
    f.version = 0x04;
    let data = f
        .row(&[&(5u32 ^ 0x8000_0000).to_be_bytes(), b"zz"])
        .row(&[&((-5i32) as u32 ^ 0x8000_0000).to_be_bytes(), b""])
        .bytes();
    let (rows, warnings) = read_with(&data, Sidecars::default(), no_fid());
    assert_eq!(rows[0], ["A_LONG_FIELD_NAME_7", "G"]);
    assert_eq!(rows[1], ["5", ""]);
    assert_eq!(rows[2], ["-5", ""]);
    assert_eq!(
        warnings,
        ["field G: field type G is not supported, left blank (2 records)"]
    );
}

#[test]
fn headers_that_lie() {
    let good = Dbf::new(&[("A", b'C', 2)])
        .row(&[b"aa"])
        .row(&[b"bb"])
        .bytes();
    // More records claimed than are there.
    let mut d = good.clone();
    d[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let (rows, w) = read(&d);
    assert_eq!(rows.len(), 3);
    assert!(w[0].contains("shorter than its header says"), "{w:?}");
    // Fewer claimed: only those are read.
    let mut d = good.clone();
    d[4..8].copy_from_slice(&1u32.to_le_bytes());
    let (rows, w) = read(&d);
    assert_eq!(rows.len(), 2);
    assert!(w[0].contains("more than the 1 records"), "{w:?}");
    // The header length and the end of the field list disagree: both ways.
    let mut d = good.clone();
    let start = u16::from_le_bytes([d[8], d[9]]);
    d.splice(usize::from(start)..usize::from(start), [0u8; 10]);
    d[8..10].copy_from_slice(&(start + 10).to_le_bytes());
    let (rows, w) = read(&d);
    assert_eq!(rows[1], ["0", "aa"]);
    assert!(w[0].contains("disagree"), "{w:?}");
    let mut d = good.clone();
    d[64] = b'x';
    let (_, w) = read(&d);
    assert!(w.iter().any(|w| w.contains("disagree")), "{w:?}");
    // Field widths that do not add up.
    let mut d = good.clone();
    d[48] = 9;
    let (rows, w) = read(&d);
    assert_eq!(rows[1], ["0", ""]);
    assert!(w.iter().any(|w| w.contains("add up")), "{w:?}");
    // Refused outright.
    let o = DbfOptions::default();
    let none = Sidecars::default();
    for (at, bytes) in [
        (0, vec![0x02]),
        (0, vec![0x77]),
        (8, vec![0, 0]),
        (8, vec![255, 255]),
        (10, vec![0, 0]),
    ] {
        let mut d = good.clone();
        d[at..at + bytes.len()].copy_from_slice(&bytes);
        assert!(dbf::inspect(&d, &none, &o).is_err(), "{at} {bytes:?}");
    }
}

#[test]
fn a_very_wide_table() {
    let mut f = Dbf::new(&[]);
    f.fields = vec![("C", b'C', 1, 0, 0); 2_046];
    let values: Vec<&[u8]> = vec![b"x"; 2_046];
    let f = f.row(&values);
    assert_eq!(read(&f.bytes()).0[1].len(), 2_047);
    let mut f = Dbf::new(&[]);
    f.fields = vec![("C", b'C', 0, 0, 0); 2_047];
    let mut d = f.bytes();
    d[10] = 2;
    assert!(dbf::inspect(&d, &Sidecars::default(), &DbfOptions::default()).is_err());
}

#[test]
fn warnings_hold_no_cell_text() {
    let data = Dbf::new(&[("SECRET", b'N', 7), ("MEMO", b'M', 10)])
        .row(&[b"*******", b"  zzqq"])
        .row(&[b"\x81\x8d", b"      12"])
        .bytes();
    let o = DbfOptions {
        encoding: "utf-8".into(),
        ..Default::default()
    };
    let (_, warnings) = read_with(&data, Sidecars::default(), o);
    assert!(warnings.len() >= 3, "{warnings:?}");
    for w in warnings {
        assert!(!w.contains("zzqq") && !w.contains("12 "), "{w}");
    }
}

#[test]
fn cut_off_at_every_byte() {
    let mut f = Dbf::new(&[("A", b'C', 3), ("M", b'M', 10)])
        .row(&[b"abc", b"1"])
        .row(&[b"def", b"1"]);
    f.version = 0x8B;
    let data = f.bytes();
    let mut dbt = vec![0u8; 80];
    dbt[20] = 64;
    dbt[64..72].copy_from_slice(&[0xFF, 0xFF, 0x08, 0x00, 12, 0, 0, 0]);
    dbt.extend_from_slice(b"memo");
    for n in 0..=data.len() {
        for k in [0, dbt.len() / 2, dbt.len()] {
            let o = ImportOptions {
                beside: Some(Arc::new(Sidecars {
                    dbt: Some(dbt[..k].to_vec()),
                    ..Default::default()
                })),
                ..Default::default()
            };
            let _ = import_report(&data[..n], Some("dbf"), &o, None, &Progress::new());
        }
    }
    for k in 0..=dbt.len() {
        let side = Sidecars {
            dbt: Some(dbt[..k].to_vec()),
            ..Default::default()
        };
        read_with(&data, side, DbfOptions::default());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    /// Random bytes, and a real table with bytes changed, never panic or hang.
    #[test]
    fn random_bytes_are_safe(
        noise in proptest::collection::vec(any::<u8>(), 0..600),
        changes in proptest::collection::vec((0usize..200, any::<u8>()), 0..8),
        memo in proptest::collection::vec(any::<u8>(), 0..1200),
        version in proptest::sample::select(vec![0x03u8, 0x83, 0x8B, 0x30, 0x04, 0x8C, 0xF5]),
    ) {
        let started = std::time::Instant::now();
        let mut f = Dbf::new(&[("A", b'C', 3), ("M", b'M', 10), ("T", b'T', 8)])
            .row(&[b"abc", b"1", b""])
            .row(&[b"def", b"2", b""]);
        f.version = version;
        let mut real = f.bytes();
        for (at, b) in changes {
            let at = at % real.len();
            real[at] = b;
        }
        let mut headed = noise.clone();
        if let Some(first) = headed.first_mut() {
            *first = version;
        }
        for data in [noise, headed, real] {
            let o = ImportOptions {
                beside: Some(Arc::new(Sidecars {
                    dbt: Some(memo.clone()),
                    cpg: Some(memo.iter().take(20).copied().collect()),
                    ..Default::default()
                })),
                ..Default::default()
            };
            let _ = import_report(&data, Some("dbf"), &o, Some(50), &Progress::new());
        }
        prop_assert!(started.elapsed().as_secs() < 5);
    }
}

#[test]
fn shapefile_parts_find_their_table() {
    let d = Folder::new("shp");
    std::fs::write(d.0.join("roads.dbf"), b"x").unwrap();
    for ext in ["shp", "SHX", "prj", "cpg", "sbn", "sbx"] {
        let found = dbf::shapefile_table(&d.0.join(format!("roads.{ext}")));
        assert_eq!(found, Some(d.0.join("roads.dbf")), "{ext}");
    }
    assert_eq!(dbf::shapefile_table(&d.0.join("roads.dbf")), None);
    assert_eq!(dbf::shapefile_table(&d.0.join("rivers.shp")), None);
    std::fs::write(d.0.join("roads.cpg"), b"UTF-8").unwrap();
    std::fs::write(d.0.join("roads.prj"), b"GEOGCS[\"made up\"]\nmore").unwrap();
    let side = Sidecars::beside(&d.0.join("roads.dbf"));
    assert_eq!(side.cpg.as_deref(), Some(&b"UTF-8"[..]));
    let info = dbf::inspect(
        &Dbf::new(&[("A", b'C', 1)]).bytes(),
        &side,
        &DbfOptions::default(),
    )
    .unwrap();
    assert_eq!(info.projection, "GEOGCS[\"made up\"]");
}

// ---- GeoPackage ----

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let d = std::env::temp_dir().join(format!("nd-gis-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Folder(d)
    }
    fn listing(&self) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    e.metadata().unwrap().len(),
                )
            })
            .collect();
        v.sort();
        v
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn geopackage(path: &Path, wal: bool) {
    let c = Connection::open(path).unwrap();
    if wal {
        c.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
            .unwrap();
    }
    c.execute_batch(
        "PRAGMA application_id = 1196444487;
         CREATE TABLE gpkg_spatial_ref_sys(srs_id INTEGER PRIMARY KEY, organization TEXT, organization_coordsys_id INTEGER);
         INSERT INTO gpkg_spatial_ref_sys VALUES(4326, 'EPSG', 4326);
         CREATE TABLE gpkg_contents(table_name TEXT PRIMARY KEY, data_type TEXT);
         INSERT INTO gpkg_contents VALUES('roads', 'features'), ('odd \"name\" here', 'attributes'), ('map', 'tiles'), ('needs_gis', 'features');
         CREATE TABLE gpkg_geometry_columns(table_name TEXT, column_name TEXT, geometry_type_name TEXT, srs_id INTEGER);
         INSERT INTO gpkg_geometry_columns VALUES('roads', 'geom', 'MULTILINESTRING', 4326);
         CREATE TABLE gpkg_ogr_contents(table_name TEXT, feature_count INTEGER);
         INSERT INTO gpkg_ogr_contents VALUES('roads', 2);
         CREATE TABLE roads(fid INTEGER PRIMARY KEY, geom BLOB, name TEXT, outline POLYGON, photo BLOB);
         INSERT INTO roads VALUES(1, zeroblob(10000000), 'Made Up Road', x'00', x'0102');
         INSERT INTO roads VALUES(2, x'00', '00123', NULL, NULL);
         CREATE TABLE \"odd \"\"name\"\" here\"(a TEXT);
         INSERT INTO \"odd \"\"name\"\" here\" VALUES('v');
         CREATE TABLE map(id INTEGER);
         CREATE TABLE rtree_roads_geom_node(id INTEGER);
         CREATE VIEW needs_gis AS SELECT ST_AsText(geom) AS g FROM roads;",
    )
    .unwrap();
}

#[test]
fn geopackage_lists_layers_and_skips_shapes() {
    let d = Folder::new("gpkg");
    let path = d.0.join("made up.gpkg");
    geopackage(&path, false);
    let tables = sqlite::tables(&path).unwrap();
    let shown: Vec<(&str, &str)> = tables
        .iter()
        .filter(|t| matches!(t.kind.as_str(), "features" | "attributes"))
        .map(|t| (t.name.as_str(), t.kind.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("needs_gis", "features"),
            ("odd \"name\" here", "attributes"),
            ("roads", "features"),
        ]
    );
    assert_eq!(
        tables.iter().find(|t| t.name == "roads").unwrap().rows,
        Some(2)
    );
    for hidden in ["map", "gpkg_contents", "rtree_roads_geom_node"] {
        let kind = &tables.iter().find(|t| t.name == hidden).unwrap().kind;
        assert!(kind == "tiles" || kind == "system", "{hidden} {kind}");
    }
    let r = sqlite::read_table_report(&path, "roads", &Progress::new()).unwrap();
    assert_eq!(r.doc.row_values(0).unwrap(), ["fid", "name", "photo"]);
    assert_eq!(
        r.doc.row_values(1).unwrap(),
        ["1", "Made Up Road", "(binary, 2 bytes)"]
    );
    assert_eq!(
        r.geometry,
        ["geom (MULTILINESTRING, EPSG:4326)", "outline (POLYGON)"]
    );
    let odd = sqlite::read_table(&path, "odd \"name\" here", &Progress::new()).unwrap();
    assert_eq!(odd.row_values(1).unwrap(), ["v"]);
    let e = sqlite::read_table(&path, "needs_gis", &Progress::new())
        .err()
        .unwrap();
    assert!(e.to_string().contains("needs a GIS program"), "{e}");
    // A file without its list of reference systems still hides its shapes.
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE gpkg_spatial_ref_sys")
        .unwrap();
    let r = sqlite::read_table_report(&path, "roads", &Progress::new()).unwrap();
    assert_eq!(r.doc.row_values(0).unwrap(), ["fid", "name", "photo"]);
    assert_eq!(r.geometry[0], "geom (MULTILINESTRING, SRS 4326)");
    // A GeoPackage is never written.
    let e = odd
        .save_sqlite(&path, "x", true, false, &Progress::new())
        .unwrap_err();
    assert!(e.to_string().contains("never written"), "{e}");
}

/// Opening never creates or changes a file, whatever the journal mode, and
/// names with odd characters work.
#[test]
fn reading_leaves_the_folder_as_it_was() {
    let d = Folder::new("quiet");
    let names = [
        "plain.gpkg",
        "wal.gpkg",
        "wal with space #1 50% é?.gpkg".trim_end_matches('?'),
        "empty-wal.sqlite",
    ];
    for (i, name) in names.iter().enumerate() {
        let name = name.replace('?', "");
        let path = d.0.join(&name);
        geopackage(&path, i > 0);
        if name.starts_with("empty-wal") {
            std::fs::write(d.0.join(format!("{name}-wal")), b"").unwrap();
        }
        let before = d.listing();
        assert!(!sqlite::tables(&path).unwrap().is_empty(), "{name}");
        let doc = sqlite::read_table(&path, "roads", &Progress::new()).unwrap();
        assert_eq!(doc.row_count(), 3, "{name}");
        assert_eq!(d.listing(), before, "{name}");
    }
}

/// A network share path, when this computer lets the test reach its own
/// drive through one.
#[cfg(windows)]
#[test]
fn reads_a_wal_database_through_a_share_path() {
    let d = Folder::new("unc");
    let path = d.0.join("share.gpkg");
    geopackage(&path, true);
    let text = path.to_string_lossy();
    let (drive, rest) = text.split_once(":\\").unwrap();
    let unc = PathBuf::from(format!(r"\\localhost\{drive}$\{rest}"));
    if !unc.is_file() {
        eprintln!("no administrative share here: skipped");
        return;
    }
    let before = d.listing();
    assert_eq!(
        sqlite::read_table(&unc, "roads", &Progress::new())
            .unwrap()
            .row_count(),
        3
    );
    assert_eq!(d.listing(), before);
}

#[test]
fn unsaved_changes_are_refused() {
    let d = Folder::new("busy");
    let path = d.0.join("busy.gpkg");
    geopackage(&path, true);
    // A writer that has not folded its changes into the file yet.
    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch(
            "PRAGMA wal_autocheckpoint = 0; INSERT INTO roads(fid, name) VALUES(3, 'new');",
        )
        .unwrap();
    let before = d.listing();
    let e = sqlite::tables(&path).unwrap_err();
    assert!(e.to_string().contains("-wal"), "{e}");
    assert!(sqlite::read_table(&path, "roads", &Progress::new()).is_err());
    assert_eq!(d.listing(), before);
    drop(writer);
    // An unfinished rollback journal.
    let plain = d.0.join("plain.sqlite");
    Connection::open(&plain)
        .unwrap()
        .execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES(1);")
        .unwrap();
    std::fs::write(
        d.0.join("plain.sqlite-journal"),
        [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7, 0, 0],
    )
    .unwrap();
    let e = sqlite::tables(&plain).unwrap_err();
    assert!(e.to_string().contains("-journal"), "{e}");
    // A journal kept but emptied is no obstacle.
    std::fs::write(d.0.join("plain.sqlite-journal"), [0u8; 512]).unwrap();
    assert_eq!(sqlite::tables(&plain).unwrap().len(), 1);
}
