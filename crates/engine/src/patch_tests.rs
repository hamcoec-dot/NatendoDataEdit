use std::sync::atomic::Ordering;

use super::*;
use crate::sort::{SortKey, SortSpec};
use crate::OpenOptions;

fn doc(text: &str) -> Document {
    Document::from_bytes(
        text.as_bytes().to_vec(),
        OpenOptions::default(),
        Some("csv"),
    )
    .unwrap()
}

fn rows(d: &Document) -> Vec<Vec<String>> {
    (0..d.row_count())
        .map(|r| d.row_values(r).unwrap())
        .collect()
}

fn spec(keys: &[u32]) -> PatchSpec {
    PatchSpec {
        keys: keys.to_vec(),
        trim: true,
        ..Default::default()
    }
}

fn make(old: &str, new: &str) -> Patch {
    doc(old)
        .make_patch(&doc(new), &spec(&[0]), &Progress::new())
        .unwrap()
        .patch
}

fn apply(d: &mut Document, p: &Patch) -> PatchReport {
    let (plan, report) = d.plan_patch(p, &Progress::new()).unwrap();
    if report.will_apply > 0 {
        d.apply_rows(plan).unwrap();
    }
    report
}

const OLD: &str = "id,name,town\r\n1,ann,York\r\n2,bob,Leeds\r\n3,cyd,Hull\r\n";
const NEW: &str = "id,name,town\r\n1,ann,Bath\r\n3,cyd,Hull\r\n4,dee,Ely\r\n";

#[test]
fn create_then_apply_gives_the_new_table_once() {
    let patch = make(OLD, NEW);
    assert_eq!(
        (patch.changes.len(), patch.adds.len(), patch.removes.len()),
        (1, 1, 1)
    );
    let mut d = doc(OLD);
    let r = apply(&mut d, &patch);
    assert_eq!((r.will_apply, r.cells, r.added, r.removed), (3, 1, 1, 1));
    assert_eq!(rows(&d), rows(&doc(NEW)));
    // Again: nothing to do.
    let r = apply(&mut d, &patch);
    assert_eq!((r.will_apply, r.already, r.unapplied()), (0, 3, 0));
    // One undo puts everything back.
    assert!(d.undo());
    assert_eq!(rows(&d), rows(&doc(OLD)));
}

#[test]
fn removes_and_adds_are_checked_like_changes() {
    let patch = make(OLD, NEW);
    // Row 2 changed since (the remove), key 4 taken by another row (the
    // add), row 1's town neither old nor new (the change).
    let mut d = doc("id,name,town\r\n1,ann,Wells\r\n2,bob,Derby\r\n3,cyd,Hull\r\n4,eve,Ely\r\n");
    let before = rows(&d);
    let r = apply(&mut d, &patch);
    assert_eq!((r.will_apply, r.conflicts), (0, 3));
    assert_eq!(rows(&d), before);
    // An identical row there already: the add is done.
    let mut d = doc("id,name,town\r\n1,ann,York\r\n4,dee,Ely\r\n");
    let r = apply(&mut d, &patch);
    assert_eq!((r.will_apply, r.already, r.conflicts), (1, 2, 0));
}

#[test]
fn keys_missing_twice_and_columns_missing() {
    let patch = make(OLD, NEW);
    let mut d = doc("id,name,town\r\n2,bob,Leeds\r\n 2 ,bob,Leeds\r\n");
    let r = apply(&mut d, &patch);
    // Key 1 is not there; key 2 is twice under the patch's trim rule.
    assert_eq!((r.not_found, r.twice, r.added), (1, 1, 1));
    // Without the town column the change, the add and the remove cannot go.
    let mut d = doc("id,name\r\n1,ann\r\n2,bob\r\n");
    let r = apply(&mut d, &patch);
    assert_eq!(r.missing, ["town"]);
    assert_eq!((r.missing_column, r.will_apply), (3, 0));
    let d = doc("name,town\r\nann,York\r\n");
    assert!(d
        .plan_patch(&patch, &Progress::new())
        .err()
        .unwrap()
        .to_string()
        .contains("no column titled \"id\""));
}

#[test]
fn extra_columns_are_ignored_and_new_rows_get_them_blank() {
    let patch = make(OLD, NEW);
    let mut d = doc("note,town,id,name\r\nx,York,1,ann\r\ny,Leeds,2,bob\r\n");
    let r = apply(&mut d, &patch);
    assert_eq!(r.extra, ["note"]);
    assert_eq!((r.cells, r.added, r.removed), (1, 1, 1));
    assert_eq!(
        rows(&d),
        [
            vec!["note", "town", "id", "name"],
            vec!["x", "Bath", "1", "ann"],
            vec!["", "Ely", "4", "dee"],
        ]
    );
}

#[test]
fn a_protected_column_is_left_alone() {
    let patch = make(OLD, NEW);
    let mut d = doc(OLD);
    d.set_protected(&[2]);
    let r = apply(&mut d, &patch);
    assert_eq!((r.protected, r.cells, r.added, r.removed), (1, 0, 1, 1));
    assert_eq!(rows(&d)[1][2], "York");
}

#[test]
fn creation_refuses_repeats_blanks_and_one_sided_columns() {
    let refuse = |old: &str, new: &str| {
        doc(old)
            .make_patch(&doc(new), &spec(&[0]), &Progress::new())
            .unwrap_err()
            .to_string()
    };
    let why = refuse("id,n\r\n7secret,a\r\n8,b\r\n7secret,c\r\n", NEW_TWO);
    assert!(why.contains("row 4 of the older table repeats"), "{why}");
    assert!(!why.contains("7secret"), "{why}");
    let why = refuse(NEW_TWO, "id,n\r\n1,a\r\n 1 ,b\r\n");
    assert!(why.contains("row 3 of the newer table repeats"), "{why}");
    assert!(refuse(NEW_TWO, "id,n\r\n,a\r\n").contains("row 2 of the newer table has a blank key"));
    let why = refuse("id,n,old\r\n1,a,x\r\n", "id,n,new\r\n1,b,y\r\n");
    assert!(why.contains("old, new"), "{why}");
    let mut s = spec(&[0]);
    s.leave_out = true;
    let made = doc("id,n,old\r\n1,a,x\r\n")
        .make_patch(&doc("id,n,new\r\n1,b,y\r\n"), &s, &Progress::new())
        .unwrap();
    assert_eq!(made.left_out, ["old", "new"]);
    assert_eq!(made.patch.columns, ["id", "n"]);
}

const NEW_TWO: &str = "id,n\r\n1,a\r\n";

#[test]
fn files_are_sealed_protected_and_checked() {
    let patch = make(OLD, NEW);
    let bytes = patch.to_bytes(None).unwrap();
    assert_eq!(Patch::from_bytes(&bytes, None).unwrap(), patch);
    let refused = |b: &[u8]| Patch::from_bytes(b, None).unwrap_err().to_string();
    // Any edit of the body breaks the seal.
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(refused(text.replace("Bath", "Bat").as_bytes()).contains("changed or damaged"));
    assert!(refused(b"{}").contains("changed or damaged"));
    assert!(refused(b"[1]").contains("not a patch"));
    assert!(refused(text.replacen('{', "{\"x\":1,\"x\":1,", 1).as_bytes()).contains("twice"));
    // A receipt's seal never passes for a patch's.
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let map = v.as_object_mut().unwrap();
    seal(map, SealKind::Receipt, SEAL).unwrap();
    assert!(refused(&serde_json::to_vec(&v).unwrap()).contains("changed or damaged"));
    // Sealed rightly but with an unknown field, or the same key twice, or a
    // column that is not there.
    let sealed = |change: &dyn Fn(&mut serde_json::Value)| {
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        change(&mut v);
        seal(v.as_object_mut().unwrap(), SealKind::Patch, SEAL).unwrap();
        refused(&serde_json::to_vec(&v).unwrap())
    };
    assert!(sealed(&|v| v["extra"] = 1.into()).contains("can read"));
    assert!(sealed(&|v| v["changes"][0]["extra"] = 1.into()).contains("can read"));
    assert!(sealed(&|v| v["adds"][0][0] = "1".into()).contains("same key twice"));
    assert!(sealed(&|v| v["changes"][0]["cells"][0][0] = 9.into()).contains("does not fit"));
    assert!(sealed(&|v| v["changes"][0]["cells"][0][0] = 0.into()).contains("does not fit"));
    assert!(sealed(&|v| v["keys"][0] = "zip".into()).contains("not among its columns"));
    assert!(sealed(&|v| v["version"] = 2.into()).contains("newer version"));
    // With a phrase.
    let phrase = Secret::new("a long made-up phrase");
    let locked = patch.to_bytes(Some(&phrase)).unwrap();
    assert!(crate::crypt::is_encrypted(&locked));
    assert!(!locked.windows(4).any(|w| w == b"Bath"));
    assert!(refused(&locked).contains("protected"));
    assert!(Patch::from_bytes(&locked, Some(&Secret::new("the wrong phrase!!"))).is_err());
    assert_eq!(Patch::from_bytes(&locked, Some(&phrase)).unwrap(), patch);
    assert!(patch.to_bytes(Some(&Secret::new("short"))).is_err());
}

/// Random edits: the patch applied to the old table, to a sorted copy and to
/// one with a column moved, gives the new table's rows.
#[test]
fn random_tables_round_trip() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let text = |titles: [usize; 3], rows: &[[String; 3]]| {
        let names = ["id", "a", "b"];
        let mut s = titles.map(|i| names[i]).join(",") + "\r\n";
        for r in rows {
            s.push_str(&titles.map(|i| r[i].clone()).join(","));
            s.push_str("\r\n");
        }
        s
    };
    for round in 0..200 {
        let value = |next: &mut dyn FnMut(u64) -> u64| {
            ["", "x", "y ", "Z", "00"][next(5) as usize].to_string()
        };
        let old: Vec<[String; 3]> = (0..next(20))
            .map(|i| [format!("k{i}"), value(&mut next), value(&mut next)])
            .collect();
        let mut new: Vec<[String; 3]> = Vec::new();
        for r in &old {
            match next(4) {
                0 => {}
                1 => new.push([r[0].clone(), value(&mut next), r[2].clone()]),
                _ => new.push(r.clone()),
            }
        }
        for i in 0..next(5) {
            new.push([format!("n{i}"), value(&mut next), value(&mut next)]);
        }
        let (a, b) = (text([0, 1, 2], &old), text([0, 1, 2], &new));
        let patch = make(&a, &b);
        let patch = Patch::from_bytes(&patch.to_bytes(None).unwrap(), None).unwrap();
        let sorted = |d: &Document| {
            let mut v = rows(d);
            v[1..].sort();
            v
        };
        let mut d = doc(&a);
        let r = apply(&mut d, &patch);
        assert_eq!(r.unapplied(), 0, "round {round}");
        assert_eq!(sorted(&d), sorted(&doc(&b)), "round {round}: {a:?} {b:?}");
        assert_eq!(apply(&mut d, &patch).will_apply, 0, "round {round}");
        // Sorted and with a column moved before applying.
        let moved = [2, 0, 1];
        let mut d = doc(&text(moved, &old));
        if !old.is_empty() {
            let sort = SortSpec {
                keys: vec![SortKey {
                    col: 1,
                    descending: true,
                    kind: Default::default(),
                    match_case: false,
                }],
                keep_first: true,
                ..Default::default()
            };
            let plan = d.plan_sort(&sort, &Progress::new()).unwrap();
            d.apply_sort(plan).unwrap();
        }
        assert_eq!(apply(&mut d, &patch).unapplied(), 0, "round {round}");
        assert_eq!(
            sorted(&d),
            sorted(&doc(&text(moved, &new))),
            "round {round} moved"
        );
    }
}

#[test]
fn report_table_and_cancel() {
    let patch = make(OLD, NEW);
    let (_, r) = doc(OLD).plan_patch(&patch, &Progress::new()).unwrap();
    let t = r.table().unwrap();
    assert_eq!((t.rows, t.cols), (4, 4));
    let text = String::from_utf8(t.bytes).unwrap();
    assert!(text.contains("will apply,change,1,town"), "{text}");
    assert!(!text.contains("Bath"));
    let p = Progress::new();
    p.cancel.store(true, Ordering::Relaxed);
    assert!(matches!(
        doc(OLD).plan_patch(&patch, &p),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        doc(OLD).make_patch(&doc(NEW), &spec(&[0]), &p),
        Err(Error::Cancelled)
    ));
}

#[test]
fn files_and_a_guarded_report() {
    let dir = std::env::temp_dir().join(format!("nde-patch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fix.ndpatch");
    let patch = make(
        "id,n
=1+1,a
",
        "id,n
=1+1,b
",
    );
    patch.save(&file, None).unwrap();
    assert!(!Patch::is_protected(&file).unwrap());
    assert_eq!(Patch::load(&file, None).unwrap(), patch);
    patch
        .save(&file, Some(&Secret::new("a long made-up phrase")))
        .unwrap();
    assert!(Patch::is_protected(&file).unwrap());
    let (_, r) = doc("id,n
=1+1,a
")
    .plan_patch(&patch, &Progress::new())
    .unwrap();
    let csv = dir.join("report.csv");
    r.save_csv(&csv).unwrap();
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.contains("'=1+1"), "{text}");
    assert_eq!(r.table().unwrap().rows, 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// More rows than one parallel chunk: file order is kept, and the first bad
/// row is the one named.
#[test]
fn many_chunks_keep_order_and_name_the_first_bad_row() {
    let n = 3 * crate::ops::CHUNK as u32 + 5;
    let table = |change: &dyn Fn(u32) -> Option<String>| {
        let mut s = String::from("id,v\r\n");
        for i in 0..n {
            if let Some(line) = change(i) {
                s.push_str(&line);
                s.push_str("\r\n");
            }
        }
        s
    };
    let old = table(&|i| Some(format!("k{i},a")));
    let new = table(&|i| match i % 1000 {
        1 => None,
        2 => Some(format!("k{i},b")),
        3 => Some(format!("k{i},a\r\nm{i},c")),
        _ => Some(format!("k{i},a")),
    });
    let patch = make(&old, &new);
    let ids = |keys: Vec<&String>| {
        keys.iter()
            .map(|k| k[1..].parse::<u32>().unwrap())
            .collect::<Vec<_>>()
    };
    let expect = |r: u32| (0..n).filter(|i| i % 1000 == r).collect::<Vec<_>>();
    assert_eq!(
        ids(patch.changes.iter().map(|c| &c.key[0]).collect()),
        expect(2)
    );
    assert_eq!(
        ids(patch.removes.iter().map(|c| &c.key[0]).collect()),
        expect(1)
    );
    assert_eq!(ids(patch.adds.iter().map(|r| &r[0]).collect()), expect(3));
    let mut d = doc(&old);
    assert_eq!(apply(&mut d, &patch).unapplied(), 0);
    assert_eq!(d.row_count(), doc(&new).row_count());
    let late = 2 * crate::ops::CHUNK as u32 + 7;
    let refuse = |bad: &str| {
        doc(bad)
            .make_patch(&doc(&new), &spec(&[0]), &Progress::new())
            .unwrap_err()
            .to_string()
    };
    // A repeat before a blank, then a blank before a repeat.
    let why = refuse(&table(&|i| {
        Some(match i {
            i if i == late => "k3,a".into(),
            i if i == late + 1 => ",a".into(),
            i => format!("k{i},a"),
        })
    }));
    assert!(
        why.contains(&format!("row {} of the older table repeats", late + 2)),
        "{why}"
    );
    let why = refuse(&table(&|i| {
        Some(match i {
            i if i == late => ",a".into(),
            i if i == late + 1 || i == n - 1 => "k3,a".into(),
            i => format!("k{i},a"),
        })
    }));
    assert!(
        why.contains(&format!("row {} of the older table has a blank", late + 2)),
        "{why}"
    );
}
