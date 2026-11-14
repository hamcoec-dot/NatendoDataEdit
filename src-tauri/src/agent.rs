//! The agent link: lets other programs on this computer (scripts, AI
//! agents, `natendo-cli app ...`) read the tables open in the app, open a
//! file in a tab and move the cursor. Nothing it offers changes data.
//!
//! Off by default. When on, it listens on 127.0.0.1 at a random port and
//! writes the port and a 256-bit random key to `agent.json` in the app's
//! data folder (inside the user's own profile). Every request must carry
//! the key; a request without it closes the connection. Requests are one
//! JSON object per line (at most 64 KB), at most 8 connections at once,
//! each closed after 30 seconds without a request. A web page cannot use
//! it: browsers speak HTTP, whose first line is not a JSON request with the
//! key.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::docs::{CmdError, CmdResult, Docs};

const MAX_LINE: u64 = 64 << 10;
const MAX_CONNECTIONS: usize = 8;
const IDLE: Duration = Duration::from_secs(30);
/// Time a connection has to present the key, in all.
const HELLO: Duration = Duration::from_secs(30);
/// Most cells one `read` returns.
pub const MAX_READ_CELLS: u64 = 10_000;
pub const FILE: &str = "agent.json";

struct Running {
    stop: Arc<AtomicBool>,
    port: u16,
    file: PathBuf,
}

/// The link's state (one per app).
#[derive(Default)]
pub struct Agent {
    running: Mutex<Option<Running>>,
}

/// What the settings screen shows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentStatus {
    pub on: bool,
    pub port: Option<u16>,
}

/// A request from a client.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Tabs,
    Info {
        tab: u32,
    },
    Read {
        tab: u32,
        #[serde(default)]
        row: u64,
        #[serde(default = "default_count")]
        count: u64,
    },
    Open {
        path: PathBuf,
    },
    Goto {
        tab: u32,
        row: u64,
        col: u32,
    },
}

fn default_count() -> u64 {
    100
}

/// Compare without stopping at the first difference.
fn same_key(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn new_key() -> CmdResult<String> {
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).map_err(|e| CmdError::new("error", e.to_string()))?;
    Ok(k.iter().map(|b| format!("{b:02x}")).collect())
}

impl Agent {
    pub fn status(&self) -> AgentStatus {
        let r = self.running.lock().unwrap_or_else(|p| p.into_inner());
        AgentStatus {
            on: r.is_some(),
            port: r.as_ref().map(|r| r.port),
        }
    }

    /// Switch the link on or off.
    pub fn set(&self, app: &AppHandle, on: bool) -> CmdResult<AgentStatus> {
        let mut r = self.running.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(old) = r.take() {
            old.stop.store(true, Ordering::Relaxed);
            let _ = std::fs::remove_file(&old.file);
        }
        if on {
            let dir = app
                .path()
                .app_data_dir()
                .map_err(|e| CmdError::new("error", e.to_string()))?;
            std::fs::create_dir_all(&dir).map_err(|e| CmdError::new("error", e.to_string()))?;
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .map_err(|e| CmdError::new("error", e.to_string()))?;
            listener
                .set_nonblocking(true)
                .map_err(|e| CmdError::new("error", e.to_string()))?;
            let port = listener
                .local_addr()
                .map_err(|e| CmdError::new("error", e.to_string()))?
                .port();
            let key = new_key()?;
            let file = dir.join(FILE);
            let body = json!({ "port": port, "key": key, "pid": std::process::id() });
            std::fs::write(&file, body.to_string())
                .map_err(|e| CmdError::new("error", e.to_string()))?;
            let stop = Arc::new(AtomicBool::new(false));
            let (stop2, app2) = (stop.clone(), app.clone());
            std::thread::spawn(move || serve(listener, key, stop2, app2));
            crate::applog::info(&format!("agent link on (port {port})"));
            *r = Some(Running { stop, port, file });
        } else {
            crate::applog::info("agent link off");
        }
        drop(r);
        Ok(self.status())
    }

    /// Remove a key file a crashed run left behind (its key no longer works).
    pub fn clear_stale(app: &AppHandle) {
        if let Ok(dir) = app.path().app_data_dir() {
            let _ = std::fs::remove_file(dir.join(FILE));
        }
    }

    /// Switch off and remove the key file (on exit).
    pub fn shutdown(&self) {
        if let Some(old) = self
            .running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            old.stop.store(true, Ordering::Relaxed);
            let _ = std::fs::remove_file(&old.file);
        }
    }
}

fn serve(listener: TcpListener, key: String, stop: Arc<AtomicBool>, app: AppHandle) {
    let active = Arc::new(AtomicUsize::new(0));
    let key = Arc::new(key);
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((s, addr)) => {
                if !addr.ip().is_loopback() || active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue;
                }
                active.fetch_add(1, Ordering::Relaxed);
                let (key, app, active, stop) =
                    (key.clone(), app.clone(), active.clone(), stop.clone());
                std::thread::spawn(move || {
                    let _ = client(s, &key, &app, &stop);
                    active.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn client(s: TcpStream, key: &str, app: &AppHandle, stop: &AtomicBool) -> std::io::Result<()> {
    s.set_nonblocking(false)?;
    s.set_read_timeout(Some(IDLE))?;
    s.set_write_timeout(Some(IDLE))?;
    let mut w = s.try_clone()?;
    let mut r = BufReader::new(s);
    let mut line = Vec::new();
    let start = std::time::Instant::now();
    let mut known = false;
    loop {
        line.clear();
        if !known {
            // Until the key arrives, the whole wait is limited, so trickled
            // bytes cannot hold a connection slot.
            match HELLO.checked_sub(start.elapsed()) {
                Some(left) if !left.is_zero() => r.get_ref().set_read_timeout(Some(left))?,
                _ => return Ok(()),
            }
        }
        let n = (&mut r).take(MAX_LINE).read_until(b'\n', &mut line)?;
        if n == 0 || stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if !line.ends_with(b"\n") && n as u64 >= MAX_LINE {
            return Ok(());
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            return Ok(());
        };
        if !v["key"].as_str().is_some_and(|k| same_key(k, key)) {
            return Ok(());
        }
        if !known {
            known = true;
            r.get_ref().set_read_timeout(Some(IDLE))?;
        }
        let reply = match serde_json::from_value::<Request>(v) {
            Ok(req) => match handle(req, app) {
                Ok(result) => json!({ "ok": true, "result": result }),
                Err(e) => json!({ "ok": false, "error": e.message }),
            },
            Err(e) => json!({ "ok": false, "error": format!("unknown request: {e}") }),
        };
        let mut out = serde_json::to_vec(&reply).unwrap_or_default();
        out.push(b'\n');
        w.write_all(&out)?;
    }
}

fn handle(req: Request, app: &AppHandle) -> CmdResult<Value> {
    let docs = app.state::<Arc<Docs>>().inner().clone();
    let tab_info = |id: u32| -> CmdResult<Value> {
        let i = docs.info(id)?;
        // Private file mode: the tab is listed, but not its name.
        if docs.is_private(id)? {
            return Ok(json!({ "tab": id, "name": "(private file)", "private": true }));
        }
        Ok(json!({
            "tab": id,
            "name": i.name,
            "rows": i.rows,
            "total_rows": i.total_rows,
            "columns": i.cols,
            "filtered": i.filtered,
            "modified": i.modified,
            "loading": !i.indexed,
        }))
    };
    match req {
        Request::Tabs => Ok(Value::Array(
            docs.ids()
                .into_iter()
                .filter_map(|id| tab_info(id).ok())
                .collect(),
        )),
        Request::Info { tab } => tab_info(tab),
        Request::Read { tab, row, count } => {
            if docs.is_private(tab)? {
                return Err(CmdError::new(
                    "private",
                    "this tab is in private file mode; the agent link cannot read it",
                ));
            }
            let rows = docs.read_rows(tab, row, count, MAX_READ_CELLS)?;
            Ok(json!({ "first_row": row + 1, "rows": rows }))
        }
        Request::Open { path } => {
            // No network paths: opening one would make Windows contact
            // that server.
            let text = path.to_string_lossy();
            if text.starts_with(r"\\") || text.starts_with("//") {
                return Err(CmdError::new(
                    "invalid",
                    "network paths are not opened through the agent link",
                ));
            }
            crate::applog::info("agent link: open a file");
            crate::open_paths(app, vec![path], true);
            Ok(json!("opening"))
        }
        Request::Goto { tab, row, col } => {
            docs.info(tab)?;
            let _ = app.emit("agent-goto", json!({ "doc": tab, "row": row, "col": col }));
            Ok(json!("moved"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_compare_whole() {
        assert!(same_key("abc", "abc"));
        assert!(!same_key("abc", "abd"));
        assert!(!same_key("abc", "ab"));
        let k = new_key().unwrap();
        assert_eq!(k.len(), 64);
        assert_ne!(k, new_key().unwrap());
    }

    #[test]
    fn requests_parse() {
        let r: Request = serde_json::from_str(r#"{"cmd":"read","tab":1}"#).unwrap();
        assert!(matches!(
            r,
            Request::Read {
                tab: 1,
                row: 0,
                count: 100
            }
        ));
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"save","tab":1}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"set_cells"}"#).is_err());
    }
}
