//! Settings persisted as a plain `key = value` file: `$XDG_CONFIG_HOME/taiko-rs/config.txt` on
//! Linux, `%APPDATA%\taiko-rs\config.txt` on Windows.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};

use winit::keyboard::KeyCode;

use crate::layout::Area;
use crate::mods::Mods;
use crate::render::PresentChoice;

/// How many maps the editor remembers as recent.
pub const MAX_RECENT_MAPS: usize = 8;
/// How many editor snap presets are kept.
const MAX_SNAP_PRESETS: usize = 24;
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
    let k = if d > 0.0 {
        (k + 1e-6).floor() + 1.0
    } else {
        (k - 1e-6).ceil() - 1.0
    };
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
    pub const ALL: [TouchLayout; 4] = [
        TouchLayout::Kddk,
        TouchLayout::Kkdd,
        TouchLayout::Dkkd,
        TouchLayout::Ddkk,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TouchLayout::Kddk => "KDDK",
            TouchLayout::Kkdd => "KKDD",
            TouchLayout::Dkkd => "DKKD",
            TouchLayout::Ddkk => "DDKK",
        }
    }

    fn parse(s: &str) -> Option<TouchLayout> {
        TouchLayout::ALL
            .into_iter()
            .find(|l| l.name().eq_ignore_ascii_case(s))
    }

    /// The layout `dir` steps away (wrapping).
    pub fn step(self, dir: i32) -> TouchLayout {
        let i = TouchLayout::ALL
            .iter()
            .position(|&l| l == self)
            .unwrap_or(0) as i32;
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
    pub const ALL: [ScreenshotTo; 3] = [
        ScreenshotTo::Clipboard,
        ScreenshotTo::Folder,
        ScreenshotTo::Both,
    ];

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
        ScreenshotTo::ALL
            .into_iter()
            .find(|t| t.key().eq_ignore_ascii_case(s))
    }

    /// The choice `dir` steps away (wrapping).
    pub fn step(self, dir: i32) -> ScreenshotTo {
        let i = ScreenshotTo::ALL
            .iter()
            .position(|&t| t == self)
            .unwrap_or(0) as i32;
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
    /// The interface's language.
    pub language: crate::lang::Lang,
    /// The global offset as osu! has it (its universal offset): positive brings the notes
    /// earlier. On `OffsetScale::Osu` this is what the player sets and `offset_ms` follows from
    /// it; on `Sound` it follows from `offset_ms`. What the player sees is `shown_offset`.
    pub offset_osu_ms: f64,
    /// Which scale the player sets the global offset on, and so which of the two stays put when
    /// the output latency changes.
    pub offset_scale: OffsetScale,
    /// The same offset the way the clock takes it, `-offset_osu_ms - OSU_TIMING_MS - latency`:
    /// positive when your audio reaches you late, notes are shifted later by this much. Kept up
    /// to date by `sync_offset` (`set_latency` follows the output latency). While
    /// `offset_pending` it is the player's old setting, played as it was.
    pub offset_ms: f64,
    /// The output latency (ms) `offset_ms` was worked out with: what the clock took off.
    pub offset_latency_ms: f64,
    /// The config has the old `offset_ms` and no `offset` yet: it becomes one the first time the
    /// engine reports its settled latency (`set_latency`).
    pub offset_pending: bool,
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
    /// How much of the volume is left while another window has focus (1: all of it).
    pub unfocused_volume: f32,
    pub fx_volume: f32,
    /// Present mode during songs.
    pub present: PresentChoice,
    /// Draw at half resolution and scale up exactly 2x (default on Android: high-DPI screens,
    /// mobile GPUs).
    pub half_resolution: bool,
    /// The graphics API: one of `RENDERERS` (`crate::graphics_backends`).
    pub graphics_api: String,
    /// Everything but the player playing a song (menus, results, replays, autoplay) uses vsync.
    pub menu_vsync: bool,
    /// The mouse wheel resizes the playfield during play (it always does in its editor).
    pub wheel_resizes_playfield: bool,
    /// Song select lists converts too.
    pub show_converts: bool,
    /// Song select lists songs under headings (`ui::Select::headings`).
    pub song_headings: bool,
    /// During songs, the background dims to black (and is black without one), rather than to
    /// the interface's grey.
    pub bg_dim_black: bool,
    /// The interface during play is hidden (the toggle), and what that hides.
    pub interface_hidden: bool,
    pub interface_hides: crate::hud::Parts,
    /// Where the interface's parts go, from the interface editor.
    pub hud: crate::hud::Layout,
    /// The player's own interface layouts.
    pub hud_presets: Vec<(String, crate::hud::Layout)>,
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
    /// The player's mod presets (`mods::Preset`), at most `mods::MAX_PRESETS`, in their order.
    pub mod_presets: Vec<crate::mods::Preset>,
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
    /// Hit notes on the main menu glitch as they spin, and the screen glitches with them and
    /// between menus.
    pub menu_glitch: bool,
    /// How autoplay hits: playstyle, binding and hand (taiko_arranger's).
    pub autoplay: crate::autoplay::Settings,
    /// GREAT hits without their flash and label (only OKs and misses show).
    pub hide_great: bool,
    /// A play failed by Sudden Death or Perfect starts over at once instead of showing results.
    pub fail_restart: bool,
    /// The lane and the drum's panel drawn pure black.
    pub black_lane: bool,
    /// .tko notes drawn as what they are, whatever their map makes them show as.
    pub true_notes: bool,
    /// .tko maps' flashes, glitches and camera moves toned down.
    pub calm_maps: bool,
    /// The drum keys move through song select and the pause menu (`drum_nav`).
    pub drum_menus: bool,
    /// The coloured flash on the hit target as notes are hit.
    pub hit_flash: bool,
    /// The interface's accent colour (`render::ACCENTS`, or any other).
    pub accent: u32,
    /// Show what you're doing in your Discord status.
    pub discord_presence: bool,
    /// Minutes without input before the idle screen shows (0: never; one of `idle::MINUTES`).
    pub idle_minutes: u32,
    /// A crash's report goes to the server on the next start (`online::Request::CrashReport`).
    pub send_crash_reports: bool,
    /// The map editor's first-run tour was finished or skipped.
    pub editor_tour_done: bool,
    /// Maps opened in the editor lately (full paths), the latest first.
    pub editor_recent: Vec<String>,
    /// In the editor a left drag on empty timeline pans it (View > Left drag pans).
    pub editor_left_pan: bool,
    /// In the map editor, a red line's new time or BPM moves its notes along.
    pub editor_follow_timing: bool,
    /// The editor's own snap presets, one `3,6,12|Name` value each (`editor::snaps`).
    pub editor_snap_presets: Vec<String>,
    /// The name of the snap preset in use (empty: the built-in "All").
    pub editor_snap_active: String,
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
    /// The flashing-lights warning shown first at startup.
    pub epilepsy_warning: bool,
    /// Start on the main menu (off: straight to song select).
    pub main_menu: bool,
    /// The song's notes drift behind the main menu.
    pub menu_notes: bool,
    /// How much the song's notes on the main menu's lane are dimmed, 0 (as they are) to 1 (gone).
    pub menu_notes_dim: f32,
    /// The first steps guide has been seen through. A config from before it says nothing, so
    /// players already playing get it once too.
    pub guide_seen: bool,
    /// Look for a newer build at startup.
    pub check_updates: bool,
    /// Start in (borderless) fullscreen; F11 toggles and remembers it.
    pub fullscreen: bool,
    /// The window's size out of fullscreen (pixels), as last left; None = the default.
    pub window_size: Option<(u32, u32)>,
    /// In fullscreen, the game at `letterbox_size` with black around it, moved from the centre by
    /// `letterbox_pos` (percent, -100 to 100).
    pub letterbox: bool,
    pub letterbox_size: (u32, u32),
    pub letterbox_pos: (f32, f32),
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
    /// Maps may draw with the skin images in their own folder (beatmap skins).
    pub map_skins: bool,
    /// The Songs folder picked in settings, scanned instead of looking for osu! installs.
    pub songs_folder: Option<PathBuf>,
    /// Multiverse's universes: each playfield's colours, note shape, speed and mods.
    pub universes: [crate::multiverse::Universe; crate::multiverse::COUNT],
    /// The player's Multiverse presets.
    pub universe_presets: Vec<crate::multiverse::Preset>,
    /// Shapeshift: how often the lane changes shape, how long the morph takes, the order.
    pub shapeshift: crate::shapeshift::Config,
    /// Random colouring, shared by retries and saved between sessions.
    pub random_seed: u64,
    /// Tact: how often notes move (index into `tact::STEPS`).
    pub tact: usize,
    /// OD Override: the OD played at.
    pub od_override: f64,
    /// Mods marked as favourites in the mod select (shown in a row of their own).
    pub favourite_mods: crate::mods::Mods,
    /// Note colours without Multiverse.
    pub note_colours: crate::multiverse::Colours,
    /// How replay videos are exported.
    pub video: crate::video::Settings,
    pub songs: Vec<PathBuf>,
}

/// What the global offset is set by (`Config::offset_scale`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OffsetScale {
    /// osu!'s universal offset: the same number as there, and like there the output latency
    /// counts: when it changes, the notes move with the audio stream, not with what you hear.
    #[default]
    Osu,
    /// 0 is hitting exactly when the sound reaches you, the output latency taken off; the number
    /// is how far ahead of the sound you hit. It stays right whatever the latency does.
    Sound,
}

impl OffsetScale {
    pub const ALL: [OffsetScale; 2] = [OffsetScale::Osu, OffsetScale::Sound];

    pub fn name(self) -> &'static str {
        match self {
            OffsetScale::Osu => "osu",
            OffsetScale::Sound => "sound",
        }
    }

    pub fn parse(v: &str) -> Option<OffsetScale> {
        OffsetScale::ALL.into_iter().find(|s| s.name() == v.trim())
    }
}

/// The play offset (what the clock subtracts, `Config::offset_ms`) that feels like osu!'s
/// universal offset `osu` when the clock also takes `latency` ms of output latency off. osu! adds
/// its offset to a clock with no latency taken off and plays maps as they are, where here maps
/// from .osu files are shifted `OSU_TIMING_MS` later and the offset is subtracted.
pub fn play_offset_of(osu: f64, latency: f64) -> f64 {
    -osu - crate::beatmap::OSU_TIMING_MS - latency
}

/// The osu! universal offset a play offset `play` feels like (`play_offset_of` the other way).
pub fn osu_offset_of(play: f64, latency: f64) -> f64 {
    -play - crate::beatmap::OSU_TIMING_MS - latency
}

impl Default for Config {
    fn default() -> Self {
        Config {
            touch_layout: TouchLayout::Kddk,
            // Android has no clipboard to put pictures on here.
            screenshot_to: if cfg!(target_os = "android") {
                ScreenshotTo::Folder
            } else {
                ScreenshotTo::Clipboard
            },
            language: crate::lang::Lang::En,
            // osu!'s default: a new player starts where an osu! player does.
            offset_osu_ms: 0.0,
            offset_scale: OffsetScale::Osu,
            offset_ms: play_offset_of(0.0, 0.0),
            offset_latency_ms: 0.0,
            offset_pending: false,
            visual_offset_ms: 0.0,
            vsync_compensation: true,
            vsync_tune_ms: 0.0,
            scroll_speed: 1.0,
            master_volume: 1.0,
            music_volume: 0.7,
            unfocused_volume: 0.35,
            fx_volume: 1.0,
            present: PresentChoice::Immediate,
            half_resolution: cfg!(target_os = "android"),
            graphics_api: "auto".into(),
            menu_vsync: true,
            wheel_resizes_playfield: false,
            show_converts: false,
            song_headings: false,
            bg_dim_black: true,
            interface_hidden: false,
            interface_hides: crate::hud::Parts::DEFAULT_HIDES,
            hud: crate::hud::Layout::default(),
            hud_presets: Vec::new(),
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
            mod_presets: Vec::new(),
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
            letterbox: false,
            letterbox_size: (1280, 720),
            letterbox_pos: (0.0, 0.0),
            data_folder: None,
            animations: true,
            splash: true,
            epilepsy_warning: true,
            main_menu: true,
            menu_notes: true,
            menu_notes_dim: 0.0,
            guide_seen: false,
            check_updates: true,
            resume_delay_ms: 1000.0,
            sort: "artist".into(),
            kiai_effects: true,
            menu_glitch: true,
            autoplay: crate::autoplay::Settings::default(),
            hide_great: true,
            fail_restart: false,
            black_lane: false,
            true_notes: false,
            calm_maps: false,
            drum_menus: false,
            hit_flash: true,
            accent: crate::render::DEFAULT_ACCENT,
            discord_presence: true,
            idle_minutes: 2,
            send_crash_reports: true,
            editor_tour_done: false,
            editor_recent: Vec::new(),
            editor_left_pan: false,
            editor_follow_timing: false,
            editor_snap_presets: Vec::new(),
            editor_snap_active: String::new(),
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
            map_skins: true,
            songs_folder: None,
            universes: crate::multiverse::defaults(),
            universe_presets: Vec::new(),
            shapeshift: Default::default(),
            random_seed: crate::fresh_seed(),
            tact: crate::tact::DEFAULT,
            od_override: crate::od_override::DEFAULT,
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
    ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y))
        .then_some((crate::layout::parse_size(size)?, (x, y)))
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
        let (xdg, fallback) = if cache {
            ("XDG_CACHE_HOME", ".cache")
        } else {
            ("XDG_CONFIG_HOME", ".config")
        };
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
    DATA_FOLDER
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Where replays, scores, videos, play history and the like are kept: the data folder, else
/// next to config.txt.
/// The graphics APIs this system offers (`Config::graphics_api`, what the setting stores).
pub const RENDERERS: &[&str] = if cfg!(windows) {
    &["auto", "dx12", "vulkan", "gl"]
} else if cfg!(target_os = "macos") {
    &["auto", "metal"]
} else {
    &["auto", "vulkan", "gl"]
};

/// A renderer's name in the settings.
pub fn renderer_name(api: &str) -> &'static str {
    match api {
        "dx12" => "DirectX 12",
        "vulkan" => "Vulkan",
        "gl" => "OpenGL",
        "metal" => "Metal",
        _ => tr!("Auto", "自動", "Авто"),
    }
}

pub fn data_dir() -> Option<PathBuf> {
    data_folder().or_else(|| user_dir(false))
}

/// Where disposable data goes (extracted `.osz` files, map analysis, previews).
pub fn cache_dir() -> Option<PathBuf> {
    data_folder()
        .map(|d| d.join("cache"))
        .or_else(|| user_dir(true))
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
    let mut out: Vec<Option<PathBuf>> = [
        "replays",
        "videos",
        MAP_OFFSETS,
        OLD_SIGN_MAP_OFFSETS,
        OLD_SIGN_KEPT,
        "plays.tsv",
        "hidden.tsv",
        "map_skins_off.txt",
    ]
    .iter()
    .map(|n| base.as_ref().map(|b| b.join(n)))
    .collect();
    out.push(own_dir_in(folder, "Songs"));
    out.push(own_dir_in(folder, "Skins"));
    out.push(folder.map(|d| d.join("cache")).or_else(|| user_dir(true)));
    out
}

/// Moves the game's data from the data folder `from` to `to` (None: the usual places): renamed
/// where they're on the same drive, else copied and the originals removed once everything has
/// copied. Nothing is overwritten; on any failure what had moved goes back. `done` / `total`
/// count bytes for a progress bar.
pub fn move_data(
    from: Option<&Path>,
    to: Option<&Path>,
    done: &std::sync::atomic::AtomicU64,
    total: &std::sync::atomic::AtomicU64,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let pairs: Vec<(PathBuf, PathBuf)> = data_paths(from)
        .into_iter()
        .zip(data_paths(to))
        .filter_map(|(a, b)| Some((a?, b?)))
        .filter(|(a, b)| a.exists() && a != b)
        .collect();
    for (a, b) in &pairs {
        if b.exists() {
            return Err(format!(
                "{} is already there: pick an empty folder",
                b.display()
            ));
        }
        if b.starts_with(a) {
            return Err(format!("can't move {} into itself", a.display()));
        }
    }
    total.store(
        pairs.iter().map(|(a, _)| size_of(a)).sum(),
        Ordering::Relaxed,
    );
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
        Ok(m) if m.is_dir() => std::fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| size_of(&e.path()))
            .sum(),
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
    if std::fs::symlink_metadata(p)?.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

/// Each map's own offset (on top of the global one), by the .osu's content hash:
/// `map_offsets_osu.tsv` in the config folder, one `hash<TAB>ms` per line. Like the global one
/// and osu!'s local offset: + brings the notes earlier.
pub const MAP_OFFSETS: &str = "map_offsets_osu.tsv";
/// Before: the other way round (+ took the notes later). Read once, turned round into
/// `MAP_OFFSETS`, then kept under `OLD_SIGN_KEPT` to go back to by hand.
const OLD_SIGN_MAP_OFFSETS: &str = "map_offsets.tsv";
const OLD_SIGN_KEPT: &str = "map_offsets.before-osu-sign.tsv";

fn parse_map_offsets(text: &str) -> std::collections::HashMap<u64, f64> {
    text.lines()
        .filter_map(|l| {
            let (h, ms) = l.split_once('\t')?;
            Some((
                u64::from_str_radix(h.trim(), 16).ok()?,
                ms.trim().parse::<f64>().ok()?,
            ))
        })
        .collect()
}

pub fn load_map_offsets() -> std::collections::HashMap<u64, f64> {
    let Some(dir) = data_dir() else {
        return Default::default();
    };
    load_map_offsets_in(&dir)
}

fn load_map_offsets_in(dir: &Path) -> std::collections::HashMap<u64, f64> {
    if let Ok(text) = std::fs::read_to_string(dir.join(MAP_OFFSETS)) {
        return parse_map_offsets(&text);
    }
    let Ok(text) = std::fs::read_to_string(dir.join(OLD_SIGN_MAP_OFFSETS)) else {
        return Default::default();
    };
    let offsets: std::collections::HashMap<u64, f64> = parse_map_offsets(&text)
        .into_iter()
        .map(|(h, ms)| (h, -ms))
        .collect();
    if save_map_offsets_in(dir, &offsets) {
        let _ = std::fs::rename(dir.join(OLD_SIGN_MAP_OFFSETS), dir.join(OLD_SIGN_KEPT));
    }
    offsets
}

pub fn save_map_offsets(offsets: &std::collections::HashMap<u64, f64>) {
    let Some(dir) = data_dir() else { return };
    save_map_offsets_in(&dir, offsets);
}

fn save_map_offsets_in(dir: &Path, offsets: &std::collections::HashMap<u64, f64>) -> bool {
    let _ = std::fs::create_dir_all(dir);
    let mut lines: Vec<String> = offsets
        .iter()
        .map(|(h, ms)| format!("{h:016x}\t{ms}"))
        .collect();
    lines.sort();
    std::fs::write(dir.join(MAP_OFFSETS), lines.join("\n") + "\n").is_ok()
}

/// The folders of the maps whose own skin is turned off: `map_skins_off.txt` in the config folder,
/// one folder name per line. A map's skin is its folder's, so all its difficulties share this.
pub fn load_map_skins_off() -> std::collections::HashSet<String> {
    let Some(text) =
        data_dir().and_then(|d| std::fs::read_to_string(d.join("map_skins_off.txt")).ok())
    else {
        return Default::default();
    };
    text.lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn save_map_skins_off(folders: &std::collections::HashSet<String>) {
    let Some(dir) = data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let mut lines: Vec<&str> = folders.iter().map(String::as_str).collect();
    lines.sort_unstable();
    let _ = std::fs::write(dir.join("map_skins_off.txt"), lines.join("\n") + "\n");
}

/// Clears every map's own offset, keeping them in `map_offsets.backup.tsv` to go back to by hand.
pub fn reset_map_offsets() {
    if let Some(dir) = data_dir() {
        let _ = std::fs::rename(dir.join(MAP_OFFSETS), dir.join("map_offsets.backup.tsv"));
    }
}

pub fn config_path() -> Option<PathBuf> {
    Some(user_dir(false)?.join("config.txt"))
}

const KEY_NAMES: &[KeyCode] = &[
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Comma,
    KeyCode::Period,
    KeyCode::Slash,
    KeyCode::Semicolon,
    KeyCode::Quote,
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::Backslash,
    KeyCode::Backquote,
    KeyCode::Space,
    KeyCode::ShiftLeft,
    KeyCode::ShiftRight,
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::AltLeft,
    KeyCode::AltRight,
    KeyCode::CapsLock,
    KeyCode::Tab,
    KeyCode::Enter,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
    KeyCode::Minus,
    KeyCode::Equal,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::Insert,
    KeyCode::Delete,
    KeyCode::Escape,
    KeyCode::Backspace,
    KeyCode::NumpadAdd,
    KeyCode::NumpadSubtract,
    KeyCode::NumpadMultiply,
    KeyCode::NumpadDivide,
    KeyCode::NumpadDecimal,
    KeyCode::NumpadEnter,
    KeyCode::Pause,
    KeyCode::ScrollLock,
    KeyCode::ContextMenu,
    KeyCode::IntlBackslash,
    NO_KEY,
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
    s.strip_prefix("Key")
        .or_else(|| s.strip_prefix("Digit"))
        .map(str::to_owned)
        .unwrap_or(s)
}

pub fn parse_key(s: &str) -> Option<KeyCode> {
    if s.eq_ignore_ascii_case("none") {
        return Some(NO_KEY);
    }
    KEY_NAMES
        .iter()
        .copied()
        .find(|k| format!("{k:?}").eq_ignore_ascii_case(s) || key_name(*k).eq_ignore_ascii_case(s))
}

impl Config {
    /// Notes that the editor opened `path`: it goes first in the recent maps.
    pub fn editor_opened(&mut self, path: &std::path::Path) {
        let path = path.to_string_lossy().into_owned();
        self.editor_recent.retain(|p| *p != path);
        self.editor_recent.insert(0, path);
        self.editor_recent.truncate(MAX_RECENT_MAPS);
    }

    /// Music and hitsound volume after the master volume.
    pub fn volumes(&self) -> (f32, f32) {
        (
            self.master_volume * self.music_volume,
            self.master_volume * self.fx_volume,
        )
    }

    pub fn load() -> Config {
        let Some(path) = config_path() else {
            return Config::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Config::default();
        };
        Config::parse_text(&text)
    }

    /// The settings in the config file's `text`.
    pub(crate) fn parse_text(text: &str) -> Config {
        let mut cfg = Config {
            existed: true,
            ..Config::default()
        };
        // The offset is worked out after the lines: `offset` and `offset_latency_ms` can come in
        // any order, and `offset_ms` may have been changed by an older build.
        let (mut old_offset, mut new_offset) = (None, None);
        let mut osu_scroll_speed = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "offset_ms" => old_offset = v.parse::<f64>().ok().or(old_offset),
                "offset" => new_offset = v.parse::<f64>().ok().or(new_offset),
                "offset_latency_ms" => {
                    cfg.offset_latency_ms = v.parse().unwrap_or(cfg.offset_latency_ms)
                }
                "visual_offset_ms" => {
                    cfg.visual_offset_ms = v.parse().unwrap_or(cfg.visual_offset_ms)
                }
                "vsync_compensation" => cfg.vsync_compensation = v != "false",
                "vsync_tune_ms" => {
                    cfg.vsync_tune_ms = v
                        .parse::<f64>()
                        .unwrap_or(cfg.vsync_tune_ms)
                        .clamp(-200.0, 200.0)
                }
                "scroll_speed" => {
                    cfg.scroll_speed = v.parse::<f64>().unwrap_or(cfg.scroll_speed).clamp(0.3, 4.0)
                }
                "scroll_speed_osu" => osu_scroll_speed = v == "true",
                "master_volume" => {
                    cfg.master_volume = v.parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0)
                }
                "unfocused_volume" => {
                    cfg.unfocused_volume = v
                        .parse::<f32>()
                        .unwrap_or(cfg.unfocused_volume)
                        .clamp(0.0, 1.0)
                }
                "music_volume" => {
                    cfg.music_volume = v.parse::<f32>().unwrap_or(cfg.music_volume).clamp(0.0, 1.0)
                }
                "fx_volume" => {
                    cfg.fx_volume = v.parse::<f32>().unwrap_or(cfg.fx_volume).clamp(0.0, 1.0)
                }
                "present_mode" => cfg.present = PresentChoice::parse(v).unwrap_or(cfg.present),
                "offset_scale" => cfg.offset_scale = OffsetScale::parse(v).unwrap_or_default(),
                "menu_vsync" => cfg.menu_vsync = v != "false",
                "half_resolution" => cfg.half_resolution = v == "true",
                "graphics_api" => cfg.graphics_api = v.trim().to_ascii_lowercase(),
                "fps_cap" => cfg.fps_cap = v.parse().unwrap_or(cfg.fps_cap),
                "playfield_area" => cfg.playfield_area = Area::parse(v).unwrap_or(Area::FULL),
                "bg_dim" => cfg.bg_dim = v.parse::<f32>().unwrap_or(0.8).clamp(0.0, 1.0),
                "rate" => {
                    cfg.rate = v
                        .parse::<f64>()
                        .unwrap_or(1.0)
                        .clamp(RATE_RANGE.0, RATE_RANGE.1)
                }
                "rate_keep_pitch" => cfg.rate_keep_pitch = v != "false",
                "calibration_bpm" => {
                    cfg.calibration_bpm = v.parse::<f64>().unwrap_or(120.0).clamp(60.0, 240.0)
                }
                "audio_buffer" => {
                    cfg.audio_buffer = v.parse::<u32>().unwrap_or(cfg.audio_buffer).clamp(32, 4096)
                }
                "show_fps" => cfg.show_fps = v == "true",
                "fullscreen" => cfg.fullscreen = v != "false",
                "window_size" => cfg.window_size = crate::layout::parse_size(v),
                "letterbox" => cfg.letterbox = v == "true",
                "letterbox_size" => {
                    cfg.letterbox_size = crate::layout::parse_size(v).unwrap_or(cfg.letterbox_size)
                }
                "letterbox_x" => cfg.letterbox_pos.0 = v.parse().unwrap_or(0.0),
                "letterbox_y" => cfg.letterbox_pos.1 = v.parse().unwrap_or(0.0),
                "data_folder" => cfg.data_folder = (!v.is_empty()).then(|| PathBuf::from(v)),
                "animations" => cfg.animations = v != "false",
                "splash" => cfg.splash = v != "false",
                "epilepsy_warning" => cfg.epilepsy_warning = v != "false",
                "main_menu" => cfg.main_menu = v != "false",
                "menu_notes" => cfg.menu_notes = v != "false",
                "menu_notes_dim" => {
                    cfg.menu_notes_dim = v.parse::<f32>().unwrap_or(0.0).clamp(0.0, 1.0)
                }
                "guide_seen" => cfg.guide_seen = v == "true",
                "check_updates" => cfg.check_updates = v != "false",
                "sort" => cfg.sort = v.to_string(),
                "kiai_effects" => cfg.kiai_effects = v != "false",
                "menu_glitch" => cfg.menu_glitch = v != "false",
                "autoplay_style" => {
                    cfg.autoplay.style =
                        crate::autoplay::Style::parse(v).unwrap_or(cfg.autoplay.style)
                }
                "autoplay_binding" => {
                    cfg.autoplay.binding = crate::autoplay::BINDINGS
                        .iter()
                        .position(|b| *b == v)
                        .unwrap_or(cfg.autoplay.binding)
                }
                "autoplay_left_handed" => cfg.autoplay.left_handed = v == "true",
                "hide_great" => cfg.hide_great = v != "false",
                "fail_restart" => cfg.fail_restart = v == "true",
                "black_lane" => cfg.black_lane = v == "true",
                "true_notes" => cfg.true_notes = v == "true",
                "calm_maps" => cfg.calm_maps = v == "true",
                "drum_menus" => cfg.drum_menus = v == "true",
                "hit_flash" => cfg.hit_flash = v != "false",
                "accent" => cfg.accent = crate::render::parse_accent(v).unwrap_or(cfg.accent),
                "discord_presence" => cfg.discord_presence = v != "false",
                "idle_minutes" => cfg.idle_minutes = v.parse().unwrap_or(cfg.idle_minutes),
                "send_crash_reports" => cfg.send_crash_reports = v != "false",
                "editor_tour_done" => cfg.editor_tour_done = v == "true",
                "editor_left_pan" => cfg.editor_left_pan = v == "true",
                "editor_follow_timing" => cfg.editor_follow_timing = v == "true",
                "editor_snap_preset" if !v.is_empty() => {
                    if cfg.editor_snap_presets.len() < MAX_SNAP_PRESETS {
                        cfg.editor_snap_presets.push(v.to_string());
                    }
                }
                "editor_snap_active" => cfg.editor_snap_active = v.to_string(),
                "editor_recent" if !v.is_empty() => {
                    if cfg.editor_recent.len() < MAX_RECENT_MAPS
                        && !cfg.editor_recent.iter().any(|r| r == v)
                    {
                        cfg.editor_recent.push(v.to_string());
                    }
                }
                "chat_filter" => cfg.chat_filter = v != "false",
                "message_sound" => cfg.message_sound = v != "false",
                "notify_playing" => cfg.notify_playing = v == "true",
                "notify_online" => cfg.notify_online = v != "false",
                "notify_messages" => cfg.notify_messages = v != "false",
                "notify_mentions" => cfg.notify_mentions = v != "false",
                "notify_requests" => cfg.notify_requests = v != "false",
                "discord_app_id" => cfg.discord_app_id = v.to_string(),
                "resume_delay_ms" => {
                    cfg.resume_delay_ms = v.parse::<f64>().unwrap_or(1000.0).clamp(0.0, 3000.0)
                }
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
                    let keys: Vec<KeyCode> =
                        v.split(',').filter_map(|s| parse_key(s.trim())).collect();
                    if let Ok(k) = keys.try_into() {
                        cfg.keys = k;
                    }
                }
                "preset_keys" => {
                    let keys: Vec<KeyCode> =
                        v.split(',').filter_map(|s| parse_key(s.trim())).collect();
                    if let Ok(k) = keys.try_into() {
                        cfg.preset_keys = k;
                    }
                }
                "touch_layout" => {
                    cfg.touch_layout = TouchLayout::parse(v).unwrap_or(cfg.touch_layout)
                }
                "screenshot_to" => {
                    cfg.screenshot_to = ScreenshotTo::parse(v).unwrap_or(cfg.screenshot_to)
                }
                "language" => cfg.language = crate::lang::Lang::parse(v).unwrap_or(cfg.language),
                "show_playfield_size" => cfg.show_playfield_size = v != "false",
                "wheel_resizes_playfield" => cfg.wheel_resizes_playfield = v == "true",
                "show_converts" => cfg.show_converts = v == "true",
                "song_headings" => cfg.song_headings = v == "true",
                "bg_dim_black" => cfg.bg_dim_black = v == "true",
                "interface_hidden" => cfg.interface_hidden = v == "true",
                "interface_hides" => cfg.interface_hides = crate::hud::Parts::parse_saved(v),
                "hud_layout" => cfg.hud = crate::hud::Layout::parse(v).unwrap_or_default(),
                "hud_preset" => cfg.hud_presets.extend(crate::hud::parse_preset(v)),
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
                    cfg.song_list_share = v
                        .parse::<f32>()
                        .ok()
                        .filter(|f| f.is_finite())
                        .map_or(cfg.song_list_share, |f| f.clamp(min, max));
                }
                "song_preview_size" => {
                    let (min, max) = crate::preview::PREVIEW_SIZE;
                    cfg.song_preview_size = v
                        .parse::<f32>()
                        .ok()
                        .filter(|f| f.is_finite())
                        .map_or(cfg.song_preview_size, |f| f.clamp(min, max));
                }
                k if k.starts_with("hotkey.") => cfg.hotkeys.parse_line(&k["hotkey.".len()..], v),
                "saved_presets" => {
                    cfg.saved_presets = v.split(',').filter_map(crate::layout::parse_size).collect()
                }
                "preset_spots" => cfg.preset_spots = v.split(',').filter_map(parse_spot).collect(),
                "skin" => cfg.skin = v.to_string(),
                "skin_part" => cfg.skin_parts.parse(v),
                "map_skins" => cfg.map_skins = v != "false",
                "songs_folder" => cfg.songs_folder = (!v.is_empty()).then(|| PathBuf::from(v)),
                "multiverse" => cfg.universes = crate::multiverse::parse(v),
                "multiverse_preset" => cfg
                    .universe_presets
                    .extend(crate::multiverse::Preset::parse(v)),
                "note_colours" => cfg.note_colours = crate::multiverse::Colours::parse(v),
                "random_seed" => cfg.random_seed = v.parse().unwrap_or(cfg.random_seed),
                "shapeshift" => cfg.shapeshift = crate::shapeshift::Config::parse(v),
                "tact" => cfg.tact = crate::tact::parse(v),
                "od_override" => cfg.od_override = crate::od_override::parse(v),
                "favourite_mods" => cfg.favourite_mods = crate::mods::Mods::parse_set(v),
                "mod_preset" => {
                    if cfg.mod_presets.len() < crate::mods::MAX_PRESETS
                        && let Some(p) = crate::mods::Preset::parse(v)
                    {
                        cfg.mod_presets.push(p);
                    }
                }
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
            cfg.scroll_speed = ((cfg.scroll_speed * OLD_SCROLL_SPEED / SCROLL_STEP).round()
                * SCROLL_STEP)
                .clamp(0.3, 4.0);
        }
        cfg.read_offset(old_offset, new_offset);
        cfg
    }

    /// Settles the offset from the config's lines: `old` is `offset_ms`, `new` is `offset`.
    ///
    /// - Both: this build wrote them together (`to_text`), the old one worked out with the latency
    ///   it also wrote. If `offset_ms` is not what `offset` makes of it, an older build changed it
    ///   since, and the player's choice is the newer one: `offset` is worked out from it again.
    /// - Only `offset_ms`: a config from before the osu! style (or one an older build stripped of
    ///   the new lines). It plays as it was and is converted once the latency is known
    ///   (`set_latency`).
    /// - Neither: a new player, at osu!'s 0.
    ///
    /// On `OffsetScale::Sound`, `offset_ms` is the player's setting whenever it's there.
    fn read_offset(&mut self, old: Option<f64>, new: Option<f64>) {
        if !(self.offset_latency_ms.is_finite() && self.offset_latency_ms >= 0.0) {
            self.offset_latency_ms = 0.0;
        }
        if self.offset_scale == OffsetScale::Sound {
            self.offset_ms =
                old.unwrap_or_else(|| play_offset_of(new.unwrap_or(0.0), self.offset_latency_ms));
            self.sync_offset();
            return;
        }
        match (old, new) {
            (Some(x), Some(o)) => {
                self.offset_osu_ms = o;
                if (x - play_offset_of(o, self.offset_latency_ms)).abs() > 0.01 {
                    self.offset_osu_ms = osu_offset_of(x, self.offset_latency_ms);
                }
                self.sync_offset();
            }
            (Some(x), None) => {
                self.offset_ms = x;
                self.offset_pending = true;
            }
            (None, Some(o)) => {
                self.offset_osu_ms = o;
                self.sync_offset();
            }
            (None, None) => self.sync_offset(),
        }
    }

    /// Works one offset out from the other and the latency: on the osu! scale `offset_ms` from
    /// the osu! offset (not while the old offset is still waiting to be converted: that one is
    /// played as it is), on the sound's the other way round.
    pub fn sync_offset(&mut self) {
        match self.offset_scale {
            OffsetScale::Osu if !self.offset_pending => {
                self.offset_ms = play_offset_of(self.offset_osu_ms, self.offset_latency_ms)
            }
            OffsetScale::Osu => {}
            OffsetScale::Sound => {
                self.offset_pending = false;
                self.offset_osu_ms = osu_offset_of(self.offset_ms, self.offset_latency_ms);
            }
        }
    }

    /// Sets the global offset the osu! way: positive brings the notes earlier.
    pub fn set_osu_offset(&mut self, osu: f64) {
        // A player's own choice leaves nothing to convert.
        self.offset_pending = false;
        self.offset_osu_ms = osu;
        self.offset_ms = play_offset_of(osu, self.offset_latency_ms);
        self.sync_offset();
    }

    /// The global offset on the player's scale (`offset_scale`). Either way positive brings the
    /// notes earlier.
    pub fn shown_offset(&self) -> f64 {
        match self.offset_scale {
            OffsetScale::Osu if !self.offset_pending => self.offset_osu_ms,
            _ => self.shown_of_play(self.offset_ms),
        }
    }

    /// The global offset on the player's scale that a clock offset `play` amounts to.
    pub fn shown_of_play(&self, play: f64) -> f64 {
        match self.offset_scale {
            OffsetScale::Osu => osu_offset_of(play, self.offset_latency_ms),
            OffsetScale::Sound => -play,
        }
    }

    /// Sets the global offset on the player's scale.
    pub fn set_shown_offset(&mut self, shown: f64) {
        match self.offset_scale {
            OffsetScale::Osu => self.set_osu_offset(shown),
            OffsetScale::Sound => {
                self.offset_ms = -shown;
                self.sync_offset();
            }
        }
    }

    /// Changes the scale. The play stays exactly as it is now: only the number's zero moves, and
    /// what stays put when the latency changes.
    pub fn set_offset_scale(&mut self, scale: OffsetScale) {
        self.offset_scale = scale;
        self.sync_offset();
    }

    /// Follows the output latency the clock takes off. `settled`: the engine has stopped changing
    /// its mind about it (`Engine::latency_settled`). A config waiting to be converted
    /// (`offset_pending`) is, at the first settled latency, given the osu! offset that plays the
    /// same as its old one did, and from then on `offset_ms` follows the latency, so the notes are
    /// where they were. Returns whether the config should be saved now.
    pub fn set_latency(&mut self, latency: f64, settled: bool) -> bool {
        if !latency.is_finite() || latency < 0.0 {
            return false;
        }
        let mut converted = false;
        if self.offset_pending && settled {
            self.offset_osu_ms = osu_offset_of(self.offset_ms, latency);
            self.offset_pending = false;
            converted = true;
        }
        self.offset_latency_ms = latency;
        self.sync_offset();
        converted
    }

    /// The offset lines of the config file. Older builds read `offset_ms` and keep the other
    /// lines when they save: if one changes `offset_ms`, the next start notices (`read_offset`).
    fn offset_lines(&self) -> String {
        if self.offset_pending {
            return format!("offset_ms = {}", self.offset_ms);
        }
        format!(
            "# what offset is set by: osu (0 is osu!'s, the output latency counts as there) | sound\n\
             # (0 is hitting exactly on the sound you hear, the latency is taken off)\n\
             offset_scale = {}\n\
             # the global offset on the osu! scale: positive brings the notes earlier\n\
             offset = {}\n\
             # the same for older versions of the game (positive shifts the notes later), worked out with\n\
             # the output latency below; if it is changed there, the new value is the one used\n\
             offset_ms = {}\n\
             offset_latency_ms = {}",
            self.offset_scale.name(),
            self.offset_osu_ms,
            self.offset_ms,
            self.offset_latency_ms
        )
    }

    /// The settings as the config file has them.
    pub(crate) fn to_text(&self) -> String {
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
             # the interface's language: en, ja or ru\n\
             language = {}\n\
             show_playfield_size = {}\n\
             # your own playfield sizes (preset list, in this order)\n\
             saved_presets = {}\n\
             # where each playfield size was last put: WxH@x/y (fractions of the screen)\n\
             preset_spots = {}\n\
             {}\n\
             # notes drawn this many ms early to make up for display latency (judging is unaffected)\n\
             visual_offset_ms = {}\n\
             # 1.00 = osu!'s scroll speed\n\
             scroll_speed = {:.2}\n\
             scroll_speed_osu = true\n\
             master_volume = {:.2}\n\
             music_volume = {:.2}\n\
             unfocused_volume = {:.2}\n\
             fx_volume = {:.2}\n\
             # mailbox | immediate | vsync\n\
             present_mode = {}\n\
             # menus always use vsync (gameplay uses present_mode)\n\
             menu_vsync = {}\n\
             # draw at half resolution, scaled up exactly 2x (crisp pixels, faster)\n\
             half_resolution = {}\n\
             # auto | vulkan | dx12 (Windows) | gl; auto picks DX12 on Windows where Vulkan is known to crash\n\
             graphics_api = {}\n\
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
             epilepsy_warning = {}\n\
             main_menu = {}\n\
             menu_notes = {}\n\
             # dims the song's notes on the main menu's lane, 0 (none) to 1 (hidden)\n\
             menu_notes_dim = {:.2}\n\
             guide_seen = {}\n\
             check_updates = {}\n\
             # countdown before play continues after pausing (ms)\n\
             resume_delay_ms = {:.0}\n\
             # song select order: artist, title, difficulty, length, bpm, recently played, date added\n\
             sort = {}\n\
             kiai_effects = {}\n\
             # GREAT hits without their flash and label (only OKs and misses show)\n\
             hide_great = {}\n\
             # a play failed by Sudden Death or Perfect starts over at once\n\
             fail_restart = {}\n\
             # the lane and the drum's panel pure black\n\
             black_lane = {}\n\
             # .tko notes drawn as what they are, whatever colour or size their map shows them as\n\
             true_notes = {}\n\
             # .tko maps' flashes, glitches and camera moves toned down\n\
             calm_maps = {}\n\
             # the drum keys move through song select and the pause menu\n\
             drum_menus = {}\n\
             # the coloured flash on the hit target as notes are hit\n\
             hit_flash = {}\n\
             show_offset_hint = {}\n\
             offset_hint_ur_scaled = {}\n\
             # offset/UR line during play (Tab toggles)\n\
             show_map_info = {}\n\
             # mods picked on song select (F1), e.g. HD,HR\n\
             mods = {}\n",
            keys.join(", "),
            self.preset_keys
                .iter()
                .map(|k| key_name(*k))
                .collect::<Vec<_>>()
                .join(", "),
            self.touch_layout.name(),
            self.screenshot_to.key(),
            self.language.code(),
            self.show_playfield_size,
            self.saved_presets
                .iter()
                .map(|(w, h)| format!("{w}x{h}"))
                .collect::<Vec<_>>()
                .join(", "),
            self.preset_spots
                .iter()
                .map(|((w, h), (x, y))| format!("{w}x{h}@{x:.4}/{y:.4}"))
                .collect::<Vec<_>>()
                .join(", "),
            self.offset_lines(),
            self.visual_offset_ms,
            self.scroll_speed,
            self.master_volume,
            self.music_volume,
            self.unfocused_volume,
            self.fx_volume,
            self.present.name(),
            self.menu_vsync,
            self.half_resolution,
            self.graphics_api,
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
            self.epilepsy_warning,
            self.main_menu,
            self.menu_notes,
            self.menu_notes_dim,
            self.guide_seen,
            self.check_updates,
            self.resume_delay_ms,
            self.sort,
            self.kiai_effects,
            self.hide_great,
            self.fail_restart,
            self.black_lane,
            self.true_notes,
            self.calm_maps,
            self.drum_menus,
            self.hit_flash,
            self.show_offset_hint,
            self.offset_hint_ur_scaled,
            self.show_map_info,
            self.mods.to_config(),
        );
        out.push_str(&format!(
            "# the live scoreboard in lobby matches (Tab toggles)\nmp_scoreboard = {}\n",
            self.mp_scoreboard
        ));
        out.push_str(&format!(
            "# how autoplay hits: full, semi, single, roll or ddkk; binding 1234, 1324 or 4231; left-handed\nautoplay_style = {}\nautoplay_binding = {}\nautoplay_left_handed = {}\n",
            self.autoplay.style.key(),
            crate::autoplay::BINDINGS[self.autoplay.binding],
            self.autoplay.left_handed
        ));
        out.push_str(&format!(
            "# osu! skin (folder name in a Skins folder), empty = built-in\nskin = {}\n",
            self.skin
        ));
        out.push_str("# parts of the skin from elsewhere: skin_part = <part> = <skin folder name> (or <built-in>)\n");
        for line in self.skin_parts.to_config() {
            out.push_str(&format!("skin_part = {line}\n"));
        }
        out.push_str(&format!(
            "# draw maps with the skin images in their own folder\nmap_skins = {}\n",
            self.map_skins
        ));
        out.push_str(&format!(
            "# main menu notes glitch when hit, and the whole screen with them and between menus\nmenu_glitch = {}\n",
            self.menu_glitch
        ));
        out.push_str(&format!(
            "# interface colour: red (the default), pink, purple, blue, teal, green, yellow, orange or #rrggbb\naccent = {}\n",
            crate::render::accent_name(self.accent)
        ));
        out.push_str(&format!(
            "# Discord status (\"Playing taiko.rs\"); app id empty = the built-in one\ndiscord_presence = {}\ndiscord_app_id = {}\n\
             # minutes without input before the idle screen (0 = never)\nidle_minutes = {}\n",
            self.discord_presence, self.discord_app_id, self.idle_minutes
        ));
        out.push_str(&format!("# after a crash, the next start sends its report to the developers\nsend_crash_reports = {}\n", self.send_crash_reports));
        out.push_str(&format!("# the map editor's first-run tour was seen (Help > Take the tour shows it again)\neditor_tour_done = {}\n", self.editor_tour_done));
        out.push_str(&format!("# in the map editor a left drag on empty timeline pans it (View > Left drag pans)\neditor_left_pan = {}\n", self.editor_left_pan));
        out.push_str(&format!("# in the map editor a red line's new time or BPM moves its notes along (Timing > Notes follow)\neditor_follow_timing = {}\n", self.editor_follow_timing));
        if !self.editor_snap_presets.is_empty() {
            out.push_str("# the map editor's own snap presets: beat divisions, then the name (one line each)\n");
            for p in self.editor_snap_presets.iter().take(MAX_SNAP_PRESETS) {
                out.push_str(&format!("editor_snap_preset = {p}\n"));
            }
        }
        if !self.editor_snap_active.is_empty() {
            out.push_str(&format!("# the snap preset in use in the map editor (none: all snaps)\neditor_snap_active = {}\n", self.editor_snap_active));
        }
        if !self.editor_recent.is_empty() {
            out.push_str("# maps lately opened in the editor, the latest first (one line each)\n");
            for path in self.editor_recent.iter().take(MAX_RECENT_MAPS) {
                out.push_str(&format!("editor_recent = {path}\n"));
            }
        }
        out.push_str(&format!(
            "# chats: star out swear words; a chime when a friend messages you\nchat_filter = {}\nmessage_sound = {}\n",
            self.chat_filter, self.message_sound
        ));
        out.push_str(&format!(
            "# pop-ups: a friend starts a map / comes online / messages you, a global chat line mentions you, friend requests\nnotify_playing = {}\nnotify_online = {}\nnotify_messages = {}\nnotify_mentions = {}\nnotify_requests = {}\n",
            self.notify_playing, self.notify_online, self.notify_messages, self.notify_mentions, self.notify_requests
        ));
        out.push_str(&format!(
            "# multiplayer\nmp_name = {}\nmp_port = {}\nmp_address = {}\n",
            self.mp_name, self.mp_port, self.mp_address
        ));
        out.push_str(&format!(
            "# online server (accounts, lobbies) and your saved login there\nonline_server = {}\nonline_user = {}\nonline_token = {}\n",
            self.online_server, self.online_user, self.online_token
        ));
        out.push_str(&format!(
            "# with vsync, draw notes where they'll be once the frame is on screen; ms added to that\nvsync_compensation = {}\nvsync_tune_ms = {:.0}\n",
            self.vsync_compensation, self.vsync_tune_ms
        ));
        let (lw, lh) = self.letterbox_size;
        out.push_str(&format!(
            "# in fullscreen, the game at this size with black around it, moved from the centre (percent)\nletterbox = {}\nletterbox_size = {lw}x{lh}\nletterbox_x = {}\nletterbox_y = {}\n",
            self.letterbox, self.letterbox_pos.0, self.letterbox_pos.1
        ));
        let window = self
            .window_size
            .map(|(w, h)| format!("{w}x{h}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "# the window's size out of fullscreen (empty = 1280x720)\nwindow_size = {window}\n"
        ));
        let data = self
            .data_folder
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        out.push_str(&format!(
            "# where replays, scores, videos, taiko.rs's Songs and Skins and caches are (empty = the usual places); move it from Settings\ndata_folder = {data}\n"
        ));
        out.push_str(&format!(
            "# the mouse wheel resizes the playfield during play\nwheel_resizes_playfield = {}\n",
            self.wheel_resizes_playfield
        ));
        out.push_str(&format!(
            "# song select lists converts too\nshow_converts = {}\n",
            self.show_converts
        ));
        out.push_str(&format!("# song select groups songs under headings (false: one list, by difficulty each difficulty on its own)\nsong_headings = {}\n", self.song_headings));
        out.push_str(&format!("# during songs the background dims to black (false: the interface's grey)\nbg_dim_black = {}\n", self.bg_dim_black));
        out.push_str(&format!(
            "# the interface during play: hidden or not, what hiding it hides (score, accuracy, pp, combo, progress, health, mods, hit_error, offset_hint, judgements, hit_animation, scoreboard, spectators, ur, counts; ! before one that came later: not hidden) and its key\ninterface_hidden = {}\ninterface_hides = {}\ninterface_key = {}\n",
            self.interface_hidden,
            self.interface_hides.format_saved(),
            interface_key_name(self.interface_key)
        ));
        out.push_str(&format!(
            "# song select: the song list's share of the width (drag the edge beside it); the preview's size is no longer used\nsong_list_share = {:.3}\nsong_preview_size = {:.3}\n",
            self.song_list_share, self.song_preview_size
        ));
        out.push_str(&self.hotkeys.config_lines());
        out.push_str(&format!(
            "# the build last started (what's new shows after an update)\nlast_build = {}\n",
            self.last_build
        ));
        out.push_str(&self.video.config_lines());
        out.push_str(&format!(
            "# Multiverse's universes, one per playfield, ; between: colours, note shape, speed and mods (C in the mod select)\nmultiverse = {}\n",
            crate::multiverse::format(&self.universes)
        ));
        out.push_str(&format!(
            "# where the interface's parts go (the interface editor), and your own layouts, one per line: name: layout\nhud_layout = {}\n",
            self.hud.format()
        ));
        for (name, layout) in &self.hud_presets {
            out.push_str(&format!(
                "hud_preset = {}\n",
                crate::hud::format_preset(name, layout)
            ));
        }
        out.push_str("# your own Multiverse presets, one per line: name: universes as above\n");
        for p in &self.universe_presets {
            out.push_str(&format!("multiverse_preset = {}\n", p.format()));
        }
        out.push_str(&format!(
            "# Random: repeatable note colours (C in the mod select)\nrandom_seed = {}\n",
            self.random_seed
        ));
        out.push_str(&format!(
            "# Shapeshift: bars per shape (4/8/16/32), bars to morph (0.5/1/2/4), order shuffle/fixed, shapes in order (- = left out)\nshapeshift = {}\n",
            self.shapeshift.format()
        ));
        out.push_str(&format!(
            "# Tact: how often notes move (2bars, 1bar, 1/2bar, 1beat, 1/2beat, 1/4beat)\ntact = {}\n",
            crate::tact::format(self.tact)
        ));
        out.push_str(&format!(
            "# OD Override: the OD played at (0 to 20)\nod_override = {:.1}\n",
            self.od_override
        ));
        out.push_str(&format!(
            "# favourite mods (right-click or F in the mod select), e.g. HD,HR\nfavourite_mods = {}\n",
            self.favourite_mods.to_config()
        ));
        if !self.mod_presets.is_empty() {
            out.push_str(
                "# mod presets (S saves one in the mod select, 1-9 apply them): speed|mods|name\n",
            );
        }
        for p in &self.mod_presets {
            out.push_str(&format!("mod_preset = {}\n", p.format()));
        }
        out.push_str(&format!(
            "# note colours without Multiverse: a palette (classic, neon, sunset...) or custom #don/#kat\nnote_colours = {}\n",
            self.note_colours.format()
        ));
        let folder = self
            .songs_folder
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        out.push_str(&format!("# Songs folder to scan instead of finding osu!'s (empty = find it)\nsongs_folder = {folder}\n"));
        out.push_str("# extra song folders / .osz files, one per line\n");
        for s in &self.songs {
            out.push_str(&format!("songs = {}\n", s.display()));
        }
        out
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        let _ = SAVES
            .get_or_init(save_worker)
            .send(Save::Write(path, self.to_text()));
    }
}

enum Save {
    Write(PathBuf, String),
    Flush(mpsc::Sender<()>),
}

static SAVES: OnceLock<mpsc::Sender<Save>> = OnceLock::new();

fn write_pending(pending: &mut std::collections::HashMap<PathBuf, String>) {
    for (path, mut text) in pending.drain() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        keep_unknown(
            &mut text,
            &std::fs::read_to_string(&path).unwrap_or_default(),
        );
        let _ = std::fs::write(path, text);
    }
}

fn save_worker() -> mpsc::Sender<Save> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("settings writer".into())
        .spawn(move || {
            let mut pending = std::collections::HashMap::new();
            while let Ok(first) = rx.recv() {
                for job in std::iter::once(first).chain(rx.try_iter()) {
                    match job {
                        Save::Write(path, text) => {
                            pending.insert(path, text);
                        }
                        Save::Flush(done) => {
                            write_pending(&mut pending);
                            let _ = done.send(());
                        }
                    }
                }
                write_pending(&mut pending);
            }
        })
        .expect("settings writer");
    tx
}

/// Called outside State's lock on shutdown, after queuing the final settings snapshot.
pub fn flush_saves() {
    if let Some(tx) = SAVES.get() {
        let (done, wait) = mpsc::channel();
        if tx.send(Save::Flush(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

/// The setting on a config line, if it has one.
fn setting(line: &str) -> Option<&str> {
    let line = line.trim();
    (!line.starts_with('#'))
        .then(|| line.split_once('=').map(|(k, _)| k.trim()))
        .flatten()
}

/// Adds to `out` (the config about to be written) what's in `old` (the file as it is) that this
/// build doesn't know: a newer build's settings. Without it, one start of an older build (another
/// channel, a checkout) loses them for good, hotkeys moved and all.
fn keep_unknown(out: &mut String, old: &str) {
    let known: std::collections::HashSet<&str> = out.lines().filter_map(setting).collect();
    // These have a line each only when there's something to say; none left means none left.
    let ours = |k: &str| {
        known.contains(k)
            || matches!(
                k,
                "songs"
                    | "multiverse_preset"
                    | "mod_preset"
                    | "skin_part"
                    | "editor_snap_preset"
                    | "editor_snap_active"
            )
            || k.strip_prefix("hotkey.")
                .is_some_and(crate::keys::Hotkey::known)
    };
    let kept: Vec<&str> = old
        .lines()
        .filter(|l| setting(l).is_some_and(|k| !ours(k)))
        .collect();
    if !kept.is_empty() {
        out.push_str("# from another version of the game, kept for it\n");
        out.push_str(&(kept.join("\n") + "\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_map_offsets_are_turned_round_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("taiko-map-offsets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(OLD_SIGN_MAP_OFFSETS), "00000000000000ab\t12\n").unwrap();
        let first = load_map_offsets_in(&dir);
        assert_eq!(first.get(&0xab), Some(&-12.0));
        assert!(!dir.join(OLD_SIGN_MAP_OFFSETS).exists());
        assert!(
            dir.join(OLD_SIGN_KEPT).exists(),
            "the old file is kept to go back to"
        );
        // From then on it's the new file, as it is.
        assert_eq!(load_map_offsets_in(&dir), first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn background_saves_flush_the_latest_snapshot_and_preserve_unknown_settings() {
        let dir = std::env::temp_dir().join(format!("taiko-config-worker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings");
        std::fs::write(&path, "volume = 0\nfuture_option = keep\n").unwrap();
        let tx = save_worker();
        for i in 0..100 {
            tx.send(Save::Write(path.clone(), format!("volume = {i}\n")))
                .unwrap();
        }
        let (done, wait) = mpsc::channel();
        tx.send(Save::Flush(done)).unwrap();
        wait.recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("volume = 99\n"));
        assert_eq!(text.matches("future_option = keep").count(), 1);
        drop(tx);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_keeps_the_settings_of_a_newer_build() {
        let old =
            "# a comment\nkeys = D, F, J, K\nfuture_setting = 3\nhotkey.future_hotkey = none\n";
        let mut out = "keys = A, S, K, L\n".to_string();
        keep_unknown(&mut out, old);
        assert_eq!(
            out,
            "keys = A, S, K, L\n# from another version of the game, kept for it\nfuture_setting = 3\nhotkey.future_hotkey = none\n"
        );
        // Written again, they stay once, and a build that knows them takes over.
        let mut again = "keys = A, S, K, L\n".to_string();
        keep_unknown(&mut again, &out);
        assert_eq!(again, out);
        let mut newer =
            "keys = A, S, K, L\nfuture_setting = 4\nhotkey.future_hotkey = none\n".to_string();
        keep_unknown(&mut newer, &out);
        assert_eq!(
            newer,
            "keys = A, S, K, L\nfuture_setting = 4\nhotkey.future_hotkey = none\n"
        );
    }

    #[test]
    fn saving_doesnt_bring_back_what_was_taken_out() {
        // A hotkey back on its default, the last extra song folder removed: no line for them now.
        let mut out = "keys = A, S, K, L\n".to_string();
        keep_unknown(
            &mut out,
            "hotkey.apply_offset = none\nsongs = /a\nmultiverse_preset = x: y\nskin_part = a = b\n",
        );
        assert_eq!(out, "keys = A, S, K, L\n");
    }

    #[test]
    fn snap_presets_round_trip_and_a_removed_one_stays_removed() {
        let cfg = Config {
            editor_snap_presets: vec!["1,2,4,8|Basic".into(), "3,6,12|Triplets".into()],
            editor_snap_active: "Triplets".into(),
            ..Config::default()
        };
        let text = cfg.to_text();
        let back = Config::parse_text(&text);
        assert_eq!(back.editor_snap_presets, cfg.editor_snap_presets);
        assert_eq!(back.editor_snap_active, "Triplets");
        // With none left, the old lines are not kept as another version's.
        let mut out = Config::default().to_text();
        keep_unknown(&mut out, &text);
        assert!(!out.contains("editor_snap"));
    }

    #[test]
    fn mod_presets_round_trip_and_a_removed_one_stays_removed() {
        use crate::mods::{MAX_PRESETS, Preset};
        let preset = |name: &str| Preset {
            name: name.into(),
            mods: Mods::parse("HD,DT"),
            rate: 1.1,
        };
        let cfg = Config {
            mod_presets: vec![preset("acc"), preset("speed")],
            ..Config::default()
        };
        let text = cfg.to_text();
        assert_eq!(Config::parse_text(&text).mod_presets, cfg.mod_presets);
        let mut out = Config::default().to_text();
        keep_unknown(&mut out, &text);
        assert!(!out.contains("mod_preset"));
        // No more than fit the row.
        let many = (0..MAX_PRESETS + 3)
            .map(|i| format!("mod_preset = {}\n", preset(&i.to_string()).format()))
            .collect::<String>();
        assert_eq!(Config::parse_text(&many).mod_presets.len(), MAX_PRESETS);
    }

    #[test]
    fn random_seeds_round_trip_including_legacy_and_maximum() {
        for seed in [0, 42, u64::MAX] {
            let cfg = Config::parse_text(&format!("random_seed = {seed}\n"));
            assert_eq!(cfg.random_seed, seed);
            assert_eq!(Config::parse_text(&cfg.to_text()).random_seed, seed);
        }
        let cfg = Config::parse_text("random_seed = 42\nrandom_seed = invalid\n");
        assert_eq!(cfg.random_seed, 42);
    }

    #[test]
    fn only_a_first_launch_starts_at_osus_zero() {
        let new = Config::default();
        assert_eq!((new.offset_osu_ms, new.offset_pending), (0.0, false));
        assert_eq!(new.offset_ms, -crate::beatmap::OSU_TIMING_MS);
        let blank = Config::parse_text("");
        assert_eq!((blank.offset_osu_ms, blank.offset_pending), (0.0, false));
        // A config with the old line only plays as it did until the latency is known.
        for x in [0.0, -12.0, 40.0] {
            let old = Config::parse_text(&format!("offset_ms = {x}\n"));
            assert_eq!((old.offset_ms, old.offset_pending), (x, true));
        }
    }

    #[test]
    fn the_two_offset_conventions_convert_both_ways() {
        for (x, l, o) in [(-25.0, 32.0, -30.0), (0.0, 7.0, -30.0), (-50.0, 27.0, 0.0)] {
            assert_eq!(osu_offset_of(x, l), o);
            assert_eq!(play_offset_of(o, l), x);
        }
        // An earlier osu! offset is later notes here.
        assert!(play_offset_of(10.0, 20.0) < play_offset_of(0.0, 20.0));
    }

    #[test]
    fn an_old_offset_is_converted_at_the_first_settled_latency_and_plays_the_same() {
        for (x, l, o) in [(-25.0, 32.0, -30.0), (0.0, 7.0, -30.0), (-50.0, 27.0, 0.0)] {
            let mut cfg = Config::parse_text(&format!("offset_ms = {x}\n"));
            // Until the latency is settled it plays as it did, and saves as it was.
            assert!(!cfg.set_latency(l, false));
            assert_eq!((cfg.offset_ms, cfg.offset_pending), (x, true));
            assert!(!cfg.to_text().contains("\noffset = "));
            assert!(cfg.to_text().contains(&format!("\noffset_ms = {x}\n")));
            // Then it is the osu! offset that feels the same, and the play offset hasn't moved.
            assert!(cfg.set_latency(l, true), "converted: to be saved");
            assert_eq!(
                (cfg.offset_osu_ms, cfg.offset_ms, cfg.offset_pending),
                (o, x, false)
            );
            assert!(!cfg.set_latency(l, true), "once only");
            // The latency changing later moves the play offset with it; the osu! one stays.
            cfg.set_latency(l + 5.0, true);
            assert_eq!((cfg.offset_osu_ms, cfg.offset_ms), (o, x - 5.0));
        }
    }

    #[test]
    fn switching_the_offset_scale_keeps_the_timing_and_shows_it_by_the_sound() {
        let mut cfg = Config::default();
        cfg.set_latency(6.666666, true);
        cfg.set_osu_offset(-20.0);
        let play = cfg.offset_ms;
        cfg.set_offset_scale(OffsetScale::Sound);
        assert_eq!(cfg.offset_ms, play, "the timing doesn't move");
        // -20 on osu!'s scale at 6.7 ms of latency: hitting about 10 ms ahead of the sound.
        assert!((cfg.shown_offset() - 9.666666).abs() < 1e-9);
        // The same direction on both: + brings the notes earlier, so the clock's offset down.
        cfg.set_shown_offset(cfg.shown_offset() + 5.0);
        assert!(cfg.offset_ms < play);
        cfg.set_offset_scale(OffsetScale::Osu);
        assert!((cfg.shown_offset() - -15.0).abs() < 1e-9);
    }

    #[test]
    fn by_the_sound_the_timing_stays_put_when_the_latency_changes() {
        let mut cfg = Config::default();
        cfg.set_latency(4.67, true);
        cfg.set_offset_scale(OffsetScale::Sound);
        cfg.set_shown_offset(10.0);
        assert_eq!(cfg.offset_ms, -10.0);
        cfg.set_latency(6.67, true);
        assert_eq!((cfg.offset_ms, cfg.shown_offset()), (-10.0, 10.0));
        // On osu!'s, the osu! number stays and the clock's offset follows the latency.
        cfg.set_offset_scale(OffsetScale::Osu);
        let osu = cfg.shown_offset();
        cfg.set_latency(4.67, true);
        assert_eq!(cfg.shown_offset(), osu);
        assert!((cfg.offset_ms - -8.0).abs() < 1e-9);
    }

    #[test]
    fn the_offset_scale_and_a_sound_offset_survive_saving() {
        let mut cfg = Config::default();
        cfg.set_latency(2.0, true);
        cfg.set_offset_scale(OffsetScale::Sound);
        cfg.set_shown_offset(11.0);
        let back = Config::parse_text(&cfg.to_text());
        assert_eq!(back.offset_scale, OffsetScale::Sound);
        assert_eq!((back.offset_ms, back.shown_offset()), (-11.0, 11.0));
        // osu!'s line is still there for builds that only know it: the same timing.
        assert_eq!(back.offset_osu_ms, osu_offset_of(-11.0, 2.0));
        // A config from before the scale, or an older build, stays on osu!'s.
        assert_eq!(
            Config::parse_text("offset = -5\n").offset_scale,
            OffsetScale::Osu
        );
    }

    #[test]
    fn an_old_offset_by_the_sound_needs_no_converting() {
        let mut cfg = Config::parse_text("offset_scale = sound\noffset_ms = -22\n");
        assert!(!cfg.offset_pending);
        assert_eq!(cfg.shown_offset(), 22.0);
        assert!(!cfg.set_latency(6.7, true), "nothing to convert");
        assert_eq!(cfg.offset_ms, -22.0);
    }

    #[test]
    fn both_offset_keys_survive_a_round_trip_and_an_older_builds_edit_is_taken() {
        let mut cfg = Config::default();
        cfg.set_latency(32.0, true);
        cfg.set_osu_offset(-30.0);
        assert_eq!(cfg.offset_ms, -25.0);
        let text = cfg.to_text();
        assert!(text.contains("\noffset = -30\n"));
        assert!(text.contains("\noffset_ms = -25\n"));
        assert!(text.contains("\noffset_latency_ms = 32\n"));
        let back = Config::parse_text(&text);
        assert_eq!(
            (back.offset_osu_ms, back.offset_ms, back.offset_pending),
            (-30.0, -25.0, false)
        );
        assert_eq!(back.offset_latency_ms, 32.0);
        // An older build moves its offset_ms by 10 and leaves the other lines be.
        let edited = text.replace("\noffset_ms = -25\n", "\noffset_ms = -35\n");
        let back = Config::parse_text(&edited);
        assert_eq!((back.offset_osu_ms, back.offset_ms), (-20.0, -35.0));
        // One that dropped the new lines altogether is converted again.
        let stripped: String = edited
            .lines()
            .filter(|l| !l.starts_with("offset = ") && !l.starts_with("offset_latency_ms"))
            .map(|l| format!("{l}\n"))
            .collect();
        let back = Config::parse_text(&stripped);
        assert!(back.offset_pending);
        assert_eq!(back.offset_ms, -35.0);
    }

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
            let read: String = keys
                .iter()
                .map(|&k| if k == 0 || k == 3 { 'K' } else { 'D' })
                .collect();
            assert_eq!(read, layout.name());
            // Both columns of a colour are different hands, so together they make a big hit.
            for kat in [true, false] {
                let hands: Vec<bool> = keys
                    .iter()
                    .filter(|&&k| (k == 0 || k == 3) == kat)
                    .map(|&k| k < 2)
                    .collect();
                assert_ne!(hands[0], hands[1], "{}", layout.name());
            }
            assert_eq!(
                TouchLayout::parse(&layout.name().to_lowercase()),
                Some(layout)
            );
        }
        assert_eq!(TouchLayout::Kddk.step(-1), TouchLayout::Ddkk);
    }

    #[test]
    fn preset_spots_read_back() {
        assert_eq!(
            parse_spot(" 1280x720@0.1000/0.2500"),
            Some(((1280, 720), (0.1, 0.25)))
        );
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
        for (path, text) in [
            ("replays/1.json", "replay"),
            ("plays.tsv", "plays"),
            ("Songs/1 map/map.osu", "osu"),
            ("Skins/mine/skin.ini", "skin"),
            ("cache/osz/x/y.osu", "cache"),
        ] {
            assert_eq!(
                std::fs::read_to_string(to.join(path)).unwrap(),
                text,
                "{path}"
            );
            assert!(!from.join(path).exists(), "{path} still in the old folder");
        }
        assert_eq!(
            done.load(std::sync::atomic::Ordering::Relaxed),
            total.load(std::sync::atomic::Ordering::Relaxed)
        );
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
        assert_eq!(
            std::fs::read_to_string(root.join("z/b/c.txt")).unwrap(),
            "hello"
        );
        assert_eq!(done.load(std::sync::atomic::Ordering::Relaxed), 7);
        assert_eq!(size_of(&root.join("z")), 7);
        remove_all(&root.join("a")).unwrap();
        assert!(!root.join("a").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
