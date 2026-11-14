//! Street range lookup (Milestone 15): the precinct, district or any other
//! value for an address, from a street index that lists each street with
//! a low and high house number and the side (odd, even or both), as county
//! election offices keep them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::ops::{ColPlan, Cols};
use crate::tidy::{address, AddressStyle};
use crate::Error;

/// Most columns brought in from the index.
pub const MAX_BRING: usize = 32;
/// Rows of the index (each range).
pub const MAX_RANGES: u64 = 5_000_000;
const CHUNK: u64 = 1 << 15;

/// Which house numbers a range holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Both,
    Odd,
    Even,
}

/// The index's columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct IndexCols {
    pub street: u32,
    pub low: u32,
    pub high: u32,
    /// Odd, even or both (`O`, `E`, `B`, words, or blank for both).
    pub side: Option<u32>,
    /// A ZIP code or city that tells same-named streets apart.
    pub area: Option<u32>,
    /// Columns brought in (the precinct, district…).
    pub bring: Vec<u32>,
    pub header: bool,
}

/// Where this table's addresses are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StreetSpec {
    /// The whole street address (`123 N Main St Apt 4`), or with `number`
    /// set, the street name alone.
    pub address: u32,
    pub number: Option<u32>,
    /// ZIP code or city, matched with the index's `area`.
    pub area: Option<u32>,
    pub keep_first: bool,
    /// Add a "Lookup note" column saying why a row found nothing.
    pub note: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreetReport {
    pub found: u64,
    /// The street is not in the index.
    pub no_street: u64,
    /// The street is there but no range holds the number (or its side).
    pub no_range: u64,
    /// More than one range holds it (the first is used).
    pub overlaps: u64,
    /// No house number could be read.
    pub no_number: u64,
    /// Rows of the index that could not be read (no street or numbers).
    pub bad_ranges: u64,
}

struct Range {
    low: u64,
    high: u64,
    side: Side,
    /// Index into `values` (bring width each).
    row: u32,
}

/// The street index, copied out of its table.
pub struct StreetIndex {
    streets: HashMap<(String, String), Vec<Range>>,
    values: Vec<String>,
    width: usize,
    titles: Vec<String>,
    by_area: bool,
    pub bad_ranges: u64,
}

impl StreetIndex {
    pub fn build(
        doc: &Document,
        cols: &IndexCols,
        progress: &Progress,
    ) -> Result<StreetIndex, Error> {
        doc.ready_for_bulk()?;
        if cols.bring.is_empty() || cols.bring.len() > MAX_BRING {
            return Err(Error::Invalid(format!(
                "choose 1 to {MAX_BRING} columns to bring in"
            )));
        }
        let n = doc.col_count();
        let mut read: Vec<u32> = vec![cols.street, cols.low, cols.high];
        read.extend(cols.side);
        read.extend(cols.area);
        read.extend_from_slice(&cols.bring);
        if read.iter().any(|&c| c >= n) {
            return Err(Error::OutOfRange);
        }
        let rows = doc.data_rows(cols.header);
        if rows.end - rows.start > MAX_RANGES {
            return Err(Error::Invalid("the street index is too large".into()));
        }
        let reader = Cols::new(doc, &read);
        let side_at = cols.side.map(|_| 3);
        let area_at = cols.area.map(|_| 3 + usize::from(cols.side.is_some()));
        let bring_at = 3 + usize::from(cols.side.is_some()) + usize::from(cols.area.is_some());
        let w = cols.bring.len();
        let budget = crate::find::split_budget(doc);
        let used = AtomicUsize::new(0);
        let bad = AtomicU64::new(0);
        progress.start(rows.end - rows.start);
        type Part = Vec<(String, String, u64, u64, Side, Vec<String>)>;
        let parts: Vec<Part> = (0..(rows.end - rows.start).div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<Part, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut out = Vec::new();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = doc.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(doc, id, &mut buf);
                    let street = street_key(reader.get(&buf, 0));
                    let low = number(reader.get(&buf, 1));
                    let high = number(reader.get(&buf, 2));
                    let (Some(low), Some(high)) = (low, high) else {
                        bad.fetch_add(1, Ordering::Relaxed);
                        continue;
                    };
                    if street.is_empty() {
                        bad.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let side = side_at.map_or(Side::Both, |i| side_of(reader.get(&buf, i)));
                    let area = area_at.map_or(String::new(), |i| area_key(reader.get(&buf, i)));
                    let vals: Vec<String> = (0..w)
                        .map(|b| reader.get(&buf, bring_at + b).to_string())
                        .collect();
                    let size = street.len()
                        + area.len()
                        + vals.iter().map(|v| v.len() + 24).sum::<usize>()
                        + 64;
                    if used.fetch_add(size, Ordering::Relaxed) + size > budget {
                        return Err(Error::OutOfMemory("the street index is too large"));
                    }
                    out.push((area, street, low.min(high), low.max(high), side, vals));
                }
                progress.step(e - s)?;
                Ok(out)
            })
            .collect::<Result<_, _>>()?;
        let mut idx = StreetIndex {
            streets: HashMap::new(),
            values: Vec::new(),
            width: w,
            titles: doc.titles(&cols.bring, cols.header),
            by_area: cols.area.is_some(),
            bad_ranges: bad.into_inner(),
        };
        for (area, street, low, high, side, vals) in parts.into_iter().flatten() {
            let row = (idx.values.len() / w) as u32;
            idx.values.extend(vals);
            idx.streets.entry((area, street)).or_default().push(Range {
                low,
                high,
                side,
                row,
            });
        }
        Ok(idx)
    }

    /// The range for a house number on a street; (row, overlaps).
    fn find(&self, area: &str, street: &str, n: u64) -> Result<(u32, bool), Miss> {
        let key = (
            if self.by_area {
                area.to_string()
            } else {
                String::new()
            },
            street.to_string(),
        );
        let list = self.streets.get(&key).ok_or(Miss::Street)?;
        let mut hit = None;
        let mut more = false;
        for r in list {
            let side_ok = match r.side {
                Side::Both => true,
                Side::Odd => n % 2 == 1,
                Side::Even => n % 2 == 0,
            };
            if side_ok && (r.low..=r.high).contains(&n) {
                if hit.is_some() {
                    more = true;
                    break;
                }
                hit = Some(r.row);
            }
        }
        hit.map(|h| (h, more)).ok_or(Miss::Range)
    }
}

enum Miss {
    Street,
    Range,
}

/// A house number: the leading digits (`123`, `123A`, `123-125` -> 123).
fn number(v: &str) -> Option<u64> {
    let d: String = v
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .take(12)
        .collect();
    d.parse().ok()
}

fn side_of(v: &str) -> Side {
    let t = v.trim().to_ascii_lowercase();
    match t.chars().next() {
        Some('o') => Side::Odd,
        Some('e') => Side::Even,
        _ => Side::Both,
    }
}

fn area_key(v: &str) -> String {
    let t = v.trim();
    // A ZIP code: its first five digits.
    let digits: String = t.chars().filter(char::is_ascii_digit).collect();
    if digits.len() >= 5 && t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return digits[..5].to_string();
    }
    t.to_uppercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A street name as looked up: short forms, capitals, no punctuation, no
/// unit (`N. Main Street, Apt 4` -> `N MAIN ST`).
pub fn street_key(v: &str) -> String {
    let v = v.split(',').next().unwrap_or("").trim();
    if v.is_empty() {
        return String::new();
    }
    // The tidy step needs a house number to place the street type.
    let tidied =
        address(&format!("1 {v}"), AddressStyle::Short).unwrap_or_else(|| format!("1 {v}"));
    let mut words: Vec<String> = Vec::new();
    for w in tidied.split_whitespace().skip(1) {
        let w: String = w
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '#')
            .collect::<String>()
            .to_uppercase();
        if w.is_empty() {
            continue;
        }
        if w.starts_with('#')
            || matches!(
                w.as_str(),
                "APT" | "STE" | "UNIT" | "RM" | "FL" | "BLDG" | "LOT" | "SPC" | "TRLR"
            )
        {
            break;
        }
        words.push(w);
    }
    words.join(" ")
}

impl Document {
    /// Add the index's columns for every shown row whose address falls in
    /// a range (and a "Lookup note" column with `spec.note`).
    pub fn plan_street_lookup(
        &self,
        index: &StreetIndex,
        spec: &StreetSpec,
        progress: &Progress,
    ) -> Result<(ColPlan, StreetReport), Error> {
        self.ready_for_bulk()?;
        let n = self.col_count();
        let mut read = vec![spec.address];
        read.extend(spec.number);
        read.extend(spec.area);
        if read.iter().any(|&c| c >= n) {
            return Err(Error::OutOfRange);
        }
        let number_at = spec.number.map(|_| 1);
        let area_at = spec.area.map(|_| 1 + usize::from(spec.number.is_some()));
        let reader = Cols::new(self, &read);
        let rows = self.data_rows(spec.keep_first);
        let at = n;
        let w = index.width as u32;
        let ncols = w + u32::from(spec.note);
        let budget = crate::find::split_budget(self);
        let used = AtomicUsize::new(0);
        progress.start(rows.end - rows.start);
        type Part = (Vec<(u32, Vec<(u32, String)>)>, StreetReport);
        let parts: Vec<Part> = (0..(rows.end - rows.start).div_ceil(CHUNK))
            .into_par_iter()
            .map(|k| -> Result<Part, Error> {
                let s = rows.start + k * CHUNK;
                let e = (s + CHUNK).min(rows.end);
                let mut out = Vec::new();
                let mut rep = StreetReport::default();
                let mut buf = Vec::new();
                for r in s..e {
                    let id = self.id_at(r).ok_or(Error::OutOfRange)?;
                    reader.read(self, id, &mut buf);
                    let addr = reader.get(&buf, 0);
                    let (num, street) = match number_at {
                        Some(i) => (number(reader.get(&buf, i)), street_key(addr)),
                        None => {
                            let t = addr.trim();
                            let rest = t.trim_start_matches(|c: char| c.is_ascii_digit());
                            // `123A Main St`: a letter right after the
                            // number belongs to it.
                            let rest = rest
                                .strip_prefix(|c: char| c.is_ascii_alphabetic())
                                .filter(|x| x.starts_with([' ', '-']))
                                .unwrap_or(rest);
                            (number(t), street_key(rest.trim_start_matches(['-', ' '])))
                        }
                    };
                    let area = area_at.map_or(String::new(), |i| area_key(reader.get(&buf, i)));
                    let mut cells: Vec<(u32, String)> = Vec::new();
                    let note = match num {
                        None if !addr.trim().is_empty() => {
                            rep.no_number += 1;
                            "no house number"
                        }
                        None => "",
                        Some(n) => match index.find(&area, &street, n) {
                            Ok((row, more)) => {
                                rep.found += 1;
                                let base = row as usize * index.width;
                                for b in 0..index.width {
                                    cells.push((at + b as u32, index.values[base + b].clone()));
                                }
                                if more {
                                    rep.overlaps += 1;
                                    "more than one range holds this number (the first is used)"
                                } else {
                                    ""
                                }
                            }
                            Err(Miss::Street) => {
                                rep.no_street += 1;
                                "street not in the index"
                            }
                            Err(Miss::Range) => {
                                rep.no_range += 1;
                                "house number not in any range of the street"
                            }
                        },
                    };
                    if spec.note && !note.is_empty() {
                        cells.push((at + w, note.to_string()));
                    }
                    if !cells.is_empty() {
                        let size: usize = cells.iter().map(|c| c.1.len() + 32).sum();
                        if used.fetch_add(size, Ordering::Relaxed) + size > budget {
                            return Err(Error::OutOfMemory(
                                "this would change too many cells at once; use fewer rows",
                            ));
                        }
                        out.push((id, cells));
                    }
                }
                progress.step(e - s)?;
                Ok((out, rep))
            })
            .collect::<Result<_, _>>()?;
        let mut plan = ColPlan::new(self);
        plan.insert = Some((at, ncols));
        plan.cols = ncols;
        if rows.start == 1 {
            let mut t: Vec<(u32, String)> = index
                .titles
                .iter()
                .enumerate()
                .map(|(b, x)| (at + b as u32, x.clone()))
                .collect();
            if spec.note {
                t.push((at + w, "Lookup note".into()));
            }
            plan.changes.push((self.id_at(0).expect("a first row"), t));
        }
        let mut rep = StreetReport {
            bad_ranges: index.bad_ranges,
            ..Default::default()
        };
        for (cells, r) in parts {
            plan.changes.extend(cells);
            rep.found += r.found;
            rep.no_street += r.no_street;
            rep.no_range += r.no_range;
            rep.overlaps += r.overlaps;
            rep.no_number += r.no_number;
        }
        plan.cells = plan.changes.len() as u64;
        Ok((plan, rep))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    #[test]
    fn keys() {
        assert_eq!(street_key("North Main Street"), "N MAIN ST");
        assert_eq!(street_key("n. main st., apt 4"), "N MAIN ST");
        assert_eq!(street_key("Main St #4"), "MAIN ST");
        assert_eq!(number("123A"), Some(123));
        assert_eq!(number("Main"), None);
        assert_eq!(area_key("37402-1234"), "37402");
    }

    #[test]
    fn looks_up_precincts() {
        let idx = doc("street,from,to,side,zip,precinct\r\nN Main St,100,198,even,37402,P1\r\nN Main St,101,199,odd,37402,P2\r\nN Main St,100,199,both,37403,P9\r\nOak Ave,1,99,,37402,P3\r\n");
        let p = Progress::new();
        let index = StreetIndex::build(
            &idx,
            &IndexCols {
                street: 0,
                low: 1,
                high: 2,
                side: Some(3),
                area: Some(4),
                bring: vec![5],
                header: true,
            },
            &p,
        )
        .unwrap();
        let mut d = doc("addr,zip\r\n120 North Main Street,37402\r\n121 n main st apt 2,37402\r\n150 N Main St,37403\r\n500 N Main St,37402\r\n5 Elm St,37402\r\nPO Box 3,37402\r\n42 Oak Avenue,37402-0001\r\n");
        let (plan, rep) = d
            .plan_street_lookup(
                &index,
                &StreetSpec {
                    address: 0,
                    number: None,
                    area: Some(1),
                    keep_first: true,
                    note: true,
                },
                &p,
            )
            .unwrap();
        d.apply_cols(plan).unwrap();
        let col = |r: u64| d.cell(r, 2).unwrap();
        assert_eq!(d.cell(0, 2).unwrap(), "precinct");
        assert_eq!(d.cell(0, 3).unwrap(), "Lookup note");
        assert_eq!(
            (col(1), col(2), col(3), col(7)),
            ("P1".into(), "P2".into(), "P9".into(), "P3".into())
        );
        assert_eq!(col(4), "");
        assert_eq!(
            d.cell(4, 3).unwrap(),
            "house number not in any range of the street"
        );
        assert_eq!(d.cell(5, 3).unwrap(), "street not in the index");
        assert_eq!(d.cell(6, 3).unwrap(), "no house number");
        assert_eq!(
            (rep.found, rep.no_range, rep.no_street, rep.no_number),
            (4, 1, 1, 1)
        );
    }
}
