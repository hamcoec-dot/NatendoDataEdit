//! Looking for outside changes to the open files, safely: each look runs on
//! its own thread, a round waits at most two seconds, and a file whose look
//! has not come back (a share that stopped answering) is not asked again
//! until it does.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use natendo_engine::disk::{DiskMark, Outside};
use natendo_engine::source::FileStamp;
use serde::Serialize;

/// Files looked at in one round at most.
pub const MAX_WATCHED: usize = 64;
/// Longest a round waits.
pub const ROUND_WAIT: Duration = Duration::from_secs(2);
/// A missing file is looked for once more after this long: programs that save
/// by renaming a new file over the old one leave it missing for a moment.
const GONE_AGAIN: Duration = Duration::from_secs(1);

/// What a look found about one tab's file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiskEvent {
    pub doc: u32,
    /// "same", "changed" or "gone".
    pub state: &'static str,
    /// Changes when the file changes again.
    pub token: String,
    /// When the file was written (ms since 1970), for a changed file.
    pub modified: Option<u64>,
}

type Answer = (u32, DiskMark, Outside);
/// The stamp a look saw and its answer.
type Seen = (FileStamp, Outside);
type Look = Arc<dyn Fn(&DiskMark, Option<Seen>) -> Outside + Send + Sync>;

struct State {
    /// Tabs whose look has not come back.
    waiting: HashSet<u32>,
    /// Answers from every round, late ones too.
    answers: Receiver<Answer>,
    send: Sender<Answer>,
    /// What the last look at each tab's mark saw, so an unchanged file is
    /// not read again.
    seen: HashMap<u32, (DiskMark, Seen)>,
    /// Where the next round starts when there are more files than one asks.
    next: usize,
}

pub struct DiskWatch {
    state: Mutex<State>,
    look: Look,
    wait: Duration,
    gone_again: Duration,
}

impl Default for DiskWatch {
    fn default() -> Self {
        DiskWatch::new(Arc::new(DiskMark::look_known), ROUND_WAIT, GONE_AGAIN)
    }
}

impl DiskWatch {
    fn new(look: Look, wait: Duration, gone_again: Duration) -> DiskWatch {
        let (send, answers) = channel();
        DiskWatch {
            state: Mutex::new(State {
                waiting: HashSet::new(),
                answers,
                send,
                seen: HashMap::new(),
                next: 0,
            }),
            look,
            wait,
            gone_again,
        }
    }

    /// One round. `open` lists the open tabs with their marks (`None`: no
    /// file, or busy being changed or saved). `current` takes a tab's mark
    /// again when its answer arrives: if it moved (the app wrote the file
    /// meanwhile) or cannot be had (a save is running), the answer is
    /// dropped, so the app never warns about its own write.
    pub fn round(
        &self,
        open: Vec<(u32, Option<DiskMark>)>,
        current: impl Fn(u32) -> Option<DiskMark>,
    ) -> Vec<DiskEvent> {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let ids: HashSet<u32> = open.iter().map(|(id, _)| *id).collect();
        // A closed tab is forgotten even if its look never comes back.
        s.waiting.retain(|id| ids.contains(id));
        s.seen.retain(|id, _| ids.contains(id));
        let mut found: Vec<Answer> = Vec::new();
        while let Ok(a) = s.answers.try_recv() {
            s.waiting.remove(&a.0);
            found.push(a);
        }
        let mut asked: HashSet<u32> = HashSet::new();
        let count = open.len();
        let start = if count > MAX_WATCHED {
            s.next % count
        } else {
            0
        };
        for k in 0..count {
            let (id, mark) = &open[(start + k) % count];
            let Some(mark) = mark else { continue };
            if s.waiting.contains(id) || found.iter().any(|a| a.0 == *id) {
                continue;
            }
            if asked.len() == MAX_WATCHED {
                s.next = (start + k) % count;
                break;
            }
            // Only what was seen against this very mark counts.
            let last = s
                .seen
                .get(id)
                .filter(|(m, _)| m == mark)
                .map(|(_, seen)| *seen);
            let (id, mark, look, send) = (*id, mark.clone(), self.look.clone(), s.send.clone());
            let gone_again = self.gone_again;
            let started = std::thread::Builder::new()
                .name("disk-look".into())
                .spawn(move || {
                    let mut outside = look(&mark, last);
                    if outside == Outside::Gone {
                        std::thread::sleep(gone_again);
                        outside = look(&mark, last);
                    }
                    let _ = send.send((id, mark, outside));
                });
            if started.is_ok() {
                s.waiting.insert(id);
                asked.insert(id);
            }
        }
        let end = Instant::now() + self.wait;
        while !asked.is_empty() {
            let left = end.saturating_duration_since(Instant::now());
            let Ok(a) = s.answers.recv_timeout(left) else {
                break;
            };
            s.waiting.remove(&a.0);
            asked.remove(&a.0);
            found.push(a);
        }
        let mut events = Vec::new();
        for (doc, mark, outside) in found {
            if !ids.contains(&doc) || current(doc).as_ref() != Some(&mark) {
                continue;
            }
            let event = |state, token: String, modified| DiskEvent {
                doc,
                state,
                token,
                modified,
            };
            match outside {
                Outside::Touched(stamp) | Outside::Changed(stamp) => {
                    s.seen.insert(doc, (mark, (stamp, outside)));
                }
                Outside::Gone => {
                    s.seen.remove(&doc);
                }
                Outside::Same => {}
            }
            events.push(match outside {
                Outside::Same | Outside::Touched(_) => event("same", String::new(), None),
                Outside::Changed(stamp) => event("changed", stamp.token(), stamp.modified_ms()),
                Outside::Gone => event("gone", "gone".into(), None),
            });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use natendo_engine::{Document, OpenOptions};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn table(name: &str) -> (std::path::PathBuf, Document) {
        let p = std::env::temp_dir().join(format!("nd-watch-{name}-{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\r\n1,2\r\n").unwrap();
        let d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
        (p, d)
    }

    #[test]
    fn changed_gone_and_same() {
        let (p, d) = table("states");
        let mark = d.disk_mark().unwrap();
        let w = DiskWatch::default();
        let round = || {
            w.round(vec![(1, Some(mark.clone())), (2, None)], |_| {
                Some(mark.clone())
            })
        };
        assert_eq!(round()[0].state, "same");
        std::fs::write(&p, "a,b\r\n1,2\r\n3,4\r\n").unwrap();
        let e = round();
        assert_eq!((e.len(), e[0].doc, e[0].state), (1, 1, "changed"));
        assert!(e[0].modified.is_some() && !e[0].token.is_empty());
        std::fs::remove_file(&p).unwrap();
        assert_eq!(round()[0].state, "gone");
    }

    /// A save between the look and its answer, or still running, never warns.
    #[test]
    fn the_apps_own_write_never_warns() {
        let (p, mut d) = table("own");
        let before = d.disk_mark().unwrap();
        d.set_cell(1, 0, "999".into()).unwrap();
        d.save(Default::default()).unwrap();
        let after = d.disk_mark().unwrap();
        let w = DiskWatch::default();
        // The look was taken before the save; the answer says "changed".
        assert!(matches!(before.look(), Outside::Changed(_)));
        assert!(w
            .round(vec![(1, Some(before.clone()))], |_| Some(after.clone()))
            .is_empty());
        // A save is running: the mark cannot be taken again.
        assert!(w.round(vec![(1, Some(before))], |_| None).is_empty());
        let e = w.round(vec![(1, Some(after.clone()))], |_| Some(after.clone()));
        assert_eq!(e[0].state, "same");
        let _ = std::fs::remove_file(p);
    }

    /// A look that does not come back: the round gives up, the file is not
    /// asked again until it does, only one thread exists for it, and its late
    /// answer is used.
    #[test]
    fn a_stuck_look_is_left_alone() {
        let (p, d) = table("stuck");
        let mark = d.disk_mark().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let (release, held) = channel::<()>();
        let held = Mutex::new(held);
        let counted = calls.clone();
        let w = DiskWatch::new(
            Arc::new(move |_: &DiskMark, _| {
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    let _ = held.lock().unwrap().recv();
                    return Outside::Changed(FileStamp {
                        len: 1,
                        modified: None,
                    });
                }
                Outside::Same
            }),
            Duration::from_millis(200),
            Duration::ZERO,
        );
        let round = |ids: &[u32]| {
            w.round(
                ids.iter().map(|&i| (i, Some(mark.clone()))).collect(),
                |_| Some(mark.clone()),
            )
        };
        let t = Instant::now();
        assert!(round(&[1]).is_empty());
        assert!(t.elapsed() < Duration::from_secs(2));
        // Skipped while stuck; another tab is still looked at.
        let e = round(&[1, 2]);
        assert_eq!((e.len(), e[0].doc), (1, 2));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        release.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let e = round(&[1]);
        assert_eq!((e.len(), e[0].state), (1, "changed"));
        // Asked again afterwards.
        assert_eq!(round(&[1])[0].state, "same");
        let _ = std::fs::remove_file(p);
    }

    /// What a look saw is handed to the next look at the same mark only.
    #[test]
    fn the_last_answer_is_remembered_for_the_same_mark() {
        let (p, mut d) = table("seen");
        let mark = d.disk_mark().unwrap();
        let stamp = FileStamp {
            len: 9,
            modified: None,
        };
        let given = Arc::new(Mutex::new(Vec::new()));
        let log = given.clone();
        let w = DiskWatch::new(
            Arc::new(move |_: &DiskMark, last: Option<Seen>| {
                log.lock().unwrap().push(last);
                Outside::Touched(stamp)
            }),
            ROUND_WAIT,
            Duration::ZERO,
        );
        for _ in 0..2 {
            let e = w.round(vec![(1, Some(mark.clone()))], |_| Some(mark.clone()));
            assert_eq!(e[0].state, "same");
        }
        // The app saves: the old answer no longer applies.
        d.set_cell(1, 0, "999".into()).unwrap();
        d.save(Default::default()).unwrap();
        let after = d.disk_mark().unwrap();
        w.round(vec![(1, Some(after.clone()))], |_| Some(after.clone()));
        assert_eq!(
            *given.lock().unwrap(),
            [None, Some((stamp, Outside::Touched(stamp))), None]
        );
        let _ = std::fs::remove_file(p);
    }

    /// A file missing for a moment (saved by renaming) is not reported gone.
    #[test]
    fn gone_is_looked_at_twice() {
        let (p, d) = table("twice");
        let mark = d.disk_mark().unwrap();
        for (second, state) in [(Outside::Same, "same"), (Outside::Gone, "gone")] {
            let calls = Arc::new(AtomicUsize::new(0));
            let counted = calls.clone();
            let w = DiskWatch::new(
                Arc::new(move |_: &DiskMark, _| {
                    if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                        Outside::Gone
                    } else {
                        second
                    }
                }),
                ROUND_WAIT,
                Duration::from_millis(50),
            );
            let t = Instant::now();
            let e = w.round(vec![(1, Some(mark.clone()))], |_| Some(mark.clone()));
            assert!(t.elapsed() >= Duration::from_millis(50));
            assert_eq!((e[0].state, calls.load(Ordering::SeqCst)), (state, 2));
        }
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn a_closed_tab_is_forgotten_and_many_files_take_turns() {
        let (p, d) = table("many");
        let mark = d.disk_mark().unwrap();
        let (_release, held) = channel::<()>();
        let held = Mutex::new(held);
        let w = DiskWatch::new(
            Arc::new(move |_: &DiskMark, _| {
                let _ = held.lock().unwrap().recv_timeout(Duration::from_secs(5));
                Outside::Same
            }),
            Duration::from_millis(10),
            Duration::ZERO,
        );
        w.round(vec![(7, Some(mark.clone()))], |_| Some(mark.clone()));
        assert!(w.state.lock().unwrap().waiting.contains(&7));
        w.round(Vec::new(), |_| None);
        assert!(w.state.lock().unwrap().waiting.is_empty());
        let quick = DiskWatch::new(
            Arc::new(|_: &DiskMark, _| Outside::Same),
            ROUND_WAIT,
            Duration::ZERO,
        );
        let all: Vec<_> = (0..100).map(|i| (i, Some(mark.clone()))).collect();
        let mut seen = HashSet::new();
        for _ in 0..2 {
            let e = quick.round(all.clone(), |_| Some(mark.clone()));
            assert_eq!(e.len(), MAX_WATCHED);
            seen.extend(e.iter().map(|e| e.doc));
        }
        assert_eq!(seen.len(), 100);
        let _ = std::fs::remove_file(p);
    }
}
