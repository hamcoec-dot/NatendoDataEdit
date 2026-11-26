//! Abbreviations and full names (C6, the Value editor's "Abbreviations"
//! look-alikes): street words (St = Street, last word only), US states and
//! territories (NY = New York, whole cell only), a few country names and
//! codes (USA = United States, whole cell only), and the person's own
//! dictionary pairs. The tables are this app's own data.
//!
//! Every value gets a key: the `look_alike_key` of its full form. Values with
//! one key form a group, a suggestion only. A group joined only through a
//! two-letter state code or a country code is "maybe" when under half of the
//! column's rows are states (or countries): "OK" in a status column is not
//! Oklahoma, so such groups are listed apart and never put in the list all at
//! once.

use std::collections::HashMap;

use crate::document::Progress;
use crate::editing::{look_alike_key, Tally};
use crate::Error;

/// Street words: (short forms, full form). Only the last word of a value of
/// two words or more ("Main St"); "St Louis" and a lone "Dr" are left alone.
/// Ste and Apt are units, not street words.
const STREET: &[(&[&str], &str)] = &[
    (&["st", "str"], "Street"),
    (&["ave", "av"], "Avenue"),
    (&["rd"], "Road"),
    (&["blvd"], "Boulevard"),
    (&["dr"], "Drive"),
    (&["ln"], "Lane"),
    (&["ct"], "Court"),
    (&["pl"], "Place"),
    (&["sq"], "Square"),
    (&["hwy"], "Highway"),
    (&["pkwy"], "Parkway"),
    (&["ter", "terr"], "Terrace"),
    (&["cir"], "Circle"),
];

/// US states, DC and the inhabited territories: (code, name).
const STATES: &[(&str, &str)] = &[
    ("AL", "Alabama"),
    ("AK", "Alaska"),
    ("AZ", "Arizona"),
    ("AR", "Arkansas"),
    ("CA", "California"),
    ("CO", "Colorado"),
    ("CT", "Connecticut"),
    ("DE", "Delaware"),
    ("FL", "Florida"),
    ("GA", "Georgia"),
    ("HI", "Hawaii"),
    ("ID", "Idaho"),
    ("IL", "Illinois"),
    ("IN", "Indiana"),
    ("IA", "Iowa"),
    ("KS", "Kansas"),
    ("KY", "Kentucky"),
    ("LA", "Louisiana"),
    ("ME", "Maine"),
    ("MD", "Maryland"),
    ("MA", "Massachusetts"),
    ("MI", "Michigan"),
    ("MN", "Minnesota"),
    ("MS", "Mississippi"),
    ("MO", "Missouri"),
    ("MT", "Montana"),
    ("NE", "Nebraska"),
    ("NV", "Nevada"),
    ("NH", "New Hampshire"),
    ("NJ", "New Jersey"),
    ("NM", "New Mexico"),
    ("NY", "New York"),
    ("NC", "North Carolina"),
    ("ND", "North Dakota"),
    ("OH", "Ohio"),
    ("OK", "Oklahoma"),
    ("OR", "Oregon"),
    ("PA", "Pennsylvania"),
    ("RI", "Rhode Island"),
    ("SC", "South Carolina"),
    ("SD", "South Dakota"),
    ("TN", "Tennessee"),
    ("TX", "Texas"),
    ("UT", "Utah"),
    ("VT", "Vermont"),
    ("VA", "Virginia"),
    ("WA", "Washington"),
    ("WV", "West Virginia"),
    ("WI", "Wisconsin"),
    ("WY", "Wyoming"),
    ("DC", "District of Columbia"),
    ("PR", "Puerto Rico"),
    ("GU", "Guam"),
    ("VI", "U.S. Virgin Islands"),
    ("AS", "American Samoa"),
    ("MP", "Northern Mariana Islands"),
];

/// Countries: (codes and other names, full name). Two-letter codes that are
/// also US state codes (CA, DE, IN...) are left out, and so are codes that
/// are everyday words or names (can, per, ken).
const COUNTRIES: &[(&[&str], &str)] = &[
    (&["us", "usa", "unitedstatesofamerica", "america"], "United States"),
    (&["uk", "gb", "gbr", "greatbritain", "britain"], "United Kingdom"),
    (&["uae"], "United Arab Emirates"),
    (&["canada"], "Canada"),
    (&["mex"], "Mexico"),
    (&["deu", "ger", "deutschland"], "Germany"),
    (&["fra"], "France"),
    (&["esp", "espana"], "Spain"),
    (&["ita", "italia"], "Italy"),
    (&["nld", "holland", "thenetherlands"], "Netherlands"),
    (&["bel"], "Belgium"),
    (&["che", "swiss"], "Switzerland"),
    (&["aut"], "Austria"),
    (&["swe", "sverige"], "Sweden"),
    (&["norge"], "Norway"),
    (&["dnk", "danmark"], "Denmark"),
    (&["suomi"], "Finland"),
    (&["irl", "eire"], "Ireland"),
    (&["prt"], "Portugal"),
    (&["pol", "polska"], "Poland"),
    (&["cze", "czechia"], "Czech Republic"),
    (&["grc", "hellas"], "Greece"),
    (&["tur", "turkiye"], "Turkey"),
    (&["rus", "russianfederation"], "Russia"),
    (&["ukr"], "Ukraine"),
    (&["chn", "prc", "peoplesrepublicofchina"], "China"),
    (&["jpn", "nippon"], "Japan"),
    (&["kor", "southkorea", "republicofkorea"], "Korea"),
    (&["bharat"], "India"),
    (&["pak"], "Pakistan"),
    (&["aus"], "Australia"),
    (&["nzl"], "New Zealand"),
    (&["bra", "brasil"], "Brazil"),
    (&["arg"], "Argentina"),
    (&["chl"], "Chile"),
    (&["colombia"], "Colombia"),
    (&["peru"], "Peru"),
    (&["zaf", "rsa"], "South Africa"),
    (&["egy"], "Egypt"),
    (&["nga"], "Nigeria"),
    (&["kenya"], "Kenya"),
    (&["isr"], "Israel"),
    (&["sau", "ksa"], "Saudi Arabia"),
    (&["phl"], "Philippines"),
    (&["vnm", "vietnam"], "Viet Nam"),
];

/// How a value reached its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Via {
    /// No table: its own letters.
    Plain,
    /// The person's dictionary.
    Dictionary,
    /// A street word at the end.
    Street,
    /// A state's name (or its code where the column is mostly states).
    StateName,
    /// A two-letter state code.
    StateCode,
    CountryName,
    /// A country code or short name.
    CountryCode,
}

/// The tables, keyed by `look_alike_key`.
pub(crate) struct Tables {
    whole: HashMap<String, (String, Via)>,
    street: HashMap<String, &'static str>,
}

impl Tables {
    /// The built-in tables plus `extra` (variant, canonical) dictionary pairs,
    /// which win over the tables.
    pub(crate) fn new(extra: &[(String, String)]) -> Tables {
        let mut whole: HashMap<String, (String, Via)> = HashMap::new();
        for &(code, name) in STATES {
            whole.insert(look_alike_key(code), (name.to_string(), Via::StateCode));
            whole.insert(look_alike_key(name), (name.to_string(), Via::StateName));
        }
        for &(short, name) in COUNTRIES {
            for s in short {
                whole.insert((*s).to_string(), (name.to_string(), Via::CountryCode));
            }
            whole.insert(look_alike_key(name), (name.to_string(), Via::CountryName));
        }
        // "Washington" and "Georgia" stay states; "US Virgin Islands" also reads as the territory.
        whole.insert(
            "usvirginislands".into(),
            ("U.S. Virgin Islands".into(), Via::StateName),
        );
        for (variant, canonical) in extra {
            let c = canonical.trim().to_string();
            if c.is_empty() {
                continue;
            }
            whole.insert(look_alike_key(variant), (c.clone(), Via::Dictionary));
            whole
                .entry(look_alike_key(&c))
                .or_insert((c, Via::Dictionary));
        }
        let mut street = HashMap::new();
        for &(short, full) in STREET {
            for s in short {
                street.insert((*s).to_string(), full);
            }
            street.insert(full.to_lowercase(), full);
        }
        Tables { whole, street }
    }

    /// The value's full form and how it got there.
    pub(crate) fn full(&self, value: &str) -> (String, Via) {
        let k = look_alike_key(value);
        if let Some((name, via)) = self.whole.get(&k) {
            return (name.clone(), *via);
        }
        // The last word, when it is a street word and there are two words or more.
        let words: Vec<&str> = value
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|w| !w.is_empty())
            .collect();
        if words.len() >= 2 {
            let last = look_alike_key(words[words.len() - 1]);
            let word = !last.is_empty() && last.chars().all(|c| c.is_alphabetic());
            if word {
                if let Some(full) = self.street.get(&last) {
                    let head = words[..words.len() - 1].join(" ");
                    return (format!("{head} {full}"), Via::Street);
                }
            }
        }
        (value.trim().to_string(), Via::Plain)
    }
}

/// The key of a value under the built-in tables alone (tests, examples).
pub fn abbrev_key(value: &str) -> String {
    let t = Tables::new(&[]);
    look_alike_key(&t.full(value).0)
}

/// Groups of values with one full form, most common (or the full form itself)
/// first; and for each group whether it is only a "maybe" (state or country
/// codes in a column that is mostly not states or countries).
pub(crate) fn group(
    values: &[Tally],
    extra: &[(String, String)],
    progress: &Progress,
) -> Result<(Vec<Vec<u32>>, Vec<bool>), Error> {
    let t = Tables::new(extra);
    let full: Vec<(String, Via)> = values.iter().map(|v| t.full(&v.value)).collect();
    progress.step(values.len() as u64)?;
    // Is the column mostly states (or countries)? By rows.
    let total: u64 = values.iter().map(|v| v.count).sum::<u64>().max(1);
    let share = |f: &dyn Fn(Via) -> bool| -> bool {
        let n: u64 = values
            .iter()
            .zip(&full)
            .filter(|(_, (_, via))| f(*via))
            .map(|(v, _)| v.count)
            .sum();
        n * 2 >= total
    };
    let states = share(&|v| matches!(v, Via::StateCode | Via::StateName));
    let countries = share(&|v| matches!(v, Via::CountryCode | Via::CountryName));
    let mut by_key: HashMap<String, Vec<u32>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for (i, (f, _)) in full.iter().enumerate() {
        let k = look_alike_key(f);
        if k.is_empty() {
            continue;
        }
        let e = by_key.entry(k.clone()).or_default();
        if e.is_empty() {
            order.push(k);
        }
        e.push(i as u32);
    }
    let mut groups = Vec::new();
    let mut maybe = Vec::new();
    for k in order {
        let mut m = by_key.remove(&k).unwrap_or_default();
        if m.len() < 2 {
            continue;
        }
        // The full form itself first when the group holds it; then the most common.
        m.sort_by(|&a, &b| {
            let (x, y) = (&values[a as usize], &values[b as usize]);
            let fx = x.value.trim() == full[a as usize].0;
            let fy = y.value.trim() == full[b as usize].0;
            fy.cmp(&fx)
                .then_with(|| y.count.cmp(&x.count))
                .then_with(|| x.value.cmp(&y.value))
        });
        let is_maybe = m.iter().any(|&i| match full[i as usize].1 {
            Via::StateCode => !states,
            Via::CountryCode => !countries,
            _ => false,
        });
        groups.push(m);
        maybe.push(is_maybe);
    }
    Ok((groups, maybe))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tallies(v: &[(&str, u64)]) -> Vec<Tally> {
        v.iter()
            .map(|(s, c)| Tally {
                value: s.to_string(),
                count: *c,
            })
            .collect()
    }

    fn named(t: &[Tally], g: &[Vec<u32>]) -> Vec<Vec<String>> {
        g.iter()
            .map(|m| m.iter().map(|&i| t[i as usize].value.clone()).collect())
            .collect()
    }

    #[test]
    fn keys_bring_variants_together() {
        let us = abbrev_key("United States");
        for v in ["USA", "U.S.A.", "usa.", "U S A", "United States of America", "US", "u.s."] {
            assert_eq!(abbrev_key(v), us, "{v}");
        }
        let main = abbrev_key("Main Street");
        for v in ["Main St", "Main St.", "main street", "MAIN ST"] {
            assert_eq!(abbrev_key(v), main, "{v}");
        }
        let ny = abbrev_key("New York");
        for v in ["NY", "N.Y.", "ny"] {
            assert_eq!(abbrev_key(v), ny, "{v}");
        }
    }

    #[test]
    fn keys_keep_apart_what_differs() {
        assert_ne!(abbrev_key("St Louis"), abbrev_key("Street Louis"));
        assert_ne!(abbrev_key("OR Lane"), abbrev_key("Oregon Lane"));
        assert_ne!(abbrev_key("Ward 1"), abbrev_key("Ward 2"));
        assert_ne!(abbrev_key("IN progress"), abbrev_key("Indiana progress"));
        // Review note 4: Dr and Ct as street words only at the end of two words or more.
        assert_eq!(abbrev_key("Smith Dr"), abbrev_key("Smith Drive"));
        assert_ne!(abbrev_key("Dr"), abbrev_key("Drive"));
        assert_eq!(abbrev_key("Main Ct"), abbrev_key("Main Court"));
        assert_eq!(abbrev_key("CT"), abbrev_key("Connecticut"));
        assert_eq!(abbrev_key("Ct"), abbrev_key("Connecticut"));
        // Ste and Apt are units, not street words.
        assert_ne!(abbrev_key("Main Ste"), abbrev_key("Main Street"));
        assert_ne!(abbrev_key("Main Apt"), abbrev_key("Main Avenue"));
        // A number after the street word: not a street word at the end.
        assert_ne!(abbrev_key("Main St 4"), abbrev_key("Main Street 4"));
    }

    #[test]
    fn groups_take_the_full_form_first() {
        let t = tallies(&[("NY", 40), ("New York", 3), ("Main St", 9), ("Main Street", 2), ("Ward 1", 50), ("Ward 2", 50)]);
        let (g, maybe) = group(&t, &[], &Progress::new()).unwrap();
        assert_eq!(
            named(&t, &g),
            vec![vec!["New York", "NY"], vec!["Main Street", "Main St"]]
        );
        // Under half the rows are states: the NY group is only a maybe.
        assert_eq!(maybe, vec![true, false]);
    }

    /// Review Q2: in a status column "OK" is not Oklahoma (maybe, apart); in a
    /// column of states it is.
    #[test]
    fn state_codes_are_maybe_unless_the_column_is_states() {
        let status = tallies(&[("Open", 50), ("OK", 30), ("Oklahoma", 1), ("Closed", 20)]);
        let (g, maybe) = group(&status, &[], &Progress::new()).unwrap();
        assert_eq!(named(&status, &g), vec![vec!["Oklahoma", "OK"]]);
        assert_eq!(maybe, vec![true]);
        let states = tallies(&[("TN", 50), ("Tennessee", 30), ("OK", 3), ("Oklahoma", 1), ("GA", 5)]);
        let (g, maybe) = group(&states, &[], &Progress::new()).unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(maybe, vec![false, false]);
        // Countries likewise.
        let c = tallies(&[("yes", 9), ("no", 9), ("USA", 1), ("United States", 1)]);
        let (_, maybe) = group(&c, &[], &Progress::new()).unwrap();
        assert_eq!(maybe, vec![true]);
    }

    #[test]
    fn dictionary_pairs_group_and_win() {
        let t = tallies(&[("Hamilton Co.", 7), ("Hamilton County", 2), ("Ham. Cnty", 1)]);
        let extra = vec![
            ("Hamilton Co.".to_string(), "Hamilton County".to_string()),
            ("Ham. Cnty".to_string(), "Hamilton County".to_string()),
        ];
        let (g, maybe) = group(&t, &extra, &Progress::new()).unwrap();
        assert_eq!(named(&t, &g), vec![vec!["Hamilton County", "Hamilton Co.", "Ham. Cnty"]]);
        assert_eq!(maybe, vec![false]);
    }
}
