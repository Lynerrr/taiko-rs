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
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use jni::objects::{JObject, JString, JValue};
use jni::sys::{jboolean, jint};
use jni::{EnvUnowned, JavaVM, NativeMethod, jni_sig, jni_str};
use winit::event_loop::EventLoopProxy;
use winit::keyboard::KeyCode;
use winit::platform::android::activity::AndroidApp;

use crate::UserEvent;
use crate::library::FileKind;

/// The running activity (for calling into its Java side) and where its keyboard's typing goes.
static APP: Mutex<Option<AndroidApp>> = Mutex::new(None);
static PROXY: Mutex<Option<EventLoopProxy<UserEvent>>> = Mutex::new(None);
type PickResult = Result<Option<PickedFile>, String>;
static FILE_PICK: OnceLock<Mutex<Option<Sender<PickResult>>>> = OnceLock::new();

/// The local copy of a file selected through Android's document picker.
pub struct PickedFile(pub PathBuf);

impl Drop for PickedFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

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
    match register_natives(app) {
        Ok(()) => {
            let _ = FILE_PICK.set(Mutex::new(None));
        }
        Err(e) => eprintln!("Android activity callbacks unavailable: {e}"),
    }
    let internal = app
        .internal_data_path()
        .unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let data = app
        .external_data_path()
        .filter(|p| std::fs::create_dir_all(p).is_ok())
        .unwrap_or_else(|| internal.clone());
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
    eprintln!("taiko.rs data: {}", data.join("taiko-rs").display());
    requeue_unfinished_imports();
}

/// Where the keyboard's typing goes (the game's event loop).
pub fn set_proxy(proxy: EventLoopProxy<UserEvent>) {
    *PROXY.lock().unwrap_or_else(|e| e.into_inner()) = Some(proxy);
}

/// Shows or hides the on-screen keyboard.
pub fn keyboard(show: bool) {
    let Some(app) = APP.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    let called = JavaVM::singleton().and_then(|vm| {
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            // SAFETY: a global reference to the activity, valid while it runs.
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            env.call_method(
                &activity,
                jni_str!("keyboard"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(show)],
            )?;
            Ok(())
        })
    });
    if let Err(e) = called {
        eprintln!("on-screen keyboard: {e}");
    }
}

/// How far in from the sides and from the top and bottom the screen's rounded corners and camera
/// reach, in pixels. None until the window is up.
pub fn safe_insets() -> Option<[u32; 2]> {
    let app = APP.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let packed = JavaVM::singleton()
        .and_then(|vm| {
            vm.attach_current_thread(|env| -> jni::errors::Result<i32> {
                // SAFETY: as in `keyboard`.
                let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
                env.call_method(&activity, jni_str!("safeInsets"), jni_sig!("()I"), &[])?
                    .i()
            })
        })
        .ok()?;
    (packed >= 0).then_some([packed as u32 >> 16, packed as u32 & 0xffff])
}

/// Lets the screen turn upright with the phone, or keeps it on its side.
pub fn allow_portrait(on: bool) {
    let Some(app) = APP.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    let called = JavaVM::singleton().and_then(|vm| {
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            // SAFETY: as in `keyboard`.
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            env.call_method(
                &activity,
                jni_str!("allowPortrait"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(on)],
            )?;
            Ok(())
        })
    });
    if let Err(e) = called {
        eprintln!("screen turning: {e}");
    }
}

/// What's playing, for the system's media controls.
pub struct NowPlaying<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    /// The difficulty.
    pub detail: &'a str,
    /// The map's background image ("": none).
    pub art: &'a str,
    pub length_ms: i64,
    pub at_ms: i64,
    /// How fast it plays (map time a second, in seconds).
    pub speed: f32,
    pub paused: bool,
}

/// Tells the system's media controls what's playing.
pub fn now_playing(song: &NowPlaying) {
    let Some(app) = APP.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    let called = JavaVM::singleton().and_then(|vm| {
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            // SAFETY: as in `keyboard`.
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            let title = env.new_string(song.title)?;
            let artist = env.new_string(song.artist)?;
            let detail = env.new_string(song.detail)?;
            let art = env.new_string(song.art)?;
            env.call_method(
                &activity,
                jni_str!("nowPlaying"),
                jni_sig!(
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJFZ)V"
                ),
                &[
                    JValue::Object(&title),
                    JValue::Object(&artist),
                    JValue::Object(&detail),
                    JValue::Object(&art),
                    JValue::Long(song.length_ms),
                    JValue::Long(song.at_ms),
                    JValue::Float(song.speed),
                    JValue::Bool(song.paused),
                ],
            )?;
            Ok(())
        })
    });
    if let Err(e) = called {
        eprintln!("media controls: {e}");
    }
}

/// Whether leaving the app should keep the music alive (the activity's `MusicService`).
pub fn keep_music(on: bool) {
    let Some(app) = APP.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    let called = JavaVM::singleton().and_then(|vm| {
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            // SAFETY: as in `keyboard`.
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            env.call_method(
                &activity,
                jni_str!("keepMusic"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(on)],
            )?;
            Ok(())
        })
    });
    if let Err(e) = called {
        eprintln!("background music: {e}");
    }
}

/// Asks the activity for one file and returns immediately. The activity copies the chosen
/// document before completing the request.
pub fn pick_file(kind: FileKind) -> Receiver<PickResult> {
    let (tx, rx) = mpsc::channel();
    let app = APP.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let (Some(app), Some(pick)) = (app, FILE_PICK.get()) else {
        let _ = tx.send(Err("file picker unavailable".into()));
        return rx;
    };
    {
        let mut pending = pick.lock().unwrap_or_else(|e| e.into_inner());
        if pending.is_some() {
            let _ = tx.send(Err("a file selection is already in progress".into()));
            return rx;
        }
        *pending = Some(tx);
    }
    let called = JavaVM::singleton().and_then(|vm| {
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            // SAFETY: a global reference to the activity, valid while it runs.
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            env.call_method(
                &activity,
                jni_str!("pickFile"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(kind == FileKind::Image)],
            )?;
            Ok(())
        })
    });
    if let Err(e) = called {
        eprintln!("file picker: {e}");
        finish_pick(Err("couldn't open the file picker".into()));
    }
    rx
}

fn finish_pick(result: PickResult) {
    if let Some(pick) = FILE_PICK.get()
        && let Some(tx) = pick.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        let _ = tx.send(result);
    }
}

/// Binds the activity's `native` methods to their Rust side here.
fn register_natives(app: &AndroidApp) -> jni::errors::Result<()> {
    JavaVM::singleton()?.attach_current_thread(|env| -> jni::errors::Result<()> {
        // SAFETY: as in `keyboard`.
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        let class = env.get_object_class(&activity)?;
        // SAFETY: the functions take what the Java declarations say (an instance method's
        // `this`, then the arguments).
        let methods = unsafe {
            [
                NativeMethod::from_raw_parts(
                    jni_str!("typed"),
                    jni_str!("(Ljava/lang/String;I)V"),
                    typed as *mut _,
                ),
                NativeMethod::from_raw_parts(
                    jni_str!("keyboardShown"),
                    jni_str!("(Z)V"),
                    keyboard_shown as *mut _,
                ),
                NativeMethod::from_raw_parts(
                    jni_str!("media"),
                    jni_str!("(IJ)V"),
                    media_press as *mut _,
                ),
                NativeMethod::from_raw_parts(
                    jni_str!("filePicked"),
                    jni_str!("(Ljava/lang/String;Ljava/lang/String;)V"),
                    file_picked as *mut _,
                ),
            ]
        };
        unsafe { env.register_native_methods(&class, &methods) }
    })
}

extern "system" fn file_picked<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    path: JString<'local>,
    error: JString<'local>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let result = if !path.is_null() {
            path.try_to_string(env)
                .map(|path| Some(PickedFile(PathBuf::from(path))))
                .map_err(|e| e.to_string())
        } else if !error.is_null() {
            Err(error
                .try_to_string(env)
                .unwrap_or_else(|_| "couldn't read the chosen file".into()))
        } else {
            Ok(None)
        };
        finish_pick(result);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// `TaikoActivity.typed(text, keyCode)`: text typed on the keyboard, or (text null) a key.
extern "system" fn typed<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    text: JString<'local>,
    key: jint,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let typed = if text.is_null() {
            let Some(code) = android_key(key) else {
                return Ok(());
            };
            Typed::Key(code)
        } else {
            let text = text.try_to_string(env)?;
            Typed::Keys(
                text.chars()
                    .map(|ch| (key_for(ch), ch.to_string()))
                    .collect(),
            )
        };
        send(typed);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// `TaikoActivity.keyboardShown(shown)`.
extern "system" fn keyboard_shown<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    shown: jboolean,
) {
    env.with_env(|_| -> jni::errors::Result<()> {
        send(Typed::Shown(shown));
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// `TaikoActivity.media(what, ms)`.
extern "system" fn media_press<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    what: jint,
    ms: jni::sys::jlong,
) {
    env.with_env(|_| -> jni::errors::Result<()> {
        // Not through the event loop: it sleeps with the app in the background, and this is
        // when they're pressed. The game takes them each frame, background ones too.
        MEDIA
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((what, ms));
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// Presses of the media controls not yet taken.
static MEDIA: Mutex<Vec<(i32, i64)>> = Mutex::new(Vec::new());

/// The media controls' presses since the last call: what (`MusicService`) and where to.
pub fn take_media() -> Vec<(i32, i64)> {
    std::mem::take(&mut *MEDIA.lock().unwrap_or_else(|e| e.into_inner()))
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
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    const DIGITS: [KeyCode; 10] = [
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ];
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
    let Some(entries) = import_dir().and_then(|d| std::fs::read_dir(d).ok()) else {
        return Vec::new();
    };
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
    let Some(entries) = import_dir().and_then(|d| std::fs::read_dir(d).ok()) else {
        return;
    };
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
        fn __android_log_write(
            prio: i32,
            tag: *const std::ffi::c_char,
            text: *const std::ffi::c_char,
        ) -> i32;
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
    let _ = std::thread::Builder::new()
        .name("logcat".into())
        .spawn(move || {
            let tag = c"taiko-rs";
            for line in BufReader::new(read).lines().map_while(Result::ok) {
                if let Ok(text) = std::ffi::CString::new(line) {
                    // SAFETY: both strings are NUL-terminated and outlive the call.
                    unsafe { __android_log_write(ANDROID_LOG_INFO, tag.as_ptr(), text.as_ptr()) };
                }
            }
        });
}
