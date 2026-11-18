//! Format › Tidy column titles: trim, collapse spaces, one case style,
//! remove odd characters and make repeated titles unique, all in one undo
//! step. Only the first row (the header row) changes.

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::Error;

/// Longest title kept.
pub const MAX_TITLE_CHARS: usize = 200;

/// One case style for every title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleCase {
    /// As written.
    #[default]
    Keep,
    /// `First Name`.
    Title,
    /// `first_name`: lower case, words joined by `_`.
    Snake,
    /// `FIRST NAME`.
    Upper,
    /// `first name`.
    Lower,
}

/// What to tidy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TitleSpec {
    /// Spaces at the ends removed, runs of spaces (and line breaks, tabs)
    /// made one space.
    pub spaces: bool,
    pub case: TitleCase,
    /// Only letters, digits, spaces, `_` and `-` kept.
    pub odd: bool,
    /// Repeated titles made unique (`Phone`, `Phone 2`); empty titles named
    /// `Column 3` (by place).
    pub unique: bool,
}

impl Default for TitleSpec {
    fn default() -> Self {
        TitleSpec {
            spaces: true,
            case: TitleCase::Keep,
            odd: false,
            unique: true,
        }
    }
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || c == '_' || c == '-')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn title_word(w: &str) -> String {
    let mut out = String::with_capacity(w.len());
    for (i, ch) in w.chars().enumerate() {
        if i == 0 {
            out.extend(ch.to_uppercase());
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

/// One title tidied (without making it unique).
pub fn tidy_title(s: &str, spec: &TitleSpec) -> String {
    let mut t: String = if spec.odd {
        s.chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '_' || *c == '-')
            .collect()
    } else {
        s.to_string()
    };
    if spec.spaces {
        t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    t = match spec.case {
        TitleCase::Keep => t,
        TitleCase::Upper => t.to_uppercase(),
        TitleCase::Lower => t.to_lowercase(),
        TitleCase::Title => {
            if spec.spaces {
                t.split(' ').map(title_word).collect::<Vec<_>>().join(" ")
            } else {
                // Keep the spacing as it is; capitalise each word.
                let mut out = String::with_capacity(t.len());
                let mut start = true;
                for ch in t.chars() {
                    if ch.is_whitespace() {
                        start = true;
                        out.push(ch);
                    } else if start {
                        out.extend(ch.to_uppercase());
                        start = false;
                    } else {
                        out.extend(ch.to_lowercase());
                    }
                }
                out
            }
        }
        TitleCase::Snake => words(&t)
            .iter()
            .map(|w| w.to_lowercase())
            .collect::<Vec<_>>()
            .join("_"),
    };
    t.chars().take(MAX_TITLE_CHARS).collect()
}

/// Every title tidied; with `unique`, repeats (ignoring case) get a number
/// and empty ones a name by place.
pub fn tidy_titles(titles: &[String], spec: &TitleSpec) -> Vec<String> {
    let mut out: Vec<String> = titles.iter().map(|t| tidy_title(t, spec)).collect();
    if !spec.unique {
        return out;
    }
    let sep = if spec.case == TitleCase::Snake {
        "_"
    } else {
        " "
    };
    for (i, t) in out.iter_mut().enumerate() {
        if t.is_empty() {
            *t = match spec.case {
                TitleCase::Snake | TitleCase::Lower => format!("column{sep}{}", i + 1),
                TitleCase::Upper => format!("COLUMN {}", i + 1),
                _ => format!("Column {}", i + 1),
            };
        }
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let all: std::collections::HashSet<String> = out.iter().map(|t| t.to_lowercase()).collect();
    // Per title, the number to try next (each repeat costs one step, even
    // with thousands of the same title).
    let mut next: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for t in out.iter_mut() {
        let key = t.to_lowercase();
        if seen.insert(key.clone()) {
            continue;
        }
        // The next number free among every title.
        let n = next.entry(key).or_insert(2);
        loop {
            let cand = format!("{t}{sep}{}", *n);
            let k = cand.to_lowercase();
            *n += 1;
            if !all.contains(&k) && !seen.contains(&k) {
                seen.insert(k);
                *t = cand;
                break;
            }
        }
    }
    out
}

impl Document {
    /// The titles that would change: (column, now, after).
    pub fn plan_tidy_titles(&self, spec: &TitleSpec) -> Result<Vec<(u32, String, String)>, Error> {
        let titles = self.first_row_values()?;
        let n = self.col_count() as usize;
        let mut titles: Vec<String> = titles.into_iter().take(n).collect();
        titles.resize(n, String::new());
        let after = tidy_titles(&titles, spec);
        Ok(titles
            .into_iter()
            .zip(after)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(c, (a, b))| (c as u32, a, b))
            .collect())
    }

    /// Tidy every column title as one undo step; returns how many changed.
    pub fn tidy_titles(&mut self, spec: &TitleSpec) -> Result<u64, Error> {
        let plan = self.plan_tidy_titles(spec)?;
        let n = plan.len() as u64;
        if n > 0 {
            self.set_first_row_cells(plan.into_iter().map(|(c, _, b)| (c, b)).collect())?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn styles() {
        let spec = |case| TitleSpec {
            case,
            ..Default::default()
        };
        assert_eq!(
            tidy_title("  First   NAME ", &spec(TitleCase::Keep)),
            "First NAME"
        );
        assert_eq!(
            tidy_title("first  name", &spec(TitleCase::Title)),
            "First Name"
        );
        assert_eq!(
            tidy_title("First Name-2 ", &spec(TitleCase::Snake)),
            "first_name_2"
        );
        assert_eq!(tidy_title("Zip code", &spec(TitleCase::Upper)), "ZIP CODE");
        let odd = TitleSpec {
            odd: true,
            ..Default::default()
        };
        assert_eq!(tidy_title("Amount ($)#", &odd), "Amount");
        assert_eq!(tidy_title("Résumé ✓", &odd), "Résumé");
        let keep = TitleSpec {
            spaces: false,
            ..Default::default()
        };
        assert_eq!(tidy_title(" a  b ", &keep), " a  b ");
    }

    #[test]
    fn many_repeats_stay_quick() {
        let titles = vec!["Phone".to_string(); 20_000];
        let t = std::time::Instant::now();
        let out = tidy_titles(&titles, &TitleSpec::default());
        assert!(t.elapsed().as_millis() < 1_000);
        assert_eq!(out[1], "Phone 2");
        assert_eq!(out[19_999], "Phone 20000");
        let set: std::collections::HashSet<&String> = out.iter().collect();
        assert_eq!(set.len(), 20_000);
    }

    #[test]
    fn unique_and_blank() {
        let t = tidy_titles(
            &s(&["Phone", "phone", "", "Phone 2", "Phone"]),
            &TitleSpec::default(),
        );
        assert_eq!(t, ["Phone", "phone 3", "Column 3", "Phone 2", "Phone 4"]);
        let snake = TitleSpec {
            case: TitleCase::Snake,
            ..Default::default()
        };
        assert_eq!(
            tidy_titles(&s(&["Phone", "PHONE", ""]), &snake),
            ["phone", "phone_2", "column_3"]
        );
        let off = TitleSpec {
            unique: false,
            ..Default::default()
        };
        assert_eq!(tidy_titles(&s(&["a", "a", ""]), &off), ["a", "a", ""]);
    }

    #[test]
    fn one_undo_step_and_protected_columns_still_rename() {
        let mut d = Document::from_bytes(
            b" a ,a,\n1,2,3\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        d.set_protected(&[0]);
        let plan = d.plan_tidy_titles(&TitleSpec::default()).unwrap();
        assert_eq!(plan.len(), 3);
        assert_eq!(d.tidy_titles(&TitleSpec::default()).unwrap(), 3);
        assert_eq!(d.row_values(0).unwrap(), ["a", "a 2", "Column 3"]);
        assert_eq!(d.row_values(1).unwrap(), ["1", "2", "3"]);
        d.undo();
        assert_eq!(d.row_values(0).unwrap(), [" a ", "a", ""]);
        // Nothing to tidy: nothing changes.
        assert_eq!(
            d.tidy_titles(&TitleSpec {
                unique: false,
                spaces: false,
                ..Default::default()
            })
            .unwrap(),
            0
        );
    }
}
