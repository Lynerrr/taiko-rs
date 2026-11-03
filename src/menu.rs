//! The main menu: a lane like the one in a song. The sections ride it as notes, the kat keys
//! bring one to the hit circle and a don key hits it. It's a layer over song select, so the
//! music and the background just carry on.

use crate::game::{DON, KAT};
use crate::player::Control;
use crate::render::{self, approach, pixel_size, rgba, snap_to_grid, Canvas, Hit, Sprite};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Play,
    Multiplayer,
    Download,
    Medals,
    Editor,
    Settings,
    Exit,
}

pub const ITEMS: [Item; 7] = [Item::Play, Item::Multiplayer, Item::Download, Item::Medals, Item::Settings, Item::Editor, Item::Exit];

/// How long a hit note shows before its screen opens (ms): the menu fades out meanwhile.
const LEAVE_MS: f32 = 220.0;
/// How long the menu takes to fade in, opened or come back to (ms).
pub const FADE_MS: f32 = 220.0;
const SHAKE_MS: f32 = 300.0;
const FLASH_MS: f32 = 200.0;
const BURST_MS: f32 = 520.0;

/// The panel of notes, with the selected section's name and what it is under it: the two
/// together in the middle of the screen's height.
const LANE_H: f32 = 124.0;
const TITLE_GAP: f32 = 22.0;
/// The name's height, the gap to the line about it and that line's.
const TITLE_H: f32 = 44.0 + 10.0;
const LANE_TOP: f32 = (render::VIRTUAL_HEIGHT - (LANE_H + TITLE_GAP + TITLE_H)) / 2.0;
const LANE_MID: f32 = LANE_TOP + LANE_H / 2.0;
/// The panel the notes sit on, in from the screen's sides.
const PANEL_X: f32 = 12.0;
const DRUM_W: f32 = 112.0;
/// Where the notes' middles are, and their biggest radius (narrow screens take less).
const NOTE_Y: f32 = LANE_MID - 10.0;
const NOTE_MAX_R: f32 = 29.0;
/// How much bigger the selected note is (it grows as the selector arrives).
const GROW: f32 = 0.4;
/// The song's own notes cross the panel like a play's, behind the menu's: a big one as big as the
/// drum, a small one in proportion (as in a song), hit in a target just past the drum. They stay in
/// the background: this faint.
const SONG_BIG_R: f32 = 44.0;
const SONG_NOTE_R: f32 = SONG_BIG_R * crate::game::NOTE_R / crate::game::BIG_R;
const SONG_ALPHA: f32 = 0.18;
/// How far apart 1/4 notes are, in small notes' widths: the scroll speed follows the song's
/// As in a song at osu!'s default slider multiplier: 1.4 x 1.4 x 100 osu!px a beat (the game's
/// units are osu!px), here in proportion to the panel's notes.
const SLIDER_MULTIPLIER: f64 = 1.4;
const TAIKO_VELOCITY: f64 = 1.4 * 100.0;
/// How long a hit lights the target (ms).
const SONG_HIT_MS: f32 = 140.0;
const TITLE_Y: f32 = LANE_TOP + LANE_H + TITLE_GAP;
/// Kiai: how long a beat's kick and the opening's flash last, and a spark's life (ms).
const BEAT_MS: f32 = 300.0;
const OPENING_MS: f32 = 420.0;
const SPARK_MS: f32 = 750.0;
/// How many sparks can be in the air, and how many a beat and the opening throw.
const SPARKS: usize = 64;
const SPARKS_A_BEAT: usize = 5;
const SPARKS_OPENING: usize = 18;
const GOLD: u32 = 0xFFD34D;

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
const MEDAL: Icon = [
    "..###......###..",
    "...###....###...",
    "....###..###....",
    ".....######.....",
    "......####......",
    ".....######.....",
    "...##########...",
    "..####....####..",
    "..###......###..",
    ".###...##...###.",
    ".###...##...###.",
    "..###......###..",
    "..####....####..",
    "...##########...",
    ".....######.....",
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
            Item::Medals => tr!("MEDALS", "メダル", "МЕДАЛИ"),
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
            Item::Medals => tr!("What you've earned, and what's next", "獲得したメダルと次の目標", "Что уже получено и что дальше").into(),
            Item::Editor => tr!("Your own maps, right in the game", "自分の譜面をゲーム内で", "Свои карты прямо в игре").into(),
            Item::Settings => tr!("Sound, keys, skin, offset", "音、キー、スキン、オフセット", "Звук, клавиши, скин, оффсет").into(),
            Item::Exit => tr!("Close taiko.rs", "taiko.rsを閉じる", "Закрыть taiko.rs").into(),
        }
    }

    fn colour(self) -> u32 {
        match self {
            Item::Play | Item::Download => DON,
            Item::Multiplayer | Item::Settings => KAT,
            Item::Medals => 0xE8B020,
            Item::Editor => 0x23232f,
            Item::Exit => 0x6f6f86,
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Item::Play => &PLAY,
            Item::Multiplayer => &PEOPLE,
            Item::Download => &ARROW,
            Item::Medals => &MEDAL,
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
    /// The music is in a kiai, and the menu's to join in.
    pub kiai: bool,
    /// The song's notes for the background.
    pub ghosts: Option<crate::menu_notes::View<'a>>,
    pub song: Option<&'a str>,
    pub maps: usize,
    pub paused: bool,
}

pub struct Menu {
    /// The selected note, counted on forever (`index` wraps it).
    selected: i32,
    /// Where the selector has got to on its way to the selected note's slot.
    shown: f32,
    /// When the menu started fading in (opened, or come back to from a screen it opened).
    appear: f32,
    /// The selected note's middle as last drawn: sparks and bursts come from there.
    spot: [f32; 2],
    /// The song's position (map ms) last frame: the notes it passed since are hit on the drum.
    song_at: Option<f64>,
    /// Which rim the next kat of the song lands on.
    song_hand: i32,
    /// The song's last hit: when, and whether a kat.
    song_hit: (f32, bool),
    /// ms since the menu opened: every animation below is timed on it.
    clock: f32,
    /// When the rim last flashed, and which side.
    rim: (f32, i32),
    /// When the face last lit, and which half (left -1, right 1): a hand hits one side.
    face: (f32, i32),
    /// How many beats into the song the music is, when it plays: the menu's notes float with
    /// it, and hop in a kiai.
    beats: Option<f64>,
    /// A hit: when, and on what.
    burst: Option<(f32, Item)>,
    shake: f32,
    leaving: Option<(Item, f32)>,
    /// Kiai: how far into it (0 to 1, easing both ways), whether it's on, when it opened, the
    /// beat it's on and when that came, and what's flying.
    kiai: f32,
    in_kiai: bool,
    kiai_at: f32,
    beat_no: i64,
    beat_at: f32,
    sparks: Vec<Spark>,
    /// For where the sparks go.
    rng: u32,
    /// How far the universes have moved along the routes behind the menu.
    universe: usize,
    arts: Box<[Art; ITEMS.len()]>,
}

/// A pixel thrown from the hit circle: where it is, its speed (units a ms), when it left.
struct Spark {
    at: [f32; 2],
    speed: [f32; 2],
    born: f32,
    col: u32,
}

impl Menu {
    pub fn new() -> Menu {
        Menu { selected: 0, shown: 0.0, appear: 0.0, spot: [0.0, NOTE_Y], song_at: None, song_hand: 1, song_hit: (f32::MIN, false), clock: 0.0, rim: (f32::MIN, 1), face: (f32::MIN, 1), beats: None, burst: None, shake: f32::MIN, leaving: None, kiai: 0.0, in_kiai: false, kiai_at: f32::MIN, beat_no: i64::MIN, beat_at: f32::MIN, sparks: Vec::new(), rng: 0x9E37_79B9, universe: 0, arts: Box::new(ITEMS.map(Item::art)) }
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
    }

    /// The player hit the drum on `side` (left -1, right 1): its rim for kat, that half of the
    /// face for don. The drum lights only for that and for the song's own notes.
    pub fn drum(&mut self, kat: bool, side: i32) {
        if kat {
            self.rim = (self.clock, side);
        } else {
            self.face = (self.clock, side);
        }
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

    /// The screen to open now, once the hit has had its moment. Shown again later (back from
    /// that screen), the menu fades in.
    pub fn ready(&mut self) -> Option<Item> {
        let (item, at) = self.leaving?;
        (self.clock >= at).then(|| {
            self.leaving = None;
            self.appear = self.clock;
            item
        })
    }

    /// How much of the menu shows: fading in after it appears, out while leaving (but not for
    /// Exit, which only asks).
    fn opacity(&self) -> f32 {
        let fade_in = ((self.clock - self.appear) / FADE_MS).clamp(0.0, 1.0);
        let fade_out = match self.leaving {
            Some((item, at)) if item != Item::Exit => ((at - self.clock) / LEAVE_MS).clamp(0.0, 1.0),
            _ => 1.0,
        };
        let k = fade_in.min(fade_out);
        k * k * (3.0 - 2.0 * k)
    }

    pub fn draw(&mut self, c: &mut Canvas, f: &Frame) {
        self.clock += c.dt_ms;
        let target = self.index() as f32;
        approach(&mut self.shown, target, c.dt_ms, 45.0);
        if (self.shown - target).abs() < 0.002 || !render::animations() {
            self.shown = target;
        }
        let (into, beat) = f.beat.filter(|b| b.1 > 1.0).unwrap_or((self.clock as f64, 500.0));
        let playing = f.beat.is_some_and(|b| b.1 > 1.0) && !f.paused && render::animations();
        self.beats = playing.then_some(into / beat);
        self.feel_kiai(f.kiai && render::animations(), f.beat.filter(|b| b.1 > 1.0), c.dt_ms);

        if let Some(ghosts) = &f.ghosts {
            crate::menu_notes::draw(c, ghosts, self.clock, self.kiai, self.kick(), self.universe);
        }
        crate::settings::brand(c);
        self.draw_flashes(c);
        self.draw_panel(c);
        self.play_song(c, f.ghosts.as_ref(), f.beat.map(|b| b.1));
        self.draw_lane(c);
        self.draw_drum(c);
        self.draw_burst(c);
        self.draw_kiai(c);
        self.draw_title(c, f.maps);
        draw_footer(c, f);
        // Coming in out of black, and going to black when it leaves: the next screen comes up
        // out of it.
        let shown = if render::animations() { self.opacity() } else { 1.0 };
        if shown < 1.0 {
            c.rect(0.0, 0.0, c.width, render::VIRTUAL_HEIGHT, rgba(0x000000, 1.0 - shown));
        }
    }

    /// What's left of this beat's kick, 1 to 0.
    fn kick(&self) -> f32 {
        (1.0 - (self.clock - self.beat_at) / BEAT_MS).clamp(0.0, 1.0) * self.kiai
    }

    /// A frame of the kiai: eases in and out of it, and on each beat in it the drum plays itself
    /// (rim, face, other rim, face) and the circle throws sparks.
    fn feel_kiai(&mut self, on: bool, beat: Option<(f64, f64)>, dt_ms: f32) {
        approach(&mut self.kiai, if on { 1.0 } else { 0.0 }, dt_ms, if on { 90.0 } else { 260.0 });
        if self.kiai < 0.002 && !on {
            self.kiai = 0.0;
        }
        if on && !self.in_kiai {
            self.kiai_at = self.clock;
            self.universe += 1;
            self.throw_sparks(SPARKS_OPENING);
        }
        self.in_kiai = on;
        let no = beat.map_or(i64::MIN, |(into, length)| (into / length).floor() as i64);
        if on && no != self.beat_no {
            self.beat_at = self.clock;
            // Every two bars the universes change places, like Multiverse's focus.
            if no.rem_euclid(8) == 0 {
                self.universe += 1;
            }
            self.throw_sparks(SPARKS_A_BEAT);
        }
        self.beat_no = no;
        let (clock, step) = (self.clock, dt_ms);
        self.sparks.retain(|s| clock - s.born < SPARK_MS);
        for s in &mut self.sparks {
            s.at[0] += s.speed[0] * step;
            s.at[1] += s.speed[1] * step;
            s.speed[1] += 0.0006 * step;
        }
    }

    /// A number from 0 to 1 for where a spark goes.
    fn roll(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn throw_sparks(&mut self, n: usize) {
        for _ in 0..n {
            if self.sparks.len() >= SPARKS {
                break;
            }
            let angle = -std::f32::consts::FRAC_PI_2 + (self.roll() - 0.5) * 2.6;
            let speed = 0.12 + self.roll() * 0.22;
            let col = [DON, KAT, GOLD, 0xffffff][(self.roll() * 4.0) as usize % 4];
            self.sparks.push(Spark { at: self.spot, speed: [angle.cos() * speed, angle.sin() * speed], born: self.clock, col });
        }
    }

    /// Light from the screen's edges, left and right in turn on the beat, like osu!'s.
    fn draw_flashes(&self, c: &mut Canvas) {
        let kick = self.kick();
        if kick <= 0.0 {
            return;
        }
        const BANDS: usize = 7;
        const BAND: f32 = 13.0;
        let right = self.beat_no.rem_euclid(2) == 1;
        for band in 0..BANDS {
            let x = band as f32 * BAND;
            let a = kick * 0.34 * (1.0 - band as f32 / BANDS as f32);
            c.rect(if right { c.width - x - BAND } else { x }, 0.0, BAND, render::VIRTUAL_HEIGHT, rgba(GOLD, a));
        }
    }

    /// Over the panel: the gold it takes on, the opening's flash and tears, the rings and sparks.
    fn draw_kiai(&self, c: &mut Canvas) {
        if self.kiai <= 0.0 && self.sparks.is_empty() {
            return;
        }
        let kick = self.kick();
        let px = pixel_size(LANE_PIXEL);
        let (left, w) = (PANEL_X, c.width - 2.0 * PANEL_X);
        c.round_rect([left, LANE_TOP], [w, LANE_H], 10.0, rgba(GOLD, self.kiai * 0.05 + kick * 0.11), rgba(GOLD, self.kiai * (0.55 + kick * 0.45)), 2.0);
        if kick > 0.0 {
            let k = 1.0 - kick / self.kiai.max(0.01);
            let col = if self.beat_no.rem_euclid(2) == 0 { KAT } else { DON };
            pixel_ring(c, self.spot, 34.0 + k * 30.0, px * 2.0, 0, px, rgba(col, kick * 0.9));
        }
        let opening = 1.0 - (self.clock - self.kiai_at) / OPENING_MS;
        if opening > 0.0 {
            c.round_rect([left, LANE_TOP], [w, LANE_H], 10.0, rgba(0xffffff, opening * opening * 0.55), rgba(0, 0.0), 0.0);
            // The panel torn, in the logo's colours, a new tear every 40 ms.
            let seed = (self.clock / 40.0) as u32;
            for tear in 0..6u32 {
                let mix = (tear + 1).wrapping_mul(2_654_435_761) ^ seed.wrapping_mul(40_503);
                let (y, x) = (LANE_TOP + 4.0 + ((mix >> 8) % (LANE_H as u32 - 8)) as f32, left + ((mix >> 16) % 240) as f32);
                c.rect(x, y, left + w - x, 3.0, rgba(if tear % 2 == 0 { GLITCH_RED } else { GLITCH_CYAN }, opening));
            }
            let up = (1.0 - opening) * 16.0;
            c.text_centered(self.spot[0], LANE_TOP - 26.0 - up, 16.0, rgba(GOLD, opening.min(0.5) * 2.0), "KIAI!");
        }
        for s in &self.sparks {
            let a = 1.0 - (self.clock - s.born) / SPARK_MS;
            let size = px * if a > 0.5 { 3.0 } else { 2.0 };
            c.rect(snap_to_grid(s.at[0]), snap_to_grid(s.at[1]), size, size, rgba(s.col, a));
        }
    }

    /// The song's lane on a screen `width` wide: where its notes are hit, and how big a big one
    /// is (as big as the drum, less on narrow screens).
    fn song_lane(width: f32) -> (f32, f32) {
        let big = SONG_BIG_R.min(width * 0.055);
        (PANEL_X + DRUM_W + 4.0 + big, big)
    }

    /// Where the notes' slots start and how wide each is, on a screen `width` wide.
    fn slots(width: f32) -> (f32, f32) {
        let left = PANEL_X + DRUM_W + 4.0;
        (left, (width - PANEL_X - 4.0 - left) / ITEMS.len() as f32)
    }

    /// The notes in their places on a dark panel, each with its name under it, and the selector:
    /// corner brackets that slide to the selected one. Choosing moves only the selector; in a
    /// kiai the notes hop on the beat.
    fn draw_panel(&self, c: &mut Canvas) {
        crate::style::panel(c, PANEL_X, LANE_TOP, c.width - 2.0 * PANEL_X, LANE_H);
    }

    /// The song playing, as autoplay would play it: its notes cross the panel behind the menu's
    /// to the drum, which hits each as it gets there (face for don, a rim for kat).
    /// `beat`: the song's beat length (ms), for its scroll speed.
    fn play_song(&mut self, c: &mut Canvas, song: Option<&crate::menu_notes::View>, beat: Option<f64>) {
        let Some(song) = song else {
            self.song_at = None;
            return;
        };
        let now = song.time();
        // Hit what got here since last frame (not after a jump: a seek or another song).
        if let Some(before) = self.song_at.filter(|&t| t <= now && now - t < 250.0) {
            for note in song.between(before, now) {
                // Hands in turn, as autoplay plays: each note on the other side.
                self.drum(note.kat, self.song_hand);
                self.song_hand = -self.song_hand;
                self.song_hit = (self.clock, note.kat);
            }
        }
        self.song_at = Some(now);
        let (hit_x, big_r) = Self::song_lane(c.width);
        let small_r = big_r * SONG_NOTE_R / SONG_BIG_R;
        let start = c.shapes.len();
        let fade = song.fade() * SONG_ALPHA;
        let colours = crate::multiverse::note_colours();
        let (don, kat) = (colours.don(), colours.kat());
        // The target, as in a song: the skin's, or rings.
        let big = c.sprite(Sprite::HitTargetBig, hit_x, LANE_MID, big_r, rgba(0xffffff, 0.22 * fade));
        let ring = c.sprite(Sprite::HitTarget, hit_x, LANE_MID, small_r, rgba(0xffffff, 0.47 * fade));
        if !big && !ring {
            c.circle(hit_x, LANE_MID, big_r, rgba(0xffffff, 0.03 * fade), rgba(0xffffff, 0.12 * fade), 2.0);
            c.circle(hit_x, LANE_MID, small_r + 1.0, rgba(0xffffff, 0.06 * fade), rgba(0xffffff, 0.4 * fade), 2.5);
        }
        // A hit lights it in the note's colour.
        let lit = 1.0 - (self.clock - self.song_hit.0) / SONG_HIT_MS;
        if lit > 0.0 {
            let col = if self.song_hit.1 { kat } else { don };
            c.circle(hit_x, LANE_MID, small_r + 6.0 * (1.0 - lit), rgba(col, 0.35 * lit * fade), rgba(0xffffff, 0.6 * lit * fade), 2.0);
        }
        let room = (c.width - PANEL_X - hit_x + big_r) as f64;
        // osu!px a ms at the song's BPM (120 without one), scaled with the notes.
        let beat_ms = beat.filter(|&b| b > 1.0).unwrap_or(500.0);
        let speed = (SLIDER_MULTIPLIER * TAIKO_VELOCITY / beat_ms * (small_r / crate::game::NOTE_R) as f64).clamp(0.05, 3.0);
        // Furthest first, so the nearer ones lie on top; each drawn as in a song (the skin's, or
        // the built-in ones), in the player's colours.
        for note in song.between(now, now + room / speed).iter().rev() {
            let x = hit_x + ((note.time - now) * speed) as f32;
            let r = if note.big { big_r } else { small_r };
            let col = if note.kat { kat } else { don };
            if !crate::game::skin_note(c, x, LANE_MID, r, note.big, col, fade) {
                crate::multiverse::note(c, crate::multiverse::Shape::Round, x, LANE_MID, r, col, fade);
            }
        }
        // Inside the panel, past the drum.
        c.clip_since(start, PANEL_X + DRUM_W + 2.0, LANE_TOP, c.width - 2.0 * PANEL_X - DRUM_W - 2.0, LANE_H);
    }

    fn draw_lane(&mut self, c: &mut Canvas) {
        let (left, slot) = Self::slots(c.width);
        let r = NOTE_MAX_R.min(slot * 0.36);
        // Names stay where they are whichever note is grown.
        let label_y = NOTE_Y + r * (1.0 + GROW) + 6.0;
        let names = ITEMS.map(Item::name);
        // One size for every name, the largest the narrowest slot takes.
        let size = names.iter().map(|n| Canvas::fit_size(7.0, 5.0, slot - 10.0, n)).fold(7.0f32, f32::min);
        let px = pixel_size(LANE_PIXEL);
        // Small marks between the notes.
        for k in 1..ITEMS.len() {
            let x = snap_to_grid(left + k as f32 * slot - px);
            c.rect(x, NOTE_Y - px, px * 2.0, px * 2.0, rgba(0x6a6a88, 0.8));
        }
        let mut hint = None;
        for (i, item) in ITEMS.iter().enumerate() {
            let x = left + (i as f32 + 0.5) * slot;
            let shaking = *item == Item::Editor && self.is_selected(i) && self.clock - self.shake < SHAKE_MS;
            let dx = if shaking { (self.clock / 18.0).sin() * 5.0 * (1.0 - (self.clock - self.shake) / SHAKE_MS) } else { 0.0 };
            // In a kiai they hop on the beat, every other one higher.
            // On every beat they bounce, every other one higher; in a kiai, higher still.
            let high = i as i64 % 2 == self.beat_no.rem_euclid(2);
            // Outside a kiai they float: a slow wave with the music (one rise and fall every two
            // beats, each note a little behind the one before), or with the clock when none
            // plays. In a kiai they hop on the beat.
            let wave = self.beats.unwrap_or(self.clock as f64 / 1000.0) / 2.0 - i as f64 * 0.12;
            let float = (std::f64::consts::TAU * wave).sin() as f32 * 3.5 * (1.0 - self.kiai);
            let hop = self.kick() * if high { 12.0 } else { 5.0 } + float;
            // The selected one grows as the selector gets to it, and shrinks back as it goes.
            let near = (1.0 - (self.shown - i as f32).abs()).clamp(0.0, 1.0);
            let grown = r * (1.0 + GROW * near);
            // It stands out: a glow of its colour behind it, the others dimmed.
            if near > 0.01 {
                c.glow(x + dx, NOTE_Y - hop, grown * 1.18, grown * 0.35, rgba(tint(item.colour(), 0xffffff, 0.2), 0.35 * near));
            }
            let start = c.shapes.len();
            draw_note(c, &self.arts[i], x + dx, NOTE_Y - hop, grown, 1.0);
            if near < 0.99 {
                c.fade_since(start, 1.0, 0.55 + 0.45 * near);
            }
            let hovered = c.hovered(left + i as f32 * slot, LANE_TOP, slot, LANE_H);
            let on = self.is_selected(i);
            let w = Canvas::text_width(size, names[i]);
            if on {
                let (pw, ph) = (w + 14.0, size + 8.0);
                c.round_rect([x - pw / 2.0, label_y - 4.0], [pw, ph], ph / 2.0, rgba(tint(item.colour(), 0x000000, 0.25), 0.95), rgba(0xffffff, 0.35), 1.0);
            }
            let col = match (on || hovered, *item == Item::Editor) {
                (true, _) => 0xffffff,
                (false, true) => 0x6f6f86,
                (false, false) => 0xc8c8d8,
            };
            c.text_centered(x, label_y, size, rgba(col, 1.0), names[i]);
            if hovered && !on {
                self.brackets(c, x, r * (1.0 + GROW), label_y + size, w, 0.35);
            }
            c.hit(left + i as f32 * slot, LANE_TOP, slot, LANE_H, Hit::MenuItem(i));
            if *item == Item::Editor && hovered {
                hint = Some(x);
            }
        }
        // The selector, on its way to the selected note: brighter on the beat, gold in a kiai.
        let x = left + (self.shown + 0.5) * slot;
        self.spot = [left + (self.index() as f32 + 0.5) * slot, NOTE_Y];
        let w = Canvas::text_width(size, names[self.index()]);
        self.brackets(c, x, r * (1.0 + GROW), label_y + size, w, 1.0);
        if let Some(x) = hint {
            tooltip(c, x, LANE_TOP - 26.0);
        }
    }

    /// Corner brackets around a note (`r`) and its name (`w` wide, ending at `bottom`).
    fn brackets(&self, c: &mut Canvas, x: f32, r: f32, bottom: f32, w: f32, alpha: f32) {
        let half = (r + 10.0).max(w / 2.0 + 10.0);
        let top = NOTE_Y - r - 8.0;
        crate::style::brackets(c, [x - half, top, 2.0 * half, bottom + 8.0 - top], alpha);
    }

    /// The head of the panel: the rim lights up on kat, the face on don.
    fn draw_drum(&self, c: &mut Canvas) {
        let x = PANEL_X + DRUM_W / 2.0;
        c.rect(PANEL_X + DRUM_W, LANE_TOP + 10.0, 2.0, LANE_H - 20.0, rgba(0x3a3a4e, 1.0));
        let px = pixel_size(LANE_PIXEL);
        pixel_ring(c, [x, LANE_MID], 44.0, 44.0, 0, px, rgba(0x23232f, 1.0));
        pixel_ring(c, [x, LANE_MID], 44.0, px * 2.0, 0, px, rgba(0x6a6a88, 1.0));
        let rim = 1.0 - (self.clock - self.rim.0) / FLASH_MS;
        if rim > 0.0 {
            pixel_ring(c, [x, LANE_MID], 42.0, 14.0, self.rim.1.signum(), px, rgba(KAT, rim));
        }
        pixel_ring(c, [x, LANE_MID], 28.0, 28.0, 0, px, rgba(0x2d2d3c, 1.0));
        let face = (1.0 - (self.clock - self.face.0) / FLASH_MS).max(0.0);
        if face > 0.0 {
            pixel_ring(c, [x, LANE_MID], 28.0, 28.0, self.face.1.signum(), px, rgba(DON, 0.35 + 0.65 * face));
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
        pixel_ring(c, self.spot, 34.0 + k * 50.0, px * 2.0, 0, px, rgba(if miss { 0x8a8aa0 } else { item.colour() }, a * 0.9));
        let word = if miss { tr!("SOON", "もうすぐ", "СКОРО") } else { "GREAT" };
        c.text_centered(self.spot[0], LANE_TOP - 22.0 - k * 18.0, 12.0, rgba(0xFFD34D, a), word);
    }

    fn draw_title(&self, c: &mut Canvas, maps: usize) {
        let item = self.current();
        let mid = c.width / 2.0;
        // A kiai's beat shakes the title apart into red and cyan for a moment.
        let split = self.kick() * 3.0;
        if split > 0.2 {
            c.text_centered(mid - split, TITLE_Y, 34.0, rgba(GLITCH_RED, 0.9), item.name());
            c.text_centered(mid + split, TITLE_Y, 34.0, rgba(GLITCH_CYAN, 0.9), item.name());
        }
        c.text_centered(mid, TITLE_Y, 34.0, rgba(0xffffff, 1.0), item.name());
        c.text_centered(mid, TITLE_Y + 44.0, 10.0, rgba(0xc8c8d8, 1.0), &item.about(maps));
        if item == Item::Editor {
            tooltip(c, mid + Canvas::text_width(34.0, item.name()) / 2.0 + 62.0, TITLE_Y + 7.0);
        }
    }
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
        m.pick(6);
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
        m.pick(5);
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
    fn a_kiai_opens_with_sparks_and_each_beat_throws_more_but_leaves_the_drum_alone() {
        let mut m = Menu::new();
        m.clock = 1000.0;
        m.feel_kiai(false, Some((0.0, 500.0)), 16.0);
        assert_eq!((m.kiai, m.sparks.len()), (0.0, 0), "nothing before it");
        m.feel_kiai(true, Some((1010.0, 500.0)), 16.0);
        assert_eq!(m.kiai_at, 1000.0);
        assert_eq!(m.sparks.len(), SPARKS_OPENING + SPARKS_A_BEAT);
        m.clock = 1100.0;
        m.feel_kiai(true, Some((1100.0, 500.0)), 16.0);
        assert_eq!(m.beat_at, 1000.0, "the same beat");
        m.clock = 1500.0;
        m.feel_kiai(true, Some((1500.0, 500.0)), 16.0);
        assert_eq!(m.beat_at, 1500.0);
        // The drum plays only the song's notes and the player's hits.
        assert_eq!((m.rim.0, m.face.0), (f32::MIN, f32::MIN));
        m.step(1);
        assert_eq!(m.rim.0, f32::MIN, "choosing isn't a hit");
        m.drum(true, -1);
        assert_eq!(m.rim, (1500.0, -1));
        m.drum(false, 1);
        assert_eq!(m.face, (1500.0, 1), "a don: the right half of the face");
    }

    #[test]
    fn sparks_stop_at_their_limit_and_die_out_after_a_kiai() {
        let mut m = Menu::new();
        for beat in 0..40 {
            m.clock += 10.0;
            m.feel_kiai(true, Some((beat as f64 * 500.0, 500.0)), 10.0);
            assert!(m.sparks.len() <= SPARKS);
        }
        m.clock += SPARK_MS;
        m.feel_kiai(false, Some((99_000.0, 500.0)), 16.0);
        assert!(m.sparks.is_empty());
        let beat_at = m.beat_at;
        m.clock += 500.0;
        m.feel_kiai(false, Some((99_500.0, 500.0)), 16.0);
        assert_eq!(m.beat_at, beat_at, "the drum rests outside a kiai");
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

    /// A frame of the menu on a 16:9 screen, `ms` after the last.
    fn frame(m: &mut Menu, ms: f32) -> Canvas {
        let mut c = Canvas { width: 853.3, dt_ms: ms, ..Default::default() };
        let f = Frame { beat: None, kiai: false, ghosts: None, song: None, maps: 0, paused: false };
        m.draw(&mut c, &f);
        c
    }

    /// Where each note's hit area is.
    fn note_spots(c: &Canvas) -> Vec<[f32; 4]> {
        (0..ITEMS.len()).map(|i| c.hits.iter().find(|h| h.hit == Hit::MenuItem(i)).unwrap().rect).collect()
    }

    #[test]
    fn the_notes_stay_put_and_the_selector_moves_to_them() {
        let mut m = Menu::new();
        let first = note_spots(&frame(&mut m, 16.0));
        // Side by side and inside the screen, in order.
        assert!(first.windows(2).all(|w| w[0][0] + w[0][2] <= w[1][0] + 0.01));
        assert!(first[0][0] > DRUM_W && first[6][0] + first[6][2] <= 853.3 - PANEL_X);
        m.step(3);
        let c = frame(&mut m, 16.0);
        assert_eq!(note_spots(&c), first, "choosing doesn't move the notes");
        // On its way (or there already, with animations off: other tests switch them).
        assert!(m.shown > 0.0 && m.shown <= 3.0, "the selector goes toward it: {}", m.shown);
        for _ in 0..60 {
            frame(&mut m, 16.0);
        }
        assert_eq!(m.shown, 3.0);
        let [x, _, w, _] = first[3];
        assert!((m.spot[0] - (x + w / 2.0)).abs() < 0.01, "sparks and bursts come from it");
        // Round past the end: it goes back along, the notes still where they were.
        m.step(4);
        assert_eq!(m.index(), 0);
        assert_eq!(note_spots(&frame(&mut m, 16.0)), first);
    }

    #[test]
    fn the_drum_plays_the_songs_notes_as_they_reach_it() {
        use crate::menu_notes::{Ghost, View};
        let notes = [Ghost { time: 1000.0, kat: false, big: false }, Ghost { time: 1100.0, kat: true, big: false }, Ghost { time: 1200.0, kat: true, big: false }];
        let mut m = Menu::new();
        let at = |m: &mut Menu, time: f64| {
            m.clock += 50.0;
            let mut c = Canvas { width: 853.3, ..Default::default() };
            m.play_song(&mut c, Some(&View::of(&notes, time)), None);
            c
        };
        let c = at(&mut m, 900.0);
        assert!(c.shapes.len() > 3, "the notes coming are drawn");
        assert_eq!((m.face.0, m.rim.0), (f32::MIN, f32::MIN), "nothing has got there yet");
        at(&mut m, 1050.0);
        assert_eq!(m.face.0, m.clock, "the don: the face");
        assert_eq!(m.rim.0, f32::MIN);
        assert!(m.face.1.abs() == 1, "one half of the face, not all of it");
        at(&mut m, 1150.0);
        let first = m.rim;
        assert_eq!(first.0, m.clock, "the kat: a rim");
        assert_eq!(first.1, -m.face.1, "hands in turn: the other side from the don");
        at(&mut m, 1250.0);
        assert_eq!(m.rim.0, m.clock);
        assert_eq!(m.rim.1, -first.1, "the next kat, the other rim");
        // A jump back (a seek, or the next song) hits nothing on the way.
        let rim = m.rim;
        at(&mut m, 0.0);
        at(&mut m, 1150.0);
        assert_eq!(m.rim, rim);
        // Without the notes (turned off), it rests.
        let mut c = Canvas { width: 853.3, ..Default::default() };
        m.play_song(&mut c, None, None);
        assert!(c.shapes.is_empty() && m.song_at.is_none());
    }

    #[test]
    fn the_menu_fades_in_and_out_but_not_for_exit() {
        let mut m = Menu::new();
        assert_eq!(m.opacity(), 0.0, "it fades in");
        m.clock = FADE_MS / 2.0;
        assert!(m.opacity() > 0.0 && m.opacity() < 1.0);
        m.clock = FADE_MS;
        assert_eq!(m.opacity(), 1.0);
        // Opening something fades it out until it goes.
        m.pick(2);
        m.hit();
        assert_eq!(m.opacity(), 1.0);
        m.clock += LEAVE_MS / 2.0;
        assert!(m.opacity() < 1.0 && m.opacity() > 0.0);
        m.clock += LEAVE_MS / 2.0;
        assert_eq!(m.opacity(), 0.0);
        assert_eq!(m.ready(), Some(Item::Download));
        // Back from there, it fades in again.
        assert_eq!(m.opacity(), 0.0);
        m.clock += FADE_MS;
        assert_eq!(m.opacity(), 1.0);
        // Exit only asks: the menu stays.
        m.pick(6);
        m.hit();
        m.clock += LEAVE_MS * 0.9;
        assert_eq!(m.opacity(), 1.0);
    }

    #[test]
    fn the_discord_face_is_16_by_12() {
        assert!(DISCORD_FACE.iter().all(|row| row.len() == 16));
        assert!(DISCORD_INVITE.starts_with("https://discord.gg/"));
    }
}
