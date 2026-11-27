//! Blank templates (titles over rows of separators only) and files with
//! some empty rows: the separator guess. Made-up data only.

use super::*;
use proptest::prelude::*;

fn with_ext(text: &[u8], ext: &'static str) -> Detection {
    detect(
        text,
        Hints {
            ext: Some(ext),
            quote: None,
        },
    )
}

/// The guess for `text` with each extension: (separator, skip, columns).
fn guesses(text: &[u8], exts: &[&'static str]) -> Vec<(Vec<u8>, u32, u32)> {
    exts.iter()
        .map(|e| {
            let c = with_ext(text, e).best().clone();
            (c.dialect.delimiter().to_vec(), c.skip_lines, c.columns)
        })
        .collect()
}

fn reads_as(text: &[u8], exts: &[&'static str], sep: &[u8], cols: u32) {
    for (e, g) in exts.iter().zip(guesses(text, exts)) {
        assert_eq!(
            g,
            (sep.to_vec(), 0, cols),
            "{e}: {}",
            String::from_utf8_lossy(text)
        );
    }
}

#[test]
fn a_comma_template_with_one_value_reads_with_comma() {
    let mut t = b"x,y,z\n".to_vec();
    for _ in 0..7 {
        t.extend_from_slice(b",,\n");
    }
    t.extend_from_slice(b",a,\n");
    reads_as(&t, &["csv", "txt"], b",", 3);
}

#[test]
fn a_comma_template_reads_with_comma_and_stays_doubtful() {
    let t = b"x,y,z\n,,\n,,\n,,\n";
    reads_as(t, &["csv", "txt"], b",", 3);
    // Nothing under the titles to judge by: the guess asks to be checked.
    for e in ["csv", "txt"] {
        assert!(!with_ext(t, e).is_confident(), "{e}");
    }
}

#[test]
fn semicolon_and_pipe_templates() {
    reads_as(b"x;y;z\n;;\n;;\n;;\n", &["csv", "txt"], b";", 3);
    reads_as(b"x|y|z\n||\n||\n||\n", &["csv", "txt"], b"|", 3);
}

#[test]
fn a_tab_template_reads_with_tab_and_stays_doubtful() {
    let t = b"a\tb\tc\n\t\t\n\t\t\n\t\t\n";
    reads_as(t, &["csv", "txt", "tsv"], b"\t", 3);
    for e in ["csv", "txt", "tsv"] {
        assert!(!with_ext(t, e).is_confident(), "{e}");
    }
}

#[test]
fn files_with_values_read_as_before() {
    reads_as(
        b"a\tb\tc\n1\t2\t3\n4\t5\t6\n7\t8\t9\n",
        &["csv", "txt", "tsv"],
        b"\t",
        3,
    );
    reads_as(b"x,y,z\n,,\n,,\n,,\n1,2,3\n,,\n", &["csv", "txt"], b",", 3);
    reads_as(b"x,y,z\n,,\n,,\n1,2,3\n", &["csv", "txt"], b",", 3);
    // A voter-style file: an id in every row, the names blank.
    let mut v = b"id,first,last,ward\n".to_vec();
    for i in 1..=20 {
        v.extend_from_slice(format!("{i},,,W{}\n", i % 4 + 1).as_bytes());
    }
    reads_as(&v, &["csv", "txt"], b",", 4);
}

#[test]
fn a_double_comma_file_with_values_keeps_its_separator() {
    let t = b"id,,name,,ward\n1,,Ann Quorrin,,W1\n2,,Bo Imbert,,W2\n3,,Cy Gallow,,W3\n4,,Di Brasko,,W4\n";
    reads_as(t, &["csv", "txt"], b",,", 3);
}

#[test]
fn a_double_comma_file_half_empty_rows_keeps_its_separator() {
    // R1: the empty rows (`,,,,`) hold no value, so they are left out of both
    // sides of the "most lines" bar.
    let mut t = b"id,,name,,ward\n".to_vec();
    for i in 1..=8 {
        t.extend_from_slice(format!("{i},,Ann Quorrin {i},,W{}\n", i % 4 + 1).as_bytes());
        t.extend_from_slice(b",,,,\n");
    }
    reads_as(&t, &["csv", "txt"], b",,", 3);
}

#[test]
fn the_comma_template_read_as_tab_still_loses() {
    // R2: under tab, `,,` rows are one field each, not three.
    let t = b"x,y,z\n,,\n,,\n,,\n";
    for e in ["tsv", "txt"] {
        let d = with_ext(t, e);
        assert_eq!(d.best().dialect.delimiter(), b",", "{e}");
        assert_eq!(d.best().columns, 3, "{e}");
    }
}

#[test]
fn mixed_comma_and_tab_empty_rows_are_not_confident() {
    let t = b"x,y,z\n,,\n\t\t\n,,\n\t\t\n,,\n\t\t\n";
    for e in ["csv", "txt", "tsv"] {
        assert!(!with_ext(t, e).is_confident(), "{e}");
    }
}

/// A cell value with no separator, quote or line break in it.
fn cell() -> impl Strategy<Value = String> {
    prop_oneof![
        "[A-Za-z]{1,8}",
        "[0-9]{1,5}",
        "[A-Za-z]{2,6} [A-Za-z]{2,6}",
        Just(String::new()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]
    /// R3: all-empty rows inserted into a table with values do not change its
    /// separator.
    #[test]
    fn empty_rows_do_not_change_the_separator(
        sep in prop::sample::select(vec![b',', b'\t', b';', b'|']),
        cols in 2usize..6,
        rows in prop::collection::vec(prop::collection::vec(cell(), 6), 4..30),
        empties in prop::collection::vec(any::<prop::sample::Index>(), 1..10),
    ) {
        let s = sep as char;
        let line = |r: &[String]| r[..cols].join(&s.to_string());
        let titles: Vec<String> = (0..cols).map(|c| format!("t{c}")).collect();
        // Every row holds at least one value.
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|mut r| {
                if r[..cols].iter().all(String::is_empty) {
                    r[0] = "v".into();
                }
                r
            })
            .collect();
        let mut plain = vec![line(&titles)];
        plain.extend(rows.iter().map(|r| line(r)));
        let blank = std::iter::repeat_n(s, cols - 1).collect::<String>();
        let mut padded = plain.clone();
        for i in &empties {
            let at = 1 + i.index(padded.len());
            padded.insert(at, blank.clone());
        }
        let a = detect((plain.join("\n") + "\n").as_bytes(), Hints::default());
        let b = detect((padded.join("\n") + "\n").as_bytes(), Hints::default());
        prop_assert_eq!(a.best().dialect.delimiter(), b.best().dialect.delimiter());
    }
}
