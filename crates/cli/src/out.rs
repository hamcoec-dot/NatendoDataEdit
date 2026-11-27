//! Everything the CLI prints goes through here. When whoever reads the output stops
//! reading (`natendo-cli view big.csv | head`), the rest of the output is dropped
//! quietly: no panic, no backtrace. The command still finishes its own work (a file it
//! writes is written whole) and exits with its own result. Any other failure to write
//! the output (a full disk under a redirect) is reported once, and the run then fails.

use std::fmt;
use std::io::{BufWriter, ErrorKind, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// One buffered writer for the whole run (a lock and a system call per line is slower
/// for long output); flushed by [`finish`].
static OUT: Mutex<Option<BufWriter<Stdout>>> = Mutex::new(None);

/// The reader went away: nothing more is printed.
static CLOSED: AtomicBool = AtomicBool::new(false);
/// Another write failure, kept for the end of the run.
static FAILED: Mutex<Option<String>> = Mutex::new(None);

/// The output can no longer be written (the reader stopped, or a write failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

/// Print to stdout. `Err(Closed)` once the output cannot take more; callers that only
/// print may stop there, and callers with other work go on with it.
pub fn write(args: fmt::Arguments) -> Result<(), Closed> {
    if CLOSED.load(Ordering::Relaxed) {
        return Err(Closed);
    }
    let mut o = OUT.lock().unwrap_or_else(|p| p.into_inner());
    let w = o.get_or_insert_with(|| BufWriter::with_capacity(1 << 16, std::io::stdout()));
    match w.write_fmt(args) {
        Ok(()) => Ok(()),
        Err(e) => {
            stop(e);
            Err(Closed)
        }
    }
}

fn stop(e: std::io::Error) {
    CLOSED.store(true, Ordering::Relaxed);
    if e.kind() != ErrorKind::BrokenPipe {
        let mut f = FAILED.lock().unwrap_or_else(|p| p.into_inner());
        f.get_or_insert_with(|| format!("natendo-cli: cannot write the output: {e}"));
    }
}

/// At the end of the run: flush, and the write failure to report, if any (a reader
/// that went away is not one).
pub fn finish() -> Option<String> {
    let mut o = OUT.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(w) = o.as_mut() {
        if !CLOSED.load(Ordering::Relaxed) {
            if let Err(e) = w.flush() {
                stop(e);
            }
        }
    }
    // What is left after a failure is dropped unwritten (dropping a BufWriter would
    // try to write it again).
    if let Some(w) = o.take() {
        let (_, _) = w.into_parts();
    }
    FAILED.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// Print to stderr; a closed stderr is ignored. What stdout holds so far goes first, so
/// the two keep their order on a terminal.
pub fn err(args: fmt::Arguments) {
    if let Some(w) = OUT.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        if !CLOSED.load(Ordering::Relaxed) {
            if let Err(e) = w.flush() {
                stop(e);
            }
        }
    }
    let _ = std::io::stderr().lock().write_fmt(args);
}

/// Whether a panic came from the standard library failing to print to a reader that went
/// away (a print this module did not handle): os error 109 or 232 on Windows, EPIPE
/// elsewhere, as `ErrorKind::BrokenPipe` words it.
pub fn is_pipe_panic(message: &str) -> bool {
    let broken = std::io::Error::from(ErrorKind::BrokenPipe).to_string();
    message.contains("failed printing to std")
        && (message.contains(&broken)
            || message.contains("os error 232")
            || message.contains("os error 109")
            || message.contains("os error 32)"))
}

/// `println!` through [`write`]: `Result<(), Closed>`.
#[macro_export]
macro_rules! outln {
    () => { $crate::out::write(format_args!("\n")) };
    ($($a:tt)*) => { $crate::out::write(format_args!("{}\n", format_args!($($a)*))) };
}

/// `print!` through [`write`].
#[macro_export]
macro_rules! out {
    ($($a:tt)*) => { $crate::out::write(format_args!($($a)*)) };
}

/// `eprintln!` that ignores a closed stderr.
#[macro_export]
macro_rules! errln {
    ($($a:tt)*) => { $crate::out::err(format_args!("{}\n", format_args!($($a)*))) };
}

#[cfg(test)]
mod tests {
    #[test]
    fn pipe_panics_are_told_apart() {
        assert!(super::is_pipe_panic(
            "failed printing to stdout: The pipe is being closed. (os error 232)"
        ));
        assert!(super::is_pipe_panic(
            "failed printing to stdout: Broken pipe (os error 32)"
        ));
        assert!(!super::is_pipe_panic("index out of bounds"));
        assert!(!super::is_pipe_panic(
            "failed printing to stdout: There is not enough space on the disk. (os error 112)"
        ));
    }
}
