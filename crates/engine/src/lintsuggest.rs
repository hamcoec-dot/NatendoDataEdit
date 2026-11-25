//! "Suggest rules from this data": rules a column's own values hint at, offered as a list
//! the person ticks. Nothing is applied here. A rule is only offered when it would pass at
//! least 99% of the sampled values, so a suggestion never starts out as a sea of problems.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::document::{Document, Progress};
use crate::find::{Pattern, PatternKind};
use crate::profile::{shape_of, shape_regex};
use crate::validate::{is_iso_date, Check, CheckKind};
use crate::value::ReadOptions;
use crate::Error;

/// Rows looked at per column (spread evenly over the table).
pub const SAMPLE_ROWS: u64 = 200_000;
/// A rule must pass at least this share of the filled sampled values.
const MIN_SHARE: f64 = 0.99;
/// Fewest filled values worth suggesting anything from.
const MIN_FILLED: usize = 5;
/// Longest value looked at (longer ones are free text).
const MAX_LEN: usize = 200;
/// Different values remembered for the "all different" test.
const MAX_DISTINCT: usize = 400_000;

/// One suggested rule.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub check: Check,
    /// Why, in a sentence ("all 1,200 sampled values are written 999-9999").
    pub why: String,
    /// Share of the filled sampled values the rule passes, 0 to 1.
    pub share: f64,
}

fn pattern_rule(col: u32, shape: &str, name: String) -> Check {
    Check {
        col: Some(col),
        name: Some(name),
        kind: CheckKind::Pattern {
            pattern: Pattern {
                text: shape_regex(shape),
                kind: PatternKind::Regex,
                match_case: true,
                ..Default::default()
            },
        },
    }
}

impl Document {
    /// Rules the values of column `col` suggest, best first. Looks at up to `SAMPLE_ROWS`
    /// rows spread over the table (the title row is skipped when `keep_first`).
    pub fn suggest_rules(
        &self,
        col: u32,
        keep_first: bool,
        progress: &Progress,
    ) -> Result<Vec<Suggestion>, Error> {
        self.ready_for_bulk()?;
        if col >= self.col_count() {
            return Err(Error::Invalid("no such column".into()));
        }
        let n = self.row_count();
        let first = u64::from(keep_first && n > 0);
        let rows = n.saturating_sub(first);
        let mut out = Vec::new();
        if rows == 0 {
            return Ok(out);
        }
        let take = rows.min(SAMPLE_ROWS);
        progress.start(take);
        let read = ReadOptions::default();
        let mut blanks = 0usize;
        let mut filled = 0usize;
        let mut shapes: HashMap<String, usize> = HashMap::new();
        let mut distinct: HashSet<String> = HashSet::new();
        let mut distinct_full = false;
        let mut numbers = 0usize;
        let mut whole = 0usize;
        let mut iso = 0usize;
        let mut lens: HashMap<usize, usize> = HashMap::new();
        for k in 0..take {
            if k % 4096 == 0 {
                progress.step(if k == 0 { 0 } else { 4096 })?;
            }
            let row = first + (k as u128 * rows as u128 / take as u128) as u64;
            let Some(v) = self.cell(row, col) else {
                continue;
            };
            if v.trim().is_empty() {
                blanks += 1;
                continue;
            }
            filled += 1;
            if v.chars().count() > MAX_LEN {
                *shapes.entry("…long".into()).or_default() += 1;
                continue;
            }
            *shapes.entry(shape_of(&v)).or_default() += 1;
            *lens.entry(v.chars().count()).or_default() += 1;
            if is_iso_date(&v) {
                iso += 1;
            }
            if let Some(x) = crate::coltype::Reader::plain(read).number(&v) {
                numbers += 1;
                if x.fract() == 0.0 {
                    whole += 1;
                }
            }
            if !distinct_full {
                if distinct.len() >= MAX_DISTINCT {
                    distinct_full = true;
                } else {
                    distinct.insert(v);
                }
            }
        }
        if filled < MIN_FILLED {
            return Ok(out);
        }
        let f = filled as f64;
        let total = filled + blanks;
        if blanks == 0 {
            out.push(Suggestion {
                check: Check {
                    col: Some(col),
                    name: None,
                    kind: CheckKind::Required,
                },
                why: format!("none of the {total} values looked at is blank"),
                share: 1.0,
            });
        }
        // The dominant shape (ties to the smallest text, so the answer is the same every time).
        let top = shapes
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(s, c)| (s.clone(), *c));
        if let Some((shape, count)) = top {
            let share = count as f64 / f;
            if share >= MIN_SHARE && !shape.ends_with('…') && shape != "…long" {
                if shape == "9999-99-99" && iso as f64 / f >= MIN_SHARE {
                    out.push(Suggestion {
                        check: Check {
                            col: Some(col),
                            name: Some("Date written YYYY-MM-DD".into()),
                            kind: CheckKind::DateIso,
                        },
                        why: format!(
                            "{iso} of {filled} filled values are real dates written year-month-day"
                        ),
                        share: iso as f64 / f,
                    });
                } else if shape.contains('9') || shape.contains('A') {
                    let only_digits = shape.chars().all(|c| c == '9');
                    out.push(Suggestion {
                        check: pattern_rule(col, &shape, format!("Matches {shape}")),
                        why: format!("{count} of {filled} filled values are written {shape}"),
                        share,
                    });
                    if only_digits {
                        let len = shape.chars().count();
                        out.push(Suggestion {
                            check: Check {
                                col: Some(col),
                                name: None,
                                kind: CheckKind::Length {
                                    min: Some(len as u32),
                                    max: Some(len as u32),
                                },
                            },
                            why: format!("{count} of {filled} filled values are {len} digits long"),
                            share,
                        });
                    }
                }
            }
        }
        if numbers as f64 / f >= MIN_SHARE
            && !out
                .iter()
                .any(|s| matches!(s.check.kind, CheckKind::DateIso))
        {
            let all_whole = whole as f64 / f >= MIN_SHARE;
            out.push(Suggestion {
                check: Check {
                    col: Some(col),
                    name: None,
                    kind: CheckKind::Number {
                        min: None,
                        max: None,
                        whole: all_whole,
                    },
                },
                why: if all_whole {
                    format!("{numbers} of {filled} filled values are whole numbers")
                } else {
                    format!("{numbers} of {filled} filled values are numbers")
                },
                share: numbers as f64 / f,
            });
        }
        if !distinct_full && distinct.len() == filled && filled >= 20 {
            out.push(Suggestion {
                check: Check {
                    col: Some(col),
                    name: None,
                    kind: CheckKind::Unique { match_case: false },
                },
                why: format!("all {filled} filled values looked at are different"),
                share: 1.0,
            });
        }
        out.retain(|s| s.share >= MIN_SHARE);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::CheckSpec;
    use crate::OpenOptions;

    fn doc(csv: String) -> Document {
        Document::from_bytes(csv.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn kinds(s: &[Suggestion]) -> Vec<&'static str> {
        s.iter()
            .map(|s| match s.check.kind {
                CheckKind::Required => "required",
                CheckKind::Number { .. } => "number",
                CheckKind::DateIso => "dateiso",
                CheckKind::Length { .. } => "length",
                CheckKind::Pattern { .. } => "pattern",
                CheckKind::Unique { .. } => "unique",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn suggests_the_intended_rules_for_made_up_columns() {
        let mut csv = String::from("zip,id,when,name,qty\r\n");
        for i in 0..300 {
            let zip = format!("{:05}", 10000 + i);
            let when = format!("2024-{:02}-{:02}", 1 + i % 12, 1 + i % 28);
            let name = ["Ann", "Bo", "Cy", "Di"][i % 4].repeat(1 + i % 3);
            csv.push_str(&format!("{zip},u{i},{when},{name},{}\r\n", i % 9));
        }
        let d = doc(csv);
        let p = Progress::new();
        let zip = d.suggest_rules(0, true, &p).unwrap();
        assert_eq!(
            kinds(&zip),
            vec!["required", "pattern", "length", "number", "unique"]
        );
        let id = d.suggest_rules(1, true, &p).unwrap();
        assert_eq!(kinds(&id), vec!["required", "unique"]);
        let when = d.suggest_rules(2, true, &p).unwrap();
        assert_eq!(kinds(&when)[..2], ["required", "dateiso"]);
        let name = d.suggest_rules(3, true, &p).unwrap();
        assert_eq!(
            kinds(&name),
            vec!["required"],
            "free text gets only the blank test"
        );
        let qty = d.suggest_rules(4, true, &p).unwrap();
        assert!(kinds(&qty).contains(&"number"));
        assert!(!kinds(&qty).contains(&"unique"));
    }

    #[test]
    fn a_suggestion_never_fails_more_than_one_per_cent_of_the_sample() {
        let mut x = 7u64;
        let mut r = move || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            x >> 33
        };
        for case in 0..20 {
            let mut csv = String::from("a,b\r\n");
            for i in 0..400 {
                let a = if r() % (3 + case) == 0 {
                    format!("x{}", r() % 99)
                } else {
                    format!("{:04}", r() % 10_000)
                };
                let b = if r() % 7 == 0 {
                    String::new()
                } else {
                    format!("{}", i)
                };
                csv.push_str(&format!("{a},{b}\r\n"));
            }
            let d = doc(csv);
            let p = Progress::new();
            for col in 0..2 {
                for s in d.suggest_rules(col, true, &p).unwrap() {
                    let spec = CheckSpec {
                        checks: vec![s.check.clone()],
                        keep_first: true,
                        ..Default::default()
                    };
                    let rep = d.run_checks(&spec, &p).unwrap();
                    let filled = (0..d.row_count())
                        .skip(1)
                        .filter(|&row| d.cell(row, col).is_some_and(|v| !v.trim().is_empty()))
                        .count();
                    // Unique counts both of a repeated pair, so it is allowed to differ by the repeats only.
                    if !matches!(s.check.kind, CheckKind::Unique { .. }) {
                        assert!(
                            rep.problems as f64 <= 0.01 * filled as f64 + 0.5,
                            "case {case} col {col} {:?}: {} problems of {filled}",
                            s.check.kind,
                            rep.problems
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn suggesting_can_be_cancelled_and_handles_tiny_and_empty_tables() {
        let d = doc("a\r\n".into());
        assert!(d
            .suggest_rules(0, true, &Progress::new())
            .unwrap()
            .is_empty());
        let d = doc("a\r\n1\r\n2\r\n".into());
        assert!(d
            .suggest_rules(0, true, &Progress::new())
            .unwrap()
            .is_empty());
        assert!(d.suggest_rules(5, true, &Progress::new()).is_err());
        let mut csv = String::from("a\r\n");
        for i in 0..50_000 {
            csv.push_str(&format!("{i}\r\n"));
        }
        let d = doc(csv);
        let p = Progress::new();
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            d.suggest_rules(0, true, &p),
            Err(Error::Cancelled)
        ));
    }
}
