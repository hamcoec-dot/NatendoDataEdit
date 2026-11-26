//! Performance budget check on a generated file (default 10M rows × 20
//! columns, about 2 GB). Exits non-zero when a budget is missed.
//!
//! ```text
//! cargo run --release -p natendo-engine --example perf -- [rows] [--keep]
//! ```
//! The fixture is written under `target/perf/` and deleted afterwards unless
//! `--keep` is given (a kept fixture is reused by the next run).

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use natendo_engine::clean::{CaseTo, CleanSpec, Cleaner, Spaces, Step};
use natendo_engine::example::ExampleSpec;
use natendo_engine::export::{ExportFormat, ExportOptions};
use natendo_engine::expr::ComputeSpec;
use natendo_engine::filter::{ColumnFilter, Condition, FilterSpec, Rule, TextOp};
use natendo_engine::find::{FindSpec, Pattern, PatternKind, Scope};
use natendo_engine::import;
use natendo_engine::join::{
    AppendSource, AppendSpec, JoinSource, JoinSpec, JoinWrite, KeepRows, KeyMatch,
};
use natendo_engine::ops::{Dir, FillSpec, PasteSpec, SplitColumn};
use natendo_engine::reshape::{Aggregate, Blanks, Duplicates, Pivot, Unpivot};
use natendo_engine::sort::{SortKey, SortKind, SortSpec};
use natendo_engine::stats::{ChartKind, ChartSpec, SubtotalSpec, Total};
use natendo_engine::validate::{Check as DataCheck, CheckKind, CheckSpec};
use natendo_engine::value::ReadOptions;
use natendo_engine::{Document, OpenOptions, Progress, Quoting, SaveOptions};

const COLS: usize = 20;

fn generate(path: &PathBuf, rows: u64) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let header: Vec<String> = (0..COLS).map(|c| format!("col{c}")).collect();
    writeln!(w, "{}\r", header.join(","))?;
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        write!(
            w,
            "{i},name {},{}.{:02},\"note, {}\",City {},2026-{:02}-{:02},{}",
            x % 9973,
            x % 100_000,
            x % 100,
            x % 1000,
            x % 311,
            x % 12 + 1,
            x % 28 + 1,
            x % 2 == 0
        )?;
        for c in 8..COLS {
            write!(w, ",v{}", (x >> (c * 2)) % 1_000_000)?;
        }
        w.write_all(b"\r\n")?;
    }
    w.flush()
}

struct Check {
    failed: bool,
}

impl Check {
    fn budget(&mut self, what: &str, took: Duration, limit: Duration) {
        let ok = took <= limit;
        self.failed |= !ok;
        println!(
            "{} {what}: {:.1} ms (budget {:.0} ms)",
            if ok { "PASS" } else { "FAIL" },
            took.as_secs_f64() * 1e3,
            limit.as_secs_f64() * 1e3
        );
    }
}

/// Several selected blocks (Ctrl+click): totals over three whole-column
/// blocks against one, and the split of 10,000 blocks. (`--multi-only` runs
/// just this on the fixture.)
fn multi_area_checks(
    big: &Document,
    rows: u64,
    read: ReadOptions,
    p: &Progress,
    check: &mut Check,
) {
    // Budgets for 10M rows, scaled by row count (as the rest of this file).
    let rs = (rows as f64 / 1e7).max(0.05);
    let budget = |secs: f64| Duration::from_secs_f64(secs * rs);
    {
        use natendo_engine::areas::{disjoint, Area};
        let col = |c: u32| Area {
            row: 1,
            rows: rows - 1,
            col: c,
            cols: 1,
        };
        // The same number of cells as one block of three columns next to each other.
        // One warm-up of each (the file's pages are read in), then the best of five,
        // so a busy moment of the machine cannot fail a ratio gate.
        let best = |f: &mut dyn FnMut()| {
            f();
            let mut best = Duration::MAX;
            for _ in 0..5 {
                let t = Instant::now();
                f();
                best = best.min(t.elapsed());
            }
            best
        };
        let one = best(&mut || {
            big.selection_totals(1, rows - 1, 2, 3, read, p).unwrap();
        });
        let mut three = None;
        let many = best(&mut || {
            three = Some(
                big.selection_totals_areas(&[col(2), col(4), col(6)], read, p)
                    .unwrap(),
            );
        });
        let three = three.expect("ran");
        assert!(three.cells >= 3 * (rows - 1));
        println!(
            "  [multi-area] totals: one 1M x 3 block {:?}, three 1M-row column blocks {:?} ({:.2}x)",
            one,
            many,
            many.as_secs_f64() / one.as_secs_f64().max(1e-9)
        );
        check.budget("quick totals, three whole-column blocks", many, budget(3.0));
        // Gate: within 1.2x of the same cells as one block (a little slack for timer noise).
        check.budget(
            "three column blocks within 1.2x one 3-column block",
            many,
            one.mul_f64(1.2) + Duration::from_millis(3),
        );
        let singles: Vec<Area> = (0..10_000u64)
            .map(|i| Area {
                row: (i * 7919) % 1_000_000,
                rows: 1,
                col: (i % 20) as u32,
                cols: 1,
            })
            .collect();
        let t = Instant::now();
        let d = disjoint(&singles);
        check.budget(
            "split 10,000 selected blocks",
            t.elapsed(),
            Duration::from_millis(50),
        );
        assert!(!d.is_empty());
    }
    find_block_checks(big, rows, p, check);
}

/// Find over several blocks: the same cells as one block should cost about the
/// same, and many blocks must not make a search slow.
fn find_block_checks(big: &Document, rows: u64, p: &Progress, check: &mut Check) {
    use natendo_engine::areas::Area;
    let rs = (rows as f64 / 1e7).max(0.05);
    let budget = |secs: f64| Duration::from_secs_f64(secs * rs);
    let spec = |text: &str, scope: Scope| FindSpec {
        pattern: Pattern {
            text: text.into(),
            kind: PatternKind::Plain,
            ..Default::default()
        },
        scope,
        ..Default::default()
    };
    let area = |row: u64, rows: u64, col: u32, cols: u32| Area {
        row,
        rows,
        col,
        cols,
    };
    let time = |f: &mut dyn FnMut()| {
        // The best of three: a search is a few hundred milliseconds and the disk cache is warm.
        let mut best = Duration::MAX;
        for _ in 0..3 {
            let t = Instant::now();
            f();
            best = best.min(t.elapsed());
        }
        best
    };
    let n = rows - 1;
    // One block of three columns: the Range scope and the same block as an Areas scope.
    let range = spec(
        "City 7",
        Scope::Range {
            row: 1,
            rows: n,
            col: 4,
            cols: 3,
        },
    );
    let one = spec(
        "City 7",
        Scope::Areas {
            areas: vec![area(1, n, 4, 3)],
        },
    );
    let mut cells = 0;
    let t_range = time(&mut || cells = big.find_count(&range, None, p).unwrap().cells);
    let t_one = time(&mut || assert_eq!(big.find_count(&one, None, p).unwrap().cells, cells));
    println!("  [find blocks] count: Range {t_range:?}, the same block as Areas {t_one:?}");
    check.budget(
        "find count: one block as Areas within 1.2x of Range",
        t_one,
        t_range.mul_f64(1.2) + Duration::from_millis(10),
    );
    // 1,000 blocks that are half of the rows (a gap between each), against one block of half the rows.
    let chunk = (n / 2_000).max(1);
    let thousand: Vec<Area> = (0..1_000u64)
        .map(|i| area(1 + i * 2 * chunk, chunk, 4, 3))
        .collect();
    let half = spec(
        "City 7",
        Scope::Range {
            row: 1,
            rows: 1_000 * chunk,
            col: 4,
            cols: 3,
        },
    );
    let many = spec(
        "City 7",
        Scope::Areas {
            areas: thousand.clone(),
        },
    );
    let t_half = time(&mut || {
        big.find_count(&half, None, p).unwrap();
    });
    let t_many = time(&mut || {
        big.find_count(&many, None, p).unwrap();
    });
    println!("  [find blocks] count: one block of 1,000 chunks' cells {t_half:?}, 1,000 blocks {t_many:?}");
    check.budget(
        "find count: 1,000 blocks within 2x of one block of the same cells",
        t_many,
        t_half.mul_f64(2.0) + Duration::from_millis(20),
    );
    let t_list_half = time(&mut || {
        big.find_all(&half, 1_000, p).unwrap();
    });
    let t_list_many = time(&mut || {
        big.find_all(&many, 1_000, p).unwrap();
    });
    println!("  [find blocks] find all: one block {t_list_half:?}, 1,000 blocks {t_list_many:?}");
    check.budget(
        "find all: 1,000 blocks within 2x of one block of the same cells",
        t_list_many,
        t_list_half.mul_f64(2.0) + Duration::from_millis(20),
    );
    // Find next with no match anywhere reads every searched cell (the worst case).
    let nomatch_half = spec("zzzz", half.scope.clone());
    let nomatch_many = spec("zzzz", many.scope.clone());
    let t_next_half = time(&mut || {
        assert!(big
            .find_next(&nomatch_half, None, false, p)
            .unwrap()
            .is_none());
    });
    let t_next_many = time(&mut || {
        assert!(big
            .find_next(&nomatch_many, None, false, p)
            .unwrap()
            .is_none());
    });
    println!("  [find blocks] find next, no match: one block {t_next_half:?}, 1,000 blocks {t_next_many:?}");
    check.budget(
        "find next (no match): 1,000 blocks within 2x of one block",
        t_next_many,
        t_next_half.mul_f64(2.0) + Duration::from_millis(20),
    );
    // 10,000 one-row blocks scattered over the file.
    let scattered: Vec<Area> = (0..10_000u64)
        .map(|i| area(1 + (i * 7919) % n, 1, 4, 3))
        .collect();
    let scat = spec("City 7", Scope::Areas { areas: scattered });
    let t_scat = time(&mut || {
        big.find_count(&scat, None, p).unwrap();
    });
    println!("  [find blocks] count over 10,000 scattered blocks: {t_scat:?}");
    check.budget(
        "find count: 10,000 scattered one-row blocks",
        t_scat,
        budget(3.0),
    );
    // Replace preview over the 1,000 blocks: the plan, then a page of it.
    let t0 = Instant::now();
    let plan = big.plan_replace(&many, "Town 7", 50, p).unwrap();
    let t_plan = t0.elapsed();
    let t1 = Instant::now();
    let page = big.replace_page(&plan, 0, 50).unwrap();
    let t_page = t1.elapsed();
    println!(
        "  [find blocks] replace plan over 1,000 blocks ({} cells): {t_plan:?}, a page of the preview: {t_page:?}",
        plan.cells
    );
    assert!(!page.is_empty() || plan.cells == 0);
    check.budget("replace plan over 1,000 blocks", t_plan, budget(3.0));
    check.budget(
        "a page of the replace preview",
        t_page,
        Duration::from_millis(200),
    );
}

/// The best of three Excel exports in parts, timed by the `xlsx_bench` example in a process of
/// its own (next to this program, else built and run by cargo). None when it cannot be run.
fn xlsx_in_own_process(rows: u64, dir: &std::path::Path, part_rows: u64) -> Option<Duration> {
    let exe = std::env::current_exe()
        .ok()?
        .with_file_name(format!("xlsx_bench{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = if exe.exists() {
        std::process::Command::new(exe)
    } else {
        let mut c =
            std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        c.args([
            "run",
            "--release",
            "-q",
            "-p",
            "natendo-engine",
            "--example",
            "xlsx_bench",
            "--",
        ]);
        c
    };
    let out = cmd
        .args([&rows.to_string(), "3", "shuffled"])
        .arg(dir)
        .arg(part_rows.to_string())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let ms: f64 = text
        .lines()
        .find_map(|l| l.strip_prefix("best "))?
        .trim_end_matches(" ms")
        .parse()
        .ok()?;
    Some(Duration::from_secs_f64(ms / 1000.0))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .iter()
        .find_map(|a| a.parse().ok())
        .unwrap_or(10_000_000);
    let keep = args.iter().any(|a| a == "--keep");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/perf");
    fs::create_dir_all(&dir).expect("create target/perf");
    let path = dir.join(format!("fixture-{rows}.csv"));
    if !path.exists() {
        let t = Instant::now();
        generate(&path, rows).expect("write fixture");
        println!(
            "generated {} in {:.1} s",
            path.display(),
            t.elapsed().as_secs_f64()
        );
    }
    let size = fs::metadata(&path).unwrap().len();
    println!("file: {rows} rows, {:.2} GB", size as f64 / 1e9);
    // Budgets scale with size so small runs are still meaningful.
    let scale = (size as f64 / 2e9).max(0.05);
    let mut check = Check { failed: false };
    if args.iter().any(|a| a == "--multi-only") {
        let big = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let p = Progress::new();
        multi_area_checks(
            &big,
            big.row_count(),
            ReadOptions::default(),
            &p,
            &mut check,
        );
        std::process::exit(i32::from(check.failed));
    }

    // Open: first screen, then the full index.
    let t = Instant::now();
    let mut doc = Document::open(&path, OpenOptions::default()).unwrap();
    doc.index_step(16 << 20).unwrap();
    let first = doc.viewport(0..60, 0..20, 4096).unwrap();
    assert_eq!(first.rows, 60);
    check.budget("first rows visible", t.elapsed(), Duration::from_secs(1));
    // Layout detection alone (it also ran inside the open above): it reads
    // the start, middle and end of the file whatever its size.
    let t = Instant::now();
    let found = doc.detect_layout(Some("csv"));
    check.budget("layout detection", t.elapsed(), Duration::from_millis(50));
    assert_eq!(found.best().dialect, natendo_engine::Dialect::CSV);
    assert!(found.is_confident());
    while !doc.index_step(64 << 20).unwrap() {}
    check.budget(
        "full index",
        t.elapsed(),
        Duration::from_secs_f64(10.0 * scale),
    );
    assert_eq!(doc.row_count(), rows + 1);
    let idx = doc.index().memory_bytes();
    println!(
        "     index memory: {:.0} MB ({:.1}% of file)",
        idx as f64 / 1e6,
        idx as f64 * 100.0 / size as f64
    );

    // Random jumps: viewport fetch time.
    let mut times = Vec::with_capacity(2000);
    let mut x: u64 = 12345;
    for _ in 0..2000 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let r = (x >> 11) % (doc.row_count() - 60);
        let t = Instant::now();
        let v = doc.viewport(r..r + 60, 0..20, 4096).unwrap();
        times.push(t.elapsed());
        assert_eq!(v.cells.len(), 1200);
    }
    times.sort();
    let p99 = times[times.len() * 99 / 100];
    println!(
        "     viewport median: {:.3} ms",
        times[times.len() / 2].as_secs_f64() * 1e3
    );
    check.budget("viewport 60x20 p99", p99, Duration::from_millis(5));

    // Milestone 4: sort, filter, find and replace (budgets for 10M rows,
    // scaled by row count).
    let rs = (rows as f64 / 1e7).max(0.05);
    let budget = |secs: f64| Duration::from_secs_f64(secs * rs);
    let p = Progress::new();
    let key = |col, kind| SortSpec {
        keys: vec![SortKey {
            col,
            descending: false,
            kind,
            match_case: false,
        }],
        keep_first: true,
        read: Default::default(),
        lists: Vec::new(),
    };
    let t = Instant::now();
    let plan = doc.plan_sort(&key(2, SortKind::Number), &p).unwrap();
    doc.apply_sort(plan).unwrap();
    check.budget("sort numeric column", t.elapsed(), budget(3.0));
    doc.undo();
    let t = Instant::now();
    let plan = doc.plan_sort(&key(1, SortKind::Auto), &p).unwrap();
    doc.apply_sort(plan).unwrap();
    check.budget("sort text column (natural)", t.elapsed(), budget(5.0));
    doc.undo();
    // v0.4.2: a cell formula filled down 100k cells (Ctrl+Enter) costs
    // about what pasting 100k rows does.
    {
        use natendo_engine::cellformula::CellFormula;
        let t = Instant::now();
        let res = doc
            .run_cell_formula(
                &CellFormula {
                    formula: "=C2*2 + $C$2".into(),
                    row: 1,
                    col: 0,
                    targets: (1..=100_000).map(|r| (r, 0)).collect(),
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        let plan = doc.formula_plan(&res).unwrap();
        doc.apply_cells(plan).unwrap();
        doc.remember_formulas(&res);
        check.budget(
            "cell formula filled down 100k cells",
            t.elapsed(),
            budget(1.0),
        );
        doc.undo();
    }
    // v0.4.2: filter by formula costs about what a filter on one column does.
    let t = Instant::now();
    let (plan, _) = doc
        .plan_formula_filter(
            &ComputeSpec {
                expr: "AND(VALUE([col2]) > 500, LEN([col4]) = 8)".into(),
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    println!("     filter by formula: {} rows", plan.rows);
    doc.apply_rows(plan).unwrap();
    check.budget("filter by formula", t.elapsed(), budget(1.0));
    doc.clear_filter();
    let t = Instant::now();
    let spec = FilterSpec {
        columns: vec![ColumnFilter {
            col: 4,
            rule: Rule::Conditions {
                first: Condition::Text {
                    op: TextOp::Contains,
                    text: "city 12".into(),
                    match_case: false,
                },
                second: None,
                or: false,
            },
        }],
        keep_first: true,
        read: Default::default(),
    };
    let plan = doc.plan_filter(&spec, &p).unwrap();
    doc.apply_filter(plan).unwrap();
    println!("     filter shows {} rows", doc.row_count());
    check.budget("filter one column", t.elapsed(), budget(1.0));
    // A 100k-row block pasted into filtered rows (it fills the shown rows and
    // stops), passing over a hidden column, costs about what a paste with
    // nothing hidden does.
    let block: Vec<Vec<String>> = (0..100_000)
        .map(|i| vec![format!("p{i}"), "x".into(), "y".into()])
        .collect();
    let paste = |skip: Vec<u32>| PasteSpec {
        row: 1,
        col: 1,
        skip_cols: skip,
        stop_at_end: true,
        ..Default::default()
    };
    let t = Instant::now();
    let plan = doc.plan_paste(&paste(vec![2]), block.clone(), &p).unwrap();
    println!(
        "     filtered paste: {} cells, {} hidden rows between",
        plan.cells, plan.hidden_rows
    );
    doc.apply_cells(plan).unwrap();
    let filtered = t.elapsed();
    check.budget(
        "paste a 100k-row block into the shown rows of a filter",
        filtered,
        budget(1.0),
    );
    doc.undo();
    doc.clear_filter();
    let t = Instant::now();
    let plan = doc.plan_paste(&paste(Vec::new()), block, &p).unwrap();
    doc.apply_cells(plan).unwrap();
    println!(
        "     unfiltered paste: {:.1} ms (filtered {:.1} ms)",
        t.elapsed().as_secs_f64() * 1e3,
        filtered.as_secs_f64() * 1e3
    );
    doc.undo();
    let find = |text: &str, kind, col| FindSpec {
        pattern: Pattern {
            text: text.into(),
            kind,
            match_case: false,
            whole_cell: false,
            ..Default::default()
        },
        scope: Scope::Columns { cols: vec![col] },
        skip_first: true,
        skip_cols: Vec::new(),
        ..Default::default()
    };
    let t = Instant::now();
    let all = doc
        .find_all(&find(r"^name 99\d$", PatternKind::Regex, 1), 10_000, &p)
        .unwrap();
    println!("     find all: {} cells", all.cells);
    check.budget("find all, one column (regex)", t.elapsed(), budget(2.0));
    let t = Instant::now();
    let plan = doc
        .plan_replace(&find("2026-01-", PatternKind::Plain, 5), "2025-01-", 10, &p)
        .unwrap();
    println!("     replace all: {} cells", plan.cells);
    doc.apply_replace(plan).unwrap();
    check.budget("replace all, one column", t.elapsed(), budget(5.0));
    doc.undo();
    assert!(!doc.is_modified());

    // Milestone 4b: fill, edges, duplicates, blanks, split, pivot, unpivot.
    let n = doc.row_count();
    let t = Instant::now();
    let plan = doc
        .plan_fill(
            &FillSpec {
                row: 1,
                col: 0,
                rows: 2,
                cols: 1,
                dir: Dir::Down,
                count: 5_000_000.min(n - 3),
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    println!("     fill series: {} cells", plan.cells);
    doc.apply_cells(plan).unwrap();
    check.budget("fill series down, 5M cells", t.elapsed(), budget(5.0));
    doc.undo();
    let t = Instant::now();
    let e = doc.edge(1, 0, Dir::Down).unwrap();
    assert_eq!(e.0, n - 1);
    check.budget(
        "data edge (Ctrl+Down, full column)",
        t.elapsed(),
        budget(1.0),
    );
    let dups = |cols: Vec<u32>| Duplicates {
        cols,
        keep_first: true,
        ..Default::default()
    };
    let t = Instant::now();
    let plan = doc.plan_show_duplicates(&dups(vec![1]), &p).unwrap();
    println!(
        "     duplicates: {} rows in {} groups",
        plan.rows, plan.groups
    );
    check.budget("find duplicates, one column", t.elapsed(), budget(3.0));
    let t = Instant::now();
    let plan = doc.plan_remove_duplicates(&dups(vec![4]), &p).unwrap();
    println!("     remove duplicates: {} rows", plan.rows);
    doc.apply_rows(plan).unwrap();
    check.budget("remove duplicates, one column", t.elapsed(), budget(5.0));
    doc.undo();
    let t = Instant::now();
    let plan = doc
        .plan_remove_blank_rows(
            &Blanks {
                spaces: true,
                keep_first: true,
            },
            &p,
        )
        .unwrap();
    assert_eq!(plan.rows, 0);
    check.budget("remove blank rows (none)", t.elapsed(), budget(3.0));
    // v0.4.1: Go To Special, compare two columns, protected columns, gzip.
    {
        use natendo_engine::colcompare::CompareCols;
        use natendo_engine::special::{SpecialKind, SpecialSpec};
        let area = |kind| SpecialSpec {
            kind,
            row: 1,
            rows: u64::MAX,
            cols: vec![0, 1, 2],
            ..Default::default()
        };
        let t = Instant::now();
        let found = doc.special_cells(&area(SpecialKind::Blanks), &p).unwrap();
        println!("     go to special: {} blank cells", found.total);
        check.budget(
            "go to special: blanks in 3 columns",
            t.elapsed(),
            budget(2.0),
        );
        let t = Instant::now();
        let found = doc.special_cells(&area(SpecialKind::Numbers), &p).unwrap();
        println!(
            "     go to special: {} number cells (listed {})",
            found.total,
            found.cells.len()
        );
        check.budget(
            "go to special: numbers in 3 columns",
            t.elapsed(),
            budget(3.0),
        );
        let spec = CompareCols {
            a: 1,
            b: 4,
            ..Default::default()
        };
        let t = Instant::now();
        let rep = doc.compare_cols(&spec, &p).unwrap();
        println!(
            "     compare columns: {} same, {} different",
            rep.same, rep.different
        );
        check.budget("compare two columns", t.elapsed(), budget(2.0));
        let t = Instant::now();
        let plan = doc.plan_compare_column(&spec, &p).unwrap();
        doc.apply_cols(plan).unwrap();
        check.budget(
            "compare two columns: add a result column",
            t.elapsed(),
            budget(5.0),
        );
        doc.undo();
        // A protected column costs nothing measurable on a paste.
        doc.set_protected(&[2]);
        let block: Vec<Vec<String>> = (0..100_000)
            .map(|i| vec![format!("q{i}"), "x".into(), "y".into()])
            .collect();
        let t = Instant::now();
        let plan = doc
            .plan_paste(
                &PasteSpec {
                    row: 1,
                    col: 1,
                    ..Default::default()
                },
                block,
                &p,
            )
            .unwrap();
        doc.apply_cells(plan).unwrap();
        println!(
            "     paste with a protected column: {} cells left",
            doc.protected_skipped()
        );
        check.budget(
            "paste 100k rows over a protected column",
            t.elapsed(),
            // The input is always 100,000 rows, whatever the file's size: one second
            // (what the 10M run has), not scaled by the rows.
            Duration::from_secs(1),
        );
        doc.undo();
        doc.set_protected(&[]);
        // A gzipped file of about a million rows opens (unpacks and reads)
        // within a few seconds.
        let gz = dir.join(format!("fixture-{rows}-part.csv.gz"));
        {
            let bytes = fs::read(&path).unwrap();
            let mut end = bytes.len().min(220 << 20);
            while end < bytes.len() && bytes[end - 1] != b'\n' {
                end += 1;
            }
            let mut enc = flate2::write::GzEncoder::new(
                BufWriter::new(File::create(&gz).unwrap()),
                flate2::Compression::fast(),
            );
            enc.write_all(&bytes[..end]).unwrap();
            enc.finish().unwrap().flush().unwrap();
        }
        let t = Instant::now();
        let mut g = Document::open_indexed(&gz, OpenOptions::default(), None).unwrap();
        println!("     gzip: {} rows", g.row_count());
        check.budget(
            "open a gzipped file (about 220 MB unpacked)",
            t.elapsed(),
            Duration::from_secs(4),
        );
        // Saved back gzipped after one edit (compressed on every core).
        g.set_cell(1, 1, "edited".into()).unwrap();
        let t = Instant::now();
        g.save(SaveOptions::default()).unwrap();
        check.budget(
            "save a gzipped file after one edit (about 220 MB unpacked)",
            t.elapsed(),
            Duration::from_secs(3),
        );
        drop(g);
        let _ = fs::remove_file(&gz);
    }
    let t = Instant::now();
    let plan = doc
        .plan_split_column(
            &SplitColumn {
                col: 3,
                separator: ", ".into(),
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    doc.apply_cols(plan).unwrap();
    check.budget("split one column in two", t.elapsed(), budget(10.0));
    doc.undo();
    assert!(!doc.is_modified());
    let t = Instant::now();
    let table = doc
        .pivot(
            &Pivot {
                rows: vec![4],
                col: 6,
                value: 2,
                aggregate: Aggregate::Sum,
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    println!("     pivot: {} rows x {} columns", table.rows, table.cols);
    check.budget("pivot (sum by city and flag)", t.elapsed(), budget(10.0));
    // Pivot tables (v2): cube read, quick preview, re-layout, many items,
    // drill-down.
    {
        use natendo_engine::pivot::{
            Agg, GridLimit, Grouping, PivotField, PivotSpec, ShowAs, SortBy, ValueField,
        };
        let titles: Vec<String> = doc.titles(&(0..COLS as u32).collect::<Vec<_>>(), true);
        let f = |col: u32, group: Grouping| PivotField {
            col,
            group,
            ..Default::default()
        };
        let v = |col: u32, agg: Agg| ValueField {
            col,
            agg,
            ..Default::default()
        };
        let mut spec = PivotSpec {
            rows: vec![f(4, Grouping::None), f(6, Grouping::None)],
            cols: vec![f(5, Grouping::YearMonth)],
            values: vec![
                v(2, Agg::Sum),
                v(0, Agg::CountRows),
                v(1, Agg::CountDistinct),
            ],
            ..Default::default()
        };
        let limit = Some(GridLimit {
            rows: 2_000,
            cols: 300,
        });
        let t = Instant::now();
        let quick = doc.pivot_cube(&spec, Some(200_000), &p).unwrap();
        let g = quick.layout(&spec, &titles, limit).unwrap();
        assert!(g.sampled || rows <= 200_000);
        check.budget(
            "pivot preview (first 200,000 rows)",
            t.elapsed(),
            Duration::from_millis(500),
        );
        let t = Instant::now();
        let cube = doc.pivot_cube(&spec, None, &p).unwrap();
        let g = cube.layout(&spec, &titles, limit).unwrap();
        println!(
            "     pivot v2: {} rows x {} value columns, cube {:.0} MB",
            g.total_rows,
            g.total_cols,
            cube.bytes as f64 / 1e6
        );
        assert_eq!(cube.rows_used, rows);
        check.budget(
            "pivot v2 (2 row fields, months, sum + rows + distinct)",
            t.elapsed(),
            budget(4.0),
        );
        // Settings that only change the layout reuse the cube.
        spec.values[0].show_as = ShowAs::PctOfGrand;
        spec.rows[0].sort.by = SortBy::Value;
        spec.rows[0].sort.desc = true;
        spec.layout.subtotals_top = false;
        let t = Instant::now();
        cube.layout(&spec, &titles, limit).unwrap();
        check.budget(
            "pivot re-layout (show as %, sort by value)",
            t.elapsed(),
            Duration::from_millis(300),
        );
        let t = Instant::now();
        let table = cube
            .layout(&spec, &titles, None)
            .unwrap()
            .to_table(false)
            .unwrap();
        println!(
            "     pivot table: {} rows x {} columns",
            table.rows, table.cols
        );
        check.budget(
            "pivot to new table",
            t.elapsed(),
            Duration::from_millis(500),
        );
        let t = Instant::now();
        let t2 = doc
            .pivot_drill(&spec, &cube, &[g.rows[0].path[0].clone()], &[], &p)
            .unwrap();
        println!("     drill-down: {} rows", t2.rows - 1);
        check.budget("pivot drill-down (one city)", t.elapsed(), budget(3.0));
        drop(cube);
        // Many combinations: name x city (millions of items).
        let many = PivotSpec {
            rows: vec![f(1, Grouping::None), f(4, Grouping::None)],
            values: vec![v(2, Agg::Sum), v(2, Agg::Median)],
            ..Default::default()
        };
        let t = Instant::now();
        let cube = doc.pivot_cube(&many, None, &p).unwrap();
        let g = cube.layout(&many, &titles, limit).unwrap();
        println!(
            "     pivot many items: {} rows, cube {:.0} MB",
            g.total_rows,
            cube.bytes as f64 / 1e6
        );
        check.budget(
            "pivot many items (name x city, sum + median)",
            t.elapsed(),
            budget(12.0),
        );
    }
    let mut filter_city = spec.clone();
    if let Rule::Conditions {
        first: Condition::Text { text, .. },
        ..
    } = &mut filter_city.columns[0].rule
    {
        *text = "city 1".into();
    }
    let plan = doc.plan_filter(&filter_city, &p).unwrap();
    doc.apply_filter(plan).unwrap();
    let t = Instant::now();
    let table = doc
        .unpivot(
            &Unpivot {
                ids: vec![0],
                values: vec![1, 2],
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    let new_doc = table.into_document().unwrap();
    println!(
        "     unpivot: {} rows shown -> {} rows",
        doc.row_count(),
        new_doc.row_count()
    );
    check.budget(
        "unpivot 2 columns (filtered rows)",
        t.elapsed(),
        budget(10.0),
    );
    drop(new_doc);
    doc.clear_filter();

    // Edits, then Save As (streamed, unedited rows copied raw).
    for i in 0..1000u64 {
        let r = (i * 7919) % doc.row_count();
        doc.set_cell(r, (i % 20) as u32, format!("edited {i}"))
            .unwrap();
    }
    let out = dir.join(format!("saved-{rows}.csv"));
    let t = Instant::now();
    let rep = doc
        .save(SaveOptions {
            path: Some(out.clone()),
            ..Default::default()
        })
        .unwrap();
    let took = t.elapsed();
    assert!(rep.verified);
    println!(
        "     save + re-index: {:.1} s ({} rows raw, {} rewritten)",
        took.as_secs_f64(),
        rep.raw_rows,
        rep.rows - rep.raw_rows
    );
    check.budget(
        "save + re-index",
        took,
        Duration::from_secs_f64(20.0 * scale),
    );

    // Column changes: every row is rewritten through the column map.
    doc.insert_cols(3, 1).unwrap();
    doc.move_cols(10, 2, 0).unwrap();
    doc.set_cell(5, 3, "new column".into()).unwrap();
    let out2 = dir.join(format!("saved-cols-{rows}.csv"));
    let t = Instant::now();
    let rep = doc
        .save(SaveOptions {
            path: Some(out2.clone()),
            ..Default::default()
        })
        .unwrap();
    let took = t.elapsed();
    assert!(rep.verified);
    println!(
        "     save after column changes: {:.1} s",
        took.as_secs_f64()
    );
    check.budget(
        "save after column changes",
        took,
        Duration::from_secs_f64(20.0 * scale),
    );
    // A sorted document saves every row out of file order, and a replace
    // leaves many patched rows.
    let plan = doc.plan_sort(&key(2, SortKind::Number), &p).unwrap();
    doc.apply_sort(plan).unwrap();
    let plan = doc
        .plan_replace(&find("2026-01-", PatternKind::Plain, 8), "2025-01-", 0, &p)
        .unwrap();
    doc.apply_replace(plan).unwrap();
    let out3 = dir.join(format!("saved-sorted-{rows}.csv"));
    let t = Instant::now();
    let rep = doc
        .save(SaveOptions {
            path: Some(out3.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(rep.verified);
    check.budget(
        "save after sort and replace",
        t.elapsed(),
        Duration::from_secs_f64(20.0 * scale),
    );

    // Milestone 5: Save As with every value quoted (every row rewritten).
    let out4 = dir.join(format!("saved-quoted-{rows}.csv"));
    let t = Instant::now();
    let rep = doc
        .save(SaveOptions {
            path: Some(out4.clone()),
            quoting: Quoting::All,
            ..Default::default()
        })
        .unwrap();
    assert!(rep.verified);
    // Quotes add about a quarter to the file, so this budget is per row
    // (20 s at 10M rows) rather than per byte.
    check.budget("save as, every value quoted", t.elapsed(), budget(20.0));

    // Milestone 7: column summary, column types, chart, subtotals.
    let read = ReadOptions::default();
    let t = Instant::now();
    let sum = doc.summary(2, true, read, &p).unwrap();
    check.budget("column summary (numbers)", t.elapsed(), budget(2.0));
    assert!(sum.rows > 0);
    let t = Instant::now();
    doc.summary(4, true, read, &p).unwrap();
    check.budget("column summary (text)", t.elapsed(), budget(2.0));
    let t = Instant::now();
    let types = doc.column_types(true, read).unwrap();
    // A sample: the same time at any size.
    check.budget("column types", t.elapsed(), Duration::from_millis(500));
    assert!(types.len() >= COLS);
    let t = Instant::now();
    let chart = doc
        .chart_data(
            &ChartSpec {
                kind: ChartKind::Line,
                label: Some(0),
                values: vec![2, 3],
                skip_first: true,
                read,
            },
            &p,
        )
        .unwrap();
    check.budget("line chart, 2 series", t.elapsed(), budget(2.0));
    assert!(chart.labels.len() <= 2_000);
    let t = Instant::now();
    doc.chart_data(
        &ChartSpec {
            kind: ChartKind::Count,
            label: Some(4),
            values: vec![],
            skip_first: true,
            read,
        },
        &p,
    )
    .unwrap();
    check.budget("count chart", t.elapsed(), budget(2.0));
    let plan = doc.plan_sort(&key(4, SortKind::Text), &p).unwrap();
    doc.apply_sort(plan).unwrap();
    let sub = SubtotalSpec {
        by: 4,
        cols: vec![2],
        total: Total::Sum,
        keep_first: true,
        read,
    };
    // A copy of a 2 GB file is over the 1 GB new-table cap: it must be
    // refused, and quickly (chunks stop once the cap is passed).
    let t = Instant::now();
    match doc.subtotals(&sub, &p) {
        Ok(table) => drop(table),
        Err(e) => assert!(e.to_string().contains("too large"), "{e}"),
    }
    check.budget(
        "subtotals, 311 groups (or refused over the cap)",
        t.elapsed(),
        budget(5.0),
    );
    drop(doc);
    let _ = fs::remove_file(&out4);

    // Export, import, join and append on a tenth of the rows (1M at 10M).
    let small_rows = (rows / 10).max(1);
    let small_path = dir.join(format!("fixture-{small_rows}.csv"));
    generate(&small_path, small_rows).unwrap();
    let mut small = Document::open_indexed(&small_path, OpenOptions::default(), None).unwrap();
    let xlsx = dir.join(format!("export-{small_rows}.xlsx"));
    let t = Instant::now();
    let r = small
        .export_file(
            &xlsx,
            &ExportOptions {
                format: ExportFormat::Xlsx,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    let first = t.elapsed();
    // The best of three (the first run also reads the file in): a busy moment of the
    // machine cannot fail the gate; the first run is printed too.
    let mut took = first;
    for _ in 0..2 {
        let t = Instant::now();
        small
            .export_file(
                &xlsx,
                &ExportOptions {
                    format: ExportFormat::Xlsx,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        took = took.min(t.elapsed());
    }
    println!(
        "     export .xlsx: {} rows, {:.0} MB, first run {:?}",
        r.rows,
        r.bytes as f64 / 1e6,
        first
    );
    check.budget("export .xlsx (1M rows)", took, budget(3.0));
    let data = fs::read(&xlsx).unwrap();
    let t = Instant::now();
    let table = import::import(&data, Some("xlsx"), &Default::default(), None, &p).unwrap();
    check.budget("import that .xlsx", t.elapsed(), budget(20.0));
    assert_eq!(table.rows, small_rows + 1);
    drop(table);
    drop(data);
    let json = dir.join(format!("export-{small_rows}.json"));
    let t = Instant::now();
    small
        .export_file(&json, &ExportOptions::default(), &p)
        .unwrap();
    check.budget("export JSON (1M rows)", t.elapsed(), budget(10.0));
    let tenth = (small_rows / 10).max(1);
    let other_path = dir.join(format!("fixture-{tenth}.csv"));
    generate(&other_path, tenth).unwrap();
    let other = Document::open_indexed(&other_path, OpenOptions::default(), None).unwrap();
    let spec = JoinSpec {
        keys: vec![0],
        other_keys: vec![0],
        columns: vec![1, 2],
        keep_first: true,
        other_header: true,
        match_case: true,
        ..Default::default()
    };
    let t = Instant::now();
    let src = JoinSource::build(&other, &spec, &p).unwrap();
    let (plan, rep) = small.plan_join(&src, &spec, &p).unwrap();
    small.apply_join(plan).unwrap();
    check.budget("join 1M rows to 100k by one key", t.elapsed(), budget(5.0));
    assert_eq!(rep.matched, tenth);
    // Milestone 11: the upgraded joins on the same tables.
    for (name, s) in [
        (
            "join, loose keys + not-found text",
            JoinSpec {
                loose: true,
                match_case: false,
                not_found: "none".into(),
                ..spec.clone()
            },
        ),
        (
            "join, update 2 columns",
            JoinSpec {
                columns: vec![2, 1],
                write: JoinWrite::Overwrite,
                targets: vec![20, 21],
                ..spec.clone()
            },
        ),
        (
            "join, keep matched rows only",
            JoinSpec {
                keep_rows: KeepRows::Matched,
                ..spec.clone()
            },
        ),
        (
            "join, all matches in one cell",
            JoinSpec {
                repeats: natendo_engine::join::Repeats::Joined,
                separator: "; ".into(),
                ..spec.clone()
            },
        ),
        (
            "join, nearest smaller key",
            JoinSpec {
                key_match: KeyMatch::Below,
                ..spec.clone()
            },
        ),
    ] {
        let t = Instant::now();
        let src = JoinSource::build(&other, &s, &p).unwrap();
        let (plan, _) = small.plan_join(&src, &s, &p).unwrap();
        small.apply_join(plan).unwrap();
        check.budget(&format!("{name} (1M rows)"), t.elapsed(), budget(5.0));
        assert!(small.undo());
    }
    let t = Instant::now();
    let src = JoinSource::build(&other, &spec, &p).unwrap();
    let pv = small.preview_join(&src, &spec, &p).unwrap();
    check.budget("join preview (1M rows)", t.elapsed(), budget(3.0));
    assert_eq!(pv.report.matched, tenth);
    let aspec = AppendSpec {
        by_name: true,
        keep_first: true,
        other_header: true,
        add_columns: true,
    };
    let big_other = Document::open_indexed(&small_path, OpenOptions::default(), None).unwrap();
    let t = Instant::now();
    let src = AppendSource::build(&big_other, &aspec, &p).unwrap();
    let (plan, _) = small.plan_append(&src, &aspec).unwrap();
    small.apply_cells(plan).unwrap();
    check.budget("append 1M rows", t.elapsed(), budget(5.0));

    // Milestone 6: cleaners and fill by example, on 1M rows.
    let mut m6 = Document::open_indexed(&small_path, OpenOptions::default(), None).unwrap();
    let run = |d: &mut Document, steps: Vec<Step>| {
        let spec = CleanSpec {
            cleaner: Cleaner {
                version: 1,
                name: String::new(),
                steps,
            },
            scope: Scope::Columns { cols: vec![1] },
            skip_first: true,
            read: Default::default(),
        };
        let plan = d
            .plan_clean(&spec, 20_000.0, 1, 0, &Progress::new())
            .unwrap();
        let n = plan.changed;
        d.apply_clean(plan).unwrap();
        n
    };
    let t = Instant::now();
    let changed = run(&mut m6, vec![Step::Case { to: CaseTo::Upper }]);
    check.budget(
        "one-step cleaner (capitals), 1M cells changed",
        t.elapsed(),
        budget(1.5),
    );
    assert_eq!(changed, small_rows);
    let t = Instant::now();
    let changed = run(
        &mut m6,
        vec![
            Step::Spaces {
                how: Spaces::Collapse,
            },
            Step::Case { to: CaseTo::Title },
            Step::Replace {
                find: Pattern {
                    text: r"(\d+)".into(),
                    kind: PatternKind::Regex,
                    ..Default::default()
                },
                with: "#$1".into(),
            },
            Step::Pad {
                width: 12,
                with: '.',
                right: true,
            },
            Step::Reverse,
        ],
    );
    check.budget("five-step cleaner, 1M cells", t.elapsed(), budget(3.0));
    assert_eq!(changed, small_rows);
    drop(m6);
    // Fill by example: 1M names, three examples typed.
    let mut text = String::from("first,last,full\r\n");
    for i in 0..small_rows {
        let full = if i < 3 {
            format!("Last{i}, F")
        } else {
            String::new()
        };
        text.push_str(&format!("First{i},Last{i},{full}\r\n"));
    }
    let mut ex =
        Document::from_bytes(text.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
    let t = Instant::now();
    let (plan, rep) = ex
        .plan_fill_example(
            &ExampleSpec {
                col: 2,
                skip_first: true,
            },
            &p,
        )
        .unwrap();
    ex.apply_cells(plan).unwrap();
    check.budget("fill by example, 1M rows", t.elapsed(), budget(1.5));
    assert_eq!(rep.filled, small_rows - 3);
    drop(ex);
    // Subtotals of a table under the cap (every row copied, plus totals).
    let plan = small.plan_sort(&key(4, SortKind::Text), &p).unwrap();
    small.apply_sort(plan).unwrap();
    let t = Instant::now();
    let table = small
        .subtotals(
            &SubtotalSpec {
                by: 4,
                cols: vec![2],
                total: Total::Sum,
                keep_first: true,
                read: ReadOptions::default(),
            },
            &p,
        )
        .unwrap();
    check.budget(
        "subtotals, a tenth of the rows in 311 groups",
        t.elapsed(),
        Duration::from_millis(500).max(budget(0.5)),
    );
    assert!(table.rows > small_rows);
    drop(table);
    // Milestone 8: PDF and fixed-width export of the tenth-size table.
    let pdf_rows = small_rows.min(100_000);
    let pdf = dir.join("perf-out.pdf");
    let t = Instant::now();
    let mut o = ExportOptions {
        format: ExportFormat::Pdf,
        ..Default::default()
    };
    o.page.rows = Some((1, pdf_rows));
    let r = small.export_file(&pdf, &o, &p).unwrap();
    check.budget(
        "PDF export, 100,000 rows",
        t.elapsed(),
        Duration::from_secs_f64(8.0 * (pdf_rows as f64 / 1e5)).max(Duration::from_millis(200)),
    );
    assert!(r.sheets > 1);
    let _ = fs::remove_file(&pdf);
    let fixed = dir.join("perf-out.txt");
    // The tenth-size table holds about twice its rows by now (append).
    let fixed_rows = small.row_count();
    let per_m = |secs: f64| {
        Duration::from_secs_f64(secs * fixed_rows as f64 / 1e6).max(Duration::from_millis(100))
    };
    let t = Instant::now();
    small
        .export_file(
            &fixed,
            &ExportOptions {
                format: ExportFormat::FixedWidth,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    check.budget(
        "fixed-width export, 3 s per million rows",
        t.elapsed(),
        per_m(3.0),
    );
    let t = Instant::now();
    let bytes = fs::read(&fixed).unwrap();
    let opts = import::ImportOptions {
        format: Some(import::ImportFormat::FixedWidth),
        ..Default::default()
    };
    let table = import::import(&bytes, Some("txt"), &opts, None, &p).unwrap();
    check.budget(
        "fixed-width import, 4 s per million rows",
        t.elapsed(),
        per_m(4.0),
    );
    assert_eq!(table.rows, fixed_rows);
    drop(table);
    drop(bytes);
    let _ = fs::remove_file(&fixed);
    drop(small);
    drop(big_other);
    drop(other);
    // Milestone 8 on the whole file: computed column and data checks.
    let mut big = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let t = Instant::now();
    let (plan, rep) = big
        .plan_compute(
            &ComputeSpec {
                expr: "ROUND([col2] * 1.07, 2)".into(),
                title: "with tax".into(),
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    big.apply_cols(plan).unwrap();
    check.budget("computed column, every row", t.elapsed(), budget(8.0));
    assert_eq!(rep.errors, 0);
    // v0.4.2: the new functions, the rows around (PREV), filter by formula
    // and cell formulas.
    {
        use natendo_engine::cellformula::CellFormula;
        // Measured against a plain computed column making the same number
        // of cells (writing them is most of the time): at most 30% more.
        let time = |big: &mut Document, expr: &str| {
            let t = Instant::now();
            let (plan, _) = big
                .plan_compute(
                    &ComputeSpec {
                        expr: expr.into(),
                        title: "x".into(),
                        keep_first: true,
                        ..Default::default()
                    },
                    &p,
                )
                .unwrap();
            big.apply_cols(plan).unwrap();
            let took = t.elapsed();
            big.undo();
            took
        };
        let plain = time(&mut big, "[col2] - 5");
        println!(
            "     plain computed column for comparison: {:.1} s",
            plain.as_secs_f64()
        );
        let limit = plain.mul_f64(1.3);
        let took = time(&mut big, "TEXT([col2], \"#,##0.00\")");
        check.budget("computed column with TEXT, every row", took, limit);
        let took = time(&mut big, "[col2] - PREV([col2])");
        check.budget("computed column with PREV, every row", took, limit);
        // Let go of the undone column kept for redo now (freeing 10M cells
        // takes seconds), so the next timed step does not pay for it.
        big.hide_rows(1, 1).unwrap();
        big.undo();
        let t = Instant::now();
        let res = big
            .run_cell_formula(
                &CellFormula {
                    formula: "=SUM(C2:C1000001)".into(),
                    row: 1,
                    col: 0,
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(res.cells.len(), 1);
        check.budget(
            "cell formula summing 1M cells",
            t.elapsed(),
            Duration::from_millis(400),
        );
        // Two whole columns (row 2 to the last row): running totals only, no
        // text kept. Sized from the file, so a 1M-row run does not reach past its end.
        let last = big.row_count();
        let t = Instant::now();
        let res = big
            .run_cell_formula(
                &CellFormula {
                    formula: format!("=SUM(C2:D{last})"),
                    row: 1,
                    col: 0,
                    keep_first: true,
                    ..Default::default()
                },
                &Progress::new(),
            )
            .unwrap();
        assert_eq!(res.cells.len(), 1);
        let summed = 2 * last.saturating_sub(1);
        check.budget(
            &format!(
                "cell formula summing {summed} cells (2 columns of {} rows)",
                last.saturating_sub(1)
            ),
            t.elapsed(),
            Duration::from_secs_f64((summed as f64 / 1e7).max(0.05)),
        );
        // A 200-cell range filled down 100k cells, and a running total down
        // 100k rows: each range is worked out from running totals.
        let fill = |f: &str| CellFormula {
            formula: f.into(),
            row: 1,
            col: 0,
            targets: (1..=100_000).map(|r| (r, 0)).collect(),
            keep_first: true,
            ..Default::default()
        };
        let t = Instant::now();
        let res = big
            .run_cell_formula(&fill("=SUM(C2:C201)"), &Progress::new())
            .unwrap();
        assert_eq!(res.cells.len(), 100_000);
        check.budget(
            "cell formula SUM(C2:C201) filled down 100k cells",
            t.elapsed(),
            Duration::from_millis(600),
        );
        let t = Instant::now();
        let res = big
            .run_cell_formula(&fill("=SUM($C$2:C2)"), &Progress::new())
            .unwrap();
        assert_eq!(res.cells.len(), 100_000);
        check.budget(
            "running total SUM($C$2:C2) filled down 100k cells",
            t.elapsed(),
            Duration::from_millis(600),
        );
    }
    // v0.4.3: exploring a file. Counts of a column, a column profile, the
    // filter panel's recount and the insight strip, on 10M rows.
    {
        use natendo_engine::counts::CountsSpec;
        use natendo_engine::filter::{FacetCard, FacetKind};
        use natendo_engine::insight::InsightSpec;
        use natendo_engine::profile::ProfileSpec;
        let t = Instant::now();
        let (table, _) = big
            .counts_table(
                &CountsSpec {
                    cols: vec![4],
                    keep_first: true,
                },
                &Progress::new(),
            )
            .unwrap();
        drop(table);
        check.budget(
            "counts tab of a 311-value column",
            t.elapsed(),
            Duration::from_secs(3),
        );
        // Every value different (an ID column): all of them counted.
        let t = Instant::now();
        let c = big
            .value_counts(
                &CountsSpec {
                    cols: vec![0],
                    keep_first: true,
                },
                &Progress::new(),
            )
            .unwrap();
        println!("     counts of an ID column: {} lines", c.lines.len());
        drop(c);
        check.budget(
            "counts tab of an all-different column",
            t.elapsed(),
            Duration::from_secs(6),
        );
        let t = Instant::now();
        big.column_profile(
            &ProfileSpec {
                col: 2,
                keep_first: true,
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap();
        check.budget(
            "column profile of a number column",
            t.elapsed(),
            Duration::from_secs(4),
        );
        let cards = [
            FacetCard {
                col: 4,
                kind: FacetKind::Values,
                range: None,
                search: None,
            },
            FacetCard {
                col: 2,
                kind: FacetKind::Numbers,
                range: Some((0.0, 100_000.0)),
                search: None,
            },
        ];
        let spec = FilterSpec {
            keep_first: true,
            ..Default::default()
        };
        // The first count (cards just added) and a recount.
        let t = Instant::now();
        big.facet_counts(&cards, &spec, &Progress::new()).unwrap();
        check.budget(
            "filter panel: two cards counted",
            t.elapsed(),
            Duration::from_secs(3),
        );
        // A few cards, then each click counts again from the cached codes.
        let card = |col: u32, kind: FacetKind| FacetCard {
            col,
            kind,
            range: None,
            search: None,
        };
        let four = vec![
            card(4, FacetKind::Values),
            card(5, FacetKind::Dates),
            card(6, FacetKind::Values),
            card(3, FacetKind::Values),
        ];
        let mut eight = four.clone();
        eight.extend([
            card(1, FacetKind::Values),
            card(2, FacetKind::Numbers),
            card(7, FacetKind::Values),
            card(8, FacetKind::Values),
        ]);
        let click = FilterSpec {
            keep_first: true,
            columns: vec![natendo_engine::filter::ColumnFilter {
                col: 4,
                rule: Rule::Values {
                    values: vec!["City 7".into(), "City 8".into()],
                    blanks: false,
                },
            }],
            ..Default::default()
        };
        for (name, cards) in [("4", &four), ("8", &eight)] {
            let mut cache = natendo_engine::filter::FacetCache::new();
            let t = Instant::now();
            big.facet_counts_cached(&mut cache, cards, &spec, &Progress::new())
                .unwrap();
            check.budget(
                &format!("filter panel: {name} cards, first count (columns read once)"),
                t.elapsed(),
                Duration::from_secs(6),
            );
            let t = Instant::now();
            big.facet_counts_cached(&mut cache, cards, &click, &Progress::new())
                .unwrap();
            check.budget(
                &format!("filter panel: {name} cards, each click"),
                t.elapsed(),
                Duration::from_millis(300),
            );
            if name == "8" {
                // Three of the eight are ID-like (a number with 10M values
                // and two 1M-value columns). A bar picked on the number
                // column is worked out once; later clicks stay quick.
                let mut bar = click.clone();
                bar.columns.push(natendo_engine::filter::ColumnFilter {
                    col: 2,
                    rule: Rule::Conditions {
                        first: natendo_engine::filter::Condition::Number {
                            op: natendo_engine::filter::CmpOp::Lt,
                            value: 5000.0,
                        },
                        second: None,
                        or: false,
                    },
                });
                let t = Instant::now();
                big.facet_counts_cached(&mut cache, cards, &bar, &Progress::new())
                    .unwrap();
                check.budget(
                    "filter panel: 8 cards, a bar picked on an ID-like column",
                    t.elapsed(),
                    Duration::from_secs(2),
                );
                bar.columns[0].rule = Rule::Values {
                    values: vec!["City 9".into()],
                    blanks: false,
                };
                let t = Instant::now();
                big.facet_counts_cached(&mut cache, cards, &bar, &Progress::new())
                    .unwrap();
                check.budget(
                    "filter panel: 8 cards, a click after it",
                    t.elapsed(),
                    Duration::from_millis(300),
                );
                // A typed cell: only that row is read again.
                big.set_cell(5, 4, "City 9".into()).unwrap();
                let t = Instant::now();
                big.facet_counts_cached(&mut cache, cards, &bar, &Progress::new())
                    .unwrap();
                check.budget(
                    "filter panel: 8 cards, count after an edit",
                    t.elapsed(),
                    Duration::from_millis(300),
                );
                big.undo();
                println!("     filter panel cache: {} MB", cache.bytes() >> 20);
            }
        }
        let t = Instant::now();
        let n = big.col_count();
        let r = big
            .column_insights(&InsightSpec {
                cols: (0..n).collect(),
                keep_first: true,
                sample: 100_000,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(r.cols.len(), n as usize);
        check.budget(
            "column insight strip (100k-row sample, every column)",
            t.elapsed(),
            Duration::from_millis(400),
        );
        let t = Instant::now();
        big.column_insights(&InsightSpec {
            cols: (0..n).collect(),
            keep_first: true,
            sample: 5_000,
            ..Default::default()
        })
        .unwrap();
        check.budget(
            "column insight strip (quick sample)",
            t.elapsed(),
            Duration::from_millis(60),
        );
    }
    let checks = |kinds: Vec<(u32, CheckKind)>| CheckSpec {
        checks: kinds
            .into_iter()
            .map(|(c, kind)| DataCheck {
                name: None,
                col: Some(c),
                kind,
            })
            .collect(),
        keep_first: true,
        ..Default::default()
    };
    let t = Instant::now();
    let r = big
        .run_checks(
            &checks(vec![
                (
                    2,
                    CheckKind::Number {
                        min: Some(0.0),
                        max: Some(50_000.0),
                        whole: false,
                    },
                ),
                (6, CheckKind::Required),
                (
                    5,
                    CheckKind::Date {
                        min: Some("2026-01-01".into()),
                        max: None,
                    },
                ),
            ]),
            &p,
        )
        .unwrap();
    check.budget(
        "data checks (number, required, date), every row",
        t.elapsed(),
        budget(3.0),
    );
    assert!(r.rows > 0);
    let t = Instant::now();
    let r = big
        .run_checks(
            &checks(vec![(
                4,
                CheckKind::Unique {
                    match_case: false,
                    also: vec![],
                },
            )]),
            &p,
        )
        .unwrap();
    check.budget(
        "data check (no repeats), every row",
        t.elapsed(),
        budget(4.5),
    );
    assert!(r.rows > 0);
    // Milestone 15: patterns on two columns (check fields), then failing
    // rows first.
    let pat = |text: &str| CheckKind::Pattern {
        pattern: Pattern {
            text: text.into(),
            kind: PatternKind::Regex,
            match_case: false,
            whole_cell: true,
            ..Default::default()
        },
    };
    let spec = checks(vec![(3, pat(r"v\d{1,5}")), (4, pat(r"v[0-4]\d*"))]);
    let t = Instant::now();
    let r = big.run_checks(&spec, &p).unwrap();
    check.budget(
        "check fields: patterns on 2 columns, every row",
        t.elapsed(),
        budget(5.0),
    );
    assert!(r.rows > 0);
    let t = Instant::now();
    let plan = big.plan_sort_problems(&r, true).unwrap();
    big.apply_sort(plan).unwrap();
    check.budget(
        "put rows that fail first (after the check)",
        t.elapsed(),
        budget(3.0),
    );

    // Milestone 15: street range lookup for every row (the "street" is
    // the city column, the house number the id) from 31,100 ranges.
    {
        use natendo_engine::streets::{IndexCols, StreetIndex, StreetSpec};
        let mut idx = String::from("street,from,to,side,precinct\r\n");
        for c in 0..311 {
            for k in 0..100u64 {
                let lo = k * 100_000;
                idx.push_str(&format!(
                    "City {c},{lo},{},{},P{c}-{k}\r\n",
                    lo + 99_999,
                    if k % 2 == 0 { "both" } else { "" }
                ));
            }
        }
        let idx =
            Document::from_bytes(idx.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let t = Instant::now();
        let index = StreetIndex::build(
            &idx,
            &IndexCols {
                street: 0,
                low: 1,
                high: 2,
                side: Some(3),
                area: None,
                bring: vec![4],
                header: true,
            },
            &p,
        )
        .unwrap();
        let (plan, rep) = big
            .plan_street_lookup(
                &index,
                &StreetSpec {
                    address: 4,
                    number: Some(0),
                    area: None,
                    keep_first: true,
                    note: false,
                },
                &p,
            )
            .unwrap();
        drop(plan);
        check.budget(
            "street range lookup, every row, 31,100 ranges (plan)",
            t.elapsed(),
            budget(15.0),
        );
        assert!(rep.found > 0 && rep.no_street == 0, "{rep:?}");
    }

    // Milestone 9 on the whole file: quick totals, hiding rows, shuffling,
    // clean-up suggestions, a recovery-style copy.
    let rows = big.row_count();
    let t = Instant::now();
    let tot = big.selection_totals(1, rows - 1, 2, 1, read, &p).unwrap();
    check.budget("quick totals, one whole column", t.elapsed(), budget(1.0));
    assert!(tot.numbers > 0);
    let t = Instant::now();
    big.selection_totals(1, rows - 1, 0, COLS as u32, read, &p)
        .unwrap();
    check.budget("quick totals, every cell", t.elapsed(), budget(6.0));
    multi_area_checks(&big, rows, read, &p, &mut check);
    let t = Instant::now();
    big.hide_rows(1, rows / 2).unwrap();
    let shown = big.row_count();
    check.budget(
        "hide half the rows (and count again)",
        t.elapsed(),
        budget(1.0),
    );
    assert_eq!(shown, rows - rows / 2);
    let t = Instant::now();
    big.show_rows(natendo_engine::rows::ShowRows::All).unwrap();
    assert_eq!(big.row_count(), rows);
    check.budget("show every hidden row", t.elapsed(), budget(1.0));
    let t = Instant::now();
    let plan = big.plan_shuffle(1, rows - 1, 7, &p).unwrap();
    big.apply_shuffle(plan).unwrap();
    check.budget("shuffle every row", t.elapsed(), budget(3.0));
    let t = Instant::now();
    let s = big.suggestions(true, read, &p).unwrap();
    check.budget(
        "clean-up suggestions (first 2,000,000 rows)",
        t.elapsed(),
        // 4 s for the 2,000,000 rows it reads at 10M; a smaller file has fewer rows to read.
        Duration::from_secs_f64(4.0 * rows.min(2_000_000) as f64 / 2_000_000.0),
    );
    assert!(s.rows > 0);
    // A recovery copy stops at 200 MB (the app's cap); time the first
    // 200 MB of a copy of the shuffled file, the worst case (rows read out
    // of file order).
    struct Upto(u64);
    impl std::io::Write for Upto {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if b.len() as u64 > self.0 {
                return Err(std::io::Error::other("cap"));
            }
            self.0 -= b.len() as u64;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let t = Instant::now();
    // (A small file is smaller than the cap: half of it then, so the cap still bites.)
    let cap = (200u64 << 20).min(size / 2);
    let r = big.write_copy(&mut Upto(cap));
    check.budget(
        "recovery copy up to its 200 MB cap, shuffled rows",
        t.elapsed(),
        // A background copy (any change stops it): its time varies with
        // the disk, 4-6 s here, so the budget leaves room.
        budget(8.0),
    );
    assert!(r.is_err(), "the cap stops the copy");

    // Milestone 10 on the whole (shuffled) file.
    use natendo_engine::tools::{CompareSpec, FillBlanks, FillFrom, Repeats, Sample, SampleSize};
    let t = Instant::now();
    let plan = big
        .plan_fill_blanks(
            &FillBlanks {
                cols: vec![2],
                from: FillFrom::Above,
                spaces: true,
                keep_first: true,
            },
            &p,
        )
        .unwrap();
    check.budget(
        "fill blanks, one column (none blank)",
        t.elapsed(),
        budget(3.0),
    );
    assert_eq!(plan.rows, 0);
    let t = Instant::now();
    let plan = big
        .plan_count_repeats(
            &Repeats {
                cols: vec![4],
                trim: true,
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    check.budget("count repeats, one column (plan)", t.elapsed(), budget(6.0));
    drop(plan);
    let t = Instant::now();
    let plan = big
        .plan_blank_repeats(
            &natendo_engine::reshape::Duplicates {
                cols: vec![4],
                trim: true,
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    check.budget("blank repeats, one column (plan)", t.elapsed(), budget(6.0));
    drop(plan);
    let t = Instant::now();
    let top = FilterSpec {
        columns: vec![ColumnFilter {
            col: 2,
            rule: Rule::Conditions {
                first: Condition::Top {
                    count: 10.0,
                    percent: false,
                    bottom: false,
                },
                second: None,
                or: false,
            },
        }],
        keep_first: true,
        read,
    };
    let shown = big.set_filter(&top).unwrap();
    check.budget("filter: 10 largest numbers", t.elapsed(), budget(2.0));
    assert!(shown >= 11);
    big.set_filter(&FilterSpec::default()).unwrap();
    let t = Instant::now();
    let r = big.number_range(2, true, read, &p).unwrap();
    check.budget(
        "number range of a column (scales, bars)",
        t.elapsed(),
        budget(1.0),
    );
    assert!(r.is_some());
    let t = Instant::now();
    big.find_extreme(2, true, true, read, &p).unwrap().unwrap();
    check.budget("go to the largest number", t.elapsed(), budget(1.0));
    let t = Instant::now();
    let v = big.typeahead_values(4, true, 2_000_000, 50_000).unwrap();
    check.budget(
        "type-ahead values (first 2,000,000 rows)",
        t.elapsed(),
        budget(1.0),
    );
    assert!(v.len() > 100);
    let t = Instant::now();
    let s = big
        .sample(
            &Sample {
                size: SampleSize::Rows { n: 100_000 },
                seed: 3,
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    check.budget("random sample of 100,000 rows", t.elapsed(), budget(3.0));
    assert_eq!(s.rows, 100_001);
    drop(s);
    // An audit sample: 10 rows from each of the 311 cities, with row numbers.
    let t = Instant::now();
    let (s, r) = big
        .sample_report(
            &Sample {
                size: SampleSize::Rows { n: 10 },
                seed: 2024,
                keep_first: true,
                per: Some(4),
                row_numbers: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    check.budget(
        "audit sample: 10 rows from each city",
        t.elapsed(),
        budget(2.0),
    );
    assert_eq!((s.rows, r.groups.len()), (3_111, 311));
    drop(s);
    // Excel export of the whole table: parts as files of their own, each
    // with the title row.
    let xdir = dir.join("xlsx-parts");
    let _ = fs::create_dir_all(&xdir);
    // A file of 1,048,575 rows or fewer is one workbook, not parts: for a smaller run
    // the table is cut into ten parts, as the 10M run is, so the parts path is what is timed.
    let part_rows = if rows > 1_048_575 {
        0
    } else {
        (rows / 10).max(1)
    };
    let t = Instant::now();
    let r = big
        .export_file(
            &xdir.join("big.xlsx"),
            &ExportOptions {
                format: ExportFormat::Xlsx,
                split_files: true,
                part_rows,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    let in_process = t.elapsed();
    println!(
        "     export .xlsx parts: {} rows in {} file(s), {:.0} MB, {:?} inside this long run",
        r.rows,
        r.files.len().max(1),
        r.bytes as f64 / 1e6,
        in_process
    );
    // The gate: the best of three of the same export in a process of its own (the
    // xlsx_bench example), because this process, hours into its checks, runs the same
    // export about twice as slowly at 1M rows (the heap and thread state of the run, not
    // the export: measured both ways). Without the example built, the time in this run counts.
    let own = xlsx_in_own_process(rows, &dir, part_rows);
    if let Some(d) = own {
        println!("     the same export in its own process (best of 3): {d:?}");
    }
    check.budget(
        "export .xlsx of every row, in parts (files)",
        own.unwrap_or(in_process),
        budget(12.0),
    );
    let _ = fs::remove_dir_all(&xdir);
    let t = Instant::now();
    let custom = SortSpec {
        keys: vec![SortKey {
            col: 4,
            descending: false,
            kind: SortKind::List(0),
            match_case: false,
        }],
        keep_first: true,
        read,
        lists: vec![(0..50).map(|i| format!("City {i}")).collect()],
    };
    big.sort(&custom).unwrap();
    check.budget(
        "sort by a custom order (50 values)",
        t.elapsed(),
        budget(6.0),
    );
    let t = Instant::now();
    let by_len = SortSpec {
        keys: vec![SortKey {
            col: 3,
            descending: true,
            kind: SortKind::Length,
            match_case: false,
        }],
        keep_first: true,
        read,
        lists: Vec::new(),
    };
    big.sort(&by_len).unwrap();
    check.budget("sort by length of the text", t.elapsed(), budget(5.0));
    let t = Instant::now();
    let accents = CleanSpec {
        cleaner: Cleaner {
            version: 1,
            name: String::new(),
            steps: vec![
                Step::Accents,
                Step::Calc {
                    op: natendo_engine::clean::CalcOp::Multiply,
                    value: 1.1,
                },
            ],
        },
        scope: Scope::Columns { cols: vec![2] },
        skip_first: true,
        read,
    };
    let plan = big.plan_clean(&accents, 20_000.0, 1, 0, &p).unwrap();
    check.budget(
        "clean: accents + multiply, one column (plan)",
        t.elapsed(),
        budget(6.0),
    );
    drop(plan);
    // Two clean copies of the file (big has a computed column by now).
    drop(big);
    let first = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let other = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let t = Instant::now();
    let (table, rep) = first
        .compare(
            &other,
            &CompareSpec {
                keys: vec![0],
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    check.budget(
        "compare two 10M-row files by key",
        t.elapsed(),
        budget(25.0),
    );
    println!(
        "  compare: {} same, {} changed, {} added, {} removed",
        rep.same, rep.changed, rep.added, rep.removed
    );
    assert_eq!(rep.same + 1, rows);
    drop(table);
    drop(first);
    drop(other);

    // Batch 6: group look-alikes (by sound, spelling and both) among 200,000
    // distinct made-up names with varied lengths and a few near-duplicates.
    {
        use natendo_engine::editing::Tally;
        use natendo_engine::lookalike::{group_lookalikes, Method};
        const SYL: [&str; 24] = [
            "ma", "ri", "an", "son", "ber", "ton", "el", "la", "ke", "dro", "vin", "sha", "ne",
            "ha", "li", "or", "ka", "tho", "wen", "pe", "sul", "mi", "gar", "den",
        ];
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut seen = std::collections::HashSet::new();
        let mut vals: Vec<Tally> = Vec::with_capacity(200_000);
        while vals.len() < 200_000 {
            let n = 2 + (rnd() % 4) as usize;
            let mut name = String::new();
            for _ in 0..n {
                name.push_str(SYL[(rnd() % 24) as usize]);
            }
            let mut c = name.chars();
            let name = c.next().unwrap().to_uppercase().collect::<String>() + c.as_str();
            if seen.insert(name.clone()) {
                // A long tail: a few common names, most seen once or twice.
                let count = 1 + (rnd() % 2) + if rnd() % 500 == 0 { 40 } else { 0 };
                vals.push(Tally { value: name, count });
            }
        }
        vals.sort_by_key(|a| std::cmp::Reverse(a.count));
        for (label, method) in [
            ("sounds", Method::Sounds),
            ("spelling", Method::Spelling),
            ("sound or spelling", Method::Both),
        ] {
            let t = Instant::now();
            let g = group_lookalikes(&vals, method, &p).unwrap();
            let took = t.elapsed();
            // 3 s per 200,000 names (the module's own gate), tied to the number of names
            // and not to the size of the file; the speed is printed too.
            let limit = Duration::from_secs_f64(3.0 * vals.len() as f64 / 200_000.0);
            check.budget(
                &format!(
                    "look-alike groups by {label} ({} distinct names, {:.0} names/s)",
                    vals.len(),
                    vals.len() as f64 / took.as_secs_f64().max(1e-9)
                ),
                took,
                limit,
            );
            println!(
                "     {} groups, {} crowded forms skipped",
                g.groups.len(),
                g.crowded
            );
        }
    }

    // Milestone 12: everyday editing tools.
    {
        use natendo_engine::editing::{
            Arrange, Extract, ExtractWhat, GroupCalc, GroupFunc, MapValues, TallySpec,
        };
        use natendo_engine::reshape::{Duplicates, KeepCopy};
        let tall = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let t = Instant::now();
        let tally = tall
            .value_tally(
                &TallySpec {
                    col: 4,
                    keep_first: true,
                    limit: None,
                },
                &p,
            )
            .unwrap();
        check.budget("value list of a 10M-row column", t.elapsed(), budget(3.0));
        println!("     {} distinct values", tally.values.len());
        drop(tall);
        let mut m12 = Document::open_indexed(&small_path, OpenOptions::default(), None).unwrap();
        let pairs: Vec<(String, String)> = tally
            .values
            .iter()
            .map(|v| (v.value.clone(), v.value.to_uppercase()))
            .collect();
        let t = Instant::now();
        let plan = m12
            .plan_map_values(
                &MapValues {
                    cols: vec![4],
                    pairs,
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        m12.apply_cells(plan).unwrap();
        check.budget(
            "rename every value of a column (1M rows)",
            t.elapsed(),
            budget(5.0),
        );
        m12.undo();
        let t = Instant::now();
        let (plan, _) = m12
            .plan_group_calc(
                &GroupCalc {
                    keys: vec![4],
                    value: Some(2),
                    func: GroupFunc::Rank,
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        m12.apply_cols(plan).unwrap();
        check.budget("rank within groups (1M rows)", t.elapsed(), budget(5.0));
        m12.undo();
        let t = Instant::now();
        let plan = m12
            .plan_extract(
                &Extract {
                    col: 3,
                    what: ExtractWhat::Number,
                    remove: true,
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        m12.apply_cols(plan).unwrap();
        check.budget(
            "extract a number into a column (1M rows)",
            t.elapsed(),
            budget(5.0),
        );
        m12.undo();
        let t = Instant::now();
        let order: Vec<u32> = (0..m12.col_count()).rev().step_by(2).collect();
        let names = vec![String::new(); order.len()];
        let plan = m12
            .plan_arrange_columns(&Arrange {
                order,
                names,
                keep_first: true,
            })
            .unwrap();
        m12.apply_cols(plan).unwrap();
        check.budget(
            "choose and order columns (1M rows)",
            t.elapsed(),
            budget(1.0),
        );
        m12.undo();
        let t = Instant::now();
        let plan = m12
            .plan_remove_duplicates(
                &Duplicates {
                    cols: vec![1],
                    keep_first: true,
                    keep: KeepCopy::Newest,
                    date_col: Some(5),
                    fill_blanks: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        m12.apply_rows(plan).unwrap();
        check.budget(
            "remove duplicates keeping the newest, filling blanks (1M rows)",
            t.elapsed(),
            budget(10.0),
        );
    }

    // Milestone 13: the private data finder and a share-safe copy of every
    // row (one column masked, one scrambled, one as an age band, one left
    // out).
    {
        use natendo_engine::clean::DateTo;
        use natendo_engine::privacy::{ColumnAction, SafeCopy, Secret};
        let d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let t = Instant::now();
        d.find_private(true, ReadOptions::default(), 20_770.0, &p)
            .unwrap();
        check.budget(
            "find private data (first 20,000 rows)",
            t.elapsed(),
            Duration::from_secs(1),
        );
        let clean = |step: Step| ColumnAction::Clean {
            cleaner: Cleaner {
                version: 1,
                name: String::new(),
                steps: vec![step],
            },
        };
        let spec = SafeCopy {
            guard: false,
            columns: vec![
                ColumnAction::Keep,
                clean(Step::Mask {
                    keep_last: 4,
                    with: '*',
                    all: false,
                }),
                clean(Step::Scramble {
                    phrase: Secret::new("a long secret phrase"),
                    loose: true,
                }),
                clean(Step::DateBand {
                    to: DateTo::AgeBand { size: 10, top: 90 },
                }),
                ColumnAction::Drop,
            ],
            header: true,
            read: ReadOptions::default(),
        };
        let t = Instant::now();
        let rep = d
            .write_safe_copy(
                &mut std::io::BufWriter::new(std::io::sink()),
                &spec,
                20_770.0,
                &p,
            )
            .unwrap();
        check.budget(
            "share-safe copy of every row (mask, scramble, age band)",
            t.elapsed(),
            budget(15.0),
        );
        assert_eq!(rep.rows + 1, rows);
    }

    // Milestone 14: search the fixture folder (the 10M-row and the 1M-row
    // files) without opening tabs.
    {
        use natendo_engine::find::{Pattern, PatternKind};
        use natendo_engine::folder::{search_folder, FolderSearch};
        let spec = FolderSearch {
            pattern: Pattern {
                text: "Tromsø".into(),
                kind: PatternKind::Plain,
                match_case: false,
                whole_cell: true,
                ..Default::default()
            },
            subfolders: false,
            types: vec!["csv".into()],
        };
        let t = Instant::now();
        let r = search_folder(&dir, &spec, &p).unwrap();
        check.budget(
            "search a folder (the 10M-row and 1M-row files)",
            t.elapsed(),
            budget(20.0),
        );
        assert!(r.files >= 2);
    }

    // Milestone 14: mailing labels for every row of the 1M-row file.
    {
        use natendo_engine::mail::MailSpec;
        let d = Document::open_indexed(&small_path, OpenOptions::default(), None).unwrap();
        let spec = MailSpec {
            template: "{1} {2}\n{3}\n{4}, {5} {6}".into(),
            ..MailSpec::default()
        };
        let t = Instant::now();
        let rep = d
            .write_mail(&mut std::io::BufWriter::new(std::io::sink()), &spec, &p)
            .unwrap();
        check.budget(
            "mailing labels, every row of 1M (30 per page)",
            t.elapsed(),
            budget(20.0),
        );
        assert_eq!(rep.rows, small_rows);
    }

    // Milestone 11 review: a 10M-row other table, indexed once for the
    // Join screen, then previewed again from the kept index.
    {
        let mine = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let theirs = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let spec = JoinSpec {
            keys: vec![0],
            other_keys: vec![0],
            columns: vec![1, 4],
            keep_first: true,
            other_header: true,
            loose: true,
            not_found: "none".into(),
            ..Default::default()
        };
        let t = Instant::now();
        let src = JoinSource::build(&theirs, &spec, &p).unwrap();
        let pv = mine.preview_join(&src, &spec, &p).unwrap();
        check.budget(
            "join 10M rows to 10M: index the other + preview",
            t.elapsed(),
            budget(10.0),
        );
        assert_eq!(pv.report.matched + 1, rows);
        let t = Instant::now();
        mine.preview_join(&src, &spec, &p).unwrap();
        check.budget(
            "join preview again from the kept index (10M)",
            t.elapsed(),
            budget(3.0),
        );
        drop(src);
        // Review round 2 N1: a real join bringing in three columns on every
        // one of 10M rows plans and applies (was refused by the budget).
        let mut mine = mine;
        let spec = JoinSpec {
            columns: vec![1, 2, 4],
            loose: false,
            not_found: String::new(),
            ..spec
        };
        // Review round 3 N4: eight columns from a 10M-row table (was
        // refused past four by a cell count).
        let wide = JoinSpec {
            columns: (1..9).collect(),
            ..spec.clone()
        };
        let t = Instant::now();
        let src = JoinSource::build(&theirs, &wide, &p).unwrap();
        let (plan, _) = mine.plan_join(&src, &wide, &p).unwrap();
        check.budget(
            "join 10M rows to 10M bringing in 8 columns: index + plan",
            t.elapsed(),
            budget(20.0),
        );
        drop(plan);
        drop(src);
        let t = Instant::now();
        let src = JoinSource::build(&theirs, &spec, &p).unwrap();
        drop(theirs);
        let (plan, rep) = mine.plan_join(&src, &spec, &p).unwrap();
        mine.apply_join(plan).unwrap();
        check.budget(
            "join 10M rows to 10M bringing in 3 columns: plan + apply",
            t.elapsed(),
            budget(20.0),
        );
        assert_eq!(rep.matched + 1, rows);
        assert_eq!(mine.col_count(), 23);
        drop(src);
        // Review round 3 N5: saving every row after the join.
        let joined = dir.join(format!("saved-join-{rows}.csv"));
        let t = Instant::now();
        let rep = mine
            .save(SaveOptions {
                path: Some(joined.clone()),
                ..Default::default()
            })
            .unwrap();
        assert!(rep.verified);
        check.budget(
            "save after a 3-column join on 10M rows",
            t.elapsed(),
            budget(20.0),
        );
        drop(mine);
        let _ = fs::remove_file(&joined);
    }
    // Milestone 15: take changes from a newer copy (1M rows, 20 columns,
    // 1,000 changed cells), all accepted as one step.
    {
        use natendo_engine::updates::{compare_newer, Choice, UpdateChoice, UpdateSpec};
        let mut old = Document::open_indexed(&small_path, OpenOptions::default(), None).unwrap();
        let mut new = Document::open_indexed(&small_path, OpenOptions::default(), None).unwrap();
        // Spread over the file whatever its size (at 1M rows the file has 100k rows).
        let stride = (small_rows / 1_000).clamp(1, 997);
        for i in 0..1_000u64 {
            new.set_cell(i * stride + 1, 3, format!("changed {i}"))
                .unwrap();
        }
        let spec = UpdateSpec {
            keys: vec![0],
            keep_first: true,
            ..Default::default()
        };
        let t = Instant::now();
        let (ups, list) = compare_newer(&old, &new, &spec, &p).unwrap();
        let all = Choice {
            all: true,
            picked: Vec::new(),
        };
        let plan = old
            .plan_updates(
                &ups,
                &UpdateChoice {
                    cells_skip_hidden: false,
                    removed_skip_hidden: false,
                    cells: all.clone(),
                    added: all.clone(),
                    removed: all,
                },
            )
            .unwrap();
        old.apply_rows(plan).unwrap();
        check.budget(
            "changes from a newer copy (1M rows): compare + take all",
            t.elapsed(),
            budget(15.0),
        );
        assert_eq!(list.cells_total, 1_000);
        // An encrypted copy of the 1M rows, and opening it again.
        let enc = dir.join("perf.ndenc");
        let pw = natendo_engine::privacy::Secret::new("a long perf password");
        let t = Instant::now();
        old.save_encrypted(&enc, &pw, &p).unwrap();
        let back = Document::open_encrypted(&enc, &pw).unwrap();
        check.budget(
            "encrypted copy of 1M rows: save + open",
            t.elapsed(),
            budget(15.0),
        );
        assert_eq!(back.row_count(), old.row_count());
        drop(back);
        let _ = fs::remove_file(&enc);
        // The 1M rows saved as a SQLite table and opened again; then as
        // SQL statements.
        let db = dir.join("perf.sqlite");
        let _ = fs::remove_file(&db);
        let t = Instant::now();
        let n = old.save_sqlite(&db, "people", true, false, &p).unwrap();
        let back = natendo_engine::sqlite::read_table(&db, "people", &p).unwrap();
        check.budget(
            "SQLite table of 1M rows: save + open",
            t.elapsed(),
            budget(30.0),
        );
        assert_eq!(back.row_count(), n + 1);
        drop(back);
        let _ = fs::remove_file(&db);
        let t = Instant::now();
        let mut sink = std::io::sink();
        let opts = natendo_engine::export::ExportOptions {
            format: natendo_engine::export::ExportFormat::Sql,
            ..Default::default()
        };
        let rep = old.export(&mut sink, &opts, None, &p).unwrap();
        check.budget("SQL statements for 1M rows", t.elapsed(), budget(10.0));
        assert_eq!(rep.rows, n);
    }

    // Milestone 15: match 1M people against a list of 10,000 with
    // spelling slips.
    {
        use natendo_engine::fuzzy::{find_matches, FieldKind, MatchField, MatchSide, MatchSpec};
        const FIRST: &[&str] = &[
            "william", "mary", "james", "patricia", "john", "jennifer", "robert", "linda",
            "michael", "barbara", "david", "susan", "richard", "jessica", "joseph", "sarah",
            "thomas", "karen", "charles", "nancy", "daniel", "lisa", "matthew", "betty", "anthony",
            "sandra", "mark", "ashley", "paul", "donna", "steven", "emily", "andrew", "carol",
            "kenneth", "michelle", "joshua", "amanda", "kevin", "melissa",
        ];
        const PART: &[&str] = &[
            "smi", "john", "will", "brow", "jon", "gar", "mil", "dav", "rod", "mar", "her", "lop",
            "gon", "wil", "and", "tho", "tay", "moo", "jack", "mart", "lee", "per", "thom", "whi",
            "har", "san", "cla", "ram", "lew", "rob",
        ];
        const END: &[&str] = &[
            "th", "son", "ez", "er", "ley", "ton", "ford", "man", "ins", "ard",
        ];
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut person = || {
            let r = next();
            let first = FIRST[(r % 40) as usize];
            let last = format!(
                "{}{}{}",
                PART[(r >> 8) as usize % 30],
                PART[(r >> 16) as usize % 30],
                END[(r >> 24) as usize % 10]
            );
            format!(
                "{first},{last},{}/{}/{}\r\n",
                (r >> 32) % 12 + 1,
                (r >> 36) % 28 + 1,
                1930 + (r >> 40) % 80
            )
        };
        fn swap_two(s: &str, at: usize) -> String {
            let mut c: Vec<char> = s.chars().collect();
            c.swap(at, at + 1);
            c.into_iter().collect()
        }
        let mut big = String::from("first,last,dob\r\n");
        let mut small = String::from("first,last,dob\r\n");
        for i in 0..1_000_000u32 {
            let p = person();
            if i % 100 == 0 {
                // The list holds every 100th person with a slip in the
                // last name.
                let mut f: Vec<&str> = p.trim_end().split(',').collect();
                let slipped = swap_two(f[1], 1);
                f[1] = &slipped;
                small.push_str(&format!("{}\r\n", f.join(",")));
            }
            big.push_str(&p);
        }
        let bigd =
            Document::from_bytes(big.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let smalld =
            Document::from_bytes(small.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let kinds = [FieldKind::Name, FieldKind::Name, FieldKind::Exact];
        let spec = MatchSpec {
            fields: (0..3)
                .map(|c| MatchField {
                    col: c,
                    other_col: c,
                    kind: kinds[c as usize],
                    weight: 1.0,
                })
                .collect(),
            min_score: 85.0,
            ..Default::default()
        };
        let read = ReadOptions::default();
        let t = Instant::now();
        let a = MatchSide::build(&bigd, &[0, 1, 2], &kinds, &[], true, read, &p).unwrap();
        let b = MatchSide::build(&smalld, &[0, 1, 2], &kinds, &[], true, read, &p).unwrap();
        let rep = find_matches(&a, Some(&b), &spec, &p).unwrap();
        check.budget(
            "match people: 1M rows against a 10,000-person list",
            t.elapsed(),
            budget(20.0),
        );
        println!(
            "     {} pairs, {} compared, {} rows windowed",
            rep.total, rep.compared, rep.windowed
        );
        assert!(rep.total >= 10_000, "every listed person is found");
        drop(bigd);
    }

    // M15 review M2: near-duplicates within 10M people (only 8 first
    // names and 3,000 last names, so groups are huge); every 1,000th
    // person is in again with a slip in the last name, somewhere later.
    {
        use natendo_engine::fuzzy::{find_matches, FieldKind, MatchField, MatchSide, MatchSpec};
        const FIRST: &[&str] = &[
            "william", "mary", "james", "patricia", "john", "jennifer", "robert", "linda",
        ];
        const PART: &[&str] = &[
            "smi", "john", "will", "brow", "jon", "gar", "mil", "dav", "rod", "mar", "her", "lop",
            "gon", "wil", "and", "tho", "tay", "moo", "jack", "mart", "lee", "per", "thom", "whi",
            "har", "san", "cla", "ram", "lew", "rob",
        ];
        const END: &[&str] = &[
            "th", "son", "ez", "er", "ley", "ton", "ford", "man", "ins", "ard",
        ];
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let n = rows.min(10_000_000) as u32;
        let mut csv = String::with_capacity(n as usize * 32);
        csv.push_str("first,last,dob\r\n");
        let mut later: Vec<String> = Vec::new();
        for i in 0..n {
            let r = next();
            let last = format!(
                "{}{}{}",
                PART[(r >> 8) as usize % 30],
                PART[(r >> 16) as usize % 10],
                END[(r >> 24) as usize % 10]
            );
            let line = format!(
                "{},{last},{}-{:02}-{:02}\r\n",
                FIRST[(r % 8) as usize],
                1930 + (r >> 40) % 80,
                (r >> 32) % 12 + 1,
                (r >> 36) % 28 + 1
            );
            if i % 1_000 == 0 {
                let mut c: Vec<char> = line.chars().collect();
                let at = line.find(',').unwrap() + 2;
                c.swap(at, at + 1);
                later.push(c.into_iter().collect());
            }
            csv.push_str(&line);
            if i % 10_000 == 9_999 {
                for l in later.drain(..) {
                    csv.push_str(&l);
                }
            }
        }
        for l in later.drain(..) {
            csv.push_str(&l);
        }
        let d =
            Document::from_bytes(csv.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
        let kinds = [FieldKind::Name, FieldKind::Name, FieldKind::Exact];
        let spec = MatchSpec {
            fields: (0..3)
                .map(|c| MatchField {
                    col: c,
                    other_col: c,
                    kind: kinds[c as usize],
                    weight: 1.0,
                })
                .collect(),
            min_score: 90.0,
            ..Default::default()
        };
        let t = Instant::now();
        let side =
            MatchSide::build(&d, &[0, 1, 2], &kinds, &[], true, Default::default(), &p).unwrap();
        let rep = find_matches(&side, None, &spec, &p).unwrap();
        check.budget(
            "near-duplicates within 10M people",
            t.elapsed(),
            budget(90.0),
        );
        println!(
            "     {} pairs, {} compared, {} rows windowed",
            rep.total, rep.compared, rep.windowed
        );
        assert!(
            rep.total >= u64::from(n / 1_000),
            "every slipped copy is found ({} of {})",
            rep.total,
            n / 1_000
        );
    }

    // M15 review M5: changes from a newer copy of the full 10M-row file
    // (2,000 changed cells), compared and all taken.
    {
        use natendo_engine::updates::{compare_newer, Choice, UpdateChoice, UpdateSpec};
        let mut old = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let mut new = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        // Spread over the file whatever its size (4,999 apart at 10M rows).
        let stride = (rows / 2_000).clamp(1, 4_999);
        for i in 0..2_000u64 {
            new.set_cell(i * stride + 1, (i % 19 + 1) as u32, format!("newer {i}"))
                .unwrap();
        }
        let spec = UpdateSpec {
            keys: vec![0],
            keep_first: true,
            ..Default::default()
        };
        let t = Instant::now();
        let (ups, list) = compare_newer(&old, &new, &spec, &p).unwrap();
        let all = Choice {
            all: true,
            picked: Vec::new(),
        };
        let plan = old
            .plan_updates(
                &ups,
                &UpdateChoice {
                    cells_skip_hidden: false,
                    removed_skip_hidden: false,
                    cells: all.clone(),
                    added: all.clone(),
                    removed: all,
                },
            )
            .unwrap();
        old.apply_rows(plan).unwrap();
        check.budget(
            "changes from a newer copy (10M rows): compare + take all",
            t.elapsed(),
            budget(60.0),
        );
        assert_eq!(list.cells_total, 2_000);
        assert_eq!((list.added_total, list.removed_total), (0, 0));
    }

    // Milestone 14: list of changes and a recipe.
    {
        use natendo_engine::recipe::{Recipe, RecipeOp, RECIPE_VERSION};
        let mut doc = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        for i in 0..1_000u64 {
            doc.set_cell(
                i * (rows / 1_100).clamp(1, 9_000) + 1,
                (i % 5) as u32,
                format!("edited {i}"),
            )
            .unwrap();
        }
        let t = Instant::now();
        let list = doc.change_list(&p).unwrap();
        check.budget(
            "list of changes, 1,000 edits in 10M rows",
            t.elapsed(),
            budget(10.0),
        );
        assert_eq!(list.cells_total, 1_000);
        // Review H1: every cell of two columns changed (20M changes), one
        // column per clean.
        for col in [3, 4] {
            let spec = CleanSpec {
                cleaner: Cleaner {
                    version: 1,
                    name: String::new(),
                    steps: vec![Step::Case { to: CaseTo::Upper }],
                },
                scope: Scope::Columns { cols: vec![col] },
                skip_first: true,
                read: Default::default(),
            };
            let plan = doc.plan_clean(&spec, 20_000.0, 1, 0, &p).unwrap();
            doc.apply_clean(plan).unwrap();
        }
        let t = Instant::now();
        let list = doc.change_list(&p).unwrap();
        check.budget(
            "change list after a full clean of 2 columns on 10M rows",
            t.elapsed(),
            budget(20.0),
        );
        assert!(list.cells_total >= 2 * (rows - 1));
        assert_eq!(list.cells.len(), natendo_engine::changes::MAX_LISTED);
        drop(list);
        let report = dir.join("perf-changes.csv");
        let t = Instant::now();
        doc.save_change_report(&report, &[], false, &p).unwrap();
        check.budget(
            "change report of 20M changed cells (streamed)",
            t.elapsed(),
            budget(60.0),
        );
        let _ = fs::remove_file(&report);
        let recipe = Recipe {
            version: RECIPE_VERSION,
            name: String::new(),
            columns: Vec::new(),
            header: true,
            steps: vec![
                RecipeOp::DeleteCols { at: 1, count: 1 },
                RecipeOp::InsertCols { at: 0, count: 1 },
                RecipeOp::MoveCols {
                    at: 3,
                    count: 2,
                    to: 0,
                },
            ],
            left_out: Vec::new(),
            questions: Vec::new(),
        };
        let t = Instant::now();
        doc.run_recipe(&recipe, 20_000.0, 1, &p).unwrap();
        check.budget(
            "recipe of 3 column steps on 10M rows",
            t.elapsed(),
            budget(20.0),
        );
        // v0.4.4: the steps panel looks back by undo and forward by redo,
        // and a step changed runs the steps after it again.
        let t = Instant::now();
        for _ in 0..3 {
            doc.undo();
        }
        for _ in 0..3 {
            doc.redo();
        }
        check.budget(
            "steps panel: back 3 steps and forward again, 10M rows",
            t.elapsed(),
            Duration::from_secs(2),
        );
        let t = Instant::now();
        for _ in 0..3 {
            doc.undo();
        }
        doc.drop_redo();
        let changed = Recipe {
            steps: vec![
                RecipeOp::InsertCols { at: 0, count: 1 },
                RecipeOp::MoveCols {
                    at: 3,
                    count: 2,
                    to: 0,
                },
            ],
            ..recipe.clone()
        };
        doc.run_recipe(&changed, 20_000.0, 1, &p).unwrap();
        check.budget(
            "steps panel: take out step 1 of 3 and run the rest again, 10M rows",
            t.elapsed(),
            budget(20.0),
        );
        // A recipe question answered (the steps' texts filled in).
        let asked: Recipe = serde_json::from_value(serde_json::json!({
            "version": RECIPE_VERSION,
            "header": true,
            "steps": [{"op": "rename_column", "col": 0, "name": "{{title}}"}],
            "questions": [{"name": "title", "prompt": "Title?", "default": "id"}],
        }))
        .unwrap();
        let t = Instant::now();
        let answered = asked
            .answered(&[("title".into(), "voter id".into())])
            .unwrap();
        doc.run_recipe(&answered, 20_000.0, 1, &p).unwrap();
        check.budget(
            "recipe question answered and run, 10M rows",
            t.elapsed(),
            Duration::from_millis(500),
        );
        // natendo-cli filter: keep a filter's rows only.
        let (plan, _) = doc
            .plan_formula_filter(
                &ComputeSpec {
                    expr: "MOD(ROW(), 2) = 0".into(),
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        doc.apply_rows(plan).unwrap();
        let t = Instant::now();
        let keep = doc.plan_keep_shown().unwrap();
        doc.apply_rows(keep).unwrap();
        check.budget(
            "command-line filter: drop the rows a filter leaves out, 10M rows",
            t.elapsed(),
            Duration::from_secs(3),
        );
        drop(doc);
    }
    for f in [&xlsx, &json, &small_path, &other_path] {
        let _ = fs::remove_file(f);
    }
    let _ = fs::remove_file(&out);
    let _ = fs::remove_file(&out2);
    let _ = fs::remove_file(&out3);
    if !keep {
        let _ = fs::remove_file(&path);
    }
    if check.failed {
        eprintln!("performance budget missed");
        std::process::exit(1);
    }
}
