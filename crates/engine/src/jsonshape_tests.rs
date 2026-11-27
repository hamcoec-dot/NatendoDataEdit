use super::*;
use crate::import::import_json_shaped;
use proptest::prelude::*;
use std::collections::BTreeMap;

fn dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "nde-jsonshape-{}-{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("t")
            .replace("::", "-")
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Open JSON text as the app does (a table plus its shape).
fn open(text: &[u8]) -> Document {
    let (t, shape) = import_json_shaped(text, &Progress::new()).unwrap();
    let mut d = t.into_document().unwrap();
    d.set_json_shape(Some(shape));
    d
}

/// Save as the file's own form; the bytes written and the report.
fn save(d: &mut Document, blanks: Blanks) -> (Vec<u8>, JsonReport) {
    let form = d.json_check().form.expect("a JSON tab");
    save_as(d, &form, blanks)
}

fn save_as(d: &mut Document, form: &JsonForm, blanks: Blanks) -> (Vec<u8>, JsonReport) {
    let p = dir().join(format!("out-{}.json", rand_name()));
    let r = d.save_json(&p, form, blanks, &Progress::new()).unwrap();
    let b = std::fs::read(&p).unwrap();
    let _ = std::fs::remove_file(&p);
    (b, r)
}

fn rand_name() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// Each record as key -> its value's JSON text (spaces outside strings taken out), from raw values so
/// numbers are compared as written.
fn records(bytes: &[u8]) -> Vec<BTreeMap<String, String>> {
    let text =
        std::str::from_utf8(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes)).unwrap();
    // A string as its value (escapes may change on a save), a number as written, a nested value compact.
    let norm = |v: &str| -> String {
        let t = v.trim();
        if t.starts_with('"') {
            serde_json::to_string(&serde_json::from_str::<String>(t).unwrap()).unwrap()
        } else {
            compact_json(t).unwrap_or_else(|| t.to_string())
        }
    };
    let one = |raw: &serde_json::value::RawValue| -> BTreeMap<String, String> {
        let pairs: BTreeMap<String, Box<serde_json::value::RawValue>> =
            serde_json::from_str(raw.get()).unwrap();
        pairs.into_iter().map(|(k, v)| (k, norm(v.get()))).collect()
    };
    let t = text.trim_start();
    if t.starts_with('[') {
        let items: Vec<&serde_json::value::RawValue> = serde_json::from_str(t).unwrap();
        items.into_iter().map(one).collect()
    } else {
        serde_json::Deserializer::from_str(text)
            .into_iter::<&serde_json::value::RawValue>()
            .map(|r| one(r.unwrap()))
            .collect()
    }
}

const MIXED: &str = concat!(
    r#"{"id":12345678901234567890,"name":"Ada \u00e9 😀","price":1.50,"ok":true,"note":null,"tags":["a",1.50,{"n":-0.0}],"meta":{"big":12345678901234567890,"e":1e3}}"#,
    "\n",
    r#"{"name":"Bo","id":2,"price":-0.0,"ok":false,"note":"","tags":[],"meta":{}}"#,
    "\n",
    r#"{"id":3,"name":"","price":1e3,"zip":"02134","note":"x"}"#,
    "\n",
);

#[test]
fn a_save_without_edits_gives_the_same_records() {
    for text in [MIXED.to_string(), MIXED.replace('\n', "\r\n")] {
        let mut d = open(text.as_bytes());
        assert!(
            !d.json_check().ask_blanks,
            "a save with no edits never asks"
        );
        let (out, r) = save(&mut d, Blanks::LeaveOut);
        assert_eq!(records(&out), records(text.as_bytes()));
        assert_eq!(r.records, 3);
        assert_eq!(r.as_text, 0);
        assert_eq!(
            text.contains("\r\n"),
            out.windows(2).any(|w| w == b"\r\n"),
            "line ends kept"
        );
    }
}

#[test]
fn nested_numbers_are_kept_exactly_on_open() {
    let d = open(br#"{"m":{"id":12345678901234567890,"p":1.50,"z":-0.0,"e":1e3},"l":[1.50,1e3]}"#);
    assert_eq!(
        d.row_values(1).unwrap(),
        [
            r#"{"id":12345678901234567890,"p":1.50,"z":-0.0,"e":1e3}"#,
            "[1.50,1e3]"
        ]
    );
}

#[test]
fn a_compact_lf_file_in_column_order_is_saved_byte_for_byte() {
    let text = "{\"a\":1,\"b\":\"x\",\"c\":[1,{\"d\":2.50}]}\n{\"a\":2,\"b\":null}\n{\"a\":3,\"c\":true,\"b\":\"\"}\n";
    // The third record's keys are out of column order: compare its record, not its bytes.
    let exact = "{\"a\":1,\"b\":\"x\",\"c\":[1,{\"d\":2.50}]}\n{\"a\":2,\"b\":null}\n{\"a\":3,\"b\":\"\",\"c\":true}\n";
    let mut d = open(text.as_bytes());
    let (out, _) = save(&mut d, Blanks::LeaveOut);
    assert_eq!(String::from_utf8(out).unwrap(), exact);
    let mut d = open(exact.as_bytes());
    let (out, _) = save(&mut d, Blanks::LeaveOut);
    assert_eq!(String::from_utf8(out).unwrap(), exact);
}

#[test]
fn an_edit_changes_only_that_value() {
    let mut d = open(MIXED.as_bytes());
    let name = d
        .row_values(0)
        .unwrap()
        .iter()
        .position(|x| x == "name")
        .unwrap() as u32;
    d.set_cell(2, name, "Bea".into()).unwrap();
    let (out, _) = save(&mut d, Blanks::LeaveOut);
    let mut want = records(MIXED.as_bytes());
    want[1].insert("name".into(), "\"Bea\"".into());
    assert_eq!(records(&out), want);
}

#[test]
fn a_number_that_no_longer_fits_is_text_and_undo_makes_it_a_number_again() {
    let mut d = open(MIXED.as_bytes());
    let id_col = 0;
    d.set_cell(2, id_col, "abc".into()).unwrap();
    let (out, r) = save(&mut d, Blanks::LeaveOut);
    assert_eq!(r.as_text, 1);
    assert_eq!(records(&out)[1]["id"], "\"abc\"");
    assert!(d.undo());
    let (out, r) = save(&mut d, Blanks::LeaveOut);
    assert_eq!(r.as_text, 0);
    assert_eq!(records(&out)[1]["id"], "2");
    for not in ["012", "1,234", " 12", "+1", "NaN", "Infinity", "1.", ".5"] {
        d.set_cell(2, id_col, not.into()).unwrap();
        let (out, r) = save(&mut d, Blanks::LeaveOut);
        assert_eq!(r.as_text, 1, "{not}");
        assert_eq!(
            records(&out)[1]["id"],
            serde_json::to_string(not).unwrap(),
            "{not}"
        );
        assert!(d.undo());
    }
}

#[test]
fn blanks_keep_their_own_form_and_only_new_ones_use_the_choice() {
    // note: null, "", and a value (mixed blanks); name: "" only (one blank kind); zip: missing only.
    let mut d = open(MIXED.as_bytes());
    let names = d.row_values(0).unwrap();
    let col = |n: &str| names.iter().position(|x| x == n).unwrap() as u32;
    // A newly blank cell in "note" (mixed blanks) needs the question; in "name" ("" only) it does not.
    d.set_cell(3, col("name"), "Cy".into()).unwrap();
    d.set_cell(3, col("name"), String::new()).unwrap();
    assert!(!d.json_check().ask_blanks, "name's blanks were all \"\"");
    let (out, _) = save(&mut d, Blanks::Null);
    assert_eq!(records(&out)[2]["name"], "\"\"");
    d.set_cell(3, col("note"), String::new()).unwrap();
    assert!(d.json_check().ask_blanks);
    for (b, want) in [
        (Blanks::LeaveOut, None),
        (Blanks::EmptyText, Some("\"\"")),
        (Blanks::Null, Some("null")),
    ] {
        let (out, _) = save(&mut d, b);
        let recs = records(&out);
        assert_eq!(recs[2].get("note").map(String::as_str), want, "{b:?}");
        // The unchanged blanks keep their own form.
        assert_eq!(recs[0]["note"], "null");
        assert_eq!(recs[1]["note"], "\"\"");
        assert!(!recs[1].contains_key("zip"));
    }
}

#[test]
fn rows_and_columns_added_deleted_moved_and_renamed() {
    let mut d = open(MIXED.as_bytes());
    let names = d.row_values(0).unwrap();
    let col = |n: &str| names.iter().position(|x| x == n).unwrap() as u32;
    // A new row: values take the column's kind.
    d.insert_rows(4, vec![vec!["4".into(), "Dee".into(), "2.5".into()]])
        .unwrap();
    // Delete the second record; rename "ok" to "active"; move "price" first; a new column "extra".
    d.delete_rows(2, 1).unwrap();
    d.set_cell(0, col("ok"), "active".into()).unwrap();
    d.move_cols(col("price"), 1, 0).unwrap();
    let n = d.col_count();
    d.insert_cols(n, 1).unwrap();
    d.set_cell(0, n, "extra".into()).unwrap();
    d.set_cell(1, n, "7".into()).unwrap();
    let (out, _) = save(&mut d, Blanks::LeaveOut);
    let recs = records(&out);
    assert_eq!(recs.len(), 3);
    assert_eq!(recs[0]["active"], "true");
    assert!(!recs[0].contains_key("ok"));
    assert_eq!(recs[0]["extra"], "\"7\"", "a new column is text");
    assert_eq!(recs[0]["price"], "1.50");
    assert_eq!(recs[1]["id"], "3");
    assert_eq!(
        recs[2]["id"], "4",
        "a new row's id is a number like its column"
    );
    assert_eq!(recs[2]["price"], "2.5");
    assert_eq!(recs[2]["name"], "\"Dee\"");
    // A key the new row has no value for is left out (the choice), the first key in the file order.
    let first = std::str::from_utf8(&out).unwrap().lines().next().unwrap();
    assert!(first.starts_with("{\"price\":"), "{first}");
}

#[test]
fn what_cannot_be_saved_back_says_why() {
    let dup = open(b"{\"id\":1}\n{\"id\":2,\"id\":3}\n");
    let why = dup.json_check().refuse.unwrap();
    assert!(why.contains("record 2 has the key 'id' twice"), "{why}");
    let lists = open(b"[[1,2],[3,4]]");
    assert!(lists
        .json_check()
        .refuse
        .unwrap()
        .contains("lists of values"));
    let wrapped = open(br#"{"rows":[{"a":1},{"a":2}]}"#);
    assert!(wrapped
        .json_check()
        .refuse
        .unwrap()
        .contains("inside an object"));
    let mut d = open(b"{\"a\":1,\"b\":2}\n");
    d.set_cell(0, 1, "a".into()).unwrap();
    let why = d.json_check().refuse.unwrap_or_default();
    assert!(why.contains("both called 'a'"), "{why}");
    d.set_cell(0, 1, String::new()).unwrap();
    assert!(d.json_check().refuse.unwrap().contains("no title"));
    // A key "" the file had is kept.
    let mut e = open(b"{\"\":1,\"b\":2}\n");
    assert_eq!(e.json_check().refuse, None);
    let (out, _) = save(&mut e, Blanks::LeaveOut);
    assert_eq!(out, b"{\"\":1,\"b\":2}\n");
}

#[test]
fn a_lone_surrogate_is_refused_at_open() {
    assert!(import_json_shaped(br#"{"a":"\ud800"}"#, &Progress::new()).is_err());
}

#[test]
fn json_arrays_keep_their_layout_and_a_bom_is_kept() {
    let compact = "[{\"a\":1,\"b\":[1.50]},{\"a\":2}]\n";
    let mut d = open(compact.as_bytes());
    assert_eq!(save(&mut d, Blanks::LeaveOut).0, compact.as_bytes());
    for ind in ["  ", "    ", "\t"] {
        let pretty = format!("[\n{ind}{{\n{ind}{ind}\"a\": 1,\n{ind}{ind}\"b\": \"x\"\n{ind}}},\n{ind}{{\n{ind}{ind}\"a\": 2\n{ind}}}\n]\n");
        let mut d = open(pretty.as_bytes());
        assert_eq!(d.json_check().form.unwrap().indent.as_deref(), Some(ind));
        assert_eq!(
            String::from_utf8(save(&mut d, Blanks::LeaveOut).0).unwrap(),
            pretty
        );
    }
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(b"{\"a\":1}\n");
    let mut d = open(&bom);
    assert_eq!(save(&mut d, Blanks::LeaveOut).0, bom);
    // Save As: the other form.
    let mut d = open(compact.as_bytes());
    let lines = JsonForm {
        lines: true,
        indent: None,
        crlf: false,
        bom: false,
        final_newline: true,
    };
    assert_eq!(
        save_as(&mut d, &lines, Blanks::LeaveOut).0,
        b"{\"a\":1,\"b\":[1.50]}\n{\"a\":2}\n"
    );
}

#[test]
fn empty_records_are_kept() {
    let text = "{\"a\":1}\n{}\n{}\n";
    let mut d = open(text.as_bytes());
    assert_eq!(save(&mut d, Blanks::LeaveOut).0, text.as_bytes());
}

#[test]
fn the_save_marks_the_tab_saved_and_keeps_undo() {
    let mut d = open(MIXED.as_bytes());
    d.set_cell(1, 1, "Ann".into()).unwrap();
    assert!(d.is_modified());
    let _ = save(&mut d, Blanks::LeaveOut);
    assert!(!d.is_modified());
    assert!(d.undo());
    assert!(d.is_modified());
}

#[test]
fn the_shape_is_small() {
    // 100,000 records, 5% of ids written as text.
    let mut s = String::new();
    for i in 0..100_000u32 {
        if i % 20 == 0 {
            s.push_str(&format!("{{\"id\":\"{i}\",\"v\":{i}.5}}\n"));
        } else {
            s.push_str(&format!("{{\"id\":{i},\"v\":{i}.5}}\n"));
        }
    }
    let d = open(s.as_bytes());
    let bytes = d.json_shape().unwrap().bytes();
    assert!(bytes < 50_000, "{bytes} bytes");
    let mut d = d;
    let (out, r) = save(&mut d, Blanks::LeaveOut);
    assert_eq!(r.as_text, 0);
    assert_eq!(out, s.as_bytes());
}

proptest! {
    #[test]
    fn random_records_save_back_unchanged(
        rows in prop::collection::vec(
            prop::collection::vec((0usize..4, prop_oneof![
                Just("1".to_string()), Just("1.50".to_string()), Just("-0.0".to_string()), Just("1e3".to_string()),
                Just("12345678901234567890".to_string()), Just("true".to_string()), Just("false".to_string()),
                Just("null".to_string()), Just("\"\"".to_string()), Just("\"x\"".to_string()), Just("\"1\"".to_string()),
                Just("\"true\"".to_string()), Just("[1.50,{\"a\":null}]".to_string()), Just("{}".to_string()),
                Just("\"\\u00e9\"".to_string()),
            ]), 0..4),
            2..12,
        ),
    ) {
        // A file whose records have no keys at all is saved as another file (tested above).
        prop_assume!(rows.iter().any(|r| !r.is_empty()));
        // One record alone whose values are all objects is read as records inside an object (as today).
        prop_assume!(rows.len() >= 2);
        let keys = ["a", "b", "c", "d"];
        let mut text = String::new();
        for r in &rows {
            let mut seen = std::collections::HashSet::new();
            let members: Vec<String> = r
                .iter()
                .filter(|(k, _)| seen.insert(*k))
                .map(|(k, v)| format!("\"{}\":{}", keys[*k], v))
                .collect();
            text.push('{');
            text.push_str(&members.join(","));
            text.push_str("}\n");
        }
        let mut d = open(text.as_bytes());
        prop_assert!(!d.json_check().ask_blanks);
        let (out, r) = save(&mut d, Blanks::LeaveOut);
        prop_assert_eq!(r.as_text, 0);
        let norm = |b: &[u8]| -> Vec<BTreeMap<String, String>> {
            records(b)
                .into_iter()
                .map(|m| {
                    m.into_iter()
                        .map(|(k, v)| (k, if v == "\"\\u00e9\"" { "\"é\"".to_string() } else { v }))
                        .collect()
                })
                .collect()
        };
        prop_assert_eq!(norm(&out), norm(text.as_bytes()));
    }
}

/// The quick paths of the writer give what serde_json gives: strings with and without escapes, nested values
/// with and without spaces.
#[test]
fn quick_writer_paths_match_serde() {
    for s in ["plain", "caf\u{e9} \u{1F600}", "say \"hi\"", "back\\slash", "tab\tline\nend\u{1}", ""] {
        let mut out = String::new();
        json_string(s, &mut out);
        assert_eq!(out, serde_json::to_string(s).unwrap(), "{s:?}");
    }
    for (s, want) in [("{\"a\":[1,2.50]}", "{\"a\":[1,2.50]}"), ("{ \"a\" : [1, \"x y\"] }", "{\"a\":[1,\"x y\"]}")] {
        assert!(is_json_container(s));
        let mut out = String::new();
        push_compact(s, &mut out);
        assert_eq!(out, want);
    }
    assert!(!is_json_container("{\"a\":"));
}
