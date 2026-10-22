mod audio;
mod beatmap;
mod clock;
mod config;
mod debug;
mod font;
mod game;
mod library;
mod render;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use audio::{Engine, Track};
use beatmap::Beatmap;
use config::{Config, FPS_CAPS};
use game::Game;
use library::Entry;
use render::{rgba, Canvas, Renderer};
use ui::{Results, Select, SelectAction};

type Loaded = Result<(Beatmap, Arc<Track>), String>;

enum Scene {
    Select,
    Loading { rx: Receiver<Loaded>, label: String, entry: usize },
    Play { game: Box<Game>, entry: usize },
    Results { results: Results, entry: usize },
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

struct App {
    cfg: Config,
    library: Vec<Entry>,
    select: Select,
    scene: Scene,
    audio: Option<Engine>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    canvas: Canvas,
    toast: Option<Toast>,
    stats: FrameStats,
    next_frame: Instant,
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
}

impl App {
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

    fn handle_global_key(&mut self, key: KeyCode, shift: bool, event_loop: &ActiveEventLoop) -> bool {
        let step = if shift { 5.0 } else { 1.0 };
        match key {
            KeyCode::Minus | KeyCode::NumpadSubtract => self.cfg.offset_ms -= step,
            KeyCode::Equal | KeyCode::NumpadAdd => self.cfg.offset_ms += step,
            KeyCode::F3 => self.cfg.scroll_speed = (self.cfg.scroll_speed - 0.05).max(0.3),
            KeyCode::F4 => self.cfg.scroll_speed = (self.cfg.scroll_speed + 0.05).min(4.0),
            KeyCode::F7 => {
                if let Some(r) = &mut self.renderer {
                    r.set_present(self.cfg.present.next());
                    // A mode the compositor lacks falls back; skip past it next time.
                    self.cfg.present = r.present;
                }
                self.toast(format!("present mode: {}", self.cfg.present.name()));
                self.cfg.save();
                return true;
            }
            KeyCode::F8 => {
                let i = FPS_CAPS.iter().position(|&c| c == self.cfg.fps_cap).unwrap_or(0);
                self.cfg.fps_cap = FPS_CAPS[(i + 1) % FPS_CAPS.len()];
                let text = if self.cfg.fps_cap == 0 { "fps cap: unlimited".into() } else { format!("fps cap: {}", self.cfg.fps_cap) };
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
                if let Some(w) = &self.window {
                    let fs = if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) };
                    w.set_fullscreen(fs);
                }
                return true;
            }
            KeyCode::F12 => {
                event_loop.exit();
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

    fn redraw(&mut self) {
        let now = Instant::now();
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

        let Some(renderer) = &self.renderer else { return };
        self.canvas.clear(renderer.virtual_width());
        let mut next: Option<Scene> = None;
        let mut capture: Option<String> = None;
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
                self.select.draw(&mut self.canvas, &self.library, dt, &self.status);
            }
            Scene::Loading { rx, label, entry } => {
                let c = &mut self.canvas;
                c.text_centered(c.width / 2.0, 200.0, 14.0, rgba(0xffffff, 1.0), "Loading");
                c.text_centered(c.width / 2.0, 226.0, 10.0, rgba(0x9a9ab0, 1.0), label);
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
                    let wall = now.duration_since(self.started).as_secs_f64() * 1000.0;
                    sm.sample(wall, game.raw_time, game.last_audio_ms);
                }
                game.draw(&mut self.canvas, self.cfg.scroll_speed);
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
            Scene::Results { results, .. } => results.draw(&mut self.canvas),
        }
        if let Some(s) = next {
            if matches!(s, Scene::Results { .. } | Scene::Select) {
                if let Some(a) = &self.audio {
                    a.stop();
                }
                if let (Some(sm), Scene::Play { .. }) = (&mut self.smooth, &self.scene) {
                    eprintln!("--- smoothness ---\n{}", sm.report());
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

        let c = &mut self.canvas;
        if self.cfg.show_fps {
            let r = self.renderer.as_ref().unwrap();
            let text = format!("{:.0} fps  {:.2} ms  worst {:.1} ms  {}", self.stats.fps, 1000.0 / self.stats.fps.max(1.0), self.stats.worst_ms, r.present.name());
            let tw = Canvas::text_width(8.0, &text);
            c.round_rect(6.0, 6.0, tw + 12.0, 16.0, 4.0, rgba(0x000000, 0.55), rgba(0, 0.0), 0.0);
            c.text(12.0, 10.0, 8.0, rgba(0x9fe09f, 1.0), &text);
        }
        if let Some(t) = &self.toast {
            if now < t.until {
                let a = (t.until.duration_since(now).as_secs_f32() / 0.3).min(1.0);
                let tw = Canvas::text_width(12.0, &t.text);
                let x = c.width / 2.0 - tw / 2.0;
                c.round_rect(x - 12.0, 380.0, tw + 24.0, 28.0, 8.0, rgba(0x000000, 0.7 * a), rgba(0xffffff, 0.2 * a), 1.0);
                c.text(x, 388.0, 12.0, rgba(0xffffff, a), &t.text);
            } else {
                self.toast = None;
            }
        }
        let renderer = self.renderer.as_mut().unwrap();
        let shot = renderer.render(&self.canvas.shapes, [0.071, 0.071, 0.098], capture.is_some());
        if let (Some(name), Some((w, h, rgb)), Some(plan)) = (capture, shot, &self.shots) {
            let path = plan.dir.join(name);
            match debug::write_png(&path, w, h, &rgb) {
                Ok(()) => eprintln!("saved {}", path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
        }
    }
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
        match Renderer::new(&instance, window.clone(), self.cfg.present) {
            Ok(r) => {
                let audio_desc = self.audio.as_ref().map(|a| a.description.clone()).unwrap_or_else(|| "no audio".into());
                self.status = format!("{}  |  {}  |  config: ~/.config/taiko-rs/config.txt", r.adapter_name, audio_desc);
                eprintln!("renderer: {}, present: {}", r.adapter_name, r.present.name());
                self.cfg.present = r.present;
                self.renderer = Some(r);
            }
            Err(e) => {
                eprintln!("failed to initialise GPU: {e}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::KeyboardInput { event, .. } => {
                // Timestamp first: judging uses when the key arrived, not when the next frame runs.
                let at = Instant::now();
                if event.state != ElementState::Pressed {
                    return;
                }
                let PhysicalKey::Code(code) = event.physical_key else { return };
                if let Scene::Play { game, .. } = &mut self.scene {
                    if let Some(k) = self.cfg.keys.iter().position(|&k| k == code) {
                        if !event.repeat {
                            game.press(k, at, self.audio.as_ref(), self.cfg.offset_ms);
                        }
                        return;
                    }
                }
                if event.repeat && !matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::Backspace | KeyCode::PageUp | KeyCode::PageDown) {
                    return;
                }
                let shift = self.modifiers_shift;
                if self.handle_global_key(code, shift, event_loop) {
                    return;
                }
                match &mut self.scene {
                    Scene::Select => match self.select.key(code, &self.library) {
                        SelectAction::Play(i) => self.start_loading(i),
                        SelectAction::Quit => event_loop.exit(),
                        SelectAction::None => {
                            if let Some(text) = &event.text {
                                self.select.type_text(text, &self.library);
                            }
                        }
                    },
                    Scene::Loading { .. } => {
                        if code == KeyCode::Escape {
                            self.to_select();
                        }
                    }
                    Scene::Play { game, .. } => match code {
                        KeyCode::Escape => self.to_select(),
                        KeyCode::Space => game.skip(self.audio.as_ref(), self.cfg.offset_ms),
                        KeyCode::Backquote => {
                            let entry = match &self.scene { Scene::Play { entry, .. } => *entry, _ => unreachable!() };
                            self.start_loading(entry);
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
            WindowEvent::ModifiersChanged(m) => self.modifiers_shift = m.state().shift_key(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            event_loop.exit();
            return;
        }
        let Some(w) = &self.window else { return };
        if self.cfg.fps_cap == 0 {
            event_loop.set_control_flow(ControlFlow::Poll);
            w.request_redraw();
            return;
        }
        let now = Instant::now();
        let period = Duration::from_secs_f64(1.0 / self.cfg.fps_cap as f64);
        if now >= self.next_frame {
            // Advance on a fixed grid so the limiter doesn't drift; resync if we fell behind.
            self.next_frame += period;
            if self.next_frame < now {
                self.next_frame = now + period;
            }
            w.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.cfg.save();
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
    let mut app = App {
        cfg,
        library,
        select,
        scene: Scene::Select,
        audio,
        window: None,
        renderer: None,
        canvas: Canvas::default(),
        toast: None,
        stats: FrameStats {
            last: Instant::now(),
            frames: 0,
            window_start: Instant::now(),
            fps: 0.0,
            worst_ms: 0.0,
            worst_acc: 0.0,
        },
        next_frame: Instant::now(),
        status: String::new(),
        modifiers_shift: false,
        auto,
        shots: debug::ShotPlan::from_env(),
        pending_screen_shots: vec!["select", "results"],
        exit_when_done: std::env::var_os("TAIKO_EXIT_AFTER").is_some(),
        quit: false,
        smooth: std::env::var_os("TAIKO_STATS").is_some().then(debug::Smoothness::default),
        started: Instant::now(),
    };
    if let Some(i) = direct {
        app.start_loading(i);
    }
    let event_loop = EventLoop::new().expect("event loop");
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
