//! `natendo-cli app ...`: talk to the running app's agent link. The link is
//! off unless switched on in the app; when on, the app writes its port and a
//! secret key to `agent.json` in its data folder (the user's own profile),
//! and every request carries that key.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use crate::USAGE;

/// Longest reply read.
const MAX_REPLY: u64 = 64 << 20;

fn link_file() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("com.natendo.dataedit").join("agent.json"))
}

fn num(s: &str, what: &str) -> Result<u64, String> {
    s.parse::<u64>()
        .map_err(|_| format!("{what} must be a number"))
}

/// Build the request for a command line.
fn request(args: &[&str]) -> Result<Value, String> {
    Ok(match args {
        ["tabs"] => json!({ "cmd": "tabs" }),
        ["info", tab] => json!({ "cmd": "info", "tab": num(tab, "the tab")? }),
        ["read", tab, rest @ ..] if rest.len() <= 2 => {
            let first = rest.first().map_or(Ok(1), |s| num(s, "the first row"))?;
            let count = rest.get(1).map_or(Ok(100), |s| num(s, "the row count"))?;
            json!({ "cmd": "read", "tab": num(tab, "the tab")?, "row": first.max(1) - 1, "count": count })
        }
        ["open", path] => {
            let p = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
            json!({ "cmd": "open", "path": p })
        }
        ["goto", tab, row, col] => json!({
            "cmd": "goto",
            "tab": num(tab, "the tab")?,
            "row": num(row, "the row")?.max(1) - 1,
            "col": num(col, "the column")?.max(1) - 1,
        }),
        _ => return Err(USAGE.to_string()),
    })
}

pub fn run(args: &[&str]) -> Result<(), String> {
    let mut req = request(args)?;
    let file = link_file().ok_or("cannot find the app's data folder")?;
    let text = std::fs::read_to_string(&file).map_err(|_| {
        "the app's agent link is off: switch it on in the app (Tools > Agent link)".to_string()
    })?;
    let link: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let port = link["port"]
        .as_u64()
        .and_then(|p| u16::try_from(p).ok())
        .ok_or("agent.json has no port")?;
    let key = link["key"].as_str().ok_or("agent.json has no key")?;
    req["key"] = json!(key);
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|_| "the app is not running, or its agent link is off".to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(60))).ok();
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    s.write_all(&line).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(s.take(MAX_REPLY))
        .read_line(&mut reply)
        .map_err(|e| e.to_string())?;
    let v: Value =
        serde_json::from_str(reply.trim()).map_err(|_| "the app sent no answer".to_string())?;
    if v["ok"].as_bool() == Some(true) {
        crate::more::print_json(&v["result"]);
        Ok(())
    } else {
        Err(v["error"]
            .as_str()
            .unwrap_or("the app refused the request")
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_built_from_arguments() {
        assert_eq!(request(&["tabs"]).unwrap(), json!({ "cmd": "tabs" }));
        assert_eq!(
            request(&["read", "2", "10", "5"]).unwrap(),
            json!({ "cmd": "read", "tab": 2, "row": 9, "count": 5 })
        );
        assert_eq!(
            request(&["goto", "1", "1", "3"]).unwrap(),
            json!({ "cmd": "goto", "tab": 1, "row": 0, "col": 2 })
        );
        assert!(request(&["read", "x"]).is_err());
        assert!(request(&["delete"]).is_err());
    }
}
