use super::*;
use crate::save::SaveOptions;
use proptest::prelude::*;
use std::sync::atomic::AtomicU32;

static N: AtomicU32 = AtomicU32::new(0);

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "nde-dup-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn doc(text: &str) -> Document {
    Document::from_bytes(
        text.as_bytes().to_vec(),
        OpenOptions::default(),
        Some("csv"),
    )
    .unwrap()
}

fn all(d: &Document) -> Vec<Vec<String>> {
    (0..d.row_count())
        .map(|r| {
            (0..d.col_count())
                .map(|c| d.cell(r, c).unwrap_or_default())
                .collect()
        })
        .collect()
}

#[test]
fn the_copy_has_the_edits_no_file_and_no_history() {
    let mut d = doc("id,city\r\n1,Ames\r\n2,Bly\r\n3,Cody\r\n");
    d.set_cell(2, 1, "Boise".into()).unwrap();
    d.insert_cols(1, 1).unwrap();
    d.set_cell(0, 1, "zip".into()).unwrap();
    d.delete_rows(3, 1).unwrap();
    d.hide_rows(1, 1).unwrap();
    let c = d.duplicate(&Progress::new()).unwrap();
    assert_eq!(all(&c), all(&d));
    assert_eq!(c.row_count(), d.row_count());
    assert_eq!(c.total_rows(), d.total_rows());
    assert!(c.path().is_none());
    assert!(c.is_modified(), "unsaved from the start");
    assert!(!c.can_undo());
    assert_eq!(c.history_pos(), 0);
    assert_eq!(c.format, d.format);
    // Save needs a path (Save As).
    let mut c = c;
    assert!(matches!(c.save(SaveOptions::default()), Err(Error::NoPath)));
    // Saved somewhere new: the file holds the rows as shown in the original.
    let p = temp("copy.csv");
    c.save(SaveOptions {
        path: Some(p.clone()),
        ..Default::default()
    })
    .unwrap();
    let mut back = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
    back.hide_rows(1, 1).unwrap();
    assert_eq!(all(&back), all(&d));
}

#[test]
fn editing_the_copy_leaves_the_original_alone_and_back() {
    let mut d = doc("a,b\r\n1,2\r\n3,4\r\n");
    d.set_cell(1, 0, "x".into()).unwrap();
    let before = all(&d);
    let mut c = d.duplicate(&Progress::new()).unwrap();
    c.set_cell(1, 0, "copy".into()).unwrap();
    c.delete_rows(2, 1).unwrap();
    assert_eq!(all(&d), before);
    d.set_cell(2, 1, "orig".into()).unwrap();
    assert_eq!(c.cell(1, 0).as_deref(), Some("copy"));
    assert_eq!(c.row_count(), 2);
    // Undo in the copy goes back to the copy's start, never past it.
    assert!(c.undo());
    assert!(c.undo());
    assert!(!c.undo());
    assert_eq!(all(&c), before);
}

#[test]
fn cancel_keeps_nothing() {
    let d = doc("a\r\n1\r\n");
    let p = Progress::new();
    p.cancel.store(true, Ordering::SeqCst);
    assert!(matches!(d.duplicate(&p), Err(Error::Cancelled)));
}

#[test]
fn still_loading_is_refused() {
    let p = temp("big.csv");
    std::fs::write(&p, "a,b\r\n1,2\r\n").unwrap();
    let d = Document::open(&p, OpenOptions::default()).unwrap();
    assert!(matches!(
        d.duplicate(&Progress::new()),
        Err(Error::StillLoading)
    ));
}

#[test]
fn the_memory_it_needs_counts_the_index_and_the_edits() {
    let mut d = doc("a,b\r\n1,2\r\n3,4\r\n");
    let plain = d.duplicate_bytes();
    assert!(plain >= d.source.bytes().len() as u64 + d.index.memory_bytes() as u64);
    d.set_cell(1, 0, "x".repeat(1000)).unwrap();
    assert!(d.duplicate_bytes() >= plain + 1000);
}

/// The original maps its file (over 32 MB). With the copy open, the original
/// still saves over its own file, and the copy keeps its rows.
#[test]
fn a_mapped_original_saves_over_itself_while_the_copy_is_open() {
    let p = temp("mapped.csv");
    let mut text = String::from("id,name\r\n");
    let mut i = 0u64;
    while text.len() < (crate::source::MAP_THRESHOLD as usize) + 1024 {
        text.push_str(&format!("{i},name {i}\r\n"));
        i += 1;
    }
    std::fs::write(&p, &text).unwrap();
    let mut d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
    #[cfg(windows)]
    assert!(d.source.is_mapped(), "the test needs a mapped file");
    let c = d.duplicate(&Progress::new()).unwrap();
    assert!(!c.source.is_mapped());
    d.set_cell(1, 1, "changed".into()).unwrap();
    d.save(SaveOptions::default()).unwrap();
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert!(on_disk.starts_with("id,name\r\n0,changed\r\n"));
    assert_eq!(c.cell(1, 1).as_deref(), Some("name 0"));
    assert_eq!(c.row_count(), i + 1);
    drop(d);
    // The original closed: the copy still reads every row.
    assert_eq!(
        c.cell(i, 1).as_deref(),
        Some(format!("name {}", i - 1).as_str())
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    /// Random edits before and after: the copy starts equal to the original,
    /// and edits to either never show in the other.
    #[test]
    fn the_copy_and_the_original_stay_apart(
        before in proptest::collection::vec((0u64..6, 0u32..3, "[a-z]{0,3}"), 0..12),
        after in proptest::collection::vec((0u64..6, 0u32..3, "[a-z]{0,3}"), 0..12),
    ) {
        let mut d = doc("a,b,c\r\n1,2,3\r\n4,5,6\r\n7,8,9\r\n");
        for (r, c, v) in &before {
            if *r < d.row_count() {
                d.set_cell(*r, *c, v.clone()).unwrap();
            }
        }
        if before.len() % 3 == 1 {
            d.insert_rows(1, vec![vec!["n".into(), "e".into(), "w".into()]]).unwrap();
        }
        let start = all(&d);
        let mut copy = d.duplicate(&Progress::new()).unwrap();
        prop_assert_eq!(all(&copy), start.clone());
        for (r, c, v) in &after {
            if *r < copy.row_count() {
                copy.set_cell(*r, *c, format!("c{v}")).unwrap();
            }
        }
        prop_assert_eq!(all(&d), start.clone());
        let copied = all(&copy);
        for (r, c, v) in &after {
            if *r < d.row_count() {
                d.set_cell(*r, *c, format!("o{v}")).unwrap();
            }
        }
        prop_assert_eq!(all(&copy), copied);
    }
}
