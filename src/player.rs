//! The music player, opened from the main menu's "now playing": the menu makes way for the
//! song's disc with its spectrum around it, the list of songs on one side and what's queued on
//! the other, and everything for playing it: shuffle, repeat, volume, speed and pitch.

use crate::game::DON;
use crate::library::Entry;
use crate::playlist::{Playlist, Repeat};
use crate::render::{pixel_size, rgba, snap_to_grid, Canvas, Hit};

/// Spectrum bands, each drawn twice (mirrored left and right).
const BANDS: usize = 64;
/// Samples the spectrum is taken over (a power of two).
const WINDOW: usize = 2048;
const LOWEST_HZ: f32 = 40.0;
const HIGHEST_HZ: f32 = 16_000.0;
/// What the quietest band shown is, in dB; lifted towards the highs, which carry less energy.
const FLOOR_DB: f32 = -54.0;
const TILT_DB: f32 = 18.0;
/// How long a band takes to fall (ms).
const FALL_MS: f32 = 140.0;

/// Pixels across the disc.
const DISC_PIXELS: usize = 46;
const DISC_R: f32 = 92.0;
/// Where the rim starts, as a part of the radius.
const DISC_RIM: f32 = 0.85;
const DISC_Y: f32 = 196.0;
/// How fast it turns (radians a second).
const SPIN: f32 = 0.6;
const BLOCK: f32 = 3.0;
const BLOCK_STEP: f32 = 4.0;
const MAX_BLOCKS: usize = 16;
const BAR_Y: f32 = 388.0;
/// The list of songs on the left, and from this wide on, what's queued on the right.
const LIST_W: f32 = 210.0;
const QUEUE_W: f32 = 170.0;
const QUEUE_FROM: f32 = 853.0;
const LIST_TOP: f32 = 52.0;
const ROW_H: f32 = 20.0;
const ROWS: usize = 18;
/// Rows the wheel moves.
const WHEEL_ROWS: i32 = 3;
const SEMITONE_RANGE: i32 = 12;
const OUTLINE: u32 = 0x0f0f16;

/// A button of the player, also in the main menu's "now playing".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    Prev,
    Toggle,
    Next,
    Open,
    Shuffle,
    Repeat,
    /// To the current song in song select.
    PlayMap,
}

/// What a speed mod does to the music.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speed {
    DoubleTime,
    Nightcore,
    HalfTime,
    Daycore,
}

impl Speed {
    pub const ALL: [Speed; 4] = [Speed::DoubleTime, Speed::Nightcore, Speed::HalfTime, Speed::Daycore];

    fn name(self) -> &'static str {
        match self {
            Speed::DoubleTime => "DT",
            Speed::Nightcore => "NC",
            Speed::HalfTime => "HT",
            Speed::Daycore => "DC",
        }
    }

    /// The speed it plays at, and whether it keeps the pitch.
    fn rate(self) -> (f64, bool) {
        match self {
            Speed::DoubleTime => (1.5, true),
            Speed::Nightcore => (1.5, false),
            Speed::HalfTime => (0.75, true),
            Speed::Daycore => (0.75, false),
        }
    }
}

/// What the pointer is dragging.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Drag {
    Timeline,
    Volume,
}

type Icon = [&'static str; 9];

const PREV: Icon = ["##.....##", "##....###", "##...####", "##..#####", "##.######", "##..#####", "##...####", "##....###", "##.....##"];
const NEXT: Icon = ["##.....##", "###....##", "####...##", "#####..##", "######.##", "#####..##", "####...##", "###....##", "##.....##"];
const PLAY: Icon = ["##.......", "####.....", "######...", "########.", "#########", "########.", "######...", "####.....", "##......."];
const PAUSE: Icon = ["###...###", "###...###", "###...###", "###...###", "###...###", "###...###", "###...###", "###...###", "###...###"];
const SHUFFLE: Icon = [".........", ".......#.", "###...###", "...#.#.#.", "....#....", "...#.#.#.", "###...###", ".......#.", "........."];
const LOOP: Icon = ["....#....", "..#####..", ".##.#..#.", ".#.....#.", ".#.....#.", ".#..#.##.", "..#####..", "....#....", "........."];
const DISC: Icon = ["..#####..", ".#######.", "###...###", "##..#..##", "##.###.##", "##..#..##", "###...###", ".#######.", "..#####.."];

/// One of the small buttons, its icon in whole screen pixels.
pub fn button(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, control: Control, paused: bool, accent: bool) {
    let hover = c.hovered(x, y, w, h);
    let (fill, border) = match (accent, hover) {
        (true, false) => (0x2b1a17, DON),
        (true, true) => (0x4a221b, 0xff6a50),
        (false, false) => (0x23232f, 0x3a3a4e),
        (false, true) => (0x34344a, 0x6a6a88),
    };
    c.round_rect(x, y, w, h, 5.0, rgba(fill, 1.0), rgba(border, 1.0), 1.0);
    c.hit(x, y, w, h, Hit::Music(control));
    let icon = match control {
        Control::Prev => &PREV,
        Control::Next => &NEXT,
        Control::Toggle if paused => &PLAY,
        Control::Toggle => &PAUSE,
        Control::Open | Control::PlayMap => &DISC,
        Control::Shuffle => &SHUFFLE,
        Control::Repeat => &LOOP,
    };
    let px = pixel_size(1.34);
    let (left, top) = (snap_to_grid(x + w / 2.0 - 4.5 * px), snap_to_grid(y + h / 2.0 - 4.5 * px));
    for (row, line) in icon.iter().enumerate() {
        for (col, b) in line.bytes().enumerate() {
            if b == b'#' {
                c.rect(left + col as f32 * px, top + row as f32 * px, px, px, rgba(0xffffff, 1.0));
            }
        }
    }
}

/// What a frame of the player shows.
pub struct Frame<'a> {
    pub song: &'a str,
    pub hint: &'a str,
    pub paused: bool,
    /// Where the song is and how long it is (ms), once known.
    pub times: Option<(f64, f64)>,
    /// The song has a background, for the disc (the renderer draws it, under everything here).
    pub pictured: bool,
    /// The sound just played is in `Player::samples`.
    pub sounding: bool,
    pub rate: u32,
    pub volume: f32,
}

pub struct Player {
    /// The last `WINDOW` samples played, filled in by the caller.
    pub samples: Vec<f32>,
    pub list: Playlist,
    /// The library the list was made from, by size.
    pub library_len: usize,
    pub drag: Option<Drag>,
    /// The search box has the keys.
    pub searching: bool,
    /// The timeline's left edge and width as last drawn, and the song's length.
    pub timeline: (f32, f32, f64),
    /// The volume's left edge and width as last drawn.
    pub volume_bar: (f32, f32),
    /// The speed mod on, if any, and how many semitones the pitch is moved.
    pub speed: Option<Speed>,
    pub semitones: i32,
    /// The first row of the list on screen, and the track that was last scrolled to.
    scroll: usize,
    followed: Option<usize>,
    angle: f32,
    bands: [f32; BANDS],
    re: Vec<f32>,
    im: Vec<f32>,
    /// The position last known (paused, the audio clock has none).
    times: (f64, f64),
}

/// The fast Fourier transform, in place (`re` and `im` a power of two long).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let (sin, cos) = (-std::f32::consts::TAU / len as f32).sin_cos();
        for start in (0..n).step_by(len) {
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                (re[b], im[b]) = (re[a] - tr, im[a] - ti);
                (re[a], im[a]) = (re[a] + tr, im[a] + ti);
                (wr, wi) = (wr * cos - wi * sin, wr * sin + wi * cos);
            }
        }
        len <<= 1;
    }
}

impl Player {
    pub fn new(library: &[Entry], entry: Option<usize>) -> Player {
        Player {
            samples: vec![0.0; WINDOW],
            list: Playlist::new(library, entry),
            library_len: library.len(),
            drag: None,
            searching: false,
            timeline: (0.0, 1.0, 0.0),
            volume_bar: (0.0, 1.0),
            speed: None,
            semitones: 0,
            scroll: 0,
            followed: None,
            angle: 0.0,
            bands: [0.0; BANDS],
            re: vec![0.0; WINDOW],
            im: vec![0.0; WINDOW],
            times: (0.0, 0.0),
        }
    }

    /// The speed the music plays at, and whether that keeps its pitch.
    pub fn rate(&self) -> (f64, bool) {
        self.speed.map_or((1.0, true), Speed::rate)
    }

    /// How far `[` `]` move the pitch.
    pub fn pitch(&self) -> f64 {
        2f64.powf(self.semitones as f64 / 12.0)
    }

    pub fn move_pitch(&mut self, by: i32) {
        self.semitones = (self.semitones + by).clamp(-SEMITONE_RANGE, SEMITONE_RANGE);
    }

    /// One speed mod at a time; the one on again turns it off.
    pub fn toggle_speed(&mut self, speed: Speed) {
        self.speed = (self.speed != Some(speed)).then_some(speed);
    }

    pub fn scroll_by(&mut self, rows: i32) {
        let last = self.list.shown.len().saturating_sub(ROWS);
        self.scroll = (self.scroll as i64 + rows as i64).clamp(0, last as i64) as usize;
    }

    pub fn wheel(&mut self, notches: i32) {
        self.scroll_by(-notches * WHEEL_ROWS);
    }

    /// Over the list of songs.
    pub fn over_list(&self, x: f32) -> bool {
        x < LIST_W + 10.0
    }

    /// Typed into the search.
    pub fn search(&mut self, filter: String) {
        self.list.set_filter(filter);
        self.scroll = 0;
        self.followed = None;
    }

    /// Each band's height (0..1) in the sound in `samples`.
    fn spectrum(&mut self, rate: u32) -> [f32; BANDS] {
        for (i, (re, s)) in self.re.iter_mut().zip(&self.samples).enumerate() {
            let hann = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos();
            *re = s * hann;
        }
        self.im.fill(0.0);
        fft(&mut self.re, &mut self.im);
        let per_bin = rate as f32 / WINDOW as f32;
        let edge = |band: usize| LOWEST_HZ * (HIGHEST_HZ / LOWEST_HZ).powf(band as f32 / BANDS as f32) / per_bin;
        let mut out = [0.0; BANDS];
        for (band, v) in out.iter_mut().enumerate() {
            let lo = edge(band) as usize;
            let hi = (edge(band + 1) as usize).clamp(lo + 1, WINDOW / 2);
            let peak = (lo..hi).map(|k| self.re[k].hypot(self.im[k])).fold(0.0, f32::max) * 4.0 / WINDOW as f32;
            let db = 20.0 * (peak + 1e-6).log10() + TILT_DB * band as f32 / BANDS as f32;
            *v = (1.0 - db / FLOOR_DB).clamp(0.0, 1.0);
        }
        out
    }

    pub fn draw(&mut self, c: &mut Canvas, f: &Frame) {
        let fall = (-c.dt_ms / FALL_MS).exp();
        let heard = if f.sounding { self.spectrum(f.rate) } else { [0.0; BANDS] };
        for (band, new) in self.bands.iter_mut().zip(heard) {
            *band = new.max(*band * fall);
        }
        if !f.paused {
            self.angle += SPIN * c.dt_ms / 1000.0;
        }
        if let Some(t) = f.times {
            self.times = t;
        }
        let wide = c.width >= QUEUE_FROM;
        let (left, right) = (LIST_W + 20.0, c.width - if wide { QUEUE_W + 20.0 } else { 10.0 });
        let x = (left + right) / 2.0;
        self.draw_rays(c, x);
        draw_disc(c, x, DISC_Y, self.angle, f.pictured);
        if f.pictured {
            // A hair past the rim's inside edge, smooth under its pixels.
            c.disc = Some([x, DISC_Y, DISC_R * (DISC_RIM + 0.02), self.angle]);
            for groove in [0.34, 0.5, 0.66] {
                c.ring(x, DISC_Y, DISC_R * groove, rgba(0x000000, 0.12), 1.0);
            }
        }
        c.button(10.0, 24.0, 74.0, 20.0, tr!("< menu", "< メニュー", "< меню"), 8.0, Hit::Key(winit::keyboard::KeyCode::Escape), false);
        c.text_right(c.width - 14.0, 14.0, 7.0, rgba(0x9a9ab0, 1.0), f.hint);
        self.draw_list(c);
        if wide {
            self.draw_queue(c, c.width - QUEUE_W - 10.0);
        }
        c.text_centered(x, BAR_Y - 18.0, 9.0, rgba(0xffffff, 1.0), &Canvas::fit(9.0, right - left, f.song));
        self.draw_timeline(c, x, right - left);
        let (w, gap, y) = (40.0, 6.0, BAR_Y + 18.0);
        let start = x - (w * 5.0 + gap * 4.0) / 2.0;
        let row = [(Control::Shuffle, self.list.is_shuffled()), (Control::Prev, false), (Control::Toggle, true), (Control::Next, false), (Control::Repeat, self.list.repeat != Repeat::Off)];
        for (i, (control, on)) in row.into_iter().enumerate() {
            button(c, start + i as f32 * (w + gap), y, w, 26.0, control, f.paused, on);
        }
        if self.list.repeat == Repeat::One {
            c.text(start + 4.0 * (w + gap) + w - 9.0, y + 3.0, 7.0, rgba(0xffffff, 1.0), "1");
        }
        self.draw_sound(c, x, y + 34.0, f.volume);
    }

    /// The song list: a search box, then a row for each song, with a `+` for the queue.
    fn draw_list(&mut self, c: &mut Canvas) {
        const X: f32 = 10.0;
        let border = if self.searching { rgba(crate::render::accent(), 1.0) } else { rgba(0x3a3a4e, 1.0) };
        c.round_rect(X, LIST_TOP, LIST_W, 18.0, 4.0, rgba(0x15151d, 0.95), border, 1.0);
        let filter = self.list.filter.clone();
        let (text, col) = if filter.is_empty() && !self.searching { (tr!("search (Ctrl+F)", "検索 (Ctrl+F)", "поиск (Ctrl+F)").to_string(), 0x6f6f86) } else { (format!("{filter}_"), 0xffffff) };
        c.text(X + 7.0, LIST_TOP + 5.5, 7.0, rgba(col, 1.0), &Canvas::fit_end(7.0, LIST_W - 14.0, &text));
        c.hit(X, LIST_TOP, LIST_W, 18.0, Hit::PlayerSearch);

        let top = LIST_TOP + 22.0;
        c.round_rect(X, top, LIST_W, ROWS as f32 * ROW_H + 4.0, 4.0, rgba(0x15151d, 0.82), rgba(0x3a3a4e, 1.0), 1.0);
        let now = self.list.now();
        // A new song comes into view; scrolling away from it stays.
        if self.followed != Some(now) {
            self.followed = Some(now);
            if let Some(pos) = self.list.shown.iter().position(|&t| t == now) {
                if pos < self.scroll || pos >= self.scroll + ROWS {
                    self.scroll = pos.saturating_sub(ROWS / 2);
                    self.scroll_by(0);
                }
            }
        }
        const ADD_W: f32 = 18.0;
        for (row, &track) in self.list.shown.iter().skip(self.scroll).take(ROWS).enumerate() {
            let y = top + 2.0 + row as f32 * ROW_H;
            let hover = c.hovered(X, y, LIST_W, ROW_H);
            if track == now {
                c.rect(X + 2.0, y, LIST_W - 4.0, ROW_H, rgba(crate::render::accent(), 0.3));
            } else if hover {
                c.rect(X + 2.0, y, LIST_W - 4.0, ROW_H, rgba(0x34344a, 0.8));
            }
            let room = LIST_W - 14.0 - ADD_W;
            c.text(X + 7.0, y + 6.5, 7.0, rgba(if track == now { 0xffffff } else { 0xc8c8d8 }, 1.0), &Canvas::fit(7.0, room, &self.list.tracks[track].label));
            c.hit(X, y, LIST_W - ADD_W - 2.0, ROW_H, Hit::PlayerRow(track));
            if hover {
                c.text_centered(X + LIST_W - ADD_W / 2.0 - 2.0, y + 6.5, 7.0, rgba(0xffffff, 1.0), "+");
            }
            c.hit(X + LIST_W - ADD_W - 2.0, y, ADD_W, ROW_H, Hit::PlayerQueue(track));
        }
        if self.list.shown.is_empty() {
            c.text(X + 7.0, top + 9.0, 7.0, rgba(0x9a9ab0, 1.0), tr!("nothing found", "見つかりません", "ничего не нашлось"));
        }
        let count = self.list.tracks.len();
        c.text(X + 2.0, top + ROWS as f32 * ROW_H + 9.0, 7.0, rgba(0x9a9ab0, 1.0), &tr!("{count} songs", "{count}曲", "{count} песен";));
    }

    /// What plays next; a click takes it off.
    fn draw_queue(&self, c: &mut Canvas, x: f32) {
        c.text(x + 2.0, LIST_TOP + 5.5, 7.0, rgba(0x9a9ab0, 1.0), tr!("UP NEXT", "次に再生", "ДАЛЬШЕ"));
        let top = LIST_TOP + 22.0;
        c.round_rect(x, top, QUEUE_W, ROWS as f32 * ROW_H + 4.0, 4.0, rgba(0x15151d, 0.82), rgba(0x3a3a4e, 1.0), 1.0);
        for (row, &track) in self.list.queue.iter().take(ROWS).enumerate() {
            let y = top + 2.0 + row as f32 * ROW_H;
            if c.hovered(x, y, QUEUE_W, ROW_H) {
                c.rect(x + 2.0, y, QUEUE_W - 4.0, ROW_H, rgba(0x34344a, 0.8));
            }
            c.text(x + 7.0, y + 6.5, 7.0, rgba(0xc8c8d8, 1.0), &Canvas::fit(7.0, QUEUE_W - 14.0, &self.list.tracks[track].label));
            c.hit(x, y, QUEUE_W, ROW_H, Hit::PlayerUnqueue(row));
        }
        if self.list.queue.is_empty() {
            c.text(x + 7.0, top + 9.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit(7.0, QUEUE_W - 14.0, tr!("+ on a song adds it", "曲の + で追加", "+ у песни добавит")));
        }
    }

    /// The bottom row: going to the map, the speed mods, the pitch, the volume.
    fn draw_sound(&mut self, c: &mut Canvas, cx: f32, y: f32, volume: f32) {
        const H: f32 = 18.0;
        let pitch = if self.semitones == 0 { tr!("pitch [ ]", "ピッチ [ ]", "питч [ ]").to_string() } else { format!("{:+} [ ]", self.semitones) };
        let play = tr!("play map", "譜面をプレイ", "сыграть карту");
        let play_w = Canvas::text_width(7.0, play) + 12.0;
        let pitch_w = Canvas::text_width(7.0, &pitch) + 10.0;
        const MOD_W: f32 = 24.0;
        const VOLUME_W: f32 = 62.0;
        const GAP: f32 = 4.0;
        let total = play_w + GAP * 2.0 + (MOD_W + GAP) * 4.0 + pitch_w + GAP * 2.0 + VOLUME_W;
        let mut x = cx - total / 2.0;
        c.button(x, y, play_w, H, play, 7.0, Hit::Music(Control::PlayMap), true);
        x += play_w + GAP * 2.0;
        for (i, speed) in Speed::ALL.into_iter().enumerate() {
            let on = self.speed == Some(speed);
            let hover = c.hovered(x, y, MOD_W, H);
            let fill = if on { crate::render::accent() } else if hover { 0x34344a } else { 0x23232f };
            c.round_rect(x, y, MOD_W, H, 4.0, rgba(fill, 1.0), rgba(if on { 0xffffff } else { 0x3a3a4e }, 1.0), 1.0);
            c.text_centered(x + MOD_W / 2.0, y + 5.5, 7.0, rgba(0xffffff, 1.0), speed.name());
            c.hit(x, y, MOD_W, H, Hit::PlayerSpeed(i));
            x += MOD_W + GAP;
        }
        c.text(x + 3.0, y + 5.5, 7.0, rgba(if self.semitones == 0 { 0x9a9ab0 } else { 0xffffff }, 1.0), &pitch);
        c.hit(x, y, pitch_w, H, Hit::PlayerPitch);
        x += pitch_w + GAP * 2.0;
        self.volume_bar = (x, VOLUME_W);
        let step = pixel_size(4.0);
        let blocks = (VOLUME_W / step) as usize;
        let lit = (volume.clamp(0.0, 1.0) * blocks as f32).round() as usize;
        for b in 0..blocks {
            let tall = pixel_size(4.0 + b as f32 / blocks as f32 * 10.0);
            let col = if b < lit { rgba(0xffffff, 1.0) } else { rgba(0x3a3a4e, 1.0) };
            c.rect(snap_to_grid(x) + b as f32 * step, snap_to_grid(y + H - 2.0 - tall), step - pixel_size(1.0), tall, col);
        }
        c.hit(x - 2.0, y, VOLUME_W + 4.0, H, Hit::PlayerVolume);
    }

    /// The spectrum: rows of blocks out of the disc, the lows at the top, mirrored.
    fn draw_rays(&self, c: &mut Canvas, cx: f32) {
        let (block, step) = (pixel_size(BLOCK), pixel_size(BLOCK_STEP));
        for (band, &v) in self.bands.iter().enumerate() {
            let blocks = (v * MAX_BLOCKS as f32).round() as usize;
            let angle = std::f32::consts::PI * (band as f32 + 0.5) / BANDS as f32;
            let (sin, cos) = angle.sin_cos();
            for b in 0..blocks {
                let r = DISC_R + 7.0 + b as f32 * step;
                let t = b as f32 / MAX_BLOCKS as f32;
                let col = rgba(crate::menu::tint(crate::render::accent(), 0xffffff, t * t), 1.0);
                for side in [-1.0, 1.0] {
                    let (x, y) = (snap_to_grid(cx + side * sin * r), snap_to_grid(DISC_Y - cos * r));
                    c.rect(x - block / 2.0, y - block / 2.0, block, block, col);
                }
            }
        }
    }

    /// The song's progress in blocks, with the times either side.
    fn draw_timeline(&mut self, c: &mut Canvas, cx: f32, room: f32) {
        let (at, len) = self.times;
        let w = (room - 80.0).min(420.0);
        let left = cx - w / 2.0;
        self.timeline = (left, w, len);
        let done = if len > 0.0 { (at / len).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let step = pixel_size(4.0);
        let blocks = (w / step) as usize;
        let lit = (done * blocks as f32).round() as usize;
        let top = snap_to_grid(BAR_Y);
        for b in 0..blocks {
            let col = if b < lit { rgba(crate::render::accent(), 1.0) } else { rgba(0x3a3a4e, 1.0) };
            c.rect(snap_to_grid(left) + b as f32 * step, top, step - pixel_size(1.0), pixel_size(6.0), col);
        }
        c.hit(left - 4.0, BAR_Y - 6.0, w + 8.0, 18.0, Hit::MusicBar);
        let clock = |ms: f64| format!("{}:{:02}", (ms / 60_000.0) as u32, (ms / 1000.0) as u32 % 60);
        c.text_right(left - 8.0, BAR_Y, 7.0, rgba(0xc8c8d8, 1.0), &clock(at.max(0.0)));
        c.text(left + w + 8.0, BAR_Y, 7.0, rgba(0xc8c8d8, 1.0), &clock(len));
    }
}

/// The record's pixel parts, around the picture the renderer puts under them: a white rim like a
/// note's and the spindle. Without a background, slices that show it turning fill it.
fn draw_disc(c: &mut Canvas, x: f32, y: f32, angle: f32, pictured: bool) {
    let px = pixel_size(DISC_R * 2.0 / DISC_PIXELS as f32);
    let half = DISC_PIXELS as f32 / 2.0;
    let (left, top) = (snap_to_grid(x - half * px), snap_to_grid(y - half * px));
    for row in 0..DISC_PIXELS {
        let mut run: Option<(usize, u32)> = None;
        for col in 0..=DISC_PIXELS {
            let (dx, dy) = (col as f32 - half + 0.5, row as f32 - half + 0.5);
            let d = dx.hypot(dy) / half;
            let colour = if col == DISC_PIXELS || d > 1.0 {
                0
            } else if d > 0.95 {
                OUTLINE
            } else if d > DISC_RIM {
                if dx + dy > half * 0.9 { 0xc8c8d8 } else { 0xffffff }
            } else if d < 0.08 {
                0x121219
            } else if d < 0.15 {
                0xffffff
            } else if pictured {
                0
            } else {
                slice_at(dy.atan2(dx) - angle)
            };
            match run {
                Some((_, was)) if was == colour => {}
                _ => {
                    if let Some((from, was)) = run.filter(|r| r.1 != 0) {
                        c.rect(left + from as f32 * px, top + row as f32 * px, (col - from) as f32 * px, px, rgba(was, 1.0));
                    }
                    run = Some((col, colour));
                }
            }
        }
    }
}

/// The disc's colour without a picture, at `angle` round it.
fn slice_at(angle: f32) -> u32 {
    if (angle / std::f32::consts::FRAC_PI_4).floor() as i32 % 2 == 0 { 0x34344a } else { 0x23232f }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, rate: u32) -> Player {
        let mut p = Player::new(&[], None);
        for (i, s) in p.samples.iter_mut().enumerate() {
            *s = (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin() * 0.5;
        }
        p
    }

    #[test]
    fn a_tone_lights_up_its_own_band() {
        for hz in [200.0, 1000.0, 8000.0] {
            let bands = tone(hz, 48_000).spectrum(48_000);
            let loudest = (0..BANDS).max_by(|&a, &b| bands[a].total_cmp(&bands[b])).unwrap();
            let expected = (hz / LOWEST_HZ).ln() / (HIGHEST_HZ / LOWEST_HZ).ln() * BANDS as f32;
            assert!((loudest as f32 - expected).abs() <= 3.0, "{hz} Hz in band {loudest}, not {expected}");
            assert!(bands[loudest] > 0.8, "{hz} Hz only reaches {}", bands[loudest]);
        }
    }

    #[test]
    fn silence_shows_nothing() {
        assert!(Player::new(&[], None).spectrum(48_000).iter().all(|&v| v == 0.0));
    }

    #[test]
    fn a_disc_without_a_picture_still_shows_it_turning() {
        assert_ne!(slice_at(0.2), slice_at(1.0));
    }
}
