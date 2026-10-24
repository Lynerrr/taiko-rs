//! Settings persisted as a plain `key = value` file: `$XDG_CONFIG_HOME/taiko-rs/config.txt` on
//! Linux, `%APPDATA%\taiko-rs\config.txt` on Windows.

use std::path::PathBuf;

use winit::keyboard::KeyCode;

use crate::layout::Area;
use crate::mods::Mods;
use crate::render::PresentChoice;

pub const RATE_RANGE: (f64, f64) = (0.5, 2.0);
pub const FPS_CAPS: [u32; 7] = [0, 2000, 1000, 500, 240, 144, 60];
/// What scroll speed 1.0 was before it matched osu!, in today's terms (at 16:9: 0.7 virtual
/// units per osu!px against the 639/693 lane ratio `game::osu_speed_scale` gives).
const OLD_SCROLL_SPEED: f64 = 0.7 * 693.333 / 639.333;

pub struct Config {
    /// Positive when your audio reaches you late; notes are shifted later by this much.
    pub offset_ms: f64,
    pub scroll_speed: f64,
    /// Scales both volumes below (Alt + mouse wheel anywhere).
    pub master_volume: f32,
    pub music_volume: f32,
    pub fx_volume: f32,
    /// Present mode during songs.
    pub present: PresentChoice,
    /// Menus (song select and its preview, settings, results) always use vsync.
    pub menu_vsync: bool,
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
    /// The map's name and the offset/UR line during play (Tab hides them).
    pub show_map_info: bool,
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
    /// osu!'s mash penalty (wrong-colour / too-early hits miss).
    pub anti_mash: bool,
    /// Kiai time effects on the lane (tint, pulsing glow).
    pub kiai_effects: bool,
    /// Show what you're doing in your Discord status.
    pub discord_presence: bool,
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
    /// Left rim, left centre, right centre, right rim.
    pub keys: [KeyCode; 4],
    /// Previous / next playfield resolution preset.
    pub preset_keys: [KeyCode; 2],
    /// Show the playfield's resolution in a corner during play.
    pub show_playfield_size: bool,
    /// The player's own playfield sizes, first in the preset list, in their order.
    pub saved_presets: Vec<(u32, u32)>,
    /// Name of the osu! skin in use (its folder name); empty = built-in.
    pub skin: String,
    pub songs: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            offset_ms: 0.0,
            scroll_speed: 1.0,
            master_volume: 1.0,
            music_volume: 0.7,
            fx_volume: 1.0,
            present: PresentChoice::Fifo,
            menu_vsync: true,
            fps_cap: 0,
            calibration_bpm: 120.0,
            playfield_area: Area::FULL,
            bg_dim: 0.8,
            rate: 1.0,
            rate_keep_pitch: true,
            show_offset_hint: true,
            show_map_info: true,
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
            animations: true,
            splash: true,
            check_updates: true,
            resume_delay_ms: 1000.0,
            sort: "artist".into(),
            kiai_effects: true,
            discord_presence: true,
            discord_app_id: String::new(),
            anti_mash: false,
            keys: [KeyCode::KeyD, KeyCode::KeyF, KeyCode::KeyJ, KeyCode::KeyK],
            preset_keys: [KeyCode::F5, KeyCode::F6],
            show_playfield_size: true,
            saved_presets: Vec::new(),
            skin: String::new(),
            songs: Vec::new(),
        }
    }
}

/// Per-user taiko-rs directory: settings (`cache == false`) or disposable data like extracted
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

/// Each map's own offset (on top of the global one), by the .osu's content hash:
/// `map_offsets.tsv` in the config folder, one `hash<TAB>ms` per line.
pub fn load_map_offsets() -> std::collections::HashMap<u64, f64> {
    let Some(text) = user_dir(false).and_then(|d| std::fs::read_to_string(d.join("map_offsets.tsv")).ok()) else { return Default::default() };
    text.lines()
        .filter_map(|l| {
            let (h, ms) = l.split_once('\t')?;
            Some((u64::from_str_radix(h.trim(), 16).ok()?, ms.trim().parse::<f64>().ok()?))
        })
        .collect()
}

pub fn save_map_offsets(offsets: &std::collections::HashMap<u64, f64>) {
    let Some(dir) = user_dir(false) else { return };
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
];

pub fn key_name(k: KeyCode) -> String {
    let s = format!("{k:?}");
    s.strip_prefix("Key").or_else(|| s.strip_prefix("Digit")).map(str::to_owned).unwrap_or(s)
}

pub fn parse_key(s: &str) -> Option<KeyCode> {
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
                "scroll_speed" => cfg.scroll_speed = v.parse::<f64>().unwrap_or(cfg.scroll_speed).clamp(0.3, 4.0),
                "scroll_speed_osu" => osu_scroll_speed = v == "true",
                "master_volume" => cfg.master_volume = v.parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0),
                "music_volume" => cfg.music_volume = v.parse::<f32>().unwrap_or(cfg.music_volume).clamp(0.0, 1.0),
                "fx_volume" => cfg.fx_volume = v.parse::<f32>().unwrap_or(cfg.fx_volume).clamp(0.0, 1.0),
                "present_mode" => cfg.present = PresentChoice::parse(v).unwrap_or(cfg.present),
                "menu_vsync" => cfg.menu_vsync = v != "false",
                "fps_cap" => cfg.fps_cap = v.parse().unwrap_or(cfg.fps_cap),
                "playfield_area" => cfg.playfield_area = Area::parse(v).unwrap_or(Area::FULL),
                "bg_dim" => cfg.bg_dim = v.parse::<f32>().unwrap_or(0.8).clamp(0.0, 1.0),
                "rate" => cfg.rate = v.parse::<f64>().unwrap_or(1.0).clamp(RATE_RANGE.0, RATE_RANGE.1),
                "rate_keep_pitch" => cfg.rate_keep_pitch = v != "false",
                "calibration_bpm" => cfg.calibration_bpm = v.parse::<f64>().unwrap_or(120.0).clamp(60.0, 240.0),
                "audio_buffer" => cfg.audio_buffer = v.parse::<u32>().unwrap_or(cfg.audio_buffer).clamp(32, 4096),
                "show_fps" => cfg.show_fps = v == "true",
                "fullscreen" => cfg.fullscreen = v != "false",
                "animations" => cfg.animations = v != "false",
                "splash" => cfg.splash = v != "false",
                "check_updates" => cfg.check_updates = v != "false",
                "sort" => cfg.sort = v.to_string(),
                "kiai_effects" => cfg.kiai_effects = v != "false",
                "discord_presence" => cfg.discord_presence = v != "false",
                "discord_app_id" => cfg.discord_app_id = v.to_string(),
                "anti_mash" => cfg.anti_mash = v == "true",
                "resume_delay_ms" => cfg.resume_delay_ms = v.parse::<f64>().unwrap_or(1000.0).clamp(0.0, 3000.0),
                "show_offset_hint" => cfg.show_offset_hint = v == "true",
                "show_map_info" => cfg.show_map_info = v != "false",
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
                "show_playfield_size" => cfg.show_playfield_size = v != "false",
                "saved_presets" => cfg.saved_presets = v.split(',').filter_map(crate::layout::parse_size).collect(),
                "skin" => cfg.skin = v.to_string(),
                "songs" => cfg.songs.push(PathBuf::from(v)),
                _ => {}
            }
        }
        // Scroll speed 1.0 used to be ~0.76x of osu!'s (at 16:9). Left at 1.0 it now simply
        // becomes osu!'s; a speed someone picked on purpose keeps looking the same.
        if !osu_scroll_speed && (cfg.scroll_speed - 1.0).abs() > 1e-3 {
            cfg.scroll_speed = (cfg.scroll_speed * OLD_SCROLL_SPEED).clamp(0.3, 4.0);
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
            "# taiko-rs settings\n\
             # keys = left rim, left centre, right centre, right rim\n\
             keys = {}\n\
             # previous / next playfield resolution preset\n\
             preset_keys = {}\n\
             show_playfield_size = {}\n\
             # your own playfield sizes (preset list, in this order)\n\
             saved_presets = {}\n\
             offset_ms = {}\n\
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
             # wrong-colour / too-early hits count as misses (osu!'s mash penalty)\n\
             anti_mash = {}\n\
             show_offset_hint = {}\n\
             offset_hint_ur_scaled = {}\n\
             # map name and UR line during play (Tab toggles)\n\
             show_map_info = {}\n\
             # mods picked on song select (F1), e.g. HD,HR\n\
             mods = {}\n",
            keys.join(", "),
            self.preset_keys.iter().map(|k| key_name(*k)).collect::<Vec<_>>().join(", "),
            self.show_playfield_size,
            self.saved_presets.iter().map(|(w, h)| format!("{w}x{h}")).collect::<Vec<_>>().join(", "),
            self.offset_ms,
            self.scroll_speed,
            self.master_volume,
            self.music_volume,
            self.fx_volume,
            self.present.name(),
            self.menu_vsync,
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
            self.anti_mash,
            self.show_offset_hint,
            self.offset_hint_ur_scaled,
            self.show_map_info,
            self.mods.to_config(),
        );
        out.push_str(&format!("# osu! skin (folder name in a Skins folder), empty = built-in\nskin = {}\n", self.skin));
        out.push_str(&format!(
            "# Discord status (\"Playing taiko-rs\"); app id empty = the built-in one\ndiscord_presence = {}\ndiscord_app_id = {}\n",
            self.discord_presence, self.discord_app_id
        ));
        out.push_str(&format!("# multiplayer\nmp_name = {}\nmp_port = {}\nmp_address = {}\n", self.mp_name, self.mp_port, self.mp_address));
        out.push_str(&format!(
            "# online server (accounts, lobbies) and your saved login there\nonline_server = {}\nonline_user = {}\nonline_token = {}\n",
            self.online_server, self.online_user, self.online_token
        ));
        out.push_str("# extra song folders / .osz files, one per line\n");
        for s in &self.songs {
            out.push_str(&format!("songs = {}\n", s.display()));
        }
        let _ = std::fs::write(path, out);
    }
}
