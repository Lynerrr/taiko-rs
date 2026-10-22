//! Settings persisted as a plain `key = value` file in `$XDG_CONFIG_HOME/taiko-rs/config.txt`.

use std::path::PathBuf;

use winit::keyboard::KeyCode;

use crate::render::PresentChoice;

pub const FPS_CAPS: [u32; 7] = [0, 2000, 1000, 500, 240, 144, 60];
pub const FRAME_DELAYS: [f64; 8] = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0];

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
            audio_buffer: 256,
            show_fps: true,
            keys: [KeyCode::KeyD, KeyCode::KeyF, KeyCode::KeyJ, KeyCode::KeyK],
            songs: Vec::new(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("taiko-rs").join("config.txt"))
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
                "frame_delay_ms" => cfg.frame_delay_ms = v.parse::<f64>().unwrap_or(0.0).clamp(0.0, 30.0),
                "audio_buffer" => cfg.audio_buffer = v.parse::<u32>().unwrap_or(cfg.audio_buffer).clamp(32, 4096),
                "show_fps" => cfg.show_fps = v == "true",
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
             # audio device buffer in frames (lower = less latency, more risk of crackle)\n\
             audio_buffer = {}\n\
             show_fps = {}\n",
            keys.join(", "),
            self.offset_ms,
            self.scroll_speed,
            self.music_volume,
            self.fx_volume,
            self.present.name(),
            self.fps_cap,
            self.frame_delay_ms,
            self.audio_buffer,
            self.show_fps,
        );
        out.push_str("# extra song folders / .osz files, one per line\n");
        for s in &self.songs {
            out.push_str(&format!("songs = {}\n", s.display()));
        }
        let _ = std::fs::write(path, out);
    }
}
