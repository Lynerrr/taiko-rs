//! Android glue. The rest of the game finds its folders through `HOME` / `XDG_*` like on Linux,
//! so those are pointed at the app's storage before anything else runs:
//!
//! - settings, scores, skins and songs: the app's external files folder
//!   (`Android/data/<package>/files/taiko-rs`, reachable over USB), else its internal one;
//! - caches (extracted .osz files) and temporary files: internal storage.
//!
//! stderr, where the game logs, is forwarded to logcat (tag `taiko-rs`).

use std::path::PathBuf;

use winit::platform::android::activity::AndroidApp;

pub fn setup(app: &AndroidApp) {
    forward_stderr_to_logcat();
    let internal = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let data = app.external_data_path().filter(|p| std::fs::create_dir_all(p).is_ok()).unwrap_or_else(|| internal.clone());
    let cache = internal.join("cache");
    let tmp = internal.join("tmp");
    for dir in [&cache, &tmp, &data.join("taiko-rs").join("Songs")] {
        let _ = std::fs::create_dir_all(dir);
    }
    // SAFETY: called first thing in android_main, before the game starts any thread of its own.
    unsafe {
        std::env::set_var("HOME", &data);
        std::env::set_var("XDG_CONFIG_HOME", &data);
        std::env::set_var("XDG_DATA_HOME", &data);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("TMPDIR", &tmp);
    }
    eprintln!("taiko-rs data: {}", data.join("taiko-rs").display());
    requeue_unfinished_imports();
}

/// The game's own Songs folder (where downloads go and you can copy maps or .osz files).
pub fn songs_dir() -> Option<PathBuf> {
    Some(crate::config::user_dir(false)?.join("Songs"))
}

/// Where the Java activity (`android/TaikoActivity.java`) copies `.osz` files opened with the
/// game: `<name>.osz.part` while copying, renamed to `<name>.osz` when complete.
fn import_dir() -> Option<PathBuf> {
    Some(crate::config::user_dir(false)?.join("import"))
}

/// Archives waiting in the import folder, with their names. Each is renamed to `.importing`
/// so it is only taken once; the import removes it when done.
pub fn take_imports() -> Vec<(PathBuf, String)> {
    let Some(entries) = import_dir().and_then(|d| std::fs::read_dir(d).ok()) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension()? != "osz" {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let taken = path.with_extension("osz.importing");
            std::fs::rename(&path, &taken).ok()?;
            Some((taken, name))
        })
        .collect()
}

/// Puts back imports a previous run took but didn't finish (the game was closed meanwhile).
fn requeue_unfinished_imports() {
    let Some(entries) = import_dir().and_then(|d| std::fs::read_dir(d).ok()) else { return };
    for path in entries.flatten().map(|e| e.path()) {
        if let Some(name) = path.to_str().and_then(|p| p.strip_suffix(".importing")) {
            let _ = std::fs::rename(&path, name);
        }
    }
}

/// Points stderr (and stdout) at a pipe whose lines a thread writes to logcat.
fn forward_stderr_to_logcat() {
    use std::io::{BufRead, BufReader};
    use std::os::fd::FromRawFd;

    unsafe extern "C" {
        fn __android_log_write(prio: i32, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> i32;
    }
    const ANDROID_LOG_INFO: i32 = 4;

    let mut fds = [0; 2];
    // SAFETY: plain libc calls on descriptors this function owns; the read end moves into the
    // logging thread, which is its only user.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        libc::dup2(fds[1], libc::STDERR_FILENO);
        libc::dup2(fds[1], libc::STDOUT_FILENO);
        libc::close(fds[1]);
    }
    let read = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let _ = std::thread::Builder::new().name("logcat".into()).spawn(move || {
        let tag = c"taiko-rs";
        for line in BufReader::new(read).lines().map_while(Result::ok) {
            if let Ok(text) = std::ffi::CString::new(line) {
                // SAFETY: both strings are NUL-terminated and outlive the call.
                unsafe { __android_log_write(ANDROID_LOG_INFO, tag.as_ptr(), text.as_ptr()) };
            }
        }
    });
}
