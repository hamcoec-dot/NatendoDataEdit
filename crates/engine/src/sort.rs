//! Sorting rows by one or more columns.
//!
//! Only the sorted columns' keys are held in memory (text keys cut to
//! [`KEY_BYTES`]; a tie between two cut keys re-reads the full values). The
//! first key is also packed into one 64-bit number per row, so most
//! comparisons are a single integer compare. The result is a new row order,
//! applied as one undo step; the file's bytes never move.

use std::cmp::Ordering;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::coltype::{ColumnType, ReadCtx};
use crate::document::{Document, Progress};
use crate::value::{self, ReadOptions};
use crate::Error;

/// Bytes of a text key kept per cell.
pub const KEY_BYTES: usize = 256;
/// Most memory sort keys may use.
///
/// Note for later ("sort only the selection", review M4 #4): every row's
/// keys are parsed and held for the whole sort. At 10M rows that stays well
/// inside this cap, but a selection sort should extract keys for the
/// selected rows only rather than reuse this path over the whole file.
pub const MAX_KEY_MEMORY: usize = 2 << 30;
/// Most sort keys (columns) at once.
pub const MAX_KEYS: usize = 64;
/// Rows per key chunk (keys are found by `pos >> CHUNK_BITS`).
const CHUNK_BITS: u32 = 16;
const CHUNK: usize = 1 << CHUNK_BITS;
/// Rows sampled to pick an automatic ordering.
const SAMPLE: u64 = 2000;

/// How a column's values are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKind {
    /// Numbers, dates or natural text, chosen from a sample of the column.
    #[default]
    Auto,
    /// Character by character (case and accents folded first).
    Text,
    /// Text with numbers inside compared by value: `file2` before `file10`.
    Natural,
    /// Numbers by value, then other text.
    Number,
    /// Dates and times, then other text.
    Date,
    /// By the number of characters.
    Length,
    /// By the last word (surnames, file extensions), natural order.
    LastWord,
    /// In the order of `SortSpec::lists[i]`; values not on the list after
    /// them, in natural order.
    List(u16),
}

/// Most custom lists in one sort, items in a list, characters in an item.
pub const MAX_LISTS: usize = 64;
pub const MAX_LIST_ITEMS: usize = 10_000;
pub const MAX_ITEM_CHARS: usize = 1_000;

/// One sort column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortKey {
    pub col: u32,
    #[serde(default)]
    pub descending: bool,
    #[serde(default)]
    pub kind: SortKind,
    /// Lowercase before uppercase of the same letter (otherwise case is
    /// ignored and equal values keep their order).
    #[serde(default)]
    pub match_case: bool,
}

/// A whole sort.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SortSpec {
    pub keys: Vec<SortKey>,
    /// The first row (column titles) stays first.
    pub keep_first: bool,
    pub read: ReadOptions,
    /// Custom orders, used by [`SortKind::List`].
    pub lists: Vec<Vec<String>>,
    /// Which rows are put in order. Left out (sorts and recipes saved before
    /// this was recorded): all of them.
    pub scope: SortScope,
}

/// Which rows a sort puts in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortScope {
    /// Every row, hidden ones too.
    #[default]
    All,
    /// The shown rows, among their own places: a row a filter or the user hid
    /// stays where it is.
    Shown,
}

/// A computed sort, applied with [`Document::apply_sort`].
pub struct SortPlan {
    revision: u64,
    /// The new order of every row id, or `None` when already in order.
    order: Option<Vec<u32>>,
}

impl SortPlan {
    /// A plan that puts every row id in `order` (all rows, filtered or
    /// not), or leaves them as they are with `None`.
    pub(crate) fn from_order(doc: &Document, order: Option<Vec<u32>>) -> SortPlan {
        SortPlan {
            revision: doc.revision(),
            order,
        }
    }

    /// False when the rows were already in this order.
    pub fn changes(&self) -> bool {
        self.order.is_some()
    }
}

/// Value classes, in ascending order; blanks always go last.
const NUM: u8 = 0;
const TEXT: u8 = 1;
const BLANK: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Natural,
    Number,
    Date,
    Length,
    LastWord,
    List,
    /// An identifier column: digit runs by value, more leading zeros after, exact text.
    Id,
}

struct Key {
    phys: Option<u32>,
    /// The type of the column (what a number or a date read of it gives).
    ty: ColumnType,
    mode: Mode,
    desc: bool,
    case: bool,
    /// Rank of each listed value (folded text key), for [`Mode::List`].
    list: Option<std::collections::HashMap<String, f64>>,
}

/// The folded part of a text key (case and accents ignored).
fn folded(s: &str, out: &mut String) {
    value::text_key(s, false, out);
    if let Some(i) = out.find('\0') {
        out.truncate(i);
    }
}

/// Keys of one column for one chunk of rows.
#[derive(Default)]
struct Col {
    class: Vec<u8>,
    num: Vec<f64>,
    ends: Vec<u32>,
    bytes: Vec<u8>,
    /// Key was cut at KEY_BYTES.
    cut: Vec<bool>,
}

impl Col {
    fn text(&self, i: usize) -> &[u8] {
        let s = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        &self.bytes[s..self.ends[i] as usize]
    }
}

/// Orderable bits of a float (larger float, larger bits).
fn float_bits(x: f64) -> u64 {
    let b = x.to_bits();
    if b >> 63 == 1 {
        !b
    } else {
        b | (1 << 63)
    }
}

/// Cut a key at `KEY_BYTES` on a character boundary; for natural order also
/// drop a digit run at the cut, whose value would be wrong.
fn cut_key(k: &str, natural: bool) -> (&[u8], bool) {
    if k.len() <= KEY_BYTES {
        return (k.as_bytes(), false);
    }
    let mut end = KEY_BYTES;
    while !k.is_char_boundary(end) {
        end -= 1;
    }
    let mut s = &k.as_bytes()[..end];
    if natural {
        while s.last().is_some_and(u8::is_ascii_digit) {
            s = &s[..s.len() - 1];
        }
    }
    (s, true)
}

impl Key {
    /// Class, number and full text key of one cell.
    fn read(&self, s: &str, ctx: &ReadCtx, text: &mut String) -> (u8, f64) {
        text.clear();
        if s.is_empty() {
            return (BLANK, 0.0);
        }
        if self.mode == Mode::Id {
            // Exact text: no folding, no trimming.
            text.push_str(s);
            return (TEXT, 0.0);
        }
        let n = match self.mode {
            Mode::Number => ctx.number_as(self.ty, s),
            Mode::Date => ctx.date_as(self.ty, s),
            Mode::Length => Some(s.chars().count() as f64),
            Mode::List => {
                folded(s.trim(), text);
                self.list
                    .as_ref()
                    .and_then(|l| l.get(text.as_str()).copied())
            }
            Mode::Text | Mode::Natural | Mode::LastWord | Mode::Id => None,
        };
        if let Some(v) = n {
            text.clear();
            return (NUM, v);
        }
        let s = if self.mode == Mode::LastWord {
            s.split_whitespace().last().unwrap_or(s)
        } else {
            s
        };
        value::text_key(s, self.case, text);
        (TEXT, 0.0)
    }

    /// Text is compared with digit runs by value.
    fn natural(&self) -> bool {
        matches!(
            self.mode,
            Mode::Natural | Mode::LastWord | Mode::List | Mode::Id
        )
    }

    /// Keeps a number per row.
    fn numeric(&self) -> bool {
        matches!(
            self.mode,
            Mode::Number | Mode::Date | Mode::Length | Mode::List
        )
    }

    fn text_cmp(&self, a: &[u8], b: &[u8]) -> Ordering {
        if self.mode == Mode::Id {
            crate::coltype::id_order_bytes(a, b)
        } else if self.natural() {
            value::natural_cmp(a, b)
        } else {
            a.cmp(b)
        }
    }
}

/// The document position of the `i`th row being sorted.
fn place(places: &Option<Vec<u32>>, first: u64, i: usize) -> u64 {
    match places {
        Some(p) => u64::from(p[i]),
        None => first + i as u64,
    }
}

struct Keys<'a> {
    places: &'a Option<Vec<u32>>,
    doc: &'a Document,
    keys: Vec<Key>,
    /// `chunks[c][k]`: keys of column `k` for chunk `c`.
    chunks: Vec<Vec<Col>>,
    /// Document position of relative position 0.
    first: u64,
    read: ReadOptions,
}

impl Keys<'_> {
    fn col(&self, k: usize, pos: u32) -> (&Col, usize) {
        let p = pos as usize;
        (&self.chunks[p >> CHUNK_BITS][k], p & (CHUNK - 1))
    }

    /// Full comparison of rows (relative positions) `a` and `b`.
    fn cmp(&self, a: u32, b: u32) -> Ordering {
        for (k, key) in self.keys.iter().enumerate() {
            let (ca, ia) = self.col(k, a);
            let (cb, ib) = self.col(k, b);
            let (xa, xb) = (ca.class[ia], cb.class[ib]);
            let ord = if xa == BLANK || xb == BLANK {
                // Blanks last in either direction.
                xa.cmp(&xb)
            } else {
                let o = if xa != xb {
                    xa.cmp(&xb)
                } else if xa == NUM {
                    ca.num[ia].total_cmp(&cb.num[ib])
                } else {
                    let (ta, tb) = (ca.text(ia), cb.text(ib));
                    // A cut key decides only where the two differ before
                    // either cut (natural order: never, digit runs matter).
                    let cut = ca.cut[ia] || cb.cut[ib];
                    if cut && (key.natural() || ta.starts_with(tb) || tb.starts_with(ta)) {
                        self.full_cmp(key, a, b)
                    } else {
                        key.text_cmp(ta, tb)
                    }
                };
                if key.desc {
                    o.reverse()
                } else {
                    o
                }
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    }

    /// Compare two cut text keys by re-reading both cells in full.
    fn full_cmp(&self, key: &Key, a: u32, b: u32) -> Ordering {
        let text = |pos: u32| -> String {
            let Some(p) = key.phys else {
                return String::new();
            };
            let id = self
                .doc
                .doc_id_at(place(self.places, self.first, pos as usize))
                .expect("sorted rows exist");
            let mut buf = Vec::new();
            self.doc.read_cells(id, &[p], &mut buf);
            let mut k = String::new();
            key.read(&buf[0], &self.doc.read_ctx(self.read), &mut k);
            k
        };
        key.text_cmp(text(a).as_bytes(), text(b).as_bytes())
    }

    /// The first key packed into 64 bits, ordered like [`Keys::cmp`] for
    /// that key (equal packs fall back to the full comparison).
    fn pack(&self, pos: u32) -> u64 {
        let key = &self.keys[0];
        let (c, i) = self.col(0, pos);
        let class = c.class[i];
        if class == BLANK {
            return u64::MAX;
        }
        let rank = if key.desc { 1 - class } else { class } as u64;
        let bits = if class == NUM {
            float_bits(c.num[i])
        } else if key.natural() {
            0
        } else {
            let t = c.text(i);
            let mut b = [0u8; 8];
            let n = t.len().min(8);
            b[..n].copy_from_slice(&t[..n]);
            u64::from_be_bytes(b)
        };
        let bits = if key.desc { !bits } else { bits };
        (rank << 62) | (bits >> 2)
    }
}

impl Document {
    /// Pick an ordering for column `phys` from a sample of rows.
    fn auto_mode(
        &self,
        phys: Option<u32>,
        col: u32,
        rows: std::ops::Range<u64>,
        ctx: &ReadCtx,
    ) -> Mode {
        let Some(p) = phys else {
            return Mode::Natural;
        };
        // A column the person typed is not guessed again.
        match ctx.type_of(col) {
            ColumnType::Id => return Mode::Id,
            ColumnType::Text => return Mode::Natural,
            ColumnType::Number => return Mode::Number,
            ColumnType::Date(_) => return Mode::Date,
            ColumnType::Auto => {}
        }
        let n = rows.end - rows.start;
        let step = (n / SAMPLE).max(1);
        let (mut seen, mut nums, mut dates) = (0u32, 0u32, 0u32);
        let mut buf = Vec::new();
        let mut pos = rows.start;
        while pos < rows.end {
            if let Some(id) = self.doc_id_at(pos) {
                self.read_cells(id, &[p], &mut buf);
                let s = buf[0].trim();
                if !s.is_empty() {
                    seen += 1;
                    if ctx.number(col, s).is_some() {
                        nums += 1;
                    } else if ctx.date(col, s).is_some() {
                        dates += 1;
                    }
                }
            }
            pos += step;
        }
        if seen == 0 {
            Mode::Natural
        } else if nums * 10 >= seen * 9 {
            Mode::Number
        } else if (nums + dates) * 10 >= seen * 9 && dates > 0 {
            Mode::Date
        } else {
            Mode::Natural
        }
    }

    /// Work out a sort (nothing changes yet). Every row is sorted, shown or
    /// not; with `keep_first` the first row stays first.
    pub fn plan_sort(&self, spec: &SortSpec, progress: &Progress) -> Result<SortPlan, Error> {
        self.ready_for_bulk()?;
        if spec.keys.is_empty() || spec.keys.len() > MAX_KEYS {
            return Err(Error::BadPattern(format!(
                "sort by 1 to {MAX_KEYS} columns"
            )));
        }
        if spec.lists.len() > MAX_LISTS
            || spec.lists.iter().any(|l| {
                l.len() > MAX_LIST_ITEMS || l.iter().any(|i| i.chars().count() > MAX_ITEM_CHARS)
            })
        {
            return Err(Error::Invalid(format!(
                "custom orders: at most {MAX_LISTS} lists of {MAX_LIST_ITEMS} items of {MAX_ITEM_CHARS} characters"
            )));
        }
        let list = |i: u16| -> Result<std::collections::HashMap<String, f64>, Error> {
            let items = spec
                .lists
                .get(i as usize)
                .ok_or_else(|| Error::Invalid("that custom order is missing".into()))?;
            let mut map = std::collections::HashMap::with_capacity(items.len());
            let mut k = String::new();
            for (r, item) in items.iter().enumerate() {
                folded(item.trim(), &mut k);
                map.entry(k.clone()).or_insert(r as f64);
            }
            Ok(map)
        };
        let total = self.total_rows();
        let first = u64::from(spec.keep_first && total > 0);
        // The document positions to put in order, when not simply every row
        // from `first` on.
        let places: Option<Vec<u32>> = match spec.scope {
            SortScope::Shown => self.shown_places().map(|shown| {
                shown
                    .iter()
                    .copied()
                    .filter(|&p| u64::from(p) >= first)
                    .collect()
            }),
            SortScope::All => None,
        };
        let n = places.as_ref().map_or((total - first) as usize, Vec::len);
        let ctx = self.read_ctx(spec.read);
        let keys: Vec<Key> = spec
            .keys
            .iter()
            .map(|k| -> Result<Key, Error> {
                let phys = (k.col < self.col_count())
                    .then(|| self.phys(k.col))
                    .flatten();
                Ok(Key {
                    phys,
                    ty: ctx.type_of(k.col),
                    mode: match k.kind {
                        SortKind::Auto => self.auto_mode(phys, k.col, first..total, &ctx),
                        SortKind::Text => Mode::Text,
                        SortKind::Natural => Mode::Natural,
                        SortKind::Number => Mode::Number,
                        SortKind::Date => Mode::Date,
                        SortKind::Length => Mode::Length,
                        SortKind::LastWord => Mode::LastWord,
                        SortKind::List(_) => Mode::List,
                    },
                    desc: k.descending,
                    case: k.match_case,
                    list: match k.kind {
                        SortKind::List(i) => Some(list(i)?),
                        _ => None,
                    },
                })
            })
            .collect::<Result<_, _>>()?;
        progress.start(2 * n as u64);
        let mut wanted: Vec<u32> = keys.iter().filter_map(|k| k.phys).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let slots: Vec<Option<usize>> = keys
            .iter()
            .map(|k| k.phys.map(|p| wanted.binary_search(&p).expect("listed")))
            .collect();
        let memory = AtomicUsize::new(0);
        let chunks: Vec<Vec<Col>> = (0..n.div_ceil(CHUNK))
            .into_par_iter()
            .map(|c| -> Result<Vec<Col>, Error> {
                let s = c * CHUNK;
                let e = (s + CHUNK).min(n);
                let mut cols: Vec<Col> = keys.iter().map(|_| Col::default()).collect();
                let mut buf = Vec::new();
                let mut text = String::new();
                for pos in s..e {
                    let at = place(&places, first, pos);
                    let id = self.doc_id_at(at).expect("in range");
                    self.read_cells(id, &wanted, &mut buf);
                    for (k, key) in keys.iter().enumerate() {
                        let cell = slots[k].map_or("", |s| &buf[s]);
                        let (class, num) = key.read(cell, &ctx, &mut text);
                        let col = &mut cols[k];
                        col.class.push(class);
                        if key.numeric() {
                            col.num.push(num);
                        }
                        let (t, cut) = cut_key(&text, key.natural());
                        col.bytes.extend_from_slice(t);
                        col.ends.push(col.bytes.len() as u32);
                        col.cut.push(cut);
                    }
                }
                let used: usize = cols
                    .iter()
                    .map(|c| c.bytes.capacity() + c.class.len() * 14 + c.num.len() * 8)
                    .sum();
                if memory.fetch_add(used, AtomicOrdering::Relaxed) + used > MAX_KEY_MEMORY {
                    return Err(Error::OutOfMemory(
                        "these columns hold too much text to sort at once",
                    ));
                }
                progress.step((e - s) as u64)?;
                Ok(cols)
            })
            .collect::<Result<_, _>>()?;
        let ks = Keys {
            places: &places,
            doc: self,
            keys,
            chunks,
            first,
            read: spec.read,
        };
        let mut perm: Vec<(u64, u32)> = (0..n as u32)
            .into_par_iter()
            .map(|p| (ks.pack(p), p))
            .collect();
        progress.step(n as u64 / 2)?;
        perm.par_sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| ks.cmp(a.1, b.1))
                .then_with(|| a.1.cmp(&b.1))
        });
        progress.step(n as u64 / 2)?;
        let sorted = perm.iter().enumerate().all(|(i, &(_, p))| p as usize == i);
        let order = if sorted {
            None
        } else {
            let id_at = |p: u64| self.doc_id_at(p).expect("in range");
            let mut order: Vec<u32> = (0..total).map(id_at).collect();
            for (i, &(_, p)) in perm.iter().enumerate() {
                order[place(&places, first, i) as usize] = id_at(place(&places, first, p as usize));
            }
            Some(order)
        };
        Ok(SortPlan {
            revision: self.revision(),
            order,
        })
    }

    /// Put a computed sort in place as one undo step. Fails with
    /// [`Error::Changed`] if the document changed since it was computed.
    pub fn apply_sort(&mut self, plan: SortPlan) -> Result<(), Error> {
        if plan.revision != self.revision() {
            return Err(Error::Changed);
        }
        if let Some(order) = plan.order {
            self.set_order(order);
        }
        Ok(())
    }

    /// Compute and apply a sort. Returns false when the rows were already
    /// in order (no undo step is added then).
    pub fn sort(&mut self, spec: &SortSpec) -> Result<bool, Error> {
        let plan = self.plan_sort(spec, &Progress::new())?;
        let changed = plan.changes();
        self.apply_sort(plan)?;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;
    use proptest::prelude::*;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    fn col(d: &Document, c: u32) -> Vec<String> {
        (0..d.row_count()).map(|r| d.cell(r, c).unwrap()).collect()
    }

    fn by(col: u32, kind: SortKind, descending: bool) -> SortSpec {
        SortSpec {
            scope: Default::default(),
            keys: vec![SortKey {
                col,
                descending,
                kind,
                match_case: false,
            }],
            keep_first: false,
            read: ReadOptions::default(),
            lists: Vec::new(),
        }
    }

    #[test]
    fn a_sort_of_the_shown_rows_leaves_hidden_rows_in_their_places() {
        let mut d = doc("h\r\n5\r\n9\r\n3\r\n8\r\n1\r\n");
        d.hide_rows(2, 1).unwrap(); // the 9
        d.hide_rows(3, 1).unwrap(); // the 8
        let before = d.revision();
        let mut spec = by(0, SortKind::Number, false);
        spec.keep_first = true;
        spec.scope = SortScope::Shown;
        assert!(d.sort(&spec).unwrap());
        assert_eq!(col(&d, 0), ["h", "1", "3", "5"]);
        d.show_rows(crate::rows::ShowRows::All).unwrap();
        assert_eq!(col(&d, 0), ["h", "1", "9", "3", "8", "5"]);
        assert!(d.revision() > before);
        // One undo step (after the one that showed the rows) puts the whole
        // order back.
        d.undo();
        d.undo();
        d.show_rows(crate::rows::ShowRows::All).unwrap();
        assert_eq!(col(&d, 0), ["h", "5", "9", "3", "8", "1"]);
        // A sort saved before the choice existed sorts every row.
        let old: SortSpec = serde_json::from_str(r#"{"keys":[{"col":0}]}"#).unwrap();
        assert_eq!(old.scope, SortScope::All);
    }

    proptest! {
        /// Shown rows come out as a plain sort of those rows alone; hidden
        /// rows do not move.
        #[test]
        fn shown_sort_equals_sorting_the_shown_rows(
            rows in proptest::collection::vec(("[a-c]{0,3}", proptest::bool::ANY), 1..40),
            desc in proptest::bool::ANY,
        ) {
            let text: String = rows.iter().enumerate().map(|(i, r)| format!("{},{i}\r\n", r.0)).collect();
            let mut d = doc(&text);
            for (i, r) in rows.iter().enumerate().rev() {
                if r.1 {
                    d.hide_rows(i as u64, 1).unwrap();
                }
            }
            let mut spec = by(0, SortKind::Text, desc);
            spec.scope = SortScope::Shown;
            let shown: String = rows.iter().enumerate().filter(|(_, r)| !r.1).map(|(i, r)| format!("{},{i}\r\n", r.0)).collect();
            d.sort(&spec).unwrap();
            if !shown.is_empty() {
                let mut alone = doc(&shown);
                alone.sort(&by(0, SortKind::Text, desc)).unwrap();
                prop_assert_eq!(col(&d, 1), col(&alone, 1));
            }
            d.show_rows(crate::rows::ShowRows::All).unwrap();
            let after = col(&d, 1);
            for (i, r) in rows.iter().enumerate() {
                if r.1 {
                    prop_assert_eq!(&after[i], &i.to_string());
                }
            }
        }
    }

    #[test]
    fn length_last_word_and_custom_lists() {
        let mut d = doc("ccc\r\na\r\n\r\nbb\r\n");
        d.sort(&by(0, SortKind::Length, false)).unwrap();
        assert_eq!(col(&d, 0), ["a", "bb", "ccc", ""]);
        d.sort(&by(0, SortKind::Length, true)).unwrap();
        assert_eq!(col(&d, 0), ["ccc", "bb", "a", ""]);
        let mut d = doc("Ann Zed\r\nBo Adams\r\nCy\r\nDee Mo 2\r\nEd Mo 10\r\n");
        d.sort(&by(0, SortKind::LastWord, false)).unwrap();
        assert_eq!(
            col(&d, 0),
            ["Dee Mo 2", "Ed Mo 10", "Bo Adams", "Cy", "Ann Zed"]
        );
        let mut d = doc("Medium\r\nother\r\nhigh\r\nLow\r\n\r\nAnother\r\nLOW\r\n");
        let mut spec = by(0, SortKind::List(0), false);
        spec.lists = vec![vec!["Low".into(), "Medium".into(), "High".into()]];
        d.sort(&spec).unwrap();
        assert_eq!(
            col(&d, 0),
            ["Low", "LOW", "Medium", "high", "Another", "other", ""]
        );
        spec.keys[0].descending = true;
        d.sort(&spec).unwrap();
        assert_eq!(
            col(&d, 0),
            ["other", "Another", "high", "Medium", "Low", "LOW", ""]
        );
        let mut bad = by(0, SortKind::List(3), false);
        assert!(d.sort(&bad).is_err());
        bad.lists = vec![vec!["x".repeat(MAX_ITEM_CHARS + 1)]];
        bad.keys[0].kind = SortKind::List(0);
        assert!(d.sort(&bad).is_err());
        // Saved with views: the kind reads back.
        let j = serde_json::to_string(&SortKind::List(2)).unwrap();
        assert_eq!(j, "{\"list\":2}");
        assert_eq!(
            serde_json::from_str::<SortKind>("\"last_word\"").unwrap(),
            SortKind::LastWord
        );
    }

    #[test]
    fn kinds_and_blanks() {
        let src = "10\r\n9\r\n\r\nabc\r\n-2.5\r\n\"$1,000\"\r\nAbc\r\n";
        let mut d = doc(src);
        d.sort(&by(0, SortKind::Number, false)).unwrap();
        assert_eq!(col(&d, 0), ["-2.5", "9", "10", "$1,000", "abc", "Abc", ""]);
        d.sort(&by(0, SortKind::Number, true)).unwrap();
        assert_eq!(col(&d, 0), ["abc", "Abc", "$1,000", "10", "9", "-2.5", ""]);
        d.sort(&by(0, SortKind::Text, false)).unwrap();
        assert_eq!(col(&d, 0), ["$1,000", "-2.5", "10", "9", "abc", "Abc", ""]);
        let mut d = doc("file10\r\nfile2\r\nFile1\r\n");
        d.sort(&by(0, SortKind::Auto, false)).unwrap();
        assert_eq!(col(&d, 0), ["File1", "file2", "file10"]);
        let mut d = doc("3/1/2024\r\n2024-01-15\r\n\"Jan 2, 2024\"\r\n");
        d.sort(&by(0, SortKind::Auto, false)).unwrap();
        assert_eq!(col(&d, 0), ["Jan 2, 2024", "2024-01-15", "3/1/2024"]);
    }

    #[test]
    fn several_keys_stable_header_and_undo() {
        let src = "name,team,score\r\nann,b,3\r\nbob,a,3\r\ncat,b,10\r\ndan,a,1\r\neve,b,3\r\n";
        let mut d = doc(src);
        let spec = SortSpec {
            scope: Default::default(),
            keys: vec![
                SortKey {
                    col: 1,
                    descending: false,
                    kind: SortKind::Text,
                    match_case: false,
                },
                SortKey {
                    col: 2,
                    descending: true,
                    kind: SortKind::Auto,
                    match_case: false,
                },
            ],
            keep_first: true,
            read: ReadOptions::default(),
            lists: Vec::new(),
        };
        assert!(d.sort(&spec).unwrap());
        assert_eq!(col(&d, 0), ["name", "bob", "dan", "cat", "ann", "eve"]);
        // Sorting again changes nothing and adds no undo step.
        assert!(!d.sort(&spec).unwrap());
        d.undo();
        assert_eq!(col(&d, 0), ["name", "ann", "bob", "cat", "dan", "eve"]);
        assert!(!d.is_modified());
        d.redo();
        assert_eq!(col(&d, 0)[1], "bob");
    }

    #[test]
    fn case_and_accents() {
        let mut d = doc("b\r\nB\r\nÉcole\r\neagle\r\na\r\nA\r\n");
        let mut spec = by(0, SortKind::Text, false);
        d.sort(&spec).unwrap();
        assert_eq!(col(&d, 0), ["a", "A", "b", "B", "eagle", "École"]);
        spec.keys[0].match_case = true;
        let mut d = doc("B\r\nb\r\nA\r\na\r\n");
        d.sort(&spec).unwrap();
        assert_eq!(col(&d, 0), ["a", "A", "b", "B"]);
    }

    #[test]
    fn long_keys_compare_in_full() {
        let base = "x".repeat(300);
        let rows = [format!("{base}b"), format!("{base}a"), base.clone()];
        let mut d = doc(&rows.join("\r\n"));
        d.sort(&by(0, SortKind::Text, false)).unwrap();
        assert_eq!(
            col(&d, 0),
            [rows[2].clone(), rows[1].clone(), rows[0].clone()]
        );
        let nat = [
            format!("{}{}", "y".repeat(250), "1000000000"),
            format!("{}{}", "y".repeat(250), "99"),
        ];
        let mut d = doc(&nat.join("\r\n"));
        d.sort(&by(0, SortKind::Natural, false)).unwrap();
        assert_eq!(col(&d, 0), [nat[1].clone(), nat[0].clone()]);
    }

    #[test]
    fn sort_with_filter_and_save() {
        let dir = std::env::temp_dir().join(format!("nd-sort-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.csv");
        std::fs::write(&path, "k,v\r\nc,\"x, 1\"\r\na,2\r\nb,3\r\n").unwrap();
        let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let spec = crate::filter::FilterSpec {
            columns: vec![crate::filter::ColumnFilter {
                col: 0,
                rule: crate::filter::Rule::Values {
                    values: vec!["c".into(), "a".into()],
                    blanks: false,
                },
            }],
            keep_first: true,
            read: ReadOptions::default(),
        };
        d.set_filter(&spec).unwrap();
        let mut s = by(0, SortKind::Text, false);
        s.keep_first = true;
        d.sort(&s).unwrap();
        assert_eq!(col(&d, 0), ["k", "a", "c"]);
        d.save(crate::SaveOptions::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "k,v\r\na,2\r\nb,3\r\nc,\"x, 1\"\r\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        /// The engine's order equals a plain stable sort by the same rules,
        /// and undoing restores every row.
        #[test]
        fn matches_reference(vals in proptest::collection::vec("(|[a-cA-C]{0,3}[0-9]{0,2}|-?[0-9]{1,3}(\\.[0-9])?)", 0..60), desc in any::<bool>(), kind in 0u8..4) {
            let body: String = vals.iter().map(|v| format!("{v}\r\n")).collect();
            let mut d = doc(&body);
            let before = col(&d, 0);
            let kind = [SortKind::Text, SortKind::Natural, SortKind::Number, SortKind::Auto][kind as usize];
            d.sort(&by(0, kind, desc)).unwrap();
            let got = col(&d, 0);
            let mut want = before.clone();
            let mode = match kind {
                SortKind::Text => Mode::Text,
                SortKind::Natural => Mode::Natural,
                SortKind::Number => Mode::Number,
                _ => d.auto_mode(Some(0), 0, 0..d.total_rows(), &d.read_ctx(ReadOptions::default())),
            };
            let key = Key { phys: Some(0), ty: ColumnType::Auto, mode, desc, case: false, list: None };
            let plain = ReadCtx::plain(ReadOptions::default());
            let class = |s: &str| { let mut t = String::new(); let (c, n) = key.read(s, &plain, &mut t); (c, n, t) };
            want.sort_by(|a, b| {
                let (ca, na, ta) = class(a);
                let (cb, nb, tb) = class(b);
                if ca == BLANK || cb == BLANK { return ca.cmp(&cb); }
                let o = if ca != cb { ca.cmp(&cb) } else if ca == NUM { na.total_cmp(&nb) } else { key.text_cmp(ta.as_bytes(), tb.as_bytes()) };
                if desc { o.reverse() } else { o }
            });
            prop_assert_eq!(got, want);
            while d.undo() {}
            prop_assert_eq!(col(&d, 0), before);
        }
    }
}
