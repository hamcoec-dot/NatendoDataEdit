//! Secrets on the command line: a scramble or patch phrase, or a file
//! password. Never an argument (it would show in the process list and the
//! shell's history): the first line of standard input, the first line of a
//! file, or an environment variable (`NATENDO_PHRASE`, `NATENDO_PASSWORD`),
//! exactly one of them. A secret is never printed: errors name where it
//! came from, never what it is.

use std::io::{BufRead, Read};
use std::sync::Mutex;

use natendo_engine::privacy::{wipe, Secret};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Phrase,
    Password,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Phrase => "phrase",
            Kind::Password => "password",
        }
    }
    pub(crate) fn var(self) -> &'static str {
        match self {
            Kind::Phrase => "NATENDO_PHRASE",
            Kind::Password => "NATENDO_PASSWORD",
        }
    }
    fn slot(self) -> usize {
        match self {
            Kind::Phrase => 0,
            Kind::Password => 1,
        }
    }
}

/// What the environment held (taken out of it at start-up): `Err` when the
/// value was not valid text.
type FromEnv = Option<Result<String, ()>>;

static FROM_ENV: Mutex<[FromEnv; 2]> = Mutex::new([None, None]);

/// Largest secret file read (only its first line is used).
const MAX_SECRET_FILE: u64 = 64 << 10;

/// The option that lets a spec, cleaner or recipe file carry a secret.
pub const ALLOW_IN_FILE: &str = "--allow-phrase-in-file";

/// Read `NATENDO_PHRASE` and `NATENDO_PASSWORD` and remove them from this
/// process's environment, so nothing started later (and no crash report)
/// inherits them. Called first thing in `main`, before rayon or any other
/// thread exists.
pub fn take_env() {
    let mut kept = FROM_ENV.lock().unwrap_or_else(|p| p.into_inner());
    for kind in [Kind::Phrase, Kind::Password] {
        if let Some(v) = std::env::var_os(kind.var()) {
            // Removing a variable races with any other thread that reads the
            // environment, so this runs while main is the only thread. On
            // edition 2024 `remove_var` is `unsafe`: keep the `unsafe` block
            // here, at the very start of main, with this reason as its
            // SAFETY comment.
            std::env::remove_var(kind.var());
            kept[kind.slot()] = Some(v.into_string().map_err(|_| ()));
        }
    }
}

/// Whether an argument looks like a secret typed on the command line
/// (`--password x`, `--phrase=x`, `--passphrase`): the option's name only,
/// never its value, goes in the message.
fn typed_secret(x: &str) -> Option<&str> {
    let name = x.split('=').next().unwrap_or(x);
    let lower = name.to_ascii_lowercase();
    let bare = matches!(
        lower.as_str(),
        "--phrase" | "--password" | "--passphrase" | "--pass" | "--pw" | "--secret"
    );
    let with_value =
        x.contains('=') && (lower.starts_with("--phrase") || lower.starts_with("--pass"));
    (bare || with_value).then_some(name)
}

/// The first line of `text`, without a byte order mark or line end.
fn first_line(text: &str) -> &str {
    let line = text.split('\n').next().unwrap_or_default();
    line.trim_start_matches('\u{feff}').trim_end_matches('\r')
}

fn read_stdin_line() -> Result<String, String> {
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("could not read standard input: {e}"))?;
    Ok(line)
}

fn read_file_line(path: &str, word: &str) -> Result<String, String> {
    let f =
        std::fs::File::open(path).map_err(|e| format!("could not read the {word} file: {e}"))?;
    let mut bytes = Vec::new();
    f.take(MAX_SECRET_FILE)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("could not read the {word} file: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(path) {
            if m.permissions().mode() & 0o077 != 0 {
                eprintln!(
                    "warning: other users can read the {word} file; keep it in a private folder (chmod 600)"
                );
            }
        }
    }
    String::from_utf8(bytes).map_err(|e| {
        let mut b = e.into_bytes();
        wipe(&mut b);
        format!("the {word} file is not valid UTF-8 text")
    })
}

/// Take this kind's secret options out of `args` and read the secret, if
/// one is given: `--<kind>-stdin`, `--<kind>-file <f>` or the environment
/// variable, exactly one. `min` is the shortest the secret may be
/// (characters), checked here, before any file is opened or written.
/// Returns the secret and the other arguments.
pub fn take<'a>(
    args: &[&'a str],
    kind: Kind,
    min: usize,
) -> Result<(Option<Secret>, Vec<&'a str>), String> {
    let w = kind.word();
    let stdin_flag = format!("--{w}-stdin");
    let file_flag = format!("--{w}-file");
    let mut rest = Vec::with_capacity(args.len());
    let mut stdin = 0usize;
    let mut files: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(&x) = it.next() {
        if x == stdin_flag {
            stdin += 1;
        } else if x == file_flag {
            files.push(
                it.next()
                    .copied()
                    .ok_or_else(|| format!("{file_flag} needs a file"))?,
            );
        } else if let Some(name) = typed_secret(x) {
            return Err(format!(
                "{name} is not taken: a {w} on the command line shows in the process list and the shell's history. Give it with {stdin_flag}, {file_flag} <file> or the {} environment variable",
                kind.var()
            ));
        } else {
            rest.push(x);
        }
    }
    // Standard input carries one thing only (P4).
    let stdins = args.iter().filter(|a| a.ends_with("-stdin")).count();
    if stdins > 1 {
        return Err(
            "standard input can carry only one thing: give one secret with -stdin and the others with a file or the environment".into(),
        );
    }
    if stdin > 0 && rest.contains(&"-") {
        return Err(format!(
            "the input is read from standard input, so the {w} cannot be: use {file_flag} <file> or {}",
            kind.var()
        ));
    }
    let from_env = FROM_ENV.lock().unwrap_or_else(|p| p.into_inner())[kind.slot()].take();
    let sources = stdin + files.len() + usize::from(from_env.is_some());
    if sources > 1 {
        return Err(format!(
            "give the {w} one way only ({stdin_flag}, {file_flag} or {})",
            kind.var()
        ));
    }
    let mut text = match (stdin, files.first(), from_env) {
        (1, _, _) => read_stdin_line()?,
        (_, Some(path), _) => read_file_line(path, w)?,
        (_, _, Some(Ok(v))) => v,
        (_, _, Some(Err(()))) => {
            return Err(format!("{} is not valid text", kind.var()));
        }
        _ => return Ok((None, rest)),
    };
    let line = first_line(&text);
    let chars = line.chars().count();
    let secret = (chars > 0).then(|| Secret::new(line));
    // The copy read is overwritten (the Secret wipes its own on drop).
    let mut bytes = std::mem::take(&mut text).into_bytes();
    wipe(&mut bytes);
    let Some(secret) = secret else {
        return Err(format!("the {w} is empty"));
    };
    if chars < min {
        return Err(format!("the {w} must be at least {min} characters"));
    }
    Ok((Some(secret), rest))
}

/// Take a plain flag out of `args`.
pub fn take_flag<'a>(args: &[&'a str], flag: &str) -> (bool, Vec<&'a str>) {
    let found = args.contains(&flag);
    (found, args.iter().copied().filter(|a| *a != flag).collect())
}

/// Keys a spec, cleaner or recipe file must not hold: a secret in a file
/// next to the data travels with it (P5). Any case, any depth.
const SECRET_KEYS: [&str; 3] = ["phrase", "password", "passphrase"];

fn find_secret_key(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => m.iter().find_map(|(k, v)| {
            if SECRET_KEYS.contains(&k.to_ascii_lowercase().as_str()) {
                Some(k.clone())
            } else {
                find_secret_key(v)
            }
        }),
        Value::Array(a) => a.iter().find_map(find_secret_key),
        _ => None,
    }
}

/// Refuse a file that holds a phrase or password, unless `allow`. Checked on
/// the raw JSON before it is read as a spec (a file that is not JSON is
/// left for the reader to refuse).
pub fn refuse_secret_in_file(path: &str, bytes: &[u8], allow: bool) -> Result<(), String> {
    if allow {
        return Ok(());
    }
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(());
    };
    match find_secret_key(&v) {
        Some(k) => Err(format!(
            "{path} holds a \"{k}\": a secret in a file next to the data travels with it. Give it with --phrase-stdin, --phrase-file or NATENDO_PHRASE, or add {ALLOW_IN_FILE}"
        )),
        None => Ok(()),
    }
}

/// Put `phrase` into every scramble step of a cleaner.
pub fn fill_scramble(cleaner: &mut natendo_engine::clean::Cleaner, phrase: &Secret) -> usize {
    use natendo_engine::clean::Step;
    let mut n = 0;
    for s in &mut cleaner.steps {
        if let Step::Scramble { phrase: p, .. } = s {
            *p = phrase.clone();
            n += 1;
        }
    }
    n
}

/// Whether a cleaner has a scramble step without a phrase.
pub fn scramble_without_phrase(cleaner: &natendo_engine::clean::Cleaner) -> bool {
    use natendo_engine::clean::Step;
    cleaner
        .steps
        .iter()
        .any(|s| matches!(s, Step::Scramble { phrase, .. } if phrase.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_variable_is_read_once_and_removed() {
        // Only this test touches NATENDO_PASSWORD in this process.
        std::env::set_var("NATENDO_PASSWORD", "a long enough password");
        take_env();
        assert!(std::env::var_os("NATENDO_PASSWORD").is_none());
        let (s, rest) = take(&["in.csv"], Kind::Password, 12).unwrap();
        assert!(s.is_some());
        assert_eq!(rest, ["in.csv"]);
        // Taken: a second read finds nothing.
        assert!(take(&[], Kind::Password, 12).unwrap().0.is_none());
    }

    #[test]
    fn a_secret_typed_as_an_argument_is_refused_without_its_value() {
        for a in [
            "--password=hunter2hunter2",
            "--phrase",
            "--Passphrase=abc",
            "--phrase-stdin=x",
        ] {
            let e = take(&["in.csv", a], Kind::Phrase, 16).unwrap_err();
            assert!(
                !e.contains("hunter2") && !e.contains("abc") && !e.contains("=x"),
                "{e}"
            );
        }
    }

    #[test]
    fn stdin_carries_one_thing() {
        let e = take(&["--phrase-stdin", "--password-stdin"], Kind::Phrase, 1).unwrap_err();
        assert!(e.contains("only one thing"), "{e}");
        let e = take(&["-", "--phrase-stdin"], Kind::Phrase, 1).unwrap_err();
        assert!(e.contains("standard input"), "{e}");
    }

    #[test]
    fn secret_keys_are_found_at_any_depth_and_case() {
        let deep = br#"{"columns":[{"action":"clean","cleaner":{"steps":[{"op":"scramble","Phrase":"x"}]}}]}"#;
        assert!(refuse_secret_in_file("s.json", deep, false).is_err());
        assert!(refuse_secret_in_file("s.json", deep, true).is_ok());
        assert!(refuse_secret_in_file("s.json", br#"{"a":{"PASSWORD":1}}"#, false).is_err());
        assert!(refuse_secret_in_file("s.json", br#"[{"passphrase":""}]"#, false).is_err());
        assert!(refuse_secret_in_file("s.json", br#"{"phrases":1}"#, false).is_ok());
    }
}
