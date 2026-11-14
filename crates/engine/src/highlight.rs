//! Highlight rules (conditional colours): a filter rule on a column plus a
//! colour from a fixed palette, for one cell or the whole row. Rules are
//! evaluated only for the cells on screen, with patterns compiled under the
//! find limits; colours are palette numbers, never style text.

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::filter::{Rule, RuleMatcher};
use crate::value::ReadOptions;
use crate::Error;

/// Most rules at once.
pub const MAX_RULES: usize = 200;
/// Colours in the palette (numbers 0..PALETTE).
pub const PALETTE: u8 = 8;
/// Largest area evaluated at once (rows x columns of a screen).
pub const MAX_ROWS: u64 = 1_000;
pub const MAX_COLS: u32 = 500;

/// One highlight rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub col: u32,
    pub rule: Rule,
    /// Palette colour.
    pub color: u8,
    /// Colour the whole row, not just the cell.
    #[serde(default)]
    pub whole_row: bool,
}

/// A coloured cell (`col` is `None` for a whole row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Mark {
    pub row: u64,
    pub col: Option<u32>,
    pub color: u8,
}

/// Check rules before use (also for rules loaded from a view file).
pub fn check(rules: &[Highlight], read: ReadOptions) -> Result<Vec<RuleMatcher>, Error> {
    if rules.len() > MAX_RULES {
        return Err(Error::Invalid(format!(
            "at most {MAX_RULES} highlight rules"
        )));
    }
    rules
        .iter()
        .map(|h| {
            if h.color >= PALETTE {
                return Err(Error::Invalid("unknown highlight colour".into()));
            }
            RuleMatcher::new(&h.rule, read)
        })
        .collect()
}

impl Document {
    /// The marks for shown rows `row..row+rows`, columns `col..col+cols`
    /// (whole-row rules look at their own column wherever it is). The first
    /// matching rule wins for a cell; whole-row marks come first.
    #[allow(clippy::too_many_arguments)] // an area (4 numbers) plus the rules and how to read them
    pub fn highlights(
        &self,
        row: u64,
        rows: u64,
        col: u32,
        cols: u32,
        rules: &[Highlight],
        skip_first: bool,
        read: ReadOptions,
    ) -> Result<Vec<Mark>, Error> {
        let m = check(rules, read)?;
        let rows = rows.min(MAX_ROWS);
        let cols = cols.min(MAX_COLS);
        let n = self.row_count();
        let r0 = row.max(u64::from(skip_first && n > 0));
        let r1 = row.saturating_add(rows).min(n);
        let mut out = Vec::new();
        let mut buf = Vec::new();
        let cols_read: Vec<u32> = rules.iter().map(|h| h.col).collect();
        let reader = crate::ops::Cols::new(self, &cols_read);
        for r in r0..r1 {
            let Some(id) = self.id_at(r) else { continue };
            reader.read(self, id, &mut buf);
            let mut row_done = false;
            let mut cells_done: Vec<u32> = Vec::new();
            for (k, (h, t)) in rules.iter().zip(&m).enumerate() {
                let v = reader.get(&buf, k);
                if h.whole_row {
                    if !row_done && t.matches(v) {
                        out.push(Mark {
                            row: r,
                            col: None,
                            color: h.color,
                        });
                        row_done = true;
                    }
                } else if (col..col.saturating_add(cols)).contains(&h.col)
                    && !cells_done.contains(&h.col)
                    && t.matches(v)
                {
                    out.push(Mark {
                        row: r,
                        col: Some(h.col),
                        color: h.color,
                    });
                    cells_done.push(h.col);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{CmpOp, Condition};
    use crate::find::{Pattern, PatternKind};
    use crate::OpenOptions;

    #[test]
    fn marks_cells_and_rows() {
        let d = Document::from_bytes(
            b"n,t\r\n5,ok\r\n50,bad\r\n500,ok\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let rules = vec![
            Highlight {
                col: 0,
                rule: Rule::Conditions {
                    first: Condition::Number {
                        op: CmpOp::Gt,
                        value: 10.0,
                    },
                    second: None,
                    or: false,
                },
                color: 1,
                whole_row: false,
            },
            Highlight {
                col: 1,
                rule: Rule::Values {
                    values: vec!["bad".into()],
                    blanks: false,
                },
                color: 3,
                whole_row: true,
            },
        ];
        let m = d
            .highlights(0, 100, 0, 10, &rules, true, ReadOptions::default())
            .unwrap();
        assert_eq!(
            m,
            [
                Mark {
                    row: 2,
                    col: Some(0),
                    color: 1
                },
                Mark {
                    row: 2,
                    col: None,
                    color: 3
                },
                Mark {
                    row: 3,
                    col: Some(0),
                    color: 1
                },
            ]
        );
        // Bounds: bad colours, too many rules, patterns under find limits.
        let mut bad = rules.clone();
        bad[0].color = PALETTE;
        assert!(check(&bad, ReadOptions::default()).is_err());
        let many = vec![rules[0].clone(); MAX_RULES + 1];
        assert!(check(&many, ReadOptions::default()).is_err());
        let huge = Highlight {
            col: 0,
            rule: Rule::Conditions {
                first: Condition::Pattern {
                    pattern: Pattern {
                        text: "(\\w{100}){100}{100}".into(),
                        kind: PatternKind::Regex,
                        ..Default::default()
                    },
                    negate: false,
                },
                second: None,
                or: false,
            },
            color: 0,
            whole_row: false,
        };
        assert!(check(&[huge], ReadOptions::default()).is_err());
    }
}
