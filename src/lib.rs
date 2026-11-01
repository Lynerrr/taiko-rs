//! The game. Desktop builds start it from `main.rs` (`desktop_main`); Android loads this crate
//! as a library and starts it from `android_main`.

// First, so `tr!` is there for every module after it.
#[macro_use]
mod lang;
#[cfg(target_os = "android")]
mod android;
mod audio;
mod autoplay;
mod beatmap;
mod browse;
mod calibrate;
mod changelog;
mod changelog_entry;
mod clock;
mod clipboard;
mod config;
mod cursor;
mod debug;
mod discord;
mod online;
mod tls;
mod difficulty;
mod download;
mod font;
mod game;
mod genres;
mod guide;
mod layout;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod results_tests;
#[cfg(test)]
mod settings_tests;
mod library;
mod link;
mod logo;
mod mapguessr;
mod mapinfo;
mod media;
mod menu;
mod menu_notes;
mod hud;
mod keys;
mod mods;
mod multiverse;
mod multiplayer;
mod net;
mod osudb;
mod player;
mod playlist;
mod pp;
mod profile;
mod preview;
mod recalc;
mod render;
mod scores;
mod search;
mod setstatus;
mod settings;
mod settings_search;
mod shapeshift;
mod skin;
mod social;
mod splash;
mod stars;
mod ui;
mod update;
mod video;
mod volume;
#[cfg(target_os = "linux")]
mod wayland_dnd;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use audio::{Engine, Track};
use beatmap::Beatmap;
use config::{Config, FPS_CAPS, RATE_RANGE};
use game::{Game, PAUSE_ITEMS};
use layout::{Area, Grip};
use winit::window::CursorIcon;
use library::Entry;
use render::{accent, rgba, themed, Canvas, PresentChoice, Renderer};
use settings::{Choices, Dropdown, Edit, Item, Kind, PresetAction, SettingsUi};
use mods::Mods;
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
    /// Offset calibration (by ear), or with `vsync` the vsync compensation's (by sight).
    Calibrate { game: Box<Game>, back: Box<Scene>, vsync: bool },
    CalibrateDone { estimate: calibrate::Estimate, back: Box<Scene>, vsync: bool },
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
    /// Watching the match in progress.
    Spectate(Box<SpectateView>),
    /// Watching a friend's solo play.
    Watch(Box<WatchView>),
    /// Turning a replay into a video: cut, crop, size, export.
    VideoEdit(Box<video::Editor>),
    MpResults,
}

/// How slowly a new background comes in on the main menu, where it's the whole picture.
const MENU_BACKGROUND_FADE_MS: f32 = 900.0;

/// How long "Reset map offsets" waits for its second press.
const RESET_CONFIRM: Duration = Duration::from_secs(4);

/// How far the player's arrow keys and wheel move the music.
const MUSIC_SEEK_MS: f64 = 5000.0;

/// How often a running game looks for a newer build (with "Check for updates" on).
const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(30 * 60);

/// How far behind the live match spectators watch, so players' presses arrive in time.
const SPECTATE_DELAY: Duration = Duration::from_millis(1500);

struct SpectateView {
    rx: Option<Receiver<Loaded>>,
    loaded: Option<(Beatmap, Arc<Track>)>,
    /// When the match started (local clock), its start point and speed.
    start: Instant,
    from: Option<f64>,
    rate: f64,
    mods: Mods,
    /// The match's map as the room named it, with the host's mod settings.
    map: net::MapInfo,
    players: Vec<(u32, String)>,
    /// One game per player, fed with their presses.
    games: Vec<(u32, Box<Game>)>,
    /// The player shown, or None for everyone at once.
    focus: Option<usize>,
    /// Presses that arrived before the games existed.
    pending: std::collections::HashMap<u32, Vec<(f64, u8)>>,
    /// Players who skipped the intro; once all have, so does the view.
    skipped: std::collections::HashSet<u32>,
    skip_done: bool,
    /// Watching everyone: what each player's strip shows (see `draw_spectate_all`), and when it
    /// was last drawn.
    zones: Vec<[f32; 4]>,
    zones_at: Option<Instant>,
}

impl SpectateView {
    /// When every player has skipped the intro, skips it too, staying `SPECTATE_DELAY` behind.
    fn follow_skip(&mut self, audio: Option<&audio::Engine>, offset: f64) {
        if self.skip_done || self.games.is_empty() || !self.players.iter().all(|(id, _)| self.skipped.contains(id)) {
            return;
        }
        self.skip_done = true;
        let target = self.games[0].1.skip_target() - SPECTATE_DELAY.as_secs_f64() * 1000.0 * self.rate;
        if self.games[0].1.time < target {
            for (_, g) in &mut self.games {
                g.seek(target, audio, offset);
            }
        }
    }
}

/// How far behind a watched friend the view plays (room for the network).
const WATCH_DELAY_MS: f64 = 2000.0;

/// Watching a friend's solo play: a replay fed live, a little behind, holding while they pause.
struct WatchView {
    player: String,
    play: Option<online::SoloPlay>,
    entry: Option<usize>,
    rx: Option<Receiver<Loaded>>,
    game: Option<Box<Game>>,
    /// Presses that arrived before the game existed.
    inputs: Vec<(f64, u8)>,
    /// Their map time when last heard, when that was, and whether they're paused.
    clock: (f64, Instant, bool),
    held: bool,
    fetching: bool,
    looked: Option<Instant>,
}

impl WatchView {
    fn new(player: String) -> WatchView {
        WatchView { player, play: None, entry: None, rx: None, game: None, inputs: Vec::new(), clock: (-1.0e9, Instant::now(), true), held: false, fetching: false, looked: None }
    }

    /// Where the player is now (map time).
    fn their_time(&self) -> f64 {
        let (t, at, paused) = self.clock;
        let rate = self.play.as_ref().map(|p| p.map.rate).unwrap_or(1.0);
        if paused { t } else { t + at.elapsed().as_secs_f64() * 1000.0 * rate }
    }

    fn status(&self) -> String {
        match (&self.play, self.entry) {
            (None, _) => {
                let player = &self.player;
                tr!("waiting for {player} to play something...", "{player}が何かプレイするのを待っています...", "ждём, когда {player} начнёт играть...";)
            }
            (Some(_), None) if self.fetching => tr!("getting the map...", "譜面を取得中...", "получаем карту...").into(),
            (Some(_), None) => tr!("looking for the map...", "譜面を探しています...", "ищем карту...").into(),
            (Some(_), Some(_)) => tr!("loading...", "読み込み中...", "загрузка...").into(),
        }
    }
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

type SetCheck = (u32, Receiver<Result<(bool, Vec<PathBuf>), String>>);
type BoardFetch = (u64, Receiver<Result<Vec<online::OnlineScore>, String>>);
type LovedCall = (Receiver<Result<online::Reply, String>>, Option<(u32, bool)>);

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
    /// What an input method (Japanese, ...) is composing, not typed yet: shown until it's
    /// committed, then typed like keys.
    ime_preedit: String,
    /// Since when files are being dragged over the window.
    drag_hover: Option<Instant>,
    stats: FrameStats,
    status: String,
    modifiers_shift: bool,
    modifiers_ctrl: bool,
    modifiers_alt: bool,
    /// When the volume overlay was last poked (Alt + wheel).
    volume: volume::Overlay,
    /// The startup splash while it plays.
    splash: Option<splash::Splash>,
    /// The main menu, up over song select (`on_menu`).
    main_menu: Option<menu::Menu>,
    /// What drifts behind it.
    menu_notes: menu_notes::Notes,
    /// When dragging the player's timeline last moved the music.
    music_seeked: Option<Instant>,
    /// The first steps guide is up, on this page.
    guide: Option<usize>,
    /// The drum (`Config::keys`) the guide is waiting for a new key for.
    guide_binding: Option<usize>,
    /// The page to go back to once calibrating from the guide is done.
    guide_after: Option<usize>,
    /// When "Reset map offsets" was pressed once, waiting for the second.
    reset_armed: Option<Instant>,
    /// The music player, instead of the main menu.
    player: Option<Box<player::Player>>,
    /// The main menu's music is paused.
    music_paused: bool,
    /// Media keys and the desktop's media controls, from the first time the player opens.
    media: Option<media::Media>,
    /// The track the desktop was last told about, and whether it's playing.
    media_told: Option<(usize, bool)>,
    /// The window's handle, for the media controls on Windows.
    window_handle: Option<usize>,
    updater: update::Updater,
    /// Clickable areas of the last frame drawn.
    hits: Vec<render::HitBox>,
    browse: browse::Browse,
    downloads: Vec<download::Job>,
    /// Song select: the set being checked for a newer version on osu! (see `poll_set_check`),
    /// and the one the selection rests on, since when.
    /// The set being checked against osu!: whether it's outdated, and its difficulties here
    /// that aren't osu!'s.
    set_check: Option<SetCheck>,
    set_check_due: Option<(u32, Instant)>,
    /// Android: when the folder of .osz files opened with the game was last looked at.
    #[cfg(target_os = "android")]
    imports_checked: Instant,
    /// Android: the on-screen keyboard (see `update_keyboard`).
    #[cfg(target_os = "android")]
    keyboard: android::Keyboard,
    /// Where downloaded sets go.
    songs_dir: PathBuf,
    /// Beatmap set IDs in the library (built when first needed).
    installed_sets: Option<std::collections::HashSet<u32>>,
    /// Whether the preview currently shows Download tab entries.
    preview_remote: bool,
    /// Receiving the lobby map from the host: where its parts go.
    host_transfer: Option<std::sync::mpsc::Sender<Option<Vec<u8>>>>,
    /// The lobby map was asked of the host: only then is what the host sends taken.
    host_map_asked: bool,
    /// Local scores (index of saved replays).
    scores: Vec<scores::ScoreRow>,
    /// Working out the pp of scores saved before pp (started once, on the first frame).
    pp_backfill: Option<Receiver<Vec<(PathBuf, f64)>>>,
    pp_backfill_started: bool,
    /// Plays on their way to the online leaderboards.
    submits: Vec<Receiver<Result<online::Reply, String>>>,
    /// The logged-in player's own pp total and rank online (for the account badge and the
    /// results' total), whose they are, and the fetch of them.
    me: Option<(f64, Option<u32>)>,
    me_for: String,
    me_fetch: Option<Receiver<Result<online::Profile, String>>>,
    /// The song select leaderboard shows the server's scores, not the local ones.
    board_online: bool,
    /// A player's profile, over everything else.
    profile: Option<profile::View>,
    /// What's new, over everything else.
    changelog: Option<changelog::View>,
    /// Update notes being fetched without opening an overlay or intercepting input.
    pending_changelog: Option<changelog::View>,
    /// Online leaderboards fetched (by map hash: when, and the rows or why not), and the one
    /// being fetched.
    online_boards: std::collections::HashMap<u64, (Instant, Result<Vec<online::OnlineScore>, String>)>,
    board_fetch: Option<BoardFetch>,
    /// The replay to attach to the game being loaded.
    pending_replay: Option<scores::Replay>,
    /// Mods of the play being loaded.
    pending_mods: Mods,
    /// The mod select is open: the mod the keyboard is on (index into `mods::ALL`).
    mod_select: Option<usize>,
    /// The mod customization over the mod select: the Multiverse tab or the Shapeshift tab (at
    /// most one is open).
    universe_setup: Option<multiverse::Setup>,
    shapeshift_setup: Option<shapeshift::Setup>,
    /// Replay files of the leaderboard rows drawn this frame, and the online rows.
    leaderboard: Vec<PathBuf>,
    /// The leaderboard's rows scrolled past, for which map and tab (a new one starts at the
    /// top), how far it goes, and where its rows were drawn (for the wheel).
    board_scroll: usize,
    board_for: (u64, bool),
    board_max: usize,
    board_rect: Option<[f32; 4]>,
    /// A song select divider being dragged: which, where the pointer started (virtual units)
    /// and the size it had then; and the last press on one (a second soon after resets it).
    split_drag: Option<(render::Hit, f32, f32)>,
    split_click: Option<(render::Hit, Instant)>,
    online_leaderboard: Vec<online::OnlineScore>,
    /// A video export in progress, and the editor's last message.
    export: Option<video::Export>,
    video_status: String,
    /// The video last exported from the open editor.
    last_video: Option<PathBuf>,
    /// When the background image last changed (it fades in).
    bg_changed: Option<Instant>,
    /// A newly loaded skin's note images, for the render thread to upload.
    sprites_upload: Option<render::Atlas>,
    /// The playing map's folder when it carries skin images (they're in the loaded skin), and
    /// the folder last checked for them.
    skin_map: Option<PathBuf>,
    skin_map_seen: Option<PathBuf>,
    /// The window has keyboard focus.
    focused: bool,
    /// The right button is held on song select (scrolling through the list).
    right_scroll: bool,
    /// The settings' bar being dragged (index into `settings::ITEMS`).
    settings_level: Option<usize>,
    /// The song list's scroll bar is held.
    scrollbar_drag: bool,
    /// Right button pressed on song select and not moved yet (virtual units, what's under it):
    /// released there it opens the row's menu; moved, it becomes right-click scrolling.
    right_press: Option<(f32, f32, Option<render::Hit>)>,
    /// Left button pressed on a song list row: dragging scrolls the list, releasing without
    /// dragging clicks the row.
    list_drag: Option<ListDrag>,
    /// The window has had focus at least once (the splash waits for it).
    focus_seen: bool,
    /// Background star rating / length / BPM analysis of the library, for sorting.
    mapinfo: Option<mapinfo::MapInfo>,
    /// Whether the song list was re-sorted after the analysis finished.
    mapinfo_sorted: bool,
    /// Folders being scanned (where osu!.db / collection.db are looked for).
    song_dirs: Vec<PathBuf>,
    osudb_rx: Option<Receiver<Option<osudb::OsuDb>>>,
    /// Maps' status from osu!.db (by path), and sets' from the mirrors (by set id; None while
    /// asking or if they couldn't say).
    db_status: std::collections::HashMap<PathBuf, u8>,
    set_status: std::collections::HashMap<u32, Option<String>>,
    set_status_tx: mpsc::Sender<(u32, Option<String>)>,
    set_status_rx: Receiver<(u32, Option<String>)>,
    /// What the mirrors said about sets, kept between runs (see `setstatus`), when it was last
    /// written, and when the list was last sorted again for new answers.
    status_cache: setstatus::Cache,
    status_saved: Instant,
    status_resorted: Instant,
    status_new: bool,
    /// Maps came in (downloaded or dropped): when to ask about their sets' statuses (a while
    /// later, so a batch of downloads is asked about once).
    status_sweep_due: Option<Instant>,
    /// Asking the online server which sets are loved there (or loving one: the set and which
    /// way, for the message), and whether it's been asked since starting.
    loved_call: Option<LovedCall>,
    loved_asked: bool,
    /// The live play going on (its time played so far), until it's sent to the leaderboards or
    /// left some other way: then it's counted on the server as played (`Request::Played`).
    playing: Option<f64>,
    /// The selected set and since when: asked about once the selection rests.
    status_wait: Option<(u32, Instant)>,
    /// Looking up the lobby map on the mirrors (it had no set ID).
    lobby_lookup: Option<Receiver<Result<download::SetInfo, String>>>,
    /// A score a `taiko-rs://` link asked for, until its results screen opens.
    score_link: Option<ScoreLink>,
    /// Which screen was drawn last and since when, for the fade between screens.
    scene_kind: Option<std::mem::Discriminant<Scene>>,
    scene_since: Instant,
    /// Dragging along the song select difficulty timeline: when it last seeked (for throttling)
    /// and to which map time.
    scrubbing: Option<(Instant, f64)>,
    auto: bool,
    shots: Option<debug::ShotPlan>,
    /// Screens still to capture once each when a shot plan is active.
    pending_screen_shots: Vec<&'static str>,
    exit_when_done: bool,
    quit: bool,
    /// "Exit taiko.rs?" is up (Ctrl+Q, Esc in song select, the window's close button).
    quit_asked: bool,
    /// The game crashed last time: its report, offered to copy (`draw_crash`).
    crash: Option<String>,
    /// F12 was pressed: the next frame is saved as a picture.
    screenshot_due: bool,
    smooth: Option<debug::Smoothness>,
    /// When the frame being built got its swapchain image (for the smoothness report).
    frame_acquired: Option<Instant>,
    /// With vsync, how much later (ms) than its image was handed out the frame's notes are drawn
    /// for, to keep them on the refresh grid (`clock::VsyncPacer`).
    pace_shift_ms: f64,
    /// The refresh interval (ms) the frames were seen to come at with vsync (the window system's
    /// `refresh_hz` can be another screen's, or a guess).
    seen_refresh_ms: Option<f64>,
    started: Instant,
    /// Requests from the input thread that only the render thread can apply.
    want_size: Option<(u32, u32)>,
    /// A touch screen was used: show the touch columns during play.
    touch_used: bool,
    /// The finger acting as the mouse (menus).
    touch_mouse: Option<u64>,
    /// When each touch column was last hit (for its flash).
    touch_flash: [Option<Instant>; 4],
    /// A new window's surface and size, or `None` to stop drawing (Android destroys the window
    /// in the background; the input thread waits until the render thread has let go of it).
    surface_swap: Option<(Option<wgpu::Surface<'static>>, u32, u32)>,
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
    /// Song select is being used by the host to pick the lobby map (in mapguessr, by anyone to
    /// pick their guess).
    picking_lobby_map: bool,
    /// Our side of a mapguessr room, while in one.
    guessr: Option<mapguessr::Guessr>,
    /// Discord status: the connection thread (while enabled), the activity last set, and when
    /// it was last worked out.
    discord: Option<discord::Presence>,
    /// Each map's own offset (by content hash), on top of the global one.
    map_offsets: std::collections::HashMap<u64, f64>,
    /// The offset in effect: global + the current map's (worked out each frame).
    play_offset: f64,
    /// Friends, chat and notifications (the F9 overlay) over the social connection.
    social: social::Social,
    /// The solo play being streamed to friends: which game, presses sent, last frame sent.
    stream: Option<(usize, usize, Instant)>,
    presence: Option<discord::Activity>,
    presence_at: Option<Instant>,
    /// Background library scan: result channel and files-read counter.
    scan: Option<(Receiver<Vec<Entry>>, Arc<AtomicUsize>)>,
    /// The Songs folder dialog while it is open: the folder picked (None = cancelled), or why
    /// there is no dialog.
    folder_pick: Option<Receiver<Result<Option<PathBuf>, String>>>,
    /// What the folder dialog is for: the Songs folder or the game data folder.
    folder_pick_for: Item,
    /// The game's data being moved to another folder (`config::move_data`).
    data_move: Option<DataMove>,
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
    /// The game's size in pixels: the window, or its part of it with Letterbox on.
    window_w: f32,
    window_h: f32,
    /// The window's own size, and where the game is in it with Letterbox on.
    real_window: (f32, f32),
    letterbox: Option<[u32; 4]>,
    /// Shapes of this frame to clip to the playfield area.
    frame_clip: Option<(std::ops::Range<usize>, Area)>,
    /// Song select preview: background, preview music, autoplay preview.
    preview: preview::Preview,
    /// Star ratings (rosu-pp), calculated in the background for the selected song.
    stars: stars::Stars,
    /// The pp the selected map could give (song select's details).
    potentials: pp::Potentials,
    /// Background brightness for this frame (0 hides it).
    bg_brightness: f32,
    /// Where the playfield size readout was drawn (virtual units), so it can be clicked.
    readout_rect: Option<[f32; 4]>,
    /// Watching a replay: the timeline drawn this frame (virtual units), and when the pointer
    /// dragging along it last seeked.
    replay_bar: Option<[f32; 4]>,
    replay_scrub: Option<Instant>,
    /// Last time the pointer moved (gameplay hides it after a second of stillness).
    mouse_moved_at: Instant,
    /// Snap guides to draw while dragging the playfield area.
    guides: Vec<layout::Guide>,
    /// Playfield presets (resolutions that fit the window), filled when the window opens.
    resolutions: Vec<(u32, u32)>,
    monitor_modes: Vec<(u32, u32)>,
    /// The screen the window is on (pixels).
    monitor_size: (u32, u32),
    /// A window size picked in settings, for the window thread to apply.
    window_request: Option<(u32, u32)>,
    /// The window was resized by hand: its size is saved once it settles.
    window_resized_at: Option<Instant>,
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
    let (start, full) = layout::begin(canvas, area, see_through, cfg.bg_dim_black);
    f(canvas);
    let range = layout::end(canvas, area, start, full);
    *clip = Some((range, area));
    if grip.is_some() || matches!(hover, Some(Grip::Resize { .. })) {
        layout::draw_frame(canvas, area, grip.is_some());
    }
}

/// A press on a song list row (see `State::list_drag`).
struct ListDrag {
    from_y: f32,
    hit: render::Hit,
    moved: bool,
}

/// How far (virtual units) a held button must move before it counts as dragging, not a click.
const DRAG_START: f32 = 5.0;

fn fmt_time(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Replays: a timeline along the bottom of the screen to click or drag through. Returns the
/// bar's rect (virtual units) for `replay_seek_at`.
fn draw_replay_bar(c: &mut Canvas, game: &Game, active: bool) -> [f32; 4] {
    let w = c.width;
    let (start, end) = game.span();
    let label = format!("{} / {}", fmt_time(game.time), fmt_time(end));
    // Sized for the longest label so the bar doesn't twitch as the time changes.
    let lw = Canvas::text_width(7.0, &format!("{} / {}", fmt_time(end), fmt_time(end)));
    let (x, y, bw) = (18.0 + lw + 10.0, 456.0, w - 18.0 - (18.0 + lw + 10.0));
    let rect = [x, y - 7.0, bw, 14.0];
    let hover = active || c.hovered(rect[0] - 6.0, rect[1], rect[2] + 12.0, rect[3]);
    let f = ((game.time - start) / (end - start).max(1.0)).clamp(0.0, 1.0) as f32;
    let th = if hover { 4.0 } else { 2.0 };
    c.text(18.0, y - 3.5, 7.0, rgba(0xc8c8d8, 1.0), &label);
    c.text(x, y + 9.0, 6.0, rgba(0x6f6f86, 1.0), tr!("Left/Right or drag: seek", "左右かドラッグ: シーク", "Влево/вправо или тяните: перемотка"));
    c.rect(x, y - th / 2.0, bw, th, rgba(0x2a2a38, 1.0));
    c.rect(x, y - th / 2.0, bw * f, th, rgba(accent(), 1.0));
    let k = if hover { 5.0 } else { 3.5 };
    c.round_rect([x + bw * f - k, y - k], [k * 2.0, k * 2.0], 1.0, rgba(0xffffff, 1.0), rgba(accent(), 1.0), 1.5);
    rect
}

/// Every spectated player at once: one lane per player, stacked, with their numbers.
/// Each player's strip shows where they play: the lane, or wherever the mods take it (rails,
/// Multiverse's playfield in focus...), fitted in and followed smoothly. `zones` keeps what each
/// strip shows between frames; `dt` is the time since the last one (s).
fn draw_spectate_all(c: &mut Canvas, games: &mut [(u32, Box<Game>)], players: &[(u32, String)], scroll: f64, zones: &mut Vec<[f32; 4]>, dt: f32) {
    let n = games.len().max(1) as f32;
    let (band_top, band_h) = game::lane_band();
    let (top, bottom) = (40.0, 470.0);
    let strip = ((bottom - top) / n).min(120.0);
    let k = strip / band_h;
    let full_w = c.width;
    // Drawn wider than the screen, so a plain lane fills the strip's width.
    let plain = [0.0, band_top, full_w / k, band_h];
    zones.resize(games.len(), [f32::NAN; 4]);
    let follow = 1.0 - (-dt / 0.15).exp();
    let head = tr!(
        "SPECTATING  -  everyone   Left/Right or A: one player   Esc leave",
        "観戦中  -  全員   左右かA: 一人   Esc 退出",
        "НАБЛЮДЕНИЕ  -  все   Влево/вправо или A: один игрок   Esc выйти"
    );
    c.text_centered(full_w / 2.0, 16.0, 9.0, rgba(0x4fb4ff, 1.0), &Canvas::fit(9.0, full_w - 16.0, head));
    for (i, (id, g)) in games.iter_mut().enumerate() {
        let y = top + i as f32 * strip;
        let start = c.shapes.len();
        c.width = full_w / k;
        g.draw_playfield(c, scroll);
        c.width = full_w;
        let target = g.play_zone().filter(|z| z[2] > 1.0 && z[3] > 1.0).unwrap_or(plain);
        let z = &mut zones[i];
        if z[0].is_nan() {
            *z = target;
        } else {
            for (v, t) in z.iter_mut().zip(target) {
                *v += (t - *v) * follow;
            }
        }
        let [zx, zy, zw, zh] = *z;
        let scale = (full_w / zw).min(strip / zh);
        let (dx, dy) = ((full_w - zw * scale) / 2.0 - zx * scale, y + (strip - zh * scale) / 2.0 - zy * scale);
        c.transform_since(start, [0.0, 0.0], scale, dx, dy);
        c.clip_since(start, 0.0, y, full_w, strip - 2.0);
        let name = players.iter().find(|(p, _)| p == id).map(|(_, n)| n.as_str()).unwrap_or("?");
        let st = &g.stats;
        let line = format!("{name}   {:08}   {:.2}%   {}x", st.score, st.accuracy() * 100.0, st.combo);
        let tw = Canvas::text_width(8.0, &line);
        c.round_rect([full_w - tw - 28.0, y + 4.0], [tw + 16.0, 16.0], 6.0, rgba(0x000000, 0.6), rgba(0, 0.0), 0.0);
        c.text_right(full_w - 20.0, y + 8.0, 8.0, rgba(0xffffff, 1.0), &line);
    }
}

/// The leaderboard's LOCAL / ONLINE tabs; returns how wide they are.
fn board_tabs(c: &mut Canvas, x: f32, y: f32, online: bool) -> f32 {
    let mut tx = x;
    for (label, on) in [(tr!("LOCAL", "ローカル", "ЛОКАЛЬНО"), false), (tr!("ONLINE", "オンライン", "ОНЛАЙН"), true)] {
        let tw = Canvas::text_width(7.0, label) + 10.0;
        let lit = on == online;
        let h = Canvas::text_height(7.0) + 3.0;
        c.round_rect([tx, y - 1.5], [tw, h], 3.0, rgba(if lit { accent() } else { 0x1d1d27 }, if lit { 0.9 } else { 1.0 }), rgba(if lit { accent() } else { 0x3a3a4e }, 1.0), 1.0);
        c.text(tx + 5.0, y + 1.5, 7.0, rgba(if lit { 0xffffff } else { 0x8a8aa0 }, 1.0), label);
        c.hit(tx, y - 1.5, tw, h, render::Hit::BoardTab(on));
        tx += tw + 4.0;
    }
    tx - x - 4.0
}

/// The server's scores for the selected map: each player's best (`board` None while loading).
fn draw_online_leaderboard(c: &mut Canvas, [x, y]: [f32; 2], w: f32, board: Option<&Result<Vec<online::OnlineScore>, String>>, me: &str, fits: usize, first: usize) {
    let tabs_w = board_tabs(c, x + 4.0, y + 1.0, true);
    let at = x + 4.0 + tabs_w + 10.0;
    let note = match board {
        None => Some(tr!("loading...", "読み込み中...", "загрузка...").to_string()),
        Some(Err(e)) => Some(tr!("can't reach the server: {e}", "サーバーに届きません: {e}", "сервер недоступен: {e}";)),
        Some(Ok(rows)) if rows.is_empty() => Some(
            tr!(
                "no online scores yet - log in (F9) and finish a play",
                "オンラインのスコアはまだありません - ログイン (F9) してプレイを終えましょう",
                "онлайн-рекордов пока нет - войдите (F9) и доиграйте карту"
            )
            .to_string(),
        ),
        Some(Ok(_)) => None,
    };
    if let Some(note) = note {
        c.text(at, y + 2.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit(7.0, x + w - 4.0 - at, &note));
        return;
    }
    let Some(Ok(rows)) = board else { return };
    let hint = tr!("click: results and replay", "クリック: リザルトとリプレイ", "клик: итоги и повтор");
    c.text_right(x + w - 4.0, y + 2.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit_hints(6.0, w - 20.0 - tabs_w, hint));
    let shown = rows.len().saturating_sub(first).min(fits);
    board_scrollbar(c, x + w + 5.0, y + 14.0, shown.max(1), first, rows.len());
    for (k, r) in rows.iter().skip(first).take(fits).enumerate() {
        let ry = y + 14.0 + k as f32 * 18.0;
        let mine = r.player.eq_ignore_ascii_case(me);
        let hover = c.hovered(x, ry, w, 16.0);
        let bg = if hover { 0x262634 } else if mine { 0x23232f } else { 0x15151d };
        c.round_rect([x, ry], [w, 16.0], 5.0, rgba(bg, 0.9), rgba(if mine { accent() } else { 0x2a2a38 }, 1.0), 1.0);
        c.hit(x, ry, w, 16.0, render::Hit::OnlineScoreRow(k));
        let rate = if (r.rate - 1.0).abs() > 1e-6 { format!("  {:.2}x", r.rate) } else { String::new() };
        let mods = if r.mods.is_empty() { String::new() } else { format!("  +{}", r.mods.concat()) };
        let pp = r.pp.map(|p| format!("  {p:.0}pp")).unwrap_or_default();
        let core = format!("#{}  {}  {:08}  {:.2}%{pp}  {}x", first + k + 1, r.player, r.score, r.accuracy * 100.0, r.max_combo);
        let room = w - 16.0;
        let right = Canvas::fit(6.5, room - Canvas::text_width(6.5, &core) - 12.0, &misses_and_age(r.miss, r.date));
        let left_room = room - if right.is_empty() { 0.0 } else { Canvas::text_width(6.5, &right) + 8.0 };
        c.text(x + 8.0, ry + 5.0, 6.5, rgba(if first + k == 0 { 0xFFD34D } else { 0xffffff }, 1.0), &Canvas::fit(6.5, left_room, &format!("{core}{rate}{mods}")));
        c.text_right(x + w - 8.0, ry + 5.0, 6.5, rgba(0x9a9ab0, 1.0), &right);
    }
}

/// A leaderboard row's "3 miss  2d".
fn misses_and_age(miss: u32, date: u64) -> String {
    let age = scores::age(date);
    tr!("{miss} miss  {age}", "ミス{miss}  {age}", "{miss} мимо  {age}";)
}

/// Local scores for the selected map, under the song select panel: `rows` from place `first`
/// on, of `total`.
fn draw_leaderboard(c: &mut Canvas, x: f32, y: f32, w: f32, rows: &[&scores::ScoreRow], first: usize, total: usize) {
    let label_w = board_tabs(c, x + 4.0, y + 1.0, false);
    if rows.is_empty() {
        let at = x + 4.0 + label_w + 14.0;
        let none = tr!("none yet - finish a play to set one", "まだありません - プレイを終えると記録されます", "пока нет - доиграйте карту");
        c.text(at, y + 2.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit(7.0, x + w - 4.0 - at, none));
        return;
    }
    let room = w - 20.0 - label_w;
    let hint = tr!("click: results and replay   video: export it", "クリック: リザルトとリプレイ   動画: 書き出し", "клик: итоги и повтор   видео: экспорт");
    c.text_right(x + w - 4.0, y + 2.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit_hints(6.0, room, hint));
    for (k, r) in rows.iter().enumerate() {
        let ry = y + 14.0 + k as f32 * 18.0;
        let hover = c.hovered(x, ry, w, 16.0);
        c.round_rect([x, ry], [w, 16.0], 5.0, rgba(if hover { 0x262634 } else { 0x15151d }, 0.9), rgba(0x2a2a38, 1.0), 1.0);
        c.hit(x, ry, w, 16.0, render::Hit::ScoreRow(k));
        let rate = if (r.rate - 1.0).abs() > 1e-6 { format!("  {:.2}x", r.rate) } else { String::new() };
        let mods = if r.mods.is_empty() { String::new() } else { format!("  +{}", r.mods) };
        let pp = r.pp.map(|p| format!("  {p:.0}pp")).unwrap_or_default();
        let core = format!("#{}  {:08}  {:.2}%{pp}  {}x", first + k + 1, r.score, r.accuracy * 100.0, r.max_combo);
        // The video button sits on top of the row (drawn after it, so it wins the click).
        let video = tr!("video", "動画", "видео");
        let bw = Canvas::text_width(6.0, video) + 12.0;
        let bx = x + w - bw - 3.0;
        // Room goes to the score first, then the player and age on the right, then rate and mods.
        let room = bx - 6.0 - (x + 8.0);
        let right = Canvas::fit(6.5, room - Canvas::text_width(6.5, &core) - 12.0, &format!("{}  {}", r.player, misses_and_age(r.miss, r.date)));
        let left_room = room - if right.is_empty() { 0.0 } else { Canvas::text_width(6.5, &right) + 8.0 };
        c.text(x + 8.0, ry + 5.0, 6.5, rgba(if first + k == 0 { 0xFFD34D } else { 0xffffff }, 1.0), &Canvas::fit(6.5, left_room, &format!("{core}{rate}{mods}")));
        c.button(bx, ry + 2.0, bw, 12.0, video, 6.0, render::Hit::ScoreVideo(k), false);
        c.text_right(bx - 6.0, ry + 5.0, 6.5, rgba(0x9a9ab0, 1.0), &right);
    }
    board_scrollbar(c, x + w + 5.0, y + 14.0, rows.len().max(1), first, total.max(rows.len()));
}

/// A song select divider in `rect` (`upright`: between left and right, else between top and
/// bottom): a short grip, lit along its length while hovered or dragged. Only the grip takes the
/// mouse, not the whole edge: passing along it shouldn't light it up.
fn draw_divider(c: &mut Canvas, [x, y, w, h]: [f32; 4], upright: bool, dragging: bool, hit: render::Hit) {
    const GRIP: f32 = 48.0;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let [gx, gy, gw, gh] = if upright { [x, cy - GRIP / 2.0, w, GRIP] } else { [cx - GRIP / 2.0, y, GRIP, h] };
    let lit = dragging || c.hovered(gx, gy, gw, gh);
    if upright {
        let len = if lit { h } else { 28.0 };
        c.round_rect([cx - 1.0, cy - len / 2.0], [2.0, len], 1.0, rgba(if lit { accent() } else { 0x3a3a4e }, 1.0), rgba(0, 0.0), 0.0);
    } else {
        let len = if lit { w } else { 28.0 };
        c.round_rect([cx - len / 2.0, cy - 1.0], [len, 2.0], 1.0, rgba(if lit { accent() } else { 0x3a3a4e }, 1.0), rgba(0, 0.0), 0.0);
    }
    c.hit(gx, gy, gw, gh, hit);
}

/// A leaderboard's scroll bar right of its rows, when there are more than the `shown`.
fn board_scrollbar(c: &mut Canvas, x: f32, y: f32, shown: usize, first: usize, total: usize) {
    if total <= shown || shown == 0 {
        return;
    }
    let h = shown as f32 * 18.0 - 2.0;
    let thumb = (h * shown as f32 / total as f32).max(10.0);
    let at = (h - thumb) * first as f32 / (total - shown) as f32;
    c.round_rect([x, y], [3.0, h], 1.5, rgba(0x2a2a38, 1.0), rgba(0, 0.0), 0.0);
    c.round_rect([x, y + at], [3.0, thumb], 1.5, rgba(accent(), 0.9), rgba(0, 0.0), 0.0);
}

/// Sends a play to the online leaderboards on a background thread.
fn submit_score(server: String, token: String, play: online::ScoreSubmit) -> Receiver<Result<online::Reply, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(online::call(&server, online::Request::SubmitScore { token, play: Box::new(play) }));
    });
    rx
}

/// "Artist - Title [Version]  1.10x  +HDHR  from 1:23" for a results screen.
fn results_title(m: &beatmap::Meta, from: Option<f64>) -> String {
    let part = from.map(|t| format!("  from {}", fmt_time(t))).unwrap_or_default();
    format!("{} - {} [{}]{part}", m.artist, m.title, m.version)
}

/// A results screen's background work (`ui::Detail`): the pp breakdown of `play` on the map at
/// `path`; with a replay file, the whole play's stats from it; with an online score id, its
/// replay fetched from `server` first (and saved, see `scores::save_online`).
/// Whether plays on library map `e` give pp (only ranked, approved and loved ones do, as on
/// osu!), from the mirrors' status else osu!.db's; None when it isn't known. The server has the
/// last word.
fn map_gives_pp(e: &Entry, set_status: &std::collections::HashMap<u32, Option<String>>, db_status: &std::collections::HashMap<PathBuf, u8>, server_loved: &std::collections::HashSet<u32>) -> Option<bool> {
    let Some(set) = e.meta.set_id else { return Some(false) };
    // Loved on the online server: it gives pp there.
    if server_loved.contains(&set) {
        return Some(true);
    }
    if let Some(Some(s)) = set_status.get(&set).filter(|s| s.as_deref() != Some(setstatus::MISSING)) {
        return Some(matches!(s.as_str(), "ranked" | "approved" | "loved"));
    }
    db_status.get(&e.path).filter(|&&s| s != 0).map(|s| matches!(s, 4 | 5 | 7))
}

fn results_job(path: PathBuf, play: pp::Play, local: Option<PathBuf>, online: Option<(String, online::OnlineScore, String)>) -> Receiver<ui::Detail> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut d = ui::Detail { breakdown: pp::breakdown(&path, &play), ..Default::default() };
        let replay = match (local, online) {
            (Some(file), _) => scores::load_replay(&file).ok(),
            (None, Some((server, score, label))) => match fetch_replay(&server, &score, &path, label) {
                Ok((file, r)) => {
                    d.replay = Some(Ok(file));
                    Some(r)
                }
                Err(e) => {
                    d.replay = Some(Err(e));
                    None
                }
            },
            (None, None) => None,
        };
        d.stats = replay.and_then(|r| scores::simulate(&path, &r));
        let _ = tx.send(d);
    });
    rx
}

/// Gets an online score's replay from the server and keeps it as a replay file of our own.
fn fetch_replay(server: &str, score: &online::OnlineScore, map_path: &std::path::Path, map_label: String) -> Result<(PathBuf, scores::Replay), String> {
    if score.id == 0 {
        return Err(tr!("the server doesn't keep replays yet", "このサーバーはまだリプレイを保存していません", "сервер пока не хранит повторы").into());
    }
    if let Some(file) = scores::online_replay_path(score.id).filter(|f| f.is_file())
        && let Ok(r) = scores::load_replay(&file) {
        return Ok((file, r));
    }
    let got = match online::call(server, online::Request::Replay { id: score.id }) {
        Ok(online::Reply::Replay(r)) => *r,
        Ok(_) => return Err(crate::lang::odd_reply()),
        // Servers from before replays don't know the request.
        Err(e) if e == "bad request" => return Err(tr!("the server doesn't hand out replays yet", "このサーバーはまだリプレイを配っていません", "сервер пока не отдаёт повторы").into()),
        Err(e) => return Err(e),
    };
    if got.version > scores::new_version() {
        return Err(tr!("replay from a newer version", "新しいバージョンのリプレイです", "повтор из более новой версии").into());
    }
    let replay = scores::Replay {
        version: got.version,
        map_hash: score.map_hash,
        map_path: map_path.to_string_lossy().into_owned(),
        map_label,
        player: score.player.clone(),
        date: score.date,
        rate: score.rate,
        keep_pitch: true,
        anti_mash: true,
        mods: score.mods.clone(),
        scroll_speed: score.scroll_speed.unwrap_or(1.0),
        score: score.score,
        accuracy: score.accuracy,
        max_combo: score.max_combo,
        great: score.great,
        ok: score.ok,
        miss: score.miss,
        ur: 0.0,
        inputs: got.inputs,
        seed: got.map.seed.unwrap_or(0),
    };
    let file = scores::save_online(score.id, &replay)?;
    Ok((file, replay))
}

/// Why there's no exporting videos.
fn no_ffmpeg() -> String {
    tr!(
        "ffmpeg wasn't found: install it to export (Windows: winget install ffmpeg; Linux: your package manager)",
        "ffmpegが見つかりません: 書き出すにはインストールしてください (Windows: winget install ffmpeg、Linux: パッケージマネージャー)",
        "ffmpeg не найден: установите его для экспорта (Windows: winget install ffmpeg; Linux: менеджер пакетов)"
    )
    .into()
}

/// A setting that's on.
fn on_text() -> &'static str {
    tr!("on", "オン", "вкл")
}

fn off_text() -> &'static str {
    tr!("off", "オフ", "выкл")
}

/// No key, no choice.
fn none_text() -> &'static str {
    tr!("none", "なし", "нет")
}

fn picking_text() -> &'static str {
    tr!("picking...", "選択中...", "выбор...")
}

fn enter_to_start() -> &'static str {
    tr!("Enter to start", "Enterで開始", "Enter — начать")
}

/// taiko.rs's own skin.
fn built_in_text() -> &'static str {
    tr!("built-in", "内蔵", "встроенный")
}

fn scroll_in_select() -> &'static str {
    tr!("change the scroll speed in song select, not during a song", "スクロール速度は曲中ではなく選曲画面で変えてください", "меняйте скорость прокрутки в выборе песни, не в песне")
}

/// The list's choice to type a folder's path.
fn type_path() -> String {
    tr!("Type a path", "パスを入力", "Ввести путь").into()
}

/// The list's choice to pick a folder in the system's dialog.
fn browse_text() -> String {
    tr!("Browse...", "参照...", "Обзор...").into()
}

/// The badge for an update to install.
fn update_badge(commit: &str) -> String {
    let build = update::short(commit);
    tr!("update {build}  Ctrl+U", "アップデート {build}  Ctrl+U", "обновить {build}  Ctrl+U";)
}

/// The badge while the update downloads (`p` 0..1).
fn updating_badge(p: f32) -> String {
    let p = p * 100.0;
    tr!("updating {p:.0}%", "更新中 {p:.0}%", "обновление {p:.0}%";)
}

/// Fetches a map's online leaderboard on a background thread.
fn fetch_board(server: String, map: u64) -> Receiver<Result<Vec<online::OnlineScore>, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let rows = online::call(&server, online::Request::MapScores { map }).and_then(|r| match r {
            online::Reply::Scores(rows) => Ok(rows),
            _ => Err(crate::lang::odd_reply()),
        });
        let _ = tx.send(rows);
    });
    rx
}

/// Opens a web page in the browser; false where that can't be done.
fn open_url(url: &str) -> bool {
    if cfg!(target_os = "android") {
        return false;
    }
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                verb: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> *mut std::ffi::c_void;
        }
        const SW_SHOWNORMAL: i32 = 1;
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (verb, file) = (wide("open"), wide(url));
        // SAFETY: both strings are nul-terminated and live through the call; the rest may be null.
        let result = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        // Above 32 is success, the rest are error codes.
        result as usize > 32
    }
    #[cfg(not(windows))]
    std::process::Command::new("xdg-open").arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().is_ok()
}

/// `file://` URI for a local path.
#[cfg(not(windows))]
fn file_uri(path: &std::path::Path) -> String {
    let mut uri = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// When each map was last played in taiko.rs (`plays.tsv`: Unix seconds, tab, path).
fn load_plays() -> std::collections::HashMap<PathBuf, u64> {
    let mut out = std::collections::HashMap::new();
    let Some(text) = config::data_dir().and_then(|d| std::fs::read_to_string(d.join("plays.tsv")).ok()) else { return out };
    for line in text.lines() {
        if let Some((t, path)) = line.split_once('\t')
            && let Ok(t) = t.parse::<u64>() {
            let slot = out.entry(PathBuf::from(path)).or_insert(0);
            *slot = (*slot).max(t);
        }
    }
    out
}

/// The library entry for a lobby map: same artist/title/difficulty, then the exact file hash.
/// Only those few candidates are hashed, so this stays cheap with a huge library.
/// A path for a settings row: its last two folders.
fn short_path(dir: &std::path::Path) -> String {
    let parts: Vec<_> = dir.components().collect();
    if parts.len() <= 3 {
        return dir.display().to_string();
    }
    let tail: PathBuf = parts[parts.len() - 2..].iter().collect();
    format!("...{}{}", std::path::MAIN_SEPARATOR, tail.display())
}

/// The game's data on its way to another folder: the result, bytes moved of all, and where to.
struct DataMove {
    rx: Receiver<Result<(), String>>,
    done: Arc<std::sync::atomic::AtomicU64>,
    total: Arc<std::sync::atomic::AtomicU64>,
    to: Option<PathBuf>,
}

/// One view of a video frame to render offscreen: its size, where the frame goes, the
/// background's brightness, and with motion blur whether it is blended and the frame's last.
struct ExportView {
    size: (u32, u32),
    tx: std::sync::mpsc::SyncSender<Vec<u8>>,
    brightness: f32,
    /// What's under the background (`App::backdrop`).
    clear: [f64; 3],
    blended: bool,
    last: bool,
}

/// Where downloaded maps go: the first Songs folder being scanned, else taiko.rs's own (which
/// is scanned at startup when it exists).
fn download_dir(inputs: &[PathBuf]) -> PathBuf {
    if let Some(dir) = inputs.iter().find(|p| p.is_dir()) {
        return dir.clone();
    }
    config::own_dir("Songs").unwrap_or_else(|| PathBuf::from("Songs"))
}

/// Marks the fullscreen preview as a preview: a blue frame and badge on the playfield area, and
/// a header with the map and the controls.
fn draw_preview_chrome(c: &mut Canvas, cfg: &Config, e: &Entry, stars: Option<f32>, lobby: bool, size_label: &str) {
    let blue = preview::MARKER;
    let a = cfg.playfield_area;
    let (ax, ay, aw, ah) = (a.x * c.width, a.y * render::VIRTUAL_HEIGHT, a.w * c.width, a.h * render::VIRTUAL_HEIGHT);
    c.round_rect([ax + 1.0, ay + 1.0], [aw - 2.0, ah - 2.0], 4.0, rgba(0, 0.0), rgba(blue, 0.55), 1.5);
    let badge = tr!("AUTOPLAY PREVIEW", "オートのプレビュー", "ПРЕВЬЮ АВТОПЛЕЯ");
    let bw = Canvas::text_width(7.0, badge) + 16.0;
    // Below the header when the area reaches the top of the screen.
    let by = (ay + 8.0).max(52.0);
    c.round_rect([ax + 8.0, by], [bw, 16.0], 8.0, rgba(blue, 0.85), rgba(0, 0.0), 0.0);
    c.text(ax + 16.0, by + 4.5, 7.0, rgba(0x0c1a28, 1.0), badge);

    c.round_rect([24.0, 10.0], [c.width - 48.0, 34.0], 8.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
    let tag = tr!("PREVIEW", "プレビュー", "ПРЕВЬЮ");
    let tag_w = Canvas::text_width(8.0, tag) + 18.0;
    c.round_rect([32.0, 18.0], [tag_w, 18.0], 9.0, rgba(0x173049, 1.0), rgba(blue, 1.0), 1.0);
    c.text(41.0, 23.0, 8.0, rgba(0xffffff, 1.0), tag);
    let pill = stars::draw_pill(c, c.width - 36.0, 22.0, 8.0, stars);
    let back = if lobby { tr!("Lobby  (Esc)", "ロビー  (Esc)", "Лобби  (Esc)") } else { tr!("Back  (Esc)", "戻る  (Esc)", "Назад  (Esc)") };
    let bw = Canvas::text_width(8.0, back) + 24.0;
    let bx = c.width - 44.0 - pill - bw;
    c.button(bx, 17.0, bw, 20.0, back, 8.0, render::Hit::Key(KeyCode::Escape), false);
    // Right to left: the size button, the hints and the title share what's left. On a narrow
    // screen the hints go first, then the size button gets shorter, so the title keeps TITLE_MIN.
    const TITLE_MIN: f32 = 120.0;
    let title_x = 32.0 + tag_w + 10.0;
    let short_size = size_label.split(" (").next().unwrap_or(size_label);
    let presets = [format!("{size_label}  (F5/F6)"), size_label.to_string(), short_size.to_string()]
        .into_iter()
        .find(|p| bx - 8.0 - (Canvas::text_width(8.0, p) + 24.0) - title_x - 12.0 >= TITLE_MIN)
        .unwrap_or_else(|| short_size.to_string());
    let pw = Canvas::text_width(8.0, &presets) + 24.0;
    let px = bx - 8.0 - pw;
    c.button(px, 17.0, pw, 20.0, &presets, 8.0, render::Hit::Presets, false);
    let hints = if lobby {
        tr!("drag / wheel: playfield", "ドラッグ / ホイール: プレイフィールド", "тяните / колесо: поле")
    } else {
        tr!("Enter play   drag / wheel: playfield", "Enter プレイ   ドラッグ / ホイール: プレイフィールド", "Enter играть   тяните / колесо: поле")
    };
    let mut title_end = px - 12.0;
    if title_end - Canvas::text_width(7.0, hints) - 12.0 - title_x >= TITLE_MIN + 60.0 {
        c.text_right(title_end, 24.0, 7.0, rgba(0x8a8aa0, 1.0), hints);
        title_end -= Canvas::text_width(7.0, hints) + 12.0;
    }
    let title = format!("{} - {} [{}]", e.meta.artist, e.meta.title, e.meta.version);
    c.text(title_x, 22.0, 10.0, rgba(0xffffff, 1.0), &Canvas::fit(10.0, title_end - title_x, &title));
}

/// Of a set's maps, the ones to show: its taiko difficulties, else (a set of converts) all.
fn main_maps<'a>(maps: impl Iterator<Item = &'a Entry>) -> Vec<PathBuf> {
    let maps: Vec<&Entry> = maps.collect();
    let taiko = maps.iter().any(|e| e.meta.mode == 1);
    maps.into_iter().filter(|e| !taiko || e.meta.mode == 1).map(|e| e.path.clone()).collect()
}

/// How a map is named to other players (lobby map, a solo play friends watch), with our
/// settings of the mods that have them.
fn map_info(e: &Entry, rate: f64, mods: Mods) -> net::MapInfo {
    let (multiverse, shapeshift) = mod_settings(mods);
    net::MapInfo {
        mods: mods.list(),
        multiverse,
        shapeshift,
        seed: None,
        hash: e.hash(),
        artist: e.meta.artist.clone(),
        title: e.meta.title.clone(),
        version: e.meta.version.clone(),
        rate,
        from: None,
        set_id: e.meta.set_id.or_else(|| e.path.parent().and_then(download::set_id_of_folder)),
    }
}

/// A new seed for Random's colouring (never 0, the one from before seeds).
fn fresh_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64).max(1)
}

/// The Random colouring a lobby map plays with: the one it has, else a new one. Every map
/// carries one, Random being each player's own mod: whoever plays it gets the same colouring,
/// and watching everyone shows each player's.
fn lobby_seed(had: Option<u64>) -> Option<u64> {
    Some(had.unwrap_or_else(fresh_seed))
}

/// Our settings of the mods in `mods` that have them (Multiverse's universes, Shapeshift's
/// setup), as the config writes them: what a lobby we host plays with.
fn mod_settings(mods: Mods) -> (Option<String>, Option<String>) {
    let universes = mods.has(mods::Mod::Multiverse).then(|| multiverse::format(&multiverse::get()));
    let shapes = mods.has(mods::Mod::Shapeshift).then(|| shapeshift::get().format());
    (universes, shapes)
}

/// Plays `game` with the mod settings `map` carries (the lobby host's, or the friend's being
/// watched) where it has them; ours otherwise. Our config and presets aren't touched.
fn use_mod_settings(game: &mut Game, map: &net::MapInfo) {
    game.use_mod_settings(map.multiverse.as_deref().map(multiverse::parse), map.shapeshift.as_deref().map(shapeshift::Config::parse));
}

/// The library entry for a map someone else named: the file's content hash identifies it (the
/// title can differ, e.g. romanised on one side and Cyrillic on the other).
fn find_map(library: &[Entry], m: &net::MapInfo) -> Option<usize> {
    library.iter().position(|e| e.meta.version == m.version && e.hash() == m.hash)
}

/// A score a `taiko-rs://` link asked for: being fetched from the server, then waiting for its
/// map (the library scan, or a download) and for the player to be out of a song.
struct ScoreLink {
    fetch: Option<Receiver<Result<online::OnlineScore, String>>>,
    score: Option<online::OnlineScore>,
    /// Its set is downloading, since then.
    downloading: Option<Instant>,
}

/// How long a score link waits for its map's download.
const SCORE_LINK_WAIT: Duration = Duration::from_secs(300);

/// The map an online score was played on: only maps of its set or with its name are hashed
/// (hashing reads the file).
fn find_score_map(library: &[Entry], s: &online::OnlineScore) -> Option<usize> {
    library.iter().position(|e| {
        let candidate = match (s.set_id, e.meta.set_id) {
            (Some(a), Some(b)) => a == b,
            _ => online::clean_text(&e.label(), 160) == s.map,
        };
        candidate && e.hash() == s.map_hash
    })
}

/// A character typed into a playfield size: a digit, or `x` between width and height.
fn size_char(code: KeyCode) -> Option<char> {
    use KeyCode::*;
    let digits = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
    let numpad = [Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9];
    if let Some(d) = digits.iter().chain(numpad.iter()).position(|&k| k == code) {
        return char::from_digit(d as u32 % 10, 10);
    }
    matches!(code, KeyX | Space | NumpadMultiply).then_some('x')
}

const AUDIO_BUFFERS: [u32; 6] = [64, 128, 256, 512, 1024, 2048];

/// The smallest the window goes (logical pixels): smaller, the menus no longer fit.
const MIN_WINDOW: (f64, f64) = (640.0, 360.0);

/// Frames on their way to the screen with vsync (the swapchain's frame latency for it, see
/// `Renderer::set_present`).
const VSYNC_FRAMES: f64 = 2.0;

/// A typed setting value as a number: units after it ("ms", "%", "x") are fine, a comma works
/// as the decimal point, and "off" / "unlimited" are 0.
fn parse_number(text: &str) -> Option<f64> {
    let t = text.trim().to_ascii_lowercase();
    if ["off", "unlimited", "none"].contains(&t.as_str()) {
        return Some(0.0);
    }
    let num: String = t.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '+' | '-')).map(|c| if c == ',' { '.' } else { c }).collect();
    num.parse().ok().filter(|n: &f64| n.is_finite())
}

/// What to scan without arguments: the Songs folder (picked in settings, else osu!'s), plus the
/// config's extra `songs =` lines.
fn song_inputs(cfg: &Config) -> Vec<PathBuf> {
    let mut inputs = library::song_dirs();
    inputs.extend(cfg.songs.iter().cloned());
    inputs
}

/// Scans `inputs` on a background thread so the window appears at once, even with a huge
/// library.
fn start_scan(inputs: Vec<PathBuf>) -> (Receiver<Vec<Entry>>, Arc<AtomicUsize>) {
    let found = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel();
    let counter = found.clone();
    std::thread::Builder::new()
        .name("library".into())
        .spawn(move || {
            let started = Instant::now();
            let library = library::scan(&inputs, &counter);
            eprintln!("scanned {} files in {:.0} ms", counter.load(Ordering::Relaxed), started.elapsed().as_secs_f64() * 1000.0);
            let _ = tx.send(library);
        })
        .expect("spawn library scan");
    (rx, found)
}

impl State {
    fn toast(&mut self, text: String) {
        self.toast = Some(Toast { text, until: Instant::now() + Duration::from_millis(1500) });
    }

    /// Scenes that use the gameplay present mode; everything else is a menu.
    /// In a song (or its pause menu's settings): what counts for pp stays as it started.
    fn mid_song(&self) -> bool {
        self.in_song() || matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Play { .. } | Scene::MpPlay { .. }))
    }

    fn in_song(&self) -> bool {
        matches!(self.scene, Scene::Play { .. } | Scene::MpPlay { .. } | Scene::MpWait { .. } | Scene::Calibrate { .. } | Scene::Spectate(_) | Scene::Watch(_))
    }

    /// The frame rate cap (0 = none). The pause menu gets no more frames than the screen shows:
    /// uncapped, its near-empty frame runs at thousands of fps and the GPU whines.
    ///
    /// Without the focus (another window, the Snipping Tool's overlay) it's 30: an uncapped game
    /// takes the whole GPU, and the screenshot tool then needs seconds to start.
    fn frame_cap(&self) -> u32 {
        if !self.focused && (self.cfg.fps_cap == 0 || self.cfg.fps_cap > UNFOCUSED_FPS) {
            return UNFOCUSED_FPS;
        }
        let menu = matches!(&self.scene, Scene::Play { game, .. } | Scene::MpPlay { game, .. } | Scene::Calibrate { game, .. } if game.paused.is_some() && game.resuming.is_none());
        let screen = (self.refresh_hz.round() as u32).max(30);
        if menu && (self.cfg.fps_cap == 0 || self.cfg.fps_cap > screen) { screen } else { self.cfg.fps_cap }
    }

    /// The present mode this frame should use.
    fn wanted_present(&self) -> PresentChoice {
        // Watching someone else play needs no low input latency: always vsync (smoothest).
        if matches!(self.scene, Scene::Spectate(_) | Scene::Watch(_) | Scene::Calibrate { vsync: true, .. }) {
            return PresentChoice::Fifo;
        }
        if self.cfg.menu_vsync && !self.player_plays() { PresentChoice::Fifo } else { self.cfg.present }
    }

    /// The player is hitting notes (the gameplay present mode's job): not a replay or autoplay,
    /// not waiting for a multiplayer match to start.
    fn player_plays(&self) -> bool {
        match &self.scene {
            Scene::Play { game, .. } => game.replay.is_none() && !self.auto && !game.mods.has(mods::Mod::Autoplay),
            Scene::MpPlay { .. } | Scene::Calibrate { .. } => true,
            _ => false,
        }
    }

    /// How far ahead (real ms) notes are drawn for vsync: with it, a frame reaches the screen
    /// `VSYNC_FRAMES` refreshes after it's drawn (unlimited fps shows one at once), plus the
    /// player's fine tune. 0 without vsync or with the compensation off.
    fn vsync_lead(&self) -> f64 {
        if !self.cfg.vsync_compensation || self.present != PresentChoice::Fifo {
            return 0.0;
        }
        VSYNC_FRAMES * self.refresh_ms() + self.cfg.vsync_tune_ms
    }

    /// The screen's refresh interval (ms): as the frames came with vsync, else as the window
    /// system says.
    fn refresh_ms(&self) -> f64 {
        self.seen_refresh_ms.unwrap_or(1000.0 / self.refresh_hz.clamp(24.0, 1000.0))
    }

    fn stop_audio(&mut self) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        self.preview.invalidate_audio();
    }

    /// The preview's decoded song, when it was prepared at the speed `entry` is about to play at.
    fn cached_track(&self, entry: usize, rate: f64, keep_pitch: bool) -> Option<Arc<Track>> {
        self.preview.cached_track(&self.library[entry], rate, keep_pitch)
    }

    /// Speed and pitch of a play started now: the selected speed, or what DT/HT/NC/DC force.
    fn play_speed(&self) -> (f64, bool) {
        self.cfg.mods.speed(self.cfg.rate, self.cfg.rate_keep_pitch)
    }

    /// Loads `entry` for a play at `rate` with `mods` (`keep_pitch` unless NC/DC say otherwise).
    fn load_with(&mut self, entry: usize, rate: f64, mods: Mods) -> Receiver<Loaded> {
        let (rate, keep_pitch) = mods.speed(rate, self.cfg.rate_keep_pitch);
        let cached = self.cached_track(entry, rate, keep_pitch);
        self.pending_mods = mods;
        self.spawn_loader(entry, rate, keep_pitch, mods, cached)
    }

    /// Speed the preview plays at: the match rate for the lobby map, else the selected rate.
    fn preview_rate(&self) -> (f64, bool) {
        let lobby_rate = self.mp.as_ref().and_then(|s| s.map.as_ref().filter(|_| s.map_entry.is_some() && s.map_entry == self.preview_target()).map(|m| m.rate));
        let lobby = matches!(self.scene, Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults | Scene::Spectate(_))
            || matches!(&self.scene, Scene::Preview { back } | Scene::Settings { prev: back } if matches!(**back, Scene::Lobby));
        if let Some(p) = &self.player {
            return p.rate();
        }
        let (rate, keep_pitch) = self.play_speed();
        (lobby_rate.filter(|_| lobby).unwrap_or(rate), keep_pitch)
    }

    /// The playfield area's size in pixels.
    fn playfield_size(&self) -> (u32, u32) {
        let a = self.cfg.playfield_area;
        ((a.w * self.window_w).round() as u32, (a.h * self.window_h).round() as u32)
    }

    /// "1280x720 (full screen)": the playfield's size in pixels.
    fn playfield_label(&self) -> String {
        let a = self.cfg.playfield_area;
        let (pw, ph) = self.playfield_size();
        let kind = if a.is_full() {
            tr!(" (full screen)", " (全画面)", " (весь экран)")
        } else if self.current_preset().is_some() {
            ""
        } else {
            tr!(" (custom)", " (カスタム)", " (свой)")
        };
        format!("{pw}x{ph}{kind}")
    }

    fn start_loading(&mut self, entry: usize) {
        self.start_loading_from(entry, None);
    }

    /// The video editor for a saved replay.
    fn open_video_editor(&mut self, file: &std::path::Path) {
        let r = match scores::load_replay(file) {
            Ok(r) => r,
            Err(e) => {
                self.toast(tr!("can't open the replay: {e}", "リプレイを開けません: {e}", "не удаётся открыть повтор: {e}";));
                return;
            }
        };
        let entry = self
            .library
            .iter()
            .position(|e| e.path.to_string_lossy() == r.map_path && e.hash() == r.map_hash)
            .or_else(|| self.library.iter().position(|e| e.label() == r.map_label && e.hash() == r.map_hash));
        let Some(entry) = entry else {
            let map = &r.map_label;
            self.toast(tr!("{map} isn't in your songs", "{map}は曲の中にありません", "{map} нет среди ваших песен";));
            return;
        };
        self.stop_audio();
        self.preview.show_background_for(&self.library[entry]);
        let rx = self.load_with(entry, r.rate, Mods::from_list(&r.mods));
        self.video_status = if video::find_ffmpeg().is_some() { String::new() } else { no_ffmpeg() };
        self.last_video = None;
        self.scene = Scene::VideoEdit(Box::new(video::Editor::new(r, rx, self.cfg.video)));
    }

    fn video_key(&mut self, code: KeyCode, repeat: bool, window: Option<&Window>) {
        if self.export.is_some() {
            if code == KeyCode::Escape && !repeat {
                // Ending ffmpeg's input (or stopping a pass) ends it early; the encoder removes
                // what it made.
                if let Some(ex) = self.export.take() {
                    ex.encoder.cancel.store(true, Ordering::Relaxed);
                }
                self.video_status = tr!("export cancelled", "書き出しを取り消しました", "экспорт отменён").into();
            }
            return;
        }
        let Scene::VideoEdit(ed) = &mut self.scene else { return };
        // Typing the target size takes the keys until Enter or Esc.
        if let Some(text) = &mut ed.typing {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    match text.parse::<f64>().ok().filter(|mb| mb.is_finite() && *mb >= 1.0) {
                        Some(mb) => {
                            ed.settings.target_mb = (mb.min(4000.0) * 10.0).round() / 10.0;
                            ed.typing = None;
                        }
                        None => self.video_status = tr!("type a size of at least 1 MB, like 10 or 25", "1 MB以上のサイズを入力 (10や25など)", "введите размер от 1 МБ, например 10 или 25").into(),
                    }
                }
                KeyCode::Escape => ed.typing = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::KeyQ => {
                    ed.typing = None;
                    ed.settings.quality = 0;
                }
                _ => {
                    let ch = match code {
                        KeyCode::Period | KeyCode::Comma | KeyCode::NumpadDecimal | KeyCode::NumpadComma => Some('.'),
                        _ => size_char(code).filter(|c| c.is_ascii_digit()),
                    };
                    if let Some(ch) = ch.filter(|_| text.len() < 6) {
                        text.push(ch);
                    }
                }
            }
            if ed.typing.is_none() {
                self.video_status.clear();
            }
            self.cfg.video = ed.settings;
            self.cfg.save();
            return;
        }
        let rate = ed.replay.rate;
        let step = if self.modifiers_shift { 5000.0 } else { 1000.0 } * rate;
        // Shift steps back through a setting's choices.
        let dir = if self.modifiers_shift { -1 } else { 1 };
        let cycle = |i: usize, n: usize| (i as i32 + dir).rem_euclid(n as i32) as usize;
        let before = ed.settings;
        match code {
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let t = ed.cursor + if code == KeyCode::ArrowRight { step } else { -step };
                ed.playing = None;
                if let Some(a) = &self.audio {
                    a.stop();
                }
                ed.seek(t);
            }
            _ if repeat => {}
            KeyCode::Space => {
                if ed.playing.take().is_some() {
                    if let Some(a) = &self.audio {
                        a.stop();
                    }
                } else if let (Some((_, track)), Some(a)) = (&ed.map, &self.audio) {
                    let at = ed.game.as_ref().map(|g| g.track_at(ed.cursor)).unwrap_or(ed.cursor / rate);
                    a.play(track.clone(), at);
                    ed.playing = Some((Instant::now(), ed.cursor));
                }
            }
            KeyCode::KeyI if ed.cursor < ed.end => ed.start = ed.cursor,
            KeyCode::KeyO if ed.cursor > ed.start => ed.end = ed.cursor,
            KeyCode::KeyC => {
                let presets = video::crop_presets();
                let i = presets.iter().position(|(n, _)| *n == ed.crop_name).map(|i| i + 1).unwrap_or(0) % presets.len();
                (ed.crop_name, ed.crop) = presets[i];
            }
            KeyCode::KeyZ => ed.settings.size = cycle(ed.settings.size, video::SIZES.len()),
            KeyCode::KeyF => ed.settings.fps = cycle(ed.settings.fps, video::FRAME_RATES.len()),
            KeyCode::KeyB => ed.settings.blur = cycle(ed.settings.blur, video::BLURS.len()),
            KeyCode::KeyQ => ed.settings.quality = cycle(ed.settings.quality, video::QUALITIES.len()),
            KeyCode::KeyT => {
                // Aiming at a size, and typing it.
                ed.settings.quality = video::QUALITIES.len() - 1;
                ed.typing = Some(String::new());
            }
            KeyCode::KeyE => ed.settings.speed = cycle(ed.settings.speed, video::SPEEDS.len()),
            KeyCode::KeyU => ed.settings.hud = !ed.settings.hud,
            KeyCode::KeyH => ed.settings.hitsounds = !ed.settings.hitsounds,
            KeyCode::Enter | KeyCode::NumpadEnter => self.start_export(),
            KeyCode::KeyS => {
                if let Some(path) = self.last_video.clone() {
                    self.show_in_folder(&path);
                }
            }
            KeyCode::Escape => {
                self.show_select();
            }
            _ => {
                self.handle_global_key(code, window);
            }
        }
        // Remembered for the next video.
        if let Scene::VideoEdit(ed) = &self.scene
            && ed.settings != before
        {
            self.video_status.clear();
            self.cfg.video = ed.settings;
            self.cfg.save();
        }
    }

    /// Starts rendering the editor's cut to an MP4.
    fn start_export(&mut self) {
        let Scene::VideoEdit(ed) = &mut self.scene else { return };
        let Some((map, track)) = &ed.map else { return };
        let Some(ffmpeg) = video::find_ffmpeg() else {
            self.video_status = no_ffmpeg();
            return;
        };
        ed.playing = None;
        if let Some(a) = &self.audio {
            a.stop();
        }
        let sr = self.audio.as_ref().map(|a| a.rate).unwrap_or(48000);
        // The hitsounds as they were judged, and the music, for the cut.
        let mut sim = video::replay_game(map, track, &ed.replay);
        sim.sfx_log = Some(Vec::new());
        sim.set_time(ed.end);
        let sfx: Vec<(f64, audio::Sfx, f32)> = if ed.settings.hitsounds { sim.sfx_log.take().unwrap_or_default() } else { Vec::new() };
        let sounds = audio::hitsound_set(sr, skin::hitsounds(&self.cfg.skin, &self.cfg.skin_parts, sr));
        let (music, fx) = self.cfg.volumes();
        let speed = sim.speed();
        let samples = video::mix_audio(track, sr, &speed, ed.start, ed.end, music.max(0.05), &sfx, &sounds, fx);
        let Some(dir) = video::videos_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let wav = dir.join(format!(".audio-{}.wav", std::process::id()));
        if let Err(e) = video::write_wav(&wav, &samples, sr) {
            self.video_status = format!("can't write the audio: {e}");
            return;
        }
        let out = video::free_path(&dir, &video::file_name(&ed.replay));
        let settings = ed.settings;
        let size = video::SIZES[settings.size];
        let fps = video::FRAME_RATES[settings.fps];
        // Frames are evenly spaced in song time (what the video's audio follows).
        let step = 1000.0 / fps as f64;
        let (t0, t1) = (speed.track(ed.start), speed.track(ed.end));
        let frames = ((t1 - t0) / step).ceil().max(1.0) as u64;
        let job = video::Job { size, fps, crop: ed.crop_px(), settings, seconds: frames as f64 / fps as f64, wav: wav.clone(), out: out.clone() };
        match video::start_encoder(&ffmpeg, job) {
            Ok(encoder) => {
                let mut game = Box::new(video::replay_game(map, track, &ed.replay));
                game.set_time(ed.start);
                self.export = Some(video::Export { game, t: t0, step, t0, sub: 0, settings, size, encoder, frame: 0, frames });
                self.video_status = String::new();
            }
            Err(e) => {
                let _ = std::fs::remove_file(&wav);
                self.video_status = e;
            }
        }
    }

    /// Render thread: the shapes of the next view of a video frame (one, or several blended
    /// with motion blur); None when there's nothing (more) to render.
    fn export_frame(&mut self, c: &mut Canvas) -> Option<ExportView> {
        let scroll = self.cfg.scroll_speed;
        let kiai = self.cfg.kiai_effects;
        let brightness = if self.preview.has_background { 1.0 - self.cfg.bg_dim } else { 0.0 };
        let ex = self.export.as_mut()?;
        if ex.frame >= ex.frames {
            // Closing ffmpeg's input lets it finish the file.
            ex.encoder.tx = None;
            return None;
        }
        let tx = ex.encoder.tx.clone()?;
        let (w, h) = ex.size;
        ex.game.kiai_effects = kiai;
        let t = ex.game.speed().map(ex.view_time());
        ex.game.set_time(t);
        c.clear(render::VIRTUAL_HEIGHT * w as f32 / h as f32);
        ex.game.draw_playfield(c, scroll);
        if ex.settings.hud {
            ex.game.draw_hud(c);
        }
        let blended = ex.views() > 1;
        let last = ex.advance();
        Some(ExportView { size: (w, h), tx, brightness, clear: self.backdrop(true), blended, last })
    }

    /// Loads the map of a saved replay and plays it back.
    fn watch_replay(&mut self, file: &std::path::Path) {
        let r = match scores::load_replay(file) {
            Ok(r) => r,
            Err(e) => {
                self.toast(tr!("can't open the replay: {e}", "リプレイを開けません: {e}", "не удаётся открыть повтор: {e}";));
                return;
            }
        };
        // The same file where it was, else any map with the same contents.
        let entry = self
            .library
            .iter()
            .position(|e| e.path.to_string_lossy() == r.map_path && e.hash() == r.map_hash)
            .or_else(|| self.library.iter().position(|e| e.label() == r.map_label && e.hash() == r.map_hash));
        let Some(entry) = entry else {
            let map = &r.map_label;
            self.toast(tr!("{map} isn't in your songs", "{map}は曲の中にありません", "{map} нет среди ваших песен";));
            return;
        };
        self.stop_audio();
        self.preview.show_background_for(&self.library[entry]);
        let (player, map) = (&r.player, self.library[entry].label());
        let label = tr!("replay of {player}: {map}", "{player}のリプレイ: {map}", "повтор {player}: {map}";);
        let rx = self.load_with(entry, r.rate, Mods::from_list(&r.mods));
        self.pending_replay = Some(r);
        self.scene = Scene::Loading { rx, label, entry, from: None };
    }

    /// Song select's layout as it comes: the list's share and the preview's size.
    fn song_layout_is_default(&self) -> bool {
        let d = Config::default();
        (self.cfg.song_list_share - d.song_list_share).abs() < 1e-4 && (self.cfg.song_preview_size - d.song_preview_size).abs() < 1e-4
    }

    /// Puts song select's divider `which` (both: None) back where it was at first.
    fn reset_song_layout(&mut self, which: Option<render::Hit>) {
        let d = Config::default();
        if which != Some(render::Hit::SplitPreview) {
            self.cfg.song_list_share = d.song_list_share;
        }
        if which != Some(render::Hit::SplitList) {
            self.cfg.song_preview_size = d.song_preview_size;
        }
        self.cfg.save();
    }

    /// The results screen of a local score on the selected map (its leaderboard row).
    fn open_local_score(&mut self, file: &std::path::Path) {
        let Some(entry) = self.select.selected() else { return };
        let r = match scores::load_replay(file) {
            Ok(r) => r,
            Err(e) => {
                self.toast(tr!("can't open the replay: {e}", "リプレイを開けません: {e}", "не удаётся открыть повтор: {e}";));
                return;
            }
        };
        let stats = game::Stats { great: r.great, ok: r.ok, miss: r.miss, max_combo: r.max_combo, score: r.score, ..Default::default() };
        let mods = Mods::from_list(&r.mods);
        let e = &self.library[entry];
        let mut results = Results::new(results_title(&e.meta, None), stats, game::Windows::new(e.meta.od));
        results.set_mods(mods, r.rate);
        results.replay = Some(file.to_path_buf());
        results.partial = true;
        results.pp = self.scores.iter().find(|row| row.file == file).and_then(|row| row.pp);
        results.no_pp = mods.unranked().then_some(tr!("Autoplay and Relax get no pp", "AutoplayとRelaxではppは付きません", "Autoplay и Relax не дают pp"));
        results.opened = Some(ui::Opened { player: r.player.clone(), date: r.date, online: false });
        let all = scores::top(&self.scores, r.map_hash, usize::MAX);
        results.rank = all.iter().position(|row| row.file == file).map(|p| (p + 1, all.len()));
        results.job = Some(results_job(e.path.clone(), r.pp_play(), Some(file.to_path_buf()), None));
        self.scene = Scene::Results { results, entry, from: None };
    }

    /// Opens what a `taiko-rs://` link asks for: a score's results, its replay ready to watch.
    fn open_link(&mut self, link: link::Link) {
        self.main_menu = None;
        match link {
            link::Link::Score(id) => {
                let server = self.cfg.online_server.clone();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let got = match online::call(&server, online::Request::Score { id }) {
                        Ok(online::Reply::Score(s)) => Ok(*s),
                        Ok(_) => Err(crate::lang::odd_reply()),
                        Err(e) if e == "bad request" => Err(tr!("the server can't open score links yet", "このサーバーはまだスコアのリンクを開けません", "сервер пока не открывает ссылки на рекорды").to_string()),
                        Err(e) => Err(e),
                    };
                    let _ = tx.send(got);
                });
                self.score_link = Some(ScoreLink { fetch: Some(rx), score: None, downloading: None });
                let later = if self.in_song() || self.mp.is_some() {
                    tr!(" (it opens once you're back in song select)", " (選曲に戻ると開きます)", " (откроется, когда вернётесь к выбору песни)")
                } else {
                    ""
                };
                self.toast(tr!("opening a score{later}...", "スコアを開いています{later}...", "открываем рекорд{later}...";));
            }
        }
    }

    /// Opens a linked score's results once it's here, its map is (downloading it if need be)
    /// and the player is in song select.
    fn poll_score_link(&mut self) {
        let Some(l) = &mut self.score_link else { return };
        if let Some(rx) = &l.fetch {
            match rx.try_recv() {
                Ok(Ok(s)) => {
                    l.score = Some(s);
                    l.fetch = None;
                }
                Ok(Err(e)) => {
                    self.score_link = None;
                    self.toast(tr!("can't open that score: {e}", "そのスコアを開けません: {e}", "не удаётся открыть рекорд: {e}";));
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.score_link = None;
                    return;
                }
            }
        }
        let free = matches!(self.scene, Scene::Select | Scene::Results { .. } | Scene::Preview { .. } | Scene::Download) && self.mp.is_none();
        if self.scan.is_some() || !free {
            return;
        }
        let Some(score) = self.score_link.as_ref().and_then(|l| l.score.clone()) else { return };
        if let Some(entry) = find_score_map(&self.library, &score) {
            self.score_link = None;
            self.scene = Scene::Select;
            let path = self.library[entry].path.clone();
            self.select.select_path(&self.library, &path);
            if self.select.selected() != Some(entry) {
                // Hidden by the search: show everything.
                self.select.filter.clear();
                self.select.refilter(&self.library);
                self.select.select_path(&self.library, &path);
            }
            self.open_online_score_on(entry, score);
            return;
        }
        let downloading = self.score_link.as_ref().and_then(|l| l.downloading);
        match (downloading, score.set_id) {
            (Some(since), _) => {
                if since.elapsed() > SCORE_LINK_WAIT && !self.downloads.iter().any(|j| Some(j.set.id) == score.set_id) {
                    self.score_link = None;
                }
            }
            (None, Some(id)) if self.installed_sets().contains(&id) => {
                self.score_link = None;
                let map = &score.map;
                self.toast(tr!(
                    "your copy of {map} is a different version than the one played",
                    "あなたの{map}はプレイされたものと別のバージョンです",
                    "ваша {map} — другая версия, не та, что играли";
                ));
            }
            (None, Some(id)) => {
                let song = score.map.rsplit_once(" [").map_or(score.map.as_str(), |(s, _)| s);
                let (artist, title) = song.split_once(" - ").unwrap_or(("", song));
                self.start_download(download::SetInfo {
                    id,
                    artist: artist.to_string(),
                    title: title.to_string(),
                    creator: String::new(),
                    status: String::new(),
                    diffs: Vec::new(),
                    play_count: 0,
                    favourite_count: 0,
                    rating: 0.0,
                    ranked_date: String::new(),
                });
                if let Some(l) = &mut self.score_link {
                    l.downloading = Some(Instant::now());
                }
                self.toast(tr!("downloading {song} for that score...", "そのスコアのため{song}をダウンロード中...", "скачиваем {song} для этого рекорда...";));
            }
            (None, None) => {
                self.score_link = None;
                let map = &score.map;
                self.toast(tr!("you haven't got {map}", "{map}を持っていません", "у вас нет {map}";));
            }
        }
    }

    /// The results screen of an online score on the selected map; its replay is fetched
    /// meanwhile.
    fn open_online_score(&mut self, score: online::OnlineScore) {
        let Some(entry) = self.select.selected() else { return };
        self.open_online_score_on(entry, score);
    }

    /// The results screen of an online score on map `entry`.
    fn open_online_score_on(&mut self, entry: usize, score: online::OnlineScore) {
        let stats = game::Stats { great: score.great, ok: score.ok, miss: score.miss, max_combo: score.max_combo, score: score.score, ..Default::default() };
        let mods = Mods::from_list(&score.mods);
        let e = &self.library[entry];
        let mut results = Results::new(results_title(&e.meta, None), stats, game::Windows::new(e.meta.od));
        results.set_mods(mods, score.rate);
        results.partial = true;
        results.pp = score.pp;
        results.no_pp = mods.unranked().then_some(tr!("Autoplay and Relax get no pp", "AutoplayとRelaxではppは付きません", "Autoplay и Relax не дают pp"));
        results.opened = Some(ui::Opened { player: score.player.clone(), date: score.date, online: true });
        let play = pp::Play { great: score.great, ok: score.ok, miss: score.miss, max_combo: score.max_combo, rate: score.rate, mods, scroll_speed: score.scroll_speed.unwrap_or(1.0), seed: 0, half: mods::HalfRule::of_pp_version(score.pp_version) };
        let online = (self.cfg.online_server.clone(), score, e.label());
        results.job = Some(results_job(e.path.clone(), play, None, Some(online)));
        self.scene = Scene::Results { results, entry, from: None };
    }

    fn start_loading_from(&mut self, entry: usize, from: Option<f64>) {
        self.pending_replay = None;
        self.record_play(entry);
        self.stop_audio();
        self.preview.show_background_for(&self.library[entry]);
        let mut label = self.library[entry].label();
        if let Some(t) = from {
            let at = fmt_time(t);
            label.push_str(&tr!("  (from {at})", "  ({at}から)", "  (с {at})";));
        }
        let rx = self.load_with(entry, self.cfg.rate, self.cfg.mods);
        self.scene = Scene::Loading { rx, label, entry, from };
    }

    /// The map the preview shows: the lobby map in the lobby (and its preview), else the song
    /// select selection.
    fn preview_target(&self) -> Option<usize> {
        if let Some(entry) = self.player.as_ref().and_then(|p| p.list.entry()).filter(|_| self.on_menu()) {
            return Some(entry);
        }
        let lobby = match &self.scene {
            Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults | Scene::Spectate(_) => true,
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

    /// Android: the app went to the background (or came back). The music holds meanwhile; a
    /// song already paused itself when focus went, and stays paused until you resume it.
    fn set_background(&mut self, background: bool) {
        let Some(a) = &self.audio else { return };
        let game_paused = matches!(&self.scene, Scene::Play { game, .. } if game.paused.is_some());
        if background {
            a.set_paused(true);
        } else if !game_paused {
            a.set_paused(false);
        }
    }

    /// Parses the map and decodes its audio on a background thread.
    /// `speed`: rate modifier to prepare the track for (time-stretched or resampled once here).
    /// Loads a map and its song prepared at `speed` (plus Wind Up / Down from `mods`).
    fn spawn_loader(&self, entry: usize, speed: f64, keep_pitch: bool, mods: Mods, cached: Option<Arc<Track>>) -> Receiver<Loaded> {
        let path = self.library[entry].path.clone();
        let rate = self.audio.as_ref().map(|a| a.rate);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let map = beatmap::load(&path)?;
                let audio_path = path.parent().unwrap_or(&path).join(&map.meta.audio_file);
                let speed = game::speed_for(&map, mods, speed);
                // A track prepared at one constant speed can't serve a changing one.
                let cached = cached.filter(|_| speed.is_constant());
                let track = match (cached, rate) {
                    (Some(track), _) => track,
                    (None, Some(rate)) => {
                        // Shared with the preview's full decode if that is still running.
                        let track = audio::decode_shared(&audio_path, rate)?;
                        if speed.is_constant() {
                            audio::with_rate(track, rate, speed.r0, keep_pitch)
                        } else {
                            Arc::new(audio::change_speed(Track { samples: track.samples.clone(), rate: track.rate }, rate, &speed, keep_pitch))
                        }
                    }
                    (None, None) => Arc::new(Track { samples: Vec::new(), rate: 1.0 }),
                };
                Ok((map, track))
            })();
            let _ = tx.send(result);
        });
        rx
    }

    fn show_select(&mut self) {
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
        let audio_desc = self.audio.as_ref().map(|a| a.description.clone()).unwrap_or_else(|| tr!("no audio", "音声なし", "нет звука").into());
        let config = config::config_path().map(|p| p.display().to_string()).unwrap_or_else(|| tr!("not saved", "保存されません", "не сохраняется").into());
        let (adapter, hz) = (&self.adapter, self.refresh_hz);
        self.status = tr!("{adapter}  |  {audio_desc}  |  {hz:.0} Hz  |  config: {config}", "{adapter}  |  {audio_desc}  |  {hz:.0} Hz  |  設定: {config}", "{adapter}  |  {audio_desc}  |  {hz:.0} Гц  |  конфиг: {config}";);
    }

    fn open_settings(&mut self) {
        let prev = std::mem::replace(&mut self.scene, Scene::Select);
        self.scene = Scene::Settings { prev: Box::new(prev) };
        self.settings_ui.binding = None;
        self.settings_ui.choices = None;
        self.settings_ui.editing = None;
        self.settings_ui.clear_search();
        self.settings_ui.krokodil = self.is_krokodil();
    }

    /// Puts a drum on `code`; false if that key is taken. In Settings and the guide alike.
    fn bind_drum_key(&mut self, slot: usize, code: KeyCode) -> bool {
        if self.cfg.preset_keys.contains(&code) {
            self.toast(tr!("{} is a playfield preset key", "{}はプリセットのキーです", "{} — клавиша пресета поля"; config::key_name(code)));
            return false;
        }
        // Swap if the key was already bound to another drum.
        if let Some(other) = self.cfg.keys.iter().position(|&k| k == code) {
            self.cfg.keys[other] = self.cfg.keys[slot];
        }
        self.cfg.keys[slot] = code;
        true
    }

    fn guide_action(&mut self, action: guide::Action) {
        let Some(page) = self.guide else { return };
        self.guide_binding = None;
        match action {
            guide::Action::Bind(slot) => self.guide_binding = Some(slot),
            guide::Action::Back => self.guide = Some(page.saturating_sub(1)),
            guide::Action::Next if page + 1 < guide::PAGES => self.guide = Some(page + 1),
            guide::Action::Next | guide::Action::Close => self.guide = None,
            guide::Action::Page(i) => self.guide = Some(i),
            guide::Action::Calibrate => {
                self.guide = None;
                self.guide_after = Some(page + 1);
                self.start_calibration_as(false);
            }
        }
        // Once closed, it doesn't come up by itself again.
        if self.guide.is_none() && !self.cfg.guide_seen {
            self.cfg.guide_seen = true;
            self.cfg.save();
        }
    }

    fn reset_is_armed(&self) -> bool {
        self.reset_armed.is_some_and(|at| at.elapsed() < RESET_CONFIRM)
    }

    /// The first press of "Reset map offsets" only asks: it's a lot to lose to a stray click.
    fn press_reset_map_offsets(&mut self) {
        if self.map_offsets.is_empty() {
            return;
        }
        if !self.reset_is_armed() {
            self.reset_armed = Some(Instant::now());
            return;
        }
        self.reset_armed = None;
        let n = self.map_offsets.len();
        config::reset_map_offsets();
        self.map_offsets.clear();
        self.refresh_play_offset();
        self.toast(tr!("offsets of {n} maps reset", "{n}譜面のオフセットをリセットしました", "сброшены оффсеты карт: {n}";));
    }

    /// Ctrl+O: settings over whatever is on (a solo song pauses first), or back out of them.
    /// Scenes that have to keep running (a match, spectating, loading) say so instead.
    fn settings_anywhere(&mut self) {
        if matches!(self.scene, Scene::Settings { .. }) {
            self.settings_ui.dropdown = None;
            self.settings_ui.choices = None;
            self.close_settings();
            return;
        }
        match &mut self.scene {
            Scene::Play { game, .. } => game.pause(self.audio.as_ref()),
            Scene::Select | Scene::Download | Scene::MpMenu | Scene::Lobby | Scene::Results { .. } | Scene::Preview { .. } => {}
            _ => {
                self.toast(tr!("settings can't open here", "ここでは設定を開けません", "здесь настройки не открыть").into());
                return;
            }
        }
        self.changelog = None;
        self.profile = None;
        self.social.close();
        self.mod_select = None;
        self.open_settings();
    }

    /// The main menu is what's showing.
    fn on_menu(&self) -> bool {
        self.main_menu.is_some() && matches!(self.scene, Scene::Select) && self.mp.is_none() && !self.picking_lobby_map
    }

    fn menu_key(&mut self, code: KeyCode) {
        if self.modifiers_ctrl || self.main_menu.as_ref().is_none_or(menu::Menu::is_leaving) {
            return;
        }
        // Rim keys on the outside, like on the drum.
        match (code, self.cfg.keys.iter().position(|&k| k == code)) {
            (KeyCode::Escape, _) => self.ask_quit(),
            (KeyCode::ArrowLeft, _) | (_, Some(0)) => self.menu_step(-1),
            (KeyCode::ArrowRight, _) | (_, Some(3)) => self.menu_step(1),
            (KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space, _) | (_, Some(1 | 2)) => self.menu_hit(),
            _ => {}
        }
    }

    /// A key in the music player: the drum's rim changes song, its face pauses.
    fn player_key(&mut self, code: KeyCode, text: Option<&str>) {
        let Some(p) = &mut self.player else { return };
        if p.searching {
            let mut filter = p.list.filter.clone();
            match code {
                KeyCode::Escape => {
                    p.searching = false;
                    filter.clear();
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    p.searching = false;
                    let first = p.list.shown.first().copied();
                    if let Some(track) = first {
                        self.music_play(track);
                    }
                    return;
                }
                KeyCode::Backspace => {
                    filter.pop();
                }
                _ => filter.extend(text.into_iter().flat_map(str::chars).filter(|c| !c.is_control())),
            }
            p.search(filter);
            return;
        }
        if self.modifiers_ctrl {
            if code == KeyCode::KeyF {
                p.searching = true;
            }
            return;
        }
        match (code, self.cfg.keys.iter().position(|&k| k == code)) {
            (KeyCode::Escape, _) if p.folders_open => p.folders_open = false,
            (KeyCode::Escape, _) => self.close_player(),
            (KeyCode::KeyL, _) => {
                let now = p.list.now();
                self.music_favourite(now);
            }
            (KeyCode::Tab, _) => p.folders_open = !p.folders_open,
            (KeyCode::BracketLeft, _) => p.move_pitch(-1),
            (KeyCode::BracketRight, _) => p.move_pitch(1),
            (KeyCode::ArrowUp, _) => p.scroll_by(-1),
            (KeyCode::ArrowDown, _) => p.scroll_by(1),
            (KeyCode::ArrowLeft, _) => self.music_seek_by(-MUSIC_SEEK_MS),
            (KeyCode::ArrowRight, _) => self.music_seek_by(MUSIC_SEEK_MS),
            (_, Some(0)) => self.music_control(player::Control::Prev),
            (_, Some(3)) => self.music_control(player::Control::Next),
            (KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space, _) | (_, Some(1 | 2)) => self.music_control(player::Control::Toggle),
            _ => {}
        }
    }

    fn music_control(&mut self, control: player::Control) {
        match control {
            player::Control::Prev => self.music_step(-1),
            player::Control::Next => self.music_step(1),
            player::Control::Toggle => self.set_music_paused(!self.music_paused),
            player::Control::Open => {
                self.player = Some(Box::new(player::Player::new(&self.library, self.select.selected())));
                if self.media.is_none() && !cfg!(test) {
                    self.media = Some(media::Media::start(self.window_handle));
                }
            }
            player::Control::Shuffle => {
                if let Some(p) = &mut self.player {
                    p.list.toggle_shuffle(fresh_seed());
                }
            }
            player::Control::Repeat => {
                if let Some(p) = &mut self.player {
                    p.list.cycle_repeat();
                }
            }
            player::Control::Favourite => {
                if let Some(now) = self.player.as_ref().map(|p| p.list.now()) {
                    self.music_favourite(now);
                }
            }
            player::Control::PlayMap => {
                self.select.filter.clear();
                self.select.refilter(&self.library);
                self.close_player();
                self.main_menu = None;
            }
        }
    }

    /// Loves a song, or not any more; the list is kept on disk.
    fn music_favourite(&mut self, track: usize) {
        if let Some(p) = &mut self.player {
            p.list.toggle_favourite(track, fresh_seed());
            playlist::save_favourites(&p.list.favourites);
        }
    }

    /// Back to the menu, the song playing still selected there.
    fn close_player(&mut self) {
        let Some(p) = self.player.take() else { return };
        if let Some(e) = p.list.entry().and_then(|i| self.library.get(i)) {
            self.select.select_path(&self.library, &e.path);
        }
        self.media_told = None;
        if let Some(m) = &self.media {
            m.stopped();
        }
    }

    /// Plays this track of the player's list.
    fn music_play(&mut self, track: usize) {
        self.set_music_paused(false);
        if let Some(p) = &mut self.player {
            p.list.play(track);
        }
    }

    /// Moves the player's volume to where the pointer is.
    fn music_volume_to_pointer(&mut self) {
        let (vx, _) = self.mouse_virtual();
        if let Some((left, w)) = self.player.as_ref().map(|p| p.volume_bar) {
            self.cfg.music_volume = ((vx - left) / w).clamp(0.0, 1.0);
            self.apply_volume();
        }
    }

    /// Takes what the media keys pressed, and tells the desktop what's on.
    fn poll_media(&mut self) {
        let Some(media) = &self.media else { return };
        let keys: Vec<media::Key> = media.keys.try_iter().collect();
        if self.on_menu() {
            for key in keys {
                match key {
                    media::Key::Toggle => self.set_music_paused(!self.music_paused),
                    media::Key::Play => self.set_music_paused(false),
                    media::Key::Pause => self.set_music_paused(true),
                    media::Key::Next => self.music_step(1),
                    media::Key::Prev => self.music_step(-1),
                }
            }
        }
        let Some((p, media)) = self.player.as_ref().zip(self.media.as_ref()) else { return };
        let state = (p.list.now(), !self.music_paused);
        if self.media_told != Some(state) {
            if self.media_told.is_none_or(|t| t.0 != state.0)
                && let Some(e) = p.list.entry().and_then(|i| self.library.get(i)) {
                media.song(e.meta.title.clone(), e.meta.artist.clone());
            }
            media.playing(state.1);
            self.media_told = Some(state);
        }
    }

    fn set_music_paused(&mut self, paused: bool) {
        self.music_paused = paused;
        if let Some(a) = &self.audio {
            a.set_paused(paused);
        }
    }

    /// The next or previous song of the list, round from one end to the other.
    fn music_step(&mut self, by: i32) {
        self.set_music_paused(false);
        self.preview.from_top = false;
        if let Some(p) = &mut self.player {
            if by < 0 {
                p.list.prev();
            } else if !p.list.next() {
                self.set_music_paused(true);
            }
            return;
        }
        let was = self.select.selected();
        self.select.jump_songs(by, &self.library);
        if self.select.selected() == was {
            self.select.jump_songs(-by * self.library.len() as i32, &self.library);
        }
    }

    fn music_seek_by(&mut self, ms: f64) {
        if let Some((at, _)) = self.preview.music_times(self.audio.as_ref(), Instant::now()) {
            self.preview.seek_music(at + ms, self.audio.as_ref());
        }
    }

    /// Moves the music to where the pointer is on the player's timeline.
    fn music_seek_to_pointer(&mut self) {
        let (vx, _) = self.mouse_virtual();
        if let Some((left, w, len)) = self.player.as_ref().map(|p| p.timeline) {
            let at = ((vx - left) / w).clamp(0.0, 1.0) as f64 * len;
            self.preview.seek_music(at, self.audio.as_ref());
        }
    }

    fn menu_step(&mut self, by: i32) {
        if let Some(m) = &mut self.main_menu {
            m.step(by);
            self.sfx(audio::Sfx::Kat);
        }
    }

    /// A note or its name clicked: it rides up to the circle and is hit, all in one click.
    fn menu_click(&mut self, i: usize) {
        let Some(m) = self.main_menu.as_mut().filter(|m| !m.is_leaving()) else { return };
        m.pick(i);
        self.menu_hit();
    }

    fn menu_hit(&mut self) {
        let Some(m) = &mut self.main_menu else { return };
        let sound = match m.hit() {
            menu::Item::Editor => audio::Sfx::ComboBreak,
            menu::Item::Play => audio::Sfx::BigDon,
            _ => audio::Sfx::Don,
        };
        self.sfx(sound);
    }

    fn sfx(&self, sound: audio::Sfx) {
        if let Some(a) = &self.audio {
            a.sfx(sound);
        }
    }

    /// Opens the screen of the note that was hit. Coming back from those lands here again,
    /// except from the songs, which is what the menu is for leaving to.
    fn menu_open(&mut self, item: menu::Item) {
        match item {
            menu::Item::Play => self.main_menu = None,
            menu::Item::Multiplayer => self.open_multiplayer(),
            menu::Item::Download => self.open_download(),
            menu::Item::Editor => {}
            menu::Item::Settings => self.open_settings(),
            menu::Item::Exit => self.ask_quit(),
        }
    }

    /// Logged in as krokodil_koban (their settings row shows).
    fn is_krokodil(&self) -> bool {
        self.mp_menu.account.as_deref().is_some_and(|a| a.eq_ignore_ascii_case("krokodil_koban"))
    }

    fn close_settings(&mut self) {
        if let Scene::Settings { prev } = std::mem::replace(&mut self.scene, Scene::Select) {
            self.scene = *prev;
        }
    }

    /// A play (or calibration) is running, not paused.
    fn playing(&self) -> bool {
        matches!(&self.scene, Scene::Play { game, .. } | Scene::MpPlay { game, .. } | Scene::Calibrate { game, .. } if game.paused.is_none())
    }

    /// Stars out the swear words already in the chats (the chat filter was just switched on).
    fn censor_chats(&mut self) {
        let lines = self.social.chats.values_mut().flatten().chain(self.social.global.iter_mut());
        for line in lines {
            line.text = net::censor(&line.text);
        }
        if let Some(s) = &mut self.mp {
            for (_, text) in &mut s.chat {
                *text = net::censor(text);
            }
        }
    }

    /// The window has a new size.
    fn set_window(&mut self, w: f32, h: f32) {
        self.real_window = (w, h);
        self.window_w = w;
        self.window_h = h;
    }

    /// Works out where the game goes in the window, for this frame.
    fn sync_letterbox(&mut self) -> Option<[u32; 4]> {
        let (w, h) = self.real_window;
        let on = self.cfg.letterbox && self.fullscreen;
        self.letterbox = layout::letterbox((w as u32, h as u32), self.cfg.letterbox_size, self.cfg.letterbox_pos).filter(|_| on);
        let (gw, gh) = self.letterbox.map_or((w, h), |[_, _, gw, gh]| (gw as f32, gh as f32));
        if (gw, gh) != (self.window_w, self.window_h) {
            (self.window_w, self.window_h) = (gw, gh);
            self.refresh_presets();
        }
        self.letterbox
    }

    /// A point of the window as the game sees it: from the game's own corner.
    fn to_game(&self, x: f64, y: f64) -> (f64, f64) {
        self.letterbox.map_or((x, y), |[gx, gy, ..]| (x - gx as f64, y - gy as f64))
    }

    /// The window's size in pixels.
    fn window_px(&self) -> (f32, f32) {
        if self.letterbox.is_some() { self.real_window } else { (self.window_w, self.window_h) }
    }

    /// The resolutions Letterbox can use: all the monitor reports, custom ones from the driver
    /// too, and the usual ones, grouped by aspect ratio.
    fn letterbox_sizes(&self) -> Vec<(u32, u32)> {
        let (mw, mh) = self.monitor_size;
        let sizes = layout::presets(&self.monitor_modes, mw, mh);
        layout::group_presets(&sizes, self.monitor_size).into_iter().flat_map(|(_, l)| l).collect()
    }

    /// How a setting's row shows its value.
    fn setting_control(&self, item: Item) -> settings::Control {
        use settings::Control;
        let c = &self.cfg;
        match item {
            Item::MasterVolume => Control::Level(c.master_volume),
            Item::MusicVolume => Control::Level(c.music_volume),
            Item::FxVolume => Control::Level(c.fx_volume),
            Item::BackgroundDim => Control::Level(c.bg_dim),
            Item::DonColour => Control::Swatch(c.note_colours.don()),
            Item::KatColour => Control::Swatch(c.note_colours.kat()),
            Item::Accent | Item::AccentHue => Control::Swatch(c.accent),
            Item::Interface => Control::Switch(!c.interface_hidden),
            _ if item.kind() == Kind::Toggle => Control::Switch(self.setting_value(item) != off_text()),
            _ => Control::Text,
        }
    }

    /// A setting's bar set from the pointer, to the nearest percent. Saved on letting go.
    fn set_level(&mut self, i: usize) {
        let level = (self.settings_ui.level_at(self.mouse_virtual().0) * 100.0).round() / 100.0;
        match settings::ITEMS[i].1 {
            Item::MasterVolume => self.cfg.master_volume = level,
            Item::MusicVolume => self.cfg.music_volume = level,
            Item::FxVolume => self.cfg.fx_volume = level,
            Item::BackgroundDim => self.cfg.bg_dim = level,
            _ => {}
        }
        self.apply_volume();
    }

    /// Text typed on the settings screen goes into its search; true if it took the key.
    fn settings_type(&mut self, code: KeyCode, text: Option<&str>) -> bool {
        let ui = &self.settings_ui;
        if ui.choices.is_some() || ui.binding.is_some() || ui.dropdown.is_some() || self.modifiers_ctrl || self.modifiers_alt {
            return false;
        }
        let searching = !ui.search.is_empty();
        let mut search = ui.search.clone();
        match code {
            KeyCode::Backspace if searching => {
                search.pop();
            }
            KeyCode::Escape if searching => search.clear(),
            // Space still works the row until there's something to search for.
            KeyCode::Space if !searching => return false,
            _ if keys::starts_text(code) || (searching && keys::types_text(code)) => search.extend(text.into_iter().flat_map(str::chars).filter(|c| !c.is_control())),
            _ => return false,
        }
        self.search_settings(search);
        true
    }

    fn search_settings(&mut self, text: String) {
        let hotkeys = self.settings_key_names();
        self.settings_ui.set_search(text, &|item| hotkeys.iter().find(|(i, _)| *i == item).map_or(String::new(), |(_, name)| name.clone()));
    }

    /// Every key setting with the key it's on.
    fn settings_key_names(&self) -> Vec<(Item, String)> {
        settings::ITEMS.iter().filter(|(_, item)| item.kind() == Kind::Key).map(|&(_, item)| (item, self.setting_value(item))).collect()
    }

    fn setting_value(&self, item: Item) -> String {
        let c = &self.cfg;
        let on = |b: bool| if b { on_text() } else { off_text() }.to_string();
        match item {
            Item::ScrollSpeed => format!("{:.2}x", c.scroll_speed),
            Item::Offset => format!("{:+.0} ms", c.offset_ms),
            Item::VisualOffset => format!("{:+.0} ms", c.visual_offset_ms),
            Item::UpdateChannel => {
                let picked = self.updater.channel();
                if picked == update::BUILD_CHANNEL {
                    picked.name().into()
                } else {
                    let (picked, running) = (picked.name(), update::BUILD_CHANNEL.name());
                    tr!("{picked} (running {running})", "{picked} (実行中は{running})", "{picked} (запущен {running})";)
                }
            }
            Item::Key(i) => config::key_name(c.keys[i]),
            Item::PresetKey(i) => config::key_name(c.preset_keys[i]),
            Item::Hotkey(i) => c.hotkeys.get(keys::Hotkey::ALL[i]).map(keys::Chord::name).unwrap_or_else(|| none_text().into()),
            Item::TouchLayout => c.touch_layout.name().into(),
            Item::AutoplayStyle => c.autoplay.style.name().into(),
            Item::AutoplayBinding => autoplay::BINDINGS[c.autoplay.binding].into(),
            Item::AutoplayLeftHanded => on(c.autoplay.left_handed),
            Item::Screenshots => c.screenshot_to.name().into(),
            Item::Language => c.language.name().into(),
            Item::ShowPlayfieldSize => on(c.show_playfield_size),
            Item::WheelResize => on(c.wheel_resizes_playfield),
            Item::BackgroundDimBlack => on(c.bg_dim_black),
            Item::KiaiEffects => on(c.kiai_effects),
            Item::HideGreat => on(c.hide_great),
            Item::HitFlash => on(c.hit_flash),
            Item::DiscordPresence => on(c.discord_presence),
            Item::ChatFilter => on(c.chat_filter),
            Item::MessageSound => on(c.message_sound),
            Item::NotifyPlaying => on(c.notify_playing),
            Item::NotifyOnline => on(c.notify_online),
            Item::NotifyMessages => on(c.notify_messages),
            Item::NotifyMentions => on(c.notify_mentions),
            Item::NotifyRequests => on(c.notify_requests),
            Item::ShowConverts => on(self.select.show_converts),
            Item::HiddenMaps => self.select.hidden.describe(),
            Item::SongsFolder if self.folder_pick.is_some() && self.folder_pick_for == Item::SongsFolder => picking_text().into(),
            Item::DataFolder if self.folder_pick.is_some() && self.folder_pick_for == Item::DataFolder => picking_text().into(),
            Item::DataFolder if self.data_move.is_some() => tr!("moving...", "移動中...", "перенос...").into(),
            Item::DataFolder => match &c.data_folder {
                Some(dir) => short_path(dir),
                None => tr!("the usual places", "いつもの場所", "обычные места").into(),
            },
            Item::SongsFolder if self.scan.is_some() => tr!("scanning...", "スキャン中...", "сканирование...").into(),
            Item::SongsFolder => match (&c.songs_folder, self.song_dirs.first()) {
                (Some(dir), _) => short_path(dir),
                (None, Some(dir)) => {
                    let dir = short_path(dir);
                    tr!("found {dir}", "見つけた: {dir}", "найдена {dir}";)
                }
                (None, None) => tr!("none found, Enter to pick", "見つかりません、Enterで選択", "не найдена, Enter — выбрать").into(),
            },
            Item::MasterVolume => format!("{:.0}%", c.master_volume * 100.0),
            Item::MusicVolume => format!("{:.0}%", c.music_volume * 100.0),
            Item::FxVolume => format!("{:.0}%", c.fx_volume * 100.0),
            Item::AudioBuffer => {
                let rate = self.audio.as_ref().map(|a| a.rate).unwrap_or(48000);
                let ms = c.audio_buffer as f64 / rate as f64 * 1000.0;
                let frames = c.audio_buffer;
                tr!("{frames} frames ({ms:.1} ms)", "{frames}フレーム ({ms:.1} ms)", "{frames} кадров ({ms:.1} мс)";)
            }
            Item::PresentMode => c.present.name().to_string(),
            Item::MenuVsync => on(c.menu_vsync),
            Item::HalfResolution => on(c.half_resolution),
            Item::FpsCap => {
                if c.fps_cap == 0 {
                    tr!("unlimited", "無制限", "без ограничения").into()
                } else {
                    c.fps_cap.to_string()
                }
            }
            Item::ShowFps => on(c.show_fps),
            Item::Animations => on(c.animations),
            Item::Splash => on(c.splash),
            Item::LegacyMenu => on(!c.main_menu),
            Item::Letterbox => on(c.letterbox),
            Item::LetterboxSize => format!("{}x{}", c.letterbox_size.0, c.letterbox_size.1),
            Item::LetterboxX => format!("{:+.0}%", c.letterbox_pos.0),
            Item::LetterboxY => format!("{:+.0}%", c.letterbox_pos.1),
            Item::MenuNotes => on(c.menu_notes),
            Item::CheckUpdates => on(c.check_updates),
            Item::Update => {
                let build = update::BUILD.map(update::short);
                match (self.updater.status(), build) {
                    (_, None) => tr!("local build (no updates)", "ローカルビルド (更新なし)", "локальная сборка (без обновлений)").into(),
                    (update::Status::Off, Some(b)) => tr!("{b}, Enter to check", "{b}、Enterで確認", "{b}, Enter — проверить";),
                    (update::Status::Checking, _) => tr!("checking...", "確認中...", "проверка...").into(),
                    (update::Status::UpToDate, Some(b)) => tr!("{b}, up to date", "{b}、最新です", "{b}, последняя";),
                    (update::Status::Available { commit, .. }, Some(b)) => {
                        let new = update::short(&commit);
                        tr!("{b} -> {new}, Enter to install", "{b} -> {new}、Enterでインストール", "{b} -> {new}, Enter — установить";)
                    }
                    (update::Status::Downloading, _) => {
                        let p = self.updater.progress() * 100.0;
                        tr!("downloading {p:.0}%", "ダウンロード中 {p:.0}%", "загрузка {p:.0}%";)
                    }
                    (update::Status::Installed, _) => tr!("restarting...", "再起動中...", "перезапуск...").into(),
                    (update::Status::Failed(_), _) => tr!("failed, Enter to retry", "失敗、Enterで再試行", "ошибка, Enter — ещё раз").into(),
                }
            }
            Item::Changelog => tr!("Enter to read", "Enterで読む", "Enter — читать").into(),
            Item::SongSelectLayout if self.song_layout_is_default() => tr!("default", "初期設定", "по умолчанию").into(),
            Item::SongSelectLayout => tr!("custom, Enter: default", "カスタム、Enter: 初期設定", "свой, Enter: по умолчанию").into(),
            Item::KrokodilKoban => String::new(),
            Item::Fullscreen => on(self.fullscreen),
            Item::WindowSize if self.fullscreen => {
                let (w, h) = self.window_px();
                tr!("fullscreen ({w:.0}x{h:.0})", "フルスクリーン ({w:.0}x{h:.0})", "полный экран ({w:.0}x{h:.0})";)
            }
            Item::WindowSize => {
                let (w, h) = self.window_px();
                format!("{w:.0}x{h:.0}")
            }
            Item::Calibrate => enter_to_start().into(),
            Item::Guide => enter_to_start().into(),
            Item::ResetMapOffsets if self.map_offsets.is_empty() => tr!("none set", "なし", "их нет").into(),
            Item::ResetMapOffsets if self.reset_is_armed() => tr!("Enter again to reset", "もう一度Enterでリセット", "Enter ещё раз — сбросить").into(),
            Item::ResetMapOffsets => {
                let n = self.map_offsets.len();
                tr!("{n} maps", "{n}譜面", "карт: {n}";)
            }
            Item::CalibrationBpm => format!("{:.0} BPM", c.calibration_bpm),
            Item::LiveOffset => on(c.show_offset_hint),
            Item::LiveOffsetUr => on(c.offset_hint_ur_scaled),
            Item::VsyncCompensation if !c.vsync_compensation => off_text().into(),
            Item::VsyncCompensation => {
                let lead = VSYNC_FRAMES * self.refresh_ms() + c.vsync_tune_ms;
                if c.present == PresentChoice::Fifo {
                    tr!("on: notes {lead:.0} ms ahead", "オン: ノーツを{lead:.0} ms先に", "вкл: ноты на {lead:.0} мс вперёд";)
                } else {
                    tr!("on ({lead:.0} ms once gameplay uses vsync)", "オン (プレイが垂直同期なら{lead:.0} ms)", "вкл ({lead:.0} мс, когда игра с vsync)";)
                }
            }
            Item::VsyncTune => format!("{:+.0} ms", c.vsync_tune_ms),
            Item::VsyncCalibrate => enter_to_start().into(),
            Item::Interface if c.interface_hidden => {
                let key = config::interface_key_name(c.interface_key);
                tr!("hidden ({key})", "非表示 ({key})", "скрыт ({key})";)
            }
            Item::Interface => tr!("shown", "表示", "виден").into(),
            Item::InterfaceHides if c.interface_hides == hud::Parts::ALL => tr!("everything", "すべて", "всё").into(),
            Item::InterfaceHides if c.interface_hides == hud::Parts::NONE => tr!("nothing", "なし", "ничего").into(),
            Item::InterfaceHides => {
                let (n, all) = (c.interface_hides.len(), hud::Part::ALL.len());
                tr!("{n} of {all}", "{all}個中{n}個", "{n} из {all}";)
            }
            Item::InterfaceKey => config::interface_key_name(c.interface_key),
            Item::ResumeDelay if c.resume_delay_ms <= 0.0 => off_text().into(),
            Item::ResumeDelay => {
                let s = c.resume_delay_ms / 1000.0;
                tr!("{s:.2} s", "{s:.2}秒", "{s:.2} с";)
            }
            Item::PlayfieldArea => c.playfield_area.describe(),
            Item::PlayfieldPreset => self.current_preset().map(|(w, h)| format!("{w}x{h}")).unwrap_or_else(|| tr!("custom", "カスタム", "свой").into()),
            Item::BackgroundDim => format!("{:.0}%", c.bg_dim * 100.0),
            Item::PlayfieldReset => tr!("Enter to reset", "Enterで元に戻す", "Enter — сбросить").into(),
            Item::Rate => format!("{:.2}x", c.rate),
            Item::RateKeepPitch => on(c.rate_keep_pitch),
            Item::Skin if c.skin.is_empty() => built_in_text().into(),
            Item::Skin => c.skin.clone(),
            Item::SkinParts => {
                let changed = skin::Part::ALL.iter().filter(|&&p| *c.skin_parts.get(p) != p.default_source()).count();
                if changed == 0 {
                    tr!("notes from the skin", "ノーツはスキンから", "ноты из скина").into()
                } else {
                    tr!("{changed} changed", "{changed}個を変更", "изменено: {changed}";)
                }
            }
            Item::SkinPart(i) => skin::Part::ALL.get(i).map(|&p| self.skin_source_name(c.skin_parts.get(p))).unwrap_or_default(),
            Item::SkinFolder => tr!("Enter to open", "Enterで開く", "Enter — открыть").into(),
            Item::Accent => render::accent_name(c.accent),
            Item::AccentHue => format!("#{:06x}", c.accent),
            Item::NoteColours => c.note_colours.label().into(),
            Item::DonColour => format!("#{:06x}", c.note_colours.don()),
            Item::KatColour => format!("#{:06x}", c.note_colours.kat()),
        }
    }

    /// Changes a setting one step in `dir` (-1 / +1), applying it immediately.
    fn adjust_setting(&mut self, item: Item, dir: i32, window: Option<&Window>) {
        let d = dir as f64;
        if item == Item::ScrollSpeed && self.mid_song() {
            self.toast(scroll_in_select().into());
            return;
        }
        match item {
            Item::ScrollSpeed => self.cfg.scroll_speed = config::step_scroll_speed(self.cfg.scroll_speed, d),
            Item::Offset => self.cfg.offset_ms += d * if self.modifiers_shift { 5.0 } else { 1.0 },
            Item::VisualOffset => self.cfg.visual_offset_ms += d * if self.modifiers_shift { 5.0 } else { 1.0 },
            Item::UpdateChannel => {
                let channel = match self.updater.channel() {
                    update::Channel::Stable => update::Channel::Dev,
                    update::Channel::Dev => update::Channel::Stable,
                };
                self.set_update_channel(channel);
                return;
            }
            Item::Key(_) | Item::PresetKey(_) | Item::InterfaceKey | Item::Hotkey(_) => {
                self.settings_ui.binding = Some(item);
                return;
            }
            Item::Interface => self.cfg.interface_hidden = !self.cfg.interface_hidden,
            // Open their lists on Enter.
            Item::InterfaceHides | Item::SkinParts | Item::SkinPart(_) => return,
            Item::TouchLayout => self.cfg.touch_layout = self.cfg.touch_layout.step(dir),
            Item::AutoplayStyle => self.cfg.autoplay.style = self.cfg.autoplay.style.step(dir),
            Item::AutoplayBinding => {
                let n = autoplay::BINDINGS.len() as i32;
                self.cfg.autoplay.binding = (self.cfg.autoplay.binding as i32 + dir).rem_euclid(n) as usize;
            }
            Item::AutoplayLeftHanded => self.cfg.autoplay.left_handed = !self.cfg.autoplay.left_handed,
            Item::Screenshots => self.cfg.screenshot_to = self.cfg.screenshot_to.step(dir),
            Item::Language => self.cfg.language = self.cfg.language.step(dir),
            Item::Accent => {
                let n = render::ACCENTS.len() as i32;
                // A colour of your own goes to the first (or last) preset.
                let next = match render::ACCENTS.iter().position(|a| a.1 == self.cfg.accent) {
                    Some(i) => (i as i32 + dir).rem_euclid(n),
                    None if dir > 0 => 0,
                    None => n - 1,
                };
                self.cfg.accent = render::ACCENTS[next as usize].1;
            }
            Item::AccentHue => self.cfg.accent = render::rotate_hue(self.cfg.accent, 10.0 * d as f32),
            Item::NoteColours => self.cfg.note_colours = self.cfg.note_colours.step(dir),
            Item::DonColour | Item::KatColour => {
                let (don, kat) = (self.cfg.note_colours.don(), self.cfg.note_colours.kat());
                let turn = |c: u32| render::rotate_hue(c, 10.0 * d as f32);
                self.cfg.note_colours = if item == Item::DonColour { multiverse::Colours::own(turn(don), kat) } else { multiverse::Colours::own(don, turn(kat)) };
            }
            Item::ShowPlayfieldSize => self.cfg.show_playfield_size = !self.cfg.show_playfield_size,
            Item::WheelResize => self.cfg.wheel_resizes_playfield = !self.cfg.wheel_resizes_playfield,
            Item::BackgroundDimBlack => self.cfg.bg_dim_black = !self.cfg.bg_dim_black,
            Item::KiaiEffects => self.cfg.kiai_effects = !self.cfg.kiai_effects,
            Item::HideGreat => self.cfg.hide_great = !self.cfg.hide_great,
            Item::HitFlash => self.cfg.hit_flash = !self.cfg.hit_flash,
            Item::DiscordPresence => self.cfg.discord_presence = !self.cfg.discord_presence,
            Item::ChatFilter => {
                self.cfg.chat_filter = !self.cfg.chat_filter;
                // Switched on: what's already in the chats gets cleaned up too.
                if self.cfg.chat_filter {
                    self.censor_chats();
                }
            }
            Item::MessageSound => self.cfg.message_sound = !self.cfg.message_sound,
            Item::NotifyPlaying => self.cfg.notify_playing = !self.cfg.notify_playing,
            Item::NotifyOnline => self.cfg.notify_online = !self.cfg.notify_online,
            Item::NotifyMessages => self.cfg.notify_messages = !self.cfg.notify_messages,
            Item::NotifyMentions => self.cfg.notify_mentions = !self.cfg.notify_mentions,
            Item::NotifyRequests => self.cfg.notify_requests = !self.cfg.notify_requests,
            Item::ShowConverts => {
                self.select.show_converts = !self.select.show_converts;
                self.select.refilter(&self.library);
                self.cfg.show_converts = self.select.show_converts;
                self.cfg.save();
                return;
            }
            // Act on Enter in `settings_key`.
            Item::HiddenMaps | Item::SongsFolder | Item::DataFolder => return,
            Item::MasterVolume => self.cfg.master_volume = (self.cfg.master_volume + 0.05 * dir as f32).clamp(0.0, 1.0),
            Item::MusicVolume => self.cfg.music_volume = (self.cfg.music_volume + 0.05 * dir as f32).clamp(0.0, 1.0),
            Item::FxVolume => self.cfg.fx_volume = (self.cfg.fx_volume + 0.05 * dir as f32).clamp(0.0, 1.0),
            Item::AudioBuffer => {
                let i = AUDIO_BUFFERS.iter().position(|&b| b >= self.cfg.audio_buffer).unwrap_or(2);
                self.set_audio_buffer(AUDIO_BUFFERS[(i as i32 + dir).clamp(0, AUDIO_BUFFERS.len() as i32 - 1) as usize]);
            }
            Item::PresentMode => {
                self.cfg.present = if dir > 0 { self.cfg.present.next() } else { self.cfg.present.next().next() };
            }
            Item::MenuVsync => self.cfg.menu_vsync = !self.cfg.menu_vsync,
            Item::HalfResolution => self.cfg.half_resolution = !self.cfg.half_resolution,
            Item::FpsCap => {
                let i = FPS_CAPS.iter().position(|&c| c == self.cfg.fps_cap).unwrap_or(0) as i32;
                self.cfg.fps_cap = FPS_CAPS[(i + dir).rem_euclid(FPS_CAPS.len() as i32) as usize];
            }
            Item::ShowFps => self.cfg.show_fps = !self.cfg.show_fps,
            Item::Animations => self.cfg.animations = !self.cfg.animations,
            Item::Splash => self.cfg.splash = !self.cfg.splash,
            Item::LegacyMenu => {
                self.cfg.main_menu = !self.cfg.main_menu;
                // Gone at once, not just from the next start.
                if !self.cfg.main_menu {
                    self.main_menu = None;
                }
            }
            Item::MenuNotes => self.cfg.menu_notes = !self.cfg.menu_notes,
            Item::Letterbox => {
                self.cfg.letterbox = !self.cfg.letterbox;
                if self.cfg.letterbox && !self.fullscreen {
                    self.toggle_fullscreen(window);
                }
            }
            Item::LetterboxSize => {
                let sizes = self.letterbox_sizes();
                let area = |&(w, h): &(u32, u32)| w as u64 * h as u64;
                let now = area(&self.cfg.letterbox_size);
                let next = if dir > 0 { sizes.iter().filter(|s| area(s) > now).min_by_key(|s| area(s)) } else { sizes.iter().filter(|s| area(s) < now).max_by_key(|s| area(s)) };
                if let Some(&size) = next {
                    self.cfg.letterbox_size = size;
                }
            }
            Item::LetterboxX => self.cfg.letterbox_pos.0 = (self.cfg.letterbox_pos.0 + 5.0 * dir as f32).clamp(-100.0, 100.0),
            Item::LetterboxY => self.cfg.letterbox_pos.1 = (self.cfg.letterbox_pos.1 + 5.0 * dir as f32).clamp(-100.0, 100.0),
            Item::CheckUpdates => self.cfg.check_updates = !self.cfg.check_updates,
            // Acts on Enter in `settings_key`.
            Item::Update | Item::Changelog | Item::KrokodilKoban | Item::SongSelectLayout => return,
            Item::Fullscreen => self.toggle_fullscreen(window),
            Item::WindowSize => {
                // The next size up or down from the window's, in its aspect ratio while there is
                // one.
                let sizes = self.window_sizes();
                let area = (self.window_w * self.window_h) as u64;
                let aspect = layout::aspect_name(self.window_w as u32, self.window_h as u32);
                let step = |same: bool| {
                    let near = sizes.iter().filter(|&&(w, h)| !same || layout::aspect_name(w, h) == aspect).map(|&(w, h)| ((w * h) as u64, (w, h)));
                    if dir > 0 {
                        near.filter(|&(a, _)| a > area).min_by_key(|&(a, _)| a).map(|(_, s)| s)
                    } else {
                        near.filter(|&(a, _)| a < area).max_by_key(|&(a, _)| a).map(|(_, s)| s)
                    }
                };
                let next = step(true).or_else(|| step(false));
                let next = next.as_ref();
                match next {
                    Some(&size) => self.window_request = Some(size),
                    None => self.toast(if dir > 0 {
            tr!("the largest size that fits this screen", "この画面に収まる最大のサイズです", "это самый большой размер для экрана").into()
        } else {
            tr!("the smallest size there is", "これが最小のサイズです", "это самый маленький размер").into()
        }),
                }
                return;
            }
            // Started with Enter in `settings_key`; arrows do nothing.
            Item::Calibrate | Item::ResetMapOffsets | Item::Guide => return,
            Item::LiveOffset => self.cfg.show_offset_hint = !self.cfg.show_offset_hint,
            Item::LiveOffsetUr => self.cfg.offset_hint_ur_scaled = !self.cfg.offset_hint_ur_scaled,
            Item::VsyncCompensation => self.cfg.vsync_compensation = !self.cfg.vsync_compensation,
            Item::VsyncTune => self.cfg.vsync_tune_ms = (self.cfg.vsync_tune_ms + d * if self.modifiers_shift { 5.0 } else { 1.0 }).clamp(-200.0, 200.0),
            // Started with Enter in `settings_key`.
            Item::VsyncCalibrate => return,
            Item::ResumeDelay => self.cfg.resume_delay_ms = (self.cfg.resume_delay_ms + 250.0 * d).clamp(0.0, 3000.0),
            // Both act on Enter in `settings_key`.
            Item::PlayfieldArea | Item::PlayfieldReset => return,
            Item::PlayfieldPreset => {
                self.cycle_preset(dir);
                return;
            }
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
        multiverse::set_note_colours(self.cfg.note_colours);
        if let (Some(a), Item::FxVolume | Item::Skin) = (&self.audio, item) {
            a.sfx(audio::Sfx::Don);
        }
        self.cfg.save();
    }

    fn set_update_channel(&mut self, channel: update::Channel) {
        self.updater.set_channel(channel);
        if update::BUILD.is_none() {
            self.toast(tr!("a local build doesn't update itself", "ローカルビルドは自分で更新しません", "локальная сборка не обновляется сама").into());
        }
    }

    /// Reopens the audio device with a buffer of `frames` (not during a song).
    fn set_audio_buffer(&mut self, frames: u32) {
        if matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Play { .. })) {
            self.toast(tr!("change the audio buffer from song select", "オーディオバッファは選曲画面で変えてください", "меняйте аудиобуфер в выборе песни").into());
            return;
        }
        self.cfg.audio_buffer = frames;
        // The old stream must close before the new one opens.
        self.audio = None;
        self.audio = match Engine::new(self.cfg.audio_buffer) {
            Ok(a) => Some(a),
            Err(e) => {
                self.toast(tr!("audio failed: {e}", "音声のエラー: {e}", "ошибка звука: {e}";));
                None
            }
        };
        self.load_skin();
        self.refresh_status();
    }

    /// Opens the list of a list setting's choices.
    fn open_choices(&mut self, item: Item) {
        let c = &self.cfg;
        let strings = |v: &mut dyn Iterator<Item = &str>| v.map(str::to_string).collect::<Vec<_>>();
        let (names, current, note) = match item {
            Item::TouchLayout => {
                let all = config::TouchLayout::ALL;
                (strings(&mut all.iter().map(|l| l.name())), all.iter().position(|&l| l == c.touch_layout), String::new())
            }
            Item::Language => {
                let all = lang::Lang::ALL;
                (strings(&mut all.iter().map(|l| l.name())), all.iter().position(|&l| l == c.language), String::new())
            }
            Item::Screenshots => {
                let all = config::ScreenshotTo::ALL;
                (strings(&mut all.iter().map(|t| t.name())), all.iter().position(|&t| t == c.screenshot_to), String::new())
            }
            Item::AutoplayStyle => {
                let all = autoplay::Style::ALL;
                (strings(&mut all.iter().map(|s| s.name())), all.iter().position(|&s| s == c.autoplay.style), String::new())
            }
            Item::AutoplayBinding => (strings(&mut autoplay::BINDINGS.iter().copied()), Some(c.autoplay.binding), String::new()),
            Item::PresentMode => {
                let all = PresentChoice::ALL;
                (strings(&mut all.iter().map(|p| p.name())), all.iter().position(|&p| p == c.present), tr!("during songs", "曲の間", "во время песен").into())
            }
            Item::AudioBuffer => {
                let rate = self.audio.as_ref().map(|a| a.rate).unwrap_or(48000);
                let names = AUDIO_BUFFERS
                    .iter()
                    .map(|&b| {
                        let ms = b as f64 / rate as f64 * 1000.0;
                        tr!("{b} frames ({ms:.1} ms)", "{b}フレーム ({ms:.1} ms)", "{b} кадров ({ms:.1} мс)";)
                    })
                    .collect();
                (names, AUDIO_BUFFERS.iter().position(|&b| b == c.audio_buffer), format!("{rate} Hz"))
            }
            Item::UpdateChannel => {
                let all = [update::Channel::Stable, update::Channel::Dev];
                let running = update::BUILD_CHANNEL.name();
                (strings(&mut all.iter().map(|ch| ch.name())), all.iter().position(|&ch| ch == self.updater.channel()), tr!("running {running}", "実行中: {running}", "запущен {running}";))
            }
            Item::Accent => (strings(&mut render::ACCENTS.iter().map(|a| a.0)), render::ACCENTS.iter().position(|a| a.1 == c.accent), String::new()),
            Item::WindowSize => {
                let sizes = self.window_sizes();
                let now = (self.window_w as u32, self.window_h as u32);
                let type_size = tr!("Type a size", "サイズを入力", "Ввести размер").to_string();
                let names = std::iter::once(type_size).chain(sizes.iter().map(|&(w, h)| format!("{w} x {h}   {}", layout::aspect_name(w, h)))).collect();
                let current = sizes.iter().position(|&s| s == now && !self.fullscreen).map(|i| i + 1);
                let (mw, mh) = self.monitor_size;
                (names, current, tr!("screen {mw}x{mh}", "画面 {mw}x{mh}", "экран {mw}x{mh}";))
            }
            Item::LetterboxSize => {
                let sizes = self.letterbox_sizes();
                let type_size = tr!("Type a size", "サイズを入力", "Ввести размер").to_string();
                let names = std::iter::once(type_size).chain(sizes.iter().map(|&(w, h)| format!("{w} x {h}   {}", layout::aspect_name(w, h)))).collect();
                let current = sizes.iter().position(|&s| s == c.letterbox_size).map(|i| i + 1);
                let (mw, mh) = self.monitor_size;
                (names, current, tr!("screen {mw}x{mh}", "画面 {mw}x{mh}", "экран {mw}x{mh}";))
            }
            Item::InterfaceHides => {
                let names = hud::Part::ALL.iter().map(|&p| format!("{}  {}", if c.interface_hides.has(p) { "[x]" } else { "[ ]" }, p.label())).collect();
                (names, None, tr!("hidden when toggled off", "オフにした時に隠すもの", "прячется при выключении").into())
            }
            Item::NoteColours => {
                let own = c.note_colours.custom;
                let custom = format!("{}  #{:06x} / #{:06x}", multiverse::palette_label(multiverse::CUSTOM), own[0], own[1]);
                let names = (0..multiverse::PALETTES.len()).map(|i| multiverse::palette_label(i).to_string()).chain(std::iter::once(custom)).collect();
                (names, Some(c.note_colours.palette), tr!("don / kat", "ドン / カッ", "дон / кат").into())
            }
            Item::SkinParts => {
                let names = skin::Part::ALL.iter().map(|&p| format!("{}:  {}", p.label(), self.skin_source_name(c.skin_parts.get(p)))).collect();
                (names, None, tr!("Enter: where it comes from", "Enter: 出どころを選ぶ", "Enter: откуда берётся").into())
            }
            Item::SkinPart(i) => {
                let Some(&part) = skin::Part::ALL.get(i) else { return };
                let skins: Vec<String> = skin::list().into_iter().map(|(n, _)| n).collect();
                let main = if c.skin.is_empty() { built_in_text().to_string() } else { c.skin.clone() };
                let picked = tr!("The skin picked ({main})", "選んだスキン ({main})", "Выбранный скин ({main})";);
                let names: Vec<String> = [picked, tr!("Built-in", "内蔵", "Встроенный").into()].into_iter().chain(skins.iter().cloned()).collect();
                let current = match c.skin_parts.get(part) {
                    skin::Source::Main => Some(0),
                    skin::Source::BuiltIn => Some(1),
                    skin::Source::Skin(n) => skins.iter().position(|s| s == n).map(|k| k + 2),
                };
                (names, current, tr!("Esc: back to the parts", "Esc: パーツに戻る", "Esc: назад к частям").into())
            }
            Item::Skin => {
                let skins = skin::list();
                let n = skins.len();
                let note = tr!("{n} installed", "{n}個インストール済み", "установлено: {n}";);
                let names: Vec<String> = std::iter::once(built_in_text().to_string()).chain(skins.into_iter().map(|(n, _)| n)).collect();
                let current = if c.skin.is_empty() { Some(0) } else { names.iter().skip(1).position(|n| *n == c.skin).map(|i| i + 1) };
                (names, current, note)
            }
            Item::SongsFolder => {
                let mut names = vec![type_path(), tr!("Find osu!'s Songs folder automatically", "osu!のSongsフォルダを自動で探す", "Искать папку Songs osu! самому").to_string()];
                if !cfg!(target_os = "android") {
                    names.insert(0, browse_text());
                }
                let note = match (&c.songs_folder, self.song_dirs.first()) {
                    (Some(dir), _) => dir.display().to_string(),
                    (None, Some(dir)) => {
                        let dir = dir.display();
                        tr!("found {dir}", "見つけた: {dir}", "найдена {dir}";)
                    }
                    (None, None) => tr!("none found", "見つかりません", "не найдена").into(),
                };
                (names, None, note)
            }
            Item::DataFolder => {
                let mut names = vec![type_path(), tr!("Back to the usual places", "いつもの場所に戻す", "Вернуть в обычные места").to_string()];
                if !cfg!(target_os = "android") {
                    names.insert(0, browse_text());
                }
                (names, None, config::data_dir().map(|d| d.display().to_string()).unwrap_or_default())
            }
            _ => return,
        };
        self.settings_ui.choices = Some(Choices::new(item, names, current, note));
    }

    /// Uses row `i` of the open list of choices and closes it.
    fn pick_choice(&mut self, i: usize) {
        let Some(list) = self.settings_ui.choices.take() else { return };
        let Some(name) = list.names.get(i) else { return };
        match list.item {
            Item::TouchLayout => self.cfg.touch_layout = config::TouchLayout::ALL[i],
            Item::Screenshots => self.cfg.screenshot_to = config::ScreenshotTo::ALL[i],
            Item::AutoplayStyle => self.cfg.autoplay.style = autoplay::Style::ALL[i],
            Item::AutoplayBinding => self.cfg.autoplay.binding = i,
            Item::Language => self.cfg.language = lang::Lang::ALL[i],
            Item::PresentMode => self.cfg.present = PresentChoice::ALL[i],
            Item::AudioBuffer if Some(i) != list.current => self.set_audio_buffer(AUDIO_BUFFERS[i]),
            Item::UpdateChannel => self.set_update_channel([update::Channel::Stable, update::Channel::Dev][i]),
            Item::Accent => self.cfg.accent = render::ACCENTS[i].1,
            Item::NoteColours => self.cfg.note_colours.palette = i,
            Item::InterfaceHides => {
                // A tick on or off; the list stays open there.
                if let Some(&p) = hud::Part::ALL.get(i) {
                    self.cfg.interface_hides.toggle(p);
                }
                self.cfg.save();
                self.open_choices(Item::InterfaceHides);
                if let Some(ch) = &mut self.settings_ui.choices {
                    ch.cursor = i;
                }
                return;
            }
            Item::WindowSize if i == 0 => {
                self.start_edit(Item::WindowSize);
                return;
            }
            Item::LetterboxSize if i == 0 => {
                self.start_edit(Item::LetterboxSize);
                return;
            }
            Item::LetterboxSize => {
                if let Some(&size) = self.letterbox_sizes().get(i - 1) {
                    self.cfg.letterbox_size = size;
                }
            }
            Item::WindowSize => {
                self.window_request = self.window_sizes().get(i - 1).copied();
                return;
            }
            Item::SkinParts => {
                self.open_choices(Item::SkinPart(i));
                return;
            }
            Item::SkinPart(p) => {
                let source = match i {
                    0 => skin::Source::Main,
                    1 => skin::Source::BuiltIn,
                    _ => skin::Source::Skin(name.clone()),
                };
                if let Some(&part) = skin::Part::ALL.get(p)
                    && *self.cfg.skin_parts.get(part) != source
                {
                    self.cfg.skin_parts.set(part, source);
                    self.load_skin();
                }
                self.cfg.save();
                self.reopen_skin_parts(p);
                return;
            }
            Item::Skin => {
                let skin = if i == 0 { String::new() } else { name.clone() };
                if skin != self.cfg.skin {
                    self.cfg.skin = skin;
                    self.load_skin();
                }
                if let Some(a) = &self.audio {
                    a.sfx(audio::Sfx::Don);
                }
            }
            Item::DataFolder => {
                let i = if cfg!(target_os = "android") { i + 1 } else { i };
                match i {
                    0 => self.browse_folder(Item::DataFolder),
                    1 => self.start_edit(Item::DataFolder),
                    _ => {
                        if let Err(e) = self.move_data_to(None) {
                            self.toast(e);
                        }
                    }
                }
                return;
            }
            Item::SongsFolder => {
                // Without the "Browse..." row (Android) the others move up one.
                let i = if cfg!(target_os = "android") { i + 1 } else { i };
                match i {
                    0 => self.browse_folder(Item::SongsFolder),
                    1 => self.start_edit(Item::SongsFolder),
                    _ => {
                        if let Err(e) = self.set_songs_folder(None) {
                            self.toast(e);
                        }
                    }
                }
                return;
            }
            _ => return,
        }
        multiverse::set_note_colours(self.cfg.note_colours);
        self.cfg.save();
    }

    /// Starts typing a value for `item`, beginning with the one it has.
    fn start_edit(&mut self, item: Item) {
        let c = &self.cfg;
        let text = match item {
            Item::LetterboxSize => format!("{}x{}", c.letterbox_size.0, c.letterbox_size.1),
            Item::LetterboxX => format!("{:.0}", c.letterbox_pos.0),
            Item::LetterboxY => format!("{:.0}", c.letterbox_pos.1),
            Item::ScrollSpeed => format!("{:.2}", c.scroll_speed),
            Item::Rate => format!("{:.2}", c.rate),
            Item::Offset => format!("{:.0}", c.offset_ms),
            Item::VisualOffset => format!("{:.0}", c.visual_offset_ms),
            Item::VsyncTune => format!("{:.0}", c.vsync_tune_ms),
            Item::CalibrationBpm => format!("{:.0}", c.calibration_bpm),
            Item::ResumeDelay => format!("{:.2}", c.resume_delay_ms / 1000.0),
            Item::MasterVolume => format!("{:.0}", c.master_volume * 100.0),
            Item::MusicVolume => format!("{:.0}", c.music_volume * 100.0),
            Item::FxVolume => format!("{:.0}", c.fx_volume * 100.0),
            Item::BackgroundDim => format!("{:.0}", c.bg_dim * 100.0),
            Item::FpsCap => c.fps_cap.to_string(),
            Item::AccentHue => format!("#{:06x}", c.accent),
            Item::DonColour => format!("#{:06x}", c.note_colours.don()),
            Item::KatColour => format!("#{:06x}", c.note_colours.kat()),
            Item::WindowSize => format!("{:.0}x{:.0}", self.window_w, self.window_h),
            Item::SongsFolder => c.songs_folder.as_ref().or(self.song_dirs.first()).map(|d| d.display().to_string()).unwrap_or_default(),
            Item::DataFolder => config::data_dir().map(|d| d.display().to_string()).unwrap_or_default(),
            _ => return,
        };
        if let Some(i) = settings::ITEMS.iter().position(|(_, it)| *it == item) {
            self.settings_ui.cursor = i;
        }
        self.settings_ui.editing = Some(Edit::new(item, text));
    }

    /// Keys while typing a setting's value: Enter sets it, Esc leaves it as it was.
    fn edit_key(&mut self, code: KeyCode, text: Option<&str>) {
        let ctrl = self.modifiers_ctrl;
        let Some(edit) = &mut self.settings_ui.editing else { return };
        let max = if matches!(edit.item, Item::SongsFolder | Item::DataFolder) { 400 } else { 12 };
        match code {
            KeyCode::Escape => self.settings_ui.editing = None,
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let Some(edit) = self.settings_ui.editing.take() else { return };
                if let Err(e) = self.apply_typed(edit.item, &edit.text) {
                    self.toast(e);
                    self.settings_ui.editing = Some(Edit { fresh: false, ..edit });
                }
            }
            KeyCode::Backspace => edit.backspace(),
            KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                edit.nav(code);
            }
            _ if ctrl => {}
            _ => {
                if let Some(text) = text {
                    edit.type_text(text, max);
                }
            }
        }
    }

    /// Sets `item` to a typed value.
    fn apply_typed(&mut self, item: Item, text: &str) -> Result<(), String> {
        if item == Item::ScrollSpeed && self.mid_song() {
            return Err(scroll_in_select().into());
        }
        let number = || parse_number(text).ok_or_else(|| tr!("type a number", "数字を入力してください", "введите число").to_string());
        let percent = || number().map(|n| (n / 100.0).clamp(0.0, 1.0) as f32);
        let c = &mut self.cfg;
        match item {
            Item::LetterboxX => c.letterbox_pos.0 = number()?.round().clamp(-100.0, 100.0) as f32,
            Item::LetterboxY => c.letterbox_pos.1 = number()?.round().clamp(-100.0, 100.0) as f32,
            Item::ScrollSpeed => c.scroll_speed = ((number()? * 100.0).round() / 100.0).clamp(0.3, 4.0),
            Item::Rate => c.rate = ((number()? * 100.0).round() / 100.0).clamp(RATE_RANGE.0, RATE_RANGE.1),
            Item::Offset => c.offset_ms = number()?.round().clamp(-1000.0, 1000.0),
            Item::VisualOffset => c.visual_offset_ms = number()?.round().clamp(-1000.0, 1000.0),
            Item::VsyncTune => c.vsync_tune_ms = number()?.round().clamp(-200.0, 200.0),
            Item::CalibrationBpm => c.calibration_bpm = number()?.round().clamp(calibrate::BPM_RANGE.0, calibrate::BPM_RANGE.1),
            Item::ResumeDelay => c.resume_delay_ms = (number()? * 1000.0).round().clamp(0.0, 3000.0),
            Item::MasterVolume => c.master_volume = percent()?,
            Item::MusicVolume => c.music_volume = percent()?,
            Item::FxVolume => c.fx_volume = percent()?,
            Item::BackgroundDim => c.bg_dim = percent()?,
            Item::FpsCap => {
                let fps = number()?.round();
                c.fps_cap = if fps <= 0.0 { 0 } else { fps.clamp(30.0, 10000.0) as u32 };
            }
            Item::AccentHue => {
                let t = text.trim();
                let hex = if t.starts_with('#') { t.to_string() } else { format!("#{t}") };
                c.accent = render::parse_accent(t).or_else(|| render::parse_accent(&hex)).ok_or(tr!("type a colour like #ff8800", "#ff8800のように色を入力", "введите цвет вроде #ff8800"))?;
            }
            Item::LetterboxSize => {
                let (w, h) = layout::parse_size(text).ok_or(tr!("type a size like 1280x720", "1280x720のようにサイズを入力", "введите размер вроде 1280x720"))?;
                let (mw, mh) = self.monitor_size;
                if w > mw || h > mh {
                    return Err(tr!("{w}x{h} is bigger than the screen ({mw}x{mh})", "{w}x{h}は画面 ({mw}x{mh}) より大きいです", "{w}x{h} больше экрана ({mw}x{mh})";));
                }
                if w < MIN_WINDOW.0 as u32 || h < MIN_WINDOW.1 as u32 {
                    return Err(tr!("at least {}x{}", "{}x{}以上", "минимум {}x{}"; MIN_WINDOW.0, MIN_WINDOW.1));
                }
                c.letterbox_size = (w, h);
            }
            Item::WindowSize => {
                let (w, h) = layout::parse_size(text).ok_or(tr!("type a size like 1280x720", "1280x720のようにサイズを入力", "введите размер вроде 1280x720"))?;
                let (mw, mh) = self.monitor_size;
                if w > mw || h > mh {
                    return Err(tr!("{w}x{h} is bigger than the screen ({mw}x{mh})", "{w}x{h}は画面 ({mw}x{mh}) より大きいです", "{w}x{h} больше экрана ({mw}x{mh})";));
                }
                self.window_request = Some((w, h));
                return Ok(());
            }
            Item::DonColour | Item::KatColour => {
                let typed = multiverse::parse_hex(text).ok_or(tr!("type a colour like #eb452c", "#eb452cのように色を入力", "введите цвет вроде #eb452c"))?;
                let (don, kat) = (c.note_colours.don(), c.note_colours.kat());
                c.note_colours = if item == Item::DonColour { multiverse::Colours::own(typed, kat) } else { multiverse::Colours::own(don, typed) };
            }
            Item::SongsFolder | Item::DataFolder => {
                let t = text.trim().trim_matches('"');
                let path = match (t.strip_prefix("~/"), std::env::var_os("HOME")) {
                    (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
                    _ => PathBuf::from(t),
                };
                return if item == Item::SongsFolder { self.set_songs_folder(Some(path)) } else { self.move_data_to(Some(path)) };
            }
            _ => return Ok(()),
        }
        self.apply_volume();
        multiverse::set_note_colours(self.cfg.note_colours);
        self.cfg.save();
        Ok(())
    }

    fn browse_songs_folder(&mut self) {
        self.browse_folder(Item::SongsFolder);
    }

    /// Opens the system's folder dialog for the Songs folder or the game data folder (on its
    /// own thread: it blocks). With no dialog to show, the path is typed in settings instead.
    fn browse_folder(&mut self, item: Item) {
        if self.folder_pick.is_some() {
            return;
        }
        self.folder_pick_for = item;
        let current = if item == Item::DataFolder { config::data_dir() } else { self.cfg.songs_folder.clone().or_else(|| self.song_dirs.first().cloned()) };
        let start = current.or_else(|| std::env::var_os("HOME").map(PathBuf::from)).unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(library::pick_folder(&start));
        });
        self.folder_pick = Some(rx);
        self.select.picking_folder = true;
    }

    fn poll_folder_pick(&mut self) {
        let Some(rx) = &self.folder_pick else { return };
        let picked = match rx.try_recv() {
            Ok(picked) => picked,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Ok(None),
        };
        self.folder_pick = None;
        self.select.picking_folder = false;
        let item = self.folder_pick_for;
        match picked {
            Ok(Some(dir)) => {
                let done = if item == Item::DataFolder { self.move_data_to(Some(dir)) } else { self.set_songs_folder(Some(dir)) };
                if let Err(e) = done {
                    self.toast(e);
                }
            }
            Ok(None) => {}
            Err(e) => {
                if !matches!(self.scene, Scene::Settings { .. }) {
                    self.open_settings();
                }
                self.start_edit(item);
                self.toast(tr!("{e}: type the folder's path", "{e}: フォルダのパスを入力してください", "{e}: введите путь к папке";));
            }
        }
    }

    /// Moves the game's data (replays, scores, videos, taiko.rs's own Songs and Skins, caches)
    /// into `target` (into a taiko-rs folder there, unless it is one; None: back to the usual
    /// places), on its own thread. `poll_data_move` takes it from there.
    fn move_data_to(&mut self, target: Option<PathBuf>) -> Result<(), String> {
        if matches!(&self.scene, Scene::Settings { prev } if !matches!(**prev, Scene::Select)) {
            return Err(tr!("move the game data from song select", "ゲームデータは選曲画面で移動してください", "переносите данные из выбора песни").into());
        }
        if self.data_move.is_some() {
            return Err(tr!("the game data is already on its way", "ゲームデータはもう移動中です", "данные уже переносятся").into());
        }
        let target = target.map(|t| if t.file_name().is_some_and(|n| n == "taiko-rs") { t } else { t.join("taiko-rs") });
        if target == self.cfg.data_folder {
            return Err(tr!("the game data is there already", "ゲームデータはもうそこにあります", "данные уже там").into());
        }
        if let Some(t) = &target {
            if !t.parent().is_some_and(std::path::Path::is_dir) {
                let dir = t.parent().unwrap_or(t).display();
                return Err(tr!("{dir} isn't a folder", "{dir}はフォルダではありません", "{dir} — не папка";));
            }
            std::fs::create_dir_all(t).map_err(|e| format!("{}: {e}", t.display()))?;
        }
        let (done, total) = (Arc::new(std::sync::atomic::AtomicU64::new(0)), Arc::new(std::sync::atomic::AtomicU64::new(0)));
        let (tx, rx) = mpsc::channel();
        let (from, to) = (self.cfg.data_folder.clone(), target.clone());
        let (d, t) = (done.clone(), total.clone());
        std::thread::spawn(move || {
            let _ = tx.send(config::move_data(from.as_deref(), to.as_deref(), &d, &t));
        });
        self.data_move = Some(DataMove { rx, done, total, to: target });
        Ok(())
    }

    /// While the game data moves: its progress; once it has, the game uses it there (and finds
    /// the maps downloaded into its Songs folder there).
    fn poll_data_move(&mut self) {
        let Some(m) = &self.data_move else { return };
        let result = match m.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                let (done, total) = (m.done.load(Ordering::Relaxed) as f64, m.total.load(Ordering::Relaxed) as f64);
                let mb = |b: f64| b / 1e6;
                let (done, total) = (mb(done), mb(total));
                self.toast(tr!("moving the game data: {done:.0} of {total:.0} MB", "ゲームデータを移動中: {total:.0} MB中{done:.0} MB", "перенос данных: {done:.0} из {total:.0} МБ";));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(tr!("the move stopped", "移動が止まりました", "перенос прервался").into()),
        };
        let Some(m) = self.data_move.take() else { return };
        match result {
            Ok(()) => {
                self.cfg.data_folder = m.to;
                config::set_data_folder(self.cfg.data_folder.clone());
                self.cfg.save();
                let _ = self.set_songs_folder(self.cfg.songs_folder.clone());
                let at = config::data_dir().map(|d| d.display().to_string()).unwrap_or_default();
                self.toast(tr!("game data moved to {at}", "ゲームデータを{at}に移動しました", "данные перенесены в {at}";));
            }
            Err(e) => self.toast(tr!("couldn't move the game data: {e}", "ゲームデータを移動できませんでした: {e}", "не удалось перенести данные: {e}";)),
        }
    }

    /// Scans `folder` for maps from now on (None: look for osu!'s again) and rescans the library.
    fn set_songs_folder(&mut self, folder: Option<PathBuf>) -> Result<(), String> {
        if matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Play { .. })) {
            return Err(tr!("change the Songs folder from song select", "Songsフォルダは選曲画面で変えてください", "меняйте папку Songs в выборе песни").into());
        }
        let folder = folder.map(|f| library::picked_songs(&f));
        if let Some(f) = folder.as_ref().filter(|f| !f.is_dir()) {
            let dir = f.display();
            return Err(tr!("{dir} isn't a folder", "{dir}はフォルダではありません", "{dir} — не папка";));
        }
        self.cfg.songs_folder = folder.clone();
        self.cfg.save();
        library::set_songs_folder(folder);
        let inputs = song_inputs(&self.cfg);
        self.songs_dir = download_dir(&inputs);
        self.song_dirs = inputs.iter().filter(|p| p.is_dir()).cloned().collect();
        self.toast(match (&self.cfg.songs_folder, self.song_dirs.first()) {
            (Some(dir), _) => {
                let dir = dir.display();
                tr!("scanning {dir}", "{dir}をスキャン中", "сканируем {dir}";)
            }
            (None, Some(dir)) => {
                let dir = dir.display();
                tr!("found {dir}", "見つけた: {dir}", "найдена {dir}";)
            }
            (None, None) => tr!("no osu! Songs folder found", "osu!のSongsフォルダが見つかりません", "папка Songs osu! не найдена").into(),
        });
        self.scan = Some(start_scan(inputs));
        self.select.scanning = Some(0);
        Ok(())
    }

    /// Installs the newer build when there is one, else looks for one.
    fn update_now(&mut self) {
        if update::BUILD.is_none() {
            self.toast(tr!("a local build doesn't update itself", "ローカルビルドは自分で更新しません", "локальная сборка не обновляется сама").into());
            return;
        }
        match self.updater.status() {
            update::Status::Available { commit, .. } => {
                self.updater.install();
                let build = update::short(&commit);
                self.toast(tr!("downloading {build}...", "{build}をダウンロード中...", "загрузка {build}...";));
            }
            update::Status::Checking | update::Status::Downloading | update::Status::Installed => {}
            _ => {
                self.updater.check();
                self.toast(tr!("checking for updates...", "アップデートを確認中...", "проверяем обновления...").into());
            }
        }
    }

    /// The badge top right while there's an update to install (menus only), and the restart
    /// once it's installed.
    fn draw_update(&mut self, c: &mut Canvas) {
        // What's new, top right in the menus (left of the update badge when there is one).
        let menus = matches!(self.scene, Scene::Select | Scene::Settings { .. } | Scene::MpMenu | Scene::Download);
        if menus && self.splash.is_none() && self.changelog.is_none() && self.player.is_none() && self.guide.is_none() {
            let badge = match self.updater.status() {
                update::Status::Available { commit, .. } => Canvas::text_width(7.0, &update_badge(&commit)) + 20.0,
                update::Status::Downloading => Canvas::text_width(7.0, &updating_badge(1.0)) + 20.0,
                _ => 0.0,
            };
            let label = tr!("what's new", "更新内容", "что нового");
            let w = Canvas::text_width(7.0, label) + 14.0;
            let x = c.width - 8.0 - badge - w;
            let hover = c.hovered(x, 3.0, w, 14.0);
            c.round_rect([x, 3.0], [w, 14.0], 4.0, rgba(if hover { 0x2a2a38 } else { 0x1d1d27 }, 0.95), rgba(0x3a3a4e, 1.0), 1.0);
            c.text(x + 7.0, 6.5, 7.0, rgba(if hover { 0xffffff } else { 0xc8c8d8 }, 1.0), label);
            c.hit(x, 3.0, w, 14.0, render::Hit::Changelog);
            // Who's logged in and their pp, left of it: their profile on the website.
            let (name, rest) = match (&self.mp_menu.account, self.me) {
                (Some(name), Some((pp, rank))) => (name.clone(), format!("  {pp:.0} pp{}", rank.map(|r| format!("  #{r}")).unwrap_or_default())),
                (Some(name), None) => (name.clone(), String::new()),
                (None, _) => (tr!("log in", "ログイン", "войти").to_string(), String::new()),
            };
            let aw = Canvas::text_width(7.0, &name) + Canvas::text_width(7.0, &rest) + 14.0;
            let ax = x - 6.0 - aw;
            let hover = c.hovered(ax, 3.0, aw, 14.0);
            c.round_rect([ax, 3.0], [aw, 14.0], 4.0, rgba(if hover { 0x2a2a38 } else { 0x1d1d27 }, 0.95), rgba(if self.mp_menu.account.is_some() { accent() } else { 0x3a3a4e }, 1.0), 1.0);
            let nw = c.text(ax + 7.0, 6.5, 7.0, rgba(0xffffff, 1.0), &name);
            c.text(ax + 7.0 + nw, 6.5, 7.0, rgba(0xff8ad8, 1.0), &rest);
            c.hit(ax, 3.0, aw, 14.0, render::Hit::Account);
        }
        let text = match self.updater.status() {
            update::Status::Available { commit, .. } => update_badge(&commit),
            update::Status::Downloading => updating_badge(self.updater.progress()),
            update::Status::Installed => {
                match update::restart() {
                    Ok(()) => self.quit = true,
                    Err(e) => {
                        self.toast(tr!("installed; restart taiko.rs ({e})", "インストールしました。taiko.rsを再起動してください ({e})", "установлено; перезапустите taiko.rs ({e})";));
                        self.updater = update::Updater::new(false);
                    }
                }
                return;
            }
            _ => return,
        };
        if self.in_song() {
            return;
        }
        let w = Canvas::text_width(7.0, &text) + 14.0;
        let x = c.width - 8.0 - w;
        let hover = c.hovered(x, 3.0, w, 14.0);
        c.round_rect([x, 3.0], [w, 14.0], 4.0, rgba(if hover { 0x2f8f4a } else { 0x1f6b36 }, 0.95), rgba(0x86E05A, 1.0), 1.0);
        c.text(x + 7.0, 6.5, 7.0, rgba(0xffffff, 1.0), &text);
        c.hit(x, 3.0, w, 14.0, render::Hit::Update);
    }

    /// Loads the configured skin: its note images go to the renderer, its hitsounds to the audio
    /// engine.
    /// How the settings name where a skin part comes from.
    fn skin_source_name(&self, source: &skin::Source) -> String {
        match source {
            skin::Source::Main if self.cfg.skin.is_empty() => {
                let skin = built_in_text();
                tr!("the skin ({skin})", "スキン ({skin})", "скин ({skin})";)
            }
            skin::Source::Main => {
                let skin = &self.cfg.skin;
                tr!("the skin ({skin})", "スキン ({skin})", "скин ({skin})";)
            }
            skin::Source::BuiltIn => built_in_text().into(),
            skin::Source::Skin(name) => name.clone(),
        }
    }

    /// The list of skin parts, open at part `i`.
    fn reopen_skin_parts(&mut self, i: usize) {
        self.open_choices(Item::SkinParts);
        if let Some(ch) = &mut self.settings_ui.choices {
            ch.cursor = i;
        }
    }

    fn load_skin(&mut self) {
        let skin = skin::load(&self.cfg.skin, &self.cfg.skin_parts, self.audio.as_ref().map(|a| a.rate), self.skin_map.as_deref());
        self.sprites_upload = Some(skin.images);
        if let Some(a) = &self.audio {
            a.set_hitsounds(skin.hitsounds);
        }
    }

    /// Swaps the playing map's own skin images in, and out again after: only when it has some,
    /// so ordinary maps never reload the skin.
    // ponytail: Play and song select's preview only (and their settings); multiplayer /
    // spectating keep your skin.
    fn follow_map_skin(&mut self) {
        let entry = match &self.scene {
            Scene::Play { entry, .. } => Some(*entry),
            Scene::Select | Scene::Preview { .. } => self.preview_target(),
            Scene::Settings { prev } => match **prev {
                Scene::Play { entry, .. } => Some(entry),
                Scene::Select => self.preview_target(),
                _ => None,
            },
            _ => None,
        };
        let dir = entry.and_then(|e| self.library.get(e)).and_then(|e| e.path.parent()).map(Path::to_path_buf);
        if dir == self.skin_map_seen {
            return;
        }
        self.skin_map_seen = dir.clone();
        let want = dir.filter(|d| skin::map_has_images(d));
        if want != self.skin_map {
            self.skin_map = want;
            self.load_skin();
        }
    }

    fn open_skins_folder(&mut self) {
        let Some(dir) = skin::own_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
        if std::process::Command::new(opener).arg(&dir).spawn().is_err() {
            let dir = dir.display();
            self.toast(tr!("skins go in {dir}", "スキンは{dir}に置きます", "скины кладутся в {dir}";));
        }
    }

    /// Adds maps to the end of the library, so the indices already handed out stay valid.
    fn append_maps(&mut self, fresh: Vec<Entry>) {
        let start = self.library.len();
        self.library.extend(fresh);
        // What's known of their sets' statuses, and the rest asked about soon.
        self.apply_cached_statuses();
        self.status_sweep_due = Some(Instant::now() + Duration::from_secs(10));
        if let Some(info) = &self.mapinfo {
            info.extend(self.library[start..].iter().enumerate().map(|(k, e)| (start + k, e.path.clone(), e.modified, e.size)).collect());
            self.mapinfo_sorted = false;
        }
        self.select.refilter(&self.library);
    }

    /// Files dragged over the window and dropped on it: .osz maps are unpacked into the Songs
    /// folder, join the list, and the file is deleted (it stays if unpacking fails).
    fn file_drag(&mut self, drag: FileDrag) {
        match drag {
            FileDrag::Hover => {
                self.drag_hover.get_or_insert_with(Instant::now);
            }
            FileDrag::Left => self.drag_hover = None,
            FileDrag::Drop(paths) => {
                self.drag_hover = None;
                for path in paths {
                    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("osz")) {
                        self.import_osz(path, &name, false);
                    } else {
                        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        self.toast(tr!("{file} isn't a map: drop .osz files", "{file}は譜面ではありません: .oszファイルをドロップしてください", "{file} — не карта: бросайте файлы .osz";));
                    }
                }
            }
        }
    }

    /// Unpacks an `.osz` into the Songs folder, unless everything in it is already in the list or
    /// on its way (the same file dropped twice). A set that's partly here (a new difficulty) is
    /// unpacked into its own folder when that's in the Songs folder. `temporary`: delete the
    /// archive whatever happens (a copy, not the player's own file).
    fn import_osz(&mut self, archive: PathBuf, name: &str, temporary: bool) {
        let maps = match download::archive_maps(&archive) {
            Ok(maps) if !maps.is_empty() => maps,
            result => {
                if temporary {
                    let _ = std::fs::remove_file(&archive);
                }
                self.toast(match result {
                    Err(e) => format!("{name}: {e}"),
                    Ok(_) => tr!("{name} has no maps in it", "{name}には譜面がありません", "в {name} нет карт";),
                });
                return;
            }
        };
        let pending: std::collections::HashSet<u64> = self.downloads.iter().flat_map(|j| j.maps.iter().copied()).collect();
        let (fresh, home) = library::osz_news(&self.library, &pending, &maps);
        if fresh == 0 {
            if temporary {
                let _ = std::fs::remove_file(&archive);
            }
            let here = home.map(|home| main_maps(self.library.iter().filter(|e| e.path.parent() == Some(home))));
            self.reveal_maps(here.unwrap_or_default());
            self.toast(tr!("{name} is already in your songs", "{name}はもう曲の中にあります", "{name} уже есть среди ваших песен";));
            return;
        }
        let folder = home.filter(|d| d.parent() == Some(self.songs_dir.as_path())).and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned());
        self.toast(tr!("importing {name}...", "{name}を取り込み中...", "добавляем {name}...";));
        let job = download::import(archive, folder.as_deref().unwrap_or(name), self.songs_dir.clone(), temporary, maps.into_iter().map(|m| m.1).collect());
        self.downloads.push(job);
    }

    /// While files are dragged over the window: where they go when dropped.
    fn draw_drag_hover(&self, canvas: &mut Canvas, now: Instant) {
        let Some(since) = self.drag_hover else { return };
        let a = (now.saturating_duration_since(since).as_secs_f32() / 0.12).min(1.0);
        let (w, h) = (canvas.width, render::VIRTUAL_HEIGHT);
        canvas.rect(0.0, 0.0, w, h, rgba(0x000000, 0.6 * a));
        canvas.round_rect([24.0, 24.0], [w - 48.0, h - 48.0], 16.0, rgba(0x000000, 0.0), rgba(accent(), 0.9 * a), 2.5);
        let drop = tr!("Drop .osz maps to add them", ".oszの譜面をドロップで追加", "Бросьте карты .osz, чтобы добавить");
        canvas.text_centered(w / 2.0, h / 2.0 - 22.0, 18.0, rgba(0xffffff, a), &Canvas::fit(18.0, w - 64.0, drop));
        let dir = self.songs_dir.display();
        let to = Canvas::fit(9.0, w - 96.0, &tr!("into {dir}", "追加先: {dir}", "в {dir}";));
        canvas.text_centered(w / 2.0, h / 2.0 + 14.0, 9.0, rgba(0x9a9ab0, a), &to);
    }

    /// Keeps the Discord status in line with what's on screen (checked about once a second).
    fn update_presence(&mut self) {
        if self.presence_at.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.presence_at = Some(Instant::now());
        let app_id = if self.cfg.discord_app_id.is_empty() { discord::DEFAULT_APP_ID } else { self.cfg.discord_app_id.as_str() };
        if !self.cfg.discord_presence || app_id.is_empty() {
            // Dropping the connection clears the status.
            self.discord = None;
            self.presence = None;
            return;
        }
        let app_id = app_id.to_string();
        let Some((details, state)) = self.presence_text() else { return };
        let discord = self.discord.get_or_insert_with(|| discord::Presence::start(app_id));
        // The timer restarts when the top line changes (a new map, a new screen).
        let start = match &self.presence {
            Some(p) if p.details == details => p.start,
            _ => discord::unix_now(),
        };
        let activity = discord::Activity { details, state, start };
        if self.presence.as_ref() != Some(&activity) {
            discord.set(Some(activity.clone()));
            self.presence = Some(activity);
        }
    }

    /// The two lines of the Discord status for the current screen.
    fn presence_text(&self) -> Option<(String, String)> {
        let map_label = |m: &net::MapInfo| format!("{} - {} [{}]", m.artist, m.title, m.version);
        let players = |s: &multiplayer::Session| {
            let n = s.players.len();
            let s = if n == 1 { "" } else { "s" };
            let ru = lang::ru_plural(n, ["игрок", "игрока", "игроков"]);
            tr!("{n} player{s}", "{n}人", "{n} {ru}";)
        };
        let rate_text = |rate: f64| if (rate - 1.0).abs() > 1e-6 { format!("  {rate:.2}x") } else { String::new() };
        let lobby = |s: &multiplayer::Session| {
            let n = players(s);
            let details = if s.is_host() {
                tr!("Multiplayer lobby ({n}, hosting)", "マルチのロビー ({n}、ホスト)", "Лобби мультиплеера ({n}, хост)";)
            } else {
                tr!("Multiplayer lobby ({n}, in)", "マルチのロビー ({n}、参加中)", "Лобби мультиплеера ({n}, участник)";)
            };
            let state = s.map.as_ref().map(map_label).unwrap_or_else(|| tr!("picking a map", "譜面を選択中", "выбирает карту").into());
            (details, state)
        };
        Some(match &self.scene {
            Scene::Loading { entry, .. } => (self.library.get(*entry)?.label(), tr!("Loading...", "読み込み中...", "Загрузка...").into()),
            Scene::Play { game, entry, .. } => {
                let label = self.library.get(*entry)?.label();
                let state = if let Some(who) = &game.replay_of {
                    tr!("Watching {who}'s replay", "{who}のリプレイを観戦中", "Смотрит повтор {who}";)
                } else if game.paused.is_some() {
                    tr!("Paused", "ポーズ中", "Пауза").into()
                } else {
                    let stars = self.stars.get(*entry, game.rate).map(|s| format!("  {s:.2}*")).unwrap_or_default();
                    let rate = rate_text(game.rate);
                    tr!("Playing{stars}{rate}", "プレイ中{stars}{rate}", "Играет{stars}{rate}";)
                };
                (label, state)
            }
            Scene::Results { entry, .. } => (self.library.get(*entry)?.label(), tr!("Looking at the results", "リザルトを見ています", "Смотрит итоги").into()),
            Scene::MpWait { .. } | Scene::MpPlay { .. } => {
                let s = self.mp.as_ref()?;
                let label = s.map.as_ref().map(map_label).unwrap_or_default();
                let (n, rate) = (players(s), rate_text(s.map.as_ref().map(|m| m.rate).unwrap_or(1.0)));
                (label, tr!("Multiplayer match, {n}{rate}", "マルチの対戦、{n}{rate}", "Матч мультиплеера, {n}{rate}";))
            }
            Scene::Spectate(v) => {
                let label = self.mp.as_ref().and_then(|s| s.map.as_ref()).map(map_label).unwrap_or_default();
                let who = v.focus.and_then(|k| v.games.get(k)).and_then(|(id, _)| v.players.iter().find(|p| p.0 == *id)).map(|p| p.1.clone());
                let state = match who {
                    Some(w) => tr!("Spectating {w}", "{w}を観戦中", "Наблюдает за {w}";),
                    None => tr!("Spectating everyone", "全員を観戦中", "Наблюдает за всеми").into(),
                };
                (label, state)
            }
            Scene::Watch(v) => {
                let who = &v.player;
                (tr!("Watching {who}", "{who}を観戦中", "Смотрит {who}";), v.play.as_ref().map(|p| p.map.label()).unwrap_or_default())
            }
            Scene::MpResults => (self.mp.as_ref().and_then(|s| s.map.as_ref()).map(map_label).unwrap_or_default(), tr!("Multiplayer results", "マルチのリザルト", "Итоги мультиплеера").into()),
            Scene::VideoEdit(ed) => (tr!("Making a replay video", "リプレイの動画を作成中", "Делает видео повтора").into(), ed.replay.map_label.clone()),
            // Anywhere else while in a room (its lobby, or song select picking its map).
            _ if self.mp.is_some() => lobby(self.mp.as_ref()?),
            Scene::MpMenu => (tr!("Multiplayer menu", "マルチのメニュー", "Меню мультиплеера").into(), String::new()),
            Scene::Select if self.player.is_some() => {
                let song = self.player.as_ref().and_then(|p| p.list.entry()).and_then(|i| self.library.get(i)).map(Entry::label);
                (tr!("Listening to music", "音楽を聴いている", "Слушает музыку").into(), song.unwrap_or_default())
            }
            Scene::Download => (tr!("Looking for new maps", "新しい譜面を探しています", "Ищет новые карты").into(), String::new()),
            Scene::Settings { .. } | Scene::LayoutEdit { .. } => (tr!("Tweaking settings", "設定を調整中", "Копается в настройках").into(), String::new()),
            Scene::Calibrate { .. } | Scene::CalibrateDone { .. } => (tr!("Calibrating", "調整中", "Калибровка").into(), String::new()),
            _ => (tr!("Choosing a song", "曲を選んでいます", "Выбирает песню").into(), self.select.selected().and_then(|i| self.library.get(i)).map(|e| e.label()).unwrap_or_default()),
        })
    }

    /// Opens the file manager on `path`'s folder with the file selected where it can.
    fn show_in_folder(&mut self, path: &std::path::Path) {
        use std::process::Command;
        #[cfg(windows)]
        let opened = {
            // Explorer wants `/select,"C:\full\path"`: the whole argument quoted (as `arg` would,
            // since video names have spaces) or a relative or forward-slashed path opens Documents.
            use std::os::windows::process::CommandExt;
            let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
            let mut arg = std::ffi::OsString::from("/select,\"");
            arg.push(full.as_os_str());
            arg.push("\"");
            Command::new("explorer").raw_arg(arg).spawn().is_ok()
        };
        #[cfg(not(windows))]
        let opened = if cfg!(target_os = "macos") {
            Command::new("open").arg("-R").arg(path).spawn().is_ok()
        } else {
            // The freedesktop file manager interface (Dolphin, Nautilus, ...) selects the file;
            // otherwise just open its folder.
            let uri = file_uri(path);
            let selected = Command::new("dbus-send")
                .args(["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "--type=method_call", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems"])
                .arg(format!("array:string:{uri}"))
                .arg("string:")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            selected || path.parent().is_some_and(|dir| Command::new("xdg-open").arg(dir).spawn().is_ok())
        };
        if !opened {
            self.toast(tr!("the video is in {}", "動画は{}にあります", "видео в {}"; path.parent().unwrap_or(path).display()));
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
        if let Some(p) = &mut self.settings_ui.choices {
            match code {
                KeyCode::ArrowUp => p.move_cursor(-1),
                KeyCode::ArrowDown => p.move_cursor(1),
                KeyCode::PageUp => p.move_cursor(-8),
                KeyCode::PageDown => p.move_cursor(8),
                KeyCode::Home => p.move_cursor(-(p.names.len() as i32)),
                KeyCode::End => p.move_cursor(p.names.len() as i32),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    let i = p.cursor;
                    self.pick_choice(i);
                }
                // From one part's skins, back to the parts.
                KeyCode::Escape | KeyCode::Backspace if let Item::SkinPart(i) = p.item => self.reopen_skin_parts(i),
                KeyCode::Escape | KeyCode::Backspace | KeyCode::Tab => self.settings_ui.choices = None,
                _ => {}
            }
            return;
        }
        if let Some(item) = self.settings_ui.binding.take() {
            let modifier = matches!(
                code,
                KeyCode::ShiftLeft | KeyCode::ShiftRight | KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::AltLeft | KeyCode::AltRight | KeyCode::SuperLeft | KeyCode::SuperRight
            );
            if matches!(item, Item::InterfaceKey | Item::Hotkey(_)) && modifier {
                // Held for the key to come.
                self.settings_ui.binding = Some(item);
                return;
            }
            if code == KeyCode::Delete {
                match item {
                    Item::Key(slot) => self.cfg.keys[slot] = config::NO_KEY,
                    Item::PresetKey(slot) => self.cfg.preset_keys[slot] = config::NO_KEY,
                    Item::InterfaceKey => self.cfg.interface_key = (false, config::NO_KEY),
                    Item::Hotkey(i) => {
                        self.cfg.hotkeys.set(keys::Hotkey::ALL[i], None);
                    }
                    _ => {}
                }
                self.toast(tr!("{}: no key", "{}: キーなし", "{}: без клавиши"; item.label()));
                self.cfg.save();
                return;
            }
            if code != KeyCode::Escape {
                match item {
                    Item::Hotkey(i) => {
                        let h = keys::Hotkey::ALL[i];
                        let chord = keys::Chord { key: code, ctrl: self.modifiers_ctrl, shift: self.modifiers_shift, alt: self.modifiers_alt };
                        if self.cfg.keys.contains(&code) && !chord.ctrl && !chord.alt && h.places().iter().any(|p| matches!(p, keys::Place::Play | keys::Place::Paused)) {
                            self.toast(tr!("{} is a drum key", "{}は太鼓のキーです", "{} — клавиша барабана"; config::key_name(code)));
                            return;
                        }
                        if let Some(other) = self.cfg.hotkeys.set(h, Some(chord)) {
                            self.toast(tr!("{} now has no key ({} was its)", "{}はキーなしになりました ({}だったもの)", "у «{}» больше нет клавиши (была {})"; other.label(), chord.name()));
                        }
                    }
                    Item::InterfaceKey => {
                        if self.cfg.keys.contains(&code) && !self.modifiers_shift {
                            self.toast(tr!("{} is a drum key", "{}は太鼓のキーです", "{} — клавиша барабана"; config::key_name(code)));
                            return;
                        }
                        self.cfg.interface_key = (self.modifiers_shift, code);
                    }
                    Item::Key(slot) => {
                        if !self.bind_drum_key(slot, code) {
                            return;
                        }
                    }
                    Item::PresetKey(slot) => {
                        if self.cfg.keys.contains(&code) {
                            self.toast(tr!("{} is a drum key", "{}は太鼓のキーです", "{} — клавиша барабана"; config::key_name(code)));
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
        if code == KeyCode::Backspace && item.kind() == settings::Kind::Key {
            let d = Config::default();
            match item {
                Item::Key(slot) => {
                    let key = d.keys[slot];
                    if let Some(other) = self.cfg.keys.iter().position(|&k| k == key) {
                        self.cfg.keys[other] = self.cfg.keys[slot];
                    }
                    self.cfg.keys[slot] = key;
                }
                Item::PresetKey(slot) => self.cfg.preset_keys[slot] = d.preset_keys[slot],
                Item::InterfaceKey => self.cfg.interface_key = d.interface_key,
                Item::Hotkey(i) => {
                    let h = keys::Hotkey::ALL[i];
                    if let Some(other) = self.cfg.hotkeys.set(h, h.first_key()) {
                        self.toast(tr!("{} now has no key", "{}はキーなしになりました", "у «{}» больше нет клавиши"; other.label()));
                    }
                }
                _ => {}
            }
            self.cfg.save();
            return;
        }
        match code {
            KeyCode::ArrowUp => self.settings_ui.move_cursor(-1),
            KeyCode::ArrowDown => self.settings_ui.move_cursor(1),
            KeyCode::PageUp | KeyCode::PageDown if !self.settings_ui.search.is_empty() => {
                self.settings_ui.move_cursor(if code == KeyCode::PageDown { 8 } else { -8 });
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                let n = settings::SECTIONS.len() as i32;
                let dir = if code == KeyCode::PageDown { 1 } else { -1 };
                let next = (self.settings_ui.section() as i32 + dir).rem_euclid(n) as usize;
                self.settings_ui.cursor = settings::section_start(next);
            }
            KeyCode::ArrowLeft => self.adjust_setting(item, -1, window),
            KeyCode::ArrowRight => self.adjust_setting(item, 1, window),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Calibrate => self.start_calibration_as(false),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Guide => self.guide = Some(0),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::ResetMapOffsets => self.press_reset_map_offsets(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::VsyncCalibrate => self.start_calibration_as(true),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::PlayfieldArea => self.open_layout_editor(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::PlayfieldPreset => self.open_dropdown(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::SkinFolder => self.open_skins_folder(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::HiddenMaps => {
                if self.select.hidden.is_empty() {
                    self.toast(tr!("nothing is hidden: right-click a song in the list to hide it", "何も隠れていません: リストの曲を右クリックで隠せます", "ничего не скрыто: правый клик по песне в списке прячет её").into());
                } else {
                    let what = self.select.hidden.describe();
                    self.select.unhide_all(&self.library);
                    self.toast(tr!("{what} back in the song list", "{what}を曲リストに戻しました", "{what} снова в списке песен";));
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item.kind() == Kind::List => self.open_choices(item),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item.kind() == Kind::Value => self.start_edit(item),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Update => self.update_now(),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::KrokodilKoban && self.settings_ui.krokodil => match library::launch_osu() {
                Ok(()) => self.quit = true,
                Err(e) => self.toast(e),
            },
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::SongSelectLayout => {
                self.reset_song_layout(None);
                self.toast(tr!("song select: the usual layout", "選曲画面: いつものレイアウト", "выбор песни: обычный макет").into());
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Changelog => self.changelog = Some(changelog::View::recent(false)),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::PlayfieldReset => {
                self.cfg.playfield_area = Area::FULL;
                self.cfg.save();
                self.toast(tr!("playfield fills the screen", "プレイフィールドを全画面にしました", "поле на весь экран").into());
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.adjust_setting(item, 1, window),
            KeyCode::Tab if matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Select)) => {
                self.open_multiplayer();
            }
            KeyCode::Escape | KeyCode::Tab | KeyCode::F1 => self.close_settings(),
            _ => {}
        }
    }

    /// Offset calibration, or again the kind just finished (Retry).
    fn start_calibration(&mut self) {
        let vsync = matches!(self.scene, Scene::CalibrateDone { vsync: true, .. });
        self.start_calibration_as(vsync);
    }

    /// Offset calibration, or with `vsync` the vsync compensation's: the same notes, in vsync and
    /// with no sound, hit by sight.
    fn start_calibration_as(&mut self, vsync: bool) {
        if matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Play { .. })) {
            self.toast(tr!("quit the song first to calibrate", "調整するには先に曲をやめてください", "сначала выйдите из песни").into());
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
        let mut track = audio::metronome(rate, beat_ms, beats, 2000.0);
        if vsync {
            // Silence the length of the click track: only the notes to go by.
            track.samples.iter_mut().for_each(|s| *s = 0.0);
        }
        let track = Arc::new(track);
        let mut game = Game::new(map, track, self.audio.as_ref());
        // Your own hitsounds would let you sync to yourself instead of to the click.
        game.mute_hits = true;
        game.show_hp = false;
        self.scene = Scene::Calibrate { game: Box::new(game), back: Box::new(back), vsync };
    }

    fn finish_calibration(&mut self) {
        let Scene::Calibrate { game, back, vsync } = std::mem::replace(&mut self.scene, Scene::Select) else { return };
        self.stop_audio();
        // By sight in vsync, hitting late means the frames reach the screen later than assumed.
        let current = if vsync { self.cfg.vsync_tune_ms } else { self.cfg.offset_ms };
        match calibrate::estimate(&game.stats.errors, current) {
            Some(estimate) if estimate.hits >= 4 => self.scene = Scene::CalibrateDone { estimate, back, vsync },
            _ => {
                self.scene = *back;
                self.toast(tr!("calibration needs more hits", "調整にはもっと叩く必要があります", "для калибровки нужно больше ударов").into());
            }
        }
    }

    fn leave_calibration(&mut self, apply: bool) {
        self.stop_audio();
        match std::mem::replace(&mut self.scene, Scene::Select) {
            Scene::Calibrate { back, .. } => self.scene = *back,
            Scene::CalibrateDone { estimate, back, vsync } => {
                if apply && vsync {
                    self.cfg.vsync_tune_ms = estimate.suggested_offset.clamp(-200.0, 200.0);
                    self.cfg.save();
                    self.toast(tr!("vsync compensation fine tune set to {:+.0} ms", "垂直同期の補正の微調整を{:+.0} msにしました", "точная компенсация vsync: {:+.0} мс"; self.cfg.vsync_tune_ms));
                } else if apply {
                    self.cfg.offset_ms = estimate.suggested_offset;
                    self.cfg.save();
                    self.toast(tr!("offset set to {:+.0} ms", "オフセットを{:+.0} msにしました", "смещение: {:+.0} мс"; estimate.suggested_offset));
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
                    game.press(calibrate::centre_key(k), at, self.audio.as_ref(), self.play_offset);
                }
                return;
            }
            if repeat {
                return;
            }
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter => self.finish_calibration(),
                KeyCode::Escape => self.leave_calibration(false),
                KeyCode::F11 => self.toggle_fullscreen(window),
                KeyCode::F12 => self.screenshot_due = true,
                KeyCode::KeyQ if self.modifiers_ctrl => self.ask_quit(),
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
            KeyCode::F12 => self.screenshot_due = true,
            KeyCode::KeyQ if self.modifiers_ctrl => self.ask_quit(),
            _ => {}
        }
    }

    /// Applies the live offset suggestion from the song being played (O key).
    /// Applies the live offset suggestion from the song being played (O key) to that map.
    fn apply_live_offset(&mut self) {
        // Shift+O and a match don't go through the hotkey, so this says it for them all.
        if self.cfg.hotkeys.get(keys::Hotkey::ApplyOffset).is_none() {
            return;
        }
        let Some(entry) = self.song_entry() else { return };
        let (Scene::Play { game, .. } | Scene::MpPlay { game, .. }) = &self.scene else { return };
        match game.offset_suggestion(self.play_offset) {
            Some(h) if !h.significant => {
                let text = tr!("offset already within +-{:.1} ms of your hits", "オフセットはもう叩きの±{:.1} ms以内です", "смещение уже в пределах +-{:.1} мс от ударов"; h.estimate.margin_ms);
                self.toast(text);
            }
            // Shift+O: the suggestion is about every map (the same on all of them), so it goes
            // to the global offset.
            Some(h) if self.modifiers_shift => {
                let was = self.cfg.offset_ms;
                self.cfg.offset_ms = h.estimate.suggested_offset - self.map_offset(Some(entry));
                self.cfg.save();
                self.refresh_play_offset();
                self.toast(tr!("offset {:+.0} ms for all maps (was {was:+.0})", "全譜面のオフセット {:+.0} ms (前は{was:+.0})", "смещение {:+.0} мс для всех карт (было {was:+.0})"; self.cfg.offset_ms));
            }
            Some(h) => {
                let was = self.map_offset(Some(entry));
                let map = h.estimate.suggested_offset - self.cfg.offset_ms;
                self.set_map_offset(entry, map);
                self.toast(tr!("this map: offset {map:+.0} ms (was {was:+.0})", "この譜面: オフセット {map:+.0} ms (前は{was:+.0})", "эта карта: смещение {map:+.0} мс (было {was:+.0})";));
            }
            None => self.toast(tr!("need at least 10 hits for a suggestion", "提案には10回以上叩く必要があります", "для подсказки нужно хотя бы 10 ударов").into()),
        }
    }

    /// The map played now, or whose results are up (its own offset is what -/+ and O change).
    fn song_entry(&self) -> Option<usize> {
        match &self.scene {
            Scene::Loading { entry, .. } | Scene::Play { entry, .. } | Scene::Results { entry, .. } => Some(*entry),
            Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults => self.mp.as_ref().and_then(|s| s.map_entry),
            _ => None,
        }
    }

    /// The map whose offset applies on this screen (also the selected one, for the preview).
    fn offset_entry(&self) -> Option<usize> {
        match &self.scene {
            Scene::Spectate(_) | Scene::Lobby => self.mp.as_ref().and_then(|s| s.map_entry),
            Scene::Watch(v) => v.entry,
            Scene::Select | Scene::Preview { .. } => self.select.selected(),
            _ => self.song_entry(),
        }
    }

    fn map_offset(&self, entry: Option<usize>) -> f64 {
        entry.and_then(|i| self.library.get(i)).and_then(|e| self.map_offsets.get(&e.hash())).copied().unwrap_or(0.0)
    }

    fn set_map_offset(&mut self, entry: usize, ms: f64) {
        let Some(e) = self.library.get(entry) else { return };
        let ms = ms.round();
        if ms == 0.0 {
            self.map_offsets.remove(&e.hash());
        } else {
            self.map_offsets.insert(e.hash(), ms);
        }
        config::save_map_offsets(&self.map_offsets);
        self.refresh_play_offset();
    }

    fn refresh_play_offset(&mut self) {
        self.play_offset = self.cfg.offset_ms + self.map_offset(self.offset_entry());
        self.preview.map_offset = self.map_offset(self.offset_entry());
    }

    /// Plays the current map again from where it was started (in game, paused or on results).
    fn restart_map(&mut self) {
        if let Scene::Play { entry, from, .. } | Scene::Results { entry, from, .. } = self.scene {
            self.start_loading_from(entry, from);
        }
    }

    fn pause_key(&mut self, code: KeyCode) {
        if code == KeyCode::KeyO {
            self.apply_live_offset();
            return;
        }
        if code == KeyCode::KeyR && self.modifiers_ctrl {
            self.restart_map();
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
            // Wrapping: Up from Continue is Quit, so pause, Up, Enter leaves at once.
            KeyCode::ArrowUp => game.paused = Some((cursor + PAUSE_ITEMS - 1) % PAUSE_ITEMS),
            KeyCode::ArrowDown => game.paused = Some((cursor + 1) % PAUSE_ITEMS),
            KeyCode::Escape => game.resume_after(delay, self.audio.as_ref()),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => match cursor {
                0 => game.resume_after(delay, self.audio.as_ref()),
                1 => {
                    let (e, f) = (*entry, *from);
                    self.start_loading_from(e, f);
                }
                2 => self.open_dropdown(),
                3 => self.open_settings(),
                _ => self.show_select(),
            },
            _ => {}
        }
    }

    fn handle_global_key(&mut self, key: KeyCode, window: Option<&Window>) -> bool {
        let step = if self.modifiers_shift { 5.0 } else { 1.0 };
        match key {
            // Song select's search box has the keys: they're letters of the search (there's an
            // artist called -45).
            KeyCode::Minus | KeyCode::Equal | KeyCode::NumpadSubtract | KeyCode::NumpadAdd
                if self.player.as_ref().is_some_and(|p| p.searching) || (matches!(self.scene, Scene::Select) && self.select.searching && !self.modifiers_ctrl) =>
            {
                return false
            }
            KeyCode::Minus | KeyCode::NumpadSubtract | KeyCode::Equal | KeyCode::NumpadAdd if self.modifiers_ctrl => {
                let d = if matches!(key, KeyCode::Minus | KeyCode::NumpadSubtract) { -step } else { step };
                self.cfg.visual_offset_ms += d;
                self.cfg.save();
                self.toast(tr!("visual offset {:+.0} ms (notes drawn earlier)", "表示オフセット {:+.0} ms (ノーツを早めに描画)", "смещение картинки {:+.0} мс (ноты раньше)"; self.cfg.visual_offset_ms));
                return true;
            }
            KeyCode::Minus | KeyCode::NumpadSubtract | KeyCode::Equal | KeyCode::NumpadAdd => {
                let d = if matches!(key, KeyCode::Minus | KeyCode::NumpadSubtract) { -step } else { step };
                match self.song_entry() {
                    Some(entry) => {
                        let ms = self.map_offset(Some(entry)) + d;
                        self.set_map_offset(entry, ms);
                        let text = tr!("this map: offset {ms:+.0} ms (total {:+.0})", "この譜面: オフセット {ms:+.0} ms (合計 {:+.0})", "эта карта: смещение {ms:+.0} мс (всего {:+.0})"; self.play_offset);
                        self.toast(text);
                    }
                    None => {
                        self.cfg.offset_ms += d;
                        self.cfg.save();
                        self.refresh_play_offset();
                        self.toast(tr!("offset {:+.0} ms (all maps)", "オフセット {:+.0} ms (全譜面)", "смещение {:+.0} мс (все карты)"; self.cfg.offset_ms));
                    }
                }
                return true;
            }
            // Like the speed, only between songs: it counts for pp.
            KeyCode::F3 | KeyCode::F4 if self.mid_song() => {
                self.toast(scroll_in_select().into());
                return true;
            }
            KeyCode::F3 => self.cfg.scroll_speed = config::step_scroll_speed(self.cfg.scroll_speed, -1.0),
            KeyCode::F4 => self.cfg.scroll_speed = config::step_scroll_speed(self.cfg.scroll_speed, 1.0),
            KeyCode::F7 => {
                self.cfg.present = self.cfg.present.next();
                self.cfg.save();
                let note = if self.cfg.menu_vsync && !self.player_plays() { tr!(" (menus stay on vsync)", " (メニューは垂直同期のまま)", " (меню остаются с vsync)") } else { "" };
                let text = tr!("gameplay present mode: {}{note}", "プレイ中の表示モード: {}{note}", "режим вывода в игре: {}{note}"; self.cfg.present.name());
                self.toast(text);
                return true;
            }
            KeyCode::F8 => {
                let i = FPS_CAPS.iter().position(|&c| c == self.cfg.fps_cap).unwrap_or(0);
                self.cfg.fps_cap = FPS_CAPS[(i + 1) % FPS_CAPS.len()];
                let mut text = if self.cfg.fps_cap == 0 {
    tr!("fps cap: unlimited", "FPSの上限: 無制限", "ограничение FPS: нет").to_string()
} else {
    tr!("fps cap: {}", "FPSの上限: {}", "ограничение FPS: {}"; self.cfg.fps_cap)
};
                if self.present == PresentChoice::Fifo {
                    text.push_str(tr!(" (mailbox/immediate only)", " (mailbox/immediateのみ)", " (только mailbox/immediate)"));
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
                self.screenshot_due = true;
                return true;
            }
            KeyCode::KeyQ if self.modifiers_ctrl => {
                self.ask_quit();
                return true;
            }
            KeyCode::KeyU if self.modifiers_ctrl && !self.modifiers_shift && !self.in_song() => {
                self.update_now();
                return true;
            }
            _ => return false,
        }
        self.toast(tr!("scroll speed {:.2}x", "スクロール速度 {:.2}x", "скорость прокрутки {:.2}x"; self.cfg.scroll_speed));
        self.cfg.save();
        true
    }

    /// Replays: seeks to the timeline position under the pointer, if it's on the timeline.
    /// `grabbed` (dragging along it) takes any height, so the pointer can wander off the bar.
    fn replay_seek_to_pointer(&mut self, grabbed: bool) -> bool {
        let Some([x, y, w, h]) = self.replay_bar else { return false };
        let (vx, vy) = self.mouse_virtual();
        if !grabbed && !(vx >= x - 6.0 && vx <= x + w + 6.0 && vy >= y && vy <= y + h) {
            return false;
        }
        let Scene::Play { game, .. } = &mut self.scene else { return false };
        let (start, end) = game.span();
        let t = start + ((vx - x) / w.max(1.0)).clamp(0.0, 1.0) as f64 * (end - start);
        game.seek(t, self.audio.as_ref(), self.play_offset);
        true
    }

    /// Gameplay hides the pointer once it has been still for a second; menus (and the pause
    /// menu) always show it.
    fn cursor_hidden(&self, now: Instant) -> bool {
        let playing = match &self.scene {
            Scene::Play { game, .. } | Scene::MpPlay { game, .. } | Scene::Calibrate { game, .. } => game.paused.is_none(),
            // Watching: hidden unless the friends overlay is open (it needs the pointer).
            Scene::Spectate(_) | Scene::Watch(_) => !self.social.open,
            _ => false,
        };
        playing
            && self.grip.is_none()
            && self.replay_scrub.is_none()
            && self.settings_ui.dropdown.is_none()
            && now.saturating_duration_since(self.mouse_moved_at) >= Duration::from_secs(1)
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
        self.preset_list().into_iter().find(|&(w, h)| {
            let p = layout::preset_area(w, h, self.window_w, self.window_h);
            (p.w - a.w).abs() * self.window_w < 1.5 && (p.h - a.h).abs() * self.window_h < 1.5
        })
    }

    /// Window sizes offered: the usual ones and the screen's own modes that fit on it with room
    /// for a title bar, largest first.
    fn window_sizes(&self) -> Vec<(u32, u32)> {
        let (mw, mh) = self.monitor_size;
        let sizes = layout::presets(&self.monitor_modes, mw, mh.saturating_sub(60));
        // Grouped like the playfield presets: the screen's aspect ratio first, largest first.
        layout::group_presets(&sizes, self.monitor_size).into_iter().flat_map(|(_, l)| l).collect()
    }

    fn screen_size(&self) -> (u32, u32) {
        (self.window_w as u32, self.window_h as u32)
    }

    /// The saved sizes first, then the built-in ones grouped by aspect ratio.
    fn preset_groups(&self) -> Vec<(&'static str, Vec<(u32, u32)>)> {
        let saved = &self.cfg.saved_presets;
        let others: Vec<(u32, u32)> = self.resolutions.iter().copied().filter(|r| !saved.contains(r)).collect();
        let mut groups = layout::group_presets(&others, self.screen_size());
        if !saved.is_empty() {
            groups.insert(0, (settings::SAVED, saved.clone()));
        }
        groups
    }

    fn fits_window(&self, (w, h): (u32, u32)) -> bool {
        w <= self.window_w as u32 && h <= self.window_h as u32
    }

    /// Presets in dropdown order (grouped by aspect ratio, largest first).
    fn preset_list(&self) -> Vec<(u32, u32)> {
        self.preset_groups().into_iter().flat_map(|(_, l)| l).collect()
    }

    fn apply_preset(&mut self, (w, h): (u32, u32)) {
        if !self.fits_window((w, h)) {
            self.toast(tr!("{w}x{h} doesn't fit this window", "{w}x{h}はこのウィンドウに収まりません", "{w}x{h} не помещается в окно";));
            return;
        }
        // Where the size being left was, and where this one was last put (else the middle).
        if let Some(old) = self.current_preset() {
            let a = self.cfg.playfield_area;
            self.cfg.preset_spots.retain(|(s, _)| *s != old);
            self.cfg.preset_spots.push((old, (a.x, a.y)));
        }
        let mut area = layout::preset_area(w, h, self.window_w, self.window_h);
        if let Some(&(_, (x, y))) = self.cfg.preset_spots.iter().find(|(s, _)| *s == (w, h)) {
            area = Area { x, y, ..area }.clamped();
        }
        self.cfg.playfield_area = area;
        self.cfg.save();
        self.toast(tr!("playfield {w}x{h}", "プレイフィールド {w}x{h}", "поле {w}x{h}";));
    }

    /// Steps through resolution presets in dropdown order, applying each as a centred area.
    fn cycle_preset(&mut self, dir: i32) {
        let list: Vec<(u32, u32)> = self.preset_list().into_iter().filter(|&r| self.fits_window(r)).collect();
        if list.is_empty() {
            self.toast(tr!("no smaller resolutions fit this window", "このウィンドウに収まる、より小さい解像度はありません", "меньших разрешений для этого окна нет").into());
            return;
        }
        let n = list.len() as i32;
        let i = match self.current_preset().and_then(|p| list.iter().position(|&r| r == p)) {
            Some(i) => (i as i32 + dir).rem_euclid(n),
            None => if dir > 0 { 0 } else { n - 1 },
        } as usize;
        self.apply_preset(list[i]);
    }

    /// Opens the preset list, even when it's empty: saved sizes can be added there.
    fn open_dropdown(&mut self) {
        let list = self.preset_list();
        let cursor = self.current_preset().and_then(|p| list.iter().position(|&r| r == p)).unwrap_or(0);
        self.settings_ui.dropdown = Some(Dropdown::new(cursor));
    }

    fn dropdown_key(&mut self, code: KeyCode) {
        let list = self.preset_list();
        let n = list.len();
        let saved = self.cfg.saved_presets.len();
        let shift = self.modifiers_shift;
        let Some(dd) = &mut self.settings_ui.dropdown else { return };
        if let Some((_, text)) = &mut dd.editing {
            match code {
                KeyCode::Escape => dd.editing = None,
                KeyCode::Backspace => settings::caret_backspace(text, &mut dd.caret),
                KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                    settings::caret_nav(text, &mut dd.caret, code);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    if let Some((index, text)) = dd.editing.take() {
                        self.finish_preset_edit(index, text);
                    }
                }
                _ => {
                    if let Some(ch) = size_char(code) {
                        settings::caret_insert(text, &mut dd.caret, ch.encode_utf8(&mut [0; 4]), 11);
                    }
                }
            }
            return;
        }
        let on_saved = dd.cursor < saved;
        match code {
            KeyCode::ArrowUp if shift && on_saved => self.preset_action(PresetAction::Up),
            KeyCode::ArrowDown if shift && on_saved => self.preset_action(PresetAction::Down),
            KeyCode::KeyS => self.preset_action(PresetAction::SaveCurrent),
            KeyCode::KeyA | KeyCode::Insert => self.preset_action(PresetAction::Add),
            KeyCode::KeyE | KeyCode::F2 => self.preset_action(PresetAction::Edit),
            KeyCode::Delete => self.preset_action(PresetAction::Delete),
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

    /// Changes the saved playfield sizes from the preset dropdown.
    fn preset_action(&mut self, action: PresetAction) {
        let saved = self.cfg.saved_presets.len();
        let size = self.playfield_size();
        let Some(dd) = &mut self.settings_ui.dropdown else { return };
        let i = dd.cursor;
        match action {
            PresetAction::SaveCurrent => {
                if let Some(at) = self.cfg.saved_presets.iter().position(|&p| p == size) {
                    dd.cursor = at;
                    self.toast(tr!("{}x{} is already saved", "{}x{}はもう保存されています", "{}x{} уже сохранён"; size.0, size.1));
                    return;
                }
                self.cfg.saved_presets.push(size);
                dd.cursor = saved;
                self.toast(tr!("saved {}x{}", "{}x{}を保存しました", "сохранён {}x{}"; size.0, size.1));
            }
            PresetAction::Add => {
                dd.editing = Some((None, String::new()));
                dd.caret = 0;
                return;
            }
            PresetAction::Edit if i < saved => {
                let (w, h) = self.cfg.saved_presets[i];
                let text = format!("{w}x{h}");
                dd.caret = text.len();
                dd.editing = Some((Some(i), text));
                return;
            }
            PresetAction::Delete if i < saved => {
                // The cursor stays on the same row, now the next size.
                let (w, h) = self.cfg.saved_presets.remove(i);
                self.toast(tr!("removed {w}x{h}", "{w}x{h}を削除しました", "удалён {w}x{h}";));
            }
            PresetAction::Up if i > 0 && i < saved => {
                self.cfg.saved_presets.swap(i, i - 1);
                dd.cursor = i - 1;
            }
            PresetAction::Down if i + 1 < saved => {
                self.cfg.saved_presets.swap(i, i + 1);
                dd.cursor = i + 1;
            }
            _ => return,
        }
        let n = self.preset_list().len();
        if let Some(dd) = &mut self.settings_ui.dropdown {
            dd.cursor = dd.cursor.min(n.saturating_sub(1));
        }
        self.cfg.save();
    }

    /// Saves a typed size: a new saved size (`index` None) or a change to saved size `index`.
    fn finish_preset_edit(&mut self, index: Option<usize>, text: String) {
        let Some(size) = layout::parse_size(&text) else {
            self.toast(tr!("type a size like 1280x720", "1280x720のようにサイズを入力", "введите размер вроде 1280x720").into());
            if let Some(dd) = &mut self.settings_ui.dropdown {
                dd.caret = text.len();
                dd.editing = Some((index, text));
            }
            return;
        };
        let saved = &mut self.cfg.saved_presets;
        let cursor = match saved.iter().position(|&p| p == size) {
            Some(at) if Some(at) != index => {
                self.toast(tr!("{}x{} is already saved", "{}x{}はもう保存されています", "{}x{} уже сохранён"; size.0, size.1));
                at
            }
            _ => match index {
                Some(i) => {
                    saved[i] = size;
                    i
                }
                None => {
                    saved.push(size);
                    saved.len() - 1
                }
            },
        };
        self.cfg.save();
        if let Some(dd) = &mut self.settings_ui.dropdown {
            dd.cursor = cursor;
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
        // Alt anywhere; with the pointer moved onto one of the volume's notes, also without Alt.
        let now = Instant::now();
        if self.volume.hovering(now) || self.modifiers_alt {
            let changed = self.volume.turn(&mut self.cfg, notches, now);
            self.apply_volume();
            self.cfg.save();
            // Hitsounds have nothing playing to judge them by: one rings at the new volume.
            if changed && self.volume.selected == volume::HITSOUNDS && !self.in_song() {
                self.sfx(audio::Sfx::Kat);
            }
            return;
        }
        if let Some(view) = &mut self.changelog {
            view.scroll_by(-notches * 3);
            return;
        }
        if self.social.open && !self.modifiers_alt {
            let (vx, vy) = self.mouse_virtual();
            let width = render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0);
            let [px, py, pw, ph] = social::panel_rect(width);
            if vx >= px && vx <= px + pw && vy >= py && vy <= py + ph {
                self.social.wheel(notches, vy, self.mp.is_some());
                return;
            }
        }
        if let Some(cur) = self.mod_select.filter(|_| !self.modifiers_alt) {
            self.mod_select = Some(mods::step(cur, 0, -notches));
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
        if self.area_editable() && (self.cfg.wheel_resizes_playfield || matches!(self.scene, Scene::LayoutEdit { .. })) {
            self.cfg.playfield_area = self.cfg.playfield_area.scaled(1.05f32.powi(notches));
            self.cfg.save();
            let text = tr!("playfield: {}", "プレイフィールド: {}", "поле: {}"; self.cfg.playfield_area.describe());
            self.toast(text);
            return;
        }
        let n = self.preset_list().len();
        if let Some(dd) = &mut self.settings_ui.dropdown {
            dd.cursor = (dd.cursor as i32 - notches).clamp(0, n as i32 - 1) as usize;
            return;
        }
        let (vx, vy) = self.mouse_virtual();
        if let Scene::Results { results, .. } = &mut self.scene {
            results.scroll(notches < 0);
            return;
        }
        if self.on_menu() {
            if let Some(p) = self.player.as_mut().filter(|p| p.over_list(vx)) {
                p.wheel(notches);
            } else if self.player.is_some() {
                self.music_seek_by(notches as f64 * MUSIC_SEEK_MS);
            } else if !self.main_menu.as_ref().is_some_and(menu::Menu::is_leaving) {
                self.menu_step(-notches.signum());
            }
            return;
        }
        let on_board = self.board_rect.is_some_and(|[bx, by, bw, bh]| vx >= bx && vx <= bx + bw && vy >= by && vy <= by + bh);
        if matches!(self.scene, Scene::Select) && on_board && self.select.picker.is_none() {
            self.board_scroll = (self.board_scroll as i64 - notches as i64).clamp(0, self.board_max as i64) as usize;
            return;
        }
        match &self.scene {
            Scene::Select if self.preview.time_at(vx, vy).is_some() => self.preview.seek_by(notches as f64 * 5000.0, self.audio.as_ref()),
            Scene::Select if self.select.picker.is_some() => {
                let n = self.select.picker_len() as i32;
                self.select.picker = self.select.picker.map(|c| (c as i32 - notches).clamp(0, n - 1) as usize);
            }
            Scene::Select => self.select.scroll_by(-notches, &self.library),
            Scene::Download if self.preview.time_at(vx, vy).is_some() => self.preview.seek_by(notches as f64 * 5000.0, self.audio.as_ref()),
            Scene::Download => self.browse.move_by(-notches),
            Scene::Settings { .. } if self.settings_ui.dropdown.is_some() => {
                if let Some(dd) = &mut self.settings_ui.dropdown {
                    dd.cursor = (dd.cursor as i32 - notches).clamp(0, n as i32 - 1) as usize;
                }
            }
            Scene::Settings { .. } if self.settings_ui.choices.is_some() => {
                if let Some(p) = &mut self.settings_ui.choices {
                    p.move_cursor(-notches);
                }
            }
            Scene::Settings { .. } => self.settings_ui.scroll_by(-notches),
            _ => {}
        }
    }

    /// Whether touches play the drum now (a song or calibration running, not paused); otherwise
    /// the first finger works as the mouse.
    fn touch_drums(&self) -> bool {
        match &self.scene {
            Scene::Play { game, .. } => game.paused.is_none() && game.replay.is_none(),
            Scene::MpPlay { game, .. } | Scene::Calibrate { game, .. } => game.paused.is_none(),
            _ => false,
        }
    }

    /// A finger on a touch screen (`x`, `y` in pixels). During play each new finger hits the drum
    /// in its column (`cfg.touch_layout`), timed like a key press; elsewhere the first finger
    /// is the mouse (tap = click, drag = drag).
    fn touch(&mut self, id: u64, phase: TouchPhase, x: f64, y: f64, at: Instant, window: Option<&Window>) {
        self.touch_used = true;
        if phase == TouchPhase::Started && self.touch_drums() {
            let col = (x / self.window_w.max(1.0) as f64 * 4.0).floor().clamp(0.0, 3.0) as usize;
            self.touch_flash[col] = Some(at);
            let code = self.cfg.keys[self.cfg.touch_layout.keys()[col]];
            self.key(code, false, None, at, window);
            return;
        }
        match phase {
            TouchPhase::Started if self.touch_mouse.is_none() => {
                self.touch_mouse = Some(id);
                self.mouse_moved(x, y);
                #[cfg(target_os = "android")]
                self.keyboard_tap();
                self.mouse_button(true, window);
            }
            TouchPhase::Moved if self.touch_mouse == Some(id) => {
                self.mouse_moved(x, y);
            }
            TouchPhase::Ended | TouchPhase::Cancelled if self.touch_mouse == Some(id) => {
                self.mouse_moved(x, y);
                self.mouse_button(false, window);
                self.touch_mouse = None;
            }
            _ => {}
        }
    }

    /// The touch columns along the bottom of the screen during play, each flashing when hit.
    fn draw_touch_zones(&self, c: &mut Canvas, now: Instant) {
        let col_w = c.width / 4.0;
        let h = render::VIRTUAL_HEIGHT;
        for (col, key) in self.cfg.touch_layout.keys().into_iter().enumerate() {
            let colours = multiverse::note_colours();
            let color = if key == 0 || key == 3 { colours.kat() } else { colours.don() };
            let x = col as f32 * col_w;
            let flash = self.touch_flash[col].map(|t| 1.0 - (now.saturating_duration_since(t).as_secs_f32() / 0.15).min(1.0)).unwrap_or(0.0);
            if flash > 0.0 {
                c.rect(x, 0.0, col_w, h, rgba(color, 0.10 * flash));
            }
            c.rect(x + 2.0, h - 6.0, col_w - 4.0, 6.0, rgba(color, 0.35 + 0.5 * flash));
        }
    }

    /// Pointer moved: drags the area if a drag is active, otherwise updates the hover state.
    /// Returns the cursor to show.
    fn mouse_moved(&mut self, x: f64, y: f64) -> CursorIcon {
        if (x, y) != self.mouse {
            self.mouse_moved_at = Instant::now();
        }
        self.mouse = (x, y);
        let (vx, vy) = self.mouse_virtual();
        let width = render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0);
        self.volume.point(vx, vy, width, Instant::now());
        if let Some(last) = self.replay_scrub {
            // Each seek re-judges the replay up to there; ~30 a second is plenty.
            if last.elapsed() >= Duration::from_millis(33) && self.replay_seek_to_pointer(true) {
                self.replay_scrub = Some(Instant::now());
            }
            return CursorIcon::Pointer;
        }
        if let Some([bx, by, bw, bh]) = self.replay_bar {
            let (vx, vy) = self.mouse_virtual();
            if vx >= bx - 6.0 && vx <= bx + bw + 6.0 && vy >= by && vy <= by + bh {
                self.hover = None;
                return CursorIcon::Pointer;
            }
        }
        if let Some((px, py, _)) = self.right_press {
            let (vx, vy) = self.mouse_virtual();
            if !self.right_scroll && ((vx - px).abs() > DRAG_START || (vy - py).abs() > DRAG_START) {
                self.right_scroll = true;
            }
        }
        let (_, drag_y) = self.mouse_virtual();
        if let Some(d) = &mut self.list_drag {
            let dy = drag_y - d.from_y;
            d.moved |= dy.abs() > DRAG_START;
            if d.moved && matches!(self.scene, Scene::Select) {
                self.select.drag_to(dy);
                return CursorIcon::NsResize;
            }
        }
        if let Some(i) = self.settings_level {
            self.set_level(i);
            return CursorIcon::EwResize;
        }
        if self.scrollbar_drag {
            if matches!(self.scene, Scene::Select) {
                let (_, vy) = self.mouse_virtual();
                self.select.scroll_to(vy, &self.library);
                return CursorIcon::NsResize;
            }
            self.scrollbar_drag = false;
        }
        if self.right_scroll {
            if matches!(self.scene, Scene::Select) {
                let (_, vy) = self.mouse_virtual();
                self.select.scroll_to(vy, &self.library);
                return CursorIcon::NsResize;
            }
            self.right_scroll = false;
        }
        if let Some((h, from, start)) = self.split_drag {
            if !matches!(self.scene, Scene::Select) {
                self.split_drag = None;
            } else {
                let (vx, vy) = self.mouse_virtual();
                let width = render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0);
                // A drag, not the first click of a double click.
                if (if h == render::Hit::SplitList { vx } else { vy } - from).abs() > 3.0 {
                    self.split_click = None;
                }
                if h == render::Hit::SplitList {
                    // Kept where the list's width still follows it.
                    if let Some((min, max)) = Select::share_range(width) {
                        self.cfg.song_list_share = (start + (vx - from) / width).clamp(min, max);
                    }
                    return CursorIcon::EwResize;
                }
                let (min, max) = preview::PREVIEW_SIZE;
                let max = self.preview.size_cap.clamp(min, max);
                self.cfg.song_preview_size = (start + (vy - from) / self.preview.size_unit.max(1.0)).clamp(min, max);
                return CursorIcon::NsResize;
            }
        }
        let (sw, sh) = (self.window_w.max(1.0) as f64, self.window_h.max(1.0) as f64);
        let (vx, vy) = self.mouse_virtual();
        let k = (self.window_h / render::VIRTUAL_HEIGHT).max(0.01) as f64;
        if let (Scene::VideoEdit(ed), None) = (&mut self.scene, &self.export) {
            if ed.scrubbing {
                let t = ed.time_at(vx);
                ed.seek(t);
                return CursorIcon::Pointer;
            }
            if let Some((grip, (px, py), from)) = ed.grip {
                let [_, _, fw, fh] = ed.frame_rect;
                ed.crop = layout::drag(from, grip, ((x - px) / (fw as f64 * k)) as f32, ((y - py) / (fh as f64 * k)) as f32).clamped();
                ed.crop_name = "custom";
                return grip.cursor();
            }
            let [fx, fy, fw, fh] = ed.frame_rect;
            if let Some(g) = layout::hit(ed.crop, x - fx as f64 * k, y - fy as f64 * k, fw as f64 * k, fh as f64 * k) {
                return g.cursor();
            }
        }
        if let Some(dd) = &mut self.settings_ui.dropdown {
            if dd.button_at(vx, vy).is_some() {
                return CursorIcon::Pointer;
            }
            if let Some(i) = dd.row_at(vx, vy).filter(|_| dd.editing.is_none()) {
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
        if self.scrubbing.is_none()
            && let Some(hit) = self.hit_at(vx, vy) {
            if let (render::Hit::PauseItem(i), Scene::Play { game, .. }) = (hit, &mut self.scene) {
                game.paused = Some(i);
            }
            self.hover = None;
            return match hit {
                render::Hit::SplitList => CursorIcon::EwResize,
                render::Hit::SplitPreview => CursorIcon::NsResize,
                _ => CursorIcon::Pointer,
            };
        }
        if let Some(drag) = self.player.as_ref().and_then(|p| p.drag) {
            match drag {
                player::Drag::Volume => self.music_volume_to_pointer(),
                player::Drag::Timeline if self.music_seeked.is_none_or(|t| t.elapsed() >= Duration::from_millis(33)) => {
                    self.music_seeked = Some(Instant::now());
                    self.music_seek_to_pointer();
                }
                player::Drag::Timeline => {}
            }
            return CursorIcon::Pointer;
        }
        if !self.on_menu() && matches!(self.scene, Scene::Select | Scene::Preview { .. } | Scene::Download) {
            if let Some((last, at)) = self.scrubbing {
                // Seeking re-decodes nothing, but limit it to ~30 per second while dragging.
                if last.elapsed() >= Duration::from_millis(33) {
                    let t = self.preview.time_at(vx, vy).filter(|&t| t != at);
                    if let Some(t) = t {
                        self.preview.seek(t, self.audio.as_ref());
                    }
                    self.scrubbing = Some((Instant::now(), t.unwrap_or(at)));
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
            if let Some(dd) = &mut self.settings_ui.dropdown
                && let Some(i) = dd.row_at(vx, vy) {
                dd.cursor = i;
                return CursorIcon::Pointer;
            }
            return CursorIcon::Default;
        }
        self.hover = layout::hit(self.cfg.playfield_area, x, y, sw, sh);
        self.hover.map(|g| g.cursor()).unwrap_or(CursorIcon::Default)
    }

    /// Right button on song select: hold it and move up and down to fly through the list, like
    /// osu!'s right-click scrolling. The selection stays.
    /// Released without moving, it opens the row's menu; on the scrollbar, it goes there.
    fn right_button(&mut self, pressed: bool) {
        if self.mod_select.is_some() {
            // Right-clicking a mod makes it a favourite (or not any more).
            if pressed && self.customization_tab().is_none() {
                let (vx, vy) = self.mouse_virtual();
                if let Some(render::Hit::ModToggle(i)) = self.hit_at(vx, vy) {
                    self.mod_select = Some(i);
                    self.toggle_favourite(mods::ALL[i]);
                }
            }
            return;
        }
        if self.on_menu() {
            return;
        }
        let free = self.splash.is_none() && self.select.picker.is_none() && self.settings_ui.dropdown.is_none();
        let (vx, vy) = self.mouse_virtual();
        if pressed {
            self.select.menu = None;
            self.right_scroll = false;
            self.right_press = (free && matches!(self.scene, Scene::Select)).then(|| (vx, vy, self.hit_at(vx, vy)));
            return;
        }
        let press = self.right_press.take();
        if std::mem::take(&mut self.right_scroll) || !matches!(self.scene, Scene::Select) {
            return;
        }
        match press {
            Some((x, y, Some(render::Hit::SongRow(row)))) => self.select.menu = Some(ui::SongMenu { row, header: false, x, y }),
            Some((x, y, Some(render::Hit::SongHeader(row)))) => self.select.menu = Some(ui::SongMenu { row, header: true, x, y }),
            Some((_, y, Some(render::Hit::SongScrollbar))) => self.select.scroll_to(y, &self.library),
            _ => {}
        }
    }

    fn mouse_button(&mut self, pressed: bool, window: Option<&Window>) {
        if pressed && self.splash.is_none() {
            let (vx, vy) = self.mouse_virtual();
            let width = render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0);
            let [px, py, pw, ph] = social::panel_rect(width);
            let on_panel = self.social.open && vx >= px && vx <= px + pw && vy >= py && vy <= py + ph;
            let on_social = matches!(self.hit_at(vx, vy), Some(render::Hit::SocialToggle | render::Hit::SocialNote(_)));
            if on_panel || on_social {
                if let Some(hit) = self.hit_at(vx, vy) {
                    self.click(hit, window);
                }
                return;
            }
            if self.social.open {
                self.social.close();
                return;
            }
        }
        if let Some(s) = &mut self.splash {
            if pressed {
                s.skip(Instant::now(), self.audio.as_ref(), "a left click");
            }
            return;
        }
        if self.profile.is_some() {
            // Only its Back button.
            if pressed {
                let (vx, vy) = self.mouse_virtual();
                if let Some(hit @ render::Hit::Key(_)) = self.hit_at(vx, vy) {
                    self.click(hit, window);
                }
            }
            return;
        }
        if self.mod_select.is_some() {
            // Only the mod select's own chips and buttons; nothing under it.
            if pressed {
                let (vx, vy) = self.mouse_virtual();
                let hit = self.hit_at(vx, vy);
                let ours = if self.customization_tab().is_some() {
                    use render::Hit as H;
                    matches!(
                        hit,
                        Some(H::UniverseCell(..) | H::UniversePreset(_) | H::UniversePresetDelete(_) | H::ShapeshiftCell(..) | H::ShapeshiftMove(_) | H::CustomTab(_) | H::Key(_))
                    )
                } else {
                    matches!(hit, Some(render::Hit::ModToggle(_) | render::Hit::Key(_)))
                };
                if let Some(hit) = hit.filter(|_| ours) {
                    self.click(hit, window);
                }
            }
            return;
        }
        if pressed && self.settings_ui.dropdown.is_none() && self.replay_seek_to_pointer(false) {
            self.replay_scrub = Some(Instant::now());
            return;
        }
        if !pressed && self.replay_scrub.take().is_some() {
            // Land exactly where the pointer was released.
            self.replay_seek_to_pointer(true);
            return;
        }
        if let (Scene::VideoEdit(ed), None) = (&mut self.scene, &self.export) {
            let (vx, vy) = (self.mouse.0 as f32 * render::VIRTUAL_HEIGHT / self.window_h.max(1.0), self.mouse.1 as f32 * render::VIRTUAL_HEIGHT / self.window_h.max(1.0));
            let k = (self.window_h / render::VIRTUAL_HEIGHT).max(0.01) as f64;
            if !pressed {
                ed.scrubbing = false;
                ed.grip = None;
            } else if ed.in_timeline(vx, vy) {
                ed.scrubbing = true;
                ed.playing = None;
                if let Some(a) = &self.audio {
                    a.stop();
                }
                let t = ed.time_at(vx);
                ed.seek(t);
                return;
            } else {
                let [fx, fy, fw, fh] = ed.frame_rect;
                if let Some(g) = layout::hit(ed.crop, self.mouse.0 - fx as f64 * k, self.mouse.1 - fy as f64 * k, fw as f64 * k, fh as f64 * k) {
                    ed.grip = Some((g, self.mouse, ed.crop));
                    return;
                }
            }
        }
        if self.on_menu() {
            let (vx, vy) = self.mouse_virtual();
            if let Some(hit) = self.hit_at(vx, vy).filter(|_| pressed) {
                self.click(hit, window);
            }
            if let Some(p) = self.player.as_mut().filter(|_| !pressed)
                && p.drag.take() == Some(player::Drag::Volume) {
                self.cfg.save();
            }
            return;
        }
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
                    self.scrubbing = Some((Instant::now(), t));
                    return;
                }
            } else if let Some((_, at)) = self.scrubbing.take() {
                // Land exactly where the pointer was released. Already there (a plain click): the
                // music has moved on from it, and seeking again would throw it back.
                if let Some(t) = self.preview.time_at(vx, vy).filter(|&t| t != at) {
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
            if let Some(action) = dd.button_at(vx, vy) {
                self.preset_action(action);
                return;
            }
            if dd.editing.is_some() {
                // Clicking elsewhere while typing a size cancels it (outside also closes the list).
                let outside = !dd.contains(vx, vy);
                if let Some(dd) = &mut self.settings_ui.dropdown {
                    dd.editing = None;
                }
                if outside {
                    self.settings_ui.dropdown = None;
                }
                return;
            }
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
        if !pressed {
            self.scrollbar_drag = false;
            if self.settings_level.take().is_some() {
                self.cfg.save();
                return;
            }
            if self.split_drag.take().is_some() {
                self.cfg.save();
                return;
            }
            if let Some(d) = self.list_drag.take() {
                if d.moved {
                    self.select.drag_end(&self.library);
                } else {
                    self.click(d.hit, window);
                }
                return;
            }
        }
        if pressed {
            let (vx, vy) = self.mouse_virtual();
            let hit = self.hit_at(vx, vy);
            // The search box takes the keys when clicked, and gives them back on a click elsewhere.
            if matches!(self.scene, Scene::Select) {
                self.select.searching = hit == Some(render::Hit::TextField);
            }
            if self.select.picker.is_some() && matches!(self.scene, Scene::Select) && !matches!(hit, Some(render::Hit::PickRow(_))) {
                self.select.picker = None;
                return;
            }
            if self.select.menu.is_some() && matches!(self.scene, Scene::Select) && !matches!(hit, Some(render::Hit::SongMenu(_))) {
                self.select.menu = None;
                return;
            }
            if let (Scene::Select, Some(h @ (render::Hit::SplitList | render::Hit::SplitPreview))) = (&self.scene, hit) {
                // Twice in quick succession: back to the usual size.
                let now = Instant::now();
                if self.split_click.take().is_some_and(|(last, at)| last == h && now.duration_since(at) < Duration::from_millis(400)) {
                    self.reset_song_layout(Some(h));
                    return;
                }
                self.split_click = Some((h, now));
                let (from, start) = match h {
                    render::Hit::SplitList => (vx, self.cfg.song_list_share),
                    _ => (vy, self.preview.size_shown),
                };
                self.split_drag = Some((h, from, start));
                return;
            }
            // A row acts on release, unless the press turns into dragging the list.
            if let (Scene::Select, Some(render::Hit::SongScrollbar)) = (&self.scene, hit) {
                self.scrollbar_drag = true;
                self.select.scroll_to(vy, &self.library);
                return;
            }
            if let (Scene::Select, Some(h @ (render::Hit::SongRow(_) | render::Hit::SongHeader(_)))) = (&self.scene, hit) {
                self.list_drag = Some(ListDrag { from_y: vy, hit: h, moved: false });
                return;
            }
            if self.settings_ui.choices.is_some() && !matches!(hit, Some(render::Hit::ChoiceRow(_))) {
                self.settings_ui.choices = None;
                return;
            }
            // Clicking anywhere while typing a value leaves it as it was.
            if self.settings_ui.editing.take().is_some() {
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
        if let (Some(s), Some(g)) = (self.mp.as_ref().filter(|s| s.mapguessr), &mut self.guessr) {
            let e = &self.library[entry];
            if g.can_guess(s) {
                s.client.send(net::ClientMsg::Guess { hash: e.hash(), song: mapguessr::entry_song(e) });
                g.last_guess = Some(format!("{} - {}", e.meta.artist, e.meta.title));
            }
            self.scene = Scene::Lobby;
            return;
        }
        let mut info = map_info(&self.library[entry], self.play_speed().0, self.cfg.mods);
        info.seed = lobby_seed(None);
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
        if let Scene::MpPlay { game, .. } = &mut self.scene
            && let Some(k) = self.cfg.keys.iter().position(|&k| k == code) {
            if !repeat && game.replay.is_none() {
                game.press(k, at, self.audio.as_ref(), self.play_offset);
            }
            return;
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
                if s.mapguessr {
                    self.guessr_key(code);
                    return;
                }
                match code {
                    KeyCode::Space | KeyCode::KeyR if s.me().is_some_and(|p| p.spectator) => {
                        self.toast("you're spectating: S to play instead".into());
                    }
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
                        let (rate, mods) = (self.cfg.mods.speed(self.cfg.rate, true).0, self.cfg.mods);
                        if let Some(mut m) = s.map.clone() {
                            m.rate = rate;
                            m.mods = mods.list();
                            (m.multiverse, m.shapeshift) = mod_settings(mods);
                            m.seed = lobby_seed(m.seed);
                            s.client.send(net::ClientMsg::SelectMap(m));
                        }
                        let text = format!("match rate {rate:.2}x");
                        self.toast(text);
                    }
                    KeyCode::KeyP if s.map_entry.is_some() => self.open_preview(),
                    KeyCode::F1 => self.open_mod_select(),
                    KeyCode::KeyS if s.in_match && s.me().is_some_and(|p| p.status == net::Status::Lobby) => {
                        if s.have_map {
                            s.client.send(net::ClientMsg::Spectate(true));
                        } else {
                            self.toast("you need the map to spectate".into());
                        }
                    }
                    // Before a match: watch it (and the next ones) instead of playing.
                    KeyCode::KeyS if !s.in_match => {
                        let on = !s.me().is_some_and(|p| p.spectator);
                        s.ready = false;
                        s.client.send(net::ClientMsg::Spectator(on));
                        self.toast(if on { "you'll spectate the match" } else { "you'll play the match" }.into());
                    }
                    // Missing the map: from the host by default (fast, works for any map), the
                    // mirrors with M or Shift+D.
                    KeyCode::KeyD | KeyCode::KeyM if !s.have_map && s.map.is_some() && !s.is_host() && (code == KeyCode::KeyM || self.modifiers_shift) => {
                        self.download_lobby_map()
                    }
                    KeyCode::KeyD | KeyCode::KeyH if !s.have_map && s.map.is_some() && !s.is_host() => {
                        if self.host_transfer.is_none() {
                            s.client.send(net::ClientMsg::RequestMap);
                            self.host_map_asked = true;
                            self.toast("getting the map from the host...".into());
                        }
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
                        let why = if s.owns_room() { "room closed" } else { "left the room" };
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
                KeyCode::Tab => {
                    self.cfg.mp_scoreboard = !self.cfg.mp_scoreboard;
                    self.cfg.save();
                    self.toast(if self.cfg.mp_scoreboard { "scoreboard shown (Tab)" } else { "scoreboard hidden (Tab)" }.into());
                }
                KeyCode::Space => {
                    if game.skip(self.audio.as_ref(), self.play_offset)
                        && let Some(s) = &self.mp {
                        s.client.send(net::ClientMsg::Skipped);
                    }
                }
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

    /// A key in a mapguessr lobby.
    fn guessr_key(&mut self, code: KeyCode) {
        let (Some(s), Some(g)) = (&self.mp, &mut self.guessr) else { return };
        let in_game = s.in_match;
        match code {
            KeyCode::KeyG | KeyCode::Tab | KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter if in_game => {
                if g.can_guess(s) {
                    self.picking_lobby_map = true;
                    self.scene = Scene::Select;
                } else if s.me().is_some_and(|p| p.guessed) {
                    self.toast("you've got this one".into());
                }
            }
            KeyCode::BracketLeft | KeyCode::BracketRight if s.is_host() && !in_game => {
                let step = if self.modifiers_shift { 5 } else { 1 };
                // Past either end it comes round again (the button only goes up).
                g.rounds = match code {
                    KeyCode::BracketRight if g.rounds >= mapguessr::MAX_ROUNDS => 1,
                    KeyCode::BracketRight => (g.rounds + step).min(mapguessr::MAX_ROUNDS),
                    _ if g.rounds <= 1 => mapguessr::MAX_ROUNDS,
                    _ => g.rounds.saturating_sub(step).max(1),
                };
            }
            KeyCode::Enter | KeyCode::NumpadEnter if s.is_host() && !in_game => {
                s.client.send(net::ClientMsg::StartGuessr { rounds: g.rounds });
            }
            KeyCode::KeyS if !in_game => {
                let on = !s.me().is_some_and(|p| p.spectator);
                s.client.send(net::ClientMsg::Spectator(on));
                self.toast(if on { "you'll watch the next game" } else { "you'll play the next game" }.into());
            }
            KeyCode::Tab => self.scene = Scene::Select,
            KeyCode::Escape => {
                let why = if s.owns_room() { "room closed" } else { "left the room" };
                self.leave_room(why);
            }
            _ => {}
        }
    }

    fn mp_menu_key(&mut self, code: KeyCode, text: Option<&str>) {
        use multiplayer::Row;
        let row = self.mp_menu.row();
        let rows = self.mp_menu.rows().len();
        let server = self.cfg.online_server.clone();
        match code {
            KeyCode::ArrowUp => self.mp_menu.cursor = self.mp_menu.cursor.min(rows - 1).saturating_sub(1),
            KeyCode::ArrowDown => self.mp_menu.cursor = (self.mp_menu.cursor + 1).min(rows - 1),
            // Between the columns: online (left), direct play (right).
            KeyCode::ArrowLeft => self.mp_menu.cursor = 0,
            KeyCode::ArrowRight => self.mp_menu.select(Row::Name),
            KeyCode::F5 => self.mp_menu.refresh(&server),
            KeyCode::Escape | KeyCode::Tab => self.scene = Scene::Select,
            KeyCode::Backspace => {
                match row {
                    Row::User => {
                        self.mp_menu.user.pop();
                    }
                    Row::Password => {
                        self.mp_menu.password.pop();
                    }
                    Row::Email => {
                        self.mp_menu.email.pop();
                    }
                    Row::Code => {
                        self.mp_menu.code.pop();
                    }
                    Row::Name => {
                        self.cfg.mp_name.pop();
                    }
                    Row::Port => self.cfg.mp_port /= 10,
                    Row::Address => {
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
                let token = self.cfg.online_token.clone();
                let account = self.mp_menu.account.clone().unwrap_or_default();
                use multiplayer::CodeFor;
                let menu = &mut self.mp_menu;
                let code = (!menu.code.trim().is_empty()).then(|| menu.code.trim().to_string());
                let email = menu.email.trim().to_string();
                match row {
                    Row::User => self.mp_menu.select(Row::Password),
                    Row::Email | Row::Code => self.mp_menu.cursor = (self.mp_menu.cursor + 1).min(rows - 1),
                    Row::LogIn | Row::Register | Row::Password | Row::Forgot | Row::LinkEmail if menu.account_request.is_some() => {}
                    // A new password typed for a reset goes with the code.
                    Row::Password if menu.code_for == Some(CodeFor::Reset) => self.mp_menu.select(Row::Forgot),
                    Row::Forgot => {
                        let who = if menu.user.trim().is_empty() { email.clone() } else { menu.user.trim().to_string() };
                        if who.is_empty() {
                            self.mp_status = "type your username (or email) first".into();
                            self.mp_menu.select(Row::User);
                        } else if menu.code_for == Some(CodeFor::Reset) && code.is_some() {
                            if !online::valid_password(&menu.password) {
                                self.mp_status = "type a new password of at least 6 characters".into();
                                self.mp_menu.select(Row::Password);
                            } else {
                                let password = Some(menu.password.clone());
                                self.mp_status = "setting the new password...".into();
                                menu.account_call(&server, online::Request::ResetPassword { who, code, password });
                            }
                        } else {
                            menu.asking = Some(CodeFor::Reset);
                            self.mp_status = "sending a code to the account's email...".into();
                            menu.account_call(&server, online::Request::ResetPassword { who, code: None, password: None });
                        }
                    }
                    Row::LinkEmail if !menu.linking => {
                        menu.linking = true;
                        menu.code_for = None;
                        menu.cursor = 0;
                        self.mp_status = "type your email, then SEND CODE".into();
                    }
                    Row::LinkEmail => {
                        if !email.contains('@') {
                            self.mp_status = "type your email first".into();
                            self.mp_menu.select(Row::Email);
                        } else if menu.code_for == Some(CodeFor::Link) && code.is_some() {
                            self.mp_status = "linking the email...".into();
                            menu.account_call(&server, online::Request::LinkEmail { token, email, code });
                        } else {
                            menu.asking = Some(CodeFor::Link);
                            self.mp_status = format!("sending a code to {email}...");
                            menu.account_call(&server, online::Request::LinkEmail { token, email, code: None });
                        }
                    }
                    Row::LinkBack => {
                        menu.linking = false;
                        menu.code_for = None;
                        menu.cursor = 0;
                        self.mp_status.clear();
                    }
                    Row::LogIn | Row::Register | Row::Password => {
                        let user = menu.user.trim().to_string();
                        // Logging in forgets the typed password; a new account's waits for its code.
                        let password = if row == Row::Register { menu.password.clone() } else { std::mem::take(&mut menu.password) };
                        let valid = if row == Row::Register { online::valid_name(&user) } else { online::valid_login(&user) };
                        if !valid {
                            self.mp_status = format!("usernames are {}", online::NAME_RULES);
                            self.mp_menu.select(Row::User);
                        } else if row == Row::Register && !online::valid_password(&password) {
                            self.mp_status = "pick a password of at least 6 characters".into();
                            self.mp_menu.select(Row::Password);
                        } else if row == Row::Register && !email.contains('@') {
                            self.mp_status = "type your email: a code to create the account goes there".into();
                            self.mp_menu.select(Row::Email);
                        } else if row == Row::Register {
                            let code = code.filter(|_| menu.code_for == Some(CodeFor::Register));
                            self.mp_status = if code.is_some() { format!("creating {user}...") } else { format!("sending a code to {email}...") };
                            menu.asking = Some(CodeFor::Register);
                            menu.account_call(&server, online::Request::Register { name: user, password, email: Some(email), code });
                        } else {
                            self.mp_status = format!("logging in as {user}...");
                            menu.account_call(&server, online::Request::Login { name: user, password });
                        }
                    }
                    Row::LogOut => {
                        self.mp_menu.account_call(&server, online::Request::Logout { token });
                        self.sign_out("logged out");
                    }
                    Row::HostOnline | Row::HostGuessr => {
                        let mapguessr = row == Row::HostGuessr;
                        self.mp_status = if mapguessr { "opening a mapguessr lobby..." } else { "opening a lobby on the server..." }.into();
                        let request = online::Request::CreateRoom { token, title: String::new(), mapguessr };
                        self.mp_connecting = Some(multiplayer::start_online(server, request, account));
                    }
                    Row::Lobby(k) => {
                        let Some(room) = self.mp_menu.rooms.get(k).cloned() else { return };
                        self.mp_status = format!("joining {}...", room.title);
                        let request = online::Request::JoinRoom { token, room: room.id };
                        self.mp_connecting = Some(multiplayer::start_online(server, request, account));
                    }
                    Row::HostDirect => {
                        self.mp_status = format!("opening room on port {}...", self.cfg.mp_port);
                        self.mp_connecting = Some(multiplayer::start_host(self.cfg.mp_port, name, false));
                    }
                    Row::JoinDirect if self.cfg.mp_address.trim().is_empty() => self.mp_status = "type the host's address first".into(),
                    Row::JoinDirect => {
                        self.mp_status = format!("connecting to {}...", self.cfg.mp_address);
                        self.mp_connecting = Some(multiplayer::start_join(self.cfg.mp_address.clone(), name));
                    }
                    Row::Name | Row::Port | Row::Address => self.mp_menu.cursor = (self.mp_menu.cursor + 1).min(rows - 1),
                }
                self.cfg.save();
            }
            _ => {
                let Some(text) = text else { return };
                for ch in text.chars().filter(|c| net::drawable_char(*c)) {
                    match row {
                        Row::User if self.mp_menu.user.len() < 254 && ch.is_ascii_graphic() => self.mp_menu.user.push(ch),
                        Row::Password if self.mp_menu.password.chars().count() < 128 => self.mp_menu.password.push(ch),
                        Row::Email if self.mp_menu.email.len() < 254 && ch.is_ascii_graphic() => self.mp_menu.email.push(ch),
                        Row::Code if self.mp_menu.code.len() < 6 && ch.is_ascii_digit() => self.mp_menu.code.push(ch),
                        Row::Name if self.cfg.mp_name.chars().count() < 20 => self.cfg.mp_name.push(ch),
                        Row::Port => {
                            if let Some(d) = ch.to_digit(10) {
                                let p = self.cfg.mp_port as u32 * 10 + d;
                                if p <= u16::MAX as u32 {
                                    self.cfg.mp_port = p as u16;
                                }
                            }
                        }
                        Row::Address if self.cfg.mp_address.len() < 64 && ch != ' ' => self.cfg.mp_address.push(ch),
                        _ => {}
                    }
                }
                self.cfg.save();
            }
        }
    }

    /// Forgets the saved online login.
    fn sign_out(&mut self, why: &str) {
        self.social = social::Social::default();
        self.select.nominator = false;
        self.mp_menu.account = None;
        self.mp_menu.has_email = None;
        self.mp_menu.linking = false;
        self.mp_menu.code_for = None;
        self.mp_menu.rooms.clear();
        self.cfg.online_token.clear();
        self.cfg.save();
        self.mp_status = why.to_string();
        self.mp_menu.cursor = 0;
    }

    /// The time played so far of the live play going on: a song or a lobby match (also while
    /// its pause menu's settings are open), not autoplay or a replay (Relax is still playing).
    fn live_play_ms(&self) -> Option<f64> {
        fn game_of(scene: &Scene) -> Option<&Game> {
            match scene {
                Scene::Play { game, .. } | Scene::MpPlay { game, .. } => Some(game),
                Scene::Settings { prev } => game_of(prev),
                _ => None,
            }
        }
        let g = game_of(&self.scene)?;
        (g.replay.is_none() && !self.auto && !g.mods.has(mods::Mod::Autoplay)).then_some(g.played_ms)
    }

    /// Follows the live play; once it's gone without going to the leaderboards (failed, quit,
    /// restarted, from a start point, a lobby match), tells the server it was played, for the
    /// profile's play count and play time.
    fn track_play(&mut self) {
        if let Some(ms) = self.live_play_ms() {
            self.playing = Some(ms);
            return;
        }
        let Some(ms) = self.playing.take() else { return };
        // A start that's left at once isn't a play.
        if ms < 1000.0 || self.cfg.online_token.is_empty() {
            return;
        }
        let (server, token) = (self.cfg.online_server.clone(), self.cfg.online_token.clone());
        std::thread::spawn(move || {
            if let Err(e) = online::call(&server, online::Request::Played { token, ms: ms as u64 }) {
                eprintln!("counting the play: {e}");
            }
        });
    }

    /// Asks the online server which map sets are loved there, or (`change`) loves one or takes
    /// it back, in the background.
    fn call_loved(&mut self, change: Option<(u32, bool)>) {
        let request = match change {
            Some((set, loved)) => online::Request::SetLoved { token: self.cfg.online_token.clone(), set, loved },
            None => online::Request::LovedSets,
        };
        let (tx, rx) = mpsc::channel();
        let server = self.cfg.online_server.clone();
        std::thread::spawn(move || {
            let _ = tx.send(online::call(&server, request));
        });
        self.loved_call = Some((rx, change));
    }

    /// The loved sets as the server has them, once it answers.
    fn poll_loved(&mut self) {
        if !self.loved_asked && self.loved_call.is_none() {
            self.loved_asked = true;
            self.call_loved(None);
        }
        let Some((rx, change)) = &self.loved_call else { return };
        let Ok(result) = rx.try_recv() else { return };
        let change = *change;
        self.loved_call = None;
        match (result, change) {
            (Ok(online::Reply::LovedSets(sets)), _) => {
                self.select.server_loved = sets.into_iter().collect();
                if !self.select.filter.is_empty() {
                    self.select.refilter(&self.library);
                }
                if let Some((set, loved)) = change {
                    self.toast(if loved { format!("loved set {set} on taiko.rs: its plays give pp now") } else { format!("set {set} isn't loved on taiko.rs any more") });
                }
            }
            (Err(e), Some(_)) => self.toast(format!("couldn't: {e}")),
            _ => {}
        }
    }

    /// The online server's side of the multiplayer menu: the saved login, account replies and
    /// a lobby list kept fresh while the menu is open.
    fn poll_online(&mut self) {
        self.poll_loved();
        let menu = &mut self.mp_menu;
        if !menu.session_loaded {
            menu.session_loaded = true;
            if !self.cfg.online_token.is_empty() {
                menu.account = Some(self.cfg.online_user.clone());
                menu.user = self.cfg.online_user.clone();
                let token = self.cfg.online_token.clone();
                menu.account_call(&self.cfg.online_server, online::Request::WhoAmI { token });
            }
        }
        if let Some(rx) = &menu.account_request {
            match rx.try_recv() {
                Ok(result) => {
                    menu.account_request = None;
                    match result {
                        Ok(online::Reply::Account { name, token, nominator, email }) => {
                            self.select.nominator = nominator;
                            self.loved_asked = false;
                            let fresh = menu.account.as_deref() != Some(&name) || self.cfg.online_token != token;
                            menu.account = Some(name.clone());
                            menu.online = Some(Ok(()));
                            menu.has_email = email;
                            menu.asking = None;
                            if menu.code_for.take().is_some() {
                                menu.code.clear();
                                menu.password.clear();
                            }
                            if fresh {
                                self.mp_status = format!("logged in as {name}");
                                menu.cursor = 0;
                                // A new login: reconnect the social side with its token.
                                self.social = social::Social::default();
                                // Accounts from before emails: a word about linking one.
                                if email == Some(false) {
                                    self.toast = Some(Toast { text: "no email on your account: link one in the Online menu to reset a forgotten password".into(), until: Instant::now() + Duration::from_secs(8) });
                                }
                            }
                            self.cfg.online_user = name;
                            self.cfg.online_token = token;
                            self.cfg.save();
                            self.mp_menu.refresh(&self.cfg.online_server);
                        }
                        Ok(online::Reply::CodeSent(to)) => {
                            menu.code_for = menu.asking.take();
                            menu.code.clear();
                            // The server's mail is new to the big mail services: it may land in spam.
                            self.mp_status = match menu.code_for {
                                Some(multiplayer::CodeFor::Reset) => format!("code sent to {to} (check spam too): type it and a new password, then SET NEW PASSWORD"),
                                Some(multiplayer::CodeFor::Link) => format!("code sent to {to} (check spam too): type it, then LINK EMAIL"),
                                _ => format!("code sent to {to} (check spam too): type it, then CREATE ACCOUNT"),
                            };
                            menu.select(multiplayer::Row::Code);
                        }
                        Ok(online::Reply::EmailLinked(to)) => {
                            menu.has_email = Some(true);
                            menu.linking = false;
                            menu.code_for = None;
                            menu.code.clear();
                            menu.cursor = 0;
                            self.mp_status = format!("email linked: {to}");
                        }
                        Ok(_) => {}
                        Err(e) if e.contains("session") => self.sign_out(&e),
                        Err(e) => {
                            if e.contains("offline") {
                                menu.online = Some(Err(e.clone()));
                            }
                            self.mp_status = e;
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => menu.account_request = None,
            }
        }
        let menu = &mut self.mp_menu;
        if let Some(rx) = &menu.rooms_request {
            match rx.try_recv() {
                Ok(result) => {
                    menu.rooms_request = None;
                    match result {
                        Ok(rooms) => {
                            // Keep the cursor on the same lobby when the list changes.
                            let on = match menu.row() {
                                multiplayer::Row::Lobby(k) => menu.rooms.get(k).map(|r| r.id),
                                _ => None,
                            };
                            menu.rooms = rooms;
                            menu.online = Some(Ok(()));
                            match on.and_then(|id| menu.rooms.iter().position(|r| r.id == id)) {
                                Some(k) => menu.select(multiplayer::Row::Lobby(k)),
                                None => menu.cursor = menu.cursor.min(menu.rows().len() - 1),
                            }
                        }
                        Err(e) => {
                            menu.rooms.clear();
                            menu.online = Some(Err(e));
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => menu.rooms_request = None,
            }
        }
        if matches!(self.scene, Scene::MpMenu) && self.mp_menu.last_refresh.is_none_or(|t| t.elapsed() > Duration::from_secs(4)) {
            self.mp_menu.refresh(&self.cfg.online_server);
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
        let status = std::mem::take(&mut self.select.status);
        let server_loved = std::mem::take(&mut self.select.server_loved);
        let nominator = self.select.nominator;
        let sort = self.select.sort;
        self.select = Select::new(&self.library);
        self.select.keep_converts(&self.library, self.cfg.show_converts);
        self.select.filter = filter;
        self.select.played = played;
        self.select.status = status;
        self.select.server_loved = server_loved;
        self.select.nominator = nominator;
        self.select.sort = sort;
        self.select.mods_label = self.cfg.mods.label();
        let maps = self.library.iter().enumerate().map(|(i, e)| (i, e.path.clone(), e.modified, e.size)).collect();
        let info = mapinfo::MapInfo::spawn(maps, self.library.len(), self.in_song());
        self.select.info = Some(info.results.clone());
        self.mapinfo = Some(info);
        self.mapinfo_sorted = false;
        let dirs = self.song_dirs.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(dirs.iter().find_map(|d| osudb::load(d)));
        });
        self.osudb_rx = Some(rx);
        self.apply_cached_statuses();
        self.select.refilter(&self.library);
        // Start on a random song rather than the first one every time (not before a map given on
        // the command line, nor in scripted runs, whose screenshots expect the first song).
        if !self.play_when_scanned && self.input_script.is_none() && self.shots.is_none() {
            self.select.random(&self.library);
        }
        // Tests: TAIKO_OPEN_VIDEO=<replay file> opens the video editor for it.
        if let Some(file) = std::env::var_os("TAIKO_OPEN_VIDEO") {
            self.open_video_editor(std::path::Path::new(&file));
        }
        // Tests: TAIKO_WATCH=<replay file> plays it back.
        if let Some(file) = std::env::var_os("TAIKO_WATCH") {
            self.watch_replay(std::path::Path::new(&file));
        }
        if self.play_when_scanned && !self.library.is_empty() {
            self.play_when_scanned = false;
            self.start_loading(0);
        }
        // A lobby map may have been announced before our library was ready.
        if let Some(s) = &mut self.mp
            && let Some(m) = &s.map {
            s.map_entry = find_map(&self.library, m);
            s.have_map = s.map_entry.is_some();
            s.client.send(net::ClientMsg::HasMap(s.have_map));
        }
    }

    /// Folds osu!'s collections and play history in once read, and re-sorts when the
    /// difficulty analysis finishes.
    fn poll_sorting(&mut self) {
        if let Some(rx) = &self.osudb_rx
            && let Ok(db) = rx.try_recv() {
            self.osudb_rx = None;
            if let Some(db) = db {
                self.apply_osu_db(db);
            }
            // With osu!.db's statuses known, the mirrors are asked about the rest.
            self.start_status_sweep();
        }
        let Some(info) = &self.mapinfo else { return };
        let (done, total) = (info.done.load(Ordering::Relaxed), info.total.load(Ordering::Relaxed));
        let finished = info.finished();
        // Sorting or searching by stars, length or BPM waits on the analysis.
        let needs_info = self.select.sort.needs_info() || search::Query::parse(&self.select.filter).needs_info();
        self.select.info_progress = if !finished && needs_info { format!("  |  analysing {done}/{total}") } else { String::new() };
        if finished && !self.mapinfo_sorted {
            self.mapinfo_sorted = true;
            if needs_info {
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
        for m in db.by_md5.values() {
            if let Some(&i) = index.get(&key(&m.folder, &m.file)) {
                self.db_status.insert(self.library[i].path.clone(), m.status);
                // Unless the mirrors have said better already.
                self.select.status.entry(self.library[i].path.clone()).or_insert(m.status);
            }
        }
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
        if let Some(file) = config::data_dir().map(|d| d.join("plays.tsv")) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                let _ = writeln!(f, "{now}\t{}", path.display());
            }
        }
        self.select.played.insert(path, now);
    }

    /// F1 / the Mods button: the mod select, on the first mod that's on.
    fn open_mod_select(&mut self) {
        let first = self.cfg.mods.iter().next().and_then(|m| mods::ALL.iter().position(|&x| x == m)).unwrap_or(0);
        self.mod_select = Some(first);
    }

    /// The mods the mod select shows as on: in a lobby as a guest, the host's speed picked for
    /// everyone (which can't be switched here) on top of our own.
    fn shown_mods(&self) -> Mods {
        match &self.mp {
            Some(s) if matches!(self.scene, Scene::Lobby) && !s.is_host() => {
                let lobby = s.map.as_ref().map(|m| Mods::from_list(&m.mods).shared()).unwrap_or(Mods::NONE);
                lobby.plus(self.cfg.mods.free())
            }
            _ => self.cfg.mods,
        }
    }

    fn toggle_mod(&mut self, m: mods::Mod) {
        // In a lobby the host picks the speed for everyone.
        if m.shared() && matches!(self.scene, Scene::Lobby) && self.mp.as_ref().is_some_and(|s| !s.is_host()) {
            self.toast(format!("the host picks the speed ({} is theirs to pick)", m.acronym()));
            return;
        }
        self.cfg.mods.toggle(m);
        self.mods_changed();
    }

    /// Tells the room the mods we picked ourselves, if it doesn't have them.
    fn send_my_mods(&self) {
        let Some(s) = &self.mp else { return };
        let mine = self.cfg.mods.free().list();
        if s.me().is_some_and(|p| p.mods != mine) {
            s.client.send(net::ClientMsg::Mods(mine));
        }
    }

    /// After the mods change: save them, and as the lobby host tell the room.
    fn mods_changed(&mut self) {
        self.cfg.save();
        self.select.mods_label = self.cfg.mods.label();
        self.update_lobby_map();
        self.send_my_mods();
    }

    /// As the lobby host: tells the room the match's rate, mods and mod settings as they are now.
    fn update_lobby_map(&self) {
        let (rate, mods) = (self.play_speed().0, self.cfg.mods);
        if let Some(s) = &self.mp
            && s.is_host() && !s.in_match
                && let Some(mut m) = s.map.clone() {
                    m.rate = rate;
                    // Only the speed's: everyone picks their other mods themselves.
                    m.mods = mods.shared().list();
                    (m.multiverse, m.shapeshift) = mod_settings(mods);
                    m.seed = lobby_seed(m.seed);
                    if Some(&m) != s.map.as_ref() {
                        s.client.send(net::ClientMsg::SelectMap(m));
                    }
                }
    }

    /// Keys while the mod select is open.
    fn mod_select_key(&mut self, code: KeyCode) {
        let Some(cur) = self.mod_select else { return };
        match code {
            KeyCode::ArrowLeft => self.mod_select = Some(mods::step(cur, -1, 0)),
            KeyCode::ArrowRight => self.mod_select = Some(mods::step(cur, 1, 0)),
            KeyCode::ArrowUp => self.mod_select = Some(mods::step(cur, 0, -1)),
            KeyCode::ArrowDown => self.mod_select = Some(mods::step(cur, 0, 1)),
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter => self.toggle_mod(mods::ALL[cur]),
            KeyCode::Backspace | KeyCode::Delete => {
                self.cfg.mods = Mods::NONE;
                self.mods_changed();
            }
            KeyCode::KeyR => {
                let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
                self.cfg.mods = Mods::shuffle(seed);
                self.mods_changed();
            }
            KeyCode::KeyF => self.toggle_favourite(mods::ALL[cur]),
            KeyCode::KeyC | KeyCode::KeyM => match mods::customizable(self.cfg.mods).first() {
                Some(&m) => self.open_customization(m),
                None => self.toast("switch on Multiverse or Shapeshift to customize it".to_string()),
            },
            KeyCode::Escape | KeyCode::F1 | KeyCode::Tab => self.mod_select = None,
            _ => {}
        }
    }

    fn toggle_favourite(&mut self, m: mods::Mod) {
        self.cfg.favourite_mods = self.cfg.favourite_mods.flip(m);
        self.cfg.save();
    }

    /// Opens the mod customization on mod `m`'s tab.
    fn open_customization(&mut self, m: mods::Mod) {
        self.universe_setup = (m == mods::Mod::Multiverse).then(|| multiverse::Setup::new(game::FIRST_FOCUS, 0));
        self.shapeshift_setup = (m == mods::Mod::Shapeshift).then(shapeshift::Setup::default);
    }

    /// The mod customization's open tab.
    fn customization_tab(&self) -> Option<mods::Mod> {
        if self.universe_setup.is_some() {
            Some(mods::Mod::Multiverse)
        } else {
            self.shapeshift_setup.is_some().then_some(mods::Mod::Shapeshift)
        }
    }

    /// Keys in the mod customization: Tab goes to the next tab (closes it with only one), the
    /// rest go to the open one.
    fn customization_key(&mut self, code: KeyCode, text: Option<&str>, repeat: bool) {
        let typing = self.universe_setup.as_ref().is_some_and(|s| s.typing.is_some());
        let tabs = mods::customizable(self.cfg.mods);
        if code == KeyCode::Tab && !typing && tabs.len() > 1 {
            let at = tabs.iter().position(|&m| Some(m) == self.customization_tab()).unwrap_or(0);
            self.open_customization(tabs[(at + 1) % tabs.len()]);
            return;
        }
        let arrows = matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight);
        if repeat && !typing && !arrows {
            return;
        }
        if let Some(setup) = &mut self.universe_setup {
            let action = setup.key(code, text, self.modifiers_shift, &mut self.cfg.universes, &mut self.cfg.universe_presets, game::FIRST_FOCUS);
            self.universe_action(action);
        } else if let Some(setup) = &mut self.shapeshift_setup {
            if matches!(code, KeyCode::Escape | KeyCode::F1 | KeyCode::Backspace | KeyCode::Tab) {
                self.shapeshift_setup = None;
            } else if setup.key(code, self.modifiers_shift, &mut self.cfg.shapeshift) {
                self.shapeshift_changed();
            }
        }
    }

    fn shapeshift_changed(&mut self) {
        shapeshift::set(self.cfg.shapeshift);
        self.cfg.save();
        self.update_lobby_map();
    }

    /// After a key or click in the Multiverse setup: the universes are saved and drawn with, or
    /// it closes (back to the mod select).
    fn universe_action(&mut self, action: multiverse::Action) {
        match action {
            multiverse::Action::Changed => {
                multiverse::set(self.cfg.universes);
                self.cfg.save();
                self.update_lobby_map();
            }
            multiverse::Action::Close => self.universe_setup = None,
            multiverse::Action::None => {}
        }
    }

    /// Tab in play: hides (or shows again) the offset / UR line.
    /// Somewhere text can be typed now (Ctrl+V pastes there).
    fn takes_text(&self) -> bool {
        self.social.open
            || self.settings_ui.editing.is_some()
            || self.player.as_ref().is_some_and(|p| p.searching)
            || self.universe_setup.as_ref().is_some_and(|u| u.typing.is_some())
            || (self.mod_select.is_none() && !self.on_menu() && matches!(self.scene, Scene::Select | Scene::Download | Scene::MpMenu | Scene::Lobby))
            || (matches!(self.scene, Scene::Settings { .. }) && self.settings_ui.binding.is_none() && self.settings_ui.choices.is_none() && self.settings_ui.dropdown.is_none())
    }

    /// Android: something is being typed that took the keys by itself (not a field that was
    /// tapped): the on-screen keyboard comes up for it.
    #[cfg(target_os = "android")]
    fn typing_on_its_own(&self) -> bool {
        (self.settings_ui.editing.is_some() && matches!(self.scene, Scene::Settings { .. }))
            || self.settings_ui.dropdown.as_ref().is_some_and(|d| d.editing.is_some())
            || self.universe_setup.as_ref().is_some_and(|u| u.typing.is_some())
    }

    /// Android: whether tapping `hit` is tapping a text field.
    #[cfg(target_os = "android")]
    fn is_text_field(&self, hit: render::Hit) -> bool {
        use render::Hit;
        match hit {
            Hit::TextField | Hit::SettingsSearch => true,
            Hit::MpRow(i) => matches!(self.scene, Scene::MpMenu) && self.mp_menu.rows().get(i).is_some_and(|r| !r.is_action()),
            Hit::SocialRow(i) => matches!(self.social.rows(self.mp.is_some()).get(i), Some(social::Row::AddFriend | social::Row::Search)),
            _ => false,
        }
    }

    /// Android: a tap brings the on-screen keyboard up on a text field (again, if the player put
    /// it away), and anywhere else puts it away.
    #[cfg(target_os = "android")]
    fn keyboard_tap(&mut self) {
        let (vx, vy) = self.mouse_virtual();
        let field = self.hit_at(vx, vy).is_some_and(|h| self.is_text_field(h));
        self.keyboard.asked = field;
        if field {
            // Asked for anew, so it starts afresh on this field.
            self.keyboard.wanted = false;
        }
    }

    /// Android: shows the on-screen keyboard while it's wanted (something typing by itself, or a
    /// tapped text field that still takes text) and hides it after.
    #[cfg(target_os = "android")]
    fn update_keyboard(&mut self) {
        let field = self.keyboard.asked && self.takes_text() && !(matches!(self.scene, Scene::MpMenu) && self.mp_menu.row().is_action());
        if !field {
            // Not again by itself when coming back to the screen.
            self.keyboard.asked = false;
        }
        let want = field || self.typing_on_its_own();
        if want != self.keyboard.wanted {
            self.keyboard.wanted = want;
            self.keyboard.up = want;
            android::keyboard(want);
        }
    }

    /// Android: what the on-screen keyboard typed, as presses of those keys.
    #[cfg(target_os = "android")]
    fn soft_keyboard(&mut self, typed: android::Typed, window: Option<&Window>) {
        let at = Instant::now();
        match typed {
            android::Typed::Keys(keys) => {
                for (code, text) in keys {
                    let text = Some(text.as_str()).filter(|t| !t.chars().any(char::is_control));
                    self.key(code, false, text, at, window);
                }
            }
            android::Typed::Key(code) => self.key(code, false, None, at, window),
            android::Typed::Shown(up) => {
                self.keyboard.up = up;
                if !up {
                    // Put away by the player: back on a tap.
                    self.keyboard.asked = false;
                }
            }
        }
    }

    /// Android: what's being typed (what it is, and the text), for showing above the on-screen
    /// keyboard, which may cover where it is. None for the search boxes, which are above it.
    #[cfg(target_os = "android")]
    fn typing_line(&self) -> Option<(String, String)> {
        if let Some((_, text)) = self.settings_ui.dropdown.as_ref().and_then(|d| d.editing.as_ref()) {
            return Some(("Size".into(), text.clone()));
        }
        if let Some(e) = self.settings_ui.editing.as_ref().filter(|_| matches!(self.scene, Scene::Settings { .. })) {
            return Some((e.item.label().into(), e.text.clone()));
        }
        if let Some(t) = self.universe_setup.as_ref().and_then(|u| u.typing.as_ref()) {
            return Some(match t {
                multiverse::Typing::Colours(s) => ("Colours".into(), s.clone()),
                multiverse::Typing::Name(s) => ("Preset name".into(), s.clone()),
            });
        }
        if self.social.open {
            let label = match self.social.row(self.mp.is_some()) {
                social::Row::AddFriend => "Add friend",
                social::Row::Search => "Search players",
                _ => "Message",
            };
            return Some((label.into(), self.social.input.clone()));
        }
        if matches!(self.scene, Scene::MpMenu) {
            use multiplayer::Row;
            return match self.mp_menu.row() {
                Row::User => Some(("Name or email".into(), self.mp_menu.user.clone())),
                Row::Password => Some(("Password".into(), "*".repeat(self.mp_menu.password.chars().count()))),
                Row::Email => Some(("Email".into(), self.mp_menu.email.clone())),
                Row::Code => Some(("Code".into(), self.mp_menu.code.clone())),
                Row::Name => Some(("Your name".into(), self.cfg.mp_name.clone())),
                Row::Port => Some(("Host on port".into(), self.cfg.mp_port.to_string())),
                Row::Address => Some(("Join address".into(), self.cfg.mp_address.clone())),
                _ => None,
            };
        }
        None
    }

    /// Android: the text being typed in a strip along the top, clear of the on-screen keyboard.
    #[cfg(target_os = "android")]
    fn draw_typing(&self, c: &mut Canvas) {
        let Some((label, text)) = self.typing_line() else { return };
        c.rect(0.0, 0.0, c.width, 30.0, rgba(0x15151d, 1.0));
        c.rect(0.0, 30.0, c.width, 1.0, rgba(render::accent(), 1.0));
        let lw = c.text(16.0, 10.5, 9.0, rgba(render::accent(), 1.0), &label);
        let x = 16.0 + lw + 12.0;
        c.text(x, 10.0, 10.0, rgba(0xffffff, 1.0), &Canvas::fit_end(10.0, c.width - x - 16.0, &format!("{text}_")));
        c.hit(0.0, 0.0, c.width, 31.0, render::Hit::TextField);
    }

    /// `/np`'s line: the map on this screen, with its set's osu! link when it has one.
    fn now_playing(&self) -> Option<String> {
        let e = self.library.get(self.offset_entry()?)?;
        let name = format!("{} - {} [{}]", e.meta.artist, e.meta.title, e.meta.version);
        let set = e.meta.set_id.or_else(|| e.path.parent().and_then(download::set_id_of_folder));
        Some(match set {
            Some(set) => format!("is playing {name} https://osu.ppy.sh/beatmapsets/{set}"),
            None => format!("is playing {name}"),
        })
    }

    /// A map set named in the chat, clicked: picked in song select when we have it, else downloaded.
    fn open_chat_map(&mut self, set: u32) {
        self.main_menu = None;
        let have = self.library.iter().position(|e| e.meta.set_id.or_else(|| e.path.parent().and_then(download::set_id_of_folder)) == Some(set));
        match have {
            Some(i) if matches!(self.scene, Scene::Select) => {
                let path = self.library[i].path.clone();
                self.select.select_path(&self.library, &path);
                self.social.close();
            }
            Some(i) => {
                let label = self.library[i].label();
                self.toast(format!("you have it: {label}"));
            }
            None => self.start_download(download::SetInfo {
                id: set,
                artist: String::new(),
                title: format!("set {set}"),
                creator: String::new(),
                status: String::new(),
                diffs: Vec::new(),
                play_count: 0,
                favourite_count: 0,
                rating: 0.0,
                ranked_date: String::new(),
            }),
        }
    }

    /// What Ctrl+C copies here: the text being typed, else the map or set picked.
    fn copy_text(&self) -> Option<String> {
        if let Some(e) = &self.settings_ui.editing {
            return Some(e.text.clone()).filter(|t| !t.is_empty());
        }
        if self.social.open {
            return Some(self.social.input.clone()).filter(|t| !t.is_empty());
        }
        match &self.scene {
            Scene::Select if !self.select.filter.is_empty() && self.select.selected().is_none() => Some(self.select.filter.clone()),
            Scene::Select => self.select.selected().and_then(|i| self.library.get(i)).map(Entry::label),
            Scene::Download => self.browse.selected().map(|s| format!("{} - {}  https://osu.ppy.sh/beatmapsets/{}", s.artist, s.title, s.id)),
            _ => None,
        }
    }

    /// The status on osu! of library map `i`, as (label, colour): the mirrors' when known,
    /// else osu!.db's (which can't tell graveyard from pending).
    fn map_status(&self, i: usize) -> Option<(&'static str, u32)> {
        let e = self.library.get(i)?;
        let Some(set) = e.meta.set_id.filter(|_| !self.select.edited.contains(&e.path)) else { return Some(("NOT SUBMITTED", 0x8a8aa0)) };
        if self.select.server_loved.contains(&set) {
            return Some(("LOVED HERE", 0xff66aa));
        }
        if let Some(Some(s)) = self.set_status.get(&set) {
            let known = match s.as_str() {
                "ranked" => Some(("RANKED", 0xb3ff66)),
                "approved" => Some(("APPROVED", 0xb3ff66)),
                "qualified" => Some(("QUALIFIED", 0x66ccff)),
                "loved" => Some(("LOVED", 0xff66aa)),
                "pending" => Some(("PENDING", 0xffd966)),
                "wip" => Some(("WIP", 0xffd966)),
                "graveyard" => Some(("GRAVEYARD", 0x8a8aa0)),
                _ => None,
            };
            // Not one the mirrors know (they don't have the set): osu!.db's.
            if known.is_some() {
                return known;
            }
        }
        Some(match self.db_status.get(&e.path)? {
            4 => ("RANKED", 0xb3ff66),
            5 => ("APPROVED", 0xb3ff66),
            6 => ("QUALIFIED", 0x66ccff),
            7 => ("LOVED", 0xff66aa),
            2 => ("PENDING", 0xffd966),
            1 => ("NOT SUBMITTED", 0x8a8aa0),
            _ => return None,
        })
    }

    fn status_cache_due(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.status_saved) >= Duration::from_secs(15)
    }

    /// The sets' statuses the cache has, for the maps in the library (before osu!.db's).
    fn apply_cached_statuses(&mut self) {
        for e in &self.library {
            let code = e.meta.set_id.and_then(|set| self.status_cache.status(set)).and_then(search::status_code);
            if let Some(code) = code {
                self.select.status.insert(e.path.clone(), code);
            }
        }
    }

    /// Asks the online server in the background about every set whose status isn't known or is
    /// old (osu!.db knows a ranked, approved or loved one for sure): it answers what it has at
    /// once and looks the rest up on osu! for the next round, so sorting by status works for the
    /// whole library. The answers are kept (`setstatus`) for the next run. The mirrors aren't
    /// asked: however many play, they hear nothing from us for this.
    fn start_status_sweep(&mut self) {
        // A newer sweep (more maps came in) takes over from one still asking.
        static LATEST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if cfg!(test) {
            return;
        }
        let generation = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
        let now = setstatus::now();
        let mut maps: Vec<(u32, u32)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for e in &self.library {
            let (Some(set), Some(map)) = (e.meta.set_id, e.meta.beatmap_id) else { continue };
            let sure = matches!(self.db_status.get(&e.path), Some(4 | 5 | 7));
            if !sure && self.status_cache.due(set, now) && seen.insert(set) {
                maps.push((set, map));
            }
        }
        if maps.is_empty() {
            return;
        }
        let (server, token) = (self.cfg.online_server.clone(), self.cfg.online_token.clone());
        let tx = self.set_status_tx.clone();
        std::thread::spawn(move || {
            let mut failed = 0;
            // The server looks up what it doesn't know in the background (some 45 maps a second):
            // ask again, now and then, about what's left.
            for round in 0..STATUS_ROUNDS {
                if round > 0 {
                    std::thread::sleep(Duration::from_secs(STATUS_ROUND_SECS));
                }
                if LATEST.load(Ordering::SeqCst) != generation {
                    return;
                }
                let mut left = Vec::new();
                for chunk in maps.chunks(online::MAX_STATUS_MAPS) {
                    match online::call(&server, online::Request::SetStatuses { token: token.clone(), maps: chunk.to_vec() }) {
                        Ok(online::Reply::SetStatuses(found)) => {
                            failed = 0;
                            let got: std::collections::HashSet<u32> = found.iter().map(|(set, _)| *set).collect();
                            left.extend(chunk.iter().filter(|(set, _)| !got.contains(set)).copied());
                            for (set, status) in found {
                                if tx.send((set, Some(status))).is_err() {
                                    return;
                                }
                            }
                        }
                        // An older server, offline: leave it to the next run.
                        _ => {
                            failed += 1;
                            left.extend_from_slice(chunk);
                        }
                    }
                }
                maps = left;
                // Without a login the server only says what it has: nothing more will come.
                if maps.is_empty() || failed >= 3 || token.is_empty() {
                    break;
                }
            }
        });
    }

    /// Asks the mirrors for the selected set's status once the selection has rested a moment
    /// (scrolling past sets asks about none of them), and takes in what they say.
    fn poll_set_status(&mut self, now: Instant) {
        // A sweep's answers come thousands at a time: the library is gone through once for them.
        let mut by_set: Option<std::collections::HashMap<u32, Vec<usize>>> = None;
        while let Ok((set, status)) = self.set_status_rx.try_recv() {
            // The mirrors know graveyard from pending, which osu!.db doesn't: the search uses theirs.
            if let Some(code) = status.as_deref().and_then(search::status_code) {
                let by_set = by_set.get_or_insert_with(|| {
                    let mut m: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
                    for (i, e) in self.library.iter().enumerate() {
                        if let Some(set) = e.meta.set_id {
                            m.entry(set).or_default().push(i);
                        }
                    }
                    m
                });
                for &i in by_set.get(&set).into_iter().flatten() {
                    self.select.status.insert(self.library[i].path.clone(), code);
                }
            }
            if let Some(status) = &status {
                self.status_cache.put(set, status, setstatus::now());
                self.status_new = true;
            }
            self.set_status.insert(set, status);
        }
        if self.status_sweep_due.is_some_and(|t| now >= t) {
            self.status_sweep_due = None;
            self.start_status_sweep();
        }
        if self.status_cache_due(now) {
            self.status_cache.save();
            self.status_saved = now;
        }
        // The list sorted (or searched) by status follows what the mirrors have said since.
        let by_status = self.select.sort == ui::SortMode::Status || self.select.filter.contains("status");
        if self.status_new && by_status && now.saturating_duration_since(self.status_resorted) >= Duration::from_secs(10) {
            self.status_new = false;
            self.status_resorted = now;
            self.select.refilter(&self.library);
        }
        let set = self.select.selected().and_then(|i| self.library.get(i)).and_then(|e| e.meta.set_id);
        let Some(set) = set.filter(|s| !self.set_status.contains_key(s)) else {
            self.status_wait = None;
            return;
        };
        match self.status_wait {
            Some((s, since)) if s == set => {
                if now.saturating_duration_since(since) < Duration::from_millis(500) {
                    return;
                }
            }
            _ => {
                self.status_wait = Some((set, now));
                return;
            }
        }
        self.status_wait = None;
        self.set_status.insert(set, None);
        // Tests don't ask the internet.
        if cfg!(test) {
            return;
        }
        let tx = self.set_status_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((set, download::set_status(set).ok()));
        });
    }

    /// Showing a song, its background dimmed by the background dim setting.
    fn dims_background(&self) -> bool {
        matches!(
            self.scene,
            Scene::Play { .. } | Scene::MpPlay { .. } | Scene::MpWait { .. } | Scene::Loading { .. } | Scene::Preview { .. } | Scene::Spectate(_) | Scene::Watch(_) | Scene::VideoEdit(_)
        )
    }

    /// What the window is cleared to under the background: black in a song when the background
    /// dims to black, else the interface's grey.
    fn backdrop(&self, in_song: bool) -> [f64; 3] {
        if in_song && self.cfg.bg_dim_black { [0.0; 3] } else { render::clear_colour() }
    }

    fn toggle_map_info(&mut self) {
        self.cfg.show_map_info = !self.cfg.show_map_info;
        self.cfg.save();
    }

    fn cycle_sort(&mut self, dir: i32) {
        self.select.cycle_sort(dir, &self.library);
        self.sort_changed();
    }

    fn sort_changed(&mut self) {
        self.cfg.sort = self.select.sort.name().to_string();
        self.cfg.save();
        self.toast(format!("sorted by {}", self.select.sort.name()));
    }

    fn open_collections(&mut self) {
        if self.select.collections.is_empty() {
            self.toast("no osu! collections found (collection.db next to your Songs folder)".into());
        } else {
            self.select.sorting = false;
            self.select.picker = Some(self.select.collection.map(|c| c + 1).unwrap_or(0));
        }
    }

    fn open_sorts(&mut self) {
        self.select.sorting = true;
        self.select.picker = ui::SortMode::ALL.iter().position(|&m| m == self.select.sort);
    }

    /// Row `i` of the collection or sort list.
    fn pick_row(&mut self, i: usize) {
        if !self.select.sorting {
            self.select.pick_collection(i, &self.library);
        } else if let Some(&sort) = ui::SortMode::ALL.get(i) {
            self.select.set_sort(sort, &self.library);
            self.sort_changed();
        }
    }

    /// The friends overlay (and notifications) can show: outside songs, and while watching
    /// someone else play (spectating a match, watching a friend), never over your own play.
    fn overlay_allowed(&self) -> bool {
        !self.in_song() || matches!(self.scene, Scene::Spectate(_) | Scene::Watch(_))
    }

    fn open_social(&mut self) {
        if !self.social.open {
            self.social.open = true;
            self.social.opened_at = Some(Instant::now());
        }
    }

    /// F9: the friends overlay (not during songs).
    fn toggle_social(&mut self) {
        if self.social.open {
            self.social.close();
        } else if self.overlay_allowed() {
            self.open_social();
            self.social.mark_read(self.mp.is_some());
        }
    }

    fn social_key(&mut self, code: KeyCode, text: Option<&str>) {
        use social::Row;
        let in_lobby = self.mp.is_some();
        let rows = self.social.rows(in_lobby).len();
        let row = self.social.row(in_lobby);
        match code {
            KeyCode::Escape => self.social.close(),
            KeyCode::Tab => self.social.cycle_tab(self.modifiers_shift),
            KeyCode::PageUp => self.social.wheel(5, f32::MAX, in_lobby),
            KeyCode::PageDown => self.social.wheel(-5, f32::MAX, in_lobby),
            KeyCode::ArrowUp => self.social.cursor = self.social.cursor.min(rows - 1).saturating_sub(1),
            KeyCode::ArrowDown => self.social.cursor = (self.social.cursor + 1).min(rows - 1),
            KeyCode::Backspace => {
                self.social.input.pop();
            }
            KeyCode::Delete if self.social.input.is_empty() => match row {
                Row::Request(_) => self.social_action(social::Action::Decline),
                Row::Friend(k) => {
                    if let (Some(f), Some(c)) = (self.social.friends.get(k), &self.social.conn) {
                        c.send(online::Up::RemoveFriend(f.name.clone()));
                        self.toast(format!("removed {} from your friends", f.name));
                    }
                }
                _ => {}
            },
            KeyCode::KeyW if self.modifiers_ctrl => self.social_action(social::Action::Watch),
            KeyCode::KeyP if self.modifiers_ctrl => self.social_action(social::Action::Profile),
            KeyCode::KeyJ if self.modifiers_ctrl => self.social_action(social::Action::Join),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let mut text = self.social.input.trim().to_string();
                // `/np` in a chat: the map on screen, with its osu! link (a link others can click).
                if text.eq_ignore_ascii_case("/np") && matches!(row, Row::Friend(_) | Row::Lobby | Row::Global) {
                    match self.now_playing() {
                        Some(np) => text = np,
                        None => {
                            self.toast("no map to share: pick one first".into());
                            return;
                        }
                    }
                }
                match row {
                    Row::AddFriend if !text.is_empty() => {
                        if let Some(c) = &self.social.conn {
                            c.send(online::Up::AddFriend(text));
                        }
                        self.social.input.clear();
                    }
                    Row::Request(_) => self.social_action(social::Action::Accept),
                    Row::Friend(k) if !text.is_empty() => {
                        if let (Some(f), Some(c)) = (self.social.friends.get(k), &self.social.conn) {
                            c.send(online::Up::Chat { to: f.name.clone(), text });
                        }
                        self.social.input.clear();
                    }
                    Row::Lobby if !text.is_empty() => {
                        if let Some(s) = &self.mp {
                            s.client.send(net::ClientMsg::Chat(text));
                        }
                        self.social.input.clear();
                    }
                    Row::Global if !text.is_empty() => {
                        if let Some(c) = self.social.conn.as_ref().filter(|_| self.social.connected) {
                            c.send(online::Up::GlobalChat(text));
                            self.social.input.clear();
                            self.social.chat_scroll = 0;
                        } else {
                            self.toast("not connected to the server".into());
                        }
                    }
                    Row::Player(k) => {
                        let name = self.social.players.as_ref().and_then(|v| v.get(k)).map(|p| p.name.clone()).unwrap_or_default();
                        match self.social.standing(&name) {
                            social::Standing::Stranger => self.social_action(social::Action::Add),
                            social::Standing::AsksUs => self.social_action(social::Action::Accept),
                            social::Standing::Friend => self.social_action(social::Action::Chat),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                let Some(text) = text else { return };
                let searching = self.social.tab == social::Tab::Players;
                for ch in text.chars().filter(|c| net::drawable_char(*c)) {
                    let len = self.social.input.chars().count();
                    let ok = match row {
                        _ if searching => len < 16 && (ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'),
                        Row::AddFriend => len < 16 && (ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'),
                        _ => len < 200,
                    };
                    if ok {
                        self.social.input.push(ch);
                    }
                }
            }
        }
        self.social.mark_read(in_lobby);
        if self.social.row(in_lobby) == Row::Lobby
            && let Some(s) = &mut self.mp {
            s.chat_unread = 0;
        }
    }

    /// A button on the selected overlay row (or its keyboard shortcut).
    fn social_action(&mut self, action: social::Action) {
        use social::{Action, Row};
        let in_lobby = self.mp.is_some();
        match (self.social.row(in_lobby), action) {
            (Row::Request(k), Action::Accept | Action::Decline) => {
                let Some(name) = self.social.requests.get(k).cloned() else { return };
                if let Some(c) = &self.social.conn {
                    c.send(if action == Action::Accept { online::Up::AddFriend(name) } else { online::Up::RemoveFriend(name) });
                }
            }
            (Row::Player(k), Action::Add | Action::Accept | Action::Chat) => {
                let Some(name) = self.social.players.as_ref().and_then(|v| v.get(k)).map(|p| p.name.clone()) else { return };
                if action == Action::Chat {
                    self.social.select_friend(&name, in_lobby);
                    self.social.mark_read(in_lobby);
                } else if let Some(c) = self.social.conn.as_ref().filter(|_| self.social.connected) {
                    // Accepting is the same request: the server sees they asked first.
                    c.send(online::Up::AddFriend(name));
                } else {
                    self.toast("not connected to the server".into());
                }
            }
            (Row::Friend(k), Action::Profile) => {
                if let Some(f) = self.social.friends.get(k) {
                    self.open_profile(f.name.clone());
                }
            }
            (Row::Player(k), Action::Profile) => {
                if let Some(p) = self.social.players.as_ref().and_then(|v| v.get(k)) {
                    self.open_profile(p.name.clone());
                }
            }
            // Elsewhere: our own.
            (_, Action::Profile) => match self.mp_menu.account.clone() {
                Some(me) => self.open_profile(me),
                None => self.toast("log in (F9) to have a profile".into()),
            },
            (Row::Friend(k), Action::Watch) => {
                if let Some(f) = self.social.friends.get(k).cloned() {
                    self.watch(f.name);
                }
            }
            (Row::Friend(k), Action::Join) => {
                let Some(f) = self.social.friends.get(k).cloned() else { return };
                let Some(online::Activity::Lobby { room: Some(room), title }) = f.activity else {
                    self.toast(format!("{} isn't in an online lobby", f.name));
                    return;
                };
                if self.mp.is_some() || self.mp_connecting.is_some() {
                    self.toast("leave your lobby first".into());
                    return;
                }
                self.social.close();
                self.mp_status = format!("joining {title}...");
                self.scene = Scene::MpMenu;
                let request = online::Request::JoinRoom { token: self.cfg.online_token.clone(), room };
                let account = self.mp_menu.account.clone().unwrap_or_default();
                self.mp_connecting = Some(multiplayer::start_online(self.cfg.online_server.clone(), request, account));
            }
            _ => {}
        }
    }

    fn open_profile(&mut self, name: String) {
        self.profile = Some(profile::View::open(self.cfg.online_server.clone(), name));
    }

    /// Starts watching a friend's solo plays.
    fn watch(&mut self, name: String) {
        if self.mp.is_some() {
            self.toast("leave your lobby first".into());
            return;
        }
        let Some(c) = self.social.conn.as_ref().filter(|_| self.social.connected) else {
            self.toast("not connected to the server".into());
            return;
        };
        c.send(online::Up::Watch(name.clone()));
        self.stop_audio();
        self.social.close();
        self.scene = Scene::Watch(Box::new(WatchView::new(name)));
    }

    fn stop_watching(&mut self) {
        if let Some(c) = &self.social.conn {
            c.send(online::Up::StopWatching);
        }
        self.stop_audio();
        self.show_select();
    }

    /// What we're doing, for friends.
    fn activity(&self) -> online::Activity {
        use online::Activity;
        let lobby_map = || self.mp.as_ref().and_then(|s| s.map.as_ref()).map(|m| m.label()).unwrap_or_default();
        match &self.scene {
            Scene::Play { game, entry, .. } if game.replay.is_none() => Activity::Playing { map: self.library.get(*entry).map(|e| e.label()).unwrap_or_default() },
            Scene::Watch(v) => Activity::Watching { name: v.player.clone() },
            Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::Spectate(_) => Activity::Match { map: lobby_map() },
            _ => match &self.mp {
                Some(s) => Activity::Lobby { room: s.online_room, title: s.label.strip_prefix("online: ").unwrap_or(&s.label).to_string() },
                None => Activity::Menus,
            },
        }
    }

    /// The social connection: (re)connects while logged in, applies what the server says and
    /// keeps our status current.
    fn poll_social(&mut self) {
        if self.social.conn.is_none() && self.mp_menu.account.is_some() && !self.cfg.online_token.is_empty() {
            self.social.conn = Some(online::Social::start(self.cfg.online_server.clone(), self.cfg.online_token.clone()));
        }
        let Some(conn) = &self.social.conn else { return };
        for event in conn.poll() {
            match event {
                online::SocialEvent::Connected(name) => {
                    self.social.connected = true;
                    self.social.me = Some(name);
                    // The server resends the friend list and recent chat.
                    self.social.chats.clear();
                    self.social.global.clear();
                    self.social.players = None;
                    self.social.have_list = false;
                    self.social.sent_activity = None;
                    self.stream = None;
                    if let Scene::Watch(v) = &self.scene {
                        let player = v.player.clone();
                        if let Some(c) = &self.social.conn {
                            c.send(online::Up::Watch(player));
                        }
                    }
                }
                online::SocialEvent::Disconnected(_) => {
                    if self.social.connected {
                        self.social.notify("offline", "lost the connection to the server", 0xFF4B4B, social::NoteAction::None);
                    }
                    self.social.connected = false;
                    self.social.players = None;
                    for f in &mut self.social.friends {
                        f.activity = None;
                    }
                }
                online::SocialEvent::SessionExpired => {
                    self.sign_out("your session expired: log in again");
                    return;
                }
                online::SocialEvent::Down(d) => self.social_down(*d),
            }
        }
        if self.social.connected {
            let a = self.activity();
            if self.social.sent_activity.as_ref() != Some(&a) {
                if let Some(c) = &self.social.conn {
                    c.send(online::Up::Status(a.clone()));
                }
                self.social.sent_activity = Some(a);
            }
        }
        let hold = !self.overlay_allowed();
        self.social.expire_notes(hold);
        if self.social.open && self.social.row(self.mp.is_some()) == social::Row::Lobby
            && let Some(s) = &mut self.mp {
            s.chat_unread = 0;
        }
    }

    fn social_down(&mut self, d: online::Down) {
        use online::{Activity, Down};
        use social::{key, NoteAction};
        match d {
            Down::Friends { friends, requests, sent } => {
                if self.social.have_list {
                    for f in &friends {
                        let old = self.social.friends.iter().find(|o| key(&o.name) == key(&f.name));
                        // A new friend isn't "coming online"; only one who was offline is.
                        let was_online = old.is_none_or(|o| o.activity.is_some());
                        let was_playing = old.is_some_and(|o| matches!(o.activity, Some(Activity::Playing { .. })));
                        match &f.activity {
                            Some(Activity::Playing { map }) if !was_playing && self.cfg.notify_playing => {
                                self.social.notify(format!("{} is playing", f.name), map.clone(), 0x438EAD, NoteAction::Watch(f.name.clone()))
                            }
                            Some(a) if !was_online && self.cfg.notify_online => {
                                self.social.notify(format!("{} is online", f.name), a.describe(), 0x86E05A, NoteAction::Chat(f.name.clone()))
                            }
                            _ => {}
                        }
                    }
                    for r in &requests {
                        if !self.social.requests.contains(r) && self.cfg.notify_requests {
                            self.social.notify("friend request", format!("{r} wants to be friends"), 0xFFD34D, NoteAction::Requests);
                        }
                    }
                }
                self.social.have_list = true;
                self.social.friends = friends;
                self.social.requests = requests;
                self.social.sent = sent;
                let rows = self.social.rows(self.mp.is_some()).len();
                self.social.cursor = self.social.cursor.min(rows - 1);
            }
            Down::Chat(mut line) => {
                if self.cfg.chat_filter {
                    line.text = net::censor(&line.text);
                }
                let me = self.social.me.clone().unwrap_or_default();
                let mine = key(&line.from) == key(&me);
                let other = if mine { line.to.clone() } else { line.from.clone() };
                // Lines from before we connected are history (sent again on connect): no alerts.
                let fresh = line.at + 10 >= discord::unix_now();
                let log = self.social.chats.entry(key(&other)).or_default();
                log.push(line.clone());
                if log.len() > 200 {
                    log.remove(0);
                }
                let viewing = self.social.open && matches!(self.social.row(self.mp.is_some()), social::Row::Friend(k) if self.social.friends.get(k).is_some_and(|f| key(&f.name) == key(&other)));
                // A chime for every fresh message from a friend, unless it would land in the middle
                // of a play.
                if !mine && fresh && self.cfg.message_sound && !self.playing()
                    && let Some(a) = &self.audio {
                    a.sfx_at(audio::Sfx::Message, 1.0);
                }
                if !mine && fresh && !viewing {
                    *self.social.unread.entry(key(&other)).or_default() += 1;
                    if self.cfg.notify_messages {
                        self.social.notify(line.from.clone(), line.text.clone(), 0x4fb4ff, NoteAction::Chat(line.from.clone()));
                    }
                }
            }
            Down::Notice(text) => self.social.notify("taiko.rs", text, 0x9a9ab0, NoteAction::None),
            Down::Players { count, players } => {
                self.social.online_total = count as usize;
                self.social.players = Some(players);
                let rows = self.social.rows(self.mp.is_some()).len();
                self.social.cursor = self.social.cursor.min(rows - 1);
            }
            Down::GlobalChat(mut line) => {
                if self.cfg.chat_filter {
                    line.text = net::censor(&line.text);
                }
                let mine = self.social.me.as_deref().is_some_and(|me| key(me) == key(&line.from));
                let fresh = line.at + 10 >= discord::unix_now();
                let viewing = self.social.open && self.social.tab == social::Tab::Chat;
                if !mine && fresh && !viewing {
                    self.social.global_unread += 1;
                    // Only lines that call us by name pop up: the rest just count.
                    if self.cfg.notify_mentions && self.social.mentions_me(&line.text) {
                        self.social.notify(format!("{} in chat", line.from), line.text.clone(), 0x4fb4ff, NoteAction::Global);
                    }
                }
                // Scrolled back, the view stays on the same lines.
                if self.social.chat_scroll > 0 && viewing {
                    self.social.chat_scroll += social::wrap(&format!("{}: {}", line.from, line.text), social::chat_width()).len();
                }
                self.social.global.push(line);
                if self.social.global.len() > 200 {
                    self.social.global.remove(0);
                }
            }
            Down::WatchStart { player, play, t, paused, inputs } => {
                if let Scene::Watch(v) = &mut self.scene
                    && key(&v.player) == key(&player) {
                    v.play = Some(play);
                    v.inputs = inputs;
                    v.clock = (t, Instant::now(), paused);
                    v.game = None;
                    v.rx = None;
                    v.entry = None;
                    v.held = false;
                    v.fetching = false;
                    v.looked = None;
                    if let Some(a) = &self.audio {
                        a.stop();
                    }
                }
            }
            Down::WatchFrame { t, paused, inputs } => {
                if let Scene::Watch(v) = &mut self.scene {
                    v.clock = (t, Instant::now(), paused);
                    match v.game.as_mut().and_then(|g| g.replay.as_mut()) {
                        Some((list, _)) => list.extend_from_slice(&inputs),
                        None => v.inputs.extend_from_slice(&inputs),
                    }
                }
            }
            Down::WatchEnd { player } => {
                if let Scene::Watch(v) = &mut self.scene
                    && key(&v.player) == key(&player) {
                    let name = v.player.clone();
                    **v = WatchView::new(name);
                    if let Some(a) = &self.audio {
                        a.stop();
                    }
                }
            }
            Down::WatchStopped(why) => {
                self.toast(why);
                if matches!(self.scene, Scene::Watch(_)) {
                    self.stop_audio();
                    self.show_select();
                }
            }
            Down::Pong => {}
        }
    }

    /// Streams a solo play to friends who watch: its start, then every 100 ms where it is and
    /// the presses since (the first frame sends all of them, for a mid-play reconnect).
    fn stream_play(&mut self) {
        let connected = self.social.connected;
        let Scene::Play { game, entry, .. } = &mut self.scene else {
            if self.stream.take().is_some() && connected
                && let Some(c) = &self.social.conn {
                c.send(online::Up::PlayEnd);
            }
            return;
        };
        let Some(conn) = self.social.conn.as_ref().filter(|_| connected && game.replay.is_none()) else {
            self.stream = None;
            return;
        };
        let id = &**game as *const Game as usize;
        if self.stream.map(|s| s.0) != Some(id) {
            if self.stream.is_some() {
                conn.send(online::Up::PlayEnd);
            }
            game.log_inputs = true;
            let Some(e) = self.library.get(*entry) else { return };
            conn.send(online::Up::PlayStart(online::SoloPlay { map: net::MapInfo { seed: (game.seed != 0).then_some(game.seed), ..map_info(e, game.rate, game.mods) }, anti_mash: true }));
            self.stream = Some((id, 0, Instant::now() - Duration::from_secs(1)));
        }
        let Some((_, sent, last)) = self.stream.as_mut() else { return };
        if last.elapsed() >= Duration::from_millis(100) {
            *last = Instant::now();
            let inputs = game.inputs.get(*sent..).unwrap_or_default().to_vec();
            *sent = game.inputs.len();
            conn.send(online::Up::PlayFrame { t: game.time, paused: game.paused.is_some() || game.resuming.is_some(), inputs });
        }
    }

    /// The watch view: finds (or downloads) and loads the map, starts the replay where the
    /// friend is (minus the delay), holds while they pause and catches up when they skip.
    fn watch_tick(&mut self) {
        if !matches!(self.scene, Scene::Watch(_)) {
            return;
        }
        let Scene::Watch(mut v) = std::mem::replace(&mut self.scene, Scene::Select) else { return };
        if let (Some(play), None, None) = (v.play.clone(), v.entry, &v.game)
            && v.looked.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            v.looked = Some(Instant::now());
            match find_map(&self.library, &play.map) {
                Some(i) => {
                    v.entry = Some(i);
                    self.preview.show_background_for(&self.library[i]);
                    v.rx = Some(self.load_with(i, play.map.rate, Mods::from_list(&play.map.mods)));
                }
                None if !v.fetching => {
                    v.fetching = true;
                    self.fetch_map(play.map.clone());
                }
                None => {}
            }
        }
        if let Some(rx) = &v.rx {
            match rx.try_recv() {
                Ok(Ok((map, track))) => {
                    v.rx = None;
                    let rate = v.play.as_ref().map(|p| p.map.rate).unwrap_or(1.0);
                    let mods = v.play.as_ref().map(|p| Mods::from_list(&p.map.mods)).unwrap_or_default();
                    let seed = v.play.as_ref().and_then(|p| p.map.seed).unwrap_or(0);
                    let mut g = Game::with_seed(map, track, self.audio.as_ref(), mods, seed);
                    g.live_pp = v.entry.and_then(|i| pp::Live::start(self.library[i].path.clone(), g.rate, mods, 1.0, seed, Default::default()));
                    if let Some(p) = &v.play {
                        use_mod_settings(&mut g, &p.map);
                    }
                    g.replay = Some((std::mem::take(&mut v.inputs), 0));
                    g.watched = true;
                    g.banner = Some(format!("WATCHING  -  {}", v.player));
                    let target = v.their_time() - WATCH_DELAY_MS * rate;
                    if target > g.time {
                        g.seek(target, self.audio.as_ref(), self.play_offset);
                    }
                    v.game = Some(Box::new(g));
                }
                Ok(Err(e)) => {
                    v.rx = None;
                    self.toast(format!("can't load the map: {e}"));
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => v.rx = None,
            }
        }
        let rate = v.play.as_ref().map(|p| p.map.rate).unwrap_or(1.0);
        let target = v.their_time() - WATCH_DELAY_MS * rate;
        if let Some(g) = &mut v.game {
            if !v.held && g.time > target + 250.0 {
                g.pause(self.audio.as_ref());
                v.held = true;
            } else if v.held && target >= g.time {
                g.resume(self.audio.as_ref());
                v.held = false;
            } else if !v.held && g.time < target - 4000.0 * rate {
                // Far behind (they skipped the intro, or we stalled): jump to them.
                g.seek(target, self.audio.as_ref(), self.play_offset);
            }
        }
        self.scene = Scene::Watch(v);
    }

    /// Finishes pending connections and applies messages from the room.
    fn poll_multiplayer(&mut self) {
        self.poll_online();
        self.poll_social();
        self.stream_play();
        self.watch_tick();
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
        if self.mp.is_none() {
            self.guessr = None;
        }
        let Some(s) = &mut self.mp else { return };
        for msg in s.client.poll() {
            self.handle_server_msg(msg);
        }
        if let (Some(s), Some(g)) = (&self.mp, &mut self.guessr) {
            g.tick(s, Instant::now());
        }
    }

    fn handle_server_msg(&mut self, msg: net::ServerMsg) {
        use net::ServerMsg;
        match msg {
            ServerMsg::Room { players, map, in_match, mapguessr, common } => {
                let library = &self.library;
                let Some(s) = &mut self.mp else { return };
                let game_over = s.in_match && !in_match;
                s.players = players;
                // The room learns our own mods (on joining, and if it has others).
                let mine = self.cfg.mods.free().list();
                if !mapguessr && s.me().is_some_and(|p| p.mods != mine) {
                    s.client.send(net::ClientMsg::Mods(mine));
                }
                s.in_match = in_match;
                s.mapguessr = mapguessr;
                s.common = common;
                if mapguessr {
                    let g = self.guessr.get_or_insert_with(|| mapguessr::Guessr::new(library));
                    if game_over {
                        g.game_over();
                        if self.picking_lobby_map && matches!(self.scene, Scene::Select) {
                            self.picking_lobby_map = false;
                            self.scene = Scene::Lobby;
                        }
                    }
                    return;
                }
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
                        // The host's speed mods, and our own others.
                        let mods = Mods::from_list(&map.mods).shared().plus(self.cfg.mods.free());
                        let rx = self.load_with(i, map.rate, mods);
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
            ServerMsg::Spectate { map, at, players } => {
                let Some(s) = &self.mp else { return };
                match find_map(&self.library, &map) {
                    Some(i) => {
                        let start = s.client.local_instant(at);
                        self.stop_audio();
                        self.preview.show_background_for(&self.library[i]);
                        let mods = Mods::from_list(&map.mods);
                        let rx = self.load_with(i, map.rate, mods);
                        self.scene = Scene::Spectate(Box::new(SpectateView {
                            rx: Some(rx),
                            loaded: None,
                            start,
                            from: map.from,
                            rate: map.rate,
                            mods,
                            map: map.clone(),
                            players,
                            games: Vec::new(),
                            focus: Some(0),
                            pending: std::collections::HashMap::new(),
                            skipped: std::collections::HashSet::new(),
                            skip_done: false,
                            zones: Vec::new(),
                            zones_at: None,
                        }));
                    }
                    None => {
                        s.client.send(net::ClientMsg::Spectate(false));
                        self.toast("you need the map to spectate".into());
                    }
                }
            }
            ServerMsg::Skipped { from } => {
                if let Scene::Spectate(v) = &mut self.scene {
                    v.skipped.insert(from);
                    v.follow_skip(self.audio.as_ref(), self.play_offset);
                }
            }
            ServerMsg::Inputs { from, inputs, .. } => {
                if let Scene::Spectate(v) = &mut self.scene {
                    match v.games.iter_mut().find(|(id, _)| *id == from) {
                        Some((_, g)) => {
                            if let Some((list, _)) = &mut g.replay {
                                list.extend_from_slice(&inputs);
                            }
                        }
                        None => {
                            v.pending.entry(from).or_default().extend_from_slice(&inputs);
                        }
                    }
                }
            }
            ServerMsg::MapFile(part) => self.receive_map_part(part),
            ServerMsg::Results(r) => {
                if let Some(s) = &mut self.mp {
                    s.results = Some(r);
                }
                if matches!(self.scene, Scene::Spectate(_)) {
                    self.stop_audio();
                    self.scene = Scene::MpResults;
                }
            }
            ServerMsg::Rejected(why) | ServerMsg::Disconnected(why) => {
                let in_mp = matches!(self.scene, Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults | Scene::Spectate(_));
                self.toast(format!("multiplayer: {why}"));
                self.mp = None;
                self.picking_lobby_map = false;
                if in_mp {
                    self.stop_audio();
                    self.mp_status = why;
                    self.scene = Scene::MpMenu;
                }
            }
            ServerMsg::GuessRound { round, rounds, hash, from, seconds } => {
                if let Some(g) = &mut self.guessr {
                    g.start_round(round, rounds, hash, from, seconds);
                }
            }
            ServerMsg::GuessWrong { tries } => {
                if let Some(g) = &mut self.guessr {
                    g.tries = tries;
                }
                self.toast(if tries == 0 { "wrong, and that was your last guess".into() } else { format!("wrong! {tries} left") });
            }
            ServerMsg::GuessReveal { hash, got, last } => {
                if let Some(g) = &mut self.guessr {
                    g.reveal(&self.library, hash, got, last);
                }
                // Picking a guess: back to the lobby to see the answer.
                if self.picking_lobby_map && matches!(self.scene, Scene::Select) {
                    self.picking_lobby_map = false;
                    self.scene = Scene::Lobby;
                }
            }
            ServerMsg::Chat { from, text } => {
                let text = if self.cfg.chat_filter { net::censor(&text) } else { text };
                if from == net::GUESSR_NAME {
                    self.toast(text.clone());
                }
                if let Some(s) = &mut self.mp {
                    s.chat.push((from, text));
                    if s.chat.len() > 200 {
                        s.chat.remove(0);
                    }
                    s.chat_unread += 1;
                }
            }
            ServerMsg::Welcome { .. } | ServerMsg::Pong { .. } => {}
        }
    }

    /// A key press: the player's own hotkeys (`keys::Bindings`) turned into the keys the game
    /// knows them by, then `key_inner`.
    /// Asks whether to exit; asked again (Ctrl+Q or the close button twice), exits.
    fn ask_quit(&mut self) {
        if self.quit_asked {
            self.quit = true;
        } else {
            self.quit_asked = true;
        }
    }

    /// A key while "Exit taiko.rs?" is up: yes, no, or nothing. 1 and 2 answer like osu!'s.
    fn quit_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::KeyY | KeyCode::Digit1 | KeyCode::Numpad1 => self.quit = true,
            KeyCode::KeyQ if self.modifiers_ctrl => self.quit = true,
            KeyCode::Escape | KeyCode::KeyN | KeyCode::Backspace | KeyCode::Digit2 | KeyCode::Numpad2 => self.quit_asked = false,
            _ => {}
        }
    }

    /// The crash question is up: after the splash, over everything but the exit question.
    fn crash_shown(&self) -> bool {
        self.crash.is_some() && self.splash.is_none() && !self.quit_asked
    }

    /// A key while "taiko.rs crashed last time" is up: copy the report, or not.
    fn crash_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::KeyC | KeyCode::Digit1 | KeyCode::Numpad1 => {
                let Some(report) = self.crash.take() else { return };
                let text = if clipboard::set(&report) {
                    tr!("crash log copied: paste it in #bugs on Discord", "クラッシュログをコピーしました: Discordの#bugsに貼ってください", "лог краша скопирован: вставьте его в #bugs в Discord")
                } else {
                    tr!("couldn't copy it: send crash.log from the game's data folder", "コピーできませんでした: データフォルダのcrash.logを送ってください", "не удалось скопировать: пришлите crash.log из папки данных игры")
                };
                self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(6) });
            }
            KeyCode::Escape | KeyCode::Backspace | KeyCode::Digit2 | KeyCode::Numpad2 => self.crash = None,
            _ => {}
        }
    }

    /// "taiko.rs crashed last time": the report to copy and send in.
    fn draw_crash(&self, c: &mut Canvas) {
        if !self.crash_shown() {
            return;
        }
        let w = c.width;
        c.rect(0.0, 0.0, w, render::VIRTUAL_HEIGHT, rgba(0x000000, 0.6));
        let (bw, bh) = (380.0f32.min(w - 32.0), 132.0);
        let (x, y) = ((w - bw) / 2.0, (render::VIRTUAL_HEIGHT - bh) / 2.0);
        c.round_rect([x, y], [bw, bh], 10.0, rgba(0x1d1d27, 1.0), rgba(accent(), 1.0), 1.5);
        c.text_centered(w / 2.0, y + 18.0, 12.0, rgba(0xffffff, 1.0), tr!("taiko.rs crashed last time", "前回taiko.rsがクラッシュしました", "taiko.rs вылетел в прошлый раз"));
        let room = bw - 32.0;
        for (k, line) in [
            tr!("Copy the crash log and paste it in #bugs", "クラッシュログをコピーして", "Скопируйте лог краша и вставьте его"),
            tr!("on the taiko.rs Discord, so it gets fixed.", "Discordの#bugsに貼ってください。", "в #bugs в Discord taiko.rs, чтобы мы починили."),
        ]
        .iter()
        .enumerate()
        {
            let size = Canvas::fit_size(8.0, 6.5, room, line);
            c.text_centered(w / 2.0, y + 42.0 + 13.0 * k as f32, size, rgba(0xc8c8d8, 1.0), &Canvas::fit(size, room, line));
        }
        let gap = 10.0;
        let button = (bw - 36.0 - gap) / 2.0;
        let copy = tr!("1. Copy log  (Enter)", "1. コピー  (Enter)", "1. Копировать  (Enter)");
        let close = tr!("2. Close  (Esc)", "2. 閉じる  (Esc)", "2. Закрыть  (Esc)");
        c.button(x + 18.0, y + 88.0, button, 26.0, copy, 9.0, render::Hit::Key(KeyCode::KeyC), true);
        c.button(x + 18.0 + button + gap, y + 88.0, button, 26.0, close, 9.0, render::Hit::Key(KeyCode::Escape), false);
    }

    /// The exit question over everything.
    fn draw_quit(&self, c: &mut Canvas) {
        if !self.quit_asked {
            return;
        }
        let w = c.width;
        c.rect(0.0, 0.0, w, render::VIRTUAL_HEIGHT, rgba(0x000000, 0.6));
        let (bw, bh) = (300.0f32.min(w - 32.0), 100.0);
        let (x, y) = ((w - bw) / 2.0, (render::VIRTUAL_HEIGHT - bh) / 2.0);
        c.round_rect([x, y], [bw, bh], 10.0, rgba(0x1d1d27, 1.0), rgba(accent(), 1.0), 1.5);
        c.text_centered(w / 2.0, y + 18.0, 12.0, rgba(0xffffff, 1.0), "Exit taiko.rs?");
        let gap = 10.0;
        let button = (bw - 36.0 - gap) / 2.0;
        c.button(x + 18.0, y + 56.0, button, 26.0, "1. Exit  (Enter)", 9.0, render::Hit::Key(KeyCode::KeyY), true);
        c.button(x + 18.0 + button + gap, y + 56.0, button, 26.0, "2. Stay  (Esc)", 9.0, render::Hit::Key(KeyCode::KeyN), false);
    }

    fn key(&mut self, code: KeyCode, repeat: bool, text: Option<&str>, at: Instant, window: Option<&Window>) {
        if self.quit_asked {
            if !repeat {
                self.quit_key(code);
            }
            return;
        }
        if self.crash_shown() {
            if !repeat {
                self.crash_key(code);
            }
            return;
        }
        let pressed = keys::Chord { key: code, ctrl: self.modifiers_ctrl, shift: self.modifiers_shift, alt: self.modifiers_alt };
        match self.hotkey_place(code).map(|place| self.cfg.hotkeys.resolve(pressed, place)) {
            None | Some(keys::Resolved::Same) => self.key_inner(code, repeat, text, at, window),
            Some(keys::Resolved::Nothing) => {}
            Some(keys::Resolved::As(c)) => {
                let held = (self.modifiers_ctrl, self.modifiers_shift, self.modifiers_alt);
                (self.modifiers_ctrl, self.modifiers_shift, self.modifiers_alt) = (c.ctrl, c.shift, c.alt);
                self.key_inner(c.key, repeat, None, at, window);
                (self.modifiers_ctrl, self.modifiers_shift, self.modifiers_alt) = held;
            }
        }
    }

    /// Where hotkeys are pressed now, or None where every key is taken as it is: something being
    /// typed or bound, a list or the mod select open, a drum key in a song, a letter typed into
    /// a search.
    fn hotkey_place(&self, code: KeyCode) -> Option<keys::Place> {
        let busy = self.splash.is_some()
            || self.profile.is_some()
            || self.settings_ui.binding.is_some()
            || self.settings_ui.editing.is_some()
            || self.settings_ui.dropdown.is_some()
            || self.settings_ui.choices.is_some()
            || self.mod_select.is_some()
            || self.customization_tab().is_some();
        let plain = !self.modifiers_ctrl && !self.modifiers_alt;
        let settings_idle = matches!(self.scene, Scene::Settings { .. }) && self.settings_ui.search.is_empty() && !keys::starts_text(code);
        if busy || (plain && self.in_song() && self.cfg.keys.contains(&code)) || (plain && self.takes_text() && keys::types_text(code) && !settings_idle) {
            return None;
        }
        Some(match &self.scene {
            Scene::Select if self.on_menu() => keys::Place::Other,
            Scene::Select if self.select.picker.is_none() && self.select.menu.is_none() => keys::Place::Select,
            Scene::Play { game, .. } if game.paused.is_some() => keys::Place::Paused,
            Scene::Play { .. } => keys::Place::Play,
            Scene::Results { .. } => keys::Place::Results,
            _ => keys::Place::Other,
        })
    }

    fn key_inner(&mut self, code: KeyCode, repeat: bool, text: Option<&str>, at: Instant, window: Option<&Window>) {
        if self.quit_asked {
            if !repeat {
                self.quit_key(code);
            }
            return;
        }
        if self.crash_shown() {
            if !repeat {
                self.crash_key(code);
            }
            return;
        }
        self.refresh_play_offset();
        if let Some(s) = &mut self.splash {
            // Any key skips the splash and does nothing else.
            if !repeat {
                s.skip(at, self.audio.as_ref(), &format!("key {code:?}"));
            }
            return;
        }
        if let Some(slot) = self.guide_binding.filter(|_| self.guide.is_some()) {
            let modifier = matches!(
                code,
                KeyCode::ShiftLeft | KeyCode::ShiftRight | KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::AltLeft | KeyCode::AltRight | KeyCode::SuperLeft | KeyCode::SuperRight
            );
            if repeat || modifier {
                return;
            }
            self.guide_binding = None;
            if code != KeyCode::Escape && self.bind_drum_key(slot, code) {
                self.cfg.save();
            }
            return;
        }
        if self.guide.is_some() {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space | KeyCode::ArrowRight => self.guide_action(guide::Action::Next),
                KeyCode::ArrowLeft | KeyCode::Backspace => self.guide_action(guide::Action::Back),
                KeyCode::Escape => self.guide_action(guide::Action::Close),
                KeyCode::KeyC if self.guide == Some(guide::OFFSET_PAGE) => self.guide_action(guide::Action::Calibrate),
                KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 if self.guide == Some(0) => {
                    let slot = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4].iter().position(|&k| k == code);
                    self.guide_binding = slot;
                }
                _ => {}
            }
            return;
        }
        // Being typed or bound, Ctrl+O is just a key.
        let free = self.settings_ui.binding.is_none() && self.settings_ui.editing.is_none();
        if free && self.modifiers_alt && !self.modifiers_ctrl && matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
            self.volume.pick_next(if code == KeyCode::ArrowLeft { -1 } else { 1 }, Instant::now());
            return;
        }
        if free && code == KeyCode::KeyO && self.modifiers_ctrl && !self.modifiers_shift && !self.modifiers_alt {
            if !repeat {
                self.settings_anywhere();
            }
            return;
        }
        if let Some(view) = &mut self.changelog {
            if (!repeat || matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::PageUp | KeyCode::PageDown))
                && view.key(code) {
                self.changelog = None;
            }
            return;
        }
        // A profile is over everything: any of these closes it, the rest do nothing.
        if self.profile.is_some() {
            if matches!(code, KeyCode::Escape | KeyCode::Backspace | KeyCode::Enter | KeyCode::NumpadEnter) {
                self.profile = None;
            }
            return;
        }
        // Ctrl+V: the clipboard's text, typed into whatever takes text here.
        if self.modifiers_ctrl && code == KeyCode::KeyV && self.takes_text() {
            if !repeat {
                let text = clipboard::get().map(|t| clipboard::one_line(&t)).unwrap_or_default();
                if !text.is_empty() {
                    self.modifiers_ctrl = false;
                    self.key_inner(KeyCode::KeyV, false, Some(&text), at, window);
                    self.modifiers_ctrl = true;
                }
            }
            return;
        }
        // Ctrl+C: what's being typed, else what's picked.
        if self.modifiers_ctrl
            && code == KeyCode::KeyC
            && !repeat
            && let Some(text) = self.copy_text()
        {
            if clipboard::set(&text) {
                self.toast(format!("copied: {text}"));
            } else {
                self.toast("no clipboard to copy to".into());
            }
            return;
        }
        // The friends overlay: F9 anywhere outside songs; while open it takes the keyboard
        // (function keys still work).
        if code == KeyCode::F9 && !repeat {
            self.toggle_social();
            return;
        }
        if self.social.open {
            let function_key = matches!(
                code,
                KeyCode::F1 | KeyCode::F2 | KeyCode::F3 | KeyCode::F4 | KeyCode::F5 | KeyCode::F6 | KeyCode::F7 | KeyCode::F8 | KeyCode::F10 | KeyCode::F11 | KeyCode::F12
            );
            if !(function_key && self.handle_global_key(code, window)) {
                self.social_key(code, text);
            }
            return;
        }
        if self.customization_tab().is_some() {
            self.customization_key(code, text, repeat);
            return;
        }
        if self.mod_select.is_some() {
            if !repeat || matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight) {
                self.mod_select_key(code);
            }
            return;
        }
        // Key capture must see every key, before any shortcut can claim it.
        if self.settings_ui.dropdown.is_some() && !repeat {
            let nav = matches!(
                code,
                KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
                    | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space | KeyCode::Escape | KeyCode::Backspace | KeyCode::Tab
                    | KeyCode::KeyS | KeyCode::KeyA | KeyCode::KeyE | KeyCode::F2 | KeyCode::Insert | KeyCode::Delete
            );
            // Typing a size takes every key.
            let typing = self.settings_ui.dropdown.as_ref().is_some_and(|d| d.editing.is_some());
            if nav || typing {
                self.dropdown_key(code);
                return;
            }
        }
        if self.settings_ui.editing.is_some() && matches!(self.scene, Scene::Settings { .. }) {
            if !repeat || !matches!(code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape) {
                self.edit_key(code, text);
            }
            return;
        }
        if !repeat && self.in_song() && self.settings_ui.binding.is_none() && (self.modifiers_shift, code) == self.cfg.interface_key && !(self.cfg.keys.contains(&code) && !self.modifiers_shift) {
            self.cfg.interface_hidden = !self.cfg.interface_hidden;
            self.cfg.save();
            return;
        }
        if self.settings_ui.binding.is_some() && matches!(self.scene, Scene::Settings { .. }) {
            if !repeat {
                self.settings_key(code, window);
            }
            return;
        }
        if !repeat && !matches!(self.scene, Scene::MpMenu | Scene::Settings { .. }) && self.settings_ui.dropdown.is_none()
            && let Some(i) = self.cfg.preset_keys.iter().position(|&k| k == code) {
            self.cycle_preset(if i == 0 { -1 } else { 1 });
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
        // Any key closes the row menu; Esc does nothing else.
        if matches!(self.scene, Scene::Select) && self.select.menu.take().is_some() && code == KeyCode::Escape {
            return;
        }
        if let (Scene::Select, Some(cursor)) = (&self.scene, self.select.picker) {
            let n = self.select.picker_len();
            match code {
                KeyCode::ArrowUp => self.select.picker = Some(cursor.saturating_sub(1)),
                KeyCode::ArrowDown => self.select.picker = Some((cursor + 1).min(n - 1)),
                KeyCode::PageUp => self.select.picker = Some(cursor.saturating_sub(10)),
                KeyCode::PageDown => self.select.picker = Some((cursor + 10).min(n - 1)),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.pick_row(cursor),
                KeyCode::Escape | KeyCode::Backspace => self.select.picker = None,
                _ => {}
            }
            return;
        }
        if matches!(self.scene, Scene::VideoEdit(_)) {
            self.video_key(code, repeat, window);
            return;
        }
        if matches!(self.scene, Scene::Watch(_)) && code == KeyCode::Escape {
            if !repeat {
                self.stop_watching();
            }
            return;
        }
        if let Scene::Spectate(v) = &mut self.scene {
            let n = v.players.len().max(1);
            match code {
                KeyCode::ArrowLeft | KeyCode::ArrowRight if !repeat => {
                    let dir = if code == KeyCode::ArrowRight { 1 } else { n - 1 };
                    v.focus = Some(v.focus.map(|f| (f + dir) % n).unwrap_or(0));
                }
                KeyCode::KeyA | KeyCode::Tab if !repeat => v.focus = if v.focus.is_some() { None } else { Some(0) },
                KeyCode::Escape if !repeat => {
                    if let Some(s) = &self.mp {
                        s.client.send(net::ClientMsg::Spectate(false));
                    }
                    self.stop_audio();
                    self.scene = Scene::Lobby;
                }
                _ => {
                    if !repeat {
                        self.handle_global_key(code, window);
                    }
                }
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
            && !self.select.searching
        {
            self.scene = Scene::Lobby;
            return;
        }
        if self.picking_lobby_map && matches!(self.scene, Scene::Select) && !repeat {
            match code {
                KeyCode::Escape if self.select.filter.is_empty() && !self.select.searching => {
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
                if !repeat && game.replay.is_none() {
                    game.press(k, at, self.audio.as_ref(), self.play_offset);
                }
                return;
            }
            if game.replay.is_some() && matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
                let step = if self.modifiers_shift { 15_000.0 } else { 5_000.0 };
                let t = game.time + if code == KeyCode::ArrowRight { step } else { -step };
                game.seek(t, self.audio.as_ref(), self.play_offset);
                return;
            }
        }
        if repeat && !matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::Backspace | KeyCode::PageUp | KeyCode::PageDown) {
            return;
        }
        if self.handle_global_key(code, window) {
            return;
        }
        if self.on_menu() {
            if self.player.is_some() {
                self.player_key(code, text);
            } else {
                self.menu_key(code);
            }
            return;
        }
        match &mut self.scene {
            Scene::Select if self.modifiers_ctrl && code == KeyCode::KeyF => self.select.searching = true,
            // In the search box the keys that have a job here otherwise (speed's [ and ]) type.
            Scene::Select if self.select.searching && !self.modifiers_ctrl && matches!(code, KeyCode::BracketLeft | KeyCode::BracketRight) => {
                if let Some(text) = text {
                    self.select.type_text(text, &self.library);
                }
            }
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
            Scene::Select if self.modifiers_ctrl && self.modifiers_shift && code == KeyCode::KeyU => {
                if let Some(i) = self.select.selected() {
                    self.update_set(i);
                }
            }
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
            Scene::Select if self.library.is_empty() && self.scan.is_none() && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) && !cfg!(target_os = "android") => {
                self.browse_songs_folder();
            }
            Scene::Select if self.modifiers_ctrl && code == KeyCode::Backspace => {
                self.select.filter.clear();
                self.select.refilter(&self.library);
            }
            Scene::Select => match self.select.key(code, &self.library) {
                SelectAction::Play(i) if self.picking_lobby_map => self.pick_lobby_map(i),
                SelectAction::Play(i) => self.start_loading(i),
                SelectAction::Settings => self.open_settings(),
                SelectAction::Mods => self.open_mod_select(),
                SelectAction::Quit if self.cfg.main_menu => self.main_menu = Some(menu::Menu::new()),
                SelectAction::Quit => self.ask_quit(),
                SelectAction::None => {
                    if let Some(text) = text {
                        self.select.type_text(text, &self.library);
                    }
                }
            },
            Scene::Loading { .. } => {
                if code == KeyCode::Escape {
                    self.show_select();
                }
            }
            Scene::Settings { .. } => {
                if !self.settings_type(code, text) {
                    self.settings_key(code, window);
                }
            }
            Scene::Calibrate { .. } | Scene::CalibrateDone { .. } | Scene::LayoutEdit { .. } | Scene::Preview { .. } | Scene::Download => {}
            Scene::MpMenu | Scene::Lobby | Scene::MpWait { .. } | Scene::MpPlay { .. } | Scene::MpResults | Scene::Spectate(_) | Scene::Watch(_) | Scene::VideoEdit(_) => {}
            Scene::Play { game, entry, from } => match code {
                KeyCode::KeyO => self.apply_live_offset(),
                KeyCode::Tab => self.toggle_map_info(),
                KeyCode::Escape => game.pause(self.audio.as_ref()),
                KeyCode::Space => {
                    game.skip(self.audio.as_ref(), self.play_offset);
                }
                KeyCode::Backquote => {
                    let (e, f) = (*entry, *from);
                    self.start_loading_from(e, f);
                }
                KeyCode::KeyR if self.modifiers_ctrl => self.restart_map(),
                _ => {}
            },
            Scene::Results { results, .. } if matches!(code, KeyCode::ArrowDown | KeyCode::ArrowUp | KeyCode::PageDown | KeyCode::PageUp | KeyCode::Home | KeyCode::End) => {
                results.scroll(matches!(code, KeyCode::ArrowDown | KeyCode::PageDown | KeyCode::End));
            }
            Scene::Results { .. } if code == KeyCode::KeyR && self.modifiers_ctrl => self.restart_map(),
            Scene::Results { results, .. } if code == KeyCode::KeyV && results.replay.is_some() => {
                if let Some(file) = results.replay.clone() {
                    self.open_video_editor(&file);
                }
            }
            // A leaderboard score's results: Enter watches it too.
            Scene::Results { results, .. }
                if results.replay.is_some() && (code == KeyCode::KeyR || (results.opened.is_some() && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter))) =>
            {
                if let Some(file) = results.replay.clone() {
                    self.watch_replay(&file);
                }
            }
            // Back to song select, the music playing on.
            Scene::Results { results, .. } if results.opened.is_some() => {
                if matches!(code, KeyCode::Escape | KeyCode::Backspace) {
                    self.scene = Scene::Select;
                }
            }
            Scene::Results { entry, from, .. } => match code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let (e, f) = (*entry, *from);
                    self.start_loading_from(e, f);
                }
                KeyCode::Escape => {
                    let path = self.library[*entry].path.clone();
                    self.show_select();
                    self.select.select_path(&self.library, &path);
                }
                _ => {}
            },
        }
    }

    /// Advances the game to `now` and fills `canvas`. Returns a screenshot name if this frame
    /// should be captured.
    fn build_frame(&mut self, canvas: &mut Canvas, now: Instant, width: f32) -> Option<String> {
        // Here too, so a new choice (or a test's) shows from the next frame on.
        lang::set(self.cfg.language);
        render::set_legacy_menu(!self.cfg.main_menu);
        self.select.legacy = !self.cfg.main_menu;
        self.select.to_menu = self.mp.is_none() && !self.picking_lobby_map;
        self.select.covered = self.customization_tab().is_some();
        // A big library's first analysis waits while a song is being played.
        if let Some(info) = &self.mapinfo {
            info.hold(self.in_song());
        }
        self.refresh_play_offset();
        self.follow_map_skin();
        if self.window_resized_at.is_some_and(|t| now.saturating_duration_since(t) > Duration::from_secs(1)) {
            self.window_resized_at = None;
            self.cfg.save();
        }
        // Notes are drawn this far ahead: the player's visual offset and, with vsync, the time
        // until the frame is on screen.
        let visual_offset = self.cfg.visual_offset_ms + self.vsync_lead() + self.pace_shift_ms;
        // The interface toggle: what it hides, while it's off.
        let hidden = if self.cfg.interface_hidden { self.cfg.interface_hides } else { hud::Parts::NONE };
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
        render::set_accent(self.cfg.accent);
        game::set_hide_great(self.cfg.hide_great);
        game::set_hit_flash(self.cfg.hit_flash);
        // Screen changes fade in.
        let kind = std::mem::discriminant(&self.scene);
        if self.scene_kind != Some(kind) {
            self.scene_kind = Some(kind);
            self.scene_since = now;
        }
        self.frame_clip = None;
        #[cfg(target_os = "android")]
        self.update_keyboard();
        self.poll_scan();
        self.poll_folder_pick();
        self.poll_data_move();
        {
            // The Download tab previews remote maps through the same preview.
            let browsing = matches!(self.scene, Scene::Download);
            if browsing != self.preview_remote {
                self.preview_remote = browsing;
                self.preview.forget();
            }
            let in_select = matches!(self.scene, Scene::Select | Scene::Preview { .. } | Scene::Download);
            let in_menu = in_select
                || matches!(&self.scene, Scene::Results { results, .. } if results.opened.is_some() && results.music.is_none())
                || matches!(&self.scene, Scene::Settings { prev } if matches!(**prev, Scene::Select | Scene::Lobby | Scene::Preview { .. }))
                || matches!(self.scene, Scene::MpMenu | Scene::Lobby | Scene::Download);
            let (entries, selected) = if browsing { (&self.browse.remote, self.browse.shown) } else { (&self.library, self.preview_target()) };
            let rate = self.preview_rate();
            self.preview.kiai_effects = self.cfg.kiai_effects;
            self.preview.autoplay = self.cfg.autoplay;
            self.preview.player = self.player.is_some();
            self.preview.menu = self.on_menu();
            self.preview.from_top &= self.preview.menu;
            self.preview.pitch = self.player.as_ref().map_or(1.0, |p| p.pitch());
            // The splash has its own sound: the preview music waits for it.
            let in_menu = in_menu && self.splash.is_none();
            self.preview.update(now, entries, selected, self.audio.as_ref(), in_menu, in_select, self.play_offset, rate);
            self.stars.poll();
            self.stars.set_mods(self.cfg.mods, self.cfg.scroll_speed);
            let play_rate = self.play_speed().0;
            if self.cfg.menu_notes && self.on_menu() {
                let playing = self.preview_target();
                self.menu_notes.update(playing, &self.library, &mut self.stars, play_rate);
            }
            if let (Scene::Preview { .. }, Some(i)) = (&self.scene, selected) {
                self.stars.request(i, &self.library[i].path, play_rate);
            }
            if matches!(self.scene, Scene::Select) {
                self.poll_set_status(now);
                for i in self.select.selected_set() {
                    self.stars.request(i, &self.library[i].path, play_rate);
                }
                let (stars, rate) = (&self.stars, play_rate);
                self.select.sort_selected_set(&|i| stars.get(i, rate));
                self.select.remember_selection(&self.library);
            }
            let has_bg = self.preview.has_background;
            self.bg_brightness = match self.scene {
                _ if !has_bg => 0.0,
                _ if self.dims_background() => 1.0 - self.cfg.bg_dim,
                _ if self.on_menu() => if self.player.is_some() { 0.25 } else { 0.5 },
                Scene::Calibrate { .. } | Scene::CalibrateDone { .. } | Scene::LayoutEdit { .. } => 0.0,
                _ => 0.35,
            };

        }
        self.poll_multiplayer();
        self.track_play();
        if self.cfg.check_updates {
            self.updater.check_every(UPDATE_CHECK_EVERY);
        }
        self.poll_downloads();
        self.poll_score_link();
        self.poll_set_check(now);
        self.poll_sorting();
        self.update_presence();
        if let Some(ex) = &self.export {
            match ex.encoder.done.try_recv() {
                Ok(Ok(path)) => {
                    self.export = None;
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.video_status = format!("saved \"{name}\" in the videos folder");
                    self.last_video = Some(path);
                    self.toast("video saved".into());
                }
                Ok(Err(e)) => {
                    self.export = None;
                    self.video_status = format!("export failed: {e}");
                }
                Err(_) => {}
            }
        }
        self.browse.poll(now);
        let mut next: Option<Scene> = None;
        let mut capture: Option<String> = None;
        if std::mem::take(&mut self.screenshot_due) {
            capture = Some(SCREENSHOT.to_string());
        }
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
                    debug::Action::RightDown(x, y) | debug::Action::RightMove(x, y) => {
                        let k = self.window_h as f64 / render::VIRTUAL_HEIGHT as f64;
                        if matches!(action, debug::Action::RightDown(..)) {
                            self.mouse = (x as f64 * k, y as f64 * k);
                            self.right_button(true);
                        } else {
                            self.mouse_moved(x as f64 * k, y as f64 * k);
                        }
                    }
                    debug::Action::RightUp => self.right_button(false),
                    debug::Action::LeftDown(x, y) => {
                        let k = self.window_h as f64 / render::VIRTUAL_HEIGHT as f64;
                        self.mouse_moved(x as f64 * k, y as f64 * k);
                        self.mouse_button(true, None);
                    }
                    debug::Action::LeftMove(x, y) => {
                        let k = self.window_h as f64 / render::VIRTUAL_HEIGHT as f64;
                        self.mouse_moved(x as f64 * k, y as f64 * k);
                    }
                    debug::Action::LeftUp => self.mouse_button(false, None),
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
            if let Some(name) = screen
                && let Some(pos) = self.pending_screen_shots.iter().position(|&s| s == name) {
                self.pending_screen_shots.remove(pos);
                capture = Some(format!("{name}.png"));
                if name == "results" && self.exit_when_done {
                    self.quit = true;
                }
            }
        }
        if !matches!(self.scene, Scene::Select | Scene::Lobby) {
            self.mod_select = None;
        }
        // A song or a replay opened by any road leaves the menu behind.
        if matches!(self.scene, Scene::Loading { .. } | Scene::Play { .. } | Scene::Results { .. } | Scene::Preview { .. } | Scene::Watch(_) | Scene::VideoEdit(_)) {
            self.main_menu = None;
        }
        // Back from calibrating: on with the guide.
        if self.guide_after.is_some() && !matches!(self.scene, Scene::Calibrate { .. } | Scene::CalibrateDone { .. }) {
            self.guide = self.guide_after.take();
        }
        if !self.on_menu() {
            if let Some(m) = &mut self.main_menu {
                m.cancel_leaving();
            }
            self.close_player();
            if self.music_paused {
                self.set_music_paused(false);
            }
        }
        if self.preview.take_finished() {
            match self.player.as_ref().map(|p| p.list.repeat) {
                Some(playlist::Repeat::One) => {
                    self.preview.seek_music(0.0, self.audio.as_ref());
                }
                Some(_) => self.music_step(1),
                None if self.on_menu() => {
                    self.music_step(1);
                    self.preview.from_top = true;
                }
                None => {}
            }
        }
        if let Some(p) = self.player.as_ref().filter(|p| p.library_len != self.library.len()) {
            // The library changed under the list: same song on, if it's still there.
            let entry = p.list.entry();
            self.player = Some(Box::new(player::Player::new(&self.library, entry)));
        }
        self.poll_media();
        if let Some(item) = self.main_menu.as_mut().and_then(menu::Menu::ready) {
            self.menu_open(item);
        }
        let on_menu = self.on_menu();
        if !matches!(self.scene, Scene::Select) {
            self.select.menu = None;
            self.list_drag = None;
            self.select.cancel_drag();
        }
        // Drawn again below if still watching a replay (after scripted input, which must see the
        // last frame's bar like real input does).
        self.replay_bar = None;
        match &mut self.scene {
            Scene::Select if on_menu => {
                self.board_rect = None;
                self.leaderboard.clear();
                let shown = self.player.as_ref().and_then(|p| p.list.entry()).or(self.select.selected());
                let song = shown.and_then(|i| self.library.get(i)).map(Entry::label);
                let k = |i: usize| config::key_name(self.cfg.keys[i]);
                let (a, b, d, e) = (k(0), k(3), k(1), k(2));
                let hint = if cfg!(target_os = "android") {
                    tr!("tap to open", "タップで開く", "нажми, чтобы открыть").to_string()
                } else {
                    tr!(
                        "wheel or arrows: move   click or Enter: open   drum: {a} {b} move, {d} {e} open",
                        "ホイールか矢印: 移動   クリックかEnter: 開く   太鼓: {a} {b} 移動、{d} {e} 開く",
                        "колесо или стрелки: листать   клик или Enter: открыть   барабан: {a} {b} листать, {d} {e} открыть";
                    )
                };
                let paused = self.music_paused;
                if let Some(p) = &mut self.player {
                    let now = Instant::now();
                    let engine = self.audio.as_ref();
                    let sounding = !paused && self.preview.sounding(engine, now, &mut p.samples);
                    let hint = tr!("{a} {b} song   {d} {e} pause   L love   Tab folders", "{a} {b} 曲   {d} {e} ポーズ   L お気に入り   Tab フォルダ", "{a} {b} трек   {d} {e} пауза   L любимое   Tab папки";);
                    let volume = self.cfg.music_volume;
                    let frame = player::Frame {
                        song: song.as_deref().unwrap_or_default(),
                        hint: &hint,
                        paused,
                        times: self.preview.music_times(engine, now),
                        pictured: self.preview.has_background,
                        sounding,
                        rate: engine.map_or(48_000, |a| a.rate),
                        volume,
                    };
                    p.draw(canvas, &frame);
                } else if let Some(m) = &mut self.main_menu {
                    let kiai = self.cfg.kiai_effects && !paused && self.preview.kiai();
                    let shows = self.cfg.menu_notes && render::animations();
                    let ghosts = self.preview.map_time(self.menu_notes.entry()).filter(|_| shows).and_then(|t| self.menu_notes.view(t));
                    let frame = menu::Frame { beat: self.preview.beat(), kiai, ghosts, song: song.as_deref(), hint: &hint, maps: self.library.len(), paused };
                    m.draw(canvas, &frame);
                }
            }
            Scene::Select => {
                // DT / HT show on the Mods button instead.
                self.select.rate_label = if (self.cfg.rate - 1.0).abs() > 1e-6 && self.cfg.mods.rate_override().is_none() {
                    format!("  |  {:.2}x{}", self.cfg.rate, if self.cfg.rate_keep_pitch { "" } else { " (pitch)" })
                } else {
                    String::new()
                };
                let (stars, rate, plays) = (&self.stars, self.play_speed().0, &self.scores);
                self.select.scrubbing = (self.right_scroll || self.scrollbar_drag).then(|| self.mouse_virtual().1);
                self.select.list_share = self.cfg.song_list_share;
                self.select.draw(canvas, &self.library, dt, &self.status, &|i| stars.get(i, rate), &|e| scores::best_grade(plays, e.hash()));
                let lw = Select::list_width(canvas.width, self.cfg.song_list_share);
                let x = 24.0 + lw + 12.0;
                let pw = canvas.width - x - 24.0;
                self.board_rect = None;
                self.preview.size = self.cfg.song_preview_size;
                let selected_stars = self.select.selected().and_then(|i| self.stars.get(i, rate));
                self.preview.potential = match self.select.selected() {
                    Some(i) => self.potentials.get(&self.library[i].path, rate, self.cfg.mods, self.cfg.scroll_speed),
                    None => None,
                };
                self.leaderboard.clear();
                if pw > 160.0 {
                    let status = self.select.selected().and_then(|i| self.map_status(i));
                    // No taller than leaves the leaderboard three rows (its tabs, rows and gaps).
                    let panel_h = 436.0 - 54.0 - 8.0 - (16.0 + 3.0 * 18.0 + 14.0);
                    if let Some(clip) = self.preview.draw_panel(canvas, &self.library, x, 54.0, pw, panel_h, self.cfg.scroll_speed, selected_stars, Some(("Play from", "Shift+Enter")), status) {
                        self.frame_clip = Some(clip);
                    }
                    if let Some(i) = self.select.selected() {
                        let top = self.preview.panel_bottom + 8.0;
                        let fits = ((436.0 - top - 16.0) / 18.0).floor().max(0.0) as usize;
                        if fits > 0 {
                            let hash = self.library[i].hash();
                            if self.board_for != (hash, self.board_online) {
                                self.board_for = (hash, self.board_online);
                                self.board_scroll = 0;
                            }
                            self.board_rect = Some([x, top + 12.0, pw, fits as f32 * 18.0]);
                            if self.board_online {
                                let board = self.online_board(hash).cloned();
                                let total = board.as_ref().and_then(|b| b.as_ref().ok()).map_or(0, Vec::len);
                                self.board_max = total.saturating_sub(fits);
                                self.board_scroll = self.board_scroll.min(self.board_max);
                                draw_online_leaderboard(canvas, [x, top], pw, board.as_ref(), &self.cfg.online_user, fits, self.board_scroll);
                                self.online_leaderboard = match board {
                                    Some(Ok(rows)) => rows.into_iter().skip(self.board_scroll).take(fits).collect(),
                                    _ => Vec::new(),
                                };
                            } else {
                                let all = scores::top(&self.scores, hash, usize::MAX);
                                self.board_max = all.len().saturating_sub(fits);
                                self.board_scroll = self.board_scroll.min(self.board_max);
                                let rows: Vec<&scores::ScoreRow> = all.iter().skip(self.board_scroll).take(fits).copied().collect();
                                self.leaderboard = rows.iter().map(|r| r.file.clone()).collect();
                                draw_leaderboard(canvas, x, top, pw, &rows, self.board_scroll, all.len());
                            }
                        }
                        // The panel's bottom edge: dragged up, the preview shrinks and the
                        // leaderboard gets more rows.
                        let dragging = matches!(self.split_drag, Some((render::Hit::SplitPreview, ..)));
                        draw_divider(canvas, [x, self.preview.panel_bottom + 1.0, pw, 6.0], false, dragging, render::Hit::SplitPreview);
                    }
                    // The song list's right edge.
                    let dragging = matches!(self.split_drag, Some((render::Hit::SplitList, ..)));
                    draw_divider(canvas, [24.0 + lw + 2.0, 54.0, 8.0, 382.0], true, dragging, render::Hit::SplitList);
                }
                let guessing = self.mp.as_ref().is_some_and(|s| s.mapguessr);
                if let Some(g) = self.guessr.as_mut().filter(|_| self.picking_lobby_map && guessing) {
                    mapguessr::draw_picking(canvas, g, self.cfg.scroll_speed, Instant::now());
                } else if self.picking_lobby_map {
                    let w = canvas.width;
                    canvas.round_rect([24.0, 438.0], [w - 48.0, 34.0], 8.0, rgba(themed(0x2b1a17, 0x15151d, 0.84), 1.0), rgba(accent(), 1.0), 1.5);
                    canvas.text_centered(w / 2.0, 450.0, 10.0, rgba(0xffffff, 1.0), &Canvas::fit(10.0, w - 64.0, "Picking the lobby map: Enter to choose, Esc to cancel"));
                }
                self.select.draw_popups(canvas, &self.library);
            }
            Scene::Loading { rx, label, entry, from } => {
                canvas.text_centered(canvas.width / 2.0, 200.0, 14.0, rgba(0xffffff, 1.0), "Loading");
                canvas.text_centered(canvas.width / 2.0, 226.0, 10.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(10.0, canvas.width - 48.0, label));
                match rx.try_recv() {
                    Ok(Ok((map, track))) => {
                        let mods = std::mem::take(&mut self.pending_mods);
                        // Random: the replay's colouring, else a new one each play.
                        let seed = match &self.pending_replay {
                            Some(r) => r.seed,
                            None if mods.has(mods::Mod::Random) => fresh_seed(),
                            None => 0,
                        };
                        let half = self.pending_replay.as_ref().map_or(Default::default(), |r| r.half_rule());
                        let mut game = Game::with_half(map, track, self.audio.as_ref(), mods, seed, half);
                        game.scroll_speed = self.cfg.scroll_speed;
                        game.live_pp = pp::Live::start(self.library[*entry].path.clone(), game.rate, mods, game.scroll_speed, game.seed, half);
                        if let Some(t) = *from {
                            game.start_from(t, self.audio.as_ref(), self.play_offset);
                        }
                        match self.pending_replay.take() {
                            Some(r) => {
                                game.mash = r.mash();
                                game.scroll_speed = r.scroll_speed;
                                game.replay_of = Some(r.player.clone());
                                game.banner = Some(format!("REPLAY  -  {}", r.player));
                                game.replay = Some((r.inputs, 0));
                            }
                            // Full plays (not autoplay, not from a start point) are kept.
                            None => {
                                game.record = (!self.auto || std::env::var_os("TAIKO_RECORD_AUTO").is_some()) && from.is_none() && !mods.unranked();
                                if mods.has(mods::Mod::Autoplay) {
                                    game.banner = Some("AUTOPLAY".into());
                                }
                            }
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
                let done = game.update(now, self.audio.as_ref(), self.play_offset);
                if (self.auto || game.mods.has(mods::Mod::Autoplay)) && game.replay.is_none() {
                    game.autoplay_style = self.cfg.autoplay;
                    game.autoplay(self.audio.as_ref());
                }
                if let Some(sm) = &mut self.smooth {
                    if game.paused.is_some() {
                        // A frozen clock isn't jitter; restart the frame-to-frame comparison on resume.
                        sm.break_sequence();
                    } else {
                        let wall = now.duration_since(self.started).as_secs_f64() * 1000.0;
                        let shown = self.frame_acquired.unwrap_or(now).duration_since(self.started).as_secs_f64() * 1000.0;
                        sm.sample(wall, game.raw_time, game.last_audio_ms, shown, game.raw_time + visual_offset);
                    }
                }
                // Offset advice is for live play (and its line would sit on the replay timeline).
                game.show_offset_hint = self.cfg.show_offset_hint && self.cfg.show_map_info && game.replay.is_none();
                game.offset_key = self.cfg.hotkeys.get(keys::Hotkey::ApplyOffset).map(keys::Chord::name);
                game.kiai_effects = self.cfg.kiai_effects;
                game.hidden = hidden;
                game.ur_scaled_hint = self.cfg.offset_hint_ur_scaled;
                game.set_visual_offset(visual_offset);
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, self.bg_brightness > 0.0, |c| {
                    game.draw_playfield(c, game.scroll_speed);
                });
                game.draw_hud(canvas);
                if game.replay.is_none() && game.paused.is_none() && !hidden.has(hud::Part::Spectators) {
                    social::draw_spectators(canvas, &self.social.spectators());
                }
                if game.replay.is_some() && game.paused.is_none() {
                    self.replay_bar = Some(draw_replay_bar(canvas, game, self.replay_scrub.is_some()));
                }
                if let Some(plan) = &mut self.shots
                    && plan.times.last().is_some_and(|&t| game.time >= t) {
                    let t = plan.times.pop().unwrap();
                    capture = Some(format!("play-{t:.0}.png"));
                }
                if done {
                    let title = results_title(&game.map.meta, *from);
                    let failed = game.failed();
                    if game.failed_early() {
                        if let Some(a) = &self.audio {
                            a.stop();
                        }
                        self.preview.invalidate_audio();
                    }
                    let mut results = Results::new(title, game.stats.clone(), game.windows);
                    results.set_mods(game.mods, game.rate);
                    results.watched = game.replay_of.clone();
                    results.failed = failed;
                    // The song plays on under the results (not after a fail that stopped it),
                    // from the preview point again once it ends.
                    if !game.failed_early() && self.audio.is_some() {
                        let preview = game.map.meta.preview_time;
                        let again = if preview >= 0.0 { game.track_at(preview) } else { game.track_at(game.end_time()) * 0.4 };
                        results.music = Some((game.track.clone(), again.max(0.0)));
                    }
                    let e = &self.library[*entry];
                    let play = pp::Play {
                        great: game.stats.great,
                        ok: game.stats.ok,
                        miss: game.stats.miss,
                        max_combo: game.stats.max_combo,
                        rate: game.rate,
                        mods: game.mods,
                        scroll_speed: game.scroll_speed,
                        seed: game.seed,
                        half: game.half,
                    };
                    results.no_pp = if game.mods.unranked() {
                        Some("Autoplay and Relax get no pp")
                    } else if failed {
                        Some("failed plays get no pp")
                    } else if from.is_some() {
                        Some("plays from a start point get no pp")
                    } else if map_gives_pp(&self.library[*entry], &self.set_status, &self.db_status, &self.select.server_loved) == Some(false) {
                        Some("only ranked and loved maps give pp")
                    } else {
                        results.job = Some(results_job(e.path.clone(), play, None, None));
                        None
                    };
                    // Failed plays aren't kept, like osu!.
                    if game.record && !failed {
                        let e = &self.library[*entry];
                        let st = &game.stats;
                        let replay = scores::Replay {
                            version: scores::new_version(),
                            map_hash: e.hash(),
                            map_path: e.path.to_string_lossy().into_owned(),
                            map_label: e.label(),
                            player: self.cfg.mp_name.clone(),
                            date: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                            rate: game.rate,
                            keep_pitch: self.cfg.rate_keep_pitch,
                            anti_mash: true,
                            mods: game.mods.list(),
                            scroll_speed: game.scroll_speed,
                            score: st.score,
                            accuracy: st.accuracy(),
                            max_combo: st.max_combo,
                            great: st.great,
                            ok: st.ok,
                            miss: st.miss,
                            ur: st.unstable_rate(),
                            inputs: std::mem::take(&mut game.inputs),
                            seed: game.seed,
                        };
                        let pp = pp::play_pp(&e.path, &replay.pp_play());
                        results.pp = pp.filter(|_| results.no_pp.is_none());
                        if pp.is_none() && results.no_pp.is_none() {
                            results.no_pp = Some("the map couldn't be rated");
                        }
                        // Whole plays go to the online leaderboards too, when logged in.
                        if from.is_none() && !self.cfg.online_token.is_empty() && !game.mods.unranked() {
                            let m = &game.map.meta;
                            let play = online::ScoreSubmit {
                                map: net::MapInfo {
                                    hash: replay.map_hash,
                                    artist: m.artist.clone(),
                                    title: m.title.clone(),
                                    version: m.version.clone(),
                                    rate: replay.rate,
                                    from: None,
                                    set_id: m.set_id,
                                    mods: replay.mods.clone(),
                                    multiverse: None,
                                    shapeshift: None,
                                    seed: (replay.seed != 0).then_some(replay.seed),
                                },
                                score: replay.score,
                                accuracy: replay.accuracy,
                                max_combo: replay.max_combo,
                                great: replay.great,
                                ok: replay.ok,
                                miss: replay.miss,
                                pp,
                                date: replay.date,
                                replay_version: replay.version,
                                inputs: replay.inputs.clone(),
                                stars: stars::star_rating_played(&e.path, replay.rate, game.mods, game.scroll_speed, game.seed, replay.half_rule()),
                                pp_version: online::PP_VERSION,
                                scroll_speed: Some(replay.scroll_speed),
                                play_ms: Some(game.played_ms as u64),
                                md5: std::fs::read(&e.path).ok().map(|b| download::md5_hex(&b)),
                                timing: {
                                    let (latency_ms, reported_latency_ms) = self.audio.as_ref().map(|a| a.latencies_ms()).unwrap_or_default();
                                    Some(online::PlayTiming {
                                        os: std::env::consts::OS.into(),
                                        present: self.present.name().into(),
                                        offset_ms: self.play_offset,
                                        visual_lead_ms: visual_offset - self.pace_shift_ms,
                                        latency_ms,
                                        reported_latency_ms,
                                    })
                                },
                            };
                            self.submits.push(submit_score(self.cfg.online_server.clone(), self.cfg.online_token.clone(), play));
                            // Counted with the play itself.
                            self.playing = None;
                            results.total_due = true;
                            self.online_boards.remove(&replay.map_hash);
                        }
                        match scores::save(&replay, pp) {
                            Ok(row) => {
                                results.replay = Some(row.file.clone());
                                self.scores.push(row);
                                let all = scores::top(&self.scores, replay.map_hash, usize::MAX);
                                let rank = all.iter().position(|r| r.date == replay.date && r.score == replay.score).map(|p| p + 1);
                                results.rank = rank.map(|r| (r, all.len()));
                            }
                            Err(e) => eprintln!("couldn't save the replay: {e}"),
                        }
                    }
                    next = Some(Scene::Results { results, entry: *entry, from: *from });
                }
            }
            Scene::Results { results, .. } => {
                results.poll();
                results.animate(canvas.dt_ms);
                // Back to the preview point when the song ends, like osu!.
                if let (Some((track, again)), Some(a)) = (&results.music, &self.audio) {
                    let end = track.frames() as f64 / a.rate as f64 * 1000.0;
                    if a.position_ms(now).is_some_and(|p| p > end - 30.0) {
                        a.play(track.clone(), *again);
                    }
                }
                results.draw(canvas);
            }
            Scene::Download => {
                let installed = self.installed_sets.take().unwrap_or_default();
                let dir = self.songs_dir.display().to_string();
                let lw = Select::list_width(canvas.width, self.cfg.song_list_share);
                self.preview.size = self.cfg.song_preview_size;
                self.browse.draw(canvas, &self.downloads, &installed, &dir, lw);
                self.installed_sets = Some(installed);
                // The selected set's preview, like song select's.
                let x = 24.0 + lw + 12.0;
                let pw = canvas.width - x - 24.0;
                let stars = self.browse.selected_diff().map(|d| d.stars);
                if pw > 160.0 && self.browse.selected().is_some() {
                    match self.preview.draw_panel(canvas, &self.browse.remote, x, 54.0, pw, 380.0, self.cfg.scroll_speed, stars, None, None) {
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
                        let stars = self.stars.get(i, self.play_speed().0);
                        draw_preview_chrome(canvas, &self.cfg, e, stars, lobby, &size_label);
                        let (x, y, w) = (24.0, 394.0, canvas.width - 48.0);
                        canvas.round_rect([x, y], [w, 78.0], 8.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
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
                multiplayer::draw_menu(canvas, &self.mp_menu, &self.cfg.mp_name, self.cfg.mp_port, &self.cfg.mp_address, &self.cfg.online_server, &self.mp_status, self.mp_connecting.is_some());
            }
            Scene::Lobby => match &self.mp {
                Some(s) if s.mapguessr => {
                    if let Some(g) = &mut self.guessr {
                        mapguessr::draw_lobby(canvas, s, g, self.cfg.scroll_speed, Instant::now());
                    }
                }
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
                if start.is_some_and(|at| now >= at)
                    && let Some((map, track)) = loaded.take() {
                    let seed = self.mp.as_ref().and_then(|s| s.map.as_ref()).and_then(|m| m.seed).unwrap_or(0);
                    let mut game = Game::with_seed(map, track, self.audio.as_ref(), self.pending_mods, seed);
                    game.scroll_speed = self.cfg.scroll_speed;
                    if let Some(i) = self.mp.as_ref().and_then(|s| s.map_entry) {
                        game.live_pp = pp::Live::start(self.library[i].path.clone(), game.rate, self.pending_mods, game.scroll_speed, game.seed, Default::default());
                    }
                    if let Some(m) = self.mp.as_ref().and_then(|s| s.map.as_ref()) {
                        use_mod_settings(&mut game, m);
                    }
                    if let Some(t) = *from {
                        game.start_from(t, self.audio.as_ref(), self.play_offset);
                    }
                    game.record = true;
                    if let Some(s) = &mut self.mp {
                        s.inputs_sent = 0;
                    }
                    next = Some(Scene::MpPlay { game: Box::new(game), quit_armed: None });
                }
                if let Some(s) = &self.mp {
                    multiplayer::draw_waiting(canvas, s, *start, loaded.is_some(), now);
                }
            }
            Scene::MpPlay { game, .. } => {
                let done = game.update(now, self.audio.as_ref(), self.play_offset);
                if self.auto || game.mods.has(mods::Mod::Autoplay) {
                    game.autoplay_style = self.cfg.autoplay;
                    game.autoplay(self.audio.as_ref());
                }
                game.show_offset_hint = self.cfg.show_offset_hint && self.cfg.show_map_info;
                game.offset_key = self.cfg.hotkeys.get(keys::Hotkey::ApplyOffset).map(keys::Chord::name);
                game.kiai_effects = self.cfg.kiai_effects;
                game.hidden = hidden;
                game.ur_scaled_hint = self.cfg.offset_hint_ur_scaled;
                game.set_visual_offset(visual_offset);
                let live = multiplayer::live_from(game);
                let mp = &self.mp;
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, self.bg_brightness > 0.0, |c| {
                    game.draw_playfield(c, game.scroll_speed);
                });
                game.draw_hud(canvas);
                if let Some(s) = mp.as_ref().filter(|_| self.cfg.mp_scoreboard && !hidden.has(hud::Part::Scoreboard)) {
                    multiplayer::draw_scoreboard(canvas, s, &live);
                }
                if let Some(s) = &mut self.mp {
                    if s.last_live.elapsed() >= Duration::from_millis(100) {
                        s.last_live = now;
                        s.client.send(net::ClientMsg::Live(live));
                        if game.inputs.len() > s.inputs_sent {
                            let inputs = game.inputs[s.inputs_sent..].to_vec();
                            s.inputs_sent = game.inputs.len();
                            s.client.send(net::ClientMsg::Inputs { inputs, anti_mash: true });
                        }
                    }
                    if done {
                        // Failing (SD / PF) ends the match for you like forfeiting does.
                        let failed = game.failed_early();
                        s.client.send(net::ClientMsg::Finished(multiplayer::final_from(&game.stats, failed)));
                        if failed
                            && let Some(a) = &self.audio {
                            a.stop();
                        }
                        next = Some(Scene::MpResults);
                    }
                }
            }
            Scene::Spectate(v) => {
                if let Some(rx) = &v.rx {
                    match rx.try_recv() {
                        Ok(Ok(l)) => {
                            v.loaded = Some(l);
                            v.rx = None;
                        }
                        Ok(Err(e)) => {
                            self.status = format!("can't spectate: {e}");
                            next = Some(Scene::Lobby);
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                        Err(mpsc::TryRecvError::Disconnected) => next = Some(Scene::Lobby),
                    }
                }
                // Start once the map is ready and the delay has passed: the music where the
                // players were SPECTATE_DELAY ago, every player's game snapping to it.
                if v.games.is_empty() && now >= v.start + SPECTATE_DELAY
                    && let (Some((map, track)), Some(audio)) = (v.loaded.take(), self.audio.as_ref()) {
                    let probe = Game::with_mods(map.clone(), track.clone(), None, v.mods);
                    let first_ms = match v.from {
                        Some(t) => (probe.track_at(t) - 2000.0).max(probe.track_at(probe.time)) + self.play_offset,
                        None => probe.track_at(probe.time),
                    };
                    let behind = (now - v.start - SPECTATE_DELAY).as_secs_f64() * 1000.0;
                    audio.play(track.clone(), first_ms + behind);
                    for (k, (id, name)) in v.players.iter().enumerate() {
                        // The lobby's speed mods, and the player's own others.
                        let own = self.mp.as_ref().and_then(|s| s.players.iter().find(|p| p.id == *id)).map(|p| Mods::from_list(&p.mods).free()).unwrap_or(Mods::NONE);
                        let mut g = Game::with_seed(map.clone(), track.clone(), None, v.mods.shared().plus(own), v.map.seed.unwrap_or(0));
                        use_mod_settings(&mut g, &v.map);
                        if let Some(t) = v.from {
                            g.skip_to(t);
                        }
                        let inputs = v.pending.remove(id).unwrap_or_default();
                        g.replay = Some((inputs, 0));
                        g.banner = Some(format!("SPECTATING  -  {name}  ({}/{})", k + 1, v.players.len()));
                        g.watched = true;
                        v.games.push((*id, Box::new(g)));
                    }
                    // Players may have skipped the intro before the view was ready.
                    v.follow_skip(self.audio.as_ref(), self.play_offset);
                }
                if v.games.is_empty() {
                    canvas.text_centered(canvas.width / 2.0, 220.0, 12.0, rgba(0xffffff, 1.0), "joining the match...");
                }
                let scroll = self.cfg.scroll_speed;
                for (k, (_, g)) in v.games.iter_mut().enumerate() {
                    // Only the player watched (or the first one) is heard.
                    g.mute_hits = v.focus.unwrap_or(0) != k;
                    g.kiai_effects = self.cfg.kiai_effects;
                    g.hidden = hidden;
                    g.update(now, self.audio.as_ref(), self.play_offset);
                }
                match v.focus {
                    Some(k) if k < v.games.len() => {
                        let g = &mut v.games[k].1;
                        draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, self.bg_brightness > 0.0, |c| {
                            g.draw_playfield(c, scroll);
                        });
                        g.draw_hud(canvas);
                        canvas.text_centered(canvas.width / 2.0, 470.0, 7.0, rgba(0x9a9ab0, 1.0), "Left/Right: other player   A: everyone   Esc: leave");
                    }
                    _ if !v.games.is_empty() => {
                        let dt = v.zones_at.map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f32());
                        v.zones_at = Some(now);
                        draw_spectate_all(canvas, &mut v.games, &v.players, scroll, &mut v.zones, dt);
                    }
                    _ => {}
                }
            }
            Scene::Watch(v) => {
                let scroll = self.cfg.scroll_speed;
                match &mut v.game {
                    Some(g) => {
                        g.kiai_effects = self.cfg.kiai_effects;
                        g.hidden = hidden;
                        g.update(now, self.audio.as_ref(), self.play_offset);
                        draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, self.bg_brightness > 0.0, |c| {
                            g.draw_playfield(c, scroll);
                        });
                        g.draw_hud(canvas);
                        if v.held {
                            canvas.text_centered(canvas.width / 2.0, 300.0, 9.0, rgba(0x9a9ab0, 1.0), &format!("{} paused", v.player));
                        }
                    }
                    None => canvas.text_centered(canvas.width / 2.0, 220.0, 12.0, rgba(0xffffff, 1.0), &v.status()),
                }
                canvas.text_centered(canvas.width / 2.0, 470.0, 7.0, rgba(0x9a9ab0, 1.0), "Esc: stop watching   F9: friends");
            }
            Scene::VideoEdit(ed) => {
                match ed.poll() {
                    Err(e) => {
                        self.status = format!("can't open the replay: {e}");
                        next = Some(Scene::Select);
                    }
                    Ok(_) => {
                        if let Some((at, from)) = ed.playing {
                            let elapsed = at.elapsed().as_secs_f64() * 1000.0;
                            let t = match &ed.game {
                                Some(g) => g.speed().map(g.track_at(from) + elapsed),
                                None => from + elapsed * ed.replay.rate,
                            };
                            if t >= ed.end {
                                ed.playing = None;
                                if let Some(a) = &self.audio {
                                    a.stop();
                                }
                            } else {
                                ed.seek(t);
                            }
                        }
                    }
                }
                let progress = self.export.as_ref().map(|ex| ex.progress());
                if let Some(g) = &mut ed.game {
                    g.kiai_effects = self.cfg.kiai_effects;
                }
                ed.draw(canvas, self.cfg.scroll_speed, progress, &self.video_status, self.last_video.is_some());
            }
            Scene::MpResults => match &self.mp {
                Some(s) => multiplayer::draw_results(canvas, s),
                None => next = Some(Scene::MpMenu),
            },
            Scene::Calibrate { game, vsync, .. } => {
                if game.update(now, self.audio.as_ref(), self.play_offset) {
                    finish_calibration = true;
                }
                if self.auto {
                    game.autoplay_style = self.cfg.autoplay;
                    game.autoplay(self.audio.as_ref());
                }
                if *vsync {
                    game.set_visual_offset(visual_offset);
                    draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, self.bg_brightness > 0.0, |c| {
                        game.draw_playfield(c, self.cfg.scroll_speed);
                    });
                    game.draw_hud(canvas);
                    calibrate::draw_by_sight(canvas, game, self.cfg.vsync_tune_ms);
                } else {
                    calibrate::draw_by_ear(canvas, game);
                }
            }
            Scene::CalibrateDone { estimate, vsync, .. } => {
                let current = if *vsync { self.cfg.vsync_tune_ms } else { self.cfg.offset_ms };
                calibrate::draw_result(canvas, estimate, current, self.cfg.calibration_bpm, *vsync);
            }
            Scene::Settings { prev } => {
                let from_game = matches!(**prev, Scene::Play { .. });
                self.settings_ui.krokodil = self.is_krokodil();
                settings::tab_bar(canvas, 1, if from_game { "game paused" } else { "" });
                let searching = !self.settings_ui.search.is_empty();
                let footer = if self.settings_ui.editing.as_ref().is_some_and(|e| matches!(e.item, Item::SongsFolder | Item::DataFolder)) {
                    "type the folder's path   Ctrl+V paste   Enter set   Esc cancel".to_string()
                } else if self.settings_ui.editing.is_some() {
                    "type a value   Enter set   Esc cancel".to_string()
                } else {
                    let enter = match self.settings_ui.item().kind() {
                        Kind::List => "Enter list",
                        Kind::Value => "Enter type a value",
                        Kind::Toggle => "Enter toggle",
                        Kind::Key => "Enter rebind",
                        Kind::Action => "Enter go",
                    };
                    let back = if searching {
                        "Esc clear the search"
                    } else if from_game {
                        "Esc back to pause menu"
                    } else {
                        "Esc or Tab back"
                    };
                    format!("Type to search   Up/Down select   Left/Right change   {enter}   {back}")
                };
                let this = &*self;
                this.settings_ui.draw(canvas, &|item| this.setting_value(item), &|item| this.setting_control(item), &footer);
                if let Some(p) = &self.settings_ui.choices {
                    p.draw(canvas);
                }
            }
            Scene::LayoutEdit { .. } => {
                draw_game_area(canvas, &self.cfg, &mut self.frame_clip, &self.grip, &self.hover, false, game::draw_preview);
                game::draw_preview_hud(canvas);
                let area = self.cfg.playfield_area;
                layout::draw_frame(canvas, area, true);
                let w = canvas.width;
                let bw = (w - 32.0).min(660.0);
                canvas.round_rect([w / 2.0 - bw / 2.0, 440.0], [bw, 34.0], 8.0, rgba(0x000000, 0.75), rgba(accent(), 0.6), 1.0);
                let room = bw - 16.0;
                canvas.text_centered(w / 2.0, 446.0, 8.0, rgba(0xffffff, 1.0), &Canvas::fit(8.0, room, &format!("Playfield area: {}", area.describe())));
                let help = "Drag: move (snaps; Shift = free), edges: resize, wheel: scale.  [ ] presets, arrows move, Shift+arrows resize, R full, Enter done";
                let size = Canvas::fit_size(7.0, 5.5, room, help);
                canvas.text_centered(w / 2.0, 460.0 + (7.0 - size) / 2.0, size, rgba(0x9a9ab0, 1.0), &Canvas::fit(size, room, help));
            }
        }
        if self.grip.is_some() {
            layout::draw_guides(canvas, &self.guides);
        }
        if self.touch_used && self.touch_drums() {
            self.draw_touch_zones(canvas, now);
        }
        if self.cfg.show_playfield_size && matches!(self.scene, Scene::Play { .. } | Scene::MpPlay { .. } | Scene::Calibrate { .. }) {
            let text = self.playfield_label();
            let tw = Canvas::text_width(7.0, &text);
            canvas.text_right(canvas.width - 8.0, 468.0, 7.0, rgba(0x8a8aa0, 0.85), &text);
            self.readout_rect = Some([canvas.width - 8.0 - tw - 4.0, 464.0, tw + 8.0, 15.0]);
        } else {
            self.readout_rect = None;
        }
        if let Some(cur) = self.mod_select {
            ui::draw_mod_select(canvas, self.shown_mods(), self.cfg.favourite_mods, cur);
            if let Some(setup) = &self.universe_setup {
                setup.draw(canvas, &self.cfg.universes, &self.cfg.universe_presets, game::FIRST_FOCUS);
            }
            if let Some(setup) = &self.shapeshift_setup {
                setup.draw_panel(canvas, &self.cfg.shapeshift, game::rail_outline);
            }
            if let Some(tab) = self.customization_tab() {
                ui::draw_customization_tabs(canvas, &mods::customizable(self.cfg.mods), tab);
            }
        }
        if self.settings_ui.dropdown.is_some() {
            let (groups, current, screen) = (self.preset_groups(), self.current_preset(), self.screen_size());
            let (saved, playfield) = (self.cfg.saved_presets.len(), self.playfield_size());
            if let Some(dd) = &mut self.settings_ui.dropdown {
                dd.draw(canvas, &groups, saved, current, screen, playfield);
            }
        }
        if finish_calibration {
            self.finish_calibration();
        }
        if let Some(s) = next {
            let music_on = matches!(&s, Scene::Results { results, .. } if results.music.is_some());
            if music_on {
                self.preview.invalidate_audio();
            } else if matches!(s, Scene::Results { .. } | Scene::Select) {
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
            canvas.round_rect([6.0, 6.0], [tw + 12.0, 16.0], 4.0, rgba(0x000000, 0.55), rgba(0, 0.0), 0.0);
            canvas.text(12.0, 10.0, 8.0, rgba(0x9fe09f, 1.0), &text);
        }
        // Not during a song, where a pointer left there would hold it over the notes for good.
        if !self.in_song() {
            self.volume.hold(now);
        }
        if self.volume.visible(now) {
            self.volume.draw(canvas, &self.cfg, now);
        }
        if self.overlay_allowed() && self.splash.is_none() {
            let logged_in = self.mp_menu.account.is_some();
            let (lobby, lobby_unread) = match &self.mp {
                Some(s) => (Some(&s.chat[..]), s.chat_unread),
                None => (None, 0),
            };
            if self.social.open {
                social::draw(canvas, &self.social, logged_in, lobby, lobby_unread);
            } else if let Some(p) = self.social.closing() {
                let (start, hits) = (canvas.shapes.len(), canvas.hits.len());
                social::draw(canvas, &self.social, logged_in, lobby, lobby_unread);
                canvas.hits.truncate(hits);
                canvas.transform_since(start, [0.0, 0.0], 1.0, p * 30.0, 0.0);
                for sh in &mut canvas.shapes[start..] {
                    sh.fill[3] *= 1.0 - p;
                    sh.border[3] *= 1.0 - p;
                }
            } else if logged_in && !self.in_song() && self.player.is_none() {
                social::draw_chip(canvas, &self.social, lobby_unread);
            }
            social::draw_notes(canvas, &self.social);
        }
        if let Some(page) = self.guide.filter(|_| self.splash.is_none()) {
            guide::draw(canvas, page, self.guide_binding, &self.cfg);
        }
        if let Some(p) = &self.profile {
            p.draw(canvas);
        }
        if let Some(view) = &self.changelog {
            view.draw(canvas);
        }
        #[cfg(target_os = "android")]
        if self.keyboard.up {
            self.draw_typing(canvas);
        }
        if !self.ime_preedit.is_empty() {
            // Above the toasts, in the colour of text being typed.
            let room = canvas.width - 72.0;
            let text = Canvas::fit_end(10.0, room, &self.ime_preedit);
            let tw = Canvas::text_width(10.0, &text);
            let x = canvas.width / 2.0 - tw / 2.0;
            canvas.round_rect([x - 10.0, 344.0], [tw + 20.0, 24.0], 6.0, rgba(0x15151d, 0.95), rgba(0xFFD34D, 1.0), 1.0);
            canvas.text(x, 351.0, 10.0, rgba(0xFFD34D, 1.0), &text);
            canvas.rect(x, 363.0, tw, 1.0, rgba(0xFFD34D, 0.8));
        }
        if let Some(t) = &self.toast {
            if now < t.until {
                let a = (t.until.duration_since(now).as_secs_f32() / 0.3).min(1.0);
                let rise = (1.0 - render::progress(t.until - Duration::from_millis(1500), 160.0)) * 14.0;
                // Long messages get smaller print, then are cut short.
                let room = canvas.width - 72.0;
                let size = Canvas::fit_size(12.0, 8.0, room, &t.text);
                let text = Canvas::fit(size, room, &t.text);
                let tw = Canvas::text_width(size, &text);
                let x = canvas.width / 2.0 - tw / 2.0;
                canvas.round_rect([x - 12.0, 380.0 + rise], [tw + 24.0, 28.0], 8.0, rgba(0x000000, 0.7 * a), rgba(0xffffff, 0.2 * a), 1.0);
                canvas.text(x, 394.0 - size / 2.0 + rise, size, rgba(0xffffff, a), &text);
            } else {
                self.toast = None;
            }
        }
        self.draw_update(canvas);
        self.draw_drag_hover(canvas, now);
        self.draw_crash(canvas);
        self.draw_quit(canvas);
        if self.splash.is_some() {
            let boot = self.boot_log();
            // Some window managers never report focus: don't wait forever.
            let ready = self.focus_seen || self.started.elapsed() > Duration::from_secs(5);
            if let Some(s) = &mut self.splash
                && !s.draw(canvas, now, self.audio.as_ref(), &boot, ready) {
                self.splash = None;
            }
        }
        std::mem::swap(&mut self.hits, &mut canvas.hits);
        capture
    }

    /// What the splash's boot log shows: what is actually starting up.
    fn boot_log(&self) -> Vec<String> {
        let songs = match &self.scan {
            Some((_, found)) => format!("songs  scanning... {} files", found.load(Ordering::Relaxed)),
            None => format!("songs  {} difficulties", self.library.len()),
        };
        let audio = self.audio.as_ref().map(|a| a.description.clone()).unwrap_or_else(|| "none (silent)".into());
        let skin = if self.cfg.skin.is_empty() { "built-in" } else { &self.cfg.skin };
        vec![
            format!("taiko.rs {}", env!("CARGO_PKG_VERSION")),
            format!("gpu    {}", self.adapter),
            format!("audio  {audio}"),
            songs,
            format!("skin   {skin}"),
            "drums  ready".into(),
        ]
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
        self.fetch_map(m);
    }

    /// Downloads the set a map someone else has comes from (mirrors: by set id, else a search).
    fn fetch_map(&mut self, m: net::MapInfo) {
        if self.lobby_lookup.is_some() {
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
                    let found = download::search(&format!("{} {}", m.artist, m.title), download::Category::Any, download::SortKey::Relevance, true, 0).and_then(|(sets, _)| {
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
        if !self.host_map_asked && self.host_transfer.is_none() {
            return;
        }
        if let Some(e) = part.error {
            // Fall back to the mirrors.
            self.host_map_asked = false;
            self.host_transfer = None;
            self.toast(format!("the host couldn't send it ({e}); trying the mirrors"));
            self.download_lobby_map();
            return;
        }
        if part.offset == 0 {
            let Some(m) = self.mp.as_ref().and_then(|s| s.map.clone()) else { return };
            self.host_map_asked = false;
            if part.total > download::MAX_HOST_MAP {
                self.toast("the host's map is too big; trying the mirrors".into());
                self.download_lobby_map();
                return;
            }
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
            let (job, feed) = download::receive(set, part.name.clone(), part.total, m.hash, self.songs_dir.clone());
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

    /// Scores saved before pp get theirs worked out in the background, once, and the index is
    /// written again with them.
    fn poll_pp_backfill(&mut self) {
        if !self.pp_backfill_started {
            self.pp_backfill_started = true;
            let missing: Vec<PathBuf> = self.scores.iter().filter(|r| r.pp.is_none()).map(|r| r.file.clone()).collect();
            if !missing.is_empty() {
                self.pp_backfill = Some(scores::backfill_pp(missing));
            }
        }
        let Some(found) = self.pp_backfill.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.pp_backfill = None;
        if found.is_empty() {
            return;
        }
        let found: std::collections::HashMap<PathBuf, f64> = found.into_iter().collect();
        for row in &mut self.scores {
            if let Some(&pp) = found.get(&row.file) {
                row.pp = Some(pp);
            }
        }
        if let Err(e) = scores::rewrite_index(&self.scores) {
            eprintln!("couldn't save the scores' pp: {e}");
        }
    }

    /// Plays sent to the online leaderboards, and the leaderboard being fetched.
    fn poll_online_scores(&mut self) {
        let mut done = Vec::new();
        self.submits.retain(|rx| match rx.try_recv() {
            Ok(r) => {
                done.push(r);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        for r in done {
            let text = match r {
                Ok(online::Reply::Submitted { map_rank, pp, rank }) => {
                    let before = self.me.map(|(b, _)| b);
                    self.me = Some((pp, rank));
                    if let Scene::Results { results, .. } = &mut self.scene
                        && results.opened.is_none() && results.replay.is_some() {
                        results.total = Some(ui::Total { before, after: pp, rank });
                    }
                    let place = map_rank.map(|r| format!("#{r} on this map")).unwrap_or_else(|| "sent (not your best here)".into());
                    let total = rank.map(|r| format!("  |  {pp:.0} pp, #{r} overall")).unwrap_or_default();
                    format!("online: {place}{total}")
                }
                // Servers from before scores don't know the request.
                Err(e) if e == "bad request" => "online: the server doesn't take scores yet".into(),
                Err(e) => format!("online: score not sent ({e})"),
                Ok(_) => continue,
            };
            self.toast(text);
        }
        if let Some((map, rx)) = &self.board_fetch
            && let Ok(rows) = rx.try_recv() {
            let rows = rows.map_err(|e| if e == "bad request" { "the server doesn't keep scores yet".into() } else { e });
            self.online_boards.insert(*map, (Instant::now(), rows));
            self.board_fetch = None;
        }
    }

    /// The online leaderboard of map `hash` (fetched in the background; again after a minute).
    fn online_board(&mut self, hash: u64) -> Option<&Result<Vec<online::OnlineScore>, String>> {
        let stale = self.online_boards.get(&hash).is_none_or(|(at, _)| at.elapsed() > Duration::from_secs(60));
        if stale && self.board_fetch.is_none() {
            self.board_fetch = Some((hash, fetch_board(self.cfg.online_server.clone(), hash)));
        }
        self.online_boards.get(&hash).map(|(_, rows)| rows)
    }

    /// The logged-in player's own pp and rank, fetched once per login.
    fn poll_me(&mut self) {
        let account = self.mp_menu.account.clone().unwrap_or_default();
        if account != self.me_for {
            self.me_for = account.clone();
            self.me = None;
            self.me_fetch = None;
            if !account.is_empty() {
                let (server, (tx, rx)) = (self.cfg.online_server.clone(), mpsc::channel());
                std::thread::spawn(move || {
                    let result = online::call(&server, online::Request::Profile { name: account }).and_then(|r| match r {
                        online::Reply::Profile(p) => Ok(*p),
                        _ => Err(crate::lang::odd_reply()),
                    });
                    let _ = tx.send(result);
                });
                self.me_fetch = Some(rx);
            }
        }
        if let Some(Ok(result)) = self.me_fetch.as_ref().map(|rx| rx.try_recv()) {
            self.me_fetch = None;
            if let Ok(p) = result {
                self.me = Some((p.pp, p.rank));
            }
        }
    }

    fn poll_changelog(&mut self) {
        if let Some(view) = &mut self.changelog {
            view.poll();
        }
        // A delayed response must not open an overlay in the middle of a song or replace
        // a changelog the player has already opened manually.
        if !self.in_song()
            && let Some(view) = changelog::poll_automatic(&mut self.pending_changelog)
            && self.changelog.is_none()
        {
            self.changelog = Some(view);
        }
    }

    fn poll_downloads(&mut self) {
        self.poll_me();
        self.poll_pp_backfill();
        self.poll_online_scores();
        if let Some(p) = &mut self.profile {
            p.poll();
        }
        self.poll_changelog();
        #[cfg(target_os = "android")]
        if self.imports_checked.elapsed() >= Duration::from_secs(1) {
            self.imports_checked = Instant::now();
            for (archive, name) in android::take_imports() {
                self.import_osz(archive, &name, true);
            }
        }
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
                finished.push((job.set.clone(), job.update, result));
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                finished.push((job.set.clone(), job.update, Err("download stopped".into())));
                false
            }
        });
        for (set, update, result) in finished {
            match result {
                Ok(folder) if update => self.add_updated(&set, &folder),
                Ok(folder) => self.add_downloaded(&set, &folder),
                Err(e) => {
                    if update {
                        self.select.updates.insert(set.id, ui::SetUpdate::Unknown);
                    }
                    eprintln!("download {} failed: {e}", set.id);
                    self.toast(format!("download failed: {}", e.chars().take(80).collect::<String>()));
                }
            }
        }
    }

    /// Song select: once the selection has rested on a song for a moment, asks a mirror whether
    /// osu! has a newer version of its set (each set once a session; not while scrolling past).
    fn poll_set_check(&mut self, now: Instant) {
        if let Some((id, rx)) = &self.set_check {
            let id = *id;
            let state = match rx.try_recv() {
                Ok(Ok((outdated, edited))) => {
                    // Like osu!stable: a difficulty whose file osu! doesn't know (edited from a
                    // ranked one) isn't ranked, whatever its set is.
                    if !edited.is_empty() {
                        self.select.edited.extend(edited);
                        self.select.refilter(&self.library);
                    }
                    if outdated { ui::SetUpdate::Available } else { ui::SetUpdate::UpToDate }
                }
                Ok(Err(e)) => {
                    eprintln!("checking set {id} for updates: {e}");
                    ui::SetUpdate::Unknown
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => ui::SetUpdate::Unknown,
            };
            self.set_check = None;
            // An update started meanwhile keeps its state.
            self.select.updates.entry(id).and_modify(|s| if *s == ui::SetUpdate::Checking { *s = state }).or_insert(state);
        }
        if !self.cfg.check_updates || !matches!(self.scene, Scene::Select) {
            self.set_check_due = None;
            return;
        }
        let Some(entry) = self.select.selected() else { return };
        let Some(id) = self.library[entry].set_id() else { return };
        if self.select.updates.contains_key(&id) {
            return;
        }
        match self.set_check_due {
            Some((due, since)) if due == id => {
                if self.set_check.is_some() || now.saturating_duration_since(since) < Duration::from_millis(800) {
                    return;
                }
            }
            _ => {
                self.set_check_due = Some((id, now));
                return;
            }
        }
        // The set's difficulties here: beatmap ID and the MD5 osu! knows each version by.
        let folder = self.library[entry].path.parent().map(|p| p.to_path_buf());
        let maps: Vec<(Option<u32>, PathBuf)> = self
            .library
            .iter()
            .filter(|e| e.set_id() == Some(id) && e.path.parent() == folder.as_deref() && !self.select.gone.contains(&e.path))
            .map(|e| (e.meta.beatmap_id, e.path.clone()))
            .collect();
        self.select.updates.insert(id, ui::SetUpdate::Checking);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = download::set_checksums(id).map(|online| {
                let here: Vec<(Option<u32>, String, &PathBuf)> = maps.iter().filter_map(|(bid, p)| Some((*bid, download::md5_hex(&std::fs::read(p).ok()?), p))).collect();
                let ids: Vec<(Option<u32>, String)> = here.iter().map(|(bid, md5, _)| (*bid, md5.clone())).collect();
                let edited = here.iter().filter(|(_, md5, _)| !download::is_osus(md5, &online)).map(|(_, _, p)| (*p).clone()).collect();
                (download::outdated(&ids, &online), edited)
            });
            let _ = tx.send(result);
        });
        self.set_check = Some((id, rx));
    }

    /// Downloads the newest version of map `entry`'s set into its folder: the set's own
    /// difficulties are replaced, home-made ones (no beatmap ID) stay.
    fn update_set(&mut self, entry: usize) {
        let e = &self.library[entry];
        let (Some(id), Some(folder)) = (e.set_id(), e.path.parent().map(|p| p.to_path_buf())) else {
            self.toast("this map has no osu! set ID to update it by".into());
            return;
        };
        if self.downloads.iter().any(|j| j.set.id == id) {
            return;
        }
        let replaced: Vec<PathBuf> = self
            .library
            .iter()
            .filter(|m| m.path.parent() == Some(folder.as_path()) && m.meta.beatmap_id.is_some() && m.set_id() == Some(id) && !self.select.gone.contains(&m.path))
            .map(|m| m.path.clone())
            .collect();
        let set = download::SetInfo {
            id,
            artist: e.meta.artist.clone(),
            title: e.meta.title.clone(),
            creator: e.meta.creator.clone(),
            status: String::new(),
            diffs: Vec::new(),
            play_count: 0,
            favourite_count: 0,
            rating: 0.0,
            ranked_date: String::new(),
        };
        self.toast(format!("updating {} - {}...", set.artist, set.title));
        self.select.updates.insert(id, ui::SetUpdate::Updating);
        self.downloads.push(download::start_update(set, folder, replaced));
    }

    /// A set downloaded again (see `update_set`): its difficulties in the library are read anew in
    /// place (indices stay valid), ones it no longer has leave the list, new ones join it.
    fn add_updated(&mut self, set: &download::SetInfo, folder: &std::path::Path) {
        let mut fresh: std::collections::HashMap<PathBuf, Entry> = library::scan(&[folder.to_path_buf()], &AtomicUsize::new(0)).into_iter().map(|e| (e.path.clone(), e)).collect();
        let mut changed = Vec::new();
        for (i, e) in self.library.iter_mut().enumerate() {
            if e.path.parent() != Some(folder) {
                continue;
            }
            match fresh.remove(&e.path) {
                Some(new) => {
                    *e = new;
                    self.select.gone.remove(&e.path);
                    changed.push((i, e.path.clone(), e.modified, e.size));
                }
                None => {
                    self.select.gone.insert(e.path.clone());
                }
            }
        }
        if let Some(info) = &self.mapinfo {
            info.extend(changed);
            self.mapinfo_sorted = false;
        }
        let mut added: Vec<Entry> = fresh.into_values().collect();
        added.sort_by(|a, b| a.path.cmp(&b.path));
        self.append_maps(added);
        self.select.updates.insert(set.id, ui::SetUpdate::UpToDate);
        self.toast(format!("updated {} - {}", set.artist, set.title));
    }

    /// Song select goes to the maps just added, whatever its filters. During a game they're only
    /// listed: the cursor stays where the player left it.
    fn reveal_maps(&mut self, maps: Vec<PathBuf>) {
        if !matches!(self.scene, Scene::Play { .. } | Scene::MpPlay { .. } | Scene::MpWait { .. }) && self.mp.is_none() {
            self.select.reveal(&self.library, maps);
        } else {
            self.select.pin(&self.library, maps);
        }
    }

    /// Adds a freshly downloaded set to the library (appended, so existing indices stay valid).
    fn add_downloaded(&mut self, set: &download::SetInfo, folder: &std::path::Path) {
        let known: std::collections::HashSet<PathBuf> = self.library.iter().filter(|e| e.path.starts_with(folder)).map(|e| e.path.clone()).collect();
        let fresh: Vec<Entry> = library::scan(&[folder.to_path_buf()], &AtomicUsize::new(0)).into_iter().filter(|e| !known.contains(&e.path)).collect();
        // Nothing new (unpacked over the same files): the set that's there is what was added.
        let maps = if fresh.is_empty() { main_maps(self.library.iter().filter(|e| e.path.starts_with(folder))) } else { main_maps(fresh.iter()) };
        let n = fresh.len();
        self.append_maps(fresh);
        self.installed_sets().insert(set.id);
        eprintln!("downloaded set {} into {} ({n} maps)", set.id, folder.display());
        // Lobby: the map we were missing may be here now.
        let library = &self.library;
        let mut lobby_note = None;
        if let Some(s) = &mut self.mp
            && let (Some(m), false) = (&s.map, s.have_map) {
            s.map_entry = find_map(library, m);
            s.have_map = s.map_entry.is_some();
            s.client.send(net::ClientMsg::HasMap(s.have_map));
            if !s.have_map && m.set_id == Some(set.id) {
                lobby_note = Some("downloaded, but the host has a different version of this map".to_string());
            }
        }
        self.reveal_maps(maps);
        let text = lobby_note.unwrap_or_else(|| match set.artist.is_empty() {
            // An imported .osz: only its file name is known.
            true if n == 0 => format!("{} is already in the list", set.title),
            true => format!("imported {} ({n} maps)", set.title),
            false => format!("downloaded {} - {}", set.artist, set.title),
        });
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
            KeyCode::Backspace if self.modifiers_ctrl => self.browse.clear(),
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

    /// The SONGS tab: the songs themselves, not the menu over them.
    fn show_songs(&mut self) {
        self.main_menu = None;
        self.scene = Scene::Select;
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
                self.show_songs();
            }
            (Scene::Settings { .. }, 2) if settings_from_menu => {
                self.close_settings();
                self.open_multiplayer();
            }
            (Scene::MpMenu | Scene::Lobby, 0) => self.show_songs(),
            (Scene::MpMenu | Scene::Lobby, 1) => self.open_settings(),
            (Scene::Select | Scene::MpMenu | Scene::Lobby, 3) => self.open_download(),
            (Scene::Settings { .. }, 3) if settings_from_menu => {
                self.close_settings();
                self.open_download();
            }
            (Scene::Download, 0) => self.show_songs(),
            (Scene::Download, 1) => self.open_settings(),
            (Scene::Download, 2) => self.open_multiplayer(),
            _ => {}
        }
    }

    fn click(&mut self, hit: render::Hit, window: Option<&Window>) {
        use render::Hit;
        // The exit question takes only its own buttons.
        if self.quit_asked && !matches!(hit, Hit::Key(KeyCode::KeyY | KeyCode::KeyN)) {
            return;
        }
        if self.crash_shown() && !matches!(hit, Hit::Key(KeyCode::KeyC | KeyCode::Escape)) {
            return;
        }
        match hit {
            Hit::Key(code) => self.key_inner(code, false, None, Instant::now(), window),
            Hit::Tab(i) => self.goto_tab(i),
            Hit::MenuItem(i) => self.menu_click(i),
            Hit::MenuStep(by) => {
                if !self.main_menu.as_ref().is_some_and(menu::Menu::is_leaving) {
                    self.menu_step(by);
                }
            }
            Hit::Guide(action) => self.guide_action(action),
            Hit::DiscordServer => {
                if !open_url(menu::DISCORD_INVITE) {
                    self.toast(menu::DISCORD_INVITE.into());
                }
            }
            Hit::Music(control) => self.music_control(control),
            Hit::MusicBar => {
                if let Some(p) = &mut self.player {
                    p.drag = Some(player::Drag::Timeline);
                }
                self.music_seek_to_pointer();
            }
            Hit::PlayerVolume => {
                if let Some(p) = &mut self.player {
                    p.drag = Some(player::Drag::Volume);
                }
                self.music_volume_to_pointer();
            }
            Hit::PlayerSearch => {
                if let Some(p) = &mut self.player {
                    p.searching = true;
                }
            }
            Hit::PlayerRow(track) => self.music_play(track),
            Hit::PlayerFavourite(track) => self.music_favourite(track),
            Hit::PlayerFolders => {
                if let Some(p) = &mut self.player {
                    p.folders_open = !p.folders_open;
                }
            }
            Hit::PlayerFolder(i) => {
                if let Some(p) = &mut self.player {
                    p.open_folder(i, fresh_seed());
                }
            }
            Hit::PlayerQueue(track) => {
                if let Some(p) = &mut self.player {
                    p.list.queue.push_back(track);
                    let n = p.list.queue.len();
                    self.toast(tr!("queued: {n}", "キュー: {n}", "в очереди: {n}";));
                }
            }
            Hit::PlayerUnqueue(row) => {
                if let Some(p) = &mut self.player {
                    p.list.queue.remove(row);
                }
            }
            Hit::PlayerSpeed(i) => {
                if let Some(p) = &mut self.player {
                    p.toggle_speed(player::Speed::ALL[i]);
                }
            }
            Hit::PlayerPitch => {
                if let Some(p) = &mut self.player {
                    p.semitones = 0;
                }
            }
            Hit::Presets => self.open_dropdown(),
            Hit::Update => self.update_now(),
            Hit::ScoreRow(k) => {
                if let Some(file) = self.leaderboard.get(k).cloned() {
                    self.open_local_score(&file);
                }
            }
            Hit::OnlineScoreRow(k) => {
                if let Some(score) = self.online_leaderboard.get(k).cloned() {
                    self.open_online_score(score);
                }
            }
            Hit::SocialToggle => self.toggle_social(),
            Hit::ChatMap(set) => self.open_chat_map(set),
            // Tapped: brings up the on-screen keyboard (`keyboard_tap`); the keys type there anyway.
            Hit::SocialPanel | Hit::TextField => {}
            Hit::SocialTab(tab) => {
                self.social.set_tab(tab);
                self.social.mark_read(self.mp.is_some());
            }
            Hit::SocialRow(i) => {
                self.social.cursor = i;
                self.social.mark_read(self.mp.is_some());
            }
            Hit::SocialAction(i, action) => {
                self.social.cursor = i;
                self.social_action(action);
            }
            Hit::SocialNote(i) => {
                if i < self.social.notes.len() {
                    let note = self.social.notes.remove(i);
                    match note.action {
                        social::NoteAction::Chat(name) => {
                            self.open_social();
                            self.social.select_friend(&name, self.mp.is_some());
                            self.social.mark_read(self.mp.is_some());
                        }
                        social::NoteAction::Requests => {
                            self.open_social();
                            self.social.set_tab(social::Tab::Friends);
                            let in_lobby = self.mp.is_some();
                            if let Some(r) = self.social.rows(in_lobby).iter().position(|r| matches!(r, social::Row::Request(_))) {
                                self.social.cursor = r;
                            }
                        }
                        social::NoteAction::Global => {
                            self.open_social();
                            self.social.set_tab(social::Tab::Chat);
                            self.social.mark_read(self.mp.is_some());
                        }
                        social::NoteAction::Watch(name) => self.watch(name),
                        social::NoteAction::None => {}
                    }
                }
            }
            Hit::GiveHost(id) => {
                if let Some(s) = &self.mp {
                    s.client.send(net::ClientMsg::GiveHost(id));
                    let name = s.players.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
                    self.toast(format!("{name} is the host now"));
                }
            }
            Hit::BoardTab(online) => self.board_online = online,
            Hit::Changelog => self.changelog = Some(changelog::View::recent(false)),
            Hit::Account => match self.mp_menu.account.clone() {
                Some(name) => {
                    let url = format!("{}/u/{name}", online::WEBSITE);
                    if !open_url(&url) {
                        self.toast(url);
                    }
                }
                // Logging in is in the multiplayer tab.
                None => self.goto_tab(2),
            },
            Hit::Overlay => {}
            // Dragged (`mouse_button`).
            Hit::SplitList | Hit::SplitPreview => {}
            Hit::ScoreVideo(k) => {
                if let Some(file) = self.leaderboard.get(k).cloned() {
                    self.open_video_editor(&file);
                }
            }
            Hit::DlCategory(i) => self.browse.set_category(download::Category::ALL[i]),
            Hit::DlSort(i) => self.browse.set_sort(download::SortKey::ALL[i]),
            Hit::SettingsSection(i) => {
                self.settings_ui.binding = None;
                self.settings_ui.clear_search();
                self.settings_ui.cursor = settings::section_start(i);
            }
            Hit::SortList => self.open_sorts(),
            Hit::CollectionPick => self.open_collections(),
            Hit::PickRow(i) => self.pick_row(i),
            Hit::ChoiceRow(i) => self.pick_choice(i),
            Hit::SongsFolder => self.browse_songs_folder(),
            // Taken when pressed (`mouse_button`).
            Hit::SongScrollbar => {}
            Hit::DlRow(i) if i == self.browse.cursor => self.download_key(KeyCode::Enter, None, window),
            Hit::DlRow(i) => self.browse.cursor = i,
            Hit::DlGet(i) => {
                self.browse.cursor = i;
                self.download_key(KeyCode::Enter, None, window);
            }
            Hit::SongHeader(row) => self.click(Hit::SongRow(row), window),
            Hit::SectionHeader(s) => self.select.click_section(s, &self.library),
            Hit::ListBack => self.select.follow_selection(),
            Hit::ToMenu if self.select.to_menu => {
                self.select.searching = false;
                self.main_menu = Some(menu::Menu::new());
            }
            Hit::ToMenu => self.key_inner(KeyCode::Escape, false, None, Instant::now(), window),
            Hit::ModsButton => self.open_mod_select(),
            Hit::ModToggle(i) => {
                self.mod_select = Some(i);
                self.toggle_mod(mods::ALL[i]);
            }
            Hit::UniverseCell(row, col) => {
                self.universe_setup = Some(multiverse::Setup::new(row, col));
                let dir = if self.modifiers_shift { -1 } else { 1 };
                multiverse::Setup::change(&mut self.cfg.universes, row, col, dir);
                self.universe_action(multiverse::Action::Changed);
            }
            Hit::ShapeshiftCell(row, col) => {
                self.shapeshift_setup = Some(shapeshift::Setup { row, col });
                shapeshift::Setup::pick(&mut self.cfg.shapeshift, row, col);
                self.shapeshift_changed();
            }
            Hit::ShapeshiftMove(dir) => {
                if let Some(setup) = &mut self.shapeshift_setup
                    && setup.move_shape(&mut self.cfg.shapeshift, dir) {
                    self.shapeshift_changed();
                }
            }
            Hit::CustomTab(i) => {
                if let Some(&m) = mods::customizable(self.cfg.mods).get(i) {
                    self.open_customization(m);
                }
            }
            Hit::UniversePreset(i) => {
                let action = multiverse::Setup::load(&mut self.cfg.universes, &self.cfg.universe_presets, i);
                self.universe_action(action);
            }
            Hit::UniversePresetDelete(i) => {
                let action = multiverse::Setup::delete(&mut self.cfg.universe_presets, i);
                self.universe_action(action);
            }
            Hit::UpdateSet(i) => self.update_set(i),
            Hit::SongMenu(k) => {
                let action = self.select.menu_items(&self.library).get(k).map(|(_, a)| *a);
                if let Some(ui::MenuAction::Love(set, loved)) = action {
                    self.select.menu = None;
                    self.toast(format!("{} set {set} on taiko.rs...", if loved { "loving" } else { "unloving" }));
                    self.call_loved(Some((set, loved)));
                } else if action == Some(ui::MenuAction::UpdateSong) {
                    if let Some(i) = self.select.menu_map() {
                        self.update_set(i);
                    }
                    self.select.menu = None;
                } else if let Some(what) = action.and_then(|a| self.select.hide(a, &self.library)) {
                    self.toast(format!("hid {what}: Settings > Hidden maps brings it back"));
                }
            }
            // Like osu!: the first click selects, clicking the selected difficulty plays it.
            Hit::SongRow(row) if row == self.select.cursor() => self.key_inner(KeyCode::Enter, false, None, Instant::now(), window),
            // A song's own row picks its remembered (or hardest) difficulty.
            Hit::SongRow(row) if self.select.is_song_start(row) && !self.select.in_open_song(row) => self.select.enter_song(row, &self.library),
            Hit::SongRow(row) => self.select.set_cursor(row),
            // One click does what the row does: flips a switch, opens a list, waits for a key.
            Hit::SettingRow(i) => {
                self.settings_ui.cursor = i;
                self.settings_key(KeyCode::Enter, window);
            }
            Hit::SettingLevel(i) => {
                self.settings_ui.cursor = i;
                self.settings_level = Some(i);
                self.set_level(i);
            }
            Hit::SettingsSearch => {}
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
                if self.mp_menu.row().is_action() {
                    self.mp_menu_key(KeyCode::Enter, None);
                }
            }
        }
    }
}

/// Rounds of asking the server about statuses it didn't have yet, and the wait between them.
const STATUS_ROUNDS: usize = 60;
const STATUS_ROUND_SECS: u64 = 45;

/// Frames per second drawn while the window doesn't have the focus.
const UNFOCUSED_FPS: u32 = 30;

/// The capture name of an F12 screenshot (scripted runs name theirs).
const SCREENSHOT: &str = "screenshot";

/// Puts an F12 screenshot on the clipboard, saves it as the next free `taiko-rs-N.png` in the
/// Screenshots folder, or both (`cfg.screenshot_to`), off the render thread (the encoding takes
/// a moment), and says where it went.
fn save_screenshot(state: &Arc<Mutex<State>>, w: u32, h: u32, rgb: Vec<u8>) {
    let state = state.clone();
    std::thread::spawn(move || {
        let to = lock(&state).cfg.screenshot_to;
        let copied = to.clipboard() && clipboard::set_image(w, h, &rgb);
        let text = if !to.folder() {
            if copied { "screenshot copied".to_string() } else { "screenshot failed: no clipboard to copy to".to_string() }
        } else {
            let saved = (|| {
                let dir = config::own_dir("Screenshots").ok_or("no folder to keep screenshots in")?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = (1..).map(|n| dir.join(format!("taiko-rs-{n}.png"))).find(|p| !p.exists()).unwrap();
                image::save_buffer(&path, &rgb, w, h, image::ColorType::Rgb8).map_err(|e| e.to_string())?;
                Ok::<_, String>(path)
            })();
            match saved {
                Ok(path) => {
                    let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    format!("screenshot {}saved: Screenshots/{name}", if copied { "copied and " } else { "" })
                }
                Err(e) => format!("screenshot {}failed: {e}", if copied { "copied, saving " } else { "" }),
            }
        };
        eprintln!("{text}");
        lock(&state).toast = Some(Toast { text, until: Instant::now() + Duration::from_millis(4000) });
    });
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
/// What the render thread tells the window thread.
enum UserEvent {
    /// The render thread stopped (quit requested or screenshot run finished).
    Stopped,
    /// Gameplay wants the pointer hidden (it's been still a second) or back.
    HideCursor(bool),
    /// Files dragged over the window, on Wayland (winit reports them itself elsewhere).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    FileDrag(FileDrag),
    /// A window size was picked (`State::window_request`).
    WindowRequest,
    /// Android: the on-screen keyboard typed something (or came up or went away).
    #[cfg(target_os = "android")]
    Typed(android::Typed),
    /// A `taiko-rs://` link opened while the game runs (`link::listen`).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    Link(link::Link),
}

/// Files dragged over the window, or dropped on it.
pub(crate) enum FileDrag {
    Hover,
    Left,
    Drop(Vec<PathBuf>),
}

fn render_loop(state: Arc<Mutex<State>>, mut renderer: Renderer, proxy: EventLoopProxy<UserEvent>) {
    let mut canvas = Canvas::default();
    let mut export_canvas = Canvas::default();
    let mut blend = video::Blend::default();
    let mut next_frame = Instant::now();
    let mut last_sample: Option<Instant> = None;
    let mut requested_present: Option<PresentChoice> = None;
    let mut pacer = clock::VsyncPacer::default();
    // TAIKO_NO_PACE=1: draw notes for when each image was handed out, as before (to compare).
    let pace = std::env::var_os("TAIKO_NO_PACE").is_none();
    let mut waits = (0.0, 0.0, 0.0);
    let mut cursor_hidden = false;
    // TAIKO_PROFILE=1: every 2 s, print where frame time goes (build = CPU under the state lock).
    let profile = std::env::var_os("TAIKO_PROFILE").is_some();
    // The waits are only measured for the profile and the smoothness report: reading the clock
    // can cost microseconds a time (HPET).
    let timing = profile || std::env::var_os("TAIKO_STATS").is_some();
    let (mut prof_start, mut prof_n, mut prof_build, mut prof_submit, mut prof_shapes) = (Instant::now(), 0u32, 0.0f64, 0.0f64, 0usize);
    let (mut prof_acquire, mut prof_acquire_max) = (0.0f64, 0.0f64);
    // TAIKO_LAYOUT=1: print each text that doesn't fit (once), see `debug::layout_problems`.
    canvas.audit = std::env::var_os("TAIKO_LAYOUT").is_some();
    let mut layout_seen = std::collections::HashSet::new();
    loop {
        let (present, cap, refresh_hz) = {
            let mut s = lock(&state);
            if s.quit {
                break;
            }
            if let Some((surface, w, h)) = s.surface_swap.take() {
                renderer.set_surface(surface, w, h);
            }
            if let Some((w, h)) = s.want_size.take() {
                renderer.resize(w, h);
            }
            renderer.letterbox = s.sync_letterbox();
            // Switch present mode when entering/leaving songs (menus can stay on vsync). Compare
            // with what was requested, not what was granted, so an unsupported mode that falls
            // back isn't re-applied every frame.
            let wanted = s.wanted_present();
            if requested_present != Some(wanted) {
                renderer.set_present(wanted);
                requested_present = Some(wanted);
                s.present = renderer.present;
            }
            if matches!(s.scene, Scene::Play { .. })
                && let Some(sm) = &mut s.smooth {
                sm.sample_waits(waits.0, waits.1, waits.2);
            }
            renderer.pixel_scale = if s.cfg.half_resolution { 2 } else { 1 };
            (s.present, s.frame_cap(), s.refresh_hz)
        };

        let t0 = timing.then(Instant::now);
        let Some(frame) = renderer.acquire() else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        let acquired = Instant::now();
        let now = acquired;
        // With vsync the notes are drawn for this frame's place on the refresh grid; the game
        // itself (and judging) runs on the real clock.
        let pace_shift_ms = if present == PresentChoice::Fifo && pace {
            let t = pacer.frame_time(acquired, refresh_hz);
            if t >= acquired { (t - acquired).as_secs_f64() * 1000.0 } else { -(acquired - t).as_secs_f64() * 1000.0 }
        } else {
            pacer.reset();
            0.0
        };
        // A video export renders offscreen for a while each UI frame (which shows its progress).
        let exporting = Instant::now();
        while exporting.elapsed() < Duration::from_millis(30) {
            let job = lock(&state).export_frame(&mut export_canvas);
            let Some(ExportView { size: (w, h), tx, brightness, clear, blended, last }) = job else { break };
            let shown = renderer.bg_brightness;
            renderer.bg_brightness = brightness;
            let vw = export_canvas.width.max(1.0);
            let clips: Vec<(std::ops::Range<usize>, [f32; 4])> = export_canvas
                .clips
                .iter()
                .map(|(r, [x, y, cw, ch])| (r.clone(), [x / vw, y / render::VIRTUAL_HEIGHT, cw / vw, ch / render::VIRTUAL_HEIGHT]))
                .collect();
            let frame = renderer.render_offscreen(&export_canvas.shapes, &clips, w, h, clear);
            renderer.bg_brightness = shown;
            let Some(px) = frame else { break };
            // Motion blur: the frame is the average of its views.
            let px = if blended {
                blend.add(&px);
                if !last {
                    continue;
                }
                blend.take()
            } else {
                px
            };
            if tx.send(px).is_err() {
                break;
            }
        }
        let (capture, clip, backdrop) = {
            let mut s = lock(&state);
            if let Some(atlas) = s.sprites_upload.take() {
                renderer.set_sprites(&atlas);
                // The video export draws with the skin too.
                for c in [&mut canvas, &mut export_canvas] {
                    c.sprites = render::PerSprite(atlas.sprites.map(|s| s.map(|(_, size)| size)));
                    c.skin_layout = atlas.layout;
                }
            }
            // Video export frames above are drawn without the grid; the screen's frame with it.
            render::set_text_grid(renderer.text_grid());
            render::set_frame_time(Some(now));
            s.frame_acquired = Some(acquired);
            s.pace_shift_ms = pace_shift_ms;
            s.seen_refresh_ms = pacer.refresh_ms();
            let capture = s.build_frame(&mut canvas, now, renderer.virtual_width());
            render::set_frame_time(None);
            render::set_text_grid(None);
            if canvas.audit {
                for p in debug::layout_problems(&canvas) {
                    if layout_seen.insert(p.clone()) {
                        eprintln!("layout: {p}");
                    }
                }
            }
            // Picked from a scripted key or a frame: the window thread applies it.
            if s.window_request.is_some() {
                let _ = proxy.send_event(UserEvent::WindowRequest);
            }
            let hide = s.cursor_hidden(now);
            if hide != cursor_hidden {
                cursor_hidden = hide;
                let _ = proxy.send_event(UserEvent::HideCursor(hide));
            }
            if let Some(image) = s.preview.bg_upload.take() {
                renderer.set_background(image);
                s.bg_changed = Some(Instant::now());
            }
            renderer.bg_brightness = s.bg_brightness;
            // A new background cross-fades from the previous one.
            let fade = if s.on_menu() { MENU_BACKGROUND_FADE_MS } else { 300.0 };
            renderer.bg_mix = s.bg_changed.map(|t| render::progress(t, fade)).unwrap_or(1.0);
            (capture, s.frame_clip.take(), s.backdrop(s.dims_background()))
        };
        // The playfield / preview clip (fractions) plus the lists' (virtual units).
        let vw = canvas.width.max(1.0);
        let to_fraction = |[x, y, w, h]: [f32; 4]| [x / vw, y / render::VIRTUAL_HEIGHT, w / vw, h / render::VIRTUAL_HEIGHT];
        let opaque = canvas
            .opaque_rect(clip.as_ref().map(|(range, a)| (range, [a.x * vw, a.y * render::VIRTUAL_HEIGHT, a.w * vw, a.h * render::VIRTUAL_HEIGHT])))
            .map(to_fraction);
        let mut clips: Vec<(std::ops::Range<usize>, [f32; 4])> = clip.into_iter().map(|(range, a)| (range, [a.x, a.y, a.w, a.h])).collect();
        clips.extend(canvas.clips.drain(..).map(|(range, r)| (range, to_fraction(r))));
        renderer.disc = canvas.disc.map(|d| (d, vw));
        let t1 = timing.then(Instant::now);
        let shot = renderer.submit(frame, &canvas.shapes, backdrop, capture.is_some(), clips, opaque);
        if let (Some(t0), Some(t1)) = (t0, t1) {
            let presented = Instant::now();
            // Clock-sample-to-next-refresh: approximates how stale the previous frame was when shown.
            let sample_age = last_sample.map(|s| (acquired - s).as_secs_f64() * 1000.0).unwrap_or(0.0);
            last_sample = Some(now);
            waits = ((acquired - t0).as_secs_f64() * 1000.0, (presented - t1).as_secs_f64() * 1000.0, sample_age);
            if profile {
                prof_n += 1;
                prof_acquire += waits.0;
                prof_acquire_max = prof_acquire_max.max(waits.0);
                prof_build += (t1 - now).as_secs_f64() * 1000.0;
                prof_submit += (presented - t1).as_secs_f64() * 1000.0;
                prof_shapes = prof_shapes.max(canvas.shapes.len());
                if prof_start.elapsed() >= Duration::from_secs(2) {
                    let n = prof_n.max(1) as f64;
                    let gpu = renderer.take_gpu_ms().map(|ms| format!("  gpu {ms:.2} ms")).unwrap_or_default();
                    // Overdraw: the pixels this frame's quads cover, in screens (TAIKO_PROFILE=shapes
                    // also lists the biggest).
                    let ppu = renderer.size().1 as f32 / render::VIRTUAL_HEIGHT;
                    let area = |sh: &render::Shape| {
                        let pad = 1.5 + sh.params[2].max(0.0) * ppu;
                        (sh.half[0] * ppu + pad) * 2.0 * (sh.half[1] * ppu + pad) * 2.0
                    };
                    let screen = (renderer.size().0 * renderer.size().1).max(1) as f32;
                    let cover: f32 = canvas.shapes.iter().map(area).sum::<f32>() / screen;
                    eprintln!(
                        "profile: {:.0} fps  acquire {:.2} ms (max {:.2})  build {:.2} ms  submit {:.2} ms  shapes {}{gpu}  cover {cover:.2}",
                        n / prof_start.elapsed().as_secs_f64(), prof_acquire / n, prof_acquire_max, prof_build / n, prof_submit / n, prof_shapes
                    );
                    if std::env::var("TAIKO_PROFILE").is_ok_and(|v| v == "shapes") {
                        let mut big: Vec<&render::Shape> = canvas.shapes.iter().collect();
                        big.sort_by(|a, b| area(b).total_cmp(&area(a)));
                        for sh in big.iter().take(12) {
                            eprintln!("  {:.3}  c {:?} h {:?} fill {:?} border {:?} p {:?}", area(sh) / screen, sh.center, sh.half, sh.fill, sh.border, sh.params);
                        }
                    }
                    (prof_start, prof_n, prof_build, prof_submit, prof_shapes) = (Instant::now(), 0, 0.0, 0.0, 0);
                    (prof_acquire, prof_acquire_max) = (0.0, 0.0);
                }
            }
        }
        if let (Some(name), Some((w, h, rgb))) = (capture, shot) {
            if name == SCREENSHOT {
                save_screenshot(&state, w, h, rgb);
            } else if let Some(dir) = lock(&state).shots.as_ref().map(|p| p.dir.clone()) {
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
            let late = Instant::now();
            if next_frame < late {
                next_frame = late + period;
            }
            sleep_until(next_frame);
        }
    }
    let _ = proxy.send_event(UserEvent::Stopped);
}

struct App {
    state: Arc<Mutex<State>>,
    /// Last cursor icon set, so it is only changed when the hover target changes.
    cursor: CursorIcon,
    /// The pixel pointer (None if the platform refused it: the system one is used).
    cursors: Option<cursor::Cursors>,
    /// The accent `cursors` were made in.
    cursor_accent: u32,
    cursor_hidden: bool,
    window: Option<Arc<Window>>,
    /// The window takes input methods (Japanese, ...): only while text is being typed, so a
    /// song's keys never wait on one.
    ime: bool,
    /// Kept to make a surface for each new window (Android, after coming back from the background).
    instance: Option<wgpu::Instance>,
    render_thread: Option<JoinHandle<()>>,
    proxy: EventLoopProxy<UserEvent>,
}

impl App {
    fn set_cursor(&self, icon: CursorIcon) {
        let Some(w) = &self.window else { return };
        match &self.cursors {
            Some(c) => w.set_cursor(c.get(icon)),
            None => w.set_cursor(icon),
        }
    }
}

impl App {
    fn handle_window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        // A new UI colour: the pointer is drawn in it too.
        if self.cursors.is_some() && self.cursor_accent != render::accent()
            && let Some(window) = &self.window {
            self.cursors = cursor::Cursors::new(event_loop, window.scale_factor());
            self.cursor_accent = render::accent();
            self.set_cursor(self.cursor);
        }
        match event {
            WindowEvent::CloseRequested => lock(&self.state).ask_quit(),
            WindowEvent::HoveredFile(_) => lock(&self.state).file_drag(FileDrag::Hover),
            WindowEvent::HoveredFileCancelled => lock(&self.state).file_drag(FileDrag::Left),
            WindowEvent::DroppedFile(path) => lock(&self.state).file_drag(FileDrag::Drop(vec![path])),
            WindowEvent::Resized(size) => self.resized(size),
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                // Timestamp before anything else, including taking the lock.
                let at = Instant::now();
                // Synthetic presses are keys already held when the window gets focus (the Tab of
                // an Alt+Tab back into the game): not real presses.
                if event.state != ElementState::Pressed || is_synthetic {
                    return;
                }
                let code = match event.physical_key {
                    PhysicalKey::Code(code) => code,
                    // Android's Back button / gesture has no key code, only this name; it does
                    // what Escape does (pause, close, go back).
                    _ if event.logical_key == Key::Named(NamedKey::BrowserBack) => KeyCode::Escape,
                    _ => return,
                };
                let mut s = lock(&self.state);
                // Alt+Tab (switching windows) isn't a Tab press for the game; nor is anything
                // that arrives while another window has focus.
                if !s.focused || (s.modifiers_alt && code == KeyCode::Tab) {
                    return;
                }
                s.key(code, event.repeat, event.text.as_deref(), at, self.window.as_deref());
            }
            WindowEvent::Ime(ime) => {
                let mut s = lock(&self.state);
                match ime {
                    winit::event::Ime::Preedit(text, _) => s.ime_preedit = text,
                    winit::event::Ime::Commit(text) => {
                        s.ime_preedit.clear();
                        // Typed like the on-screen keyboard's text (Android): a key with no
                        // meaning of its own, carrying each character.
                        let at = Instant::now();
                        for ch in text.chars().filter(|c| !c.is_control()) {
                            s.key(KeyCode::Lang1, false, Some(&ch.to_string()), at, self.window.as_deref());
                        }
                    }
                    winit::event::Ime::Enabled | winit::event::Ime::Disabled => s.ime_preedit.clear(),
                }
            }
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => self.read_monitor(),
            WindowEvent::Focused(true) => {
                self.read_monitor();
                let mut s = lock(&self.state);
                s.focused = true;
                s.focus_seen = true;
            }
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
            WindowEvent::MouseInput { state: button_state, button: winit::event::MouseButton::Right, .. } => {
                lock(&self.state).right_button(button_state == ElementState::Pressed);
            }
            WindowEvent::Touch(t) => {
                let at = Instant::now();
                let mut s = lock(&self.state);
                let (x, y) = s.to_game(t.location.x, t.location.y);
                s.touch(t.id, t.phase, x, y, at, self.window.as_deref());
            }
            WindowEvent::CursorMoved { position, .. } => {
                let icon = {
                    let mut s = lock(&self.state);
                    let (x, y) = s.to_game(position.x, position.y);
                    s.mouse_moved(x, y)
                };
                if icon != self.cursor {
                    self.cursor = icon;
                    self.set_cursor(icon);
                }
                // Moving brings a hidden pointer back at once, not a frame later.
                if self.cursor_hidden && !lock(&self.state).cursor_hidden(Instant::now()) {
                    self.cursor_hidden = false;
                    if let Some(w) = &self.window {
                        w.set_cursor_visible(true);
                    }
                }
            }
            WindowEvent::Focused(false) => {
                // Losing focus mid-song (alt-tab, notification) pauses instead of failing the map.
                let mut guard = lock(&self.state);
                let s = &mut *guard;
                s.focused = false;
                // The release may happen in another window.
                s.right_scroll = false;
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

    /// The screen the window is on: its size, modes and refresh. Read again as the window is
    /// resized, moved or focused: on Wayland it isn't known until the window is on screen, and
    /// the window can go to another one.
    fn read_monitor(&self) {
        let Some(m) = self.window.as_ref().and_then(|w| w.current_monitor()) else { return };
        let mut s = lock(&self.state);
        s.monitor_size = (m.size().width, m.size().height);
        let modes: Vec<(u32, u32)> = m.video_modes().map(|v| (v.size().width, v.size().height)).collect();
        if !modes.is_empty() {
            s.monitor_modes = modes;
        }
        if let Some(hz) = m.refresh_rate_millihertz().map(|mhz| mhz as f64 / 1000.0).filter(|&hz| hz >= 24.0)
            && (hz - s.refresh_hz).abs() > 0.01
        {
            eprintln!("screen refresh {hz:.2} Hz");
            s.refresh_hz = hz;
            s.refresh_status();
        }
    }

    /// The window's new size (a resize event, or a size requested and granted at once).
    fn resized(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        self.read_monitor();
        let mut s = lock(&self.state);
        s.want_size = Some((size.width, size.height));
        s.set_window(size.width as f32, size.height as f32);
        s.refresh_presets();
        // Out of fullscreen, the size is remembered for the next start (saved once it stops
        // changing: dragging an edge resizes many times a second).
        if !s.fullscreen && size.width > 0 && size.height > 0 {
            s.cfg.window_size = Some((size.width, size.height));
            s.window_resized_at = Some(Instant::now());
        }
    }

    /// A window size picked in settings: out of fullscreen if need be, then that size.
    /// Lets input methods in while text is being typed, and keeps them out otherwise.
    #[cfg(not(target_os = "android"))]
    fn update_ime(&mut self) {
        let Some(window) = &self.window else { return };
        let mut s = lock(&self.state);
        let want = s.takes_text() && !s.quit_asked && s.splash.is_none();
        if want != self.ime {
            self.ime = want;
            window.set_ime_allowed(want);
            if want {
                // Its candidates go by where the composed text is shown (`ime_preedit`).
                let scale = window.inner_size().height as f64 / render::VIRTUAL_HEIGHT as f64;
                let x = window.inner_size().width as f64 / 2.0;
                window.set_ime_cursor_area(winit::dpi::PhysicalPosition::new(x, 344.0 * scale), winit::dpi::PhysicalSize::new(1.0, 24.0 * scale));
            } else {
                s.ime_preedit.clear();
            }
        }
    }

    fn apply_window_request(&mut self) {
        let Some(window) = &self.window else { return };
        let mut s = lock(&self.state);
        let Some((w, h)) = s.window_request.take() else { return };
        if window.fullscreen().is_some() {
            window.set_fullscreen(None);
            s.fullscreen = false;
            s.cfg.fullscreen = false;
        }
        let granted = window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
        s.cfg.window_size = Some((w, h));
        s.cfg.save();
        drop(s);
        // Granted at once (Wayland): no resize event comes for it.
        if let Some(size) = granted {
            self.resized(size);
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            // Back from the background (Android): the old window is gone, draw into the new one.
            let Some(instance) = &self.instance else { return };
            let size = window.inner_size();
            match instance.create_surface(window.clone()) {
                Ok(surface) => {
                    let mut s = lock(&self.state);
                    s.surface_swap = Some((Some(surface), size.width, size.height));
                    s.set_window(size.width as f32, size.height as f32);
                    s.set_background(false);
                }
                Err(e) => eprintln!("can't draw to the new window: {e}"),
            }
            return;
        }
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut attrs = Window::default_attributes()
            .with_title("taiko.rs")
            .with_window_icon(logo::window_icon())
            .with_min_inner_size(winit::dpi::LogicalSize::new(MIN_WINDOW.0, MIN_WINDOW.1))
            .with_fullscreen(lock(&self.state).cfg.fullscreen.then_some(Fullscreen::Borderless(None)));
        #[cfg(target_os = "linux")]
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            attrs = attrs.with_name("taiko-rs", "taiko-rs");
        }
        let attrs = match lock(&self.state).cfg.window_size {
            Some((w, h)) => attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h)),
            None => attrs.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(RawWindowHandle::Win32(h)) = window.window_handle().map(|h| h.as_raw()) {
                lock(&self.state).window_handle = Some(h.hwnd.get() as usize);
            }
        }
        #[cfg(target_os = "linux")]
        {
            use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
            if let Ok(RawDisplayHandle::Wayland(h)) = window.display_handle().map(|h| h.as_raw()) {
                let proxy = self.proxy.clone();
                wayland_dnd::start(h.display.as_ptr(), move |drag| {
                    let _ = proxy.send_event(UserEvent::FileDrag(drag));
                });
            }
        }
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
            s.set_window(window.inner_size().width as f32, window.inner_size().height as f32);
            s.monitor_modes = window
                .current_monitor()
                .map(|m| m.video_modes().map(|v| (v.size().width, v.size().height)).collect())
                .unwrap_or_default();
            if let Some(m) = window.current_monitor() {
                s.monitor_size = (m.size().width, m.size().height);
            }
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
        self.cursors = cursor::Cursors::new(event_loop, window.scale_factor());
        self.cursor_accent = render::accent();
        self.window = Some(window);
        self.instance = Some(instance);
        self.set_cursor(self.cursor);
    }

    /// Android only: the window is about to be destroyed. The render thread must let go of its
    /// surface first, so wait (briefly) until it has.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        {
            let mut s = lock(&self.state);
            s.surface_swap = Some((None, 0, 0));
            s.set_background(true);
        }
        let deadline = Instant::now() + Duration::from_millis(1000);
        while self.render_thread.as_ref().is_some_and(|t| !t.is_finished())
            && lock(&self.state).surface_swap.is_some()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Stopped => event_loop.exit(),
            UserEvent::FileDrag(drag) => lock(&self.state).file_drag(drag),
            UserEvent::WindowRequest => self.apply_window_request(),
            UserEvent::Link(link) => {
                lock(&self.state).open_link(link);
                if let Some(w) = &self.window {
                    w.set_minimized(false);
                    w.focus_window();
                }
            }
            #[cfg(target_os = "android")]
            UserEvent::Typed(typed) => lock(&self.state).soft_keyboard(typed, self.window.as_deref()),
            UserEvent::HideCursor(hide) => {
                if hide != self.cursor_hidden {
                    self.cursor_hidden = hide;
                    if let Some(w) = &self.window {
                        w.set_cursor_visible(!hide);
                    }
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        self.handle_window_event(event_loop, event);
        self.apply_window_request();
        #[cfg(not(target_os = "android"))]
        self.update_ime();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        lock(&self.state).quit = true;
        // The Discord status goes with the game (dropping it clears it and says goodbye).
        let discord = lock(&self.state).discord.take();
        drop(discord);
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

impl State {
    #[allow(clippy::too_many_arguments)]
    fn new(
        mut cfg: Config,
        audio: Option<Engine>,
        splash: Option<splash::Splash>,
        songs_dir: PathBuf,
        song_dirs: Vec<PathBuf>,
        scan: Option<(Receiver<Vec<Entry>>, Arc<AtomicUsize>)>,
        play_when_scanned: bool,
        auto: bool,
    ) -> State {
        lang::set(cfg.language);
        let library = Vec::new();
        let mut select = Select::new(&library);
        let (set_status_tx, set_status_rx) = mpsc::channel();
        let status_cache = setstatus::Cache::load();
        select.sort = ui::SortMode::parse(&cfg.sort).unwrap_or(ui::SortMode::Artist);
        select.mods_label = cfg.mods.label();
        select.played = load_plays();
        let updater = update::Updater::new(cfg.check_updates);
        let pending_changelog = whats_new(&mut cfg);
        let present = cfg.present;
        let fullscreen = cfg.fullscreen;
        State {
            cfg,
            library,
            select,
            scene: Scene::Select,
            audio,
            toast: None,
            ime_preedit: String::new(),
            drag_hover: None,
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
            volume: volume::Overlay::new(),
            splash,
            main_menu: None,
            menu_notes: Default::default(),
            guide: None,
            guide_binding: None,
            guide_after: None,
            reset_armed: None,
            music_seeked: None,
            player: None,
            music_paused: false,
            media: None,
            media_told: None,
            window_handle: None,
            updater,
            hits: Vec::new(),
            browse: browse::Browse::default(),
            downloads: Vec::new(),
            set_check: None,
            set_check_due: None,
            #[cfg(target_os = "android")]
            imports_checked: Instant::now(),
            #[cfg(target_os = "android")]
            keyboard: android::Keyboard::default(),
            songs_dir,
            installed_sets: None,
            lobby_lookup: None,
            score_link: None,
            preview_remote: false,
            host_transfer: None,
            host_map_asked: false,
            scores: scores::load_index(),
            pp_backfill: None,
            pp_backfill_started: false,
            submits: Vec::new(),
            me: None,
            me_for: String::new(),
            me_fetch: None,
            board_online: false,
            profile: None,
            online_boards: std::collections::HashMap::new(),
            board_fetch: None,
            pending_replay: None,
            changelog: None,
            pending_changelog,
            pending_mods: Mods::NONE,
            mod_select: None,
            universe_setup: None,
            shapeshift_setup: None,
            leaderboard: Vec::new(),
            board_scroll: 0,
            board_for: (0, false),
            board_max: 0,
            board_rect: None,
            split_drag: None,
            split_click: None,
            online_leaderboard: Vec::new(),
            export: None,
            video_status: String::new(),
            last_video: None,
            bg_changed: None,
            sprites_upload: None,
            skin_map: None,
            skin_map_seen: None,
            focused: true,
            right_scroll: false,
            settings_level: None,
            scrollbar_drag: false,
            right_press: None,
            list_drag: None,
            focus_seen: false,
            mapinfo: None,
            mapinfo_sorted: false,
            song_dirs,
            osudb_rx: None,
            db_status: Default::default(),
            set_status: status_cache.iter().map(|(set, status)| (set, Some(status.to_string()))).collect(),
            status_cache,
            status_saved: Instant::now(),
            status_resorted: Instant::now(),
            status_new: false,
            status_sweep_due: None,
            set_status_tx,
            set_status_rx,
            loved_call: None,
            loved_asked: false,
            playing: None,
            status_wait: None,
            scene_kind: None,
            scene_since: Instant::now(),
            scrubbing: None,
            auto,
            shots: debug::ShotPlan::from_env(),
            pending_screen_shots: vec!["select", "results"],
            exit_when_done: std::env::var_os("TAIKO_EXIT_AFTER").is_some(),
            quit: false,
            quit_asked: false,
            crash: crash_report(),
            screenshot_due: false,
            smooth: std::env::var_os("TAIKO_STATS").is_some().then(debug::Smoothness::default),
            frame_acquired: None,
            pace_shift_ms: 0.0,
            seen_refresh_ms: None,
            started: Instant::now(),
            want_size: None,
            touch_used: false,
            touch_mouse: None,
            touch_flash: [None; 4],
            surface_swap: None,
            present,
            refresh_hz: 60.0,
            settings_ui: SettingsUi::default(),
            fullscreen,
            adapter: String::new(),
            input_script: debug::InputScript::from_env(),
            wall_shots: debug::wall_shots_from_env(),
            mp: None,
            mp_connecting: None,
            mp_menu: multiplayer::Menu::default(),
            mp_status: String::new(),
            picking_lobby_map: false,
            guessr: None,
            discord: None,
            map_offsets: config::load_map_offsets(),
            play_offset: 0.0,
            social: social::Social::default(),
            stream: None,
            presence: None,
            presence_at: None,
            scan,
            folder_pick: None,
            folder_pick_for: Item::SongsFolder,
            data_move: None,
            play_when_scanned,
            wheel_acc: 0.0,
            mouse: (0.0, 0.0),
            grip: None,
            hover: None,
            window_w: 1280.0,
            window_h: 720.0,
            real_window: (1280.0, 720.0),
            letterbox: None,
            frame_clip: None,
            guides: Vec::new(),
            readout_rect: None,
            replay_bar: None,
            replay_scrub: None,
            mouse_moved_at: Instant::now(),
            preview: preview::Preview::default(),
            stars: stars::Stars::default(),
            potentials: pp::Potentials::default(),
            bg_brightness: 0.0,
            resolutions: Vec::new(),
            monitor_modes: Vec::new(),
            monitor_size: (1920, 1080),
            window_request: None,
            window_resized_at: None,
        }
    }
}

/// Starts loading update notes in the background, and notes this build as started.
/// The caller keeps the view pending until a nonempty list of changes is available.
fn whats_new(cfg: &mut Config) -> Option<changelog::View> {
    let build = update::BUILD?;
    if cfg.last_build == build {
        return None;
    }
    let old = std::mem::replace(&mut cfg.last_build, build.to_string());
    cfg.save();
    if !old.is_empty() {
        Some(changelog::View::since(old))
    } else if cfg.existed {
        Some(changelog::View::recent(true))
    } else {
        None
    }
}

/// Desktop entry point: the command line, then the game.
/// `--recalc-pp`: see `recalc`.
pub fn recalc_pp(args: &[String]) -> Result<(), String> {
    recalc::run(args)
}

pub fn desktop_main() {
    let cfg = Config::load();
    config::set_data_folder(cfg.data_folder.clone());
    log_crashes();
    let mut args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let auto = args.iter().any(|a| a == "--auto");
    args.retain(|a| a != "--auto");
    // A `taiko-rs://` link (a score's "Watch replay" button): the running game opens it, else
    // this one does.
    let link = args.iter().find_map(|a| link::parse(&a.to_string_lossy()));
    args.retain(|a| link::parse(&a.to_string_lossy()).is_none());
    if link.is_some_and(link::forward) {
        return;
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: taiko-rs [--auto] [Songs folder | map folder | file.osz | file.osu | taiko-rs://score/<id>]...");
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
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    // Scripted runs (tests) neither take links nor become their handler.
    if std::env::var_os("TAIKO_SHOTS").is_none() && std::env::var_os("TAIKO_INPUT").is_none() {
        let proxy = event_loop.create_proxy();
        link::listen(move |l| {
            let _ = proxy.send_event(UserEvent::Link(l));
        });
        link::register();
    }
    run(cfg, args, auto, link, event_loop);
}

/// The latest crash's report, kept until the next start has offered to copy it.
const CRASH_REPORT: &str = "crash-report.txt";

/// A crash leaves `crash.log` next to the scores (release builds on Windows have no console to
/// print it to), and the report on its own for the next start to offer (`crash_report`). On
/// Windows the report also goes on the clipboard and a message box says to send it in, since
/// the game just vanishes otherwise.
fn log_crashes() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let text = format!(
            "--- taiko.rs crash at unix {at}, build {} ({}, {})\nthread {thread}: {info}\n{}\n",
            update::BUILD.unwrap_or("local"),
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::backtrace::Backtrace::force_capture()
        );
        if let Some(dir) = config::data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = std::io::Write::write_all(&mut f, text.as_bytes());
            }
            if cfg!(panic = "abort") {
                let _ = std::fs::write(dir.join(CRASH_REPORT), &text);
            }
        }
        default(info);
        // Only where a panic ends the game (release builds): a debug build's caught one isn't a crash.
        #[cfg(all(windows, panic = "abort"))]
        {
            // A clipboard of its own: the game's may be what was held when it crashed.
            let copied = arboard::Clipboard::new().and_then(|mut b| b.set_text(text.clone())).is_ok();
            let what = if copied { "The crash log is copied: paste it (Ctrl+V)" } else { "Send the crash log (crash.log in the game's data folder)" };
            crash_box(&format!("taiko.rs crashed.\n\n{what} in #bugs on the taiko.rs Discord, so it can be fixed.\n\n{}", info));
        }
    }));
}

/// A message box (Windows), there even without a window of the game's.
#[cfg(all(windows, panic = "abort"))]
fn crash_box(text: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    const MB_ICONERROR: u32 = 0x10;
    const MB_TOPMOST: u32 = 0x4_0000;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, caption) = (wide(text), wide("taiko.rs crashed"));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; no owner window.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR | MB_TOPMOST);
    }
}

/// The report of a crash last time that hasn't been offered yet (and forgets it).
fn crash_report() -> Option<String> {
    let file = config::data_dir()?.join(CRASH_REPORT);
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    Some(text).filter(|t| !t.trim().is_empty())
}

/// Android entry point (the native activity's `android_main`): no command line, the app's own
/// storage for settings and songs.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android::setup(&app);
    let cfg = Config::load();
    let event_loop = EventLoop::<UserEvent>::with_user_event().with_android_app(app).build().expect("event loop");
    android::set_proxy(event_loop.create_proxy());
    run(cfg, Vec::new(), false, None, event_loop);
}

/// Scans the library, opens audio and runs the game until it quits. `args`: songs folders, map
/// folders, .osz or a single .osu to play; empty = the usual song folders. `link`: a score to open.
fn run(cfg: Config, args: Vec<PathBuf>, auto: bool, link: Option<link::Link>, event_loop: EventLoop<UserEvent>) {
    config::set_data_folder(cfg.data_folder.clone());
    library::set_songs_folder(cfg.songs_folder.clone());
    multiverse::set(cfg.universes);
    shapeshift::set(cfg.shapeshift);
    multiverse::set_note_colours(cfg.note_colours);
    let inputs = if args.is_empty() { song_inputs(&cfg) } else { args.clone() };
    let songs_dir = download_dir(&inputs);
    let songs_dirs_for_db: Vec<PathBuf> = inputs.iter().filter(|p| p.is_dir()).cloned().collect();
    let scan = start_scan(inputs);

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

    // A single .osu on the command line goes straight into gameplay.
    let play_when_scanned = args.len() == 1 && args[0].extension().is_some_and(|e| e == "osu");
    // No splash before a map given on the command line, or in scripted runs (TAIKO_SPLASH=1 forces it).
    let scripted = std::env::var_os("TAIKO_SHOTS").is_some() || std::env::var_os("TAIKO_INPUT").is_some();
    let want_splash = std::env::var_os("TAIKO_SPLASH").is_some() || (cfg.splash && !play_when_scanned && !scripted);
    let splash = if want_splash { splash::Splash::new(audio.as_ref().map(|a| a.rate)) } else { None };
    update::cleanup();
    let main_menu = cfg.main_menu && !play_when_scanned && !scripted;
    let guide = !cfg.guide_seen && !play_when_scanned && !scripted;
    let mut state = State::new(cfg, audio, splash, songs_dir, songs_dirs_for_db, Some(scan), play_when_scanned, auto);
    state.main_menu = main_menu.then(menu::Menu::new);
    state.guide = guide.then_some(0);
    state.load_skin();
    if let Some(l) = link {
        state.open_link(l);
    }
    let mut app = App {
        state: Arc::new(Mutex::new(state)),
        cursor: CursorIcon::Default,
        cursors: None,
        cursor_accent: render::DEFAULT_ACCENT,
        cursor_hidden: false,
        window: None,
        ime: false,
        instance: None,
        render_thread: None,
        proxy: event_loop.create_proxy(),
    };
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
