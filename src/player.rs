//! The music player, opened from the main menu's "now playing": the menu makes way for the
//! song's disc with its spectrum around it, the list of songs on one side and what's queued on
//! the other, and everything for playing it: shuffle, repeat, volume, speed and pitch.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::game::DON;
use crate::genres::GENRES;
use crate::library::Entry;
use crate::playlist::{Folder, Playlist, Repeat};
use crate::render::{Canvas, Hit, pixel_size, rgba, snap_to_grid};

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
pub(crate) const DISC_RIM: f32 = 0.85;
fn disc_y() -> f32 {
    crate::mobile::pick(196.0, 172.0)
}
/// How fast it turns (radians a second).
const SPIN: f32 = 0.6;
const BLOCK: f32 = 3.0;
const BLOCK_STEP: f32 = 4.0;
const MAX_BLOCKS: usize = 16;
fn bar_y() -> f32 {
    crate::mobile::pick(388.0, 372.0)
}
/// The list of songs on the left, and from this wide on, what's queued on the right.
fn list_w() -> f32 {
    if tall() {
        return TALL_W.load(Ordering::Relaxed) as f32;
    }
    crate::mobile::pick(210.0, 300.0)
}
fn queue_w() -> f32 {
    crate::mobile::pick(170.0, 180.0)
}
const QUEUE_FROM: f32 = 853.0;
fn list_top() -> f32 {
    if tall() {
        return 36.0;
    }
    crate::mobile::pick(52.0, 54.0)
}
fn row_h() -> f32 {
    if tall() {
        return 34.0;
    }
    crate::mobile::pick(20.0, 40.0)
}
fn rows() -> usize {
    if tall() {
        return 11;
    }
    if crate::mobile::active() { 8 } else { 18 }
}

/// The player is drawn upright, on a phone held that way: its own layout.
static TALL: AtomicBool = AtomicBool::new(false);
/// The screen's width then.
static TALL_W: AtomicU32 = AtomicU32::new(0);

pub fn tall() -> bool {
    TALL.load(Ordering::Relaxed)
}

/// Text sizes upright, where a virtual unit is twice as big: whole font pixels on the grid.
const TALL_SMALL: f32 = 5.0;
const TALL_TEXT: f32 = 7.0;
/// The disc upright.
const TALL_DISC_R: f32 = 66.0;
const TALL_DISC_Y: f32 = 140.0;
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
    /// Love the song playing, or not any more.
    Favourite,
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
    pub const ALL: [Speed; 4] = [
        Speed::DoubleTime,
        Speed::Nightcore,
        Speed::HalfTime,
        Speed::Daycore,
    ];

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

const PREV: Icon = [
    "##.....##",
    "##....###",
    "##...####",
    "##..#####",
    "##.######",
    "##..#####",
    "##...####",
    "##....###",
    "##.....##",
];
const NEXT: Icon = [
    "##.....##",
    "###....##",
    "####...##",
    "#####..##",
    "######.##",
    "#####..##",
    "####...##",
    "###....##",
    "##.....##",
];
const PLAY: Icon = [
    "##.......",
    "####.....",
    "######...",
    "########.",
    "#########",
    "########.",
    "######...",
    "####.....",
    "##.......",
];
const PAUSE: Icon = [
    "###...###",
    "###...###",
    "###...###",
    "###...###",
    "###...###",
    "###...###",
    "###...###",
    "###...###",
    "###...###",
];
const HEART: Icon = [
    ".........",
    ".##...##.",
    "####.####",
    "#########",
    "#########",
    ".#######.",
    "..#####..",
    "...###...",
    "....#....",
];
const SHUFFLE: Icon = [
    ".........",
    ".......#.",
    "###...###",
    "...#.#.#.",
    "....#....",
    "...#.#.#.",
    "###...###",
    ".......#.",
    ".........",
];
const LOOP: Icon = [
    "....#....",
    "..#####..",
    ".##.#..#.",
    ".#.....#.",
    ".#.....#.",
    ".#..#.##.",
    "..#####..",
    "....#....",
    ".........",
];
const DISC: Icon = [
    "..#####..",
    ".#######.",
    "###...###",
    "##..#..##",
    "##.###.##",
    "##..#..##",
    "###...###",
    ".#######.",
    "..#####..",
];

/// One of the small buttons, its icon in whole screen pixels.
pub fn button(
    c: &mut Canvas,
    [x, y, w, h]: [f32; 4],
    control: Control,
    paused: bool,
    accent: bool,
) {
    let hover = c.hovered(x, y, w, h);
    let (fill, border) = match (accent, hover) {
        (true, false) => (0x2b1a17, DON),
        (true, true) => (0x4a221b, 0xff6a50),
        (false, false) => (0x23232f, 0x3a3a4e),
        (false, true) => (0x34344a, 0x6a6a88),
    };
    c.round_rect([x, y], [w, h], 5.0, rgba(fill, 1.0), rgba(border, 1.0), 1.0);
    c.hit(x, y, w, h, Hit::Music(control));
    let icon = match control {
        Control::Prev => &PREV,
        Control::Next => &NEXT,
        Control::Toggle if paused => &PLAY,
        Control::Toggle => &PAUSE,
        Control::Open | Control::PlayMap => &DISC,
        Control::Shuffle => &SHUFFLE,
        Control::Repeat => &LOOP,
        Control::Favourite => &HEART,
    };
    let px = pixel_size(1.34);
    let (left, top) = (
        snap_to_grid(x + w / 2.0 - 4.5 * px),
        snap_to_grid(y + h / 2.0 - 4.5 * px),
    );
    for (row, line) in icon.iter().enumerate() {
        for (col, b) in line.bytes().enumerate() {
            if b == b'#' {
                c.rect(
                    left + col as f32 * px,
                    top + row as f32 * px,
                    px,
                    px,
                    rgba(0xffffff, 1.0),
                );
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

/// The music's spectrum as rows of blocks out of a disc: the player's, and the idle screen's.
pub struct Spectrum {
    /// The last `WINDOW` samples played, filled in by the caller.
    pub samples: Vec<f32>,
    bands: [f32; BANDS],
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Default for Spectrum {
    fn default() -> Spectrum {
        Spectrum {
            samples: vec![0.0; WINDOW],
            bands: [0.0; BANDS],
            re: vec![0.0; WINDOW],
            im: vec![0.0; WINDOW],
        }
    }
}

impl Spectrum {
    /// Takes in what `samples` holds now (`sounding`: they're what just played), the bands
    /// falling back over `dt_ms` where it's quieter.
    pub fn update(&mut self, dt_ms: f32, sounding: bool, rate: u32) {
        let fall = (-dt_ms / FALL_MS).exp();
        let heard = if sounding {
            self.analyse(rate)
        } else {
            [0.0; BANDS]
        };
        for (band, new) in self.bands.iter_mut().zip(heard) {
            *band = new.max(*band * fall);
        }
    }

    fn analyse(&mut self, rate: u32) -> [f32; BANDS] {
        for (i, (re, s)) in self.re.iter_mut().zip(&self.samples).enumerate() {
            let hann = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos();
            *re = s * hann;
        }
        self.im.fill(0.0);
        fft(&mut self.re, &mut self.im);
        let per_bin = rate as f32 / WINDOW as f32;
        let edge = |band: usize| {
            LOWEST_HZ * (HIGHEST_HZ / LOWEST_HZ).powf(band as f32 / BANDS as f32) / per_bin
        };
        let mut out = [0.0; BANDS];
        for (band, v) in out.iter_mut().enumerate() {
            let lo = edge(band) as usize;
            let hi = (edge(band + 1) as usize).clamp(lo + 1, WINDOW / 2);
            let peak = (lo..hi)
                .map(|k| self.re[k].hypot(self.im[k]))
                .fold(0.0, f32::max)
                * 4.0
                / WINDOW as f32;
            let db = 20.0 * (peak + 1e-6).log10() + TILT_DB * band as f32 / BANDS as f32;
            *v = (1.0 - db / FLOOR_DB).clamp(0.0, 1.0);
        }
        out
    }

    /// The rays: rows of blocks out of a disc of radius `disc_r` at (`cx`, `cy`), the lows at
    /// the top, mirrored left and right, at opacity `alpha`.
    pub fn draw_rays(&self, c: &mut Canvas, cx: f32, cy: f32, disc_r: f32, alpha: f32) {
        let (block, step) = (pixel_size(BLOCK), pixel_size(BLOCK_STEP));
        for (band, &v) in self.bands.iter().enumerate() {
            let blocks = (v * MAX_BLOCKS as f32).round() as usize;
            let angle = std::f32::consts::PI * (band as f32 + 0.5) / BANDS as f32;
            let (sin, cos) = angle.sin_cos();
            for b in 0..blocks {
                let r = disc_r + 7.0 + b as f32 * step;
                let t = b as f32 / MAX_BLOCKS as f32;
                let col = rgba(
                    crate::menu::tint(crate::render::accent(), 0xffffff, t * t),
                    alpha,
                );
                for side in [-1.0, 1.0] {
                    let (x, y) = (
                        snap_to_grid(cx + side * sin * r),
                        snap_to_grid(cy - cos * r),
                    );
                    c.rect(x - block / 2.0, y - block / 2.0, block, block, col);
                }
            }
        }
    }
}

/// How far the rays reach past the disc's edge at most.
pub const RAYS_REACH: f32 = 7.0 + MAX_BLOCKS as f32 * BLOCK_STEP;

pub struct Player {
    /// The music's spectrum around the disc.
    pub spectrum: Spectrum,
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
    /// The folders are showing, instead of the list.
    pub folders_open: bool,
    /// Upright: the list is up, over the player.
    pub list_open: bool,
    /// The speed picked by hand and whether it keeps the pitch, with no speed mod on.
    custom_rate: (f64, bool),
    /// The first row of the list on screen, and the track that was last scrolled to.
    scroll: usize,
    followed: Option<usize>,
    angle: f32,
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
            spectrum: Spectrum::default(),
            list: Playlist::new(library, entry),
            library_len: library.len(),
            drag: None,
            searching: false,
            timeline: (0.0, 1.0, 0.0),
            volume_bar: (0.0, 1.0),
            speed: None,
            semitones: 0,
            folders_open: false,
            list_open: false,
            custom_rate: (1.0, true),
            scroll: 0,
            followed: None,
            angle: 0.0,
            times: (0.0, 0.0),
        }
    }

    /// The speed the music plays at, and whether that keeps its pitch.
    pub fn rate(&self) -> (f64, bool) {
        self.speed.map_or(self.custom_rate, Speed::rate)
    }

    /// The speed picked by hand: the speed mods give way to it.
    pub fn set_custom_rate(&mut self, rate: f64, keep_pitch: bool) {
        self.speed = None;
        self.custom_rate = (rate, keep_pitch);
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
        let last = self.list.shown.len().saturating_sub(self::rows());
        self.scroll = (self.scroll as i64 + rows as i64).clamp(0, last as i64) as usize;
    }

    pub fn wheel(&mut self, notches: i32) {
        self.scroll_by(-notches * WHEEL_ROWS);
    }

    /// Over the list of songs.
    pub fn over_list(&self, x: f32) -> bool {
        x < list_w() + 10.0
    }

    /// Goes into folder `i` of the list's folders.
    pub fn open_folder(&mut self, i: usize, seed: u64) {
        if let Some(&(folder, _)) = self.list.folders().get(i) {
            self.list.set_folder(folder, seed);
            self.folders_open = false;
            self.scroll = 0;
            self.followed = None;
        }
    }

    /// Typed into the search.
    pub fn search(&mut self, filter: String) {
        self.list.set_filter(filter);
        self.scroll = 0;
        self.followed = None;
    }

    pub fn draw(&mut self, c: &mut Canvas, f: &Frame) {
        self.spectrum.update(c.dt_ms, f.sounding, f.rate);
        if !f.paused {
            self.angle += SPIN * c.dt_ms / 1000.0;
        }
        if let Some(t) = f.times {
            self.times = t;
        }
        let upright = crate::mobile::active() && c.width < crate::render::VIRTUAL_HEIGHT;
        TALL.store(upright, Ordering::Relaxed);
        TALL_W.store(c.width as u32, Ordering::Relaxed);
        if upright {
            return self.draw_tall(c, f);
        }
        let wide = c.width >= QUEUE_FROM;
        let (left, right) = (
            list_w() + 20.0,
            c.width - if wide { queue_w() + 20.0 } else { 10.0 },
        );
        let x = (left + right) / 2.0;
        self.spectrum.draw_rays(c, x, disc_y(), DISC_R, 1.0);
        draw_disc(c, x, disc_y(), self.angle, f.pictured);
        if f.pictured {
            // A hair past the rim's inside edge, smooth under its pixels.
            c.disc = Some([x, disc_y(), DISC_R * (DISC_RIM + 0.02), self.angle]);
            for groove in [0.34, 0.5, 0.66] {
                c.ring(x, disc_y(), DISC_R * groove, rgba(0x000000, 0.12), 1.0);
            }
        }
        let phone = crate::mobile::active();
        let back = if phone {
            [0.0, 0.0, 96.0, 46.0]
        } else {
            [10.0, 24.0, 74.0, 20.0]
        };
        c.button(
            back[0],
            back[1],
            back[2],
            back[3],
            tr!("< menu"),
            crate::mobile::pick(8.0, 12.0),
            Hit::Key(winit::keyboard::KeyCode::Escape),
            false,
        );
        if wide && !phone {
            c.text_right(c.width - 14.0, 34.0, 7.0, rgba(0x9a9ab0, 1.0), f.hint);
        }
        if self.folders_open {
            self.draw_folders(c);
        } else {
            self.draw_list(c);
        }
        if wide {
            self.draw_queue(c, c.width - queue_w() - 10.0);
        }
        c.text_centered(
            x,
            bar_y() - crate::mobile::pick(18.0, 22.0),
            9.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(9.0, right - left, f.song),
        );
        self.draw_timeline(c, x, right - left);
        let (w, gap, y) = if phone {
            (60.0, 8.0, bar_y() + 16.0)
        } else {
            (40.0, 6.0, bar_y() + 18.0)
        };
        let button_h = crate::mobile::pick(26.0, 42.0);
        let start = x - (w * 6.0 + gap * 5.0) / 2.0;
        let loved = self.list.is_favourite(self.list.now());
        let row = [
            (Control::Shuffle, self.list.is_shuffled()),
            (Control::Prev, false),
            (Control::Toggle, true),
            (Control::Next, false),
            (Control::Repeat, self.list.repeat != Repeat::Off),
            (Control::Favourite, loved),
        ];
        for (i, (control, on)) in row.into_iter().enumerate() {
            button(
                c,
                [start + i as f32 * (w + gap), y, w, button_h],
                control,
                f.paused,
                on,
            );
        }
        if self.list.repeat == Repeat::One {
            c.text(
                start + 4.0 * (w + gap) + w - 9.0,
                y + 3.0,
                7.0,
                rgba(0xffffff, 1.0),
                "1",
            );
        }
        self.draw_sound(c, x, y + crate::mobile::pick(34.0, 50.0), f.volume);
    }

    /// The player on a phone held upright: the disc, the song, and big controls under the thumb.
    /// The list comes up over it.
    fn draw_tall(&mut self, c: &mut Canvas, f: &Frame) {
        let (w, cx) = (c.width, c.width / 2.0);
        // The picture stays, quieter, so the text sits on something calm.
        c.rect(0.0, 0.0, w, 480.0, rgba(0x0b0b10, 0.45));
        if self.list_open {
            return self.draw_tall_list(c);
        }
        let escape = Hit::Key(winit::keyboard::KeyCode::Escape);
        c.button(0.0, 0.0, 44.0, 30.0, "<", TALL_TEXT, escape, false);
        let heading = tr!("NOW PLAYING");
        c.text_centered(cx, 11.0, TALL_SMALL, rgba(0x9a9ab0, 1.0), heading);
        c.button(
            w - 44.0,
            0.0,
            44.0,
            30.0,
            "",
            TALL_TEXT,
            Hit::PlayerList,
            false,
        );
        for k in 0..3 {
            let y = 9.0 + k as f32 * 5.0;
            c.rect(w - 30.0, y, 16.0, pixel_size(2.0), rgba(0xffffff, 1.0));
        }

        self.spectrum
            .draw_rays(c, cx, TALL_DISC_Y, TALL_DISC_R, 1.0);
        draw_disc_sized(c, cx, TALL_DISC_Y, TALL_DISC_R, self.angle, f.pictured, 1.0);
        if f.pictured {
            let r = TALL_DISC_R * (DISC_RIM + 0.02);
            c.disc = Some([cx, TALL_DISC_Y, r, self.angle]);
            for groove in [0.34, 0.5, 0.66] {
                c.ring(
                    cx,
                    TALL_DISC_Y,
                    TALL_DISC_R * groove,
                    rgba(0x000000, 0.12),
                    1.0,
                );
            }
        }

        // The song: its name big, the artist under it.
        let (artist, title) = f.song.split_once(" - ").unwrap_or(("", f.song));
        let room = w - 8.0;
        let chars = (room / Canvas::char_width(TALL_TEXT)).max(8.0) as usize;
        let lines = crate::social::wrap(title, chars);
        let mut y = 284.0;
        for line in lines.iter().take(2) {
            let shown = Canvas::fit(TALL_TEXT, room, line);
            c.text_centered(cx, y, TALL_TEXT, rgba(0xffffff, 1.0), &shown);
            y += 15.0;
        }
        let by = Canvas::fit(TALL_SMALL, room, artist);
        c.text_centered(cx, y + 1.0, TALL_SMALL, rgba(0xc8c8d8, 1.0), &by);

        self.draw_tall_timeline(c, 332.0);

        // Play in the middle, the biggest thing here.
        let loved = self.list.is_favourite(self.list.now());
        let (y, h, gap) = (358.0, 40.0, 6.0);
        let widths = [32.0, 38.0, 52.0, 38.0, 32.0];
        let row = [
            (Control::Shuffle, self.list.is_shuffled()),
            (Control::Prev, false),
            (Control::Toggle, true),
            (Control::Next, false),
            (Control::Repeat, self.list.repeat != Repeat::Off),
        ];
        let total = widths.iter().sum::<f32>() + gap * 4.0;
        let mut x = cx - total / 2.0;
        for ((control, on), bw) in row.into_iter().zip(widths) {
            button(c, [x, y, bw, h], control, f.paused, on);
            if control == Control::Repeat && self.list.repeat == Repeat::One {
                c.text(x + bw - 8.0, y + 3.0, TALL_SMALL, rgba(0xffffff, 1.0), "1");
            }
            x += bw + gap;
        }

        // The speed mods, the pitch and the heart.
        let (y, h) = (404.0, 28.0);
        let heart_w = 34.0;
        button(c, [0.0, y, heart_w, h], Control::Favourite, f.paused, loved);
        let mut x = heart_w + 5.0;
        let speed_w = 46.0;
        let chip = (w - x - speed_w - 5.0 * 4.0) / 4.0;
        for (i, speed) in Speed::ALL.into_iter().enumerate() {
            let on = self.speed == Some(speed);
            let fill = if on {
                crate::render::accent()
            } else {
                0x23232f
            };
            let edge = if on { 0xffffff } else { 0x3a3a4e };
            c.round_rect(
                [x, y],
                [chip, h],
                4.0,
                rgba(fill, 1.0),
                rgba(edge, 1.0),
                1.0,
            );
            let ty = y + (h - TALL_SMALL) / 2.0;
            c.text_centered(
                x + chip / 2.0,
                ty,
                TALL_SMALL,
                rgba(0xffffff, 1.0),
                speed.name(),
            );
            c.hit(x, y, chip, h, Hit::PlayerSpeed(i));
            x += chip + 5.0;
        }
        let label = format!("{:.2}x", self.rate().0);
        let changed = (self.rate().0 - 1.0).abs() > 1e-6 || self.semitones != 0;
        c.button(x, y, speed_w, h, &label, TALL_SMALL, Hit::RateOpen, changed);

        // The volume, and the way into the map.
        let (y, h) = (440.0, 34.0);
        let play = tr!("play map");
        let play_w = 96.0;
        let hit = Hit::Music(Control::PlayMap);
        c.button(w - play_w, y, play_w, h, play, TALL_SMALL, hit, true);
        let volume_w = w - play_w - 12.0;
        self.volume_bar = (0.0, volume_w);
        let step = pixel_size(4.0);
        let blocks = (volume_w / step) as usize;
        let lit = (f.volume.clamp(0.0, 1.0) * blocks as f32).round() as usize;
        for b in 0..blocks {
            let high = pixel_size(4.0 + b as f32 / blocks as f32 * 14.0);
            let col = if b < lit { 0xffffff } else { 0x3a3a4e };
            c.rect(
                snap_to_grid(b as f32 * step),
                snap_to_grid(y + h - 8.0 - high),
                step - pixel_size(1.0),
                high,
                rgba(col, 1.0),
            );
        }
        c.hit(0.0, y - 4.0, volume_w, h + 8.0, Hit::PlayerVolume);
    }

    /// The song's progress across the screen, the times under it.
    fn draw_tall_timeline(&mut self, c: &mut Canvas, y: f32) {
        let (at, len) = self.times;
        let w = c.width;
        self.timeline = (0.0, w, len);
        let done = if len > 0.0 {
            (at / len).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let step = pixel_size(3.0);
        let blocks = (w / step) as usize;
        let lit = (done * blocks as f32).round() as usize;
        for b in 0..blocks {
            let col = if b < lit {
                crate::render::accent()
            } else {
                0x3a3a4e
            };
            c.rect(
                b as f32 * step,
                snap_to_grid(y),
                step - pixel_size(1.0),
                pixel_size(5.0),
                rgba(col, 1.0),
            );
        }
        c.hit(0.0, y - 8.0, w, 24.0, Hit::MusicBar);
        let clock = |ms: f64| {
            let s = (ms / 1000.0).max(0.0) as u32;
            format!("{}:{:02}", s / 60, s % 60)
        };
        let colour = rgba(0xc8c8d8, 1.0);
        c.text(0.0, y + 9.0, TALL_SMALL, colour, &clock(at));
        c.text_right(w, y + 9.0, TALL_SMALL, colour, &clock(len));
    }

    /// The list over the player, the whole screen for it.
    fn draw_tall_list(&mut self, c: &mut Canvas) {
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x0e0e14, 0.9));
        c.button(0.0, 0.0, 44.0, 30.0, "<", TALL_TEXT, Hit::PlayerList, false);
        if self.folders_open {
            self.draw_folders(c);
        } else {
            self.draw_list(c);
        }
        self.draw_folder_button(c);
    }

    /// The name of a folder.
    fn folder_name(folder: Folder) -> &'static str {
        match folder {
            Folder::All => tr!("All songs"),
            Folder::Favourites => tr!("Favourites"),
            Folder::Genre(i) => (GENRES[i].name)(),
            Folder::Other => tr!("No genre"),
        }
    }

    /// The button at the list's top that goes to the folders, showing the one that's open.
    fn draw_folder_button(&self, c: &mut Canvas) {
        let [bx, by, bw, bh] = if tall() {
            [50.0, 0.0, list_w() - 50.0, 30.0]
        } else if crate::mobile::active() {
            [104.0, 0.0, list_w() - 104.0, 46.0]
        } else {
            [94.0, 24.0, list_w() - 84.0, 20.0]
        };
        let hover = c.hovered(bx, by, bw, bh);
        let open = self.folders_open;
        c.round_rect(
            [bx, by],
            [bw, bh],
            4.0,
            rgba(if hover { 0x34344a } else { 0x23232f }, 1.0),
            rgba(
                if open {
                    crate::render::accent()
                } else {
                    0x3a3a4e
                },
                1.0,
            ),
            1.0,
        );
        draw_folder_icon(
            c,
            bx + 6.0,
            by + bh / 2.0 - 5.0,
            pixel_size(1.0),
            folder_colour(self.list.folder),
        );
        c.text(
            bx + 26.0,
            by + (bh - 7.0) / 2.0,
            7.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(7.0, bw - 34.0, Self::folder_name(self.list.folder)),
        );
        c.hit(bx, by, bw, bh, Hit::PlayerFolders);
    }

    /// The folders, one to a row: a pixel folder in its colour, its name and how many.
    fn draw_folders(&self, c: &mut Canvas) {
        const X: f32 = 10.0;
        const GAP: f32 = 3.0;
        self.draw_folder_button(c);
        let folders = self.list.folders();
        let room = rows() as f32 * row_h() + 26.0;
        let h = ((room + GAP) / folders.len().max(1) as f32 - GAP).clamp(17.0, 36.0);
        for (i, &(folder, count)) in folders.iter().enumerate() {
            let y = list_top() + i as f32 * (h + GAP);
            let on = folder == self.list.folder;
            let hover = c.hovered(X, y, list_w(), h);
            let colour = folder_colour(folder);
            c.round_rect(
                [X, y],
                [list_w(), h],
                4.0,
                rgba(if hover { 0x2b2b3d } else { 0x15151d }, 0.92),
                rgba(if on { colour } else { 0x3a3a4e }, 1.0),
                1.0,
            );
            let px = pixel_size(1.0);
            draw_folder_icon(c, X + 7.0, y + (h - 12.0 * px) / 2.0, px, colour);
            let count = count.to_string();
            let name_x = X + 8.0 + 16.0 * px + 8.0;
            let room = X + list_w() - 8.0 - Canvas::text_width(7.0, &count) - 8.0 - name_x;
            c.text(
                name_x,
                y + (h - 7.0) / 2.0,
                7.0,
                rgba(0xffffff, 1.0),
                &Canvas::fit(7.0, room, Self::folder_name(folder)),
            );
            c.text_right(
                X + list_w() - 8.0,
                y + (h - 7.0) / 2.0,
                7.0,
                rgba(0x9a9ab0, 1.0),
                &count,
            );
            c.hit(X, y, list_w(), h, Hit::PlayerFolder(i));
        }
    }

    /// The song list: a search box, then a row for each song, with a heart and a `+`.
    fn draw_list(&mut self, c: &mut Canvas) {
        let x0 = crate::mobile::pick(10.0, 0.0);
        let search_h = if tall() {
            30.0
        } else {
            crate::mobile::pick(18.0, 40.0)
        };
        let text_size = if tall() { TALL_SMALL } else { 7.0 };
        if !tall() {
            self.draw_folder_button(c);
        }
        let border = if self.searching {
            rgba(crate::render::accent(), 1.0)
        } else {
            rgba(0x3a3a4e, 1.0)
        };
        c.round_rect(
            [x0, list_top()],
            [list_w(), search_h],
            4.0,
            rgba(0x15151d, 0.95),
            border,
            1.0,
        );
        let filter = self.list.filter.clone();
        let (text, col) = if filter.is_empty() && !self.searching {
            (
                if crate::mobile::active() {
                    tr!("search").to_string()
                } else {
                    tr!("search (Ctrl+F)").to_string()
                },
                0x6f6f86,
            )
        } else {
            (format!("{filter}_"), 0xffffff)
        };
        c.text(
            x0 + 7.0,
            list_top() + (search_h - text_size) / 2.0,
            text_size,
            rgba(col, 1.0),
            &Canvas::fit_end(text_size, list_w() - 14.0, &text),
        );
        c.hit(x0, list_top(), list_w(), search_h, Hit::PlayerSearch);

        let top = list_top() + search_h + 4.0;
        c.round_rect(
            [x0, top],
            [list_w(), rows() as f32 * row_h() + 4.0],
            4.0,
            rgba(0x15151d, 0.82),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        let now = self.list.now();
        // A new song comes into view; scrolling away from it stays.
        if self.followed != Some(now) {
            self.followed = Some(now);
            if let Some(pos) = self.list.shown.iter().position(|&t| t == now)
                && (pos < self.scroll || pos >= self.scroll + rows())
            {
                self.scroll = pos.saturating_sub(rows() / 2);
                self.scroll_by(0);
            }
        }
        let add_w = if tall() {
            34.0
        } else {
            crate::mobile::pick(18.0, 44.0)
        };
        let heart_w = if tall() {
            30.0
        } else {
            crate::mobile::pick(16.0, 44.0)
        };
        for (row, &track) in self
            .list
            .shown
            .iter()
            .skip(self.scroll)
            .take(rows())
            .enumerate()
        {
            let y = top + 2.0 + row as f32 * row_h();
            let hover = c.hovered(x0, y, list_w(), row_h());
            if track == now {
                c.rect(
                    x0 + 2.0,
                    y,
                    list_w() - 4.0,
                    row_h(),
                    rgba(crate::render::accent(), 0.3),
                );
            } else if hover {
                c.rect(x0 + 2.0, y, list_w() - 4.0, row_h(), rgba(0x34344a, 0.8));
            }
            let room = list_w() - 14.0 - add_w - heart_w;
            c.text(
                x0 + 7.0,
                y + (row_h() - text_size) / 2.0,
                text_size,
                rgba(if track == now { 0xffffff } else { 0xc8c8d8 }, 1.0),
                &Canvas::fit(text_size, room, &self.list.tracks[track].label),
            );
            let heart_x = x0 + list_w() - add_w - heart_w - 2.0;
            c.hit(x0, y, heart_x - x0, row_h(), Hit::PlayerRow(track));
            let loved = self.list.is_favourite(track);
            if loved || hover || tall() {
                draw_heart(
                    c,
                    heart_x + 4.0,
                    y + (row_h() - 8.0) / 2.0,
                    pixel_size(1.0),
                    if loved { 0xff5fa2 } else { 0x6a6a88 },
                );
            }
            c.hit(heart_x, y, heart_w, row_h(), Hit::PlayerFavourite(track));
            if hover || tall() {
                c.text_centered(
                    x0 + list_w() - add_w / 2.0 - 2.0,
                    y + (row_h() - 7.0) / 2.0,
                    7.0,
                    rgba(0xffffff, 1.0),
                    "+",
                );
            }
            c.hit(
                x0 + list_w() - add_w - 2.0,
                y,
                add_w,
                row_h(),
                Hit::PlayerQueue(track),
            );
        }
        if self.list.shown.is_empty() {
            c.text(
                x0 + 7.0,
                top + 9.0,
                7.0,
                rgba(0x9a9ab0, 1.0),
                tr!("nothing found"),
            );
        }
        let count = self.list.shown.len();
        c.text(
            x0 + 2.0,
            top + rows() as f32 * row_h() + 9.0,
            text_size,
            rgba(0x9a9ab0, 1.0),
            &tr!("{count} songs"; count),
        );
    }

    /// What plays next; a click takes it off.
    fn draw_queue(&self, c: &mut Canvas, x: f32) {
        c.text(
            x + 2.0,
            list_top() + crate::mobile::pick(5.5, 14.0),
            7.0,
            rgba(0x9a9ab0, 1.0),
            tr!("UP NEXT"),
        );
        let top = list_top() + crate::mobile::pick(18.0, 40.0) + 4.0;
        c.round_rect(
            [x, top],
            [queue_w(), rows() as f32 * row_h() + 4.0],
            4.0,
            rgba(0x15151d, 0.82),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        for (row, &track) in self.list.queue.iter().take(rows()).enumerate() {
            let y = top + 2.0 + row as f32 * row_h();
            if c.hovered(x, y, queue_w(), row_h()) {
                c.rect(x + 2.0, y, queue_w() - 4.0, row_h(), rgba(0x34344a, 0.8));
            }
            c.text(
                x + 7.0,
                y + (row_h() - 7.0) / 2.0,
                7.0,
                rgba(0xc8c8d8, 1.0),
                &Canvas::fit(7.0, queue_w() - 14.0, &self.list.tracks[track].label),
            );
            c.hit(x, y, queue_w(), row_h(), Hit::PlayerUnqueue(row));
        }
        if self.list.queue.is_empty() {
            c.text(
                x + 7.0,
                top + 9.0,
                7.0,
                rgba(0x6f6f86, 1.0),
                &Canvas::fit(7.0, queue_w() - 14.0, tr!("+ on a song adds it")),
            );
        }
    }

    /// The bottom row: going to the map, the speed mods, the pitch, the volume.
    fn draw_sound(&mut self, c: &mut Canvas, cx: f32, y: f32, volume: f32) {
        let phone = crate::mobile::active();
        let h = crate::mobile::pick(18.0, 36.0);
        let pitch = if self.semitones == 0 {
            if phone {
                tr!("pitch").to_string()
            } else {
                tr!("pitch [ ]").to_string()
            }
        } else {
            if phone {
                format!("{:+}", self.semitones)
            } else {
                format!("{:+} [ ]", self.semitones)
            }
        };
        let play = tr!("play map");
        let play_w = Canvas::text_width(7.0, play) + 12.0;
        let pitch_w = if phone {
            76.0
        } else {
            Canvas::text_width(7.0, &pitch) + 10.0
        };
        let mod_w = crate::mobile::pick(24.0, 40.0);
        let volume_w = crate::mobile::pick(62.0, 80.0);
        const GAP: f32 = 6.0;
        let total = play_w + GAP * 2.0 + (mod_w + GAP) * 4.0 + pitch_w + GAP * 2.0 + volume_w;
        let mut x = cx - total / 2.0;
        c.button(
            x,
            y,
            play_w,
            h,
            play,
            7.0,
            Hit::Music(Control::PlayMap),
            true,
        );
        x += play_w + GAP * 2.0;
        for (i, speed) in Speed::ALL.into_iter().enumerate() {
            let on = self.speed == Some(speed);
            let hover = c.hovered(x, y, mod_w, h);
            let fill = if on {
                crate::render::accent()
            } else if hover {
                0x34344a
            } else {
                0x23232f
            };
            c.round_rect(
                [x, y],
                [mod_w, h],
                4.0,
                rgba(fill, 1.0),
                rgba(if on { 0xffffff } else { 0x3a3a4e }, 1.0),
                1.0,
            );
            c.text_centered(
                x + mod_w / 2.0,
                y + (h - 7.0) / 2.0,
                7.0,
                rgba(0xffffff, 1.0),
                speed.name(),
            );
            c.hit(x, y, mod_w, h, Hit::PlayerSpeed(i));
            x += mod_w + GAP;
        }
        if !phone {
            c.text(
                x + 3.0,
                y + (h - 7.0) / 2.0,
                7.0,
                rgba(
                    if self.semitones == 0 {
                        0x9a9ab0
                    } else {
                        0xffffff
                    },
                    1.0,
                ),
                &pitch,
            );
        }
        if phone {
            // Over the label: on a phone it opens the speed panel, with the pitch in it.
            let label = format!("{:.2}x", self.rate().0);
            let changed = (self.rate().0 - 1.0).abs() > 1e-6 || self.semitones != 0;
            c.button(x, y, pitch_w, h, &label, 7.0, Hit::RateOpen, changed);
        } else {
            c.hit(x, y, pitch_w, h, Hit::PlayerPitch);
        }
        x += pitch_w + GAP * 2.0;
        self.volume_bar = (x, volume_w);
        let step = pixel_size(4.0);
        let blocks = (volume_w / step) as usize;
        let lit = (volume.clamp(0.0, 1.0) * blocks as f32).round() as usize;
        for b in 0..blocks {
            let tall = pixel_size(4.0 + b as f32 / blocks as f32 * 10.0);
            let col = if b < lit {
                rgba(0xffffff, 1.0)
            } else {
                rgba(0x3a3a4e, 1.0)
            };
            c.rect(
                snap_to_grid(x) + b as f32 * step,
                snap_to_grid(y + h - 2.0 - tall),
                step - pixel_size(1.0),
                tall,
                col,
            );
        }
        c.hit(x - 2.0, y, volume_w + 4.0, h, Hit::PlayerVolume);
    }

    /// The song's progress in blocks, with the times either side.
    fn draw_timeline(&mut self, c: &mut Canvas, cx: f32, room: f32) {
        let (at, len) = self.times;
        let w = (room - 80.0).min(420.0);
        let left = cx - w / 2.0;
        self.timeline = (left, w, len);
        let done = if len > 0.0 {
            (at / len).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let step = pixel_size(4.0);
        let blocks = (w / step) as usize;
        let lit = (done * blocks as f32).round() as usize;
        let top = snap_to_grid(bar_y());
        for b in 0..blocks {
            let col = if b < lit {
                rgba(crate::render::accent(), 1.0)
            } else {
                rgba(0x3a3a4e, 1.0)
            };
            c.rect(
                snap_to_grid(left) + b as f32 * step,
                top,
                step - pixel_size(1.0),
                pixel_size(6.0),
                col,
            );
        }
        c.hit(
            left - 4.0,
            bar_y() - 10.0,
            w + 8.0,
            crate::mobile::pick(18.0, 26.0),
            Hit::MusicBar,
        );
        let clock = |ms: f64| {
            format!(
                "{}:{:02}",
                (ms / 60_000.0) as u32,
                (ms / 1000.0) as u32 % 60
            )
        };
        c.text_right(
            left - 8.0,
            bar_y(),
            7.0,
            rgba(0xc8c8d8, 1.0),
            &clock(at.max(0.0)),
        );
        c.text(
            left + w + 8.0,
            bar_y(),
            7.0,
            rgba(0xc8c8d8, 1.0),
            &clock(len),
        );
    }
}

/// A small heart, its top left at (`x`, `y`).
pub fn draw_heart(c: &mut Canvas, x: f32, y: f32, px: f32, colour: u32) {
    let (x, y) = (snap_to_grid(x), snap_to_grid(y));
    for (row, line) in HEART.iter().skip(1).enumerate() {
        for (col, b) in line.bytes().enumerate() {
            if b == b'#' {
                c.rect(
                    x + col as f32 * px,
                    y + row as f32 * px,
                    px,
                    px,
                    rgba(colour, 1.0),
                );
            }
        }
    }
}

fn folder_colour(folder: Folder) -> u32 {
    match folder {
        Folder::All => 0xc8c8d8,
        Folder::Favourites => 0xff5fa2,
        Folder::Genre(i) => GENRES[i].colour,
        Folder::Other => 0x6f6f86,
    }
}

/// A pixel folder, 16 by 12 pixels, its top left at (`x`, `y`): a tab, a lit front, a shaded base.
fn draw_folder_icon(c: &mut Canvas, x: f32, y: f32, px: f32, colour: u32) {
    const ART: [&str; 12] = [
        ".######.........",
        "#++++++#........",
        "#++++++########.",
        "#++++++++++++++#",
        "################",
        "#++++++++++++++#",
        "#++++++++++++++#",
        "#++++++++++++++#",
        "#++++++++++++++#",
        "#--------------#",
        "#--------------#",
        ".##############.",
    ];
    let (x, y) = (snap_to_grid(x), snap_to_grid(y));
    let (line_colour, front, base) = (
        crate::menu::tint(colour, 0x000000, 0.55),
        colour,
        crate::menu::tint(colour, 0x000000, 0.25),
    );
    for (row, line) in ART.iter().enumerate() {
        for (col, b) in line.bytes().enumerate() {
            let shade = match b {
                b'#' => line_colour,
                b'+' => front,
                b'-' => base,
                _ => continue,
            };
            c.rect(
                x + col as f32 * px,
                y + row as f32 * px,
                px,
                px,
                rgba(shade, 1.0),
            );
        }
    }
}

/// The record's pixel parts, around the picture the renderer puts under them: a white rim like a
/// note's and the spindle. Without a background, slices that show it turning fill it.
fn draw_disc(c: &mut Canvas, x: f32, y: f32, angle: f32, pictured: bool) {
    draw_disc_sized(c, x, y, DISC_R, angle, pictured, 1.0);
}

/// The same record at radius `r` and opacity `alpha` (the idle screen's).
pub(crate) fn draw_disc_sized(
    c: &mut Canvas,
    x: f32,
    y: f32,
    r: f32,
    angle: f32,
    pictured: bool,
    alpha: f32,
) {
    let px = pixel_size(r * 2.0 / DISC_PIXELS as f32);
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
                if dx + dy > half * 0.9 {
                    0xc8c8d8
                } else {
                    0xffffff
                }
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
                        c.rect(
                            left + from as f32 * px,
                            top + row as f32 * px,
                            (col - from) as f32 * px,
                            px,
                            rgba(was, alpha),
                        );
                    }
                    run = Some((col, colour));
                }
            }
        }
    }
}

/// The disc's colour without a picture, at `angle` round it.
fn slice_at(angle: f32) -> u32 {
    if (angle / std::f32::consts::FRAC_PI_4).floor() as i32 % 2 == 0 {
        0x34344a
    } else {
        0x23232f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, rate: u32) -> Spectrum {
        let mut sp = Spectrum::default();
        for (i, s) in sp.samples.iter_mut().enumerate() {
            *s = (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin() * 0.5;
        }
        sp
    }

    #[test]
    fn a_tone_lights_up_its_own_band() {
        for hz in [200.0, 1000.0, 8000.0] {
            let bands = tone(hz, 48_000).analyse(48_000);
            let loudest = (0..BANDS)
                .max_by(|&a, &b| bands[a].total_cmp(&bands[b]))
                .unwrap();
            let expected = (hz / LOWEST_HZ).ln() / (HIGHEST_HZ / LOWEST_HZ).ln() * BANDS as f32;
            assert!(
                (loudest as f32 - expected).abs() <= 3.0,
                "{hz} Hz in band {loudest}, not {expected}"
            );
            assert!(
                bands[loudest] > 0.8,
                "{hz} Hz only reaches {}",
                bands[loudest]
            );
        }
    }

    #[test]
    fn silence_shows_nothing() {
        assert!(
            Spectrum::default()
                .analyse(48_000)
                .iter()
                .all(|&v| v == 0.0)
        );
    }

    #[test]
    fn the_bands_fall_back_in_silence() {
        let mut sp = tone(1000.0, 48_000);
        sp.update(16.0, true, 48_000);
        let lit = sp.bands.iter().copied().fold(0.0, f32::max);
        assert!(lit > 0.8);
        sp.update(100.0, false, 48_000);
        let after = sp.bands.iter().copied().fold(0.0, f32::max);
        assert!(
            after < lit && after > 0.0,
            "falls, not drops: {lit} then {after}"
        );
        sp.update(1000.0, false, 48_000);
        assert!(
            sp.bands.iter().all(|&v| v < 0.01),
            "a second of silence and it's gone"
        );
    }

    #[test]
    fn a_disc_without_a_picture_still_shows_it_turning() {
        assert_ne!(slice_at(0.2), slice_at(1.0));
    }
}
