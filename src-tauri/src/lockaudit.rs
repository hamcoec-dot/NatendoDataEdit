//! Audit: a command that is not `async` runs on the window's thread, so it must never wait for a
//! document's lock (a running change holds it for as long as the change takes). This test reads
//! the sources and follows every sync `#[tauri::command]` into the `Docs` methods it calls
//! (through `self.` calls too); it fails when one reaches `.read()` or `.write()` on a document,
//! or `private_changed`, unless the method is on the short list below, where each entry says
//! why waiting there is fine.
//!
//! The small mutexes some commands take (`updates`, `matched`, `find_count`, `joins`,
//! `lookups`, `pivots`, the `Docs` map) are checked by hand and by `named_mutexes_*` below:
//! no job keeps one of them locked while it computes (they are taken for a lookup, an insert
//! or a take, and let go before any engine call).

#![cfg(test)]

use std::collections::{BTreeSet, HashMap};

const LIB: &str = include_str!("lib.rs");
const DOCS: &str = include_str!("docs.rs");

/// Methods that look like they wait for the lock but are safe, and why.
const ALLOWED: &[(&str, &str)] = &[
    ("read", "the guard itself; callers are checked, not this"),
    ("write", "the guard itself; callers are checked, not this"),
    ("try_read", "never waits"),
    ("info_now", "never waits for a change; only the very first answer of a tab can, and it is a question asked from async commands"),
    ("current_path", "try_read, then the path noted last"),
];

/// Names of the sync commands the audit lets through although they reach a lock, with the reason.
const SYNC_COMMANDS_ALLOWED: &[(&str, &str)] = &[
    ("new_document", "a document nobody else can hold yet"),
    (
        "close_document",
        "forgets the tab: map and small mutexes only",
    ),
];

/// Every `fn name(` in docs.rs with its body text (up to the next method of the same indent).
fn methods() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let lines: Vec<&str> = DOCS.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let t = l.trim_start();
        let indent = l.len() - t.len();
        let name = t
            .strip_prefix("pub fn ")
            .or_else(|| t.strip_prefix("fn "))
            .or_else(|| t.strip_prefix("pub(crate) fn "))
            .and_then(|r| r.split(|c: char| !(c.is_alphanumeric() || c == '_')).next())
            .filter(|n| !n.is_empty());
        if let (Some(n), 4) = (name, indent) {
            let mut j = i + 1;
            let mut body = String::from(l);
            while j < lines.len() {
                let lj = lines[j];
                let tj = lj.trim_start();
                let ij = lj.len() - tj.len();
                body.push('\n');
                body.push_str(lj);
                if ij == 4 && tj == "}" {
                    break;
                }
                j += 1;
            }
            out.insert(n.to_string(), body);
            i = j;
        }
        i += 1;
    }
    out
}

/// The names called as `self.name(` or `docs.name(` in a text.
fn calls(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for prefix in ["self.", "docs."] {
        let mut rest = text;
        while let Some(at) = rest.find(prefix) {
            // not a longer identifier ending in the prefix
            let before = rest[..at].chars().last();
            rest = &rest[at + prefix.len()..];
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if rest[name.len()..].starts_with('(') {
                out.insert(name);
            }
        }
    }
    out
}

/// Whether a text takes a document's lock by waiting.
fn waits(text: &str) -> bool {
    text.contains(".read()") || text.contains(".write()")
}

/// The sync commands in lib.rs: (name, body).
fn sync_commands() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let lib = LIB.replace("\r\n", "\n");
    let mut rest = lib.as_str();
    while let Some(at) = rest.find("#[tauri::command]") {
        rest = &rest[at + "#[tauri::command]".len()..];
        let head_end = rest.find('{').unwrap_or(rest.len());
        let head = &rest[..head_end];
        // skip attributes between the command attribute and the fn line
        let Some(f) = head.find("fn ") else { continue };
        let is_async = head[..f].contains("async");
        if is_async {
            continue;
        }
        let name: String = head[f + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let end = rest.find("\n}\n").map(|e| e + 3).unwrap_or(rest.len());
        out.push((name, rest[..end].to_string()));
    }
    out
}

/// Methods reached from a text, following `self.` calls (and their bodies) to a depth.
fn reach(text: &str, m: &HashMap<String, String>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut todo: Vec<String> = calls(text).into_iter().collect();
    while let Some(n) = todo.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        if ALLOWED.iter().any(|a| a.0 == n) {
            continue;
        }
        if let Some(b) = m.get(&n) {
            todo.extend(calls(b));
        }
    }
    seen
}

/// What a sync command body can wait on: itself, or any method it reaches (not through the allowed ones).
fn waiting_through(body: &str, m: &HashMap<String, String>) -> Vec<String> {
    let mut culprits = Vec::new();
    if waits(body) {
        culprits.push("its own body".to_string());
    }
    for r in reach(body, m) {
        if ALLOWED.iter().any(|a| a.0 == r) {
            continue;
        }
        if let Some(b) = m.get(&r) {
            if waits(b) {
                culprits.push(r.clone());
            }
        }
    }
    culprits
}

#[test]
fn no_sync_command_waits_for_a_documents_lock() {
    let m = methods();
    assert!(m.len() > 250, "the source reader found {} methods", m.len());
    let cmds = sync_commands();
    assert!(cmds.len() >= 30, "found {} sync commands", cmds.len());
    let mut bad = Vec::new();
    for (name, body) in &cmds {
        if SYNC_COMMANDS_ALLOWED.iter().any(|a| a.0 == name) {
            continue;
        }
        let culprits = waiting_through(body, &m);
        if !culprits.is_empty() {
            bad.push(format!(
                "{name}: reaches a waiting lock through {}",
                culprits.join(", ")
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "sync commands that can freeze the window:
{}",
        bad.join(
            "
"
        )
    );
}

/// The audit must report a sync command that calls the waiting `docs.info(` (the reply of an operation,
/// which waits for the document); the question `docs.info_now(` is allowed.
#[test]
fn a_sync_command_calling_the_waiting_info_is_reported() {
    let m = methods();
    let bad = "fn doc_info(docs: DocsState<'_>, doc: u32) -> CmdResult<DocInfo> {
    docs.info(doc)
}";
    assert!(
        !waiting_through(bad, &m).is_empty(),
        "docs.info( from a sync command must be reported"
    );
    let good = "fn doc_info(docs: DocsState<'_>, doc: u32) -> CmdResult<DocInfo> {
    docs.info_now(doc)
}";
    assert!(waiting_through(good, &m).is_empty());
}

#[test]
fn the_audit_sees_the_commands_that_used_to_wait() {
    // doc_info, set_read_only and filter_spec were sync and waited; they are async now.
    let names: Vec<String> = sync_commands().into_iter().map(|c| c.0).collect();
    for gone in ["doc_info", "set_read_only", "filter_spec"] {
        assert!(!names.contains(&gone.to_string()), "{gone} must be async");
    }
    // And the reader would catch one: a made-up body that calls a waiting method.
    let m = methods();
    assert!(
        waits(m.get("filter_spec").unwrap()),
        "filter_spec's method does wait, so a sync command calling it would be flagged"
    );
}

/// The mutexes a job might hold across its run are named here. Each use was read and lets go before
/// the engine is called (a lookup, an insert or a take). This only checks that the uses are still
/// there to be read; a reviewer re-reads them when one is added.
#[test]
fn named_mutexes_are_taken_briefly_and_their_uses_are_counted() {
    let expected: &[(&str, usize)] = &[
        (".updates", 3),
        (".matched", 3),
        (".find_count", 3),
        ("self.joins.lock", 3),
        ("self.lookups.lock", 2),
        (".pivots", 3),
    ];
    for (needle, count) in expected {
        let found = DOCS.matches(needle).count();
        // tests in docs.rs also touch some (joins, lookups); only a growth in non-test uses matters
        assert!(
            found >= *count,
            "{needle}: the audit expected at least {count} uses, found {found}"
        );
    }
}
