//! Settings persisted as a plain `key = value` file: `$XDG_CONFIG_HOME/taiko-rs/config.txt` on
//! Linux, `%APPDATA%\taiko-rs\config.txt` on Windows.

use std::path::PathBuf;

use winit::keyboard::KeyCode;

use crate::layout::Area;
use crate::render::PresentChoice;

pub const RATE_RANGE: (f64, f64) = (0.5, 2.0);
pub const FPS_CAPS: [u32; 7] = [0, 2000, 1000, 500, 240, 144, 60];
pub const FRAME_DELAYS: [f64; 13] = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 25.0, 33.0];
pub const MAX_FRAME_DELAY: f64 = 33.0;

pub struct Config {
    /// Positive when your audio reaches you late; notes are shifted later by this much.
    pub offset_ms: f64,
    pub scroll_speed: f64,
    pub music_volume: f32,
    pub fx_volume: f32,
    pub present: PresentChoice,
    /// 0 = uncapped. Only applies to mailbox/immediate; vsync paces itself.
    pub fps_cap: u32,
    /// Vsync only: wait this long after the previous frame before drawing the next, so the frame is
    /// sampled closer to the compositor's deadline (lower latency; too high causes stutter).
    pub frame_delay_ms: f64,
    /// Tempo of the offset-calibration click track.
    pub calibration_bpm: f64,
    /// Where gameplay is drawn on screen (fractions; full screen by default).
    pub playfield_area: Area,
    /// HUD (score, accuracy, hit error meter...) inside the playfield area, or fixed on screen.
    pub hud_in_playfield: bool,
    /// Speed modifier (1.5 = DT, 0.75 = HT) and whether it keeps the music's pitch.
    pub rate: f64,
    pub rate_keep_pitch: bool,
    /// Show a live offset suggestion during play.
    pub show_offset_hint: bool,
    /// Multiplayer: display name, port to host on, last address joined.
    pub mp_name: String,
    pub mp_port: u16,
    pub mp_address: String,
    /// Scale that suggestion to current UR (off: plain average of the last 40 hits).
    pub offset_hint_ur_scaled: bool,
    pub audio_buffer: u32,
    pub show_fps: bool,
    /// Left rim, left centre, right centre, right rim.
    pub keys: [KeyCode; 4],
    pub songs: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            offset_ms: 0.0,
            scroll_speed: 1.0,
            music_volume: 0.7,
            fx_volume: 1.0,
            present: PresentChoice::Fifo,
            fps_cap: 0,
            frame_delay_ms: 0.0,
            calibration_bpm: 120.0,
            playfield_area: Area::FULL,
            hud_in_playfield: true,
            rate: 1.0,
            rate_keep_pitch: true,
            show_offset_hint: true,
            mp_name: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok()
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| "player".into()),
            mp_port: 7777,
            mp_address: String::new(),
            offset_hint_ur_scaled: true,
            audio_buffer: 256,
            show_fps: true,
            keys: [KeyCode::KeyD, KeyCode::KeyF, KeyCode::KeyJ, KeyCode::KeyK],
            songs: Vec::new(),
        }
    }
}

/// Per-user taiko-rs directory: settings (`cache == false`) or disposable data like extracted
/// `.osz` files (`cache == true`).
pub fn user_dir(cache: bool) -> Option<PathBuf> {
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
    KeyCode::Numpad8, KeyCode::Numpad9,
];

pub fn key_name(k: KeyCode) -> String {
    let s = format!("{k:?}");
    s.strip_prefix("Key").or_else(|| s.strip_prefix("Digit")).map(str::to_owned).unwrap_or(s)
}

pub fn parse_key(s: &str) -> Option<KeyCode> {
    KEY_NAMES.iter().copied().find(|k| format!("{k:?}").eq_ignore_ascii_case(s) || key_name(*k).eq_ignore_ascii_case(s))
}

impl Config {
    pub fn load() -> Config {
        let mut cfg = Config::default();
        let Some(path) = config_path() else { return cfg };
        let Ok(text) = std::fs::read_to_string(&path) else { return cfg };
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
                "music_volume" => cfg.music_volume = v.parse::<f32>().unwrap_or(cfg.music_volume).clamp(0.0, 1.0),
                "fx_volume" => cfg.fx_volume = v.parse::<f32>().unwrap_or(cfg.fx_volume).clamp(0.0, 1.0),
                "present_mode" => cfg.present = PresentChoice::parse(v).unwrap_or(cfg.present),
                "fps_cap" => cfg.fps_cap = v.parse().unwrap_or(cfg.fps_cap),
                "playfield_area" => cfg.playfield_area = Area::parse(v).unwrap_or(Area::FULL),
                "hud_in_playfield" => cfg.hud_in_playfield = v != "false",
                "rate" => cfg.rate = v.parse::<f64>().unwrap_or(1.0).clamp(RATE_RANGE.0, RATE_RANGE.1),
                "rate_keep_pitch" => cfg.rate_keep_pitch = v != "false",
                "calibration_bpm" => cfg.calibration_bpm = v.parse::<f64>().unwrap_or(120.0).clamp(60.0, 240.0),
                "frame_delay_ms" => cfg.frame_delay_ms = v.parse::<f64>().unwrap_or(0.0).clamp(0.0, MAX_FRAME_DELAY),
                "audio_buffer" => cfg.audio_buffer = v.parse::<u32>().unwrap_or(cfg.audio_buffer).clamp(32, 4096),
                "show_fps" => cfg.show_fps = v == "true",
                "show_offset_hint" => cfg.show_offset_hint = v == "true",
                "mp_name" if !v.is_empty() => cfg.mp_name = v.to_string(),
                "mp_port" => cfg.mp_port = v.parse().unwrap_or(cfg.mp_port),
                "mp_address" => cfg.mp_address = v.to_string(),
                "offset_hint_ur_scaled" => cfg.offset_hint_ur_scaled = v == "true",
                "keys" => {
                    let keys: Vec<KeyCode> = v.split(',').filter_map(|s| parse_key(s.trim())).collect();
                    if let Ok(k) = keys.try_into() {
                        cfg.keys = k;
                    }
                }
                "songs" => cfg.songs.push(PathBuf::from(v)),
                _ => {}
            }
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
             offset_ms = {}\n\
             scroll_speed = {:.2}\n\
             music_volume = {:.2}\n\
             fx_volume = {:.2}\n\
             # mailbox | immediate | vsync\n\
             present_mode = {}\n\
             # mailbox/immediate only; 0 = uncapped\n\
             fps_cap = {}\n\
             # vsync only: ms to wait before drawing each frame (lower latency; too high stutters)\n\
             frame_delay_ms = {}\n\
             calibration_bpm = {}\n\
             # playfield area: x, y, width, height as fractions of the screen\n\
             playfield_area = {}\n\
             hud_in_playfield = {}\n\
             # speed modifier (1.5 = DT) and whether it keeps the music's pitch\n\
             rate = {:.2}\n\
             rate_keep_pitch = {}\n\
             # audio device buffer in frames (lower = less latency, more risk of crackle)\n\
             audio_buffer = {}\n\
             show_fps = {}\n\
             show_offset_hint = {}\n\
             offset_hint_ur_scaled = {}\n",
            keys.join(", "),
            self.offset_ms,
            self.scroll_speed,
            self.music_volume,
            self.fx_volume,
            self.present.name(),
            self.fps_cap,
            self.frame_delay_ms,
            self.calibration_bpm,
            self.playfield_area.to_config(),
            self.hud_in_playfield,
            self.rate,
            self.rate_keep_pitch,
            self.audio_buffer,
            self.show_fps,
            self.show_offset_hint,
            self.offset_hint_ur_scaled,
        );
        out.push_str(&format!("# multiplayer\nmp_name = {}\nmp_port = {}\nmp_address = {}\n", self.mp_name, self.mp_port, self.mp_address));
        out.push_str("# extra song folders / .osz files, one per line\n");
        for s in &self.songs {
            out.push_str(&format!("songs = {}\n", s.display()));
        }
        let _ = std::fs::write(path, out);
    }
}
