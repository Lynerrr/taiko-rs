//! The game. Desktop builds start it from `main.rs` (`desktop_main`); Android loads this crate
//! as a library and starts it from `android_main`.

// First, so `tr!` is there for every module after it.
#[macro_use]
extern crate taiko_proto;
use taiko_proto::lang;
mod admin;
#[cfg(target_os = "android")]
mod android;
mod app;
mod audio;
mod audiolog;
use taiko_logic::autoplay;
mod avatars;
mod beatmap;
#[cfg(test)]
mod beatmap_tests;
mod browse;
mod calibrate;
mod changelog;
use taiko_proto::changelog_entry;
mod clipboard;
mod collections;
use taiko_logic::clock;
mod config;
mod cursor;
mod custom;
mod debug;
use taiko_logic::difficulty;
mod discord;
mod download;
mod drum_nav;
mod editor;
#[cfg(test)]
mod editor_friends_tests;
#[cfg(test)]
mod editor_test_play_tests;
mod folder_watch;
use taiko_logic::font;
mod game;
use taiko_logic::genres;
mod guide;
mod hit;
mod hud;
mod hud_edit;
mod idle;
mod keys;
mod layout;
#[cfg(test)]
mod layout_tests;
mod library;
mod link;
mod logo;
mod mapguessr;
mod mapinfo;
mod medal_sound;
mod medals;
mod media;
mod menu;
mod menu_notes;
mod mobile;
mod mod_setup;
use taiko_logic::mods;
#[cfg(test)]
mod mods_tests;
mod multiplayer;
use mod_setup::multiverse::LookText as _;
mod multiverse;
use taiko_proto::net;
mod od_override;
mod offline;
use taiko_proto::online;
mod osudb;
#[cfg(test)]
mod perf_audit;
mod pictures;
mod player;
mod playlist;
mod pp;
#[cfg(test)]
mod pp_tests;
mod preview;
mod profile;
#[cfg(test)]
mod regression;
mod render;
#[cfg(test)]
mod results_tests;
mod scores;
mod screens;
use taiko_logic::library::search;
mod services;
mod setstatus;
mod settings;
mod settings_search;
#[cfg(test)]
mod settings_tests;
mod shapeshift;
mod shots;
mod skin;
mod social;
mod splash;
mod stars;
mod store;
mod stream;
mod style;
mod tact;
#[doc(hidden)]
pub mod tko;
mod ui;
mod update;
mod video;
mod volume;
#[cfg(target_os = "linux")]
mod wayland_dnd;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
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
use game::{Game, PAUSE_ITEMS, Play};
use hit::Canvas;
use layout::{Area, Grip};
use library::Entry;
use mods::Mods;
use net::ItemText;
use render::{PresentChoice, Renderer, accent, rgba, themed};
use settings::{Choices, Dropdown, Edit, Item, Kind, PresetAction, SettingsUi};
use ui::{Results, Select, SelectAction};
use winit::window::CursorIcon;

use app::crash::{CRASH_SCENE, SEND_CRASH_REPORTS};
pub use app::gpu::graphics_backends;
use app::input::{ime_has_key, pointer_wake_px};
use app::run_loop::{FileDrag, UserEvent};
pub use app::shell::{desktop_main, recalc_pp};
use audio::sound;
use config::fresh_seed;
use download::{download_text, map_download};
use layout::draw_game_area;
use library::{
    Scanned, download_dir, find_map, find_score_map, load_plays, main_maps, song_inputs, start_scan,
};
use multiplayer::{lobby_seed, map_info, mod_settings, use_mod_settings};
use screens::editor_home::EditorHome;
use screens::hud_editor::HudEdit;
use screens::loading::Loading;
use screens::lobby::Lobby;
use screens::map_editor::MapEditor;
use screens::menu::{update_badge, updating_badge};
use screens::mp_menu::MpMenu;
use screens::mp_results::MpResults;
use screens::mp_wait::MpWait;
use screens::play::{FAIL_RESTART_LINGER, PlayScreen, draw_spectate_all, pause_label, pause_width};
use screens::preview::Preview;
use screens::results::{ResultsScreen, results_job, results_title};
use screens::select::leaderboard::{
    BOARD_HEAD, BOARD_ROW, draw_divider, draw_leaderboard, draw_online_leaderboard, fetch_board,
    misses_and_age,
};
use screens::select::presets::{draw_preset_deleting, draw_preset_naming, preset_key};
use screens::select::preview_chrome::{draw_analysis, draw_preview_chrome};
use screens::select::song_select::SongSelect;
use screens::settings::{
    SettingsScreen, browse_text, built_in_text, enter_to_start, none_text, off_text,
    offset_scale_name, on_text, parse_number, picking_text, scroll_in_select, short_path,
    size_char, type_path,
};
use screens::spectate::{Spectate, SpectateView};
use screens::video_edit::VideoEdit;
use screens::watch::Watch;
use screens::{fmt_time, shows_own_background};
use services::jobs::Job;
#[cfg(not(windows))]
use services::system::file_uri;
use services::system::{map_url, open_url, profile_url};
use video::no_ffmpeg;

type Loaded = Result<(Beatmap, Arc<Track>), String>;

enum Scene {
    /// A screen that lives outside lib.rs (`screens::screen`): song select is
    /// `screens::select::SongSelect`.
    Other(Box<dyn screens::screen::Screen>),
}

/// How slowly a new background comes in on the main menu, where it's the whole picture.
const MENU_BACKGROUND_FADE_MS: f32 = 900.0;

/// How long "Reset map offsets" waits for its second press.
const RESET_CONFIRM: Duration = Duration::from_secs(4);

/// How far the player's arrow keys and wheel move the music.
const MUSIC_SEEK_MS: f64 = 5000.0;

/// How often a running game looks for a newer build (with "Check for updates" on).
const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(30 * 60);

/// How often the open song list asks again which of its custom sets the website approved.
const CUSTOM_REFRESH: Duration = Duration::from_secs(2 * 60);

/// How far behind a watched friend the view plays (room for the network).
const WATCH_DELAY_MS: f64 = 2000.0;

/// Watching a friend's solo play: a replay fed live, a little behind, holding while they pause.
struct WatchView {
    player: String,
    play: Option<online::SoloPlay>,
    entry: Option<usize>,
    rx: Job<Loaded>,
    game: Option<Box<Play>>,
    /// Presses that arrived before the game existed.
    inputs: Vec<(f64, u8)>,
    /// Their map time when last heard, when that was, and whether they're paused.
    clock: (f64, Instant, bool),
    held: bool,
    fetching: bool,
    looked: Option<Instant>,
    /// They stopped playing: their map time at the end and when we heard (we're a little
    /// behind them, so the last seconds are still to be watched before this one is over).
    ending: Option<(f64, Instant)>,
    /// Their next play, started while we were still watching the end of the last.
    next: Option<WatchStart>,
}

/// A friend's play as the server announced it.
struct WatchStart {
    play: online::SoloPlay,
    t: f64,
    paused: bool,
    inputs: Vec<(f64, u8)>,
}

impl WatchView {
    fn new(player: String) -> WatchView {
        WatchView {
            player,
            play: None,
            entry: None,
            rx: Job::idle(),
            game: None,
            inputs: Vec::new(),
            clock: (-1.0e9, Instant::now(), true),
            held: false,
            fetching: false,
            looked: None,
            ending: None,
            next: None,
        }
    }

    /// Starts watching `start`'s play (the map is looked for again).
    fn begin(&mut self, start: WatchStart) {
        self.play = Some(start.play);
        self.inputs = start.inputs;
        self.clock = (start.t, Instant::now(), start.paused);
        self.game = None;
        self.rx.cancel();
        self.entry = None;
        self.held = false;
        self.fetching = false;
        self.looked = None;
        self.ending = None;
        self.next = None;
    }

    /// Where the player is now (map time).
    fn their_time(&self) -> f64 {
        let (t, at, paused) = self.clock;
        let rate = self.play.as_ref().map(|p| p.map.rate).unwrap_or(1.0);
        if paused {
            t
        } else {
            t + at.elapsed().as_secs_f64() * 1000.0 * rate
        }
    }

    fn status(&self) -> String {
        match (&self.play, self.entry) {
            (None, _) => {
                let player = &self.player;
                tr!("waiting for {player} to play something...", "{player}が何かプレイするのを待っています...", "ждём, когда {player} начнёт играть...";)
            }
            (Some(_), None) if self.fetching => {
                tr!("getting the map...", "譜面を取得中...", "получаем карту...").into()
            }
            (Some(_), None) => tr!(
                "looking for the map...",
                "譜面を探しています...",
                "ищем карту..."
            )
            .into(),
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
/// An online leaderboard, or why there isn't one.
type Board = Result<Vec<online::OnlineScore>, String>;
type BoardFetch = ((u64, bool), Receiver<Board>);
type LovedCall = (Receiver<Result<online::Reply, String>>, LovedAsk);

/// What a call about the server's loved maps asked.
#[derive(Clone, Debug, PartialEq)]
enum LovedAsk {
    /// The loved sets and their difficulties.
    Maps,
    /// Just the sets (a server from before single difficulties were loved).
    Sets,
    /// A nominator loves a set (true) or takes it back, or just its difficulty named.
    Change(u32, bool, Option<String>),
}
/// Custom sets known: map folder -> set number, and the numbers the server has approved.
type CustomKnown = (std::collections::HashMap<PathBuf, u32>, Vec<u32>);

/// Everything the input and render threads share. The render thread holds the lock only while
/// building a frame (well under a millisecond) and never across a GPU wait, so key presses are
/// handled — and timestamped — immediately no matter how the swapchain paces frames.
struct State {
    cfg: Config,
    library: Vec<Entry>,
    select: Select,
    scene: Scene,
    /// The game of the song, match or calibration up (`PlayScreen`, `MpPlay`, `Calibration`),
    /// kept while the settings are open over its pause menu. `State::game` reads it.
    play: Option<Box<Play>>,
    /// The calibration run's last note has gone by: it finishes after this frame.
    calibration_over: bool,
    /// The scene the settings were opened over, while they are up (`State::behind_settings`);
    /// closing them goes back to it. Left over when the settings went another way.
    behind: Option<Box<Scene>>,
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
    volume_save_at: Option<Instant>,
    /// The startup splash while it plays.
    splash: Option<splash::Splash>,
    /// The main menu, up over song select (`on_menu`).
    main_menu: Option<menu::Menu>,
    /// A screen just opened fades in: how long it has been showing (ms).
    screen_fade: Option<f32>,
    /// The screen last drawn, to fade in the next one.
    shown_scene: Option<screens::screen::SceneKind>,
    /// What drifts behind it.
    menu_notes: menu_notes::Notes,
    /// When dragging the player's timeline last moved the music.
    music_seeked: Option<Instant>,
    /// The first steps guide: its page, if it's up, and what it waits for.
    guide: guide::Tour,
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
    hits: Vec<hit::HitBox>,
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
    /// When the host was asked for it.
    host_asked_at: Option<Instant>,
    /// The lobby map last fetched by itself: each is tried once.
    auto_fetched: Option<u64>,
    /// Local scores (index of saved replays).
    scores: Vec<scores::ScoreRow>,
    /// Working out the pp of scores saved before pp (started once, on the first frame).
    pp_backfill: Job<Vec<(PathBuf, f64)>>,
    pp_backfill_started: bool,
    /// Plays on their way to the online leaderboards.
    submits: Vec<Receiver<Result<online::Reply, String>>>,
    /// The logged-in player's own pp total and rank online (for the account badge and the
    /// results' total), whose they are, and the fetch of them.
    me: Option<(f64, Option<u32>)>,
    /// The same on the Relax boards, which count apart.
    me_relax: Option<(f64, Option<u32>)>,
    me_for: String,
    me_fetch: Job<Result<online::Profile, String>>,
    /// The song select leaderboard shows the server's scores, not the local ones.
    board_online: bool,
    /// A player's profile, over everything else.
    profile: Option<profile::View>,
    /// The medals screen, over everything but a medal being earned.
    medals: Option<medals::View>,
    /// Medals earned, shown as soon as nothing is playing.
    unlock: medals::Unlock,
    /// What's new, over everything else.
    changelog: Option<changelog::View>,
    /// Update notes being fetched without opening an overlay or intercepting input.
    pending_changelog: Option<changelog::View>,
    /// Online leaderboards fetched (by map hash and whether it's the Relax board: when, and the
    /// rows or why not), and the one being fetched.
    online_boards: std::collections::HashMap<(u64, bool), (Instant, Board)>,
    board_fetch: Option<BoardFetch>,
    /// The replay to attach to the game being loaded.
    pending_replay: Option<scores::Replay>,
    /// Mods of the play being loaded.
    pending_mods: Mods,
    /// The mod select is open: the mod the keyboard is on (index into `mods::ALL`).
    mod_select: Option<usize>,
    /// The mod customization over the mod select: the Multiverse tab or the Shapeshift tab (at
    /// most one is open).
    universe_setup: Option<mod_setup::multiverse::Setup>,
    shapeshift_setup: Option<mod_setup::shapeshift::Setup>,
    random_setup: Option<String>,
    /// The name being typed for a new mod preset (S or "+ Save" in the mod select).
    preset_naming: Option<String>,
    /// The mod preset a right-click asked to delete, waiting for a yes.
    preset_deleting: Option<usize>,
    tact_setup: Option<mod_setup::tact::Setup>,
    od_setup: Option<mod_setup::od_override::Setup>,
    /// Replay files of the leaderboard rows drawn this frame, and the online rows.
    leaderboard: Vec<PathBuf>,
    /// The leaderboard's rows scrolled past, for which map and tab (a new one starts at the
    /// top), how far it goes, and where its rows were drawn (for the wheel).
    board_scroll: usize,
    board_for: (u64, bool, bool),
    board_max: usize,
    board_rect: Option<[f32; 4]>,
    /// A song select divider being dragged: which, where the pointer started (virtual units)
    /// and the size it had then; and the last press on one (a second soon after resets it).
    split_drag: Option<(hit::Hit, f32, f32)>,
    split_click: Option<(hit::Hit, Instant)>,
    online_leaderboard: Vec<online::OnlineScore>,
    /// A video export in progress (`screens::video_edit`).
    export: Option<video::Export>,
    /// When the background image last changed (it fades in).
    bg_changed: Option<Instant>,
    /// A newly loaded skin's note images, for the render thread to upload.
    sprites_upload: Option<Arc<render::Atlas>>,
    /// Profile pictures: fetched as they're drawn, uploaded by the render thread.
    avatars: avatars::Avatars,
    /// The playing or edited `.tko` map's own pictures.
    pictures: pictures::Pictures,
    /// The account logged in is an admin (the server says so; it checks every admin action).
    admin: bool,
    skin_loader: Option<skin::Loader>,
    skin_key: Option<(skin::Key, std::sync::Weak<render::Atlas>)>,
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
    right_press: Option<(f32, f32, Option<hit::Hit>)>,
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
    osudb_rx: Job<Option<osudb::OsuDb>>,
    /// The collections: the player's own, then osu!'s (`collections.rs`).
    collections: Vec<collections::Collection>,
    /// `collections` came from `taiko.db` this run (so writing them back keeps what's there).
    collections_read: bool,
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
    /// Asking the website which custom sets in the library are approved, and when it was last
    /// asked for this library (None: ask now).
    custom_call: Job<CustomKnown>,
    custom_asked: Option<Instant>,
    /// The folder of the map in the editor, looked at for maps dropped into it.
    folder_watch: Option<folder_watch::Watch>,
    /// The editor a test play (F5) came from: the play goes back to it, saves nothing and sends
    /// nothing, and a retry plays the map as the editor has it.
    editor_back: Option<Box<editor::view::View>>,
    /// The editor's test play is played by autoplay (Tab in it), retries included.
    test_auto: bool,
    /// The live play going on (its time played so far), until it's sent to the leaderboards or
    /// left some other way: then it's counted on the server as played (`Request::Played`), with
    /// the map it was on.
    playing: Option<(f64, Option<online::PlayedMap>)>,
    /// Time in songs since the game started (ms), pauses left out.
    session_play_ms: f64,
    /// The key last pressed, as it is on the keyboard (a rebound one arrives as its default),
    /// and when.
    last_key: (KeyCode, Instant),
    /// The quick retry key being held in a song: which key, and since when.
    retry_hold: Option<(KeyCode, Instant)>,
    /// The menu screen showing as of the last frame (`menu_screen`), and when it last changed to
    /// another, for the glitch between them.
    menu_screen_seen: Option<u8>,
    menu_changed_at: Option<Instant>,
    /// How fast the replay being watched is going against how it was played, what's asked for
    /// (the music takes a moment), and that music on its way.
    replay_speed: f64,
    replay_speed_wanted: f64,
    replay_speed_rx: Job<ReplayMusic>,
    /// Song select's selector with the drum in menus (`drum_nav`), and the button rows it
    /// moves along, as drawn last.
    select_zone: drum_nav::Zone,
    select_rows: drum_nav::Rows,
    /// The selected set and since when: asked about once the selection rests.
    status_wait: Option<(u32, Instant)>,
    /// Looking up the lobby map on the mirrors (it had no set ID).
    lobby_lookup: Job<Result<download::SetInfo, String>>,
    /// A score a `taiko-rs://` link asked for, until its results screen opens.
    score_link: Option<ScoreLink>,
    /// Which screen was drawn last and since when, for the fade between screens.
    scene_kind: Option<screens::screen::SceneKind>,
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
    /// When the exit question came up, for its slide in.
    quit_asked_at: Option<Instant>,
    /// The game crashed last time: its report, offered to copy (`draw_crash`).
    crash: Option<String>,
    /// That report is on its way to the server (`send_crash_report`).
    crash_sent: bool,
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
    /// The present modes the renderer can do (`Renderer::present_choices`): the only ones the
    /// settings offer.
    present_modes: Vec<PresentChoice>,
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
    /// Song select is picking a map to open in the editor (from the editor's front door).
    pick_to_edit: bool,
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
    scan: Option<(Receiver<Scanned>, Arc<AtomicUsize>)>,
    /// The list is up from the cache, and the scan is checking it against the disk.
    scan_checking: bool,
    /// The Songs folder dialog while it is open: the folder picked (None = cancelled), or why
    /// there is no dialog.
    folder_pick: Job<Result<Option<PathBuf>, String>>,
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
    /// The phone interface's own state.
    mob: mobile::Ui,
    /// Android: the app is in the background.
    backgrounded: bool,
    /// Android: what the activity was last told about keeping the music alive.
    #[cfg(target_os = "android")]
    music_service: bool,
    /// Android: the screen may turn upright now (the player's up).
    #[cfg(target_os = "android")]
    may_turn: bool,
    /// Android: what the media controls were last told: the song, paused and its length; where
    /// it was then, and when.
    #[cfg(target_os = "android")]
    media_sent: Option<((PathBuf, bool, i64), f64, Instant)>,
    /// The screen's rounded corners and camera: how far in from the sides and from the top and
    /// bottom the interface stays, in pixels. None: right to the edges.
    safe_insets: Option<[u32; 2]>,
    /// When the phone was last asked for them.
    #[cfg(target_os = "android")]
    safe_asked: Option<Instant>,
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
    /// The idle screen, after a while with no input.
    idle: idle::Idle,
    /// Where the playfield size readout was drawn (virtual units), so it can be clicked.
    readout_rect: Option<[f32; 4]>,
    /// Watching a replay: the timeline drawn this frame (virtual units), and when the pointer
    /// dragging along it last seeked.
    replay_bar: Option<[f32; 4]>,
    replay_scrub: Option<Instant>,
    /// How far the replay's seek bar has grown towards its hovered height (0 to 1).
    replay_bar_grow: f32,
    /// The last F12's picture: width, height and RGB, for `/shot`.
    last_shot: Option<Arc<(u32, u32, Vec<u8>)>>,
    /// A screenshot on its way to the server, and who it's for.
    shot_upload: Option<(Receiver<Result<u128, String>>, ShotTo)>,
    /// A screenshot being looked at, over everything.
    shot_view: Option<ShotView>,
    /// Last time the pointer moved (gameplay hides it after a second of stillness).
    mouse_moved_at: Instant,
    /// Where the pointer was at `mouse_moved_at`: a hidden pointer comes back only once it
    /// travels away from here, not on every twitch.
    mouse_moved_from: (f64, f64),
    /// Snap guides to draw while dragging the playfield area.
    guides: Vec<layout::Guide>,
    /// The made-up play the editors show the interface with.
    hud_sample: Option<Box<game::Play>>,
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

/// A press on a song list row (see `State::list_drag`).
struct ListDrag {
    from_y: f32,
    hit: hit::Hit,
    moved: bool,
}

/// How far (virtual units) a held button must move before it counts as dragging, not a click.
const DRAG_START: f32 = 5.0;

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

/// A score a `taiko-rs://` link asked for: being fetched from the server, then waiting for its
/// map (the library scan, or a download) and for the player to be out of a song.
struct ScoreLink {
    fetch: Job<Result<online::OnlineScore, String>>,
    score: Option<online::OnlineScore>,
    /// Its set is downloading, since then.
    downloading: Option<Instant>,
}

/// How long a score link waits for its map's download.
const SCORE_LINK_WAIT: Duration = Duration::from_secs(300);

const AUDIO_BUFFERS: [u32; 6] = [64, 128, 256, 512, 1024, 2048];

/// The smallest the window goes (logical pixels): smaller, the menus no longer fit.
const MIN_WINDOW: (f64, f64) = (640.0, 360.0);

/// Frames on their way to the screen with vsync (the swapchain's frame latency for it, see
/// `Renderer::set_present`).
const VSYNC_FRAMES: f64 = 2.0;

/// Who a screenshot is being sent to.
enum ShotTo {
    Friend(String),
    Lobby,
    Global,
}

/// A chat screenshot opened: being fetched until `rx` has gone.
struct ShotView {
    rx: Job<Result<shots::Pixels, String>>,
}

impl State {
    fn toast(&mut self, text: String) {
        self.toast = Some(Toast {
            text,
            until: Instant::now() + Duration::from_millis(1500),
        });
    }

    /// Scenes that use the gameplay present mode; everything else is a menu.
    /// In a song (or its pause menu's settings): what counts for pp stays as it started.
    fn mid_song(&self) -> bool {
        self.in_song() || self.behind_settings().is_some_and(|prev| prev.plays_song())
    }

    /// Screens that go idle after a while: menus and lists, never a song or an open dialog.
    fn idle_allowed(&self) -> bool {
        (self.scene.is_select() || self.scene.screen().is_some_and(|s| s.idle_allowed()))
            && !self.quit_asked
            && self.crash.is_none()
            && self.splash.is_none()
            && self.settings_ui.binding.is_none()
    }

    /// The idle screen's song: the player's, or the one on this screen (title, artist).
    fn idle_song(&self) -> Option<(String, String)> {
        let entry = match &self.player {
            Some(p) => p.list.entry(),
            None => self.offset_entry(),
        }?;
        let e = self.library.get(entry)?;
        Some((e.meta.title.clone(), e.meta.artist.clone()))
    }

    /// The room's lobby is up.
    fn in_lobby(&self) -> bool {
        self.scene.screen().is_some_and(|s| s.room_lobby())
    }

    fn in_song(&self) -> bool {
        self.scene.screen().is_some_and(|s| s.in_song())
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
        let menu = self
            .game()
            .is_some_and(|game| game.paused().is_some() && game.resuming().is_none());
        let screen = (self.refresh_hz.round() as u32).max(30);
        if menu && (self.cfg.fps_cap == 0 || self.cfg.fps_cap > screen) {
            screen
        } else {
            self.cfg.fps_cap
        }
    }

    /// The present mode this frame should use.
    fn wanted_present(&self) -> PresentChoice {
        // Watching someone else play needs no low input latency: always vsync (smoothest).
        if self.watching() || self.calibration_run() == Some(true) {
            return PresentChoice::Fifo;
        }
        if self.cfg.menu_vsync && !self.player_plays() {
            PresentChoice::Fifo
        } else {
            self.cfg.present
        }
    }

    /// The player is hitting notes (the gameplay present mode's job): not a replay or autoplay,
    /// not waiting for a multiplayer match to start.
    fn player_plays(&self) -> bool {
        match &self.scene {
            Scene::Other(s) if s.song_play().is_some() => self.game().is_some_and(|game| {
                game.replay().is_none() && !self.auto && !game.mods().has(mods::Mod::Autoplay)
            }),
            Scene::Other(s) => s.holds_game(),
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
        self.seen_refresh_ms
            .unwrap_or(1000.0 / self.refresh_hz.clamp(24.0, 1000.0))
    }

    fn stop_audio(&mut self) {
        if let Some(a) = &self.audio {
            a.stop();
        }
        self.preview.invalidate_audio();
    }

    /// The preview's decoded song, when it was prepared at the speed `entry` is about to play at.
    fn cached_track(&self, entry: usize, rate: f64, keep_pitch: bool) -> Option<Arc<Track>> {
        self.preview
            .cached_track(&self.library[entry], rate, keep_pitch)
    }

    /// Speed and pitch of a play started now: the selected speed, or what DT/HT/NC/DC force.
    fn play_speed(&self) -> (f64, bool) {
        self.cfg.mods.speed(self.cfg.rate, self.cfg.rate_keep_pitch)
    }

    /// Loads `entry` for a play at `rate` with `mods` (`keep_pitch` unless NC/DC say otherwise).
    fn load_with(&mut self, entry: usize, rate: f64, mods: Mods) -> Job<Loaded> {
        self.load(entry, rate, mods, true, None)
    }

    /// The map `text` (as the editor has it now, saved or not), with `entry`'s music: a test
    /// play, at full speed and with no mods.
    fn load_text(&mut self, entry: usize, text: String) -> Job<Loaded> {
        // Played through to the end like osu!'s editor test, however it goes; or watched (Tab).
        let mods = Self::test_mods(self.test_auto);
        self.load(entry, 1.0, mods, true, Some(text))
    }

    /// `load_with` for what reads the whole song itself (a video): never a stream.
    fn load_whole(&mut self, entry: usize, rate: f64, mods: Mods) -> Job<Loaded> {
        self.load(entry, rate, mods, false, None)
    }

    fn load(
        &mut self,
        entry: usize,
        rate: f64,
        mods: Mods,
        may_stream: bool,
        text: Option<String>,
    ) -> Job<Loaded> {
        let (rate, keep_pitch) = mods.speed(rate, self.cfg.rate_keep_pitch);
        let cached = self
            .cached_track(entry, rate, keep_pitch)
            .filter(|t| may_stream || t.stream.is_none());
        self.pending_mods = mods;
        if cached.is_none()
            && self.preview.release_long_music()
            && let Some(a) = &self.audio
        {
            a.stop();
        }
        self.spawn_loader(entry, (rate, keep_pitch), mods, cached, may_stream, text)
    }

    /// Speed the preview plays at: the match rate for the lobby map, else the selected rate.
    fn preview_rate(&self) -> (f64, bool) {
        let lobby_rate = self.mp.as_ref().and_then(|s| {
            s.map
                .as_ref()
                .filter(|_| s.map_entry.is_some() && s.map_entry == self.preview_target())
                .map(|m| m.rate)
        });
        let lobby = self
            .behind_settings()
            .is_some_and(|back| back.screen().is_some_and(|s| s.room_lobby()))
            || self
                .scene
                .screen()
                .is_some_and(|s| s.over_lobby() || s.in_room() || s.spectating());
        if let Some(p) = &self.player {
            return p.rate();
        }
        let (rate, keep_pitch) = self.play_speed();
        (lobby_rate.filter(|_| lobby).unwrap_or(rate), keep_pitch)
    }

    /// The playfield area's size in pixels.
    fn playfield_size(&self) -> (u32, u32) {
        let a = self.cfg.playfield_area;
        (
            (a.w * self.window_w).round() as u32,
            (a.h * self.window_h).round() as u32,
        )
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

    /// Maps dropped into (or taken out of) the folder of the map being edited: new ones join the
    /// library, rewritten ones are read again in place (indices stay valid), removed ones leave
    /// the list.
    fn folder_changed(&mut self, news: folder_watch::Changes) {
        let mut added = Vec::new();
        let mut changed = Vec::new();
        let mut unreadable = Vec::new();
        for path in news.changed {
            let Some(meta) = beatmap::read_meta_file(&path) else {
                unreadable.push(path);
                continue;
            };
            let entry = Entry::new(path.clone(), meta);
            match self.library.iter().position(|e| e.path == path) {
                Some(i) => {
                    let old = &self.library[i];
                    if (old.size, old.modified) != (entry.size, entry.modified) {
                        changed.push((i, path.clone(), entry.modified, entry.size));
                        self.library[i] = entry;
                    }
                    self.select.gone.remove(&path);
                    self.select.edited.remove(&path);
                }
                None => added.push(entry),
            }
        }
        for path in news.gone {
            if self.library.iter().any(|e| e.path == path) {
                self.select.gone.insert(path);
            }
        }
        if let Some(info) = &self.mapinfo {
            info.extend(changed);
            self.mapinfo_sorted = false;
        }
        added.sort_by(|a, b| a.path.cmp(&b.path));
        let paths: Vec<PathBuf> = added.iter().map(|e| e.path.clone()).collect();
        let n = paths.len();
        self.append_maps(added);
        if n > 0 {
            // Listed in song select, which stays where the player left it.
            self.select.pin(&self.library, paths);
            self.toast(if n == 1 {
                tr!(
                    "picked up 1 new map from the song folder",
                    "曲フォルダから譜面を1つ追加しました",
                    "из папки песни подхвачена 1 новая карта"
                )
                .to_string()
            } else {
                tr!("picked up {n} new maps from the song folder", "曲フォルダから譜面を{n}個追加しました", "из папки песни подхвачено новых карт: {n}";)
            });
        }
        for path in unreadable {
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.toast(tr!("{file} isn't a taiko map the game can read", "{file}は読み込めない譜面です", "{file} — не читается как карта";));
        }
        self.select.refilter(&self.library);
    }

    /// The editor saved its map under a new file name: the library entry (and the editor's
    /// recent maps) follow it, so the folder watch reads it again in place instead of listing a
    /// new map and a missing one.
    fn map_renamed(&mut self, from: &Path, to: PathBuf) {
        if let Some(e) = self.library.iter_mut().find(|e| e.path == from) {
            e.path = to.clone();
        }
        self.select.gone.remove(from);
        let (from, to) = (
            from.to_string_lossy().into_owned(),
            to.to_string_lossy().into_owned(),
        );
        if self.cfg.editor_recent.contains(&from) {
            for p in &mut self.cfg.editor_recent {
                if *p == from {
                    p.clone_from(&to);
                }
            }
            self.cfg.save();
        }
    }

    /// The library entry of a map file; one that isn't in the library (a map made just now, a file
    /// dropped from elsewhere) is added to it.
    fn entry_for(&mut self, path: &Path) -> Option<usize> {
        if let Some(i) = self.library.iter().position(|e| e.path == path) {
            return Some(i);
        }
        let meta = beatmap::read_meta_file(path)?;
        let start = self.library.len();
        self.append_maps(vec![library::Entry::new(path.to_path_buf(), meta)]);
        (self.library.len() > start).then_some(start)
    }

    /// Opens the map editor on library entry `entry`.
    fn open_editor(&mut self, entry: usize) {
        let path = self.library[entry].path.clone();
        self.open_editor_at(path, entry);
    }

    fn open_editor_at(&mut self, path: PathBuf, entry: usize) {
        match editor::view::View::open(path.clone(), entry) {
            Ok(mut view) => {
                self.cfg.editor_opened(&path);
                self.cfg.save();
                self.stop_audio();
                self.preview.show_background_for(&self.library[entry]);
                let (rx, rate) = self.spawn_track_loader(entry, 1.0);
                view.set_track_loader(rx, rate);
                view.left_pan = self.cfg.editor_left_pan;
                view.ed.follow_timing = self.cfg.editor_follow_timing;
                view.set_snap_presets(&self.cfg.editor_snap_presets, &self.cfg.editor_snap_active);
                // `TAIKO_EDIT_AT=<ms>`: starts with the playhead there (headless screenshots).
                if let Some(at) = std::env::var("TAIKO_EDIT_AT")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
                {
                    view.set_time(at);
                }
                // The first time, a short tour of what is where.
                if !self.cfg.editor_tour_done {
                    view.start_tour();
                }
                self.main_menu = None;
                self.scene = MapEditor::scene(Box::new(view));
            }
            Err(e) => self.toast(format!("can't open the map: {e}")),
        }
    }

    /// A new, empty difficulty beside `entry`'s map (same timing, music and storyboard), opened.
    fn new_difficulty(&mut self, entry: usize) {
        let path = self.library[entry].path.clone();
        match editor::new_difficulty(&path) {
            Ok(new) => {
                self.toast(format!(
                    "made {}",
                    new.file_name().and_then(|n| n.to_str()).unwrap_or("it")
                ));
                self.open_editor_at(new, entry);
            }
            Err(e) => self.toast(format!("can't make a difficulty: {e}")),
        }
    }

    /// Decodes the music of `entry`'s map for the editor at `speed` (1.0 = normal), without
    /// needing the map to have any notes. Returns the loader and the engine's sample rate.
    fn spawn_track_loader(
        &self,
        entry: usize,
        speed: f64,
    ) -> (Receiver<editor::view::TrackResult>, f64) {
        let path = self.library[entry].path.clone();
        let rate = self.audio.as_ref().map(|a| a.rate);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let meta = beatmap::read_meta_file(&path).ok_or("can't read the map")?;
                let audio_path = path.parent().unwrap_or(&path).join(&meta.audio_file);
                match rate {
                    Some(sr) => {
                        let track = audio::decode_shared(&audio_path, sr)?;
                        Ok(audio::with_rate(track, sr, speed, true))
                    }
                    None => Ok(Arc::new(Track {
                        samples: Vec::new(),
                        rate: 1.0,
                        stream: None,
                    })),
                }
            })();
            let _ = tx.send(result);
        });
        (rx, rate.map_or(48000.0, |r| r as f64))
    }

    fn editor_keys(&self) -> editor::view::Keys {
        editor::view::Keys {
            shift: self.modifiers_shift,
            ctrl: self.modifiers_ctrl,
            alt: self.modifiers_alt,
        }
    }

    /// Render thread: the shapes of the next view of a video frame (one, or several blended
    /// with motion blur); None when there's nothing (more) to render.
    fn export_frame(&mut self, c: &mut Canvas) -> Option<ExportView> {
        let scroll = self.cfg.scroll_speed;
        let kiai = self.cfg.kiai_effects;
        let brightness = if self.preview.has_background {
            1.0 - self.cfg.bg_dim
        } else {
            0.0
        };
        let ex = self.export.as_mut()?;
        if ex.frame >= ex.frames {
            // Closing ffmpeg's input lets it finish the file.
            ex.encoder.tx = None;
            return None;
        }
        let tx = ex.encoder.tx.clone()?;
        let (w, h) = ex.size;
        ex.game.view.kiai_effects = kiai;
        let t = ex.game.speed().map(ex.view_time());
        ex.game.set_time(t);
        ex.game.view.hud = self.cfg.hud;
        ex.game.view.hidden = self.cfg.hud.off;
        c.clear(render::VIRTUAL_HEIGHT * w as f32 / h as f32);
        ex.game.draw_playfield(c, scroll);
        if ex.settings.hud {
            ex.game.draw_hud(c);
        }
        let blended = ex.views() > 1;
        let last = ex.advance();
        Some(ExportView {
            size: (w, h),
            tx,
            brightness,
            clear: self.backdrop(true),
            blended,
            last,
        })
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
            .or_else(|| {
                self.library
                    .iter()
                    .position(|e| e.label() == r.map_label && e.hash() == r.map_hash)
            });
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
        self.scene = Loading::scene(rx, label, entry, None);
    }

    /// Song select's layout as it comes: the list's share.
    fn song_layout_is_default(&self) -> bool {
        let d = Config::default();
        (self.cfg.song_list_share - d.song_list_share).abs() < 1e-4
    }

    /// Puts song select's layout back where it was at first.
    fn reset_song_layout(&mut self) {
        let d = Config::default();
        self.cfg.song_list_share = d.song_list_share;
        self.cfg.song_preview_size = d.song_preview_size;
        self.cfg.save();
    }

    /// The results screen of a local score on the selected map (its leaderboard row).
    fn open_local_score(&mut self, file: &std::path::Path) {
        let Some(entry) = self.select.selected() else {
            return;
        };
        let r = match scores::load_replay(file) {
            Ok(r) => r,
            Err(e) => {
                self.toast(tr!("can't open the replay: {e}", "リプレイを開けません: {e}", "не удаётся открыть повтор: {e}";));
                return;
            }
        };
        let stats = game::Stats {
            great: r.great,
            ok: r.ok,
            miss: r.miss,
            max_combo: r.max_combo,
            score: r.score,
            ..Default::default()
        };
        let mods = Mods::from_list(&r.mods);
        let e = &self.library[entry];
        let mut results = Results::new(
            results_title(&e.meta, None),
            stats,
            game::Windows::new(e.meta.od),
        );
        results.set_mods(mods, r.rate);
        results.seed = Some(r.seed);
        results.replay = Some(file.to_path_buf());
        results.partial = true;
        results.pp = self
            .scores
            .iter()
            .find(|row| row.file == file)
            .and_then(|row| row.pp);
        results.no_pp = mods.unranked().then_some(tr!(
            "Autoplay gets no pp",
            "Autoplayではppは付きません",
            "Autoplay не даёт pp"
        ));
        results.opened = Some(ui::Opened {
            player: r.player.clone(),
            date: r.date,
            online: false,
        });
        let all = scores::top(&self.scores, r.map_hash, usize::MAX, mods.relax());
        results.rank = all
            .iter()
            .position(|row| row.file == file)
            .map(|p| (p + 1, all.len()));
        results.job = Some(results_job(
            e.path.clone(),
            r.pp_play(),
            Some(file.to_path_buf()),
            None,
        ));
        self.scene = ResultsScreen::scene(Box::new(results), entry, None);
    }

    /// Opens what a `taiko-rs://` link asks for: a score's results, its replay ready to watch.
    fn open_link(&mut self, link: link::Link) {
        self.main_menu = None;
        match link {
            link::Link::Score(id) => {
                let server = self.cfg.online_server.clone();
                let fetch = Job::spawn(move || {
                    match online::call(&server, online::Request::Score { id }) {
                        Ok(online::Reply::Score(s)) => Ok(*s),
                        Ok(_) => Err(crate::lang::odd_reply()),
                        Err(e) if e == "bad request" => Err(tr!(
                            "the server can't open score links yet",
                            "このサーバーはまだスコアのリンクを開けません",
                            "сервер пока не открывает ссылки на рекорды"
                        )
                        .to_string()),
                        Err(e) => Err(e),
                    }
                });
                self.score_link = Some(ScoreLink {
                    fetch,
                    score: None,
                    downloading: None,
                });
                let later = if self.in_song() || self.mp.is_some() {
                    tr!(
                        " (it opens once you're back in song select)",
                        " (選曲に戻ると開きます)",
                        " (откроется, когда вернётесь к выбору песни)"
                    )
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
        let Some(l) = &mut self.score_link else {
            return;
        };
        if l.fetch.is_busy() {
            match l.fetch.poll_ended() {
                Some(Some(Ok(s))) => l.score = Some(s),
                Some(Some(Err(e))) => {
                    self.score_link = None;
                    self.toast(tr!("can't open that score: {e}", "そのスコアを開けません: {e}", "не удаётся открыть рекорд: {e}";));
                    return;
                }
                None => return,
                Some(None) => {
                    self.score_link = None;
                    return;
                }
            }
        }
        let free = (self.scene.is_select()
            || self
                .scene
                .screen()
                .is_some_and(|s| s.song_select() || s.opens_score_links()))
            && self.mp.is_none();
        if self.scan.is_some() || !free {
            return;
        }
        let Some(score) = self.score_link.as_ref().and_then(|l| l.score.clone()) else {
            return;
        };
        if let Some(entry) = find_score_map(&self.library, &score) {
            self.score_link = None;
            self.scene = SongSelect::scene();
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
                if since.elapsed() > SCORE_LINK_WAIT
                    && !self
                        .downloads
                        .iter()
                        .any(|j| j.set.is_osu_set(score.set_id))
                {
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
                let song = score
                    .map
                    .rsplit_once(" [")
                    .map_or(score.map.as_str(), |(s, _)| s);
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
                    custom: None,
                });
                if let Some(l) = &mut self.score_link {
                    l.downloading = Some(Instant::now());
                }
                self.toast(tr!("downloading {song} for that score...", "そのスコアのため{song}をダウンロード中...", "скачиваем {song} для этого рекорда...";));
            }
            (None, None) => {
                self.score_link = None;
                let map = &score.map;
                self.toast(
                    tr!("you haven't got {map}", "{map}を持っていません", "у вас нет {map}";),
                );
            }
        }
    }

    /// The results screen of an online score on the selected map; its replay is fetched
    /// meanwhile.
    fn open_online_score(&mut self, score: online::OnlineScore) {
        let Some(entry) = self.select.selected() else {
            return;
        };
        self.open_online_score_on(entry, score);
    }

    /// The results screen of an online score on map `entry`.
    fn open_online_score_on(&mut self, entry: usize, score: online::OnlineScore) {
        let stats = game::Stats {
            great: score.great,
            ok: score.ok,
            miss: score.miss,
            max_combo: score.max_combo,
            score: score.score,
            ..Default::default()
        };
        let mods = Mods::from_list(&score.mods);
        let e = &self.library[entry];
        let mut results = Results::new(
            results_title(&e.meta, None),
            stats,
            game::Windows::new(e.meta.od),
        );
        results.set_mods(mods, score.rate);
        results.seed = score.seed;
        results.partial = true;
        results.pp = score.pp;
        results.no_pp = mods.unranked().then_some(tr!(
            "Autoplay gets no pp",
            "Autoplayではppは付きません",
            "Autoplay не даёт pp"
        ));
        results.opened = Some(ui::Opened {
            player: score.player.clone(),
            date: score.date,
            online: true,
        });
        let play = pp::Play {
            great: score.great,
            ok: score.ok,
            miss: score.miss,
            max_combo: score.max_combo,
            rate: score.rate,
            mods,
            scroll_speed: score.scroll_speed.unwrap_or(1.0),
            seed: score.seed.unwrap_or(0),
            half: mods::HalfRule::of_pp_version(score.pp_version),
        };
        let online = (self.cfg.online_server.clone(), score, e.label());
        results.job = Some(results_job(e.path.clone(), play, None, Some(online)));
        self.scene = ResultsScreen::scene(Box::new(results), entry, None);
    }

    fn start_loading_from(&mut self, entry: usize, from: Option<f64>) {
        self.pending_replay = None;
        // A test play from the editor plays the editor's map and isn't counted.
        let test = self.editor_back.as_ref().map(|v| v.ed.chart.to_text());
        if test.is_none() {
            self.record_play(entry);
        }
        self.stop_audio();
        self.preview.show_background_for(&self.library[entry]);
        let mut label = self.library[entry].label();
        audiolog::line(&format!(
            "play {label} audio={} global_offset={} map_offset={} visual_offset={} present={} fps_cap={}",
            self.library[entry].meta.audio_file,
            self.cfg.offset_ms,
            self.map_offset(Some(entry)),
            self.cfg.visual_offset_ms,
            self.cfg.present.name(),
            self.cfg.fps_cap
        ));
        if let Some(t) = from {
            let at = fmt_time(t);
            label.push_str(&tr!("  (from {at})", "  ({at}から)", "  (с {at})";));
        }
        let rx = match test {
            Some(text) => {
                label = format!("TEST  {label}");
                self.load_text(entry, text)
            }
            None => self.load_with(entry, self.cfg.rate, self.cfg.mods),
        };
        self.scene = Loading::scene(rx, label, entry, from);
    }

    /// Plays the map again, like starting it except that the intro is skipped.
    fn retry_from(&mut self, entry: usize, from: Option<f64>) {
        self.start_loading_from(entry, from);
        if let Some(l) = self.scene.screen_mut::<Loading>() {
            l.retry = true;
        }
    }

    /// Leaves the editor for a test play of `text` (its map as it is now) from `from`.
    fn start_test_play(&mut self, entry: usize, from: Option<f64>) {
        if entry >= self.library.len() {
            if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>() {
                view.status = "this map isn't in your songs yet: save it first".into();
            }
            return;
        }
        let Some(editor) = self.scene.take_screen::<MapEditor>() else {
            return;
        };
        self.editor_back = Some(editor.view);
        self.test_auto = false;
        self.start_loading_from(entry, from);
    }

    /// The editor's test play's mods: No Fail, or Autoplay (Tab).
    fn test_mods(auto: bool) -> Mods {
        let mut mods = Mods::default();
        mods.toggle(if auto {
            mods::Mod::Autoplay
        } else {
            mods::Mod::NoFail
        });
        mods
    }

    fn test_banner(auto: bool) -> String {
        if auto {
            "TEST PLAY  -  AUTO".into()
        } else {
            "TEST PLAY".into()
        }
    }

    /// Tab in the editor's test play, as in osu!'s: autoplay plays on from here, or hands the
    /// drum back.
    fn toggle_test_auto(&mut self) {
        let (true, Some(game)) = (self.scene.is_play(), self.play.as_deref_mut()) else {
            return;
        };
        self.test_auto = !self.test_auto;
        game.set_mods(Self::test_mods(self.test_auto));
        game.view.banner = Some(Self::test_banner(self.test_auto));
    }

    /// Back to the editor a test play came from, as it was left.
    fn back_to_editor(&mut self) -> bool {
        let Some(view) = self.editor_back.take() else {
            return false;
        };
        self.test_auto = false;
        if let Some(a) = &self.audio {
            a.stop();
        }
        self.preview.invalidate_audio();
        self.scene = MapEditor::scene(view);
        true
    }

    /// The map the preview shows: the lobby map in the lobby (and its preview), else the song
    /// select selection.
    fn preview_target(&self) -> Option<usize> {
        if let Some(entry) = self
            .player
            .as_ref()
            .and_then(|p| p.list.entry())
            .filter(|_| self.on_menu())
        {
            return Some(entry);
        }
        let lobby = match &self.scene {
            Scene::Other(s) if s.settings() => self
                .behind
                .as_deref()
                .is_some_and(|back| back.screen().is_some_and(|s| s.room_lobby())),
            Scene::Other(s) => s.over_lobby() || s.in_room() || s.spectating(),
        };
        let lobby_map = self.mp.as_ref().and_then(|s| s.map_entry).filter(|_| lobby);
        lobby_map.or_else(|| self.select.selected())
    }

    fn apply_volume(&self) {
        if let Some(a) = &self.audio {
            let (music, fx) = self.heard_volumes();
            a.set_volume(music, fx);
        }
    }

    /// Music and hitsound volume as played now: lower while another window has focus.
    fn heard_volumes(&self) -> (f32, f32) {
        let (music, fx) = self.cfg.volumes();
        // A phone's other apps aren't beside it: the music plays on at its own volume.
        let left = if self.focused || mobile::active() {
            1.0
        } else {
            self.cfg.unfocused_volume
        };
        (music * left, fx * left)
    }

    /// Whether the music may carry on with the app in the background: anywhere but a song.
    fn music_goes_on(&self) -> bool {
        !self.in_song() && !self.loading_song()
    }

    /// A song is being loaded to be played.
    fn loading_song(&self) -> bool {
        self.scene.screen().is_some_and(|s| s.loads_song())
    }

    /// Android: the app went to the background (or came back). The menus' music and the player
    /// play on (the render thread keeps them going, `background_tick`); a song holds, having
    /// already paused itself when focus went, and stays paused until you resume it.
    fn set_background(&mut self, background: bool) {
        self.backgrounded = background;
        let Some(a) = &self.audio else { return };
        let game_paused = self.scene.is_play() && self.game().is_some_and(|g| g.paused().is_some());
        if background {
            if !self.music_goes_on() {
                a.set_paused(true);
            }
        } else if !game_paused && !self.music_paused {
            a.set_paused(false);
        }
    }

    /// Parses the map and decodes its audio on a background thread.
    /// `speed`: rate modifier to prepare the track for (time-stretched or resampled once here).
    /// Loads a map and its song prepared at `speed` (plus Wind Up / Down from `mods`).
    fn spawn_loader(
        &self,
        entry: usize,
        (speed, keep_pitch): (f64, bool),
        mods: Mods,
        cached: Option<Arc<Track>>,
        may_stream: bool,
        text: Option<String>,
    ) -> Job<Loaded> {
        let path = self.library[entry].path.clone();
        let rate = self.audio.as_ref().map(|a| a.rate);
        Job::spawn(move || {
            (|| {
                let map = match &text {
                    Some(text) => beatmap::from_text(text)?,
                    None => beatmap::load(&path)?,
                };
                let audio_path = path.parent().unwrap_or(&path).join(&map.meta.audio_file);
                let speed = game::speed_for(&map, mods, speed);
                // A track prepared at one constant speed can't serve a changing one.
                let cached = cached.filter(|_| speed.is_constant());
                let track = match (cached, rate) {
                    (Some(track), _) => track,
                    (None, Some(rate)) => {
                        // A long song plays as it's decoded: held whole it's gigabytes.
                        let streamed = if may_stream {
                            stream::open(&audio_path, rate, speed, keep_pitch)?
                        } else {
                            None
                        };
                        if let Some(track) = streamed {
                            return Ok((map, track));
                        }
                        // Shared with the preview's full decode if that is still running.
                        let track = audio::decode_shared(&audio_path, rate)?;
                        if speed.is_constant() {
                            audio::with_rate(track, rate, speed.r0, keep_pitch)
                        } else {
                            Arc::new(audio::change_speed(
                                audio::owned(track),
                                rate,
                                &speed,
                                keep_pitch,
                            ))
                        }
                    }
                    (None, None) => Arc::new(Track {
                        samples: Vec::new(),
                        rate: 1.0,
                        stream: None,
                    }),
                };
                Ok((map, track))
            })()
        })
    }

    fn show_select(&mut self) {
        // Leaving a test play goes back to the editor it came from.
        if self.back_to_editor() {
            return;
        }
        self.stop_audio();
        self.report_smoothness();
        self.scene = SongSelect::scene();
    }

    fn report_smoothness(&mut self) {
        if let (Some(sm), true) = (&mut self.smooth, self.scene.is_play()) {
            eprintln!(
                "--- smoothness ({}) ---\n{}",
                self.present.name(),
                sm.report()
            );
            *sm = debug::Smoothness::default();
        }
    }

    fn refresh_status(&mut self) {
        let audio_desc = self
            .audio
            .as_ref()
            .map(|a| a.description.clone())
            .unwrap_or_else(|| tr!("no audio", "音声なし", "нет звука").into());
        let config = config::config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| tr!("not saved", "保存されません", "не сохраняется").into());
        let (adapter, hz) = (&self.adapter, self.refresh_hz);
        self.status = tr!("{adapter}  |  {audio_desc}  |  {hz:.0} Hz  |  config: {config}", "{adapter}  |  {audio_desc}  |  {hz:.0} Hz  |  設定: {config}", "{adapter}  |  {audio_desc}  |  {hz:.0} Гц  |  конфиг: {config}";);
    }

    fn open_settings(&mut self) {
        // Never over themselves (nothing opens them there).
        if self.in_settings() {
            return;
        }
        let prev = std::mem::replace(&mut self.scene, SongSelect::scene());
        self.behind = Some(Box::new(prev));
        self.scene = SettingsScreen::scene();
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

    fn reset_is_armed(&self) -> bool {
        self.reset_armed
            .is_some_and(|at| at.elapsed() < RESET_CONFIRM)
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
        if self.in_settings() {
            self.settings_ui.dropdown = None;
            self.settings_ui.choices = None;
            self.close_settings();
            return;
        }
        match &mut self.scene {
            Scene::Other(s) if s.song_play().is_some() => {
                if let Some(game) = self.play.as_deref_mut() {
                    game.pause(sound(&self.audio));
                }
            }
            Scene::Other(sel) if sel.select_screen() => {}
            Scene::Other(s) if s.allows_settings() => {}
            _ => {
                self.toast(
                    tr!(
                        "settings can't open here",
                        "ここでは設定を開けません",
                        "здесь настройки не открыть"
                    )
                    .into(),
                );
                return;
            }
        }
        self.changelog = None;
        self.profile = None;
        self.medals = None;
        self.social.close();
        self.mod_select = None;
        self.open_settings();
    }

    /// The main menu is what's showing.
    /// Which menu screen is showing, if one is. Songs, loading and the editors aren't: going into
    /// or out of them never glitches, so nothing gets in the way of a play.
    fn menu_screen(&self) -> Option<u8> {
        match self.scene {
            Scene::Other(ref sel) if sel.select_screen() && self.on_menu() => Some(0),
            Scene::Other(ref sel) if sel.select_screen() => Some(1),
            Scene::Other(ref s) => s.menu_screen(),
        }
    }

    /// Glitches the whole screen for a moment when one menu screen gives way to another.
    fn glitch_between_menus(&mut self, canvas: &mut Canvas, now: Instant) {
        let screen = self.menu_screen();
        let was = std::mem::replace(&mut self.menu_screen_seen, screen);
        if was.is_some() && screen.is_some() && was != screen {
            self.menu_changed_at = Some(now);
        }
        if !self.cfg.menu_glitch || !self.cfg.animations {
            return;
        }
        let ms = self
            .menu_changed_at
            .map(|at| now.saturating_duration_since(at).as_secs_f32() * 1000.0);
        canvas.glitch = ms.and_then(menu::screen_glitch);
    }

    fn on_menu(&self) -> bool {
        self.main_menu.is_some()
            && self.scene.is_select()
            && self.mp.is_none()
            && !self.picking_lobby_map
    }

    fn menu_key(&mut self, code: KeyCode) {
        if self.modifiers_ctrl || self.main_menu.as_ref().is_none_or(menu::Menu::is_leaving) {
            return;
        }
        // Rim keys on the outside, like on the drum.
        let drum = self.cfg.keys.iter().position(|&k| k == code);
        // The player's own drum keys play the drum: the rims outside, the face inside.
        if let (Some(k), Some(m)) = (drum, self.main_menu.as_mut()) {
            m.drum(k == 0 || k == 3, if k < 2 { -1 } else { 1 });
        }
        match (code, drum) {
            (KeyCode::Escape, _) => self.ask_quit(),
            (KeyCode::ArrowLeft, _) | (_, Some(0)) => self.menu_step(-1),
            (KeyCode::ArrowRight, _) | (_, Some(3)) => self.menu_step(1),
            (KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space, _) | (_, Some(1 | 2)) => {
                self.menu_hit()
            }
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
                _ => filter.extend(
                    text.into_iter()
                        .flat_map(str::chars)
                        .filter(|c| !c.is_control()),
                ),
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
            (KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space, _) | (_, Some(1 | 2)) => {
                self.music_control(player::Control::Toggle)
            }
            _ => {}
        }
    }

    fn music_control(&mut self, control: player::Control) {
        match control {
            player::Control::Prev => self.music_step(-1),
            player::Control::Next => self.music_step(1),
            player::Control::Toggle => self.set_music_paused(!self.music_paused),
            player::Control::Open => {
                self.player = Some(Box::new(player::Player::new(
                    &self.library,
                    self.select.selected(),
                )));
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
            self.select.favourites = p.list.favourites.clone();
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
        let Some((p, media)) = self.player.as_ref().zip(self.media.as_ref()) else {
            return;
        };
        let state = (p.list.now(), !self.music_paused);
        if self.media_told != Some(state) {
            if self.media_told.is_none_or(|t| t.0 != state.0)
                && let Some(e) = p.list.entry().and_then(|i| self.library.get(i))
            {
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
            self.select
                .jump_songs(-by * self.library.len() as i32, &self.library);
        }
    }

    fn music_seek_by(&mut self, ms: f64) {
        if let Some((at, _)) = self
            .preview
            .music_times(self.audio.as_ref(), Instant::now())
        {
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
        let Some(m) = self.main_menu.as_mut().filter(|m| !m.is_leaving()) else {
            return;
        };
        m.pick(i);
        self.menu_hit();
    }

    fn menu_hit(&mut self) {
        let Some(m) = &mut self.main_menu else { return };
        let sound = match m.hit() {
            menu::Item::Play | menu::Item::Editor => audio::Sfx::BigDon,
            _ => audio::Sfx::Don,
        };
        self.sfx(sound);
    }

    fn sfx(&self, sound: audio::Sfx) {
        if let Some(a) = &self.audio {
            a.sfx(sound);
        }
    }

    /// The medals screen; `name`: whose, or None for just the list.
    fn open_medals(&mut self, name: Option<String>) {
        self.medals = Some(medals::View::open(self.cfg.online_server.clone(), name));
    }

    /// Medals to celebrate; past the few that get the full show, a line says how many more.
    fn medals_earned(&mut self, new: Vec<online::MedalNote>) {
        let more = self.unlock.add(new);
        if more > 0 {
            self.toast(
                tr!("and {more} more medals", "ほか{more}個のメダル", "и ещё медалей: {more}";),
            );
        }
    }

    /// Opens the screen of the note that was hit. Coming back from those lands here again,
    /// except from the songs, which is what the menu is for leaving to.
    fn menu_open(&mut self, item: menu::Item) {
        if !matches!(item, menu::Item::Exit) {
            self.screen_fade = Some(0.0);
        }
        match item {
            menu::Item::Play => self.main_menu = None,
            menu::Item::Multiplayer => self.open_multiplayer(),
            menu::Item::Download => self.open_download(),
            menu::Item::Medals => self.open_medals(self.mp_menu.account.clone()),
            menu::Item::Editor => self.open_editor_home(),
            menu::Item::Settings => self.open_settings(),
            menu::Item::Exit => self.ask_quit(),
        }
    }

    /// Logged in as krokodil_koban (their settings row shows).
    fn is_krokodil(&self) -> bool {
        self.mp_menu
            .account
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case("krokodil_koban"))
    }

    fn close_settings(&mut self) {
        if self.in_settings() {
            self.scene = self.behind.take().map_or(SongSelect::scene(), |prev| *prev);
        }
    }

    /// The settings are up.
    fn in_settings(&self) -> bool {
        self.scene.screen().is_some_and(|s| s.settings())
    }

    /// The scene the settings up were opened over; None when they aren't up.
    fn behind_settings(&self) -> Option<&Scene> {
        self.behind.as_deref().filter(|_| self.in_settings())
    }

    /// The game of the song, match or calibration up; None on any other screen (the settings
    /// over its pause menu included).
    fn game(&self) -> Option<&Play> {
        match &self.scene {
            Scene::Other(s) if s.holds_game() => self.play.as_deref(),
            _ => None,
        }
    }

    /// A calibration run is up: Some(true) the vsync compensation's (by sight).
    fn calibration_run(&self) -> Option<bool> {
        self.scene.screen().and_then(|s| s.calibration())
    }

    /// Drops the game when its scene has gone (the settings over its pause menu keep it).
    fn drop_finished_play(&mut self) {
        let scene = self.behind_settings().unwrap_or(&self.scene);
        let holds = scene.screen().is_some_and(|s| s.holds_game());
        if !holds {
            self.play = None;
        }
    }

    /// A play (or calibration) is running, not paused.
    fn playing(&self) -> bool {
        self.game().is_some_and(|game| game.paused().is_none())
    }

    /// Stars out the swear words already in the chats (the chat filter was just switched on).
    fn censor_chats(&mut self) {
        let lines = self
            .social
            .chats
            .values_mut()
            .flatten()
            .chain(self.social.global.iter_mut());
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
        let safe = self.safe_area();
        self.letterbox = safe.or(layout::letterbox(
            (w as u32, h as u32),
            self.cfg.letterbox_size,
            self.cfg.letterbox_pos,
        )
        .filter(|_| on));
        let (gw, gh) = self
            .letterbox
            .map_or((w, h), |[_, _, gw, gh]| (gw as f32, gh as f32));
        if (gw, gh) != (self.window_w, self.window_h) {
            (self.window_w, self.window_h) = (gw, gh);
            self.refresh_presets();
        }
        self.letterbox.filter(|_| safe.is_none())
    }

    /// The part of the screen clear of its rounded corners and camera (x, y, w, h in pixels, all
    /// even). The same on both sides, so turning the phone over moves nothing. To the game's
    /// input it's a letterbox; the renderer draws the background past it (`Renderer::safe`).
    fn safe_area(&mut self) -> Option<[u32; 4]> {
        #[cfg(target_os = "android")]
        if self
            .safe_asked
            .is_none_or(|at| at.elapsed() > Duration::from_millis(500))
        {
            // Asked again and again: turning the phone upright changes them.
            self.safe_asked = Some(Instant::now());
            if let Some(insets) = android::safe_insets() {
                self.safe_insets = Some(insets);
            }
        }
        let [side, edge] = self.safe_insets?;
        let (w, h) = (self.real_window.0 as u32, self.real_window.1 as u32);
        let (x, y) = (side.next_multiple_of(2), edge.next_multiple_of(2));
        (w > 2 * x + 320 && h > 2 * y + 240).then(|| [x, y, (w - 2 * x) & !1, (h - 2 * y) & !1])
    }

    /// A point of the window as the game sees it: from the game's own corner.
    fn to_game(&self, x: f64, y: f64) -> (f64, f64) {
        self.letterbox
            .map_or((x, y), |[gx, gy, ..]| (x - gx as f64, y - gy as f64))
    }

    /// The window's size in pixels.
    fn window_px(&self) -> (f32, f32) {
        if self.letterbox.is_some() {
            self.real_window
        } else {
            (self.window_w, self.window_h)
        }
    }

    /// The resolutions Letterbox can use: all the monitor reports, custom ones from the driver
    /// too, and the usual ones, grouped by aspect ratio.
    fn letterbox_sizes(&self) -> Vec<(u32, u32)> {
        let (mw, mh) = self.monitor_size;
        let sizes = layout::presets(&self.monitor_modes, mw, mh);
        layout::group_presets(&sizes, self.monitor_size)
            .into_iter()
            .flat_map(|(_, l)| l)
            .collect()
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
            Item::MenuNotesDim => Control::Level(c.menu_notes_dim),
            Item::UnfocusedVolume => Control::Level(c.unfocused_volume),
            Item::DonColour => Control::Swatch(c.note_colours.don()),
            Item::KatColour => Control::Swatch(c.note_colours.kat()),
            Item::Accent | Item::AccentHue => Control::Swatch(c.accent),
            Item::Interface => Control::Switch(!c.interface_hidden),
            _ if item.kind() == Kind::Toggle => {
                Control::Switch(self.setting_value(item) != off_text())
            }
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
            Item::MenuNotesDim => self.cfg.menu_notes_dim = level,
            Item::UnfocusedVolume => self.cfg.unfocused_volume = level,
            _ => {}
        }
        self.apply_volume();
    }

    /// Text typed on the settings screen goes into its search; true if it took the key.
    fn settings_type(&mut self, code: KeyCode, text: Option<&str>) -> bool {
        let ui = &self.settings_ui;
        if ui.choices.is_some()
            || ui.binding.is_some()
            || ui.dropdown.is_some()
            || self.modifiers_ctrl
            || self.modifiers_alt
        {
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
            _ if keys::starts_text(code) || (searching && keys::types_text(code)) => search.extend(
                text.into_iter()
                    .flat_map(str::chars)
                    .filter(|c| !c.is_control()),
            ),
            _ => return false,
        }
        self.search_settings(search);
        true
    }

    fn search_settings(&mut self, text: String) {
        let hotkeys = self.settings_key_names();
        self.settings_ui.set_search(text, &|item| {
            hotkeys
                .iter()
                .find(|(i, _)| *i == item)
                .map_or(String::new(), |(_, name)| name.clone())
        });
    }

    /// Every key setting with the key it's on.
    fn settings_key_names(&self) -> Vec<(Item, String)> {
        settings::ITEMS
            .iter()
            .filter(|(_, item)| item.kind() == Kind::Key)
            .map(|&(_, item)| (item, self.setting_value(item)))
            .collect()
    }

    fn setting_value(&self, item: Item) -> String {
        let c = &self.cfg;
        let on = |b: bool| if b { on_text() } else { off_text() }.to_string();
        match item {
            Item::ScrollSpeed => format!("{:.2}x", c.scroll_speed),
            Item::Offset => format!("{:+.0} ms", c.shown_offset()),
            Item::OffsetScale => offset_scale_name(c.offset_scale).into(),
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
            Item::Hotkey(i) => c
                .hotkeys
                .get(keys::Hotkey::ALL[i])
                .map(keys::Chord::name)
                .unwrap_or_else(|| none_text().into()),
            Item::TouchLayout => c.touch_layout.name().into(),
            Item::AutoplayStyle => c.autoplay.style.name().into(),
            Item::AutoplayBinding => autoplay::BINDINGS[c.autoplay.binding].into(),
            Item::AutoplayLeftHanded => on(c.autoplay.left_handed),
            Item::Screenshots => c.screenshot_to.name().into(),
            Item::IdleScreen => idle::minutes_name(c.idle_minutes),
            Item::Language => c.language.name().into(),
            Item::ShowPlayfieldSize => on(c.show_playfield_size),
            Item::WheelResize => on(c.wheel_resizes_playfield),
            Item::BackgroundDimBlack => on(c.bg_dim_black),
            Item::KiaiEffects => on(c.kiai_effects),
            Item::MenuGlitch => on(c.menu_glitch),
            Item::BlackLane => on(c.black_lane),
            Item::TrueNotes => on(c.true_notes),
            Item::CalmMaps => on(c.calm_maps),
            Item::DrumMenus => on(c.drum_menus),
            Item::MapSkins => on(c.map_skins),
            Item::HideGreat => on(c.hide_great),
            Item::FailRestart => on(c.fail_restart),
            Item::HitFlash => on(c.hit_flash),
            Item::DiscordPresence => on(c.discord_presence),
            Item::CrashReports => on(c.send_crash_reports),
            Item::ChatFilter => on(c.chat_filter),
            Item::MessageSound => on(c.message_sound),
            Item::NotifyPlaying => on(c.notify_playing),
            Item::NotifyOnline => on(c.notify_online),
            Item::NotifyMessages => on(c.notify_messages),
            Item::NotifyMentions => on(c.notify_mentions),
            Item::NotifyRequests => on(c.notify_requests),
            Item::ShowConverts => on(self.select.show_converts),
            Item::SongHeadings => on(self.select.headings),
            Item::HiddenMaps => self.select.hidden.describe(),
            Item::SongsFolder
                if self.folder_pick.is_busy() && self.folder_pick_for == Item::SongsFolder =>
            {
                picking_text().into()
            }
            Item::DataFolder
                if self.folder_pick.is_busy() && self.folder_pick_for == Item::DataFolder =>
            {
                picking_text().into()
            }
            Item::DataFolder if self.data_move.is_some() => {
                tr!("moving...", "移動中...", "перенос...").into()
            }
            Item::DataFolder => match &c.data_folder {
                Some(dir) => short_path(dir),
                None => tr!("the usual places", "いつもの場所", "обычные места").into(),
            },
            Item::SongsFolder if self.scan.is_some() => {
                tr!("scanning...", "スキャン中...", "сканирование...").into()
            }
            Item::SongsFolder => match (&c.songs_folder, self.song_dirs.first()) {
                (Some(dir), _) => short_path(dir),
                (None, Some(dir)) => {
                    let dir = short_path(dir);
                    tr!("found {dir}", "見つけた: {dir}", "найдена {dir}";)
                }
                (None, None) => tr!(
                    "none found, Enter to pick",
                    "見つかりません、Enterで選択",
                    "не найдена, Enter — выбрать"
                )
                .into(),
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
            Item::PresentMode => c.present.within(&self.present_modes).name().to_string(),
            Item::Renderer => {
                // What's drawing now (the adapter's name ends in its API), and a word when the
                // pick waits for a restart.
                let api = self
                    .adapter
                    .rsplit_once('(')
                    .map(|(_, a)| a.trim_end_matches(')').to_ascii_lowercase())
                    .unwrap_or_default();
                let running = match api.as_str() {
                    "vulkan" => "vulkan",
                    "dx12" => "dx12",
                    "gl" => "gl",
                    "metal" => "metal",
                    _ => "",
                };
                let name = config::renderer_name(&c.graphics_api);
                if c.graphics_api == "auto" && !running.is_empty() {
                    format!("{name} ({})", config::renderer_name(running))
                } else if c.graphics_api != "auto"
                    && !running.is_empty()
                    && c.graphics_api != running
                {
                    tr!("{name} (after a restart)", "{name} (再起動後)", "{name} (после перезапуска)";)
                } else {
                    name.to_string()
                }
            }
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
            Item::EpilepsyWarning => on(c.epilepsy_warning),
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
                    (_, None) => tr!(
                        "local build (no updates)",
                        "ローカルビルド (更新なし)",
                        "локальная сборка (без обновлений)"
                    )
                    .into(),
                    (update::Status::Off, Some(b)) => {
                        tr!("{b}, Enter to check", "{b}、Enterで確認", "{b}, Enter — проверить";)
                    }
                    (update::Status::Checking, _) => {
                        tr!("checking...", "確認中...", "проверка...").into()
                    }
                    (update::Status::UpToDate, Some(b)) => {
                        tr!("{b}, up to date", "{b}、最新です", "{b}, последняя";)
                    }
                    (update::Status::Available { commit, .. }, Some(b)) => {
                        let new = update::short(&commit);
                        tr!("{b} -> {new}, Enter to install", "{b} -> {new}、Enterでインストール", "{b} -> {new}, Enter — установить";)
                    }
                    (update::Status::Downloading, _) => {
                        let p = self.updater.progress() * 100.0;
                        tr!("downloading {p:.0}%", "ダウンロード中 {p:.0}%", "загрузка {p:.0}%";)
                    }
                    (update::Status::Installed, _) => {
                        tr!("restarting...", "再起動中...", "перезапуск...").into()
                    }
                    (update::Status::Failed(_), _) => tr!(
                        "failed, Enter to retry",
                        "失敗、Enterで再試行",
                        "ошибка, Enter — ещё раз"
                    )
                    .into(),
                }
            }
            Item::Changelog => tr!("Enter to read", "Enterで読む", "Enter — читать").into(),
            Item::SongSelectLayout if self.song_layout_is_default() => {
                tr!("default", "初期設定", "по умолчанию").into()
            }
            Item::SongSelectLayout => tr!(
                "custom, Enter: default",
                "カスタム、Enter: 初期設定",
                "свой, Enter: по умолчанию"
            )
            .into(),
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
            Item::ResetMapOffsets if self.map_offsets.is_empty() => {
                tr!("none set", "なし", "их нет").into()
            }
            Item::ResetMapOffsets if self.reset_is_armed() => tr!(
                "Enter again to reset",
                "もう一度Enterでリセット",
                "Enter ещё раз — сбросить"
            )
            .into(),
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
            Item::InterfaceHides if c.interface_hides == hud::Parts::ALL => {
                tr!("everything", "すべて", "всё").into()
            }
            Item::InterfaceHides if c.interface_hides == hud::Parts::NONE => {
                tr!("nothing", "なし", "ничего").into()
            }
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
            Item::InterfaceEditor => {
                let builtin = hud::builtin_presets();
                let named = builtin
                    .iter()
                    .find(|p| p.1 == c.hud)
                    .map(|p| p.0.to_string());
                let own = c.hud_presets.iter().find(|p| p.1 == c.hud);
                own.map(|p| p.0.clone())
                    .or(named)
                    .unwrap_or_else(|| tr!("custom", "カスタム", "своя").into())
            }
            Item::PlayfieldPreset => self
                .current_preset()
                .map(|(w, h)| format!("{w}x{h}"))
                .unwrap_or_else(|| tr!("custom", "カスタム", "свой").into()),
            Item::BackgroundDim => format!("{:.0}%", c.bg_dim * 100.0),
            Item::MenuNotesDim => format!("{:.0}%", c.menu_notes_dim * 100.0),
            Item::UnfocusedVolume => format!("{:.0}%", c.unfocused_volume * 100.0),
            Item::PlayfieldReset => {
                tr!("Enter to reset", "Enterで元に戻す", "Enter — сбросить").into()
            }
            Item::Rate => format!("{:.2}x", c.rate),
            Item::RateKeepPitch => on(c.rate_keep_pitch),
            Item::Skin if c.skin.is_empty() => built_in_text().into(),
            Item::Skin => c.skin.clone(),
            Item::SkinParts => {
                let changed = skin::Part::ALL
                    .iter()
                    .filter(|&&p| *c.skin_parts.get(p) != p.default_source())
                    .count();
                if changed == 0 {
                    tr!("notes from the skin", "ノーツはスキンから", "ноты из скина").into()
                } else {
                    tr!("{changed} changed", "{changed}個を変更", "изменено: {changed}";)
                }
            }
            Item::SkinPart(i) => skin::Part::ALL
                .get(i)
                .map(|&p| self.skin_source_name(c.skin_parts.get(p)))
                .unwrap_or_default(),
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
            Item::ScrollSpeed => {
                self.cfg.scroll_speed = config::step_scroll_speed(self.cfg.scroll_speed, d)
            }
            Item::Offset => self.cfg.set_shown_offset(
                self.cfg.shown_offset() + d * if self.modifiers_shift { 5.0 } else { 1.0 },
            ),
            Item::OffsetScale => {
                let scale = match self.cfg.offset_scale {
                    config::OffsetScale::Osu => config::OffsetScale::Sound,
                    config::OffsetScale::Sound => config::OffsetScale::Osu,
                };
                self.set_offset_scale(scale);
                return;
            }
            Item::VisualOffset => {
                self.cfg.visual_offset_ms += d * if self.modifiers_shift { 5.0 } else { 1.0 }
            }
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
                self.cfg.autoplay.binding =
                    (self.cfg.autoplay.binding as i32 + dir).rem_euclid(n) as usize;
            }
            Item::AutoplayLeftHanded => {
                self.cfg.autoplay.left_handed = !self.cfg.autoplay.left_handed
            }
            Item::Screenshots => self.cfg.screenshot_to = self.cfg.screenshot_to.step(dir),
            Item::IdleScreen => {
                let i = idle::MINUTES
                    .iter()
                    .position(|&m| m == self.cfg.idle_minutes)
                    .unwrap_or(2) as i32;
                self.cfg.idle_minutes =
                    idle::MINUTES[(i + dir).rem_euclid(idle::MINUTES.len() as i32) as usize];
            }
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
            Item::AccentHue => {
                self.cfg.accent = render::rotate_hue(self.cfg.accent, 10.0 * d as f32)
            }
            Item::NoteColours => self.cfg.note_colours = self.cfg.note_colours.step(dir),
            Item::DonColour | Item::KatColour => {
                let (don, kat) = (self.cfg.note_colours.don(), self.cfg.note_colours.kat());
                let turn = |c: u32| render::rotate_hue(c, 10.0 * d as f32);
                self.cfg.note_colours = if item == Item::DonColour {
                    multiverse::Colours::own(turn(don), kat)
                } else {
                    multiverse::Colours::own(don, turn(kat))
                };
            }
            Item::ShowPlayfieldSize => self.cfg.show_playfield_size = !self.cfg.show_playfield_size,
            Item::WheelResize => {
                self.cfg.wheel_resizes_playfield = !self.cfg.wheel_resizes_playfield
            }
            Item::BackgroundDimBlack => self.cfg.bg_dim_black = !self.cfg.bg_dim_black,
            Item::KiaiEffects => self.cfg.kiai_effects = !self.cfg.kiai_effects,
            Item::MenuGlitch => self.cfg.menu_glitch = !self.cfg.menu_glitch,
            Item::BlackLane => self.cfg.black_lane = !self.cfg.black_lane,
            Item::TrueNotes => self.cfg.true_notes = !self.cfg.true_notes,
            Item::CalmMaps => self.cfg.calm_maps = !self.cfg.calm_maps,
            Item::DrumMenus => {
                self.cfg.drum_menus = !self.cfg.drum_menus;
                self.select_zone = drum_nav::Zone::List;
            }
            Item::MapSkins => self.cfg.map_skins = !self.cfg.map_skins,
            Item::HideGreat => self.cfg.hide_great = !self.cfg.hide_great,
            Item::FailRestart => self.cfg.fail_restart = !self.cfg.fail_restart,
            Item::HitFlash => self.cfg.hit_flash = !self.cfg.hit_flash,
            Item::DiscordPresence => self.cfg.discord_presence = !self.cfg.discord_presence,
            Item::CrashReports => {
                self.cfg.send_crash_reports = !self.cfg.send_crash_reports;
                SEND_CRASH_REPORTS.store(self.cfg.send_crash_reports, Ordering::Relaxed);
            }
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
            Item::SongHeadings => {
                self.select
                    .set_headings(!self.select.headings, &self.library);
                self.cfg.song_headings = self.select.headings;
                self.cfg.save();
                return;
            }
            // Act on Enter in `settings_key`.
            Item::HiddenMaps | Item::SongsFolder | Item::DataFolder => return,
            Item::MasterVolume => {
                self.cfg.master_volume =
                    (self.cfg.master_volume + 0.05 * dir as f32).clamp(0.0, 1.0)
            }
            Item::MusicVolume => {
                self.cfg.music_volume = (self.cfg.music_volume + 0.05 * dir as f32).clamp(0.0, 1.0)
            }
            Item::FxVolume => {
                self.cfg.fx_volume = (self.cfg.fx_volume + 0.05 * dir as f32).clamp(0.0, 1.0)
            }
            Item::AudioBuffer => {
                let i = AUDIO_BUFFERS
                    .iter()
                    .position(|&b| b >= self.cfg.audio_buffer)
                    .unwrap_or(2);
                self.set_audio_buffer(
                    AUDIO_BUFFERS
                        [(i as i32 + dir).clamp(0, AUDIO_BUFFERS.len() as i32 - 1) as usize],
                );
            }
            Item::PresentMode => self.cfg.present = self.step_present(dir),
            // Each pick restarts the game, so Left/Right open the list rather than step through it.
            Item::Renderer => self.open_choices(item),
            Item::MenuVsync => self.cfg.menu_vsync = !self.cfg.menu_vsync,
            Item::HalfResolution => self.cfg.half_resolution = !self.cfg.half_resolution,
            Item::FpsCap => {
                let i = FPS_CAPS
                    .iter()
                    .position(|&c| c == self.cfg.fps_cap)
                    .unwrap_or(0) as i32;
                self.cfg.fps_cap = FPS_CAPS[(i + dir).rem_euclid(FPS_CAPS.len() as i32) as usize];
            }
            Item::ShowFps => self.cfg.show_fps = !self.cfg.show_fps,
            Item::Animations => self.cfg.animations = !self.cfg.animations,
            Item::Splash => self.cfg.splash = !self.cfg.splash,
            Item::EpilepsyWarning => self.cfg.epilepsy_warning = !self.cfg.epilepsy_warning,
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
                let next = if dir > 0 {
                    sizes
                        .iter()
                        .filter(|s| area(s) > now)
                        .min_by_key(|s| area(s))
                } else {
                    sizes
                        .iter()
                        .filter(|s| area(s) < now)
                        .max_by_key(|s| area(s))
                };
                if let Some(&size) = next {
                    self.cfg.letterbox_size = size;
                }
            }
            Item::LetterboxX => {
                self.cfg.letterbox_pos.0 =
                    (self.cfg.letterbox_pos.0 + 5.0 * dir as f32).clamp(-100.0, 100.0)
            }
            Item::LetterboxY => {
                self.cfg.letterbox_pos.1 =
                    (self.cfg.letterbox_pos.1 + 5.0 * dir as f32).clamp(-100.0, 100.0)
            }
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
                    let near = sizes
                        .iter()
                        .filter(|&&(w, h)| !same || layout::aspect_name(w, h) == aspect)
                        .map(|&(w, h)| ((w * h) as u64, (w, h)));
                    if dir > 0 {
                        near.filter(|&(a, _)| a > area)
                            .min_by_key(|&(a, _)| a)
                            .map(|(_, s)| s)
                    } else {
                        near.filter(|&(a, _)| a < area)
                            .max_by_key(|&(a, _)| a)
                            .map(|(_, s)| s)
                    }
                };
                let next = step(true).or_else(|| step(false));
                let next = next.as_ref();
                match next {
                    Some(&size) => self.window_request = Some(size),
                    None => self.toast(if dir > 0 {
                        tr!(
                            "the largest size that fits this screen",
                            "この画面に収まる最大のサイズです",
                            "это самый большой размер для экрана"
                        )
                        .into()
                    } else {
                        tr!(
                            "the smallest size there is",
                            "これが最小のサイズです",
                            "это самый маленький размер"
                        )
                        .into()
                    }),
                }
                return;
            }
            // Started with Enter in `settings_key`; arrows do nothing.
            Item::Calibrate | Item::ResetMapOffsets | Item::Guide => return,
            Item::LiveOffset => self.cfg.show_offset_hint = !self.cfg.show_offset_hint,
            Item::LiveOffsetUr => self.cfg.offset_hint_ur_scaled = !self.cfg.offset_hint_ur_scaled,
            Item::VsyncCompensation => self.cfg.vsync_compensation = !self.cfg.vsync_compensation,
            Item::VsyncTune => {
                self.cfg.vsync_tune_ms = (self.cfg.vsync_tune_ms
                    + d * if self.modifiers_shift { 5.0 } else { 1.0 })
                .clamp(-200.0, 200.0)
            }
            // Started with Enter in `settings_key`.
            Item::VsyncCalibrate => return,
            Item::ResumeDelay => {
                self.cfg.resume_delay_ms = (self.cfg.resume_delay_ms + 250.0 * d).clamp(0.0, 3000.0)
            }
            // Both act on Enter in `settings_key`.
            Item::PlayfieldArea | Item::InterfaceEditor | Item::PlayfieldReset => return,
            Item::PlayfieldPreset => {
                self.cycle_preset(dir);
                return;
            }
            Item::BackgroundDim => {
                self.cfg.bg_dim =
                    (((self.cfg.bg_dim + 0.05 * dir as f32) * 20.0).round() / 20.0).clamp(0.0, 1.0)
            }
            Item::UnfocusedVolume => {
                self.cfg.unfocused_volume =
                    (((self.cfg.unfocused_volume + 0.05 * dir as f32) * 20.0).round() / 20.0)
                        .clamp(0.0, 1.0)
            }
            Item::MenuNotesDim => {
                self.cfg.menu_notes_dim =
                    (((self.cfg.menu_notes_dim + 0.05 * dir as f32) * 20.0).round() / 20.0)
                        .clamp(0.0, 1.0)
            }
            Item::Rate => {
                let step = 0.05 * if self.modifiers_shift { 5.0 } else { 1.0 };
                self.cfg.rate = (((self.cfg.rate + step * d) * 100.0).round() / 100.0)
                    .clamp(RATE_RANGE.0, RATE_RANGE.1);
            }
            Item::RateKeepPitch => self.cfg.rate_keep_pitch = !self.cfg.rate_keep_pitch,
            Item::Skin => {
                let names: Vec<String> = std::iter::once(String::new())
                    .chain(skin::list().into_iter().map(|(n, _)| n))
                    .collect();
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
            self.toast(
                tr!(
                    "a local build doesn't update itself",
                    "ローカルビルドは自分で更新しません",
                    "локальная сборка не обновляется сама"
                )
                .into(),
            );
        }
    }

    /// Reopens the audio device with a buffer of `frames` (not during a song).
    fn set_audio_buffer(&mut self, frames: u32) {
        if self.behind_settings().is_some_and(|prev| prev.is_play()) {
            self.toast(
                tr!(
                    "change the audio buffer from song select",
                    "オーディオバッファは選曲画面で変えてください",
                    "меняйте аудиобуфер в выборе песни"
                )
                .into(),
            );
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
                (
                    strings(&mut all.iter().map(|l| l.name())),
                    all.iter().position(|&l| l == c.touch_layout),
                    String::new(),
                )
            }
            Item::Language => {
                let all = lang::Lang::ALL;
                (
                    strings(&mut all.iter().map(|l| l.name())),
                    all.iter().position(|&l| l == c.language),
                    String::new(),
                )
            }
            Item::IdleScreen => (
                idle::MINUTES
                    .iter()
                    .map(|&m| idle::minutes_name(m))
                    .collect(),
                idle::MINUTES.iter().position(|&m| m == c.idle_minutes),
                String::new(),
            ),
            Item::Screenshots => {
                let all = config::ScreenshotTo::ALL;
                (
                    strings(&mut all.iter().map(|t| t.name())),
                    all.iter().position(|&t| t == c.screenshot_to),
                    String::new(),
                )
            }
            Item::AutoplayStyle => {
                let all = autoplay::Style::ALL;
                (
                    strings(&mut all.iter().map(|s| s.name())),
                    all.iter().position(|&s| s == c.autoplay.style),
                    String::new(),
                )
            }
            Item::AutoplayBinding => (
                strings(&mut autoplay::BINDINGS.iter().copied()),
                Some(c.autoplay.binding),
                String::new(),
            ),
            Item::Renderer => {
                let all = config::RENDERERS;
                (
                    strings(&mut all.iter().map(|&r| config::renderer_name(r))),
                    all.iter().position(|&r| r == c.graphics_api),
                    tr!(
                        "picking one restarts the game",
                        "選ぶとゲームが再起動します",
                        "выбор перезапустит игру"
                    )
                    .into(),
                )
            }
            Item::PresentMode => {
                let all = &self.present_modes;
                let note = if all.len() == 1 {
                    tr!(
                        "the only one this renderer has",
                        "このレンダラーはこれだけ",
                        "у этого рендера только он"
                    )
                } else {
                    tr!("during songs", "曲の間", "во время песен")
                };
                (
                    strings(&mut all.iter().map(|p| p.name())),
                    all.iter().position(|&p| p == c.present.within(all)),
                    note.into(),
                )
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
                (
                    names,
                    AUDIO_BUFFERS.iter().position(|&b| b == c.audio_buffer),
                    format!("{rate} Hz"),
                )
            }
            Item::OffsetScale => (
                strings(
                    &mut config::OffsetScale::ALL
                        .iter()
                        .map(|&scale| offset_scale_name(scale)),
                ),
                config::OffsetScale::ALL
                    .iter()
                    .position(|&scale| scale == c.offset_scale),
                String::new(),
            ),
            Item::UpdateChannel => {
                let all = [update::Channel::Stable, update::Channel::Dev];
                let running = update::BUILD_CHANNEL.name();
                (
                    strings(&mut all.iter().map(|ch| ch.name())),
                    all.iter().position(|&ch| ch == self.updater.channel()),
                    tr!("running {running}", "実行中: {running}", "запущен {running}";),
                )
            }
            Item::Accent => (
                strings(&mut render::ACCENTS.iter().map(|a| a.0)),
                render::ACCENTS.iter().position(|a| a.1 == c.accent),
                String::new(),
            ),
            Item::WindowSize => {
                let sizes = self.window_sizes();
                let now = (self.window_w as u32, self.window_h as u32);
                let type_size = tr!("Type a size", "サイズを入力", "Ввести размер").to_string();
                let names = std::iter::once(type_size)
                    .chain(
                        sizes
                            .iter()
                            .map(|&(w, h)| format!("{w} x {h}   {}", layout::aspect_name(w, h))),
                    )
                    .collect();
                let current = sizes
                    .iter()
                    .position(|&s| s == now && !self.fullscreen)
                    .map(|i| i + 1);
                let (mw, mh) = self.monitor_size;
                (
                    names,
                    current,
                    tr!("screen {mw}x{mh}", "画面 {mw}x{mh}", "экран {mw}x{mh}";),
                )
            }
            Item::LetterboxSize => {
                let sizes = self.letterbox_sizes();
                let type_size = tr!("Type a size", "サイズを入力", "Ввести размер").to_string();
                let names = std::iter::once(type_size)
                    .chain(
                        sizes
                            .iter()
                            .map(|&(w, h)| format!("{w} x {h}   {}", layout::aspect_name(w, h))),
                    )
                    .collect();
                let current = sizes
                    .iter()
                    .position(|&s| s == c.letterbox_size)
                    .map(|i| i + 1);
                let (mw, mh) = self.monitor_size;
                (
                    names,
                    current,
                    tr!("screen {mw}x{mh}", "画面 {mw}x{mh}", "экран {mw}x{mh}";),
                )
            }
            Item::InterfaceHides => {
                let names = hud::Part::ALL
                    .iter()
                    .map(|&p| {
                        format!(
                            "{}  {}",
                            if c.interface_hides.has(p) {
                                "[x]"
                            } else {
                                "[ ]"
                            },
                            p.label()
                        )
                    })
                    .collect();
                (
                    names,
                    None,
                    tr!(
                        "hidden when toggled off",
                        "オフにした時に隠すもの",
                        "прячется при выключении"
                    )
                    .into(),
                )
            }
            Item::NoteColours => {
                let own = c.note_colours.custom;
                let custom = format!(
                    "{}  #{:06x} / #{:06x}",
                    mod_setup::multiverse::palette_label(multiverse::CUSTOM),
                    own[0],
                    own[1]
                );
                let names = (0..multiverse::PALETTES.len())
                    .map(|i| mod_setup::multiverse::palette_label(i).to_string())
                    .chain(std::iter::once(custom))
                    .collect();
                (
                    names,
                    Some(c.note_colours.palette),
                    tr!("don / kat", "ドン / カッ", "дон / кат").into(),
                )
            }
            Item::SkinParts => {
                let names = skin::Part::ALL
                    .iter()
                    .map(|&p| {
                        format!(
                            "{}:  {}",
                            p.label(),
                            self.skin_source_name(c.skin_parts.get(p))
                        )
                    })
                    .collect();
                (
                    names,
                    None,
                    tr!(
                        "Enter: where it comes from",
                        "Enter: 出どころを選ぶ",
                        "Enter: откуда берётся"
                    )
                    .into(),
                )
            }
            Item::SkinPart(i) => {
                let Some(&part) = skin::Part::ALL.get(i) else {
                    return;
                };
                let skins: Vec<String> = skin::list().into_iter().map(|(n, _)| n).collect();
                let main = if c.skin.is_empty() {
                    built_in_text().to_string()
                } else {
                    c.skin.clone()
                };
                let picked = tr!("The skin picked ({main})", "選んだスキン ({main})", "Выбранный скин ({main})";);
                let names: Vec<String> = [picked, tr!("Built-in", "内蔵", "Встроенный").into()]
                    .into_iter()
                    .chain(skins.iter().cloned())
                    .collect();
                let current = match c.skin_parts.get(part) {
                    skin::Source::Main => Some(0),
                    skin::Source::BuiltIn => Some(1),
                    skin::Source::Skin(n) => skins.iter().position(|s| s == n).map(|k| k + 2),
                };
                (
                    names,
                    current,
                    tr!(
                        "Esc: back to the parts",
                        "Esc: パーツに戻る",
                        "Esc: назад к частям"
                    )
                    .into(),
                )
            }
            Item::Skin => {
                let skins = skin::list();
                let n = skins.len();
                let note = tr!("{n} installed", "{n}個インストール済み", "установлено: {n}";);
                let names: Vec<String> = std::iter::once(built_in_text().to_string())
                    .chain(skins.into_iter().map(|(n, _)| n))
                    .collect();
                let current = if c.skin.is_empty() {
                    Some(0)
                } else {
                    names
                        .iter()
                        .skip(1)
                        .position(|n| *n == c.skin)
                        .map(|i| i + 1)
                };
                (names, current, note)
            }
            Item::SongsFolder => {
                let mut names = vec![
                    type_path(),
                    tr!(
                        "Find osu!'s Songs folder automatically",
                        "osu!のSongsフォルダを自動で探す",
                        "Искать папку Songs osu! самому"
                    )
                    .to_string(),
                ];
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
                let mut names = vec![
                    type_path(),
                    tr!(
                        "Back to the usual places",
                        "いつもの場所に戻す",
                        "Вернуть в обычные места"
                    )
                    .to_string(),
                ];
                if !cfg!(target_os = "android") {
                    names.insert(0, browse_text());
                }
                (
                    names,
                    None,
                    config::data_dir()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default(),
                )
            }
            _ => return,
        };
        self.settings_ui.choices = Some(Choices::new(item, names, current, note));
    }

    /// Uses row `i` of the open list of choices and closes it.
    fn pick_choice(&mut self, i: usize) {
        let Some(list) = self.settings_ui.choices.take() else {
            return;
        };
        let Some(name) = list.names.get(i) else {
            return;
        };
        match list.item {
            Item::TouchLayout => self.cfg.touch_layout = config::TouchLayout::ALL[i],
            Item::Screenshots => self.cfg.screenshot_to = config::ScreenshotTo::ALL[i],
            Item::IdleScreen => self.cfg.idle_minutes = idle::MINUTES[i],
            Item::AutoplayStyle => self.cfg.autoplay.style = autoplay::Style::ALL[i],
            Item::AutoplayBinding => self.cfg.autoplay.binding = i,
            Item::Language => self.cfg.language = lang::Lang::ALL[i],
            Item::PresentMode => {
                self.cfg.present = self
                    .present_modes
                    .get(i)
                    .copied()
                    .unwrap_or(PresentChoice::Fifo)
            }
            Item::Renderer if config::RENDERERS[i] != self.cfg.graphics_api => {
                self.cfg.graphics_api = config::RENDERERS[i].to_string();
                self.restart_for_renderer();
            }
            Item::Renderer => {}
            Item::AudioBuffer if Some(i) != list.current => self.set_audio_buffer(AUDIO_BUFFERS[i]),
            Item::UpdateChannel => {
                self.set_update_channel([update::Channel::Stable, update::Channel::Dev][i])
            }
            Item::OffsetScale => self.set_offset_scale(config::OffsetScale::ALL[i]),
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
                let i = if cfg!(target_os = "android") {
                    i + 1
                } else {
                    i
                };
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
                let i = if cfg!(target_os = "android") {
                    i + 1
                } else {
                    i
                };
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
            Item::Offset => format!("{:.0}", c.shown_offset()),
            Item::VisualOffset => format!("{:.0}", c.visual_offset_ms),
            Item::VsyncTune => format!("{:.0}", c.vsync_tune_ms),
            Item::CalibrationBpm => format!("{:.0}", c.calibration_bpm),
            Item::ResumeDelay => format!("{:.2}", c.resume_delay_ms / 1000.0),
            Item::MasterVolume => format!("{:.0}", c.master_volume * 100.0),
            Item::MusicVolume => format!("{:.0}", c.music_volume * 100.0),
            Item::FxVolume => format!("{:.0}", c.fx_volume * 100.0),
            Item::BackgroundDim => format!("{:.0}", c.bg_dim * 100.0),
            Item::MenuNotesDim => format!("{:.0}", c.menu_notes_dim * 100.0),
            Item::UnfocusedVolume => format!("{:.0}", c.unfocused_volume * 100.0),
            Item::FpsCap => c.fps_cap.to_string(),
            Item::AccentHue => format!("#{:06x}", c.accent),
            Item::DonColour => format!("#{:06x}", c.note_colours.don()),
            Item::KatColour => format!("#{:06x}", c.note_colours.kat()),
            Item::WindowSize => format!("{:.0}x{:.0}", self.window_w, self.window_h),
            Item::SongsFolder => c
                .songs_folder
                .as_ref()
                .or(self.song_dirs.first())
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            Item::DataFolder => config::data_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
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
        let Some(edit) = &mut self.settings_ui.editing else {
            return;
        };
        let max = if matches!(edit.item, Item::SongsFolder | Item::DataFolder) {
            400
        } else {
            12
        };
        match code {
            KeyCode::Escape => self.settings_ui.editing = None,
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let Some(edit) = self.settings_ui.editing.take() else {
                    return;
                };
                if let Err(e) = self.apply_typed(edit.item, &edit.text) {
                    self.toast(e);
                    self.settings_ui.editing = Some(Edit {
                        fresh: false,
                        ..edit
                    });
                }
            }
            KeyCode::Backspace => edit.backspace(),
            KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Delete => {
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
        let number = || {
            parse_number(text).ok_or_else(|| {
                tr!("type a number", "数字を入力してください", "введите число").to_string()
            })
        };
        let percent = || number().map(|n| (n / 100.0).clamp(0.0, 1.0) as f32);
        let c = &mut self.cfg;
        match item {
            Item::LetterboxX => c.letterbox_pos.0 = number()?.round().clamp(-100.0, 100.0) as f32,
            Item::LetterboxY => c.letterbox_pos.1 = number()?.round().clamp(-100.0, 100.0) as f32,
            Item::ScrollSpeed => {
                c.scroll_speed = ((number()? * 100.0).round() / 100.0).clamp(0.3, 4.0)
            }
            Item::Rate => {
                c.rate = ((number()? * 100.0).round() / 100.0).clamp(RATE_RANGE.0, RATE_RANGE.1)
            }
            Item::Offset => c.set_shown_offset(number()?.round().clamp(-1000.0, 1000.0)),
            Item::VisualOffset => c.visual_offset_ms = number()?.round().clamp(-1000.0, 1000.0),
            Item::VsyncTune => c.vsync_tune_ms = number()?.round().clamp(-200.0, 200.0),
            Item::CalibrationBpm => {
                c.calibration_bpm = number()?
                    .round()
                    .clamp(calibrate::BPM_RANGE.0, calibrate::BPM_RANGE.1)
            }
            Item::ResumeDelay => {
                c.resume_delay_ms = (number()? * 1000.0).round().clamp(0.0, 3000.0)
            }
            Item::MasterVolume => c.master_volume = percent()?,
            Item::MusicVolume => c.music_volume = percent()?,
            Item::FxVolume => c.fx_volume = percent()?,
            Item::BackgroundDim => c.bg_dim = percent()?,
            Item::MenuNotesDim => c.menu_notes_dim = percent()?,
            Item::UnfocusedVolume => c.unfocused_volume = percent()?,
            Item::FpsCap => {
                let fps = number()?.round();
                c.fps_cap = if fps <= 0.0 {
                    0
                } else {
                    fps.clamp(30.0, 10000.0) as u32
                };
            }
            Item::AccentHue => {
                let t = text.trim();
                let hex = if t.starts_with('#') {
                    t.to_string()
                } else {
                    format!("#{t}")
                };
                c.accent = render::parse_accent(t)
                    .or_else(|| render::parse_accent(&hex))
                    .ok_or(tr!(
                        "type a colour like #ff8800",
                        "#ff8800のように色を入力",
                        "введите цвет вроде #ff8800"
                    ))?;
            }
            Item::LetterboxSize => {
                let (w, h) = layout::parse_size(text).ok_or(tr!(
                    "type a size like 1280x720",
                    "1280x720のようにサイズを入力",
                    "введите размер вроде 1280x720"
                ))?;
                let (mw, mh) = self.monitor_size;
                if w > mw || h > mh {
                    return Err(
                        tr!("{w}x{h} is bigger than the screen ({mw}x{mh})", "{w}x{h}は画面 ({mw}x{mh}) より大きいです", "{w}x{h} больше экрана ({mw}x{mh})";),
                    );
                }
                if w < MIN_WINDOW.0 as u32 || h < MIN_WINDOW.1 as u32 {
                    return Err(
                        tr!("at least {}x{}", "{}x{}以上", "минимум {}x{}"; MIN_WINDOW.0, MIN_WINDOW.1),
                    );
                }
                c.letterbox_size = (w, h);
            }
            Item::WindowSize => {
                let (w, h) = layout::parse_size(text).ok_or(tr!(
                    "type a size like 1280x720",
                    "1280x720のようにサイズを入力",
                    "введите размер вроде 1280x720"
                ))?;
                let (mw, mh) = self.monitor_size;
                if w > mw || h > mh {
                    return Err(
                        tr!("{w}x{h} is bigger than the screen ({mw}x{mh})", "{w}x{h}は画面 ({mw}x{mh}) より大きいです", "{w}x{h} больше экрана ({mw}x{mh})";),
                    );
                }
                self.window_request = Some((w, h));
                return Ok(());
            }
            Item::DonColour | Item::KatColour => {
                let typed = multiverse::parse_hex(text).ok_or(tr!(
                    "type a colour like #eb452c",
                    "#eb452cのように色を入力",
                    "введите цвет вроде #eb452c"
                ))?;
                let (don, kat) = (c.note_colours.don(), c.note_colours.kat());
                c.note_colours = if item == Item::DonColour {
                    multiverse::Colours::own(typed, kat)
                } else {
                    multiverse::Colours::own(don, typed)
                };
            }
            Item::SongsFolder | Item::DataFolder => {
                let t = text.trim().trim_matches('"');
                let path = match (t.strip_prefix("~/"), std::env::var_os("HOME")) {
                    (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
                    _ => PathBuf::from(t),
                };
                return if item == Item::SongsFolder {
                    self.set_songs_folder(Some(path))
                } else {
                    self.move_data_to(Some(path))
                };
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
        if self.folder_pick.is_busy() {
            return;
        }
        self.folder_pick_for = item;
        let current = if item == Item::DataFolder {
            config::data_dir()
        } else {
            self.cfg
                .songs_folder
                .clone()
                .or_else(|| self.song_dirs.first().cloned())
        };
        let start = current
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_default();
        self.folder_pick = Job::spawn(move || library::pick_folder(&start));
        self.select.picking_folder = true;
    }

    fn poll_folder_pick(&mut self) {
        let picked = match self.folder_pick.poll_ended() {
            Some(Some(picked)) => picked,
            None => return,
            Some(None) => Ok(None),
        };
        self.select.picking_folder = false;
        let item = self.folder_pick_for;
        match picked {
            Ok(Some(dir)) => {
                let done = if item == Item::DataFolder {
                    self.move_data_to(Some(dir))
                } else {
                    self.set_songs_folder(Some(dir))
                };
                if let Err(e) = done {
                    self.toast(e);
                }
            }
            Ok(None) => {}
            Err(e) => {
                if !self.in_settings() {
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
        if self.behind_settings().is_some_and(|prev| !prev.is_select()) {
            return Err(tr!(
                "move the game data from song select",
                "ゲームデータは選曲画面で移動してください",
                "переносите данные из выбора песни"
            )
            .into());
        }
        if self.data_move.is_some() {
            return Err(tr!(
                "the game data is already on its way",
                "ゲームデータはもう移動中です",
                "данные уже переносятся"
            )
            .into());
        }
        let target = target.map(|t| {
            if t.file_name().is_some_and(|n| n == "taiko-rs") {
                t
            } else {
                t.join("taiko-rs")
            }
        });
        if target == self.cfg.data_folder {
            return Err(tr!(
                "the game data is there already",
                "ゲームデータはもうそこにあります",
                "данные уже там"
            )
            .into());
        }
        if let Some(t) = &target {
            if !t.parent().is_some_and(std::path::Path::is_dir) {
                let dir = t.parent().unwrap_or(t).display();
                return Err(
                    tr!("{dir} isn't a folder", "{dir}はフォルダではありません", "{dir} — не папка";),
                );
            }
            std::fs::create_dir_all(t).map_err(|e| format!("{}: {e}", t.display()))?;
        }
        let (done, total) = (
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
        );
        let (tx, rx) = mpsc::channel();
        let (from, to) = (self.cfg.data_folder.clone(), target.clone());
        let (d, t) = (done.clone(), total.clone());
        std::thread::spawn(move || {
            let _ = tx.send(config::move_data(from.as_deref(), to.as_deref(), &d, &t));
        });
        self.data_move = Some(DataMove {
            rx,
            done,
            total,
            to: target,
        });
        Ok(())
    }

    /// While the game data moves: its progress; once it has, the game uses it there (and finds
    /// the maps downloaded into its Songs folder there).
    fn poll_data_move(&mut self) {
        let Some(m) = &self.data_move else { return };
        let result = match m.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                let (done, total) = (
                    m.done.load(Ordering::Relaxed) as f64,
                    m.total.load(Ordering::Relaxed) as f64,
                );
                let mb = |b: f64| b / 1e6;
                let (done, total) = (mb(done), mb(total));
                self.toast(tr!("moving the game data: {done:.0} of {total:.0} MB", "ゲームデータを移動中: {total:.0} MB中{done:.0} MB", "перенос данных: {done:.0} из {total:.0} МБ";));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(tr!(
                "the move stopped",
                "移動が止まりました",
                "перенос прервался"
            )
            .into()),
        };
        let Some(m) = self.data_move.take() else {
            return;
        };
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
        if self.behind_settings().is_some_and(|prev| prev.is_play()) {
            return Err(tr!(
                "change the Songs folder from song select",
                "Songsフォルダは選曲画面で変えてください",
                "меняйте папку Songs в выборе песни"
            )
            .into());
        }
        let folder = folder.map(|f| library::picked_songs(&f));
        if let Some(f) = folder.as_ref().filter(|f| !f.is_dir()) {
            let dir = f.display();
            return Err(
                tr!("{dir} isn't a folder", "{dir}はフォルダではありません", "{dir} — не папка";),
            );
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
            (None, None) => tr!(
                "no osu! Songs folder found",
                "osu!のSongsフォルダが見つかりません",
                "папка Songs osu! не найдена"
            )
            .into(),
        });
        self.scan = Some(start_scan(inputs));
        self.scan_checking = false;
        self.select.scanning = Some(0);
        Ok(())
    }

    /// Installs the newer build when there is one, else looks for one.
    fn update_now(&mut self) {
        if update::BUILD.is_none() {
            self.toast(
                tr!(
                    "a local build doesn't update itself",
                    "ローカルビルドは自分で更新しません",
                    "локальная сборка не обновляется сама"
                )
                .into(),
            );
            return;
        }
        match self.updater.status() {
            update::Status::Available { commit, .. } => {
                self.updater.install();
                self.toast_download(&commit);
            }
            update::Status::Checking | update::Status::Downloading | update::Status::Installed => {}
            _ => {
                self.updater.check();
                self.toast(
                    tr!(
                        "checking for updates...",
                        "アップデートを確認中...",
                        "проверяем обновления..."
                    )
                    .into(),
                );
            }
        }
    }

    fn toast_download(&mut self, commit: &str) {
        let build = update::short(commit);
        self.toast(
            tr!("downloading {build}...", "{build}をダウンロード中...", "загрузка {build}...";),
        );
    }

    /// Nothing is lost by restarting the game now: it's in a menu, with no game paused behind the
    /// settings and no multiplayer room to drop out of.
    fn can_restart(&self) -> bool {
        let scene = self.behind_settings().unwrap_or(&self.scene);
        (scene.is_select() || scene.screen().is_some_and(|s| s.update_corner()))
            && self.mp.is_none()
            && self.splash.is_none()
    }

    /// The badge top right while there's an update to install (menus only), and the restart
    /// once it's installed (which waits for a moment that costs the player nothing).
    fn draw_update(&mut self, c: &mut Canvas) {
        // What's new, top right in the menus (left of the update badge when there is one).
        let menus = (self.scene.is_select() || self.in_settings())
            || self.scene.screen().is_some_and(|s| s.update_corner());
        if menus
            && !self.mobile_bar()
            && self.splash.is_none()
            && self.changelog.is_none()
            && self.player.is_none()
            && self.guide.page.is_none()
        {
            let badge = match self.updater.status() {
                update::Status::Available { commit, .. } => {
                    Canvas::text_width(7.0, &update_badge(&commit)) + 20.0
                }
                update::Status::Downloading => Canvas::text_width(7.0, &updating_badge(1.0)) + 20.0,
                _ => 0.0,
            };
            // Tall enough for a thumb on a phone.
            let (chip_y, chip_h, chip_pad) = if mobile::active() {
                (0.0, 34.0, 28.0)
            } else {
                (2.0, 14.0, 14.0)
            };
            let chip_text = chip_y + (chip_h - 7.0) / 2.0 + 1.0;
            let label = tr!("what's new", "更新内容", "что нового");
            let w = Canvas::text_width(7.0, label) + chip_pad;
            let x = c.width - 8.0 - badge - w;
            let hover = c.hovered(x, chip_y, w, chip_h);
            c.round_rect(
                [x, chip_y],
                [w, chip_h],
                4.0,
                rgba(if hover { 0x2a2a38 } else { 0x1d1d27 }, 0.95),
                rgba(0x3a3a4e, 1.0),
                1.0,
            );
            c.text(
                x + chip_pad / 2.0,
                chip_text,
                7.0,
                rgba(if hover { 0xffffff } else { 0xc8c8d8 }, 1.0),
                label,
            );
            c.hit(x, chip_y, w, chip_h, hit::Hit::Changelog);
            // Who's logged in and their pp, left of it: their profile on the website.
            // With Relax on, how they stand on its boards.
            let relax = self.cfg.mods.relax();
            let me = if relax { self.me_relax } else { self.me };
            let (name, rest) = match (&self.mp_menu.account, me) {
                (Some(name), Some((pp, rank))) => (
                    name.clone(),
                    format!(
                        "  {}{pp:.0} pp{}",
                        if relax { "RX " } else { "" },
                        rank.map(|r| format!("  #{r}")).unwrap_or_default()
                    ),
                ),
                (Some(name), None) => (name.clone(), String::new()),
                (None, _) => (
                    tr!("log in", "ログイン", "войти").to_string(),
                    String::new(),
                ),
            };
            // Their picture first.
            let pic = if self.mp_menu.account.is_some() {
                12.0
            } else {
                0.0
            };
            let name_w = social::styled_name_width(&name, 7.0);
            let aw = name_w + Canvas::text_width(7.0, &rest) + 14.0 + pic;
            let ax = x - 6.0 - aw;
            let hover = c.hovered(ax, chip_y, aw, chip_h);
            c.round_rect(
                [ax, chip_y],
                [aw, chip_h],
                4.0,
                rgba(if hover { 0x2a2a38 } else { 0x1d1d27 }, 0.95),
                rgba(
                    if self.mp_menu.account.is_some() {
                        accent()
                    } else {
                        0x3a3a4e
                    },
                    1.0,
                ),
                1.0,
            );
            if pic > 0.0 {
                c.avatar(&name, ax + 9.0, chip_y + chip_h / 2.0, 4.5, accent(), 1.0);
            }
            let white = rgba(0xffffff, 1.0);
            let nw = social::styled_name(c, ax + 7.0 + pic, chip_text, 7.0, &name, white, name_w);
            c.text(
                ax + 7.0 + pic + nw,
                chip_text,
                7.0,
                rgba(0xff8ad8, 1.0),
                &rest,
            );
            c.hit(ax, chip_y, aw, chip_h, hit::Hit::Account);
        }
        let text = match self.updater.status() {
            update::Status::Available { commit, .. } => update_badge(&commit),
            update::Status::Downloading => updating_badge(self.updater.progress()),
            update::Status::Installed => {
                if !self.can_restart() {
                    return;
                }
                // On disk before the new game reads it (saves are written in the background).
                self.cfg.save();
                config::flush_saves();
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
        let hover = c.hovered(x, 2.0, w, 14.0);
        c.round_rect(
            [x, 2.0],
            [w, 14.0],
            4.0,
            rgba(if hover { 0x2f8f4a } else { 0x1f6b36 }, 0.95),
            rgba(0x86E05A, 1.0),
            1.0,
        );
        c.text(x + 7.0, 5.5, 7.0, rgba(0xffffff, 1.0), &text);
        c.hit(x, 2.0, w, 14.0, hit::Hit::Update);
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
        self.request_skin(true);
    }

    fn request_skin(&mut self, reload: bool) {
        self.skin_loader
            .get_or_insert_with(skin::Loader::new)
            .request(
                skin::Key {
                    main: self.cfg.skin.clone(),
                    parts: self.cfg.skin_parts.clone(),
                    rate: self.audio.as_ref().map(|a| a.rate),
                    map_dir: self.skin_map.clone(),
                },
                reload,
            );
    }

    fn poll_skin(&mut self) {
        if let Some(ready) = self.skin_loader.as_ref().and_then(skin::Loader::poll)
            && self.skin_key.as_ref().is_none_or(|(key, image)| {
                *key != ready.key || !image.ptr_eq(&Arc::downgrade(&ready.assets.images))
            })
        {
            self.skin_key = Some((ready.key, Arc::downgrade(&ready.assets.images)));
            self.sprites_upload = Some(ready.assets.images.clone());
            skin::apply_map_spinner_warning(ready.assets.map_spinner);
            if let (Some(audio), Some(sounds)) = (&self.audio, &ready.assets.sounds) {
                audio.set_hitsounds(sounds.clone());
            }
        }
    }

    /// Swaps the playing map's own skin images in, and out again after: only when it has some,
    /// so ordinary maps never reload the skin. Beatmap skins turned off, for all maps or this
    /// one, count as none, so switching either takes effect from here on the next frame.
    // ponytail: Play and song select's preview only (and their settings); multiplayer /
    // spectating keep your skin.
    fn follow_map_skin(&mut self) {
        let entry = match &self.scene {
            Scene::Other(s) if s.song_play().is_some() => s.song_play().map(|p| p.0),
            Scene::Other(sel) if sel.select_screen() => self.preview_target(),
            Scene::Other(s) if s.map_from_song_select() => self.preview_target(),
            Scene::Other(s) if s.settings() => match self.behind.as_deref() {
                Some(b) if b.is_play() => b.song_play().map(|p| p.0),
                Some(b) if b.is_select() => self.preview_target(),
                _ => None,
            },
            _ => None,
        };
        let dir = entry
            .and_then(|e| self.library.get(e))
            .and_then(|e| e.path.parent())
            .filter(|dir| self.cfg.map_skins && !self.select.map_skin_off(dir))
            .map(Path::to_path_buf);
        if dir == self.skin_map_seen {
            return;
        }
        self.skin_map_seen = dir.clone();
        self.skin_map = dir;
        self.request_skin(false);
    }

    /// The folder a `.tko` map's pictures come from: the map played or edited.
    // ponytail: multiplayer and spectating draw a map's text but not its pictures.
    fn picture_dir(&self) -> Option<PathBuf> {
        if let Some(e) = self.scene.screen_as::<MapEditor>() {
            return e.view.ed.path.parent().map(Path::to_path_buf);
        }
        let entry = match &self.scene {
            // A custom map's preview may show its own pictures (the results' covers come from
            // their own folder, `pictures::COVER_PREFIX`).
            Scene::Other(s) if s.previews_remote() => {
                return self
                    .browse
                    .shown
                    .and_then(|i| self.browse.remote.get(i))
                    .and_then(|e| e.path.parent())
                    .map(Path::to_path_buf);
            }
            Scene::Other(s) if s.song_play().is_some() => s.song_play()?.0,
            Scene::Other(s) if s.settings() => match self.behind.as_deref() {
                Some(b) if b.is_play() => b.song_play()?.0,
                _ => return None,
            },
            _ => return None,
        };
        self.library
            .get(entry)
            .and_then(|e| e.path.parent())
            .map(Path::to_path_buf)
    }

    fn open_skins_folder(&mut self) {
        let Some(dir) = skin::own_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let opener = if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        if std::process::Command::new(opener)
            .arg(&dir)
            .spawn()
            .is_err()
        {
            let dir = dir.display();
            self.toast(
                tr!("skins go in {dir}", "スキンは{dir}に置きます", "скины кладутся в {dir}";),
            );
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
            info.extend(
                self.library[start..]
                    .iter()
                    .enumerate()
                    .map(|(k, e)| (start + k, e.path.clone(), e.modified, e.size))
                    .collect(),
            );
            self.mapinfo_sorted = false;
        }
        // A map downloaded for a collection joins it.
        self.list_collections();
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
                    // A picture dropped on the editor goes into the map's folder.
                    if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>()
                        && path
                            .file_name()
                            .is_some_and(|n| pictures::is_picture(&n.to_string_lossy()))
                    {
                        view.drop_picture(&path);
                        continue;
                    }
                    if self.scene.screen_mut::<EditorHome>().is_some() {
                        let rate = self.audio.as_ref().map(|a| a.rate);
                        let out = match self.scene.screen_mut::<EditorHome>() {
                            Some(h) => h.home.dropped(path, rate),
                            _ => editor::home::Out::None,
                        };
                        self.home_out(out);
                        continue;
                    }
                    // A song dropped on song select or the menu starts a map from it.
                    if editor::home::is_audio(&path)
                        && self.scene.is_select()
                        && !self.picking_lobby_map
                        && self.mp.is_none()
                    {
                        self.open_editor_home();
                        let rate = self.audio.as_ref().map(|a| a.rate);
                        if let Some(h) = self.scene.screen_mut::<EditorHome>() {
                            h.home.dropped(path, rate);
                        }
                        continue;
                    }
                    let name = path
                        .file_stem()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if library::is_map_archive(&path) {
                        self.import_osz(path, &name, false);
                    } else {
                        let file = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        self.toast(tr!("{file} isn't a map: drop .osz or .rsz files", "{file}は譜面ではありません: .oszか.rszファイルをドロップしてください", "{file} — не карта: бросайте файлы .osz или .rsz";));
                    }
                }
            }
        }
    }

    /// A map archive opened from the file manager (or the command line): imported like one
    /// dropped on the window.
    fn open_archive(&mut self, path: PathBuf) {
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.import_osz(path, &name, false);
    }

    /// Unpacks an `.osz` (or `.rsz`) into the Songs folder, unless everything in it is already in the list or
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
        let pending: std::collections::HashSet<u64> = self
            .downloads
            .iter()
            .flat_map(|j| j.maps.iter().copied())
            .collect();
        let (fresh, home) = library::osz_news(&self.library, &pending, &maps);
        if fresh == 0 {
            if temporary {
                let _ = std::fs::remove_file(&archive);
            }
            let here = home.map(|home| {
                main_maps(
                    self.library
                        .iter()
                        .filter(|e| e.path.parent() == Some(home)),
                )
            });
            self.reveal_maps(here.unwrap_or_default());
            self.toast(tr!("{name} is already in your songs", "{name}はもう曲の中にあります", "{name} уже есть среди ваших песен";));
            return;
        }
        let folder = home
            .filter(|d| d.parent() == Some(self.songs_dir.as_path()))
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        self.toast(tr!("importing {name}...", "{name}を取り込み中...", "добавляем {name}...";));
        let job = download::import(
            archive,
            folder.as_deref().unwrap_or(name),
            self.songs_dir.clone(),
            temporary,
            maps.into_iter().map(|m| m.1).collect(),
        );
        self.downloads.push(job);
    }

    /// While files are dragged over the window: where they go when dropped.
    fn draw_drag_hover(&self, canvas: &mut Canvas, now: Instant) {
        let Some(since) = self.drag_hover else { return };
        let a = (now.saturating_duration_since(since).as_secs_f32() / 0.12).min(1.0);
        let (w, h) = (canvas.width, render::VIRTUAL_HEIGHT);
        canvas.rect(0.0, 0.0, w, h, rgba(0x000000, 0.6 * a));
        canvas.round_rect(
            [24.0, 24.0],
            [w - 48.0, h - 48.0],
            16.0,
            rgba(0x000000, 0.0),
            rgba(accent(), 0.9 * a),
            2.5,
        );
        let drop = tr!(
            "Drop .osz or .rsz maps to add them",
            ".oszか.rszの譜面をドロップで追加",
            "Бросьте карты .osz или .rsz, чтобы добавить"
        );
        canvas.text_centered(
            w / 2.0,
            h / 2.0 - 22.0,
            18.0,
            rgba(0xffffff, a),
            &Canvas::fit(18.0, w - 64.0, drop),
        );
        let dir = self.songs_dir.display();
        let to = Canvas::fit(
            9.0,
            w - 96.0,
            &tr!("into {dir}", "追加先: {dir}", "в {dir}";),
        );
        canvas.text_centered(w / 2.0, h / 2.0 + 14.0, 9.0, rgba(0x9a9ab0, a), &to);
    }

    /// Keeps the Discord status in line with what's on screen (checked about once a second).
    fn update_presence(&mut self) {
        if self
            .presence_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.presence_at = Some(Instant::now());
        let app_id = if self.cfg.discord_app_id.is_empty() {
            discord::DEFAULT_APP_ID
        } else {
            self.cfg.discord_app_id.as_str()
        };
        if !self.cfg.discord_presence || app_id.is_empty() {
            // Dropping the connection clears the status.
            self.discord = None;
            self.presence = None;
            return;
        }
        let app_id = app_id.to_string();
        let Some((details, state)) = self.presence_text() else {
            return;
        };
        let discord = self
            .discord
            .get_or_insert_with(|| discord::Presence::start(app_id));
        // The timer restarts when the top line changes (a new map, a new screen).
        let start = match &self.presence {
            Some(p) if p.details == details => p.start,
            _ => discord::unix_now(),
        };
        let activity = discord::Activity {
            details,
            state,
            start,
        };
        if self.presence.as_ref() != Some(&activity) {
            discord.set(Some(activity.clone()));
            self.presence = Some(activity);
        }
    }

    /// The two lines of the Discord status for the current screen.
    fn presence_text(&self) -> Option<(String, String)> {
        let map_label = multiplayer::map_label;
        let players = |s: &multiplayer::Session| {
            let n = s.players.len();
            let s = if n == 1 { "" } else { "s" };
            let ru = lang::ru_plural(n, ["игрок", "игрока", "игроков"]);
            tr!("{n} player{s}", "{n}人", "{n} {ru}";)
        };
        let rate_text = |rate: f64| {
            if (rate - 1.0).abs() > 1e-6 {
                format!("  {rate:.2}x")
            } else {
                String::new()
            }
        };
        let lobby = |s: &multiplayer::Session| {
            let n = players(s);
            let details = if s.is_host() {
                tr!("Multiplayer lobby ({n}, hosting)", "マルチのロビー ({n}、ホスト)", "Лобби мультиплеера ({n}, хост)";)
            } else {
                tr!("Multiplayer lobby ({n}, in)", "マルチのロビー ({n}、参加中)", "Лобби мультиплеера ({n}, участник)";)
            };
            let state =
                s.map.as_ref().map(map_label).unwrap_or_else(|| {
                    tr!("picking a map", "譜面を選択中", "выбирает карту").into()
                });
            (details, state)
        };
        // In a match: waiting for it to start, or playing it.
        let in_match = || {
            let s = self.mp.as_ref()?;
            let label = s.map.as_ref().map(map_label).unwrap_or_default();
            let (n, rate) = (
                players(s),
                rate_text(s.map.as_ref().map(|m| m.rate).unwrap_or(1.0)),
            );
            Some((
                label,
                tr!("Multiplayer match, {n}{rate}", "マルチの対戦、{n}{rate}", "Матч мультиплеера, {n}{rate}";),
            ))
        };
        // Away: the idle screen is all the way in (the timer then counts the time away).
        if self.idle.amount() >= 1.0 {
            let song = self
                .idle_song()
                .map(|(title, artist)| format!("{artist} - {title}"))
                .unwrap_or_default();
            return Some((tr!("AFK", "離席中", "Отошёл").into(), song));
        }
        Some(match &self.scene {
            Scene::Other(s) if s.in_match() => in_match()?,
            // Anywhere else while in a room (its lobby, or song select picking its map).
            Scene::Other(s) if s.presence_first() => s.presence(self)?,
            _ if self.mp.is_some() => lobby(self.mp.as_ref()?),
            Scene::Other(sel) if sel.select_screen() && self.player.is_some() => {
                let song = self
                    .player
                    .as_ref()
                    .and_then(|p| p.list.entry())
                    .and_then(|i| self.library.get(i))
                    .map(Entry::label);
                (
                    tr!("Listening to music", "音楽を聴いている", "Слушает музыку").into(),
                    song.unwrap_or_default(),
                )
            }
            _ if self.on_menu() => {
                let song = self
                    .idle_song()
                    .map(|(title, artist)| format!("{artist} - {title}"))
                    .unwrap_or_default();
                (
                    tr!("In the main menu", "メインメニュー", "В главном меню").into(),
                    song,
                )
            }
            Scene::Other(s) if s.presence(self).is_some() => s.presence(self).unwrap_or_default(),
            _ => (
                tr!("Choosing a song", "曲を選んでいます", "Выбирает песню").into(),
                self.select
                    .selected()
                    .and_then(|i| self.library.get(i))
                    .map(|e| e.label())
                    .unwrap_or_default(),
            ),
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
                .args([
                    "--session",
                    "--print-reply",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                ])
                .arg(format!("array:string:{uri}"))
                .arg("string:")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            selected
                || path
                    .parent()
                    .is_some_and(|dir| Command::new("xdg-open").arg(dir).spawn().is_ok())
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
                KeyCode::Escape | KeyCode::Backspace if let Item::SkinPart(i) = p.item => {
                    self.reopen_skin_parts(i)
                }
                KeyCode::Escape | KeyCode::Backspace | KeyCode::Tab => {
                    self.settings_ui.choices = None
                }
                _ => {}
            }
            return;
        }
        if let Some(item) = self.settings_ui.binding.take() {
            let modifier = matches!(
                code,
                KeyCode::ShiftLeft
                    | KeyCode::ShiftRight
                    | KeyCode::ControlLeft
                    | KeyCode::ControlRight
                    | KeyCode::AltLeft
                    | KeyCode::AltRight
                    | KeyCode::SuperLeft
                    | KeyCode::SuperRight
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
                        let chord = keys::Chord {
                            key: code,
                            ctrl: self.modifiers_ctrl,
                            shift: self.modifiers_shift,
                            alt: self.modifiers_alt,
                        };
                        if self.cfg.keys.contains(&code)
                            && !chord.ctrl
                            && !chord.alt
                            && h.places()
                                .iter()
                                .any(|p| matches!(p, keys::Place::Play | keys::Place::Paused))
                        {
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
                self.settings_ui
                    .move_cursor(if code == KeyCode::PageDown { 8 } else { -8 });
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                let n = settings::SECTIONS.len() as i32;
                let dir = if code == KeyCode::PageDown { 1 } else { -1 };
                let next = (self.settings_ui.section() as i32 + dir).rem_euclid(n) as usize;
                self.settings_ui.cursor = settings::section_start(next);
            }
            KeyCode::ArrowLeft => self.adjust_setting(item, -1, window),
            KeyCode::ArrowRight => self.adjust_setting(item, 1, window),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Calibrate => {
                self.start_calibration_as(false)
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Guide => {
                self.guide.page = Some(0)
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item == Item::ResetMapOffsets =>
            {
                self.press_reset_map_offsets()
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item == Item::VsyncCalibrate =>
            {
                self.start_calibration_as(true)
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item == Item::PlayfieldArea =>
            {
                self.open_layout_editor()
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item == Item::InterfaceEditor =>
            {
                self.open_hud_editor()
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item == Item::PlayfieldPreset =>
            {
                self.open_dropdown()
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::SkinFolder => {
                self.open_skins_folder()
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::HiddenMaps => {
                if self.select.hidden.is_empty() {
                    self.toast(
                        tr!(
                            "nothing is hidden: right-click a song in the list to hide it",
                            "何も隠れていません: リストの曲を右クリックで隠せます",
                            "ничего не скрыто: правый клик по песне в списке прячет её"
                        )
                        .into(),
                    );
                } else {
                    let what = self.select.hidden.describe();
                    self.select.unhide_all(&self.library);
                    self.toast(tr!("{what} back in the song list", "{what}を曲リストに戻しました", "{what} снова в списке песен";));
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item.kind() == Kind::List => {
                self.open_choices(item)
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item.kind() == Kind::Value =>
            {
                self.start_edit(item)
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Update => {
                self.update_now()
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item == Item::KrokodilKoban && self.settings_ui.krokodil =>
            {
                match library::launch_osu() {
                    Ok(()) => self.quit = true,
                    Err(e) => self.toast(e),
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item == Item::SongSelectLayout =>
            {
                self.reset_song_layout();
                self.toast(
                    tr!(
                        "song select: the usual layout",
                        "選曲画面: いつものレイアウト",
                        "выбор песни: обычный макет"
                    )
                    .into(),
                );
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space if item == Item::Changelog => {
                self.changelog = Some(changelog::View::recent(false))
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if item == Item::PlayfieldReset =>
            {
                self.cfg.playfield_area = Area::FULL;
                self.cfg.save();
                self.toast(
                    tr!(
                        "playfield fills the screen",
                        "プレイフィールドを全画面にしました",
                        "поле на весь экран"
                    )
                    .into(),
                );
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                self.adjust_setting(item, 1, window)
            }
            KeyCode::Tab if self.behind_settings().is_some_and(|prev| prev.is_select()) => {
                self.open_multiplayer();
            }
            KeyCode::Escape | KeyCode::Tab | KeyCode::F1 => self.close_settings(),
            _ => {}
        }
    }

    /// (The run and its result leave on their own keys; tests leave them this way.)
    #[cfg(test)]
    /// Leaves calibrating (or its result, taking its suggestion with `apply`) for where it was
    /// started from.
    fn leave_calibration(&mut self, apply: bool) {
        if let Some(run) = self.scene.screen_mut::<screens::calibration::Calibration>() {
            let back = run.take_back();
            self.stop_audio();
            self.scene = *back;
            return;
        }
        match std::mem::replace(&mut self.scene, SongSelect::scene()) {
            other @ Scene::Other(_) if other.screen().is_some_and(|s| s.calibrating()) => {
                // The result screen leaves on its own keys.
                self.scene = other;
                let key = if apply {
                    KeyCode::Enter
                } else {
                    KeyCode::Escape
                };
                self.screen_key(screens::screen::Key {
                    code: key,
                    repeat: false,
                    text: None,
                    at: Instant::now(),
                    window: None,
                });
            }
            other => {
                self.stop_audio();
                self.scene = other;
            }
        }
    }

    /// Applies the live offset suggestion from the song being played (O key).
    /// Applies the live offset suggestion from the song being played (O key) to that map.
    fn apply_live_offset(&mut self) {
        // Shift+O and a match don't go through the hotkey, so this says it for them all.
        if self.cfg.hotkeys.get(keys::Hotkey::ApplyOffset).is_none() {
            return;
        }
        let Some(entry) = self.song_entry() else {
            return;
        };
        let (true, Some(game)) = (self.scene.plays_song(), self.play.as_deref()) else {
            return;
        };
        match game.offset_suggestion(self.play_offset) {
            Some(h) if !h.significant => {
                let text = tr!("offset already within +-{:.1} ms of your hits", "オフセットはもう叩きの±{:.1} ms以内です", "смещение уже в пределах +-{:.1} мс от ударов"; h.estimate.margin_ms);
                self.toast(text);
            }
            // Shift+O: the suggestion is about every map (the same on all of them), so it goes
            // to the global offset.
            Some(h) if self.modifiers_shift => {
                // Late by e is the clock's offset up by e, so osu!'s down by e.
                let was = self.cfg.shown_offset();
                let shown = game.suggested_shown_offset(h.estimate.suggested_offset);
                self.cfg.set_shown_offset(shown);
                self.cfg.save();
                self.refresh_play_offset();
                self.toast(tr!("offset {:+.0} ms for all maps (was {was:+.0})", "全譜面のオフセット {:+.0} ms (前は{was:+.0})", "смещение {:+.0} мс для всех карт (было {was:+.0})"; self.cfg.shown_offset()));
            }
            Some(h) => {
                let was = self.map_offset(Some(entry));
                // The suggestion is the clock's offset in real ms; a map's offset is song ms, the
                // osu! way round.
                let map = (self.cfg.offset_ms - h.estimate.suggested_offset) * self.song_rate();
                self.set_map_offset(entry, map);
                self.toast(tr!("this map: offset {map:+.0} ms (was {was:+.0})", "この譜面: オフセット {map:+.0} ms (前は{was:+.0})", "эта карта: смещение {map:+.0} мс (было {was:+.0})";));
            }
            None => self.toast(
                tr!(
                    "need at least 10 hits for a suggestion",
                    "提案には10回以上叩く必要があります",
                    "для подсказки нужно хотя бы 10 ударов"
                )
                .into(),
            ),
        }
    }

    /// The map played now, or whose results are up (its own offset is what -/+ and O change).
    fn song_entry(&self) -> Option<usize> {
        match &self.scene {
            Scene::Other(s) if s.map_entry().is_some() => s.map_entry(),
            // The room's map is the song of its match, of the wait for it and of its results;
            // the lobby only shows it.
            Scene::Other(s) if s.in_room() && !s.room_lobby() => {
                self.mp.as_ref().and_then(|s| s.map_entry)
            }
            _ => None,
        }
    }

    /// The map whose offset applies on this screen (also the selected one, for the preview).
    fn offset_entry(&self) -> Option<usize> {
        match &self.scene {
            Scene::Other(s) if s.spectating() => self.mp.as_ref().and_then(|s| s.map_entry),
            Scene::Other(s) if s.room_lobby() => self.mp.as_ref().and_then(|s| s.map_entry),
            Scene::Other(s) if s.watching() => s.watched_map(),
            Scene::Other(sel) if sel.select_screen() => self.select.selected(),
            Scene::Other(s) if s.map_from_song_select() => self.select.selected(),
            _ => self.song_entry(),
        }
    }

    fn map_offset(&self, entry: Option<usize>) -> f64 {
        if self.map_offsets.is_empty() {
            return 0.0;
        }
        entry
            .and_then(|i| self.library.get(i))
            .and_then(|e| self.map_offsets.get(&e.hash()))
            .copied()
            .unwrap_or(0.0)
    }

    fn set_map_offset(&mut self, entry: usize, ms: f64) {
        let Some(e) = self.library.get(entry) else {
            return;
        };
        let ms = ms.round();
        if ms == 0.0 {
            self.map_offsets.remove(&e.hash());
        } else {
            self.map_offsets.insert(e.hash(), ms);
        }
        config::save_map_offsets(&self.map_offsets);
        self.refresh_play_offset();
    }

    /// How fast the song goes by: the play's, else what one would be now.
    fn song_rate(&self) -> f64 {
        let rate = match &self.scene {
            _ if self.scene.plays_song() && self.play.is_some() => {
                self.play.as_ref().map_or(1.0, |game| game.speed().r0)
            }
            _ => self
                .cfg
                .mods
                .rate_override()
                .map_or(self.cfg.rate, |(rate, _)| rate),
        };
        rate.max(0.01)
    }

    /// Switches the scale the global offset is set by; the timing stays as it is.
    fn set_offset_scale(&mut self, scale: config::OffsetScale) {
        self.cfg.set_offset_scale(scale);
        self.cfg.save();
        self.refresh_play_offset();
    }

    fn refresh_play_offset(&mut self) {
        self.follow_latency();
        let was = self.play_offset;
        let offset = self.map_offset(self.offset_entry());
        // As osu! has them: the global offset is about this machine, so real ms at any rate; a
        // map's is about its song, so it goes by as the song does. Both bring the notes earlier
        // with +, which takes the clock's offset down.
        self.play_offset = self.cfg.offset_ms - offset / self.song_rate();
        if self.play_offset != was {
            audiolog::line(&format!(
                "offset total={} global={}",
                self.play_offset, self.cfg.offset_ms
            ));
        }
        self.preview.map_offset = offset;
        if self.scene.plays_song()
            && let Some(game) = self.play.as_deref_mut()
        {
            game.set_shown_offset(self.cfg.shown_offset());
            game.set_latency(self.cfg.offset_latency_ms);
        }
    }

    /// Keeps the play offset to what the clock takes off: the global offset is osu!'s, and the
    /// play offset follows from it and the output latency. The first settled latency also turns a
    /// config from before into osu!'s terms (`Config::set_latency`).
    fn follow_latency(&mut self) {
        let Some(audio) = &self.audio else { return };
        let (latency, _) = audio.latencies_ms();
        self.apply_latency(latency, audio.latency_settled());
    }

    fn apply_latency(&mut self, latency: f64, settled: bool) {
        let moved = (latency - self.cfg.offset_latency_ms).abs() > 0.01;
        if (moved || (settled && self.cfg.offset_pending)) && self.cfg.set_latency(latency, settled)
        {
            self.cfg.save();
        }
    }

    /// Plays the current map again from where it was started (in game, paused or on results).
    fn restart_map(&mut self) {
        if let Some((entry, from)) = self.scene.song_play() {
            self.retry_from(entry, from);
        } else if let Some(r) = self.scene.screen_as::<ResultsScreen>() {
            let (entry, from) = (r.entry, r.from);
            self.retry_from(entry, from);
        }
    }

    /// One step faster or slower through the replay being watched.
    fn step_replay_speed(&mut self, by: i32) {
        let (true, Some(game)) = (self.scene.is_play(), self.play.as_deref()) else {
            return;
        };
        if !game.has_constant_speed() {
            self.toast(
                tr!(
                    "this replay's speed changes by itself",
                    "このリプレイの速度は変更できません",
                    "скорость этого повтора меняется сама"
                )
                .into(),
            );
            return;
        }
        let at = REPLAY_SPEEDS
            .iter()
            .position(|&s| s >= self.replay_speed_wanted)
            .unwrap_or(REPLAY_SPEEDS.len() - 1) as i32;
        let next = (at + by).clamp(0, REPLAY_SPEEDS.len() as i32 - 1);
        self.replay_speed_wanted = REPLAY_SPEEDS[next as usize];
    }

    /// Makes the replay's music at the speed asked for, one at a time however fast it's asked,
    /// and plays on with it when it's ready.
    fn poll_replay_speed(&mut self) {
        let (Some((entry, _)), Some(game)) = (self.scene.song_play(), self.play.as_deref_mut())
        else {
            self.replay_speed = 1.0;
            self.replay_speed_wanted = 1.0;
            self.replay_speed_rx.cancel();
            return;
        };
        if self.replay_speed_rx.is_busy() {
            match self.replay_speed_rx.poll() {
                Some(Ok((track, speed))) => {
                    let rate = game.rate() * speed;
                    game.change_speed(track, rate, sound(&self.audio), self.play_offset);
                    self.replay_speed = speed;
                }
                Some(Err(e)) => {
                    self.replay_speed_wanted = self.replay_speed;
                    self.status = format!("failed to change the speed: {e}");
                }
                None => {}
            }
            return;
        }
        if self.replay_speed_wanted == self.replay_speed {
            return;
        }
        let Some(sample_rate) = self.audio.as_ref().map(|a| a.rate) else {
            return;
        };
        let speed = self.replay_speed_wanted;
        let rate = game.rate() * speed;
        let keep_pitch = game.mods().speed(1.0, self.cfg.rate_keep_pitch).1;
        let path = preview::audio_path(&self.library[entry]);
        self.replay_speed_rx = Job::spawn(move || {
            let constant = mods::Speed::constant(rate);
            let track = stream::open(&path, sample_rate, constant, keep_pitch).and_then(|t| {
                t.map_or_else(
                    || {
                        audio::decode_shared(&path, sample_rate)
                            .map(|t| audio::with_rate(t, sample_rate, rate, keep_pitch))
                    },
                    Ok,
                )
            });
            track.map(|t| (t, speed))
        });
    }

    /// Pauses the fullscreen preview, or sets it going again.
    fn toggle_preview_held(&mut self) {
        self.preview.held = !self.preview.held;
        if let Some(a) = &self.audio {
            a.set_paused(self.preview.held);
        }
    }

    /// The quick retry key went down: the map restarts if it's still held in `RETRY_HOLD_MS`.
    fn hold_retry(&mut self) {
        self.retry_hold.get_or_insert(self.last_key);
    }

    /// A key came up.
    pub(crate) fn key_released(&mut self, code: KeyCode) {
        if self.retry_hold.is_some_and(|(key, _)| key == code) {
            self.retry_hold = None;
        }
    }

    /// How far through the quick retry hold it is, 0 to 1.
    fn retry_progress(&self, now: Instant) -> Option<f32> {
        let (_, at) = self.retry_hold?;
        let held = now.saturating_duration_since(at).as_secs_f64() * 1000.0;
        Some((held / RETRY_HOLD_MS).min(1.0) as f32)
    }

    /// Restarts the map once the quick retry key has been held long enough.
    fn poll_retry(&mut self, now: Instant) {
        if !self.scene.is_play() {
            self.retry_hold = None;
        }
        if self.retry_progress(now).is_some_and(|t| t >= 1.0) {
            self.retry_hold = None;
            self.restart_map();
        }
    }

    fn pause_key(&mut self, code: KeyCode) {
        if code == KeyCode::KeyO {
            self.apply_live_offset();
            return;
        }
        if code == KeyCode::Backquote {
            self.hold_retry();
            return;
        }
        if code == KeyCode::KeyR && self.modifiers_ctrl {
            self.restart_map();
            return;
        }
        let (Some((entry, from)), Some(game)) = (self.scene.song_play(), self.play.as_deref_mut())
        else {
            return;
        };
        if game.resuming().is_some() {
            // Counting down: Esc goes back to the menu, everything else waits.
            if code == KeyCode::Escape {
                game.cancel_resume();
            }
            return;
        }
        let delay = Duration::from_millis(self.cfg.resume_delay_ms as u64);
        let cursor = game.paused().unwrap_or(0);
        match code {
            // Wrapping: Up from Continue is Quit, so pause, Up, Enter leaves at once.
            KeyCode::ArrowUp => game.set_pause_cursor((cursor + PAUSE_ITEMS - 1) % PAUSE_ITEMS),
            KeyCode::ArrowDown => game.set_pause_cursor((cursor + 1) % PAUSE_ITEMS),
            KeyCode::Escape => game.resume_after(delay, sound(&self.audio)),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => match cursor {
                0 => game.resume_after(delay, sound(&self.audio)),
                1 => {
                    let (e, f) = (entry, from);
                    self.retry_from(e, f);
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
                if self.player.as_ref().is_some_and(|p| p.searching)
                    || (self.scene.is_select()
                        && self.select.searching
                        && !self.modifiers_ctrl) =>
            {
                return false;
            }
            KeyCode::Minus | KeyCode::NumpadSubtract | KeyCode::Equal | KeyCode::NumpadAdd
                if self.modifiers_ctrl =>
            {
                let d = if matches!(key, KeyCode::Minus | KeyCode::NumpadSubtract) {
                    -step
                } else {
                    step
                };
                self.cfg.visual_offset_ms += d;
                self.cfg.save();
                self.toast(tr!("visual offset {:+.0} ms (notes drawn earlier)", "表示オフセット {:+.0} ms (ノーツを早めに描画)", "смещение картинки {:+.0} мс (ноты раньше)"; self.cfg.visual_offset_ms));
                return true;
            }
            KeyCode::Minus | KeyCode::NumpadSubtract | KeyCode::Equal | KeyCode::NumpadAdd => {
                let d = if matches!(key, KeyCode::Minus | KeyCode::NumpadSubtract) {
                    -step
                } else {
                    step
                };
                match self.song_entry() {
                    // As in osu!, for a map as for all of them: + brings the notes earlier.
                    Some(entry) => {
                        let ms = self.map_offset(Some(entry)) + d;
                        self.set_map_offset(entry, ms);
                        let text = tr!("this map: offset {ms:+.0} ms (total {:+.0})", "この譜面: オフセット {ms:+.0} ms (合計 {:+.0})", "эта карта: смещение {ms:+.0} мс (всего {:+.0})"; self.cfg.shown_of_play(self.play_offset));
                        self.toast(text);
                    }
                    None => {
                        // As in osu!: + brings the notes earlier.
                        self.cfg.set_shown_offset(self.cfg.shown_offset() + d);
                        self.cfg.save();
                        self.refresh_play_offset();
                        self.toast(tr!("offset {:+.0} ms (all maps)", "オフセット {:+.0} ms (全譜面)", "смещение {:+.0} мс (все карты)"; self.cfg.shown_offset()));
                    }
                }
                return true;
            }
            // Like the speed, only between songs: it counts for pp.
            KeyCode::F3 | KeyCode::F4 if self.mid_song() => {
                self.toast(scroll_in_select().into());
                return true;
            }
            KeyCode::F3 => {
                self.cfg.scroll_speed = config::step_scroll_speed(self.cfg.scroll_speed, -1.0)
            }
            KeyCode::F4 => {
                self.cfg.scroll_speed = config::step_scroll_speed(self.cfg.scroll_speed, 1.0)
            }
            KeyCode::F7 => {
                self.cfg.present = self.step_present(1);
                self.cfg.save();
                let note = if self.cfg.menu_vsync && !self.player_plays() {
                    tr!(
                        " (menus stay on vsync)",
                        " (メニューは垂直同期のまま)",
                        " (меню остаются с vsync)"
                    )
                } else {
                    ""
                };
                let text = tr!("gameplay present mode: {}{note}", "プレイ中の表示モード: {}{note}", "режим вывода в игре: {}{note}"; self.cfg.present.name());
                self.toast(text);
                return true;
            }
            KeyCode::F8 => {
                let i = FPS_CAPS
                    .iter()
                    .position(|&c| c == self.cfg.fps_cap)
                    .unwrap_or(0);
                self.cfg.fps_cap = FPS_CAPS[(i + 1) % FPS_CAPS.len()];
                let mut text = if self.cfg.fps_cap == 0 {
                    tr!(
                        "fps cap: unlimited",
                        "FPSの上限: 無制限",
                        "ограничение FPS: нет"
                    )
                    .to_string()
                } else {
                    tr!("fps cap: {}", "FPSの上限: {}", "ограничение FPS: {}"; self.cfg.fps_cap)
                };
                if self.present == PresentChoice::Fifo {
                    text.push_str(tr!(
                        " (mailbox/immediate only)",
                        " (mailbox/immediateのみ)",
                        " (только mailbox/immediate)"
                    ));
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
        let Some([x, y, w, h]) = self.replay_bar else {
            return false;
        };
        let (vx, vy) = self.mouse_virtual();
        if !grabbed && !(vx >= x - 6.0 && vx <= x + w + 6.0 && vy >= y && vy <= y + h) {
            return false;
        }
        let (true, Some(game)) = (self.scene.is_play(), self.play.as_deref_mut()) else {
            return false;
        };
        let (start, end) = game.span();
        let t = start + ((vx - x) / w.max(1.0)).clamp(0.0, 1.0) as f64 * (end - start);
        game.seek(t, sound(&self.audio), self.play_offset);
        true
    }

    /// Gameplay hides the pointer once it has been still for a second; menus (and the pause
    /// menu) always show it.
    fn cursor_hidden(&self, now: Instant) -> bool {
        let playing = match &self.scene {
            Scene::Other(s) if s.holds_game() => {
                self.game().is_some_and(|game| game.paused().is_none())
            }
            // Watching: hidden unless the friends overlay is open (it needs the pointer).
            Scene::Other(s) if s.watching() => !self.social.open,
            _ => false,
        };
        // The idle screen all the way in: nothing to point at.
        if self.idle.amount() >= 1.0 {
            return true;
        }
        playing
            && self.grip.is_none()
            && self.replay_scrub.is_none()
            && self.settings_ui.dropdown.is_none()
            && now.saturating_duration_since(self.mouse_moved_at) >= Duration::from_secs(1)
    }

    /// Scenes where the playfield area can be moved and resized with the mouse.
    fn area_editable(&self) -> bool {
        self.scene.is_play() || self.scene.screen().is_some_and(|s| s.area_editable())
    }

    fn refresh_presets(&mut self) {
        self.resolutions = layout::presets(
            &self.monitor_modes,
            self.window_w as u32,
            self.window_h as u32,
        );
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
        layout::group_presets(&sizes, self.monitor_size)
            .into_iter()
            .flat_map(|(_, l)| l)
            .collect()
    }

    fn screen_size(&self) -> (u32, u32) {
        (self.window_w as u32, self.window_h as u32)
    }

    /// The saved sizes first, then the built-in ones grouped by aspect ratio.
    fn preset_groups(&self) -> Vec<(&'static str, Vec<(u32, u32)>)> {
        let saved = &self.cfg.saved_presets;
        let others: Vec<(u32, u32)> = self
            .resolutions
            .iter()
            .copied()
            .filter(|r| !saved.contains(r))
            .collect();
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
        self.preset_groups()
            .into_iter()
            .flat_map(|(_, l)| l)
            .collect()
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
        let list: Vec<(u32, u32)> = self
            .preset_list()
            .into_iter()
            .filter(|&r| self.fits_window(r))
            .collect();
        if list.is_empty() {
            self.toast(
                tr!(
                    "no smaller resolutions fit this window",
                    "このウィンドウに収まる、より小さい解像度はありません",
                    "меньших разрешений для этого окна нет"
                )
                .into(),
            );
            return;
        }
        let n = list.len() as i32;
        let i = match self
            .current_preset()
            .and_then(|p| list.iter().position(|&r| r == p))
        {
            Some(i) => (i as i32 + dir).rem_euclid(n),
            None => {
                if dir > 0 {
                    0
                } else {
                    n - 1
                }
            }
        } as usize;
        self.apply_preset(list[i]);
    }

    /// Opens the preset list, even when it's empty: saved sizes can be added there.
    fn open_dropdown(&mut self) {
        let list = self.preset_list();
        let cursor = self
            .current_preset()
            .and_then(|p| list.iter().position(|&r| r == p))
            .unwrap_or(0);
        self.settings_ui.dropdown = Some(Dropdown::new(cursor));
    }

    fn dropdown_key(&mut self, code: KeyCode) {
        if let Some(dd) = &mut self.settings_ui.dropdown {
            dd.show_cursor();
        }
        let list = self.preset_list();
        let n = list.len();
        let saved = self.cfg.saved_presets.len();
        let shift = self.modifiers_shift;
        let Some(dd) = &mut self.settings_ui.dropdown else {
            return;
        };
        if let Some((_, text)) = &mut dd.editing {
            match code {
                KeyCode::Escape => dd.editing = None,
                KeyCode::Backspace => settings::caret_backspace(text, &mut dd.caret),
                KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Delete => {
                    settings::caret_nav(text, &mut dd.caret, code);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    if let Some((index, text)) = dd.editing.take() {
                        self.finish_preset_edit(index, text);
                    }
                }
                _ => {
                    if let Some(ch) = size_char(code) {
                        settings::caret_insert(
                            text,
                            &mut dd.caret,
                            ch.encode_utf8(&mut [0; 4]),
                            11,
                        );
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
        let Some(dd) = &mut self.settings_ui.dropdown else {
            return;
        };
        dd.show_cursor();
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
                self.toast(
                    tr!("saved {}x{}", "{}x{}を保存しました", "сохранён {}x{}"; size.0, size.1),
                );
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
            self.toast(
                tr!(
                    "type a size like 1280x720",
                    "1280x720のようにサイズを入力",
                    "введите размер вроде 1280x720"
                )
                .into(),
            );
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
    /// Turns the volume picked on the volume overlay (Alt+wheel, Alt+Up/Down) `notches` up.
    fn turn_volume(&mut self, notches: i32) {
        let changed = self.volume.turn(&mut self.cfg, notches, Instant::now());
        if changed {
            self.apply_volume();
            self.volume_save_at = Some(Instant::now() + Duration::from_millis(250));
        }
        // Hitsounds have nothing playing to judge them by: one rings at the new volume.
        if changed && self.volume.selected == volume::HITSOUNDS && !self.in_song() {
            self.sfx(audio::Sfx::Kat);
        }
    }

    fn wheel(&mut self, notches: i32) {
        if self.idle.input(Instant::now()) {
            return;
        }
        if notches == 0 {
            return;
        }
        if self.scene.screen_is::<MapEditor>() && !self.social.open {
            let keys = self.editor_keys();
            // The volume's own notes, once it is up, take the wheel there as they do elsewhere.
            let mine = !self.volume.hovering(Instant::now())
                && match self.scene.screen_mut::<MapEditor>() {
                    Some(MapEditor { view, .. }) => view.wheel(notches, keys),
                    None => true,
                };
            if !mine {
                self.turn_volume(notches);
            }
            return;
        }
        if self.scene.screen_mut::<EditorHome>().is_some() && !self.social.open {
            let x = self.mouse_virtual().0;
            let keys = self.editor_keys();
            if let Some(h) = self.scene.screen_mut::<EditorHome>() {
                h.home.wheel(notches, x, keys.ctrl || keys.alt);
            }
            return;
        }
        if self.layers_wheel(&app::overlays::WHEEL_LAYERS, notches) {
            return;
        }
        if let Some(dd) = &mut self.settings_ui.dropdown {
            dd.wheel(notches);
            return;
        }
        if self.scene.screen_is::<Preview>() {
            let (vx, vy) = self.mouse_virtual();
            if self.preview_on_top(vx, vy) && self.preview.time_at(vx, vy).is_some() {
                self.preview
                    .seek_by(notches as f64 * 5000.0, self.audio.as_ref());
                return;
            }
        }
        let (vx, vy) = self.mouse_virtual();
        if let Some(h) = self.scene.screen_mut::<HudEdit>()
            && h.ed.wheel(vx, vy, notches, &mut self.cfg)
        {
            return;
        }
        if self.area_editable()
            && (self.cfg.wheel_resizes_playfield
                || self.scene.screen().is_some_and(|s| s.wheel_scales_area()))
        {
            self.cfg.playfield_area = self.cfg.playfield_area.scaled(1.05f32.powi(notches));
            self.cfg.save();
            let text = tr!("playfield: {}", "プレイフィールド: {}", "поле: {}"; self.cfg.playfield_area.describe());
            self.toast(text);
            return;
        }
        if let Some(dd) = &mut self.settings_ui.dropdown {
            dd.wheel(notches);
            return;
        }
        let (vx, vy) = self.mouse_virtual();
        if let Some(r) = self.scene.screen_mut::<ResultsScreen>() {
            r.results.scroll(notches < 0);
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
        let on_board = self
            .board_rect
            .is_some_and(|[bx, by, bw, bh]| vx >= bx && vx <= bx + bw && vy >= by && vy <= by + bh);
        if self.scene.is_select() && on_board && self.select.picker.is_none() {
            self.board_scroll = (self.board_scroll as i64 - notches as i64)
                .clamp(0, self.board_max as i64) as usize;
            return;
        }
        match &self.scene {
            Scene::Other(sel)
                if sel.select_screen()
                    && self.preview_on_top(vx, vy)
                    && self.preview.time_at(vx, vy).is_some() =>
            {
                self.preview
                    .seek_by(notches as f64 * 5000.0, self.audio.as_ref())
            }
            Scene::Other(sel) if sel.select_screen() && self.select.picker.is_some() => {
                let n = self.select.picker_len() as i32;
                self.select.picker = self
                    .select
                    .picker
                    .map(|c| (c as i32 - notches).clamp(0, n - 1) as usize);
            }
            Scene::Other(sel) if sel.select_screen() => {
                self.select.scroll_by(-notches, &self.library)
            }
            Scene::Other(s)
                if s.previews_remote()
                    && self.preview_on_top(vx, vy)
                    && self.preview.time_at(vx, vy).is_some() =>
            {
                self.preview
                    .seek_by(notches as f64 * 5000.0, self.audio.as_ref())
            }
            Scene::Other(s) if s.previews_remote() => self.browse.scroll_by(notches),
            Scene::Other(s) if s.settings() && self.settings_ui.dropdown.is_some() => {
                if let Some(dd) = &mut self.settings_ui.dropdown {
                    dd.wheel(notches);
                }
            }
            Scene::Other(s) if s.settings() && self.settings_ui.choices.is_some() => {
                if let Some(p) = &mut self.settings_ui.choices {
                    p.move_cursor(-notches);
                }
            }
            Scene::Other(s) if s.settings() => self.settings_ui.scroll_by(-notches),
            _ => {}
        }
    }

    /// Whether touches play the drum now (a song or calibration running, not paused); otherwise
    /// the first finger works as the mouse.
    fn touch_drums(&self) -> bool {
        match &self.scene {
            Scene::Other(s) if s.song_play().is_some() => self
                .game()
                .is_some_and(|game| game.paused().is_none() && game.replay().is_none()),
            Scene::Other(s) if s.holds_game() => {
                self.game().is_some_and(|game| game.paused().is_none())
            }
            _ => false,
        }
    }

    /// A finger on a touch screen (`x`, `y` in pixels). During play each new finger hits the drum
    /// in its column (`cfg.touch_layout`), timed like a key press; elsewhere the first finger
    /// is the mouse (tap = click, drag = drag).
    fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f64,
        y: f64,
        at: Instant,
        window: Option<&Window>,
    ) {
        self.idle.input(at);
        self.touch_used = true;
        if self.mobile_touch(id, phase, x, y, at, window) {
            return;
        }
        if phase == TouchPhase::Started
            && self.touch_drums()
            && mobile::on_pause_button(self.mouse_at(x, y), self.window_width())
        {
            // Calibrating, it finishes with the hits so far; the back gesture cancels.
            let code = if self.calibration_run().is_some() {
                KeyCode::Enter
            } else {
                KeyCode::Escape
            };
            self.key(code, false, None, at, window);
            return;
        }
        if phase == TouchPhase::Started && self.touch_drums() {
            let (vx, vy) = self.mouse_at(x, y);
            if self.hit_at(vx, vy) == Some(hit::Hit::SkipIntro) {
                self.click(hit::Hit::SkipIntro, window);
                return;
            }
            let col = (x / self.window_w.max(1.0) as f64 * 4.0)
                .floor()
                .clamp(0.0, 3.0) as usize;
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

    /// The window's pixels `x`, `y` in virtual units.
    fn mouse_at(&self, x: f64, y: f64) -> (f32, f32) {
        let k = render::VIRTUAL_HEIGHT / self.window_h.max(1.0);
        (x as f32 * k, y as f32 * k)
    }

    /// The canvas's width in virtual units.
    fn window_width(&self) -> f32 {
        render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0)
    }

    /// The touch columns along the bottom of the screen during play, each flashing when hit.
    fn draw_touch_zones(&self, c: &mut Canvas, now: Instant) {
        mobile::draw_pause_button(c, self.calibration_run().is_some());
        let col_w = c.width / 4.0;
        let h = render::VIRTUAL_HEIGHT;
        for (col, key) in self.cfg.touch_layout.keys().into_iter().enumerate() {
            let colours = crate::game::note_colours();
            let color = if key == 0 || key == 3 {
                colours.kat()
            } else {
                colours.don()
            };
            let x = col as f32 * col_w;
            let flash = self.touch_flash[col]
                .map(|t| 1.0 - (now.saturating_duration_since(t).as_secs_f32() / 0.15).min(1.0))
                .unwrap_or(0.0);
            if flash > 0.0 {
                c.rect(x, 0.0, col_w, h, rgba(color, 0.10 * flash));
            }
            c.rect(
                x + 2.0,
                h - 6.0,
                col_w - 4.0,
                6.0,
                rgba(color, 0.35 + 0.5 * flash),
            );
        }
    }

    /// Pointer moved: drags the area if a drag is active, otherwise updates the hover state.
    /// Returns the cursor to show.
    fn mouse_moved(&mut self, x: f64, y: f64) -> CursorIcon {
        let now = Instant::now();
        let mut moved = (x, y) != self.mouse;
        // While gameplay has the pointer hidden, a twitch (a bumped or vibrating desk, sensor
        // noise) doesn't count: it has to travel a little before it shows again.
        if moved && self.cursor_hidden(now) && self.idle.amount() < 1.0 {
            let (fx, fy) = self.mouse_moved_from;
            moved = (x - fx).hypot(y - fy) >= pointer_wake_px(self.window_px().1);
        }
        if moved {
            self.mouse_moved_at = now;
            self.mouse_moved_from = (x, y);
            self.idle.input(now);
        }
        self.mouse = (x, y);
        let (vx, vy) = self.mouse_virtual();
        let width = render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0);
        self.volume.point(vx, vy, width, Instant::now());
        // On the main menu, pointing at a note selects it (moving: a pointer left resting doesn't
        // take the selection back from the keys).
        if moved
            && self.on_menu()
            && self.player.is_none()
            && let Some(hit::Hit::MenuItem(i)) = self.hit_at(vx, vy)
            && let Some(m) = self
                .main_menu
                .as_mut()
                .filter(|m| !m.is_leaving() && !m.is_selected(i))
        {
            m.pick(i);
        }
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
            if !self.right_scroll && ((vx - px).abs() > DRAG_START || (vy - py).abs() > DRAG_START)
            {
                self.right_scroll = true;
            }
        }
        let (_, drag_y) = self.mouse_virtual();
        if let Some(d) = &mut self.list_drag {
            let dy = drag_y - d.from_y;
            d.moved |= dy.abs() > DRAG_START;
            if d.moved && self.scene.is_select() {
                self.select.drag_to(dy);
                return CursorIcon::NsResize;
            }
        }
        if let Some(i) = self.settings_level {
            self.set_level(i);
            return CursorIcon::EwResize;
        }
        if self.scrollbar_drag {
            if self.scene.is_select() {
                let (_, vy) = self.mouse_virtual();
                self.select.scroll_to(vy, &self.library);
                return CursorIcon::NsResize;
            }
            self.scrollbar_drag = false;
        }
        if self.right_scroll {
            if self.scene.is_select() {
                let (_, vy) = self.mouse_virtual();
                self.select.scroll_to(vy, &self.library);
                return CursorIcon::NsResize;
            }
            self.right_scroll = false;
        }
        if let Some((_, from, start)) = self.split_drag {
            if !self.scene.is_select() {
                self.split_drag = None;
            } else {
                let (vx, _) = self.mouse_virtual();
                let width = render::VIRTUAL_HEIGHT * self.window_w / self.window_h.max(1.0);
                // A drag, not the first click of a double click.
                if (vx - from).abs() > 3.0 {
                    self.split_click = None;
                }
                // Kept where the list's width still follows it.
                if let Some((min, max)) = Select::share_range(width) {
                    self.cfg.song_list_share = (start + (vx - from) / width).clamp(min, max);
                }
                return CursorIcon::EwResize;
            }
        }
        let (sw, sh) = (self.window_w.max(1.0) as f64, self.window_h.max(1.0) as f64);
        let (vx, vy) = self.mouse_virtual();
        if self.scene.screen_is::<MapEditor>() && !self.social.open {
            let keys = self.editor_keys();
            if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>() {
                view.mouse_move(vx, vy, keys);
                return view.cursor();
            }
            return CursorIcon::Default;
        }
        if let Some(h) = self.scene.screen_mut::<EditorHome>()
            && h.home.dragging()
        {
            h.home.mouse_move(vx);
            return CursorIcon::Grabbing;
        }
        if let Some(cursor) = self
            .with_screen(|s, state| s.pointer_moved(state, x, y))
            .flatten()
        {
            return cursor;
        }
        if let Some(dd) = &mut self.settings_ui.dropdown {
            if dd.drag_bar(vy) {
                return CursorIcon::Default;
            }
            if dd.button_at(vx, vy).is_some() {
                return CursorIcon::Pointer;
            }
            if let Some(i) = dd.row_at(vx, vy).filter(|_| dd.editing.is_none()) {
                dd.cursor = i;
                return CursorIcon::Pointer;
            }
            return CursorIcon::Default;
        }
        if self.grip.is_none()
            && self
                .readout_rect
                .is_some_and(|r| vx >= r[0] && vx <= r[0] + r[2] && vy >= r[1] && vy <= r[1] + r[3])
        {
            self.hover = None;
            return CursorIcon::Pointer;
        }
        if let Some(ed) = self
            .scene
            .screen_mut::<HudEdit>()
            .map(|h| &mut h.ed)
            .filter(|ed| ed.dragging())
        {
            ed.drag_to(vx, vy, self.modifiers_shift, &mut self.cfg);
            return CursorIcon::Move;
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
        // The preview's own areas go to its check below: a hand only where they take a click.
        if self.scrubbing.is_none()
            && let Some(hit) = self
                .hit_at(vx, vy)
                .filter(|h| !matches!(h, hit::Hit::Preview))
        {
            if let (hit::Hit::PauseItem(i), true, Some(game)) =
                (hit, self.scene.is_play(), self.play.as_deref_mut())
            {
                game.set_pause_cursor(i);
            }
            self.hover = None;
            return match hit {
                hit::Hit::SplitList => CursorIcon::EwResize,
                _ => CursorIcon::Pointer,
            };
        }
        if let Some(drag) = self.player.as_ref().and_then(|p| p.drag) {
            match drag {
                player::Drag::Volume => self.music_volume_to_pointer(),
                player::Drag::Timeline
                    if self
                        .music_seeked
                        .is_none_or(|t| t.elapsed() >= Duration::from_millis(33)) =>
                {
                    self.music_seeked = Some(Instant::now());
                    self.music_seek_to_pointer();
                }
                player::Drag::Timeline => {}
            }
            return CursorIcon::Pointer;
        }
        if !self.on_menu() && self.in_song_select() {
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
            let on_box = self.scene.is_select() && self.preview.box_at(vx, vy);
            if self.preview_on_top(vx, vy)
                && (self.preview.time_at(vx, vy).is_some()
                    || self.preview.play_button_at(vx, vy)
                    || on_box)
            {
                self.hover = None;
                return CursorIcon::Pointer;
            }
        }
        if !self.area_editable() {
            self.hover = None;
            let (vx, vy) = self.mouse_virtual();
            if let Some(dd) = &mut self.settings_ui.dropdown {
                if dd.drag_bar(vy) {
                    return CursorIcon::Default;
                }
                if let Some(i) = dd.row_at(vx, vy) {
                    dd.cursor = i;
                    return CursorIcon::Pointer;
                }
            }
            return CursorIcon::Default;
        }
        // A hidden pointer isn't pointing at an edge: no frame for it.
        self.hover = if self.cursor_hidden(now) {
            None
        } else {
            layout::hit(self.cfg.playfield_area, x, y, sw, sh)
        };
        self.hover
            .map(|g| g.cursor())
            .unwrap_or(CursorIcon::Default)
    }

    /// The pointer left the window: it no longer hovers an edge of the playfield area, so the
    /// area's frame doesn't stay up (a drag in progress keeps going).
    fn mouse_left(&mut self) {
        self.hover = None;
    }

    /// Right button on song select: hold it and move up and down to fly through the list, like
    /// osu!'s right-click scrolling. The selection stays.
    /// Released without moving, it opens the row's menu; on the scrollbar, it goes there.
    /// The middle button: pans the map editor's timeline.
    fn middle_button(&mut self, pressed: bool) {
        if self.scene.screen_is::<MapEditor>() && !self.social.open {
            let (vx, vy) = self.mouse_virtual();
            if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>() {
                view.middle(pressed, vx, vy);
            }
        }
    }

    fn right_button(&mut self, pressed: bool) {
        // A click that wakes the idle screen does nothing else, its release neither.
        if pressed {
            if self.idle.input(Instant::now()) {
                self.idle.swallow_right = true;
                return;
            }
        } else if std::mem::take(&mut self.idle.swallow_right) {
            return;
        }
        if self.scene.screen_is::<MapEditor>() && !self.social.open {
            if pressed {
                let (vx, vy) = self.mouse_virtual();
                if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>() {
                    view.right_click(vx, vy);
                }
            }
            return;
        }
        if self.mod_select.is_some() {
            // Right-clicking a mod makes it a favourite (or not any more).
            if pressed && self.customization_tab().is_none() {
                let (vx, vy) = self.mouse_virtual();
                match self.hit_at(vx, vy) {
                    Some(hit::Hit::ModToggle(i)) => {
                        self.mod_select = Some(i);
                        self.toggle_favourite(mods::ALL[i]);
                    }
                    Some(hit::Hit::ModPreset(k))
                        if self.preset_naming.is_none() && self.preset_deleting.is_none() =>
                    {
                        self.preset_deleting = Some(k);
                    }
                    _ => {}
                }
            }
            return;
        }
        if self.on_menu() {
            return;
        }
        let free = self.splash.is_none()
            && self.select.picker.is_none()
            && self.settings_ui.dropdown.is_none();
        let (vx, vy) = self.mouse_virtual();
        if pressed {
            self.select.menu = None;
            self.right_scroll = false;
            self.right_press =
                (free && self.scene.is_select()).then(|| (vx, vy, self.hit_at(vx, vy)));
            return;
        }
        let press = self.right_press.take();
        if std::mem::take(&mut self.right_scroll) || !self.scene.is_select() {
            return;
        }
        match press {
            Some((x, y, Some(hit::Hit::SongRow(row)))) => {
                let skin = self.menu_has_skin(row);
                self.select.menu = Some(ui::SongMenu {
                    row,
                    header: false,
                    skin,
                    x,
                    y,
                })
            }
            Some((x, y, Some(hit::Hit::SongHeader(row)))) => {
                let skin = self.menu_has_skin(row);
                self.select.menu = Some(ui::SongMenu {
                    row,
                    header: true,
                    skin,
                    x,
                    y,
                })
            }
            Some((_, y, Some(hit::Hit::SongScrollbar))) => self.select.scroll_to(y, &self.library),
            _ => {}
        }
    }

    /// Whether the song menu opened on `row` offers turning the map's own skin off: it has one,
    /// and beatmap skins aren't all off anyway. Read once here, not in the menu's every frame.
    fn menu_has_skin(&self, row: usize) -> bool {
        self.cfg.map_skins
            && self
                .select
                .row_map(row)
                .and_then(|i| self.library.get(i))
                .and_then(|e| e.path.parent())
                .is_some_and(skin::map_has_images)
    }

    fn mouse_button(&mut self, pressed: bool, window: Option<&Window>) {
        if self.layers_button(&[app::overlays::Layer::Shot], pressed, window) {
            return;
        }
        if pressed {
            if self.idle.input(Instant::now()) {
                self.idle.swallow_left = true;
                return;
            }
        } else if std::mem::take(&mut self.idle.swallow_left) {
            return;
        }
        // Letting go of the preset list's scroll bar is all a release does then.
        if !pressed
            && self
                .settings_ui
                .dropdown
                .as_mut()
                .is_some_and(Dropdown::release_bar)
        {
            return;
        }
        if !pressed
            && let Some(h) = self.scene.screen_mut::<EditorHome>()
            && h.home.dragging()
        {
            let vx = self.mouse_virtual().0;
            if let Some(h) = self.scene.screen_mut::<EditorHome>() {
                h.home.release(vx, self.audio.as_ref());
            }
            return;
        }
        if self.scene.screen_is::<MapEditor>() && !self.social.open && self.splash.is_none() {
            let (vx, vy) = self.mouse_virtual();
            let keys = self.editor_keys();
            if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>() {
                if pressed {
                    view.mouse_down(vx, vy, keys, self.audio.as_ref());
                } else {
                    view.mouse_up(vx, vy, keys);
                }
            }
            return;
        }
        if self.layers_button(&app::overlays::BUTTON_LAYERS, pressed, window) {
            return;
        }
        // A replay's buttons: nothing else in a song is clicked.
        if pressed
            && self.replay_bar.is_some()
            && let (vx, vy) = self.mouse_virtual()
            && let Some(hit @ hit::Hit::Key(_)) = self.hit_at(vx, vy)
        {
            self.click(hit, window);
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
        if self.with_screen(|s, state| s.mouse_button(state, pressed)) == Some(true) {
            return;
        }
        if self.on_menu() {
            let (vx, vy) = self.mouse_virtual();
            if let Some(hit) = self.hit_at(vx, vy).filter(|_| pressed) {
                self.click(hit, window);
            }
            if let Some(p) = self.player.as_mut().filter(|_| !pressed)
                && p.drag.take() == Some(player::Drag::Volume)
            {
                self.cfg.save();
            }
            return;
        }
        if self.in_song_select() {
            let (vx, vy) = self.mouse_virtual();
            if pressed {
                if !self.preview_on_top(vx, vy) {
                    // Something's drawn over the preview here: it takes the click below.
                } else if self.preview.play_button_at(vx, vy) {
                    if let Some(p) = self.scene.screen_mut::<Preview>() {
                        let lobby = screens::screen::Screen::over_lobby(p);
                        self.preview_button(lobby);
                    } else if !self.picking_lobby_map {
                        self.play_from_marker();
                    }
                    return;
                } else if self.scene.is_select() && self.preview.box_at(vx, vy) {
                    self.open_preview();
                    return;
                } else if let Some(t) = self.preview.time_at(vx, vy) {
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
            if self
                .readout_rect
                .is_some_and(|r| vx >= r[0] && vx <= r[0] + r[2] && vy >= r[1] && vy <= r[1] + r[3])
            {
                // Clicking the resolution readout opens the preset list; single-player pauses
                // first (a multiplayer match keeps running).
                if (self.scene.is_play() || self.calibration_run().is_some())
                    && let Some(game) = self.play.as_deref_mut()
                {
                    game.pause(sound(&self.audio));
                }
                self.open_dropdown();
                return;
            }
        }
        if pressed && self.settings_ui.dropdown.is_some() {
            let (vx, vy) = self.mouse_virtual();
            if let Some(dd) = self
                .settings_ui
                .dropdown
                .as_mut()
                .filter(|d| d.editing.is_none() && d.on_bar(vx, vy))
            {
                dd.grab_bar(vy);
                return;
            }
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
            if self.scene.is_select() {
                self.select.searching = hit == Some(hit::Hit::TextField);
            }
            if self.select.picker.is_some()
                && self.scene.is_select()
                && !matches!(hit, Some(hit::Hit::PickRow(_) | hit::Hit::PickRowMore(_)))
            {
                self.select.picker = None;
                return;
            }
            if self.select.menu.is_some()
                && self.scene.is_select()
                && !matches!(hit, Some(hit::Hit::SongMenu(_)))
            {
                self.select.menu = None;
                return;
            }
            if let (true, Some(h @ hit::Hit::SplitList)) = (self.scene.is_select(), hit) {
                // Twice in quick succession: back to the usual size.
                let now = Instant::now();
                if self.split_click.take().is_some_and(|(last, at)| {
                    last == h && now.duration_since(at) < Duration::from_millis(400)
                }) {
                    self.reset_song_layout();
                    return;
                }
                self.split_click = Some((h, now));
                self.split_drag = Some((h, vx, self.cfg.song_list_share));
                return;
            }
            // A row acts on release, unless the press turns into dragging the list.
            if let (true, Some(hit::Hit::SongScrollbar)) = (self.scene.is_select(), hit) {
                self.scrollbar_drag = true;
                self.select.scroll_to(vy, &self.library);
                return;
            }
            if let (true, Some(h @ (hit::Hit::SongRow(_) | hit::Hit::SongHeader(_)))) =
                (self.scene.is_select(), hit)
            {
                self.list_drag = Some(ListDrag {
                    from_y: vy,
                    hit: h,
                    moved: false,
                });
                return;
            }
            if self.settings_ui.choices.is_some() && !matches!(hit, Some(hit::Hit::ChoiceRow(_))) {
                self.settings_ui.choices = None;
                return;
            }
            // Clicking anywhere while typing a value leaves it as it was.
            if self.settings_ui.editing.take().is_some() {
                return;
            }
            if let Some(ed) = self
                .scene
                .screen_mut::<HudEdit>()
                .map(|h| &mut h.ed)
                .filter(|ed| ed.naming())
            {
                ed.finish_naming(&mut self.cfg);
            }
            if let Some(hit) = hit {
                self.click(hit, window);
                return;
            }
            let (vx, vy) = self.mouse_virtual();
            if let Some(h) = self.scene.screen_mut::<HudEdit>()
                && h.ed.press(vx, vy, &self.cfg)
            {
                return;
            }
            if self.area_editable() {
                let (sw, sh) = (self.window_w as f64, self.window_h as f64);
                if let Some(g) =
                    layout::hit(self.cfg.playfield_area, self.mouse.0, self.mouse.1, sw, sh)
                {
                    self.grip = Some((g, self.mouse, self.cfg.playfield_area));
                }
            }
        } else if self
            .scene
            .screen_mut::<HudEdit>()
            .is_some_and(|h| h.ed.release(&self.cfg))
        {
        } else if self.grip.take().is_some() {
            self.guides.clear();
            self.cfg.save();
        }
    }

    /// Scripted drag for headless tests: press at the area centre (or bottom-right corner).
    fn simulate_drag(&mut self, corner: bool, dx: f64, dy: f64, hold: bool) {
        let a = self.cfg.playfield_area;
        let (sw, sh) = (self.window_w as f64, self.window_h as f64);
        let (fx, fy) = if corner {
            (a.x + a.w, a.y + a.h)
        } else {
            (a.x + a.w / 2.0, a.y + a.h / 2.0)
        };
        let start = (
            fx as f64 * sw - if corner { 2.0 } else { 0.0 },
            fy as f64 * sh - if corner { 2.0 } else { 0.0 },
        );
        self.mouse_moved(start.0, start.1);
        self.mouse_button(true, None);
        self.mouse_moved(start.0 + dx, start.1 + dy);
        if !hold {
            self.mouse_button(false, None);
        }
    }

    fn open_multiplayer(&mut self) {
        self.scene = if self.mp.is_some() {
            Lobby::scene()
        } else {
            MpMenu::scene()
        };
    }

    /// A library map's numbers for a lobby (stars, length and BPM -1 while the background work
    /// hasn't reached it).
    fn map_stats(&self, entry: usize) -> Option<Box<net::MapStats>> {
        let e = self.library.get(entry)?;
        let info = self
            .select
            .info
            .as_ref()
            .and_then(|i| i.lock().unwrap().get(entry).copied().flatten());
        Some(Box::new(net::MapStats {
            stars: info.map_or(-1.0, |i| i.stars),
            length_ms: info.map_or(-1.0, |i| i.length_ms),
            bpm: info.map_or(-1.0, |i| i.bpm),
            od: e.meta.od as f32,
            hp: e.meta.hp as f32,
            creator: e.meta.creator.clone(),
        }))
    }

    fn pick_lobby_map(&mut self, entry: usize) {
        self.picking_lobby_map = false;
        if let (Some(s), Some(g)) = (self.mp.as_ref().filter(|s| s.mapguessr), &mut self.guessr) {
            let e = &self.library[entry];
            if g.can_guess(s) {
                s.client.send(net::ClientMsg::Guess {
                    hash: e.hash(),
                    song: mapguessr::entry_song(e),
                });
                g.last_guess = Some(format!("{} - {}", e.meta.artist, e.meta.title));
            }
            self.scene = Lobby::scene();
            return;
        }
        let mut info = map_info(&self.library[entry], self.play_speed().0, self.cfg.mods);
        info.stats = self.map_stats(entry);
        info.seed = lobby_seed(Some(self.cfg.random_seed));
        // Defense stays on (with its rules) from one map to the next.
        info.defense = self.lobby_defense();
        let lobby_mods = self.cfg.mods.lobby(info.defense);
        info.mods = lobby_mods.list();
        (info.multiverse, info.shapeshift) = mod_settings(lobby_mods);
        if let Some(s) = &self.mp {
            s.client.send(net::ClientMsg::SelectMap(info));
        }
        self.scene = Lobby::scene();
    }

    /// The lobby's Defense rules, when the map picked is a Defense match.
    fn lobby_defense(&self) -> Option<net::Defense> {
        self.mp
            .as_ref()
            .and_then(|s| s.map.as_ref())
            .and_then(|m| m.defense)
    }

    /// T or a click on the chat line: the keys go to a message until Enter or Esc.
    fn focus_lobby_chat(&mut self) {
        if let Some(s) = self.mp.as_mut().filter(|s| !s.mapguessr) {
            s.chat_input.get_or_insert_with(String::new);
            s.chat_caret = s.chat_input.as_ref().map_or(0, |t| t.chars().count());
        }
    }

    /// A key while a lobby chat line is being typed.
    fn lobby_chat_key(&mut self, code: KeyCode, text: Option<&str>) {
        let Some(s) = self.mp.as_mut() else {
            return;
        };
        let Some(typed) = s.chat_input.as_mut() else {
            return;
        };
        match code {
            KeyCode::Escape => s.chat_input = None,
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let mut line = typed.trim().to_string();
                // A line sent leaves the box open for the next; Enter on an empty one puts it away.
                s.chat_input = (!line.is_empty()).then(String::new);
                s.chat_caret = 0;
                // `/np`: the map on screen, with its osu! link.
                if line.eq_ignore_ascii_case("/np") {
                    match self.now_playing() {
                        Some(np) => line = np,
                        None => {
                            self.toast("no map to share: pick one first".into());
                            return;
                        }
                    }
                }
                if !line.is_empty()
                    && let Some(s) = &self.mp
                {
                    s.client.send(net::ClientMsg::Chat(line));
                }
            }
            KeyCode::Backspace => settings::caret_backspace(typed, &mut s.chat_caret),
            _ if settings::caret_nav(typed, &mut s.chat_caret, code) => {}
            _ => settings::caret_insert(typed, &mut s.chat_caret, text.unwrap_or(""), 200),
        }
    }

    fn leave_room(&mut self, why: &str) {
        self.stop_audio();
        self.mp = None;
        self.picking_lobby_map = false;
        self.mp_status = why.to_string();
        self.scene = MpMenu::scene();
    }

    /// A key in a mapguessr lobby.
    fn guessr_key(&mut self, code: KeyCode) {
        let (Some(s), Some(g)) = (&self.mp, &mut self.guessr) else {
            return;
        };
        let in_game = s.in_match;
        match code {
            KeyCode::KeyG
            | KeyCode::Tab
            | KeyCode::Space
            | KeyCode::Enter
            | KeyCode::NumpadEnter
                if in_game =>
            {
                if g.can_guess(s) {
                    self.picking_lobby_map = true;
                    self.scene = SongSelect::scene();
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
                s.client
                    .send(net::ClientMsg::StartGuessr { rounds: g.rounds });
            }
            KeyCode::KeyS if !in_game => {
                let on = !s.me().is_some_and(|p| p.spectator);
                s.client.send(net::ClientMsg::Spectator(on));
                self.toast(
                    if on {
                        "you'll watch the next game"
                    } else {
                        "you'll play the next game"
                    }
                    .into(),
                );
            }
            KeyCode::Tab => self.scene = SongSelect::scene(),
            KeyCode::Escape => {
                let why = if s.owns_room() {
                    "room closed"
                } else {
                    "left the room"
                };
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
            KeyCode::ArrowUp => {
                self.mp_menu.cursor = self.mp_menu.cursor.min(rows - 1).saturating_sub(1)
            }
            KeyCode::ArrowDown => self.mp_menu.cursor = (self.mp_menu.cursor + 1).min(rows - 1),
            // The new lobby's steppers step; elsewhere the arrows go between the lobbies
            // (left) and the new lobby (right).
            KeyCode::ArrowLeft | KeyCode::ArrowRight if row == Row::NewMode => {
                self.mp_menu.new_guessr = !self.mp_menu.new_guessr
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight if row == Row::NewSlots => self
                .mp_menu
                .step_slots(if code == KeyCode::ArrowLeft { -1 } else { 1 }),
            KeyCode::ArrowLeft => self.mp_menu.cursor = 0,
            KeyCode::ArrowRight => self.mp_menu.select(Row::NewMode),
            KeyCode::F5 => self.mp_menu.refresh(&server),
            KeyCode::Escape if self.mp_menu.joining.is_some() => {
                self.mp_menu.joining = None;
                self.mp_menu.cursor = 0;
            }
            KeyCode::Escape if self.mp_menu.form != multiplayer::Form::Main => {
                self.mp_menu.open(multiplayer::Form::Main);
                self.mp_status.clear();
            }
            KeyCode::Escape | KeyCode::Tab => self.scene = SongSelect::scene(),
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
                    Row::NewPassword => {
                        self.mp_menu.new_password.pop();
                    }
                    Row::JoinPassword => {
                        self.mp_menu.join_password.pop();
                    }
                    _ => {}
                }
                self.cfg.save();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if self.mp_connecting.is_some() {
                    return;
                }
                let token = self.cfg.online_token.clone();
                let account = self.mp_menu.account.clone().unwrap_or_default();
                use multiplayer::{CodeFor, Form};
                let menu = &mut self.mp_menu;
                let code = (!menu.code.trim().is_empty()).then(|| menu.code.trim().to_string());
                let email = menu.email.trim().to_string();
                let busy = menu.account_request.is_some();
                match row {
                    Row::User | Row::Email | Row::Code => {
                        self.mp_menu.cursor = (self.mp_menu.cursor + 1).min(rows - 1)
                    }
                    // Only on the login form does the password send.
                    Row::Password if menu.form != Form::Main => {
                        self.mp_menu.cursor = (self.mp_menu.cursor + 1).min(rows - 1)
                    }
                    // The ways to the other forms.
                    Row::Register if menu.form == Form::Main => {
                        menu.open(Form::Register);
                        self.mp_status.clear();
                    }
                    Row::Forgot if menu.form == Form::Main => {
                        menu.open(Form::Reset);
                        self.mp_status.clear();
                    }
                    Row::ChangePassword if menu.has_email == Some(false) => {
                        menu.open(Form::Link);
                        self.mp_status = tr!("link an email first: the code for a new password goes there", "先にメールを登録してください: 新しいパスワードのコードはそこに届きます", "сначала привяжи почту: код для нового пароля придёт туда").into();
                    }
                    Row::ChangePassword => {
                        menu.open(Form::Reset);
                        self.mp_status.clear();
                    }
                    Row::LinkEmail if menu.form != Form::Link => {
                        menu.open(Form::Link);
                        self.mp_status.clear();
                    }
                    Row::Back => {
                        menu.open(Form::Main);
                        self.mp_status.clear();
                    }
                    Row::LogIn | Row::Register | Row::Password | Row::Forgot | Row::LinkEmail
                        if busy => {}
                    Row::Forgot => {
                        // Logged in, it's this account's password.
                        let who = match &menu.account {
                            Some(a) => a.clone(),
                            None => menu.user.trim().to_string(),
                        };
                        if who.is_empty() {
                            self.mp_status = tr!(
                                "type your name or email first",
                                "先に名前かメールを入力してください",
                                "сначала впиши ник или почту"
                            )
                            .into();
                            self.mp_menu.select(Row::User);
                        } else if menu.code_for == Some(CodeFor::Reset) {
                            if code.is_none() {
                                self.mp_status = tr!(
                                    "type the code from the email first",
                                    "先にメールのコードを入力してください",
                                    "сначала введи код из письма"
                                )
                                .into();
                                self.mp_menu.select(Row::Code);
                            } else if !online::valid_password(&menu.password) {
                                self.mp_status = tr!(
                                    "type a new password of at least 6 characters",
                                    "6文字以上の新しいパスワードを入力してください",
                                    "впиши новый пароль от 6 символов"
                                )
                                .into();
                                self.mp_menu.select(Row::Password);
                            } else {
                                let password = Some(menu.password.clone());
                                self.mp_status = tr!(
                                    "setting the new password...",
                                    "新しいパスワードを設定中...",
                                    "сохраняю новый пароль..."
                                )
                                .into();
                                menu.account_call(
                                    &server,
                                    online::Request::ResetPassword {
                                        who,
                                        code,
                                        password,
                                    },
                                );
                            }
                        } else {
                            menu.asking = Some(CodeFor::Reset);
                            self.mp_status = tr!(
                                "sending a code to the account's email...",
                                "アカウントのメールにコードを送信中...",
                                "отправляю код на почту аккаунта..."
                            )
                            .into();
                            menu.account_call(
                                &server,
                                online::Request::ResetPassword {
                                    who,
                                    code: None,
                                    password: None,
                                },
                            );
                        }
                    }
                    Row::LinkEmail => {
                        if !email.contains('@') {
                            self.mp_status = tr!(
                                "type your email first",
                                "先にメールを入力してください",
                                "сначала впиши почту"
                            )
                            .into();
                            self.mp_menu.select(Row::Email);
                        } else if menu.code_for == Some(CodeFor::Link) && code.is_some() {
                            self.mp_status = tr!(
                                "linking the email...",
                                "メールを登録中...",
                                "привязываю почту..."
                            )
                            .into();
                            menu.account_call(
                                &server,
                                online::Request::LinkEmail { token, email, code },
                            );
                        } else {
                            menu.asking = Some(CodeFor::Link);
                            self.mp_status = tr!("sending a code to {email}...", "{email} にコードを送信中...", "отправляю код на {email}...";);
                            menu.account_call(
                                &server,
                                online::Request::LinkEmail {
                                    token,
                                    email,
                                    code: None,
                                },
                            );
                        }
                    }
                    Row::Register => {
                        let user = menu.user.trim().to_string();
                        if !online::valid_name(&user) {
                            self.mp_status =
                                tr!("names are {}", "名前は {}", "ник: {}"; online::NAME_RULES);
                            self.mp_menu.select(Row::User);
                        } else if !online::valid_password(&menu.password) {
                            self.mp_status = tr!(
                                "pick a password of at least 6 characters",
                                "6文字以上のパスワードを決めてください",
                                "придумай пароль от 6 символов"
                            )
                            .into();
                            self.mp_menu.select(Row::Password);
                        } else if !email.contains('@') {
                            self.mp_status = tr!(
                                "type your email: the code goes there",
                                "メールを入力してください: コードはそこに届きます",
                                "впиши почту: туда придёт код"
                            )
                            .into();
                            self.mp_menu.select(Row::Email);
                        } else {
                            let code = code.filter(|_| menu.code_for == Some(CodeFor::Register));
                            self.mp_status = if code.is_some() {
                                tr!("creating {user}...", "{user} を作成中...", "создаю {user}...";)
                            } else {
                                tr!("sending a code to {email}...", "{email} にコードを送信中...", "отправляю код на {email}...";)
                            };
                            menu.asking = Some(CodeFor::Register);
                            let password = menu.password.clone();
                            menu.account_call(
                                &server,
                                online::Request::Register {
                                    name: user,
                                    password,
                                    email: Some(email),
                                    code,
                                },
                            );
                        }
                    }
                    Row::LogIn | Row::Password => {
                        let user = menu.user.trim().to_string();
                        if !online::valid_login(&user) {
                            self.mp_status = tr!(
                                "type your name (or the account's email)",
                                "名前(またはアカウントのメール)を入力してください",
                                "впиши ник (или почту аккаунта)"
                            )
                            .into();
                            self.mp_menu.select(Row::User);
                        } else if menu.password.is_empty() {
                            self.mp_status = tr!(
                                "type your password",
                                "パスワードを入力してください",
                                "впиши пароль"
                            )
                            .into();
                            self.mp_menu.select(Row::Password);
                        } else {
                            // Logging in forgets the typed password.
                            let password = std::mem::take(&mut menu.password);
                            self.mp_status = tr!("logging in as {user}...", "{user} でログイン中...", "вхожу как {user}...";);
                            menu.account_call(
                                &server,
                                online::Request::Login {
                                    name: user,
                                    password,
                                },
                            );
                        }
                    }
                    Row::LogOut => {
                        self.mp_menu
                            .account_call(&server, online::Request::Logout { token });
                        self.sign_out("logged out");
                    }
                    Row::NewMode => menu.new_guessr = !menu.new_guessr,
                    Row::NewSlots => menu.step_slots(1),
                    Row::NewPassword => {
                        self.mp_menu.cursor = (self.mp_menu.cursor + 1).min(rows - 1)
                    }
                    Row::HostOnline => {
                        let mapguessr = menu.new_guessr;
                        let settings = menu.new_settings();
                        self.mp_status = if mapguessr {
                            "opening a mapguessr lobby..."
                        } else {
                            "opening a lobby on the server..."
                        }
                        .into();
                        let request = online::Request::CreateRoom {
                            token,
                            title: String::new(),
                            mapguessr,
                            settings,
                        };
                        self.mp_connecting =
                            Some(multiplayer::start_online(server, request, account, None));
                    }
                    Row::Lobby(k) => {
                        let Some(room) = menu.rooms.get(k).cloned() else {
                            return;
                        };
                        if room.players >= room.slots {
                            self.mp_status = tr!(
                                "that lobby is full",
                                "そのロビーは満員です",
                                "это лобби заполнено"
                            )
                            .into();
                        } else if room.locked {
                            // Its password first.
                            menu.joining = Some(room);
                            menu.join_password.clear();
                            menu.cursor = 0;
                            self.mp_status.clear();
                        } else {
                            self.mp_status = format!("joining {}...", room.title);
                            let request = online::Request::JoinRoom {
                                token,
                                room: room.id,
                            };
                            self.mp_connecting =
                                Some(multiplayer::start_online(server, request, account, None));
                        }
                    }
                    Row::CancelJoin => {
                        menu.joining = None;
                        menu.cursor = 0;
                    }
                    Row::JoinPassword | Row::JoinLocked => {
                        let Some(room) = menu.joining.clone() else {
                            return;
                        };
                        if menu.join_password.is_empty() {
                            self.mp_status = tr!(
                                "type the lobby's password",
                                "ロビーのパスワードを入力",
                                "введи пароль лобби"
                            )
                            .into();
                            menu.select(Row::JoinPassword);
                            return;
                        }
                        let password = Some(std::mem::take(&mut menu.join_password));
                        menu.joining = None;
                        menu.cursor = 0;
                        self.mp_status = format!("joining {}...", room.title);
                        let request = online::Request::JoinRoom {
                            token,
                            room: room.id,
                        };
                        self.mp_connecting = Some(multiplayer::start_online(
                            server, request, account, password,
                        ));
                    }
                }
                self.cfg.save();
            }
            _ => {
                let Some(text) = text else { return };
                for ch in text.chars().filter(|c| net::drawable_char(*c)) {
                    match row {
                        Row::User if self.mp_menu.user.len() < 254 && ch.is_ascii_graphic() => {
                            self.mp_menu.user.push(ch)
                        }
                        Row::Password if self.mp_menu.password.chars().count() < 128 => {
                            self.mp_menu.password.push(ch)
                        }
                        Row::Email if self.mp_menu.email.len() < 254 && ch.is_ascii_graphic() => {
                            self.mp_menu.email.push(ch)
                        }
                        Row::Code if self.mp_menu.code.len() < 6 && ch.is_ascii_digit() => {
                            self.mp_menu.code.push(ch)
                        }
                        Row::NewPassword
                            if self.mp_menu.new_password.chars().count() < 32 && ch != ' ' =>
                        {
                            self.mp_menu.new_password.push(ch)
                        }
                        Row::JoinPassword
                            if self.mp_menu.join_password.chars().count() < 32 && ch != ' ' =>
                        {
                            self.mp_menu.join_password.push(ch)
                        }
                        _ => {}
                    }
                }
                self.cfg.save();
            }
        }
    }

    /// The present mode `dir` steps from the one in effect, among those the renderer can do.
    fn step_present(&self, dir: i32) -> PresentChoice {
        let modes = &self.present_modes;
        let at = modes
            .iter()
            .position(|&p| p == self.cfg.present.within(modes))
            .unwrap_or(0) as i32;
        modes
            .get((at + dir).rem_euclid(modes.len().max(1) as i32) as usize)
            .copied()
            .unwrap_or(PresentChoice::Fifo)
    }

    /// The renderer is picked when the graphics start: the game starts again with the new one.
    fn restart_for_renderer(&mut self) {
        // On disk before the new game reads it (saves are written in the background).
        self.cfg.save();
        config::flush_saves();
        #[cfg(not(target_os = "android"))]
        match update::restart() {
            Ok(()) => {
                eprintln!("closing: restarting for the renderer ({})", self.cfg.graphics_api);
                self.quit = true;
            }
            Err(e) => self.toast(tr!("restart taiko.rs to switch the renderer ({e})", "レンダラーの切り替えはtaiko.rsの再起動後 ({e})", "рендер сменится после перезапуска taiko.rs ({e})";)),
        }
        #[cfg(target_os = "android")]
        self.toast(
            tr!(
                "restart taiko.rs to switch the renderer",
                "レンダラーの切り替えはtaiko.rsの再起動後",
                "рендер сменится после перезапуска taiko.rs"
            )
            .into(),
        );
    }

    /// What's on screen, and the map, for a crash report (`CRASH_SCENE`).
    fn note_scene_for_crashes(&self) {
        let screen = match &self.scene {
            Scene::Other(sel) if sel.select_screen() && self.on_menu() => "main menu",
            Scene::Other(sel) if sel.select_screen() => "song select",
            Scene::Other(s) if s.song_play().is_some() => {
                if self.game().is_some_and(|game| game.replay().is_some()) {
                    "watching a replay"
                } else {
                    "playing"
                }
            }
            Scene::Other(s) => s.activity(),
        };
        let map = self
            .offset_entry()
            .and_then(|i| self.library.get(i))
            .map(|e| {
                format!(
                    ", map {} [{}] by {}",
                    e.meta.title, e.meta.version, e.meta.creator
                )
            })
            .unwrap_or_default();
        let mods = self.cfg.mods.label();
        let mods = if mods.is_empty() {
            String::new()
        } else {
            format!(", mods {mods}")
        };
        if let Ok(mut s) = CRASH_SCENE.lock() {
            *s = format!("screen: {screen}{map}{mods}");
        }
    }

    /// Forgets the saved online login.
    fn sign_out(&mut self, why: &str) {
        self.social = social::Social::default();
        self.select.nominator = false;
        self.admin = false;
        self.mp_menu.account = None;
        self.mp_menu.has_email = None;
        self.mp_menu.form = multiplayer::Form::Main;
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
        let scene = self.behind_settings().unwrap_or(&self.scene);
        let plays = scene.plays_song();
        let g = self.play.as_deref().filter(|_| plays)?;
        (g.replay().is_none() && !self.auto && !g.mods().has(mods::Mod::Autoplay))
            .then_some(g.played_ms())
    }

    /// The selected map's page on the website.
    fn open_map_website(&mut self) {
        let Some(i) = self.select.selected() else {
            return;
        };
        let url = map_url(self.library[i].hash());
        if !open_url(&url) {
            self.toast(url);
        }
    }

    /// The map of the live play, as the server is told it.
    fn live_map(&self) -> Option<online::PlayedMap> {
        let entry = self
            .behind_settings()
            .unwrap_or(&self.scene)
            .song_play()
            .map(|p| p.0);
        if let Some(e) = entry.and_then(|i| self.library.get(i)) {
            return Some(online::PlayedMap {
                hash: e.hash(),
                name: e.label(),
                set_id: e.set_id(),
            });
        }
        let map = self.mp.as_ref()?.map.as_ref()?;
        Some(online::PlayedMap {
            hash: map.hash,
            name: format!("{} - {} [{}]", map.artist, map.title, map.version),
            set_id: map.set_id,
        })
    }

    /// Follows the live play; once it's gone without going to the leaderboards (failed, quit,
    /// restarted, from a start point, a lobby match), tells the server it was played, for the
    /// profile's play count and play time.
    fn track_play(&mut self) {
        if let Some(ms) = self.live_play_ms() {
            let (before, map) = match self.playing.take() {
                Some(playing) => playing,
                None => (0.0, self.live_map()),
            };
            self.session_play_ms += (ms - before).max(0.0);
            self.playing = Some((ms, map));
            return;
        }
        let Some((ms, map)) = self.playing.take() else {
            return;
        };
        // A start that's left at once isn't a play.
        if ms < 1000.0 || self.cfg.online_token.is_empty() {
            return;
        }
        let (server, token) = (
            self.cfg.online_server.clone(),
            self.cfg.online_token.clone(),
        );
        std::thread::spawn(move || {
            if let Err(e) = online::call(
                &server,
                online::Request::Played {
                    token,
                    ms: ms as u64,
                    map,
                },
            ) {
                eprintln!("counting the play: {e}");
            }
        });
    }

    /// Asks the online server which map sets (and difficulties) are loved there, or loves one
    /// or takes it back, in the background.
    fn call_loved(&mut self, ask: LovedAsk) {
        let request = match &ask {
            LovedAsk::Change(set, loved, version) => online::Request::SetLoved {
                token: self.cfg.online_token.clone(),
                set: *set,
                loved: *loved,
                version: version.clone(),
            },
            LovedAsk::Maps => online::Request::LovedMaps,
            LovedAsk::Sets => online::Request::LovedSets,
        };
        let (tx, rx) = mpsc::channel();
        let server = self.cfg.online_server.clone();
        std::thread::spawn(move || {
            let _ = tx.send(online::call(&server, request));
        });
        self.loved_call = Some((rx, ask));
    }

    /// Which map folders of the library were sent to (or came from) the custom maps, and which
    /// of those sets are approved: asked on a thread once per library, then again every
    /// `CUSTOM_REFRESH` while the song list is open, so a set approved meanwhile shows it.
    fn poll_custom(&mut self, now: Instant) {
        let due = self.custom_asked.is_none_or(|t| {
            self.scene.is_select() && now.saturating_duration_since(t) >= CUSTOM_REFRESH
        });
        if due
            && self.custom_call.is_idle()
            && !self.library.is_empty()
            && self.cfg.online_server == online::DEFAULT_SERVER
        {
            self.custom_asked = Some(now);
            let mut folders: Vec<PathBuf> = self
                .library
                .iter()
                .filter_map(|e| e.path.parent().map(Path::to_path_buf))
                .collect();
            folders.sort();
            folders.dedup();
            self.custom_call = Job::spawn(move || {
                let known: std::collections::HashMap<PathBuf, u32> = folders
                    .into_iter()
                    .filter_map(|d| custom::sidecar_id(&d).map(|id| (d, id)))
                    .collect();
                if known.is_empty() {
                    return (known, Vec::new());
                }
                let mut ids: Vec<u32> = known.values().copied().collect();
                ids.sort_unstable();
                ids.dedup();
                // A failed answer keeps them listed as custom, not approved.
                let approved = custom::approved_of(&ids).unwrap_or_default();
                (known, approved)
            });
        }
        match self.custom_call.poll() {
            None => {}
            Some((folders, approved)) => {
                self.select.custom_folder = folders;
                self.select.custom_approved = approved.into_iter().collect();
                if !self.select.filter.is_empty() || self.select.sort == ui::SortMode::Status {
                    self.select.refilter(&self.library);
                }
            }
        }
    }

    /// The loved sets as the server has them, once it answers.
    fn poll_loved(&mut self) {
        if !self.loved_asked && self.loved_call.is_none() {
            self.loved_asked = true;
            self.call_loved(LovedAsk::Maps);
        }
        let Some((rx, _)) = &self.loved_call else {
            return;
        };
        let Ok(result) = rx.try_recv() else { return };
        let Some((_, ask)) = self.loved_call.take() else {
            return;
        };
        let loved: Option<ui::ServerLoved> = match (result, &ask) {
            (Ok(online::Reply::LovedMaps(maps)), _) => Some(maps.into_iter().collect()),
            // Just the sets: the whole of each (the server doesn't love single difficulties yet).
            (Ok(online::Reply::LovedSets(sets)), LovedAsk::Maps | LovedAsk::Sets) => {
                Some(sets.into_iter().map(|s| (s, Vec::new())).collect())
            }
            (Ok(online::Reply::LovedSets(_)), LovedAsk::Change(set, on, version)) => {
                let what = match version {
                    Some(v) => format!("[{v}] of set {set}"),
                    None => format!("set {set}"),
                };
                self.toast(if *on {
                    format!("loved {what} on taiko.rs: its plays give pp now")
                } else {
                    format!("{what} isn't loved on taiko.rs any more")
                });
                // Which difficulties, as it stands now.
                self.call_loved(LovedAsk::Maps);
                None
            }
            (Err(e), LovedAsk::Change(..)) => {
                self.toast(format!("couldn't: {e}"));
                None
            }
            // A server from before `LovedMaps`: the sets alone.
            (Err(_), LovedAsk::Maps) => {
                self.call_loved(LovedAsk::Sets);
                None
            }
            _ => None,
        };
        if let Some(loved) = loved {
            self.browse.set_loved_here(loved.keys().copied());
            self.select.server_loved = loved;
            if !self.select.filter.is_empty() {
                self.select.refilter(&self.library);
            }
        }
    }

    /// The online server's side of the multiplayer menu: the saved login, account replies and
    /// a lobby list kept fresh while the menu is open.
    fn poll_online(&mut self) {
        self.poll_loved();
        self.poll_custom(Instant::now());
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
                        Ok(online::Reply::Account {
                            name,
                            token,
                            nominator,
                            admin,
                            email,
                        }) => {
                            self.select.nominator = nominator;
                            self.admin = admin;
                            self.loved_asked = false;
                            let fresh = menu.account.as_deref() != Some(&name)
                                || self.cfg.online_token != token;
                            menu.account = Some(name.clone());
                            menu.online = Some(Ok(()));
                            menu.has_email = email;
                            menu.asking = None;
                            let reset =
                                menu.form == multiplayer::Form::Reset && menu.code_for.is_some();
                            if menu.form != multiplayer::Form::Main || menu.code_for.is_some() {
                                menu.open(multiplayer::Form::Main);
                            }
                            if reset {
                                self.mp_status = tr!("password changed: you're logged in as {name}", "パスワードを変更しました: {name} でログイン中", "пароль изменён, ты вошёл как {name}";);
                            } else if fresh {
                                self.mp_status = tr!("logged in as {name}", "{name} でログインしました", "ты вошёл как {name}";);
                            }
                            // Local plays take the account's name. Plays under a local name (the
                            // computer's user, at first) move to it; another account's stay theirs.
                            if self.cfg.mp_name != name {
                                let local = self.cfg.mp_name != self.cfg.online_user;
                                if local {
                                    match scores::rename_player(
                                        &mut self.scores,
                                        &self.cfg.mp_name,
                                        &name,
                                    ) {
                                        Ok(0) => {}
                                        Ok(n) => {
                                            self.toast = Some(Toast {
                                                text: tr!("{n} local plays now show as {name}", "ローカルの {n} プレイを {name} の名前にしました", "локальные игры ({n}) теперь под ником {name}";),
                                                until: Instant::now() + Duration::from_secs(5),
                                            })
                                        }
                                        Err(e) => eprintln!("couldn't rename the local plays: {e}"),
                                    }
                                }
                                self.cfg.mp_name = name.clone();
                            }
                            if fresh {
                                menu.cursor = 0;
                                // A new login: reconnect the social side with its token.
                                self.social = social::Social::default();
                                // Accounts from before emails: a word about linking one.
                                if email == Some(false) {
                                    self.toast = Some(Toast { text: tr!("no email on your account: link one in the Online menu to reset a forgotten password", "アカウントにメールがありません: オンラインメニューで登録するとパスワードを再設定できます", "к аккаунту не привязана почта: привяжи её в меню онлайна, чтобы можно было сбросить пароль").into(), until: Instant::now() + Duration::from_secs(8) });
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
                            self.mp_status = tr!("code sent to {to}: look in spam too if it isn't there", "{to} にコードを送信しました: 見つからなければ迷惑メールも確認", "код отправлен на {to}: если его нет, загляни в спам";);
                            menu.select(multiplayer::Row::Code);
                        }
                        Ok(online::Reply::EmailLinked(to)) => {
                            menu.has_email = Some(true);
                            menu.open(multiplayer::Form::Main);
                            self.mp_status = tr!("email linked: {to}", "メールを登録しました: {to}", "почта привязана: {to}";);
                        }
                        Ok(_) => {}
                        Err(e) if e.contains("session") => self.sign_out(&e),
                        Err(e) => {
                            if online::is_unreachable(&e) {
                                menu.online = Some(Err(e.clone()));
                            }
                            // A wrong password: where to go from there.
                            self.mp_status = if e.starts_with("wrong name or password")
                                && menu.account.is_none()
                            {
                                menu.select(multiplayer::Row::Password);
                                tr!("wrong name or password (forgot it? FORGOT PASSWORD below)", "名前かパスワードが違います(忘れたら下の「パスワードを忘れた?」)", "неверный ник или пароль (забыл? жми ЗАБЫЛ ПАРОЛЬ ниже)").into()
                            } else {
                                e
                            };
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
        if self.scene.screen_is::<MpMenu>()
            && self
                .mp_menu
                .last_refresh
                .is_none_or(|t| t.elapsed() > Duration::from_secs(4))
        {
            self.mp_menu.refresh(&self.cfg.online_server);
        }
    }

    /// Installs the library once the background scan finishes.
    fn poll_scan(&mut self) {
        let Some((rx, found)) = &self.scan else {
            return;
        };
        if !self.scan_checking {
            self.select.scanning = Some(found.load(Ordering::Relaxed));
        }
        let scanned = match rx.try_recv() {
            Ok(scanned) => scanned,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) if self.scan_checking => {
                self.scan = None;
                self.scan_checking = false;
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Scanned::Done(Vec::new()),
        };
        match scanned {
            Scanned::Cached(library) => {
                self.use_library(library, None);
                self.scan_checking = true;
            }
            Scanned::Done(library) if self.scan_checking => {
                self.scan = None;
                self.scan_checking = false;
                self.check_library(library);
            }
            Scanned::Done(library) => {
                self.scan = None;
                self.use_library(library, None);
            }
        }
    }

    /// The scan's list against the one up from the cache: what's new joins the end and what's
    /// changed takes its place, so the indices handed out stay good. What's gone takes the list
    /// being rebuilt, which waits for a next launch if anything could be holding on to one.
    fn check_library(&mut self, checked: Vec<Entry>) {
        let index: std::collections::HashMap<PathBuf, usize> = self
            .library
            .iter()
            .enumerate()
            .map(|(i, e)| (e.path.clone(), i))
            .collect();
        let gone = {
            let there: std::collections::HashSet<&Path> =
                checked.iter().map(|e| e.path.as_path()).collect();
            self.library
                .iter()
                .any(|e| !there.contains(e.path.as_path()))
        };
        if gone && self.scene.is_select() && self.mp.is_none() {
            let selected = self
                .select
                .selected()
                .and_then(|i| self.library.get(i))
                .map(|e| e.path.clone());
            self.use_library(checked, selected);
            return;
        }
        let mut fresh = Vec::new();
        let mut changed = false;
        for entry in checked {
            match index.get(&entry.path) {
                Some(&i) => {
                    let old = &self.library[i];
                    if old.size != entry.size || old.modified != entry.modified {
                        self.library[i] = entry;
                        changed = true;
                    }
                }
                None => fresh.push(entry),
            }
        }
        if !fresh.is_empty() {
            self.append_maps(fresh);
        } else if changed {
            self.select.refilter(&self.library);
        }
        self.find_lobby_map();
    }

    /// `library` in place of the list, on `selected` if that's still there, else a random song.
    fn use_library(&mut self, library: Vec<Entry>, selected: Option<PathBuf>) {
        self.stars.clear();
        eprintln!("found {} playable difficulties", library.len());
        self.library = library;
        // Keep whatever was typed into the search box while scanning, and the sort.
        let filter = std::mem::take(&mut self.select.filter);
        let played = std::mem::take(&mut self.select.played);
        let status = std::mem::take(&mut self.select.status);
        let server_loved = std::mem::take(&mut self.select.server_loved);
        let custom_folder = std::mem::take(&mut self.select.custom_folder);
        let custom_approved = std::mem::take(&mut self.select.custom_approved);
        self.custom_asked = None;
        let nominator = self.select.nominator;
        let sort = self.select.sort;
        self.select = Select::new(&self.library);
        self.select
            .keep_converts(&self.library, self.cfg.show_converts);
        self.select.filter = filter;
        self.select.played = played;
        self.select.status = status;
        self.select.server_loved = server_loved;
        self.select.custom_folder = custom_folder;
        self.select.custom_approved = custom_approved;
        self.select.nominator = nominator;
        self.select.sort = sort;
        self.select.headings = self.cfg.song_headings;
        self.select.mods_label = self.cfg.mods.label();
        let maps = self
            .library
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.path.clone(), e.modified, e.size))
            .collect();
        let info = mapinfo::MapInfo::spawn(maps, self.library.len(), self.in_song());
        self.select.info = Some(info.results.clone());
        self.mapinfo = Some(info);
        self.mapinfo_sorted = false;
        let dirs = self.song_dirs.clone();
        self.osudb_rx = Job::spawn(move || dirs.iter().find_map(|d| osudb::load(d)));
        // The ones kept from before, until osu!'s are read again.
        self.show_collections(None);
        self.apply_cached_statuses();
        self.select.refilter(&self.library);
        // Start on a random song rather than the first one every time (not before a map given on
        // the command line, nor in scripted runs, whose screenshots expect the first song).
        if let Some(path) = &selected {
            self.select.select_path(&self.library, path);
        } else if !self.play_when_scanned && self.input_script.is_none() && self.shots.is_none() {
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
        self.find_lobby_map();
    }

    /// A lobby map may have been announced before our library was ready.
    fn find_lobby_map(&mut self) {
        if let Some(s) = &mut self.mp
            && let Some(m) = &s.map
            && !s.have_map
        {
            s.map_entry = find_map(&self.library, m);
            s.have_map = s.map_entry.is_some();
            s.client.send(net::ClientMsg::HasMap(s.have_map));
        }
    }

    /// Folds osu!'s collections and play history in once read, and re-sorts when the
    /// difficulty analysis finishes.
    fn poll_sorting(&mut self) {
        if let Some(db) = self.osudb_rx.poll() {
            if let Some(db) = db {
                self.apply_osu_db(db);
            }
            // With osu!.db's statuses known, the mirrors are asked about the rest.
            self.start_status_sweep();
        }
        let Some(info) = &self.mapinfo else { return };
        let finished = info.finished();
        // Sorting or searching by stars, length or BPM waits on the analysis.
        let needs_info =
            self.select.sort.needs_info() || search::Query::parse(&self.select.filter).needs_info();
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
        for m in db.by_md5.values() {
            if let Some(&i) = index.get(&key(&m.folder, &m.file)) {
                self.db_status
                    .insert(self.library[i].path.clone(), m.status);
                // Unless the mirrors have said better already.
                self.select
                    .status
                    .entry(self.library[i].path.clone())
                    .or_insert(m.status);
            }
        }
        for m in db.by_md5.values().filter(|m| m.last_played > 0) {
            if let Some(&i) = index.get(&key(&m.folder, &m.file)) {
                let t = self
                    .select
                    .played
                    .entry(self.library[i].path.clone())
                    .or_insert(0);
                *t = (*t).max(m.last_played);
            }
        }
        eprintln!("osu!.db: {} maps", db.by_md5.len());
        self.show_collections(Some(&db));
        self.select.refilter(&self.library);
    }

    /// Remembers that `entry` was just played (for "recently played").
    fn record_play(&mut self, entry: usize) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = self.library[entry].path.clone();
        if let Some(file) = config::data_dir().map(|d| d.join("plays.tsv")) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)
            {
                let _ = writeln!(f, "{now}\t{}", path.display());
            }
        }
        store::with(|s| s.add_play(&path, now));
        self.select.played.insert(path, now);
    }

    /// F1 / the Mods button: the mod select, on the first mod that's on.
    fn open_mod_select(&mut self) {
        let first = self
            .cfg
            .mods
            .iter()
            .next()
            .and_then(|m| mods::ALL.iter().position(|&x| x == m))
            .unwrap_or(0);
        self.mod_select = Some(first);
    }

    /// The mods the mod select shows as on: in a lobby as a guest, the host's speed picked for
    /// everyone (which can't be switched here) on top of our own.
    fn shown_mods(&self) -> Mods {
        match &self.mp {
            // In Defense the host's mods are everyone's.
            Some(s) if self.in_lobby() && s.map.as_ref().is_some_and(|m| m.defense.is_some()) => {
                match s.map.as_ref().filter(|_| !s.is_host()) {
                    Some(m) => Mods::from_list(&m.mods),
                    None => self.cfg.mods.lobby(self.lobby_defense()),
                }
            }
            Some(s) if self.in_lobby() && !s.is_host() => {
                let lobby = s
                    .map
                    .as_ref()
                    .map(|m| Mods::from_list(&m.mods).shared())
                    .unwrap_or(Mods::NONE);
                lobby.plus(self.cfg.mods.free())
            }
            _ => self.cfg.mods,
        }
    }

    fn toggle_mod(&mut self, m: mods::Mod) {
        // In a lobby the host picks the speed for everyone.
        if m.shared() && self.in_lobby() && self.mp.as_ref().is_some_and(|s| !s.is_host()) {
            self.toast(format!(
                "the host picks the speed ({} is theirs to pick)",
                m.acronym()
            ));
            return;
        }
        // In Defense the host picks all of them, from the ones a team can have.
        if self.in_lobby()
            && let Some(s) = self.mp.as_ref().filter(|_| self.lobby_defense().is_some())
        {
            if !s.is_host() {
                self.toast(
                    tr!(
                        "in Defense the host picks the mods",
                        "ディフェンスではホストがMODを選ぶ",
                        "в Обороне моды выбирает хост"
                    )
                    .into(),
                );
                return;
            }
            if !m.defense_ok() {
                self.toast(format!(
                    "{} {}",
                    m.acronym(),
                    tr!(
                        "doesn't work in Defense",
                        "はディフェンスでは使えない",
                        "не работает в Обороне"
                    )
                ));
                return;
            }
        }
        self.cfg.mods.toggle(m);
        self.mods_changed();
    }

    /// Tells the room the mods we picked ourselves, if it doesn't have them.
    fn send_my_mods(&self) {
        let Some(s) = &self.mp else { return };
        // Our own others don't play in Defense: the host's are the whole team's.
        let mine = if self.lobby_defense().is_some() {
            Vec::new()
        } else {
            self.cfg.mods.free().list()
        };
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
        let rate = self.play_speed().0;
        if let Some(s) = &self.mp
            && s.is_host()
            && !s.in_match
            && let Some(mut m) = s.map.clone()
        {
            m.rate = rate;
            // Only the speed's: everyone picks their other mods themselves. In Defense, all of
            // the host's (a team plays one set of notes).
            let mods = self.cfg.mods.lobby(m.defense);
            m.mods = mods.list();
            (m.multiverse, m.shapeshift) = mod_settings(mods);
            m.seed = lobby_seed(Some(self.cfg.random_seed));
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
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter => {
                self.toggle_mod(mods::ALL[cur])
            }
            KeyCode::Backspace | KeyCode::Delete => {
                self.cfg.mods = Mods::NONE;
                self.mods_changed();
            }
            KeyCode::KeyR => {
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64);
                self.cfg.mods = Mods::shuffle(seed);
                self.mods_changed();
            }
            KeyCode::KeyF => self.toggle_favourite(mods::ALL[cur]),
            KeyCode::KeyS => self.start_preset_naming(),
            _ if preset_key(code).is_some() => self.apply_mod_preset(preset_key(code).unwrap_or(0)),
            KeyCode::KeyC | KeyCode::KeyM => match mods::customizable(self.cfg.mods).first() {
                Some(&m) => self.open_customization(m),
                None => self.toast(
                    "switch on Random, Multiverse, Shapeshift, Tact or OD Override to customize it"
                        .to_string(),
                ),
            },
            KeyCode::Escape | KeyCode::F1 | KeyCode::Tab => self.mod_select = None,
            _ => {}
        }
    }

    /// Starts naming a new preset of the mods on now and the speed (S / "+ Save").
    fn start_preset_naming(&mut self) {
        if self.cfg.mod_presets.len() >= mods::MAX_PRESETS {
            self.toast(format!(
                "{} presets at most: right-click one to remove it",
                mods::MAX_PRESETS
            ));
            return;
        }
        self.preset_naming = Some(mods::Preset::auto_name(self.cfg.mods, self.cfg.rate));
    }

    /// Keys while a new preset is being named: Enter keeps it, Esc doesn't.
    fn preset_naming_key(&mut self, code: KeyCode, text: Option<&str>) {
        let Some(name) = &mut self.preset_naming else {
            return;
        };
        match code {
            KeyCode::Escape | KeyCode::F1 => self.preset_naming = None,
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                self.preset_naming = None;
                if self.cfg.mod_presets.len() < mods::MAX_PRESETS {
                    self.cfg.mod_presets.push(mods::Preset {
                        name: name.clone(),
                        mods: self.cfg.mods,
                        rate: self.cfg.rate,
                    });
                    self.cfg.save();
                    self.toast(format!(
                        "preset {} saved: {name}",
                        self.cfg.mod_presets.len()
                    ));
                }
            }
            KeyCode::Backspace => {
                name.pop();
            }
            KeyCode::Delete => name.clear(),
            KeyCode::KeyA if self.modifiers_ctrl => name.clear(),
            _ => {
                if let Some(text) = text {
                    let room = 40usize.saturating_sub(name.chars().count());
                    name.extend(text.chars().filter(|c| !c.is_control()).take(room));
                }
            }
        }
    }

    /// Puts on preset `k`'s mods and speed. In a lobby a guest keeps the host's speed, and in
    /// Defense the host picks everything.
    fn apply_mod_preset(&mut self, k: usize) {
        let Some(p) = self.cfg.mod_presets.get(k).cloned() else {
            return;
        };
        let guest = self.in_lobby() && self.mp.as_ref().is_some_and(|s| !s.is_host());
        if guest && self.lobby_defense().is_some() {
            self.toast(
                tr!(
                    "in Defense the host picks the mods",
                    "ディフェンスではホストがMODを選ぶ",
                    "в Обороне моды выбирает хост"
                )
                .into(),
            );
            return;
        }
        if guest {
            self.cfg.mods = self.cfg.mods.shared().plus(p.mods.free());
        } else {
            self.cfg.mods = p.mods;
            self.cfg.rate = p.rate.clamp(RATE_RANGE.0, RATE_RANGE.1);
        }
        self.mods_changed();
        self.toast(format!("preset {}: {}", k + 1, p.name));
    }

    /// Keys while deleting a preset is being asked: Enter, Y or Delete deletes it, Esc or N
    /// keeps it.
    fn preset_deleting_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::KeyY | KeyCode::Delete => {
                if let Some(k) = self.preset_deleting.take() {
                    self.remove_mod_preset(k);
                }
            }
            KeyCode::Escape | KeyCode::F1 | KeyCode::KeyN | KeyCode::Backspace => {
                self.preset_deleting = None;
            }
            _ => {}
        }
    }

    /// Takes preset `k` away (once a right-click on it is confirmed).
    fn remove_mod_preset(&mut self, k: usize) {
        if k < self.cfg.mod_presets.len() {
            let p = self.cfg.mod_presets.remove(k);
            self.cfg.save();
            self.toast(format!("preset removed: {}", p.name));
        }
    }

    fn toggle_favourite(&mut self, m: mods::Mod) {
        self.cfg.favourite_mods = self.cfg.favourite_mods.flip(m);
        self.cfg.save();
    }

    /// Opens the mod customization on mod `m`'s tab.
    fn open_customization(&mut self, m: mods::Mod) {
        if m == mods::Mod::Random
            && self.in_lobby()
            && self.mp.as_ref().is_some_and(|s| !s.is_host())
        {
            self.toast("The host picks the Random seed for this lobby".into());
            return;
        }
        self.random_setup = (m == mods::Mod::Random).then(|| self.cfg.random_seed.to_string());
        self.universe_setup = (m == mods::Mod::Multiverse)
            .then(|| mod_setup::multiverse::Setup::new(game::FIRST_FOCUS, 0));
        self.shapeshift_setup =
            (m == mods::Mod::Shapeshift).then(mod_setup::shapeshift::Setup::default);
        self.tact_setup =
            (m == mods::Mod::Tact).then(|| mod_setup::tact::Setup::new(self.cfg.tact));
        self.od_setup = (m == mods::Mod::OdOverride).then_some(mod_setup::od_override::Setup);
    }

    /// The mod customization's open tab.
    fn customization_tab(&self) -> Option<mods::Mod> {
        if self.random_setup.is_some() {
            Some(mods::Mod::Random)
        } else if self.universe_setup.is_some() {
            Some(mods::Mod::Multiverse)
        } else if self.shapeshift_setup.is_some() {
            Some(mods::Mod::Shapeshift)
        } else if self.tact_setup.is_some() {
            Some(mods::Mod::Tact)
        } else {
            self.od_setup.is_some().then_some(mods::Mod::OdOverride)
        }
    }

    /// Keys in the mod customization: Tab goes to the next tab (closes it with only one), the
    /// rest go to the open one.
    fn customization_key(&mut self, code: KeyCode, text: Option<&str>, repeat: bool) {
        let typing = self.random_setup.is_some()
            || self
                .universe_setup
                .as_ref()
                .is_some_and(|s| s.typing.is_some());
        let tabs = mods::customizable(self.cfg.mods);
        if code == KeyCode::Tab && !typing && tabs.len() > 1 {
            let at = tabs
                .iter()
                .position(|&m| Some(m) == self.customization_tab())
                .unwrap_or(0);
            self.open_customization(tabs[(at + 1) % tabs.len()]);
            return;
        }
        let arrows = matches!(
            code,
            KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight
        );
        if repeat && !typing && !arrows {
            return;
        }
        if let Some(input) = &mut self.random_setup {
            match code {
                KeyCode::Escape | KeyCode::F1 | KeyCode::Tab => self.random_setup = None,
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    if let Ok(seed) = input.parse::<u64>() {
                        self.cfg.random_seed = seed;
                        self.cfg.save();
                        self.random_setup = None;
                        self.update_lobby_map();
                    } else {
                        self.toast("Enter a seed from 0 to 18446744073709551615".into());
                    }
                }
                KeyCode::KeyR => *input = fresh_seed().to_string(),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Delete => input.clear(),
                KeyCode::KeyA if self.modifiers_ctrl => input.clear(),
                _ => {
                    if let Some(text) = text {
                        input.extend(
                            text.chars()
                                .filter(char::is_ascii_digit)
                                .take(20usize.saturating_sub(input.len())),
                        );
                    }
                }
            }
        } else if let Some(setup) = &mut self.universe_setup {
            let action = setup.key(
                code,
                text,
                self.modifiers_shift,
                &mut self.cfg.universes,
                &mut self.cfg.universe_presets,
                game::FIRST_FOCUS,
            );
            self.universe_action(action);
        } else if let Some(setup) = &mut self.shapeshift_setup {
            if matches!(
                code,
                KeyCode::Escape | KeyCode::F1 | KeyCode::Backspace | KeyCode::Tab
            ) {
                self.shapeshift_setup = None;
            } else if setup.key(code, self.modifiers_shift, &mut self.cfg.shapeshift) {
                self.shapeshift_changed();
            }
        } else if let Some(setup) = &mut self.tact_setup {
            if matches!(
                code,
                KeyCode::Escape | KeyCode::F1 | KeyCode::Backspace | KeyCode::Tab
            ) {
                self.tact_setup = None;
            } else if let Some(step) = setup.key(code) {
                self.tact_changed(step);
            }
        } else if let Some(setup) = &self.od_setup {
            if matches!(
                code,
                KeyCode::Escape | KeyCode::F1 | KeyCode::Backspace | KeyCode::Tab
            ) {
                self.od_setup = None;
            } else if let Some(od) = setup.key(code, self.modifiers_shift, self.cfg.od_override) {
                self.cfg.od_override = od;
                od_override::set(od);
                self.cfg.save();
            }
        }
    }

    fn tact_changed(&mut self, step: usize) {
        self.cfg.tact = step.min(tact::STEPS.len() - 1);
        tact::set(self.cfg.tact);
        self.cfg.save();
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
        self.social_keys()
            || self.scene.screen().is_some_and(|s| s.typing())
            || self.settings_ui.editing.is_some()
            || self.player.as_ref().is_some_and(|p| p.searching)
            || self.random_setup.is_some()
            || self.preset_naming.is_some()
            || self.select.naming.is_some()
            || self.scene.screen().is_some_and(|s| s.typing())
            || self
                .universe_setup
                .as_ref()
                .is_some_and(|u| u.typing.is_some())
            || (self.mod_select.is_none()
                && !self.on_menu()
                && (self.scene.is_select()
                    || self.scene.screen().is_some_and(|s| s.tab().is_some())))
            || (self.in_settings()
                && self.settings_ui.binding.is_none()
                && self.settings_ui.choices.is_none()
                && self.settings_ui.dropdown.is_none())
    }

    /// Android: something is being typed that took the keys by itself (not a field that was
    /// tapped): the on-screen keyboard comes up for it.
    #[cfg(target_os = "android")]
    fn typing_on_its_own(&self) -> bool {
        (self.settings_ui.editing.is_some() && self.in_settings())
            || self
                .settings_ui
                .dropdown
                .as_ref()
                .is_some_and(|d| d.editing.is_some())
            || self.random_setup.is_some()
            || self.preset_naming.is_some()
            || self.select.naming.is_some()
            || self.scene.screen().is_some_and(|s| s.typing())
            || self
                .universe_setup
                .as_ref()
                .is_some_and(|u| u.typing.is_some())
    }

    /// Android: whether tapping `hit` is tapping a text field.
    #[cfg(target_os = "android")]
    fn is_text_field(&self, hit: hit::Hit) -> bool {
        use hit::Hit;
        match hit {
            Hit::TextField | Hit::SettingsSearch | Hit::LobbyChat => true,
            Hit::MpRow(i) => {
                self.scene.screen_is::<MpMenu>()
                    && self.mp_menu.rows().get(i).is_some_and(|r| !r.is_action())
            }
            Hit::SocialRow(i) => matches!(
                self.social.rows(self.mp.is_some()).get(i),
                Some(social::Row::AddFriend | social::Row::Search)
            ),
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
        let field = self.keyboard.asked
            && self.takes_text()
            && !(self.scene.screen_is::<MpMenu>() && self.mp_menu.row().is_action());
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

    /// Android: tells the system's media controls what's playing, when that changes or the song
    /// isn't where they'd think it is by now.
    #[cfg(target_os = "android")]
    fn tell_media_controls(&mut self, now: Instant) {
        let Some(e) = self.preview_target().and_then(|i| self.library.get(i)) else {
            return;
        };
        // Only the song's own music says where it is: just picked, the last one's still going.
        if !self.preview.plays(e) {
            return;
        }
        let paused = self.music_paused;
        let speed = self.preview_rate().0;
        let sent = self.media_sent.as_ref();
        // Paused, the music has no clock: it's where it was last told.
        let (at, length) = match self.preview.music_times(self.audio.as_ref(), now) {
            Some(times) => times,
            None => match sent {
                Some(((path, _, length), at, _)) if *path == e.path => (*at, *length as f64),
                _ => return,
            },
        };
        let key = (e.path.clone(), paused, length as i64);
        // Where the controls think it is by now, their bar running at the music's speed.
        let expected = sent.map_or(at, |(_, sent_at, when)| {
            let gone = now.saturating_duration_since(*when).as_secs_f64() * 1000.0;
            sent_at + if paused { 0.0 } else { gone * speed }
        });
        let same = sent.is_some_and(|sent| sent.0 == key);
        if same && (at - expected).abs() < 700.0 {
            return;
        }
        let art = e
            .meta
            .background
            .as_ref()
            .and_then(|b| Some(e.path.parent()?.join(b)))
            .filter(|p| p.is_file());
        android::now_playing(&android::NowPlaying {
            title: &e.meta.title,
            artist: &e.meta.artist,
            detail: &e.meta.version,
            art: art.as_deref().and_then(|p| p.to_str()).unwrap_or_default(),
            length_ms: length as i64,
            at_ms: at as i64,
            speed: speed as f32,
            paused,
        });
        self.media_sent = Some((key, at, now));
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
        if let Some((_, text)) = self
            .settings_ui
            .dropdown
            .as_ref()
            .and_then(|d| d.editing.as_ref())
        {
            return Some(("Size".into(), text.clone()));
        }
        if let Some(e) = self
            .settings_ui
            .editing
            .as_ref()
            .filter(|_| self.in_settings())
        {
            return Some((e.item.label().into(), e.text.clone()));
        }
        if let Some(seed) = &self.random_setup {
            return Some(("Random seed".into(), seed.clone()));
        }
        if let Some(t) = self.universe_setup.as_ref().and_then(|u| u.typing.as_ref()) {
            return Some(match t {
                multiverse::Typing::Colours(s) => ("Colours".into(), s.clone()),
                multiverse::Typing::Name(s) => ("Preset name".into(), s.clone()),
            });
        }
        if self.social_keys() {
            let label = match self.social.row(self.mp.is_some()) {
                social::Row::AddFriend => "Add friend",
                social::Row::Search => "Search players",
                _ => "Message",
            };
            return Some((label.into(), self.social.input.clone()));
        }
        if self.scene.screen_is::<MpMenu>() {
            use multiplayer::Row;
            return match self.mp_menu.row() {
                r @ Row::User => Some((
                    self.mp_menu.field_label(r).unwrap_or_default().into(),
                    self.mp_menu.user.clone(),
                )),
                r @ Row::Password => Some((
                    self.mp_menu.field_label(r).unwrap_or_default().into(),
                    "*".repeat(self.mp_menu.password.chars().count()),
                )),
                r @ Row::Email => Some((
                    self.mp_menu.field_label(r).unwrap_or_default().into(),
                    self.mp_menu.email.clone(),
                )),
                r @ Row::Code => Some((
                    self.mp_menu.field_label(r).unwrap_or_default().into(),
                    self.mp_menu.code.clone(),
                )),
                Row::NewPassword => Some((
                    "Lobby password".into(),
                    "*".repeat(self.mp_menu.new_password.chars().count()),
                )),
                Row::JoinPassword => Some((
                    "Lobby password".into(),
                    "*".repeat(self.mp_menu.join_password.chars().count()),
                )),
                _ => None,
            };
        }
        if let Some(typed) = self
            .mp
            .as_ref()
            .and_then(|s| s.chat_input.as_ref())
            .filter(|_| self.in_lobby())
        {
            return Some(("Chat".into(), typed.clone()));
        }
        // The host typing a new lobby password (K).
        if let Some(typed) = self
            .mp
            .as_ref()
            .and_then(|s| s.edit_password.as_ref())
            .filter(|_| self.in_lobby())
        {
            return Some(("New lobby password".into(), typed.clone()));
        }
        None
    }

    /// Android: the text being typed in a strip along the top, clear of the on-screen keyboard.
    #[cfg(target_os = "android")]
    fn draw_typing(&self, c: &mut Canvas) {
        let Some((label, text)) = self.typing_line() else {
            return;
        };
        c.rect(0.0, 0.0, c.width, 30.0, rgba(0x15151d, 1.0));
        c.rect(0.0, 30.0, c.width, 1.0, rgba(render::accent(), 1.0));
        let lw = c.text(16.0, 10.5, 9.0, rgba(render::accent(), 1.0), &label);
        let x = 16.0 + lw + 12.0;
        c.text(
            x,
            10.0,
            10.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit_end(10.0, c.width - x - 16.0, &format!("{text}_")),
        );
        c.hit(0.0, 0.0, c.width, 31.0, hit::Hit::TextField);
    }

    /// `/np`'s line: the map on this screen, with its set's osu! link when it has one.
    fn now_playing(&self) -> Option<String> {
        let e = self.library.get(self.offset_entry()?)?;
        let name = format!("{} - {} [{}]", e.meta.artist, e.meta.title, e.meta.version);
        let set = e.set_id();
        Some(match set {
            Some(set) => format!("is playing {name} https://osu.ppy.sh/beatmapsets/{set}"),
            None => format!("is playing {name}"),
        })
    }

    /// A map set named in the chat, clicked: picked in song select when we have it, else downloaded.
    fn open_chat_map(&mut self, set: u32) {
        self.main_menu = None;
        let have = self.library.iter().position(|e| e.set_id() == Some(set));
        match have {
            Some(i) if self.scene.is_select() => {
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
                custom: None,
            }),
        }
    }

    /// What Ctrl+C copies here: the text being typed, else the map or set picked.
    fn copy_text(&self) -> Option<String> {
        if let Some(seed) = &self.random_setup {
            return Some(seed.clone());
        }
        if let Some(r) = self.scene.screen_as::<ResultsScreen>()
            && r.results.mods.has(mods::Mod::Random)
        {
            return r.results.seed.map(|s| s.to_string());
        }
        if let Some(e) = &self.settings_ui.editing {
            return Some(e.text.clone()).filter(|t| !t.is_empty());
        }
        if self.social_keys() {
            return Some(self.social.input.clone()).filter(|t| !t.is_empty());
        }
        match &self.scene {
            Scene::Other(sel)
                if sel.select_screen()
                    && !self.select.filter.is_empty()
                    && self.select.selected().is_none() =>
            {
                Some(self.select.filter.clone())
            }
            Scene::Other(sel) if sel.select_screen() => self
                .select
                .selected()
                .and_then(|i| self.library.get(i))
                .map(Entry::label),
            Scene::Other(s) if s.previews_remote() => self.browse.selected().map(|s| {
                format!(
                    "{} - {}  https://osu.ppy.sh/beatmapsets/{}",
                    s.artist, s.title, s.id
                )
            }),
            _ => None,
        }
    }

    /// The status of library map `i`, as (label, colour): the custom maps' for a set sent to
    /// (or downloaded from) them, else osu!'s: the mirrors' when known, else osu!.db's (which
    /// can't tell graveyard from pending).
    fn map_status(&self, i: usize) -> Option<(&'static str, u32)> {
        let e = self.library.get(i)?;
        let edited = self.select.edited.contains(&e.path);
        // As the song list has it (`Select::status_of`): an edited copy is nobody's.
        let custom = e
            .path
            .parent()
            .and_then(|d| self.select.custom_folder.get(d));
        if let Some(id) = custom.filter(|_| !edited) {
            return Some(if self.select.custom_approved.contains(id) {
                ("CUSTOM APPROVED", 0x4fd1b0)
            } else {
                ("CUSTOM", 0x9b6cff)
            });
        }
        let Some(set) = e.set_id().filter(|_| !edited) else {
            return Some(("NOT SUBMITTED", 0x8a8aa0));
        };
        if ui::loved_here(&self.select.server_loved, e) {
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
            let code = e
                .set_id()
                .and_then(|set| self.status_cache.status(set))
                .and_then(search::status_code);
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
            let (Some(set), Some(map)) = (e.meta.set_id, e.meta.beatmap_id) else {
                continue;
            };
            let sure = matches!(self.db_status.get(&e.path), Some(4 | 5 | 7));
            if !sure && self.status_cache.due(set, now) && seen.insert(set) {
                maps.push((set, map));
            }
        }
        if maps.is_empty() {
            return;
        }
        let (server, token) = (
            self.cfg.online_server.clone(),
            self.cfg.online_token.clone(),
        );
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
                    match online::call(
                        &server,
                        online::Request::SetStatuses {
                            token: token.clone(),
                            maps: chunk.to_vec(),
                        },
                    ) {
                        Ok(online::Reply::SetStatuses(found)) => {
                            failed = 0;
                            let got: std::collections::HashSet<u32> =
                                found.iter().map(|(set, _)| *set).collect();
                            left.extend(
                                chunk.iter().filter(|(set, _)| !got.contains(set)).copied(),
                            );
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
                    let mut m: std::collections::HashMap<u32, Vec<usize>> =
                        std::collections::HashMap::new();
                    for (i, e) in self.library.iter().enumerate() {
                        if let Some(set) = e.set_id() {
                            m.entry(set).or_default().push(i);
                        }
                    }
                    m
                });
                for &i in by_set.get(&set).into_iter().flatten() {
                    self.select
                        .status
                        .insert(self.library[i].path.clone(), code);
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
        let by_status =
            self.select.sort == ui::SortMode::Status || self.select.filter.contains("status");
        if self.status_new
            && by_status
            && now.saturating_duration_since(self.status_resorted) >= Duration::from_secs(10)
        {
            self.status_new = false;
            self.status_resorted = now;
            self.select.refilter(&self.library);
        }
        let set = self
            .select
            .selected()
            .and_then(|i| self.library.get(i))
            .and_then(Entry::set_id);
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
        self.scene.is_play() || self.scene.screen().is_some_and(|s| s.dims_background())
    }

    /// What the window is cleared to under the background: black in a song when the background
    /// dims to black, else the interface's grey.
    fn backdrop(&self, in_song: bool) -> [f64; 3] {
        if in_song && self.cfg.bg_dim_black {
            [0.0; 3]
        } else {
            render::clear_colour()
        }
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

    fn open_sorts(&mut self) {
        self.select.list = ui::PickList::Sorts;
        self.select.picker = ui::SortMode::ALL
            .iter()
            .position(|&m| m == self.select.sort);
    }

    /// Row `i` of the collection or sort list.
    fn pick_row(&mut self, i: usize) {
        if self.select.list != ui::PickList::Sorts {
            self.pick_collection_row(i);
        } else if let Some(&sort) = ui::SortMode::ALL.get(i) {
            self.select.set_sort(sort, &self.library);
            self.sort_changed();
        }
    }

    /// Someone else's play is being watched (a friend's, or a match spectated).
    fn watching(&self) -> bool {
        self.scene.screen().is_some_and(|s| s.watching())
    }

    /// The friends overlay (and notifications) can show: outside songs, and while watching
    /// someone else play (spectating a match, watching a friend), never over your own play.
    fn overlay_allowed(&self) -> bool {
        !self.in_song() || self.watching()
    }

    /// Where the friends tab goes on this screen, if it shows: not over a play, or over anything
    /// that takes up the whole screen (F9 still opens the panel there).
    fn social_tab_middle(&self) -> Option<f32> {
        let covered = self.in_song()
            || self.in_editor()
            || self.player.is_some()
            || self.mod_select.is_some()
            || self.random_setup.is_some()
            || self.universe_setup.is_some()
            || self.select.menu.is_some()
            || self.in_settings()
            || self.scene.screen().is_some_and(|s| s.covers_social_tab());
        let middle = if self.on_menu() {
            social::TAB_MENU_MIDDLE
        } else {
            social::TAB_MIDDLE
        };
        (!covered).then_some(middle)
    }

    /// Song select, or a screen that is part of it (its fullscreen preview, the download tab):
    /// the song list's pointer, the preview panel and the map's stars work there.
    fn in_song_select(&self) -> bool {
        self.scene.is_select() || self.scene.screen().is_some_and(|s| s.song_select())
    }

    /// The map editor or its start screen: the friends chip replaces the edge tab there, and an
    /// open panel leaves the keys to the editor until one of its fields is clicked.
    fn in_editor(&self) -> bool {
        self.scene.screen().is_some_and(|s| s.editor())
    }

    /// Where the friends overlay's chip goes in the editors' top bar (its left edge), on a
    /// screen `width` wide; None outside the editors.
    fn editor_chip_x(&self, width: f32) -> Option<f32> {
        match &self.scene {
            Scene::Other(_) if self.scene.screen_is::<MapEditor>() => {
                Some(editor::view::View::menu_end() + 6.0)
            }
            Scene::Other(s) => s.chip_from_right().map(|right| width - right),
        }
    }

    /// The open friends panel has the keyboard.
    fn social_keys(&self) -> bool {
        self.social.open && (self.social.typing || !self.in_editor())
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
        let talking = self.social.in_conversation(in_lobby);
        match code {
            KeyCode::Escape if talking => self.social.close_conversation(),
            KeyCode::Escape
                if self.social.tab == social::Tab::Admin && self.social.info.is_some() =>
            {
                self.social.info = None
            }
            KeyCode::Escape => self.social.close(),
            KeyCode::Tab => self.social.cycle_tab(self.modifiers_shift),
            KeyCode::PageUp => self.social.wheel(5, in_lobby),
            KeyCode::PageDown => self.social.wheel(-5, in_lobby),
            KeyCode::ArrowUp if talking => self.social.wheel(1, in_lobby),
            KeyCode::ArrowDown if talking => self.social.wheel(-1, in_lobby),
            KeyCode::ArrowUp => {
                self.social.cursor = self.social.cursor.min(rows - 1).saturating_sub(1)
            }
            KeyCode::ArrowDown => self.social.cursor = (self.social.cursor + 1).min(rows - 1),
            KeyCode::Backspace => {
                self.social.input.pop();
            }
            KeyCode::Delete if self.social.input.is_empty() && !talking => match row {
                Row::Request(_) => self.social_action(social::Action::Decline),
                Row::AdminRoom(_) => self.social_action(social::Action::CloseRoom),
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
            KeyCode::KeyK if self.modifiers_ctrl && self.admin => {
                self.social_action(social::Action::Kick)
            }
            KeyCode::KeyM if self.modifiers_ctrl && self.admin => {
                self.social_action(social::Action::Mute)
            }
            KeyCode::KeyL if self.modifiers_ctrl && self.admin => {
                self.social_action(social::Action::Lookup)
            }
            // An admin's /command, from any chat box (or the admin tab's).
            KeyCode::Enter | KeyCode::NumpadEnter
                if self.admin
                    && admin::is_command(&self.social.input)
                    && matches!(
                        row,
                        Row::AdminInput | Row::Friend(_) | Row::Lobby | Row::Global
                    ) =>
            {
                let text = std::mem::take(&mut self.social.input);
                self.admin_command(text.trim());
            }
            KeyCode::Enter | KeyCode::NumpadEnter if row == Row::AdminInput => {
                if !self.social.input.trim().is_empty() {
                    self.toast("admin commands start with / (/help lists them)".into());
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let mut text = self.social.input.trim().to_string();
                if text.eq_ignore_ascii_case("/shot")
                    && matches!(row, Row::Friend(_) | Row::Lobby | Row::Global)
                {
                    self.social.input.clear();
                    self.send_shot(row);
                    return;
                }
                // `/np` in a chat: the map on screen, with its osu! link (a link others can click).
                if text.eq_ignore_ascii_case("/np")
                    && matches!(row, Row::Friend(_) | Row::Lobby | Row::Global)
                {
                    match self.now_playing() {
                        Some(np) => text = np,
                        None => {
                            self.toast("no map to share: pick one first".into());
                            return;
                        }
                    }
                }
                match row {
                    Row::Friend(_) | Row::Lobby if !talking => self.social.open_conversation(),
                    Row::AddFriend if !text.is_empty() => {
                        if let Some(c) = &self.social.conn {
                            c.send(online::Up::AddFriend(text));
                        }
                        self.social.input.clear();
                    }
                    Row::Request(_) => self.social_action(social::Action::Accept),
                    Row::Friend(k) if !text.is_empty() => {
                        if let (Some(f), Some(c)) = (self.social.friends.get(k), &self.social.conn)
                        {
                            c.send(online::Up::Chat {
                                to: f.name.clone(),
                                text,
                            });
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
                        if let Some(c) = self.social.conn.as_ref().filter(|_| self.social.connected)
                        {
                            c.send(online::Up::GlobalChat(text));
                            self.social.input.clear();
                            self.social.chat_scroll = 0;
                        } else {
                            self.toast("not connected to the server".into());
                        }
                    }
                    Row::Player(k) => {
                        let name = self
                            .social
                            .players
                            .as_ref()
                            .and_then(|v| v.get(k))
                            .map(|p| p.name.clone())
                            .unwrap_or_default();
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
                // Typing on a friend starts talking to them.
                if matches!(row, Row::Friend(_) | Row::Lobby)
                    && !talking
                    && text.chars().any(|c| net::drawable_char(c) && c != ' ')
                {
                    self.social.open_conversation();
                }
                for ch in text.chars().filter(|c| net::drawable_char(*c)) {
                    let len = self.social.input.chars().count();
                    let ok = match row {
                        _ if searching => {
                            len < 16 && (ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
                        }
                        Row::AddFriend => {
                            len < 16 && (ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
                        }
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
            && self.social.in_conversation(in_lobby)
            && let Some(s) = &mut self.mp
        {
            s.chat_unread = 0;
        }
    }

    /// A button on the selected overlay row (or its keyboard shortcut).
    fn social_action(&mut self, action: social::Action) {
        use social::{Action, Row};
        let in_lobby = self.mp.is_some();
        match (self.social.row(in_lobby), action) {
            (Row::Request(k), Action::Accept | Action::Decline) => {
                let Some(name) = self.social.requests.get(k).cloned() else {
                    return;
                };
                if let Some(c) = &self.social.conn {
                    c.send(if action == Action::Accept {
                        online::Up::AddFriend(name)
                    } else {
                        online::Up::RemoveFriend(name)
                    });
                }
            }
            (Row::Player(k), Action::Add | Action::Accept | Action::Chat) => {
                let Some(name) = self
                    .social
                    .players
                    .as_ref()
                    .and_then(|v| v.get(k))
                    .map(|p| p.name.clone())
                else {
                    return;
                };
                if action == Action::Chat {
                    self.social.select_friend(&name, in_lobby);
                    self.social.mark_read(in_lobby);
                } else if let Some(c) = self.social.conn.as_ref().filter(|_| self.social.connected)
                {
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
            (Row::AdminPlayer(k), action) => {
                let Some(p) = self
                    .social
                    .overview
                    .as_ref()
                    .and_then(|o| o.players.get(k))
                    .cloned()
                else {
                    return;
                };
                let muted = p.muted_until.is_some_and(|t| t > discord::unix_now());
                match action {
                    Action::Watch => self.watch(p.name),
                    Action::Profile => self.open_profile(p.name),
                    Action::Kick => self.admin_op(online::AdminOp::Kick { name: p.name }),
                    Action::Mute => self.admin_op(online::AdminOp::Mute {
                        name: p.name,
                        minutes: if muted { 0 } else { 30 },
                    }),
                    Action::Lookup => self.admin_op(online::AdminOp::Lookup { name: p.name }),
                    _ => {}
                }
            }
            (Row::AdminRoom(k), action) => {
                let Some(r) = self
                    .social
                    .overview
                    .as_ref()
                    .and_then(|o| o.rooms.get(k))
                    .cloned()
                else {
                    return;
                };
                match action {
                    Action::Join => self.join_room(r.id, r.title),
                    Action::CloseRoom => self.admin_op(online::AdminOp::CloseRoom { room: r.id }),
                    _ => {}
                }
            }
            // Admins watch any solo play from the players tab too.
            (Row::Player(k), Action::Watch) if self.admin => {
                if let Some(p) = self.social.players.as_ref().and_then(|v| v.get(k)).cloned() {
                    self.watch(p.name);
                }
            }
            (Row::Friend(k), Action::Join) => {
                let Some(f) = self.social.friends.get(k).cloned() else {
                    return;
                };
                let Some(online::Activity::Lobby {
                    room: Some(room),
                    title,
                }) = f.activity
                else {
                    self.toast(format!("{} isn't in an online lobby", f.name));
                    return;
                };
                self.join_room(room, title);
            }
            _ => {}
        }
    }

    /// Joins online lobby `room` (from a friend's row, or an admin's list).
    fn join_room(&mut self, room: u32, title: String) {
        if self.mp.is_some() || self.mp_connecting.is_some() {
            self.toast("leave your lobby first".into());
            return;
        }
        self.social.close();
        self.mp_status = format!("joining {title}...");
        self.scene = MpMenu::scene();
        let request = online::Request::JoinRoom {
            token: self.cfg.online_token.clone(),
            room,
        };
        let account = self.mp_menu.account.clone().unwrap_or_default();
        self.mp_connecting = Some(multiplayer::start_online(
            self.cfg.online_server.clone(),
            request,
            account,
            None,
        ));
    }

    /// Sends an admin action (the server answers with a notice).
    fn admin_op(&mut self, op: online::AdminOp) {
        match self.social.conn.as_ref().filter(|_| self.social.connected) {
            Some(c) => {
                c.send(online::Up::Admin(op));
                // What it changed shows in the next overview, soon.
                self.social.overview_asked = None;
            }
            None => self.toast("not connected to the server".into()),
        }
    }

    /// An admin's typed /command.
    fn admin_command(&mut self, text: &str) {
        match admin::parse(text) {
            Ok(admin::Command::Op(op)) => self.admin_op(op),
            Ok(admin::Command::Watch(name)) => self.watch(name),
            Ok(admin::Command::Help) => {
                self.social.set_tab(social::Tab::Admin);
                self.social.info = Some((
                    "admin commands".into(),
                    admin::HELP
                        .split(" /")
                        .map(|c| {
                            if c.starts_with('/') {
                                c.to_string()
                            } else {
                                format!("/{c}")
                            }
                        })
                        .collect(),
                ));
            }
            Err(e) => self.toast(e),
        }
    }

    /// Sends the last F12's picture to the chat on `row`, on a thread.
    fn send_shot(&mut self, row: social::Row) {
        use social::Row;
        let to = match row {
            Row::Friend(k) => match self.social.friends.get(k) {
                Some(f) => ShotTo::Friend(f.name.clone()),
                None => return,
            },
            Row::Lobby => ShotTo::Lobby,
            Row::Global => ShotTo::Global,
            _ => return,
        };
        if self.cfg.online_server != online::DEFAULT_SERVER || self.cfg.online_token.is_empty() {
            self.toast("log in to send screenshots".into());
            return;
        }
        if self.shot_upload.is_some() {
            self.toast("one's on its way already".into());
            return;
        }
        let Some(shot) = self.last_shot.clone() else {
            self.toast("take one with F12 first".into());
            return;
        };
        let token = self.cfg.online_token.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let sent = shots::shot_jpeg(shot.0, shot.1, shot.2.clone())
                .and_then(|jpeg| shots::upload_shot(&token, jpeg));
            let _ = tx.send(sent);
        });
        self.shot_upload = Some((rx, to));
        self.toast("sending the screenshot...".into());
    }

    /// A screenshot uploaded goes in its chat; one opened goes up on the screen.
    fn poll_shots(&mut self) {
        if let Some((rx, _)) = &self.shot_upload {
            match rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.shot_upload = None,
                Ok(result) => {
                    let to = self.shot_upload.take().map(|u| u.1);
                    match (result, to) {
                        (Ok(id), Some(to)) => self.post_shot(id, to),
                        (Err(e), _) => self.toast(format!("couldn't send it: {e}")),
                        _ => {}
                    }
                }
            }
        }
        let got = self.shot_view.as_mut().and_then(|v| v.rx.poll_ended());
        match got {
            Some(Some(Ok((w, h, rgba)))) => {
                self.preview
                    .show_image(render::BgImage::from_rgba(w, h, &rgba));
            }
            Some(Some(Err(e))) => {
                self.shot_view = None;
                self.toast(format!("couldn't open it: {e}"));
            }
            // The fetch ended without a picture: nothing to show.
            Some(None) => self.shot_view = None,
            None => {}
        }
    }

    fn post_shot(&mut self, id: u128, to: ShotTo) {
        let text = shots::shot_url(id);
        match to {
            ShotTo::Lobby => {
                if let Some(s) = &self.mp {
                    s.client.send(net::ClientMsg::Chat(text));
                }
            }
            ShotTo::Friend(to) => {
                if let Some(c) = &self.social.conn {
                    c.send(online::Up::Chat { to, text });
                }
            }
            ShotTo::Global => {
                if let Some(c) = self.social.conn.as_ref().filter(|_| self.social.connected) {
                    c.send(online::Up::GlobalChat(text));
                    self.social.chat_scroll = 0;
                }
            }
        }
    }

    /// Opens a chat screenshot over everything. Not over a play being watched: its background
    /// is in use.
    fn open_shot(&mut self, id: u128) {
        if self.in_song() {
            self.toast("open it once this play is over".into());
            return;
        }
        self.shot_view = Some(ShotView {
            rx: Job::spawn(move || shots::fetch_shot(id)),
        });
    }

    /// Back from a screenshot, the background as it was.
    fn close_shot(&mut self) {
        self.shot_view = None;
        self.preview.forget_background();
        if let Some(e) = self.preview_target().and_then(|i| self.library.get(i)) {
            self.preview.show_background_for(e);
        }
    }

    /// The screenshot open, alone on the screen: nothing else of the frame is left.
    fn draw_shot_view(&mut self, canvas: &mut Canvas) {
        let Some(view) = &self.shot_view else { return };
        let loading = view.rx.is_busy();
        let w = canvas.width;
        canvas.clear(w);
        self.frame_clip = None;
        self.bg_brightness = 1.0;
        if loading {
            canvas.rect(0.0, 0.0, w, render::VIRTUAL_HEIGHT, rgba(0x000000, 1.0));
            let text = tr!("loading...", "読み込み中...", "загрузка...");
            canvas.text_centered(w / 2.0, 234.0, 9.0, rgba(0xc8c8d8, 1.0), text);
        }
        let hint = tr!(
            "click or any key to close",
            "クリックかキーで閉じる",
            "клик или клавиша: закрыть"
        );
        let hw = Canvas::text_width(7.0, hint) + 12.0;
        canvas.round_rect(
            [w / 2.0 - hw / 2.0, 462.0],
            [hw, 13.0],
            3.0,
            rgba(0x000000, 0.6),
            rgba(0, 0.0),
            0.0,
        );
        canvas.text_centered(w / 2.0, 465.0, 7.0, rgba(0xc8c8d8, 1.0), hint);
    }

    fn open_profile(&mut self, name: String) {
        self.profile = Some(profile::View::open(
            self.cfg.online_server.clone(),
            name,
            self.cfg.mods.relax(),
        ));
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
        self.scene = Watch::scene(Box::new(WatchView::new(name)));
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
        let lobby_map = || {
            self.mp
                .as_ref()
                .and_then(|s| s.map.as_ref())
                .map(|m| m.label())
                .unwrap_or_default()
        };
        if let Some(activity) = self.scene.screen().and_then(|s| s.friends_activity()) {
            return activity;
        }
        match &self.scene {
            // The editor and its test plays (`editor_back` holds the editor while one runs)
            // count as editing: a test play is the mapper checking their own work, there's
            // nothing to watch, and flipping to "playing" would offer friends a Watch button.
            _ if self.editor_back.is_some() && (self.scene.is_play() || self.loading_song()) => {
                Activity::Editing {
                    map: self
                        .editor_back
                        .as_ref()
                        .map(|v| v.label())
                        .unwrap_or_default(),
                }
            }
            Scene::Other(s)
                if s.song_play().is_some() && self.game().is_some_and(|g| g.replay().is_none()) =>
            {
                Activity::Playing {
                    map: s
                        .song_play()
                        .and_then(|(entry, _)| self.library.get(entry))
                        .map(|e| e.label())
                        .unwrap_or_default(),
                }
            }
            Scene::Other(s) if s.in_match() || s.spectating() => {
                Activity::Match { map: lobby_map() }
            }
            _ => match &self.mp {
                Some(s) => Activity::Lobby {
                    room: s.online_room,
                    title: s
                        .label
                        .strip_prefix("online: ")
                        .unwrap_or(&s.label)
                        .to_string(),
                },
                None => Activity::Menus,
            },
        }
    }

    /// The social connection: (re)connects while logged in, applies what the server says and
    /// keeps our status current.
    fn poll_social(&mut self) {
        if self.social.conn.is_none()
            && self.mp_menu.account.is_some()
            && !self.cfg.online_token.is_empty()
        {
            self.social.conn = Some(online::Social::start(
                self.cfg.online_server.clone(),
                self.cfg.online_token.clone(),
            ));
        }
        let Some(conn) = &self.social.conn else {
            return;
        };
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
                    if let Some(w) = self.scene.screen_as::<Watch>() {
                        let player = w.view.player.clone();
                        if let Some(c) = &self.social.conn {
                            c.send(online::Up::Watch(player));
                        }
                    }
                }
                online::SocialEvent::Disconnected(_) => {
                    if self.social.connected {
                        self.social.notify(
                            "offline",
                            "lost the connection to the server",
                            0xFF4B4B,
                            social::NoteAction::None,
                        );
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
        self.social.admin = self.admin;
        if !self.admin && self.social.tab == social::Tab::Admin {
            self.social.set_tab(social::Tab::Friends);
        }
        // The admin tab keeps its overview fresh while it's open.
        if self.admin
            && self.social.open
            && self.social.tab == social::Tab::Admin
            && self.social.connected
            && self
                .social
                .overview_asked
                .is_none_or(|t| t.elapsed() > Duration::from_secs(3))
            && let Some(c) = &self.social.conn
        {
            c.send(online::Up::Admin(online::AdminOp::Overview));
            self.social.overview_asked = Some(Instant::now());
        }
        let hold = !self.overlay_allowed();
        self.social.expire_notes(hold);
        if self.social.open
            && self.social.row(self.mp.is_some()) == social::Row::Lobby
            && self.social.in_conversation(self.mp.is_some())
            && let Some(s) = &mut self.mp
        {
            s.chat_unread = 0;
        }
    }

    fn social_down(&mut self, d: online::Down) {
        use online::{Activity, Down};
        use social::{NoteAction, key};
        match d {
            Down::Friends {
                friends,
                requests,
                sent,
            } => {
                if self.social.have_list {
                    for f in &friends {
                        let old = self
                            .social
                            .friends
                            .iter()
                            .find(|o| key(&o.name) == key(&f.name));
                        // A new friend isn't "coming online"; only one who was offline is.
                        let was_online = old.is_none_or(|o| o.activity.is_some());
                        let was_playing = old
                            .is_some_and(|o| matches!(o.activity, Some(Activity::Playing { .. })));
                        match &f.activity {
                            Some(Activity::Playing { map })
                                if !was_playing && self.cfg.notify_playing =>
                            {
                                self.social.notify(
                                    format!("{} is playing", f.name),
                                    map.clone(),
                                    0x438EAD,
                                    NoteAction::Watch(f.name.clone()),
                                )
                            }
                            Some(a) if !was_online && self.cfg.notify_online => self.social.notify(
                                format!("{} is online", f.name),
                                a.describe(),
                                0x86E05A,
                                NoteAction::Chat(f.name.clone()),
                            ),
                            _ => {}
                        }
                    }
                    for r in &requests {
                        if !self.social.requests.contains(r) && self.cfg.notify_requests {
                            self.social.notify(
                                "friend request",
                                format!("{r} wants to be friends"),
                                0xFFD34D,
                                NoteAction::Requests,
                            );
                        }
                    }
                }
                self.social.have_list = true;
                self.social
                    .set_friends(friends, requests, sent, self.mp.is_some());
            }
            Down::Chat(mut line) => {
                if self.cfg.chat_filter {
                    line.text = net::censor(&line.text);
                }
                let me = self.social.me.clone().unwrap_or_default();
                let mine = key(&line.from) == key(&me);
                let other = if mine {
                    line.to.clone()
                } else {
                    line.from.clone()
                };
                // Lines from before we connected are history (sent again on connect): no alerts.
                let fresh = line.at + 10 >= discord::unix_now();
                let log = self.social.chats.entry(key(&other)).or_default();
                log.push(line.clone());
                if log.len() > 200 {
                    log.remove(0);
                }
                let viewing = self.social.open
                    && self.social.in_conversation(self.mp.is_some())
                    && matches!(self.social.row(self.mp.is_some()), social::Row::Friend(k) if self.social.friends.get(k).is_some_and(|f| key(&f.name) == key(&other)));
                // A chime for every fresh message from a friend, unless it would land in the middle
                // of a play.
                if !mine
                    && fresh
                    && self.cfg.message_sound
                    && !self.playing()
                    && let Some(a) = &self.audio
                {
                    a.sfx_at(audio::Sfx::Message, 1.0);
                }
                if !mine && fresh && !viewing {
                    self.social.add_unread(&other, self.mp.is_some());
                    if self.cfg.notify_messages {
                        self.social.notify(
                            line.from.clone(),
                            line.text.clone(),
                            0x4fb4ff,
                            NoteAction::Chat(line.from.clone()),
                        );
                    }
                }
            }
            Down::Notice(text) => self
                .social
                .notify("taiko.rs", text, 0x9a9ab0, NoteAction::None),
            Down::GlobalChatDeleted { from, at } => self.social.delete_global(&from, at),
            Down::Groups(list) => self.social.set_groups(list),
            Down::AdminOverview(o) => self.social.set_overview(*o, self.mp.is_some()),
            Down::AdminLookup(l) => {
                let now = discord::unix_now();
                let mut lines = vec![format!(
                    "{}  joined {} ago  {} plays  {}",
                    if l.online { "online" } else { "offline" },
                    admin::span(now.saturating_sub(l.created)),
                    l.plays,
                    if l.has_email {
                        "email linked"
                    } else {
                        "no email"
                    }
                )];
                let mut marks = Vec::new();
                if l.banned {
                    marks.push("BANNED".to_string());
                }
                if l.nominator {
                    marks.push("loved nominator".to_string());
                }
                if let Some(t) = l.muted_until.filter(|&t| t > now) {
                    marks.push(format!("muted for {}", admin::span(t - now)));
                }
                if !marks.is_empty() {
                    lines.push(marks.join("  "));
                }
                lines.push(if l.alts.is_empty() {
                    "no other accounts from the same places".into()
                } else {
                    format!("same places as: {}", l.alts.join(", "))
                });
                self.social.info = Some((l.name.clone(), lines));
            }
            Down::Players { count, players } => {
                self.social
                    .set_players(count as usize, players, self.mp.is_some());
            }
            Down::Medals(new) => self.medals_earned(new),
            Down::GlobalChat(mut line) => {
                if self.cfg.chat_filter {
                    line.text = net::censor(&line.text);
                }
                let mine = self
                    .social
                    .me
                    .as_deref()
                    .is_some_and(|me| key(me) == key(&line.from));
                let fresh = line.at + 10 >= discord::unix_now();
                let viewing = self.social.open && self.social.tab == social::Tab::Chat;
                if !mine && fresh && !viewing {
                    self.social.global_unread += 1;
                    // Only lines that call us by name pop up: the rest just count.
                    if self.cfg.notify_mentions && self.social.mentions_me(&line.text) {
                        self.social.notify(
                            format!("{} in chat", line.from),
                            line.shown(),
                            0x4fb4ff,
                            NoteAction::Global,
                        );
                    }
                }
                // Scrolled back, the view stays on the same lines.
                if self.social.chat_scroll > 0 && viewing {
                    self.social.chat_scroll += self.social.global_rows_added(&line);
                }
                self.social.global.push(line);
                if self.social.global.len() > 200 {
                    self.social.global.remove(0);
                }
            }
            Down::WatchStart {
                player,
                play,
                t,
                paused,
                inputs,
            } => {
                if let Some(v) = self.scene.screen_mut::<Watch>().map(|w| &mut w.view)
                    && key(&v.player) == key(&player)
                {
                    let start = WatchStart {
                        play,
                        t,
                        paused,
                        inputs,
                    };
                    if v.ending.is_some() && v.game.is_some() {
                        // Finish the last play first.
                        v.next = Some(start);
                    } else {
                        v.begin(start);
                        if let Some(a) = &self.audio {
                            a.stop();
                        }
                    }
                }
            }
            Down::WatchFrame { t, paused, inputs } => {
                if let Some(v) = self.scene.screen_mut::<Watch>().map(|w| &mut w.view) {
                    if let Some(n) = &mut v.next {
                        n.t = t;
                        n.paused = paused;
                        n.inputs.extend_from_slice(&inputs);
                    } else if v.ending.is_none() {
                        v.clock = (t, Instant::now(), paused);
                        match v.game.as_mut().and_then(|g| g.replay_mut().as_mut()) {
                            Some((list, _)) => list.extend_from_slice(&inputs),
                            None => v.inputs.extend_from_slice(&inputs),
                        }
                    }
                }
            }
            Down::WatchEnd { player } => {
                if let Some(v) = self.scene.screen_mut::<Watch>().map(|w| &mut w.view)
                    && key(&v.player) == key(&player)
                {
                    if v.game.is_some() && v.ending.is_none() {
                        // Watch the rest of it: we're a couple of seconds behind.
                        v.ending = Some((v.their_time(), Instant::now()));
                    } else if v.game.is_none() {
                        let name = v.player.clone();
                        **v = WatchView::new(name);
                        if let Some(a) = &self.audio {
                            a.stop();
                        }
                    }
                }
            }
            Down::WatchStopped(why) => {
                self.toast(why);
                if self.scene.screen_is::<Watch>() {
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
        let (Some((entry, _)), Some(game)) = (self.scene.song_play(), self.play.as_deref_mut())
        else {
            if self.stream.take().is_some()
                && connected
                && let Some(c) = &self.social.conn
            {
                c.send(online::Up::PlayEnd);
            }
            return;
        };
        let Some(conn) = self
            .social
            .conn
            .as_ref()
            // OD Override's value doesn't travel: a watcher would judge by their own.
            .filter(|_| {
                connected && game.replay().is_none() && !game.mods().has(mods::Mod::OdOverride)
            })
        else {
            self.stream = None;
            return;
        };
        let id = &*game as *const Play as usize;
        if self.stream.map(|s| s.0) != Some(id) {
            if self.stream.is_some() {
                conn.send(online::Up::PlayEnd);
            }
            game.set_log_inputs(true);
            let Some(e) = self.library.get(entry) else {
                return;
            };
            conn.send(online::Up::PlayStart(online::SoloPlay {
                map: net::MapInfo {
                    seed: (game.seed() != 0).then_some(game.seed()),
                    ..map_info(e, game.rate(), game.mods())
                },
                anti_mash: true,
            }));
            self.stream = Some((id, 0, Instant::now() - Duration::from_secs(1)));
        }
        let Some((_, sent, last)) = self.stream.as_mut() else {
            return;
        };
        if last.elapsed() >= Duration::from_millis(100) {
            *last = Instant::now();
            let inputs = game.inputs().get(*sent..).unwrap_or_default().to_vec();
            *sent = game.inputs().len();
            conn.send(online::Up::PlayFrame {
                t: game.time(),
                paused: game.paused().is_some() || game.resuming().is_some(),
                inputs,
            });
        }
    }

    /// The watch view: finds (or downloads) and loads the map, starts the replay where the
    /// friend is (minus the delay), holds while they pause and catches up when they skip.
    fn watch_tick(&mut self) {
        // Out of the scene while it runs (the scene is song select meanwhile), as when it was
        // a variant.
        let Some(watch) = self.scene.take_screen::<Watch>() else {
            return;
        };
        let mut v = watch.view;
        if let (Some(play), None, None) = (v.play.clone(), v.entry, &v.game)
            && v.looked
                .is_none_or(|t| t.elapsed() > Duration::from_secs(1))
        {
            v.looked = Some(Instant::now());
            match find_map(&self.library, &play.map) {
                Some(i) => {
                    v.entry = Some(i);
                    self.preview.show_background_for(&self.library[i]);
                    v.rx = self.load_with(i, play.map.rate, Mods::from_list(&play.map.mods));
                }
                None if !v.fetching => {
                    v.fetching = true;
                    self.fetch_map(play.map.clone());
                }
                None => {}
            }
        }
        if v.rx.is_busy() {
            match v.rx.poll() {
                Some(Ok((map, track))) => {
                    let rate = v.play.as_ref().map(|p| p.map.rate).unwrap_or(1.0);
                    let mods = v
                        .play
                        .as_ref()
                        .map(|p| Mods::from_list(&p.map.mods))
                        .unwrap_or_default();
                    let seed = v.play.as_ref().and_then(|p| p.map.seed).unwrap_or(0);
                    let mut g = Play::new(
                        Game::with_seed(map, track, sound(&self.audio), mods, seed)
                            .with_settings(&crate::game::settings()),
                    );
                    let live_pp = v.entry.and_then(|i| {
                        pp::Live::start(
                            self.library[i].path.clone(),
                            stars::read,
                            g.rate(),
                            mods,
                            1.0,
                            seed,
                            Default::default(),
                        )
                    });
                    g.set_live_pp(live_pp);
                    if let Some(p) = &v.play {
                        use_mod_settings(&mut g, &p.map);
                    }
                    g.set_replay(Some((std::mem::take(&mut v.inputs), 0)));
                    g.view.watched = true;
                    g.view.banner = Some(format!("WATCHING  -  {}", v.player));
                    let target = v.their_time() - WATCH_DELAY_MS * rate;
                    if target > g.time() {
                        g.seek(target, sound(&self.audio), self.play_offset);
                    }
                    v.game = Some(Box::new(g));
                }
                Some(Err(e)) => self.toast(format!("can't load the map: {e}")),
                None => {}
            }
        }
        let rate = v.play.as_ref().map(|p| p.map.rate).unwrap_or(1.0);
        // Once they've stopped, play on to where they stopped.
        let target = match v.ending {
            Some((end, _)) => end,
            None => v.their_time() - WATCH_DELAY_MS * rate,
        };
        let mut over = false;
        if let Some(g) = &mut v.game {
            if let Some((end, heard)) = v.ending {
                // Their last note is on screen; or the view stalled / the map ended.
                over = g.time() >= end - 50.0
                    || heard.elapsed()
                        > Duration::from_millis((WATCH_DELAY_MS * 2.0 + 4000.0) as u64);
                if v.held {
                    g.resume(sound(&self.audio));
                    v.held = false;
                }
            } else if !v.held && g.time() > target + 250.0 {
                g.pause(sound(&self.audio));
                v.held = true;
            } else if v.held && target >= g.time() {
                g.resume(sound(&self.audio));
                v.held = false;
            } else if !v.held && g.time() < target - 4000.0 * rate {
                // Far behind (they skipped the intro, or we stalled): jump to them.
                g.seek(target, sound(&self.audio), self.play_offset);
            }
        }
        if over {
            match v.next.take() {
                Some(next) => v.begin(next),
                None => {
                    let name = v.player.clone();
                    *v = WatchView::new(name);
                }
            }
            if let Some(a) = &self.audio {
                a.stop();
            }
        }
        self.scene = Watch::scene(v);
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
                    if self.scene.screen_is::<MpMenu>() {
                        self.scene = Lobby::scene();
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
            ServerMsg::Room {
                players,
                map,
                in_match,
                mapguessr,
                common,
                team,
                locked,
                slots,
            } => {
                let library = &self.library;
                let Some(s) = &mut self.mp else { return };
                s.team = team;
                s.locked = locked;
                s.slots = slots;
                let game_over = s.in_match && !in_match;
                s.players = players;
                // The room learns our own mods (on joining, and if it has others).
                let mine = if map.as_ref().is_some_and(|m| m.defense.is_some()) {
                    Vec::new()
                } else {
                    self.cfg.mods.free().list()
                };
                if !mapguessr && s.me().is_some_and(|p| p.mods != mine) {
                    s.client.send(net::ClientMsg::Mods(mine));
                }
                s.in_match = in_match;
                s.mapguessr = mapguessr;
                s.common = common;
                if mapguessr {
                    let g = self
                        .guessr
                        .get_or_insert_with(|| mapguessr::Guessr::new(library));
                    if game_over {
                        g.game_over();
                        if self.picking_lobby_map && self.scene.is_select() {
                            self.picking_lobby_map = false;
                            self.scene = Lobby::scene();
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
                if dropped
                    && self
                        .scene
                        .screen_mut::<MpWait>()
                        .is_some_and(|w| w.start.is_none())
                {
                    self.scene = Lobby::scene();
                    self.toast("the match started without you (map didn't load in time)".into());
                }
            }
            ServerMsg::Prepare(map) => {
                let Some(s) = &mut self.mp else { return };
                s.contrib.clear();
                // Not in a position to play (single-player song, calibration): sit this one out.
                if self.scene.is_play()
                    || self.scene.screen().is_some_and(|s| s.loads_song())
                    || self
                        .scene
                        .screen()
                        .is_some_and(|s| s.calibration().is_some())
                {
                    s.client.send(net::ClientMsg::Loaded(false));
                    return;
                }
                match find_map(&self.library, &map) {
                    Some(i) => {
                        self.stop_audio();
                        self.picking_lobby_map = false;
                        self.preview.show_background_for(&self.library[i]);
                        // The host's speed mods, and our own others; in Defense the host's all.
                        let mods = if map.defense.is_some() {
                            Mods::from_list(&map.mods)
                        } else {
                            Mods::from_list(&map.mods)
                                .shared()
                                .plus(self.cfg.mods.free())
                        };
                        let rx = self.load_with(i, map.rate, mods);
                        self.scene = MpWait::scene(rx, map.from);
                    }
                    None => s.client.send(net::ClientMsg::Loaded(false)),
                }
            }
            ServerMsg::Start { at } => {
                let Some(s) = &self.mp else { return };
                let when = s.client.local_instant(at);
                if let Some(w) = self.scene.screen_mut::<MpWait>() {
                    w.start = Some(when);
                }
            }
            ServerMsg::MapRequested { from } => self.send_map_to(from),
            ServerMsg::Spectate { map, at, players } => {
                let Some(s) = &mut self.mp else { return };
                s.contrib.clear();
                match find_map(&self.library, &map) {
                    Some(i) => {
                        let start = s.client.local_instant(at);
                        self.stop_audio();
                        self.preview.show_background_for(&self.library[i]);
                        let mods = Mods::from_list(&map.mods);
                        let rx = self.load_with(i, map.rate, mods);
                        self.scene = Spectate::scene(Box::new(SpectateView {
                            rx,
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
                            ended: None,
                        }));
                    }
                    None => {
                        s.client.send(net::ClientMsg::Spectate(false));
                        self.toast("you need the map to spectate".into());
                    }
                }
            }
            ServerMsg::Skipped { from } => {
                if let Some(Spectate { view: v }) = self.scene.screen_mut::<Spectate>() {
                    v.skipped.insert(from);
                    v.follow_skip(self.audio.as_ref(), self.play_offset);
                }
            }
            ServerMsg::Inputs { from, inputs, .. } => {
                if let Some(Spectate { view: v }) = self.scene.screen_mut::<Spectate>() {
                    match v.games.iter_mut().find(|(id, _)| *id == from) {
                        Some((_, g)) => {
                            if let Some((list, _)) = g.replay_mut() {
                                list.extend_from_slice(&inputs);
                            }
                        }
                        None => {
                            v.pending
                                .entry(from)
                                .or_default()
                                .extend_from_slice(&inputs);
                        }
                    }
                }
            }
            ServerMsg::Defense { from, act } => {
                if let Some(s) = &mut self.mp {
                    let c = s.contrib.entry(from).or_default();
                    match act {
                        net::DefenseAct::Buy { .. } => c.0 += 1,
                        net::DefenseAct::TagIn { .. } => c.1 += 1,
                    }
                }
                if self
                    .scene
                    .screen()
                    .is_some_and(|s| s.in_match() && s.holds_game())
                {
                    if let Some(game) = self.play.as_deref_mut() {
                        game.defense_act(from, act);
                    }
                } else if let Some(Spectate { view: v }) = self.scene.screen_mut::<Spectate>() {
                    for (_, g) in v.games.iter_mut() {
                        g.defense_act(from, act);
                    }
                }
            }
            ServerMsg::SkipVotes { votes, of } => {
                if let Some(s) = &mut self.mp {
                    s.skip_votes = Some((votes, of));
                }
            }
            ServerMsg::SkipNow => {
                if let Some(s) = &mut self.mp {
                    s.skip_votes = None;
                }
                if self
                    .scene
                    .screen()
                    .is_some_and(|s| s.in_match() && s.holds_game())
                    && let Some(game) = self.play.as_deref_mut()
                {
                    game.skip(sound(&self.audio), self.play_offset);
                }
            }
            ServerMsg::MapFile(part) => self.receive_map_part(part),
            ServerMsg::Results(r) => {
                if let Some(s) = &mut self.mp {
                    s.results = Some(r);
                }
                if let Some(Spectate { view: v }) = self.scene.screen_mut::<Spectate>() {
                    if v.games.is_empty() {
                        self.stop_audio();
                        self.scene = MpResults::scene();
                    } else {
                        // Watch the rest of it: the view is `SPECTATE_DELAY` behind.
                        v.ended.get_or_insert_with(Instant::now);
                    }
                }
            }
            ServerMsg::Rejected(why) | ServerMsg::Disconnected(why) => {
                let in_mp = self
                    .scene
                    .screen()
                    .is_some_and(|s| s.in_room() || s.spectating());
                self.toast(format!("multiplayer: {why}"));
                self.mp = None;
                self.picking_lobby_map = false;
                if in_mp {
                    self.stop_audio();
                    self.mp_status = why;
                    self.scene = MpMenu::scene();
                }
            }
            ServerMsg::GuessRound {
                round,
                rounds,
                hash,
                from,
                seconds,
            } => {
                if let Some(g) = &mut self.guessr {
                    g.start_round(round, rounds, hash, from, seconds);
                }
            }
            ServerMsg::GuessWrong { tries } => {
                if let Some(g) = &mut self.guessr {
                    g.tries = tries;
                }
                self.toast(if tries == 0 {
                    "wrong, and that was your last guess".into()
                } else {
                    format!("wrong! {tries} left")
                });
            }
            ServerMsg::GuessReveal { hash, got, last } => {
                if let Some(g) = &mut self.guessr {
                    g.reveal(&self.library, hash, got, last);
                }
                // Picking a guess: back to the lobby to see the answer.
                if self.picking_lobby_map && self.scene.is_select() {
                    self.picking_lobby_map = false;
                    self.scene = Lobby::scene();
                }
            }
            ServerMsg::Chat { from, text } => {
                let text = if self.cfg.chat_filter {
                    net::censor(&text)
                } else {
                    text
                };
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
            self.quit_asked_at = Some(Instant::now());
        }
    }

    /// How far in the exit question has slid at `now`, 0 to 1: fast, then easing to a stop.
    fn quit_slide(&self, now: Instant) -> f32 {
        let Some(at) = self.quit_asked_at.filter(|_| self.cfg.animations) else {
            return 1.0;
        };
        let ms = now.saturating_duration_since(at).as_secs_f32() * 1000.0;
        1.0 - (1.0 - (ms / QUIT_SLIDE_MS).min(1.0)).powi(3)
    }

    fn key(
        &mut self,
        code: KeyCode,
        repeat: bool,
        text: Option<&str>,
        at: Instant,
        window: Option<&Window>,
    ) {
        // Back first takes the upright player's list away.
        if code == KeyCode::Escape
            && let Some(p) = self.player.as_mut().filter(|p| p.list_open)
        {
            p.list_open = false;
            return;
        }
        // A key that wakes the idle screen does nothing else.
        if self.idle.input(at) {
            return;
        }
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
        self.last_key = (code, at);
        if self.select_zone_key(code, window) {
            return;
        }
        if let Some(mapped) = self.drum_menu_key(code) {
            if !repeat {
                self.key(mapped, false, None, at, window);
            }
            return;
        }
        // Tab in the editor's test play is autoplay, as in osu!'s, whatever the map info key is.
        if code == KeyCode::Tab
            && !(self.modifiers_shift || self.modifiers_ctrl || self.modifiers_alt)
            && self.scene.is_play()
            && self
                .game()
                .is_some_and(|game| game.view.test_play && game.paused().is_none())
        {
            if !repeat {
                self.toggle_test_auto();
            }
            return;
        }
        let pressed = keys::Chord {
            key: code,
            ctrl: self.modifiers_ctrl,
            shift: self.modifiers_shift,
            alt: self.modifiers_alt,
        };
        match self
            .hotkey_place(code)
            .map(|place| self.cfg.hotkeys.resolve(pressed, place))
        {
            None | Some(keys::Resolved::Same) => self.key_inner(code, repeat, text, at, window),
            Some(keys::Resolved::Nothing) => {}
            Some(keys::Resolved::As(c)) => {
                let held = (
                    self.modifiers_ctrl,
                    self.modifiers_shift,
                    self.modifiers_alt,
                );
                (
                    self.modifiers_ctrl,
                    self.modifiers_shift,
                    self.modifiers_alt,
                ) = (c.ctrl, c.shift, c.alt);
                self.key_inner(c.key, repeat, None, at, window);
                (
                    self.modifiers_ctrl,
                    self.modifiers_shift,
                    self.modifiers_alt,
                ) = held;
            }
        }
    }

    /// Song select with nothing over it.
    fn select_open(&self) -> bool {
        self.scene.is_select()
            && !self.on_menu()
            && self.mod_select.is_none()
            && self.select.picker.is_none()
            && self.customization_tab().is_none()
            && !self.social.open
            && self.profile.is_none()
            && self.medals.is_none()
    }

    /// What a drum key does in a menu with `Config::drum_menus`, as the key that does the same.
    fn drum_menu_key(&self, code: KeyCode) -> Option<KeyCode> {
        if !self.cfg.drum_menus || self.modifiers_ctrl || self.modifiers_alt {
            return None;
        }
        let drum = self.cfg.keys.iter().position(|&k| k == code)?;
        let paused = self.scene.is_play()
            && self
                .game()
                .is_some_and(|game| game.paused().is_some() && game.resuming().is_none());
        let place = if self.select_open() && !self.select.searching {
            drum_nav::Place::Select(self.select_zone)
        } else if self.mod_select.is_some() || (paused && !self.social.open) {
            drum_nav::Place::Menu
        } else {
            return None;
        };
        Some(drum_nav::key_of(drum, place))
    }

    /// Song select's rows of buttons with `Config::drum_menus`: Tab moves between them and the
    /// list. True if `code` was theirs.
    fn select_zone_key(&mut self, code: KeyCode, window: Option<&Window>) -> bool {
        if !self.cfg.drum_menus || !self.select_open() || self.picking_lobby_map {
            return false;
        }
        if code == KeyCode::Tab {
            self.select.searching = false;
            self.select_zone = self
                .select_zone
                .tabbed(&self.select_rows, self.modifiers_shift);
            return true;
        }
        if self.select.searching {
            if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
                self.select_zone = drum_nav::Zone::List;
            }
            return false;
        }
        let drum_nav::Zone::Row { bottom, at } = self.select_zone else {
            return false;
        };
        match code {
            KeyCode::ArrowLeft => self.select_zone = self.select_zone.moved(&self.select_rows, -1),
            KeyCode::ArrowRight => self.select_zone = self.select_zone.moved(&self.select_rows, 1),
            KeyCode::Escape => self.select_zone = drum_nav::Zone::List,
            KeyCode::Enter | KeyCode::NumpadEnter => match self.select_rows.get(bottom, at) {
                Some(hit::Hit::TextField) => self.select.searching = true,
                Some(hit) => {
                    self.select_zone = drum_nav::Zone::List;
                    self.click(hit, window);
                }
                None => self.select_zone = drum_nav::Zone::List,
            },
            KeyCode::ArrowUp | KeyCode::ArrowDown => {
                self.select_zone = drum_nav::Zone::List;
                return false;
            }
            _ => return false,
        }
        true
    }

    /// Where hotkeys are pressed now, or None where every key is taken as it is: something being
    /// typed or bound, a list or the mod select open, a drum key in a song, a letter typed into
    /// a search.
    fn hotkey_place(&self, code: KeyCode) -> Option<keys::Place> {
        let busy = self.splash.is_some()
            || self.scene.screen().is_some_and(|s| s.takes_every_key())
            || self.profile.is_some()
            || self.medals.is_some()
            || self.unlock.showing()
            || self.settings_ui.binding.is_some()
            || self.settings_ui.editing.is_some()
            || self.settings_ui.dropdown.is_some()
            || self.settings_ui.choices.is_some()
            || self.mod_select.is_some()
            || self.customization_tab().is_some();
        let plain = !self.modifiers_ctrl && !self.modifiers_alt;
        let settings_idle =
            self.in_settings() && self.settings_ui.search.is_empty() && !keys::starts_text(code);
        if busy
            || (plain && self.in_song() && self.cfg.keys.contains(&code))
            || (plain && self.takes_text() && keys::types_text(code) && !settings_idle)
        {
            return None;
        }
        Some(match &self.scene {
            Scene::Other(sel) if sel.select_screen() && self.on_menu() => keys::Place::Other,
            Scene::Other(sel)
                if sel.select_screen()
                    && self.select.picker.is_none()
                    && self.select.menu.is_none() =>
            {
                keys::Place::Select
            }
            Scene::Other(s)
                if s.song_play().is_some() && self.game().is_some_and(|g| g.paused().is_some()) =>
            {
                keys::Place::Paused
            }
            Scene::Other(s) if s.song_play().is_some() => keys::Place::Play,
            Scene::Other(s) => s.key_place().unwrap_or(keys::Place::Other),
        })
    }

    fn key_inner(
        &mut self,
        code: KeyCode,
        repeat: bool,
        text: Option<&str>,
        at: Instant,
        window: Option<&Window>,
    ) {
        if self.layers_key(
            &app::overlays::OVER_EVERYTHING,
            code,
            repeat,
            text,
            at,
            window,
        ) {
            return;
        }
        self.refresh_play_offset();
        if self.layers_key(&app::overlays::OVER_SCREENS, code, repeat, text, at, window) {
            return;
        }
        // A collection's name being typed takes every key.
        if self.select.naming.is_some() && self.scene.is_select() {
            self.collection_naming_key(code, text);
            return;
        }
        // Key capture must see every key, before any shortcut can claim it.
        if self.settings_ui.dropdown.is_some() && !repeat {
            let nav = matches!(
                code,
                KeyCode::ArrowUp
                    | KeyCode::ArrowDown
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::Enter
                    | KeyCode::NumpadEnter
                    | KeyCode::Space
                    | KeyCode::Escape
                    | KeyCode::Backspace
                    | KeyCode::Tab
                    | KeyCode::KeyS
                    | KeyCode::KeyA
                    | KeyCode::KeyE
                    | KeyCode::F2
                    | KeyCode::Insert
                    | KeyCode::Delete
            );
            // Typing a size takes every key.
            let typing = self
                .settings_ui
                .dropdown
                .as_ref()
                .is_some_and(|d| d.editing.is_some());
            if nav || typing {
                self.dropdown_key(code);
                return;
            }
        }
        if self.settings_ui.editing.is_some() && self.in_settings() {
            if !repeat
                || !matches!(
                    code,
                    KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape
                )
            {
                self.edit_key(code, text);
            }
            return;
        }
        if !repeat
            && self.in_song()
            && self.settings_ui.binding.is_none()
            && (self.modifiers_shift, code) == self.cfg.interface_key
            && !(self.cfg.keys.contains(&code) && !self.modifiers_shift)
        {
            self.cfg.interface_hidden = !self.cfg.interface_hidden;
            self.cfg.save();
            return;
        }
        if self.settings_ui.binding.is_some() && self.in_settings() {
            if !repeat {
                self.settings_key(code, window);
            }
            return;
        }
        if !repeat
            && !self.in_settings()
            && !self
                .scene
                .screen()
                .is_some_and(|s| s.editor() || s.keeps_preset_keys())
            && self.settings_ui.dropdown.is_none()
            && let Some(i) = self.cfg.preset_keys.iter().position(|&k| k == code)
        {
            self.cycle_preset(if i == 0 { -1 } else { 1 });
            return;
        }
        // A drum hit goes straight to the game, the busiest key there is: no screen in between.
        if self.scene.is_play()
            && let Some(game) = self.play.as_deref_mut()
            && game.paused().is_none()
            && let Some(k) = self.cfg.keys.iter().position(|&k| k == code)
        {
            if !repeat && game.replay().is_none() {
                game.press(k, at, sound(&self.audio), self.play_offset);
            }
            return;
        }
        self.screen_key(screens::screen::Key {
            code,
            repeat,
            text,
            at,
            window,
        });
    }

    /// Advances the game to `now` and fills `canvas`. Returns a screenshot name if this frame
    /// should be captured.
    fn build_frame(&mut self, canvas: &mut Canvas, now: Instant, width: f32) -> Option<String> {
        self.drop_finished_play();
        // Here too, so a new choice (or a test's) shows from the next frame on.
        lang::set(self.cfg.language);
        // What this frame is drawn with; the beat carries on from the last frame until
        // `style::set_beat` below.
        let last = render::theme();
        render::set_frame_theme(Some(render::Theme {
            accent: self.cfg.accent,
            animations: self.cfg.animations,
            legacy_menu: !self.cfg.main_menu,
            pulse: last.pulse,
            kiai: last.kiai,
        }));
        game::set_frame_look(Some(game::Look {
            hide_great: self.cfg.hide_great,
            hit_flash: self.cfg.hit_flash,
            black_lane: self.cfg.black_lane,
            true_notes: self.cfg.true_notes,
            note_colours: self.cfg.note_colours,
        }));
        self.select.legacy = !self.cfg.main_menu;
        self.select.to_menu = self.mp.is_none() && !self.picking_lobby_map;
        self.select.covered = self.customization_tab().is_some();
        // A big library's first analysis waits while a song is being played.
        if let Some(info) = &self.mapinfo {
            info.hold(self.in_song());
        }
        self.refresh_play_offset();
        self.follow_map_skin();
        if self.volume_save_at.is_some_and(|at| now >= at) {
            self.volume_save_at = None;
            self.cfg.save();
        }
        if self
            .window_resized_at
            .is_some_and(|t| now.saturating_duration_since(t) > Duration::from_secs(1))
        {
            self.window_resized_at = None;
            self.cfg.save();
        }
        // Notes are drawn this far ahead: the player's visual offset and, with vsync, the time
        // until the frame is on screen.
        let visual_offset = self.cfg.visual_offset_ms + self.vsync_lead() + self.pace_shift_ms;
        // The interface toggle: what it hides, while it's off.
        // The parts switched off in the interface editor are never there.
        let hidden = if self.cfg.interface_hidden {
            self.cfg.interface_hides
        } else {
            hud::Parts::NONE
        }
        .with(self.cfg.hud.off);
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

        // The pictures the last frame drew, then the ones the renderer has for this one.
        self.avatars.update(&canvas.avatars_wanted);
        let picture_dir = self.picture_dir();
        self.pictures
            .update(picture_dir.as_deref(), &canvas.pictures_wanted);
        canvas.clear(width);
        canvas.avatar_slots = self.avatars.slots();
        canvas.picture_slots = self.pictures.slots();
        canvas.mouse = self.mouse_virtual();
        canvas.dt_ms = (dt as f32).clamp(0.0, 100.0);
        self.mob.begin_frame();
        self.mobile_long_press(now);
        self.glitch_between_menus(canvas, now);
        let idle_allowed = self.idle_allowed();
        self.idle
            .update(now, canvas.dt_ms, idle_allowed, self.cfg.idle_minutes);
        // For threads not building frames: the window thread draws the pointer in the accent.
        render::set_animations(self.cfg.animations);
        render::set_accent(self.cfg.accent);
        tko::set_calm(self.cfg.calm_maps);
        // Screen changes fade in.
        let kind = self.scene.kind();
        if self.scene_kind != Some(kind) {
            self.scene_kind = Some(kind);
            self.scene_since = now;
            self.note_scene_for_crashes();
        }
        self.frame_clip = None;
        #[cfg(target_os = "android")]
        {
            self.update_keyboard();
            // A service keeps the music alive with the screen off; it starts on leaving the app.
            // Once out of the app it stays while paused: its controls are the way back.
            let wanted = self.music_goes_on() && (!self.music_paused || self.backgrounded);
            if wanted != self.music_service {
                self.music_service = wanted;
                android::keep_music(wanted);
            }
            for (what, ms) in android::take_media() {
                match what {
                    0 => self.set_music_paused(!self.music_paused),
                    1 => self.music_step(1),
                    2 => self.music_step(-1),
                    _ => {
                        self.preview.seek_music(ms as f64, self.audio.as_ref());
                        // The controls already show it there.
                        if let Some(sent) = &mut self.media_sent {
                            (sent.1, sent.2) = (ms as f64, now);
                        }
                    }
                }
            }
            self.tell_media_controls(now);
            // Only the player turns upright.
            let upright = self.player.is_some();
            if upright != self.may_turn {
                self.may_turn = upright;
                android::allow_portrait(upright);
            }
        }
        self.poll_scan();
        self.poll_folder_pick();
        self.poll_data_move();
        {
            // The Download tab previews remote maps through the same preview.
            let browsing = self.scene.screen().is_some_and(|s| s.previews_remote());
            if browsing != self.preview_remote {
                self.preview_remote = browsing;
                self.preview.forget();
            }
            let in_select = self.in_song_select();
            let in_menu = in_select
                || self
                    .scene
                    .screen_as::<ResultsScreen>()
                    .is_some_and(|r| r.results.opened.is_some() && r.results.music.is_none())
                || self.behind_settings().is_some_and(|prev| {
                    prev.is_select()
                        || prev.screen().is_some_and(|s| s.room_lobby())
                        || prev.screen_is::<Preview>()
                })
                || self.scene.screen().is_some_and(|s| s.tab().is_some());
            let (entries, selected) = if browsing {
                (&self.browse.remote, self.browse.shown)
            } else {
                (&self.library, self.preview_target())
            };
            let rate = self.preview_rate();
            self.preview.kiai_effects = self.cfg.kiai_effects;
            self.preview.own_background =
                shows_own_background(self.behind_settings().unwrap_or(&self.scene));
            self.preview.autoplay = self.cfg.autoplay;
            self.preview.player = self.player.is_some();
            self.preview.menu = self.on_menu();
            self.preview.from_top &= self.preview.menu;
            self.preview.pitch = self.player.as_ref().map_or(1.0, |p| p.pitch());
            // The splash has its own sound: the preview music waits for it.
            let in_menu = in_menu && self.splash.is_none();
            // As they'd play: a lobby leaves OD Override out.
            let shown = self.shown_mods();
            self.preview.mods = if self.mp.is_some() {
                shown.shared().plus(shown.free())
            } else {
                shown
            };
            self.preview.update(
                now,
                entries,
                selected,
                self.audio.as_ref(),
                in_menu,
                in_select,
                self.play_offset,
                rate,
            );
            self.stars.poll();
            self.stars.set_mods(self.cfg.mods, self.cfg.scroll_speed);
            let play_rate = self.play_speed().0;
            if self.cfg.menu_notes && self.on_menu() {
                let playing = self.preview_target();
                self.menu_notes
                    .update(playing, &self.library, &mut self.stars, play_rate);
            }
            if let (true, Some(i)) = (self.scene.screen_is::<Preview>(), selected) {
                self.stars.request(i, &self.library[i].path, play_rate);
            }
            if self.scene.is_select() {
                self.poll_set_status(now);
                for i in self.select.selected_set() {
                    self.stars.request(i, &self.library[i].path, play_rate);
                }
                let (stars, rate) = (&self.stars, play_rate);
                self.select
                    .sort_selected_set(&self.library, &|i| stars.get(i, rate));
                self.select.remember_selection(&self.library);
            }
            let has_bg = self.preview.has_background;
            self.bg_brightness = match self.scene {
                _ if !has_bg => 0.0,
                _ if self.dims_background() => 1.0 - self.cfg.bg_dim,
                _ if self.on_menu() => {
                    if self.player.is_some() {
                        0.25
                    } else {
                        0.5
                    }
                }
                Scene::Other(ref s) => s.background_dim().unwrap_or(0.35),
            };
            self.bg_brightness *= 1.0 - idle::DARKEN * self.idle.amount();
        }
        self.poll_multiplayer();
        self.track_play();
        self.poll_retry(now);
        self.poll_replay_speed();
        // Paused in the fullscreen preview, and left it.
        if self.preview.held && !self.scene.screen_is::<Preview>() {
            self.toggle_preview_held();
        }
        if self.cfg.check_updates {
            self.updater.check_every(UPDATE_CHECK_EVERY);
        }
        // A song started before the check came back keeps the machine to itself until it's over.
        if !self.in_song()
            && let Some(commit) = self.updater.install_at_launch()
        {
            self.toast_download(&commit);
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
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if let Some(v) = self.scene.screen_mut::<VideoEdit>() {
                        v.status = format!("saved \"{name}\" in the videos folder");
                        v.last_video = Some(path);
                    }
                    self.toast("video saved".into());
                }
                Ok(Err(e)) => {
                    self.export = None;
                    if let Some(v) = self.scene.screen_mut::<VideoEdit>() {
                        v.status = format!("export failed: {e}");
                    }
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
        // A test play that couldn't load (or ended up in song select) goes back to its editor.
        if self.editor_back.is_some() && self.scene.is_select() {
            let why = std::mem::take(&mut self.status);
            self.back_to_editor();
            if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>()
                && !why.is_empty()
            {
                view.status = why;
            }
        }
        let mut editor_reload: Option<(usize, f64)> = None;
        let mut editor_test: Option<(usize, Option<f64>)> = None;
        let mut test_over = false;
        let mut fail_restart = false;
        let mut editor_folder: Option<PathBuf> = None;
        let mut editor_renamed: Option<(PathBuf, PathBuf)> = None;
        let mut folder_news: Option<folder_watch::Changes> = None;
        let mut home_out: Option<editor::home::Out> = None;
        let elapsed = now.duration_since(self.started).as_secs_f64() * 1000.0;
        // `TAIKO_EDIT=<a .osu>`: opens the map editor on it once the library has loaded (for
        // headless screenshots and tests, like `TAIKO_INPUT`).
        if let Some(path) = std::env::var_os("TAIKO_EDIT")
            && self.scene.is_select()
            && !self.library.is_empty()
        {
            static ONCE: AtomicBool = AtomicBool::new(false);
            if !ONCE.swap(true, Ordering::Relaxed) {
                let path = PathBuf::from(path);
                let entry = self
                    .library
                    .iter()
                    .position(|e| e.path == path)
                    .unwrap_or(0);
                self.open_editor_at(path, entry);
            }
        }
        // `TAIKO_HOME=<song>[|details|timing]`: opens the editor's front door, on a song if one is
        // given (for headless screenshots, like `TAIKO_EDIT`).
        if let Some(spec) = std::env::var_os("TAIKO_HOME")
            && self.scene.is_select()
            && self.splash.is_none()
        {
            static ONCE: AtomicBool = AtomicBool::new(false);
            if !ONCE.swap(true, Ordering::Relaxed) {
                let spec = spec.to_string_lossy().into_owned();
                let (song, page) = spec.split_once('|').unwrap_or((&spec, ""));
                self.open_editor_home();
                let rate = self.audio.as_ref().map(|a| a.rate);
                if let Some(h) = self.scene.screen_mut::<EditorHome>() {
                    if !song.is_empty() {
                        h.home.dropped(PathBuf::from(song), rate);
                    }
                    h.home.debug_page(page);
                }
            }
        }
        if let Some(mut script) = self.input_script.take() {
            for action in script.due(elapsed) {
                match action {
                    debug::Action::Key(code) => self.key(code, false, None, now, None),
                    debug::Action::Shot => self.screenshot_due = true,
                    debug::Action::Wheel(n) => self.wheel(n),
                    debug::Action::Drag {
                        corner,
                        dx,
                        dy,
                        hold,
                    } => self.simulate_drag(corner, dx, dy, hold),
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
                Scene::Other(ref sel) if sel.select_screen() => Some("select"),
                _ if self.scene.screen_is::<ResultsScreen>() => Some("results"),
                _ => None,
            };
            if let Some(name) = screen
                && let Some(pos) = self.pending_screen_shots.iter().position(|&s| s == name)
            {
                self.pending_screen_shots.remove(pos);
                capture = Some(format!("{name}.png"));
                if name == "results" && self.exit_when_done {
                    self.quit = true;
                }
            }
        }
        if !(self.scene.is_select() || self.in_lobby()) {
            self.mod_select = None;
        }
        if self.pick_to_edit && (self.main_menu.is_some() || !self.scene.is_select()) {
            self.pick_to_edit = false;
        }
        // A song or a replay opened by any road leaves the menu behind.
        if self.scene.is_play() || self.scene.screen().is_some_and(|s| s.closes_main_menu()) {
            self.main_menu = None;
        }
        // Back from calibrating: on with the guide.
        if self.guide.after.is_some()
            && self.calibration_run().is_none()
            && !self.scene.screen().is_some_and(|s| s.calibrating())
        {
            self.guide.page = self.guide.after.take();
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
        if let Some(p) = self
            .player
            .as_ref()
            .filter(|p| p.library_len != self.library.len())
        {
            // The library changed under the list: same song on, if it's still there.
            let entry = p.list.entry();
            self.player = Some(Box::new(player::Player::new(&self.library, entry)));
        }
        self.poll_media();
        if let Some(item) = self.main_menu.as_mut().and_then(menu::Menu::ready) {
            self.menu_open(item);
        }
        let on_menu = self.on_menu();
        if !self.scene.is_select() {
            self.select.menu = None;
            self.list_drag = None;
            self.select.cancel_drag();
        }
        // Drawn again below if still watching a replay (after scripted input, which must see the
        // last frame's bar like real input does).
        self.replay_bar = None;
        // Another screen (not a song: nothing may stand between the player and the notes)
        // fades in.
        let scene = self.scene.kind();
        if self.shown_scene.is_some_and(|s| s != scene)
            && !self.in_song()
            && !self.scene.is_play()
            && !self.loading_song()
        {
            self.screen_fade = Some(0.0);
        }
        self.shown_scene = Some(scene);
        // Every screen's selector and titles move with the music, as the main menu does.
        let playing = !self.music_paused && !self.in_song();
        let phase = self
            .preview
            .beat()
            .filter(|b| b.1 > 1.0 && playing)
            .map(|(into, len)| (into / len).rem_euclid(1.0) as f32);
        style::set_beat(
            phase,
            playing && self.cfg.kiai_effects && self.preview.kiai(),
        );
        match &mut self.scene {
            Scene::Other(_) => {
                let frame = screens::screen::Frame {
                    now,
                    preview_target,
                    lobby_host,
                    size_label: &size_label,
                    lobby_download,
                    visual_offset,
                    hidden,
                    on_menu,
                    dt,
                };
                if let Some(s) = self.draw_screen(canvas, &frame) {
                    next = Some(s);
                }
                // The front door's frame may ask for something, done once the frame is drawn.
                if let Some(h) = self.scene.screen_mut::<EditorHome>() {
                    home_out = h.out.take();
                }
                // So may the play's.
                if let Some(p) = self.scene.screen_mut::<PlayScreen>() {
                    let out = std::mem::take(&mut p.out);
                    if out.capture.is_some() {
                        capture = out.capture;
                    }
                    test_over = out.test_over;
                    fail_restart = out.fail_restart;
                }
                // So may the editor's.
                if let Some(e) = self.scene.screen_mut::<MapEditor>() {
                    let out = std::mem::take(&mut e.out);
                    editor_reload = out.reload;
                    editor_test = out.test;
                    editor_folder = out.folder;
                    editor_renamed = out.renamed;
                    folder_news = out.folder_news;
                }
            }
        }
        // A screen just opened comes up out of black, its background too.
        if let Some(ms) = &mut self.screen_fade {
            *ms += canvas.dt_ms;
            let k = (*ms / menu::FADE_MS).min(1.0);
            if k >= 1.0 || !self.cfg.animations {
                self.screen_fade = None;
            } else {
                let w = canvas.width;
                canvas.rect(
                    0.0,
                    0.0,
                    w,
                    render::VIRTUAL_HEIGHT,
                    rgba(0x000000, 1.0 - k * k * (3.0 - 2.0 * k)),
                );
            }
        }
        if self.grip.is_some() {
            layout::draw_guides(canvas, &self.guides);
        }
        if self.touch_used && self.touch_drums() {
            self.draw_touch_zones(canvas, now);
        }
        if self.cfg.show_playfield_size
            && !mobile::active()
            && (self.scene.plays_song() || self.calibration_run().is_some())
        {
            let text = self.playfield_label();
            let tw = Canvas::text_width(7.0, &text);
            canvas.text_right(canvas.width - 8.0, 468.0, 7.0, rgba(0x8a8aa0, 0.85), &text);
            self.readout_rect = Some([canvas.width - 8.0 - tw - 4.0, 464.0, tw + 8.0, 15.0]);
        } else {
            self.readout_rect = None;
        }
        self.draw_mobile_rate(canvas);
        if let Some(cur) = self.mod_select {
            let seed = if self.in_lobby() {
                self.mp
                    .as_ref()
                    .and_then(|s| s.map.as_ref())
                    .and_then(|m| m.seed)
                    .unwrap_or(self.cfg.random_seed)
            } else {
                self.cfg.random_seed
            };
            if mobile::active() {
                self.draw_mobile_mods(canvas, self.shown_mods());
            } else {
                ui::draw_mod_select(
                    canvas,
                    self.shown_mods(),
                    self.cfg.favourite_mods,
                    &self.cfg.mod_presets,
                    self.cfg.rate,
                    cur,
                    seed,
                );
            }
            if let Some(name) = &self.preset_naming {
                draw_preset_naming(canvas, name);
            }
            if let Some(k) = self.preset_deleting
                && let Some(p) = self.cfg.mod_presets.get(k)
            {
                draw_preset_deleting(canvas, k, &p.name);
            }
            if let Some(input) = &self.random_setup {
                canvas.hits.clear();
                let w = canvas.width;
                canvas.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.85));
                let phone = mobile::active();
                canvas.text_centered(w / 2.0, 160.0, 14.0, rgba(0xffffff, 1.0), "Random seed");
                canvas.text_centered(
                    w / 2.0,
                    200.0,
                    12.0,
                    rgba(0xffffff, 1.0),
                    &format!("{input}_"),
                );
                canvas.text_centered(
                    w / 2.0,
                    234.0,
                    7.0,
                    rgba(0x9a9ab0, 1.0),
                    "Same map + same seed = same colours. Retries keep the seed.",
                );
                if phone {
                    // Tapping the number brings up the keyboard.
                    canvas.hit(w / 2.0 - 220.0, 188.0, 440.0, 36.0, hit::Hit::TextField);
                    let (bw, bh, by) = (150.0, 46.0, 280.0);
                    let key = hit::Hit::Key;
                    canvas.button(
                        w / 2.0 - 235.0,
                        by,
                        bw,
                        bh,
                        "Apply",
                        12.0,
                        key(KeyCode::Enter),
                        true,
                    );
                    canvas.button(
                        w / 2.0 - 75.0,
                        by,
                        bw,
                        bh,
                        "New seed",
                        12.0,
                        key(KeyCode::KeyR),
                        false,
                    );
                    canvas.button(
                        w / 2.0 + 85.0,
                        by,
                        bw,
                        bh,
                        "Cancel",
                        12.0,
                        key(KeyCode::Escape),
                        false,
                    );
                } else {
                    canvas.text_centered(
                        w / 2.0,
                        250.0,
                        7.0,
                        rgba(0x9a9ab0, 1.0),
                        "Delete clears   R new seed   Ctrl+C / Ctrl+V copy / paste",
                    );
                    canvas.button(
                        w / 2.0 - 100.0,
                        280.0,
                        90.0,
                        24.0,
                        "Apply",
                        9.0,
                        hit::Hit::Key(KeyCode::Enter),
                        true,
                    );
                    canvas.button(
                        w / 2.0 + 10.0,
                        280.0,
                        90.0,
                        24.0,
                        "Cancel",
                        9.0,
                        hit::Hit::Key(KeyCode::Escape),
                        false,
                    );
                }
            }
            if let Some(setup) = &self.universe_setup {
                setup.draw(
                    canvas,
                    &self.cfg.universes,
                    &self.cfg.universe_presets,
                    game::FIRST_FOCUS,
                );
            }
            if let Some(setup) = &self.shapeshift_setup {
                setup.draw_panel(canvas, &self.cfg.shapeshift, game::rail_outline);
            }
            if let Some(setup) = &self.tact_setup {
                setup.draw_panel(canvas, self.cfg.tact);
            }
            if let Some(setup) = &self.od_setup {
                setup.draw_panel(canvas, self.cfg.od_override);
            }
            if let Some(tab) = self.customization_tab() {
                ui::draw_customization_tabs(canvas, &mods::customizable(self.cfg.mods), tab);
            }
        }
        if self.settings_ui.dropdown.is_some() {
            let (groups, current, screen) = (
                self.preset_groups(),
                self.current_preset(),
                self.screen_size(),
            );
            let (saved, playfield) = (self.cfg.saved_presets.len(), self.playfield_size());
            if let Some(dd) = &mut self.settings_ui.dropdown {
                dd.draw(canvas, &groups, saved, current, screen, playfield);
            }
        }
        if let Some(file) = editor_folder {
            self.show_in_folder(&file);
        }
        if let Some((from, to)) = editor_renamed {
            self.map_renamed(&from, to);
        }
        if let Some(news) = folder_news {
            self.folder_changed(news);
        }
        if let Some((entry, from)) = editor_test {
            self.start_test_play(entry, from);
        }
        if test_over {
            self.back_to_editor();
        }
        if fail_restart {
            self.stop_audio();
            self.restart_map();
        }
        if let Some((entry, speed)) = editor_reload {
            let (rx, rate) = self.spawn_track_loader(entry, speed);
            if let Some(MapEditor { view, .. }) = self.scene.screen_mut::<MapEditor>() {
                view.set_track_loader(rx, rate);
            }
        }
        if let Some(out) = home_out {
            self.home_out(out);
        }
        if std::mem::take(&mut self.calibration_over) {
            self.finish_calibration();
        }
        if let Some(s) = next {
            let results = s.screen_as::<ResultsScreen>();
            let music_on = results.is_some_and(|r| r.results.music.is_some());
            if music_on {
                self.preview.invalidate_audio();
            } else if results.is_some() || s.is_select() {
                self.stop_audio();
            }
            // TAIKO_STATS: the play's smoothness, whether the music plays on at the results or not.
            if (results.is_some() || s.is_select())
                && let (Some(sm), true) = (&mut self.smooth, self.scene.is_play())
            {
                eprintln!(
                    "--- smoothness ({}) ---\n{}",
                    self.present.name(),
                    sm.report()
                );
                *sm = debug::Smoothness::default();
            }
            if let (Some(r), true) = (results, self.scene.is_play()) {
                let st = &r.results.stats;
                eprintln!(
                    "result: {:.2}%  great {} ok {} miss {}  max combo {}  UR {:.1}  mean {:+.2} ms  rolls {}  swells {}  big {}",
                    st.accuracy() * 100.0,
                    st.great,
                    st.ok,
                    st.miss,
                    st.max_combo,
                    st.unstable_rate(),
                    st.mean_error(),
                    st.roll_ticks,
                    st.swells,
                    st.strong
                );
            }
            self.scene = s;
            self.drop_finished_play();
        }

        let fade = render::progress(self.scene_since, 160.0);
        if fade < 1.0 {
            canvas.rect(
                0.0,
                0.0,
                canvas.width,
                render::VIRTUAL_HEIGHT,
                rgba(0x121219, 0.85 * (1.0 - fade)),
            );
        }
        if self.cfg.show_fps && !self.mobile_bar() {
            let mode = match self.present {
                PresentChoice::Fifo => "vsync".to_string(),
                p if self.cfg.fps_cap > 0 => format!("{} cap {}", p.name(), self.cfg.fps_cap),
                p => p.name().to_string(),
            };
            let text = format!(
                "{:.0} fps  {:.2} ms  worst {:.1} ms  {mode}",
                self.stats.fps,
                1000.0 / self.stats.fps.max(1.0),
                self.stats.worst_ms
            );
            let tw = Canvas::text_width(8.0, &text);
            canvas.round_rect(
                [6.0, 6.0],
                [tw + 12.0, 16.0],
                4.0,
                rgba(0x000000, 0.55),
                rgba(0, 0.0),
                0.0,
            );
            canvas.text(12.0, 10.0, 8.0, rgba(0x9fe09f, 1.0), &text);
        }
        if let Some((done, total)) = self
            .mapinfo
            .as_ref()
            .and_then(|i| i.progress(now))
            .filter(|_| !self.in_song() && self.splash.is_none())
        {
            draw_analysis(canvas, done, total);
        }
        // Not during a song, where a pointer left there would hold it over the notes for good.
        // Holding quick retry: the screen goes dark on the way to the restart.
        if let Some(t) = self.retry_progress(now) {
            canvas.rect(0.0, 0.0, canvas.width, 480.0, rgba(0x000000, t * 0.95));
        }
        self.draw_layers(&app::overlays::DRAW_UNDER_TOASTS, canvas, now);
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
            canvas.round_rect(
                [x - 10.0, 344.0],
                [tw + 20.0, 24.0],
                6.0,
                rgba(0x15151d, 0.95),
                rgba(0xFFD34D, 1.0),
                1.0,
            );
            canvas.text(x, 351.0, 10.0, rgba(0xFFD34D, 1.0), &text);
            canvas.rect(x, 363.0, tw, 1.0, rgba(0xFFD34D, 0.8));
        }
        if let Some(t) = &self.toast {
            if now < t.until {
                let a = (t.until.duration_since(now).as_secs_f32() / 0.3).min(1.0);
                let rise =
                    (1.0 - render::progress(t.until - Duration::from_millis(1500), 160.0)) * 14.0;
                // Long messages get smaller print, then are cut short.
                let room = canvas.width - 72.0;
                let size = Canvas::fit_size(12.0, 8.0, room, &t.text);
                let text = Canvas::fit(size, room, &t.text);
                let tw = Canvas::text_width(size, &text);
                let x = canvas.width / 2.0 - tw / 2.0;
                canvas.round_rect(
                    [x - 12.0, 380.0 + rise],
                    [tw + 24.0, 28.0],
                    8.0,
                    rgba(0x000000, 0.7 * a),
                    rgba(0xffffff, 0.2 * a),
                    1.0,
                );
                canvas.text(x, 394.0 - size / 2.0 + rise, size, rgba(0xffffff, a), &text);
            } else {
                self.toast = None;
            }
        }
        self.draw_update(canvas);
        // Over everything so far (the top bar's buttons too); a dialog or a drop goes on top.
        // The idle screen's rays hear what the menu or player plays.
        if self.idle.amount() > 0.0 {
            let engine = self.audio.as_ref();
            let sounding = !self.music_paused
                && self
                    .preview
                    .sounding(engine, now, &mut self.idle.spectrum.samples);
            self.idle
                .spectrum
                .update(canvas.dt_ms, sounding, engine.map_or(48_000, |a| a.rate));
        }
        let song = self.idle_song();
        self.idle.draw(
            canvas,
            self.preview.has_background,
            song.as_ref().map(|(t, a)| (t.as_str(), a.as_str())),
        );
        self.draw_drag_hover(canvas, now);
        self.draw_layers(&app::overlays::DRAW_ON_TOP, canvas, now);
        if self.splash.is_some() {
            let boot = self.boot_log();
            // Some window managers never report focus: don't wait forever.
            let ready = self.focus_seen || self.started.elapsed() > Duration::from_secs(5);
            if let Some(s) = &mut self.splash
                && !s.draw(canvas, now, self.audio.as_ref(), &boot, ready)
            {
                self.splash = None;
            }
        }
        self.draw_shot_view(canvas);
        std::mem::swap(&mut self.hits, &mut canvas.hits);
        capture
    }

    /// What the splash's boot log shows: what is actually starting up.
    fn boot_log(&self) -> Vec<String> {
        let songs = match &self.scan {
            Some((_, found)) => {
                format!("songs  scanning... {} files", found.load(Ordering::Relaxed))
            }
            None => format!("songs  {} difficulties", self.library.len()),
        };
        let audio = self
            .audio
            .as_ref()
            .map(|a| a.description.clone())
            .unwrap_or_else(|| "none (silent)".into());
        let skin = if self.cfg.skin.is_empty() {
            "built-in"
        } else {
            &self.cfg.skin
        };
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
        self.installed_sets
            .get_or_insert_with(|| library.iter().filter_map(Entry::set_id).collect())
    }

    /// Downloads a set unless it's already here or on its way.
    fn start_download(&mut self, set: download::SetInfo) {
        let here = match set.custom {
            Some(_) => self.select.custom_folder.values().any(|&id| id == set.id),
            None => self.installed_sets().contains(&set.id),
        };
        if here {
            self.toast(format!(
                "{} - {} is already in your songs",
                set.artist, set.title
            ));
            return;
        }
        if self.downloads.iter().any(|j| j.set.same(&set)) {
            return;
        }
        self.browse.failed_jobs.retain(|(s, _)| !s.same(&set));
        let dir = self.songs_dir.clone();
        self.downloads.push(download::start(set, dir));
    }

    /// Download tab: goes to song select on an installed result's maps.
    fn play_downloaded(&mut self, set: &download::SetInfo) {
        let custom_folders: Vec<&Path> = match set.custom {
            Some(_) => self
                .select
                .custom_folder
                .iter()
                .filter(|(_, id)| **id == set.id)
                .map(|(dir, _)| dir.as_path())
                .collect(),
            None => Vec::new(),
        };
        let maps = main_maps(self.library.iter().filter(|e| match set.custom {
            Some(_) => e.path.parent().is_some_and(|d| custom_folders.contains(&d)),
            None => e.set_id() == Some(set.id),
        }));
        if maps.is_empty() {
            self.toast(format!(
                "{} - {} isn't in your songs",
                set.artist, set.title
            ));
            return;
        }
        self.browse.menu = None;
        self.scene = SongSelect::scene();
        self.reveal_maps(maps);
    }

    /// Lobby: fetch the map we don't have (by its set ID, else by searching for it).
    fn download_lobby_map(&mut self) {
        let Some(m) = self.mp.as_ref().and_then(|s| s.map.clone()) else {
            return;
        };
        if self.mp.as_ref().is_some_and(|s| s.have_map) || self.lobby_lookup.is_busy() {
            return;
        }
        self.fetch_map(m);
    }

    /// Downloads the set a map someone else has comes from (mirrors: by set id, else a search).
    fn fetch_map(&mut self, m: net::MapInfo) {
        if self.lobby_lookup.is_busy() {
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
                custom: None,
            }),
            None => {
                self.lobby_lookup = Job::spawn(move || {
                    download::search(
                        &format!("{} {}", m.artist, m.title),
                        download::Category::Any,
                        download::SortKey::Relevance,
                        true,
                        0,
                    )
                    .and_then(|(sets, _)| {
                        sets.into_iter()
                            .find(|s| {
                                s.title.eq_ignore_ascii_case(&m.title)
                                    && s.diffs.iter().any(|d| d.version == m.version)
                            })
                            .ok_or_else(|| "not found on the mirrors".to_string())
                    })
                });
                self.toast("looking for the map on the mirrors...".into());
            }
        }
    }

    /// Host: packs the lobby map's set folder and sends it to player `to`, on a thread.
    fn send_map_to(&mut self, to: u32) {
        let Some(s) = &self.mp else { return };
        let tx = s.client.sender();
        let folder = s
            .map_entry
            .and_then(|i| self.library[i].path.parent().map(|p| p.to_path_buf()));
        std::thread::spawn(move || {
            let send = |part: net::MapPart| tx.send(net::ClientMsg::MapFile { to, part }).is_ok();
            let error = |why: String| net::MapPart {
                name: String::new(),
                total: 0,
                offset: 0,
                data: String::new(),
                error: Some(why),
            };
            let Some(folder) = folder else {
                send(error("the host doesn't have the map either".into()));
                return;
            };
            let name = folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
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
            self.toast(format!(
                "the host couldn't send it ({e}); trying the mirrors"
            ));
            self.download_lobby_map();
            return;
        }
        if part.offset == 0 {
            let Some(m) = self.mp.as_ref().and_then(|s| s.map.clone()) else {
                return;
            };
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
                custom: None,
            };
            let (job, feed) = download::receive(
                set,
                part.name.clone(),
                part.total,
                m.hash,
                self.songs_dir.clone(),
            );
            self.downloads.push(job);
            self.host_transfer = Some(feed);
        }
        let Some(feed) = &self.host_transfer else {
            return;
        };
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

    /// A lobby map we don't have comes by itself: from the host, who has it whatever it is, and
    /// from the mirrors if that fails.
    fn auto_fetch_lobby_map(&mut self) {
        let missing = self
            .mp
            .as_ref()
            .filter(|s| !s.is_host() && !s.have_map)
            .and_then(|s| s.map.as_ref())
            .map(|m| m.hash);
        // Nothing's still being waited for once it's here, or we've left.
        self.host_map_asked &= missing.is_some();
        let busy = self.host_transfer.is_some() || self.lobby_download_status().is_some();
        let waiting = self
            .host_asked_at
            .filter(|_| self.host_map_asked)
            .map(|at| at.elapsed());
        match multiplayer::next_fetch(missing, self.auto_fetched, busy, waiting) {
            multiplayer::Fetch::Nothing => {}
            multiplayer::Fetch::Tried(map) => self.auto_fetched = Some(map),
            multiplayer::Fetch::AskHost(map) => {
                self.auto_fetched = Some(map);
                if let Some(s) = &self.mp {
                    s.client.send(net::ClientMsg::RequestMap);
                    self.host_map_asked = true;
                    self.host_asked_at = Some(Instant::now());
                }
            }
            multiplayer::Fetch::Mirrors => {
                self.host_map_asked = false;
                self.toast("the host isn't answering; trying the mirrors".into());
                self.download_lobby_map();
            }
        }
    }

    /// The lobby map's download progress for the lobby screen: what's going on, and how much of
    /// it is done once that's known.
    fn lobby_download_status(&self) -> Option<(String, Option<f32>)> {
        if self.lobby_lookup.is_busy() {
            return Some(("looking it up...".into(), None));
        }
        if self.host_map_asked {
            return Some(("asking the host for the map...".into(), None));
        }
        let m = self.mp.as_ref()?.map.as_ref()?;
        let done = map_download(&self.downloads, m)?;
        Some((download_text(done), done))
    }

    /// What frames to come will do (not a thread): a preview waiting out its debounce, a
    /// resume countdown running out.
    #[cfg(test)]
    fn is_due(&self) -> bool {
        // The resume countdown ends at a wall-clock time (`Sim::resume_after`) that frames
        // reach as their time goes on.
        let resuming = self.game().is_some_and(|game| game.resuming().is_some());
        self.preview.is_due() || resuming
    }

    /// The background work still on its way, by name; empty when no thread works for the
    /// client. The regression harness waits for this instead of watching frames for changes.
    #[cfg(test)]
    fn busy(&self) -> Vec<&'static str> {
        let info_pending = self.select.info.as_ref().is_some_and(|i| {
            let i = i.lock().unwrap();
            i.len() < self.library.len() || i.iter().any(Option::is_none)
        });
        let map_loading = match &self.scene {
            Scene::Other(s) => s.loading(),
        };
        let live_pp =
            self.scene.plays_song() && self.game().is_some_and(|game| game.live_pp_busy());
        [
            ("the library scan", self.scan.is_some()),
            ("the maps' analysis", info_pending),
            ("the song preview", !self.preview.is_idle()),
            ("a map loading", map_loading),
            ("a leaderboard", self.board_fetch.is_some()),
            ("a set's update check", self.set_check.is_some()),
            ("the loved list", self.loved_call.is_some()),
            ("the custom maps' call", self.custom_call.is_busy()),
            ("osu!.db", self.osudb_rx.is_busy()),
            ("the pp backfill", self.pp_backfill.is_busy()),
            ("the player's profile", self.me_fetch.is_busy()),
            ("a score's submit", !self.submits.is_empty()),
            ("the lobby map's lookup", self.lobby_lookup.is_busy()),
            ("the folder dialog", self.folder_pick.is_busy()),
            ("the data move", self.data_move.is_some()),
            (
                "a screenshot",
                self.shot_upload.is_some()
                    || self.shot_view.as_ref().is_some_and(|v| v.rx.is_busy()),
            ),
            (
                "a score link",
                self.score_link.as_ref().is_some_and(|l| l.fetch.is_busy()),
            ),
            ("the replay's music", self.replay_speed_rx.is_busy()),
            ("the live pp", live_pp),
            ("the download browser", self.browse.is_busy()),
            (
                "the account server",
                self.mp_menu.account_request.is_some() || self.mp_menu.rooms_request.is_some(),
            ),
        ]
        .into_iter()
        .filter_map(|(name, busy)| busy.then_some(name))
        .collect()
    }

    /// Scores saved before pp get theirs worked out in the background, once, and the index is
    /// written again with them.
    fn poll_pp_backfill(&mut self) {
        if !self.pp_backfill_started {
            self.pp_backfill_started = true;
            let missing: Vec<PathBuf> = self
                .scores
                .iter()
                .filter(|r| r.pp.is_none())
                .map(|r| r.file.clone())
                .collect();
            if !missing.is_empty() {
                self.pp_backfill = Job::from_receiver(scores::backfill_pp(missing));
            }
        }
        let Some(found) = self.pp_backfill.poll() else {
            return;
        };
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
                Ok(online::Reply::Submitted {
                    relax,
                    map_rank,
                    pp,
                    rank,
                    medals,
                }) => {
                    self.medals_earned(medals);
                    let mine = if relax {
                        &mut self.me_relax
                    } else {
                        &mut self.me
                    };
                    let before = mine.map(|(b, _)| b);
                    // + 0.0: servers before the fix sent a new player's pp as -0.0 ("-0 pp").
                    let pp = pp + 0.0;
                    *mine = Some((pp, rank));
                    if let Some(results) = self
                        .scene
                        .screen_mut::<ResultsScreen>()
                        .map(|r| &mut r.results)
                        && results.opened.is_none()
                        && results.replay.is_some()
                    {
                        results.total = Some(ui::Total {
                            before,
                            after: pp,
                            rank,
                        });
                    }
                    let board = if relax { " (Relax)" } else { "" };
                    let place = map_rank
                        .map(|r| format!("#{r} on this map{board}"))
                        .unwrap_or_else(|| "sent (not your best here)".into());
                    let total = rank
                        .map(|r| format!("  |  {pp:.0} pp, #{r} overall{board}"))
                        .unwrap_or_default();
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
            && let Ok(rows) = rx.try_recv()
        {
            let rows = rows.map_err(|e| {
                if e == "bad request" {
                    "the server doesn't keep scores yet".into()
                } else {
                    e
                }
            });
            self.online_boards.insert(*map, (Instant::now(), rows));
            self.board_fetch = None;
        }
    }

    /// The online leaderboard of map `hash` (fetched in the background; again after a minute).
    fn online_board(
        &mut self,
        hash: u64,
        relax: bool,
    ) -> Option<&Result<Vec<online::OnlineScore>, String>> {
        let key = (hash, relax);
        let stale = self
            .online_boards
            .get(&key)
            .is_none_or(|(at, _)| at.elapsed() > Duration::from_secs(60));
        if stale && self.board_fetch.is_none() {
            self.board_fetch = Some((
                key,
                fetch_board(self.cfg.online_server.clone(), hash, relax),
            ));
        }
        self.online_boards.get(&key).map(|(_, rows)| rows)
    }

    /// The logged-in player's own pp and rank, fetched once per login.
    fn poll_me(&mut self) {
        let account = self.mp_menu.account.clone().unwrap_or_default();
        if account != self.me_for {
            self.me_for = account.clone();
            self.me = None;
            self.me_relax = None;
            self.me_fetch.cancel();
            if !account.is_empty() {
                let server = self.cfg.online_server.clone();
                self.me_fetch = Job::spawn(move || {
                    online::call(&server, online::Request::Profile { name: account }).and_then(
                        |r| match r {
                            online::Reply::Profile(p) => Ok(*p),
                            _ => Err(crate::lang::odd_reply()),
                        },
                    )
                });
            }
        }
        if let Some(Ok(p)) = self.me_fetch.poll() {
            self.me = Some((p.pp + 0.0, p.rank));
            self.me_relax = Some((p.relax.pp + 0.0, p.relax.rank));
        }
    }

    fn poll_changelog(&mut self) {
        if let Some(view) = &mut self.changelog {
            view.poll();
        }
        // A delayed response must not open an overlay in the middle of a song or replace
        // a changelog the player has already opened manually, and the guide comes first: what's
        // new would cover it while its keys still went to the guide, closing it unseen.
        if !self.in_song()
            && self.guide.page.is_none()
            && self.guide.after.is_none()
            && let Some(view) = changelog::poll_automatic(&mut self.pending_changelog)
            && self.changelog.is_none()
        {
            self.changelog = Some(view);
        }
    }

    fn poll_downloads(&mut self) {
        self.poll_shots();
        self.auto_fetch_lobby_map();
        self.poll_me();
        self.poll_pp_backfill();
        self.poll_online_scores();
        if let Some(p) = &mut self.profile {
            p.poll();
        }
        if let Some(view) = &mut self.medals {
            view.poll();
        }
        // The splash is drawn over everything, so it waits for that as well as for a song.
        if let Some((tier, secret)) = self.unlock.tick(!self.in_song() && self.splash.is_none()) {
            self.sfx(audio::Sfx::medal(tier, secret));
        }
        self.tick_guide();
        self.poll_changelog();
        #[cfg(target_os = "android")]
        if self.imports_checked.elapsed() >= Duration::from_secs(1) {
            self.imports_checked = Instant::now();
            for (archive, name) in android::take_imports() {
                self.import_osz(archive, &name, true);
            }
        }
        match self.lobby_lookup.poll() {
            Some(Ok(set)) => self.start_download(set),
            Some(Err(e)) => self.toast(format!("download failed: {e}")),
            None => {}
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
                Err(e) if e == download::CANCELLED => {
                    if update {
                        self.select.updates.insert(set.id, ui::SetUpdate::Unknown);
                    }
                    self.toast(format!(
                        "stopped downloading {} - {}",
                        set.artist, set.title
                    ));
                }
                Err(e) => {
                    if update {
                        self.select.updates.insert(set.id, ui::SetUpdate::Unknown);
                    } else if !set.diffs.is_empty() || set.custom.is_some() {
                        // A search result: the download tab offers to try it again.
                        let short: String = e.chars().take(60).collect();
                        self.browse.failed_jobs.retain(|(s, _)| !s.same(&set));
                        self.browse.failed_jobs.push((set.clone(), short));
                    }
                    eprintln!("download {} failed: {e}", set.id);
                    self.toast(format!(
                        "download failed: {}",
                        e.chars().take(80).collect::<String>()
                    ));
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
                    if outdated {
                        ui::SetUpdate::Available
                    } else {
                        ui::SetUpdate::UpToDate
                    }
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
            self.select
                .updates
                .entry(id)
                .and_modify(|s| {
                    if *s == ui::SetUpdate::Checking {
                        *s = state
                    }
                })
                .or_insert(state);
        }
        if !self.cfg.check_updates || !self.scene.is_select() {
            self.set_check_due = None;
            return;
        }
        let Some(entry) = self.select.selected() else {
            return;
        };
        let Some(id) = self.library[entry].set_id() else {
            return;
        };
        if self.select.updates.contains_key(&id) {
            return;
        }
        match self.set_check_due {
            Some((due, since)) if due == id => {
                if self.set_check.is_some()
                    || now.saturating_duration_since(since) < Duration::from_millis(800)
                {
                    return;
                }
            }
            _ => {
                self.set_check_due = Some((id, now));
                return;
            }
        }
        // The set's difficulties here: beatmap ID, [Version] and the MD5 osu! knows each version by.
        let folder = self.library[entry].path.parent().map(|p| p.to_path_buf());
        let maps: Vec<(Option<u32>, String, PathBuf)> = self
            .library
            .iter()
            .filter(|e| {
                e.set_id() == Some(id)
                    && e.path.parent() == folder.as_deref()
                    && !self.select.gone.contains(&e.path)
            })
            .map(|e| (e.meta.beatmap_id, e.meta.version.clone(), e.path.clone()))
            .collect();
        self.select.updates.insert(id, ui::SetUpdate::Checking);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = download::set_checksums(id).map(|online| {
                let here: Vec<(Option<u32>, String, String, &PathBuf)> = maps
                    .iter()
                    .filter_map(|(bid, version, p)| {
                        Some((
                            *bid,
                            version.clone(),
                            download::md5_hex(&std::fs::read(p).ok()?),
                            p,
                        ))
                    })
                    .collect();
                let ids: Vec<(Option<u32>, String, String)> = here
                    .iter()
                    .map(|(bid, version, md5, _)| (*bid, version.clone(), md5.clone()))
                    .collect();
                let edited = here
                    .iter()
                    .filter(|(_, _, md5, _)| !download::is_osus(md5, &online))
                    .map(|(_, _, _, p)| (*p).clone())
                    .collect();
                (download::outdated(&ids, &online), edited)
            });
            let _ = tx.send(result);
        });
        self.set_check = Some((id, rx));
    }

    /// Downloads the newest version of map `entry`'s set into its folder: the set's own
    /// difficulties are replaced; home-made ones and edited copies (rates) stay (`download::is_own`).
    fn update_set(&mut self, entry: usize) {
        let e = &self.library[entry];
        let (Some(id), Some(folder)) = (e.set_id(), e.path.parent().map(|p| p.to_path_buf()))
        else {
            self.toast("this map has no osu! set ID to update it by".into());
            return;
        };
        if self.downloads.iter().any(|j| j.set.is_osu_set(Some(id))) {
            return;
        }
        // Which of these are the set's own is asked of osu! on the download's thread.
        let here: Vec<download::LocalDiff> = self
            .library
            .iter()
            .filter(|m| {
                m.path.parent() == Some(folder.as_path())
                    && m.meta.beatmap_id.is_some()
                    && m.set_id() == Some(id)
                    && !self.select.gone.contains(&m.path)
            })
            .map(|m| download::LocalDiff {
                id: m.meta.beatmap_id,
                version: m.meta.version.clone(),
                path: m.path.clone(),
            })
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
            custom: None,
        };
        self.toast(format!("updating {} - {}...", set.artist, set.title));
        self.select.updates.insert(id, ui::SetUpdate::Updating);
        self.downloads
            .push(download::start_update(set, folder, here));
    }

    /// A set downloaded again (see `update_set`): its difficulties in the library are read anew in
    /// place (indices stay valid), ones it no longer has leave the list, new ones join it.
    fn add_updated(&mut self, set: &download::SetInfo, folder: &std::path::Path) {
        let mut fresh: std::collections::HashMap<PathBuf, Entry> =
            library::scan(&[folder.to_path_buf()], &AtomicUsize::new(0), &mut |_| {})
                .into_iter()
                .map(|e| (e.path.clone(), e))
                .collect();
        let mut changed = Vec::new();
        for (i, e) in self.library.iter_mut().enumerate() {
            if e.path.parent() != Some(folder) {
                continue;
            }
            match fresh.remove(&e.path) {
                Some(new) => {
                    *e = new;
                    self.select.gone.remove(&e.path);
                    self.select.edited.remove(&e.path);
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
        if !(self.scene.is_play() || self.scene.screen().is_some_and(|s| s.in_match()))
            && self.mp.is_none()
        {
            self.select.reveal(&self.library, maps);
        } else {
            self.select.pin(&self.library, maps);
        }
    }

    /// Adds a freshly downloaded set to the library (appended, so existing indices stay valid).
    fn add_downloaded(&mut self, set: &download::SetInfo, folder: &std::path::Path) {
        let known: std::collections::HashSet<PathBuf> = self
            .library
            .iter()
            .filter(|e| e.path.starts_with(folder))
            .map(|e| e.path.clone())
            .collect();
        let fresh: Vec<Entry> =
            library::scan(&[folder.to_path_buf()], &AtomicUsize::new(0), &mut |_| {})
                .into_iter()
                .filter(|e| !known.contains(&e.path))
                .collect();
        // Nothing new (unpacked over the same files): the set that's there is what was added.
        let maps = if fresh.is_empty() {
            main_maps(self.library.iter().filter(|e| e.path.starts_with(folder)))
        } else {
            main_maps(fresh.iter())
        };
        let n = fresh.len();
        self.append_maps(fresh);
        match &set.custom {
            // Song select knows it as the website's set, approved or not, straight away.
            Some(custom) => {
                self.select
                    .custom_folder
                    .insert(folder.to_path_buf(), set.id);
                if custom.approved {
                    self.select.custom_approved.insert(set.id);
                }
                self.select.refilter(&self.library);
            }
            None => {
                self.installed_sets().insert(set.id);
            }
        }
        eprintln!(
            "downloaded set {} into {} ({n} maps)",
            set.id,
            folder.display()
        );
        // Lobby: the map we were missing may be here now.
        let library = &self.library;
        let mut lobby_note = None;
        if let Some(s) = &mut self.mp
            && let (Some(m), false) = (&s.map, s.have_map)
        {
            s.map_entry = find_map(library, m);
            s.have_map = s.map_entry.is_some();
            s.client.send(net::ClientMsg::HasMap(s.have_map));
            if !s.have_map && m.set_id == Some(set.id) {
                lobby_note = Some(
                    "downloaded, but the host has a different version of this map".to_string(),
                );
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

    /// The topmost clickable thing under (`x`, `y`), from the last frame.
    /// Whether the song preview is the topmost thing at a point, so it takes the pointer there.
    /// Anything drawn over it (the friends panel, a drop-down, the changelog) gets it instead.
    fn preview_on_top(&self, x: f32, y: f32) -> bool {
        // The mod select and both drop-downs take every click while open; the download one is
        // drawn before the preview, so its click-away area is under it.
        self.mod_select.is_none()
            && self.settings_ui.dropdown.is_none()
            && !(self.scene.screen().is_some_and(|s| s.previews_remote())
                && self.browse.menu.is_some())
            && matches!(self.hit_at(x, y), Some(hit::Hit::Preview))
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<hit::Hit> {
        if mobile::active() {
            return mobile::hit_near(&self.hits, x, y);
        }
        self.hits
            .iter()
            .rev()
            .find(|h| h.contains(x, y))
            .map(|h| h.hit)
    }

    /// The SONGS tab: the songs themselves, not the menu over them.
    fn show_songs(&mut self) {
        self.main_menu = None;
        self.scene = SongSelect::scene();
    }

    /// Switches between the top bar's tabs (songs, settings, multiplayer) from the screens that
    /// show it.
    fn goto_tab(&mut self, tab: usize) {
        let settings_from_menu = self.behind_settings().is_some_and(|prev| {
            prev.is_select() || prev.screen().is_some_and(|s| s.tab().is_some())
        });
        match (self.scene.is_select(), tab) {
            (true, 1) => self.open_settings(),
            (true, 2) => {
                self.picking_lobby_map = false;
                self.open_multiplayer();
            }
            (_, 0) if settings_from_menu => {
                self.close_settings();
                self.show_songs();
            }
            (_, 2) if settings_from_menu => {
                self.close_settings();
                self.open_multiplayer();
            }
            (true, 3) => self.open_download(),
            (_, 3) if settings_from_menu => {
                self.close_settings();
                self.open_download();
            }
            // Another tab than its own.
            (_, tab)
                if self
                    .scene
                    .screen()
                    .and_then(|s| s.tab())
                    .is_some_and(|own| own != tab) =>
            {
                match tab {
                    0 => self.show_songs(),
                    1 => self.open_settings(),
                    2 => self.open_multiplayer(),
                    3 => self.open_download(),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn click(&mut self, hit: hit::Hit, window: Option<&Window>) {
        if let Some(h) = self.scene.screen_mut::<HudEdit>() {
            let ed = &mut h.ed;
            match hit {
                hit::Hit::HudPanel => return,
                hit::Hit::HudRow(i) => return ed.select(i),
                hit::Hit::HudToggle(i) => return ed.toggle_part(i, &mut self.cfg),
                hit::Hit::HudPreset(i) => return ed.load_preset(i, &mut self.cfg),
                hit::Hit::HudColour(colour) => return ed.set_colour(colour, &mut self.cfg),
                hit::Hit::HudButton(b) => {
                    let outcome = ed.press_button(b, &mut self.cfg);
                    return self.hud_outcome(outcome);
                }
                _ => {}
            }
        }
        use hit::Hit;
        // The exit question takes only its own buttons.
        if self.quit_asked && !matches!(hit, Hit::Key(KeyCode::KeyY | KeyCode::KeyN)) {
            return;
        }
        if self.crash_shown() && !matches!(hit, Hit::Key(KeyCode::KeyC | KeyCode::Escape)) {
            return;
        }
        // Any click but the chat line itself puts the chat line away.
        if hit != Hit::LobbyChat
            && let Some(s) = self.mp.as_mut()
        {
            s.chat_input = None;
        }
        match hit {
            hit::Hit::HudPanel
            | hit::Hit::HudRow(_)
            | hit::Hit::HudToggle(_)
            | hit::Hit::HudPreset(_)
            | hit::Hit::HudColour(_)
            | hit::Hit::HudButton(_) => {}
            Hit::LobbyChat => self.focus_lobby_chat(),
            Hit::SkipIntro => {
                self.refresh_play_offset();
                if let Some(game) = self.play.as_deref_mut().filter(|g| g.show_intro_skip()) {
                    if self.scene.screen_is::<PlayScreen>() {
                        game.skip(sound(&self.audio), self.play_offset);
                    } else if self.scene.screen_is::<screens::mp_play::MpPlay>()
                        && let Some(s) = &mut self.mp
                    {
                        s.ask_to_skip();
                    }
                }
            }
            Hit::Key(code) => self.key_inner(code, false, None, Instant::now(), window),
            Hit::Tab(i) => self.goto_tab(i),
            Hit::MenuItem(i) => self.menu_click(i),
            Hit::MedalTab(tab) => {
                if let Some(view) = &mut self.medals {
                    view.set_tab(tab);
                }
            }
            Hit::Guide(action) => self.guide_action(action),
            Hit::EditorHome(act) => {
                let x = self.mouse_virtual().0;
                let out = match self.scene.screen_mut::<EditorHome>() {
                    Some(h) => h.home.click(act, x, self.audio.as_ref()),
                    _ => editor::home::Out::None,
                };
                self.home_out(out);
            }
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
            Hit::SelectLike => {
                if let Some(e) = self.preview_target().and_then(|i| self.library.get(i)) {
                    self.select.toggle_like(e);
                }
            }
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
            Hit::ChatShot(id) => self.open_shot(id),
            Hit::SendShot => {
                let row = self.social.row(self.mp.is_some());
                self.send_shot(row);
            }
            // Tapped: brings up the on-screen keyboard (`keyboard_tap`); the keys type there anyway.
            Hit::SocialPanel => {}
            Hit::TextField => self.social.typing = self.social.open,
            Hit::SocialTab(tab) => {
                self.social.set_tab(tab);
                self.social.mark_read(self.mp.is_some());
            }
            Hit::SocialRow(i) => {
                let in_lobby = self.mp.is_some();
                self.social.cursor = i;
                self.social.typing = matches!(
                    self.social.row(in_lobby),
                    social::Row::AddFriend | social::Row::Search
                );
                if matches!(
                    self.social.row(in_lobby),
                    social::Row::Friend(_) | social::Row::Lobby
                ) {
                    self.social.open_conversation();
                }
                self.social.mark_read(in_lobby);
                if self.social.row(in_lobby) == social::Row::Lobby
                    && let Some(s) = &mut self.mp
                {
                    s.chat_unread = 0;
                }
            }
            Hit::SocialBack => self.social.close_conversation(),
            Hit::SocialDeleteChat(i) => {
                if let Some(l) = self.social.global.get(i).cloned() {
                    self.admin_op(online::AdminOp::DeleteGlobal {
                        from: l.from,
                        at: l.at,
                    });
                }
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
                            self.social.close_conversation();
                            let in_lobby = self.mp.is_some();
                            if let Some(r) = self
                                .social
                                .rows(in_lobby)
                                .iter()
                                .position(|r| matches!(r, social::Row::Request(_)))
                            {
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
            Hit::Kick(id) => {
                if let Some(s) = &self.mp {
                    s.client.send(net::ClientMsg::Kick(id));
                    let name = s
                        .players
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    self.toast(tr!("{name} is out of the lobby", "{name} をロビーから外しました", "{name} убран из лобби";));
                }
            }
            Hit::GiveHost(id) => {
                if let Some(s) = &self.mp {
                    s.client.send(net::ClientMsg::GiveHost(id));
                    let name = s
                        .players
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    self.toast(format!("{name} is the host now"));
                }
            }
            Hit::BoardTab(online) => self.board_online = online,
            Hit::BoardPage => self.mob.flip_board(),
            Hit::RateOpen
            | Hit::RateClose
            | Hit::RateStep(_)
            | Hit::RateSet(_)
            | Hit::RateKeepPitch
            | Hit::PitchStep(_) => self.mobile_rate(hit),
            Hit::PlayerList => {
                if let Some(p) = &mut self.player {
                    p.list_open = !p.list_open;
                }
            }
            Hit::BoardWebsite => self.open_map_website(),
            Hit::Changelog => self.changelog = Some(changelog::View::recent(false)),
            Hit::Account => match self.mp_menu.account.clone() {
                Some(name) => self.open_profile(name),
                // Logging in is in the multiplayer tab.
                None => self.goto_tab(2),
            },
            Hit::Overlay | Hit::Preview => {}
            // Dragged (`mouse_button`).
            Hit::SplitList => {}
            Hit::ScoreVideo(k) => {
                if let Some(file) = self.leaderboard.get(k).cloned() {
                    self.open_video_editor(&file);
                }
            }
            Hit::DlSource(i) => self.browse.set_source(browse::Source::ALL[i]),
            Hit::DlMenu(0) => self.browse.toggle_menu(browse::Menu::Status),
            Hit::DlMenu(_) => self.browse.toggle_menu(browse::Menu::Sort),
            Hit::DlMenuClose => self.browse.menu = None,
            Hit::DlSortDir => self.browse.flip_sort(),
            Hit::DlPlay(i) => {
                self.browse.cursor = i;
                if let Some(set) = self.browse.selected().cloned() {
                    self.play_downloaded(&set);
                }
            }
            Hit::DlCancel(i) => {
                if let Some(job) = self.downloads.get(i) {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Hit::DlRetry(i) => {
                if i < self.browse.failed_jobs.len() {
                    let (set, _) = self.browse.failed_jobs.remove(i);
                    self.start_download(set);
                }
            }
            Hit::DlDismiss(i) => {
                if i < self.browse.failed_jobs.len() {
                    self.browse.failed_jobs.remove(i);
                }
            }
            Hit::DlCategory(i) => {
                if let Some(&c) = self.browse.categories().get(i) {
                    self.browse.set_category(c);
                }
            }
            Hit::DlSort(i) => {
                if let Some(&s) = self.browse.sorts().get(i) {
                    self.browse.set_sort(s);
                }
            }
            Hit::SettingsSection(i) => {
                self.settings_ui.binding = None;
                self.settings_ui.clear_search();
                self.settings_ui.cursor = settings::section_start(i);
            }
            Hit::SortList => self.open_sorts(),
            Hit::CollectionPick => self.open_collections(),
            Hit::PickRow(i) => self.pick_row(i),
            Hit::PickRowMore(i) => self.open_collection_menu(i.saturating_sub(1)),
            Hit::ChoiceRow(i) => self.pick_choice(i),
            Hit::SongsFolder => self.browse_songs_folder(),
            // Taken when pressed (`mouse_button`).
            Hit::SongScrollbar => {}
            Hit::DlRow(i) if self.browse.menu.is_some() => {
                let _ = i;
                self.browse.menu = None;
            }
            Hit::DlRow(i) if i == self.browse.cursor => {
                self.download_key(KeyCode::Enter, None, window)
            }
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
            Hit::ModPreset(k) => self.apply_mod_preset(k),
            Hit::ModPresetSave => self.start_preset_naming(),
            Hit::UniverseCell(row, col) => {
                self.universe_setup = Some(mod_setup::multiverse::Setup::new(row, col));
                let dir = if self.modifiers_shift { -1 } else { 1 };
                mod_setup::multiverse::Setup::change(&mut self.cfg.universes, row, col, dir);
                self.universe_action(multiverse::Action::Changed);
            }
            Hit::ShapeshiftCell(row, col) => {
                self.shapeshift_setup = Some(mod_setup::shapeshift::Setup { row, col });
                mod_setup::shapeshift::Setup::pick(&mut self.cfg.shapeshift, row, col);
                self.shapeshift_changed();
            }
            Hit::ShapeshiftMove(dir) => {
                if let Some(setup) = &mut self.shapeshift_setup
                    && setup.move_shape(&mut self.cfg.shapeshift, dir)
                {
                    self.shapeshift_changed();
                }
            }
            Hit::TactCell(col) => {
                self.tact_setup = Some(mod_setup::tact::Setup::new(col));
                self.tact_changed(col);
            }
            Hit::CustomTab(i) => {
                if let Some(&m) = mods::customizable(self.cfg.mods).get(i) {
                    self.open_customization(m);
                }
            }
            Hit::UniversePreset(i) => {
                let action = mod_setup::multiverse::Setup::load(
                    &mut self.cfg.universes,
                    &self.cfg.universe_presets,
                    i,
                );
                self.universe_action(action);
            }
            Hit::UniversePresetDelete(i) => {
                let action =
                    mod_setup::multiverse::Setup::delete(&mut self.cfg.universe_presets, i);
                self.universe_action(action);
            }
            Hit::UpdateSet(i) => self.update_set(i),
            Hit::SongMenu(k) => {
                let action = self
                    .select
                    .menu_items(&self.library)
                    .get(k)
                    .map(|(_, a)| *a);
                if let Some(ui::MenuAction::Love(set, loved, one)) = action {
                    let version = self
                        .select
                        .menu_map()
                        .filter(|_| one)
                        .map(|i| self.library[i].meta.version.clone());
                    self.select.menu = None;
                    self.toast(format!(
                        "{} {} on taiko.rs...",
                        if loved { "loving" } else { "unloving" },
                        match &version {
                            Some(v) => format!("[{v}]"),
                            None => format!("set {set}"),
                        }
                    ));
                    self.call_loved(LovedAsk::Change(set, loved, version));
                } else if let Some(ui::MenuAction::Collect(whole)) = action {
                    self.start_collecting(whole);
                } else if action == Some(ui::MenuAction::UpdateSong) {
                    if let Some(i) = self.select.menu_map() {
                        self.update_set(i);
                    }
                    self.select.menu = None;
                } else if action == Some(ui::MenuAction::Edit) {
                    let entry = self.select.menu_map();
                    self.select.menu = None;
                    if let Some(i) = entry {
                        self.open_editor(i);
                    }
                } else if action == Some(ui::MenuAction::NewDifficulty) {
                    let entry = self.select.menu_map();
                    self.select.menu = None;
                    if let Some(i) = entry {
                        self.new_difficulty(i);
                    }
                } else if action == Some(ui::MenuAction::Like) {
                    if let Some(e) = self.select.menu_map().and_then(|i| self.library.get(i)) {
                        let title = e.meta.title.clone();
                        let liked = self.select.toggle_like(e);
                        self.toast(if liked {
                            tr!("liked {title}", "{title}をお気に入りに追加", "в любимых: {title}";)
                        } else {
                            tr!("unliked {title}", "{title}をお気に入りから削除", "убрано из любимых: {title}";)
                        });
                    }
                    self.select.menu = None;
                } else if action == Some(ui::MenuAction::MapSkin) {
                    if let Some(e) = self.select.menu_map().and_then(|i| self.library.get(i)) {
                        let title = e.meta.title.clone();
                        let off = self.select.toggle_map_skin(e);
                        self.toast(if off {
                            tr!("{title}: your skin", "{title}: 自分のスキン", "{title}: ваш скин";)
                        } else {
                            tr!("{title}: its own skin", "{title}: 譜面のスキン", "{title}: скин карты";)
                        });
                    }
                    self.select.menu = None;
                } else if let Some(what) = action.and_then(|a| self.select.hide(a, &self.library)) {
                    self.toast(format!("hid {what}: Settings > Hidden maps brings it back"));
                }
            }
            // Like osu!: the first click selects, clicking the selected difficulty plays it.
            Hit::SongRow(row) if row == self.select.cursor() => {
                self.key_inner(KeyCode::Enter, false, None, Instant::now(), window)
            }
            // A song's own row picks its remembered (or hardest) difficulty.
            Hit::SongRow(row)
                if self.select.is_song_start(row) && !self.select.in_open_song(row) =>
            {
                self.select.enter_song(row, &self.library)
            }
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
                if let (true, Some(game)) = (self.scene.is_play(), self.play.as_deref_mut()) {
                    game.set_pause_cursor(i);
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

/// How long the quick retry key is held for the map to restart (ms).
/// How long the exit question takes to slide in.
const QUIT_SLIDE_MS: f32 = 160.0;
const RETRY_HOLD_MS: f64 = 150.0;
/// A replay's music made at another speed, and that speed.
type ReplayMusic = Result<(Arc<Track>, f64), String>;
/// The speeds a replay can be watched at, against how it was played.
const REPLAY_SPEEDS: [f64; 7] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// The capture name of an F12 screenshot (scripted runs name theirs).
const SCREENSHOT: &str = "screenshot";
