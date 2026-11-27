use super::*;
use crate::reshape::SplitRows;
use crate::OpenOptions;
use proptest::prelude::*;

fn doc(text: &str) -> Document {
    Document::from_bytes(
        text.as_bytes().to_vec(),
        OpenOptions::default(),
        Some("csv"),
    )
    .unwrap()
}

/// Every row (hidden ones too) in file order.
fn all(d: &Document) -> Vec<Vec<String>> {
    (0..d.total_rows())
        .map(|p| {
            let mut r = d.logical_values(d.doc_id_at(p).unwrap()).unwrap();
            r.resize(d.col_count() as usize, String::new());
            r
        })
        .collect()
}

fn text(d: &Document) -> String {
    let mut out = Vec::new();
    d.write_copy(&mut out).unwrap();
    String::from_utf8(out).unwrap().replace("\r\n", "\n")
}

fn spec(kind: ExplodeKind, how: ExplodeHow) -> Explode {
    Explode {
        col: 1,
        kind,
        how,
        ..Default::default()
    }
}

fn cols(d: &mut Document, s: &Explode) -> ExplodeReport {
    let (p, r) = d.plan_explode_columns(s, &Progress::new()).unwrap();
    d.apply_cols(p).unwrap();
    r
}

fn rows(d: &mut Document, s: &Explode) -> ExplodeReport {
    let (p, r) = d.plan_explode_rows(s, &Progress::new()).unwrap();
    d.apply_rows(p).unwrap();
    r
}

fn err_cols(d: &Document, s: &Explode) -> String {
    d.plan_explode_columns(s, &Progress::new())
        .err()
        .unwrap()
        .to_string()
}

#[test]
fn flat_and_nested_objects() {
    let csv = "id,details\n1,\"{\"\"plan\"\":\"\"gold\"\",\"\"seats\"\":3,\"\"who\"\":{\"\"name\"\":\"\"Ann\"\",\"\"at\"\":{\"\"city\"\":\"\"Oslo\"\"}}}\"\n2,\"{\"\"seats\"\":5}\"\n";
    let mut d = doc(csv);
    cols(&mut d, &spec(ExplodeKind::Object, ExplodeHow::Columns));
    assert_eq!(
        all(&d),
        vec![
            vec![
                "id",
                "details",
                "details.plan",
                "details.seats",
                "details.who"
            ],
            vec![
                "1",
                &d.cell(1, 1).unwrap(),
                "gold",
                "3",
                "{\"name\":\"Ann\",\"at\":{\"city\":\"Oslo\"}}"
            ],
            vec!["2", "{\"seats\":5}", "", "5", ""],
        ]
    );
    for (depth, want) in [
        (
            2,
            vec![
                "details.plan",
                "details.seats",
                "details.who.name",
                "details.who.at",
            ],
        ),
        (
            3,
            vec![
                "details.plan",
                "details.seats",
                "details.who.name",
                "details.who.at.city",
            ],
        ),
    ] {
        let mut d = doc(csv);
        cols(
            &mut d,
            &Explode {
                depth,
                ..spec(ExplodeKind::Object, ExplodeHow::Columns)
            },
        );
        assert_eq!(all(&d)[0][2..], want[..], "depth {depth}");
        if depth == 2 {
            assert_eq!(all(&d)[1][5], "{\"city\":\"Oslo\"}");
        }
    }
}

#[test]
fn json_lists_fan_out_and_unnest() {
    let csv = "id,tags\n1,\"[\"\"a\"\",2,null,[1,2]]\"\n2,[]\n";
    let mut d = doc(csv);
    cols(&mut d, &spec(ExplodeKind::JsonList, ExplodeHow::Columns));
    assert_eq!(
        all(&d)[0],
        vec!["id", "tags", "tags 1", "tags 2", "tags 3", "tags 4"]
    );
    assert_eq!(all(&d)[1][2..], ["a", "2", "", "[1,2]"]);
    let mut d = doc(csv);
    rows(&mut d, &spec(ExplodeKind::JsonList, ExplodeHow::Rows));
    assert_eq!(
        all(&d),
        vec![
            vec!["id", "tags"],
            vec!["1", "a"],
            vec!["1", "2"],
            vec!["1", ""],
            vec!["1", "[1,2]"],
            vec!["2", ""],
        ]
    );
}

#[test]
fn empty_object_adds_nothing_and_is_not_bad() {
    let mut d = doc("id,j\n1,{}\n2,\"{\"\"a\"\":1}\"\n");
    let s = spec(ExplodeKind::Object, ExplodeHow::Columns);
    let scan = d.explode_scan(&s, &Progress::new()).unwrap();
    assert_eq!((scan.bad, scan.filled, scan.distinct), (0, 2, 1));
    let r = cols(&mut d, &s);
    assert_eq!(r.bad, 0);
    assert_eq!(all(&d)[1], vec!["1", "{}", ""]);
    // Only empty objects: nothing to add.
    let d = doc("id,j\n1,{}\n");
    assert!(err_cols(&d, &s).contains("nothing to add"));
}

#[test]
fn invalid_json_is_left_and_counted() {
    let csv = "id,j\n1,\"{\"\"a\"\":1}\"\n2,{oops\n3,plain text\n4,\"{\"\"a\"\":1,\"\"a\"\":2}\"\n";
    let mut d = doc(csv);
    let s = spec(ExplodeKind::Object, ExplodeHow::Columns);
    let scan = d.explode_scan(&s, &Progress::new()).unwrap();
    assert_eq!((scan.bad, scan.filled), (3, 4));
    // Show those rows: exactly the three (and the title row).
    let p = d.plan_explode_bad(&s, &Progress::new()).unwrap();
    assert_eq!(p.rows, 3);
    let r = cols(&mut d, &s);
    assert_eq!(r.bad, 3);
    let a = all(&d);
    assert_eq!(a[2], vec!["2", "{oops", ""]);
    assert_eq!(a[3], vec!["3", "plain text", ""]);
    assert_eq!(a[4][1], "{\"a\":1,\"a\":2}");
    // Dropping the original would lose them: refused.
    let d = doc(csv);
    let drop = Explode {
        keep_original: false,
        ..s
    };
    assert!(err_cols(&d, &drop).contains("3 cells did not parse"));
}

#[test]
fn shows_bad_rows() {
    let mut d = doc("id,j\n1,\"{\"\"a\"\":1}\"\n2,{oops\n3,\"{\"\"a\"\":2}\"\n");
    let s = spec(ExplodeKind::Object, ExplodeHow::Columns);
    let p = d.plan_explode_bad(&s, &Progress::new()).unwrap();
    d.apply_rows(p).unwrap();
    assert_eq!(d.row_count(), 2);
    assert_eq!(d.cell(1, 0).unwrap(), "2");
    let good = doc("id,j\n1,\"{\"\"a\"\":1}\"\n");
    assert!(good
        .plan_explode_bad(&s, &Progress::new())
        .err()
        .unwrap()
        .to_string()
        .contains("every filled cell"));
}

#[test]
fn numbers_keep_their_text_and_strings_unescape() {
    let csv = "id,j\n1,\"{\"\"p\"\":1.50,\"\"big\"\":12345678901234567890,\"\"q\"\":\"\"a\\\"\"b\"\",\"\"t\"\":true,\"\"n\"\":null,\"\"e\"\":1e5}\"\n";
    let mut d = doc(csv);
    cols(&mut d, &spec(ExplodeKind::Object, ExplodeHow::Columns));
    assert_eq!(
        all(&d)[1][2..],
        ["1.50", "12345678901234567890", "a\"b", "true", "", "1e5"]
    );
    // null as the text "null".
    let mut d = doc(csv);
    cols(
        &mut d,
        &Explode {
            null_blank: false,
            ..spec(ExplodeKind::Object, ExplodeHow::Columns)
        },
    );
    assert_eq!(all(&d)[1][6], "null");
    // The saved bytes keep 1.50.
    assert!(text(&d).contains(",1.50,12345678901234567890,"));
}

#[test]
fn unicode_dotted_and_colliding_keys() {
    let csv = "id,j\n1,\"{\"\"städt\"\":\"\"Zürich\"\",\"\"a.b\"\":1,\"\"a\"\":{\"\"b\"\":2},\"\"\"\":3,\"\"x\\ny\"\":4}\"\n";
    let mut d = doc(csv);
    cols(
        &mut d,
        &Explode {
            depth: 2,
            ..spec(ExplodeKind::Object, ExplodeHow::Columns)
        },
    );
    assert_eq!(
        all(&d)[0][2..],
        ["j.städt", "j.a.b", "j.a.b (2)", "j.(empty)", "j.x y"]
    );
    assert_eq!(all(&d)[1][2..], ["Zürich", "1", "2", "3", "4"]);
    // A new title that clashes with a column the table has gets " (2)".
    let mut d = doc("id,j,j.a\n1,\"{\"\"a\"\":1}\",x\n");
    cols(&mut d, &spec(ExplodeKind::Object, ExplodeHow::Columns));
    assert_eq!(all(&d)[0], vec!["id", "j", "j.a (2)", "j.a"]);
}

#[test]
fn many_keys_top_thousand_and_refused() {
    let mut csv = String::from("id,j\n");
    for i in 0..5_000 {
        // Key k0 in every row, so it is the most common.
        csv.push_str(&format!("{i},\"{{\"\"k0\"\":1,\"\"k{}\"\":2}}\"\n", i + 1));
    }
    let d = doc(&csv);
    let s = spec(ExplodeKind::Object, ExplodeHow::Columns);
    let scan = d.explode_scan(&s, &Progress::new()).unwrap();
    assert_eq!(scan.distinct, 5_001);
    assert_eq!(scan.keys.len(), TOP_KEYS);
    assert_eq!(
        scan.keys[0],
        KeyCount {
            key: "k0".into(),
            count: 5_000
        }
    );
    assert!(err_cols(&d, &s).contains("this would add 5001 columns; at most 1000"));
    // With the top 1,000 ticked it runs, and the rest are named new.
    let mut d = d;
    let ticked = Explode {
        keys: Some(scan.keys.iter().map(|k| k.key.clone()).collect()),
        ..s
    };
    let r = cols(&mut d, &ticked);
    assert_eq!(
        (r.cols, r.new_count, r.new_keys.len()),
        (1_000, 4_001, MAX_NAMED)
    );
}

#[test]
fn pipe_tags_trim_on_and_off() {
    let csv = "id,t\n1,vip | beta|annual\n2,VIP|beta\n3,\n";
    let mut d = doc(csv);
    let s = spec(ExplodeKind::List, ExplodeHow::Flags);
    let scan = d.explode_scan(&s, &Progress::new()).unwrap();
    assert_eq!(
        scan.keys,
        vec![
            KeyCount {
                key: "vip".into(),
                count: 2
            },
            KeyCount {
                key: "beta".into(),
                count: 2
            },
            KeyCount {
                key: "annual".into(),
                count: 1
            },
        ]
    );
    cols(&mut d, &s);
    assert_eq!(
        all(&d),
        vec![
            vec!["id", "t", "t: vip", "t: beta", "t: annual"],
            vec!["1", "vip | beta|annual", "1", "1", "1"],
            vec!["2", "VIP|beta", "1", "1", ""],
            vec!["3", "", "", "", ""],
        ]
    );
    // Trim off and case on: "vip " and " beta" and "VIP" are their own.
    let d = doc(csv);
    let raw = Explode {
        trim: false,
        match_case: true,
        ..s
    };
    let scan = d.explode_scan(&raw, &Progress::new()).unwrap();
    let keys: Vec<&str> = scan.keys.iter().map(|k| k.key.as_str()).collect();
    assert_eq!(keys, ["vip ", " beta", "annual", "VIP", "beta"]);
}

#[test]
fn key_value_split_at_first_sign() {
    let mut d = doc("id,kv\n1,a=1;b=x=y; c = 3 ;\n2,a=2;a=3\n3,a;b=1\n");
    let s = Explode {
        separator: ";".into(),
        ..spec(ExplodeKind::KeyValue, ExplodeHow::Columns)
    };
    let r = cols(&mut d, &s);
    assert_eq!(r.bad, 2, "a repeated key and a pair without = do not parse");
    assert_eq!(
        all(&d),
        vec![
            vec!["id", "kv", "kv.a", "kv.b", "kv.c"],
            vec!["1", "a=1;b=x=y; c = 3 ;", "1", "x=y", "3"],
            vec!["2", "a=2;a=3", "", "", ""],
            vec!["3", "a;b=1", "", "", ""],
        ]
    );
}

#[test]
fn each_kind_is_one_undo_step() {
    let csv = "id,j\n1,\"{\"\"a\"\":1,\"\"b\"\":2}\"\n2,x|y|z\n";
    for (kind, how, sep) in [
        (ExplodeKind::Object, ExplodeHow::Columns, "|"),
        (ExplodeKind::List, ExplodeHow::Columns, "|"),
        (ExplodeKind::List, ExplodeHow::Flags, "|"),
        (ExplodeKind::List, ExplodeHow::Rows, "|"),
    ] {
        for original in [false, true] {
            let mut d = doc(csv);
            let before = all(&d);
            let s = Explode {
                separator: sep.into(),
                original_col: original,
                ..spec(kind, how)
            };
            if how == ExplodeHow::Rows {
                rows(&mut d, &s);
            } else {
                cols(&mut d, &s);
            }
            assert_ne!(all(&d), before);
            assert!(d.undo());
            assert_eq!(all(&d), before, "{kind:?} {how:?} {original}");
            assert!(!d.undo(), "one step");
        }
    }
}

#[test]
fn unnest_with_the_original_column() {
    let mut d = doc("id,t,n\n1,a|b,x\n2,c,y\n3,,z\n");
    let s = Explode {
        original_col: true,
        ..spec(ExplodeKind::List, ExplodeHow::Rows)
    };
    let r = rows(&mut d, &s);
    assert_eq!((r.rows, r.cols), (1, 1));
    assert_eq!(
        all(&d),
        vec![
            vec!["id", "t", "t (original)", "n"],
            vec!["1", "a", "a|b", "x"],
            vec!["1", "b", "a|b", "x"],
            vec!["2", "c", "c", "y"],
            vec!["3", "", "", "z"],
        ]
    );
    assert!(text(&d).starts_with("id,t,t (original),n\n1,a,a|b,x\n1,b,a|b,x\n"));
    assert!(d.undo());
    assert_eq!(text(&d), "id,t,n\n1,a|b,x\n2,c,y\n3,,z\n");
}

#[test]
fn hidden_rows_columns_cover_them_rows_leave_them() {
    let csv = "id,t\n1,a|b\n2,c|d\n3,e|f\n";
    // Row 2 (the second data row) hidden.
    let mut d = doc(csv);
    d.hide_rows(2, 1).unwrap();
    cols(&mut d, &spec(ExplodeKind::List, ExplodeHow::Columns));
    let a = all(&d);
    assert_eq!(a[2], vec!["2", "c|d", "c", "d"], "columns fill hidden rows");
    let mut d = doc(csv);
    d.hide_rows(2, 1).unwrap();
    let scan = d
        .explode_scan(&spec(ExplodeKind::List, ExplodeHow::Rows), &Progress::new())
        .unwrap();
    assert_eq!((scan.new_rows, scan.all_rows), (2, false));
    rows(&mut d, &spec(ExplodeKind::List, ExplodeHow::Rows));
    let a = all(&d);
    assert_eq!(a.len(), 6);
    assert!(
        a.contains(&vec!["2".to_string(), "c|d".to_string()]),
        "hidden row left"
    );
}

#[test]
fn listed_keys_new_and_missing() {
    let csv = "id,j\n1,\"{\"\"plan\"\":\"\"gold\"\",\"\"region\"\":\"\"N\"\"}\"\n";
    let listed = Explode {
        keys: Some(vec!["plan".into(), "seats".into()]),
        ..spec(ExplodeKind::Object, ExplodeHow::Columns)
    };
    let mut d = doc(csv);
    let r = cols(&mut d, &listed);
    assert_eq!(all(&d)[0], vec!["id", "j", "j.plan", "j.seats"]);
    assert_eq!(
        (r.new_keys.clone(), r.missing_keys.clone()),
        (vec!["region".to_string()], vec!["seats".to_string()])
    );
    let note = r.note("j");
    assert!(
        note.contains("new key in j: \u{201c}region\u{201d} (left out"),
        "{note}"
    );
    assert!(
        note.contains("found in no cell of j: \u{201c}seats\u{201d}"),
        "{note}"
    );
    // Add new keys on: region becomes a column after the listed ones.
    let mut d = doc(csv);
    let r = cols(
        &mut d,
        &Explode {
            add_new_keys: true,
            ..listed
        },
    );
    assert_eq!(all(&d)[0], vec!["id", "j", "j.plan", "j.seats", "j.region"]);
    assert!(r.note("j").contains("added as columns"));
}

#[test]
fn recipe_replay_with_a_new_key() {
    use crate::recipe::RecipeOp;
    let month1 = "id,j\n1,\"{\"\"plan\"\":\"\"gold\"\"}\"\n";
    let month2 = "id,j\n1,\"{\"\"plan\"\":\"\"gold\"\",\"\"region\"\":\"\"N\"\"}\"\n";
    for (keys, want) in [
        (Some(vec!["plan".to_string()]), vec!["id", "j", "j.plan"]),
        (None, vec!["id", "j", "j.plan", "j.region"]),
    ] {
        let step = RecipeOp::ExplodeColumns {
            spec: Explode {
                keys: keys.clone(),
                ..spec(ExplodeKind::Object, ExplodeHow::Columns)
            },
        };
        // The step's JSON holds key names and options, never cell text.
        let json = serde_json::to_string(&step).unwrap();
        assert!(!json.contains("gold"), "{json}");
        let back: RecipeOp = serde_json::from_str(&json).unwrap();
        let mut d = doc(month1);
        d.run_recipe_step(&back, 0.0, 0, &Progress::new()).unwrap();
        let mut d2 = doc(month2);
        d2.run_recipe_step(&back, 0.0, 0, &Progress::new()).unwrap();
        assert_eq!(all(&d2)[0], want, "{keys:?}");
        assert!(back.describe().starts_with("Explode JSON in column 2 to "));
    }
}

#[test]
fn too_deep_and_too_big_do_not_parse() {
    let deep = format!("{}1{}", "{\"a\":".repeat(70), "}".repeat(70));
    let big = format!("{{\"a\":\"{}\"}}", "x".repeat(MAX_JSON_CELL));
    let s = spec(ExplodeKind::Object, ExplodeHow::Columns);
    assert_eq!(read_cell(&deep, &s), Read::Bad);
    assert_eq!(read_cell(&big, &s), Read::Bad);
    let ok = format!("{}1{}", "{\"a\":".repeat(60), "}".repeat(60));
    assert!(matches!(read_cell(&ok, &s), Read::Pairs(_)));
    let deep_list = format!("{}{}", "[".repeat(70), "]".repeat(70));
    assert_eq!(
        read_cell(&deep_list, &spec(ExplodeKind::JsonList, ExplodeHow::Rows)),
        Read::Bad
    );
}

#[test]
fn detection_words() {
    let cells = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let d = detect(&cells(&[
        "{\"a\":1}",
        "{\"b\":2}",
        "{}",
        "{\"c\":3}",
        "{\"c\":3}",
    ]));
    assert_eq!(
        (d.kind, d.words.as_str()),
        (ExplodeKind::Object, "JSON objects (5 of 5 sampled)")
    );
    let d = detect(&cells(&["[1,2]", "[\"a\"]"]));
    assert_eq!(d.kind, ExplodeKind::JsonList);
    let d = detect(&cells(&["vip|beta", "annual|vip", "beta"]));
    assert_eq!((d.kind, d.separator.as_str()), (ExplodeKind::List, "|"));
    assert_eq!(d.words, "mixed: 2 of 3 look like lists separated by |");
    let d = detect(&cells(&["a=1;b=2", "a=3"]));
    assert_eq!(
        (d.kind, d.separator.as_str(), d.kv.as_str()),
        (ExplodeKind::KeyValue, ";", "=")
    );
    assert_eq!(d.words, "key=value pairs separated by ; (2 of 2 sampled)");
    let mut mixed = vec!["{\"a\":1}"; 61];
    mixed.extend(vec!["plain"; 39]);
    let d = detect(&cells(&mixed));
    assert_eq!(d.words, "mixed: 61 of 100 look like JSON objects");
}

#[test]
fn preview_tells_missing_null_and_empty_apart() {
    let d = doc(
        "id,j\n1,\"{\"\"a\"\":\"\"\"\",\"\"b\"\":null}\"\n2,\"{\"\"a\"\":\"\"x\"\"}\"\n3,bad\n",
    );
    let p = d
        .explode_preview(&spec(ExplodeKind::Object, ExplodeHow::Columns))
        .unwrap();
    assert_eq!(p.titles, vec!["j.a", "j.b"]);
    let shown = |r: usize| -> Vec<Shown> { p.rows[r].lines[0].iter().map(|c| c.shown).collect() };
    assert_eq!(shown(0), vec![Shown::Empty, Shown::Null]);
    assert_eq!(shown(1), vec![Shown::Value, Shown::Missing]);
    assert!(p.rows[2].bad && p.rows[2].lines.is_empty());
    // Unnest: one line per new row.
    let d = doc("id,t\n1,a|b|c\n");
    let p = d
        .explode_preview(&Explode {
            original_col: true,
            ..spec(ExplodeKind::List, ExplodeHow::Rows)
        })
        .unwrap();
    assert_eq!(p.titles, vec!["t (original)"]);
    assert_eq!(p.rows[0].lines.len(), 3);
    assert_eq!(p.rows[0].lines[2][1].text, "a|b|c");
}

#[test]
fn drop_original_puts_the_first_column_in_its_place() {
    let mut d = doc("id,j,n\n1,\"{\"\"a\"\":1,\"\"b\"\":2}\",x\n2,\"{\"\"b\"\":3}\",y\n3,,z\n");
    cols(
        &mut d,
        &Explode {
            keep_original: false,
            ..spec(ExplodeKind::Object, ExplodeHow::Columns)
        },
    );
    assert_eq!(
        all(&d),
        vec![
            vec!["id", "j.a", "j.b", "n"],
            vec!["1", "1", "2", "x"],
            vec!["2", "", "3", "y"],
            vec!["3", "", "", "z"],
        ]
    );
}

#[test]
fn cancel_stops_a_scan() {
    let mut csv = String::from("id,t\n");
    for i in 0..100_000 {
        csv.push_str(&format!("{i},a|b\n"));
    }
    let d = doc(&csv);
    let p = Progress::new();
    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let s = spec(ExplodeKind::List, ExplodeHow::Rows);
    assert!(matches!(d.explode_scan(&s, &p), Err(Error::Cancelled)));
    assert!(matches!(d.plan_explode_rows(&s, &p), Err(Error::Cancelled)));
}

proptest! {
    /// Unnest of a delimited list is Split into rows.
    #[test]
    fn unnest_equals_split_rows(
        cells in prop::collection::vec("[ab| ]{0,8}", 1..30),
        trim in any::<bool>(),
        skip_empty in any::<bool>(),
    ) {
        let mut csv = String::from("id,t\n");
        for (i, c) in cells.iter().enumerate() {
            csv.push_str(&format!("{i},{c}\n"));
        }
        let mut a = doc(&csv);
        let mut b = doc(&csv);
        let s = Explode { trim, skip_empty, ..spec(ExplodeKind::List, ExplodeHow::Rows) };
        let (p, _) = a.plan_explode_rows(&s, &Progress::new()).unwrap();
        a.apply_rows(p).unwrap();
        let split = SplitRows { col: 1, separator: "|".into(), trim, skip_empty, keep_first: true };
        let p = b.plan_split_rows(&split, &Progress::new()).unwrap();
        b.apply_rows(p).unwrap();
        prop_assert_eq!(all(&a), all(&b));
    }
}
