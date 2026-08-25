//! `zyris-core` is a library. Nothing in it may decide to write on a terminal, to end a process,
//! or to listen for a signal — those belong to the caller, and every one of the three consumer
//! repositories has already had to work around a place where this crate took one of them anyway.
//!
//! This reads the crate's own sources rather than its API, because the constructs it forbids leave
//! no trace in a signature: a `println!` buried in a polling loop compiles, passes every other
//! test, and only shows up as an enrollment code drawn over somebody's full-screen UI.
//!
//! Only `src/` is walked, so the needles spelled out below — which are, unavoidably, the very
//! strings being searched for — cannot make this file trip over itself.

use std::path::{Path, PathBuf};

/// Ordered longest-first inside the print family: `eprintln!` contains `println!`, and `eprint!`
/// contains `print!`, so a shorter needle placed first would name the wrong construct in the
/// report.
const PROGRAM_SHAPED: &[&str] =
    &["eprintln!", "eprint!", "println!", "print!", "ExitCode", "tokio::signal", "is_terminal"];

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("a readable directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            into.push(path);
        }
    }
}

#[test]
fn the_library_holds_no_program() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    assert!(!files.is_empty(), "found no sources to scan; the walk is broken, not the crate");

    let mut offences = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for (index, line) in text.lines().enumerate() {
            // A comment cannot print anything. Prose that names one of these is how this crate
            // explains what a caller may do with the values it hands back.
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some(needle) = PROGRAM_SHAPED.iter().find(|needle| line.contains(**needle)) {
                let shown = file.strip_prefix(root).unwrap_or(file);
                offences.push(format!("  {}:{}  {needle}", shown.display(), index + 1));
            }
        }
    }

    assert!(
        offences.is_empty(),
        "a library does not print, exit, or catch signals, but {} line(s) here do:\n{}",
        offences.len(),
        offences.join("\n")
    );
}
