//! Crash reports: the panic hook, Windows' native crashes, the crash window, and sending
//! last time's report in.

#[allow(unused_imports)]
use crate::*;

/// The latest crash's report, kept until the next start has offered to copy it.
pub(super) const CRASH_REPORT: &str = "crash-report.txt";

/// A crash leaves `crash.log` next to the scores (release builds on Windows have no console to
/// print it to), and the report on its own for the next start to offer (`crash_report`). On
/// Windows the report also goes on the clipboard and a message box says to send it in, since
/// the game just vanishes otherwise.
pub(super) fn log_crashes() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let text = format!(
            "--- taiko.rs crash at unix {at}, build {} ({}, {})\nthread {thread}: {info}\n{}\n{}",
            update::BUILD.unwrap_or("local"),
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::backtrace::Backtrace::force_capture(),
            crash_context(),
        );
        // A release build's backtrace has no names on Windows: where each frame is, for
        // finding them with that build's .pdb (docs/ci.md).
        #[cfg(windows)]
        let text = format!("{text}{}", native_crashes::frames());
        if let Some(dir) = config::data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("crash.log"))
            {
                let _ = std::io::Write::write_all(&mut f, text.as_bytes());
            }
            if cfg!(panic = "abort") {
                let _ = std::fs::write(dir.join(CRASH_REPORT), &text);
            }
        }
        default(info);
        // Only where a panic ends the game (release builds): a debug build's caught one isn't a crash.
        #[cfg(all(windows, panic = "abort"))]
        {
            // A clipboard of its own: the game's may be what was held when it crashed.
            let copied = arboard::Clipboard::new()
                .and_then(|mut b| b.set_text(text.clone()))
                .is_ok();
            let what = if copied {
                "The crash log is copied: paste it (Ctrl+V)"
            } else {
                "Send the crash log (crash.log in the game's data folder)"
            };
            let what = format!("{what} in #bugs on the taiko.rs Discord, so it can be fixed.");
            crash_box(&format!(
                "taiko.rs crashed.\n\n{}\n\n{}",
                crash_box_words(&what),
                info
            ));
        }
    }));
}

/// Windows crashes that aren't panics: an access violation (often in a graphics or audio driver),
/// a stack overflow, an abort. The panic hook never sees those, and a release build has no
/// console, so the game just vanished with nothing left behind. Now what it printed goes to
/// `game.log` (the last run's is kept as `game-prev.log`), and an unhandled exception leaves the
/// same `crash.log` entry and next-start report a panic does, with the exception's code, where
/// it happened (which module) and the end of the log.
#[cfg(windows)]
pub(crate) mod native_crashes {
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        record: *mut ExceptionRecord,
        address: *mut c_void,
        parameters: u32,
        information: [usize; 15],
    }

    #[repr(C)]
    struct ExceptionPointers {
        record: *mut ExceptionRecord,
        context: *mut c_void,
    }

    type Filter = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
        fn SetStdHandle(which: u32, handle: *mut c_void) -> i32;
        fn GetModuleHandleExW(flags: u32, address: *const c_void, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    }

    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;
    const CONTINUE_SEARCH: i32 = 0;

    static DIR: OnceLock<PathBuf> = OnceLock::new();

    pub fn install() {
        let Some(dir) = crate::config::data_dir() else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("game.log");
        let _ = std::fs::rename(&log, dir.join("game-prev.log"));
        if let Ok(file) = std::fs::File::create(&log) {
            use std::os::windows::io::IntoRawHandle;
            // std looks the handle up on every write, so eprintln! lands in the file from here
            // on, and so does std's own last word on a stack overflow or an abort.
            // SAFETY: the handle is a file we own and never close (it lives as long as the game).
            unsafe {
                SetStdHandle(STD_ERROR_HANDLE, file.into_raw_handle());
            }
        }
        let _ = DIR.set(dir);
        // SAFETY: `filter` matches the signature Windows calls it with.
        unsafe {
            SetUnhandledExceptionFilter(Some(filter));
        }
        // To see this work: a crash of the kind a driver has.
        if std::env::var_os("TAIKO_TEST_NATIVE_CRASH").is_some() {
            eprintln!("crashing on purpose (TAIKO_TEST_NATIVE_CRASH)");
            // SAFETY: not safe at all, on purpose: it's the access violation being tested.
            unsafe { std::ptr::null_mut::<u8>().write_volatile(1) };
        }
    }

    fn what(code: u32) -> &'static str {
        match code {
            0xC000_0005 => "access violation",
            0xC000_00FD => "stack overflow",
            0xC000_001D => "illegal instruction",
            0xC000_0094 => "integer divide by zero",
            0xC000_0409 => "fail fast (abort)",
            0xC000_0374 => "heap corruption",
            0x8000_0003 => "breakpoint",
            _ => "exception",
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn RtlCaptureStackBackTrace(
            skip: u32,
            count: u32,
            frames: *mut *mut c_void,
            hash: *mut u32,
        ) -> u16;
    }

    /// This thread's stack as `frames: taiko-rs.exe+0x1a2b3c ...` (the module and offset of
    /// each return address): files and lines with the build's .pdb (docs/ci.md).
    pub fn frames() -> String {
        let mut frames = [std::ptr::null_mut::<c_void>(); 62];
        // SAFETY: the buffer holds the 62 frames asked for.
        let n = unsafe {
            RtlCaptureStackBackTrace(
                1,
                frames.len() as u32,
                frames.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        } as usize;
        let mut out = String::from("frames:");
        for f in &frames[..n.min(frames.len())] {
            out.push(' ');
            out.push_str(&module_of(*f));
        }
        out.push('\n');
        out
    }

    /// The DLL or exe `address` is in, and how far into it.
    fn module_of(address: *const c_void) -> String {
        let mut module = std::ptr::null_mut();
        // SAFETY: plain lookups with a valid out pointer and buffer.
        unsafe {
            if GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, address, &mut module) == 0 {
                return format!("{address:p} (in no module)");
            }
            let mut name = [0u16; 260];
            let n = GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize;
            let path = String::from_utf16_lossy(&name[..n.min(name.len())]);
            let file = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
            format!("{file}+0x{:x}", address as usize - module as usize)
        }
    }

    unsafe extern "system" fn filter(info: *mut ExceptionPointers) -> i32 {
        // SAFETY: Windows passes valid exception pointers to an unhandled exception filter.
        let Some(record) = (unsafe { info.as_ref().and_then(|i| i.record.as_ref()) }) else {
            return CONTINUE_SEARCH;
        };
        let Some(dir) = DIR.get() else {
            return CONTINUE_SEARCH;
        };
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut text = format!(
            "--- taiko.rs crash at unix {at}, build {} ({}, windows)\n{} 0x{:08X} at {}\n",
            crate::update::BUILD.unwrap_or("local"),
            env!("CARGO_PKG_VERSION"),
            what(record.code),
            record.code,
            module_of(record.address),
        );
        if record.code == 0xC000_0005 && record.parameters >= 2 {
            let op = match record.information[0] {
                0 => "reading",
                1 => "writing",
                _ => "running",
            };
            text.push_str(&format!("{op} 0x{:x}\n", record.information[1]));
        }
        text.push_str(&super::crash_context());
        let frames = frames();
        text.push_str(&frames);
        let hook = crate::app::gpu::drawing_hook(&frames);
        if let Some((dll, who)) = hook {
            text.push_str(&format!("in a hook: {who} ({dll})\n"));
        }
        let to_dx12 = hook.is_some()
            && crate::app::gpu::VULKAN_ON_AUTO.load(std::sync::atomic::Ordering::Relaxed);
        if to_dx12 {
            crate::app::gpu::gpu_start::hook_crashed();
        }
        // What the game printed last: the screen, the audio device, the map...
        if let Ok(log) = std::fs::read_to_string(dir.join("game.log")) {
            let lines: Vec<&str> = log.lines().collect();
            text.push_str("last lines of game.log:\n");
            for line in &lines[lines.len().saturating_sub(25)..] {
                text.push_str(line);
                text.push('\n');
            }
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("crash.log"))
        {
            let _ = std::io::Write::write_all(&mut f, text.as_bytes());
        }
        let _ = std::fs::write(dir.join(super::CRASH_REPORT), &text);
        let words = super::crash_box_words(
            "The next start offers to copy the crash log: paste it in #bugs on the taiko.rs Discord, so it can be fixed.",
        );
        let advice = hook
            .map(|(dll, who)| {
                format!(
                    "{}\n\n",
                    crate::app::gpu::drawing_hook_advice(
                        who,
                        dll,
                        to_dx12,
                        crate::app::gpu::VULKAN.load(std::sync::atomic::Ordering::Relaxed)
                    )
                )
            })
            .unwrap_or_default();
        super::crash_box(&format!(
            "taiko.rs crashed.\n\n{advice}{words}\n\n{} 0x{:08X}",
            what(record.code),
            record.code
        ));
        // Windows' own handling goes on (its error report, and the end of the process).
        CONTINUE_SEARCH
    }
}

/// A message box (Windows), there even without a window of the game's.
#[cfg(windows)]
pub(super) fn crash_box(text: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            hwnd: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            kind: u32,
        ) -> i32;
    }
    const MB_ICONERROR: u32 = 0x10;
    const MB_TOPMOST: u32 = 0x4_0000;
    let wide = |s: &str| {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let (text, caption) = (wide(text), wide("taiko.rs crashed"));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_ICONERROR | MB_TOPMOST,
        );
    }
}

/// Whether a crash's report goes to the server on the next start (`Config::send_crash_reports`),
/// for the crash handlers, which can't reach the config.
pub(crate) static SEND_CRASH_REPORTS: AtomicBool = AtomicBool::new(true);

/// What the game is running on (the graphics card and API), for crash reports: set once the
/// graphics have started.
pub(super) static CRASH_CONTEXT: OnceLock<String> = OnceLock::new();

/// The screen (and map and mods) when it crashed: set as screens change, never under the
/// state's lock (the crashing thread may hold that one).
pub(crate) static CRASH_SCENE: Mutex<String> = Mutex::new(String::new());

pub(super) fn crash_context() -> String {
    let mut out = CRASH_CONTEXT
        .get()
        .map(|c| format!("{c}\n"))
        .unwrap_or_default();
    if let Ok(scene) = CRASH_SCENE.try_lock()
        && !scene.is_empty()
    {
        out.push_str(&scene);
        out.push('\n');
    }
    out
}

/// The crash window's words on what happens to the report: `otherwise` is how to send it in
/// by hand, when the game doesn't.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn crash_box_words(otherwise: &str) -> String {
    if SEND_CRASH_REPORTS.load(Ordering::Relaxed) {
        "The next start sends the crash report to the developers by itself, so it can be fixed (Settings > Online > Send crash reports turns that off).".into()
    } else {
        otherwise.into()
    }
}

/// Sends last time's crash report to the server in the background: true when it's on its way.
/// The player's folder names and system user name are taken out first (`scrub_report`).
pub(crate) fn send_crash_report(server: &str, report: &str) -> bool {
    if update::BUILD.is_none() {
        // A local build's crashes are the developer's own, seen already.
        return false;
    }
    let mut text = scrub_report(report, &system_user_name());
    if text.len() > online::MAX_CRASH_REPORT {
        // The start (what crashed) and the end (the log's last lines) say most.
        let head: String = text.chars().take(online::MAX_CRASH_REPORT / 2).collect();
        let tail: String = text
            .chars()
            .rev()
            .take(online::MAX_CRASH_REPORT / 2 - 100)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        text = format!("{head}\n[...]\n{tail}");
    }
    let r = online::CrashReport {
        build: update::BUILD.unwrap_or("local").to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        report: text,
    };
    let server = server.to_string();
    std::thread::spawn(move || {
        match online::call(&server, online::Request::CrashReport(Box::new(r))) {
            Ok(_) => eprintln!("crash report sent"),
            Err(e) => eprintln!("crash report not sent: {e}"),
        }
    });
    true
}

/// A crash report as it may leave the computer: no user folders in its paths
/// (`online::scrub_paths`) and no system user name (`user`) anywhere else.
pub(crate) fn scrub_report(report: &str, user: &str) -> String {
    online::scrub_user_name(&online::scrub_paths(report), user)
}

/// Who is logged in to the system: `USERNAME` on Windows, `USER` or `LOGNAME` elsewhere.
fn system_user_name() -> String {
    ["USERNAME", "USER", "LOGNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_default()
}

/// The report of a crash last time that hasn't been offered yet (and forgets it).
pub(super) fn crash_report() -> Option<String> {
    let file = config::data_dir()?.join(CRASH_REPORT);
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    Some(text).filter(|t| !t.trim().is_empty())
}
