//! A second window (version 1: one extra window, labelled "w2") and where each
//! open file lives.
//!
//! Documents and their undo history stay in `Docs`, shared by every window; a
//! window only shows tabs. A tab is in exactly one window. Rust keeps the truth
//! about which window holds which file (the owner map), so a file can never end
//! up in no window: when window 2 closes, is destroyed, stops responding, or
//! never takes a move, its files go back to the main window by themselves,
//! whatever its page is doing.
//!
//! Everything here is in memory only, and never logged: the hand-over holds a
//! tab's view (column widths, selection and the like), and `Debug` for it says
//! nothing about the content.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// The first window's label.
pub const MAIN: &str = "main";
/// The one extra window's label.
pub const SECOND: &str = "w2";
/// A move window 2 has not taken within this long goes back to main.
pub const TAKE_WITHIN: Duration = Duration::from_secs(10);
/// Window 2 owns files but has not reported for this long: it is gone.
pub const BEAT_WITHIN: Duration = Duration::from_secs(12);

/// A tab on its way between windows (or its last known state, kept so a file
/// window 2 held can come back as it was).
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct HandOver {
    pub doc: u32,
    /// The tab's look and place, built and read by the window's page.
    #[serde(default)]
    pub state: serde_json::Value,
    /// The tab as a "reopen last time" view (layout only), for the session.
    #[serde(default)]
    pub view: Option<serde_json::Value>,
    #[serde(default)]
    pub row: u64,
    #[serde(default)]
    pub col: u32,
    /// Why the tab came back on its own (shown to the person), if it did.
    #[serde(default)]
    pub note: Option<String>,
}

impl fmt::Debug for HandOver {
    /// Nothing but the file's number: the view and place are never printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HandOver(doc {}, <not shown>)", self.doc)
    }
}

#[derive(Default)]
struct State {
    /// File → window label. A file not listed is in the main window.
    owner: HashMap<u32, String>,
    /// Hand-overs waiting for a window to take them.
    queue: HashMap<String, Vec<HandOver>>,
    /// Moves to window 2 not taken yet, and since when.
    pending: HashMap<u32, Instant>,
    /// The last state window 2 reported for each file it holds.
    last: HashMap<u32, HandOver>,
    /// When window 2 last took its queue or reported.
    beat: Option<Instant>,
}

/// What `expire` found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Expired {
    /// Files whose move was never taken: back in main.
    pub rolled_back: Vec<u32>,
    /// Window 2 stopped reporting: its files are back in main and the window
    /// should be destroyed.
    pub second_dead: bool,
}

#[derive(Default)]
pub struct Windows(Mutex<State>);

impl Windows {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Move a file to window 2: it is window 2's from now on (until taken it
    /// can still roll back).
    pub fn move_to_second(&self, h: HandOver, now: Instant) {
        let mut s = self.lock();
        s.owner.insert(h.doc, SECOND.to_string());
        s.pending.insert(h.doc, now);
        s.last.insert(h.doc, h.clone());
        s.queue.entry(SECOND.to_string()).or_default().push(h);
        if s.beat.is_none() {
            s.beat = Some(now);
        }
    }

    /// A window takes what is waiting for it (a taken move is confirmed).
    pub fn take(&self, label: &str, now: Instant) -> Vec<HandOver> {
        let mut s = self.lock();
        let items = s.queue.remove(label).unwrap_or_default();
        if label == SECOND {
            for h in &items {
                s.pending.remove(&h.doc);
            }
            s.beat = Some(now);
        }
        items
    }

    /// Window 2 made a document itself (a result tab): it is window 2's.
    pub fn claim(&self, h: HandOver, now: Instant) {
        let mut s = self.lock();
        s.owner.insert(h.doc, SECOND.to_string());
        s.last.insert(h.doc, h);
        s.beat = Some(now);
    }

    /// A file goes back to the main window.
    pub fn move_back(&self, h: HandOver) {
        let mut s = self.lock();
        s.owner.remove(&h.doc);
        s.pending.remove(&h.doc);
        s.last.remove(&h.doc);
        if let Some(q) = s.queue.get_mut(SECOND) {
            q.retain(|x| x.doc != h.doc);
        }
        s.queue.entry(MAIN.to_string()).or_default().push(h);
    }

    /// Window 2 reports the state of the files it holds (and that it is alive).
    pub fn sync(&self, items: Vec<HandOver>, now: Instant) {
        let mut s = self.lock();
        for h in items {
            if s.owner.get(&h.doc).map(String::as_str) == Some(SECOND) {
                s.last.insert(h.doc, h);
            }
        }
        s.beat = Some(now);
    }

    /// Every file window 2 holds goes back to main, as it was last reported.
    /// Window 2's page is not asked: it may be gone. Returns the files.
    pub fn release_second(&self, note: Option<&str>) -> Vec<u32> {
        let mut s = self.lock();
        let mut docs: Vec<u32> = s
            .owner
            .iter()
            .filter(|(_, l)| l.as_str() == SECOND)
            .map(|(d, _)| *d)
            .collect();
        docs.sort_unstable();
        for d in &docs {
            let mut h = s.last.remove(d).unwrap_or(HandOver {
                doc: *d,
                ..Default::default()
            });
            h.note = note.map(str::to_string);
            s.owner.remove(d);
            s.pending.remove(d);
            s.queue.entry(MAIN.to_string()).or_default().push(h);
        }
        s.queue.remove(SECOND);
        s.beat = None;
        docs
    }

    /// Moves never taken, and a window 2 that stopped reporting.
    pub fn expire(&self, now: Instant) -> Expired {
        let stale: Vec<u32> = {
            let s = self.lock();
            let mut v: Vec<u32> = s
                .pending
                .iter()
                .filter(|(_, since)| now.duration_since(**since) >= TAKE_WITHIN)
                .map(|(d, _)| *d)
                .collect();
            v.sort_unstable();
            v
        };
        let mut out = Expired::default();
        for d in stale {
            let mut s = self.lock();
            let mut h = s.last.remove(&d).unwrap_or(HandOver {
                doc: d,
                ..Default::default()
            });
            h.note = Some("The second window did not respond, so the tab is back here.".into());
            s.owner.remove(&d);
            s.pending.remove(&d);
            if let Some(q) = s.queue.get_mut(SECOND) {
                q.retain(|x| x.doc != d);
            }
            s.queue.entry(MAIN.to_string()).or_default().push(h);
            out.rolled_back.push(d);
        }
        let dead = {
            let s = self.lock();
            let holds = s.owner.values().any(|l| l == SECOND);
            holds && s.beat.is_some_and(|b| now.duration_since(b) >= BEAT_WITHIN)
        };
        if dead {
            let back = self.release_second(Some(
                "The second window stopped responding, so its tabs are back here.",
            ));
            out.rolled_back.extend(back);
            out.second_dead = true;
        }
        out
    }

    /// The files window 2 holds, with their last reported state (for the
    /// "reopen last time" list, which lists them as ordinary tabs).
    pub fn owned_by_second(&self) -> Vec<HandOver> {
        let s = self.lock();
        let mut v: Vec<HandOver> = s
            .owner
            .iter()
            .filter(|(_, l)| l.as_str() == SECOND)
            .map(|(d, _)| {
                s.last.get(d).cloned().unwrap_or(HandOver {
                    doc: *d,
                    ..Default::default()
                })
            })
            .collect();
        v.sort_by_key(|h| h.doc);
        v
    }

    /// Which window a file is in.
    #[cfg(test)]
    pub fn owner_of(&self, doc: u32) -> String {
        self.lock()
            .owner
            .get(&doc)
            .cloned()
            .unwrap_or_else(|| MAIN.to_string())
    }

    /// Does window 2 hold any file (or wait for one)?
    pub fn second_has_files(&self) -> bool {
        self.lock().owner.values().any(|l| l == SECOND)
    }

    /// A file was closed: it is no one's now.
    pub fn forget(&self, doc: u32) {
        let mut s = self.lock();
        s.owner.remove(&doc);
        s.pending.remove(&doc);
        s.last.remove(&doc);
        for q in s.queue.values_mut() {
            q.retain(|x| x.doc != doc);
        }
    }
}

/// Which window gets a "file opened" event: the main window while there is
/// one, otherwise any window that is open (never nobody).
pub fn event_target(open: &[String]) -> Option<String> {
    if open.iter().any(|l| l == MAIN) {
        Some(MAIN.to_string())
    } else {
        open.iter().min().cloned()
    }
}

/// The labels of the windows open now.
pub fn open_labels(app: &AppHandle) -> Vec<String> {
    app.webview_windows().keys().cloned().collect()
}

/// Open window 2 (the same page, in second-window mode) unless it is open.
pub fn ensure_second(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(SECOND).is_some() {
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        app,
        SECOND,
        tauri::WebviewUrl::App("index.html?window=2".into()),
    )
    .title("Natendo Data Edit (window 2)")
    .inner_size(1100.0, 760.0)
    .build()
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Tell the window that gets hand-overs (main) that something is waiting.
pub fn tell_main(app: &AppHandle) {
    if let Some(label) = event_target(&open_labels(app)) {
        let _ = app.emit_to(label, "handover", ());
    }
}

/// Destroy window 2 a moment from now (a command that asked for it can answer
/// first).
fn destroy_second_soon(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(w) = app.get_webview_window(SECOND) {
            let _ = w.destroy();
        }
    });
}

/// Window 2 and its files: the files go back to main (as last reported), the
/// window is destroyed. Safe to call again, from any thread; it never waits
/// for window 2's page.
pub fn close_second(app: &AppHandle, note: Option<&str>) {
    app.state::<Windows>().release_second(note);
    if let Some(w) = app.get_webview_window(SECOND) {
        let _ = w.destroy();
    }
    tell_main(app);
}

/// Window 2's close button: ask its page for its latest views and to hand the
/// tabs back, but never depend on it: after two seconds Rust does it.
pub fn begin_close_second(app: &AppHandle) {
    let _ = app.emit_to(SECOND, "close-requested", ());
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        if app.get_webview_window(SECOND).is_some() {
            close_second(&app, None);
        }
    });
}

/// Window 2 holds nothing (a tab moved back, or closed): close it.
pub fn maybe_close_second(app: &AppHandle) {
    if app.get_webview_window(SECOND).is_some() && !app.state::<Windows>().second_has_files() {
        destroy_second_soon(app);
    }
}

/// Watch for a move window 2 never took and for a window 2 that stopped
/// responding (a crashed page fires no event): either way the files come
/// back to main.
pub fn watchdog(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        let e = app.state::<Windows>().expire(Instant::now());
        if e.second_dead {
            if let Some(w) = app.get_webview_window(SECOND) {
                let _ = w.destroy();
            }
        }
        if !e.rolled_back.is_empty() || e.second_dead {
            tell_main(&app);
            maybe_close_second(&app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(doc: u32) -> HandOver {
        HandOver {
            doc,
            state: serde_json::json!({ "secret": "cell text" }),
            row: 3,
            col: 1,
            ..Default::default()
        }
    }

    fn at(s: u64) -> Instant {
        // A fixed starting point plus `s` seconds.
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *START.get_or_init(Instant::now) + Duration::from_secs(s)
    }

    #[test]
    fn a_file_is_in_main_until_moved_and_back_after() {
        let w = Windows::default();
        assert_eq!(w.owner_of(1), MAIN);
        w.move_to_second(h(1), at(0));
        assert_eq!(w.owner_of(1), SECOND);
        assert!(w.second_has_files());
        let taken = w.take(SECOND, at(1));
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].doc, 1);
        // Taken once only.
        assert!(w.take(SECOND, at(1)).is_empty());
        w.move_back(h(1));
        assert_eq!(w.owner_of(1), MAIN);
        assert!(!w.second_has_files());
        assert_eq!(w.take(MAIN, at(2)).len(), 1);
    }

    #[test]
    fn the_queue_returns_items_in_order() {
        let w = Windows::default();
        w.move_to_second(h(5), at(0));
        w.move_to_second(h(2), at(0));
        let docs: Vec<u32> = w.take(SECOND, at(1)).iter().map(|x| x.doc).collect();
        assert_eq!(docs, vec![5, 2]);
    }

    #[test]
    fn closing_window_two_returns_its_files_without_its_page() {
        let w = Windows::default();
        w.move_to_second(h(1), at(0));
        w.move_to_second(h(2), at(0));
        w.take(SECOND, at(1));
        // Window 2 reported its latest state for file 1 only.
        let mut latest = h(1);
        latest.row = 99;
        w.sync(vec![latest], at(2));
        let docs = w.release_second(None);
        assert_eq!(docs, vec![1, 2]);
        assert!(!w.second_has_files());
        let back = w.take(MAIN, at(3));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].row, 99, "the last reported state comes back");
        assert_eq!(back[1].row, 3, "otherwise the state it was moved with");
        // Releasing again does nothing.
        assert!(w.release_second(None).is_empty());
    }

    #[test]
    fn a_move_never_taken_rolls_back() {
        let w = Windows::default();
        w.move_to_second(h(7), at(0));
        // Not yet.
        assert_eq!(w.expire(at(5)), Expired::default());
        assert_eq!(w.owner_of(7), SECOND);
        let e = w.expire(at(10));
        assert_eq!(e.rolled_back, vec![7]);
        assert!(!e.second_dead);
        assert_eq!(w.owner_of(7), MAIN);
        let back = w.take(MAIN, at(11));
        assert_eq!(back.len(), 1);
        assert!(back[0].note.is_some(), "the person is told why");
        // Window 2 finding the queue empty afterwards.
        assert!(w.take(SECOND, at(11)).is_empty());
    }

    #[test]
    fn a_taken_move_does_not_roll_back_but_silence_does() {
        let w = Windows::default();
        w.move_to_second(h(7), at(0));
        w.take(SECOND, at(1));
        // Reporting keeps it alive.
        w.sync(vec![h(7)], at(8));
        assert_eq!(w.expire(at(15)), Expired::default());
        // Silence past BEAT_WITHIN: window 2 crashed without a Destroyed.
        let e = w.expire(at(8 + 12));
        assert!(e.second_dead);
        assert_eq!(e.rolled_back, vec![7]);
        assert_eq!(w.owner_of(7), MAIN);
        assert_eq!(w.take(MAIN, at(21)).len(), 1);
        // No files, no beat: nothing to watch.
        assert_eq!(w.expire(at(100)), Expired::default());
    }

    #[test]
    fn session_lists_what_window_two_holds() {
        let w = Windows::default();
        w.move_to_second(h(4), at(0));
        w.move_to_second(h(3), at(0));
        let owned: Vec<u32> = w.owned_by_second().iter().map(|x| x.doc).collect();
        assert_eq!(owned, vec![3, 4]);
        w.forget(3);
        assert_eq!(w.owned_by_second().len(), 1);
        assert_eq!(w.owner_of(3), MAIN);
    }

    #[test]
    fn events_go_to_main_else_any_window_never_nobody() {
        let open = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(event_target(&open(&["w2", "main"])).as_deref(), Some(MAIN));
        assert_eq!(event_target(&open(&["w2"])).as_deref(), Some(SECOND));
        assert_eq!(event_target(&open(&[])), None);
    }

    #[test]
    fn the_hand_over_is_never_printed() {
        let text = format!("{:?}", h(9));
        assert!(!text.contains("cell text") && !text.contains("secret"));
        assert!(text.contains('9'));
    }
}
