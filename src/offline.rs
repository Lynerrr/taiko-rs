//! Tests never start other programs and never reach the internet. Whatever a test clicks or
//! presses, no browser, folder dialog, osu!, ffmpeg, clipboard tool or restart is started, and
//! HTTP goes only to this machine (the tests' own fake servers). Outside tests everything goes
//! through.

use std::process::Command;

/// The error a blocked request fails with in tests.
pub const BLOCKED: &str = "blocked in tests: no programs, no internet";

/// A program that doesn't exist: what tests "start" instead, so a spawn fails the way a missing
/// program does and the caller's own error path runs.
const NOWHERE: &str = "/nonexistent/taiko-rs-tests-start-no-programs";

/// Whether another program may be started.
pub fn may_spawn() -> bool {
    !cfg!(test)
}

/// `Command::new(program)`, except in tests, where it names a program that isn't there.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    if may_spawn() {
        Command::new(program)
    } else {
        Command::new(NOWHERE)
    }
}

/// Ok for a request to a fixed internet host (the mirrors, the website, GitHub), except in tests.
pub fn internet() -> Result<(), String> {
    if cfg!(test) {
        Err(BLOCKED.into())
    } else {
        Ok(())
    }
}

/// Ok for a request to `url`; in tests only to this machine.
pub fn url(url: &str) -> Result<(), String> {
    if !cfg!(test) || is_loopback(url) {
        Ok(())
    } else {
        Err(BLOCKED.into())
    }
}

fn is_loopback(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match host_port.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host_port.rsplit_once(':').map_or(host_port, |(h, _)| h),
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_reach_only_this_machine() {
        assert!(url("http://127.0.0.1:9/set").is_ok());
        assert!(url("http://localhost:8080").is_ok());
        assert!(url("http://[::1]:5000/x?y").is_ok());
        assert!(url("https://osu.direct/api/v2/search").is_err());
        assert!(url("https://127.0.0.1.evil.example/").is_err());
        assert!(url("https://localhost@evil.example/").is_err());
        assert_eq!(internet(), Err(BLOCKED.to_string()));
    }

    #[test]
    fn tests_start_no_programs() {
        assert!(!may_spawn());
        assert!(command("true").spawn().is_err());
    }
}
