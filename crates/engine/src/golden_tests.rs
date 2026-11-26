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

fn rows(t: Table) -> Vec<Vec<String>> {
    let d = t.into_document().unwrap();
    (0..d.row_count())
        .map(|r| d.row_values(r).unwrap())
        .collect()
}

fn plan(d: &Document, pairs: &[(u32, u32)], spec: &GoldenSpec) -> GoldenPlan {
    d.plan_golden(pairs, spec, &Progress::new()).unwrap()
}

fn tables(d: &Document, p: &GoldenPlan) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let (a, b) = d.golden_tables(p, true, &Progress::new()).unwrap();
    (rows(a), rows(b.unwrap()))
}

fn members(p: &GoldenPlan) -> Vec<Vec<u32>> {
    p.clusters.iter().map(|c| c.members.clone()).collect()
}

fn reasons(p: &GoldenPlan) -> Vec<(u32, Reason)> {
    p.review.clone()
}

const PEOPLE: &str = "id,name\r\nA,a\r\nB,b\r\nC,c\r\nD,d\r\nE,e\r\nF,f\r\nG,g\r\n";
// Row ids (the title row is 0).
const A: u32 = 1;
const B: u32 = 2;
const C: u32 = 3;
const D: u32 = 4;
const E: u32 = 5;
const F: u32 = 6;
const G: u32 = 7;

#[test]
fn a_chain_never_pulls_a_stranger_in() {
    let d = doc(PEOPLE);
    // A~B~C: B is the hub; A and C were each accepted with it.
    let p = plan(&d, &[(A, B), (B, C)], &Default::default());
    assert_eq!(members(&p), [[B, A, C]]);
    assert!(p.clusters[0].hub_only && p.review.is_empty());
    // A~B~C~D: D is not pulled in through C, which goes to review.
    let p = plan(&d, &[(A, B), (B, C), (C, D)], &Default::default());
    assert_eq!(members(&p), [[B, A]]);
    assert_eq!(
        reasons(&p),
        [(C, Reason::TwoClusters), (D, Reason::PartnerUnderReview)]
    );
    assert_eq!(p.counts.unmerged_pairs, 2);
    assert_eq!(p.counts.review, 2);
}

/// Every paired row is merged or listed for review (a reviewer's graph that
/// once left row 1 out without a word).
#[test]
fn no_paired_row_is_dropped_silently() {
    let mut text = String::from("id\r\n");
    for i in 1..=10 {
        text.push_str(&format!("{i}\r\n"));
    }
    let d = doc(&text);
    let pairs = [
        (1, 6),
        (9, 7),
        (4, 3),
        (2, 10),
        (3, 9),
        (6, 4),
        (4, 8),
        (10, 8),
        (2, 1),
        (1, 9),
        (7, 2),
    ];
    for require_all in [false, true] {
        for max_cluster in [2, 3, 10] {
            let spec = GoldenSpec {
                require_all,
                max_cluster,
                ..Default::default()
            };
            let p = plan(&d, &pairs, &spec);
            for (a, b) in pairs {
                for r in [a, b] {
                    assert!(
                        p.clusters.iter().any(|c| c.members.contains(&r))
                            || p.review.iter().any(|x| x.0 == r),
                        "row {r} (strict {require_all}, at most {max_cluster})"
                    );
                }
            }
        }
    }
}

#[test]
fn pairs_into_two_clusters_are_reviewed() {
    let d = doc(PEOPLE);
    // Two hubs (A and E) share B: B is torn, and so is E, whose pair with B
    // leads into A's cluster.
    let pairs = [(A, B), (A, C), (A, D), (E, F), (E, G), (E, B)];
    let p = plan(&d, &pairs, &Default::default());
    assert_eq!(members(&p), [[A, C, D]]);
    assert_eq!(
        reasons(&p),
        [
            (B, Reason::TwoClusters),
            (E, Reason::TwoClusters),
            (F, Reason::HubUnderReview),
            (G, Reason::HubUnderReview),
        ]
    );
}

#[test]
fn large_and_partly_accepted_clusters() {
    let d = doc(PEOPLE);
    let star = [(A, B), (A, C), (A, D)];
    let spec = GoldenSpec {
        max_cluster: 3,
        ..Default::default()
    };
    let p = plan(&d, &star, &spec);
    assert!(p.clusters.is_empty());
    assert_eq!(p.review.len(), 4);
    assert!(p.review.iter().all(|r| r.1 == Reason::TooLarge));
    // Every pair accepted: not marked; one missing: marked, or reviewed
    // with the tick.
    let full = [(A, B), (A, C), (B, C), (E, F), (E, G)];
    let p = plan(&d, &full, &Default::default());
    let marks: Vec<bool> = p.clusters.iter().map(|c| c.hub_only).collect();
    assert_eq!(marks, [false, true]);
    assert_eq!(p.counts.hub_only, 1);
    let view = d.golden_page(&p, 1, 1).unwrap();
    assert!(view[0].hub_only);
    assert_eq!(view[0].members, ["row 6", "row 7", "row 8"]);
    let strict = GoldenSpec {
        require_all: true,
        ..Default::default()
    };
    let p = plan(&d, &full, &strict);
    assert_eq!(members(&p), [[A, B, C]]);
    assert_eq!(
        reasons(&p),
        [
            (E, Reason::NotAllPairs),
            (F, Reason::NotAllPairs),
            (G, Reason::NotAllPairs)
        ]
    );
    let lines = d.golden_review(&p).unwrap();
    assert_eq!(lines[0].name, "row 6");
}

/// Random pair graphs: no row in two clusters, every merged row was accepted
/// with its hub, and every row of the table is in the result or merged away.
/// Reversed, shuffled and repeated pairs give the same bytes.
#[test]
fn random_graphs_keep_every_row_and_ignore_the_pair_order() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for round in 0..600 {
        let n = 2 + next(14) as u32;
        let mut text = String::from("id,v\r\n");
        for i in 1..=n {
            text.push_str(&format!("{i},{}\r\n", next(4)));
        }
        let d = doc(&text);
        let mut pairs: Vec<(u32, u32)> = (0..1 + next(2 * u64::from(n)))
            .map(|_| (1 + next(n.into()) as u32, 1 + next(n.into()) as u32))
            .collect();
        let spec = GoldenSpec {
            id_col: Some(0),
            max_cluster: 2 + next(5) as u32,
            require_all: next(3) == 0,
            rules: vec![ColumnRule {
                col: 1,
                rule: Rule::Union {
                    separator: "|".into(),
                },
            }],
            ..Default::default()
        };
        let p = plan(&d, &pairs, &spec);
        let accepted = |a: u32, b: u32| pairs.iter().any(|&p| p == (a, b) || p == (b, a));
        let mut seen = HashSet::new();
        for c in &p.clusters {
            assert!(c.members.len() >= 2 && c.members.len() <= spec.max_cluster as usize);
            assert_eq!(c.hub, c.members[0]);
            for &m in &c.members {
                assert!(seen.insert(m), "round {round}: {m} twice");
                assert!(m == c.hub || accepted(m, c.hub), "round {round}");
                if spec.require_all {
                    assert!(c.members.iter().all(|&o| o == m || accepted(o, m)));
                }
            }
        }
        for &(r, _) in &p.review {
            assert!(seen.insert(r), "round {round}: {r} merged and reviewed");
        }
        // And no paired row is in neither.
        for &(a, b) in pairs.iter().filter(|(a, b)| a != b) {
            assert!(
                seen.contains(&a) && seen.contains(&b),
                "round {round}: a row of ({a}, {b}) is not accounted for"
            );
        }
        let first = (p.counts.clone(), p.clusters.clone(), p.review.clone());
        // The same pairs another way.
        pairs.reverse();
        for i in (1..pairs.len()).rev() {
            pairs.swap(i, next(i as u64 + 1) as usize);
        }
        let again: Vec<(u32, u32)> = pairs
            .iter()
            .flat_map(|&(a, b)| [(b, a), (a, b), (b, a)])
            .collect();
        let q = plan(&d, &again, &spec);
        assert_eq!(
            first,
            (q.counts.clone(), q.clusters.clone(), q.review.clone()),
            "round {round}"
        );
        if p.counts.clusters == 0 {
            continue;
        }
        let (out, away) = tables(&d, &p);
        assert_eq!((out.clone(), away.clone()), tables(&d, &q), "round {round}");
        // Conservation: each row once, as itself, as a merged row's hub, or
        // merged away.
        let mut found: Vec<u32> = Vec::new();
        for r in &out[1..] {
            if r[3].is_empty() {
                found.push(r[0].parse().unwrap());
            } else {
                let hub: u32 = r[2].split("; ").next().unwrap().parse().unwrap();
                found.push(hub);
            }
        }
        for r in &away[1..] {
            let id: u32 = r[2].parse().unwrap();
            assert_eq!(r[1], (id + 1).to_string());
            found.push(id);
        }
        found.sort_unstable();
        assert_eq!(found, (1..=n).collect::<Vec<_>>(), "round {round}");
        assert_eq!(out.len() as u64 - 1, p.counts.rows);
        assert_eq!(away.len() as u64 - 1, p.counts.folded);
    }
}

#[test]
fn hidden_rows_are_kept_in_place() {
    let mut d = doc("id,name\r\nA,a\r\nB,b\r\nC,c\r\nD,d\r\nE,e\r\n");
    d.hide_rows(3, 2).unwrap();
    assert_eq!(d.row_count(), 4);
    let p = plan(
        &d,
        &[(1, 2), (2, 5)],
        &GoldenSpec {
            id_col: Some(0),
            ..Default::default()
        },
    );
    assert_eq!(p.counts.hidden, 2);
    let (out, away) = tables(&d, &p);
    assert_eq!(
        out,
        [
            vec!["id", "name", "Merged_IDs", "Merged_Count"],
            vec!["B", "b", "B; A; E", "3"],
            vec!["C", "c", "", ""],
            vec!["D", "d", "", ""],
        ]
    );
    assert_eq!(
        away,
        [
            vec!["Cluster", "File row", "id", "name"],
            vec!["1", "2", "A", "a"],
            vec!["1", "6", "E", "e"],
        ]
    );
    // The original is as it was.
    assert_eq!(d.row_count(), 4);
}

#[test]
fn names_after_a_sort_new_rows_and_taken_titles() {
    let mut d = doc("n,merged_ids,Merged_Count\r\nzed,1,\r\namy,2,\r\n");
    d.insert_rows(3, vec![vec!["bob".into(), "3".into()]])
        .unwrap();
    let sort = SortSpec {
        keys: vec![SortKey {
            col: 0,
            descending: false,
            kind: Default::default(),
            match_case: false,
        }],
        keep_first: true,
        ..Default::default()
    };
    let s = d.plan_sort(&sort, &Progress::new()).unwrap();
    d.apply_sort(s).unwrap();
    // amy (file row 3), bob (added), zed (file row 2).
    let ids: Vec<u32> = d.ids().skip(1).collect();
    let p = plan(
        &d,
        &[(ids[0], ids[1]), (ids[0], ids[2])],
        &Default::default(),
    );
    let (out, away) = tables(&d, &p);
    assert_eq!(
        out[0],
        [
            "n",
            "merged_ids",
            "Merged_Count",
            "Merged_IDs (2)",
            "Merged_Count (2)"
        ]
    );
    assert_eq!(out[1][3], "row 3; added by hand; row 2");
    assert_eq!(away[2][1], "2");
    assert_eq!(away[1][1], "added by hand");
}

#[test]
fn a_skipped_cluster_stays_and_a_changed_table_is_refused() {
    let mut d = doc(PEOPLE);
    let pairs = [(A, B), (C, D)];
    let spec = GoldenSpec {
        skip: vec![A],
        id_col: Some(0),
        ..Default::default()
    };
    let p = plan(&d, &pairs, &spec);
    assert_eq!(
        (p.counts.clusters, p.counts.skipped, p.counts.folded),
        (1, 1, 1)
    );
    let (out, _) = tables(&d, &p);
    assert_eq!(out.len(), 7);
    assert_eq!(out[1], ["A", "a", "", ""]);
    d.set_cell(1, 1, "changed".into()).unwrap();
    assert!(matches!(
        d.golden_tables(&p, false, &Progress::new()),
        Err(Error::Changed)
    ));
    assert!(matches!(d.golden_page(&p, 0, 10), Err(Error::Changed)));
    assert!(matches!(d.golden_review(&p), Err(Error::Changed)));
    assert!(d
        .plan_golden(&[], &spec, &Progress::new())
        .unwrap_err()
        .to_string()
        .contains("at least one pair"));
    // A pair naming a row that is not there (any more).
    assert!(matches!(
        d.plan_golden(&[(1, 99)], &spec, &Progress::new()),
        Err(Error::Changed)
    ));
}

fn one(rule: Rule, values: &[&str]) -> (String, Option<Flag>) {
    let r: Vec<(&str, &str)> = values.iter().map(|v| (*v, "")).collect();
    choose(&rule, &r, 0, ReadOptions::default())
}

#[test]
fn text_rules_keep_text_and_blank_never_wins() {
    assert_eq!(one(Rule::Keep, &["007", "7"]).0, "007");
    // The hub's value, wherever it is; else the first.
    let r = [("x", ""), ("y", "")];
    assert_eq!(choose(&Rule::Keep, &r, 1, Default::default()).0, "y");
    let r = [("x", ""), (" ", "")];
    assert_eq!(choose(&Rule::Keep, &r, 1, Default::default()).0, "x");
    assert_eq!(one(Rule::First, &["", " 00 ", "b"]).0, " 00 ");
    assert_eq!(one(Rule::Last, &["a", "b", ""]).0, "b");
    assert_eq!(one(Rule::Longest, &["ab", "cde", "fgh"]).0, "cde");
    assert_eq!(one(Rule::MostCommon, &["a", "b", "b", "a"]).0, "a");
    assert_eq!(one(Rule::MostCommon, &["", "", "b"]).0, "b");
    let union = Rule::Union {
        separator: "; ".into(),
    };
    assert_eq!(one(union.clone(), &["Tag", "tag", "", "Tag"]).0, "Tag; tag");
    assert_eq!(one(union, &["", ""]).0, "");
    let long = "x".repeat(MAX_JOINED);
    let union = Rule::Union {
        separator: ",".into(),
    };
    assert_eq!(one(union, &["a", &long]), ("a".into(), Some(Flag::TooLong)));
}

#[test]
fn numbers_are_exact_or_left() {
    assert_eq!(one(Rule::Sum, &["0.1", "0.2"]), ("0.3".into(), None));
    assert_eq!(one(Rule::Sum, &["1,000.50", "-0.5", ""]).0, "1000.00");
    assert_eq!(one(Rule::Sum, &["0.05", "-0.10"]).0, "-0.05");
    assert_eq!(one(Rule::Sum, &[".5", "+1"]).0, "1.5");
    let nines = "9".repeat(28);
    assert_eq!(
        one(Rule::Sum, &[&nines, "1"]),
        (nines.clone(), Some(Flag::Overflow))
    );
    for odd in [
        "1e3", "$12.50", "12.50-", "1,00", "12,34.5", "ten", "1.", "-", "1 000",
    ] {
        assert_eq!(
            one(Rule::Sum, &["2", odd]),
            ("2".into(), Some(Flag::NotNumber)),
            "{odd}"
        );
    }
    let big = "1".repeat(29);
    assert_eq!(one(Rule::Max, &["2", &big]).1, Some(Flag::NotNumber));
    // Scaling past what can be held.
    let small = format!("0.{}", "1".repeat(28));
    assert_eq!(one(Rule::Max, &[&nines, &small]).1, Some(Flag::Overflow));
    // Min and max keep the winner's own text.
    assert_eq!(one(Rule::Min, &["010", "9.50", "9.5"]).0, "9.50");
    assert_eq!(one(Rule::Max, &["010", "9.50"]).0, "010");
    let comma = ReadOptions {
        decimal_comma: true,
        ..Default::default()
    };
    let r = [("1.000,5", ""), ("0,25", "")];
    assert_eq!(choose(&Rule::Sum, &r, 0, comma).0, "1000,75");
}

#[test]
fn newest_and_oldest_need_certain_dates() {
    let newest = Rule::Newest {
        blank_counts: false,
    };
    let pick = |rule: &Rule, r: &[(&str, &str)]| choose(rule, r, 0, Default::default());
    let r = [
        ("old", "2024-01-05"),
        ("new", "March 3, 2025"),
        ("mid", "12/31/2024"),
    ];
    assert_eq!(pick(&newest, &r), ("new".into(), None));
    assert_eq!(pick(&Rule::Oldest, &r), ("old".into(), None));
    // A tie: file order.
    let r = [("a", "2025-01-01"), ("b", "2025-01-01")];
    assert_eq!(pick(&newest, &r).0, "a");
    for doubt in ["03/04/2025", "1/2/24", "20250101", "soon", ""] {
        let r = [("a", "2020-01-01"), ("b", doubt)];
        assert_eq!(
            pick(&newest, &r),
            ("a".into(), Some(Flag::Date)),
            "{doubt:?}"
        );
    }
    // A blank value's date does not matter, unless a newer blank counts.
    let r = [("a", "2020-01-01"), ("", "2025-01-01")];
    assert_eq!(pick(&newest, &r).0, "a");
    assert_eq!(
        pick(&Rule::Newest { blank_counts: true }, &r),
        ("".into(), None)
    );
}

#[test]
fn flags_show_in_the_preview() {
    let d = doc("n,paid,when\r\nann,$5,03/04/2025\r\nann,2,2025-01-01\r\n");
    let spec = GoldenSpec {
        rules: vec![
            ColumnRule {
                col: 0,
                rule: Rule::Newest {
                    blank_counts: false,
                },
            },
            ColumnRule {
                col: 1,
                rule: Rule::Sum,
            },
        ],
        date_col: Some(2),
        ..Default::default()
    };
    let p = plan(&d, &[(1, 2)], &spec);
    assert_eq!(p.counts.flagged, 2);
    let view = d.golden_page(&p, 0, 10).unwrap();
    let cell = |col| view[0].cells.iter().find(|c| c.col == col).unwrap();
    // Flagged although the members agree.
    assert!(cell(0).flag.as_deref().unwrap().contains("date"));
    assert!(cell(1)
        .flag
        .as_deref()
        .unwrap()
        .contains("not an exact number"));
    assert_eq!(cell(1).value, "$5");
    assert_eq!(cell(1).differ, ["2"]);
    // No date column chosen.
    let spec = GoldenSpec {
        date_col: None,
        ..spec
    };
    assert!(d.plan_golden(&[(1, 2)], &spec, &Progress::new()).is_err());
}
