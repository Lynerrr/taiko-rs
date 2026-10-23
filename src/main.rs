// Release builds on Windows are a GUI app: no console window behind the game.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod beatmap;
mod browse;
mod calibrate;
mod clock;
mod config;
mod debug;
mod difficulty;
mod download;
mod font;
mod game;
mod layout;
mod library;
mod mapinfo;
mod multiplayer;
mod net;
mod osudb;
mod preview;
mod render;
mod settings;
mod skin;
mod stars;
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
use config::{Config, FPS_CAPS, RATE_RANGE};
use game::{Game, PAUSE_ITEMS};
use layout::{Area, Grip};
use winit::window::CursorIcon;
use library::Entry;
use render::{rgba, Canvas, PresentChoice, Renderer};
use settings::{Dropdown, Item, SettingsUi};
use ui::{Results, Select, SelectAction};

type Loaded = Result<(Beatmap, Arc<Track>), String>;

enum Scene {
    Select,
    /// `from`: map time to start at (picked on the song select timeline), None = from the start.
    Loading { rx: Receiver<Loaded>, label: String, entry: usize, from: Option<f64> },
    Play { game: Box<Game>, entry: usize, from: Option<f64> },
    Results { results: Results, entry: usize, from: Option<f64> },
    /// Settings tab; `prev` is restored on exit (song select, or a paused game).
    Settings { prev: Box<Scene> },
    /// Offset calibration run; `back` is where to return (the settings tab).
    Calibrate { game: Box<Game>, back: Box<Scene> },
    CalibrateDone { estimate: calibrate::Estimate, back: Box<Scene> },
    /// Full-screen editor for the playfield area, opened from settings.
    LayoutEdit { back: Box<Scene> },
    /// The autoplay preview over the whole screen (in the playfield area, which can be moved
    /// and resized here), with the timeline. `back`: song select or the multiplayer lobby.
    Preview { back: Box<Scene> },
    /// Download tab: search the mirrors, download sets into the Songs folder.
    Download,
    /// Multiplayer: host/join form, room lobby, waiting/countdown, match, results.
    MpMenu,
    Lobby,
    /// `from`: map time the match starts at.
    MpWait { rx: Option<Receiver<Loaded>>, loaded: Option<(Beatmap, Arc<Track>)>, start: Option<Instant>, from: Option<f64> },
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
    modifiers_ctrl: bool,
    modifiers_alt: bool,
    /// When the volume overlay was last poked (Alt + wheel).
    volume_shown: Option<Instant>,
    /// Clickable areas of the last frame drawn.
    hits: Vec<render::HitBox>,
    browse: browse::Browse,
    downloads: Vec<download::Job>,
    /// Where downloaded sets go.
    songs_dir: PathBuf,
    /// Beatmap set IDs in the library (built when first needed).
    installed_sets: Option<std::collections::HashSet<u32>>,
    /// Whether the preview currently shows Download tab entries.
    preview_remote: bool,
    /// Receiving the lobby map from the host: where its parts go.
    host_transfer: Option<std::sync::mpsc::Sender<Option<Vec<u8>>>>,
    /// When the background image last changed (it fades in).
    bg_changed: Option<Instant>,
    /// A newly loaded skin's note images, for the render thread to upload.
    sprites_upload: Option<render::Atlas>,
    /// The window has keyboard focus.
    focused: bool,
    /// Background star rating / length / BPM analysis of the library, for sorting.
    mapinfo: Option<mapinfo::MapInfo>,
    /// Whether the song list was re-sorted after the analysis finished.
    mapinfo_sorted: bool,
    /// Folders being scanned (where osu!.db / collection.db are looked for).
    song_dirs: Vec<PathBuf>,
    osudb_rx: Option<Receiver<Option<osudb::OsuDb>>>,
    /// Looking up the lobby map on the mirrors (it had no set ID).
    lobby_lookup: Option<Receiver<Result<download::SetInfo, String>>>,
    /// Which screen was drawn last and since when, for the fade between screens.
    scene_kind: Option<std::mem::Discriminant<Scene>>,
    scene_since: Instant,
    /// Dragging along the song select difficulty timeline; last seek time for throttling.
    scrubbing: Option<Instant>,
    auto: bool,
    shots: Option<debug::ShotPlan>,
    /// Screens still to capture once each when a shot plan is active.
    pending_screen_shots: Vec<&'static str>,
    exit_when_done: bool,
    quit: bool,
    smooth: Option<debug::Smoothness>,
    started: Instant,
    /// Requests from the input thread that only the render thread can apply.
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
    /// Song select preview: background, preview music, autoplay preview.
    preview: preview::Preview,
    /// Star ratings (rosu-pp), calculated in the background for the selected song.
    stars: stars::Stars,
    /// Background brightness for this frame (0 hides it).
    bg_brightness: f32,
    /// Where the playfield size readout was drawn (virtual units), so it can be clicked.
    readout_rect: Option<[f32; 4]>,
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
    see_through: bool,
    f: impl FnOnce(&mut Canvas),
) {
    let area = cfg.playfield_area;
    let (start, full) = layout::begin(canvas, area, see_through);
    f(canvas);
    let range = layout::end(canvas, area, start, full);
    *clip = Some((range, area));
    if grip.is_some() || matches!(hover, Some(Grip::Resize { .. })) {
        layout::draw_frame(canvas, area, grip.is_some());
    }
}

fn fmt_time(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Volume overlay (bottom right) shown while Alt + wheel changes the master volume.
fn draw_volume(c: &mut Canvas, cfg: &Config, a: f32, slide: f32) {
    let (w, h) = (190.0, 58.0);
    let (x, y) = (c.width - w - 16.0 + slide, 480.0 - h - 30.0);
    c.round_rect(x, y, w, h, 8.0, rgba(0x101018, 0.88 * a), rgba(0x3a3a4e, a), 1.0);
    c.text(x + 12.0, y + 10.0, 9.0, rgba(0xffffff, a), "Volume");
    c.text_right(x + w - 12.0, y + 10.0, 9.0, rgba(0xffffff, a), &format!("{:.0}%", cfg.master_volume * 100.0));
    let (bx, by, bw) = (x + 12.0, y + 26.0, w - 24.0);
    c.round_rect(bx, by, bw, 6.0, 3.0, rgba(0x2a2a38, a), rgba(0, 0.0), 0.0);
    if cfg.master_volume > 0.0 {
        c.round_rect(bx, by, (bw * cfg.master_volume).max(6.0), 6.0, 3.0, rgba(0xEB452C, a), rgba(0, 0.0), 0.0);
    }
    let detail = format!("music {:.0}%   hitsounds {:.0}%", cfg.music_volume * 100.0, cfg.fx_volume * 100.0);
    c.text(x + 12.0, y + 40.0, 7.0, rgba(0x9a9ab0, a), &detail);
}

/// When each map was last played in taiko-rs (`plays.tsv`: Unix seconds, tab, path).
fn load_plays() -> std::collections::HashMap<PathBuf, u64> {
    let mut out = std::collections::HashMap::new();
    let Some(text) = config::user_dir(false).and_then(|d| std::fs::read_to_string(d.join("plays.tsv")).ok()) else { return out };
    for line in text.lines() {
        if let Some((t, path)) = line.split_once('\t') {
            if let Ok(t) = t.parse::<u64>() {
                let slot = out.entry(PathBuf::from(path)).or_insert(0);
                *slot = (*slot).max(t);
            }
        }
    }
    out
}

/// The library entry for a lobby map: same artist/title/difficulty, then the exact file hash.
/// Only those few candidates are hashed, so this stays cheap with a huge library.
/// Where downloaded maps go: the first Songs folder being scanned, else taiko-rs's own (which
/// is scanned at startup when it exists).
fn download_dir(inputs: &[PathBuf]) -> PathBuf {
    if let Some(dir) = inputs.iter().find(|p| p.is_dir()) {
        return dir.clone();
    }
    #[cfg(windows)]
    let own = config::user_dir(false).map(|d| d.join("Songs"));
    #[cfg(not(windows))]
    let own = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("taiko-rs").join("Songs"));
    own.unwrap_or_else(|| PathBuf::from("Songs"))
}

/// Marks the fullscreen preview as a preview: a blue frame and badge on the playfield area, and
/// a header with the map and the controls.
fn draw_preview_chrome(c: &mut Canvas, cfg: &Config, e: &Entry, stars: Option<f32>, lobby: bool, size_label: &str) {
    let blue = preview::MARKER;
    let a = cfg.playfield_area;
    let (ax, ay, aw, ah) = (a.x * c.width, a.y * render::VIRTUAL_HEIGHT, a.w * c.width, a.h * render::VIRTUAL_HEIGHT);
    c.round_rect(ax + 1.0, ay + 1.0, aw - 2.0, ah - 2.0, 4.0, rgba(0, 0.0), rgba(blue, 0.55), 1.5);
    let badge = "AUTOPLAY PREVIEW";
    let bw = Canvas::text_width(7.0, badge) + 16.0;
    // Below the header when the area reaches the top of the screen.
    let by = (ay + 8.0).max(52.0);
    c.round_rect(ax + 8.0, by, bw, 16.0, 8.0, rgba(blue, 0.85), rgba(0, 0.0), 0.0);
    c.text(ax + 16.0, by + 4.5, 7.0, rgba(0x0c1a28, 1.0), badge);

    c.round_rect(24.0, 10.0, c.width - 48.0, 34.0, 8.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
    c.round_rect(32.0, 18.0, 62.0, 18.0, 9.0, rgba(0x173049, 1.0), rgba(blue, 1.0), 1.0);
    c.text(41.0, 23.0, 8.0, rgba(0xffffff, 1.0), "PREVIEW");
    let pill = stars::draw_pill(c, c.width - 36.0, 22.0, 8.0, stars);
    let back = if lobby { "Lobby  (Esc)" } else { "Back  (Esc)" };
    let bw = Canvas::text_width(8.0, back) + 24.0;
    let bx = c.width - 44.0 - pill - bw;
    c.button(bx, 17.0, bw, 20.0, back, 8.0, render::Hit::Key(KeyCode::Escape), false);
    let presets = format!("{size_label}  (F5/F6)");
    let pw = Canvas::text_width(8.0, &presets) + 24.0;
    let px = bx - 8.0 - pw;
    c.button(px, 17.0, pw, 20.0, &presets, 8.0, render::Hit::Presets, false);
    let hints = if lobby { "drag / wheel: playfield" } else { "Enter play   drag / wheel: playfield" };
    let hw = Canvas::text_width(7.0, hints) + bw + pw + 20.0;
    c.text_right(px - 12.0, 24.0, 7.0, rgba(0x8a8aa0, 1.0), hints);
    let room = c.width - 48.0 - 110.0 - hw - pill - 24.0;
    let mut title = format!("{} - {} [{}]", e.meta.artist, e.meta.title, e.meta.version);
    let max = (room / (10.0 / 7.0 * 6.0)).max(4.0) as usize;
    if title.chars().count() > max {
        title = title.chars().take(max.saturating_sub(3)).collect::<String>() + "...";
    }
    c.text(104.0, 22.0, 10.0, rgba(0xffffff, 1.0), &title);
}

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

    /// Scenes that use the gameplay present mode; everything else is a menu.
    fn in_song(&self) -> bool {
        matches!(self.scene, Scene::Play { .. } | Scene::MpPlay { .. } | Scene::MpWait { .. } | Scene::Calibrate { .. })
    }

    /// The present mode this frame should use.
    fn wanted_present(&self) -> PresentChoice {
        if self.cfg.menu_vsync && !self.in_song() { PresentChoice::Fifo } else { self.cfg.present }
    }

    fn stop_audio(&mut self) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        self.preview.invalidate_audio();
    }

    /// The preview's decoded song, when it was prepared at the speed `entry` is about to play at.
    fn cached_track(&self, entry: usize, rate: f64) -> Option<Arc<Track>> {
        self.preview.cached_track(&self.library[entry], rate, self.cfg.rate_keep_pitch)
    }

    /// Speed the preview plays at: the match rate for the lobby map, else the selected rate.
    fn preview_rate(&self) -> (f64, bool) {
        let lobby_rate = self.mp.as_ref().and_then(|s| s.map.as_ref().filter(|_| s.map_entry.is_some() && s.map_entry == self.preview_target()).map(|m| m.rate));
        let lobby = matches!(self.scene, Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults)
            || matches!(&self.scene, Scene::Preview { back } | Scene::Settings { prev: back } if matches!(**back, Scene::Lobby));
        let rate = lobby_rate.filter(|_| lobby).unwrap_or(self.cfg.rate);
        (rate, self.cfg.rate_keep_pitch)
    }

    /// "1280x720 (full screen)": the playfield's size in pixels.
    fn playfield_label(&self) -> String {
        let a = self.cfg.playfield_area;
        let (pw, ph) = ((a.w * self.window_w).round() as u32, (a.h * self.window_h).round() as u32);
        let kind = if a.is_full() {
            " (full screen)"
        } else if self.current_preset().is_some() {
            ""
        } else {
            " (custom)"
        };
        format!("{pw}x{ph}{kind}")
    }

    fn start_loading(&mut self, entry: usize) {
        self.start_loading_from(entry, None);
    }

    fn start_loading_from(&mut self, entry: usize, from: Option<f64>) {
        self.record_play(entry);
        self.stop_audio();
        self.preview.show_background_for(&self.library[entry]);
        let mut label = self.library[entry].label();
        if let Some(t) = from {
            label.push_str(&format!("  (from {})", fmt_time(t)));
        }
        let rx = self.spawn_loader(entry, self.cfg.rate, self.cached_track(entry, self.cfg.rate));
        self.scene = Scene::Loading { rx, label, entry, from };
    }

    /// The map the preview shows: the lobby map in the lobby (and its preview), else the song
    /// select selection.
    fn preview_target(&self) -> Option<usize> {
        let lobby = match &self.scene {
            Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults => true,
            Scene::Preview { back } | Scene::Settings { prev: back } => matches!(**back, Scene::Lobby),
            _ => false,
        };
        let lobby_map = self.mp.as_ref().and_then(|s| s.map_entry).filter(|_| lobby);
        lobby_map.or_else(|| self.select.selected())
    }

    fn open_preview(&mut self) {
        if self.preview_target().is_none() {
            return;
        }
        let back = std::mem::replace(&mut self.scene, Scene::Select);
        self.scene = Scene::Preview { back: Box::new(back) };
    }

    fn close_preview(&mut self) {
        if let Scene::Preview { back } = std::mem::replace(&mut self.scene, Scene::Select) {
            self.scene = *back;
        }
    }

    fn previewing_lobby(&self) -> bool {
        matches!(&self.scene, Scene::Preview { back } if matches!(**back, Scene::Lobby))
    }

    /// Host, in the lobby preview: the picked start point becomes the match's.
    fn set_match_start(&mut self) {
        let Some(s) = &mut self.mp else { return };
        if !s.is_host() || s.in_match {
            return;
        }
        let (Some(i), Some(mut m)) = (s.map_entry, s.map.clone()) else { return };
        m.from = self.preview.marker(&self.library[i]);
        let text = match m.from {
            Some(t) => format!("the match starts at {}", fmt_time(t)),
            None => "the match starts at the beginning".to_string(),
        };
        s.client.send(net::ClientMsg::SelectMap(m));
        self.toast(text);
    }

    /// The preview's button: play from the start point (or the match's start point).
    fn preview_button(&mut self) {
        if self.previewing_lobby() {
            self.set_match_start();
        } else if self.picking_lobby_map {
            if let Some(i) = self.select.selected() {
                self.pick_lobby_map(i);
            }
        } else {
            self.play_from_marker();
        }
    }

    fn preview_key(&mut self, code: KeyCode, repeat: bool, window: Option<&Window>) {
        if matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
            let step = if code == KeyCode::ArrowRight { 5000.0 } else { -5000.0 };
            self.preview.seek_by(step, self.audio.as_ref());
            return;
        }
        if repeat || self.handle_global_key(code, window) {
            return;
        }
        match code {
            KeyCode::Escape | KeyCode::Backspace => self.close_preview(),
            KeyCode::Enter | KeyCode::NumpadEnter if self.modifiers_ctrl => self.close_preview(),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let marked = self.preview_target().is_some_and(|i| self.preview.marker(&self.library[i]).is_some());
                if marked || self.previewing_lobby() || self.picking_lobby_map {
                    self.preview_button();
                } else if let Some(i) = self.select.selected() {
                    self.start_loading(i);
                }
            }
            _ => {}
        }
    }

    /// Plays the selected map from the start point picked on its timeline.
    fn play_from_marker(&mut self) {
        let Some(i) = self.select.selected() else { return };
        if let Some(t) = self.preview.marker(&self.library[i]) {
            self.start_loading_from(i, Some(t));
        }
    }

    fn apply_volume(&self) {
        if let Some(a) = &self.audio {
            let (music, fx) = self.cfg.volumes();
            a.set_volume(music, fx);
        }
    }

    /// Parses the map and decodes its audio on a background thread.
    /// `speed`: rate modifier to prepare the track for (time-stretched or resampled once here).
    fn spawn_loader(&self, entry: usize, speed: f64, cached: Option<Arc<Track>>) -> Receiver<Loaded> {
        let path = self.library[entry].path.clone();
        let rate = self.audio.as_ref().map(|a| a.rate);
        let keep_pitch = self.cfg.rate_keep_pitch;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let map = beatmap::load(&path)?;
                let audio_path = path.parent().unwrap_or(&path).join(&map.meta.audio_file);
                let track = match (cached, rate) {
                    (Some(track), _) => track,
                    (None, Some(rate)) => Arc::new(audio::change_rate(audio::decode(&audio_path, rate)?, rate, speed, keep_pitch)),
                    (None, None) => Arc::new(Track { samples: Vec::new(), rate: 1.0 }),
                };
                Ok((map, track))
            })();
            let _ = tx.send(result);
        });
        rx
    }

    fn to_select(&mut self) {
        self.stop_audio();
        self.report_smoothness();
        self.scene = Scene::Select;
    }

    fn report_smoothness(&mut self) {
        if let (Some(sm), Scene::Play { .. }) = (&mut self.smooth, &self.scene) {
            eprintln!("--- smoothness ({}) ---\n{}", self.present.name(), sm.report());
            *sm = debug::Smoothness::default();
        }
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
            Item::PresetKey(i) => config::key_name(c.preset_keys[i]),
            Item::ShowPlayfieldSize => on(c.show_playfield_size),
            Item::KiaiEffects => on(c.kiai_effects),
            Item::AntiMash => on(c.anti_mash),
            Item::ShowConverts => on(self.select.show_converts),
            Item::MasterVolume => format!("{:.0}%", c.master_volume * 100.0),
            Item::MusicVolume => format!("{:.0}%", c.music_volume * 100.0),
            Item::FxVolume => format!("{:.0}%", c.fx_volume * 100.0),
            Item::AudioBuffer => {
                let rate = self.audio.as_ref().map(|a| a.rate).unwrap_or(48000);
                let ms = c.audio_buffer as f64 / rate as f64 * 1000.0;
                format!("{} frames ({ms:.1} ms)", c.audio_buffer)
            }
            Item::PresentMode => c.present.name().to_string(),
            Item::MenuVsync => on(c.menu_vsync),
            Item::FpsCap => {
                if c.fps_cap == 0 {
                    "unlimited".into()
                } else {
                    c.fps_cap.to_string()
                }
            }
            Item::ShowFps => on(c.show_fps),
            Item::Animations => on(c.animations),
            Item::Fullscreen => on(self.fullscreen),
            Item::Calibrate => "Enter to start".into(),
            Item::CalibrationBpm => format!("{:.0} BPM", c.calibration_bpm),
            Item::LiveOffset => on(c.show_offset_hint),
            Item::LiveOffsetUr => on(c.offset_hint_ur_scaled),
            Item::ResumeDelay if c.resume_delay_ms <= 0.0 => "off".into(),
            Item::ResumeDelay => format!("{:.2} s", c.resume_delay_ms / 1000.0),
            Item::PlayfieldArea => c.playfield_area.describe(),
            Item::PlayfieldPreset => self.current_preset().map(|(w, h)| format!("{w}x{h}")).unwrap_or_else(|| "custom".into()),
            Item::HudPosition => if c.hud_in_playfield { "with playfield".into() } else { "screen".into() },
            Item::BackgroundDim => format!("{:.0}%", c.bg_dim * 100.0),
            Item::PlayfieldReset => "Enter to reset".into(),
            Item::Rate => format!("{:.2}x", c.rate),
            Item::RateKeepPitch => on(c.rate_keep_pitch),
            Item::Skin if c.skin.is_empty() => "built-in".into(),
            Item::Skin if c.skin.chars().count() > 24 => c.skin.chars().take(21).collect::<String>() + "...",
            Item::Skin => c.skin.clone(),
            Item::SkinFolder => "Enter to open".into(),
        }
    }

    /// Changes a setting one step in `dir` (-1 / +1), applying it immediately.
    fn adjust_setting(&mut self, item: Item, dir: i32, window: Option<&Window>) {
        let d = dir as f64;
        match item {
            Item::ScrollSpeed => self.cfg.scroll_speed = (self.cfg.scroll_speed + 0.05 * d).clamp(0.3, 4.0),
            Item::Offset => self.cfg.offset_ms += d * if self.modifiers_shift { 5.0 } else { 1.0 },
            Item::Key(_) | Item::PresetKey(_) => {
                self.settings_ui.binding = Some(item);
                return;
            }
            Item::ShowPlayfieldSize => self.cfg.show_playfield_size = !self.cfg.show_playfield_size,
            Item::KiaiEffects => self.cfg.kiai_effects = !self.cfg.kiai_effects,
            Item::AntiMash => self.cfg.anti_mash = !self.cfg.anti_mash,
            Item::ShowConverts => {
                self.select.show_converts = !self.select.show_converts;
                self.select.refilter(&self.library);
                return;
            }
            Item::MasterVolume => self.cfg.master_volume = (self.cfg.master_volume + 0.05 * dir as f32).clamp(0.0, 1.0),
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
                self.load_skin();
                self.refresh_status();
            }
            Item::PresentMode => {
                self.cfg.present = if dir > 0 { self.cfg.present.next() } else { self.cfg.present.next().next() };
            }
            Item::MenuVsync => self.cfg.menu_vsync = !self.cfg.menu_vsync,
            Item::FpsCap => {
                let i = FPS_CAPS.iter().position(|&c| c == self.cfg.fps_cap).unwrap_or(0) as i32;
                self.cfg.fps_cap = FPS_CAPS[(i + dir).rem_euclid(FPS_CAPS.len() as i32) as usize];
            }
            Item::ShowFps => self.cfg.show_fps = !self.cfg.show_fps,
            Item::Animations => self.cfg.animations = !self.cfg.animations,
            Item::Fullscreen => self.toggle_fullscreen(window),
            // Started with Enter in `settings_key`; arrows do nothing.
            Item::Calibrate => return,
            Item::LiveOffset => self.cfg.show_offset_hint = !self.cfg.show_offset_hint,
            Item::LiveOffsetUr => self.cfg.offset_hint_ur_scaled = !self.cfg.offset_hint_ur_scaled,
            Item::ResumeDelay => self.cfg.resume_delay_ms = (self.cfg.resume_delay_ms + 250.0 * d).clamp(0.0, 3000.0),
            // Both act on Enter in `settings_key`.
            Item::PlayfieldArea | Item::PlayfieldReset => return,
            Item::PlayfieldPreset => {
                self.cycle_preset(dir);
                return;
            }
            Item::HudPosition => self.cfg.hud_in_playfield = !self.cfg.hud_in_playfield,
            Item::BackgroundDim => self.cfg.bg_dim = (((self.cfg.bg_dim + 0.05 * dir as f32) * 20.0).round() / 20.0).clamp(0.0, 1.0),
            Item::Rate => {
                let step = 0.05 * if self.modifiers_shift { 5.0 } else { 1.0 };
                self.cfg.rate = (((self.cfg.rate + step * d) * 100.0).round() / 100.0).clamp(RATE_RANGE.0, RATE_RANGE.1);
            }
            Item::RateKeepPitch => self.cfg.rate_keep_pitch = !self.cfg.rate_keep_pitch,
            Item::Skin => {
                let names: Vec<String> = std::iter::once(String::new()).chain(skin::list().into_iter().map(|(n, _)| n)).collect();
                let i = names.iter().position(|n| *n == self.cfg.skin).unwrap_or(0) as i32;
                self.cfg.skin = names[(i + dir).rem_euclid(names.len() as i32) as usize].clone();
                self.load_skin();
            }
            // Acts on Enter in `settings_key`.
            Item::SkinFolder => return,
            Item::CalibrationBpm => {
                let (lo, hi) = calibrate::BPM_RANGE;
                self.cfg.calibration_bpm = (self.cfg.calibration_bpm + 5.0 * d).clamp(lo, hi);
            }
        }
        self.apply_volume();
        if let (Some(a), Item::FxVolume | Item::Skin) = (&self.audio, item) {
            a.sfx(audio::Sfx::Don);
        }
        self.cfg.save();
    }

    /// Loads the configured skin: its note images go to the renderer, its hitsounds to the audio
    /// engine.
    fn load_skin(&mut self) {
        let skin = skin::load(&self.cfg.skin, self.audio.as_ref().map(|a| a.rate));
        self.sprites_upload = Some(skin.notes);
        if let Some(a) = &self.audio {
            let h = skin.hitsounds;
            a.set_hitsounds(h.don, h.kat, h.finish);
        }
    }

    fn open_skins_folder(&mut self) {
        let Some(dir) = skin::own_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
        if std::process::Command::new(opener).arg(&dir).spawn().is_err() {
            self.toast(format!("skins go in {}", dir.display()));
        }
    }

    fn toggle_fullscreen(&mut self, window: Option<&Window>) {
        if let Some(w) = window {
            self.fullscreen = w.fullscreen().is_none();
            w.set_fullscreen(self.fullscreen.then_some(Fullscreen::Borderless(None)));
            self.cfg.fullscreen = self.fullscreen;
            self.cfg.save();
        }
    }

    fn settings_key(&mut self, code: KeyCode, window: Option<&Window>) {
        if let Some(item) = self.settings_ui.binding.take() {
            if code != KeyCode::Escape {
                match item {
                    Item::Key(slot) => {
                        if self.cfg.preset_keys.contains(&code) {
                            self.toast(format!("{} is a playfield preset key", config::key_name(code)));
                            return;
                        }
                        // Swap if the key was already bound to another drum.
                        if let Some(other) = self.cfg.keys.iter().position(|&k| k == code) {
                            self.cfg.keys[other] = self.cfg.keys[slot];
                        }
                        self.cfg.keys[slot] = code;
                    }
                    Item::PresetKey(slot) => {
                        if self.cfg.keys.contains(&code) {
                            self.toast(format!("{} is a drum key", config::key_name(code)));
                            return;
                        }
                        if let Some(other) = self.cfg.preset_keys.iter().position(|&k| k == code) {
                            self.cfg.preset_keys[other] = self.cfg.preset_keys[slot];
                        }
                        self.cfg.preset_keys[slot] = code;
                    }
                    _ => {}
                }
                self.cfg.save();
            }
            return;
        }
        if self.settings_ui.dropdown.is_some() {
            self.dropdown_key(code);
            return;
        }
        let item = self.settings_ui.item();
        match code {
            KeyCode::ArrowUp => self.settings_ui.move_cursor(-1),
            KeyCode::ArrowDown => self.settings_ui.move_cursor(1),
            KeyCode::PageUp | KeyCode::PageDown => {
                let n = settings::SECTIONS.len() as i32;
                let dir = if code == KeyCode::PageDown { 1 } else { -1 };
                let next = (self.settings_ui.section() as i32 + dir).rem_euclid(n) as usize;
                self.settings_ui.cursor = settings::section_start(next);
            }
            KeyCode::ArrowLeft => self.adjust_setting(item, -1, window),
            KeyCode::ArrowRight => self.adjust_setting(item, 1, window),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Calibrate => self.start_calibration(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::PlayfieldArea => self.open_layout_editor(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::PlayfieldPreset => self.open_dropdown(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::SkinFolder => self.open_skins_folder(),
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
        self.stop_audio();
        match calibrate::estimate(&game.stats.errors, self.cfg.offset_ms) {
            Some(estimate) if estimate.hits >= 4 => self.scene = Scene::CalibrateDone { estimate, back },
            _ => {
                self.scene = *back;
                self.toast("calibration needs more hits".into());
            }
        }
    }

    fn leave_calibration(&mut self, apply: bool) {
        self.stop_audio();
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
        let Scene::Play { game, entry, from } = &mut self.scene else { return };
        if game.resuming.is_some() {
            // Counting down: Esc goes back to the menu, everything else waits.
            if code == KeyCode::Escape {
                game.cancel_resume();
            }
            return;
        }
        let delay = Duration::from_millis(self.cfg.resume_delay_ms as u64);
        let cursor = game.paused.unwrap_or(0);
        match code {
            KeyCode::ArrowUp => game.paused = Some(cursor.saturating_sub(1)),
            KeyCode::ArrowDown => game.paused = Some((cursor + 1).min(PAUSE_ITEMS.len() - 1)),
            KeyCode::Escape => game.resume_after(delay, self.audio.as_ref()),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => match cursor {
                0 => game.resume_after(delay, self.audio.as_ref()),
                1 => {
                    let (e, f) = (*entry, *from);
                    self.start_loading_from(e, f);
                }
                2 => self.open_dropdown(),
                3 => self.open_settings(),
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
                self.cfg.present = self.cfg.present.next();
                self.cfg.save();
                let note = if self.cfg.menu_vsync && !self.in_song() { " (menus stay on vsync)" } else { "" };
                let text = format!("gameplay present mode: {}{note}", self.cfg.present.name());
                self.toast(text);
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
        matches!(self.scene, Scene::Play { .. } | Scene::MpPlay { .. } | Scene::Calibrate { .. } | Scene::LayoutEdit { .. } | Scene::Preview { .. })
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

    fn screen_size(&self) -> (u32, u32) {
        (self.window_w as u32, self.window_h as u32)
    }

    fn preset_groups(&self) -> Vec<(&'static str, Vec<(u32, u32)>)> {
        layout::group_presets(&self.resolutions, self.screen_size())
    }

    /// Presets in dropdown order (grouped by aspect ratio, largest first).
    fn preset_list(&self) -> Vec<(u32, u32)> {
        self.preset_groups().into_iter().flat_map(|(_, l)| l).collect()
    }

    fn apply_preset(&mut self, (w, h): (u32, u32)) {
        self.cfg.playfield_area = layout::preset_area(w, h, self.window_w, self.window_h);
        self.cfg.save();
        self.toast(format!("playfield {w}x{h}"));
    }

    /// Steps through resolution presets in dropdown order, applying each as a centred area.
    fn cycle_preset(&mut self, dir: i32) {
        let list = self.preset_list();
        if list.is_empty() {
            self.toast("no smaller resolutions fit this window".into());
            return;
        }
        let n = list.len() as i32;
        let i = match self.current_preset().and_then(|p| list.iter().position(|&r| r == p)) {
            Some(i) => (i as i32 + dir).rem_euclid(n),
            None => if dir > 0 { 0 } else { n - 1 },
        } as usize;
        self.apply_preset(list[i]);
    }

    fn open_dropdown(&mut self) {
        let list = self.preset_list();
        if list.is_empty() {
            self.toast("no smaller resolutions fit this window".into());
            return;
        }
        let cursor = self.current_preset().and_then(|p| list.iter().position(|&r| r == p)).unwrap_or(0);
        self.settings_ui.dropdown = Some(Dropdown::new(cursor));
    }

    fn dropdown_key(&mut self, code: KeyCode) {
        let list = self.preset_list();
        let n = list.len();
        let Some(dd) = &mut self.settings_ui.dropdown else { return };
        match code {
            KeyCode::ArrowUp => dd.cursor = dd.cursor.saturating_sub(1),
            KeyCode::ArrowDown => dd.cursor = (dd.cursor + 1).min(n.saturating_sub(1)),
            KeyCode::PageUp => dd.cursor = dd.cursor.saturating_sub(8),
            KeyCode::PageDown => dd.cursor = (dd.cursor + 8).min(n.saturating_sub(1)),
            KeyCode::Home => dd.cursor = 0,
            KeyCode::End => dd.cursor = n.saturating_sub(1),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                let pick = list.get(dd.cursor).copied();
                self.settings_ui.dropdown = None;
                if let Some(p) = pick {
                    self.apply_preset(p);
                }
            }
            KeyCode::Escape | KeyCode::Backspace | KeyCode::Tab => self.settings_ui.dropdown = None,
            _ => {}
        }
    }

    /// Pointer position in virtual units (480 tall).
    fn mouse_virtual(&self) -> (f32, f32) {
        let k = render::VIRTUAL_HEIGHT / self.window_h.max(1.0);
        (self.mouse.0 as f32 * k, self.mouse.1 as f32 * k)
    }

    /// Mouse wheel: scales the playfield area in game / in the editor, scrolls lists elsewhere.
    fn wheel(&mut self, notches: i32) {
        if notches == 0 {
            return;
        }
        if self.modifiers_alt {
            let v = ((self.cfg.master_volume + 0.05 * notches as f32) * 20.0).round() / 20.0;
            self.cfg.master_volume = v.clamp(0.0, 1.0);
            self.apply_volume();
            self.cfg.save();
            self.volume_shown = Some(Instant::now());
            return;
        }
        let n = self.preset_list().len();
        if let Some(dd) = &mut self.settings_ui.dropdown {
            dd.cursor = (dd.cursor as i32 - notches).clamp(0, n as i32 - 1) as usize;
            return;
        }
        if matches!(self.scene, Scene::Preview { .. }) {
            let (vx, vy) = self.mouse_virtual();
            if self.preview.time_at(vx, vy).is_some() {
                self.preview.seek_by(notches as f64 * 5000.0, self.audio.as_ref());
                return;
            }
        }
        if self.area_editable() {
            self.cfg.playfield_area = self.cfg.playfield_area.scaled(1.05f32.powi(notches));
            self.cfg.save();
            let text = format!("playfield: {}", self.cfg.playfield_area.describe());
            self.toast(text);
            return;
        }
        let n = self.preset_list().len();
        if let Some(dd) = &mut self.settings_ui.dropdown {
            dd.cursor = (dd.cursor as i32 - notches).clamp(0, n as i32 - 1) as usize;
            return;
        }
        let (vx, vy) = self.mouse_virtual();
        match &self.scene {
            Scene::Select if self.preview.time_at(vx, vy).is_some() => self.preview.seek_by(notches as f64 * 5000.0, self.audio.as_ref()),
            Scene::Select if self.select.picker.is_some() => {
                let n = self.select.picker_len() as i32;
                self.select.picker = self.select.picker.map(|c| (c as i32 - notches).clamp(0, n - 1) as usize);
            }
            Scene::Select => self.select.move_by(-notches),
            Scene::Download if self.preview.time_at(vx, vy).is_some() => self.preview.seek_by(notches as f64 * 5000.0, self.audio.as_ref()),
            Scene::Download => self.browse.move_by(-notches),
            Scene::Settings { .. } if self.settings_ui.dropdown.is_some() => {
                if let Some(dd) = &mut self.settings_ui.dropdown {
                    dd.cursor = (dd.cursor as i32 - notches).clamp(0, n as i32 - 1) as usize;
                }
            }
            Scene::Settings { .. } => self.settings_ui.move_cursor(-notches),
            _ => {}
        }
    }

    /// Pointer moved: drags the area if a drag is active, otherwise updates the hover state.
    /// Returns the cursor to show.
    fn mouse_moved(&mut self, x: f64, y: f64) -> CursorIcon {
        self.mouse = (x, y);
        let (sw, sh) = (self.window_w.max(1.0) as f64, self.window_h.max(1.0) as f64);
        let (vx, vy) = self.mouse_virtual();
        if let Some(dd) = &mut self.settings_ui.dropdown {
            if let Some(i) = dd.row_at(vx, vy) {
                dd.cursor = i;
                return CursorIcon::Pointer;
            }
            return CursorIcon::Default;
        }
        if self.grip.is_none() && self.readout_rect.is_some_and(|r| vx >= r[0] && vx <= r[0] + r[2] && vy >= r[1] && vy <= r[1] + r[3]) {
            self.hover = None;
            return CursorIcon::Pointer;
        }
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
        if self.scrubbing.is_none() {
            if let Some(hit) = self.hit_at(vx, vy) {
                if let (render::Hit::PauseItem(i), Scene::Play { game, .. }) = (hit, &mut self.scene) {
                    game.paused = Some(i);
                }
                self.hover = None;
                return CursorIcon::Pointer;
            }
        }
        if matches!(self.scene, Scene::Select | Scene::Preview { .. } | Scene::Download) {
            if let Some(last) = self.scrubbing {
                // Seeking re-decodes nothing, but limit it to ~30 per second while dragging.
                if last.elapsed() >= Duration::from_millis(33) {
                    if let Some(t) = self.preview.time_at(vx, vy) {
                        self.preview.seek(t, self.audio.as_ref());
                    }
                    self.scrubbing = Some(Instant::now());
                }
                return CursorIcon::Pointer;
            }
            let on_box = matches!(self.scene, Scene::Select) && self.preview.box_at(vx, vy);
            if self.preview.time_at(vx, vy).is_some() || self.preview.play_button_at(vx, vy) || on_box {
                self.hover = None;
                return CursorIcon::Pointer;
            }
        }
        if !self.area_editable() {
            self.hover = None;
            let (vx, vy) = self.mouse_virtual();
            if let Some(dd) = &mut self.settings_ui.dropdown {
                if let Some(i) = dd.row_at(vx, vy) {
                    dd.cursor = i;
                    return CursorIcon::Pointer;
                }
            }
            return CursorIcon::Default;
        }
        self.hover = layout::hit(self.cfg.playfield_area, x, y, sw, sh);
        self.hover.map(|g| g.cursor()).unwrap_or(CursorIcon::Default)
    }

    fn mouse_button(&mut self, pressed: bool, window: Option<&Window>) {
        if matches!(self.scene, Scene::Select | Scene::Preview { .. } | Scene::Download) {
            let (vx, vy) = self.mouse_virtual();
            if pressed {
                if self.preview.play_button_at(vx, vy) {
                    if matches!(self.scene, Scene::Preview { .. }) {
                        self.preview_button();
                    } else if !self.picking_lobby_map {
                        self.play_from_marker();
                    }
                    return;
                }
                if matches!(self.scene, Scene::Select) && self.preview.box_at(vx, vy) {
                    self.open_preview();
                    return;
                }
                if let Some(t) = self.preview.time_at(vx, vy) {
                    self.preview.seek(t, self.audio.as_ref());
                    self.scrubbing = Some(Instant::now());
                    return;
                }
            } else if self.scrubbing.take().is_some() {
                // Land exactly where the pointer was released.
                if let Some(t) = self.preview.time_at(vx, vy) {
                    self.preview.seek(t, self.audio.as_ref());
                }
                return;
            }
        }
        if pressed && self.settings_ui.dropdown.is_none() {
            let (vx, vy) = self.mouse_virtual();
            if self.readout_rect.is_some_and(|r| vx >= r[0] && vx <= r[0] + r[2] && vy >= r[1] && vy <= r[1] + r[3]) {
                // Clicking the resolution readout opens the preset list; single-player pauses
                // first (a multiplayer match keeps running).
                if let Scene::Play { game, .. } | Scene::Calibrate { game, .. } = &mut self.scene {
                    game.pause(self.audio.as_ref());
                }
                self.open_dropdown();
                return;
            }
        }
        if pressed && self.settings_ui.dropdown.is_some() {
            let (vx, vy) = self.mouse_virtual();
            let dd = self.settings_ui.dropdown.as_ref().unwrap();
            let hit = dd.row_at(vx, vy);
            let inside = dd.contains(vx, vy);
            let pick = hit.and_then(|i| self.preset_list().get(i).copied());
            if hit.is_some() || !inside {
                self.settings_ui.dropdown = None;
            }
            if let Some(p) = pick {
                self.apply_preset(p);
            }
            return;
        }
        if pressed {
            let (vx, vy) = self.mouse_virtual();
            let hit = self.hit_at(vx, vy);
            if self.select.picker.is_some() && matches!(self.scene, Scene::Select) && !matches!(hit, Some(render::Hit::PickRow(_))) {
                self.select.picker = None;
                return;
            }
            if let Some(hit) = hit {
                self.click(hit, window);
                return;
            }
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
        self.mouse_button(true, None);
        self.mouse_moved(start.0 + dx, start.1 + dy);
        if !hold {
            self.mouse_button(false, None);
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
            from: None,
            set_id: e.meta.set_id.or_else(|| e.path.parent().and_then(download::set_id_of_folder)),
        };
        if let Some(s) = &self.mp {
            s.client.send(net::ClientMsg::SelectMap(info));
        }
        self.scene = Scene::Lobby;
    }

    fn leave_room(&mut self, why: &str) {
        self.stop_audio();
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
                    KeyCode::KeyP if s.map_entry.is_some() => self.open_preview(),
                    KeyCode::KeyD if !s.have_map => self.download_lobby_map(),
                    KeyCode::KeyH if !s.have_map && s.map.is_some() && self.host_transfer.is_none() => {
                        s.client.send(net::ClientMsg::RequestMap);
                        self.toast("asking the host for the map...".into());
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
                        self.stop_audio();
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
        self.stars.clear();
        eprintln!("found {} playable difficulties", library.len());
        self.library = library;
        // Keep whatever was typed into the search box while scanning, and the sort.
        let filter = std::mem::take(&mut self.select.filter);
        let played = std::mem::take(&mut self.select.played);
        let sort = self.select.sort;
        self.select = Select::new(&self.library);
        self.select.filter = filter;
        self.select.played = played;
        self.select.sort = sort;
        let maps = self.library.iter().enumerate().map(|(i, e)| (i, e.path.clone(), e.modified, e.size)).collect();
        let info = mapinfo::MapInfo::spawn(maps, self.library.len());
        self.select.info = Some(info.results.clone());
        self.mapinfo = Some(info);
        self.mapinfo_sorted = false;
        let dirs = self.song_dirs.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(dirs.iter().find_map(|d| osudb::load(d)));
        });
        self.osudb_rx = Some(rx);
        self.select.refilter(&self.library);
        if self.play_when_scanned && !self.library.is_empty() {
            self.play_when_scanned = false;
            self.start_loading(0);
        }
        // A lobby map may have been announced before our library was ready.
        if let Some(s) = &mut self.mp {
            if let Some(m) = &s.map {
                s.map_entry = find_map(&self.library, m);
                s.have_map = s.map_entry.is_some();
                s.client.send(net::ClientMsg::HasMap(s.have_map));
            }
        }
    }

    /// Folds osu!'s collections and play history in once read, and re-sorts when the
    /// difficulty analysis finishes.
    fn poll_sorting(&mut self) {
        if let Some(rx) = &self.osudb_rx {
            if let Ok(db) = rx.try_recv() {
                self.osudb_rx = None;
                if let Some(db) = db {
                    self.apply_osu_db(db);
                }
            }
        }
        let Some(info) = &self.mapinfo else { return };
        let (done, total) = (info.done.load(Ordering::Relaxed), info.total.load(Ordering::Relaxed));
        let finished = info.finished();
        self.select.info_progress = if !finished && self.select.sort.needs_info() { format!("  |  analysing {done}/{total}") } else { String::new() };
        if finished && !self.mapinfo_sorted {
            self.mapinfo_sorted = true;
            if self.select.sort.needs_info() {
                self.select.refilter(&self.library);
            }
        }
    }

    fn apply_osu_db(&mut self, db: osudb::OsuDb) {
        let key = |folder: &str, file: &str| (folder.to_lowercase(), file.to_lowercase());
        let index: std::collections::HashMap<(String, String), usize> = self
            .library
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let folder = e.path.parent()?.file_name()?.to_str()?;
                let file = e.path.file_name()?.to_str()?;
                Some((key(folder, file), i))
            })
            .collect();
        let entry_of = |md5: &str| db.by_md5.get(md5).and_then(|m| index.get(&key(&m.folder, &m.file)).copied());
        for m in db.by_md5.values().filter(|m| m.last_played > 0) {
            if let Some(&i) = index.get(&key(&m.folder, &m.file)) {
                let t = self.select.played.entry(self.library[i].path.clone()).or_insert(0);
                *t = (*t).max(m.last_played);
            }
        }
        self.select.collections = db
            .collections
            .iter()
            .map(|(name, maps)| (name.clone(), maps.iter().filter_map(|h| entry_of(h)).collect::<std::collections::HashSet<usize>>()))
            .filter(|(_, set)| !set.is_empty())
            .collect();
        eprintln!("osu!.db: {} maps, {} collections with maps here", db.by_md5.len(), self.select.collections.len());
        self.select.refilter(&self.library);
    }

    /// Remembers that `entry` was just played (for "recently played").
    fn record_play(&mut self, entry: usize) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let path = self.library[entry].path.clone();
        if let Some(file) = config::user_dir(false).map(|d| d.join("plays.tsv")) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                let _ = writeln!(f, "{now}\t{}", path.display());
            }
        }
        self.select.played.insert(path, now);
    }

    fn cycle_sort(&mut self, dir: i32) {
        self.select.cycle_sort(dir, &self.library);
        self.cfg.sort = self.select.sort.name().to_string();
        self.cfg.save();
        self.toast(format!("sorted by {}", self.select.sort.name()));
    }

    fn open_collections(&mut self) {
        if self.select.collections.is_empty() {
            self.toast("no osu! collections found (collection.db next to your Songs folder)".into());
        } else {
            self.select.picker = Some(self.select.collection.map(|c| c + 1).unwrap_or(0));
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
                    s.map_entry = map.as_ref().and_then(|m| find_map(library, m));
                    s.have_map = s.map_entry.is_some();
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
                        self.stop_audio();
                        self.picking_lobby_map = false;
                        self.preview.show_background_for(&self.library[i]);
                        let rx = self.spawn_loader(i, map.rate, self.cached_track(i, map.rate));
                        self.scene = Scene::MpWait { rx: Some(rx), loaded: None, start: None, from: map.from };
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
            ServerMsg::MapRequested { from } => self.send_map_to(from),
            ServerMsg::MapFile(part) => self.receive_map_part(part),
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
                    self.stop_audio();
                    self.mp_status = why;
                    self.scene = Scene::MpMenu;
                }
            }
            ServerMsg::Welcome { .. } | ServerMsg::Pong { .. } => {}
        }
    }

    fn key(&mut self, code: KeyCode, repeat: bool, text: Option<&str>, at: Instant, window: Option<&Window>) {
        // Key capture must see every key, before any shortcut can claim it.
        if self.settings_ui.dropdown.is_some() && !repeat {
            let nav = matches!(
                code,
                KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
                    | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space | KeyCode::Escape | KeyCode::Backspace | KeyCode::Tab
            );
            if nav {
                self.dropdown_key(code);
                return;
            }
        }
        if self.settings_ui.binding.is_some() && matches!(self.scene, Scene::Settings { .. }) {
            if !repeat {
                self.settings_key(code, window);
            }
            return;
        }
        if !repeat && !matches!(self.scene, Scene::MpMenu) && self.settings_ui.dropdown.is_none() {
            if let Some(i) = self.cfg.preset_keys.iter().position(|&k| k == code) {
                self.cycle_preset(if i == 0 { -1 } else { 1 });
                return;
            }
        }
        if matches!(self.scene, Scene::Calibrate { .. } | Scene::CalibrateDone { .. }) {
            self.calibration_key(code, repeat, at, window);
            return;
        }
        if matches!(self.scene, Scene::LayoutEdit { .. }) {
            self.layout_key(code);
            return;
        }
        if matches!(self.scene, Scene::Preview { .. }) {
            self.preview_key(code, repeat, window);
            return;
        }
        if matches!(self.scene, Scene::Download) {
            if !repeat || matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::Backspace | KeyCode::PageUp | KeyCode::PageDown) {
                self.download_key(code, text, window);
            }
            return;
        }
        if let (Scene::Select, Some(cursor)) = (&self.scene, self.select.picker) {
            let n = self.select.picker_len();
            match code {
                KeyCode::ArrowUp => self.select.picker = Some(cursor.saturating_sub(1)),
                KeyCode::ArrowDown => self.select.picker = Some((cursor + 1).min(n - 1)),
                KeyCode::PageUp => self.select.picker = Some(cursor.saturating_sub(10)),
                KeyCode::PageDown => self.select.picker = Some((cursor + 10).min(n - 1)),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.select.pick_collection(cursor, &self.library),
                KeyCode::Escape | KeyCode::Backspace => self.select.picker = None,
                _ => {}
            }
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
            Scene::Select if self.modifiers_shift && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) && !self.picking_lobby_map => {
                self.play_from_marker();
            }
            Scene::Select if self.modifiers_ctrl && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) => self.open_preview(),
            Scene::Select if self.modifiers_ctrl && code == KeyCode::KeyD => self.open_download(),
            Scene::Select if self.modifiers_ctrl && code == KeyCode::KeyS => {
                let dir = if self.modifiers_shift { -1 } else { 1 };
                self.cycle_sort(dir);
            }
            Scene::Select if self.modifiers_ctrl && code == KeyCode::KeyL => self.open_collections(),
            Scene::Select if code == KeyCode::F2 => {
                if self.modifiers_shift {
                    self.select.random_back();
                } else {
                    self.select.random(&self.library);
                }
            }
            Scene::Select if self.modifiers_ctrl && matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) => {
                let step = if code == KeyCode::ArrowRight { 5000.0 } else { -5000.0 };
                self.preview.seek_by(step, self.audio.as_ref());
            }
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
            Scene::Calibrate { .. } | Scene::CalibrateDone { .. } | Scene::LayoutEdit { .. } | Scene::Preview { .. } | Scene::Download => {}
            Scene::MpMenu | Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults => {}
            Scene::Play { game, entry, from } => match code {
                KeyCode::KeyO => self.apply_live_offset(),
                KeyCode::Escape => game.pause(self.audio.as_ref()),
                KeyCode::Space => game.skip(self.audio.as_ref(), self.cfg.offset_ms),
                KeyCode::Backquote => {
                    let (e, f) = (*entry, *from);
                    self.start_loading_from(e, f);
                }
                _ => {}
            },
            Scene::Results { entry, from, .. } => match code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let (e, f) = (*entry, *from);
                    self.start_loading_from(e, f);
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
        canvas.mouse = self.mouse_virtual();
        canvas.dt_ms = (dt as f32).clamp(0.0, 100.0);
        render::set_animations(self.cfg.animations);
        // Screen changes fade in.
        let kind = std::mem::discriminant(&self.scene);
        if self.scene_kind != Some(kind) {
            self.scene_kind = Some(kind);
            self.scene_since = now;
        }
        self.frame_clip = None;
        self.poll_scan();
        {
            // The Download tab previews remote maps through the same preview.
            let browsing = matches!(self.scene, Scene::Download);
            if browsing != self.preview_remote {
                self.preview_remote = browsing;
                self.preview.forget();
            }
            let in_select = matches!(self.scene, Scene::Select | Scene::Preview { .. } | Scene::Download);
            let in_menu = in_select
                || matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Select | Scene::Lobby))
                || matches!(self.scene, Scene::MpMenu | Scene::Lobby | Scene::Download);
            let (entries, selected) = if browsing { (&self.browse.remote, self.browse.shown) } else { (&self.library, self.preview_target()) };
            let rate = self.preview_rate();
            self.preview.kiai_effects = self.cfg.kiai_effects;
            self.preview.update(now, entries, selected, self.audio.as_ref(), in_menu, in_select, self.cfg.offset_ms, rate);
            self.stars.poll();
            if let (Scene::Preview { .. }, Some(i)) = (&self.scene, selected) {
                self.stars.request(i, &self.library[i].path, self.cfg.rate);
            }
            if matches!(self.scene, Scene::Select) {
                for i in self.select.selected_set() {
                    self.stars.request(i, &self.library[i].path, self.cfg.rate);
                }
                let (stars, rate) = (&self.stars, self.cfg.rate);
                self.select.sort_selected_set(&|i| stars.get(i, rate));
                self.select.remember_selection(&self.library);
            }
            let has_bg = self.preview.has_background;
            self.bg_brightness = match self.scene {
                _ if !has_bg => 0.0,
                Scene::Play { .. } | Scene::MpPlay { .. } | Scene::MpWait { .. } | Scene::Loading { .. } | Scene::Preview { .. } => 1.0 - self.cfg.bg_dim,
                Scene::Calibrate { .. } | Scene::CalibrateDone { .. } | Scene::LayoutEdit { .. } => 0.0,
                _ => 0.35,
            };

        }
        self.poll_multiplayer();
        self.poll_downloads();
        self.poll_sorting();
        self.browse.poll(now);
        let mut next: Option<Scene> = None;
        let mut capture: Option<String> = None;
        let preview_target = self.preview_target();
        let lobby_download = self.lobby_download_status();
        let lobby_host = self.mp.as_ref().is_some_and(|s| s.is_host() && !s.in_match);
        let size_label = format!("Playfield {}", self.playfield_label());
        let mut finish_calibration = false;
        let elapsed = now.duration_since(self.started).as_secs_f64() * 1000.0;
        if let Some(mut script) = self.input_script.take() {
            for action in script.due(elapsed) {
                match action {
                    debug::Action::Key(code) => self.key(code, false, None, now, None),
                    debug::Action::Wheel(n) => self.wheel(n),
                    debug::Action::Drag { corner, dx, dy, hold } => self.simulate_drag(corner, dx, dy, hold),
                    debug::Action::Release => self.mouse_button(false, None),
                    debug::Action::ClickGraph(f) => {
                        if let Some([gx, gy, gw, gh]) = self.preview.graph_rect() {
                            // Virtual units back to pixels, then a real press/release.
                            let k = self.window_h as f64 / render::VIRTUAL_HEIGHT as f64;
                            self.mouse = (((gx + gw * f) as f64) * k, ((gy + gh / 2.0) as f64) * k);
                            self.mouse_button(true, None);
                            self.mouse_button(false, None);
                        }
                    }
                    debug::Action::SeekBy(ms) => self.preview.seek_by(ms, self.audio.as_ref()),
                    debug::Action::Click(x, y) => {
                        let k = self.window_h as f64 / render::VIRTUAL_HEIGHT as f64;
                        self.mouse_moved(x as f64 * k, y as f64 * k);
                        self.mouse_button(true, None);
                        self.mouse_button(false, None);
                    }
                    debug::Action::CtrlKey(code) => {
                        self.modifiers_ctrl = true;
                        self.key(code, false, None, now, None);
                        self.modifiers_ctrl = false;
                    }
                    debug::Action::Type(text) => {
                        for ch in text.chars() {
                            self.key(KeyCode::KeyA, false, Some(&ch.to_string()), now, None);
                        }
                    }
                    debug::Action::ShiftKey(code) => {
                        self.modifiers_shift = true;
                        self.key(code, false, None, now, None);
                        self.modifiers_shift = false;
                    }
                    debug::Action::AltWheel(n) => {
                        self.modifiers_alt = true;
                        self.wheel(n);
                        self.modifiers_alt = false;
                    }
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
                let (stars, rate) = (&self.stars, self.cfg.rate);
                self.select.draw(canvas, &self.library, dt, &self.status, &|i| stars.get(i, rate));
                let x = 24.0 + Select::list_width(canvas.width) + 12.0;
                let pw = canvas.width - x - 24.0;
                let selected_stars = self.select.selected().and_then(|i| self.stars.get(i, rate));
                if pw > 160.0 {
                    if let Some(clip) = self.preview.draw_panel(canvas, &self.library, x, 54.0, pw, 380.0, self.cfg.scroll_speed, selected_stars, Some(("Play from", "Shift+Enter"))) {
                        self.frame_clip = Some(clip);
                    }
                }
                if self.picking_lobby_map {
                    let w = canvas.width;
                    canvas.round_rect(24.0, 438.0, w - 48.0, 34.0, 8.0, rgba(0x2b1a17, 1.0), rgba(0xEB452C, 1.0), 1.5);
                    canvas.text_centered(w / 2.0, 450.0, 10.0, rgba(0xffffff, 1.0), "Picking the lobby map: Enter to choose, Esc to cancel");
                }
            }
            Scene::Loading { rx, label, entry, from } => {
                canvas.text_centered(canvas.width / 2.0, 200.0, 14.0, rgba(0xffffff, 1.0), "Loading");
                canvas.text_centered(canvas.width / 2.0, 226.0, 10.0, rgba(0x9a9ab0, 1.0), label);
                match rx.try_recv() {
                    Ok(Ok((map, track))) => {
                        let mut game = Game::new(map, track, self.audio.as_ref());
                        if let Some(t) = *from {
                            game.start_from(t, self.audio.as_ref(), self.cfg.offset_ms);
                        }
                        next = Some(Scene::Play { game: Box::new(game), entry: *entry, from: *from });
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
            Scene::Play { game, entry, from } => {
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
                game.kiai_effects = self.cfg.kiai_effects;
                game.anti_mash = self.cfg.anti_mash;
                game.ur_scaled_hint = self.cfg.offset_hint_ur_scaled;
                let hud_in = self.cfg.hud_in_playfield;
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, self.bg_brightness > 0.0, |c| {
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
                    let part = from.map(|t| format!("  from {}", fmt_time(t))).unwrap_or_default();
                    let title = format!("{} - {} [{}]{rate}{part}", m.artist, m.title, m.version);
                    next = Some(Scene::Results {
                        results: Results { shown: Instant::now(), title, stats: game.stats.clone(), windows: game.windows },
                        entry: *entry,
                        from: *from,
                    });
                }
            }
            Scene::Results { results, .. } => results.draw(canvas),
            Scene::Download => {
                let installed = self.installed_sets.take().unwrap_or_default();
                let dir = self.songs_dir.display().to_string();
                let lw = Select::list_width(canvas.width);
                self.browse.draw(canvas, &self.downloads, &installed, &dir, lw);
                self.installed_sets = Some(installed);
                // The selected set's preview, like song select's.
                let x = 24.0 + lw + 12.0;
                let pw = canvas.width - x - 24.0;
                let stars = self.browse.selected_diff().map(|d| d.stars);
                if pw > 160.0 && self.browse.selected().is_some() {
                    match self.preview.draw_panel(canvas, &self.browse.remote, x, 54.0, pw, 380.0, self.cfg.scroll_speed, stars, None) {
                        Some(clip) => self.frame_clip = Some(clip),
                        None if self.browse.shown.is_none() => {
                            canvas.text_centered(x + pw / 2.0, 160.0, 8.0, rgba(0x8a8aa0, 1.0), "loading preview...");
                        }
                        None => {}
                    }
                }
            }
            Scene::Preview { back } => {
                let lobby = matches!(**back, Scene::Lobby);
                match preview_target {
                    Some(i) => {
                        let scroll = self.cfg.scroll_speed;
                        let see_through = self.bg_brightness > 0.0;
                        let kiai = self.cfg.kiai_effects;
                        if let Some(game) = self.preview.game_for(&self.library, i) {
                            game.kiai_effects = kiai;
                            draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, see_through, |c| game.draw_playfield(c, scroll));
                        }
                        let e = &self.library[i];
                        let stars = self.stars.get(i, self.cfg.rate);
                        draw_preview_chrome(canvas, &self.cfg, e, stars, lobby, &size_label);
                        let (x, y, w) = (24.0, 394.0, canvas.width - 48.0);
                        canvas.round_rect(x, y, w, 78.0, 8.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
                        let button = match (lobby, lobby_host) {
                            (false, _) => Some(("Play from", "Enter")),
                            (true, true) => Some(("Start the match from", "Enter")),
                            (true, false) => None,
                        };
                        self.preview.draw_timeline(canvas, e, [x + 12.0, y + 10.0, w - 24.0, 34.0], button);
                    }
                    None => next = Some(*std::mem::replace(back, Box::new(Scene::Select))),
                }
            }
            Scene::MpMenu => {
                multiplayer::draw_menu(canvas, &self.mp_menu, &self.cfg.mp_name, self.cfg.mp_port, &self.cfg.mp_address, &self.mp_status, self.mp_connecting.is_some());
            }
            Scene::Lobby => match &self.mp {
                Some(s) => {
                    multiplayer::draw_lobby(canvas, s, s.have_map, lobby_download);
                }
                None => next = Some(Scene::MpMenu),
            },
            Scene::MpWait { rx, loaded, start, from } => {
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
                        let mut game = Game::new(map, track, self.audio.as_ref());
                        if let Some(t) = *from {
                            game.start_from(t, self.audio.as_ref(), self.cfg.offset_ms);
                        }
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
                game.kiai_effects = self.cfg.kiai_effects;
                game.anti_mash = self.cfg.anti_mash;
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
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, self.bg_brightness > 0.0, |c| {
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
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, self.bg_brightness > 0.0, |c| {
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
                    "Up/Down select   PgUp/PgDn section   Left/Right change   Enter toggle / rebind   Esc back to pause menu"
                } else {
                    "Up/Down select   PgUp/PgDn section   Left/Right change   Enter toggle / rebind   Esc or Tab back"
                };
                let this = &*self;
                this.settings_ui.draw(canvas, &|item| this.setting_value(item), footer);
            }
            Scene::LayoutEdit { .. } => {
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, false, game::draw_preview);
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
        if self.cfg.show_playfield_size && matches!(self.scene, Scene::Play { .. } | Scene::MpPlay { .. } | Scene::Calibrate { .. }) {
            let text = self.playfield_label();
            let tw = Canvas::text_width(7.0, &text);
            canvas.text_right(canvas.width - 8.0, 468.0, 7.0, rgba(0x8a8aa0, 0.85), &text);
            self.readout_rect = Some([canvas.width - 8.0 - tw - 4.0, 464.0, tw + 8.0, 15.0]);
        } else {
            self.readout_rect = None;
        }
        if self.settings_ui.dropdown.is_some() {
            let (groups, current, screen) = (self.preset_groups(), self.current_preset(), self.screen_size());
            if let Some(dd) = &mut self.settings_ui.dropdown {
                dd.draw(canvas, &groups, current, screen);
            }
        }
        if finish_calibration {
            self.finish_calibration();
        }
        if let Some(s) = next {
            if matches!(s, Scene::Results { .. } | Scene::Select) {
                self.stop_audio();
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

        let fade = render::progress(self.scene_since, 160.0);
        if fade < 1.0 {
            canvas.rect(0.0, 0.0, canvas.width, render::VIRTUAL_HEIGHT, rgba(0x121219, 0.85 * (1.0 - fade)));
        }
        if self.cfg.show_fps {
            let mode = match self.present {
                PresentChoice::Fifo => "vsync".to_string(),
                p if self.cfg.fps_cap > 0 => format!("{} cap {}", p.name(), self.cfg.fps_cap),
                p => p.name().to_string(),
            };
            let text = format!("{:.0} fps  {:.2} ms  worst {:.1} ms  {mode}", self.stats.fps, 1000.0 / self.stats.fps.max(1.0), self.stats.worst_ms);
            let tw = Canvas::text_width(8.0, &text);
            canvas.round_rect(6.0, 6.0, tw + 12.0, 16.0, 4.0, rgba(0x000000, 0.55), rgba(0, 0.0), 0.0);
            canvas.text(12.0, 10.0, 8.0, rgba(0x9fe09f, 1.0), &text);
        }
        if let Some(shown) = self.volume_shown {
            let age = now.saturating_duration_since(shown).as_secs_f32();
            if age < 1.6 {
                let a = ((1.6 - age) / 0.3).min(1.0);
                let slide = (1.0 - render::progress(shown, 160.0)) * 40.0;
                draw_volume(canvas, &self.cfg, a, slide);
            } else {
                self.volume_shown = None;
            }
        }
        if let Some(t) = &self.toast {
            if now < t.until {
                let a = (t.until.duration_since(now).as_secs_f32() / 0.3).min(1.0);
                let rise = (1.0 - render::progress(t.until - Duration::from_millis(1500), 160.0)) * 14.0;
                let tw = Canvas::text_width(12.0, &t.text);
                let x = canvas.width / 2.0 - tw / 2.0;
                canvas.round_rect(x - 12.0, 380.0 + rise, tw + 24.0, 28.0, 8.0, rgba(0x000000, 0.7 * a), rgba(0xffffff, 0.2 * a), 1.0);
                canvas.text(x, 388.0 + rise, 12.0, rgba(0xffffff, a), &t.text);
            } else {
                self.toast = None;
            }
        }
        std::mem::swap(&mut self.hits, &mut canvas.hits);
        capture
    }

    fn installed_sets(&mut self) -> &mut std::collections::HashSet<u32> {
        let library = &self.library;
        self.installed_sets.get_or_insert_with(|| {
            library.iter().filter_map(|e| e.meta.set_id.or_else(|| e.path.parent().and_then(download::set_id_of_folder))).collect()
        })
    }

    fn open_download(&mut self) {
        self.picking_lobby_map = false;
        self.installed_sets();
        self.browse.opened();
        self.scene = Scene::Download;
    }

    /// Downloads a set unless it's already here or on its way.
    fn start_download(&mut self, set: download::SetInfo) {
        if self.installed_sets().contains(&set.id) {
            self.toast(format!("{} - {} is already in your songs", set.artist, set.title));
            return;
        }
        if self.downloads.iter().any(|j| j.set.id == set.id) {
            return;
        }
        let dir = self.songs_dir.clone();
        self.downloads.push(download::start(set, dir));
    }

    /// Lobby: fetch the map we don't have (by its set ID, else by searching for it).
    fn download_lobby_map(&mut self) {
        let Some(m) = self.mp.as_ref().and_then(|s| s.map.clone()) else { return };
        if self.mp.as_ref().is_some_and(|s| s.have_map) || self.lobby_lookup.is_some() {
            return;
        }
        match m.set_id {
            Some(id) => self.start_download(download::SetInfo {
                id,
                artist: m.artist.clone(),
                title: m.title.clone(),
                creator: String::new(),
                status: String::new(),
                diffs: Vec::new(),
                play_count: 0,
                favourite_count: 0,
                rating: 0.0,
                ranked_date: String::new(),
            }),
            None => {
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let found = download::search(&format!("{} {}", m.artist, m.title), download::Category::Any, download::SortKey::Relevance, true).and_then(|sets| {
                        sets.into_iter()
                            .find(|s| s.title.eq_ignore_ascii_case(&m.title) && s.diffs.iter().any(|d| d.version == m.version))
                            .ok_or_else(|| "not found on the mirrors".to_string())
                    });
                    let _ = tx.send(found);
                });
                self.lobby_lookup = Some(rx);
                self.toast("looking for the map on the mirrors...".into());
            }
        }
    }

    /// Host: packs the lobby map's set folder and sends it to player `to`, on a thread.
    fn send_map_to(&mut self, to: u32) {
        let Some(s) = &self.mp else { return };
        let tx = s.client.sender();
        let folder = s.map_entry.and_then(|i| self.library[i].path.parent().map(|p| p.to_path_buf()));
        std::thread::spawn(move || {
            let send = |part: net::MapPart| tx.send(net::ClientMsg::MapFile { to, part }).is_ok();
            let error = |why: String| net::MapPart { name: String::new(), total: 0, offset: 0, data: String::new(), error: Some(why) };
            let Some(folder) = folder else {
                send(error("the host doesn't have the map either".into()));
                return;
            };
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match download::pack_folder(&folder) {
                Ok(bytes) => {
                    let total = bytes.len() as u64;
                    for (k, chunk) in bytes.chunks(download::PART_BYTES).enumerate() {
                        let part = net::MapPart {
                            name: name.clone(),
                            total,
                            offset: (k * download::PART_BYTES) as u64,
                            data: download::base64_encode(chunk),
                            error: None,
                        };
                        if !send(part) {
                            return;
                        }
                    }
                }
                Err(e) => {
                    send(error(e));
                }
            }
        });
    }

    /// Player: a piece of the lobby map from the host.
    fn receive_map_part(&mut self, part: net::MapPart) {
        if let Some(e) = part.error {
            self.host_transfer = None;
            self.toast(format!("the host couldn't send the map: {e}"));
            return;
        }
        if part.offset == 0 {
            let Some(m) = self.mp.as_ref().and_then(|s| s.map.clone()) else { return };
            let set = download::SetInfo {
                id: m.set_id.unwrap_or(0),
                artist: m.artist.clone(),
                title: m.title.clone(),
                creator: String::new(),
                status: String::new(),
                diffs: Vec::new(),
                play_count: 0,
                favourite_count: 0,
                rating: 0.0,
                ranked_date: String::new(),
            };
            let (job, feed) = download::receive(set, part.name.clone(), part.total, self.songs_dir.clone());
            self.downloads.push(job);
            self.host_transfer = Some(feed);
        }
        let Some(feed) = &self.host_transfer else { return };
        match download::base64_decode(&part.data) {
            Some(bytes) => {
                let last = part.offset + bytes.len() as u64 >= part.total;
                if feed.send(Some(bytes)).is_err() || last {
                    self.host_transfer = None;
                }
            }
            None => {
                let _ = feed.send(None);
                self.host_transfer = None;
            }
        }
    }

    /// The lobby map's download progress, for the lobby screen.
    fn lobby_download_status(&self) -> Option<String> {
        if self.lobby_lookup.is_some() {
            return Some("looking it up...".into());
        }
        let m = self.mp.as_ref()?.map.as_ref()?;
        let job = self.downloads.iter().find(|j| Some(j.set.id) == m.set_id || (j.set.title == m.title && j.set.artist == m.artist))?;
        Some(match job.fraction() {
            Some(f) => format!("downloading {:.0}%", f * 100.0),
            None => "downloading...".into(),
        })
    }

    fn poll_downloads(&mut self) {
        if let Some(rx) = &self.lobby_lookup {
            match rx.try_recv() {
                Ok(Ok(set)) => {
                    self.lobby_lookup = None;
                    self.start_download(set);
                }
                Ok(Err(e)) => {
                    self.lobby_lookup = None;
                    self.toast(format!("download failed: {e}"));
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.lobby_lookup = None,
            }
        }
        let mut finished = Vec::new();
        self.downloads.retain(|job| match job.rx.try_recv() {
            Ok(result) => {
                finished.push((job.set.clone(), result));
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                finished.push((job.set.clone(), Err("download stopped".into())));
                false
            }
        });
        for (set, result) in finished {
            match result {
                Ok(folder) => self.add_downloaded(&set, &folder),
                Err(e) => {
                    eprintln!("download {} failed: {e}", set.id);
                    self.toast(format!("download failed: {}", e.chars().take(80).collect::<String>()));
                }
            }
        }
    }

    /// Adds a freshly downloaded set to the library (appended, so existing indices stay valid).
    fn add_downloaded(&mut self, set: &download::SetInfo, folder: &std::path::Path) {
        let known: std::collections::HashSet<PathBuf> = self.library.iter().filter(|e| e.path.starts_with(folder)).map(|e| e.path.clone()).collect();
        let fresh: Vec<Entry> = library::scan(&[folder.to_path_buf()], &AtomicUsize::new(0)).into_iter().filter(|e| !known.contains(&e.path)).collect();
        let first = fresh.iter().find(|e| e.meta.mode == 1).or(fresh.first()).map(|e| e.path.clone());
        let n = fresh.len();
        let start = self.library.len();
        self.library.extend(fresh);
        if let Some(info) = &self.mapinfo {
            info.extend(self.library[start..].iter().enumerate().map(|(k, e)| (start + k, e.path.clone(), e.modified, e.size)).collect());
            self.mapinfo_sorted = false;
        }
        self.select.refilter(&self.library);
        self.installed_sets().insert(set.id);
        eprintln!("downloaded set {} into {} ({n} maps)", set.id, folder.display());
        // Lobby: the map we were missing may be here now.
        let library = &self.library;
        let mut lobby_note = None;
        if let Some(s) = &mut self.mp {
            if let (Some(m), false) = (&s.map, s.have_map) {
                s.map_entry = find_map(library, m);
                s.have_map = s.map_entry.is_some();
                s.client.send(net::ClientMsg::HasMap(s.have_map));
                if !s.have_map && m.set_id == Some(set.id) {
                    lobby_note = Some("downloaded, but the host has a different version of this map".to_string());
                }
            }
        }
        if !matches!(self.scene, Scene::Play { .. } | Scene::MpPlay { .. } | Scene::MpWait { .. }) && self.mp.is_none() {
            if let Some(path) = &first {
                self.select.select_path(&self.library, path);
            }
        }
        let text = lobby_note.unwrap_or_else(|| format!("downloaded {} - {}", set.artist, set.title));
        self.toast(text);
    }

    fn download_key(&mut self, code: KeyCode, text: Option<&str>, window: Option<&Window>) {
        let function_key = matches!(
            code,
            KeyCode::F1 | KeyCode::F2 | KeyCode::F3 | KeyCode::F4 | KeyCode::F7 | KeyCode::F8 | KeyCode::F9 | KeyCode::F10 | KeyCode::F11 | KeyCode::F12
        );
        if function_key && self.handle_global_key(code, window) {
            return;
        }
        match code {
            KeyCode::ArrowLeft | KeyCode::ArrowRight if self.modifiers_ctrl => self.browse.step_category(if code == KeyCode::ArrowRight { 1 } else { -1 }),
            KeyCode::ArrowUp | KeyCode::ArrowDown if self.modifiers_ctrl => self.browse.step_sort(if code == KeyCode::ArrowDown { 1 } else { -1 }),
            KeyCode::ArrowUp => self.browse.move_by(-1),
            KeyCode::ArrowDown => self.browse.move_by(1),
            KeyCode::ArrowLeft => self.browse.cycle_diff(-1),
            KeyCode::ArrowRight => self.browse.cycle_diff(1),
            KeyCode::PageUp => self.browse.move_by(-8),
            KeyCode::PageDown => self.browse.move_by(8),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(set) = self.browse.selected().cloned() {
                    self.start_download(set);
                }
            }
            KeyCode::Backspace => self.browse.backspace(),
            KeyCode::Escape if !self.browse.query.is_empty() => self.browse.clear(),
            KeyCode::Escape | KeyCode::Tab => self.scene = Scene::Select,
            _ => {
                if let Some(text) = text {
                    self.browse.type_text(text);
                }
            }
        }
    }

    /// The topmost clickable thing under (`x`, `y`), from the last frame.
    fn hit_at(&self, x: f32, y: f32) -> Option<render::Hit> {
        self.hits.iter().rev().find(|h| h.contains(x, y)).map(|h| h.hit)
    }

    /// Switches between the top bar's tabs (songs, settings, multiplayer) from the screens that
    /// show it.
    fn goto_tab(&mut self, tab: usize) {
        let settings_from_menu = matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Select | Scene::Lobby | Scene::MpMenu | Scene::Download));
        match (&self.scene, tab) {
            (Scene::Select, 1) => self.open_settings(),
            (Scene::Select, 2) => {
                self.picking_lobby_map = false;
                self.open_multiplayer();
            }
            (Scene::Settings { .. }, 0) if settings_from_menu => {
                self.close_settings();
                self.scene = Scene::Select;
            }
            (Scene::Settings { .. }, 2) if settings_from_menu => {
                self.close_settings();
                self.open_multiplayer();
            }
            (Scene::MpMenu | Scene::Lobby, 0) => self.scene = Scene::Select,
            (Scene::MpMenu | Scene::Lobby, 1) => self.open_settings(),
            (Scene::Select | Scene::MpMenu | Scene::Lobby, 3) => self.open_download(),
            (Scene::Settings { .. }, 3) if settings_from_menu => {
                self.close_settings();
                self.open_download();
            }
            (Scene::Download, 0) => self.scene = Scene::Select,
            (Scene::Download, 1) => self.open_settings(),
            (Scene::Download, 2) => self.open_multiplayer(),
            _ => {}
        }
    }

    fn click(&mut self, hit: render::Hit, window: Option<&Window>) {
        use render::Hit;
        match hit {
            Hit::Key(code) => self.key(code, false, None, Instant::now(), window),
            Hit::Tab(i) => self.goto_tab(i),
            Hit::Presets => self.open_dropdown(),
            Hit::DlCategory(i) => self.browse.set_category(download::Category::ALL[i]),
            Hit::DlSort(i) => self.browse.set_sort(download::SortKey::ALL[i]),
            Hit::SettingsSection(i) => {
                self.settings_ui.binding = None;
                self.settings_ui.cursor = settings::section_start(i);
            }
            Hit::SortCycle => {
                let dir = if self.modifiers_shift { -1 } else { 1 };
                self.cycle_sort(dir);
            }
            Hit::CollectionPick => self.open_collections(),
            Hit::PickRow(i) => self.select.pick_collection(i, &self.library),
            Hit::DlRow(i) if i == self.browse.cursor => self.download_key(KeyCode::Enter, None, window),
            Hit::DlRow(i) => self.browse.cursor = i,
            Hit::DlGet(i) => {
                self.browse.cursor = i;
                self.download_key(KeyCode::Enter, None, window);
            }
            // Like osu!: the first click selects, clicking the selected difficulty plays it.
            Hit::SongRow(row) if row == self.select.cursor() => self.key(KeyCode::Enter, false, None, Instant::now(), window),
            // A song's own row picks its remembered (or easiest) difficulty.
            Hit::SongRow(row) if self.select.is_song_start(row) && !self.select.in_open_song(row) => self.select.enter_song(row, &self.library),
            Hit::SongRow(row) => self.select.set_cursor(row),
            Hit::SettingRow(i) if i == self.settings_ui.cursor => self.settings_key(KeyCode::Enter, window),
            Hit::SettingRow(i) => self.settings_ui.cursor = i,
            Hit::SettingStep(i, dir) => {
                self.settings_ui.cursor = i;
                self.adjust_setting(settings::ITEMS[i].1, dir, window);
            }
            Hit::PauseItem(i) => {
                if let Scene::Play { game, .. } = &mut self.scene {
                    game.paused = Some(i);
                }
                self.pause_key(KeyCode::Enter);
            }
            Hit::MpRow(i) => {
                self.mp_menu.cursor = i;
                if matches!(i, multiplayer::ROW_HOST | multiplayer::ROW_JOIN) {
                    self.mp_menu_key(KeyCode::Enter, None);
                }
            }
        }
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
    let mut requested_present: Option<PresentChoice> = None;
    let mut waits = (0.0, 0.0, 0.0);
    // TAIKO_PROFILE=1: every 2 s, print where frame time goes (build = CPU under the state lock).
    let profile = std::env::var_os("TAIKO_PROFILE").is_some();
    let (mut prof_start, mut prof_n, mut prof_build, mut prof_submit, mut prof_shapes) = (Instant::now(), 0u32, 0.0f64, 0.0f64, 0usize);
    loop {
        let (present, cap) = {
            let mut s = lock(&state);
            if s.quit {
                break;
            }
            if let Some((w, h)) = s.want_size.take() {
                renderer.resize(w, h);
            }
            // Switch present mode when entering/leaving songs (menus can stay on vsync). Compare
            // with what was requested, not what was granted, so an unsupported mode that falls
            // back isn't re-applied every frame.
            let wanted = s.wanted_present();
            if requested_present != Some(wanted) {
                renderer.set_present(wanted);
                requested_present = Some(wanted);
                s.present = renderer.present;
            }
            if matches!(s.scene, Scene::Play { .. }) {
                if let Some(sm) = &mut s.smooth {
                    sm.sample_waits(waits.0, waits.1, waits.2);
                }
            }
            (s.present, s.cfg.fps_cap)
        };

        let t0 = Instant::now();
        let Some(frame) = renderer.acquire() else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        let acquired = Instant::now();
        let now = acquired;
        let (capture, clip) = {
            let mut s = lock(&state);
            if let Some(atlas) = s.sprites_upload.take() {
                renderer.set_sprites(&atlas);
                canvas.sprites = atlas.sprites.map(|s| s.map(|(_, size)| size));
            }
            let capture = s.build_frame(&mut canvas, now, renderer.virtual_width());
            if let Some(image) = s.preview.bg_upload.take() {
                renderer.set_background(image);
                s.bg_changed = Some(Instant::now());
            }
            renderer.bg_brightness = s.bg_brightness;
            // A new background cross-fades from the previous one.
            renderer.bg_mix = s.bg_changed.map(|t| render::progress(t, 300.0)).unwrap_or(1.0);
            (capture, s.frame_clip.take())
        };
        // The playfield / preview clip (fractions) plus the lists' (virtual units).
        let vw = canvas.width.max(1.0);
        let mut clips: Vec<(std::ops::Range<usize>, [f32; 4])> = clip.into_iter().map(|(range, a)| (range, [a.x, a.y, a.w, a.h])).collect();
        clips.extend(canvas.clips.drain(..).map(|(range, [x, y, w, h])| (range, [x / vw, y / render::VIRTUAL_HEIGHT, w / vw, h / render::VIRTUAL_HEIGHT])));
        let t1 = Instant::now();
        let shot = renderer.submit(frame, &canvas.shapes, [0.071, 0.071, 0.098], capture.is_some(), clips);
        let presented = Instant::now();
        if profile {
            prof_n += 1;
            prof_build += (t1 - now).as_secs_f64() * 1000.0;
            prof_submit += (presented - t1).as_secs_f64() * 1000.0;
            prof_shapes = prof_shapes.max(canvas.shapes.len());
            if prof_start.elapsed() >= Duration::from_secs(2) {
                let n = prof_n.max(1) as f64;
                eprintln!(
                    "profile: {:.0} fps  build {:.2} ms  submit {:.2} ms  shapes {}",
                    n / prof_start.elapsed().as_secs_f64(), prof_build / n, prof_submit / n, prof_shapes
                );
                (prof_start, prof_n, prof_build, prof_submit, prof_shapes) = (Instant::now(), 0, 0.0, 0.0, 0);
            }
        }
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
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_fullscreen(lock(&self.state).cfg.fullscreen.then_some(Fullscreen::Borderless(None)));
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
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                // Timestamp before anything else, including taking the lock.
                let at = Instant::now();
                // Synthetic presses are keys already held when the window gets focus (the Tab of
                // an Alt+Tab back into the game): not real presses.
                if event.state != ElementState::Pressed || is_synthetic {
                    return;
                }
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let mut s = lock(&self.state);
                // Alt+Tab (switching windows) isn't a Tab press for the game; nor is anything
                // that arrives while another window has focus.
                if !s.focused || (s.modifiers_alt && code == KeyCode::Tab) {
                    return;
                }
                s.key(code, event.repeat, event.text.as_deref(), at, self.window.as_deref());
            }
            WindowEvent::Focused(true) => lock(&self.state).focused = true,
            WindowEvent::ModifiersChanged(m) => {
                let mut s = lock(&self.state);
                s.modifiers_shift = m.state().shift_key();
                s.modifiers_ctrl = m.state().control_key();
                s.modifiers_alt = m.state().alt_key();
            }
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
                lock(&self.state).mouse_button(button_state == ElementState::Pressed, self.window.as_deref());
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
                s.focused = false;
                // Alt is released in the other window; don't keep it "held" here.
                s.modifiers_alt = false;
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
    let start_fullscreen = cfg.fullscreen;
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
    let songs_dir = download_dir(&inputs);
    let songs_dirs_for_db: Vec<PathBuf> = inputs.iter().filter(|p| p.is_dir()).cloned().collect();
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
            let (music, fx) = cfg.volumes();
            a.set_volume(music, fx);
            Some(a)
        }
        Err(e) => {
            eprintln!("audio unavailable ({e}); running silent");
            None
        }
    };
    cfg.save();

    let library = Vec::new();
    let mut select = Select::new(&library);
    select.sort = ui::SortMode::parse(&cfg.sort).unwrap_or(ui::SortMode::Artist);
    select.played = load_plays();
    // A single .osu on the command line goes straight into gameplay.
    let play_when_scanned = args.len() == 1 && args[0].extension().is_some_and(|e| e == "osu");
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
        modifiers_ctrl: false,
        modifiers_alt: false,
        volume_shown: None,
        hits: Vec::new(),
        browse: browse::Browse::default(),
        downloads: Vec::new(),
        songs_dir,
        installed_sets: None,
        lobby_lookup: None,
        preview_remote: false,
        host_transfer: None,
        bg_changed: None,
        sprites_upload: None,
        focused: true,
        mapinfo: None,
        mapinfo_sorted: false,
        song_dirs: songs_dirs_for_db,
        osudb_rx: None,
        scene_kind: None,
        scene_since: Instant::now(),
        scrubbing: None,
        auto,
        shots: debug::ShotPlan::from_env(),
        pending_screen_shots: vec!["select", "results"],
        exit_when_done: std::env::var_os("TAIKO_EXIT_AFTER").is_some(),
        quit: false,
        smooth: std::env::var_os("TAIKO_STATS").is_some().then(debug::Smoothness::default),
        started: Instant::now(),
        want_size: None,
        present,
        refresh_hz: 60.0,
        settings_ui: SettingsUi::default(),
        fullscreen: start_fullscreen,
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
        readout_rect: None,
        preview: preview::Preview::default(),
        stars: stars::Stars::default(),
        bg_brightness: 0.0,
        resolutions: Vec::new(),
        monitor_modes: Vec::new(),
    };
    state.load_skin();
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
    let (sr, sr_dt) = (stars::star_rating(path, 1.0), stars::star_rating(path, 1.5));
    println!("  stars {:.2} (1.5x: {:.2})", sr.unwrap_or(f64::NAN), sr_dt.unwrap_or(f64::NAN));
    let audio_path = path.parent().unwrap_or(path).join(&m.audio_file);
    let t = Instant::now();
    match audio::decode(&audio_path, 48000) {
        Ok(track) => println!("  audio {:.1} s decoded in {:.0} ms", track.frames() as f64 / 48000.0, t.elapsed().as_secs_f64() * 1000.0),
        Err(e) => println!("  audio error: {e}"),
    }
}
