//! Android glue. The rest of the game finds its folders through `HOME` / `XDG_*` like on Linux,
//! so those are pointed at the app's storage before anything else runs:
//!
//! - settings, scores, skins and songs: the app's external files folder
//!   (`Android/data/<package>/files/taiko-rs`, reachable over USB), else its internal one;
//! - caches (extracted .osz files) and temporary files: internal storage.
//!
//! stderr, where the game logs, is forwarded to logcat (tag `taiko-rs`).
//!
//! The on-screen keyboard is the Java activity's (`android/TaikoActivity.java`): [`keyboard`]
//! shows or hides it, and what is typed on it comes back as [`Typed`] events.

use std::path::PathBuf;
use std::sync::Mutex;

use jni::objects::{JObject, JString, JValue};
use jni::sys::{jboolean, jint};
use jni::{EnvUnowned, JavaVM, NativeMethod, jni_sig, jni_str};
use winit::event_loop::EventLoopProxy;
use winit::keyboard::KeyCode;
use winit::platform::android::activity::AndroidApp;

use crate::UserEvent;

/// The running activity (for calling into its Java side) and where its keyboard's typing goes.
static APP: Mutex<Option<AndroidApp>> = Mutex::new(None);
static PROXY: Mutex<Option<EventLoopProxy<UserEvent>>> = Mutex::new(None);

/// The on-screen keyboard, as the game sees it.
#[derive(Default)]
pub struct Keyboard {
    /// A text field was tapped (and is still there to type in).
    pub asked: bool,
    /// Whether it was last asked to show.
    pub wanted: bool,
    /// It's up (as asked, or as it said: the player can put it away).
    pub up: bool,
}

/// From the on-screen keyboard.
#[derive(Debug)]
pub enum Typed {
    /// Text typed on it, as presses of the keys a (US) keyboard would type it with.
    Keys(Vec<(KeyCode, String)>),
    /// A key without text: Backspace, Enter (its action button), arrows.
    Key(KeyCode),
    /// It came up or went away (the player can close it themselves).
    Shown(bool),
}

pub fn setup(app: &AndroidApp) {
    forward_stderr_to_logcat();
    *APP.lock().unwrap_or_else(|e| e.into_inner()) = Some(app.clone());
    if let Err(e) = register_natives(app) {
        eprintln!("on-screen keyboard unavailable: {e}");
    }
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

/// Where the keyboard's typing goes (the game's event loop).
pub fn set_proxy(proxy: EventLoopProxy<UserEvent>) {
    *PROXY.lock().unwrap_or_else(|e| e.into_inner()) = Some(proxy);
}

/// Shows or hides the on-screen keyboard.
pub fn keyboard(show: bool) {
    let Some(app) = APP.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
    let called = JavaVM::singleton().and_then(|vm| {
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            // SAFETY: a global reference to the activity, valid while it runs.
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            env.call_method(&activity, jni_str!("keyboard"), jni_sig!("(Z)V"), &[JValue::Bool(show)])?;
            Ok(())
        })
    });
    if let Err(e) = called {
        eprintln!("on-screen keyboard: {e}");
    }
}

/// Binds the activity's `native` methods (the keyboard's) to their Rust side here.
fn register_natives(app: &AndroidApp) -> jni::errors::Result<()> {
    JavaVM::singleton()?.attach_current_thread(|env| -> jni::errors::Result<()> {
        // SAFETY: as in `keyboard`.
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        let class = env.get_object_class(&activity)?;
        // SAFETY: the functions take what the Java declarations say (an instance method's
        // `this`, then the arguments).
        let methods = unsafe {
            [
                NativeMethod::from_raw_parts(jni_str!("typed"), jni_str!("(Ljava/lang/String;I)V"), typed as *mut _),
                NativeMethod::from_raw_parts(jni_str!("keyboardShown"), jni_str!("(Z)V"), keyboard_shown as *mut _),
            ]
        };
        unsafe { env.register_native_methods(&class, &methods) }
    })
}

/// `TaikoActivity.typed(text, keyCode)`: text typed on the keyboard, or (text null) a key.
extern "system" fn typed<'local>(mut env: EnvUnowned<'local>, _this: JObject<'local>, text: JString<'local>, key: jint) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let typed = if text.is_null() {
            let Some(code) = android_key(key) else { return Ok(()) };
            Typed::Key(code)
        } else {
            let text = text.try_to_string(env)?;
            Typed::Keys(text.chars().map(|ch| (key_for(ch), ch.to_string())).collect())
        };
        send(typed);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// `TaikoActivity.keyboardShown(shown)`.
extern "system" fn keyboard_shown<'local>(mut env: EnvUnowned<'local>, _this: JObject<'local>, shown: jboolean) {
    env.with_env(|_| -> jni::errors::Result<()> {
        send(Typed::Shown(shown));
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

fn send(typed: Typed) {
    if let Some(proxy) = &*PROXY.lock().unwrap_or_else(|e| e.into_inner()) {
        let _ = proxy.send_event(UserEvent::Typed(typed));
    }
}

/// The keys without text the keyboard sends (Android's `KeyEvent.KEYCODE_*`).
fn android_key(code: jint) -> Option<KeyCode> {
    Some(match code {
        19 => KeyCode::ArrowUp,
        20 => KeyCode::ArrowDown,
        21 => KeyCode::ArrowLeft,
        22 => KeyCode::ArrowRight,
        61 => KeyCode::Tab,
        66 => KeyCode::Enter,
        67 => KeyCode::Backspace,
        92 => KeyCode::PageUp,
        93 => KeyCode::PageDown,
        111 => KeyCode::Escape,
        112 => KeyCode::Delete,
        122 => KeyCode::Home,
        123 => KeyCode::End,
        160 => KeyCode::NumpadEnter,
        _ => return None,
    })
}

/// The key a US keyboard types `ch` with, so what reads key codes (the digits of a size) works
/// as with a real keyboard. Characters it has no key for go on a key nothing uses.
pub fn key_for(ch: char) -> KeyCode {
    use KeyCode::*;
    const LETTERS: [KeyCode; 26] = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV,
        KeyW, KeyX, KeyY, KeyZ,
    ];
    const DIGITS: [KeyCode; 10] = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
    match ch {
        'a'..='z' => LETTERS[ch as usize - 'a' as usize],
        'A'..='Z' => LETTERS[ch as usize - 'A' as usize],
        '0'..='9' => DIGITS[ch as usize - '0' as usize],
        ')' => Digit0,
        '!' => Digit1,
        '@' => Digit2,
        '#' => Digit3,
        '$' => Digit4,
        '%' => Digit5,
        '^' => Digit6,
        '&' => Digit7,
        '*' => Digit8,
        '(' => Digit9,
        ' ' => Space,
        '\n' => Enter,
        '\t' => Tab,
        '-' | '_' => Minus,
        '=' | '+' => Equal,
        '[' | '{' => BracketLeft,
        ']' | '}' => BracketRight,
        '\\' | '|' => Backslash,
        ';' | ':' => Semicolon,
        '\'' | '"' => Quote,
        ',' | '<' => Comma,
        '.' | '>' => Period,
        '/' | '?' => Slash,
        '`' | '~' => Backquote,
        _ => Lang1,
    }
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
