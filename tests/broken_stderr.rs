#![cfg(unix)]

use std::os::fd::FromRawFd;
use std::process::{Command, Stdio};

fn broken_pipe_stdio() -> Stdio {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid 2-element array for `pipe(2)`.
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    // Close the read end before spawning so writes to fd 2 immediately fail with EPIPE.
    // SAFETY: `fds[0]` is an open descriptor owned by this helper.
    assert_eq!(unsafe { libc::close(fds[0]) }, 0);
    // SAFETY: `fds[1]` is an open descriptor transferred exclusively to `Stdio`.
    unsafe { Stdio::from_raw_fd(fds[1]) }
}

/// Regression test for Linux `thread library` / `eprintln!` panics when launched with a closed
/// `stderr` pipe (`failed printing to stderr: Broken pipe (os error 32)`).
#[test]
fn eprintln_does_not_panic_on_closed_stderr_pipe() {
    let tmp = std::env::temp_dir().join(format!("taiko-rs-broken-stderr-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);

    let status = Command::new(env!("CARGO_BIN_EXE_taiko-rs"))
        .args(["--info", "/nonexistent-taiko-rs-broken-stderr-test.osu"])
        .env("XDG_CONFIG_HOME", &tmp)
        .env("XDG_DATA_HOME", &tmp)
        .stdout(Stdio::null())
        .stderr(broken_pipe_stdio())
        .status()
        .expect("spawn taiko-rs --info");

    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        status.success(),
        "taiko-rs --info must not panic when stderr is a broken pipe (got {status})"
    );
}
