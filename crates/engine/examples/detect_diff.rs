//! Prints the separator guess for each file named on the command line (or
//! listed one per line in a file given with `@list`), one line per file:
//! path, then for each of the extensions csv/txt/tsv and the file's own:
//! separator, skip, columns, confident. Used to compare two builds.

use natendo_engine::detect::{detect, Hints};

fn main() {
    let mut files: Vec<String> = Vec::new();
    for a in std::env::args().skip(1) {
        if let Some(list) = a.strip_prefix('@') {
            let text = std::fs::read_to_string(list).expect("list");
            files.extend(text.lines().filter(|l| !l.is_empty()).map(String::from));
        } else {
            files.push(a);
        }
    }
    for f in files {
        let Ok(data) = std::fs::read(&f) else {
            println!("{f}\tunreadable");
            continue;
        };
        let own = std::path::Path::new(&f)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let mut out = vec![f.clone()];
        for ext in [Some("csv"), Some("txt"), Some("tsv"), own.as_deref()] {
            let d = detect(&data, Hints { ext, quote: None });
            let b = d.best();
            out.push(format!(
                "{}:{:?} skip {} cols {} conf {}",
                ext.unwrap_or("-"),
                String::from_utf8_lossy(b.dialect.delimiter()),
                b.skip_lines,
                b.columns,
                d.is_confident()
            ));
        }
        if std::env::var_os("DETECT_DETAIL").is_some() {
            let d = detect(
                &data,
                Hints {
                    ext: own.as_deref(),
                    quote: None,
                },
            );
            for c in &d.choices {
                out.push(format!(
                    "  {:?} skip {} cols {} score {:.3}",
                    String::from_utf8_lossy(c.dialect.delimiter()),
                    c.skip_lines,
                    c.columns,
                    c.score
                ));
            }
        }
        println!("{}", out.join("\t"));
    }
}
