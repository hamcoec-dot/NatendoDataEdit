//! Rules kept with a file: `voters.csv.ndchecks` beside the table (like `.ndnotes`).
//! Written only when the person asks, never automatically. The file holds only what the
//! person typed into the rules (names, patterns, lists, expressions), never cell values,
//! and it is read as untrusted: size, count, nesting, pattern and expression limits are
//! checked before anything is used, unknown fields are refused, and a rule that does not
//! fit the table is skipped and counted instead of failing the whole file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::validate::{check_spec, Check, CheckKind, CheckSpec};
use crate::Error;

/// The newest file version this release reads. Files are written as version 1 unless they
/// use something version 1 does not have (a unique rule over several columns), so files
/// without it stay byte-identical and readable by older releases.
pub const VERSION: u32 = 2;
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
}

/// The file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecksFile {
    pub version: u32,
    #[serde(default = "yes")]
    pub keep_first: bool,
    pub rules: Vec<StoredRule>,
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
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(Error::Invalid("the rules file is too large".into()));
        }
        let f: ChecksFile = serde_json::from_slice(bytes)
            .map_err(|e| Error::Invalid(format!("the rules file cannot be read: {e}")))?;
        if f.version > VERSION {
            return Err(Error::Invalid(
                "the rules file was saved by a newer version of the app".into(),
            ));
        }
        let spec = CheckSpec {
            checks: f.rules.iter().map(|r| r.check()).collect(),
            keep_first: f.keep_first,
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
            })
            .collect();
        let version = if rules.iter().any(|r| !r.also_titles.is_empty()) {
            2
        } else {
            1
        };
        ChecksFile {
            version,
            keep_first,
            rules,
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
        LoadedRules {
            checks,
            skipped,
            keep_first: self.keep_first,
        }
    }

    /// Write the file (a temp file moved into place); no rules removes it.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        if self.rules.is_empty() {
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
