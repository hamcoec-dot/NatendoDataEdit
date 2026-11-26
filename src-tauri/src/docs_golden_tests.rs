use super::*;
use natendo_engine::fuzzy::{FieldKind, MatchField, MatchSpec};

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

/// Match people pairs each later row with one earlier row, so three of one
/// person are joined through one row: marked, and left for review when every
/// pair must be accepted.
#[test]
fn three_of_a_kind_are_joined_through_one_row() {
    let dir = std::env::temp_dir().join(format!("nde-sh-golden3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let docs = Docs::default();
    let text = "name\r\nZebedee Quillfeather\r\nZebedee Quillfeather\r\nZebedee Quillfeather\r\n";
    let id = tab(&docs, &dir, "three.csv", text);
    let spec = MatchSpec {
        fields: vec![MatchField {
            col: 0,
            other_col: 0,
            kind: FieldKind::Name,
            weight: 1.0,
        }],
        keep_first: true,
        other_header: true,
        min_score: 90.0,
        ..Default::default()
    };
    let rep = docs.match_find(id, None, &spec, &[]).unwrap();
    let accepted: Vec<(u32, u64, f32)> = rep
        .listed
        .iter()
        .map(|c| (c.id, c.other_row, c.score))
        .collect();
    assert_eq!(accepted.len(), 2);
    let s = docs
        .golden_plan(id, &accepted, GoldenSpec::default())
        .unwrap();
    assert_eq!((s.counts.clusters, s.counts.hub_only), (1, 1));
    assert!(docs.golden_page(id, 0).unwrap()[0].hub_only);
    let strict = GoldenSpec {
        require_all: true,
        ..Default::default()
    };
    let s = docs.golden_plan(id, &accepted, strict).unwrap();
    assert_eq!((s.counts.clusters, s.counts.review), (0, 3));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn plan_preview_create_stale_and_privacy() {
    let dir = std::env::temp_dir().join(format!("nde-sh-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let docs = Docs::default();
    let text = "id,name,city\r\n1,Zebedee Quillfeather,Yarrowby\r\n2,Zebedee Quillfeather,\r\n3,Ottilie Vantree,Mossford\r\n4,Perrin Ashgrove,Hollin\r\n";
    let id = tab(&docs, &dir, "people.csv", text);
    let other = tab(&docs, &dir, "other.csv", text);
    let spec = MatchSpec {
        fields: vec![MatchField {
            col: 1,
            other_col: 1,
            kind: FieldKind::Name,
            weight: 1.0,
        }],
        keep_first: true,
        other_header: true,
        min_score: 90.0,
        ..Default::default()
    };
    let golden = GoldenSpec {
        id_col: Some(0),
        ..Default::default()
    };
    assert!(docs
        .golden_plan(id, &[(1, 2, 100.0)], golden.clone())
        .is_err());
    // Two lists are refused.
    docs.match_find(id, Some(other), &spec, &[]).unwrap();
    assert!(docs
        .golden_plan(id, &[(1, 2, 100.0)], golden.clone())
        .unwrap_err()
        .message
        .contains("one list"));
    let rep = docs.match_find(id, None, &spec, &[]).unwrap();
    let accepted: Vec<(u32, u64, f32)> = rep
        .listed
        .iter()
        .map(|c| (c.id, c.other_row, c.score))
        .collect();
    assert_eq!(accepted.len(), 1);
    docs.set_private(id, true).unwrap();
    let s = docs.golden_plan(id, &accepted, golden.clone()).unwrap();
    assert_eq!(
        (s.counts.clusters, s.counts.folded, s.never_offered),
        (1, 1, 0)
    );
    let page = docs.golden_page(id, 0).unwrap();
    assert_eq!(page[0].members, ["1", "2"]);
    assert!(docs.golden_page(id, 1).unwrap().is_empty());
    let made = docs.golden_create(id, true).unwrap();
    assert_eq!(column(&docs, made.info.id, 0), ["id", "1", "3", "4"]);
    assert_eq!(column(&docs, made.info.id, 3)[1], "1; 2");
    let away = made.away.unwrap();
    assert_eq!(column(&docs, away.id, 2), ["id", "2"]);
    assert!(docs.is_private(made.info.id).unwrap());
    assert!(docs.is_private(away.id).unwrap());
    // The source is as it was.
    assert_eq!(column(&docs, id, 0), ["id", "1", "2", "3", "4"]);
    // Changed after the plan: no result. Changed after the match: no plan.
    docs.set_cells(id, vec![(1, 2, "Moved".into())]).unwrap();
    assert_eq!(docs.golden_create(id, false).unwrap_err().kind, "changed");
    assert_eq!(docs.golden_page(id, 0).unwrap_err().kind, "changed");
    let e = docs.golden_plan(id, &accepted, golden.clone()).unwrap_err();
    assert!(
        e.message.contains("run Match people again"),
        "{}",
        e.message
    );
    // Undone, it is still not the table that was matched.
    docs.undo(id).unwrap();
    let e = docs.golden_plan(id, &accepted, golden.clone()).unwrap_err();
    assert_eq!(e.kind, "changed");
    // Hiding rows moves which rows are shown: the match is refused too.
    docs.match_find(id, None, &spec, &[]).unwrap();
    docs.hide_rows(id, 3, 1).unwrap();
    let e = docs.golden_plan(id, &accepted, golden.clone()).unwrap_err();
    assert_eq!(e.kind, "changed");
    docs.golden_close(id);
    assert!(docs.golden_create(id, false).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
