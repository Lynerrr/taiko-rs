//! Golden files: a test renders what it checks as text and compares it with the file kept in
//! `src/regression/snapshots/`. A difference fails the test and leaves the new text next to the
//! old one (`<name>.snap.new`) to look at.
//!
//! An intended change is accepted by running the tests with `TAIKO_BLESS=1`, which rewrites the
//! files; the diff of the `.snap` files then shows the change for review.

use std::fmt::Write as _;
use std::path::PathBuf;

/// Where the golden files live (in the source tree, so they're reviewed with the code).
pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/regression/snapshots")
}

fn blessing() -> bool {
    std::env::var_os("TAIKO_BLESS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Line endings and trailing space don't count (a Windows checkout may turn `\n` into `\r\n`).
fn normal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.replace("\r\n", "\n").lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Fails unless `actual` matches the golden file `name` (see the module notes).
#[track_caller]
pub fn assert_snapshot(name: &str, actual: &str) {
    let path = dir().join(format!("{name}.snap"));
    let new_path = dir().join(format!("{name}.snap.new"));
    let actual = normal(actual);
    if blessing() {
        std::fs::create_dir_all(dir()).expect("make the snapshot folder");
        let same = std::fs::read_to_string(&path).is_ok_and(|old| normal(&old) == actual);
        if !same {
            std::fs::write(&path, &actual).expect("write the snapshot");
        }
        let _ = std::fs::remove_file(&new_path);
        return;
    }
    let Ok(expected) = std::fs::read_to_string(&path) else {
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(&new_path, &actual);
        panic!(
            "no golden file {} yet: run the tests with TAIKO_BLESS=1 to make it, \
             then review and commit it",
            path.display()
        );
    };
    let expected = normal(&expected);
    if expected == actual {
        let _ = std::fs::remove_file(&new_path);
        return;
    }
    let _ = std::fs::write(&new_path, &actual);
    panic!(
        "{name}: the behaviour changed (golden {}):\n{}\nIf the change is intended, rerun with \
         TAIKO_BLESS=1 and commit the updated .snap file.",
        path.display(),
        diff(&expected, &actual)
    );
}

/// The lines that differ (by longest common subsequence), at most 60 of them: `-` golden and
/// `+` now, with the golden line number.
fn diff(expected: &str, actual: &str) -> String {
    let (e, a): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    // lcs[i][j]: the longest common subsequence of e[i..] and a[j..].
    let mut lcs = vec![vec![0u32; a.len() + 1]; e.len() + 1];
    for i in (0..e.len()).rev() {
        for j in (0..a.len()).rev() {
            lcs[i][j] = if e[i] == a[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = String::new();
    let (mut i, mut j, mut shown) = (0, 0, 0);
    while (i < e.len() || j < a.len()) && shown < 60 {
        if i < e.len() && j < a.len() && e[i] == a[j] {
            i += 1;
            j += 1;
        } else if j < a.len() && (i == e.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            let _ = writeln!(out, "  {:>5} + {}", i + 1, a[j]);
            j += 1;
            shown += 1;
        } else {
            let _ = writeln!(out, "  {:>5} - {}", i + 1, e[i]);
            i += 1;
            shown += 1;
        }
    }
    if i < e.len() || j < a.len() {
        out.push_str("  ...\n");
    }
    out
}

/// A float in a golden file: rounded so the last bits of libm functions, which differ between
/// platforms, don't show.
pub fn num(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    // No "-0.000".
    if s.trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        s.trim_start_matches('-').to_string()
    } else {
        s
    }
}

#[test]
fn snapshot_text_ignores_line_endings_and_trailing_space() {
    assert_eq!(normal("a  \r\nb\n\n\n"), "a\nb\n");
    assert_eq!(num(-0.0001, 3), "0.000");
    assert_eq!(num(1.23456, 2), "1.23");
    assert!(diff("a\nb\n", "a\nc\n").contains("- b"));
}
