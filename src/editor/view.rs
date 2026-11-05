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
use super::tools::{GimmickTool, SvParams, Target};
use crate::audio::{Engine, Sfx, Track};
use crate::beatmap::{self, OSU_TIMING_MS};
use crate::game::Game;
use crate::render::{Canvas, accent, rgba};
use crate::video::FRAME_VW;

mod keys;
mod nodes;
mod panel;

use keys::KAct;
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
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Btn {
    Back,
    Save,
    Undo,
    Redo,
    Tool(Tool),
    Big,
    Snap(i32),
    Rate(u32),
    Play,
    Retype(Option<bool>, Option<bool>),
    Flip,
    Resnap,
    Delete,
    AddRed,
    AddGreen,
    DeleteLine,
    Kiai,
    Omit,
    Dec(Field),
    Inc(Field),
    Edit(Field),
    /// Switches the right-hand panel between the inspector and the gimmick tools.
    Gimmicks,
    GPage(u8),
    GTool(GimmickTool),
    GStep(GParam, i32),
    GFlag(GFlag),
    GCurve(i32),
    GApply(Target),
    GGenerate,
    GPreview,
    GKeep,
    GCancel,
    GBase,
    K(KAct),
    N(NAct),
}

#[derive(Clone, Copy)]
enum Drag {
    None,
    Scrub,
    Box { from: f64, from_x: f32, to_x: f32 },
    Move { anchor: u64, grab: f64 },
    Resize { id: u64 },
    Place { start: f64, to: f64, swell: bool },
    Line { id: u64 },
    Key { index: usize },
    Node { id: u32, dx: f32, dy: f32 },
    Wire { from: u32 },
    Pan { last: (f32, f32) },
}

const TOOLS: [(&str, Tool); 5] = [
    ("1 Select", Tool::Select),
    ("2 Don", Tool::Don),
    ("3 Kat", Tool::Kat),
    ("4 Roll", Tool::Roll),
    ("5 Swell", Tool::Swell),
];

const RATES: [u32; 4] = [25, 50, 75, 100];

const DON: u32 = 0xEB452C;
const KAT: u32 = 0x438EAD;
const ROLL: u32 = 0xFCB806;
const SWELL: u32 = 0xF58A2A;
const RED_LINE: u32 = 0xE2463C;
const GREEN_LINE: u32 = 0x4FD16B;

/// Where the pieces of the screen go.
#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub bar_y: f32,
    pub preview: [f32; 4],
    pub inspector: [f32; 4],
    pub timeline: [f32; 4],
    pub graph: [f32; 4],
    /// The node canvas and the strip above it, which take the timeline's place on the Nodes page.
    pub nodes: [f32; 4],
    pub strip: [f32; 4],
}

impl Layout {
    pub fn new(w: f32) -> Layout {
        let ph = 186.0;
        let pw = ph * 16.0 / 9.0;
        Layout {
            bar_y: 24.0,
            preview: [8.0, 46.0, pw, ph],
            inspector: [pw + 16.0, 46.0, (w - pw - 24.0).max(120.0), ph],
            timeline: [8.0, 240.0, w - 16.0, 104.0],
            graph: [8.0, 350.0, w - 16.0, 80.0],
            strip: [8.0, 240.0, w - 16.0, 20.0],
            nodes: [8.0, 264.0, w - 16.0, 166.0],
        }
    }
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
    exit_armed: bool,
    pub sel_line: Option<u64>,
    typing: Option<(Field, String)>,
    mouse: (f32, f32),
    pub width: f32,
    /// The right-hand panel shows the gimmick tools.
    pub gimmicks: bool,
    g_page: u8,
    g_tool: GimmickTool,
    g_sv: SvParams,
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
    n_clip: usize,
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
            layout: Layout::new(853.0),
            status: String::new(),
            exit_armed: false,
            sel_line: None,
            typing: None,
            mouse: (0.0, 0.0),
            width: 853.0,
            gimmicks: false,
            g_page: 0,
            g_tool: GimmickTool::BarlineNote,
            g_sv: SvParams::default(),
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
            n_clip: 0,
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
        let last = self
            .ed
            .chart
            .objects
            .iter()
            .map(|o| self.ed.end_of(o))
            .fold(0.0, f64::max);
        match &self.track {
            Some(t) if t.frames() > 0 => t.frames() as f64 / self.audio_rate * 1000.0 * t.rate,
            _ => last + 10_000.0,
        }
    }

    // ---- loading and playing ----

    pub fn update(&mut self, now: Instant, audio: Option<&Engine>) {
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
        self.exit_armed = false;
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
        self.layout = Layout::new(c.width);
        self.buttons.clear();
        self.refresh_game();
        self.sync_game(autoplay);
        let l = self.layout;
        c.text(
            8.0,
            l.bar_y - 14.0,
            7.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(7.0, c.width - 16.0, &self.label()),
        );
        self.draw_bar(c);
        self.draw_preview(c, scroll_speed);
        if self.gimmicks {
            self.draw_gimmicks(c);
        } else {
            self.draw_inspector(c);
        }
        if self.node_mode() {
            self.draw_node_strip(c);
            self.draw_node_canvas(c);
        } else {
            self.draw_timeline(c);
            self.draw_graph(c);
        }
        self.draw_footer(c);
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

    fn draw_bar(&mut self, c: &mut Canvas) {
        let y = self.layout.bar_y;
        let modified = self.ed.modified();
        let mut x = 8.0;
        let items = [
            ("Back".to_string(), Btn::Back, false),
            (
                if modified { "Save *" } else { "Save" }.to_string(),
                Btn::Save,
                modified,
            ),
            ("Undo".to_string(), Btn::Undo, false),
            ("Redo".to_string(), Btn::Redo, false),
        ];
        x = self.row(c, x, y, &items) + 6.0;
        let tools: Vec<_> = TOOLS
            .iter()
            .map(|(n, t)| (n.to_string(), Btn::Tool(*t), self.ed.tool == *t))
            .collect();
        x = self.row(c, x, y, &tools);
        let big = [("Big".to_string(), Btn::Big, self.ed.big)];
        x = self.row(c, x, y, &big) + 6.0;
        let snap = [
            ("-".to_string(), Btn::Snap(-1), false),
            (format!("1/{}", self.ed.snap), Btn::Edit(Field::Time), false),
            ("+".to_string(), Btn::Snap(1), false),
        ];
        // The middle one isn't a button of its own: it only shows the snap.
        let sx = x;
        x = self.row(c, x, y, &snap);
        self.buttons.retain(|(r, _)| r[0] != sx + 20.0 || r[1] != y);
        x += 6.0;
        let rates: Vec<_> = RATES
            .iter()
            .map(|r| (format!("{r}%"), Btn::Rate(*r), self.rate == *r))
            .collect();
        x = self.row(c, x, y, &rates) + 6.0;
        let play = [(
            if self.playing.is_some() {
                "Pause"
            } else {
                "Play"
            }
            .to_string(),
            Btn::Play,
            self.playing.is_some(),
        )];
        x = self.row(c, x, y, &play) + 6.0;
        let panel = [("Gimmicks".to_string(), Btn::Gimmicks, self.gimmicks)];
        self.row(c, x, y, &panel);
    }

    fn draw_preview(&mut self, c: &mut Canvas, scroll_speed: f64) {
        let [fx, fy, fw, fh] = self.layout.preview;
        c.rect(fx, fy, fw, fh, rgba(0x0c0c12, 0.7));
        if let Some(g) = &mut self.game {
            let start = c.shapes.len();
            let full = c.width;
            c.width = FRAME_VW;
            g.draw_playfield(c, scroll_speed);
            c.width = full;
            c.transform_since(start, [0.0, 0.0], fw / FRAME_VW, fx, fy);
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

    fn draw_inspector(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.inspector;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x15151d, 0.92),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        let (x, mut y) = (x + 8.0, y + 8.0);
        let dim = rgba(0x9a9ab0, 1.0);
        let white = rgba(0xffffff, 1.0);
        let sel = self.selected_objects();
        // What's selected.
        let head = match sel.as_slice() {
            [] => "Nothing selected".to_string(),
            [o] => {
                let what = match o.taiko() {
                    Taiko::Don => "don",
                    Taiko::Kat => "kat",
                    Taiko::Roll => "drumroll",
                    Taiko::Swell => "swell",
                };
                let end = self.ed.end_of(o);
                if end > o.time {
                    format!(
                        "{}{what}  {} - {}",
                        if o.big() { "big " } else { "" },
                        fmt_time(o.time),
                        fmt_time(end)
                    )
                } else {
                    format!(
                        "{}{what}  {}",
                        if o.big() { "big " } else { "" },
                        fmt_time(o.time)
                    )
                }
            }
            many => format!("{} notes selected", many.len()),
        };
        c.text(x, y, 7.0, white, &Canvas::fit(7.0, w - 16.0, &head));
        y += 12.0;
        if !sel.is_empty() {
            let items = [
                ("Don".to_string(), Btn::Retype(Some(false), None), false),
                ("Kat".to_string(), Btn::Retype(Some(true), None), false),
                ("Big".to_string(), Btn::Retype(None, Some(true)), false),
                ("Small".to_string(), Btn::Retype(None, Some(false)), false),
                ("Flip F".to_string(), Btn::Flip, false),
                ("Resnap".to_string(), Btn::Resnap, false),
                ("Delete".to_string(), Btn::Delete, false),
            ];
            self.row(c, x, y, &items);
        }
        y += 24.0;
        // The line in force at the playhead, or the one selected.
        let line = self
            .sel_line
            .and_then(|id| self.ed.chart.timing_point(id).cloned());
        let line_text = match &line {
            Some(p) => format!(
                "{} line at {}",
                if p.is_red() { "Red" } else { "Green" },
                fmt_time(p.time)
            ),
            None => {
                let t = &self.ed.chart.timing;
                let p = timing::active_at(t, self.ed.time);
                let bpm = timing::active_red(t, self.ed.time)
                    .and_then(|p| p.bpm())
                    .unwrap_or(0.0);
                format!("Now: {bpm:.2} BPM   SV x{:.2}", p.map_or(1.0, |p| p.sv()))
            }
        };
        c.text(x, y, 7.0, white, &Canvas::fit(7.0, w - 16.0, &line_text));
        y += 12.0;
        if let Some(p) = line {
            let mut fields: Vec<(Field, String)> = Vec::new();
            if p.is_red() {
                fields.push((Field::Bpm, format!("BPM {:.3}", p.bpm().unwrap_or(0.0))));
                fields.push((Field::Meter, format!("Meter {}", p.meter)));
            } else {
                fields.push((Field::Sv, format!("SV x{:.2}", p.sv())));
            }
            fields.push((Field::Volume, format!("Volume {}", p.volume)));
            fields.push((Field::Time, format!("Time {}", fmt_time(p.time))));
            let mut fy = y;
            for (f, label) in fields {
                let shown = match &self.typing {
                    Some((tf, text)) if *tf == f => format!("{label}: {text}_"),
                    _ => label,
                };
                let lw = Canvas::text_width(7.0, &shown).max(90.0);
                self.button(
                    c,
                    [x, fy, lw + 8.0, 14.0],
                    &shown,
                    Btn::Edit(f),
                    self.typing.as_ref().is_some_and(|(tf, _)| *tf == f),
                );
                self.button(c, [x + lw + 12.0, fy, 16.0, 14.0], "-", Btn::Dec(f), false);
                self.button(c, [x + lw + 32.0, fy, 16.0, 14.0], "+", Btn::Inc(f), false);
                fy += 17.0;
            }
            let items = [
                ("Kiai".to_string(), Btn::Kiai, p.kiai()),
                ("Omit bar".to_string(), Btn::Omit, p.omits_barline()),
                ("Delete".to_string(), Btn::DeleteLine, false),
            ];
            self.row(c, x, fy, &items);
        } else {
            let items = [
                ("+ Red line".to_string(), Btn::AddRed, false),
                ("+ Green line".to_string(), Btn::AddGreen, false),
            ];
            self.row(c, x, y, &items);
            c.text(
                x,
                y + 22.0,
                6.0,
                dim,
                &Canvas::fit(6.0, w - 16.0, "at the playhead; click a line to edit it"),
            );
        }
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

    fn draw_timeline(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.timeline;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x101016, 0.95),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        let start = c.shapes.len();
        let (from, to) = (self.view_start, self.view_end());
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
                c.rect(sx, y + 1.0, ex - sx, h - 2.0, rgba(0xFFAA00, 0.1));
            }
        }
        // Grid.
        for (t, kind) in self.grid() {
            let gx = self.x_of(t);
            let (alpha, top) = match kind {
                2 => (0.55, 12.0),
                1 => (0.3, 12.0),
                _ => (0.14, 24.0),
            };
            c.rect(gx, y + top, 1.0, h - top - 2.0, rgba(0xffffff, alpha));
        }
        // Ruler: a label every so often.
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
            c.rect(lx, y + 1.0, 1.0, 9.0, rgba(0xffffff, 0.5));
            c.text(lx + 3.0, y + 2.0, 6.0, rgba(0x9a9ab0, 1.0), &fmt_time(lt));
            lt += label_every;
        }
        // Timing lines.
        let first = pts.partition_point(|p| p.time < from - 2000.0);
        for p in pts[first..].iter().take_while(|p| p.time <= to + 2000.0) {
            let lx = self.x_of(p.time);
            let selected = self.sel_line == Some(p.id);
            let (col, tall) = if p.is_red() {
                (RED_LINE, 14.0)
            } else {
                (GREEN_LINE, 9.0)
            };
            c.rect(
                lx - if selected { 1.0 } else { 0.5 },
                y + 12.0,
                if selected { 2.0 } else { 1.0 },
                tall,
                rgba(if selected { 0xffffff } else { col }, 1.0),
            );
            if lx > x && lx < x + w - 40.0 && self.ms_per_px < 12.0 {
                let label = match p.bpm() {
                    Some(b) => format!("{b:.1}"),
                    None => format!("x{:.2}", p.sv()),
                };
                c.text(
                    lx + 3.0,
                    y + if p.is_red() { 13.0 } else { 25.0 },
                    6.0,
                    rgba(col, 1.0),
                    &label,
                );
            }
        }
        // Notes.
        let cy = y + 66.0;
        let objs = &self.ed.chart.objects;
        let first = objs.partition_point(|o| o.time < from - 60_000.0);
        let hover_x = self.mouse.0;
        for o in &objs[first..] {
            if o.time > to + 40.0 * self.ms_per_px {
                break;
            }
            let end = self.ed.end_of(o);
            if end < from - 20.0 * self.ms_per_px {
                continue;
            }
            let (ox, ex) = (self.x_of(o.time), self.x_of(end));
            let selected = self.ed.selection.contains(&o.id);
            let big = o.big();
            let r = if big { 11.0 } else { 8.0 };
            let outline = if selected {
                rgba(0xffffff, 1.0)
            } else {
                rgba(0x000000, 0.8)
            };
            match o.taiko() {
                Taiko::Roll | Taiko::Swell if ex > ox => {
                    let col = if o.taiko() == Taiko::Roll {
                        ROLL
                    } else {
                        SWELL
                    };
                    c.capsule([ox, ex], cy, r * 0.8, rgba(col, 0.9), outline, 1.5);
                    c.circle(ox, cy, r, rgba(col, 1.0), outline, 1.5);
                }
                _ => {
                    let col = if o.taiko() == Taiko::Kat { KAT } else { DON };
                    c.circle(
                        ox,
                        cy,
                        r,
                        rgba(col, 1.0),
                        outline,
                        if selected { 2.0 } else { 1.0 },
                    );
                }
            }
            let _ = hover_x;
        }
        // The ghost of what a click would place.
        let (mx, my) = self.mouse;
        if inside(self.layout.timeline, mx, my) && my > y + 12.0 && self.playing.is_none() {
            let gx = self.x_of(self.ed.snapped(self.t_at(mx)));
            match self.ed.tool {
                Tool::Don | Tool::Kat => c.circle(
                    gx,
                    cy,
                    if self.ed.big { 11.0 } else { 8.0 },
                    rgba(if self.ed.tool == Tool::Kat { KAT } else { DON }, 0.4),
                    rgba(0xffffff, 0.5),
                    1.0,
                ),
                Tool::Roll | Tool::Swell => {
                    c.rect(gx - 0.5, y + 24.0, 1.0, h - 28.0, rgba(0xffffff, 0.5))
                }
                Tool::Select => {}
            }
        }
        match self.drag {
            Drag::Box { from_x, to_x, .. } => {
                let (a, b) = (from_x.min(to_x), from_x.max(to_x));
                c.rect(a, y + 24.0, b - a, h - 28.0, rgba(accent(), 0.18));
                c.round_rect(
                    [a, y + 24.0],
                    [b - a, h - 28.0],
                    0.0,
                    rgba(0, 0.0),
                    rgba(accent(), 0.8),
                    1.0,
                );
            }
            Drag::Place { start, to, swell } => {
                let (a, b) = (self.x_of(start), self.x_of(to.max(start)));
                c.capsule(
                    [a, b],
                    cy,
                    6.0,
                    rgba(if swell { SWELL } else { ROLL }, 0.6),
                    rgba(0xffffff, 0.8),
                    1.0,
                );
            }
            _ => {}
        }
        // The playhead.
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 1.0));
        c.clip_since(start, x, y, w, h);
    }

    /// The SV graph's vertical range: log scale, wide enough for what's in view.
    fn graph_range(&self) -> (f64, f64) {
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

    fn sv_y(&self, sv: f64, range: (f64, f64)) -> f32 {
        let [_, y, _, h] = self.layout.graph;
        let f = (sv.max(1e-3).ln() - range.0.ln()) / (range.1.ln() - range.0.ln());
        y + h - 4.0 - (f.clamp(0.0, 1.0) as f32) * (h - 8.0)
    }

    fn y_sv(&self, py: f32, range: (f64, f64)) -> f64 {
        let [_, y, _, h] = self.layout.graph;
        let f = ((y + h - 4.0 - py) / (h - 8.0)).clamp(0.0, 1.0) as f64;
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
        // 1.0x and the round multiples.
        for v in [0.5, 1.0, 2.0, 4.0] {
            if v > range.0 && v < range.1 {
                let ly = self.sv_y(v, range);
                c.rect(
                    x + 1.0,
                    ly,
                    w - 2.0,
                    1.0,
                    rgba(0xffffff, if v == 1.0 { 0.22 } else { 0.08 }),
                );
                c.text(
                    x + 3.0,
                    ly - 8.0,
                    6.0,
                    rgba(0x6a6a80, 1.0),
                    &format!("x{v}"),
                );
            }
        }
        // The velocity as the game reads it: a step at each line.
        let (from, to) = (self.view_start, self.view_end());
        let pts = &self.ed.chart.timing;
        let first = pts.partition_point(|p| p.time <= from).saturating_sub(1);
        let mut prev_x = x;
        let mut prev_sv = timing::sv_at(pts, from);
        for p in pts[first..].iter().filter(|p| p.time > from) {
            if p.time > to {
                break;
            }
            let px = self.x_of(p.time);
            let (ya, yb) = (self.sv_y(prev_sv, range), self.sv_y(p.sv(), range));
            c.rect(
                prev_x,
                ya - 0.5,
                (px - prev_x).max(1.0),
                1.5,
                rgba(GREEN_LINE, 0.9),
            );
            c.rect(
                px - 0.5,
                ya.min(yb),
                1.5,
                (ya - yb).abs().max(1.0),
                rgba(GREEN_LINE, 0.5),
            );
            prev_x = px;
            prev_sv = p.sv();
        }
        let ya = self.sv_y(prev_sv, range);
        c.rect(
            prev_x,
            ya - 0.5,
            (x + w - prev_x).max(1.0),
            1.5,
            rgba(GREEN_LINE, 0.9),
        );
        // The lines themselves.
        for p in pts[first..].iter().take_while(|p| p.time <= to) {
            if p.time < from {
                continue;
            }
            let px = self.x_of(p.time);
            if p.is_red() {
                c.rect(px - 0.5, y + 1.0, 1.0, h - 2.0, rgba(RED_LINE, 0.55));
            } else {
                let on = self.sel_line == Some(p.id);
                c.circle(
                    px,
                    self.sv_y(p.sv(), range),
                    if on { 4.5 } else { 3.5 },
                    rgba(if on { 0xffffff } else { GREEN_LINE }, 1.0),
                    rgba(0x000000, 0.8),
                    1.0,
                );
            }
        }
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 0.8));
        c.clip_since(start, x, y, w, h);
        c.text(
            x + w
                - 4.0
                - Canvas::text_width(6.0, "slider velocity   click: add a line   drag: change"),
            y + 3.0,
            6.0,
            rgba(0x6a6a80, 1.0),
            "slider velocity   click: add a line   drag: change",
        );
    }

    fn draw_footer(&mut self, c: &mut Canvas) {
        let y = self.layout.graph[1] + self.layout.graph[3] + 6.0;
        let t = &self.ed.chart.timing;
        let bpm = timing::active_red(t, self.ed.time)
            .and_then(|p| p.bpm())
            .unwrap_or(0.0);
        let info = format!(
            "{}   1/{}   {bpm:.1} BPM   SV x{:.2}   {} notes{}",
            fmt_time(self.ed.time),
            self.ed.snap,
            timing::sv_at(t, self.ed.time),
            self.ed.chart.objects.len(),
            if self.ed.modified() {
                "   modified"
            } else {
                ""
            }
        );
        c.text(8.0, y, 8.0, rgba(0xffffff, 1.0), &info);
        let msg = if self.status.is_empty() {
            "Space play   Left/Right step   Up/Down snap   D/K place   Del delete   Ctrl+S save   Esc back"
                .to_string()
        } else {
            self.status.clone()
        };
        c.text(
            8.0,
            y + 14.0,
            7.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(7.0, c.width - 16.0, &msg),
        );
    }

    // ---- the pointer ----

    /// The object under `x` on the timeline: a note by its head, a roll or swell along its body.
    fn object_at_x(&self, x: f32) -> Option<u64> {
        let t = self.t_at(x);
        self.ed.object_at(t, 9.0 * self.ms_per_px)
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
        self.mouse = (x, y);
        match self.drag {
            Drag::Scrub => {
                let t = self.t_at(x);
                self.set_time(t);
            }
            Drag::Box { from, .. } => {
                self.drag = Drag::Box {
                    from,
                    from_x: self.x_of(from),
                    to_x: x,
                };
            }
            Drag::Move { anchor, grab } => {
                let Some(now) = self.ed.chart.object(anchor).map(|o| o.time) else {
                    return;
                };
                let want = self.ed.snapped(self.t_at(x) - grab);
                if self.ed.move_selected(want - now, true) {
                    self.mark_stale();
                }
            }
            Drag::Resize { id } => {
                let end = self.ed.snapped(self.t_at(x));
                if self.ed.set_end(id, end, true) {
                    self.mark_stale();
                }
            }
            Drag::Place { start, swell, .. } => {
                let to = self.ed.snapped(self.t_at(x));
                self.drag = Drag::Place { start, to, swell };
            }
            Drag::Line { id } => {
                let time = self.ed.snapped(self.t_at(x));
                let range = self.graph_range();
                let sv = (self.y_sv(y, range) * 100.0).round() / 100.0;
                let only_sv = keys.shift;
                let moved = self.ed.edit_timing(
                    id,
                    |p| {
                        if !only_sv {
                            p.time = time;
                        }
                        let _ = p.set_sv(sv.max(0.01));
                    },
                    true,
                );
                if moved.is_ok() {
                    self.mark_stale();
                }
            }
            Drag::Key { index } => self.key_graph_drag(index, x, y),
            Drag::Node { .. } | Drag::Pan { .. } => self.node_drag(x, y),
            Drag::Wire { .. } | Drag::None => {}
        }
    }

    fn graph_dot_near(&self, x: f32, y: f32) -> Option<u64> {
        let range = self.graph_range();
        self.ed
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red())
            .filter(|p| {
                let (px, py) = (self.x_of(p.time), self.sv_y(p.sv(), range));
                (px - x).abs() <= 7.0 && (py - y).abs() <= 7.0
            })
            .min_by(|a, b| a.time.total_cmp(&b.time))
            .map(|p| p.id)
    }

    pub fn mouse_down(&mut self, x: f32, y: f32, keys: Keys, audio: Option<&Engine>) {
        self.mouse = (x, y);
        self.typing = None;
        self.exit_armed = false;
        if let Some((_, b)) = self.buttons.iter().rev().find(|(r, _)| inside(*r, x, y)) {
            let b = *b;
            self.press(b, keys, audio);
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
            self.graph_down(x, y);
        }
    }

    fn timeline_down(&mut self, x: f32, y: f32, keys: Keys) {
        let tl = self.layout.timeline;
        let t = self.t_at(x);
        if y < tl[1] + 12.0 {
            self.drag = Drag::Scrub;
            self.set_time(t);
            return;
        }
        match self.ed.tool {
            Tool::Select => {
                if y < tl[1] + 28.0
                    && let Some(id) = self.line_near(x)
                {
                    self.sel_line = Some(id);
                    return;
                }
                if let Some(id) = self.object_at_x(x) {
                    let o = self.ed.chart.object(id).cloned();
                    let Some(o) = o else { return };
                    // The right end of a roll or swell stretches it.
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
                    self.sel_line = None;
                    self.drag = Drag::Move {
                        anchor: id,
                        grab: t - o.time,
                    };
                } else {
                    self.sel_line = None;
                    self.drag = Drag::Box {
                        from: t,
                        from_x: x,
                        to_x: x,
                    };
                }
            }
            Tool::Don | Tool::Kat => {
                let at = self.ed.snapped(t);
                if self.ed.place_hit(at, self.ed.tool == Tool::Kat).is_some() {
                    self.mark_stale();
                }
            }
            Tool::Roll | Tool::Swell => {
                let start = self.ed.snapped(t);
                self.drag = Drag::Place {
                    start,
                    to: start,
                    swell: self.ed.tool == Tool::Swell,
                };
            }
        }
    }

    fn graph_down(&mut self, x: f32, y: f32) {
        if self.keyframe_mode() {
            return self.key_graph_down(x, y);
        }
        if let Some(id) = self.graph_dot_near(x, y) {
            self.sel_line = Some(id);
            self.drag = Drag::Line { id };
            return;
        }
        let time = self.ed.snapped(self.t_at(x));
        let range = self.graph_range();
        let sv = ((self.y_sv(y, range) * 100.0).round() / 100.0).max(0.01);
        match self.ed.add_green(time, sv) {
            Ok(id) => {
                self.sel_line = Some(id);
                self.drag = Drag::Line { id };
                self.mark_stale();
            }
            Err(e) => self.status = e,
        }
    }

    pub fn mouse_up(&mut self, x: f32, y: f32, keys: Keys) {
        self.mouse = (x, y);
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        match drag {
            Drag::Box { from, from_x, .. } => {
                if (x - from_x).abs() < 3.0 {
                    self.ed.select_none();
                    let t = self.ed.snapped(from);
                    self.set_time(t);
                } else {
                    self.ed.select_range(from, self.t_at(x), keys.shift);
                }
            }
            Drag::Place { start, swell, .. } => {
                let end = self.ed.snapped(self.t_at(x));
                let made = if end > start {
                    if swell {
                        self.ed.place_swell(start, end)
                    } else {
                        self.ed.place_roll(start, end)
                    }
                } else {
                    None
                };
                if made.is_some() {
                    self.mark_stale();
                }
            }
            Drag::Move { .. } | Drag::Resize { .. } | Drag::Line { .. } | Drag::Key { .. } => {
                self.k_frozen = None;
                self.ed.end_drag();
                self.mark_stale();
            }
            Drag::Node { .. } => self.ed.end_drag(),
            Drag::Wire { from } => self.wire_dropped(from, x, y),
            Drag::Scrub | Drag::Pan { .. } | Drag::None => {}
        }
    }

    /// Right click: deletes what's under the pointer.
    pub fn right_click(&mut self, x: f32, y: f32) {
        if self.node_mode() {
            if inside(self.layout.nodes, x, y) {
                self.node_right(x, y);
            }
            return;
        }
        if inside(self.layout.timeline, x, y) {
            if let Some(id) = self.object_at_x(x) {
                self.ed.selection = [id].into();
                self.ed.delete_selected();
                self.mark_stale();
            }
        } else if self.keyframe_mode() {
            self.key_graph_right(x, y);
        } else if inside(self.layout.graph, x, y)
            && let Some(id) = self.graph_dot_near(x, y)
        {
            self.delete_line(id);
        }
    }

    pub fn wheel(&mut self, notches: i32, keys: Keys) {
        let (mx, my) = self.mouse;
        if self.node_mode() && inside(self.layout.nodes, mx, my) {
            self.node_zoom(notches);
            return;
        }
        if keys.ctrl {
            self.zoom(if notches > 0 { 0.8 } else { 1.25 }, mx);
        } else if keys.alt {
            self.step_snap(notches.signum());
        } else {
            let divisor = if keys.shift { 1 } else { self.ed.snap };
            let mut t = self.ed.time;
            for _ in 0..notches.unsigned_abs().min(20) {
                t = timing::step(&self.ed.chart.timing, t, notches < 0, divisor);
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
                    self.sel_line = None;
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

    fn change(&mut self, f: Field, dir: f64, keys: Keys) {
        let Some(id) = self.sel_line else { return };
        let k = if keys.shift {
            10.0
        } else if keys.alt {
            0.1
        } else {
            1.0
        };
        let r = self.ed.edit_timing(
            id,
            |p| match f {
                Field::Bpm => {
                    if let Some(bpm) = p.bpm() {
                        p.beat_length = 60_000.0 / (bpm + dir * k).max(1.0);
                    }
                }
                Field::Sv => {
                    let _ = p.set_sv((p.sv() + dir * 0.1 * k).max(0.01));
                }
                Field::Volume => p.volume = (p.volume + (dir * 5.0 * k) as i32).clamp(0, 100),
                Field::Meter => p.meter = (p.meter + (dir * k.max(1.0)) as i32).max(1),
                Field::Time => p.time = (p.time + dir * k).max(0.0),
            },
            false,
        );
        match r {
            Ok(()) => self.mark_stale(),
            Err(e) => self.status = e,
        }
    }

    fn apply_typed(&mut self, f: Field, text: &str) {
        let Some(id) = self.sel_line else { return };
        let Ok(v) = text.trim().parse::<f64>() else {
            self.status = "not a number".into();
            return;
        };
        let r = self.ed.edit_timing(
            id,
            |p| match f {
                Field::Bpm => {
                    if v > 0.0 {
                        p.beat_length = 60_000.0 / v;
                    }
                }
                Field::Sv => {
                    let _ = p.set_sv(v);
                }
                Field::Volume => p.volume = v as i32,
                Field::Meter => p.meter = v as i32,
                Field::Time => p.time = v,
            },
            false,
        );
        match r {
            Ok(()) => self.mark_stale(),
            Err(e) => self.status = e,
        }
    }

    pub fn press(&mut self, b: Btn, keys: Keys, audio: Option<&Engine>) {
        match b {
            Btn::Back => {
                self.status = "press Esc to leave".into();
            }
            Btn::Save => self.save(),
            Btn::Undo => {
                if self.ed.undo() {
                    self.mark_stale();
                }
            }
            Btn::Redo => {
                if self.ed.redo() {
                    self.mark_stale();
                }
            }
            Btn::Tool(t) => self.ed.tool = t,
            Btn::Big => self.ed.big = !self.ed.big,
            Btn::Snap(d) => self.step_snap(d),
            Btn::Rate(r) => {
                self.stop(audio);
                self.rate = r;
                self.want_rate = Some(r as f64 / 100.0);
            }
            Btn::Play => self.toggle_play(audio),
            Btn::Retype(kat, big) => {
                self.ed.retype_selected(kat, big);
                self.mark_stale();
            }
            Btn::Flip => {
                self.ed.flip_selected();
                self.mark_stale();
            }
            Btn::Resnap => {
                let n = self.ed.resnap_selected();
                self.status = format!("resnapped {n}");
                self.mark_stale();
            }
            Btn::Delete => {
                self.ed.delete_selected();
                self.mark_stale();
            }
            Btn::AddRed => {
                let at = self.ed.time;
                let bpm = timing::active_red(&self.ed.chart.timing, at)
                    .and_then(|p| p.bpm())
                    .unwrap_or(120.0);
                match self.ed.add_red(at, bpm) {
                    Ok(id) => {
                        self.sel_line = Some(id);
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
                        self.sel_line = Some(id);
                        self.mark_stale();
                    }
                    Err(e) => self.status = e,
                }
            }
            Btn::DeleteLine => {
                if let Some(id) = self.sel_line {
                    self.delete_line(id);
                }
            }
            Btn::Kiai | Btn::Omit => {
                if let Some(id) = self.sel_line {
                    let r = self.ed.edit_timing(
                        id,
                        |p| {
                            if b == Btn::Kiai {
                                p.set_kiai(!p.kiai());
                            } else {
                                p.set_omit_barline(!p.omits_barline());
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
                if self.sel_line.is_some() {
                    self.typing = Some((f, String::new()));
                }
            }
            Btn::Gimmicks => self.gimmicks = !self.gimmicks,
            Btn::GPage(p) => self.g_page = p,
            Btn::GTool(t) => self.g_tool = t,
            Btn::GStep(p, dir) => self.step_param(p, dir as f64, keys.shift),
            Btn::GFlag(f) => self.toggle_flag(f),
            Btn::GCurve(d) => self.cycle_curve(d),
            Btn::GApply(t) => self.gimmick_apply(t, false),
            Btn::GGenerate => self.gimmick_generate(false),
            Btn::GPreview => self.gimmick_preview(),
            Btn::GKeep => self.settle_preview(),
            Btn::GCancel => self.gimmick_cancel(),
            Btn::K(a) => self.press_key_action(a, keys.shift, keys.alt),
            Btn::N(a) => self.press_node_action(a, keys.shift),
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
        let was_armed = std::mem::take(&mut self.exit_armed);
        let t = self.ed.time;
        match code {
            KeyCode::Escape => {
                if repeat {
                    return KeyResult::Handled;
                }
                if self.ed.modified() && !was_armed {
                    self.exit_armed = true;
                    self.status =
                        "unsaved changes: Esc again leaves without saving, Ctrl+S saves".into();
                    return KeyResult::Handled;
                }
                self.stop(audio);
                return KeyResult::Exit;
            }
            KeyCode::Space if !repeat => self.toggle_play(audio),
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let divisor = if keys.shift { 1 } else { self.ed.snap };
                let to = timing::step(
                    &self.ed.chart.timing,
                    t,
                    code == KeyCode::ArrowRight,
                    divisor,
                );
                if self.playing.is_some() {
                    self.stop(audio);
                }
                self.set_time(to);
            }
            KeyCode::ArrowUp => self.step_snap(1),
            KeyCode::ArrowDown => self.step_snap(-1),
            KeyCode::Home => self.set_time(0.0),
            KeyCode::End => {
                let end = self.ed.chart.objects.last().map_or(0.0, |o| o.time);
                self.set_time(end);
            }
            KeyCode::Digit1 => self.ed.tool = Tool::Select,
            KeyCode::Digit2 => self.ed.tool = Tool::Don,
            KeyCode::Digit3 => self.ed.tool = Tool::Kat,
            KeyCode::Digit4 => self.ed.tool = Tool::Roll,
            KeyCode::Digit5 => self.ed.tool = Tool::Swell,
            KeyCode::KeyD | KeyCode::KeyK if !keys.ctrl && !repeat => {
                let at = self.ed.snapped(t);
                if self.ed.place_hit(at, code == KeyCode::KeyK).is_some() {
                    self.mark_stale();
                }
            }
            KeyCode::KeyF if !keys.ctrl && !repeat => self.press(Btn::Flip, keys, audio),
            KeyCode::KeyB if !keys.ctrl && !repeat => {
                let big = self.selected_objects().iter().any(|o| !o.big());
                self.ed.retype_selected(None, Some(big));
                self.mark_stale();
            }
            KeyCode::Delete | KeyCode::Backspace if self.node_mode() => self.delete_selected_node(),
            KeyCode::Delete | KeyCode::Backspace => self.press(Btn::Delete, keys, audio),
            KeyCode::KeyA if keys.ctrl => self.ed.select_all(),
            KeyCode::KeyC if keys.ctrl => {
                let n = self.ed.copy();
                self.status = format!("copied {n}");
            }
            KeyCode::KeyX if keys.ctrl => {
                let n = self.ed.cut();
                self.status = format!("cut {n}");
                self.mark_stale();
            }
            KeyCode::KeyV if keys.ctrl => {
                let at = self.ed.snapped(t);
                let n = self.ed.paste(at);
                self.status = format!("pasted {n}");
                self.mark_stale();
            }
            KeyCode::KeyZ if keys.ctrl => {
                self.press(if keys.shift { Btn::Redo } else { Btn::Undo }, keys, audio)
            }
            KeyCode::KeyY if keys.ctrl => self.press(Btn::Redo, keys, audio),
            KeyCode::KeyS if keys.ctrl => self.save(),
            KeyCode::KeyG if !keys.ctrl && !repeat => self.press(Btn::AddGreen, keys, audio),
            KeyCode::Tab if !repeat => self.gimmicks = !self.gimmicks,
            _ => return KeyResult::Pass,
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
    use crate::editor::chart::tests::MAP;
    use crate::editor::sv_curves::Curve;

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
        v.layout = Layout::new(853.0);
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
        v.ed.tool = Tool::Kat;
        // 1/4 of 500 ms = 125: x for t = 3130 snaps to 3125.
        let x = v.x_of(3130.0);
        let y = v.layout.timeline[1] + 66.0;
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
        v.mouse_down(a, v.layout.timeline[1] + 66.0, Keys::default(), None);
        v.mouse_move(b, v.layout.timeline[1] + 66.0, Keys::default());
        v.mouse_up(b, v.layout.timeline[1] + 66.0, Keys::default());
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
        let y = v.layout.timeline[1] + 66.0;
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
        let y = v.layout.timeline[1] + 40.0;
        let xx = v.x_of(900.0);
        v.mouse_down(xx, y, Keys::default(), None);
        v.mouse_move(v.x_of(2100.0), y, Keys::default());
        v.mouse_up(v.x_of(2100.0), y, Keys::default());
        assert_eq!(v.ed.selection.len(), 3);
        let x = v.x_of(3000.0);
        click(&mut v, x, y);
        assert!(v.ed.selection.is_empty());
        assert_eq!(v.ed.time, 3000.0);
    }

    #[test]
    fn clicking_the_graph_adds_a_green_line_and_dragging_it_changes_its_velocity() {
        let mut v = view();
        draw(&mut v);
        let g = v.layout.graph;
        let (x, y) = (v.x_of(2000.0), g[1] + g[3] / 2.0);
        v.mouse_down(x, y, Keys::default(), None);
        v.mouse_move(x, y - 10.0, Keys::default());
        v.mouse_up(x, y - 10.0, Keys::default());
        let greens: Vec<_> = v.ed.chart.timing.iter().filter(|p| !p.is_red()).collect();
        assert_eq!(greens.len(), 2);
        assert!(v.sel_line.is_some());
        assert!(v.ed.undo(), "the add and the drag are one step");
        assert_eq!(v.ed.chart.timing.iter().filter(|p| !p.is_red()).count(), 2);
    }

    #[test]
    fn keys_step_the_playhead_change_the_snap_and_ask_before_leaving_unsaved_work() {
        let mut v = view();
        v.ed.time = 1000.0;
        v.key(KeyCode::ArrowRight, false, Keys::default(), None);
        assert_eq!(v.ed.time, 1125.0);
        v.key(
            KeyCode::ArrowRight,
            false,
            Keys {
                shift: true,
                ..Keys::default()
            },
            None,
        );
        assert_eq!(v.ed.time, 1500.0);
        v.key(KeyCode::ArrowUp, false, Keys::default(), None);
        assert_eq!(v.ed.snap, 5);
        // Place a note by key and try to leave.
        v.set_time(3000.0);
        v.key(KeyCode::KeyD, false, Keys::default(), None);
        assert!(v.ed.modified());
        assert!(matches!(
            v.key(KeyCode::Escape, false, Keys::default(), None),
            KeyResult::Handled
        ));
        assert!(matches!(
            v.key(KeyCode::Escape, false, Keys::default(), None),
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
        press(&mut v, Btn::Gimmicks);
        assert!(v.gimmicks);
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
        // The panel draws, on both pages.
        draw(&mut v);
        press(&mut v, Btn::GPage(1));
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
        press(&mut v, Btn::GCurve(1));
        assert_eq!(v.g_sv.curve, Curve::Exp(1.3));
        press(&mut v, Btn::GStep(GParam::SvTo, 1));
        assert!((v.g_sv.to - 2.1).abs() < 1e-9);
        v.ed.time = 2000.0;
        press(&mut v, Btn::GPage(1));
        press(&mut v, Btn::GGenerate);
        assert!(v.status.starts_with("SV:"), "{}", v.status);
    }

    #[test]
    fn the_keyframe_page_makes_a_tko_and_keys_a_track() {
        use crate::editor::keyframes::Subject;
        use crate::tko::gimmick::FieldProp;
        let mut v = view();
        press(&mut v, Btn::Gimmicks);
        press(&mut v, Btn::GPage(2));
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
                ctrl: true,
                ..Keys::default()
            },
        );
        assert!(v.ms_per_px < before);
        v.wheel(
            1,
            Keys {
                alt: true,
                ..Keys::default()
            },
        );
        assert_eq!(v.ed.snap, 5);
    }

    use crate::tko::graph::Kind;

    /// A view of a `.tko` map on the Nodes page.
    fn nodes_view() -> View {
        let mut v = view();
        press(&mut v, Btn::Gimmicks);
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
        press(&mut v, Btn::Gimmicks);
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
        assert_eq!(v.ed.node(pulse).unwrap().inputs, [None]);
        assert!(v.status.contains("trigger"), "{}", v.status);
        wire_up(&mut v, beat, pulse, 0);
        assert_eq!(v.ed.node(pulse).unwrap().inputs, [Some(beat)]);
        // The wire is drawn and the preview still builds.
        draw(&mut v);
        assert!(v.game.is_some(), "{:?}", v.game_err);
        // Numbers, target and easing of the selected node.
        v.n_sel = Some(pulse);
        press(&mut v, Btn::N(NAct::Param(0, 1)));
        assert!(v.ed.node(pulse).unwrap().param(0) > 0.1);
        press(&mut v, Btn::N(NAct::Target(1)));
        assert!(v.ed.node(pulse).unwrap().target.is_some());
        let ease = v.ed.node(pulse).unwrap().ease;
        press(&mut v, Btn::N(NAct::Ease));
        assert_ne!(v.ed.node(pulse).unwrap().ease, ease);
        press(&mut v, Btn::N(NAct::NoTarget));
        assert!(v.ed.node(pulse).unwrap().target.is_none());
        // Right-click on an input dot lifts the wire; undo puts it back.
        let p = v.in_pos(v.ed.node(pulse).unwrap(), 0);
        v.right_click(p.0, p.1);
        assert_eq!(v.ed.node(pulse).unwrap().inputs, [None]);
        assert!(v.ed.undo());
        assert_eq!(v.ed.node(pulse).unwrap().inputs, [Some(beat)]);
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
        v.wheel(-3, Keys::default());
        assert!(v.n_zoom > 1.0 && v.n_zoom <= nodes::MAX_ZOOM);
        let after = v.to_world(ex, ey);
        assert!((before.0 - after.0).abs() < 0.01 && (before.1 - after.1).abs() < 0.01);
        for _ in 0..30 {
            v.wheel(1, Keys::default());
        }
        assert_eq!(v.n_zoom, nodes::MIN_ZOOM);
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
}
