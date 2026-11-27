use super::*;

const PHRASE: &str = "a long made-up phrase";

fn tab(docs: &Docs, dir: &Path, name: &str, text: &str) -> u32 {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    let id = docs.open_path(&p, false).unwrap().id;
    for _ in 0..500 {
        if docs.info(id).unwrap().indexed {
            return id;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("indexing did not finish");
}

fn column(docs: &Docs, id: u32, col: u32) -> Vec<String> {
    let e = docs.get(id).unwrap();
    let d = e.read();
    (0..d.row_count())
        .map(|r| d.row_values(r).unwrap()[col as usize].clone())
        .collect()
}

#[test]
fn make_dry_run_apply_undo_and_privacy() {
    let dir = crate::testdir::path("nde-sh-patch");
    std::fs::create_dir_all(&dir).unwrap();
    let docs = Docs::default();
    let old = "id,n\r\n1,a\r\n2,b\r\n";
    let a = tab(&docs, &dir, "old.csv", old);
    let b = tab(&docs, &dir, "new.csv", "id,n\r\n1,z\r\n3,c\r\n");
    let target = tab(&docs, &dir, "target.csv", old);
    let spec = PatchSpec {
        keys: vec![0],
        ..Default::default()
    };
    let file = dir.join("fix.ndpatch");
    let _ = std::fs::remove_file(&file);
    let make = |f: &Path, phrase: Option<&str>, plain| {
        docs.patch_make(a, b, &spec, f, phrase.map(String::from), plain)
    };
    assert!(docs.patch_make(a, a, &spec, &file, None, true).is_err());
    assert!(make(&dir.join("fix.csv"), None, true).is_err());
    // A private source: a phrase, or the user's say-so.
    docs.set_private(b, true).unwrap();
    assert!(make(&file, None, false)
        .unwrap_err()
        .message
        .contains("private"));
    assert!(!file.exists());
    let saved = make(&file, Some(PHRASE), false).unwrap();
    assert_eq!(
        (saved.changes, saved.adds, saved.removes, saved.protected),
        (1, 1, 1, true)
    );
    assert!(docs.patch_apply(target).is_err());
    assert!(docs.patch_preview(target, &file, None).is_err());
    let preview = || docs.patch_preview(target, &file, Some(PHRASE.into()));
    let p = preview().unwrap();
    assert!(p.private);
    assert_eq!(p.report.will_apply, 3);
    assert!(
        !docs.is_private(target).unwrap(),
        "a dry run changes nothing"
    );
    // Changed after the dry run: refused.
    docs.set_cells(target, vec![(1, 1, "q".into())]).unwrap();
    assert_eq!(docs.patch_apply(target).unwrap_err().kind, "changed");
    assert_eq!(column(&docs, target, 0), ["id", "1", "2"]);
    docs.undo(target).unwrap();
    preview().unwrap();
    let done = docs.patch_apply(target).unwrap();
    assert_eq!((done.cells, done.added, done.removed), (1, 1, 1));
    assert!(docs.is_private(target).unwrap());
    assert_eq!(column(&docs, target, 0), ["id", "1", "3"]);
    assert_eq!(column(&docs, target, 1), ["n", "z", "c"]);
    assert!(docs.patch_apply(target).is_err(), "not twice");
    let csv = dir.join("report.csv");
    docs.patch_report_save(target, &csv).unwrap();
    assert!(std::fs::read_to_string(&csv)
        .unwrap()
        .contains("will apply"));
    // One undo step.
    docs.undo(target).unwrap();
    assert_eq!(column(&docs, target, 0), ["id", "1", "2"]);
    assert_eq!(column(&docs, target, 1), ["n", "a", "b"]);
    docs.patch_close(target);
    assert!(docs.patch_report_save(target, &csv).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
