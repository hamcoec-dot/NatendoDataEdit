//! Cells dropped on a tab in the other window (gap #6).
//!
//! A drop is begun by the window the cells come from and taken by the window
//! that holds the tab they go to. The shell keeps each drop here between the
//! two: which document and blocks, which document they go to, and by when.
//! The cells themselves never travel between the windows: the target window
//! names the cell they go in at, and the shell reads the source and pastes
//! (in memory only; nothing here is logged or written).
//!
//! Every drop has a deadline. The target can take it only before then; the
//! source waits past it, and whatever it then reads is what happened, so its
//! message cannot disagree with the data: a drop not taken in time is marked
//! too late at once, and a late take is refused.
//!
//! Also here: each window's tab strip (rects of its tabs), kept so a drag out
//! of one window can find the tab under the pointer in the other.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How long the target window has to take a drop.
pub const DEADLINE: Duration = Duration::from_secs(10);
/// How long past the deadline the source waits for an answer that was taken
/// in time (a big paste can take a while).
pub const TAKEN_GRACE: Duration = Duration::from_secs(300);

/// A block of cells, in shown positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Area {
    pub row: u64,
    pub rows: u64,
    pub col: u32,
    pub cols: u32,
}

/// What the target window is told (no cell values).
#[derive(Debug, Clone, Serialize)]
pub struct DropNotice {
    pub id: u64,
    pub doc: u32,
    pub from_doc: u32,
    pub from_name: String,
    /// Rows and columns of the packed block that will go in.
    pub rows: u64,
    pub cols: u64,
    pub moving: bool,
    /// Milliseconds left to take it.
    pub within_ms: u64,
}

/// How a drop ended, as the source window reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// The cells are in the target: `rows` x `width` written, `added` rows added.
    Done { rows: u64, width: u64, added: u64 },
    /// The target said no (read-only, busy, Cancel, private): nothing written.
    Refused { message: String },
    /// Not taken before the deadline: nothing written, and it never will be.
    TooLate,
}

#[derive(Debug)]
enum State {
    Waiting,
    Taken,
    Ended(Outcome),
}

/// What the shell needs to read the source when the target takes the drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub doc: u32,
    pub dst: u32,
    pub areas: Vec<Area>,
    /// Blocks side by side (same rows) rather than one under the other.
    pub side_by_side: bool,
    /// Hidden columns of the source (left out of the copy).
    pub hidden: Vec<u32>,
}

#[derive(Debug)]
struct Drop {
    source: Source,
    deadline: Instant,
    state: State,
}

#[derive(Default)]
struct Inner {
    next: u64,
    drops: HashMap<u64, Drop>,
    /// Window label → its tabs' rects in its own viewport (CSS px).
    strips: HashMap<String, Vec<TabRect>>,
}

/// The drops in flight and the windows' tab strips.
#[derive(Default)]
pub struct Drops {
    inner: Mutex<Inner>,
    changed: Condvar,
}

/// Why the blocks cannot go anywhere as one packed block, or `None`.
pub fn line_up(areas: &[Area]) -> Result<bool, &'static str> {
    let Some(first) = areas.first() else {
        return Err("nothing is selected");
    };
    let same_rows = areas
        .iter()
        .all(|a| a.row == first.row && a.rows == first.rows);
    let same_cols = areas
        .iter()
        .all(|a| a.col == first.col && a.cols == first.cols);
    if areas.len() > 1 && !same_rows && !same_cols {
        return Err("dragging several blocks needs them to line up: all with the same rows (side by side) or all with the same columns (one under the other)");
    }
    Ok(same_rows)
}

/// Rows and columns of the packed block.
pub fn packed_size(areas: &[Area], side_by_side: bool) -> (u64, u64) {
    if side_by_side {
        (
            areas.first().map_or(0, |a| a.rows),
            areas.iter().map(|a| u64::from(a.cols)).sum(),
        )
    } else {
        (
            areas.iter().map(|a| a.rows).sum(),
            areas.first().map_or(0, |a| u64::from(a.cols)),
        )
    }
}

impl Drops {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Note a drop; its id.
    pub fn begin(&self, source: Source, now: Instant) -> u64 {
        let mut s = self.lock();
        s.next += 1;
        let id = s.next;
        s.drops.insert(
            id,
            Drop {
                source,
                deadline: now + DEADLINE,
                state: State::Waiting,
            },
        );
        id
    }

    /// The target window takes drop `id` to paste it into `dst`: only once,
    /// only before the deadline. Refused (with why) otherwise.
    pub fn take(&self, id: u64, dst: u32, now: Instant) -> Result<Source, String> {
        let mut s = self.lock();
        let Some(d) = s.drops.get_mut(&id) else {
            return Err("that drop is no longer waiting".into());
        };
        if d.source.dst != dst {
            return Err("that drop is for another file".into());
        }
        match d.state {
            State::Waiting if now < d.deadline => {
                d.state = State::Taken;
                Ok(d.source.clone())
            }
            State::Waiting => {
                d.state = State::Ended(Outcome::TooLate);
                self.changed.notify_all();
                Err("the drop came too late, so nothing was put here".into())
            }
            _ => Err("that drop is no longer waiting".into()),
        }
    }

    /// The paste of a taken drop finished (or failed).
    pub fn finish(&self, id: u64, outcome: Outcome) {
        let mut s = self.lock();
        if let Some(d) = s.drops.get_mut(&id) {
            if matches!(d.state, State::Taken) {
                d.state = State::Ended(outcome);
                self.changed.notify_all();
            }
        }
    }

    /// The target window says no before taking it.
    pub fn refuse(&self, id: u64, message: String) {
        let mut s = self.lock();
        if let Some(d) = s.drops.get_mut(&id) {
            if matches!(d.state, State::Waiting) {
                d.state = State::Ended(Outcome::Refused { message });
                self.changed.notify_all();
            }
        }
    }

    /// The source waits for the end of drop `id` (and forgets it). Not taken
    /// by the deadline: too late, at once and for good.
    pub fn wait(&self, id: u64) -> Outcome {
        let mut s = self.lock();
        loop {
            let now = Instant::now();
            let Some(d) = s.drops.get_mut(&id) else {
                return Outcome::TooLate;
            };
            match &d.state {
                State::Ended(o) => {
                    let o = o.clone();
                    s.drops.remove(&id);
                    return o;
                }
                State::Waiting if now >= d.deadline => {
                    s.drops.remove(&id);
                    return Outcome::TooLate;
                }
                State::Taken if now >= d.deadline + TAKEN_GRACE => {
                    // Still pasting after minutes: keep the record so the
                    // paste can still finish, and say so plainly.
                    return Outcome::Refused {
                        message: "the other window is still busy with the drop; check that file before trying again".into(),
                    };
                }
                _ => {}
            }
            let until = match d.state {
                State::Waiting => d.deadline,
                _ => d.deadline + TAKEN_GRACE,
            };
            let left = until
                .saturating_duration_since(now)
                .max(Duration::from_millis(1));
            s = self
                .changed
                .wait_timeout(s, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// Window `label` reports its tab strip (empty: hidden, minimised or
    /// closed).
    pub fn set_strip(&self, label: &str, rects: Vec<TabRect>) {
        let mut s = self.lock();
        if rects.is_empty() {
            s.strips.remove(label);
        } else {
            s.strips.insert(label.to_string(), rects);
        }
    }

    pub fn strip(&self, label: &str) -> Vec<TabRect> {
        self.lock().strips.get(label).cloned().unwrap_or_default()
    }
}

/// One tab's place in its window's viewport (CSS px).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TabRect {
    pub doc: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where a window is on the screen (physical px) and its scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geom {
    /// The whole window, frame included.
    pub outer: (f64, f64, f64, f64),
    /// The viewport's top-left corner.
    pub inner: (f64, f64),
    pub scale: f64,
    pub visible: bool,
    pub minimized: bool,
}

/// The tab of the other window under `(x, y)`, a point in the source
/// window's viewport (CSS px, possibly outside it). Only when the point is
/// outside the source window altogether and the other window is shown: a
/// window behind the source is never a target.
pub fn hit(src: &Geom, x: f64, y: f64, other: &Geom, strip: &[TabRect]) -> Option<u32> {
    if !other.visible || other.minimized {
        return None;
    }
    let sx = src.inner.0 + x * src.scale;
    let sy = src.inner.1 + y * src.scale;
    let (ox, oy, ow, oh) = src.outer;
    if sx >= ox && sx < ox + ow && sy >= oy && sy < oy + oh {
        return None;
    }
    let cx = (sx - other.inner.0) / other.scale;
    let cy = (sy - other.inner.1) / other.scale;
    strip
        .iter()
        .find(|r| cx >= r.x && cx < r.x + r.w && cy >= r.y && cy < r.y + r.h)
        .map(|r| r.doc)
}

/// The drop's blocks as one tab-separated text, packed as Copy packs them
/// (side by side, or one under the other), hidden columns left out. Read and
/// handed straight to the paste, in memory only.
pub fn read_packed(docs: &crate::docs::Docs, s: &Source) -> crate::docs::CmdResult<String> {
    if let [a] = s.areas.as_slice() {
        // One block: the copy's own text, as Copy gives it.
        return docs.copy_skipping(s.doc, a.row, a.rows, a.col, a.cols, &s.hidden);
    }
    let mut parts = Vec::with_capacity(s.areas.len());
    for a in &s.areas {
        let text = docs.copy_skipping(s.doc, a.row, a.rows, a.col, a.cols, &s.hidden)?;
        parts.push(natendo_engine::ops::parse_tsv(
            &text,
            natendo_engine::ops::MAX_BLOCK_CELLS,
        )?);
    }
    let rows: Vec<Vec<String>> = if s.side_by_side {
        let n = parts.iter().map(Vec::len).max().unwrap_or(0);
        (0..n)
            .map(|i| {
                parts
                    .iter()
                    .flat_map(|p| p.get(i).cloned().unwrap_or_default())
                    .collect()
            })
            .collect()
    } else {
        parts.into_iter().flatten().collect()
    };
    Ok(to_tsv(&rows))
}

/// Rows as clipboard text: tabs between cells, CRLF between rows, `"`
/// quoting for cells holding a tab, quote or line break.
pub fn to_tsv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i > 0 {
                out.push('\t');
            }
            if c.contains(['\t', '"', '\r', '\n']) {
                out.push('"');
                out.push_str(&c.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(c);
            }
        }
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> Source {
        Source {
            doc: 1,
            dst: 2,
            areas: vec![Area {
                row: 1,
                rows: 2,
                col: 0,
                cols: 2,
            }],
            side_by_side: true,
            hidden: vec![],
        }
    }

    #[test]
    fn a_drop_is_taken_once_and_its_end_is_read() {
        let d = Drops::default();
        let now = Instant::now();
        let id = d.begin(src(), now);
        assert!(d.take(id, 3, now).is_err(), "another file");
        assert!(d.take(id, 2, now).is_ok());
        assert!(d.take(id, 2, now).is_err(), "only once");
        d.finish(
            id,
            Outcome::Done {
                rows: 2,
                width: 2,
                added: 0,
            },
        );
        assert_eq!(
            d.wait(id),
            Outcome::Done {
                rows: 2,
                width: 2,
                added: 0
            }
        );
        assert!(d.take(id, 2, now).is_err(), "forgotten");
    }

    #[test]
    fn a_late_take_is_refused_and_the_source_hears_too_late() {
        let d = Drops::default();
        let then = Instant::now() - DEADLINE - Duration::from_millis(5);
        let id = d.begin(src(), then);
        let e = d.take(id, 2, Instant::now()).unwrap_err();
        assert!(e.contains("too late"), "{e}");
        assert_eq!(d.wait(id), Outcome::TooLate);
    }

    #[test]
    fn nothing_taken_by_the_deadline_is_too_late_for_good() {
        let d = Drops::default();
        // Begun so that its deadline is 50 ms away.
        let id = d.begin(src(), Instant::now() - DEADLINE + Duration::from_millis(50));
        let t = Instant::now();
        assert_eq!(d.wait(id), Outcome::TooLate);
        assert!(t.elapsed() >= Duration::from_millis(40));
        assert!(
            d.take(id, 2, Instant::now()).is_err(),
            "a take after that is refused"
        );
    }

    #[test]
    fn a_refusal_ends_it_and_a_taken_drop_waits_for_its_paste() {
        let d = std::sync::Arc::new(Drops::default());
        let id = d.begin(src(), Instant::now());
        d.refuse(id, "read-only".into());
        assert_eq!(
            d.wait(id),
            Outcome::Refused {
                message: "read-only".into()
            }
        );
        let id = d.begin(src(), Instant::now());
        d.take(id, 2, Instant::now()).unwrap();
        d.refuse(id, "too late to say no".into());
        let d2 = d.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            d2.finish(
                id,
                Outcome::Done {
                    rows: 1,
                    width: 1,
                    added: 1,
                },
            );
        });
        assert_eq!(
            d.wait(id),
            Outcome::Done {
                rows: 1,
                width: 1,
                added: 1
            }
        );
        t.join().unwrap();
    }

    #[test]
    fn blocks_line_up_or_are_refused() {
        let a = |row, rows, col, cols| Area {
            row,
            rows,
            col,
            cols,
        };
        assert_eq!(line_up(&[a(1, 3, 0, 1), a(1, 3, 4, 2)]), Ok(true));
        assert_eq!(line_up(&[a(1, 3, 0, 2), a(9, 2, 0, 2)]), Ok(false));
        assert!(line_up(&[a(1, 3, 0, 2), a(9, 2, 1, 2)]).is_err());
        assert_eq!(packed_size(&[a(1, 3, 0, 1), a(1, 3, 4, 2)], true), (3, 3));
        assert_eq!(packed_size(&[a(1, 3, 0, 2), a(9, 2, 0, 2)], false), (5, 2));
    }

    fn g(x: f64, y: f64, w: f64, h: f64, scale: f64) -> Geom {
        Geom {
            outer: (x, y, w, h),
            inner: (x + 8.0, y + 31.0 * scale),
            scale,
            visible: true,
            minimized: false,
        }
    }

    #[test]
    fn the_other_windows_tab_is_found_only_outside_the_source() {
        // Source at 0,0 (1000 x 800, scale 1); other on a second monitor to the
        // right at 1920,0 (scale 1.5), its tabs 40 px high at y 100.
        let s = g(0.0, 0.0, 1000.0, 800.0, 1.0);
        let o = g(1920.0, 0.0, 1500.0, 1200.0, 1.5);
        let strip = [
            TabRect {
                doc: 7,
                x: 0.0,
                y: 100.0,
                w: 160.0,
                h: 40.0,
            },
            TabRect {
                doc: 8,
                x: 160.0,
                y: 100.0,
                w: 160.0,
                h: 40.0,
            },
        ];
        // Over doc 8: other viewport (200, 110) -> screen 1928 + 300, 46.5 + 165.
        let (sx, sy) = (1928.0 + 200.0 * 1.5, 46.5 + 110.0 * 1.5);
        assert_eq!(hit(&s, sx - 8.0, sy - 31.0, &o, &strip), Some(8));
        // The same tab's place when it lies inside the source window: nothing.
        let inside = g(0.0, 0.0, 2600.0, 800.0, 1.0);
        assert_eq!(hit(&inside, sx - 8.0, sy - 31.0, &o, &strip), None);
        // Minimised or hidden: nothing.
        assert_eq!(
            hit(
                &s,
                sx - 8.0,
                sy - 31.0,
                &Geom {
                    minimized: true,
                    ..o
                },
                &strip
            ),
            None
        );
        assert_eq!(
            hit(
                &s,
                sx - 8.0,
                sy - 31.0,
                &Geom {
                    visible: false,
                    ..o
                },
                &strip
            ),
            None
        );
        // Between the tabs' rows: nothing.
        assert_eq!(hit(&s, sx - 8.0, sy - 31.0 + 100.0, &o, &strip), None);
        // A source at scale 1.5 too: its viewport px are scaled.
        let s15 = g(0.0, 0.0, 1000.0, 800.0, 1.5);
        assert_eq!(
            hit(&s15, (sx - 8.0) / 1.5, (sy - 46.5) / 1.5, &o, &strip),
            Some(8)
        );
    }

    #[test]
    fn packed_text_round_trips() {
        let rows = vec![
            vec!["a".to_string(), "b\tc".to_string()],
            vec!["say \"hi\"".to_string(), "two\nlines".to_string()],
        ];
        let text = to_tsv(&rows);
        assert_eq!(natendo_engine::ops::parse_tsv(&text, 100).unwrap(), rows);
    }

    #[test]
    fn a_strip_is_forgotten_when_its_window_hides() {
        let d = Drops::default();
        d.set_strip(
            "w2",
            vec![TabRect {
                doc: 1,
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            }],
        );
        assert_eq!(d.strip("w2").len(), 1);
        d.set_strip("w2", vec![]);
        assert!(d.strip("w2").is_empty());
    }
}
