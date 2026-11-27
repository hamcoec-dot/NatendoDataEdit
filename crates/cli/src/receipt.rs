//! Processing receipts from the command line: `--receipt` on `run` and `convert` writes
//! `<out>.ndreceipt.json` beside the output (see `natendo_engine::receipt`), and
//! `receipt verify` checks one against files named on the command line only.

use std::path::{Path, PathBuf};

use natendo_engine::receipt::{verify, FileRecord, Receipt, StepsRecord};
use natendo_engine::{Document, Progress};
use serde_json::{json, Value};

/// What `--receipt`, `--receipt-paths` and `--receipt-full-steps` asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ask {
    pub on: bool,
    pub paths: bool,
    pub full_steps: bool,
}

impl Ask {
    /// Take the receipt options out of `args`.
    pub fn take<'a>(args: &[&'a str]) -> Result<(Ask, Vec<&'a str>), String> {
        let mut a = Ask::default();
        let mut rest = Vec::new();
        for &x in args {
            match x {
                "--receipt" => a.on = true,
                "--receipt-paths" => a.paths = true,
                "--receipt-full-steps" => a.full_steps = true,
                _ => rest.push(x),
            }
        }
        if (a.paths || a.full_steps) && !a.on {
            return Err("--receipt-paths and --receipt-full-steps go with --receipt".into());
        }
        Ok((a, rest))
    }
}

/// A receipt being made for one run from `input`.
pub struct Making {
    receipt: Receipt,
    ask: Ask,
    /// The input file as it was before it was read (for imported files, which keep no path).
    before: Option<FileRecord>,
    input: PathBuf,
}

impl Making {
    /// Start, before `input` is read (only when asked).
    pub fn start(ask: Ask, action: &str, input: &str) -> Result<Option<Making>, String> {
        if !ask.on {
            return Ok(None);
        }
        let p = PathBuf::from(input);
        let before = FileRecord::of_file(&p, ask.paths, &Progress::new())
            .map_err(|e| format!("{input}: {e}"))?;
        Ok(Some(Making {
            receipt: Receipt::start(action),
            ask,
            before: Some(before),
            input: p,
        }))
    }

    /// The input as read into `d`: the parsed bytes when the table keeps its file, else the
    /// file hashed before reading, kept only when it is still the same now.
    pub fn read(&mut self, d: &Document) -> Result<(), String> {
        let p = Progress::new();
        let mut rec =
            match FileRecord::of_input(d, self.ask.paths, &p).map_err(|e| e.to_string())? {
                Some(r) => r,
                None => {
                    let mut was = self.before.take().unwrap_or_else(|| FileRecord {
                        name: String::new(),
                        path: None,
                        bytes: 0,
                        sha256: None,
                        rows: None,
                        cols: None,
                    });
                    let now = FileRecord::of_file(&self.input, self.ask.paths, &p)
                        .map_err(|e| e.to_string())?;
                    if now.sha256 != was.sha256 || now.bytes != was.bytes {
                        was.sha256 = None;
                    }
                    was
                }
            };
        rec.rows = Some(d.total_rows());
        rec.cols = Some(u64::from(d.col_count()));
        self.receipt.add_input(rec, &self.input);
        Ok(())
    }

    /// The output is written: hash it, add the steps (JSON, recipes' form) and write the
    /// receipt beside it. A receipt that cannot be written is an error after the output stands.
    pub fn finish(
        mut self,
        out: &str,
        rows: u64,
        cols: u32,
        steps: Option<(Value, Vec<String>)>,
    ) -> Result<PathBuf, String> {
        let p = Progress::new();
        let mut rec =
            FileRecord::of_file(Path::new(out), self.ask.paths, &p).map_err(|e| e.to_string())?;
        rec.rows = Some(rows);
        rec.cols = Some(u64::from(cols));
        self.receipt.outputs.push(rec);
        self.receipt.steps =
            steps.map(|(s, titles)| StepsRecord::of(&s, titles, false, self.ask.full_steps));
        self.receipt
            .write_beside(Path::new(out))
            .map_err(|e| format!("{out} was written, but its receipt was not: {e}"))
    }
}

/// `receipt verify <receipt> [--input f]... [--output g]... [--chain] [--json]`: exit 0 when
/// every check passes, 1 when one fails, 2 on an error.
pub fn verify_cmd(args: &[&str], json_out: bool) -> Result<bool, String> {
    let mut receipt = None;
    let (mut inputs, mut outputs) = (Vec::new(), Vec::new());
    let mut chain = false;
    let mut it = args.iter();
    while let Some(&x) = it.next() {
        match x {
            "--input" => inputs.push(PathBuf::from(*it.next().ok_or("--input needs a file")?)),
            "--output" => outputs.push(PathBuf::from(*it.next().ok_or("--output needs a file")?)),
            "--chain" => chain = true,
            _ if x.starts_with('-') => return Err(format!("unknown option {x}")),
            _ if receipt.is_none() => receipt = Some(PathBuf::from(x)),
            _ => return Err("receipt verify takes one receipt".into()),
        }
    }
    let receipt = receipt.ok_or("usage: natendo-cli receipt verify <receipt> [--input <file>]... [--output <file>]... [--chain] [--json]")?;
    let r = verify(&receipt, &inputs, &outputs, chain, &Progress::new())
        .map_err(|e| format!("{}: {e}", receipt.display()))?;
    if json_out {
        crate::more::print_json(&json!({
            "ok": r.ok,
            "compared": r.compared,
            "verdict": r.verdict,
            "checks": r.lines,
            "receipt": r.summary,
        }));
    } else {
        for l in &r.lines {
            let _ = crate::outln!(
                "{} {}: {}",
                if l.ok { "ok  " } else { "FAIL" },
                l.what,
                l.why
            );
        }
        let _ = crate::outln!("{}", r.verdict);
    }
    Ok(r.ok)
}
