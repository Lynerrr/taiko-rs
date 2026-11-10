//! The system around the game: the browser, the file manager, and the website's addresses.

use crate::{offline, online};

/// Opens a web page in the browser; false where that can't be done.
/// Where a player's profile is on the website.
pub(crate) fn profile_url(name: &str) -> String {
    format!("{}/u/{name}", online::WEBSITE)
}

/// A map's page on the website, by its hash.
pub(crate) fn map_url(hash: u64) -> String {
    format!("{}/m/{hash:x}", online::WEBSITE)
}

pub(crate) fn open_url(url: &str) -> bool {
    if cfg!(target_os = "android") || !offline::may_spawn() {
        return false;
    }
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                verb: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> *mut std::ffi::c_void;
        }
        const SW_SHOWNORMAL: i32 = 1;
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (verb, file) = (wide("open"), wide(url));
        // SAFETY: both strings are nul-terminated and live through the call; the rest may be null.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // Above 32 is success, the rest are error codes.
        result as usize > 32
    }
    #[cfg(not(windows))]
    offline::command("xdg-open")
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// `file://` URI for a local path.
#[cfg(not(windows))]
pub(crate) fn file_uri(path: &std::path::Path) -> String {
    let mut uri = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}
