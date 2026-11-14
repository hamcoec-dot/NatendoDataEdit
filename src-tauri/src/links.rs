//! Opening web and e-mail addresses from cells (Ctrl+click).
//!
//! The webview asks to open an address; it is untrusted, so the address is
//! checked here again: only `http://`, `https://` and `mailto:`, at most
//! 2,048 plain printable characters, no spaces, quotes or other characters
//! a command line could read specially, and a host name for web addresses.
//! Files, `file:`, `javascript:`, network paths and every other scheme are
//! refused. E-mail addresses keep only the subject, body, cc and bcc parts:
//! anything else (such as `attach=`, which some mail programs read as a
//! local file to attach) is dropped. The check passes the address on to the
//! system's handler (the default browser or mail program) without any
//! shell in between.

use crate::docs::{CmdError, CmdResult};

/// Longest address opened.
pub const MAX_LINK: usize = 2048;

/// The address, checked, or why it cannot be opened.
pub fn check_link(url: &str) -> CmdResult<String> {
    let bad = || {
        CmdError::new(
            "bad_link",
            "only web (http, https) and e-mail addresses can be opened",
        )
    };
    if url.is_empty() || url.len() > MAX_LINK {
        return Err(bad());
    }
    // Printable ASCII only (international addresses arrive punycoded and
    // percent-encoded from the webview's URL parser), and none of the
    // characters a command line or the handler could read specially.
    if !url.bytes().all(|b| {
        b.is_ascii_graphic()
            && !matches!(
                b,
                b'"' | b'\'' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'|' | b'{' | b'}'
            )
    }) {
        return Err(bad());
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("mailto:") {
        let rest = &url["mailto:".len()..];
        let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (user, host) = addr.split_once('@').ok_or_else(bad)?;
        if user.is_empty() || !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
            return Err(bad());
        }
        let kept: Vec<&str> = query
            .split('&')
            .filter(|part| {
                let name = part.split('=').next().unwrap_or("").to_ascii_lowercase();
                matches!(name.as_str(), "subject" | "body" | "cc" | "bcc")
            })
            .collect();
        let mut out = format!("mailto:{addr}");
        if !kept.is_empty() {
            out.push('?');
            out.push_str(&kept.join("&"));
        }
        return Ok(out);
    }
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .ok_or_else(bad)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // No user name or password in the address, and a real host.
    if authority.is_empty() || authority.contains('@') {
        return Err(bad());
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            authority
        }
    });
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'[' | b']' | b':'))
    {
        return Err(bad());
    }
    Ok(url.to_string())
}

/// Open a checked address with the system's handler.
pub fn open_link(url: &str) -> CmdResult<()> {
    let url = check_link(url)?;
    #[cfg(windows)]
    {
        // rundll32 by its full path (never found through PATH) hands the
        // address to the handler Windows has for its scheme.
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let exe = std::path::Path::new(&root)
            .join("System32")
            .join("rundll32.exe");
        std::process::Command::new(exe)
            .arg("url.dll,FileProtocolHandler")
            .arg(&url)
            .spawn()
            .map(|_| ())
            .map_err(|e| CmdError::new("error", e.to_string()))
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err(CmdError::new(
            "error",
            "opening addresses works on Windows only",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_addresses() {
        for ok in [
            "https://example.com",
            "http://example.com:8080/a/b?q=1#x",
            "https://xn--bcher-kva.example/p%20q",
            "mailto:someone@example.org",
            "mailto:a.b+c@mail.example.co.uk?subject=Hi",
            "https://[::1]/",
        ] {
            assert!(check_link(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "file:///C:/Windows/system32/calc.exe",
            "C:\\Windows\\notepad.exe",
            "\\\\server\\share\\x.exe",
            "javascript:alert(1)",
            "ms-settings:privacy",
            "https://",
            "https://user:pw@example.com/",
            "https://exa mple.com",
            "https://example.com/\"&calc",
            "https://example.com/a|b",
            "https://ex\u{e4}mple.com",
            "mailto:nobody",
            "mailto:@example.com",
            "ftp://example.com",
            "HTTPS://example.com\n",
        ] {
            assert!(check_link(bad).is_err(), "{bad:?}");
        }
        assert!(check_link(&format!("https://e.com/{}", "a".repeat(MAX_LINK))).is_err());
        // E-mail addresses keep only subject, body, cc and bcc.
        assert_eq!(
            check_link("mailto:x@evil.example?attach=C:/Users/me/list.csv").unwrap(),
            "mailto:x@evil.example"
        );
        assert_eq!(
            check_link("mailto:x@e.org?Subject=Hi&ATTACHMENT=/etc/passwd&cc=y@e.org").unwrap(),
            "mailto:x@e.org?Subject=Hi&cc=y@e.org"
        );
    }
}
