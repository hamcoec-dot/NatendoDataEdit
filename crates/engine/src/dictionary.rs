//! Value dictionaries (C6): a saved list of merges (variant -> canonical) as
//! a small JSON file (`.ndmap`), loaded into the Value editor and applied
//! through the usual "change values from a list" step. A file is checked
//! before it is used: UTF-8, at most 16 MB and 100,000 pairs, no variant
//! with two targets, no chains or cycles (A -> B and B -> C): the person
//! fixes those, the app never picks for them. A pair from a value to itself
//! says nothing and is left out (counted).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::editing::Tally;
use crate::ops::MAX_CELL_BYTES;
use crate::Error;

/// What a dictionary file says it is.
pub const DICTIONARY_KIND: &str = "natendo-dictionary";
pub const DICTIONARY_VERSION: u32 = 1;
/// Largest dictionary file read.
pub const MAX_DICTIONARY_BYTES: usize = 16 << 20;
/// Most pairs (the same as a translation list).
pub const MAX_DICTIONARY_PAIRS: usize = 100_000;

/// A saved list of merges.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dictionary {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub match_case: bool,
    #[serde(default)]
    pub trim: bool,
    /// Variant -> canonical.
    pub pairs: Vec<(String, String)>,
}

/// The file as written.
#[derive(Serialize, Deserialize)]
struct File {
    kind: String,
    version: u32,
    #[serde(flatten)]
    dict: Dictionary,
}

/// A dictionary read from a file, with the pairs left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReadDictionary {
    pub dict: Dictionary,
    /// Pairs from a value to itself (left out).
    pub self_pairs: u64,
    /// The same pair twice (kept once).
    pub repeats: u64,
}

fn bad(m: impl Into<String>) -> Error {
    Error::Invalid(m.into())
}

impl Dictionary {
    /// How a value is compared under this dictionary's settings.
    pub fn norm<'a>(&self, v: &'a str) -> std::borrow::Cow<'a, str> {
        let v = if self.trim { v.trim() } else { v };
        if self.match_case {
            std::borrow::Cow::Borrowed(v)
        } else {
            std::borrow::Cow::Owned(v.to_lowercase())
        }
    }

    /// Check the pairs: sizes, a variant with two targets, chains and
    /// cycles; pairs to themselves and repeats are left out (counted).
    pub fn checked(mut self) -> Result<ReadDictionary, Error> {
        if self.pairs.len() > MAX_DICTIONARY_PAIRS {
            return Err(bad(format!(
                "the dictionary has {} pairs; at most {MAX_DICTIONARY_PAIRS} can be used",
                self.pairs.len()
            )));
        }
        if self.name.len() > 200 {
            return Err(bad("the dictionary's name is too long"));
        }
        let mut out = ReadDictionary::default();
        let mut target: HashMap<String, (String, String)> = HashMap::new();
        let mut kept = Vec::with_capacity(self.pairs.len());
        for (from, to) in std::mem::take(&mut self.pairs) {
            if from.is_empty() || to.is_empty() {
                return Err(bad("the dictionary has an empty value"));
            }
            if from.len() > MAX_CELL_BYTES || to.len() > MAX_CELL_BYTES {
                return Err(bad("the dictionary has a value that is too long"));
            }
            let (f, t) = (self.norm(&from).into_owned(), self.norm(&to).into_owned());
            if f == t {
                out.self_pairs += 1;
                continue;
            }
            match target.get(&f) {
                Some((_, had)) if self.norm(had) == t => {
                    out.repeats += 1;
                    continue;
                }
                Some((_, had)) => {
                    return Err(bad(format!(
                        "the dictionary says \u{201c}{from}\u{201d} becomes both \u{201c}{had}\u{201d} and \u{201c}{to}\u{201d}"
                    )))
                }
                None => {}
            }
            target.insert(f, (from.clone(), to.clone()));
            kept.push((from, to));
        }
        // A target that is itself a variant: a chain (or, round again, a cycle).
        for (from, to) in &kept {
            if let Some((_, next)) = target.get(self.norm(to).as_ref()) {
                let cycle = self.norm(next) == self.norm(from);
                return Err(bad(if cycle {
                    format!("the dictionary goes round in a circle: \u{201c}{from}\u{201d} becomes \u{201c}{to}\u{201d} and \u{201c}{to}\u{201d} becomes \u{201c}{from}\u{201d}; fix the file first")
                } else {
                    format!("the dictionary has a chain: \u{201c}{from}\u{201d} becomes \u{201c}{to}\u{201d}, which becomes \u{201c}{next}\u{201d}; make \u{201c}{from}\u{201d} become \u{201c}{next}\u{201d} directly")
                }));
            }
        }
        self.pairs = kept;
        out.dict = self;
        Ok(out)
    }

    /// Read a dictionary file.
    pub fn parse(bytes: &[u8]) -> Result<ReadDictionary, Error> {
        if bytes.len() > MAX_DICTIONARY_BYTES {
            return Err(bad("the dictionary file is larger than 16 MB"));
        }
        let text =
            std::str::from_utf8(bytes).map_err(|_| bad("the dictionary file is not UTF-8 text"))?;
        let file: File = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| bad("this is not a dictionary file"))?;
        if file.kind != DICTIONARY_KIND {
            return Err(bad("this is not a dictionary file"));
        }
        if file.version != DICTIONARY_VERSION {
            return Err(bad(format!(
                "this dictionary was written by a newer version (version {}); this app reads version {DICTIONARY_VERSION}",
                file.version
            )));
        }
        file.dict.checked()
    }

    /// The file's text (checked first).
    pub fn to_json(&self) -> Result<String, Error> {
        let d = self.clone().checked()?.dict;
        serde_json::to_string_pretty(&File {
            kind: DICTIONARY_KIND.into(),
            version: DICTIONARY_VERSION,
            dict: d,
        })
        .map_err(|e| bad(e.to_string()))
    }

    /// Values of a column (with their counts) that are neither a variant nor
    /// a canonical value of the dictionary: "new since last time".
    pub fn unmatched(&self, values: &[Tally]) -> Vec<Tally> {
        let mut known = std::collections::HashSet::with_capacity(self.pairs.len() * 2);
        for (a, b) in &self.pairs {
            known.insert(self.norm(a).into_owned());
            known.insert(self.norm(b).into_owned());
        }
        values
            .iter()
            .filter(|v| !v.value.is_empty() && !known.contains(self.norm(&v.value).as_ref()))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(pairs: &[(&str, &str)]) -> Dictionary {
        Dictionary {
            name: "Counties".into(),
            match_case: false,
            trim: true,
            pairs: pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    #[test]
    fn round_trip() {
        let a = d(&[
            ("Hamilton Co.", "Hamilton"),
            ("Ham", "Hamilton"),
            ("Knox Co", "Knox"),
        ]);
        let text = a.to_json().unwrap();
        assert!(text.contains("\"kind\": \"natendo-dictionary\""));
        let b = Dictionary::parse(text.as_bytes()).unwrap();
        assert_eq!(b.dict, a);
        assert_eq!((b.self_pairs, b.repeats), (0, 0));
    }

    #[test]
    fn bad_files_are_refused() {
        let e = |b: &[u8]| Dictionary::parse(b).err().unwrap().to_string();
        assert!(e(&[0xff, 0xfe, 0x00]).contains("not UTF-8"));
        assert!(e(b"{\"a\":1}").contains("not a dictionary"));
        assert!(e(b"{\"kind\":\"other\",\"version\":1,\"pairs\":[]}").contains("not a dictionary"));
        assert!(
            e(b"{\"kind\":\"natendo-dictionary\",\"version\":9,\"pairs\":[]}")
                .contains("newer version")
        );
        let big = vec![b' '; MAX_DICTIONARY_BYTES + 1];
        assert!(e(&big).contains("16 MB"));
        let many = Dictionary {
            pairs: (0..=MAX_DICTIONARY_PAIRS)
                .map(|i| (format!("a{i}"), "b".into()))
                .collect(),
            ..d(&[])
        };
        assert!(many
            .checked()
            .err()
            .unwrap()
            .to_string()
            .contains("at most 100000"));
        assert!(d(&[("a", "")]).checked().is_err());
    }

    #[test]
    fn two_targets_chains_and_cycles_are_refused() {
        let msg = |x: Dictionary| x.checked().err().unwrap().to_string();
        assert!(msg(d(&[("NY", "New York"), ("ny ", "Newark")])).contains("becomes both"));
        assert!(msg(d(&[("A", "B"), ("B", "C")])).contains("chain"));
        assert!(msg(d(&[("A", "B"), ("b", "a")])).contains("circle"));
        // Self-pairs and repeats are left out and counted.
        let r = d(&[("A", "a"), ("X", "Y"), ("x", "y"), ("Q", "Q")])
            .checked()
            .unwrap();
        assert_eq!(r.dict.pairs, vec![("X".to_string(), "Y".to_string())]);
        assert_eq!((r.self_pairs, r.repeats), (2, 1));
        // With match case on, A and a differ.
        let mut c = d(&[("A", "a")]);
        c.match_case = true;
        assert_eq!(c.checked().unwrap().dict.pairs.len(), 1);
    }

    #[test]
    fn unmatched_lists_new_values() {
        let t = |v: &[(&str, u64)]| -> Vec<Tally> {
            v.iter()
                .map(|(s, c)| Tally {
                    value: s.to_string(),
                    count: *c,
                })
                .collect()
        };
        let dict = d(&[("Hamilton Co.", "Hamilton")]);
        let values = t(&[
            ("Hamilton", 9),
            ("hamilton co.", 3),
            ("Knox", 2),
            (" Hamilton ", 1),
            ("", 4),
        ]);
        assert_eq!(dict.unmatched(&values), t(&[("Knox", 2)]));
        let mut strict = dict.clone();
        strict.match_case = true;
        strict.trim = false;
        assert_eq!(
            strict.unmatched(&values),
            t(&[("hamilton co.", 3), ("Knox", 2), (" Hamilton ", 1)])
        );
    }
}
