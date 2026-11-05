//! The editor's screen: the playfield preview, the timeline with the notes on it, the slider
//! velocity graph, and the inspector, with the mouse and keyboard handling that goes with them.
//!
//! It's drawn in the game's virtual units like every other screen and does its own hit testing
//! (the buttons it draws are remembered for the next click), so the rest of the client only has
//! to hand it the pointer, the wheel and the keys.

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use winit::keyboard::KeyCode;

use super::chart::{HitObject, Taiko};
use super::state::{Editor, Tool};
use super::timing::{self, SNAP_DIVISORS};
use super::tools::{GimmickTool, Target};
use crate::audio::{Engine, Sfx, Track};
use crate::beatmap::{self, OSU_TIMING_MS};
use crate::game::Game;
use crate::render::{Canvas, accent, rgba};
use crate::video::FRAME_VW;

mod chrome;
mod cmd;
mod curves;
mod help;
mod keys;
mod lines;
mod manual;
mod manual_text;
mod menu;
mod nodes;
mod panel;
mod run;
mod timing_win;
mod upload;

use cmd::Cmd;
pub use cmd::Mode;
use curves::{SAct, SF};
use keys::KAct;
use lines::TAct;
use nodes::NAct;
use panel::{GFlag, GParam};

pub type TrackResult = Result<Arc<Track>, String>;

/// The modifier keys held.
#[derive(Clone, Copy, Default)]
pub struct Keys {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

pub enum KeyResult {
    Handled,
    /// Not the editor's: the game's own keys (volume, screenshots...) get it.
    Pass,
    Exit,
}

/// Something typed into a value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Bpm,
    Sv,
    Volume,
    Meter,
    Time,
    /// The sample set (cycles Auto, Normal, Soft, Drum) and the custom sample index.
    Sample,
    Index,
    /// A number of the SV tool.
    S(SF),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Btn {
    Resnap,
    AddRed,
    AddGreen,
    Kiai,
    Omit,
    Dec(Field),
    Inc(Field),
    Edit(Field),
    GPage(u8),
    GTool(GimmickTool),
    GStep(GParam, i32),
    GFlag(GFlag),
    GApply(Target),
    GPreview,
    GKeep,
    GCancel,
    GBase,
    K(KAct),
    N(NAct),
    T(TAct),
    /// The SV tool and the velocity graph.
    S(SAct),
    /// A command by name (the same ones the keys and the menus run).
    C(Cmd),
    /// The upload window.
    U(upload::UpAct),
    /// The manual.
    Man(manual::ManAct),
    /// Opens or closes the menu at this position of the bar.
    Menu(usize),
    TourNext,
    TourBack,
    TourSkip,
    /// A row of the timing window.
    Line(u64),
    /// The window's filter chip.
    TwFilter(u8),
    /// The window's scroll bar track (a press starts the drag).
    TwBar,
}

#[derive(Clone, Copy)]
enum Drag {
    None,
    Scrub,
    /// A box over the notes, in map ms (so it stays right while the view scrolls under it).
    Box {
        from: f64,
        to: f64,
        down_x: f32,
        /// The pointer has left the press by more than a click's worth.
        moved: bool,
        /// Shift: keep what was selected.
        add: bool,
    },
    Move {
        anchor: u64,
        grab: f64,
    },
    Resize {
        id: u64,
    },
    Place {
        start: f64,
        swell: bool,
        from_x: f32,
    },
    /// Middle button: the timeline follows the pointer.
    Slide {
        from_x: f32,
        start: f64,
    },
    /// Dragging along the whole-song bar.
    Overview,
    /// The timing window's scroll bar.
    TwScroll,
    Lines {
        anchor: u64,
    },
    /// A box dragged over the velocity graph.
    GBox {
        from: (f32, f32),
        to: (f32, f32),
    },
    Key {
        index: usize,
    },
    Node {
        id: u32,
        dx: f32,
        dy: f32,
    },
    Wire {
        from: u32,
    },
    Pan {
        last: (f32, f32),
    },
    /// A curve drawn across the graph, in map ms and the graph's unit.
    Brush {
        t0: f64,
        v0: f64,
        t1: f64,
        v1: f64,
    },
    /// The end of the picked curve being dragged (false: its start).
    Handle {
        end: bool,
    },
}

/// The tools, in the order of their keys.
const TOOLS: [(&str, &str, Tool); 4] = [
    ("1", "Select", Tool::Select),
    ("2", "Note", Tool::Note),
    ("3", "Roll", Tool::Roll),
    ("4", "Denden", Tool::Swell),
];

const RATES: [u32; 4] = [25, 50, 75, 100];

const NOTE_R: f32 = 12.0;
const BIG_R: f32 = 16.0;
const DON: u32 = 0xEB452C;
const KAT: u32 = 0x438EAD;
const ROLL: u32 = 0xFCB806;
const SWELL: u32 = 0xF58A2A;
const RED_LINE: u32 = 0xE2463C;
const GREEN_LINE: u32 = 0x4FD16B;

/// Where the pieces of the screen go.
#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub menu: [f32; 4],
    pub modes: [f32; 4],
    pub preview: [f32; 4],
    /// The panel beside the preview that belongs to the mode.
    pub inspector: [f32; 4],
    /// The notes' lane (the ruler is its top strip); the tool palette sits left of it.
    pub timeline: [f32; 4],
    pub palette: [f32; 4],
    pub graph: [f32; 4],
    /// The node canvas and the strip above it, which take the timeline's place on the Nodes page.
    pub nodes: [f32; 4],
    pub strip: [f32; 4],
    /// The whole song, small, under everything: where you are and where the notes are.
    pub overview: [f32; 4],
    pub status: [f32; 4],
    /// The F6 timing window (empty while it is closed).
    pub window: [f32; 4],
}

/// The colour of effect marks (.tko).
pub(super) const MARK_COL: u32 = 0xB36BFF;
/// How tall the ruler across the top of the timeline is.
const RULER: f32 = 14.0;

impl Layout {
    pub fn new(w: f32, mode: Mode, windowed: bool) -> Layout {
        let ph = 158.0;
        let pw = ph * 16.0 / 9.0;
        let top = 44.0;
        let low = top + ph + 6.0;
        // The velocity graph is the thing Timing mode is about, so it gets the room.
        let (tl_h, gap) = if mode == Mode::Timing {
            (96.0, 4.0)
        } else {
            (146.0, 4.0)
        };
        let graph_y = low + tl_h + gap;
        let graph_h = 430.0 - graph_y;
        if windowed {
            // The window takes the preview, the panel and the graph; the timeline stays under it.
            let low = 322.0;
            return Layout {
                menu: [0.0, 0.0, w, 14.0],
                modes: [8.0, 17.0, w - 16.0, 22.0],
                window: [8.0, top, w - 16.0, low - top - 6.0],
                palette: [8.0, low, 30.0, 108.0],
                timeline: [42.0, low, w - 50.0, 108.0],
                overview: [8.0, 434.0, w - 16.0, 9.0],
                status: [8.0, 447.0, w - 16.0, 30.0],
                ..Layout::default()
            };
        }
        Layout {
            menu: [0.0, 0.0, w, 14.0],
            modes: [8.0, 17.0, w - 16.0, 22.0],
            preview: [8.0, top, pw, ph],
            inspector: [pw + 16.0, top, (w - pw - 24.0).max(120.0), ph],
            palette: [8.0, low, 30.0, tl_h],
            timeline: [42.0, low, w - 50.0, tl_h],
            graph: [8.0, graph_y, w - 16.0, graph_h],
            strip: [8.0, low, w - 16.0, 20.0],
            nodes: [8.0, low + 24.0, w - 16.0, 430.0 - low - 24.0],
            overview: [8.0, 434.0, w - 16.0, 9.0],
            status: [8.0, 447.0, w - 16.0, 30.0],
            window: [0.0; 4],
        }
    }
}

/// What an open modal question is asking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Modal {
    /// Leaving with unsaved changes.
    Leave,
    /// Sending the map to taiko.rs.
    Upload,
    /// The manual.
    Manual,
}

pub struct View {
    pub ed: Editor,
    /// The library entry being edited (its music is the track).
    pub entry: usize,
    pub track: Option<Arc<Track>>,
    track_rx: Option<Receiver<TrackResult>>,
    /// Sample rate the engine plays at, to know how long the track is.
    audio_rate: f64,
    pub rate: u32,
    pub want_rate: Option<f64>,
    game: Option<Box<Game>>,
    game_err: Option<String>,
    /// The chart changed at this moment and the preview hasn't caught up.
    stale: Option<Instant>,
    /// Map ms the preview was last set to.
    shown: f64,
    playing: Option<(Instant, f64)>,
    last_played: f64,
    /// Left edge of the timeline (ms) and how many ms one unit of width holds.
    pub view_start: f64,
    pub ms_per_px: f64,
    drag: Drag,
    buttons: Vec<([f32; 4], Btn)>,
    pub layout: Layout,
    pub status: String,
    pub sel_line: Option<u64>,
    typing: Option<(Field, String)>,
    mouse: (f32, f32),
    pub width: f32,
    /// What the middle of the screen is for.
    pub mode: Mode,
    /// Gimmicks mode: 0 structures, 1 generate SV. Effects mode: 2 keyframes, 3 nodes.
    g_page: u8,
    g_tool: GimmickTool,
    /// A gimmick was applied as a preview and can still be cancelled.
    g_pending: bool,
    /// Keyframe page: editing the playfield's tracks (else a group's), which property, which key.
    k_field: bool,
    k_group: usize,
    k_prop: usize,
    k_sel: Option<usize>,
    k_ease: crate::tko::track::Ease,
    /// The graph's value range while a key is being dragged, so it doesn't rescale under the pointer.
    k_frozen: Option<(f64, f64)>,
    /// Nodes page: the selected node, the canvas's pan and zoom, the palette's category, the clip
    /// preset last given.
    n_sel: Option<u32>,
    n_pan: (f32, f32),
    n_zoom: f32,
    n_cat: usize,
    /// A dropdown over the node canvas (targets, recipes), and which group it lists.
    n_pick: Option<nodes::Pick>,
    n_pick_group: usize,
    /// A number being typed into parameter `.0` of the selected node.
    n_typing: Option<(usize, String)>,
    /// Which group of playfield properties the Keyframes page lists.
    k_tab: usize,
    n_clip: usize,
    /// The menu bar's open menu.
    menu: Option<usize>,
    /// A question that has to be answered before anything else.
    modal: Option<Modal>,
    /// The upload window's state.
    upload: upload::UploadUi,
    /// The manual's chapter and scroll.
    manual: manual::ManualUi,
    /// The game's token, when logged in to taiko.rs (set by the screen).
    pub account: Option<String>,
    /// Set when the editor should close (the screen asks `take_exit`).
    exit: bool,
    /// The shortcut sheet is open.
    sheet: bool,
    /// The F6 timing window is open: which lines it lists (0 all, 1 red, 2 green), how far down it
    /// is scrolled, the selection it last scrolled to and its scroll bar's track.
    tw: bool,
    tw_filter: u8,
    tw_scroll: usize,
    tw_seen: Option<u64>,
    tw_bar: [f32; 4],
    /// The step of the tour being shown.
    tour: Option<usize>,
    /// The tour was finished or skipped for good (the screen saves that once).
    tour_ended: bool,
    /// What the pointer is over, for tooltips: (area, text), rebuilt every frame.
    tips: Vec<([f32; 4], String)>,
    /// When the pointer last moved.
    moved_at: Instant,
    /// Buttons before this index sit under the overlay on top and don't take clicks.
    modal_from: usize,
    /// A roll or swell whose start was clicked and whose end is still to be.
    roll_start: Option<f64>,
    /// The last click on a note, for double clicks.
    last_click: Option<(Instant, u64)>,
    /// The last click on empty graph, for the double click that adds a line.
    g_click: Option<(Instant, (f32, f32))>,
    /// Where the press that started dragging graph dots was, to tell a click from a drag.
    g_down: (f32, f32),
    revert_armed: bool,
    /// The last status message drawn and when it appeared (it gives way to the hint after a while).
    status_seen: String,
    status_at: Instant,
    /// Where the open sheet or list sits, so a click outside it can close it.
    overlay_rect: [f32; 4],
    /// Timing lines selected besides `sel_line` (see `selected_lines`).
    sel_extra: std::collections::BTreeSet<u64>,
    /// What the selected lines were (id, time, velocity) when a drag of them began.
    line_origin: Vec<(u64, f64, f64)>,
    /// The graph's range, held still while its dots are dragged.
    g_frozen: Option<(f64, f64)>,
    /// The Timing panel's tools: the velocity to set and the factor to multiply by.
    t_value: f64,
    /// The modifier keys as of the last pointer event, for the scrolling that carries on without one.
    held: Keys,
    svt: curves::SvtUi,
    /// The last `update`, for how long the view scrolls at an edge.
    last_tick: Option<Instant>,
}

fn fmt_time(ms: f64) -> String {
    let neg = ms < 0.0;
    let ms = ms.abs().round() as u64;
    format!(
        "{}{}:{:02}.{:03}",
        if neg { "-" } else { "" },
        ms / 60_000,
        ms / 1000 % 60,
        ms % 1000
    )
}

fn inside(r: [f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]
}

impl View {
    pub fn open(path: std::path::PathBuf, entry: usize) -> Result<View, String> {
        let ed = Editor::open(path)?;
        let mut v = View {
            ed,
            entry,
            track: None,
            track_rx: None,
            audio_rate: 48000.0,
            rate: 100,
            want_rate: None,
            game: None,
            game_err: None,
            stale: Some(Instant::now()),
            shown: f64::NAN,
            playing: None,
            last_played: 0.0,
            view_start: 0.0,
            ms_per_px: 4.0,
            drag: Drag::None,
            buttons: Vec::new(),
            layout: Layout::new(853.0, Mode::Compose, false),
            status: String::new(),
            sel_line: None,
            typing: None,
            mouse: (0.0, 0.0),
            width: 853.0,
            mode: Mode::Compose,
            g_page: 0,
            g_tool: GimmickTool::BarlineNote,
            g_pending: false,
            k_field: true,
            k_group: 0,
            k_prop: 0,
            k_sel: None,
            k_ease: crate::tko::track::Ease::Linear,
            k_frozen: None,
            n_sel: None,
            n_pan: (0.0, 0.0),
            n_zoom: 1.0,
            n_cat: 0,
            n_pick: None,
            n_pick_group: 0,
            n_typing: None,
            k_tab: 0,
            n_clip: 0,
            menu: None,
            modal: None,
            upload: upload::UploadUi::default(),
            manual: manual::ManualUi::default(),
            account: None,
            exit: false,
            sheet: false,
            tw: false,
            tw_filter: 0,
            tw_scroll: 0,
            tw_seen: None,
            tw_bar: [0.0; 4],
            tour: None,
            tour_ended: false,
            tips: Vec::new(),
            moved_at: Instant::now(),
            modal_from: 0,
            roll_start: None,
            last_click: None,
            g_click: None,
            g_down: (0.0, 0.0),
            revert_armed: false,
            status_seen: String::new(),
            status_at: Instant::now(),
            overlay_rect: [0.0; 4],
            sel_extra: Default::default(),
            line_origin: Vec::new(),
            g_frozen: None,
            t_value: 1.0,
            held: Keys::default(),
            svt: curves::SvtUi::new(),
            last_tick: None,
        };
        v.follow(true);
        Ok(v)
    }

    pub fn label(&self) -> String {
        let c = &self.ed.chart;
        format!(
            "{} - {} [{}]",
            c.get_key("Metadata", "Artist").unwrap_or_default(),
            c.get_key("Metadata", "Title").unwrap_or_default(),
            c.get_key("Metadata", "Version").unwrap_or_default()
        )
    }

    pub fn set_track_loader(&mut self, rx: Receiver<TrackResult>, audio_rate: f64) {
        self.track_rx = Some(rx);
        self.audio_rate = audio_rate;
    }

    pub fn take_rate_request(&mut self) -> Option<f64> {
        self.want_rate.take()
    }

    fn speed(&self) -> f64 {
        self.rate as f64 / 100.0
    }

    // ---- time ----

    pub fn set_time(&mut self, t: f64) {
        self.ed.time = t.max(0.0).round();
        self.follow(false);
    }

    fn timeline_w(&self) -> f32 {
        self.layout.timeline[2]
    }

    /// Keeps the playhead on screen: while playing it sits a third in, off screen it is brought
    /// back to there.
    fn follow(&mut self, force: bool) {
        let w = self.timeline_w() as f64 * self.ms_per_px;
        let t = self.ed.time;
        if force || self.playing.is_some() || t < self.view_start || t > self.view_start + w * 0.96
        {
            self.view_start = t - w * 0.3;
        }
    }

    fn x_of(&self, t: f64) -> f32 {
        self.layout.timeline[0] + ((t - self.view_start) / self.ms_per_px) as f32
    }

    fn t_at(&self, x: f32) -> f64 {
        self.view_start + (x - self.layout.timeline[0]) as f64 * self.ms_per_px
    }

    fn view_end(&self) -> f64 {
        self.view_start + self.timeline_w() as f64 * self.ms_per_px
    }

    fn zoom(&mut self, factor: f64, around_x: f32) {
        let t = self.t_at(around_x);
        self.ms_per_px = (self.ms_per_px * factor).clamp(0.25, 80.0);
        self.view_start = t - (around_x - self.layout.timeline[0]) as f64 * self.ms_per_px;
    }

    /// The track's length in map ms, or a little past the last object.
    pub fn end_ms(&self) -> f64 {
        match &self.track {
            Some(t) if t.frames() > 0 => t.frames() as f64 / self.audio_rate * 1000.0 * t.rate,
            _ => {
                let last = self
                    .ed
                    .chart
                    .objects
                    .iter()
                    .map(|o| self.ed.end_of(o))
                    .fold(0.0, f64::max);
                last + 10_000.0
            }
        }
    }

    // ---- loading and playing ----

    pub fn update(&mut self, now: Instant, audio: Option<&Engine>) {
        self.edge_scroll(now);
        self.poll_upload();
        if let Some(rx) = &self.track_rx
            && let Ok(result) = rx.try_recv()
        {
            self.track_rx = None;
            match result {
                Ok(t) => {
                    self.track = Some(t);
                    self.stale = Some(now);
                }
                Err(e) => self.status = format!("no music: {e}"),
            }
        }
        if let Some((started, from)) = self.playing {
            let t = (from + started.elapsed().as_secs_f64() * 1000.0 * self.speed()).round();
            if t >= self.end_ms() {
                self.stop(audio);
            } else {
                if let Some(a) = audio {
                    // Each note the playhead passes rings once, as a hit would.
                    let (from_t, to_t) = (self.last_played, t);
                    let first = self.ed.chart.objects.partition_point(|o| o.time <= from_t);
                    for o in self.ed.chart.objects[first..]
                        .iter()
                        .take_while(|o| o.time <= to_t)
                        .take(6)
                    {
                        a.sfx(match (o.taiko(), o.big()) {
                            (Taiko::Kat, false) => Sfx::Kat,
                            (Taiko::Kat, true) => Sfx::BigKat,
                            (_, true) => Sfx::BigDon,
                            _ => Sfx::Don,
                        });
                    }
                }
                self.last_played = t;
                self.ed.time = t;
                self.follow(false);
            }
        }
    }

    fn start(&mut self, audio: Option<&Engine>) {
        let (Some(track), Some(a)) = (&self.track, audio) else {
            self.status = "the music isn't loaded".into();
            return;
        };
        a.play(
            track.clone(),
            (self.ed.time + OSU_TIMING_MS) / self.speed().max(0.01),
        );
        self.playing = Some((Instant::now(), self.ed.time));
        self.last_played = self.ed.time;
    }

    fn stop(&mut self, audio: Option<&Engine>) {
        if self.playing.take().is_some()
            && let Some(a) = audio
        {
            a.stop();
        }
    }

    fn toggle_play(&mut self, audio: Option<&Engine>) {
        if self.playing.is_some() {
            self.stop(audio);
        } else {
            self.start(audio);
        }
    }

    // ---- the preview ----

    fn mark_stale(&mut self) {
        self.stale.get_or_insert_with(Instant::now);
    }

    /// Rebuilds the preview from the chart as it is now (at most every so often during a drag,
    /// since converting a big map takes a moment).
    fn refresh_game(&mut self) {
        let Some(since) = self.stale else {
            return;
        };
        let dragging = !matches!(self.drag, Drag::None);
        if dragging && since.elapsed().as_millis() < 150 {
            return;
        }
        self.stale = None;
        match beatmap::from_text(&self.ed.chart.to_text()) {
            Ok(map) => {
                let track = self.track.clone().unwrap_or_else(|| {
                    Arc::new(Track {
                        samples: Vec::new(),
                        rate: 1.0,
                    })
                });
                let mut g = Game::new(map, track, None);
                g.mute_hits = true;
                self.game = Some(Box::new(g));
                self.game_err = None;
                self.shown = f64::NAN;
            }
            Err(e) => {
                self.game = None;
                self.game_err = Some(e);
            }
        }
    }

    fn sync_game(&mut self, autoplay: crate::autoplay::Settings) {
        let t = self.ed.time + OSU_TIMING_MS;
        if self.shown == t {
            return;
        }
        let backwards = self.shown.is_nan() || t < self.shown;
        if let Some(g) = &mut self.game {
            if backwards && !self.shown.is_nan() {
                // Notes already played stay played going forwards only: a seek back starts over.
                let map = g.map.clone();
                let track = g.track.clone();
                **g = Game::new(map, track, None);
                g.mute_hits = true;
            }
            g.skip_to(t - 50.0);
            g.set_time(t);
            g.autoplay_style = autoplay;
            g.autoplay(None);
        }
        self.shown = t;
    }

    // ---- drawing ----

    pub fn draw(&mut self, c: &mut Canvas, scroll_speed: f64, autoplay: crate::autoplay::Settings) {
        self.width = c.width;
        self.layout = Layout::new(c.width, self.mode, self.windowed());
        self.buttons.clear();
        self.tips.clear();
        self.svt.live = false;
        self.modal_from = 0;
        self.refresh_game();
        self.sync_game(autoplay);
        if self.windowed() {
            self.draw_palette(c);
            self.draw_timeline(c);
            self.draw_timing_window(c);
        } else if self.node_mode() {
            self.draw_preview(c, scroll_speed);
            self.draw_gimmicks(c);
            self.draw_node_strip(c);
            self.draw_node_canvas(c);
        } else {
            self.draw_preview(c, scroll_speed);
            match self.mode {
                Mode::Compose => self.draw_compose_panel(c),
                Mode::Timing => self.draw_timing_panel(c),
                Mode::Gimmicks | Mode::Effects => self.draw_gimmicks(c),
            }
            self.draw_palette(c);
            self.draw_timeline(c);
            self.draw_graph(c);
        }
        self.draw_overview(c);
        self.draw_status(c);
        self.draw_modebar(c);
        self.draw_menubar(c);
        self.draw_overlays(c);
    }

    fn button(&mut self, c: &mut Canvas, [x, y, w, h]: [f32; 4], label: &str, b: Btn, on: bool) {
        let hover = c.hovered(x, y, w, h);
        let (bg, border) = match (on, hover) {
            (true, _) => (0x4a221b, accent()),
            (false, true) => (0x34344a, 0x6a6a88),
            (false, false) => (0x23232f, 0x3a3a4e),
        };
        c.round_rect([x, y], [w, h], 3.0, rgba(bg, 1.0), rgba(border, 1.0), 1.0);
        let size = 7.0;
        c.text_centered(
            x + w / 2.0,
            y + (h - size) / 2.0,
            size,
            rgba(0xffffff, 1.0),
            &Canvas::fit(size, w - 4.0, label),
        );
        self.buttons.push(([x, y, w, h], b));
        if let Btn::C(cmd) = b {
            self.tips.push(([x, y, w, h], cmd.tip()));
        }
    }

    /// A row of buttons from `x`, each as wide as its text. Returns where it ended.
    fn row(&mut self, c: &mut Canvas, mut x: f32, y: f32, items: &[(String, Btn, bool)]) -> f32 {
        for (label, b, on) in items {
            let w = Canvas::text_width(7.0, label) + 12.0;
            self.button(c, [x, y, w, 16.0], label, *b, *on);
            x += w + 4.0;
        }
        x
    }

    /// Like `row`, but a button that would pass `x + w` starts a new line. Returns the next y.
    fn wrap_row(
        &mut self,
        c: &mut Canvas,
        x0: f32,
        mut y: f32,
        w: f32,
        items: &[(String, Btn, bool)],
    ) -> f32 {
        let mut x = x0;
        for (label, b, on) in items {
            let bw = Canvas::text_width(7.0, label) + 12.0;
            if x > x0 && x + bw > x0 + w {
                x = x0;
                y += 19.0;
            }
            self.button(c, [x, y, bw, 16.0], label, *b, *on);
            x += bw + 4.0;
        }
        y + 19.0
    }

    fn draw_preview(&mut self, c: &mut Canvas, scroll_speed: f64) {
        let [fx, fy, fw, fh] = self.layout.preview;
        c.rect(fx, fy, fw, fh, rgba(0x0c0c12, 0.7));
        if let Some(g) = &mut self.game {
            let start = c.shapes.len();
            let clips = c.clips.len();
            let full = c.width;
            c.width = FRAME_VW;
            g.draw_playfield(c, scroll_speed);
            c.width = full;
            c.transform_since(start, [0.0, 0.0], fw / FRAME_VW, fx, fy);
            // Clips the playfield made for itself (a glitch's slices) move with its shapes.
            let k = fw / FRAME_VW;
            for (_, [x, y, w, h]) in &mut c.clips[clips..] {
                (*x, *y, *w, *h) = (*x * k + fx, *y * k + fy, *w * k, *h * k);
            }
            c.clip_since(start, fx, fy, fw, fh);
        } else {
            let why = match &self.game_err {
                Some(e) => e.clone(),
                None => "loading...".to_string(),
            };
            c.text_centered(
                fx + fw / 2.0,
                fy + fh / 2.0 - 4.0,
                8.0,
                rgba(0x9a9ab0, 1.0),
                &Canvas::fit(8.0, fw - 16.0, &why),
            );
        }
        c.round_rect(
            [fx, fy],
            [fw, fh],
            2.0,
            rgba(0, 0.0),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
    }

    fn selected_objects(&self) -> Vec<&HitObject> {
        self.ed
            .chart
            .objects
            .iter()
            .filter(|o| self.ed.selection.contains(&o.id))
            .collect()
    }

    /// Grid lines in view: (time, 0 sub-beat / 1 beat / 2 bar).
    fn grid(&self) -> Vec<(f64, u8)> {
        let (from, to) = (self.view_start, self.view_end());
        let points = &self.ed.chart.timing;
        let reds: Vec<_> = points
            .iter()
            .filter(|p| p.is_red() && p.beat_length > 0.0 && p.beat_length.is_finite())
            .collect();
        let snap = self.ed.snap.max(1) as f64;
        let mut out = Vec::new();
        for (i, p) in reds.iter().enumerate() {
            let end = reds.get(i + 1).map_or(f64::INFINITY, |n| n.time);
            if end < from || p.time > to {
                continue;
            }
            let step = p.beat_length / snap;
            if step / self.ms_per_px < 3.0 {
                continue;
            }
            let start = from.max(p.time);
            let mut k = ((start - p.time) / step).ceil() as i64;
            while out.len() < 4000 {
                let t = p.time + k as f64 * step;
                if t >= to.min(end) {
                    break;
                }
                let beat = k % self.ed.snap.max(1) as i64 == 0;
                let beats = k / self.ed.snap.max(1) as i64;
                let bar = beat && beats % p.meter.max(1) as i64 == 0;
                out.push((t, if bar { 2 } else { beat as u8 }));
                k += 1;
            }
        }
        out
    }

    /// The tools, in a column beside the timeline.
    fn draw_palette(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.palette;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x15151d, 0.95),
            rgba(0x2c2c3c, 1.0),
            1.0,
        );
        let bh = (h - 4.0) / TOOLS.len() as f32;
        for (i, (key, name, tool)) in TOOLS.iter().enumerate() {
            let r = [x + 2.0, y + 2.0 + i as f32 * bh, w - 4.0, bh - 2.0];
            let on = self.ed.tool == *tool;
            let hover = c.hovered(r[0], r[1], r[2], r[3]);
            c.round_rect(
                [r[0], r[1]],
                [r[2], r[3]],
                3.0,
                rgba(
                    if on {
                        0x4a221b
                    } else if hover {
                        0x34344a
                    } else {
                        0x1d1d28
                    },
                    1.0,
                ),
                rgba(if on { accent() } else { 0x3a3a4e }, 1.0),
                1.0,
            );
            let tall = r[3] >= 26.0;
            c.text_centered(
                r[0] + r[2] / 2.0,
                r[1] + if tall { 4.0 } else { (r[3] - 8.0) / 2.0 },
                8.0,
                rgba(0xffffff, 1.0),
                key,
            );
            if tall {
                c.text_centered(
                    r[0] + r[2] / 2.0,
                    r[1] + r[3] - 10.0,
                    5.0,
                    rgba(0xb0b0c4, 1.0),
                    name,
                );
            }
            self.buttons.push((r, Btn::C(Cmd::Tool(*tool))));
        }
    }

    fn draw_timeline(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.timeline;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x101016, 0.97),
            rgba(0x2c2c3c, 1.0),
            1.0,
        );
        let start = c.shapes.len();
        let (from, to) = (self.view_start, self.view_end());
        let lane_top = y + RULER + 16.0;
        let cy = lane_top + (y + h - lane_top) / 2.0;
        // The ruler.
        c.rect(x + 1.0, y + 1.0, w - 2.0, RULER - 1.0, rgba(0x1a1a26, 1.0));
        // Kiai.
        let pts = &self.ed.chart.timing;
        let first = pts.partition_point(|p| p.time <= from).saturating_sub(1);
        let mut kiai_from: Option<f64> = None;
        let mut spans: Vec<(f64, f64)> = Vec::new();
        for (i, p) in pts.iter().enumerate().skip(first) {
            if p.time > to && kiai_from.is_none() {
                break;
            }
            match (p.kiai(), kiai_from) {
                (true, None) => {
                    kiai_from = Some(p.time.max(if i == first { from } else { p.time }))
                }
                (false, Some(s)) => {
                    spans.push((s, p.time));
                    kiai_from = None;
                }
                _ => {}
            }
            if p.time > to {
                break;
            }
        }
        if let Some(s) = kiai_from {
            spans.push((s, to));
        }
        for (s, e) in spans {
            let (sx, ex) = (self.x_of(s).max(x), self.x_of(e).min(x + w));
            if ex > sx {
                c.rect(
                    sx,
                    y + RULER,
                    ex - sx,
                    h - RULER - 1.0,
                    rgba(0xFFAA00, 0.09),
                );
            }
        }
        // Grid: bars and beats reach into the ruler.
        for (t, kind) in self.grid() {
            let gx = self.x_of(t);
            let (alpha, top) = match kind {
                2 => (0.6, y + RULER - 8.0),
                1 => (0.32, y + RULER - 4.0),
                _ => (0.13, lane_top),
            };
            c.rect(gx, top, 1.0, y + h - top - 1.0, rgba(0xffffff, alpha));
        }
        // Ruler labels.
        let label_every = {
            let mut e = 1000.0;
            while e / self.ms_per_px < 90.0 {
                e *= 2.0;
            }
            e
        };
        let mut lt = (from / label_every).ceil() * label_every;
        while lt < to {
            let lx = self.x_of(lt);
            c.rect(lx, y + 1.0, 1.0, 5.0, rgba(0xffffff, 0.5));
            c.text(lx + 3.0, y + 3.0, 5.5, rgba(0x9a9ab0, 1.0), &fmt_time(lt));
            lt += label_every;
        }
        // Timing lines, in a strip of their own under the ruler.
        c.rect(x + 1.0, y + RULER, w - 2.0, 16.0, rgba(0x14141c, 0.9));
        let first = pts.partition_point(|p| p.time < from - 2000.0);
        for p in pts[first..].iter().take_while(|p| p.time <= to + 2000.0) {
            let lx = self.x_of(p.time);
            let selected = self.selected_lines().contains(&p.id);
            let col = if p.is_red() { RED_LINE } else { GREEN_LINE };
            c.rect(
                lx - if selected { 1.0 } else { 0.5 },
                y + RULER + 1.0,
                if selected { 2.0 } else { 1.0 },
                14.0,
                rgba(if selected { 0xffffff } else { col }, 1.0),
            );
            if lx > x && lx < x + w - 40.0 && self.ms_per_px < 12.0 {
                let label = match p.bpm() {
                    Some(b) => format!("{b:.1} BPM"),
                    None => format!("x{:.2}", p.sv()),
                };
                c.text(lx + 3.0, y + RULER + 5.0, 5.5, rgba(col, 1.0), &label);
            }
        }
        // Bookmarks.
        for b in &self.ed.bookmarks {
            if *b < from || *b > to {
                continue;
            }
            let bx = self.x_of(*b);
            c.rect(
                bx - 0.5,
                lane_top,
                1.0,
                y + h - lane_top - 1.0,
                rgba(0x4f9dff, 0.5),
            );
            c.rect(bx - 0.5, y + 1.0, 6.0, 5.0, rgba(0x4f9dff, 1.0));
        }
        // Effect marks (.tko).
        let marks: Vec<f64> = self
            .ed
            .gimmick()
            .map_or_else(Vec::new, |g| g.marks.iter().map(|m| m.time).collect());
        for (i, m) in marks.iter().enumerate() {
            if *m < from || *m > to {
                continue;
            }
            let mx = self.x_of(*m);
            c.rect(
                mx - 0.5,
                lane_top,
                1.0,
                y + h - lane_top - 1.0,
                rgba(MARK_COL, 0.6),
            );
            c.rect(mx - 0.5, y + 1.0, 14.0, 7.0, rgba(MARK_COL, 1.0));
            c.text(
                mx + 2.0,
                y + 1.5,
                5.5,
                rgba(0xffffff, 1.0),
                &format!("{}", i + 1),
            );
        }
        // Notes.
        let objs = &self.ed.chart.objects;
        let first = objs.partition_point(|o| o.time < from - 60_000.0);
        let hovered = if inside(self.layout.timeline, self.mouse.0, self.mouse.1)
            && self.mouse.1 > lane_top - 16.0
            && matches!(self.drag, Drag::None)
        {
            self.object_at_point(self.mouse.0, self.mouse.1)
        } else {
            None
        };
        // While a box is being dragged, what it covers shows as selected already.
        let boxed = match self.drag {
            Drag::Box {
                from,
                to,
                moved: true,
                add,
                ..
            } => Some((from.min(to), from.max(to), add)),
            _ => None,
        };
        for o in &objs[first..] {
            if o.time > to + 40.0 * self.ms_per_px {
                break;
            }
            let end = self.ed.end_of(o);
            if end < from - 20.0 * self.ms_per_px {
                continue;
            }
            let (ox, ex) = (self.x_of(o.time), self.x_of(end));
            let selected = match boxed {
                Some((a, b, add)) => {
                    (o.time >= a && o.time <= b) || (add && self.ed.selection.contains(&o.id))
                }
                None => self.ed.selection.contains(&o.id),
            };
            let r = if o.big() { BIG_R } else { NOTE_R };
            let outline = if selected {
                rgba(0xffffff, 1.0)
            } else if hovered == Some(o.id) {
                rgba(0xffe9a8, 1.0)
            } else {
                rgba(0x000000, 0.8)
            };
            let lw = if selected || hovered == Some(o.id) {
                2.0
            } else {
                1.0
            };
            match o.taiko() {
                Taiko::Roll | Taiko::Swell if ex > ox => {
                    let col = if o.taiko() == Taiko::Roll {
                        ROLL
                    } else {
                        SWELL
                    };
                    c.capsule([ox, ex], cy, r * 0.8, rgba(col, 0.9), outline, lw);
                    c.circle(ox, cy, r, rgba(col, 1.0), outline, lw);
                    if selected {
                        // The handle that stretches it.
                        c.rect(ex - 1.5, cy - r * 0.8, 3.0, r * 1.6, rgba(0xffffff, 0.9));
                    }
                }
                _ => {
                    let col = if o.taiko() == Taiko::Kat { KAT } else { DON };
                    c.circle(ox, cy, r, rgba(col, 1.0), outline, lw);
                }
            }
        }
        // The ghost of what a click would place.
        let (mx, my) = self.mouse;
        let in_lane = inside(self.layout.timeline, mx, my) && my > y + RULER + 16.0;
        if in_lane && self.playing.is_none() && matches!(self.drag, Drag::None | Drag::Place { .. })
        {
            let at = self.ed.snapped(self.t_at(mx));
            let gx = self.x_of(at);
            match self.ed.tool {
                Tool::Note => {
                    let r = if self.ed.big { BIG_R } else { NOTE_R };
                    c.rect(
                        gx - 0.5,
                        lane_top,
                        1.0,
                        y + h - lane_top - 1.0,
                        rgba(0xffffff, 0.22),
                    );
                    c.circle(
                        gx,
                        cy,
                        r,
                        rgba(if self.ed.kat { KAT } else { DON }, 0.4),
                        rgba(0xffffff, 0.6),
                        1.0,
                    );
                    c.text(
                        gx + r + 3.0,
                        cy - r - 8.0,
                        5.5,
                        rgba(0xffffff, 0.8),
                        &fmt_time(at),
                    );
                }
                Tool::Roll | Tool::Swell => {
                    c.rect(
                        gx - 0.5,
                        lane_top,
                        1.0,
                        y + h - lane_top - 1.0,
                        rgba(0xffffff, 0.4),
                    );
                    c.text(
                        gx + 4.0,
                        cy - NOTE_R - 8.0,
                        5.5,
                        rgba(0xffffff, 0.8),
                        &fmt_time(at),
                    );
                }
                Tool::Select => {}
            }
        }
        if let Drag::Box {
            from,
            to,
            moved: true,
            ..
        } = self.drag
        {
            let (xa, xb) = (self.x_of(from), self.x_of(to));
            let (a, b) = (xa.min(xb).max(x), xa.max(xb).min(x + w));
            c.rect(
                a,
                lane_top,
                b - a,
                y + h - lane_top - 1.0,
                rgba(accent(), 0.18),
            );
            c.round_rect(
                [a, lane_top],
                [b - a, y + h - lane_top - 1.0],
                0.0,
                rgba(0, 0.0),
                rgba(accent(), 0.8),
                1.0,
            );
        }
        if let Some((start, swell)) = self.placing() {
            let to = self.ed.snapped(self.t_at(self.mouse.0)).max(start);
            let (a, b) = (self.x_of(start), self.x_of(to));
            let col = if swell { SWELL } else { ROLL };
            c.capsule([a, b], cy, 9.0, rgba(col, 0.55), rgba(0xffffff, 0.9), 1.0);
            c.circle(a, cy, NOTE_R, rgba(col, 0.9), rgba(0xffffff, 0.9), 1.0);
            let beats = (to - start) / timing::beat_length_at(&self.ed.chart.timing, start);
            c.text(
                b + 6.0,
                cy - NOTE_R - 8.0,
                5.5,
                rgba(0xffffff, 0.9),
                &format!("{} ({beats:.2} beats)", fmt_time(to)),
            );
        }
        // The playhead.
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 1.0));
        c.clip_since(start, x, y, w, h);
        self.tips.push((
            [x, y, w, RULER],
            "The ruler\nClick or drag to move the playhead. Alt+wheel zooms, the middle button pans."
                .to_string(),
        ));
    }

    /// A drumroll or denden being drawn out: where it starts, and whether it's a denden.
    fn placing(&self) -> Option<(f64, bool)> {
        match self.drag {
            Drag::Place { start, swell, .. } => Some((start, swell)),
            _ => self.roll_start.map(|s| (s, self.ed.tool == Tool::Swell)),
        }
    }

    /// The SV graph's vertical range: log scale, wide enough for what's in view.
    fn graph_range(&self) -> (f64, f64) {
        if let Some(r) = self.g_frozen {
            return r;
        }
        let pts = &self.ed.chart.timing;
        let first = pts
            .partition_point(|p| p.time < self.view_start)
            .saturating_sub(1);
        let (mut lo, mut hi) = (0.5f64, 2.0f64);
        for p in pts[first..]
            .iter()
            .take_while(|p| p.time <= self.view_end())
        {
            let v = p.sv();
            if v.is_finite() && v > 0.0 {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        (lo / 1.15, hi * 1.15)
    }

    /// The part of the graph values are drawn in: the top strip holds its buttons and hint.
    pub(super) fn plot(&self) -> (f32, f32) {
        let [_, y, _, h] = self.layout.graph;
        (y + h - 4.0, h - 22.0)
    }

    fn sv_y(&self, sv: f64, range: (f64, f64)) -> f32 {
        let (bottom, span) = self.plot();
        let f = (sv.max(1e-3).ln() - range.0.ln()) / (range.1.ln() - range.0.ln());
        bottom - (f.clamp(0.0, 1.0) as f32) * span
    }

    fn y_sv(&self, py: f32, range: (f64, f64)) -> f64 {
        let (bottom, span) = self.plot();
        let f = ((bottom - py) / span).clamp(0.0, 1.0) as f64;
        (range.0.ln() + f * (range.1.ln() - range.0.ln())).exp()
    }

    fn draw_graph(&mut self, c: &mut Canvas) {
        if self.keyframe_mode() {
            return self.draw_key_graph(c);
        }
        let [x, y, w, h] = self.layout.graph;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x101016, 0.95),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        let start = c.shapes.len();
        let range = self.graph_range();
        let volume = self.svt.volume_view;
        let line_col = if volume {
            curves::VOLUME_COL
        } else {
            GREEN_LINE
        };
        // 1.0x and the round multiples, or the quarters of the volume.
        let marks: &[f64] = if volume {
            &[25.0, 50.0, 75.0]
        } else {
            &[0.5, 1.0, 2.0, 4.0]
        };
        for &v in marks {
            if volume || (v > range.0 && v < range.1) {
                let ly = self.val_y(v, range);
                c.rect(
                    x + 1.0,
                    ly,
                    w - 2.0,
                    1.0,
                    rgba(0xffffff, if v == 1.0 { 0.22 } else { 0.08 }),
                );
                let label = if volume {
                    format!("{v}")
                } else {
                    format!("x{v}")
                };
                c.text(x + 3.0, ly - 8.0, 6.0, rgba(0x6a6a80, 1.0), &label);
            }
        }
        // The value as the game reads it: a step at each line.
        let (from, to) = (self.view_start, self.view_end());
        let pts = &self.ed.chart.timing;
        let first = pts.partition_point(|p| p.time <= from).saturating_sub(1);
        let mut prev_x = x;
        let mut prev_sv = self.val_at(from);
        for p in pts[first..].iter().filter(|p| p.time > from) {
            if p.time > to {
                break;
            }
            let px = self.x_of(p.time);
            let (ya, yb) = (
                self.val_y(prev_sv, range),
                self.val_y(self.graph_val(p), range),
            );
            c.rect(
                prev_x,
                ya - 0.5,
                (px - prev_x).max(1.0),
                1.5,
                rgba(line_col, 0.9),
            );
            c.rect(
                px - 0.5,
                ya.min(yb),
                1.5,
                (ya - yb).abs().max(1.0),
                rgba(line_col, 0.5),
            );
            prev_x = px;
            prev_sv = self.graph_val(p);
        }
        let ya = self.val_y(prev_sv, range);
        c.rect(
            prev_x,
            ya - 0.5,
            (x + w - prev_x).max(1.0),
            1.5,
            rgba(line_col, 0.9),
        );
        // The lines themselves.
        for p in pts[first..].iter().take_while(|p| p.time <= to) {
            if p.time < from {
                continue;
            }
            let px = self.x_of(p.time);
            if p.is_red() {
                c.rect(px - 0.5, y + 1.0, 1.0, h - 2.0, rgba(RED_LINE, 0.55));
            }
            if !p.is_red() || volume {
                let on = self.selected_lines().contains(&p.id);
                let base = if p.is_red() { RED_LINE } else { line_col };
                c.circle(
                    px,
                    self.val_y(self.graph_val(p), range),
                    if on { 4.5 } else { 3.5 },
                    rgba(if on { 0xffffff } else { base }, 1.0),
                    rgba(0x000000, 0.8),
                    1.0,
                );
            }
        }
        self.draw_curve_layer(c, range);
        self.draw_svt_ghost(c, range);
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 0.8));
        c.clip_since(start, x, y, w, h);
        self.draw_line_selection(c);
        let hint = if self.mode == Mode::Timing && self.svt.brush {
            "Brush: drag from where the curve starts to where it ends   Shift: free time"
        } else if self.mode == Mode::Timing && volume {
            "Volume   double click: add a line   drag a dot: change it   drag a box: select several   right click: delete"
        } else if self.mode == Mode::Timing {
            "Slider velocity   double click: add a line   drag a dot: change it (Alt: add)   drag a box: select   click a curve: edit it"
        } else {
            "Slider velocity (read only here: edit it in Timing mode, F3)"
        };
        let reserve = if self.mode == Mode::Timing {
            140.0
        } else {
            0.0
        };
        c.text(
            x + w - 6.0 - reserve - Canvas::text_width(6.0, hint),
            y + 3.0,
            6.0,
            rgba(0x6a6a80, 1.0),
            hint,
        );
        self.draw_graph_corner(c);
    }

    // ---- the pointer ----

    /// The object under `x` on the timeline: a note by its head, a roll or swell along its body.
    /// The middle of the notes' lane.
    fn lane_cy(&self) -> f32 {
        let [_, y, _, h] = self.layout.timeline;
        let top = y + RULER + 16.0;
        top + (y + h - top) / 2.0
    }

    /// The note under the pointer: its head by the circle drawn, a roll or swell also along its
    /// body. Anywhere else in the lane is empty, which is what lets a box start between notes.
    fn object_at_point(&self, x: f32, y: f32) -> Option<u64> {
        let cy = self.lane_cy();
        let (t0, t1) = (self.t_at(x - 20.0), self.t_at(x + 20.0));
        let first = self
            .ed
            .chart
            .objects
            .partition_point(|o| o.time < t0 - 60_000.0);
        let mut best: Option<(u8, f32, u64)> = None;
        for o in &self.ed.chart.objects[first..] {
            if o.time > t1 {
                break;
            }
            let end = self.ed.end_of(o);
            let r = if o.big() { BIG_R } else { NOTE_R } + 2.0;
            let (ox, ex) = (self.x_of(o.time), self.x_of(end));
            let (dx, dy) = (x - ox, y - cy);
            let key = if dx * dx + dy * dy <= r * r {
                Some((0, dx * dx + dy * dy))
            } else if ex > ox && x >= ox && x <= ex && dy.abs() <= r * 0.8 {
                Some((1, dy * dy))
            } else {
                None
            };
            if let Some((kind, d)) = key
                && best.is_none_or(|(k, bd, _)| (kind, d) < (k, bd))
            {
                best = Some((kind, d, o.id));
            }
        }
        best.map(|b| b.2)
    }

    /// Keeps the view moving while a drag is held at the edge of the timeline (or past it), faster
    /// the further out the pointer is, and carries the drag along with it.
    fn edge_scroll(&mut self, now: Instant) {
        let dt = self
            .last_tick
            .replace(now)
            .map_or(0.0, |p| now.duration_since(p).as_secs_f64().min(0.1));
        let graph = matches!(
            self.drag,
            Drag::GBox { .. } | Drag::Lines { .. } | Drag::Brush { .. }
        );
        if dt == 0.0
            || !(graph
                || matches!(
                    self.drag,
                    Drag::Box { .. } | Drag::Move { .. } | Drag::Resize { .. } | Drag::Place { .. }
                ))
        {
            return;
        }
        let [l, _, w, _] = if graph {
            self.layout.graph
        } else {
            self.layout.timeline
        };
        let (mx, my) = self.mouse;
        let zone = 28.0;
        let push = if mx < l + zone {
            -((l + zone - mx) / zone).min(2.0)
        } else if mx > l + w - zone {
            ((mx - (l + w - zone)) / zone).min(2.0)
        } else {
            return;
        };
        let span = self.timeline_w() as f64 * self.ms_per_px;
        let moved = (self.view_start + push as f64 * span * 0.75 * dt)
            .clamp(-1000.0, self.end_ms().max(1000.0));
        if moved == self.view_start {
            return;
        }
        self.view_start = moved;
        self.mouse_move(mx, my, self.held);
    }

    fn line_near(&self, x: f32) -> Option<u64> {
        let t = self.t_at(x);
        let slack = 4.0 * self.ms_per_px;
        self.ed
            .chart
            .timing
            .iter()
            .filter(|p| (p.time - t).abs() <= slack)
            .min_by(|a, b| (a.time - t).total_cmp(&(b.time - t)).then(a.id.cmp(&b.id)))
            .map(|p| p.id)
    }

    pub fn mouse_move(&mut self, x: f32, y: f32, keys: Keys) {
        if (x - self.mouse.0).abs() + (y - self.mouse.1).abs() > 0.5 {
            self.moved_at = Instant::now();
        }
        self.mouse = (x, y);
        self.held = keys;
        match self.drag {
            Drag::Scrub => {
                let t = self.t_at(x);
                self.set_time(t);
            }
            Drag::Overview => {
                let t = self.overview_time(x);
                self.set_time(t);
            }
            Drag::TwScroll => self.tw_drag(y),
            Drag::Slide { from_x, start } => {
                self.view_start = start - (x - from_x) as f64 * self.ms_per_px;
            }
            Drag::Box {
                from,
                down_x,
                moved,
                ..
            } => {
                let to = self.t_at(x);
                self.drag = Drag::Box {
                    from,
                    to,
                    down_x,
                    moved: moved
                        || (x - down_x).abs() >= 3.0
                        || (to - from).abs() >= 3.0 * self.ms_per_px,
                    add: keys.shift,
                };
            }
            Drag::Move { anchor, grab } => {
                let Some(now) = self.ed.chart.object(anchor).map(|o| o.time) else {
                    return;
                };
                let want = self.grid_or_free(self.t_at(x) - grab, keys);
                if self.ed.move_selected(want - now, true) {
                    self.mark_stale();
                }
            }
            Drag::Resize { id } => {
                let end = self.grid_or_free(self.t_at(x), keys);
                if self.ed.set_end(id, end, true) {
                    self.mark_stale();
                }
            }
            Drag::Place { .. } => {}
            Drag::Lines { anchor } => self.line_drag(anchor, x, y, keys),
            Drag::GBox { from, .. } => self.drag = Drag::GBox { from, to: (x, y) },
            Drag::Brush { .. } => self.brush_move(x, y, keys),
            Drag::Handle { end } => self.handle_drag(end, y),
            Drag::Key { index } => self.key_graph_drag(index, x, y),
            Drag::Node { .. } | Drag::Pan { .. } => self.node_drag(x, y),
            Drag::Wire { .. } | Drag::None => {}
        }
    }

    /// `t` on the grid, or just rounded to the millisecond while Shift is held.
    fn grid_or_free(&self, t: f64, keys: Keys) -> f64 {
        if keys.shift {
            t.round()
        } else {
            self.ed.snapped(t)
        }
    }

    /// The time at `x` on the whole-song bar.
    fn overview_time(&self, x: f32) -> f64 {
        let [ox, _, ow, _] = self.layout.overview;
        ((x - ox) / ow.max(1.0)).clamp(0.0, 1.0) as f64 * self.end_ms()
    }

    /// The pointer's shape: what a click here would do.
    pub fn cursor(&self) -> winit::window::CursorIcon {
        use winit::window::CursorIcon as I;
        let (x, y) = self.mouse;
        match self.drag {
            Drag::Move { .. } | Drag::Slide { .. } | Drag::Node { .. } | Drag::Pan { .. } => {
                return I::Grabbing;
            }
            Drag::Resize { .. } | Drag::Scrub | Drag::Overview => return I::EwResize,
            Drag::Box { .. } | Drag::Place { .. } | Drag::Brush { .. } => return I::Crosshair,
            Drag::Handle { .. } => return I::NsResize,
            _ => {}
        }
        if self.modal.is_some() || self.tour.is_some() || self.sheet {
            return if self.buttons[self.modal_from.min(self.buttons.len())..]
                .iter()
                .any(|(r, _)| inside(*r, x, y))
            {
                I::Pointer
            } else {
                I::Default
            };
        }
        if self.buttons.iter().any(|(r, _)| inside(*r, x, y)) {
            return I::Pointer;
        }
        if self.node_mode() {
            return I::Default;
        }
        if inside(self.layout.overview, x, y) {
            return I::EwResize;
        }
        let tl = self.layout.timeline;
        if inside(tl, x, y) {
            if y < tl[1] + RULER + 16.0 {
                return I::EwResize;
            }
            if let Some(id) = self.object_at_point(x, y) {
                let end_grab = self.ed.chart.object(id).is_some_and(|o| {
                    let end = self.ed.end_of(o);
                    end > o.time
                        && self.ed.selection.contains(&id)
                        && (self.x_of(end) - x).abs() <= 6.0
                });
                return if end_grab { I::EwResize } else { I::Grab };
            }
            return match self.ed.tool {
                Tool::Select => I::Default,
                _ => I::Crosshair,
            };
        }
        I::Default
    }

    fn graph_dot_near(&self, x: f32, y: f32) -> Option<u64> {
        let range = self.graph_range();
        self.ed
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red() || self.svt.volume_view)
            .filter(|p| {
                let (px, py) = (self.x_of(p.time), self.val_y(self.graph_val(p), range));
                (px - x).abs() <= 7.0 && (py - y).abs() <= 7.0
            })
            .min_by(|a, b| a.time.total_cmp(&b.time))
            .map(|p| p.id)
    }

    /// The button under (`x`, `y`) that can take a click now: while an overlay is up, only its own.
    fn button_at(&self, x: f32, y: f32) -> Option<Btn> {
        let from = self.modal_from.min(self.buttons.len());
        self.buttons[from..]
            .iter()
            .rev()
            .find(|(r, _)| inside(*r, x, y))
            .map(|(_, b)| *b)
    }

    pub fn mouse_down(&mut self, x: f32, y: f32, keys: Keys, audio: Option<&Engine>) {
        if (x - self.mouse.0).abs() + (y - self.mouse.1).abs() > 0.5 {
            self.moved_at = Instant::now();
        }
        self.mouse = (x, y);
        self.held = keys;
        self.typing = None;
        self.n_typing = None;
        self.revert_armed = false;
        // A dropdown closes on any press; one that lists switches stays open for the next.
        let was = self.svt.drop.take();
        if let Some(b) = self.button_at(x, y) {
            self.press(b, keys, audio);
            match b {
                Btn::S(SAct::Hits | SAct::Snaps | SAct::Lines) => self.svt.drop = was,
                Btn::S(SAct::Drop(k)) if was.is_some_and(|d| d.0 == k) => self.svt.drop = None,
                _ => {}
            }
            return;
        }
        // A click that hits nothing closes what's open (a menu, a dropdown) and goes no further.
        if self.menu.take().is_some() || self.n_pick.take().is_some() || was.is_some() {
            return;
        }
        if self.modal_from > 0 {
            let [ox, oy, ow, oh] = self.overlay_rect;
            if self.sheet
                && self.modal.is_none()
                && self.tour.is_none()
                && !inside([ox, oy, ow, oh], x, y)
            {
                self.sheet = false;
            }
            return;
        }
        if inside(self.layout.overview, x, y) {
            self.drag = Drag::Overview;
            let t = self.overview_time(x);
            self.set_time(t);
            return;
        }
        if self.node_mode() {
            if inside(self.layout.strip, x, y) {
                self.drag = Drag::Scrub;
                let t = self.t_at(x);
                self.set_time(t);
            } else if inside(self.layout.nodes, x, y) {
                self.node_down(x, y);
            }
            return;
        }
        let tl = self.layout.timeline;
        if inside(tl, x, y) {
            self.timeline_down(x, y, keys);
        } else if inside(self.layout.graph, x, y) {
            self.graph_down(x, y, keys);
        }
    }

    /// The middle button pans the timeline.
    pub fn middle(&mut self, pressed: bool, x: f32, y: f32) {
        self.mouse = (x, y);
        if pressed {
            if inside(self.layout.timeline, x, y) || inside(self.layout.graph, x, y) {
                self.drag = Drag::Slide {
                    from_x: x,
                    start: self.view_start,
                };
            }
        } else if matches!(self.drag, Drag::Slide { .. }) {
            self.drag = Drag::None;
        }
    }

    /// Selects the note under a press and starts dragging it; a second press on it soon after
    /// puts the playhead there instead.
    fn grab_object(&mut self, id: u64, x: f32, t: f64, keys: Keys) {
        let Some(o) = self.ed.chart.object(id).cloned() else {
            return;
        };
        let now = Instant::now();
        if let Some((at, last)) = self.last_click
            && last == id
            && now.duration_since(at).as_millis() < 400
        {
            self.last_click = None;
            self.stop(None);
            self.set_time(o.time);
            return;
        }
        self.last_click = Some((now, id));
        // The right end of a selected roll or swell stretches it.
        let end = self.ed.end_of(&o);
        if end > o.time && (self.x_of(end) - x).abs() <= 6.0 {
            self.ed.selection = [id].into();
            self.drag = Drag::Resize { id };
            return;
        }
        if keys.shift {
            self.ed.toggle(id);
        } else if !self.ed.selection.contains(&id) {
            self.ed.selection = [id].into();
        }
        self.select_line(None);
        self.drag = Drag::Move {
            anchor: id,
            grab: t - o.time,
        };
    }

    fn timeline_down(&mut self, x: f32, y: f32, keys: Keys) {
        let tl = self.layout.timeline;
        let t = self.t_at(x);
        let strip = y < tl[1] + RULER + 16.0;
        if y < tl[1] + RULER {
            self.drag = Drag::Scrub;
            self.set_time(t);
            return;
        }
        if strip {
            // The strip of timing lines: pick one, or else it moves the playhead like the ruler.
            if let Some(id) = self.line_near(x) {
                self.select_line(Some(id));
                self.ed.select_none();
            } else {
                self.drag = Drag::Scrub;
                self.set_time(t);
            }
            return;
        }
        // Ctrl drags a box whatever the tool is, and even starting on a note.
        if keys.ctrl {
            self.select_line(None);
            self.drag = Drag::Box {
                from: t,
                to: t,
                down_x: x,
                moved: false,
                add: keys.shift,
            };
            return;
        }
        match self.ed.tool {
            Tool::Select | Tool::Note => {
                if let Some(id) = self.object_at_point(x, y) {
                    self.grab_object(id, x, t, keys);
                } else if self.ed.tool == Tool::Select {
                    self.select_line(None);
                    self.drag = Drag::Box {
                        from: t,
                        to: t,
                        down_x: x,
                        moved: false,
                        add: keys.shift,
                    };
                } else {
                    let at = self.grid_or_free(t, keys);
                    if self.ed.place_hit(at, self.ed.kat).is_some() {
                        self.select_line(None);
                        self.mark_stale();
                    } else if at < 0.0 {
                        self.status = "notes can't go before 0:00.000".into();
                    } else {
                        self.status = format!("there is already a note at {}", fmt_time(at));
                    }
                }
            }
            Tool::Roll | Tool::Swell => {
                let at = self.grid_or_free(t, keys);
                let swell = self.ed.tool == Tool::Swell;
                if let Some(start) = self.roll_start.take() {
                    self.finish_roll(start, at, swell);
                } else {
                    self.drag = Drag::Place {
                        start: at,
                        swell,
                        from_x: x,
                    };
                }
            }
        }
    }

    /// Makes the drumroll or denden from `start` to `end`, saying why if it can't be.
    fn finish_roll(&mut self, start: f64, end: f64, swell: bool) {
        if end <= start {
            self.status = "a drumroll needs to end after it starts".into();
            return;
        }
        let made = if swell {
            self.ed.place_swell(start, end)
        } else {
            self.ed.place_roll(start, end)
        };
        if made.is_some() {
            self.mark_stale();
        } else {
            self.status = format!("there is already a note at {}", fmt_time(start));
        }
    }

    fn graph_down(&mut self, x: f32, y: f32, keys: Keys) {
        if self.keyframe_mode() {
            return self.key_graph_down(x, y);
        }
        if self.mode != Mode::Timing {
            self.status = "slider velocity is edited in Timing mode (F3)".into();
            return;
        }
        self.line_graph_down(x, y, keys);
    }

    pub fn mouse_up(&mut self, x: f32, y: f32, keys: Keys) {
        self.mouse = (x, y);
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        match drag {
            Drag::Box { from, moved, .. } => {
                if !moved {
                    self.ed.select_none();
                    let t = self.ed.snapped(from);
                    self.set_time(t);
                } else {
                    self.ed.select_range(from, self.t_at(x), keys.shift);
                }
            }
            Drag::Place {
                start,
                swell,
                from_x,
                ..
            } => {
                let end = self.grid_or_free(self.t_at(x), keys);
                if (x - from_x).abs() < 4.0 {
                    // A click, not a drag: the end is the next click.
                    self.roll_start = Some(start);
                } else {
                    self.finish_roll(start, end, swell);
                }
            }
            Drag::Lines { .. } => {
                self.g_frozen = None;
                self.line_origin.clear();
                self.ed.end_drag();
                self.mark_stale();
                // A click on a dot of a curve (not a drag, no Shift) picks the whole curve.
                let (dx, dy) = (x - self.g_down.0, y - self.g_down.1);
                if dx.abs() < 3.0
                    && dy.abs() < 3.0
                    && !keys.shift
                    && let Some(i) = self.curve_near(x, y)
                {
                    self.press_svt(SAct::Pick(i));
                }
            }
            Drag::GBox { from, .. } => self.line_box_up(from, x, y, keys),
            Drag::Brush { t0, v0, t1, v1 } => self.brush_up(t0, v0, t1, v1),
            Drag::Handle { .. } => self.handle_up(),
            Drag::Move { .. } | Drag::Resize { .. } | Drag::Key { .. } => {
                self.k_frozen = None;
                self.ed.end_drag();
                self.mark_stale();
            }
            Drag::Node { .. } => self.ed.end_drag(),
            Drag::Wire { from } => self.wire_dropped(from, x, y),
            Drag::Scrub
            | Drag::Overview
            | Drag::TwScroll
            | Drag::Slide { .. }
            | Drag::Pan { .. }
            | Drag::None => {}
        }
    }

    /// Right click: cancels a roll being drawn, else deletes what's under the pointer.
    pub fn right_click(&mut self, x: f32, y: f32) {
        if self.menu.take().is_some() || self.modal_from > 0 {
            return;
        }
        if self.node_mode() {
            if inside(self.layout.nodes, x, y) {
                self.node_right(x, y);
            }
            return;
        }
        if self.roll_start.take().is_some() || matches!(self.drag, Drag::Place { .. }) {
            self.drag = Drag::None;
            return;
        }
        if inside(self.layout.timeline, x, y) {
            if let Some(id) = self.object_at_point(x, y) {
                self.ed.selection = [id].into();
                self.ed.delete_selected();
                self.mark_stale();
            }
        } else if self.keyframe_mode() {
            self.key_graph_right(x, y);
        } else if self.mode == Mode::Timing
            && inside(self.layout.graph, x, y)
            && let Some(id) = self.graph_dot_near(x, y)
        {
            self.delete_line(id);
        }
    }

    pub fn wheel(&mut self, notches: i32, keys: Keys) {
        let (mx, my) = self.mouse;
        if self.modal == Some(Modal::Manual) {
            self.manual_scroll(-notches * 3);
            return;
        }
        if self.sheet || self.modal.is_some() || self.tour.is_some() {
            return;
        }
        if self.windowed() && inside(self.layout.window, mx, my) {
            let n = notches.unsigned_abs() as usize * 3;
            self.tw_scroll = if notches > 0 {
                self.tw_scroll.saturating_sub(n)
            } else {
                self.tw_scroll + n
            };
            return;
        }
        if self.node_mode() && inside(self.layout.nodes, mx, my) {
            self.node_zoom(notches);
            return;
        }
        if keys.ctrl && keys.alt {
            let at = TOOLS
                .iter()
                .position(|(_, _, t)| *t == self.ed.tool)
                .unwrap_or(0) as i32;
            let next = (at - notches.signum()).rem_euclid(TOOLS.len() as i32) as usize;
            self.ed.tool = TOOLS[next].2;
            self.roll_start = None;
        } else if keys.alt {
            self.zoom(if notches > 0 { 0.8 } else { 1.25 }, mx);
        } else if keys.ctrl {
            self.step_snap(notches.signum());
        } else {
            let per = if keys.shift { 4 } else { 1 };
            let mut t = self.ed.time;
            for _ in 0..(notches.unsigned_abs().min(20) * per) {
                t = timing::step(&self.ed.chart.timing, t, notches < 0, self.ed.snap);
            }
            self.set_time(t);
        }
    }

    fn step_snap(&mut self, dir: i32) {
        let at = SNAP_DIVISORS
            .iter()
            .position(|d| *d == self.ed.snap)
            .unwrap_or(3) as i32;
        let next = (at + dir).clamp(0, SNAP_DIVISORS.len() as i32 - 1) as usize;
        self.ed.snap = SNAP_DIVISORS[next];
    }

    // ---- actions ----

    fn delete_line(&mut self, id: u64) {
        match self.ed.delete_timing(id) {
            Ok(()) => {
                if self.sel_line == Some(id) {
                    self.select_line(None);
                }
                self.mark_stale();
            }
            Err(e) => self.status = e,
        }
    }

    fn save(&mut self) {
        self.status = match self.ed.save() {
            Ok(()) => format!("saved {}", self.ed.path.display()),
            Err(e) => format!("can't save: {e}"),
        };
    }

    /// The lines an edit of `f` reaches: the volume and hitsounds of all the selected lines, the
    /// rest (time, BPM, meter, velocity) of the main one.
    fn edit_targets(&self, f: Field) -> Vec<u64> {
        match f {
            Field::Volume | Field::Sample | Field::Index => self.selected_lines(),
            _ => self.sel_line.into_iter().collect(),
        }
    }

    fn change(&mut self, f: Field, dir: f64, keys: Keys) {
        if let Field::S(s) = f {
            return self.svt_step(s, dir, keys);
        }
        let ids = self.edit_targets(f);
        let k = if keys.shift {
            10.0
        } else if keys.alt {
            0.1
        } else {
            1.0
        };
        let r = self.ed.edit_timings(
            &ids,
            |p| match f {
                Field::Bpm => {
                    if let Some(bpm) = p.bpm() {
                        p.beat_length = 60_000.0 / (bpm + dir * k).max(1.0);
                    }
                }
                Field::Sv => {
                    if !p.is_red() {
                        let _ = p.set_sv((p.sv() + dir * 0.1 * k).max(0.01));
                    }
                }
                Field::Volume => p.volume = (p.volume + (dir * 5.0 * k) as i32).clamp(0, 100),
                Field::Meter => p.meter = (p.meter + (dir * k.max(1.0)) as i32).max(1),
                Field::Time => p.time = (p.time + dir * k).max(0.0),
                Field::Sample => p.sample_set = (p.sample_set + dir as i32).rem_euclid(4),
                Field::Index => p.sample_index = (p.sample_index + dir as i32).max(0),
                Field::S(_) => {}
            },
            false,
        );
        match r {
            Ok(()) => self.mark_stale(),
            Err(e) => self.status = e,
        }
    }

    fn apply_typed(&mut self, f: Field, text: &str) {
        if let Field::S(s) = f {
            match text.trim().parse::<f64>() {
                Ok(v) => self.svt_set(s, v),
                Err(_) => self.status = "not a number".into(),
            }
            return;
        }
        let Ok(v) = text.trim().parse::<f64>() else {
            self.status = "not a number".into();
            return;
        };
        let ids = self.edit_targets(f);
        let r = self.ed.edit_timings(
            &ids,
            |p| match f {
                Field::Bpm => {
                    if v > 0.0 && p.is_red() {
                        p.beat_length = 60_000.0 / v;
                    }
                }
                Field::Sv => {
                    let _ = p.set_sv(v);
                }
                Field::Volume => p.volume = v as i32,
                Field::Meter => p.meter = v as i32,
                Field::Time => p.time = v,
                Field::Sample => p.sample_set = (v as i32).clamp(0, 3),
                Field::Index => p.sample_index = (v as i32).max(0),
                Field::S(_) => {}
            },
            false,
        );
        match r {
            Ok(()) => self.mark_stale(),
            Err(e) => self.status = e,
        }
    }

    pub fn press(&mut self, b: Btn, keys: Keys, audio: Option<&Engine>) {
        if !matches!(b, Btn::Menu(_)) {
            self.menu = None;
        }
        match b {
            Btn::C(cmd) => self.run(cmd, audio),
            Btn::U(act) => self.press_upload(act),
            Btn::Man(act) => self.press_manual(act, audio),
            Btn::Menu(i) => self.menu = if self.menu == Some(i) { None } else { Some(i) },
            Btn::TourNext => {
                let at = self.tour.unwrap_or(0);
                self.tour_goto(at + 1);
            }
            Btn::TourBack => {
                if let Some(at) = self.tour {
                    self.tour_goto(at.saturating_sub(1));
                }
            }
            Btn::TourSkip => self.tour_end(),
            Btn::Line(id) => self.pick_row(id, keys, audio),
            Btn::TwBar => {
                self.drag = Drag::TwScroll;
                self.tw_drag(self.mouse.1);
            }
            Btn::TwFilter(f) => {
                self.tw_filter = f;
                self.tw_scroll = 0;
            }
            Btn::Resnap => {
                let n = self.ed.resnap_selected();
                self.status = format!("resnapped {n}");
                self.mark_stale();
            }
            Btn::AddRed => {
                let at = self.ed.time;
                let bpm = timing::active_red(&self.ed.chart.timing, at)
                    .and_then(|p| p.bpm())
                    .unwrap_or(120.0);
                match self.ed.add_red(at, bpm) {
                    Ok(id) => {
                        self.select_line(Some(id));
                        self.mark_stale();
                    }
                    Err(e) => self.status = e,
                }
            }
            Btn::AddGreen => {
                let at = self.ed.time;
                let sv = timing::sv_at(&self.ed.chart.timing, at);
                match self.ed.add_green(at, sv) {
                    Ok(id) => {
                        self.select_line(Some(id));
                        self.mark_stale();
                    }
                    Err(e) => self.status = e,
                }
            }
            Btn::Kiai | Btn::Omit => {
                let ids = self.selected_lines();
                let on = self
                    .sel_line
                    .and_then(|id| self.ed.chart.timing_point(id))
                    .map(|p| {
                        if b == Btn::Kiai {
                            !p.kiai()
                        } else {
                            !p.omits_barline()
                        }
                    });
                if let Some(on) = on {
                    let r = self.ed.edit_timings(
                        &ids,
                        |p| {
                            if b == Btn::Kiai {
                                p.set_kiai(on);
                            } else if p.is_red() {
                                p.set_omit_barline(on);
                            }
                        },
                        false,
                    );
                    if r.is_ok() {
                        self.mark_stale();
                    }
                }
            }
            Btn::Dec(f) => self.change(f, -1.0, keys),
            Btn::Inc(f) => self.change(f, 1.0, keys),
            Btn::Edit(f) => {
                if self.sel_line.is_some() || matches!(f, Field::S(_)) {
                    self.typing = Some((f, String::new()));
                }
            }
            Btn::GPage(p) => {
                self.set_mode(if p < 2 { Mode::Gimmicks } else { Mode::Effects });
                self.g_page = p;
            }
            Btn::GTool(t) => self.g_tool = t,
            Btn::GStep(p, dir) => self.step_param(p, dir as f64, keys.shift),
            Btn::GFlag(f) => self.toggle_flag(f),
            Btn::GApply(t) => self.gimmick_apply(t, false),
            Btn::GPreview => self.gimmick_preview(),
            Btn::GKeep => self.settle_preview(),
            Btn::GCancel => self.gimmick_cancel(),
            Btn::K(a) => self.press_key_action(a, keys.shift, keys.alt),
            Btn::N(a) => self.press_node_action(a, keys.shift),
            Btn::T(a) => self.press_line_action(a),
            Btn::S(a) => self.press_svt(a),
            Btn::GBase => {
                self.ed.reset_base();
                self.status = "base timing is now the current chart".into();
            }
        }
    }

    // ---- the keyboard ----

    pub fn key(
        &mut self,
        code: KeyCode,
        repeat: bool,
        keys: Keys,
        audio: Option<&Engine>,
    ) -> KeyResult {
        if let Some((i, text)) = &mut self.n_typing {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let (i, text) = (*i, std::mem::take(text));
                    self.n_typing = None;
                    if let (Some(id), Ok(v)) = (self.n_sel, text.trim().parse::<f64>())
                        && self.ed.set_node_param(id, i, v)
                    {
                        self.mark_stale();
                    }
                }
                KeyCode::Escape => self.n_typing = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Period | KeyCode::NumpadDecimal => text.push('.'),
                KeyCode::Minus | KeyCode::NumpadSubtract => text.push('-'),
                c => {
                    if let Some(d) = digit(c) {
                        text.push(d);
                    }
                }
            }
            return KeyResult::Handled;
        }
        if let Some((f, text)) = &mut self.typing {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let (f, text) = (*f, std::mem::take(text));
                    self.typing = None;
                    self.apply_typed(f, &text);
                }
                KeyCode::Escape => self.typing = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Period | KeyCode::NumpadDecimal => text.push('.'),
                KeyCode::Minus | KeyCode::NumpadSubtract => text.push('-'),
                c => {
                    if let Some(d) = digit(c) {
                        text.push(d);
                    }
                }
            }
            return KeyResult::Handled;
        }
        if code == KeyCode::Escape {
            if repeat {
                return KeyResult::Handled;
            }
            // Esc backs out of the innermost thing first.
            if self.tour.is_some() {
                self.tour_end();
                return KeyResult::Handled;
            }
            if self.menu.take().is_some()
                || self.n_pick.take().is_some()
                || self.svt.drop.take().is_some()
                || self.roll_start.take().is_some()
            {
                return KeyResult::Handled;
            }
            if self.modal.take().is_some() {
                return KeyResult::Handled;
            }
            if self.sheet {
                self.sheet = false;
                return KeyResult::Handled;
            }
            if self.tw {
                self.tw = false;
                return KeyResult::Handled;
            }
        }
        if self.tour.is_some() {
            if !repeat {
                match code {
                    KeyCode::Enter
                    | KeyCode::NumpadEnter
                    | KeyCode::ArrowRight
                    | KeyCode::Space => self.press(Btn::TourNext, keys, audio),
                    KeyCode::ArrowLeft => self.press(Btn::TourBack, keys, audio),
                    _ => {}
                }
            }
            return KeyResult::Handled;
        }
        if self.modal == Some(Modal::Manual) {
            let page = self.manual_page();
            match code {
                KeyCode::ArrowDown => self.manual_scroll(1),
                KeyCode::ArrowUp => self.manual_scroll(-1),
                KeyCode::PageDown | KeyCode::Space => self.manual_scroll(page),
                KeyCode::PageUp => self.manual_scroll(-page),
                KeyCode::Home => self.manual_scroll(i32::MIN / 2),
                KeyCode::End => self.manual_scroll(i32::MAX / 2),
                _ => {}
            }
            return KeyResult::Handled;
        }
        if self.modal.is_some() {
            return KeyResult::Handled;
        }
        if self.typing.is_none() && self.tw_key(code, keys, audio) {
            return KeyResult::Handled;
        }
        let Some(cmd) = cmd::find(code, keys, repeat) else {
            return KeyResult::Pass;
        };
        self.run(cmd, audio);
        if self.exit {
            return KeyResult::Exit;
        }
        KeyResult::Handled
    }
}

fn digit(c: KeyCode) -> Option<char> {
    Some(match c {
        KeyCode::Digit0 | KeyCode::Numpad0 => '0',
        KeyCode::Digit1 | KeyCode::Numpad1 => '1',
        KeyCode::Digit2 | KeyCode::Numpad2 => '2',
        KeyCode::Digit3 | KeyCode::Numpad3 => '3',
        KeyCode::Digit4 | KeyCode::Numpad4 => '4',
        KeyCode::Digit5 | KeyCode::Numpad5 => '5',
        KeyCode::Digit6 | KeyCode::Numpad6 => '6',
        KeyCode::Digit7 | KeyCode::Numpad7 => '7',
        KeyCode::Digit8 | KeyCode::Numpad8 => '8',
        KeyCode::Digit9 | KeyCode::Numpad9 => '9',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::TimingPoint;
    use crate::editor::chart::tests::MAP;
    use crate::editor::sv_curves::Curve;
    use crate::editor::svt::Shape;

    fn view() -> View {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "taiko-rs-editor-view-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("map.osu");
        std::fs::write(&path, MAP).unwrap();
        let mut v = View::open(path, 0).unwrap();
        v.layout = Layout::new(853.0, Mode::Compose, false);
        v.view_start = 0.0;
        v.ms_per_px = 10.0;
        v
    }

    fn draw(v: &mut View) -> Canvas {
        let mut c = Canvas::default();
        c.clear(853.0);
        v.draw(&mut c, 1.0, crate::autoplay::Settings::default());
        c
    }

    /// A y inside the notes' lane.
    fn lane_y(v: &View) -> f32 {
        v.lane_cy()
    }

    /// A click at map time `t` and height `y`.
    fn click_t(v: &mut View, t: f64, y: f32) {
        let x = v.x_of(t);
        click(v, x, y);
    }

    /// Clicks the middle of the button `b` as drawn last.
    fn click_btn(v: &mut View, b: Btn) {
        let (r, _) = *v
            .buttons
            .iter()
            .rev()
            .find(|(_, x)| *x == b)
            .unwrap_or_else(|| panic!("no button {b:?} on screen"));
        click(v, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    }

    fn click(v: &mut View, x: f32, y: f32) {
        v.mouse_down(x, y, Keys::default(), None);
        v.mouse_up(x, y, Keys::default());
    }

    #[test]
    fn the_screen_draws_with_and_without_a_preview() {
        let mut v = view();
        let c = draw(&mut v);
        assert!(!c.shapes.is_empty());
        assert!(v.game.is_some(), "{:?}", v.game_err);
        // A chart with no notes has nothing to preview, and says why.
        v.ed.select_all();
        v.ed.delete_selected();
        v.mark_stale();
        draw(&mut v);
        assert!(v.game.is_none() && v.game_err.is_some());
    }

    #[test]
    fn a_click_on_the_timeline_places_with_the_tool_on_the_snapped_ms() {
        let mut v = view();
        draw(&mut v);
        v.ed.tool = Tool::Note;
        v.ed.kat = true;
        // 1/4 of 500 ms = 125: x for t = 3130 snaps to 3125.
        let x = v.x_of(3130.0);
        let y = lane_y(&v);
        click(&mut v, x, y);
        assert!(
            v.ed.chart
                .objects
                .iter()
                .any(|o| o.time == 3125.0 && o.taiko() == Taiko::Kat)
        );
        // A drag with the roll tool makes a roll of that length.
        v.ed.tool = Tool::Roll;
        let (a, b) = (v.x_of(5000.0), v.x_of(5500.0));
        let y = lane_y(&v);
        v.mouse_down(a, y, Keys::default(), None);
        v.mouse_move(b, y, Keys::default());
        v.mouse_up(b, y, Keys::default());
        let roll =
            v.ed.chart
                .objects
                .iter()
                .find(|o| o.time == 5000.0)
                .unwrap();
        assert_eq!(v.ed.end_of(roll), 5500.0);
    }

    #[test]
    fn dragging_a_selected_note_moves_it_as_one_undo() {
        let mut v = view();
        draw(&mut v);
        let y = lane_y(&v);
        let (a, b) = (v.x_of(1000.0), v.x_of(1250.0));
        v.mouse_down(a, y, Keys::default(), None);
        for step in 1..=5 {
            v.mouse_move(a + (b - a) * step as f32 / 5.0, y, Keys::default());
        }
        v.mouse_up(b, y, Keys::default());
        assert!(v.ed.chart.objects.iter().any(|o| o.time == 1250.0));
        assert!(v.ed.undo());
        assert!(v.ed.chart.objects.iter().any(|o| o.time == 1000.0));
    }

    #[test]
    fn a_box_selects_what_it_covers_and_a_plain_click_moves_the_playhead() {
        let mut v = view();
        draw(&mut v);
        v.ed.tool = Tool::Select;
        let y = lane_y(&v);
        let xx = v.x_of(800.0);
        v.mouse_down(xx, y, Keys::default(), None);
        v.mouse_move(v.x_of(2100.0), y, Keys::default());
        v.mouse_up(v.x_of(2100.0), y, Keys::default());
        assert_eq!(v.ed.selection.len(), 3);
        let x = v.x_of(3000.0);
        click(&mut v, x, y);
        assert!(v.ed.selection.is_empty());
        assert_eq!(v.ed.time, 3000.0);
    }

    /// Notes every 40 ms from `a` to `b`: closer together than they are wide.
    fn crowd(v: &mut View, a: f64, b: f64) {
        let mut t = a;
        while t <= b {
            v.ed.place_hit(t, false);
            t += 40.0;
        }
    }

    #[test]
    fn a_box_can_start_between_crowded_notes_and_a_press_on_a_circle_grabs_it() {
        let mut v = view();
        crowd(&mut v, 6000.0, 7000.0);
        draw(&mut v);
        v.ed.tool = Tool::Select;
        let before = v.ed.chart.objects.len();
        // Above the circles of the crowd, the lane is empty: a drag there boxes the notes.
        let above = v.lane_cy() - NOTE_R - 8.0;
        let (a, b) = (v.x_of(6190.0), v.x_of(6410.0));
        v.mouse_down(a, above, Keys::default(), None);
        v.mouse_move(b, above, Keys::default());
        draw(&mut v);
        v.mouse_up(b, above, Keys::default());
        assert_eq!(v.ed.chart.objects.len(), before);
        assert_eq!(v.ed.selection.len(), 6, "190..=410 holds 200, 240 ... 400");
        // On a circle the same drag moves the note instead.
        v.ed.select_none();
        let y = v.lane_cy();
        let x = v.x_of(6000.0);
        assert!(v.object_at_point(x, y).is_some());
        assert!(v.object_at_point(x, above).is_none());
        v.mouse_down(x, y, Keys::default(), None);
        assert!(matches!(v.drag, Drag::Move { .. }));
        v.mouse_up(x, y, Keys::default());
    }

    #[test]
    fn a_box_dragged_past_the_edge_scrolls_and_keeps_growing() {
        let mut v = view();
        crowd(&mut v, 9000.0, 14_000.0);
        draw(&mut v);
        v.ed.tool = Tool::Select;
        let above = v.lane_cy() - NOTE_R - 8.0;
        let start = v.x_of(7000.0);
        v.mouse_down(start, above, Keys::default(), None);
        // The pointer goes to the right end of the timeline and stays there.
        let edge = v.layout.timeline[0] + v.layout.timeline[2] - 2.0;
        v.mouse_move(edge, above, Keys::default());
        let t0 = Instant::now();
        let seen = v.view_start;
        for i in 0..40 {
            v.update(t0 + std::time::Duration::from_millis(50 * i), None);
        }
        assert!(v.view_end() > 14_000.0, "scrolled to {}", v.view_start);
        assert!(v.view_start > seen);
        v.mouse_up(edge, above, Keys::default());
        assert!(v.ed.selection.len() > 100, "got {}", v.ed.selection.len());
        // Letting go ends the scrolling.
        let at = v.view_start;
        v.update(t0 + std::time::Duration::from_secs(5), None);
        assert_eq!(v.view_start, at);
    }

    #[test]
    fn ctrl_drag_boxes_with_any_tool_instead_of_placing_a_note() {
        let mut v = view();
        draw(&mut v);
        v.ed.tool = Tool::Note;
        let before = v.ed.chart.objects.len();
        let ctrl = Keys {
            ctrl: true,
            ..Keys::default()
        };
        let y = v.lane_cy();
        let (a, b) = (v.x_of(800.0), v.x_of(2100.0));
        v.mouse_down(a, y, ctrl, None);
        v.mouse_move(b, y, ctrl);
        v.mouse_up(b, y, ctrl);
        assert_eq!(v.ed.chart.objects.len(), before);
        assert_eq!(v.ed.selection.len(), 3);
    }

    #[test]
    fn double_clicking_the_graph_adds_a_green_line_and_dragging_it_changes_its_velocity() {
        let mut v = view();
        v.run(Cmd::Mode(Mode::Timing), None);
        draw(&mut v);
        let g = v.layout.graph;
        let (x, y) = (v.x_of(2000.0), g[1] + g[3] / 2.0);
        click(&mut v, x, y);
        let count = |v: &View| v.ed.chart.timing.iter().filter(|p| !p.is_red()).count();
        assert_eq!(count(&v), 1, "one click only lets go of the selection");
        click(&mut v, x, y);
        assert_eq!(count(&v), 2, "a second click right after adds a line");
        let id = v.sel_line.unwrap();
        // Two clicks far apart in time are not a double click.
        v.g_click = None;
        let (far, _) = (v.x_of(3000.0), 0);
        click(&mut v, far, y);
        click(&mut v, x + 40.0, y);
        assert_eq!(count(&v), 2);
        v.g_click = None;
        // Drag the new dot up: its velocity changes, as one undo step.
        draw(&mut v);
        let before = v.ed.chart.timing_point(id).unwrap().sv();
        let range = v.graph_range();
        let (dx, dy) = (v.x_of(2000.0), v.sv_y(before, range));
        v.mouse_down(dx, dy, Keys::default(), None);
        for i in 1..=4 {
            v.mouse_move(dx, dy - 6.0 * i as f32, Keys::default());
        }
        v.mouse_up(dx, dy - 24.0, Keys::default());
        assert!(v.ed.chart.timing_point(id).unwrap().sv() > before);
        assert!(v.ed.undo());
        assert_eq!(v.ed.chart.timing_point(id).unwrap().sv(), before);
        assert!(v.ed.undo());
        assert_eq!(v.ed.chart.timing.iter().filter(|p| !p.is_red()).count(), 1);
    }

    fn greens(v: &View) -> Vec<TimingPoint> {
        v.ed.chart
            .timing
            .iter()
            .filter(|p| !p.is_red())
            .cloned()
            .collect()
    }

    /// A stroke of the brush from (`t0`, `v0`) to (`t1`, `v1`) in the graph.
    fn stroke(v: &mut View, (t0, v0): (f64, f64), (t1, v1): (f64, f64)) {
        let range = v.graph_range();
        let (a, b) = (
            (v.x_of(t0), v.val_y(v0, range)),
            (v.x_of(t1), v.val_y(v1, range)),
        );
        v.mouse_down(a.0, a.1, Keys::default(), None);
        v.mouse_move((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, Keys::default());
        v.mouse_move(b.0, b.1, Keys::default());
        draw(v);
        v.mouse_up(b.0, b.1, Keys::default());
    }

    #[test]
    fn the_brush_draws_a_curve_over_the_notes_and_its_ends_reshape_it() {
        let mut v = view();
        v.run(Cmd::Mode(Mode::Timing), None);
        crowd(&mut v, 5000.0, 7000.0);
        press(&mut v, Btn::S(SAct::Brush));
        draw(&mut v);
        let before = greens(&v).len();
        stroke(&mut v, (5000.0, 1.0), (7000.0, 2.0));
        let made = greens(&v);
        assert_eq!(made.len(), before + 51, "one line for each of the 51 notes");
        let first = made
            .iter()
            .find(|p| p.time == 4999.0)
            .expect("a ms before the first note");
        assert!((first.sv() - 1.0).abs() < 0.02, "{}", first.sv());
        let last = made.iter().find(|p| p.time == 6999.0).unwrap();
        assert!((last.sv() - 2.0).abs() < 0.02, "{}", last.sv());
        assert!(v.svt.picked.is_some(), "the new curve is picked");
        // The chart reads it back as a curve with a chip, and the ends have handles.
        draw(&mut v);
        assert!(
            v.buttons
                .iter()
                .any(|(_, b)| matches!(b, Btn::S(SAct::Pick(_))))
        );
        assert_eq!(v.svt.handles.len(), 2);
        // Dragging the end handle up raises the last line and keeps the shape; one undo step.
        let (r, _) = v.svt.handles[1];
        let (hx, hy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        v.mouse_down(hx, hy, Keys::default(), None);
        for i in 1..=4 {
            v.mouse_move(hx, hy - 5.0 * i as f32, Keys::default());
        }
        v.mouse_up(hx, hy - 20.0, Keys::default());
        let after = greens(&v);
        let raised = after.iter().find(|p| p.time == 6999.0).unwrap().sv();
        assert!(raised > last.sv() + 0.1, "{raised}");
        let mid = after.iter().find(|p| p.time == 5999.0).unwrap().sv();
        assert!(mid > 1.4 && mid < raised, "{mid}");
        assert!(v.ed.undo());
        let back = greens(&v);
        assert!((back.iter().find(|p| p.time == 6999.0).unwrap().sv() - last.sv()).abs() < 1e-6);
        assert!(v.ed.undo());
        assert_eq!(greens(&v).len(), before);
    }

    #[test]
    fn a_picked_curve_takes_another_shape_in_one_undo_step() {
        let mut v = view();
        v.run(Cmd::Mode(Mode::Timing), None);
        crowd(&mut v, 5000.0, 7000.0);
        press(&mut v, Btn::S(SAct::Brush));
        draw(&mut v);
        stroke(&mut v, (5000.0, 1.0), (7000.0, 2.0));
        press(&mut v, Btn::S(SAct::Brush));
        draw(&mut v);
        let at = |v: &View| greens(v).iter().find(|p| p.time == 5999.0).unwrap().sv();
        let linear = at(&v);
        assert!((linear - 1.5).abs() < 0.05, "{linear}");
        // Geometric: halfway is the geometric mean, sqrt(2).
        press(&mut v, Btn::S(SAct::Shape(1)));
        assert!((at(&v) - 2f64.sqrt()).abs() < 0.05, "{}", at(&v));
        draw(&mut v);
        assert!(v.svt.picked.is_some(), "still the same curve");
        assert!(v.ed.undo());
        assert!((at(&v) - linear).abs() < 1e-6);
    }

    #[test]
    fn clicking_a_curve_picks_it_and_opens_its_editor() {
        let mut v = view();
        v.run(Cmd::Mode(Mode::Timing), None);
        crowd(&mut v, 5000.0, 7000.0);
        press(&mut v, Btn::S(SAct::Brush));
        draw(&mut v);
        stroke(&mut v, (5000.0, 1.0), (7000.0, 2.0));
        press(&mut v, Btn::S(SAct::Brush));
        // Let go of it, back on the Line tab.
        press(&mut v, Btn::S(SAct::Done));
        press(&mut v, Btn::S(SAct::Tab(0)));
        draw(&mut v);
        assert!(v.svt.picked.is_none());
        // A click on the curve (its dots are a few units apart here) picks it and shows the SV tool.
        let range = v.graph_range();
        // Halfway between two of its dots, so the press is on the line and not on a dot.
        let t = 5519.0;
        let run_v = 1.0 + (t - 5000.0) / 2000.0;
        let (x, y) = (v.x_of(t), v.sv_y(run_v, range));
        click(&mut v, x, y);
        assert!(v.svt.picked.is_some(), "the curve is picked");
        assert_eq!(v.svt.tab, 1, "its editor opens");
        draw(&mut v);
        assert_eq!(v.svt.handles.len(), 2);
        // The editor's end fields rewrite the curve, as one undo step.
        let svs = |v: &View| {
            greens(v)
                .iter()
                .map(|p| (p.time, p.sv()))
                .collect::<Vec<_>>()
        };
        let before = svs(&v);
        press(&mut v, Btn::Inc(Field::S(SF::ToSv)));
        assert_ne!(before, svs(&v));
        assert!(v.ed.undo());
        assert_eq!(svs(&v), before);
        // Clicking empty graph lets go of it.
        draw(&mut v);
        let g = v.layout.graph;
        let x = v.x_of(1500.0);
        click(&mut v, x, g[1] + g[3] - 6.0);
        draw(&mut v);
        assert!(v.svt.picked.is_none());
    }

    #[test]
    fn the_sv_tools_exclusive_settings_are_dropdowns() {
        let (mut v, ids) = lines_view();
        v.sel_line = Some(ids[0]);
        v.sel_extra = [ids[2]].into();
        press(&mut v, Btn::S(SAct::Tab(1)));
        draw(&mut v);
        // Open the shape list: it offers the shapes, picking one sets it and closes the list.
        click_btn(&mut v, Btn::S(SAct::Drop(0)));
        assert!(v.svt.drop.is_some());
        draw(&mut v);
        click_btn(&mut v, Btn::S(SAct::Shape(3)));
        assert!(v.svt.drop.is_none());
        assert!(matches!(v.svt.params.shape, Shape::Between(Curve::Sin)));
        // Writing volume only: the speed fields go and the volume ones come.
        draw(&mut v);
        click_btn(&mut v, Btn::S(SAct::Drop(1)));
        draw(&mut v);
        click_btn(&mut v, Btn::S(SAct::Write(1)));
        assert!(!v.svt.params.write_sv && v.svt.params.write_volume);
        draw(&mut v);
        assert!(
            v.buttons
                .iter()
                .any(|(_, b)| *b == Btn::Edit(Field::S(SF::FromVol)))
        );
        assert!(
            !v.buttons
                .iter()
                .any(|(_, b)| *b == Btn::Edit(Field::S(SF::FromSv)))
        );
        // The places list takes several; it stays open while they are toggled, Esc closes it.
        click_btn(&mut v, Btn::S(SAct::Drop(2)));
        draw(&mut v);
        click_btn(&mut v, Btn::S(SAct::Snaps));
        assert!(v.svt.drop.is_some() && v.svt.params.snaps);
        v.key(KeyCode::Escape, false, Keys::default(), None);
        assert!(v.svt.drop.is_none());
        // A click elsewhere closes a list without doing anything else.
        click_btn(&mut v, Btn::S(SAct::Drop(1)));
        draw(&mut v);
        click(&mut v, 300.0, 470.0);
        assert!(v.svt.drop.is_none());
    }

    #[test]
    fn the_manual_opens_from_the_help_menu_scrolls_and_shows_every_chapter() {
        use crate::lang::{self, Lang};
        let mut v = view();
        assert!(menu::MENUS.iter().any(|m| {
            m.name == "Help"
                && m.items
                    .iter()
                    .any(|i| matches!(i, menu::Item::Do(Cmd::Manual)))
        }));
        press(&mut v, Btn::C(Cmd::Manual));
        assert_eq!(v.modal, Some(Modal::Manual));
        for language in [Lang::En, Lang::Ru] {
            lang::set(language);
            for i in 0..manual_text::COUNT {
                press(&mut v, Btn::Man(manual::ManAct::Chapter(i as u8)));
                draw(&mut v);
                assert_eq!(v.manual.scroll, 0, "a new chapter starts at the top");
            }
        }
        lang::set(Lang::En);
        // The long chapter scrolls, and not past its end or above its start.
        press(&mut v, Btn::Man(manual::ManAct::Chapter(6)));
        draw(&mut v);
        v.wheel(-2, Keys::default());
        assert!(v.manual.scroll > 0);
        v.wheel(1000, Keys::default());
        assert_eq!(v.manual.scroll, 0);
        v.wheel(-1000, Keys::default());
        draw(&mut v);
        let end = v.manual.scroll;
        v.wheel(-5, Keys::default());
        assert_eq!(v.manual.scroll, end);
        // Esc closes it; a "try it" button closes it too and goes there.
        v.key(KeyCode::Escape, false, Keys::default(), None);
        assert!(v.modal.is_none());
        press(&mut v, Btn::C(Cmd::Manual));
        press(&mut v, Btn::Man(manual::ManAct::Chapter(1)));
        press(&mut v, Btn::Man(manual::ManAct::Try));
        assert!(
            v.modal.is_none() && v.windowed(),
            "the timing window opened"
        );
    }

    #[test]
    fn every_sv_preset_sets_where_and_how_it_writes() {
        let mut v = view();
        press(&mut v, Btn::S(SAct::Tab(1)));
        for i in 0..curves::PRESET_COUNT {
            press(&mut v, Btn::S(SAct::Preset(i as u8)));
            let p = &v.svt.params;
            assert!(p.write_sv, "{i}");
            assert!(
                p.lines != p.hits,
                "{i}: written on lines or on notes, not both"
            );
        }
        press(&mut v, Btn::S(SAct::Preset(4)));
        assert!(matches!(
            v.svt.params.shape,
            crate::editor::svt::Shape::Between(crate::editor::sv_curves::Curve::Exp(n)) if n > 1.0
        ));
        press(&mut v, Btn::S(SAct::Preset(9)));
        assert!(v.svt.params.hits);
        // A number past the list does nothing.
        press(&mut v, Btn::S(SAct::Preset(200)));
        assert!(v.svt.params.hits);
        draw(&mut v);
    }

    #[test]
    fn the_sv_tool_applies_between_selected_lines_and_previews_first() {
        let (mut v, ids) = lines_view();
        v.sel_line = Some(ids[0]);
        v.sel_extra = [ids[2]].into();
        press(&mut v, Btn::S(SAct::Tab(1)));
        press(&mut v, Btn::S(SAct::Preset(1)));
        draw(&mut v);
        // The ghost shows what Apply would do and nothing has been written yet.
        let ghost = v.svt.ghost.as_ref().unwrap().result.as_ref().unwrap();
        assert_eq!(
            ghost.timing.iter().map(|p| p.time).collect::<Vec<_>>(),
            vec![999.0, 1999.0, 2999.0]
        );
        assert_eq!(greens(&v).len(), 4, "the map's own line at 4000 stays");
        press(&mut v, Btn::S(SAct::Apply));
        let g = greens(&v);
        assert_eq!(g.len(), 4);
        let mid = g
            .iter()
            .find(|p| p.time == 1999.0)
            .expect("beside the old line");
        assert!((mid.sv() - 2f64.sqrt()).abs() < 1e-6, "{}", mid.sv());
        assert!(v.ed.undo());
        assert_eq!(
            greens(&v).iter().find(|p| p.time == 2000.0).unwrap().sv(),
            1.5
        );
    }

    #[test]
    fn alt_adds_to_the_selected_dots_instead_of_scaling_them() {
        let (mut v, ids) = lines_view();
        v.sel_line = Some(ids[1]);
        v.sel_extra = [ids[2]].into();
        let range = v.graph_range();
        let (x, y) = (v.x_of(2000.0), v.sv_y(1.5, range));
        let drag = |v: &mut View, keys: Keys| {
            v.mouse_down(x, y, keys, None);
            v.mouse_move(x, y - 10.0, keys);
            v.mouse_up(x, y - 10.0, keys);
            let sv = |id| v.ed.chart.timing_point(id).unwrap().sv();
            (sv(ids[1]), sv(ids[2]))
        };
        let (a, b) = drag(&mut v, Keys::default());
        assert!(
            ((b / 2.0) - (a / 1.5)).abs() < 0.02,
            "scaled together: {a} {b}"
        );
        assert!(v.ed.undo());
        let alt = Keys {
            alt: true,
            ..Keys::default()
        };
        let (a, b) = drag(&mut v, alt);
        assert!(
            ((b - 2.0) - (a - 1.5)).abs() < 0.02,
            "moved by the same amount: {a} {b}"
        );
    }

    #[test]
    fn the_volume_view_edits_volume_and_leaves_the_speed() {
        let (mut v, ids) = lines_view();
        press(&mut v, Btn::S(SAct::VolumeView(true)));
        draw(&mut v);
        let line = v.ed.chart.timing_point(ids[1]).unwrap().clone();
        let (x, y) = (
            v.x_of(2000.0),
            v.val_y(f64::from(line.volume), v.graph_range()),
        );
        v.mouse_down(x, y, Keys::default(), None);
        v.mouse_move(x, y + 15.0, Keys::default());
        v.mouse_move(x, y + 30.0, Keys::default());
        v.mouse_up(x, y + 30.0, Keys::default());
        let now = v.ed.chart.timing_point(ids[1]).unwrap().clone();
        assert!(
            now.volume < line.volume,
            "{} -> {}",
            line.volume,
            now.volume
        );
        assert_eq!(now.sv(), line.sv());
        assert_eq!(now.time, line.time);
        assert!(v.ed.undo());
        assert_eq!(v.ed.chart.timing_point(ids[1]).unwrap().volume, line.volume);
        draw(&mut v);
    }

    #[test]
    fn a_number_of_the_sv_tool_is_typed_and_stepped() {
        let mut v = view();
        v.run(Cmd::Mode(Mode::Timing), None);
        press(&mut v, Btn::S(SAct::Tab(1)));
        draw(&mut v);
        press(&mut v, Btn::Edit(Field::S(SF::Offset)));
        for k in [KeyCode::Minus, KeyCode::Digit2, KeyCode::Enter] {
            v.key(k, false, Keys::default(), None);
        }
        assert_eq!(v.svt.params.offset, -2);
        press(&mut v, Btn::Inc(Field::S(SF::Offset)));
        assert_eq!(v.svt.params.offset, -1);
        press(&mut v, Btn::S(SAct::Shape(2)));
        press(&mut v, Btn::Inc(Field::S(SF::Exponent)));
        assert!(
            matches!(v.svt.params.shape, Shape::Between(Curve::Exp(n)) if (n - 0.6).abs() < 1e-9)
        );
        draw(&mut v);
    }

    /// A Timing-mode view with green lines at 1000, 2000 and 3000 ms, velocities 1.0, 1.5, 2.0.
    fn lines_view() -> (View, Vec<u64>) {
        let mut v = view();
        v.run(Cmd::Mode(Mode::Timing), None);
        let ids = [(1000.0, 1.0), (2000.0, 1.5), (3000.0, 2.0)]
            .iter()
            .map(|(t, sv)| v.ed.add_green(*t, *sv).unwrap())
            .collect();
        draw(&mut v);
        (v, ids)
    }

    #[test]
    fn a_box_in_the_graph_selects_lines_and_they_drag_together_as_one_undo() {
        let (mut v, ids) = lines_view();
        let g = v.layout.graph;
        // A box around all three dots.
        let (a, b) = (
            (v.x_of(900.0), g[1] + 4.0),
            (v.x_of(3100.0), g[1] + g[3] - 4.0),
        );
        v.mouse_down(a.0, a.1, Keys::default(), None);
        v.mouse_move(b.0, b.1, Keys::default());
        draw(&mut v);
        v.mouse_up(b.0, b.1, Keys::default());
        let mut got = v.selected_lines();
        got.sort_unstable();
        for id in &ids {
            assert!(got.contains(id), "{got:?}");
        }
        draw(&mut v);
        // Drag the middle one right and up: every selected line follows, in one step.
        let range = v.graph_range();
        let mid = v.ed.chart.timing_point(ids[1]).unwrap().clone();
        let (px, py) = (v.x_of(mid.time), v.sv_y(mid.sv(), range));
        let t0: Vec<f64> = ids
            .iter()
            .map(|i| v.ed.chart.timing_point(*i).unwrap().time)
            .collect();
        let sv0: Vec<f64> = ids
            .iter()
            .map(|i| v.ed.chart.timing_point(*i).unwrap().sv())
            .collect();
        v.mouse_down(px, py, Keys::default(), None);
        for i in 1..=5 {
            v.mouse_move(px + 5.0 * i as f32, py - 4.0 * i as f32, Keys::default());
        }
        v.mouse_up(px + 25.0, py - 20.0, Keys::default());
        for (i, id) in ids.iter().enumerate() {
            let p = v.ed.chart.timing_point(*id).unwrap();
            assert!(p.time > t0[i], "line {i} moved later");
            assert!(p.sv() > sv0[i], "line {i} got faster");
        }
        assert!(v.ed.undo());
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(v.ed.chart.timing_point(*id).unwrap().time, t0[i]);
        }
        assert!(matches!(v.drag, Drag::None) && v.g_frozen.is_none());
    }

    #[test]
    fn shift_click_adds_to_the_selection_and_the_tools_set_scale_and_ramp() {
        let (mut v, ids) = lines_view();
        let range = v.graph_range();
        let at = |v: &View, id: u64| {
            let p = v.ed.chart.timing_point(id).unwrap();
            (v.x_of(p.time), v.sv_y(p.sv(), range))
        };
        let shift = Keys {
            shift: true,
            ..Keys::default()
        };
        // (A plain click on a dot of a curve would pick the whole curve; Shift takes single lines.)
        for id in &ids {
            let (x, y) = at(&v, *id);
            v.mouse_down(x, y, shift, None);
            v.mouse_up(x, y, shift);
        }
        assert_eq!(v.selected_lines().len(), 3);
        let c = draw(&mut v);
        assert!(!c.shapes.is_empty(), "the tools panel draws");
        let sv = |v: &View, i: usize| v.ed.chart.timing_point(ids[i]).unwrap().sv();
        press(&mut v, Btn::T(TAct::SetValue));
        assert!((0..3).all(|i| (sv(&v, i) - 1.0).abs() < 1e-9));
        // The ramps and multiplying live in the SV tool, one press away.
        press(&mut v, Btn::S(SAct::Tab(1)));
        assert_eq!(v.svt.tab, 1);
        press(&mut v, Btn::T(TAct::Copy));
        v.ed.time = 20000.0;
        press(&mut v, Btn::T(TAct::Paste));
        assert_eq!(v.ed.chart.timing.iter().filter(|p| !p.is_red()).count(), 7);
        press(&mut v, Btn::T(TAct::Delete));
        assert_eq!(v.ed.chart.timing.iter().filter(|p| !p.is_red()).count(), 4);
        assert!(v.selected_lines().is_empty());
    }

    #[test]
    fn keys_step_the_playhead_change_the_snap_and_ask_before_leaving_unsaved_work() {
        let mut v = view();
        let shift = Keys {
            shift: true,
            ..Keys::default()
        };
        v.ed.time = 1000.0;
        v.key(KeyCode::ArrowRight, false, Keys::default(), None);
        assert_eq!(v.ed.time, 1125.0);
        // Shift takes four snap steps at once.
        v.key(KeyCode::ArrowRight, false, shift, None);
        assert_eq!(v.ed.time, 1625.0);
        v.key(KeyCode::Digit5, false, shift, None);
        assert_eq!(v.ed.snap, 5);
        // Up and Down go between timing lines, not snaps.
        v.ed.time = 0.0;
        v.key(KeyCode::ArrowDown, false, Keys::default(), None);
        assert!(v.ed.time > 0.0, "the next line");
        // Make a change and try to leave.
        v.ed.place_hit(3000.0, false);
        assert!(v.ed.modified());
        assert!(matches!(
            v.key(KeyCode::Escape, false, Keys::default(), None),
            KeyResult::Handled
        ));
        assert_eq!(v.modal, Some(Modal::Leave));
        // Esc on the question keeps editing; the answer to leave anyway leaves.
        v.key(KeyCode::Escape, false, Keys::default(), None);
        assert!(v.modal.is_none() && !v.exit);
        v.key(KeyCode::Escape, false, Keys::default(), None);
        press(&mut v, Btn::C(Cmd::LeaveDiscard));
        assert!(v.take_exit());
        // Saving first also leaves.
        let mut w = view();
        w.ed.place_hit(3000.0, false);
        w.key(KeyCode::Escape, false, Keys::default(), None);
        press(&mut w, Btn::C(Cmd::LeaveSave));
        assert!(w.take_exit() && !w.ed.modified(), "{}", w.status);
        // A clean map leaves at once.
        let mut z = view();
        assert!(matches!(
            z.key(KeyCode::Escape, false, Keys::default(), None),
            KeyResult::Exit
        ));
    }

    #[test]
    fn a_value_can_be_typed_into_a_timing_line() {
        let mut v = view();
        draw(&mut v);
        v.sel_line = Some(v.ed.chart.timing[0].id);
        v.press(Btn::Edit(Field::Bpm), Keys::default(), None);
        for k in [KeyCode::Digit1, KeyCode::Digit8, KeyCode::Digit0] {
            v.key(k, false, Keys::default(), None);
        }
        v.key(KeyCode::Enter, false, Keys::default(), None);
        assert_eq!(v.ed.chart.timing[0].bpm(), Some(180.0));
        // Not allowed: a zero velocity.
        v.sel_line = Some(v.ed.chart.timing[1].id);
        v.press(Btn::Edit(Field::Sv), Keys::default(), None);
        v.key(KeyCode::Digit0, false, Keys::default(), None);
        v.key(KeyCode::Enter, false, Keys::default(), None);
        assert_eq!(v.ed.chart.timing[1].sv(), 2.0);
    }

    fn press(v: &mut View, b: Btn) {
        v.press(b, Keys::default(), None);
    }

    #[test]
    fn the_gimmick_panel_applies_previews_and_cancels() {
        let mut v = view();
        press(&mut v, Btn::C(Cmd::Mode(Mode::Gimmicks)));
        assert_eq!(v.mode, Mode::Gimmicks);
        v.ed.time = 4000.0;
        let lines = v.ed.chart.timing.len();
        press(&mut v, Btn::GTool(GimmickTool::FakeNote));
        press(&mut v, Btn::GApply(Target::Playhead(true)));
        assert!(v.ed.chart.timing.len() > lines, "{}", v.status);
        assert!(!v.g_pending);
        assert!(v.ed.undo());
        // A preview can be cancelled, and kept.
        press(&mut v, Btn::GPreview);
        assert!(v.g_pending);
        press(&mut v, Btn::GCancel);
        assert_eq!(v.ed.chart.timing.len(), lines);
        press(&mut v, Btn::GPreview);
        press(&mut v, Btn::GKeep);
        assert!(!v.g_pending && v.ed.chart.timing.len() > lines);
        draw(&mut v);
    }

    #[test]
    fn the_panels_numbers_and_switches_change_what_a_tool_writes() {
        let mut v = view();
        press(&mut v, Btn::GStep(GParam::GimmickBpm, 1));
        assert_eq!(v.ed.gimmick.gimmick_bpm, 61_000.0);
        press(&mut v, Btn::GStep(GParam::KatSpacing, 1));
        assert_eq!(
            (v.ed.gimmick.kat_spacing1_ms, v.ed.gimmick.kat_spacing2_ms),
            (2, 4)
        );
        press(&mut v, Btn::GFlag(GFlag::Hide));
        assert!(!v.ed.gimmick.hide_note);
    }

    #[test]
    fn the_keyframe_page_makes_a_tko_and_keys_a_track() {
        use crate::editor::keyframes::Subject;
        use crate::tko::gimmick::FieldProp;
        let mut v = view();
        press(&mut v, Btn::GPage(2));
        assert_eq!(v.mode, Mode::Effects);
        draw(&mut v);
        assert!(!v.ed.is_tko());
        // A click on a plain chart's graph still means a green line, not a key.
        press(&mut v, Btn::K(KAct::MakeTko));
        assert!(v.ed.is_tko(), "{}", v.status);
        assert!(v.ed.path.extension().is_some_and(|e| e == "tko"));
        press(&mut v, Btn::K(KAct::Prop(3)));
        assert_eq!(v.subject(), Subject::Field(FieldProp::Zoom));
        draw(&mut v);
        // Click the graph twice: two keys on the zoom track.
        let g = v.layout.graph;
        click(&mut v, 300.0, g[1] + 20.0);
        click(&mut v, 500.0, g[1] + 50.0);
        let keys = v.ed.track_of(v.subject()).unwrap().keys.clone();
        assert_eq!(keys.len(), 2);
        assert!(keys[0].t < keys[1].t);
        assert_eq!(v.k_sel, Some(1));
        // Drag the second key: one undo step.
        v.mouse_down(500.0, g[1] + 50.0, Keys::default(), None);
        v.mouse_move(560.0, g[1] + 30.0, Keys::default());
        v.mouse_up(560.0, g[1] + 30.0, Keys::default());
        let moved = v.ed.track_of(v.subject()).unwrap().keys[1];
        assert!(moved.t > keys[1].t);
        assert!(v.ed.undo());
        assert_eq!(v.ed.track_of(v.subject()).unwrap().keys[1].t, keys[1].t);
        // The easing and value of the selected key, then deleting it.
        press(&mut v, Btn::K(KAct::Ease));
        assert_ne!(
            v.ed.track_of(v.subject()).unwrap().keys[1].ease,
            keys[1].ease
        );
        press(&mut v, Btn::K(KAct::Value(1)));
        assert!(v.ed.track_of(v.subject()).unwrap().keys[1].v > keys[1].v);
        press(&mut v, Btn::K(KAct::DeleteKey));
        assert_eq!(v.ed.track_of(v.subject()).unwrap().keys.len(), 1);
        // Right-click deletes too. A group needs notes selected first.
        let (kx, ky) = {
            let r = v.key_range();
            (
                v.x_of(v.ed.track_of(v.subject()).unwrap().keys[0].t),
                v.key_y(v.ed.track_of(v.subject()).unwrap().keys[0].v, r),
            )
        };
        v.right_click(kx, ky);
        assert!(v.ed.track_of(v.subject()).is_none_or(|t| t.is_empty()));
        press(&mut v, Btn::K(KAct::NewGroup));
        assert!(v.status.contains("select"), "{}", v.status);
        v.ed.select_all();
        press(&mut v, Btn::K(KAct::NewGroup));
        assert!(!v.k_field);
        draw(&mut v);
        press(&mut v, Btn::K(KAct::KeyHere));
        assert_eq!(v.ed.track_of(v.subject()).unwrap().keys.len(), 1);
        // The preview still builds from the chart with its gimmick.
        draw(&mut v);
        assert!(v.game.is_some(), "{:?}", v.game_err);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn a_wall_without_notes_says_why_and_writes_nothing() {
        let mut v = view();
        v.ed.chart.objects.clear();
        press(&mut v, Btn::GTool(GimmickTool::AntiBarline));
        let lines = v.ed.chart.timing.len();
        press(&mut v, Btn::GApply(Target::Selection));
        assert_eq!(v.ed.chart.timing.len(), lines);
        assert!(v.status.contains("no notes"), "{}", v.status);
    }

    #[test]
    fn the_wheel_steps_zooms_and_changes_the_snap() {
        let mut v = view();
        v.ed.time = 1000.0;
        v.wheel(-1, Keys::default());
        assert_eq!(v.ed.time, 1125.0);
        let before = v.ms_per_px;
        v.wheel(
            1,
            Keys {
                alt: true,
                ..Keys::default()
            },
        );
        assert!(v.ms_per_px < before, "Alt+wheel zooms");
        v.wheel(
            1,
            Keys {
                ctrl: true,
                ..Keys::default()
            },
        );
        assert_eq!(v.ed.snap, 5, "Ctrl+wheel changes the snap");
        v.ed.snap = 4;
        v.ed.time = 1000.0;
        v.wheel(
            -1,
            Keys {
                shift: true,
                ..Keys::default()
            },
        );
        assert_eq!(v.ed.time, 1500.0, "Shift+wheel takes four steps");
        // Ctrl+Alt+wheel changes the tool.
        v.ed.tool = Tool::Note;
        v.wheel(
            -1,
            Keys {
                ctrl: true,
                alt: true,
                ..Keys::default()
            },
        );
        assert_eq!(v.ed.tool, Tool::Roll);
    }

    use crate::tko::graph::Kind;

    /// A view of a `.tko` map on the Nodes page.
    fn nodes_view() -> View {
        let mut v = view();
        press(&mut v, Btn::GPage(2));
        press(&mut v, Btn::K(KAct::MakeTko));
        press(&mut v, Btn::GPage(3));
        draw(&mut v);
        v
    }

    fn put(v: &mut View, kind: Kind, x: f32, y: f32) -> u32 {
        let id = v.ed.add_node(kind, x, y).unwrap();
        draw(v);
        id
    }

    /// Drags from the output of `from` to input `port` of `to` with the mouse.
    fn wire_up(v: &mut View, from: u32, to: u32, port: usize) {
        let a = v.out_pos(v.ed.node(from).unwrap());
        let b = v.in_pos(v.ed.node(to).unwrap(), port);
        v.mouse_down(a.0, a.1, Keys::default(), None);
        v.mouse_move((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, Keys::default());
        v.mouse_up(b.0, b.1, Keys::default());
    }

    #[test]
    fn the_nodes_page_needs_a_tko_and_replaces_the_timeline_with_a_canvas() {
        let mut v = view();
        press(&mut v, Btn::GPage(3));
        draw(&mut v);
        assert!(!v.node_mode(), "a plain .osu has no nodes");
        let mut v = nodes_view();
        assert!(v.node_mode(), "{}", v.status);
        let c = draw(&mut v);
        assert!(!c.shapes.is_empty());
        // A click on the strip moves the playhead; one on the canvas does not.
        let s = v.layout.strip;
        click(&mut v, 400.0, s[1] + 10.0);
        assert!((v.ed.time - v.t_at(400.0)).abs() < 1.0);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn nodes_are_added_from_the_palette_wired_by_dragging_and_edited() {
        let mut v = nodes_view();
        press(&mut v, Btn::N(NAct::Add(Kind::OnBeat)));
        let beat = v.n_sel.unwrap();
        press(&mut v, Btn::N(NAct::Cat(4)));
        press(&mut v, Btn::N(NAct::Add(Kind::Pulse)));
        let pulse = v.n_sel.unwrap();
        assert_ne!(beat, pulse);
        // Far apart, so the boxes don't cover each other's dots.
        v.ed.move_node(beat, 20.0, 20.0, false);
        v.ed.move_node(pulse, 260.0, 20.0, false);
        draw(&mut v);
        let wave = put(&mut v, Kind::Wave, 20.0, 90.0);
        // A number into a trigger input is refused with a reason, and is no undo step.
        wire_up(&mut v, wave, pulse, 0);
        assert_eq!(v.ed.node(pulse).unwrap().inputs, [None, None]);
        assert!(v.status.contains("trigger"), "{}", v.status);
        wire_up(&mut v, beat, pulse, 0);
        assert_eq!(v.ed.node(pulse).unwrap().inputs, [Some(beat), None]);
        // The wire is drawn and the preview still builds.
        draw(&mut v);
        assert!(v.game.is_some(), "{:?}", v.game_err);
        // Numbers, target and easing of the selected node.
        v.n_sel = Some(pulse);
        press(&mut v, Btn::N(NAct::Param(0, 1)));
        assert!(v.ed.node(pulse).unwrap().param(0) > 0.1);
        press(
            &mut v,
            Btn::N(NAct::SetTarget(crate::tko::graph::Target::Field(
                crate::tko::gimmick::FieldProp::Zoom,
            ))),
        );
        assert!(v.ed.node(pulse).unwrap().target.is_some());
        let ease = v.ed.node(pulse).unwrap().ease;
        press(&mut v, Btn::N(NAct::Ease));
        assert_ne!(v.ed.node(pulse).unwrap().ease, ease);
        press(&mut v, Btn::N(NAct::NoTarget));
        assert!(v.ed.node(pulse).unwrap().target.is_none());
        // Right-click on an input dot lifts the wire; undo puts it back.
        let p = v.in_pos(v.ed.node(pulse).unwrap(), 0);
        v.right_click(p.0, p.1);
        assert_eq!(v.ed.node(pulse).unwrap().inputs, [None, None]);
        assert!(v.ed.undo());
        assert_eq!(v.ed.node(pulse).unwrap().inputs, [Some(beat), None]);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn a_node_number_is_typed_picked_from_a_list_and_made_from_a_recipe() {
        use crate::tko::graph::Target;
        use crate::tko::recipes::Recipe;
        let mut v = nodes_view();
        press(&mut v, Btn::N(NAct::Cat(4)));
        press(&mut v, Btn::N(NAct::Add(Kind::Pulse)));
        let pulse = v.n_sel.unwrap();
        // Click a number, type over it, Enter applies it (clamped by the node's own range).
        press(&mut v, Btn::N(NAct::Edit(0)));
        assert!(v.n_typing.is_some());
        for k in [KeyCode::Digit0, KeyCode::Period, KeyCode::Digit2] {
            v.key(k, false, Keys::default(), None);
        }
        v.key(KeyCode::Enter, false, Keys::default(), None);
        assert!(v.n_typing.is_none());
        assert!((v.ed.node(pulse).unwrap().param(0) - 0.2).abs() < 1e-9);
        // Escape throws the typing away.
        press(&mut v, Btn::N(NAct::Edit(0)));
        v.key(KeyCode::Digit9, false, Keys::default(), None);
        v.key(KeyCode::Escape, false, Keys::default(), None);
        assert!((v.ed.node(pulse).unwrap().param(0) - 0.2).abs() < 1e-9);
        // The target comes from a dropdown that closes once something is picked.
        press(&mut v, Btn::N(NAct::Open(nodes::Pick::Target)));
        assert_eq!(v.n_pick, Some(nodes::Pick::Target));
        draw(&mut v);
        press(
            &mut v,
            Btn::N(NAct::SetTarget(Target::Field(
                crate::tko::gimmick::FieldProp::Glitch,
            ))),
        );
        assert_eq!(v.n_pick, None);
        assert_eq!(
            v.ed.node(pulse).unwrap().target,
            Some(Target::Field(crate::tko::gimmick::FieldProp::Glitch))
        );
        // A recipe drops its wired nodes in one undo step.
        let before = v.ed.gimmick().unwrap().graph.nodes.len();
        press(&mut v, Btn::N(NAct::AddRecipe(Recipe::KiaiDrop)));
        let after = v.ed.gimmick().unwrap().graph.nodes.len();
        assert_eq!(after, before + 4);
        draw(&mut v);
        assert!(v.game.is_some(), "{:?}", v.game_err);
        v.ed.undo();
        assert_eq!(v.ed.gimmick().unwrap().graph.nodes.len(), before);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn m_drops_a_mark_on_the_playhead_and_shift_m_takes_it_away() {
        let mut v = nodes_view();
        v.ed.time = 3000.0;
        v.run(Cmd::EffectMark, None);
        assert_eq!(v.ed.gimmick().unwrap().marks.len(), 1);
        v.run(Cmd::EffectMark, None);
        assert_eq!(v.ed.gimmick().unwrap().marks.len(), 1, "no doubles");
        draw(&mut v);
        v.ed.time = 3200.0;
        v.run(Cmd::EffectUnmark, None);
        assert!(v.ed.gimmick().unwrap().marks.is_empty());
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn a_wire_picked_up_by_its_end_moves_and_dropping_on_nothing_removes_it() {
        let mut v = nodes_view();
        let beat = put(&mut v, Kind::OnBeat, 20.0, 20.0);
        let a = put(&mut v, Kind::Once, 260.0, 20.0);
        let b = put(&mut v, Kind::Once, 260.0, 90.0);
        wire_up(&mut v, beat, a, 0);
        assert_eq!(v.ed.node(a).unwrap().inputs, [Some(beat)]);
        // Take it off `a` and drop it on `b`.
        let from = v.in_pos(v.ed.node(a).unwrap(), 0);
        let to = v.in_pos(v.ed.node(b).unwrap(), 0);
        v.mouse_down(from.0, from.1, Keys::default(), None);
        assert!(matches!(v.drag, Drag::Wire { from } if from == beat));
        v.mouse_up(to.0, to.1, Keys::default());
        assert_eq!(v.ed.node(a).unwrap().inputs, [None]);
        assert_eq!(v.ed.node(b).unwrap().inputs, [Some(beat)]);
        // Dropped on nothing: gone.
        let from = v.in_pos(v.ed.node(b).unwrap(), 0);
        v.mouse_down(from.0, from.1, Keys::default(), None);
        v.mouse_up(from.0 + 5.0, from.1 + 60.0, Keys::default());
        assert_eq!(v.ed.node(b).unwrap().inputs, [None]);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn nodes_drag_the_canvas_pans_and_zooms_and_delete_unwires() {
        let mut v = nodes_view();
        let a = put(&mut v, Kind::OnStart, 20.0, 20.0);
        let b = put(&mut v, Kind::Once, 260.0, 20.0);
        wire_up(&mut v, a, b, 0);
        // Drag a node by its header: one undo step.
        let r = v.node_rect(v.ed.node(b).unwrap());
        let (x, y) = (r[0] + 30.0, r[1] + 4.0);
        v.mouse_down(x, y, Keys::default(), None);
        for i in 1..=8 {
            v.mouse_move(x + i as f32 * 6.0, y + i as f32 * 3.0, Keys::default());
        }
        v.mouse_up(x + 48.0, y + 24.0, Keys::default());
        let moved = v.ed.node(b).unwrap().x;
        assert!(moved > 260.0 && v.n_sel == Some(b));
        assert!(v.ed.undo());
        assert_eq!(v.ed.node(b).unwrap().x, 260.0);
        // Dragging empty canvas pans it; the wheel zooms about the pointer.
        let n = v.layout.nodes;
        let (ex, ey) = (n[0] + n[2] - 40.0, n[1] + n[3] - 40.0);
        v.mouse_down(ex, ey, Keys::default(), None);
        v.mouse_move(ex - 30.0, ey - 10.0, Keys::default());
        v.mouse_up(ex - 30.0, ey - 10.0, Keys::default());
        assert!(v.n_pan.0 > 0.0 && v.n_pan.1 > 0.0);
        assert!(v.n_sel.is_none());
        let before = v.to_world(ex, ey);
        v.mouse = (ex, ey);
        v.wheel(3, Keys::default());
        assert!(v.n_zoom > 1.0 && v.n_zoom <= nodes::MAX_ZOOM);
        let after = v.to_world(ex, ey);
        assert!((before.0 - after.0).abs() < 0.01 && (before.1 - after.1).abs() < 0.01);
        for _ in 0..60 {
            v.wheel(-1, Keys::default());
        }
        assert_eq!(v.n_zoom, nodes::MIN_ZOOM);
        // Far out the canvas still draws (boxes shrink to blocks), and Fit frames every node.
        draw(&mut v);
        v.node_fit();
        assert!(v.n_zoom >= nodes::MIN_ZOOM && v.n_zoom <= 1.0);
        draw(&mut v);
        // Deleting a node (right-click on its body) unwires what it fed.
        let r = v.node_rect(v.ed.node(a).unwrap());
        v.right_click(r[0] + r[2] / 2.0, r[1] + 3.0);
        assert!(v.ed.node(a).is_none());
        assert_eq!(v.ed.node(b).unwrap().inputs, [None]);
        // Delete key on the selected node.
        v.n_sel = Some(b);
        v.key(KeyCode::Delete, false, Keys::default(), None);
        assert!(v.ed.node(b).is_none());
        assert!(v.ed.chart.dirty);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn a_node_lights_up_while_it_fires_in_the_preview() {
        let mut v = nodes_view();
        let beat = put(&mut v, Kind::OnBeat, 20.0, 20.0);
        let start = v.ed.chart.timing[0].time;
        v.mark_stale();
        v.set_time(start + 50.0);
        draw(&mut v);
        assert!(v.lit(beat), "just after a beat");
        v.set_time(start + 400.0);
        draw(&mut v);
        assert!(!v.lit(beat), "between beats");
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn the_seed_steps_and_saves_with_the_map() {
        let mut v = nodes_view();
        press(&mut v, Btn::N(NAct::Seed(1)));
        press(&mut v, Btn::N(NAct::Seed(1)));
        assert_eq!(v.ed.gimmick().unwrap().graph.seed, 2);
        press(&mut v, Btn::N(NAct::Seed(-1)));
        assert_eq!(v.ed.gimmick().unwrap().graph.seed, 1);
        put(&mut v, Kind::Random, 20.0, 20.0);
        let text = v.ed.chart.to_text();
        let back = crate::editor::chart::Chart::parse_tko(&text).unwrap();
        assert_eq!(back.gimmick.unwrap().graph, v.ed.gimmick().unwrap().graph);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    fn key(v: &mut View, code: KeyCode) {
        v.key(code, false, Keys::default(), None);
    }

    fn ctrl() -> Keys {
        Keys {
            ctrl: true,
            ..Keys::default()
        }
    }

    #[test]
    fn w_e_and_r_set_the_next_note_or_the_selected_ones() {
        let mut v = view();
        // Nothing selected: they set what the next note is.
        key(&mut v, KeyCode::KeyW);
        assert!(v.ed.kat);
        key(&mut v, KeyCode::KeyE);
        assert!(v.ed.big);
        draw(&mut v);
        let y = lane_y(&v);
        click_t(&mut v, 3130.0, y);
        let made =
            v.ed.chart
                .objects
                .iter()
                .find(|o| o.time == 3125.0)
                .unwrap()
                .clone();
        assert!(made.taiko() == Taiko::Kat && made.big());
        // With a note selected they change that note, and toggle back.
        v.ed.selection = [made.id].into();
        key(&mut v, KeyCode::KeyW);
        assert_eq!(v.ed.chart.object(made.id).unwrap().taiko(), Taiko::Don);
        key(&mut v, KeyCode::KeyR);
        assert_eq!(v.ed.chart.object(made.id).unwrap().taiko(), Taiko::Kat);
        key(&mut v, KeyCode::KeyE);
        assert!(!v.ed.chart.object(made.id).unwrap().big());
        assert!(v.ed.undo(), "each is an undo step");
    }

    #[test]
    fn a_click_on_a_note_selects_it_with_the_note_tool_and_a_second_one_goes_to_it() {
        let mut v = view();
        draw(&mut v);
        assert_eq!(v.ed.tool, Tool::Note, "notes are placed from the start");
        let (x, y) = (v.x_of(1000.0), lane_y(&v));
        let count = v.ed.chart.objects.len();
        click(&mut v, x, y);
        assert_eq!(
            v.ed.chart.objects.len(),
            count,
            "no second note on top of it"
        );
        assert_eq!(v.ed.selection.len(), 1);
        v.ed.time = 5000.0;
        click(&mut v, x, y);
        assert_eq!(v.ed.time, 1000.0, "double click puts the playhead on it");
    }

    #[test]
    fn a_roll_can_be_made_with_two_clicks_and_right_click_cancels() {
        let mut v = view();
        draw(&mut v);
        v.ed.tool = Tool::Roll;
        let y = lane_y(&v);
        click_t(&mut v, 5000.0, y);
        assert_eq!(v.roll_start, Some(5000.0), "the start waits for the end");
        assert!(v.placing().is_some());
        click_t(&mut v, 5500.0, y);
        assert!(v.roll_start.is_none());
        let roll =
            v.ed.chart
                .objects
                .iter()
                .find(|o| o.time == 5000.0)
                .unwrap();
        assert_eq!(roll.taiko(), Taiko::Roll);
        assert_eq!(v.ed.end_of(roll), 5500.0);
        // Cancelled before the end.
        click_t(&mut v, 7000.0, y);
        v.right_click(v.x_of(7200.0), y);
        assert!(v.roll_start.is_none());
        assert!(v.ed.chart.objects.iter().all(|o| o.time != 7000.0));
        // A denden the same way.
        v.ed.tool = Tool::Swell;
        click_t(&mut v, 6000.0, y);
        click_t(&mut v, 6500.0, y);
        assert!(
            v.ed.chart
                .objects
                .iter()
                .any(|o| o.time == 6000.0 && o.taiko() == Taiko::Swell)
        );
    }

    #[test]
    fn shift_ignores_the_grid_while_dragging() {
        let mut v = view();
        draw(&mut v);
        let y = lane_y(&v);
        let shift = Keys {
            shift: true,
            ..Keys::default()
        };
        let (a, b) = (v.x_of(1000.0), v.x_of(1237.0));
        v.mouse_down(a, y, Keys::default(), None);
        v.mouse_move(b, y, shift);
        v.mouse_up(b, y, shift);
        assert!(v.ed.chart.objects.iter().any(|o| o.time == 1237.0));
    }

    #[test]
    fn the_middle_button_pans_the_timeline() {
        let mut v = view();
        draw(&mut v);
        let (x, y) = (400.0, lane_y(&v));
        let start = v.view_start;
        v.middle(true, x, y);
        v.mouse_move(x - 50.0, y, Keys::default());
        v.middle(false, x - 50.0, y);
        assert_eq!(v.view_start, start + 50.0 * v.ms_per_px);
        assert!(matches!(v.drag, Drag::None));
    }

    #[test]
    fn bookmarks_are_keys_stay_in_the_file_and_make_the_map_modified() {
        let mut v = view();
        v.ed.time = 2000.0;
        v.key(KeyCode::KeyB, false, ctrl(), None);
        assert_eq!(v.ed.bookmarks, [2000.0]);
        assert!(v.ed.modified());
        v.ed.time = 4000.0;
        v.key(KeyCode::KeyB, false, ctrl(), None);
        v.key(KeyCode::ArrowLeft, false, ctrl(), None);
        assert_eq!(v.ed.time, 2000.0);
        v.key(KeyCode::ArrowRight, false, ctrl(), None);
        assert_eq!(v.ed.time, 4000.0);
        v.save();
        assert!(!v.ed.modified(), "{}", v.status);
        let text = std::fs::read_to_string(&v.ed.path).unwrap();
        assert!(text.contains("Bookmarks:2000,4000"), "{text}");
        let again = Editor::open(v.ed.path.clone()).unwrap();
        assert_eq!(again.bookmarks, [2000.0, 4000.0]);
        // Removing the nearest one.
        v.ed.time = 4100.0;
        v.key(
            KeyCode::KeyB,
            false,
            Keys {
                ctrl: true,
                shift: true,
                ..Keys::default()
            },
            None,
        );
        assert_eq!(v.ed.bookmarks, [2000.0]);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn selection_keys_nudge_clone_reverse_and_flip() {
        let mut v = view();
        v.ed.select_all();
        let n = v.ed.chart.objects.len();
        v.key(KeyCode::KeyD, false, ctrl(), None);
        assert!(v.ed.chart.objects.len() > n, "{}", v.status);
        let mut w = view();
        w.ed.selection = w.ed.chart.objects.iter().take(3).map(|o| o.id).collect();
        let before: Vec<f64> = w.ed.chart.objects.iter().take(3).map(|o| o.time).collect();
        w.key(KeyCode::KeyK, false, Keys::default(), None);
        let after: Vec<f64> = w.ed.chart.objects.iter().take(3).map(|o| o.time).collect();
        assert!(after.iter().zip(&before).all(|(a, b)| a > b), "later");
        w.key(KeyCode::KeyJ, false, Keys::default(), None);
        let back: Vec<f64> = w.ed.chart.objects.iter().take(3).map(|o| o.time).collect();
        assert_eq!(back, before);
        w.key(KeyCode::KeyG, false, ctrl(), None);
        let rev: Vec<f64> = w.ed.chart.objects.iter().take(3).map(|o| o.time).collect();
        assert_eq!(rev, before, "the times are the same set");
        let kinds = |v: &View| -> Vec<Taiko> {
            v.ed.chart
                .objects
                .iter()
                .take(3)
                .map(|o| o.taiko())
                .collect()
        };
        let a = kinds(&w);
        w.key(KeyCode::KeyQ, false, Keys::default(), None);
        assert_ne!(a, kinds(&w), "flip");
    }

    #[test]
    fn ctrl_l_asks_twice_then_throws_the_changes_away() {
        let mut v = view();
        let count = v.ed.chart.objects.len();
        assert!(v.ed.place_hit(7000.0, false).is_some());
        v.key(KeyCode::KeyL, false, ctrl(), None);
        assert_eq!(v.ed.chart.objects.len(), count + 1, "asked first");
        v.key(KeyCode::KeyL, false, ctrl(), None);
        assert_eq!(v.ed.chart.objects.len(), count);
        assert!(!v.ed.modified());
    }

    #[test]
    fn the_function_keys_switch_modes_and_the_panel_follows() {
        let mut v = view();
        for (k, m) in [
            (KeyCode::F2, Mode::Gimmicks),
            (KeyCode::F3, Mode::Timing),
            (KeyCode::F4, Mode::Effects),
            (KeyCode::F1, Mode::Compose),
        ] {
            key(&mut v, k);
            assert_eq!(v.mode, m);
            let c = draw(&mut v);
            assert!(!c.shapes.is_empty());
        }
        // Timing gives the velocity graph the room; Compose has less of it.
        let compose = Layout::new(853.0, Mode::Compose, false).graph[3];
        let timing = Layout::new(853.0, Mode::Timing, false).graph[3];
        assert!(timing > compose);
        // In Compose the graph is read only: a click there adds no line.
        let g = v.layout.graph;
        let lines = v.ed.chart.timing.len();
        click(&mut v, 300.0, g[1] + g[3] / 2.0);
        assert_eq!(v.ed.chart.timing.len(), lines);
        assert!(v.status.contains("Timing mode"), "{}", v.status);
    }

    #[test]
    fn the_menus_open_run_and_close() {
        let mut v = view();
        draw(&mut v);
        // Click "File", then "Save".
        let (r, _) = *v.buttons.iter().find(|(_, b)| *b == Btn::Menu(0)).unwrap();
        click(&mut v, r[0] + 3.0, r[1] + 3.0);
        assert_eq!(v.menu, Some(0));
        assert!(v.ed.place_hit(7000.0, false).is_some());
        assert!(v.ed.modified());
        draw(&mut v);
        let r = v
            .buttons
            .iter()
            .rev()
            .find(|(_, b)| *b == Btn::C(Cmd::Save))
            .map(|(r, _)| *r)
            .unwrap();
        click(&mut v, r[0] + 3.0, r[1] + 3.0);
        assert!(v.menu.is_none() && !v.ed.modified(), "{}", v.status);
        // Esc closes an open menu without leaving.
        v.menu = Some(1);
        draw(&mut v);
        assert!(matches!(
            v.key(KeyCode::Escape, false, Keys::default(), None),
            KeyResult::Handled
        ));
        assert!(v.menu.is_none() && !v.exit);
        // A click outside closes it and does nothing else.
        v.menu = Some(1);
        draw(&mut v);
        let count = v.ed.chart.objects.len();
        let y = lane_y(&v);
        click_t(&mut v, 9500.0, y);
        assert!(v.menu.is_none());
        assert_eq!(v.ed.chart.objects.len(), count);
        let _ = std::fs::remove_file(&v.ed.path);
    }

    #[test]
    fn every_menu_item_has_a_command_and_the_screen_draws_with_each_overlay() {
        let mut v = view();
        for m in 0..menu::MENUS.len() {
            v.menu = Some(m);
            draw(&mut v);
        }
        v.menu = None;
        for mode in Mode::ALL {
            v.set_mode(mode);
            v.sheet = true;
            draw(&mut v);
            v.sheet = false;
            v.tw = true;
            draw(&mut v);
            v.tw = false;
            v.modal = Some(Modal::Leave);
            draw(&mut v);
            v.modal = None;
            v.open_upload();
            draw(&mut v);
            v.modal = None;
            v.open_manual();
            draw(&mut v);
            v.modal = None;
        }
        // A tooltip appears after the pointer rests on a button.
        v.set_mode(Mode::Compose);
        draw(&mut v);
        let (r, _) = *v
            .buttons
            .iter()
            .find(|(_, b)| *b == Btn::C(Cmd::Play))
            .unwrap();
        v.mouse_move(r[0] + 3.0, r[1] + 3.0, Keys::default());
        v.moved_at = Instant::now() - std::time::Duration::from_secs(1);
        let before = draw(&mut v).shapes.len();
        v.moved_at = Instant::now();
        assert!(
            draw(&mut v).shapes.len() < before,
            "the tip is drawn once the pointer has rested"
        );
    }

    #[test]
    fn the_timing_window_selects_seeks_and_leaves_the_timeline_alive() {
        let mut v = view();
        press(&mut v, Btn::C(Cmd::TimingList));
        assert!(v.tw);
        draw(&mut v);
        assert!(v.windowed());
        let [_, wy, _, wh] = v.layout.window;
        assert!(
            v.layout.timeline[1] > wy + wh,
            "the timeline sits under the window"
        );
        let green =
            v.ed.chart
                .timing
                .iter()
                .find(|p| !p.is_red())
                .unwrap()
                .clone();
        let row = |v: &View, id: u64| {
            v.buttons
                .iter()
                .find(|(_, b)| *b == Btn::Line(id))
                .map(|(r, _)| *r)
                .unwrap()
        };
        let r = row(&v, green.id);
        click(&mut v, r[0] + 5.0, r[1] + 5.0);
        assert_eq!(v.sel_line, Some(green.id));
        assert_eq!(v.ed.time, green.time);
        // Ctrl adds the red row, Shift takes the range between.
        draw(&mut v);
        let red = v.ed.chart.timing.iter().find(|p| p.is_red()).unwrap().id;
        let r = row(&v, red);
        v.mouse_down(
            r[0] + 5.0,
            r[1] + 5.0,
            Keys {
                ctrl: true,
                ..Keys::default()
            },
            None,
        );
        v.mouse_up(r[0] + 5.0, r[1] + 5.0, Keys::default());
        assert_eq!(v.selected_lines().len(), 2);
        // The filter chips narrow the table.
        press(&mut v, Btn::TwFilter(1));
        assert!(
            v.tw_ids()
                .iter()
                .all(|id| v.ed.chart.timing_point(*id).unwrap().is_red())
        );
        press(&mut v, Btn::TwFilter(0));
        // The timeline still takes presses while the window is open: a note is selected.
        draw(&mut v);
        let o = v.ed.chart.objects[0].clone();
        let (x, y) = (v.x_of(o.time), lane_y(&v));
        click(&mut v, x, y);
        assert!(v.ed.selection.contains(&o.id));
        // Esc closes the window; so does the Close button.
        v.key(KeyCode::Escape, false, Keys::default(), None);
        assert!(!v.tw);
        press(&mut v, Btn::C(Cmd::TimingList));
        press(&mut v, Btn::C(Cmd::TimingList));
        assert!(!v.tw);
    }

    #[test]
    fn the_timing_window_edits_every_selected_line() {
        let mut v = view();
        press(&mut v, Btn::C(Cmd::TimingList));
        draw(&mut v);
        let all: Vec<u64> = v.ed.chart.timing.iter().map(|p| p.id).collect();
        v.sel_line = all.last().copied();
        v.sel_extra = all.iter().copied().collect();
        press(&mut v, Btn::Inc(Field::Volume));
        let vols: Vec<i32> = v.ed.chart.timing.iter().map(|p| p.volume).collect();
        assert!(vols.windows(2).all(|w| w[0] == w[1]), "{vols:?}");
        let set = v.ed.chart.timing[0].sample_set;
        press(&mut v, Btn::Inc(Field::Sample));
        let want = (set + 1).rem_euclid(4);
        assert!(v.ed.chart.timing.iter().all(|p| p.sample_set == want));
        // The index belongs to the same group; one undo takes back one edit.
        let idx = v.ed.chart.timing[0].sample_index;
        press(&mut v, Btn::Inc(Field::Index));
        assert!(v.ed.chart.timing.iter().all(|p| p.sample_index == idx + 1));
        v.ed.undo();
        assert!(v.ed.chart.timing.iter().all(|p| p.sample_index == idx));
    }

    #[test]
    fn the_timing_window_scrolls_and_follows_the_selection() {
        let mut v = view();
        for i in 0..40 {
            let _ =
                v.ed.chart
                    .insert_timing(crate::editor::chart::TimingPoint::green(
                        5000.0 + i as f64 * 100.0,
                        1.0,
                    ));
        }
        press(&mut v, Btn::C(Cmd::TimingList));
        draw(&mut v);
        let rows = v.tw_rows();
        let n = v.tw_ids().len();
        assert!(n > rows);
        v.mouse_move(100.0, 100.0, Keys::default());
        v.wheel(-2, Keys::default());
        assert_eq!(v.tw_scroll, 6);
        // Walking down past the last visible row scrolls the table along.
        let first = v.tw_ids()[0];
        v.pick_row(first, Keys::default(), None);
        for _ in 0..rows + 3 {
            assert!(v.tw_key(KeyCode::ArrowDown, Keys::default(), None));
            draw(&mut v);
        }
        let at = v
            .tw_ids()
            .iter()
            .position(|i| Some(*i) == v.sel_line)
            .unwrap();
        assert!(at >= v.tw_scroll && at < v.tw_scroll + rows);
        // The scroll bar drags.
        let [bx, by, bw, bh] = v.tw_bar;
        assert!(bh > 0.0);
        v.mouse_down(bx + bw / 2.0, by + bh, Keys::default(), None);
        assert_eq!(v.tw_scroll, n - rows);
        v.mouse_up(bx, by, Keys::default());
    }

    #[test]
    fn the_tour_walks_every_step_once_and_reports_it_ended() {
        let mut v = view();
        v.start_tour();
        assert_eq!(v.tour, Some(0));
        let mut steps = 0;
        while v.tour.is_some() && steps < 40 {
            let c = draw(&mut v);
            assert!(!c.shapes.is_empty());
            // A click that misses the card does nothing; the buttons move on.
            click(&mut v, 1.0, 1.0);
            assert!(v.tour.is_some());
            let (r, _) = *v.buttons.iter().find(|(_, b)| *b == Btn::TourNext).unwrap();
            click(&mut v, r[0] + 3.0, r[1] + 3.0);
            steps += 1;
        }
        assert!(v.tour.is_none() && steps > 5);
        assert!(v.take_tour_ended());
        assert!(!v.take_tour_ended(), "reported once");
        assert_eq!(v.mode, Mode::Compose, "the tour puts the editor back");
        // Esc skips it too.
        v.start_tour();
        v.key(KeyCode::Escape, false, Keys::default(), None);
        assert!(v.tour.is_none() && v.take_tour_ended());
    }

    #[test]
    fn the_shortcut_sheet_lists_what_the_table_binds() {
        let mut v = view();
        press(&mut v, Btn::C(Cmd::Shortcuts));
        assert!(v.sheet);
        let c = draw(&mut v);
        assert!(!c.shapes.is_empty());
        // Every keyboard binding that has a label is in the sheet's source table.
        assert!(cmd::BINDS.iter().filter(|b| !b.label.is_empty()).count() > 40);
        v.key(KeyCode::Escape, false, Keys::default(), None);
        assert!(!v.sheet);
        // The question mark opens it.
        v.key(
            KeyCode::Slash,
            false,
            Keys {
                shift: true,
                ..Keys::default()
            },
            None,
        );
        assert!(v.sheet);
    }
}
