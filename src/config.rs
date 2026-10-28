//! Settings persisted as a plain `key = value` file: `$XDG_CONFIG_HOME/taiko-rs/config.txt` on
//! Linux, `%APPDATA%\taiko-rs\config.txt` on Windows.

use std::path::{Path, PathBuf};

use winit::keyboard::KeyCode;

use crate::layout::Area;
use crate::mods::Mods;
use crate::render::PresentChoice;

pub const RATE_RANGE: (f64, f64) = (0.5, 2.0);
pub const FPS_CAPS: [u32; 7] = [0, 2000, 1000, 500, 240, 144, 60];
/// What scroll speed 1.0 was before it matched osu!, in today's terms (at 16:9: 0.7 virtual
/// units per osu!px across a 639-unit lane, where osu! takes its 693-unit one at 1.0).
const OLD_SCROLL_SPEED: f64 = 0.7 * 693.333 / 639.333;
/// F3 / F4 and the settings change scroll speed in steps of this.
const SCROLL_STEP: f64 = 0.05;

/// Scroll speed one step up (`d` = 1) or down (-1). A speed between steps goes to the nearest
/// step in that direction first, so 1.01 goes down to 1.00 and up to 1.05.
pub fn step_scroll_speed(speed: f64, d: f64) -> f64 {
    let k = speed / SCROLL_STEP;
    let k = if d > 0.0 { (k + 1e-6).floor() + 1.0 } else { (k - 1e-6).ceil() - 1.0 };
    (k * SCROLL_STEP).clamp(0.3, 4.0)
}

/// Order of the four touch columns, left to right (K = rim / kat, D = centre / don), for the
/// kddk and kkdd playstyles and their mirrors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TouchLayout {
    Kddk,
    Kkdd,
    Dkkd,
    Ddkk,
}

impl TouchLayout {
    pub const ALL: [TouchLayout; 4] = [TouchLayout::Kddk, TouchLayout::Kkdd, TouchLayout::Dkkd, TouchLayout::Ddkk];

    pub fn name(self) -> &'static str {
        match self {
            TouchLayout::Kddk => "KDDK",
            TouchLayout::Kkdd => "KKDD",
            TouchLayout::Dkkd => "DKKD",
            TouchLayout::Ddkk => "DDKK",
        }
    }

    fn parse(s: &str) -> Option<TouchLayout> {
        TouchLayout::ALL.into_iter().find(|l| l.name().eq_ignore_ascii_case(s))
    }

    /// The layout `dir` steps away (wrapping).
    pub fn step(self, dir: i32) -> TouchLayout {
        let i = TouchLayout::ALL.iter().position(|&l| l == self).unwrap_or(0) as i32;
        TouchLayout::ALL[(i + dir).rem_euclid(4) as usize]
    }

    /// The drum key (0 left rim, 1 left centre, 2 right centre, 3 right rim) of each column.
    /// Two columns of one colour always map to different hands, so hitting both is a big hit.
    pub fn keys(self) -> [usize; 4] {
        match self {
            TouchLayout::Kddk => [0, 1, 2, 3],
            TouchLayout::Kkdd => [0, 3, 1, 2],
            TouchLayout::Dkkd => [1, 0, 3, 2],
            TouchLayout::Ddkk => [1, 2, 0, 3],
        }
    }
}

/// Where an F12 screenshot goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenshotTo {
    Clipboard,
    Folder,
    Both,
}

impl ScreenshotTo {
    pub const ALL: [ScreenshotTo; 3] = [ScreenshotTo::Clipboard, ScreenshotTo::Folder, ScreenshotTo::Both];

    pub fn name(self) -> &'static str {
        match self {
            ScreenshotTo::Clipboard => "Clipboard",
            ScreenshotTo::Folder => "Screenshots folder",
            ScreenshotTo::Both => "Clipboard and folder",
        }
    }

    /// Its name in the config file.
    fn key(self) -> &'static str {
        match self {
            ScreenshotTo::Clipboard => "clipboard",
            ScreenshotTo::Folder => "folder",
            ScreenshotTo::Both => "both",
        }
    }

    fn parse(s: &str) -> Option<ScreenshotTo> {
        ScreenshotTo::ALL.into_iter().find(|t| t.key().eq_ignore_ascii_case(s))
    }

    /// The choice `dir` steps away (wrapping).
    pub fn step(self, dir: i32) -> ScreenshotTo {
        let i = ScreenshotTo::ALL.iter().position(|&t| t == self).unwrap_or(0) as i32;
        ScreenshotTo::ALL[(i + dir).rem_euclid(3) as usize]
    }

    pub fn clipboard(self) -> bool {
        self != ScreenshotTo::Folder
    }

    pub fn folder(self) -> bool {
        self != ScreenshotTo::Clipboard
    }
}

pub struct Config {
    /// Columns of the touch drum, left to right.
    pub touch_layout: TouchLayout,
    /// Where F12 screenshots go.
    pub screenshot_to: ScreenshotTo,
    /// Positive when your audio reaches you late; notes are shifted later by this much.
    pub offset_ms: f64,
    /// Notes are drawn this many ms ahead to make up for display latency (judging is unaffected).
    pub visual_offset_ms: f64,
    /// With vsync, notes drawn where they'll be when the frame is shown (`State::vsync_lead`),
    /// and the player's adjustment of that (ms, on top).
    pub vsync_compensation: bool,
    pub vsync_tune_ms: f64,
    pub scroll_speed: f64,
    /// Scales both volumes below (Alt + mouse wheel anywhere).
    pub master_volume: f32,
    pub music_volume: f32,
    pub fx_volume: f32,
    /// Present mode during songs.
    pub present: PresentChoice,
    /// Draw at half resolution and scale up exactly 2x (default on Android: high-DPI screens,
    /// mobile GPUs).
    pub half_resolution: bool,
    /// Everything but the player playing a song (menus, results, replays, autoplay) uses vsync.
    pub menu_vsync: bool,
    /// The mouse wheel resizes the playfield during play (it always does in its editor).
    pub wheel_resizes_playfield: bool,
    /// During songs, the background dims to black (and is black without one), rather than to
    /// the interface's grey.
    pub bg_dim_black: bool,
    /// The interface during play is hidden (the toggle), and what that hides.
    pub interface_hidden: bool,
    pub interface_hides: crate::hud::Parts,
    /// The interface toggle's key, with Shift or not.
    pub interface_key: (bool, KeyCode),
    /// 0 = uncapped. Only applies to mailbox/immediate; vsync paces itself.
    pub fps_cap: u32,
    /// Vsync only: wait this long after the previous frame before drawing the next, so the frame is
    /// sampled closer to the compositor's deadline (lower latency; too high causes stutter).
    /// Tempo of the offset-calibration click track.
    pub calibration_bpm: f64,
    /// Where gameplay is drawn on screen (fractions; full screen by default).
    pub playfield_area: Area,
    /// How much the map background is darkened during play (1.0 = hidden).
    pub bg_dim: f32,
    /// Speed modifier (1.5 = DT, 0.75 = HT) and whether it keeps the music's pitch.
    pub rate: f64,
    pub rate_keep_pitch: bool,
    /// Show a live offset suggestion during play.
    pub show_offset_hint: bool,
    /// The offset/UR line during play (Tab hides it).
    pub show_map_info: bool,
    /// The live scoreboard in lobby matches (Tab there hides it).
    pub mp_scoreboard: bool,
    /// Mods picked in the mod select (F1 on song select).
    pub mods: Mods,
    /// Multiplayer: display name, port to host on, last address joined.
    pub mp_name: String,
    pub mp_port: u16,
    pub mp_address: String,
    /// The online server (accounts, lobbies), and the saved login there.
    pub online_server: String,
    pub online_user: String,
    pub online_token: String,
    /// Scale that suggestion to current UR (off: plain average of the last 40 hits).
    pub offset_hint_ur_scaled: bool,
    pub audio_buffer: u32,
    pub show_fps: bool,
    /// Kiai time effects on the lane (tint, pulsing glow).
    pub kiai_effects: bool,
    /// How autoplay hits: playstyle, binding and hand (taiko_arranger's).
    pub autoplay: crate::autoplay::Settings,
    /// GREAT hits without their flash and label (only OKs and misses show).
    pub hide_great: bool,
    /// The interface's accent colour (`render::ACCENTS`, or any other).
    pub accent: u32,
    /// Show what you're doing in your Discord status.
    pub discord_presence: bool,
    /// Star out swear words in chats.
    pub chat_filter: bool,
    /// A chime when a friend messages you.
    pub message_sound: bool,
    /// Pop-ups when a friend starts a map (off by default: busy friend lists spam them).
    pub notify_playing: bool,
    /// Pop-ups when a friend comes online.
    pub notify_online: bool,
    /// Pop-ups for friends' messages.
    pub notify_messages: bool,
    /// Pop-ups for global chat lines that mention you.
    pub notify_mentions: bool,
    /// Pop-ups for friend requests.
    pub notify_requests: bool,
    /// Discord application whose name the status shows (empty = the built-in one).
    pub discord_app_id: String,
    /// Song select order (ui::SortMode name).
    pub sort: String,
    /// Countdown before play continues after the pause menu (ms).
    pub resume_delay_ms: f64,
    /// Menu animations (screen fades, sliding highlights, ...).
    pub animations: bool,
    /// The animated splash screen at startup.
    pub splash: bool,
    /// Look for a newer build at startup.
    pub check_updates: bool,
    /// Start in (borderless) fullscreen; F11 toggles and remembers it.
    pub fullscreen: bool,
    /// The window's size out of fullscreen (pixels), as last left; None = the default.
    pub window_size: Option<(u32, u32)>,
    /// Where the game's data was moved to (see `data_folder()`); None = the usual places.
    pub data_folder: Option<PathBuf>,
    /// Left rim, left centre, right centre, right rim.
    pub keys: [KeyCode; 4],
    /// Previous / next playfield resolution preset.
    pub preset_keys: [KeyCode; 2],
    /// The commit of the build last started (what's new is shown when it changes), and whether
    /// there was a config to read (not the first start).
    pub last_build: String,
    pub existed: bool,
    /// Song select's layout: the share of the width the song list takes (`ui::LIST_SHARE`), and
    /// how big the autoplay preview is (`preview::PREVIEW_SIZE`; smaller leaves the leaderboard
    /// more room).
    pub song_list_share: f32,
    pub song_preview_size: f32,
    /// Every other shortcut's key (`keys::Hotkey`).
    pub hotkeys: crate::keys::Bindings,
    /// Show the playfield's resolution in a corner during play.
    pub show_playfield_size: bool,
    /// The player's own playfield sizes, first in the preset list, in their order.
    pub saved_presets: Vec<(u32, u32)>,
    /// Where each playfield size was last put (its top-left corner, as fractions of the screen),
    /// so going back to a preset puts it back there instead of in the middle.
    pub preset_spots: Vec<((u32, u32), (f32, f32))>,
    /// Name of the osu! skin in use (its folder name); empty = built-in.
    pub skin: String,
    /// Where each part of the skin comes from: that skin, the built-in look or another skin.
    pub skin_parts: crate::skin::Sources,
    /// The Songs folder picked in settings, scanned instead of looking for osu! installs.
    pub songs_folder: Option<PathBuf>,
    /// Multiverse's universes: each playfield's colours, note shape, speed and mods.
    pub universes: [crate::multiverse::Universe; crate::multiverse::COUNT],
    /// The player's Multiverse presets.
    pub universe_presets: Vec<crate::multiverse::Preset>,
    /// Shapeshift: how often the lane changes shape, how long the morph takes, the order.
    pub shapeshift: crate::shapeshift::Config,
    /// Mods marked as favourites in the mod select (shown in a row of their own).
    pub favourite_mods: crate::mods::Mods,
    /// Note colours without Multiverse.
    pub note_colours: crate::multiverse::Colours,
    /// How replay videos are exported.
    pub video: crate::video::Settings,
    pub songs: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            touch_layout: TouchLayout::Kddk,
            // Android has no clipboard to put pictures on here.
            screenshot_to: if cfg!(target_os = "android") { ScreenshotTo::Folder } else { ScreenshotTo::Clipboard },
            offset_ms: 0.0,
            visual_offset_ms: 0.0,
            vsync_compensation: true,
            vsync_tune_ms: 0.0,
            scroll_speed: 1.0,
            master_volume: 1.0,
            music_volume: 0.7,
            fx_volume: 1.0,
            present: PresentChoice::Immediate,
            half_resolution: cfg!(target_os = "android"),
            menu_vsync: true,
            wheel_resizes_playfield: false,
            bg_dim_black: true,
            interface_hidden: false,
            interface_hides: crate::hud::Parts::DEFAULT_HIDES,
            interface_key: (true, KeyCode::Tab),
            fps_cap: 0,
            calibration_bpm: 120.0,
            playfield_area: Area::FULL,
            bg_dim: 0.8,
            rate: 1.0,
            rate_keep_pitch: true,
            show_offset_hint: true,
            show_map_info: true,
            mp_scoreboard: true,
            mods: Mods::NONE,
            mp_name: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok()
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| "player".into()),
            mp_port: 7777,
            mp_address: String::new(),
            online_server: crate::online::DEFAULT_SERVER.to_string(),
            online_user: String::new(),
            online_token: String::new(),
            offset_hint_ur_scaled: true,
            audio_buffer: 256,
            show_fps: true,
            fullscreen: true,
            window_size: None,
            data_folder: None,
            animations: true,
            splash: true,
            check_updates: true,
            resume_delay_ms: 1000.0,
            sort: "artist".into(),
            kiai_effects: true,
            autoplay: crate::autoplay::Settings::default(),
            hide_great: true,
            accent: crate::render::DEFAULT_ACCENT,
            discord_presence: true,
            discord_app_id: String::new(),
            chat_filter: true,
            message_sound: true,
            notify_playing: false,
            notify_online: true,
            notify_messages: true,
            notify_mentions: true,
            notify_requests: true,
            keys: [KeyCode::KeyD, KeyCode::KeyF, KeyCode::KeyJ, KeyCode::KeyK],
            preset_keys: [KeyCode::F5, KeyCode::F6],
            hotkeys: crate::keys::Bindings::default(),
            song_list_share: crate::ui::LIST_SHARE.2,
            song_preview_size: crate::preview::PREVIEW_SIZE.1,
            last_build: String::new(),
            existed: false,
            show_playfield_size: true,
            saved_presets: Vec::new(),
            preset_spots: Vec::new(),
            skin: String::new(),
            skin_parts: Default::default(),
            songs_folder: None,
            universes: crate::multiverse::defaults(),
            universe_presets: Vec::new(),
            shapeshift: Default::default(),
            favourite_mods: crate::mods::Mods::NONE,
            note_colours: crate::multiverse::Colours::CLASSIC,
            video: Default::default(),
            songs: Vec::new(),
        }
    }
}

/// One of `preset_spots`: `1280x720@0.1000/0.2000`.
fn parse_spot(s: &str) -> Option<((u32, u32), (f32, f32))> {
    let (size, spot) = s.trim().split_once('@')?;
    let (x, y) = spot.split_once('/')?;
    let (x, y) = (x.parse::<f32>().ok()?, y.parse::<f32>().ok()?);
    ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some((crate::layout::parse_size(size)?, (x, y)))
}

/// Per-user taiko.rs directory: settings (`cache == false`) or disposable data like extracted
/// `.osz` files (`cache == true`).
pub fn user_dir(cache: bool) -> Option<PathBuf> {
    // Tests never read or write the player's own settings, scores or replays.
    if cfg!(test) {
        return None;
    }
    #[cfg(windows)]
    {
        let var = if cache { "LOCALAPPDATA" } else { "APPDATA" };
        std::env::var_os(var).map(|d| PathBuf::from(d).join("taiko-rs"))
    }
    #[cfg(not(windows))]
    {
        let (xdg, fallback) = if cache { ("XDG_CACHE_HOME", ".cache") } else { ("XDG_CONFIG_HOME", ".config") };
        std::env::var_os(xdg)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
            .map(|d| d.join("taiko-rs"))
    }
}

/// The folder the player moved the game's data to (`data_folder` in config.txt, which itself
/// stays where it is); None: the usual places.
static DATA_FOLDER: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

pub fn set_data_folder(dir: Option<PathBuf>) {
    *DATA_FOLDER.write().unwrap_or_else(|e| e.into_inner()) = dir;
}

pub fn data_folder() -> Option<PathBuf> {
    DATA_FOLDER.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Where replays, scores, videos, play history and the like are kept: the data folder, else
/// next to config.txt.
pub fn data_dir() -> Option<PathBuf> {
    data_folder().or_else(|| user_dir(false))
}

/// Where disposable data goes (extracted `.osz` files, map analysis, previews).
pub fn cache_dir() -> Option<PathBuf> {
    data_folder().map(|d| d.join("cache")).or_else(|| user_dir(true))
}

/// taiko.rs's own folder `name` ("Songs", "Skins"): in the data folder, else next to config.txt
/// on Windows and in ~/.local/share/taiko-rs elsewhere.
pub fn own_dir(name: &str) -> Option<PathBuf> {
    own_dir_in(data_folder().as_deref(), name)
}

fn own_dir_in(folder: Option<&Path>, name: &str) -> Option<PathBuf> {
    if let Some(d) = folder {
        return Some(d.join(name));
    }
    #[cfg(windows)]
    let dir = user_dir(false);
    #[cfg(not(windows))]
    let dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("taiko-rs"));
    Some(dir?.join(name))
}

/// The game's data with the data folder `folder` (None: the usual places), each where it'd be:
/// the files next to config.txt, its own Songs and Skins, and the cache.
fn data_paths(folder: Option<&Path>) -> Vec<Option<PathBuf>> {
    let base = folder.map(Path::to_path_buf).or_else(|| user_dir(false));
    let mut out: Vec<Option<PathBuf>> = ["replays", "videos", "map_offsets.tsv", "plays.tsv", "hidden.tsv"].iter().map(|n| base.as_ref().map(|b| b.join(n))).collect();
    out.push(own_dir_in(folder, "Songs"));
    out.push(own_dir_in(folder, "Skins"));
    out.push(folder.map(|d| d.join("cache")).or_else(|| user_dir(true)));
    out
}

/// Moves the game's data from the data folder `from` to `to` (None: the usual places): renamed
/// where they're on the same drive, else copied and the originals removed once everything has
/// copied. Nothing is overwritten; on any failure what had moved goes back. `done` / `total`
/// count bytes for a progress bar.
pub fn move_data(from: Option<&Path>, to: Option<&Path>, done: &std::sync::atomic::AtomicU64, total: &std::sync::atomic::AtomicU64) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let pairs: Vec<(PathBuf, PathBuf)> = data_paths(from)
        .into_iter()
        .zip(data_paths(to))
        .filter_map(|(a, b)| Some((a?, b?)))
        .filter(|(a, b)| a.exists() && a != b)
        .collect();
    for (a, b) in &pairs {
        if b.exists() {
            return Err(format!("{} is already there: pick an empty folder", b.display()));
        }
        if b.starts_with(a) {
            return Err(format!("can't move {} into itself", a.display()));
        }
    }
    total.store(pairs.iter().map(|(a, _)| size_of(a)).sum(), Ordering::Relaxed);
    // Moved so far: (from, to, copied rather than renamed).
    let mut moved: Vec<(&Path, &Path, bool)> = Vec::new();
    let mut failed = None;
    for (a, b) in &pairs {
        if let Some(parent) = b.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            failed = Some(format!("{}: {e}", parent.display()));
            break;
        }
        if std::fs::rename(a, b).is_ok() {
            done.fetch_add(size_of(b), Ordering::Relaxed);
            moved.push((a, b, false));
            continue;
        }
        // Another drive: copy now, remove the originals at the end.
        match copy_all(a, b, done) {
            Ok(()) => moved.push((a, b, true)),
            Err(e) => {
                let _ = remove_all(b);
                failed = Some(format!("{}: {e}", a.display()));
                break;
            }
        }
    }
    if let Some(e) = failed {
        for &(a, b, copied) in moved.iter().rev() {
            if copied {
                let _ = remove_all(b);
            } else {
                let _ = std::fs::rename(b, a);
            }
        }
        return Err(e);
    }
    for &(a, _, copied) in &moved {
        if copied {
            let _ = remove_all(a);
        }
    }
    Ok(())
}

fn size_of(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| size_of(&e.path())).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn copy_all(from: &Path, to: &Path, done: &std::sync::atomic::AtomicU64) -> std::io::Result<()> {
    if std::fs::symlink_metadata(from)?.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_all(&e.path(), &to.join(e.file_name()), done)?;
        }
        Ok(())
    } else {
        let n = std::fs::copy(from, to)?;
        done.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

fn remove_all(p: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(p)?.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

/// Each map's own offset (on top of the global one), by the .osu's content hash:
/// `map_offsets.tsv` in the config folder, one `hash<TAB>ms` per line.
pub fn load_map_offsets() -> std::collections::HashMap<u64, f64> {
    let Some(text) = data_dir().and_then(|d| std::fs::read_to_string(d.join("map_offsets.tsv")).ok()) else { return Default::default() };
    text.lines()
        .filter_map(|l| {
            let (h, ms) = l.split_once('\t')?;
            Some((u64::from_str_radix(h.trim(), 16).ok()?, ms.trim().parse::<f64>().ok()?))
        })
        .collect()
}

pub fn save_map_offsets(offsets: &std::collections::HashMap<u64, f64>) {
    let Some(dir) = data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let mut lines: Vec<String> = offsets.iter().map(|(h, ms)| format!("{h:016x}\t{ms}")).collect();
    lines.sort();
    let _ = std::fs::write(dir.join("map_offsets.tsv"), lines.join("\n") + "\n");
}

pub fn config_path() -> Option<PathBuf> {
    Some(user_dir(false)?.join("config.txt"))
}

const KEY_NAMES: &[KeyCode] = &[
    KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG,
    KeyCode::KeyH, KeyCode::KeyI, KeyCode::KeyJ, KeyCode::KeyK, KeyCode::KeyL, KeyCode::KeyM, KeyCode::KeyN,
    KeyCode::KeyO, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::KeyR, KeyCode::KeyS, KeyCode::KeyT, KeyCode::KeyU,
    KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ, KeyCode::Digit0, KeyCode::Digit1,
    KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7,
    KeyCode::Digit8, KeyCode::Digit9, KeyCode::Comma, KeyCode::Period, KeyCode::Slash, KeyCode::Semicolon,
    KeyCode::Quote, KeyCode::BracketLeft, KeyCode::BracketRight, KeyCode::Backslash, KeyCode::Backquote,
    KeyCode::Space, KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight,
    KeyCode::AltLeft, KeyCode::AltRight, KeyCode::CapsLock, KeyCode::Tab, KeyCode::Enter, KeyCode::ArrowLeft,
    KeyCode::ArrowRight, KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::Numpad0, KeyCode::Numpad1,
    KeyCode::Numpad2, KeyCode::Numpad3, KeyCode::Numpad4, KeyCode::Numpad5, KeyCode::Numpad6, KeyCode::Numpad7,
    KeyCode::Numpad8, KeyCode::Numpad9, KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4, KeyCode::F5,
    KeyCode::F6, KeyCode::F7, KeyCode::F8, KeyCode::F9, KeyCode::F10, KeyCode::F11, KeyCode::F12, KeyCode::Minus,
    KeyCode::Equal, KeyCode::PageUp, KeyCode::PageDown, KeyCode::Home, KeyCode::End, KeyCode::Insert, KeyCode::Delete,
    KeyCode::Escape, KeyCode::Backspace, KeyCode::NumpadAdd, KeyCode::NumpadSubtract, KeyCode::NumpadMultiply,
    KeyCode::NumpadDivide, KeyCode::NumpadDecimal, KeyCode::NumpadEnter, KeyCode::Pause, KeyCode::ScrollLock,
    KeyCode::ContextMenu, KeyCode::IntlBackslash, NO_KEY,
];

/// A key taken away: nothing presses it.
pub const NO_KEY: KeyCode = KeyCode::F35;

/// "Shift+Tab".
pub fn interface_key_name((shift, k): (bool, KeyCode)) -> String {
    format!("{}{}", if shift { "Shift+" } else { "" }, key_name(k))
}

pub fn key_name(k: KeyCode) -> String {
    if k == NO_KEY {
        return "none".into();
    }
    let s = format!("{k:?}");
    s.strip_prefix("Key").or_else(|| s.strip_prefix("Digit")).map(str::to_owned).unwrap_or(s)
}

pub fn parse_key(s: &str) -> Option<KeyCode> {
    if s.eq_ignore_ascii_case("none") {
        return Some(NO_KEY);
    }
    KEY_NAMES.iter().copied().find(|k| format!("{k:?}").eq_ignore_ascii_case(s) || key_name(*k).eq_ignore_ascii_case(s))
}

impl Config {
    /// Music and hitsound volume after the master volume.
    pub fn volumes(&self) -> (f32, f32) {
        (self.master_volume * self.music_volume, self.master_volume * self.fx_volume)
    }

    pub fn load() -> Config {
        let mut cfg = Config::default();
        let Some(path) = config_path() else { return cfg };
        let Ok(text) = std::fs::read_to_string(&path) else { return cfg };
        cfg.existed = true;
        let mut osu_scroll_speed = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "offset_ms" => cfg.offset_ms = v.parse().unwrap_or(cfg.offset_ms),
                "visual_offset_ms" => cfg.visual_offset_ms = v.parse().unwrap_or(cfg.visual_offset_ms),
                "vsync_compensation" => cfg.vsync_compensation = v != "false",
                "vsync_tune_ms" => cfg.vsync_tune_ms = v.parse::<f64>().unwrap_or(cfg.vsync_tune_ms).clamp(-200.0, 200.0),
                "scroll_speed" => cfg.scroll_speed = v.parse::<f64>().unwrap_or(cfg.scroll_speed).clamp(0.3, 4.0),
                "scroll_speed_osu" => osu_scroll_speed = v == "true",
                "master_volume" => cfg.master_volume = v.parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0),
                "music_volume" => cfg.music_volume = v.parse::<f32>().unwrap_or(cfg.music_volume).clamp(0.0, 1.0),
                "fx_volume" => cfg.fx_volume = v.parse::<f32>().unwrap_or(cfg.fx_volume).clamp(0.0, 1.0),
                "present_mode" => cfg.present = PresentChoice::parse(v).unwrap_or(cfg.present),
                "menu_vsync" => cfg.menu_vsync = v != "false",
                "half_resolution" => cfg.half_resolution = v == "true",
                "fps_cap" => cfg.fps_cap = v.parse().unwrap_or(cfg.fps_cap),
                "playfield_area" => cfg.playfield_area = Area::parse(v).unwrap_or(Area::FULL),
                "bg_dim" => cfg.bg_dim = v.parse::<f32>().unwrap_or(0.8).clamp(0.0, 1.0),
                "rate" => cfg.rate = v.parse::<f64>().unwrap_or(1.0).clamp(RATE_RANGE.0, RATE_RANGE.1),
                "rate_keep_pitch" => cfg.rate_keep_pitch = v != "false",
                "calibration_bpm" => cfg.calibration_bpm = v.parse::<f64>().unwrap_or(120.0).clamp(60.0, 240.0),
                "audio_buffer" => cfg.audio_buffer = v.parse::<u32>().unwrap_or(cfg.audio_buffer).clamp(32, 4096),
                "show_fps" => cfg.show_fps = v == "true",
                "fullscreen" => cfg.fullscreen = v != "false",
                "window_size" => cfg.window_size = crate::layout::parse_size(v),
                "data_folder" => cfg.data_folder = (!v.is_empty()).then(|| PathBuf::from(v)),
                "animations" => cfg.animations = v != "false",
                "splash" => cfg.splash = v != "false",
                "check_updates" => cfg.check_updates = v != "false",
                "sort" => cfg.sort = v.to_string(),
                "kiai_effects" => cfg.kiai_effects = v != "false",
                "autoplay_style" => cfg.autoplay.style = crate::autoplay::Style::parse(v).unwrap_or(cfg.autoplay.style),
                "autoplay_binding" => cfg.autoplay.binding = crate::autoplay::BINDINGS.iter().position(|b| *b == v).unwrap_or(cfg.autoplay.binding),
                "autoplay_left_handed" => cfg.autoplay.left_handed = v == "true",
                "hide_great" => cfg.hide_great = v != "false",
                "accent" => cfg.accent = crate::render::parse_accent(v).unwrap_or(cfg.accent),
                "discord_presence" => cfg.discord_presence = v != "false",
                "chat_filter" => cfg.chat_filter = v != "false",
                "message_sound" => cfg.message_sound = v != "false",
                "notify_playing" => cfg.notify_playing = v == "true",
                "notify_online" => cfg.notify_online = v != "false",
                "notify_messages" => cfg.notify_messages = v != "false",
                "notify_mentions" => cfg.notify_mentions = v != "false",
                "notify_requests" => cfg.notify_requests = v != "false",
                "discord_app_id" => cfg.discord_app_id = v.to_string(),
                "resume_delay_ms" => cfg.resume_delay_ms = v.parse::<f64>().unwrap_or(1000.0).clamp(0.0, 3000.0),
                "show_offset_hint" => cfg.show_offset_hint = v == "true",
                "show_map_info" => cfg.show_map_info = v != "false",
                "mp_scoreboard" => cfg.mp_scoreboard = v != "false",
                "mods" => cfg.mods = Mods::parse(v),
                "mp_name" if !v.is_empty() => cfg.mp_name = v.to_string(),
                "mp_port" => cfg.mp_port = v.parse().unwrap_or(cfg.mp_port),
                "mp_address" => cfg.mp_address = v.to_string(),
                "online_server" if !v.is_empty() => cfg.online_server = v.to_string(),
                "online_user" => cfg.online_user = v.to_string(),
                "online_token" => cfg.online_token = v.to_string(),
                "offset_hint_ur_scaled" => cfg.offset_hint_ur_scaled = v == "true",
                "keys" => {
                    let keys: Vec<KeyCode> = v.split(',').filter_map(|s| parse_key(s.trim())).collect();
                    if let Ok(k) = keys.try_into() {
                        cfg.keys = k;
                    }
                }
                "preset_keys" => {
                    let keys: Vec<KeyCode> = v.split(',').filter_map(|s| parse_key(s.trim())).collect();
                    if let Ok(k) = keys.try_into() {
                        cfg.preset_keys = k;
                    }
                }
                "touch_layout" => cfg.touch_layout = TouchLayout::parse(v).unwrap_or(cfg.touch_layout),
                "screenshot_to" => cfg.screenshot_to = ScreenshotTo::parse(v).unwrap_or(cfg.screenshot_to),
                "show_playfield_size" => cfg.show_playfield_size = v != "false",
                "wheel_resizes_playfield" => cfg.wheel_resizes_playfield = v == "true",
                "bg_dim_black" => cfg.bg_dim_black = v == "true",
                "interface_hidden" => cfg.interface_hidden = v == "true",
                "interface_hides" => cfg.interface_hides = crate::hud::Parts::parse_saved(v),
                "interface_key" => {
                    let (shift, name) = match v.strip_prefix("Shift+") {
                        Some(rest) => (true, rest),
                        None => (false, v),
                    };
                    if let Some(k) = parse_key(name) {
                        cfg.interface_key = (shift, k);
                    }
                }
                "last_build" => cfg.last_build = v.to_string(),
                "song_list_share" => {
                    let (min, max, _) = crate::ui::LIST_SHARE;
                    cfg.song_list_share = v.parse::<f32>().ok().filter(|f| f.is_finite()).map_or(cfg.song_list_share, |f| f.clamp(min, max));
                }
                "song_preview_size" => {
                    let (min, max) = crate::preview::PREVIEW_SIZE;
                    cfg.song_preview_size = v.parse::<f32>().ok().filter(|f| f.is_finite()).map_or(cfg.song_preview_size, |f| f.clamp(min, max));
                }
                k if k.starts_with("hotkey.") => cfg.hotkeys.parse_line(&k["hotkey.".len()..], v),
                "saved_presets" => cfg.saved_presets = v.split(',').filter_map(crate::layout::parse_size).collect(),
                "preset_spots" => cfg.preset_spots = v.split(',').filter_map(parse_spot).collect(),
                "skin" => cfg.skin = v.to_string(),
                "skin_part" => cfg.skin_parts.parse(v),
                "songs_folder" => cfg.songs_folder = (!v.is_empty()).then(|| PathBuf::from(v)),
                "multiverse" => cfg.universes = crate::multiverse::parse(v),
                "multiverse_preset" => cfg.universe_presets.extend(crate::multiverse::Preset::parse(v)),
                "note_colours" => cfg.note_colours = crate::multiverse::Colours::parse(v),
                "shapeshift" => cfg.shapeshift = crate::shapeshift::Config::parse(v),
                "favourite_mods" => cfg.favourite_mods = crate::mods::Mods::parse_set(v),
                k if k.starts_with("video_") => {
                    cfg.video.parse(k, v);
                }
                "songs" => cfg.songs.push(PathBuf::from(v)),
                _ => {}
            }
        }
        // Scroll speed 1.0 used to be ~0.76x of osu!'s (at 16:9). Left at 1.0 it now simply
        // becomes osu!'s; a speed someone picked on purpose keeps looking (about) the same.
        if !osu_scroll_speed && (cfg.scroll_speed - 1.0).abs() > 1e-3 {
            cfg.scroll_speed = ((cfg.scroll_speed * OLD_SCROLL_SPEED / SCROLL_STEP).round() * SCROLL_STEP).clamp(0.3, 4.0);
        }
        cfg
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let keys: Vec<String> = self.keys.iter().map(|k| key_name(*k)).collect();
        let mut out = format!(
            "# taiko.rs settings\n\
             # keys = left rim, left centre, right centre, right rim\n\
             keys = {}\n\
             # previous / next playfield resolution preset\n\
             preset_keys = {}\n\
             # touch columns left to right: KDDK, KKDD, DKKD or DDKK (K = rim, D = centre)\n\
             touch_layout = {}\n\
             # where F12 screenshots go: clipboard, folder (Screenshots) or both\n\
             screenshot_to = {}\n\
             show_playfield_size = {}\n\
             # your own playfield sizes (preset list, in this order)\n\
             saved_presets = {}\n\
             # where each playfield size was last put: WxH@x/y (fractions of the screen)\n\
             preset_spots = {}\n\
             offset_ms = {}\n\
             # notes drawn this many ms early to make up for display latency (judging is unaffected)\n\
             visual_offset_ms = {}\n\
             # 1.00 = osu!'s scroll speed\n\
             scroll_speed = {:.2}\n\
             scroll_speed_osu = true\n\
             master_volume = {:.2}\n\
             music_volume = {:.2}\n\
             fx_volume = {:.2}\n\
             # mailbox | immediate | vsync\n\
             present_mode = {}\n\
             # menus always use vsync (gameplay uses present_mode)\n\
             menu_vsync = {}\n\
             # draw at half resolution, scaled up exactly 2x (crisp pixels, faster)\n\
             half_resolution = {}\n\
             # mailbox/immediate only; 0 = uncapped\n\
             fps_cap = {}\n\
             calibration_bpm = {}\n\
             # playfield area: x, y, width, height as fractions of the screen\n\
             playfield_area = {}\n\
             # background darkening during play, 0 (none) to 1 (hidden)\n\
             bg_dim = {:.2}\n\
             # speed modifier (1.5 = DT) and whether it keeps the music's pitch\n\
             rate = {:.2}\n\
             rate_keep_pitch = {}\n\
             # audio device buffer in frames (lower = less latency, more risk of crackle)\n\
             audio_buffer = {}\n\
             show_fps = {}\n\
             fullscreen = {}\n\
             animations = {}\n\
             splash = {}\n\
             check_updates = {}\n\
             # countdown before play continues after pausing (ms)\n\
             resume_delay_ms = {:.0}\n\
             # song select order: artist, title, difficulty, length, bpm, recently played, date added\n\
             sort = {}\n\
             kiai_effects = {}\n\
             # GREAT hits without their flash and label (only OKs and misses show)\n\
             hide_great = {}\n\
             show_offset_hint = {}\n\
             offset_hint_ur_scaled = {}\n\
             # offset/UR line during play (Tab toggles)\n\
             show_map_info = {}\n\
             # mods picked on song select (F1), e.g. HD,HR\n\
             mods = {}\n",
            keys.join(", "),
            self.preset_keys.iter().map(|k| key_name(*k)).collect::<Vec<_>>().join(", "),
            self.touch_layout.name(),
            self.screenshot_to.key(),
            self.show_playfield_size,
            self.saved_presets.iter().map(|(w, h)| format!("{w}x{h}")).collect::<Vec<_>>().join(", "),
            self.preset_spots.iter().map(|((w, h), (x, y))| format!("{w}x{h}@{x:.4}/{y:.4}")).collect::<Vec<_>>().join(", "),
            self.offset_ms,
            self.visual_offset_ms,
            self.scroll_speed,
            self.master_volume,
            self.music_volume,
            self.fx_volume,
            self.present.name(),
            self.menu_vsync,
            self.half_resolution,
            self.fps_cap,
            self.calibration_bpm,
            self.playfield_area.to_config(),
            self.bg_dim,
            self.rate,
            self.rate_keep_pitch,
            self.audio_buffer,
            self.show_fps,
            self.fullscreen,
            self.animations,
            self.splash,
            self.check_updates,
            self.resume_delay_ms,
            self.sort,
            self.kiai_effects,
            self.hide_great,
            self.show_offset_hint,
            self.offset_hint_ur_scaled,
            self.show_map_info,
            self.mods.to_config(),
        );
        out.push_str(&format!("# the live scoreboard in lobby matches (Tab toggles)\nmp_scoreboard = {}\n", self.mp_scoreboard));
        out.push_str(&format!(
            "# how autoplay hits: full, semi, single, roll or ddkk; binding 1234, 1324 or 4231; left-handed\nautoplay_style = {}\nautoplay_binding = {}\nautoplay_left_handed = {}\n",
            self.autoplay.style.key(),
            crate::autoplay::BINDINGS[self.autoplay.binding],
            self.autoplay.left_handed
        ));
        out.push_str(&format!("# osu! skin (folder name in a Skins folder), empty = built-in\nskin = {}\n", self.skin));
        out.push_str("# parts of the skin from elsewhere: skin_part = <part> = <skin folder name> (or <built-in>)\n");
        for line in self.skin_parts.to_config() {
            out.push_str(&format!("skin_part = {line}\n"));
        }
        out.push_str(&format!(
            "# interface colour: red (the default), pink, purple, blue, teal, green, yellow, orange or #rrggbb\naccent = {}\n",
            crate::render::accent_name(self.accent)
        ));
        out.push_str(&format!(
            "# Discord status (\"Playing taiko.rs\"); app id empty = the built-in one\ndiscord_presence = {}\ndiscord_app_id = {}\n",
            self.discord_presence, self.discord_app_id
        ));
        out.push_str(&format!(
            "# chats: star out swear words; a chime when a friend messages you\nchat_filter = {}\nmessage_sound = {}\n",
            self.chat_filter, self.message_sound
        ));
        out.push_str(&format!(
            "# pop-ups: a friend starts a map / comes online / messages you, a global chat line mentions you, friend requests\nnotify_playing = {}\nnotify_online = {}\nnotify_messages = {}\nnotify_mentions = {}\nnotify_requests = {}\n",
            self.notify_playing, self.notify_online, self.notify_messages, self.notify_mentions, self.notify_requests
        ));
        out.push_str(&format!("# multiplayer\nmp_name = {}\nmp_port = {}\nmp_address = {}\n", self.mp_name, self.mp_port, self.mp_address));
        out.push_str(&format!(
            "# online server (accounts, lobbies) and your saved login there\nonline_server = {}\nonline_user = {}\nonline_token = {}\n",
            self.online_server, self.online_user, self.online_token
        ));
        out.push_str(&format!(
            "# with vsync, draw notes where they'll be once the frame is on screen; ms added to that\nvsync_compensation = {}\nvsync_tune_ms = {:.0}\n",
            self.vsync_compensation, self.vsync_tune_ms
        ));
        let window = self.window_size.map(|(w, h)| format!("{w}x{h}")).unwrap_or_default();
        out.push_str(&format!("# the window's size out of fullscreen (empty = 1280x720)\nwindow_size = {window}\n"));
        let data = self.data_folder.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        out.push_str(&format!(
            "# where replays, scores, videos, taiko.rs's Songs and Skins and caches are (empty = the usual places); move it from Settings\ndata_folder = {data}\n"
        ));
        out.push_str(&format!("# the mouse wheel resizes the playfield during play\nwheel_resizes_playfield = {}\n", self.wheel_resizes_playfield));
        out.push_str(&format!("# during songs the background dims to black (false: the interface's grey)\nbg_dim_black = {}\n", self.bg_dim_black));
        out.push_str(&format!(
            "# the interface during play: hidden or not, what hiding it hides (score, accuracy, pp, combo, progress, health, mods, hit_error, offset_hint, judgements, hit_animation, scoreboard) and its key\ninterface_hidden = {}\ninterface_hides = {}\ninterface_key = {}\n",
            self.interface_hidden,
            self.interface_hides.format(),
            interface_key_name(self.interface_key)
        ));
        out.push_str(&format!(
            "# song select: the song list's share of the width, and the autoplay preview's size (drag the edges between them)\nsong_list_share = {:.3}\nsong_preview_size = {:.3}\n",
            self.song_list_share, self.song_preview_size
        ));
        out.push_str(&self.hotkeys.config_lines());
        out.push_str(&format!("# the build last started (what's new shows after an update)\nlast_build = {}\n", self.last_build));
        out.push_str(&self.video.config_lines());
        out.push_str(&format!(
            "# Multiverse's universes, one per playfield, ; between: colours, note shape, speed and mods (C in the mod select)\nmultiverse = {}\n",
            crate::multiverse::format(&self.universes)
        ));
        out.push_str("# your own Multiverse presets, one per line: name: universes as above\n");
        for p in &self.universe_presets {
            out.push_str(&format!("multiverse_preset = {}\n", p.format()));
        }
        out.push_str(&format!(
            "# Shapeshift: bars per shape (4/8/16/32), bars to morph (0.5/1/2/4), order shuffle/fixed, shapes in order (- = left out)\nshapeshift = {}\n",
            self.shapeshift.format()
        ));
        out.push_str(&format!(
            "# favourite mods (right-click or F in the mod select), e.g. HD,HR\nfavourite_mods = {}\n",
            self.favourite_mods.to_config()
        ));
        out.push_str(&format!(
            "# note colours without Multiverse: a palette (classic, neon, sunset...) or custom #don/#kat\nnote_colours = {}\n",
            self.note_colours.format()
        ));
        let folder = self.songs_folder.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        out.push_str(&format!("# Songs folder to scan instead of finding osu!'s (empty = find it)\nsongs_folder = {folder}\n"));
        out.push_str("# extra song folders / .osz files, one per line\n");
        for s in &self.songs {
            out.push_str(&format!("songs = {}\n", s.display()));
        }
        let _ = std::fs::write(path, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_choices_read_back_and_go_to_the_clipboard_first() {
        assert_eq!(Config::default().screenshot_to, ScreenshotTo::Clipboard);
        for to in ScreenshotTo::ALL {
            assert_eq!(ScreenshotTo::parse(to.key()), Some(to));
        }
        assert!(ScreenshotTo::Both.clipboard() && ScreenshotTo::Both.folder());
        assert!(!ScreenshotTo::Clipboard.folder() && !ScreenshotTo::Folder.clipboard());
        assert_eq!(ScreenshotTo::Clipboard.step(-1), ScreenshotTo::Both);
    }

    #[test]
    fn touch_layouts_match_their_names_and_split_hands() {
        for layout in TouchLayout::ALL {
            let keys = layout.keys();
            // Each drum key exactly once.
            let mut sorted = keys;
            sorted.sort();
            assert_eq!(sorted, [0, 1, 2, 3], "{}", layout.name());
            // Columns read like the name (K = rims 0/3, D = centres 1/2).
            let read: String = keys.iter().map(|&k| if k == 0 || k == 3 { 'K' } else { 'D' }).collect();
            assert_eq!(read, layout.name());
            // Both columns of a colour are different hands, so together they make a big hit.
            for kat in [true, false] {
                let hands: Vec<bool> = keys.iter().filter(|&&k| (k == 0 || k == 3) == kat).map(|&k| k < 2).collect();
                assert_ne!(hands[0], hands[1], "{}", layout.name());
            }
            assert_eq!(TouchLayout::parse(&layout.name().to_lowercase()), Some(layout));
        }
        assert_eq!(TouchLayout::Kddk.step(-1), TouchLayout::Ddkk);
    }

    #[test]
    fn preset_spots_read_back() {
        assert_eq!(parse_spot(" 1280x720@0.1000/0.2500"), Some(((1280, 720), (0.1, 0.25))));
        assert_eq!(parse_spot("1280x720@1.5/0"), None);
        assert_eq!(parse_spot("1280x720"), None);
    }

    #[test]
    fn scroll_speed_steps_land_on_the_grid() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(step_scroll_speed(1.01, -1.0), 1.0));
        assert!(close(step_scroll_speed(1.01, 1.0), 1.05));
        assert!(close(step_scroll_speed(1.0, -1.0), 0.95));
        assert!(close(step_scroll_speed(1.0, 1.0), 1.05));
        // Floating-point sums of 0.05 don't skip a step.
        assert!(close(step_scroll_speed(0.05 * 21.0, -1.0), 1.0));
        assert!(close(step_scroll_speed(0.3, -1.0), 0.3));
        assert!(close(step_scroll_speed(4.0, 1.0), 4.0));
    }

    #[test]
    fn game_data_moves_to_another_folder_and_nothing_is_overwritten() {
        use std::sync::atomic::AtomicU64;
        let root = std::env::temp_dir().join(format!("taiko-rs-data-test-{}", std::process::id()));
        let (from, to) = (root.join("old"), root.join("new"));
        let _ = std::fs::remove_dir_all(&root);
        let file = |p: PathBuf, text: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        file(from.join("replays/1.json"), "replay");
        file(from.join("plays.tsv"), "plays");
        file(from.join("Songs/1 map/map.osu"), "osu");
        file(from.join("Skins/mine/skin.ini"), "skin");
        file(from.join("cache/osz/x/y.osu"), "cache");
        let (done, total) = (AtomicU64::new(0), AtomicU64::new(0));
        move_data(Some(&from), Some(&to), &done, &total).unwrap();
        for (path, text) in [("replays/1.json", "replay"), ("plays.tsv", "plays"), ("Songs/1 map/map.osu", "osu"), ("Skins/mine/skin.ini", "skin"), ("cache/osz/x/y.osu", "cache")] {
            assert_eq!(std::fs::read_to_string(to.join(path)).unwrap(), text, "{path}");
            assert!(!from.join(path).exists(), "{path} still in the old folder");
        }
        assert_eq!(done.load(std::sync::atomic::Ordering::Relaxed), total.load(std::sync::atomic::Ordering::Relaxed));
        // Back, onto a folder that has replays of its own: refused, nothing touched.
        file(from.join("replays/other.json"), "other");
        assert!(move_data(Some(&to), Some(&from), &done, &total).is_err());
        assert!(to.join("replays/1.json").exists() && from.join("replays/other.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copying_to_another_drive_copies_everything_then_removes_it() {
        use std::sync::atomic::AtomicU64;
        let root = std::env::temp_dir().join(format!("taiko-rs-copy-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/c.txt"), "hello").unwrap();
        std::fs::write(root.join("a/d.txt"), "hi").unwrap();
        let done = AtomicU64::new(0);
        copy_all(&root.join("a"), &root.join("z"), &done).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("z/b/c.txt")).unwrap(), "hello");
        assert_eq!(done.load(std::sync::atomic::Ordering::Relaxed), 7);
        assert_eq!(size_of(&root.join("z")), 7);
        remove_all(&root.join("a")).unwrap();
        assert!(!root.join("a").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
