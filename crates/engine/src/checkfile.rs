//! Rules kept with a file: `voters.csv.ndchecks` beside the table (like `.ndnotes`).
//! Written only when the person asks, never automatically. The file holds only what the
//! person typed into the rules (names, patterns, lists, expressions), never cell values,
//! and it is read as untrusted: size, count, nesting, pattern and expression limits are
//! checked before anything is used, unknown fields are refused, and a rule that does not
//! fit the table is skipped and counted instead of failing the whole file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::tablecheck::{TableCheck, TableCheckKind};
use crate::validate::{check_spec, Check, CheckKind, CheckSpec};
use crate::Error;

/// The newest file version this release reads. Files are written as version 1 unless they
/// use something version 1 does not have (version 2: a unique rule over several columns, table
/// checks; version 3: a "settled by" rule), so files without it stay byte-identical and
/// readable by older releases.
pub const VERSION: u32 = 3;
/// Largest rules file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 4 << 20;

/// One rule as stored: the column by title and by number (the title wins when both fit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredRule {
    #[serde(default)]
    pub col: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: CheckKind,
    /// A unique rule's `also` columns by title, in the same order as their numbers in
    /// `kind` (found the way `title` is; a key column that is gone skips the rule).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_titles: Vec<String>,
    /// A "settled by" rule's `by` column by title (found the way `title` is; when it is
    /// gone the rule is skipped). Written in version 3 files only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_title: Option<String>,
}

/// One table check as stored: a column it names by title too (a formula names its
/// columns by title already).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredTableCheck {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: TableCheckKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecksFile {
    pub version: u32,
    #[serde(default = "yes")]
    pub keep_first: bool,
    pub rules: Vec<StoredRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_checks: Vec<StoredTableCheck>,
}

fn yes() -> bool {
    true
}

/// Rules that fit the table, and how many did not.
#[derive(Debug, Clone, Serialize)]
pub struct LoadedRules {
    pub checks: Vec<Check>,
    pub skipped: usize,
    pub keep_first: bool,
    pub table_checks: Vec<TableCheck>,
}

fn table_col(k: &mut TableCheckKind) -> Option<&mut u32> {
    match k {
        TableCheckKind::NoBlanks { col } | TableCheckKind::Distinct { col, .. } => Some(col),
        _ => None,
    }
}

/// The rules file beside a table.
pub fn checks_path(table: &Path) -> PathBuf {
    let mut name = table
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".ndchecks");
    table.with_file_name(name)
}

impl ChecksFile {
    /// Read and validate a rules file's bytes (before anything is used).
    pub fn from_bytes(bytes: &[u8]) -> Result<ChecksFile, Error> {
        Self::read(bytes, VERSION)
    }

    /// [`ChecksFile::from_bytes`] as a release that reads up to version `newest` would.
    fn read(bytes: &[u8], newest: u32) -> Result<ChecksFile, Error> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(Error::Invalid("the rules file is too large".into()));
        }
        // The version first, so a newer file is turned away by its version, not by a
        // rule kind or field this build does not know.
        #[derive(Deserialize)]
        struct Version {
            version: u32,
        }
        let v: Version = serde_json::from_slice(bytes)
            .map_err(|e| Error::Invalid(format!("the rules file cannot be read: {e}")))?;
        if v.version > newest {
            return Err(Error::Invalid(
                "the rules file was saved by a newer version of the app".into(),
            ));
        }
        let f: ChecksFile = serde_json::from_slice(bytes)
            .map_err(|e| Error::Invalid(format!("the rules file cannot be read: {e}")))?;
        let spec = CheckSpec {
            checks: f.rules.iter().map(|r| r.check()).collect(),
            keep_first: f.keep_first,
            table_checks: f
                .table_checks
                .iter()
                .map(|c| TableCheck {
                    name: c.name.clone(),
                    kind: c.kind.clone(),
                })
                .collect(),
            ..Default::default()
        };
        check_spec(&spec)?;
        for r in &f.rules {
            let also = match &r.kind {
                CheckKind::Unique { also, .. } => also.len(),
                _ => 0,
            };
            if !r.also_titles.is_empty() && r.also_titles.len() != also {
                return Err(Error::Invalid(
                    "the rules file names other key columns that do not match their numbers".into(),
                ));
            }
            if r.by_title.is_some() && !matches!(r.kind, CheckKind::SettledBy { .. }) {
                return Err(Error::Invalid(
                    "the rules file names a settling column for a rule that has none".into(),
                ));
            }
        }
        Ok(f)
    }

    /// Read a rules file; `None` when there is none.
    pub fn load(path: &Path) -> Result<Option<ChecksFile>, Error> {
        let Ok(meta) = std::fs::metadata(path) else {
            return Ok(None);
        };
        if meta.len() > MAX_FILE_BYTES {
            return Err(Error::Invalid("the rules file is too large".into()));
        }
        Self::from_bytes(&std::fs::read(path)?).map(Some)
    }

    /// The rules to store for `checks` on `doc` (titles from the table, so a reordered
    /// or newer copy of the file finds its columns again).
    pub fn for_rules(doc: &Document, checks: &[Check], keep_first: bool) -> ChecksFile {
        Self::for_checks(doc, checks, &[], keep_first)
    }

    /// [`ChecksFile::for_rules`] with table checks too.
    pub fn for_checks(
        doc: &Document,
        checks: &[Check],
        table_checks: &[TableCheck],
        keep_first: bool,
    ) -> ChecksFile {
        let titles = doc.rule_titles(keep_first);
        let rules: Vec<StoredRule> = checks
            .iter()
            .map(|c| StoredRule {
                col: c.col,
                title: c.col.and_then(|k| titles.get(k as usize).cloned()),
                name: c.name.clone(),
                kind: c.kind.clone(),
                also_titles: match &c.kind {
                    CheckKind::Unique { also, .. } => also
                        .iter()
                        .map(|&k| {
                            titles
                                .get(k as usize)
                                .cloned()
                                .unwrap_or_else(|| format!("Column {}", k + 1))
                        })
                        .collect(),
                    _ => Vec::new(),
                },
                by_title: match &c.kind {
                    CheckKind::SettledBy { by, .. } => Some(
                        titles
                            .get(*by as usize)
                            .cloned()
                            .unwrap_or_else(|| format!("Column {}", by + 1)),
                    ),
                    _ => None,
                },
            })
            .collect();
        let table_checks: Vec<StoredTableCheck> = table_checks
            .iter()
            .map(|c| {
                let mut kind = c.kind.clone();
                StoredTableCheck {
                    name: c.name.clone(),
                    title: table_col(&mut kind).and_then(|k| titles.get(*k as usize).cloned()),
                    kind,
                }
            })
            .collect();
        // The lowest version that holds what is written, so older builds keep reading
        // the files they can.
        let version = if rules
            .iter()
            .any(|r| matches!(r.kind, CheckKind::SettledBy { .. }))
        {
            3
        } else if !table_checks.is_empty() || rules.iter().any(|r| !r.also_titles.is_empty()) {
            2
        } else {
            1
        };
        ChecksFile {
            version,
            keep_first,
            rules,
            table_checks,
        }
    }

    /// The rules that fit `doc`: a rule's column is found by title (else by number when
    /// it has no title), and a rule whose column is gone, or whose pattern or expression
    /// does not compile against this table, is skipped and counted.
    pub fn resolve(&self, doc: &Document) -> LoadedRules {
        let titles = doc.rule_titles(self.keep_first);
        let mut checks = Vec::new();
        let mut skipped = 0;
        for r in &self.rules {
            let col = find(&titles, r.title.as_ref(), r.col);
            let mut c = r.check();
            if let CheckKind::Unique { also, .. } = &mut c.kind {
                if !r.also_titles.is_empty() {
                    let found: Option<Vec<u32>> = also
                        .iter()
                        .zip(&r.also_titles)
                        .map(|(&k, t)| find(&titles, Some(t), Some(k)))
                        .collect();
                    match found {
                        Some(v) => *also = v,
                        None => {
                            skipped += 1;
                            continue;
                        }
                    }
                }
            }
            if let CheckKind::SettledBy { by, .. } = &mut c.kind {
                match find(&titles, r.by_title.as_ref(), Some(*by)) {
                    Some(k) => *by = k,
                    None => {
                        skipped += 1;
                        continue;
                    }
                }
            }
            if !matches!(c.kind, CheckKind::Expression { .. }) {
                if col.is_none() {
                    skipped += 1;
                    continue;
                }
                c.col = col;
            }
            let one = CheckSpec {
                checks: vec![c.clone()],
                keep_first: self.keep_first,
                ..Default::default()
            };
            if doc.rules_fit(&one).is_err() {
                skipped += 1;
                continue;
            }
            checks.push(c);
        }
        let names = doc.expr_names(self.keep_first);
        let mut table_checks = Vec::new();
        for s in &self.table_checks {
            let mut kind = s.kind.clone();
            let fits = match table_col(&mut kind) {
                Some(col) => match find(&titles, s.title.as_ref(), Some(*col)) {
                    Some(k) if k < doc.col_count() => {
                        *col = k;
                        true
                    }
                    _ => false,
                },
                None => match &kind {
                    TableCheckKind::Aggregate { expr, .. } => {
                        crate::expr::Expr::parse_table(expr, &names).is_ok()
                    }
                    _ => true,
                },
            };
            if fits {
                table_checks.push(TableCheck {
                    name: s.name.clone(),
                    kind,
                });
            } else {
                skipped += 1;
            }
        }
        LoadedRules {
            checks,
            skipped,
            keep_first: self.keep_first,
            table_checks,
        }
    }

    /// Write the file (a temp file moved into place); no rules removes it.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        if self.rules.is_empty() && self.table_checks.is_empty() {
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => return Ok(()),
            }
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?;
        // What is written must read back: the same limits as loading.
        Self::from_bytes(&bytes)?;
        let (temp, mut file) = crate::save::create_temp(path)?;
        let done = (|| -> std::io::Result<()> {
            use std::io::Write;
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        drop(file);
        if let Err(e) = done.and_then(|_| crate::save::replace(&temp, path)) {
            let _ = std::fs::remove_file(&temp);
            return Err(e.into());
        }
        Ok(())
    }
}

/// A stored column: same place when its title still sits there, else the first column
/// with that title; by number when it has no title.
fn find(titles: &[String], title: Option<&String>, col: Option<u32>) -> Option<u32> {
    match (title, col) {
        (Some(t), c) => match c.filter(|&k| titles.get(k as usize) == Some(t)) {
            Some(k) => Some(k),
            None => titles.iter().position(|x| x == t).map(|k| k as u32),
        },
        (None, c) => c,
    }
}

impl StoredRule {
    fn check(&self) -> Check {
        Check {
            col: self.col,
            kind: self.kind.clone(),
            name: self.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::{Pattern, PatternKind};
    use crate::validate::MAX_CHECKS;
    use crate::OpenOptions;

    fn doc(csv: &str) -> Document {
        Document::from_bytes(csv.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn rules() -> Vec<Check> {
        vec![
            Check {
                name: Some("5-digit ZIP".into()),
                ..Check::new(
                    Some(1),
                    CheckKind::Pattern {
                        pattern: Pattern {
                            text: "[0-9]{5}".into(),
                            kind: PatternKind::Regex,
                            ..Default::default()
                        },
                    },
                )
            },
            Check::new(Some(0), CheckKind::Required),
            Check::new(
                None,
                CheckKind::Expression {
                    expr: "LEN([name]) > 1".into(),
                },
            ),
        ]
    }

    fn settled() -> Check {
        Check::new(
            Some(1),
            CheckKind::SettledBy {
                by: 0,
                match_case: true,
            },
        )
    }

    #[test]
    fn a_settled_by_rule_is_written_as_version_3_and_only_then() {
        let d = doc("precinct,ward\r\nP1,W1\r\n");
        // Without it: the same bytes as before (version 1, no new field).
        let plain = ChecksFile::for_rules(&d, &rules(), true);
        assert_eq!(plain.version, 1);
        let text = String::from_utf8(serde_json::to_vec_pretty(&plain).unwrap()).unwrap();
        assert!(
            !text.contains("by_title") && !text.contains("settled_by"),
            "{text}"
        );
        let unique2 = vec![Check::new(
            Some(0),
            CheckKind::Unique {
                match_case: false,
                also: vec![1],
            },
        )];
        assert_eq!(ChecksFile::for_rules(&d, &unique2, true).version, 2);
        // With it: version 3, and the settling column by title.
        let f = ChecksFile::for_rules(&d, &[settled()], true);
        assert_eq!(f.version, 3);
        assert_eq!(f.rules[0].by_title.as_deref(), Some("precinct"));
        let bytes = serde_json::to_vec_pretty(&f).unwrap();
        assert_eq!(ChecksFile::from_bytes(&bytes).unwrap(), f);
        // A release that reads up to version 2 turns it away by its version.
        let e = ChecksFile::read(&bytes, 2).unwrap_err().to_string();
        assert!(e.contains("saved by a newer version"), "{e}");
        // And this one turns away a version 4 file the same way, whatever it holds.
        let newer = br#"{"version":4,"rules":[{"col":0,"kind":{"type":"some_new_kind"}}]}"#;
        let e = ChecksFile::from_bytes(newer).unwrap_err().to_string();
        assert!(e.contains("saved by a newer version"), "{e}");
    }

    #[test]
    fn old_files_read_as_before() {
        let v1 = br#"{"version":1,"keep_first":true,"rules":[{"col":0,"title":"precinct","kind":{"type":"required"}}]}"#;
        let v2 = br#"{"version":2,"keep_first":true,"rules":[{"col":0,"title":"precinct","kind":{"type":"unique","match_case":false,"also":[1]},"also_titles":["ward"]}]}"#;
        let d = doc("precinct,ward\r\nP1,W1\r\n");
        for b in [&v1[..], &v2[..]] {
            let f = ChecksFile::from_bytes(b).unwrap();
            assert_eq!(f.resolve(&d).checks.len(), 1);
            assert!(f.rules[0].by_title.is_none());
        }
        // A settling column named on a rule without one is refused.
        let odd =
            br#"{"version":3,"rules":[{"col":0,"kind":{"type":"required"},"by_title":"ward"}]}"#;
        assert!(ChecksFile::from_bytes(odd).is_err());
    }

    #[test]
    fn a_settled_by_rule_follows_its_columns_by_title() {
        let d = doc("precinct,ward\r\nP1,W1\r\n");
        let f = ChecksFile::for_rules(&d, &[settled()], true);
        // The columns swapped, and a new one in front.
        let moved = doc("note,ward,precinct\r\nx,W1,P1\r\n");
        let got = f.resolve(&moved);
        assert_eq!(got.skipped, 0);
        assert_eq!(got.checks[0].col, Some(1));
        assert_eq!(
            got.checks[0].kind,
            CheckKind::SettledBy {
                by: 2,
                match_case: true
            }
        );
        // The settling column gone: skipped.
        let gone = doc("ward,note\r\nW1,x\r\n");
        let got = f.resolve(&gone);
        assert_eq!((got.checks.len(), got.skipped), (0, 1));
    }

    #[test]
    fn rules_round_trip_and_follow_their_columns_by_title() {
        let d = doc("name,zip\r\nAnn,12345\r\n");
        let f = ChecksFile::for_rules(&d, &rules(), true);
        let dir = std::env::temp_dir().join(format!("nde-checks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = checks_path(&dir.join("voters.csv"));
        assert!(path.to_string_lossy().ends_with("voters.csv.ndchecks"));
        f.save(&path).unwrap();
        let back = ChecksFile::load(&path).unwrap().unwrap();
        assert_eq!(back, f);
        let got = back.resolve(&d);
        assert_eq!((got.checks.len(), got.skipped), (3, 0));
        assert_eq!(got.checks[0].name.as_deref(), Some("5-digit ZIP"));
        // A newer copy of the file with the columns turned round: the rules follow their titles.
        let swapped = doc("zip,name\r\n12345,Ann\r\n");
        let got = back.resolve(&swapped);
        assert_eq!(got.skipped, 0);
        assert_eq!(got.checks[0].col, Some(0));
        assert_eq!(got.checks[1].col, Some(1));
        // A column that is gone: its rule is skipped and counted, the others stay.
        let lost = doc("name,city\r\nAnn,X\r\n");
        let got = back.resolve(&lost);
        assert_eq!(got.skipped, 1, "the zip rule has no column left");
        assert_eq!(got.checks.len(), 2);
        // The file stores rules, not cell values.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("Ann") && !text.contains("12345"));
        // No file is no rules; saving no rules removes the file.
        ChecksFile {
            rules: vec![],
            ..back
        }
        .save(&path)
        .unwrap();
        assert!(ChecksFile::load(&path).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn file_with(rules_json: &str) -> Vec<u8> {
        format!(r#"{{"version":1,"rules":{rules_json}}}"#).into_bytes()
    }

    #[test]
    fn a_malicious_or_broken_rules_file_is_refused_and_changes_nothing() {
        for bad in [
            &b"not json"[..],
            b"[]",
            b"{}",
            br#"{"version":1,"rules":[{"kind":{"type":"required"},"surprise":1}]}"#,
            br#"{"version":1,"rules":[],"extra":true}"#,
            br#"{"version":9,"rules":[]}"#,
            br#"{"version":1,"rules":[{"kind":{"type":"nope"}}]}"#,
        ] {
            assert!(
                ChecksFile::from_bytes(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        // too many rules
        let many = (0..=MAX_CHECKS)
            .map(|_| r#"{"col":0,"kind":{"type":"required"}}"#)
            .collect::<Vec<_>>()
            .join(",");
        assert!(ChecksFile::from_bytes(&file_with(&format!("[{many}]"))).is_err());
        // a huge list
        let list = (0..100_001)
            .map(|i| format!("\"v{i}\""))
            .collect::<Vec<_>>()
            .join(",");
        let huge = file_with(&format!(
            r#"[{{"col":0,"kind":{{"type":"list","values":[{list}]}}}}]"#
        ));
        assert!(ChecksFile::from_bytes(&huge).is_err());
        // a file over 4 MiB
        let big = vec![b' '; (MAX_FILE_BYTES + 1) as usize];
        assert!(ChecksFile::from_bytes(&big).is_err());
        // deep nesting
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(ChecksFile::from_bytes(deep.as_bytes()).is_err());
        // a name that is too long
        let long = "x".repeat(101);
        assert!(ChecksFile::from_bytes(&file_with(&format!(
            r#"[{{"col":0,"name":"{long}","kind":{{"type":"required"}}}}]"#
        )))
        .is_err());
        // an expression that is too long
        let e = "1+".repeat(6_000);
        assert!(ChecksFile::from_bytes(&file_with(&format!(
            r#"[{{"kind":{{"type":"expression","expr":"{e}1"}}}}]"#
        )))
        .is_err());
    }

    #[test]
    fn evil_patterns_and_expressions_are_skipped_not_run() {
        let d = doc("a,b\r\n1,2\r\n");
        // A regex too big to compile, an expression nested too deep, a reference to a column that is not there.
        let huge_regex = "(a{1000}){1000}";
        let deep = format!("{}1{}", "(".repeat(500), ")".repeat(500));
        let f = ChecksFile::from_bytes(&file_with(&format!(
            r#"[
              {{"col":0,"kind":{{"type":"pattern","pattern":{{"text":"{huge_regex}","kind":"regex"}}}}}},
              {{"kind":{{"type":"expression","expr":"{deep}"}}}},
              {{"kind":{{"type":"expression","expr":"[nope] > 1"}}}},
              {{"col":0,"kind":{{"type":"required"}}}}
            ]"#
        )))
        .unwrap();
        let got = f.resolve(&d);
        assert_eq!(got.checks.len(), 1);
        assert_eq!(got.skipped, 3);
    }

    #[test]
    fn a_unique_rule_over_several_columns_follows_every_key_column_by_title() {
        let d = doc("first,last,city\r\nAnn,Lee,Rome\r\n");
        let rules = vec![Check::new(
            Some(0),
            CheckKind::Unique {
                match_case: false,
                also: vec![2],
            },
        )];
        let f = ChecksFile::for_rules(&d, &rules, true);
        assert_eq!(f.version, 2, "a key over several columns needs version 2");
        assert_eq!(f.rules[0].also_titles, vec!["city".to_string()]);
        let bytes = serde_json::to_vec(&f).unwrap();
        let back = ChecksFile::from_bytes(&bytes).unwrap();
        // Columns turned round: both key columns are found by title.
        let moved = doc("city,first,last\r\nRome,Ann,Lee\r\n");
        let got = back.resolve(&moved);
        assert_eq!(got.skipped, 0);
        assert_eq!(got.checks[0].col, Some(1));
        assert_eq!(
            got.checks[0].kind,
            CheckKind::Unique {
                match_case: false,
                also: vec![0]
            }
        );
        // A key column that is gone skips the whole rule.
        let lost = doc("first,last\r\nAnn,Lee\r\n");
        let got = back.resolve(&lost);
        assert_eq!((got.checks.len(), got.skipped), (0, 1));
        // Titles that do not match the numbers are refused.
        assert!(ChecksFile::from_bytes(&file_with(
            r#"[{"col":0,"kind":{"type":"unique","also":[1]},"also_titles":["a","b"]}]"#
        ))
        .is_err());
        // Without a key over several columns the file is still version 1, byte for byte.
        let one = ChecksFile::for_rules(&d, &rules_one(), true);
        assert_eq!(one.version, 1);
        let text = String::from_utf8(serde_json::to_vec_pretty(&one).unwrap()).unwrap();
        assert!(!text.contains("also"), "{text}");
        let old = br#"{"version":1,"keep_first":true,"rules":[{"col":0,"title":"first","kind":{"type":"unique","match_case":false}}]}"#;
        let f = ChecksFile::from_bytes(old).unwrap();
        assert_eq!(serde_json::to_vec(&f).unwrap(), old.to_vec());
    }

    #[test]
    fn table_checks_are_kept_by_title_and_skipped_when_they_do_not_fit() {
        use crate::tablecheck::{Compare, Tolerance};
        let d = doc("id,votes,party\r\n1,5,R\r\n2,7,D\r\n");
        let tc = vec![
            TableCheck {
                name: Some("Two rows".into()),
                kind: TableCheckKind::RowCount {
                    compare: Compare::Equals,
                    value: 2,
                },
            },
            TableCheck {
                name: None,
                kind: TableCheckKind::NoBlanks { col: 2 },
            },
            TableCheck {
                name: None,
                kind: TableCheckKind::Aggregate {
                    expr: "SUM([votes]) = 12".into(),
                    tolerance: Tolerance::Relative { value: 0.01 },
                },
            },
        ];
        let f = ChecksFile::for_checks(&d, &[], &tc, true);
        assert_eq!(f.version, 2);
        assert_eq!(f.table_checks[1].title.as_deref(), Some("party"));
        let bytes = serde_json::to_vec_pretty(&f).unwrap();
        let back = ChecksFile::from_bytes(&bytes).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.resolve(&d).table_checks, tc);
        // Columns moved: found by title. A column or a formula's column gone: skipped and counted.
        let moved = doc("party,id,votes\r\nR,1,5\r\n");
        let r = back.resolve(&moved);
        assert_eq!(r.table_checks[1].kind, TableCheckKind::NoBlanks { col: 0 });
        assert_eq!((r.table_checks.len(), r.skipped), (3, 0));
        let gone = doc("id,count\r\n1,5\r\n");
        let r = back.resolve(&gone);
        assert_eq!((r.table_checks.len(), r.skipped), (1, 2));
        // Without table checks the file stays version 1 and has no table_checks field.
        let plain = ChecksFile::for_rules(&d, &rules_one(), true);
        assert_eq!(plain.version, 1);
        assert!(!String::from_utf8(serde_json::to_vec(&plain).unwrap())
            .unwrap()
            .contains("table_checks"));
        // Bad table checks refuse the file: a negative tolerance, too many checks, an unknown field.
        let bad = |s: &str| {
            ChecksFile::from_bytes(
                format!(r#"{{"version":2,"rules":[],"table_checks":[{s}]}}"#).as_bytes(),
            )
        };
        assert!(bad(
            r#"{"kind":{"type":"aggregate","expr":"1","tolerance":{"kind":"absolute","value":-1}}}"#
        )
        .is_err());
        assert!(bad(r#"{"kind":{"type":"no_blanks","col":0},"extra":1}"#).is_err());
        assert!(bad(
            r#"{"kind":{"type":"aggregate","expr":"1","tolerance":{"kind":"sideways","value":1}}}"#
        )
        .is_err());
        let many = vec![r#"{"kind":{"type":"no_blanks","col":0}}"#; 51].join(",");
        assert!(bad(&many).is_err());
        assert!(bad(r#"{"kind":{"type":"no_blanks","col":0}}"#).is_ok());
    }

    fn rules_one() -> Vec<Check> {
        vec![Check::new(
            Some(0),
            CheckKind::Unique {
                match_case: false,
                also: vec![],
            },
        )]
    }
}
