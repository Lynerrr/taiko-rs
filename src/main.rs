// Release builds on Windows are a GUI app: no console window behind the game.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod beatmap;
mod calibrate;
mod clock;
mod config;
mod debug;
mod font;
mod game;
mod layout;
mod library;
mod multiplayer;
mod net;
mod render;
mod settings;
mod ui;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use audio::{Engine, Track};
use beatmap::Beatmap;
use config::{Config, FPS_CAPS, FRAME_DELAYS, MAX_FRAME_DELAY, RATE_RANGE};
use game::{Game, PAUSE_ITEMS};
use layout::{Area, Grip};
use winit::window::CursorIcon;
use library::Entry;
use render::{rgba, Canvas, PresentChoice, Renderer};
use settings::{Item, SettingsUi};
use ui::{Results, Select, SelectAction};

type Loaded = Result<(Beatmap, Arc<Track>), String>;

enum Scene {
    Select,
    Loading { rx: Receiver<Loaded>, label: String, entry: usize },
    Play { game: Box<Game>, entry: usize },
    Results { results: Results, entry: usize },
    /// Settings tab; `prev` is restored on exit (song select, or a paused game).
    Settings { prev: Box<Scene> },
    /// Offset calibration run; `back` is where to return (the settings tab).
    Calibrate { game: Box<Game>, back: Box<Scene> },
    CalibrateDone { estimate: calibrate::Estimate, back: Box<Scene> },
    /// Full-screen editor for the playfield area, opened from settings.
    LayoutEdit { back: Box<Scene> },
    /// Multiplayer: host/join form, room lobby, waiting/countdown, match, results.
    MpMenu,
    Lobby,
    MpWait { rx: Option<Receiver<Loaded>>, loaded: Option<(Beatmap, Arc<Track>)>, start: Option<Instant> },
    MpPlay { game: Box<Game>, quit_armed: Option<Instant> },
    MpResults,
}

struct Toast {
    text: String,
    until: Instant,
}

struct FrameStats {
    last: Instant,
    frames: u32,
    window_start: Instant,
    fps: f64,
    worst_ms: f64,
    worst_acc: f64,
}

/// Everything the input and render threads share. The render thread holds the lock only while
/// building a frame (well under a millisecond) and never across a GPU wait, so key presses are
/// handled — and timestamped — immediately no matter how the swapchain paces frames.
struct State {
    cfg: Config,
    library: Vec<Entry>,
    select: Select,
    scene: Scene,
    audio: Option<Engine>,
    toast: Option<Toast>,
    stats: FrameStats,
    status: String,
    modifiers_shift: bool,
    auto: bool,
    shots: Option<debug::ShotPlan>,
    /// Screens still to capture once each when a shot plan is active.
    pending_screen_shots: Vec<&'static str>,
    exit_when_done: bool,
    quit: bool,
    smooth: Option<debug::Smoothness>,
    started: Instant,
    /// Requests from the input thread that only the render thread can apply.
    want_present: Option<PresentChoice>,
    want_size: Option<(u32, u32)>,
    present: PresentChoice,
    refresh_hz: f64,
    settings_ui: SettingsUi,
    fullscreen: bool,
    adapter: String,
    input_script: Option<debug::InputScript>,
    wall_shots: Vec<f64>,
    mp: Option<multiplayer::Session>,
    mp_connecting: Option<multiplayer::Connecting>,
    mp_menu: multiplayer::Menu,
    mp_status: String,
    /// Song select is being used by the host to pick the lobby map.
    picking_lobby_map: bool,
    /// Background library scan: result channel and files-read counter.
    scan: Option<(Receiver<Vec<Entry>>, Arc<AtomicUsize>)>,
    /// A single .osu was given on the command line: play it once the scan finishes.
    play_when_scanned: bool,
    /// Mouse: fractional wheel notches (touchpads), left-button drag of the playfield, window
    /// height for converting pixels to virtual units, and how long the settings preview shows.
    wheel_acc: f32,
    /// Pointer position (physical px), the active playfield-area drag (grip, where it started,
    /// area at that moment) and what the pointer hovers.
    mouse: (f64, f64),
    grip: Option<(Grip, (f64, f64), Area)>,
    hover: Option<Grip>,
    window_w: f32,
    window_h: f32,
    /// Shapes of this frame to clip to the playfield area.
    frame_clip: Option<(std::ops::Range<usize>, Area)>,
    /// Snap guides to draw while dragging the playfield area.
    guides: Vec<layout::Guide>,
    /// Playfield presets (resolutions that fit the window), filled when the window opens.
    resolutions: Vec<(u32, u32)>,
    monitor_modes: Vec<(u32, u32)>,
}

/// Draws gameplay (via `f`) into the playfield area, records the shapes to clip, and shows the
/// area's frame while it's being dragged or an edge is hovered.
fn draw_game_area(
    canvas: &mut Canvas,
    cfg: &Config,
    clip: &mut Option<(std::ops::Range<usize>, Area)>,
    grip: &Option<(Grip, (f64, f64), Area)>,
    hover: &Option<Grip>,
    f: impl FnOnce(&mut Canvas),
) {
    let area = cfg.playfield_area;
    let (start, full) = layout::begin(canvas, area);
    f(canvas);
    let range = layout::end(canvas, area, start, full);
    *clip = Some((range, area));
    if grip.is_some() || matches!(hover, Some(Grip::Resize { .. })) {
        layout::draw_frame(canvas, area, grip.is_some());
    }
}

/// The library entry for a lobby map: same artist/title/difficulty, then the exact file hash.
/// Only those few candidates are hashed, so this stays cheap with a huge library.
fn find_map(library: &[Entry], m: &net::MapInfo) -> Option<usize> {
    library
        .iter()
        .position(|e| e.meta.version == m.version && e.meta.title == m.title && e.meta.artist == m.artist && e.hash() == m.hash)
}

const AUDIO_BUFFERS: [u32; 6] = [64, 128, 256, 512, 1024, 2048];

impl State {
    fn toast(&mut self, text: String) {
        self.toast = Some(Toast { text, until: Instant::now() + Duration::from_millis(1500) });
    }

    fn start_loading(&mut self, entry: usize) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        let label = self.library[entry].label();
        let rx = self.spawn_loader(entry, self.cfg.rate);
        self.scene = Scene::Loading { rx, label, entry };
    }

    /// Parses the map and decodes its audio on a background thread.
    /// `speed`: rate modifier to prepare the track for (time-stretched or resampled once here).
    fn spawn_loader(&self, entry: usize, speed: f64) -> Receiver<Loaded> {
        let path = self.library[entry].path.clone();
        let rate = self.audio.as_ref().map(|a| a.rate);
        let keep_pitch = self.cfg.rate_keep_pitch;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let map = beatmap::load(&path)?;
                let audio_path = path.parent().unwrap_or(&path).join(&map.meta.audio_file);
                let track = match rate {
                    Some(rate) => audio::change_rate(audio::decode(&audio_path, rate)?, rate, speed, keep_pitch),
                    None => Track { samples: Vec::new(), rate: 1.0 },
                };
                Ok((map, Arc::new(track)))
            })();
            let _ = tx.send(result);
        });
        rx
    }

    fn to_select(&mut self) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        self.report_smoothness();
        self.scene = Scene::Select;
    }

    fn report_smoothness(&mut self) {
        if let (Some(sm), Scene::Play { .. }) = (&mut self.smooth, &self.scene) {
            eprintln!("--- smoothness ({}, frame delay {:.0} ms) ---\n{}", self.present.name(), self.cfg.frame_delay_ms, sm.report());
            *sm = debug::Smoothness::default();
        }
    }

    /// Longest frame delay that reliably leaves room to render before the next refresh. Larger
    /// values are allowed but may miss the compositor deadline.
    fn safe_frame_delay(&self) -> f64 {
        (1000.0 / self.refresh_hz - 3.0).max(0.0)
    }

    fn refresh_status(&mut self) {
        let audio_desc = self.audio.as_ref().map(|a| a.description.clone()).unwrap_or_else(|| "no audio".into());
        let config = config::config_path().map(|p| p.display().to_string()).unwrap_or_else(|| "not saved".into());
        self.status = format!("{}  |  {}  |  {:.0} Hz  |  config: {config}", self.adapter, audio_desc, self.refresh_hz);
    }

    fn open_settings(&mut self) {
        let prev = std::mem::replace(&mut self.scene, Scene::Select);
        self.scene = Scene::Settings { prev: Box::new(prev) };
        self.settings_ui.binding = None;
    }

    fn close_settings(&mut self) {
        if let Scene::Settings { prev } = std::mem::replace(&mut self.scene, Scene::Select) {
            self.scene = *prev;
        }
    }

    fn setting_value(&self, item: Item) -> String {
        let c = &self.cfg;
        let on = |b: bool| if b { "on" } else { "off" }.to_string();
        match item {
            Item::ScrollSpeed => format!("{:.2}x", c.scroll_speed),
            Item::Offset => format!("{:+.0} ms", c.offset_ms),
            Item::Key(i) => config::key_name(c.keys[i]),
            Item::ShowConverts => on(self.select.show_converts),
            Item::MusicVolume => format!("{:.0}%", c.music_volume * 100.0),
            Item::FxVolume => format!("{:.0}%", c.fx_volume * 100.0),
            Item::AudioBuffer => {
                let rate = self.audio.as_ref().map(|a| a.rate).unwrap_or(48000);
                let ms = c.audio_buffer as f64 / rate as f64 * 1000.0;
                format!("{} frames ({ms:.1} ms)", c.audio_buffer)
            }
            Item::PresentMode => self.present.name().to_string(),
            Item::FrameDelay if c.frame_delay_ms > self.safe_frame_delay() => {
                format!("{:.0} ms (past safe {:.1} ms)", c.frame_delay_ms, self.safe_frame_delay())
            }
            Item::FrameDelay => format!("{:.0} ms", c.frame_delay_ms),
            Item::FpsCap => {
                if c.fps_cap == 0 {
                    "unlimited".into()
                } else {
                    c.fps_cap.to_string()
                }
            }
            Item::ShowFps => on(c.show_fps),
            Item::Fullscreen => on(self.fullscreen),
            Item::Calibrate => "Enter to start".into(),
            Item::CalibrationBpm => format!("{:.0} BPM", c.calibration_bpm),
            Item::LiveOffset => on(c.show_offset_hint),
            Item::LiveOffsetUr => on(c.offset_hint_ur_scaled),
            Item::PlayfieldArea => c.playfield_area.describe(),
            Item::PlayfieldPreset => self.current_preset().map(|(w, h)| format!("{w}x{h}")).unwrap_or_else(|| "custom".into()),
            Item::HudPosition => if c.hud_in_playfield { "with playfield".into() } else { "screen".into() },
            Item::PlayfieldReset => "Enter to reset".into(),
            Item::Rate => format!("{:.2}x", c.rate),
            Item::RateKeepPitch => on(c.rate_keep_pitch),
        }
    }

    /// Changes a setting one step in `dir` (-1 / +1), applying it immediately.
    fn adjust_setting(&mut self, item: Item, dir: i32, window: Option<&Window>) {
        let d = dir as f64;
        match item {
            Item::ScrollSpeed => self.cfg.scroll_speed = (self.cfg.scroll_speed + 0.05 * d).clamp(0.3, 4.0),
            Item::Offset => self.cfg.offset_ms += d * if self.modifiers_shift { 5.0 } else { 1.0 },
            Item::Key(i) => {
                self.settings_ui.binding = Some(i);
                return;
            }
            Item::ShowConverts => {
                self.select.show_converts = !self.select.show_converts;
                self.select.refilter(&self.library);
                return;
            }
            Item::MusicVolume => self.cfg.music_volume = (self.cfg.music_volume + 0.05 * dir as f32).clamp(0.0, 1.0),
            Item::FxVolume => self.cfg.fx_volume = (self.cfg.fx_volume + 0.05 * dir as f32).clamp(0.0, 1.0),
            Item::AudioBuffer => {
                if matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Play { .. })) {
                    self.toast("change the audio buffer from song select".into());
                    return;
                }
                let i = AUDIO_BUFFERS.iter().position(|&b| b >= self.cfg.audio_buffer).unwrap_or(2);
                self.cfg.audio_buffer = AUDIO_BUFFERS[(i as i32 + dir).clamp(0, AUDIO_BUFFERS.len() as i32 - 1) as usize];
                // Reopen the device with the new buffer size; the old stream must close first.
                self.audio = None;
                self.audio = match Engine::new(self.cfg.audio_buffer) {
                    Ok(a) => Some(a),
                    Err(e) => {
                        self.toast(format!("audio failed: {e}"));
                        None
                    }
                };
                self.refresh_status();
            }
            Item::PresentMode => {
                self.want_present = Some(if dir > 0 { self.present.next() } else { self.present.next().next() });
            }
            Item::FrameDelay => self.cfg.frame_delay_ms = (self.cfg.frame_delay_ms + d).clamp(0.0, MAX_FRAME_DELAY).round(),
            Item::FpsCap => {
                let i = FPS_CAPS.iter().position(|&c| c == self.cfg.fps_cap).unwrap_or(0) as i32;
                self.cfg.fps_cap = FPS_CAPS[(i + dir).rem_euclid(FPS_CAPS.len() as i32) as usize];
            }
            Item::ShowFps => self.cfg.show_fps = !self.cfg.show_fps,
            Item::Fullscreen => self.toggle_fullscreen(window),
            // Started with Enter in `settings_key`; arrows do nothing.
            Item::Calibrate => return,
            Item::LiveOffset => self.cfg.show_offset_hint = !self.cfg.show_offset_hint,
            Item::LiveOffsetUr => self.cfg.offset_hint_ur_scaled = !self.cfg.offset_hint_ur_scaled,
            // Both act on Enter in `settings_key`.
            Item::PlayfieldArea | Item::PlayfieldReset => return,
            Item::PlayfieldPreset => {
                self.cycle_preset(dir);
                return;
            }
            Item::HudPosition => self.cfg.hud_in_playfield = !self.cfg.hud_in_playfield,
            Item::Rate => {
                let step = 0.05 * if self.modifiers_shift { 5.0 } else { 1.0 };
                self.cfg.rate = (((self.cfg.rate + step * d) * 100.0).round() / 100.0).clamp(RATE_RANGE.0, RATE_RANGE.1);
            }
            Item::RateKeepPitch => self.cfg.rate_keep_pitch = !self.cfg.rate_keep_pitch,
            Item::CalibrationBpm => {
                let (lo, hi) = calibrate::BPM_RANGE;
                self.cfg.calibration_bpm = (self.cfg.calibration_bpm + 5.0 * d).clamp(lo, hi);
            }
        }
        if let Some(a) = &self.audio {
            a.set_volume(self.cfg.music_volume, self.cfg.fx_volume);
            if item == Item::FxVolume {
                a.sfx(audio::Sfx::Don);
            }
        }
        self.cfg.save();
    }

    fn toggle_fullscreen(&mut self, window: Option<&Window>) {
        if let Some(w) = window {
            self.fullscreen = w.fullscreen().is_none();
            w.set_fullscreen(self.fullscreen.then_some(Fullscreen::Borderless(None)));
        }
    }

    fn settings_key(&mut self, code: KeyCode, window: Option<&Window>) {
        if let Some(slot) = self.settings_ui.binding.take() {
            if code != KeyCode::Escape {
                // Swap if the key was already bound to another drum.
                if let Some(other) = self.cfg.keys.iter().position(|&k| k == code) {
                    self.cfg.keys[other] = self.cfg.keys[slot];
                }
                self.cfg.keys[slot] = code;
                self.cfg.save();
            }
            return;
        }
        let item = self.settings_ui.item();
        match code {
            KeyCode::ArrowUp => self.settings_ui.move_cursor(-1),
            KeyCode::ArrowDown => self.settings_ui.move_cursor(1),
            KeyCode::ArrowLeft => self.adjust_setting(item, -1, window),
            KeyCode::ArrowRight => self.adjust_setting(item, 1, window),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Calibrate => self.start_calibration(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::PlayfieldArea => self.open_layout_editor(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::PlayfieldReset => {
                self.cfg.playfield_area = Area::FULL;
                self.cfg.save();
                self.toast("playfield fills the screen".into());
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.adjust_setting(item, 1, window),
            KeyCode::Tab if matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Select)) => {
                self.open_multiplayer();
            }
            KeyCode::Escape | KeyCode::Tab | KeyCode::F1 => self.close_settings(),
            _ => {}
        }
    }

    fn start_calibration(&mut self) {
        if matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Play { .. })) {
            self.toast("quit the song first to calibrate".into());
            return;
        }
        let back = match std::mem::replace(&mut self.scene, Scene::Select) {
            // Retrying from the result screen returns to wherever calibration was started from.
            Scene::CalibrateDone { back, .. } => *back,
            other => other,
        };
        let beat_ms = 60000.0 / self.cfg.calibration_bpm;
        let beats = calibrate::total_beats(beat_ms);
        let map = beatmap::calibration_map(beat_ms, calibrate::COUNT_IN_BEATS, beats);
        let rate = self.audio.as_ref().map(|a| a.rate).unwrap_or(48000);
        let track = Arc::new(audio::metronome(rate, beat_ms, beats, 2000.0));
        let mut game = Game::new(map, track, self.audio.as_ref());
        // Your own hitsounds would let you sync to yourself instead of to the click.
        game.mute_hits = true;
        self.scene = Scene::Calibrate { game: Box::new(game), back: Box::new(back) };
    }

    fn finish_calibration(&mut self) {
        let Scene::Calibrate { game, back } = std::mem::replace(&mut self.scene, Scene::Select) else { return };
        if let Some(a) = &self.audio {
            a.stop();
        }
        match calibrate::estimate(&game.stats.errors, self.cfg.offset_ms) {
            Some(estimate) if estimate.hits >= 4 => self.scene = Scene::CalibrateDone { estimate, back },
            _ => {
                self.scene = *back;
                self.toast("calibration needs more hits".into());
            }
        }
    }

    fn leave_calibration(&mut self, apply: bool) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        match std::mem::replace(&mut self.scene, Scene::Select) {
            Scene::Calibrate { back, .. } => self.scene = *back,
            Scene::CalibrateDone { estimate, back } => {
                if apply {
                    self.cfg.offset_ms = estimate.suggested_offset;
                    self.cfg.save();
                    self.toast(format!("offset set to {:+.0} ms", estimate.suggested_offset));
                }
                self.scene = *back;
            }
            other => self.scene = other,
        }
    }

    fn calibration_key(&mut self, code: KeyCode, repeat: bool, at: Instant, window: Option<&Window>) {
        if let Scene::Calibrate { game, .. } = &mut self.scene {
            if let Some(k) = self.cfg.keys.iter().position(|&k| k == code) {
                if !repeat {
                    game.press(k, at, self.audio.as_ref(), self.cfg.offset_ms);
                }
                return;
            }
            if repeat {
                return;
            }
            match code {
                KeyCode::KeyH => game.hide_notes = !game.hide_notes,
                KeyCode::Enter | KeyCode::NumpadEnter => self.finish_calibration(),
                KeyCode::Escape => self.leave_calibration(false),
                KeyCode::F11 => self.toggle_fullscreen(window),
                KeyCode::F12 => self.quit = true,
                _ => {}
            }
            return;
        }
        if repeat {
            return;
        }
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter => self.leave_calibration(true),
            KeyCode::Escape => self.leave_calibration(false),
            KeyCode::KeyR => self.start_calibration(),
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let step = if code == KeyCode::ArrowRight { 5.0 } else { -5.0 };
                let (lo, hi) = calibrate::BPM_RANGE;
                self.cfg.calibration_bpm = (self.cfg.calibration_bpm + step).clamp(lo, hi);
                self.cfg.save();
            }
            KeyCode::F11 => self.toggle_fullscreen(window),
            KeyCode::F12 => self.quit = true,
            _ => {}
        }
    }

    /// Applies the live offset suggestion from the song being played (O key).
    fn apply_live_offset(&mut self) {
        let (Scene::Play { game, .. } | Scene::MpPlay { game, .. }) = &self.scene else { return };
        match game.offset_suggestion(self.cfg.offset_ms) {
            Some(h) if !h.significant => {
                let text = format!("offset already within +-{:.1} ms of your hits", h.estimate.margin_ms);
                self.toast(text);
            }
            Some(h) => {
                let was = self.cfg.offset_ms;
                self.cfg.offset_ms = h.estimate.suggested_offset;
                self.cfg.save();
                self.toast(format!("offset {:+.0} ms (was {was:+.0})", h.estimate.suggested_offset));
            }
            None => self.toast("need at least 10 hits for a suggestion".into()),
        }
    }

    fn pause_key(&mut self, code: KeyCode) {
        if code == KeyCode::KeyO {
            self.apply_live_offset();
            return;
        }
        let Scene::Play { game, entry } = &mut self.scene else { return };
        let cursor = game.paused.unwrap_or(0);
        match code {
            KeyCode::ArrowUp => game.paused = Some(cursor.saturating_sub(1)),
            KeyCode::ArrowDown => game.paused = Some((cursor + 1).min(PAUSE_ITEMS.len() - 1)),
            KeyCode::Escape => game.resume(self.audio.as_ref()),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => match cursor {
                0 => game.resume(self.audio.as_ref()),
                1 => {
                    let e = *entry;
                    self.start_loading(e);
                }
                2 => self.open_settings(),
                _ => self.to_select(),
            },
            _ => {}
        }
    }

    fn handle_global_key(&mut self, key: KeyCode, window: Option<&Window>) -> bool {
        let step = if self.modifiers_shift { 5.0 } else { 1.0 };
        match key {
            KeyCode::Minus | KeyCode::Equal if matches!(self.scene, Scene::Select) && !self.select.filter.is_empty() => return false,
            KeyCode::Minus | KeyCode::NumpadSubtract => self.cfg.offset_ms -= step,
            KeyCode::Equal | KeyCode::NumpadAdd => self.cfg.offset_ms += step,
            KeyCode::F3 => self.cfg.scroll_speed = (self.cfg.scroll_speed - 0.05).max(0.3),
            KeyCode::F4 => self.cfg.scroll_speed = (self.cfg.scroll_speed + 0.05).min(4.0),
            KeyCode::F7 => {
                self.want_present = Some(self.present.next());
                return true;
            }
            KeyCode::F8 => {
                let i = FPS_CAPS.iter().position(|&c| c == self.cfg.fps_cap).unwrap_or(0);
                self.cfg.fps_cap = FPS_CAPS[(i + 1) % FPS_CAPS.len()];
                let mut text = if self.cfg.fps_cap == 0 { "fps cap: unlimited".to_string() } else { format!("fps cap: {}", self.cfg.fps_cap) };
                if self.present == PresentChoice::Fifo {
                    text.push_str(" (mailbox/immediate only)");
                }
                self.toast(text);
                self.cfg.save();
                return true;
            }
            KeyCode::F9 => {
                let i = FRAME_DELAYS.iter().position(|&d| d == self.cfg.frame_delay_ms).unwrap_or(0);
                let next = FRAME_DELAYS[(i + 1) % FRAME_DELAYS.len()];
                self.cfg.frame_delay_ms = next;
                let mut text = format!("frame delay: {next:.0} ms");
                if next > self.safe_frame_delay() {
                    text.push_str(&format!(" (past {:.1} ms may miss refreshes)", self.safe_frame_delay()));
                }
                if self.present != PresentChoice::Fifo {
                    text.push_str(" (vsync only)");
                }
                self.toast(text);
                self.cfg.save();
                return true;
            }
            KeyCode::F10 => {
                self.cfg.show_fps = !self.cfg.show_fps;
                self.cfg.save();
                return true;
            }
            KeyCode::F11 => {
                self.toggle_fullscreen(window);
                return true;
            }
            KeyCode::F12 => {
                self.quit = true;
                return true;
            }
            _ => return false,
        }
        match key {
            KeyCode::F3 | KeyCode::F4 => self.toast(format!("scroll speed {:.2}x", self.cfg.scroll_speed)),
            _ => self.toast(format!("offset {:+.0} ms", self.cfg.offset_ms)),
        }
        self.cfg.save();
        true
    }

    /// Scenes where the playfield area can be moved and resized with the mouse.
    fn area_editable(&self) -> bool {
        matches!(self.scene, Scene::Play { .. } | Scene::MpPlay { .. } | Scene::Calibrate { .. } | Scene::LayoutEdit { .. })
    }

    fn open_layout_editor(&mut self) {
        let back = std::mem::replace(&mut self.scene, Scene::Select);
        self.scene = Scene::LayoutEdit { back: Box::new(back) };
    }

    fn layout_key(&mut self, code: KeyCode) {
        let a = self.cfg.playfield_area;
        let step = 0.01;
        // Arrows move; Shift+arrows resize from the bottom-right, like a window.
        let resize = |dx: f32, dy: f32| layout::drag(a, Grip::Resize { left: false, right: true, top: false, bottom: true }, dx, dy);
        let moved = |dx: f32, dy: f32| layout::drag(a, Grip::Move, dx, dy);
        let shift = self.modifiers_shift;
        let next = match code {
            KeyCode::ArrowLeft if shift => resize(-step, 0.0),
            KeyCode::ArrowRight if shift => resize(step, 0.0),
            KeyCode::ArrowUp if shift => resize(0.0, -step),
            KeyCode::ArrowDown if shift => resize(0.0, step),
            KeyCode::ArrowLeft => moved(-step, 0.0),
            KeyCode::ArrowRight => moved(step, 0.0),
            KeyCode::ArrowUp => moved(0.0, -step),
            KeyCode::ArrowDown => moved(0.0, step),
            KeyCode::KeyR => Area::FULL,
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                self.cycle_preset(if code == KeyCode::BracketRight { 1 } else { -1 });
                return;
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape => {
                self.cfg.save();
                if let Scene::LayoutEdit { back } = std::mem::replace(&mut self.scene, Scene::Select) {
                    self.scene = *back;
                }
                return;
            }
            _ => return,
        };
        self.cfg.playfield_area = next;
        self.cfg.save();
    }

    fn refresh_presets(&mut self) {
        self.resolutions = layout::presets(&self.monitor_modes, self.window_w as u32, self.window_h as u32);
    }

    /// The preset the current area matches (within a pixel), if any.
    fn current_preset(&self) -> Option<(u32, u32)> {
        let a = self.cfg.playfield_area;
        self.resolutions.iter().copied().find(|&(w, h)| {
            let p = layout::preset_area(w, h, self.window_w, self.window_h);
            (p.w - a.w).abs() * self.window_w < 1.5 && (p.h - a.h).abs() * self.window_h < 1.5
        })
    }

    /// Steps through resolution presets, applying each as a centred area.
    fn cycle_preset(&mut self, dir: i32) {
        if self.resolutions.is_empty() {
            self.toast("no smaller resolutions fit this window".into());
            return;
        }
        let n = self.resolutions.len() as i32;
        let i = match self.current_preset().and_then(|p| self.resolutions.iter().position(|&r| r == p)) {
            Some(i) => (i as i32 + dir).rem_euclid(n),
            // From a custom area, start at the largest (right) or smallest (left).
            None => if dir > 0 { 0 } else { n - 1 },
        } as usize;
        let (w, h) = self.resolutions[i];
        self.cfg.playfield_area = layout::preset_area(w, h, self.window_w, self.window_h);
        self.cfg.save();
        self.toast(format!("playfield {w}x{h}"));
    }

    /// Mouse wheel: scales the playfield area in game / in the editor, scrolls lists elsewhere.
    fn wheel(&mut self, notches: i32) {
        if notches == 0 {
            return;
        }
        if self.area_editable() {
            self.cfg.playfield_area = self.cfg.playfield_area.scaled(1.05f32.powi(notches));
            self.cfg.save();
            let text = format!("playfield: {}", self.cfg.playfield_area.describe());
            self.toast(text);
            return;
        }
        match &self.scene {
            Scene::Select => self.select.move_by(-notches),
            Scene::Settings { .. } => self.settings_ui.move_cursor(-notches),
            _ => {}
        }
    }

    /// Pointer moved: drags the area if a drag is active, otherwise updates the hover state.
    /// Returns the cursor to show.
    fn mouse_moved(&mut self, x: f64, y: f64) -> CursorIcon {
        self.mouse = (x, y);
        let (sw, sh) = (self.window_w.max(1.0) as f64, self.window_h.max(1.0) as f64);
        if let Some((grip, (px, py), from)) = self.grip {
            let raw = layout::drag(from, grip, ((x - px) / sw) as f32, ((y - py) / sh) as f32);
            // Snap within 12 px of the screen's edges / centre lines; Shift places freely.
            let (area, guides) = if self.modifiers_shift {
                (raw, Vec::new())
            } else {
                layout::snap(raw, grip, (12.0 / sw) as f32, (12.0 / sh) as f32)
            };
            self.cfg.playfield_area = area;
            self.guides = guides;
            return grip.cursor();
        }
        if !self.area_editable() {
            self.hover = None;
            return CursorIcon::Default;
        }
        self.hover = layout::hit(self.cfg.playfield_area, x, y, sw, sh);
        self.hover.map(|g| g.cursor()).unwrap_or(CursorIcon::Default)
    }

    fn mouse_button(&mut self, pressed: bool) {
        if pressed {
            if self.area_editable() {
                let (sw, sh) = (self.window_w as f64, self.window_h as f64);
                if let Some(g) = layout::hit(self.cfg.playfield_area, self.mouse.0, self.mouse.1, sw, sh) {
                    self.grip = Some((g, self.mouse, self.cfg.playfield_area));
                }
            }
        } else if self.grip.take().is_some() {
            self.guides.clear();
            self.cfg.save();
        }
    }

    /// Scripted drag for headless tests: press at the area centre (or bottom-right corner).
    fn simulate_drag(&mut self, corner: bool, dx: f64, dy: f64, hold: bool) {
        let a = self.cfg.playfield_area;
        let (sw, sh) = (self.window_w as f64, self.window_h as f64);
        let (fx, fy) = if corner { (a.x + a.w, a.y + a.h) } else { (a.x + a.w / 2.0, a.y + a.h / 2.0) };
        let start = (fx as f64 * sw - if corner { 2.0 } else { 0.0 }, fy as f64 * sh - if corner { 2.0 } else { 0.0 });
        self.mouse_moved(start.0, start.1);
        self.mouse_button(true);
        self.mouse_moved(start.0 + dx, start.1 + dy);
        if !hold {
            self.mouse_button(false);
        }
    }

    fn open_multiplayer(&mut self) {
        self.scene = if self.mp.is_some() { Scene::Lobby } else { Scene::MpMenu };
    }

    fn pick_lobby_map(&mut self, entry: usize) {
        self.picking_lobby_map = false;
        let e = &self.library[entry];
        let info = net::MapInfo {
            hash: e.hash(),
            artist: e.meta.artist.clone(),
            title: e.meta.title.clone(),
            version: e.meta.version.clone(),
            rate: self.cfg.rate,
        };
        if let Some(s) = &self.mp {
            s.client.send(net::ClientMsg::SelectMap(info));
        }
        self.scene = Scene::Lobby;
    }

    fn leave_room(&mut self, why: &str) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        self.mp = None;
        self.picking_lobby_map = false;
        self.mp_status = why.to_string();
        self.scene = Scene::MpMenu;
    }

    fn multiplayer_key(&mut self, code: KeyCode, repeat: bool, text: Option<&str>, at: Instant, window: Option<&Window>) {
        // Drum keys during a match go straight to the game.
        if let Scene::MpPlay { game, .. } = &mut self.scene {
            if let Some(k) = self.cfg.keys.iter().position(|&k| k == code) {
                if !repeat {
                    game.press(k, at, self.audio.as_ref(), self.cfg.offset_ms);
                }
                return;
            }
        }
        if matches!(self.scene, Scene::MpMenu) {
            // The form takes typed text, so only function keys act as shortcuts here.
            let function_key = matches!(
                code,
                KeyCode::F1 | KeyCode::F2 | KeyCode::F3 | KeyCode::F4 | KeyCode::F5 | KeyCode::F6
                    | KeyCode::F7 | KeyCode::F8 | KeyCode::F9 | KeyCode::F10 | KeyCode::F11 | KeyCode::F12
            );
            if !(function_key && !repeat && self.handle_global_key(code, window)) {
                self.mp_menu_key(code, text);
            }
            return;
        }
        if repeat || self.handle_global_key(code, window) {
            return;
        }
        match &mut self.scene {
            Scene::Lobby => {
                let Some(s) = &mut self.mp else {
                    self.scene = Scene::MpMenu;
                    return;
                };
                match code {
                    KeyCode::Space | KeyCode::KeyR => {
                        if s.me().is_some_and(|p| p.has_map) {
                            s.ready = !s.me().is_some_and(|p| p.ready);
                            s.client.send(net::ClientMsg::Ready(s.ready));
                        } else {
                            self.toast("you don't have this map".into());
                        }
                    }
                    KeyCode::BracketLeft | KeyCode::BracketRight if s.is_host() && !s.in_match => {
                        let step = if code == KeyCode::BracketRight { 0.05 } else { -0.05 } * if self.modifiers_shift { 5.0 } else { 1.0 };
                        self.cfg.rate = ((self.cfg.rate + step) * 100.0).round() / 100.0;
                        self.cfg.rate = self.cfg.rate.clamp(RATE_RANGE.0, RATE_RANGE.1);
                        self.cfg.save();
                        if let Some(mut m) = s.map.clone() {
                            m.rate = self.cfg.rate;
                            s.client.send(net::ClientMsg::SelectMap(m));
                        }
                        let text = format!("match rate {:.2}x", self.cfg.rate);
                        self.toast(text);
                    }
                    KeyCode::KeyM if s.is_host() && !s.in_match => {
                        self.picking_lobby_map = true;
                        self.scene = Scene::Select;
                    }
                    KeyCode::Enter | KeyCode::NumpadEnter if s.is_host() => {
                        if s.players.iter().any(|p| p.ready && p.has_map) {
                            s.client.send(net::ClientMsg::StartMatch);
                        } else {
                            self.toast("nobody is ready yet".into());
                        }
                    }
                    KeyCode::Tab => self.scene = Scene::Select,
                    KeyCode::Escape => {
                        let why = if s.is_host() { "room closed" } else { "left the room" };
                        self.leave_room(why);
                    }
                    _ => {}
                }
            }
            Scene::MpWait { .. } => {
                if code == KeyCode::Escape {
                    if let Some(s) = &self.mp {
                        s.client.send(net::ClientMsg::Loaded(false));
                        s.client.send(net::ClientMsg::Finished(net::Final { quit: true, ..Default::default() }));
                    }
                    self.scene = Scene::Lobby;
                }
            }
            Scene::MpPlay { game, quit_armed } => match code {
                KeyCode::KeyO => self.apply_live_offset(),
                KeyCode::Space => game.skip(self.audio.as_ref(), self.cfg.offset_ms),
                KeyCode::Escape => {
                    // No pausing in a match; a second Esc forfeits.
                    if quit_armed.is_some_and(|t| t.elapsed() < Duration::from_millis(1500)) {
                        if let Some(s) = &self.mp {
                            s.client.send(net::ClientMsg::Finished(multiplayer::final_from(&game.stats, true)));
                        }
                        if let Some(a) = &self.audio {
                            a.stop();
                        }
                        self.scene = Scene::MpResults;
                    } else {
                        *quit_armed = Some(Instant::now());
                        self.toast("press Esc again to forfeit".into());
                    }
                }
                _ => {}
            },
            Scene::MpResults => {
                // Results stay on the session so the lobby can show the last winner.
                if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape) {
                    self.scene = Scene::Lobby;
                }
            }
            _ => {}
        }
    }

    fn mp_menu_key(&mut self, code: KeyCode, text: Option<&str>) {
        use multiplayer::{MENU_ROWS, ROW_ADDRESS, ROW_HOST, ROW_JOIN, ROW_NAME, ROW_PORT};
        let row = self.mp_menu.cursor;
        match code {
            KeyCode::ArrowUp => self.mp_menu.cursor = row.saturating_sub(1),
            KeyCode::ArrowDown => self.mp_menu.cursor = (row + 1).min(MENU_ROWS - 1),
            KeyCode::Escape | KeyCode::Tab => self.scene = Scene::Select,
            KeyCode::Backspace => {
                match row {
                    ROW_NAME => {
                        self.cfg.mp_name.pop();
                    }
                    ROW_PORT => self.cfg.mp_port /= 10,
                    ROW_ADDRESS => {
                        self.cfg.mp_address.pop();
                    }
                    _ => {}
                }
                self.cfg.save();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if self.mp_connecting.is_some() {
                    return;
                }
                let name = self.cfg.mp_name.clone();
                match row {
                    ROW_HOST => {
                        self.mp_status = format!("opening room on port {}...", self.cfg.mp_port);
                        self.mp_connecting = Some(multiplayer::start_host(self.cfg.mp_port, name));
                    }
                    ROW_JOIN if self.cfg.mp_address.trim().is_empty() => self.mp_status = "type the host's address first".into(),
                    ROW_JOIN => {
                        self.mp_status = format!("connecting to {}...", self.cfg.mp_address);
                        self.mp_connecting = Some(multiplayer::start_join(self.cfg.mp_address.clone(), name));
                    }
                    _ => self.mp_menu.cursor = (row + 1).min(MENU_ROWS - 1),
                }
                self.cfg.save();
            }
            _ => {
                let Some(text) = text else { return };
                for ch in text.chars().filter(|c| c.is_ascii_graphic() || *c == ' ') {
                    match row {
                        ROW_NAME if self.cfg.mp_name.len() < 20 => self.cfg.mp_name.push(ch),
                        ROW_PORT => {
                            if let Some(d) = ch.to_digit(10) {
                                let p = self.cfg.mp_port as u32 * 10 + d;
                                if p <= u16::MAX as u32 {
                                    self.cfg.mp_port = p as u16;
                                }
                            }
                        }
                        ROW_ADDRESS if self.cfg.mp_address.len() < 64 && ch != ' ' => self.cfg.mp_address.push(ch),
                        _ => {}
                    }
                }
                self.cfg.save();
            }
        }
    }

    /// Installs the library once the background scan finishes.
    fn poll_scan(&mut self) {
        let Some((rx, found)) = &self.scan else { return };
        self.select.scanning = Some(found.load(Ordering::Relaxed));
        let library = match rx.try_recv() {
            Ok(library) => library,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        self.scan = None;
        eprintln!("found {} playable difficulties", library.len());
        self.library = library;
        // Keep whatever was typed into the search box while scanning.
        let filter = std::mem::take(&mut self.select.filter);
        self.select = Select::new(&self.library);
        self.select.filter = filter;
        self.select.refilter(&self.library);
        if self.play_when_scanned && !self.library.is_empty() {
            self.play_when_scanned = false;
            self.start_loading(0);
        }
        // A lobby map may have been announced before our library was ready.
        if let Some(s) = &mut self.mp {
            if let Some(m) = &s.map {
                s.have_map = find_map(&self.library, m).is_some();
                s.client.send(net::ClientMsg::HasMap(s.have_map));
            }
        }
    }

    /// Finishes pending connections and applies messages from the room.
    fn poll_multiplayer(&mut self) {
        if let Some(rx) = &self.mp_connecting {
            match rx.try_recv() {
                Ok(Ok(session)) => {
                    self.mp = Some(session);
                    self.mp_connecting = None;
                    self.mp_status.clear();
                    if matches!(self.scene, Scene::MpMenu) {
                        self.scene = Scene::Lobby;
                    }
                }
                Ok(Err(e)) => {
                    self.mp_status = e;
                    self.mp_connecting = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.mp_connecting = None,
            }
        }
        let Some(s) = &mut self.mp else { return };
        for msg in s.client.poll() {
            self.handle_server_msg(msg);
        }
    }

    fn handle_server_msg(&mut self, msg: net::ServerMsg) {
        use net::ServerMsg;
        match msg {
            ServerMsg::Room { players, map, in_match } => {
                let library = &self.library;
                let Some(s) = &mut self.mp else { return };
                s.players = players;
                s.in_match = in_match;
                let hash = map.as_ref().map(|m| m.hash);
                if hash != s.reported_map {
                    s.reported_map = hash;
                    s.have_map = map.as_ref().is_some_and(|m| find_map(library, m).is_some());
                    s.client.send(net::ClientMsg::HasMap(s.have_map));
                    s.ready = false;
                }
                s.map = map;
                // Dropped from the match (e.g. load timeout) while waiting for it to start.
                let dropped = s.me().is_some_and(|p| p.status == net::Status::Lobby);
                if dropped && matches!(self.scene, Scene::MpWait { start: None, .. }) {
                    self.scene = Scene::Lobby;
                    self.toast("the match started without you (map didn't load in time)".into());
                }
            }
            ServerMsg::Prepare(map) => {
                let Some(s) = &self.mp else { return };
                // Not in a position to play (single-player song, calibration): sit this one out.
                if matches!(self.scene, Scene::Play { .. } | Scene::Calibrate { .. } | Scene::Loading { .. }) {
                    s.client.send(net::ClientMsg::Loaded(false));
                    return;
                }
                match find_map(&self.library, &map) {
                    Some(i) => {
                        if let Some(a) = &self.audio {
                            a.stop();
                        }
                        self.picking_lobby_map = false;
                        let rx = self.spawn_loader(i, map.rate);
                        self.scene = Scene::MpWait { rx: Some(rx), loaded: None, start: None };
                    }
                    None => s.client.send(net::ClientMsg::Loaded(false)),
                }
            }
            ServerMsg::Start { at } => {
                let Some(s) = &self.mp else { return };
                let when = s.client.local_instant(at);
                if let Scene::MpWait { start, .. } = &mut self.scene {
                    *start = Some(when);
                }
            }
            ServerMsg::Results(r) => {
                if let Some(s) = &mut self.mp {
                    s.results = Some(r);
                }
            }
            ServerMsg::Rejected(why) | ServerMsg::Disconnected(why) => {
                let in_mp = matches!(self.scene, Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults);
                self.toast(format!("multiplayer: {why}"));
                self.mp = None;
                self.picking_lobby_map = false;
                if in_mp {
                    if let Some(a) = &self.audio {
                        a.stop();
                    }
                    self.mp_status = why;
                    self.scene = Scene::MpMenu;
                }
            }
            ServerMsg::Welcome { .. } | ServerMsg::Pong { .. } => {}
        }
    }

    fn key(&mut self, code: KeyCode, repeat: bool, text: Option<&str>, at: Instant, window: Option<&Window>) {
        // Key capture must see every key, before any shortcut can claim it.
        if self.settings_ui.binding.is_some() && matches!(self.scene, Scene::Settings { .. }) {
            if !repeat {
                self.settings_key(code, window);
            }
            return;
        }
        if matches!(self.scene, Scene::Calibrate { .. } | Scene::CalibrateDone { .. }) {
            self.calibration_key(code, repeat, at, window);
            return;
        }
        if matches!(self.scene, Scene::LayoutEdit { .. }) {
            self.layout_key(code);
            return;
        }
        if matches!(self.scene, Scene::MpMenu | Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults) {
            self.multiplayer_key(code, repeat, text, at, window);
            return;
        }
        if self.mp.is_some()
            && !self.picking_lobby_map
            && matches!(self.scene, Scene::Select)
            && code == KeyCode::Escape
            && self.select.filter.is_empty()
        {
            self.scene = Scene::Lobby;
            return;
        }
        if self.picking_lobby_map && matches!(self.scene, Scene::Select) && !repeat {
            match code {
                KeyCode::Escape if self.select.filter.is_empty() => {
                    self.picking_lobby_map = false;
                    self.scene = Scene::Lobby;
                    return;
                }
                KeyCode::Tab | KeyCode::F1 => {
                    self.picking_lobby_map = false;
                    self.scene = Scene::Lobby;
                    return;
                }
                _ => {}
            }
        }
        if let Scene::Play { game, .. } = &mut self.scene {
            if game.paused.is_some() {
                if !repeat && !self.cfg.keys.contains(&code) && !self.handle_global_key(code, window) {
                    self.pause_key(code);
                }
                return;
            }
            if let Some(k) = self.cfg.keys.iter().position(|&k| k == code) {
                if !repeat {
                    game.press(k, at, self.audio.as_ref(), self.cfg.offset_ms);
                }
                return;
            }
        }
        if repeat && !matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::Backspace | KeyCode::PageUp | KeyCode::PageDown) {
            return;
        }
        if self.handle_global_key(code, window) {
            return;
        }
        match &mut self.scene {
            Scene::Select if matches!(code, KeyCode::BracketLeft | KeyCode::BracketRight) => {
                let step = if code == KeyCode::BracketRight { 0.05 } else { -0.05 } * if self.modifiers_shift { 5.0 } else { 1.0 };
                self.cfg.rate = (((self.cfg.rate + step) * 100.0).round() / 100.0).clamp(RATE_RANGE.0, RATE_RANGE.1);
                self.cfg.save();
                let pitch = if self.cfg.rate_keep_pitch { "keeps pitch" } else { "pitch shifts" };
                self.toast(format!("speed {:.2}x ({pitch})", self.cfg.rate));
            }
            Scene::Select => match self.select.key(code, &self.library) {
                SelectAction::Play(i) if self.picking_lobby_map => self.pick_lobby_map(i),
                SelectAction::Play(i) => self.start_loading(i),
                SelectAction::Settings => self.open_settings(),
                SelectAction::Quit => self.quit = true,
                SelectAction::None => {
                    if let Some(text) = text {
                        self.select.type_text(text, &self.library);
                    }
                }
            },
            Scene::Loading { .. } => {
                if code == KeyCode::Escape {
                    self.to_select();
                }
            }
            Scene::Settings { .. } => self.settings_key(code, window),
            Scene::Calibrate { .. } | Scene::CalibrateDone { .. } | Scene::LayoutEdit { .. } => {}
            Scene::MpMenu | Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults => {}
            Scene::Play { game, entry } => match code {
                KeyCode::KeyO => self.apply_live_offset(),
                KeyCode::Escape => game.pause(self.audio.as_ref()),
                KeyCode::Space => game.skip(self.audio.as_ref(), self.cfg.offset_ms),
                KeyCode::Backquote => {
                    let e = *entry;
                    self.start_loading(e);
                }
                _ => {}
            },
            Scene::Results { entry, .. } => match code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let e = *entry;
                    self.start_loading(e);
                }
                KeyCode::Escape => {
                    let path = self.library[*entry].path.clone();
                    self.to_select();
                    self.select.select_path(&self.library, &path);
                }
                _ => {}
            },
        }
    }

    /// Advances the game to `now` and fills `canvas`. Returns a screenshot name if this frame
    /// should be captured.
    fn build_frame(&mut self, canvas: &mut Canvas, now: Instant, width: f32) -> Option<String> {
        let dt = now.duration_since(self.stats.last).as_secs_f64() * 1000.0;
        self.stats.last = now;
        self.stats.frames += 1;
        self.stats.worst_acc = self.stats.worst_acc.max(dt);
        let span = now.duration_since(self.stats.window_start).as_secs_f64();
        if span >= 0.5 {
            self.stats.fps = self.stats.frames as f64 / span;
            self.stats.worst_ms = self.stats.worst_acc;
            self.stats.frames = 0;
            self.stats.worst_acc = 0.0;
            self.stats.window_start = now;
        }

        canvas.clear(width);
        self.frame_clip = None;
        self.poll_scan();
        self.poll_multiplayer();
        let mut next: Option<Scene> = None;
        let mut capture: Option<String> = None;
        let mut finish_calibration = false;
        let elapsed = now.duration_since(self.started).as_secs_f64() * 1000.0;
        if let Some(mut script) = self.input_script.take() {
            for action in script.due(elapsed) {
                match action {
                    debug::Action::Key(code) => self.key(code, false, None, now, None),
                    debug::Action::Wheel(n) => self.wheel(n),
                    debug::Action::Drag { corner, dx, dy, hold } => self.simulate_drag(corner, dx, dy, hold),
                    debug::Action::Release => self.mouse_button(false),
                }
            }
            self.input_script = Some(script);
        }
        if self.wall_shots.last().is_some_and(|&t| elapsed >= t) {
            capture = Some(format!("wall-{:.0}.png", self.wall_shots.pop().unwrap()));
        }
        if self.shots.is_some() && self.stats.frames > 5 {
            let screen = match self.scene {
                Scene::Select => Some("select"),
                Scene::Results { .. } => Some("results"),
                _ => None,
            };
            if let Some(name) = screen {
                if let Some(pos) = self.pending_screen_shots.iter().position(|&s| s == name) {
                    self.pending_screen_shots.remove(pos);
                    capture = Some(format!("{name}.png"));
                    if name == "results" && self.exit_when_done {
                        self.quit = true;
                    }
                }
            }
        }
        match &mut self.scene {
            Scene::Select => {
                self.select.rate_label = if (self.cfg.rate - 1.0).abs() > 1e-6 {
                    format!("  |  {:.2}x{}", self.cfg.rate, if self.cfg.rate_keep_pitch { "" } else { " (pitch)" })
                } else {
                    String::new()
                };
                self.select.draw(canvas, &self.library, dt, &self.status);
                if self.picking_lobby_map {
                    let w = canvas.width;
                    canvas.round_rect(24.0, 438.0, w - 48.0, 34.0, 8.0, rgba(0x2b1a17, 1.0), rgba(0xEB452C, 1.0), 1.5);
                    canvas.text_centered(w / 2.0, 450.0, 10.0, rgba(0xffffff, 1.0), "Picking the lobby map: Enter to choose, Esc to cancel");
                }
            }
            Scene::Loading { rx, label, entry } => {
                canvas.text_centered(canvas.width / 2.0, 200.0, 14.0, rgba(0xffffff, 1.0), "Loading");
                canvas.text_centered(canvas.width / 2.0, 226.0, 10.0, rgba(0x9a9ab0, 1.0), label);
                match rx.try_recv() {
                    Ok(Ok((map, track))) => {
                        let game = Game::new(map, track, self.audio.as_ref());
                        next = Some(Scene::Play { game: Box::new(game), entry: *entry });
                    }
                    Ok(Err(e)) => {
                        self.status = format!("failed to load {label}: {e}");
                        eprintln!("{}", self.status);
                        next = Some(Scene::Select);
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => next = Some(Scene::Select),
                }
            }
            Scene::Play { game, entry } => {
                let done = game.update(now, self.audio.as_ref(), self.cfg.offset_ms);
                if self.auto {
                    game.autoplay(self.audio.as_ref());
                }
                if let Some(sm) = &mut self.smooth {
                    if game.paused.is_some() {
                        // A frozen clock isn't jitter; restart the frame-to-frame comparison on resume.
                        sm.break_sequence();
                    } else {
                        let wall = now.duration_since(self.started).as_secs_f64() * 1000.0;
                        sm.sample(wall, game.raw_time, game.last_audio_ms);
                    }
                }
                game.show_offset_hint = self.cfg.show_offset_hint;
                game.ur_scaled_hint = self.cfg.offset_hint_ur_scaled;
                let hud_in = self.cfg.hud_in_playfield;
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, |c| {
                    game.draw_playfield(c, self.cfg.scroll_speed);
                    if hud_in {
                        game.draw_hud(c);
                    }
                });
                if !hud_in {
                    game.draw_hud(canvas);
                }
                if let Some(plan) = &mut self.shots {
                    if plan.times.last().is_some_and(|&t| game.time >= t) {
                        let t = plan.times.pop().unwrap();
                        capture = Some(format!("play-{t:.0}.png"));
                    }
                }
                if done {
                    let m = &game.map.meta;
                    let rate = if (game.rate - 1.0).abs() > 1e-6 { format!("  {:.2}x", game.rate) } else { String::new() };
                    let title = format!("{} - {} [{}]{rate}", m.artist, m.title, m.version);
                    next = Some(Scene::Results {
                        results: Results { title, stats: game.stats.clone(), windows: game.windows },
                        entry: *entry,
                    });
                }
            }
            Scene::Results { results, .. } => results.draw(canvas),
            Scene::MpMenu => {
                multiplayer::draw_menu(canvas, &self.mp_menu, &self.cfg.mp_name, self.cfg.mp_port, &self.cfg.mp_address, &self.mp_status, self.mp_connecting.is_some());
            }
            Scene::Lobby => match &self.mp {
                Some(s) => {
                    multiplayer::draw_lobby(canvas, s, s.have_map);
                }
                None => next = Some(Scene::MpMenu),
            },
            Scene::MpWait { rx, loaded, start } => {
                if let Some(r) = rx {
                    match r.try_recv() {
                        Ok(Ok(l)) => {
                            *loaded = Some(l);
                            *rx = None;
                            if let Some(s) = &self.mp {
                                s.client.send(net::ClientMsg::Loaded(true));
                            }
                        }
                        Ok(Err(e)) => {
                            if let Some(s) = &self.mp {
                                s.client.send(net::ClientMsg::Loaded(false));
                            }
                            self.status = format!("failed to load map: {e}");
                            next = Some(Scene::Lobby);
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                        Err(mpsc::TryRecvError::Disconnected) => next = Some(Scene::Lobby),
                    }
                }
                if start.is_some_and(|at| now >= at) {
                    if let Some((map, track)) = loaded.take() {
                        let game = Game::new(map, track, self.audio.as_ref());
                        next = Some(Scene::MpPlay { game: Box::new(game), quit_armed: None });
                    }
                }
                if let Some(s) = &self.mp {
                    multiplayer::draw_waiting(canvas, s, *start, loaded.is_some(), now);
                }
            }
            Scene::MpPlay { game, .. } => {
                let done = game.update(now, self.audio.as_ref(), self.cfg.offset_ms);
                if self.auto {
                    game.autoplay(self.audio.as_ref());
                }
                game.show_offset_hint = self.cfg.show_offset_hint;
                game.ur_scaled_hint = self.cfg.offset_hint_ur_scaled;
                let live = multiplayer::live_from(game);
                let mp = &self.mp;
                let hud_in = self.cfg.hud_in_playfield;
                let hud = |c: &mut Canvas, game: &Game| {
                    game.draw_hud(c);
                    if let Some(s) = mp {
                        multiplayer::draw_scoreboard(c, s, &live);
                    }
                };
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, |c| {
                    game.draw_playfield(c, self.cfg.scroll_speed);
                    if hud_in {
                        hud(c, game);
                    }
                });
                if !hud_in {
                    hud(canvas, game);
                }
                if let Some(s) = &mut self.mp {
                    if s.last_live.elapsed() >= Duration::from_millis(100) {
                        s.last_live = now;
                        s.client.send(net::ClientMsg::Live(live));
                    }
                    if done {
                        s.client.send(net::ClientMsg::Finished(multiplayer::final_from(&game.stats, false)));
                        next = Some(Scene::MpResults);
                    }
                }
            }
            Scene::MpResults => match &self.mp {
                Some(s) => multiplayer::draw_results(canvas, s),
                None => next = Some(Scene::MpMenu),
            },
            Scene::Calibrate { game, .. } => {
                if game.update(now, self.audio.as_ref(), self.cfg.offset_ms) {
                    finish_calibration = true;
                }
                if self.auto {
                    game.autoplay(self.audio.as_ref());
                }
                let offset = self.cfg.offset_ms;
                let hud_in = self.cfg.hud_in_playfield;
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, |c| {
                    game.draw_playfield(c, self.cfg.scroll_speed);
                    if hud_in {
                        game.draw_hud(c);
                        calibrate::draw_overlay(c, game, offset, game.hide_notes);
                    }
                });
                if !hud_in {
                    game.draw_hud(canvas);
                    calibrate::draw_overlay(canvas, game, offset, game.hide_notes);
                }
            }
            Scene::CalibrateDone { estimate, .. } => {
                calibrate::draw_result(canvas, estimate, self.cfg.offset_ms, self.cfg.calibration_bpm);
            }
            Scene::Settings { prev } => {
                let from_game = matches!(**prev, Scene::Play { .. });
                settings::tab_bar(canvas, 1, if from_game { "game paused" } else { "" });
                let footer = if from_game {
                    "Up/Down select   Left/Right change   Enter toggle / rebind   Esc back to pause menu"
                } else {
                    "Up/Down select   Left/Right change   Enter toggle / rebind   Esc or Tab back"
                };
                let this = &*self;
                this.settings_ui.draw(canvas, &|item| this.setting_value(item), footer);
            }
            Scene::LayoutEdit { .. } => {
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, game::draw_preview);
                let area = self.cfg.playfield_area;
                layout::draw_frame(canvas, area, true);
                let w = canvas.width;
                canvas.round_rect(w / 2.0 - 330.0, 440.0, 660.0, 34.0, 8.0, rgba(0x000000, 0.75), rgba(0xEB452C, 0.6), 1.0);
                canvas.text_centered(w / 2.0, 446.0, 8.0, rgba(0xffffff, 1.0), &format!("Playfield area: {}", area.describe()));
                canvas.text_centered(w / 2.0, 460.0, 7.0, rgba(0x9a9ab0, 1.0), "Drag: move (snaps; Shift = free), edges: resize, wheel: scale.  [ ] presets, arrows move, Shift+arrows resize, R full, Enter done");
            }
        }
        if self.grip.is_some() {
            layout::draw_guides(canvas, &self.guides);
        }
        if finish_calibration {
            self.finish_calibration();
        }
        if let Some(s) = next {
            if matches!(s, Scene::Results { .. } | Scene::Select) {
                if let Some(a) = &self.audio {
                    a.stop();
                }
                if let (Some(sm), Scene::Play { .. }) = (&mut self.smooth, &self.scene) {
                    eprintln!("--- smoothness ({}) ---\n{}", self.present.name(), sm.report());
                    *sm = debug::Smoothness::default();
                }
            }
            if let (Scene::Results { results, .. }, Scene::Play { .. }) = (&s, &self.scene) {
                let st = &results.stats;
                eprintln!(
                    "result: {:.2}%  great {} ok {} miss {}  max combo {}  UR {:.1}  mean {:+.2} ms  rolls {}  swells {}  big {}",
                    st.accuracy() * 100.0, st.great, st.ok, st.miss, st.max_combo, st.unstable_rate(), st.mean_error(), st.roll_ticks, st.swells, st.strong
                );
            }
            self.scene = s;
        }

        if self.cfg.show_fps {
            let mode = match self.present {
                PresentChoice::Fifo if self.cfg.frame_delay_ms > 0.0 => format!("vsync +{:.0}ms delay", self.cfg.frame_delay_ms),
                PresentChoice::Fifo => "vsync".to_string(),
                p if self.cfg.fps_cap > 0 => format!("{} cap {}", p.name(), self.cfg.fps_cap),
                p => p.name().to_string(),
            };
            let text = format!("{:.0} fps  {:.2} ms  worst {:.1} ms  {mode}", self.stats.fps, 1000.0 / self.stats.fps.max(1.0), self.stats.worst_ms);
            let tw = Canvas::text_width(8.0, &text);
            canvas.round_rect(6.0, 6.0, tw + 12.0, 16.0, 4.0, rgba(0x000000, 0.55), rgba(0, 0.0), 0.0);
            canvas.text(12.0, 10.0, 8.0, rgba(0x9fe09f, 1.0), &text);
        }
        if let Some(t) = &self.toast {
            if now < t.until {
                let a = (t.until.duration_since(now).as_secs_f32() / 0.3).min(1.0);
                let tw = Canvas::text_width(12.0, &t.text);
                let x = canvas.width / 2.0 - tw / 2.0;
                canvas.round_rect(x - 12.0, 380.0, tw + 24.0, 28.0, 8.0, rgba(0x000000, 0.7 * a), rgba(0xffffff, 0.2 * a), 1.0);
                canvas.text(x, 388.0, 12.0, rgba(0xffffff, a), &t.text);
            } else {
                self.toast = None;
            }
        }
        capture
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    // A panic elsewhere shouldn't take the game down with a poisoned lock.
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sleeps until `deadline` with sub-100µs precision: coarse sleep, then spin the last stretch.
fn sleep_until(deadline: Instant) {
    const SPIN: Duration = Duration::from_micros(500);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > SPIN {
            std::thread::sleep(left - SPIN);
        } else {
            std::hint::spin_loop();
        }
    }
}

/// The render loop. Order matters for latency: (under vsync) optionally delay, wait for the
/// swapchain, and only then sample the clock and build the frame.
fn render_loop(state: Arc<Mutex<State>>, mut renderer: Renderer, proxy: EventLoopProxy<()>) {
    let mut canvas = Canvas::default();
    let mut next_frame = Instant::now();
    let mut last_sample: Option<Instant> = None;
    let mut waits = (0.0, 0.0, 0.0);
    loop {
        let (present, cap, delay) = {
            let mut s = lock(&state);
            if s.quit {
                break;
            }
            if let Some((w, h)) = s.want_size.take() {
                renderer.resize(w, h);
            }
            if let Some(p) = s.want_present.take() {
                renderer.set_present(p);
                // A mode the compositor lacks falls back; remember what we actually got.
                s.present = renderer.present;
                s.cfg.present = renderer.present;
                let name = renderer.present.name();
                s.toast(format!("present mode: {name}"));
                s.cfg.save();
            }
            if matches!(s.scene, Scene::Play { .. }) {
                if let Some(sm) = &mut s.smooth {
                    sm.sample_waits(waits.0, waits.1, waits.2);
                }
            }
            (s.present, s.cfg.fps_cap, s.cfg.frame_delay_ms)
        };

        let t0 = Instant::now();
        let Some(frame) = renderer.acquire() else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        let acquired = Instant::now();
        // Under vsync, acquire returns right after a refresh. Sleeping here (not before acquire)
        // moves the clock sample later into the refresh interval, closer to the compositor's
        // deadline, which is what actually reduces visual latency.
        if present == PresentChoice::Fifo && delay > 0.0 {
            sleep_until(acquired + Duration::from_secs_f64(delay / 1000.0));
        }
        let now = Instant::now();
        let (capture, clip) = {
            let mut s = lock(&state);
            let capture = s.build_frame(&mut canvas, now, renderer.virtual_width());
            (capture, s.frame_clip.take())
        };
        let clip = clip.map(|(range, a)| (range, [a.x, a.y, a.w, a.h]));
        let t1 = Instant::now();
        let shot = renderer.submit(frame, &canvas.shapes, [0.071, 0.071, 0.098], capture.is_some(), clip);
        let presented = Instant::now();
        // Clock-sample-to-next-refresh: approximates how stale the previous frame was when shown.
        let sample_age = last_sample.map(|s| (acquired - s).as_secs_f64() * 1000.0).unwrap_or(0.0);
        last_sample = Some(now);
        waits = ((acquired - t0).as_secs_f64() * 1000.0, (presented - t1).as_secs_f64() * 1000.0, sample_age);
        if let (Some(name), Some((w, h, rgb))) = (capture, shot) {
            if let Some(dir) = lock(&state).shots.as_ref().map(|p| p.dir.clone()) {
                let path = dir.join(name);
                match debug::write_png(&path, w, h, &rgb) {
                    Ok(()) => eprintln!("saved {}", path.display()),
                    Err(e) => eprintln!("screenshot failed: {e}"),
                }
            }
        }

        if present != PresentChoice::Fifo && cap > 0 {
            // Fixed grid so frames are evenly spaced; resync if we fell behind.
            let period = Duration::from_secs_f64(1.0 / cap as f64);
            next_frame += period;
            if next_frame < Instant::now() {
                next_frame = Instant::now() + period;
            }
            sleep_until(next_frame);
        }
    }
    let _ = proxy.send_event(());
}

struct App {
    state: Arc<Mutex<State>>,
    /// Last cursor icon set, so it is only changed when the hover target changes.
    cursor: CursorIcon,
    window: Option<Arc<Window>>,
    render_thread: Option<JoinHandle<()>>,
    proxy: EventLoopProxy<()>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut attrs = Window::default_attributes()
            .with_title("taiko-rs")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        #[cfg(target_os = "linux")]
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            attrs = attrs.with_name("taiko-rs", "taiko-rs");
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
            event_loop.owned_display_handle(),
        )));
        let present = lock(&self.state).cfg.present;
        let renderer = match Renderer::new(&instance, window.clone(), present) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("failed to initialise GPU: {e}");
                event_loop.exit();
                return;
            }
        };
        let refresh_hz = window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| mhz as f64 / 1000.0)
            .unwrap_or(60.0);
        {
            let mut s = lock(&self.state);
            s.adapter = renderer.adapter_name.clone();
            s.window_w = window.inner_size().width as f32;
            s.window_h = window.inner_size().height as f32;
            s.monitor_modes = window
                .current_monitor()
                .map(|m| m.video_modes().map(|v| (v.size().width, v.size().height)).collect())
                .unwrap_or_default();
            s.refresh_presets();
            s.refresh_hz = refresh_hz;
            s.refresh_status();
            eprintln!("renderer: {}, present: {}, refresh {refresh_hz:.2} Hz", renderer.adapter_name, renderer.present.name());
            s.present = renderer.present;
            s.cfg.present = renderer.present;
            s.refresh_hz = refresh_hz;
        }
        let state = self.state.clone();
        let proxy = self.proxy.clone();
        self.render_thread = Some(
            std::thread::Builder::new()
                .name("render".into())
                .spawn(move || render_loop(state, renderer, proxy))
                .expect("spawn render thread"),
        );
        // Input only wakes this thread when there is something to do.
        event_loop.set_control_flow(ControlFlow::Wait);
        self.window = Some(window);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
        // The render thread stopped (quit requested or screenshot run finished).
        event_loop.exit();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => lock(&self.state).quit = true,
            WindowEvent::Resized(size) => {
                let mut s = lock(&self.state);
                s.want_size = Some((size.width, size.height));
                s.window_w = size.width as f32;
                s.window_h = size.height as f32;
                s.refresh_presets();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Timestamp before anything else, including taking the lock.
                let at = Instant::now();
                if event.state != ElementState::Pressed {
                    return;
                }
                let PhysicalKey::Code(code) = event.physical_key else { return };
                lock(&self.state).key(code, event.repeat, event.text.as_deref(), at, self.window.as_deref());
            }
            WindowEvent::ModifiersChanged(m) => lock(&self.state).modifiers_shift = m.state().shift_key(),
            WindowEvent::MouseWheel { delta, .. } => {
                let mut s = lock(&self.state);
                s.wheel_acc += match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    // Touchpads report pixels; ~50 px per notch feels like a wheel.
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 50.0,
                };
                let notches = s.wheel_acc.trunc();
                s.wheel_acc -= notches;
                s.wheel(notches as i32);
            }
            WindowEvent::MouseInput { state: button_state, button: winit::event::MouseButton::Left, .. } => {
                lock(&self.state).mouse_button(button_state == ElementState::Pressed);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let icon = lock(&self.state).mouse_moved(position.x, position.y);
                if icon != self.cursor {
                    self.cursor = icon;
                    if let Some(w) = &self.window {
                        w.set_cursor(icon);
                    }
                }
            }
            WindowEvent::Focused(false) => {
                // Losing focus mid-song (alt-tab, notification) pauses instead of failing the map.
                let mut guard = lock(&self.state);
                let s = &mut *guard;
                if let (Scene::Play { game, .. }, false) = (&mut s.scene, s.auto) {
                    game.pause(s.audio.as_ref());
                }
            }
            _ => {}
        }
        if self.render_thread.is_none() && lock(&self.state).quit {
            event_loop.exit();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        lock(&self.state).quit = true;
        if let Some(t) = self.render_thread.take() {
            // A vsync'd present can block indefinitely while the window is minimised; don't let
            // that keep the game from closing.
            let deadline = Instant::now() + Duration::from_millis(750);
            while !t.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
        lock(&self.state).cfg.save();
    }
}

fn main() {
    let cfg = Config::load();
    let mut args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let auto = args.iter().any(|a| a == "--auto");
    args.retain(|a| a != "--auto");
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: taiko-rs [--auto] [Songs folder | map folder | file.osz | file.osu]...");
        println!("       taiko-rs --info file.osu...   print converted map stats");
        println!("With no arguments, scans osu! stable install locations and `songs =` lines in the config.");
        return;
    }
    if args.first().is_some_and(|a| a == "--info") {
        for path in &args[1..] {
            info(path);
        }
        return;
    }
    let mut inputs = if args.is_empty() { library::default_song_dirs() } else { args.clone() };
    if args.is_empty() {
        inputs.extend(cfg.songs.iter().cloned());
    }
    // Scan on a background thread so the window appears at once, even with a huge library.
    let found = Arc::new(AtomicUsize::new(0));
    let (scan_tx, scan_rx) = mpsc::channel();
    {
        let found = found.clone();
        std::thread::Builder::new()
            .name("library".into())
            .spawn(move || {
                let started = Instant::now();
                let library = library::scan(&inputs, &found);
                eprintln!("scanned {} files in {:.0} ms", found.load(Ordering::Relaxed), started.elapsed().as_secs_f64() * 1000.0);
                let _ = scan_tx.send(library);
            })
            .expect("spawn library scan");
    }

    let audio = match Engine::new(cfg.audio_buffer) {
        Ok(a) => {
            eprintln!("audio: {}", a.description);
            a.set_volume(cfg.music_volume, cfg.fx_volume);
            Some(a)
        }
        Err(e) => {
            eprintln!("audio unavailable ({e}); running silent");
            None
        }
    };
    cfg.save();

    let library = Vec::new();
    let select = Select::new(&library);
    // A single .osu on the command line goes straight into gameplay.
    let play_when_scanned = args.len() == 1 && args[0].extension().is_some_and(|e| e == "osu");
    let present = cfg.present;
    let state = State {
        cfg,
        library,
        select,
        scene: Scene::Select,
        audio,
        toast: None,
        stats: FrameStats {
            last: Instant::now(),
            frames: 0,
            window_start: Instant::now(),
            fps: 0.0,
            worst_ms: 0.0,
            worst_acc: 0.0,
        },
        status: String::new(),
        modifiers_shift: false,
        auto,
        shots: debug::ShotPlan::from_env(),
        pending_screen_shots: vec!["select", "results"],
        exit_when_done: std::env::var_os("TAIKO_EXIT_AFTER").is_some(),
        quit: false,
        smooth: std::env::var_os("TAIKO_STATS").is_some().then(debug::Smoothness::default),
        started: Instant::now(),
        want_present: None,
        want_size: None,
        present,
        refresh_hz: 60.0,
        settings_ui: SettingsUi::default(),
        fullscreen: false,
        adapter: String::new(),
        input_script: debug::InputScript::from_env(),
        wall_shots: debug::wall_shots_from_env(),
        mp: None,
        mp_connecting: None,
        mp_menu: multiplayer::Menu::default(),
        mp_status: String::new(),
        picking_lobby_map: false,
        scan: Some((scan_rx, found)),
        play_when_scanned,
        wheel_acc: 0.0,
        mouse: (0.0, 0.0),
        grip: None,
        hover: None,
        window_w: 1280.0,
        window_h: 720.0,
        frame_clip: None,
        guides: Vec::new(),
        resolutions: Vec::new(),
        monitor_modes: Vec::new(),
    };
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App { state: Arc::new(Mutex::new(state)), cursor: CursorIcon::Default, window: None, render_thread: None, proxy: event_loop.create_proxy() };
    event_loop.run_app(&mut app).expect("run");
}

/// `taiko-rs --info map.osu...`: prints what the converter produced, for checking maps headlessly.
fn info(path: &std::path::Path) {
    let map = match beatmap::load(path) {
        Ok(m) => m,
        Err(e) => return eprintln!("{}: {e}", path.display()),
    };
    let count = |k: beatmap::Kind| map.notes.iter().filter(|n| n.kind == k).count();
    let big = map.notes.iter().filter(|n| n.big).count();
    let m = &map.meta;
    println!("{} - {} [{}] mode {} v{} OD {}", m.artist, m.title, m.version, m.mode, m.format_version, m.od);
    println!(
        "  don {}  kat {}  big {}  roll {}  swell {}  barlines {}  control points {}",
        count(beatmap::Kind::Don),
        count(beatmap::Kind::Kat),
        big,
        count(beatmap::Kind::Roll),
        count(beatmap::Kind::Swell),
        map.barlines.len(),
        map.control_points.len()
    );
    let (lo, hi) = map.notes.iter().fold((f64::MAX, 0.0f64), |(lo, hi), n| (lo.min(n.velocity), hi.max(n.velocity)));
    println!("  first {:.0} ms  last {:.0} ms  velocity {lo:.3}..{hi:.3} px/ms", map.notes[0].time, map.notes.last().unwrap().end_time);
    let audio_path = path.parent().unwrap_or(path).join(&m.audio_file);
    let t = Instant::now();
    match audio::decode(&audio_path, 48000) {
        Ok(track) => println!("  audio {:.1} s decoded in {:.0} ms", track.frames() as f64 / 48000.0, t.elapsed().as_secs_f64() * 1000.0),
        Err(e) => println!("  audio error: {e}"),
    }
}
