//! The main menu: a lane like the one in a song. The sections ride it as notes, the kat keys
//! bring one to the hit circle and a don key hits it. It's a layer over song select, so the
//! music and the background just carry on.

use crate::game::{DON, KAT};
use crate::player::Control;
use crate::render::{self, approach, pixel_size, rgba, snap_to_grid, Canvas, Hit};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Play,
    Multiplayer,
    Download,
    Editor,
    Settings,
    Exit,
}

pub const ITEMS: [Item; 6] = [Item::Play, Item::Multiplayer, Item::Download, Item::Settings, Item::Editor, Item::Exit];

/// How long a hit note shows before its screen opens (ms).
const LEAVE_MS: f32 = 170.0;
const SHAKE_MS: f32 = 300.0;
const FLASH_MS: f32 = 200.0;
const BURST_MS: f32 = 520.0;

const LANE_TOP: f32 = 118.0;
const LANE_H: f32 = 124.0;
const LANE_MID: f32 = LANE_TOP + LANE_H / 2.0;
const DRUM_W: f32 = 112.0;
const CIRCLE_X: f32 = 188.0;
const NOTE_GAP: f32 = 132.0;
/// The row naming every section, and the hint under it.
const STRIP_Y: f32 = 348.0;
const HINT_Y: f32 = 376.0;
/// How long the lane sits still before the next note starts nudging at the circle (ms).
const NUDGE_AFTER: f32 = 6000.0;
const NUDGE_EVERY: f32 = 2400.0;

const COMING_SOON: &str = "Coming soon";

/// Our Discord server.
pub const DISCORD_INVITE: &str = "https://discord.gg/6JUm46p9Zz";
const DISCORD_BLURPLE: u32 = 0x5865F2;
/// The face on its button, 16 by 12: `#` white, the rest the button's colour.
const DISCORD_FACE: [&str; 12] = [
    "...##......##...",
    "..############..",
    ".##############.",
    ".##############.",
    ".####..##..####.",
    "#####..##..#####",
    "#####..##..#####",
    "################",
    "################",
    ".######..######.",
    "..###......###..",
    "...#........#...",
];

/// A note as pixel art, like the logo: `ART` x `ART` pixels, 0 = none.
const ART: usize = 36;
type Art = [[u32; ART]; ART];
/// The note's radius in art pixels, and how far it sits from the art's top left corner.
const NOTE_R: f32 = 15.5;
const ART_MID: f32 = 17.5;
/// One pixel of the hit circle and the drum (virtual units).
const LANE_PIXEL: f32 = 2.0;
/// The rows the logo's glitch shifts sideways, and how far.
const GLITCH: [(usize, usize); 7] = [(6, 2), (7, 3), (12, 1), (18, 2), (23, 3), (24, 2), (29, 1)];
const GLITCH_RED: u32 = 0x9f2137;
const GLITCH_CYAN: u32 = 0x2a9fb5;
const OUTLINE: u32 = 0x0f0f16;

/// 16 x 16 pixels, a row to a string.
type Icon = [&'static str; 16];

const PLAY: Icon = [
    "................",
    "...##...........",
    "...####.........",
    "...######.......",
    "...########.....",
    "...##########...",
    "...############.",
    "...#############",
    "...#############",
    "...############.",
    "...##########...",
    "...########.....",
    "...######.......",
    "...####.........",
    "...##...........",
    "................",
];
const PEOPLE: Icon = [
    "................",
    "................",
    "....####........",
    "...######..###..",
    "...######.#####.",
    "...######.#####.",
    "....####...###..",
    "................",
    "...########.###.",
    "..##########.###",
    "..##########.###",
    "..##########.###",
    "..##########.###",
    "................",
    "................",
    "................",
];
const ARROW: Icon = [
    "................",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
    "..############..",
    "...##########...",
    "....########....",
    ".....######.....",
    "......####......",
    ".......##.......",
    "................",
    ".##############.",
    ".##############.",
    "................",
];
const PENCIL: Icon = [
    "................",
    "............##..",
    "...........####.",
    "..........#.####",
    ".........###.##.",
    "........#####...",
    ".......#####....",
    "......#####.....",
    ".....#####......",
    "....#####.......",
    "...#####........",
    "..#####.........",
    "..####..........",
    "..##............",
    "..#.............",
    "................",
];
const GEAR: Icon = [
    "................",
    "......####......",
    "......####......",
    "...##.####.##...",
    "..############..",
    "..#####..#####..",
    "...###....###...",
    ".####......####.",
    ".####......####.",
    "...###....###...",
    "..#####..#####..",
    "..############..",
    "...##.####.##...",
    "......####......",
    "......####......",
    "................",
];
const CROSS: Icon = [
    "................",
    ".##..........##.",
    ".###........###.",
    "..###......###..",
    "...###....###...",
    "....###..###....",
    ".....######.....",
    "......####......",
    "......####......",
    ".....######.....",
    "....###..###....",
    "...###....###...",
    "..###......###..",
    ".###........###.",
    ".##..........##.",
    "................",
];

impl Item {
    fn name(self) -> &'static str {
        match self {
            Item::Play => tr!("PLAY", "プレイ", "ИГРАТЬ"),
            Item::Multiplayer => tr!("MULTIPLAYER", "マルチ", "МУЛЬТИПЛЕЕР"),
            Item::Download => tr!("DOWNLOAD", "ダウンロード", "ЗАГРУЗКА"),
            Item::Editor => tr!("EDITOR", "エディタ", "РЕДАКТОР"),
            Item::Settings => tr!("SETTINGS", "設定", "НАСТРОЙКИ"),
            Item::Exit => tr!("EXIT", "終了", "ВЫХОД"),
        }
    }

    fn about(self, maps: usize) -> String {
        match self {
            Item::Play => tr!("{maps} maps in your library", "ライブラリに{maps}譜面", "{maps} карт в библиотеке";),
            Item::Multiplayer => tr!("Play with friends and everyone else", "フレンドやみんなと遊ぶ", "Играть с друзьями и не только").into(),
            Item::Download => tr!("Find and get new maps", "新しい譜面を探す", "Найти и скачать новые карты").into(),
            Item::Editor => tr!("Your own maps, right in the game", "自分の譜面をゲーム内で", "Свои карты прямо в игре").into(),
            Item::Settings => tr!("Sound, keys, skin, offset", "音、キー、スキン、オフセット", "Звук, клавиши, скин, оффсет").into(),
            Item::Exit => tr!("Close taiko.rs", "taiko.rsを閉じる", "Закрыть taiko.rs").into(),
        }
    }

    fn colour(self) -> u32 {
        match self {
            Item::Play | Item::Download => DON,
            Item::Multiplayer | Item::Settings => KAT,
            Item::Editor => 0x23232f,
            Item::Exit => 0x6f6f86,
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Item::Play => &PLAY,
            Item::Multiplayer => &PEOPLE,
            Item::Download => &ARROW,
            Item::Editor => &PENCIL,
            Item::Settings => &GEAR,
            Item::Exit => &CROSS,
        }
    }

    /// Its note: a white rim with a dark outline, a lit and a shaded edge, the logo's glitch
    /// down the sides. The editor isn't a real note yet: only a dashed outline, in grey.
    fn art(self) -> Art {
        let mut art = [[0; ART]; ART];
        let ghost = self == Item::Editor;
        let fill = self.colour();
        for (y, row) in art.iter_mut().enumerate() {
            for (x, px) in row.iter_mut().enumerate() {
                let (dx, dy) = (x as f32 - ART_MID, y as f32 - ART_MID);
                let d = dx.hypot(dy);
                if d > NOTE_R {
                    continue;
                }
                let slant = dx + dy;
                *px = if ghost {
                    let dash = ((dy.atan2(dx) / std::f32::consts::TAU + 0.5) * 20.0) as i32;
                    if d > 13.0 && dash % 2 == 0 { 0x8a8aa0 } else { fill }
                } else if d > 14.4 {
                    OUTLINE
                } else if d > 11.6 {
                    if slant > 11.0 { 0xc8c8d8 } else { 0xffffff }
                } else if d > 10.2 && slant < -6.0 {
                    tint(fill, 0xffffff, 0.3)
                } else if d > 10.2 && slant > 6.0 {
                    tint(fill, 0x000000, 0.28)
                } else {
                    fill
                };
            }
        }
        if !ghost {
            for (y, by) in GLITCH {
                let row = &mut art[y];
                // Over the outline, so it starts at the white rim.
                let rim = |p: &u32| *p != 0 && *p != OUTLINE;
                let (Some(left), Some(right)) = (row.iter().position(rim), row.iter().rposition(rim)) else { continue };
                row[left.saturating_sub(by + 1)..left].fill(GLITCH_RED);
                row[right + 1..(right + 2 + by).min(ART)].fill(GLITCH_CYAN);
            }
        }
        let (ink, shadow) = if ghost { (0x8a8aa0, 0x15151d) } else { (0xffffff, tint(fill, 0x000000, 0.45)) };
        let at = (ART - 16) / 2;
        let pixels = || self.icon().iter().enumerate().flat_map(|(y, line)| line.bytes().enumerate().filter(|b| b.1 == b'#').map(move |(x, _)| (x + at, y + at)));
        for (x, y) in pixels() {
            art[y + 1][x + 1] = shadow;
        }
        for (x, y) in pixels() {
            art[y][x] = ink;
        }
        art
    }
}

/// `col` moved `t` of the way to `to`.
pub fn tint(col: u32, to: u32, t: f32) -> u32 {
    let mix = |shift: u32| {
        let (a, b) = ((col >> shift & 255) as f32, (to >> shift & 255) as f32);
        ((a + (b - a) * t) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

/// What a frame of the menu shows besides itself.
pub struct Frame<'a> {
    /// Into the beat (ms) and the beat's length, while a song plays.
    pub beat: Option<(f64, f64)>,
    pub song: Option<&'a str>,
    pub hint: &'a str,
    pub maps: usize,
    pub paused: bool,
}

pub struct Menu {
    /// The selected note, counted on forever so it always rides the same way round.
    selected: i32,
    /// Where the lane has got to on the way there.
    shown: f32,
    /// ms since the menu opened: every animation below is timed on it.
    clock: f32,
    /// When the rim last flashed, and which side.
    rim: (f32, i32),
    face: f32,
    /// A hit: when, and on what.
    burst: Option<(f32, Item)>,
    shake: f32,
    leaving: Option<(Item, f32)>,
    /// When the lane last moved, for the nudge at a player who hasn't found out it does.
    moved: f32,
    arts: Box<[Art; ITEMS.len()]>,
}

impl Menu {
    pub fn new() -> Menu {
        Menu { selected: 0, shown: 0.0, clock: 0.0, rim: (f32::MIN, 1), face: f32::MIN, burst: None, shake: f32::MIN, leaving: None, moved: 0.0, arts: Box::new(ITEMS.map(Item::art)) }
    }

    fn index(&self) -> usize {
        self.selected.rem_euclid(ITEMS.len() as i32) as usize
    }

    pub fn current(&self) -> Item {
        ITEMS[self.index()]
    }

    pub fn is_selected(&self, i: usize) -> bool {
        i == self.index()
    }

    pub fn step(&mut self, by: i32) {
        self.selected += by;
        self.rim = (self.clock, by);
        self.moved = self.clock;
    }

    /// Brings note `i` to the circle the short way round.
    pub fn pick(&mut self, i: usize) {
        let n = ITEMS.len() as i32;
        let mut by = (i as i32 - self.selected).rem_euclid(n);
        if by > n / 2 {
            by -= n;
        }
        if by != 0 {
            self.step(by);
        }
    }

    /// A don on the selected note. The editor isn't there yet: it just shakes it off.
    pub fn hit(&mut self) -> Item {
        let item = self.current();
        self.face = self.clock;
        self.burst = Some((self.clock, item));
        if item == Item::Editor {
            self.shake = self.clock;
        } else {
            self.leaving = Some((item, self.clock + LEAVE_MS));
        }
        item
    }

    /// A note was hit and its screen is about to open.
    pub fn is_leaving(&self) -> bool {
        self.leaving.is_some()
    }

    pub fn cancel_leaving(&mut self) {
        self.leaving = None;
    }

    /// The screen to open now, once the hit has had its moment.
    pub fn ready(&mut self) -> Option<Item> {
        let (item, at) = self.leaving?;
        (self.clock >= at).then(|| {
            self.leaving = None;
            item
        })
    }

    pub fn draw(&mut self, c: &mut Canvas, f: &Frame) {
        self.clock += c.dt_ms;
        let target = self.selected as f32;
        approach(&mut self.shown, target, c.dt_ms, 45.0);
        if (self.shown - target).abs() < 0.002 {
            self.shown = target;
        }
        let (into, beat) = f.beat.filter(|b| b.1 > 1.0).unwrap_or((self.clock as f64, 500.0));
        let phase = (into / beat).rem_euclid(1.0) as f32;
        let bar = (into / (beat * 4.0)).rem_euclid(1.0) as f32;
        let pulse = if render::animations() { (1.0 - phase).powi(3) } else { 0.0 };

        crate::settings::brand(c);
        self.draw_lane(c, bar, pulse);
        self.draw_drum(c);
        self.draw_burst(c);
        self.draw_title(c, f.maps);
        self.draw_strip(c, f.hint);
        draw_footer(c, f);
    }

    fn draw_lane(&self, c: &mut Canvas, bar: f32, pulse: f32) {
        c.rect(0.0, LANE_TOP, c.width, LANE_H, rgba(0x15151d, 0.93));
        c.rect(0.0, LANE_TOP - 2.0, c.width, 2.0, rgba(0x3a3a4e, 1.0));
        c.rect(0.0, LANE_TOP + LANE_H, c.width, 2.0, rgba(0x3a3a4e, 1.0));
        // Bar lines run with the song; a tick on every beat.
        let bars = NOTE_GAP * 2.0;
        let mut x = CIRCLE_X - bar * bars;
        while x < c.width {
            c.rect(x, LANE_TOP, 2.0, LANE_H, rgba(0xffffff, 0.14));
            for tick in 1..4 {
                c.rect(x + tick as f32 * bars / 4.0, LANE_MID - 4.0, 2.0, 8.0, rgba(0xffffff, 0.13));
            }
            x += bars;
        }
        let grow = 1.0 + pulse * 0.05;
        let px = pixel_size(LANE_PIXEL);
        pixel_ring(c, [CIRCLE_X, LANE_MID], 46.0 * grow, 46.0, 0, px, rgba(0xffffff, 0.04));
        pixel_ring(c, [CIRCLE_X, LANE_MID], 46.0 * grow, px * 2.0, 0, px, rgba(0xffffff, 0.5));
        pixel_ring(c, [CIRCLE_X, LANE_MID], 33.0, px, 0, px, rgba(0xffffff, 0.24));

        let first = self.shown.floor() as i32;
        let n = ITEMS.len() as i32;
        let mut hint = None;
        for k in (0..=n).rev() {
            let i = (first + k).rem_euclid(n) as usize;
            let item = ITEMS[i];
            let x = CIRCLE_X + (first + k) as f32 * NOTE_GAP - self.shown * NOTE_GAP - if k == 1 { self.nudge() } else { 0.0 };
            if x > c.width + 60.0 {
                continue;
            }
            let close = (1.0 - (x - CIRCLE_X).abs() / NOTE_GAP).clamp(0.0, 1.0);
            let r = 29.0 + close * 11.0 + if close > 0.5 { pulse * 2.0 } else { 0.0 };
            let fade = ((x - 130.0) / 50.0).clamp(0.2, 1.0);
            let shaking = item == Item::Editor && self.is_selected(i) && self.clock - self.shake < SHAKE_MS;
            let dx = if shaking { (self.clock / 18.0).sin() * 5.0 * (1.0 - (self.clock - self.shake) / SHAKE_MS) } else { 0.0 };
            draw_note(c, &self.arts[i], x + dx, LANE_MID, r, fade);
            // A note just coming on screen shows its name once all of it fits.
            if close < 0.5 && x + Canvas::text_width(7.0, item.name()) / 2.0 < c.width - 4.0 {
                c.text_centered(x, LANE_MID + r + 9.0, 7.0, rgba(0xc8c8d8, fade), item.name());
            }
            c.hit(x - 42.0, LANE_TOP, 84.0, LANE_H, Hit::MenuItem(i));
            if item == Item::Editor && c.hovered(x - 42.0, LANE_TOP, 84.0, LANE_H) {
                hint = Some(x);
            }
        }
        if let Some(x) = hint {
            tooltip(c, x, LANE_TOP - 26.0);
        }
    }

    /// The head of the lane: the rim lights up on kat, the face on don.
    fn draw_drum(&self, c: &mut Canvas) {
        let x = DRUM_W / 2.0;
        c.rect(0.0, LANE_TOP, DRUM_W, LANE_H, rgba(0x1d1d27, 1.0));
        c.rect(DRUM_W, LANE_TOP, 2.0, LANE_H, rgba(0x3a3a4e, 1.0));
        let px = pixel_size(LANE_PIXEL);
        pixel_ring(c, [x, LANE_MID], 44.0, 44.0, 0, px, rgba(0x23232f, 1.0));
        pixel_ring(c, [x, LANE_MID], 44.0, px * 2.0, 0, px, rgba(0x6a6a88, 1.0));
        let rim = 1.0 - (self.clock - self.rim.0) / FLASH_MS;
        if rim > 0.0 {
            pixel_ring(c, [x, LANE_MID], 42.0, 14.0, self.rim.1.signum(), px, rgba(KAT, rim));
        }
        pixel_ring(c, [x, LANE_MID], 28.0, 28.0, 0, px, rgba(0x2d2d3c, 1.0));
        let face = (1.0 - (self.clock - self.face) / FLASH_MS).max(0.0);
        if face > 0.0 {
            pixel_ring(c, [x, LANE_MID], 28.0, 28.0, 0, px, rgba(DON, 0.35 + 0.65 * face));
        }
        c.rect(x - 0.5, LANE_MID - 28.0, 1.0, 56.0, rgba(0xffffff, 0.2));
    }

    fn draw_burst(&self, c: &mut Canvas) {
        let Some((at, item)) = self.burst else { return };
        let k = (self.clock - at) / BURST_MS;
        if k >= 1.0 {
            return;
        }
        let a = 1.0 - k;
        let miss = item == Item::Editor;
        let px = pixel_size(LANE_PIXEL);
        pixel_ring(c, [CIRCLE_X, LANE_MID], 40.0 + k * 70.0, px * 2.0, 0, px, rgba(if miss { 0x8a8aa0 } else { item.colour() }, a * 0.9));
        let word = if miss { tr!("SOON", "もうすぐ", "СКОРО") } else { "GREAT" };
        c.text_centered(CIRCLE_X, LANE_TOP - 22.0 - k * 18.0, 12.0, rgba(0xFFD34D, a), word);
    }

    fn draw_title(&self, c: &mut Canvas, maps: usize) {
        let item = self.current();
        let width = c.text(40.0, 274.0, 34.0, rgba(0xffffff, 1.0), item.name());
        c.text(42.0, 318.0, 10.0, rgba(0xc8c8d8, 1.0), &item.about(maps));
        if item == Item::Editor {
            tooltip(c, 40.0 + width + 62.0, 281.0);
        }
    }

    /// How far the next note leans towards the circle: a small hop now and then while nothing
    /// has moved, showing the lane is there to be moved.
    fn nudge(&self) -> f32 {
        if render::animations() { nudge_after(self.clock - self.moved) } else { 0.0 }
    }

    /// Every section by name with arrows at the ends: nothing has to be found by scrolling, and
    /// the arrows say that the lane moves. What moves it is spelled out underneath.
    fn draw_strip(&self, c: &mut Canvas, hint: &str) {
        const LEFT: f32 = 40.0;
        const ARROW: f32 = 22.0;
        const GAP: f32 = 18.0;
        let room = c.width - 2.0 * LEFT - 2.0 * (ARROW + 8.0);
        let names = ITEMS.map(Item::name);
        let size = Canvas::fit_size(9.0, 6.0, room - GAP * 5.0, &names.concat());
        let total: f32 = names.iter().map(|n| Canvas::text_width(size, n)).sum();
        let gap = ((room - total) / 5.0).clamp(4.0, GAP);
        c.button(LEFT, STRIP_Y - 5.0, ARROW, 20.0, "<", 8.0, Hit::MenuStep(-1), false);
        let mut x = LEFT + ARROW + 8.0;
        let y = STRIP_Y + (9.0 - size) / 2.0;
        for (i, name) in names.iter().enumerate() {
            let w = Canvas::text_width(size, name);
            let (on, hover) = (self.is_selected(i), c.hovered(x - gap / 2.0, STRIP_Y - 6.0, w + gap, 22.0));
            let col = match (on, hover) {
                (true, _) | (_, true) => 0xffffff,
                _ if ITEMS[i] == Item::Editor => 0x6f6f86,
                _ => 0xa8a8bc,
            };
            c.text(x, y, size, rgba(col, 1.0), name);
            if on {
                c.rect(x, STRIP_Y + 13.0, w, 2.0, rgba(render::accent(), 1.0));
            }
            c.hit(x - gap / 2.0, STRIP_Y - 6.0, w + gap, 22.0, Hit::MenuItem(i));
            x += w + gap;
        }
        c.button(x - gap + 8.0, STRIP_Y - 5.0, ARROW, 20.0, ">", 8.0, Hit::MenuStep(1), false);
        c.text(LEFT + 2.0, HINT_Y, 7.0, rgba(0x9a9ab0, 1.0), &Canvas::fit_hints(7.0, c.width - 2.0 * LEFT, hint));
    }
}

/// The next note's lean after `still` ms without the lane moving.
fn nudge_after(still: f32) -> f32 {
    let idle = still - NUDGE_AFTER;
    if idle < 0.0 {
        return 0.0;
    }
    let t = (idle % NUDGE_EVERY) / 420.0;
    if t < 1.0 { (t * std::f32::consts::PI).sin() * 16.0 } else { 0.0 }
}

/// A note `r` units in radius, its pixels a whole number of screen pixels: it grows in steps,
/// staying sharp.
fn draw_note(c: &mut Canvas, art: &Art, x: f32, y: f32, r: f32, alpha: f32) {
    let px = pixel_size(r / NOTE_R);
    pixel_ring(c, [x, y + px * 2.0], NOTE_R * px, NOTE_R * px, 0, px, rgba(0x000000, 0.35 * alpha));
    let half = ART as f32 * px / 2.0;
    let (left, top) = (snap_to_grid(x - half), snap_to_grid(y - half));
    for (row, line) in art.iter().enumerate() {
        let mut col = 0;
        while col < ART {
            let end = line[col..].iter().position(|&p| p != line[col]).map_or(ART, |n| col + n);
            if line[col] != 0 {
                c.rect(left + col as f32 * px, top + row as f32 * px, (end - col) as f32 * px, px, rgba(line[col], alpha));
            }
            col = end;
        }
    }
}

/// A ring `width` thick inside radius `r` in pixels `px` across (as wide as `r`: a disc). `side`:
/// only the left (-1) or right (1) half; 0 the whole of it.
pub fn pixel_ring(c: &mut Canvas, [x, y]: [f32; 2], r: f32, width: f32, side: i32, px: f32, col: [f32; 4]) {
    let (x, y) = (snap_to_grid(x), snap_to_grid(y));
    let (outer, inner) = (r / px, (r - width) / px);
    let n = outer.ceil() as i32;
    for row in -n..n {
        let dy = row as f32 + 0.5;
        let mut start = None;
        for column in -n..=n {
            let dx = column as f32 + 0.5;
            let d = dx.hypot(dy);
            let on = column < n && d <= outer && d > inner && (side == 0 || (side < 0) == (dx < 0.0));
            match (on, start) {
                (true, None) => start = Some(column),
                (false, Some(from)) => {
                    c.rect(x + from as f32 * px, y + row as f32 * px, (column - from) as f32 * px, px, col);
                    start = None;
                }
                _ => {}
            }
        }
    }
}

/// An item's note on its own (the guide's pictures).
pub fn note(c: &mut Canvas, item: Item, x: f32, y: f32, r: f32) {
    draw_note(c, &item.art(), x, y, r, 1.0);
}

fn tooltip(c: &mut Canvas, x: f32, y: f32) {
    let w = Canvas::text_width(8.0, COMING_SOON) + 16.0;
    let left = (x - w / 2.0).clamp(8.0, (c.width - w - 8.0).max(8.0));
    c.round_rect([left, y], [w, 19.0], 5.0, rgba(0x1d1d27, 1.0), rgba(0xFFD34D, 1.0), 1.0);
    c.text(left + 8.0, y + 5.5, 8.0, rgba(0xFFD34D, 1.0), COMING_SOON);
}

/// The way to our Discord server, in pixels like the notes.
fn draw_discord(c: &mut Canvas, right: f32, bottom: f32) {
    let px = pixel_size(1.5);
    let (w, h) = (16.0 * px + 12.0, 12.0 * px + 10.0);
    let (x, y) = (right - w, bottom - h);
    let hover = c.hovered(x, y, w, h);
    let fill = if hover { tint(DISCORD_BLURPLE, 0xffffff, 0.15) } else { DISCORD_BLURPLE };
    c.round_rect([x, y], [w, h], 5.0, rgba(fill, 1.0), rgba(if hover { 0xffffff } else { tint(DISCORD_BLURPLE, 0x000000, 0.3) }, 1.0), 1.0);
    let (left, top) = (snap_to_grid(x + 6.0), snap_to_grid(y + 5.0));
    for (row, line) in DISCORD_FACE.iter().enumerate() {
        for (col, b) in line.bytes().enumerate() {
            if b == b'#' {
                c.rect(left + col as f32 * px, top + row as f32 * px, px, px, rgba(0xffffff, 1.0));
            }
        }
    }
    c.hit(x, y, w, h, Hit::DiscordServer);
    if hover {
        c.text_right(x - 6.0, y + (h - 7.0) / 2.0, 7.0, rgba(0xffffff, 1.0), "Discord");
    }
}

fn draw_footer(c: &mut Canvas, f: &Frame) {
    const Y: f32 = 436.0;
    draw_discord(c, c.width - 14.0, Y + 32.0);
    let Some(song) = f.song else { return };
    const BUTTON_W: f32 = 26.0;
    const BUTTON_GAP: f32 = 4.0;
    const SONG_W: f32 = 240.0;
    // The same size whatever plays: a long name is cut, and the buttons stay put.
    let w = SONG_W.min((c.width - 100.0 - (BUTTON_W + BUTTON_GAP) * 4.0).max(120.0));
    let song = Canvas::fit(8.0, w - 22.0, song);
    c.round_rect([14.0, Y - 6.0], [w, 38.0], 6.0, rgba(0x15151d, 0.88), rgba(0x3a3a4e, 1.0), 1.0);
    c.text(25.0, Y + 1.0, 7.0, rgba(0x9a9ab0, 1.0), tr!("NOW PLAYING", "再生中", "СЕЙЧАС ИГРАЕТ"));
    c.text(25.0, Y + 16.0, 8.0, rgba(0xffffff, 1.0), &song);
    let controls = [Control::Prev, Control::Toggle, Control::Next, Control::Open];
    for (i, control) in controls.into_iter().enumerate() {
        let x = 14.0 + w + 6.0 + i as f32 * (BUTTON_W + BUTTON_GAP);
        crate::player::button(c, [x, Y + 1.0, BUTTON_W, 24.0], control, f.paused, control == Control::Open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_short_way_round_and_back_to_the_same_note() {
        let mut m = Menu::new();
        m.pick(5);
        assert_eq!(m.selected, -1, "Exit is one step back from Play");
        assert_eq!(m.current(), Item::Exit);
        m.step(2);
        assert_eq!(m.current(), Item::Multiplayer);
        m.pick(1);
        assert_eq!(m.selected, 1, "already there");
    }

    #[test]
    fn the_editor_is_only_shaken_the_rest_open_after_the_hit() {
        let mut m = Menu::new();
        m.pick(4);
        assert_eq!(m.hit(), Item::Editor);
        assert!(!m.is_leaving());
        m.pick(0);
        assert_eq!(m.hit(), Item::Play);
        assert_eq!(m.ready(), None);
        m.clock += LEAVE_MS;
        assert_eq!(m.ready(), Some(Item::Play));
        assert_eq!(m.ready(), None);
    }

    #[test]
    fn the_next_note_only_nudges_once_the_lane_has_sat_still() {
        assert_eq!(nudge_after(0.0), 0.0);
        assert_eq!(nudge_after(NUDGE_AFTER - 1.0), 0.0);
        assert!(nudge_after(NUDGE_AFTER + 210.0) > 15.0);
        assert_eq!(nudge_after(NUDGE_AFTER + 1000.0), 0.0, "a hop, then still until the next");
        let mut m = Menu::new();
        m.clock = NUDGE_AFTER * 2.0;
        m.step(1);
        assert_eq!(m.clock - m.moved, 0.0, "moving it starts the wait over");
    }

    #[test]
    fn icons_are_16_pixels_square_and_notes_stay_inside_their_art() {
        for item in ITEMS {
            assert!(item.icon().iter().all(|row| row.len() == 16), "{item:?}");
            assert!(item.art().iter().any(|row| row.iter().any(|&p| p != 0)), "{item:?}");
        }
        assert_eq!(tint(0x204060, 0xffffff, 0.0), 0x204060);
        assert_eq!(tint(0x204060, 0x000000, 1.0), 0x000000);
    }

    #[test]
    fn the_discord_face_is_16_by_12() {
        assert!(DISCORD_FACE.iter().all(|row| row.len() == 16));
        assert!(DISCORD_INVITE.starts_with("https://discord.gg/"));
    }
}
