//! Crash reports the game sends on the start after a crash (`Request::CrashReport`): each is
//! kept in DIR/crashes.jsonl and goes to the Discord bot's crash channel (`discord::crash`),
//! the same crash on the same build as one post with a count.

use std::collections::HashMap;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::online::{CrashReport, MAX_CRASH_REPORT};

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// Reports from each address in the last hour.
static RECENT: OnceLock<Mutex<HashMap<IpAddr, Vec<Instant>>>> = OnceLock::new();
const PER_HOUR: usize = 6;

pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// What tells one crash from another: the panic's place and message, or the exception and the
/// module it was in, without the addresses and numbers that differ between runs of the same crash.
pub fn signature(report: &str) -> String {
    let mut lines = report
        .lines()
        .skip_while(|l| l.starts_with("---"))
        .map(str::trim);
    let first = lines.next().unwrap_or("");
    // "thread main: panicked at src/x.rs:1:2:" and the message on the next line.
    if let Some(at) = first.find("panicked at ") {
        let place = first[at + "panicked at ".len()..].trim_end_matches(':');
        let message: String = lines.next().unwrap_or("").chars().take(120).collect();
        return format!("{place}: {}", without_numbers(&message));
    }
    // "access violation 0xC0000005 at igvk64.dll+0x13b702": the module, not where in it.
    let first = first.split('+').next().unwrap_or(first);
    first.chars().take(160).collect()
}

/// Runs of digits as "N": "index 7 out of 3" and "index 9 out of 4" are the same crash.
fn without_numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_number = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !in_number {
                out.push('N');
            }
            in_number = true;
        } else {
            in_number = false;
            out.push(c);
        }
    }
    out
}

/// Takes a report from `ip`: kept, and posted on Discord in the background.
pub fn report(ip: IpAddr, r: CrashReport) -> Result<(), String> {
    if r.report.trim().is_empty()
        || r.report.len() > MAX_CRASH_REPORT
        || r.build.len() > 64
        || r.os.len() > 64
    {
        return Err("that crash report doesn't look right".into());
    }
    {
        let mut recent = RECENT.get_or_init(Default::default).lock().unwrap();
        let times = recent.entry(crate::limit_key(ip)).or_default();
        times.retain(|t| t.elapsed() < Duration::from_secs(3600));
        if times.len() >= PER_HOUR {
            return Err("too many crash reports from here, thanks: they're in".into());
        }
        times.push(Instant::now());
    }
    let signature = signature(&r.report);
    if let Some(dir) = DIR.get() {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = serde_json::json!({ "at": at, "build": r.build, "os": r.os, "signature": signature, "report": r.report });
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("crashes.jsonl"))
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = written {
            eprintln!("crashes.jsonl: {e}");
        }
    }
    println!("crash report: {signature} (build {}, {})", r.build, r.os);
    std::thread::spawn(move || crate::discord::crash(&signature, &r));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_crash_has_the_same_signature() {
        let a = "--- taiko.rs crash at unix 1, build abc (0.1.0, windows)\nthread render: panicked at src/render.rs:120:9:\nindex out of bounds: the len is 3 but the index is 7\nstack backtrace:";
        let b = "--- taiko.rs crash at unix 2, build abc (0.1.0, windows)\nthread render: panicked at src/render.rs:120:9:\nindex out of bounds: the len is 4 but the index is 12\n";
        assert_eq!(signature(a), signature(b));
        assert_eq!(
            signature(a),
            "src/render.rs:120:9: index out of bounds: the len is N but the index is N"
        );
        let native = "--- taiko.rs crash at unix 3, build abc (0.1.0, windows)\naccess violation 0xC0000005 at igvk64.dll+0x13b702\nreading 0x0\n";
        assert_eq!(
            signature(native),
            "access violation 0xC0000005 at igvk64.dll"
        );
    }
}
