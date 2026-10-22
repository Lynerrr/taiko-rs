mod audio;
mod beatmap;
mod clock;
mod config;
mod debug;
mod font;
mod game;
mod library;
mod render;
mod settings;
mod ui;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
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
use config::{Config, FPS_CAPS, FRAME_DELAYS};
use game::{Game, PAUSE_ITEMS};
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
        let path = self.library[entry].path.clone();
        let label = self.library[entry].label();
        let rate = self.audio.as_ref().map(|a| a.rate);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let map = beatmap::load(&path)?;
                let audio_path = path.parent().unwrap_or(&path).join(&map.meta.audio_file);
                let track = match rate {
                    Some(rate) => audio::decode(&audio_path, rate)?,
                    None => Track { samples: Vec::new() },
                };
                Ok((map, Arc::new(track)))
            })();
            let _ = tx.send(result);
        });
        self.scene = Scene::Loading { rx, label, entry };
    }

    fn to_select(&mut self) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        self.scene = Scene::Select;
    }

    /// Longest frame delay that still leaves room to render before the next refresh.
    fn max_frame_delay(&self) -> f64 {
        (1000.0 / self.refresh_hz - 3.0).max(0.0)
    }

    fn refresh_status(&mut self) {
        let audio_desc = self.audio.as_ref().map(|a| a.description.clone()).unwrap_or_else(|| "no audio".into());
        self.status = format!("{}  |  {}  |  {:.0} Hz  |  config: ~/.config/taiko-rs/config.txt", self.adapter, audio_desc, self.refresh_hz);
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
            Item::FrameDelay => self.cfg.frame_delay_ms = (self.cfg.frame_delay_ms + d).clamp(0.0, self.max_frame_delay()).round(),
            Item::FpsCap => {
                let i = FPS_CAPS.iter().position(|&c| c == self.cfg.fps_cap).unwrap_or(0) as i32;
                self.cfg.fps_cap = FPS_CAPS[(i + dir).rem_euclid(FPS_CAPS.len() as i32) as usize];
            }
            Item::ShowFps => self.cfg.show_fps = !self.cfg.show_fps,
            Item::Fullscreen => self.toggle_fullscreen(window),
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
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.adjust_setting(item, 1, window),
            KeyCode::Escape | KeyCode::Tab | KeyCode::F1 => self.close_settings(),
            _ => {}
        }
    }

    fn pause_key(&mut self, code: KeyCode) {
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
                let mut next = FRAME_DELAYS[(i + 1) % FRAME_DELAYS.len()];
                if next > self.max_frame_delay() {
                    next = 0.0;
                }
                self.cfg.frame_delay_ms = next;
                let mut text = format!("frame delay: {next:.0} ms");
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

    fn key(&mut self, code: KeyCode, repeat: bool, text: Option<&str>, at: Instant, window: Option<&Window>) {
        // Key capture must see every key, before any shortcut can claim it.
        if self.settings_ui.binding.is_some() && matches!(self.scene, Scene::Settings { .. }) {
            if !repeat {
                self.settings_key(code, window);
            }
            return;
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
            Scene::Select => match self.select.key(code, &self.library) {
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
            Scene::Play { game, entry } => match code {
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
        let mut next: Option<Scene> = None;
        let mut capture: Option<String> = None;
        let elapsed = now.duration_since(self.started).as_secs_f64() * 1000.0;
        if let Some(mut script) = self.input_script.take() {
            for code in script.due(elapsed) {
                self.key(code, false, None, now, None);
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
                self.select.draw(canvas, &self.library, dt, &self.status);
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
                game.draw(canvas, self.cfg.scroll_speed);
                if let Some(plan) = &mut self.shots {
                    if plan.times.last().is_some_and(|&t| game.time >= t) {
                        let t = plan.times.pop().unwrap();
                        capture = Some(format!("play-{t:.0}.png"));
                    }
                }
                if done {
                    let m = &game.map.meta;
                    let title = format!("{} - {} [{}]", m.artist, m.title, m.version);
                    next = Some(Scene::Results {
                        results: Results { title, stats: game.stats.clone(), windows: game.windows },
                        entry: *entry,
                    });
                }
            }
            Scene::Results { results, .. } => results.draw(canvas),
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
            (s.present, s.cfg.fps_cap, s.cfg.frame_delay_ms.min(s.max_frame_delay()))
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
        let capture = lock(&state).build_frame(&mut canvas, now, renderer.virtual_width());
        let t1 = Instant::now();
        let shot = renderer.submit(frame, &canvas.shapes, [0.071, 0.071, 0.098], capture.is_some());
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
    window: Option<Arc<Window>>,
    render_thread: Option<JoinHandle<()>>,
    proxy: EventLoopProxy<()>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
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
            WindowEvent::Resized(size) => lock(&self.state).want_size = Some((size.width, size.height)),
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
            let _ = t.join();
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
    let started = Instant::now();
    let library = library::scan(&inputs);
    eprintln!("found {} playable difficulties in {:.0} ms", library.len(), started.elapsed().as_secs_f64() * 1000.0);

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

    let select = Select::new(&library);
    // A single .osu on the command line goes straight into gameplay.
    let direct = (args.len() == 1 && library.len() == 1 && args[0].extension().is_some_and(|e| e == "osu")).then_some(0);
    let present = cfg.present;
    let mut state = State {
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
    };
    if let Some(i) = direct {
        state.start_loading(i);
    }
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App { state: Arc::new(Mutex::new(state)), window: None, render_thread: None, proxy: event_loop.create_proxy() };
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
