//! Multiverse's universes: each of its playfields with note colours, a note shape, a scroll speed
//! and visual mods of its own, set up by the player (M in the mod select) and kept in the config,
//! along with presets of whole setups. They only change how a playfield is drawn: every universe
//! plays the same map, judged the same. Also the note colours of a play without Multiverse.

use std::sync::RwLock;

use winit::keyboard::KeyCode;

use crate::mods::{Mod, Mods};
use crate::render::{accent, rgba, Canvas, Hit};

/// How many universes (playfields) there are.
pub const COUNT: usize = 8;

/// Note colours: (name, don, kat). Dons are always the darker of the two, as in the classic
/// colours, so they tell apart by brightness too.
pub const PALETTES: [(&str, u32, u32); 10] = [
    ("classic", crate::game::DON, crate::game::KAT),
    ("neon", 0xff3fb4, 0x2fe0ff),
    ("sunset", 0xd9541a, 0xa98bff),
    ("ice", 0x2f5fd0, 0x9fe8ff),
    ("forest", 0xb8901c, 0x6fe0a0),
    ("candy", 0xff6f91, 0x5fd4a8),
    ("ember", 0xff4a2a, 0xffb02f),
    ("mono", 0x5c5c78, 0xf2f2f2),
    ("toxic", 0xc04dff, 0xb8ff3a),
    ("swapped", 0x2f6f96, 0xff7a5c),
];

/// `Colours::palette` for the player's own colours (after the palettes).
pub const CUSTOM: usize = PALETTES.len();

/// Grey with no colour in it: notes out of focus (and Colorless).
const GREY: u32 = 0x8c8ca6;

/// Note colours: one of `PALETTES`, or the player's own (`custom`, kept while a palette is
/// picked, so going back to it finds them again).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Colours {
    pub palette: usize,
    /// Don, kat.
    pub custom: [u32; 2],
}

impl Colours {
    pub const CLASSIC: Colours = Colours { palette: 0, custom: [crate::game::DON, crate::game::KAT] };

    pub fn don(self) -> u32 {
        PALETTES.get(self.palette).map_or(self.custom[0], |p| p.1)
    }

    pub fn kat(self) -> u32 {
        PALETTES.get(self.palette).map_or(self.custom[1], |p| p.2)
    }

    pub fn name(self) -> &'static str {
        PALETTES.get(self.palette).map_or("custom", |p| p.0)
    }

    /// Dons and kats alike, out of focus: grey with a hint of the colours (but not which is
    /// which).
    pub fn idle(self) -> u32 {
        if self.palette == 0 { GREY } else { crate::game::mix(GREY, crate::game::mix(self.don(), self.kat(), 0.5), 0.35) }
    }

    /// The next (`dir` 1) or previous palette; the player's own come after the last.
    pub fn step(self, dir: i32) -> Colours {
        Colours { palette: (self.palette as i32 + dir).rem_euclid(CUSTOM as i32 + 1) as usize, ..self }
    }

    /// The player's own colours, picked.
    pub fn own(don: u32, kat: u32) -> Colours {
        Colours { palette: CUSTOM, custom: [don, kat] }
    }

    /// "neon", "custom #ff4020/#40a0ff", or a palette and the own colours kept behind it.
    pub fn format(self) -> String {
        if self.palette != CUSTOM && self.custom == Colours::CLASSIC.custom {
            return self.name().to_string();
        }
        format!("{} #{:06x}/#{:06x}", self.name(), self.custom[0], self.custom[1])
    }

    /// Takes one word of `format`'s form; false if it isn't about colours.
    fn read(&mut self, word: &str) -> bool {
        if word.eq_ignore_ascii_case("custom") {
            self.palette = CUSTOM;
        } else if let Some(p) = PALETTES.iter().position(|p| p.0.eq_ignore_ascii_case(word)) {
            self.palette = p;
        } else if let Some(pair) = parse_pair(word) {
            self.custom = pair;
        } else {
            return false;
        }
        true
    }

    /// `format`'s form (the config's `note_colours`).
    pub fn parse(s: &str) -> Colours {
        let mut out = Colours::CLASSIC;
        for word in s.split_whitespace() {
            out.read(word);
        }
        out
    }
}

/// A colour typed as "#ff4020" or "ff4020".
pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten()
}

/// Two colours, don then kat: "#ff4020 #40a0ff", "ff4020/40a0ff", "ff4020, 40a0ff"...
pub fn parse_pair(s: &str) -> Option<[u32; 2]> {
    let parts: Vec<&str> = s.split(|c: char| c.is_whitespace() || c == '/' || c == ',').filter(|p| !p.is_empty()).collect();
    match parts[..] {
        [don, kat] => Some([parse_hex(don)?, parse_hex(kat)?]),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Round,
    Square,
    Diamond,
    Ring,
    Pill,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Round, Shape::Square, Shape::Diamond, Shape::Ring, Shape::Pill];

    pub fn name(self) -> &'static str {
        match self {
            Shape::Round => "round",
            Shape::Square => "square",
            Shape::Diamond => "diamond",
            Shape::Ring => "ring",
            Shape::Pill => "pill",
        }
    }
}

/// Scroll speeds, times the player's own.
pub const SPEEDS: [f64; 5] = [0.75, 0.9, 1.0, 1.25, 1.5];
const NORMAL_SPEED: usize = 2;

/// The mods a universe can have: those that only change how notes are drawn and move, worked
/// out as each playfield is drawn (nothing about them is set up when the play starts).
pub const MODS: [Mod; 10] = [Mod::Hidden, Mod::Sudden, Mod::Blink, Mod::Snake, Mod::Party, Mod::Wave, Mod::Accel, Mod::Brake, Mod::Pulse, Mod::Stutter];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Universe {
    pub colours: Colours,
    pub shape: Shape,
    pub speed: usize,
    pub mods: Mods,
}

impl Universe {
    /// The usual playfield (and every playfield without Multiverse, in the player's colours).
    pub const CLASSIC: Universe = Universe { colours: Colours::CLASSIC, shape: Shape::Round, speed: NORMAL_SPEED, mods: Mods::NONE };

    pub fn don(&self) -> u32 {
        self.colours.don()
    }

    pub fn kat(&self) -> u32 {
        self.colours.kat()
    }

    pub fn idle(&self) -> u32 {
        self.colours.idle()
    }

    pub fn speed(&self) -> f64 {
        SPEEDS[self.speed]
    }

    /// "neon diamond 1.25x HD,SK" (the config's form; own colours as "custom #rrggbb/#rrggbb").
    fn format(&self) -> String {
        let mods = if self.mods.is_empty() { "-".to_string() } else { self.mods.list().join(",") };
        format!("{} {} {}x {mods}", self.colours.format(), self.shape.name(), SPEEDS[self.speed])
    }

    /// Reads `format`'s form; what isn't understood stays as in `base`.
    fn parse(s: &str, base: Universe) -> Universe {
        let mut u = base;
        for word in s.split_whitespace() {
            if u.colours.read(word) {
                continue;
            }
            if let Some(&shape) = Shape::ALL.iter().find(|s| s.name().eq_ignore_ascii_case(word)) {
                u.shape = shape;
            } else if let Some(speed) = word.strip_suffix('x').and_then(|v| v.parse::<f64>().ok()) {
                u.speed = SPEEDS.iter().position(|&s| (s - speed).abs() < 1e-6).unwrap_or(u.speed);
            } else if word == "-" {
                u.mods = Mods::NONE;
            } else {
                u.mods = only_universe_mods(Mods::parse(word));
            }
        }
        u
    }
}

/// Just the mods a universe can have (`MODS`).
fn only_universe_mods(mods: Mods) -> Mods {
    let mut out = Mods::NONE;
    for m in mods.iter().filter(|m| MODS.contains(m)) {
        out.toggle(m);
    }
    out
}

/// The universes a Multiverse play starts with: each its own look, a few with a twist. The one
/// with the focus first is the usual playfield.
pub fn defaults() -> [Universe; COUNT] {
    let u = |palette, shape, speed, mods: &str| Universe {
        colours: Colours { palette, ..Colours::CLASSIC },
        shape,
        speed,
        mods: only_universe_mods(Mods::parse(mods)),
    };
    [
        u(1, Shape::Diamond, 2, ""),
        u(2, Shape::Square, 3, ""),
        u(3, Shape::Ring, 1, "WV"),
        u(4, Shape::Pill, 2, "SK"),
        Universe::CLASSIC,
        u(5, Shape::Round, 2, "PY"),
        u(6, Shape::Diamond, 3, "AC"),
        u(7, Shape::Square, 2, "BL"),
    ]
}

/// Every universe the usual playfield (Multiverse as it first was).
pub fn classic() -> [Universe; COUNT] {
    [Universe::CLASSIC; COUNT]
}

/// Random universes, each a different look, about half with a mod. The first focus stays the
/// usual playfield, to get into the map.
pub fn shuffle(seed: u64, first_focus: usize) -> [Universe; COUNT] {
    let mut rng = seed | 1;
    let mut next = |n: usize| {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        (rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n
    };
    let mut palettes: Vec<usize> = (1..PALETTES.len()).collect();
    std::array::from_fn(|i| {
        if i == first_focus {
            return Universe::CLASSIC;
        }
        let palette = palettes.remove(next(palettes.len()));
        let shape = Shape::ALL[next(Shape::ALL.len())];
        let speed = next(SPEEDS.len());
        let mods = if next(2) == 0 { Mods::NONE } else { Mods::parse(MODS[next(MODS.len())].acronym()) };
        Universe { colours: Colours { palette, ..Colours::CLASSIC }, shape, speed, mods }
    })
}

/// The universes as the config keeps them: `; ` between them.
pub fn format(all: &[Universe; COUNT]) -> String {
    all.iter().map(Universe::format).collect::<Vec<_>>().join("; ")
}

pub fn parse(s: &str) -> [Universe; COUNT] {
    let parts: Vec<&str> = s.split(';').collect();
    let base = defaults();
    std::array::from_fn(|i| parts.get(i).map_or(base[i], |p| Universe::parse(p, base[i])))
}

/// A setup of all the universes saved under a name by the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    pub universes: [Universe; COUNT],
}

impl Preset {
    /// "name: universes" (a config `multiverse_preset` line).
    pub fn format(&self) -> String {
        format!("{}: {}", self.name, format(&self.universes))
    }

    pub fn parse(s: &str) -> Option<Preset> {
        let (name, rest) = s.split_once(':')?;
        let name = clean_name(name);
        (!name.is_empty()).then(|| Preset { name, universes: parse(rest) })
    }
}

/// A preset name as typed, without what the config uses to separate things.
fn clean_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() && !matches!(c, ':' | ';')).collect::<String>().trim().chars().take(24).collect()
}

/// Every preset in the order the setup lists them: the built-in ones, then the player's.
fn presets(own: &[Preset]) -> Vec<(String, [Universe; COUNT])> {
    let builtin = [("defaults".to_string(), defaults()), ("classic".to_string(), classic())];
    builtin.into_iter().chain(own.iter().map(|p| (p.name.clone(), p.universes))).collect()
}
const BUILTIN_PRESETS: usize = 2;

static CURRENT: RwLock<[Universe; COUNT]> = RwLock::new([Universe::CLASSIC; COUNT]);
static NOTE_COLOURS: RwLock<Colours> = RwLock::new(Colours::CLASSIC);

/// The universes Multiverse plays are drawn with (from the config).
pub fn set(all: [Universe; COUNT]) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = all;
}

pub fn get() -> [Universe; COUNT] {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// The note colours of plays without Multiverse (Settings > Skin).
pub fn set_note_colours(colours: Colours) {
    *NOTE_COLOURS.write().unwrap_or_else(|e| e.into_inner()) = colours;
}

pub fn note_colours() -> Colours {
    *NOTE_COLOURS.read().unwrap_or_else(|e| e.into_inner())
}

/// A don or kat `r` across in `shape`. Round is the usual note (skins draw their own).
pub fn note(c: &mut Canvas, shape: Shape, x: f32, y: f32, r: f32, col: u32, a: f32) {
    let (fill, white) = (rgba(col, a), rgba(0xffffff, a));
    match shape {
        Shape::Round => {
            c.circle(x, y, r + 2.0, rgba(0x000000, 0.35 * a), rgba(0x000000, 0.0), 0.0);
            c.circle(x, y, r, fill, white, r * 0.1);
        }
        Shape::Square => {
            let h = r * 0.9;
            c.round_rect(x - h, y - h, 2.0 * h, 2.0 * h, r * 0.28, fill, white, r * 0.1);
        }
        Shape::Diamond => {
            let first = c.shapes.len();
            let h = r * 0.82;
            c.round_rect(x - h, y - h, 2.0 * h, 2.0 * h, r * 0.18, fill, white, r * 0.1);
            c.rotate_since(first, [x, y], std::f32::consts::FRAC_PI_4, 1.0, [x, y]);
        }
        Shape::Ring => {
            c.circle(x, y, r, rgba(col, 0.22 * a), fill, r * 0.32);
            c.ring(x, y, r * 0.62, rgba(0xffffff, 0.8 * a), r * 0.07);
        }
        Shape::Pill => {
            c.capsule(x - r * 0.45, x + r * 0.45, y, r * 0.78, fill, white, r * 0.1);
        }
    }
}

/// The columns of the setup: colours, shape, speed, then one per mod.
const COLUMNS: usize = 3 + MODS.len();

/// Text being typed in the setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typing {
    /// The selected universe's own colours, don then kat.
    Colours(String),
    /// A name to save the universes under.
    Name(String),
}

/// The Multiverse setup (M in the mod select): a row per universe, a column per setting, and
/// the presets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub row: usize,
    pub col: usize,
    pub typing: Option<Typing>,
    /// What was typed didn't work (the prompt shows red until the next key).
    pub bad: bool,
}

/// What a key or click in the setup asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The universes or the presets changed (save them).
    Changed,
    Close,
}

impl Setup {
    pub fn new(row: usize, col: usize) -> Setup {
        Setup { row, col, ..Default::default() }
    }

    /// Changes the setting at `row`, `col`: the next (`dir` 1) or previous (-1) colours, shape or
    /// speed; a mod switches on or off.
    pub fn change(all: &mut [Universe; COUNT], row: usize, col: usize, dir: i32) {
        let u = &mut all[row];
        let step = |i: usize, n: usize| (i as i32 + dir).rem_euclid(n as i32) as usize;
        match col {
            0 => u.colours = u.colours.step(dir),
            1 => u.shape = Shape::ALL[step(Shape::ALL.iter().position(|&s| s == u.shape).unwrap_or(0), Shape::ALL.len())],
            2 => u.speed = step(u.speed, SPEEDS.len()),
            c => u.mods.toggle(MODS[(c - 3).min(MODS.len() - 1)]),
        }
    }

    /// The preset (index in the setup's list) the universes are now, if any.
    pub fn current(all: &[Universe; COUNT], own: &[Preset]) -> Option<usize> {
        presets(own).iter().position(|p| p.1 == *all)
    }

    /// Loads preset `i` of the setup's list.
    pub fn load(all: &mut [Universe; COUNT], own: &[Preset], i: usize) -> Action {
        match presets(own).get(i) {
            Some(p) => {
                *all = p.1;
                Action::Changed
            }
            None => Action::None,
        }
    }

    /// Deletes preset `i` of the setup's list (only the player's own).
    pub fn delete(own: &mut Vec<Preset>, i: usize) -> Action {
        match i.checked_sub(BUILTIN_PRESETS).filter(|&k| k < own.len()) {
            Some(k) => {
                own.remove(k);
                Action::Changed
            }
            None => Action::None,
        }
    }

    /// Enter while typing: the universe gets its own colours, or the universes are saved under
    /// the name (replacing a preset of that name). Something unusable keeps the typing open.
    fn finish_typing(&mut self, all: &mut [Universe; COUNT], own: &mut Vec<Preset>) -> Action {
        match &self.typing {
            Some(Typing::Colours(s)) => match parse_pair(s) {
                Some([don, kat]) => all[self.row].colours = Colours::own(don, kat),
                None => {
                    self.bad = true;
                    return Action::None;
                }
            },
            Some(Typing::Name(s)) => {
                let name = clean_name(s);
                if name.is_empty() {
                    self.bad = true;
                    return Action::None;
                }
                let preset = Preset { name, universes: *all };
                match own.iter_mut().find(|p| p.name.eq_ignore_ascii_case(&preset.name)) {
                    Some(p) => *p = preset,
                    None => own.push(preset),
                }
            }
            None => return Action::None,
        }
        self.typing = None;
        Action::Changed
    }

    pub fn key(&mut self, code: KeyCode, text: Option<&str>, shift: bool, all: &mut [Universe; COUNT], own: &mut Vec<Preset>, first_focus: usize) -> Action {
        self.bad = false;
        if let Some(Typing::Colours(s) | Typing::Name(s)) = &mut self.typing {
            match code {
                KeyCode::Escape => self.typing = None,
                KeyCode::Enter | KeyCode::NumpadEnter => return self.finish_typing(all, own),
                KeyCode::Backspace => {
                    s.pop();
                }
                _ => {
                    for ch in text.unwrap_or("").chars().filter(|c| !c.is_control()) {
                        if s.chars().count() < 40 {
                            s.push(ch);
                        }
                    }
                }
            }
            return Action::None;
        }
        let dir = if shift { -1 } else { 1 };
        match code {
            KeyCode::ArrowUp => self.row = (self.row + COUNT - 1) % COUNT,
            KeyCode::ArrowDown => self.row = (self.row + 1) % COUNT,
            KeyCode::ArrowLeft => self.col = (self.col + COLUMNS - 1) % COLUMNS,
            KeyCode::ArrowRight => self.col = (self.col + 1) % COLUMNS,
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter => {
                Self::change(all, self.row, self.col, dir);
                return Action::Changed;
            }
            KeyCode::KeyE => {
                let c = all[self.row].colours;
                self.col = 0;
                self.typing = Some(Typing::Colours(format!("#{:06x} #{:06x}", c.don(), c.kat())));
            }
            KeyCode::KeyS => {
                let name = Self::current(all, own).filter(|&i| i >= BUILTIN_PRESETS).map(|i| own[i - BUILTIN_PRESETS].name.clone());
                self.typing = Some(Typing::Name(name.unwrap_or_default()));
            }
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                let n = presets(own).len() as i32;
                let step = if code == KeyCode::BracketRight { 1 } else { -1 };
                let at = Self::current(all, own).map_or(if step > 0 { -1 } else { 0 }, |i| i as i32);
                return Self::load(all, own, (at + step).rem_euclid(n) as usize);
            }
            KeyCode::Delete => {
                if let Some(i) = Self::current(all, own) {
                    return Self::delete(own, i);
                }
            }
            KeyCode::KeyR => {
                let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
                *all = shuffle(seed, first_focus);
                return Action::Changed;
            }
            KeyCode::KeyC => {
                *all = classic();
                return Action::Changed;
            }
            KeyCode::KeyD => {
                *all = defaults();
                return Action::Changed;
            }
            KeyCode::Escape | KeyCode::KeyM | KeyCode::F1 | KeyCode::Backspace | KeyCode::Tab => return Action::Close,
            _ => {}
        }
        Action::None
    }

    /// Draws the setup over the mod select. `first_focus`: the universe the focus starts on.
    pub fn draw(&self, c: &mut Canvas, all: &[Universe; COUNT], own: &[Preset], first_focus: usize) {
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.6));
        let (px, py, pw, ph) = (16.0, 44.0, w - 32.0, 410.0);
        c.round_rect(px, py, pw, ph, 10.0, rgba(0x15151d, 0.99), rgba(0xff5fa2, 0.9), 1.5);
        let title_w = c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), "MULTIVERSE");
        let hint = "arrows choose   Space change (Shift back)   E own colours   S save preset   [ ] presets   R shuffle   Esc back";
        c.text(px + 26.0 + title_w, py + 16.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit(6.0, pw - title_w - 42.0, hint));

        // Columns: number and its notes, colours, shape, speed, the mod chips.
        let (num_w, look_w, shape_w, speed_w) = (70.0, 72.0, 62.0, 46.0);
        let x0 = px + 12.0;
        let chips_x = x0 + num_w + look_w + shape_w + speed_w;
        let chip_w = ((px + pw - 12.0 - chips_x) / MODS.len() as f32).min(34.0);
        let col_x = |col: usize| match col {
            0 => (x0 + num_w, look_w),
            1 => (x0 + num_w + look_w, shape_w),
            2 => (x0 + num_w + look_w + shape_w, speed_w),
            c => (chips_x + (c - 3) as f32 * chip_w, chip_w),
        };
        let head_y = py + 36.0;
        for (col, name) in [(0, "COLOURS"), (1, "SHAPE"), (2, "SPEED"), (3, "MODS")] {
            c.text(col_x(col).0 + 4.0, head_y, 6.0, rgba(0x8a8aa0, 1.0), name);
        }
        const ROW: f32 = 36.0;
        let mut hovered = None;
        for (i, u) in all.iter().enumerate() {
            let y = head_y + 12.0 + i as f32 * ROW;
            let (h, mid) = (ROW - 5.0, y + (ROW - 5.0) / 2.0);
            c.round_rect(x0, y, px + pw - 12.0 - x0, h, 6.0, rgba(if i == self.row { 0x23232f } else { 0x1b1b24 }, 1.0), rgba(0x2a2a38, 1.0), 1.0);
            let label = if i == first_focus { format!("{}*", i + 1) } else { (i + 1).to_string() };
            c.text(x0 + 8.0, mid - 4.0, 8.0, rgba(0xc8c8d8, 1.0), &label);
            note(c, u.shape, x0 + 36.0, mid, 9.5, u.don(), 1.0);
            note(c, u.shape, x0 + 58.0, mid, 9.5, u.kat(), 1.0);
            for col in 0..COLUMNS {
                let (cx, cw) = col_x(col);
                let (cx, cw) = (cx + 2.0, cw - 4.0);
                let selected = i == self.row && col == self.col;
                if c.hovered(cx, y + 3.0, cw, h - 6.0) {
                    hovered = Some(col);
                }
                let on = col >= 3 && u.mods.has(MODS[col - 3]);
                let fill = if on { 0x6b2a4c } else if selected { 0x34344a } else { 0x1d1d27 };
                let border = if selected { accent() } else if on { 0xff5fa2 } else { 0x2a2a38 };
                c.round_rect(cx, y + 3.0, cw, h - 6.0, 4.0, rgba(fill, 1.0), rgba(border, 1.0), if selected { 1.5 } else { 1.0 });
                let text = match col {
                    0 => u.colours.name().to_string(),
                    1 => u.shape.name().to_string(),
                    2 => format!("{}x", SPEEDS[u.speed]),
                    c => MODS[c - 3].acronym().to_string(),
                };
                let col_text = if col >= 3 && !on { 0x6f6f86 } else { 0xffffff };
                c.text_centered(cx + cw / 2.0, mid - 3.5, 7.0, rgba(col_text, 1.0), &Canvas::fit(7.0, cw - 4.0, &text));
                c.hit(cx, y + 3.0, cw, h - 6.0, Hit::UniverseCell(i, col));
            }
        }

        // What's being typed, else what the pointed-at (or the selected) setting is.
        let line_y = py + ph - 48.0;
        let (line, col) = match &self.typing {
            Some(Typing::Colours(s)) => (format!("Universe {}'s own colours, don then kat: {s}_   Enter set   Esc cancel", self.row + 1), 0xFFD34D),
            Some(Typing::Name(s)) => (format!("Save these universes as: {s}_   Enter save (the same name replaces it)   Esc cancel"), 0xFFD34D),
            None => {
                let about = match hovered.unwrap_or(self.col) {
                    0 => "Colours: dons' and kats' (out of focus they stay alike, faintly tinted). E: your own, like #d03020 #40a0ff.".to_string(),
                    1 => "Shape: how its notes look. Round ones use your skin.".to_string(),
                    2 => "Speed: how fast its notes scroll, times your scroll speed. The timing is the map's either way.".to_string(),
                    c => format!("{}: {}", MODS[c - 3].name(), MODS[c - 3].description()),
                };
                (about, 0x9a9ab0)
            }
        };
        let col = if self.bad { 0xFF4B4B } else { col };
        c.text(px + 16.0, line_y, 7.0, rgba(col, 1.0), &Canvas::fit_end(7.0, pw - 32.0, &line));

        // Presets: the built-in ones and the player's, the one the universes are now lit; the
        // player's have an x to delete them.
        let (by, bh, bw) = (py + ph - 26.0, 16.0, 56.0);
        c.button(px + pw - bw - 12.0, by, bw, bh, "Back", 7.0, Hit::Key(KeyCode::Escape), true);
        c.button(px + pw - 2.0 * bw - 18.0, by, bw, bh, "Shuffle", 7.0, Hit::Key(KeyCode::KeyR), false);
        c.button(px + pw - 3.0 * bw - 24.0, by, bw, bh, "+ Save", 7.0, Hit::Key(KeyCode::KeyS), false);
        let room_end = px + pw - 3.0 * bw - 32.0;
        let mut x = px + 16.0 + c.text(px + 16.0, by + 5.0, 6.0, rgba(0x8a8aa0, 1.0), "PRESETS") + 8.0;
        let current = Self::current(all, own);
        let list = presets(own);
        for (i, (name, _)) in list.iter().enumerate() {
            let mine = i >= BUILTIN_PRESETS;
            let name_w = Canvas::text_width(7.0, name).min(90.0);
            let cw = name_w + 14.0 + if mine { 12.0 } else { 0.0 };
            if x + cw > room_end {
                c.text(x, by + 4.5, 7.0, rgba(0x8a8aa0, 1.0), &format!("+{}", list.len() - i));
                break;
            }
            let on = current == Some(i);
            c.round_rect(x, by, cw, bh, 4.0, rgba(if on { 0x6b2a4c } else { 0x1d1d27 }, 1.0), rgba(if on { 0xff5fa2 } else { 0x3a3a4e }, 1.0), 1.0);
            c.text(x + 7.0, by + 4.5, 7.0, rgba(0xffffff, 1.0), &Canvas::fit(7.0, name_w, name));
            c.hit(x, by, cw - if mine { 12.0 } else { 0.0 }, bh, Hit::UniversePreset(i));
            if mine {
                c.text(x + cw - 11.0, by + 4.5, 7.0, rgba(0x8a8aa0, 1.0), "x");
                c.hit(x + cw - 13.0, by, 13.0, bh, Hit::UniversePresetDelete(i));
            }
            x += cw + 5.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dons_are_darker_than_kats_in_every_palette() {
        // How bright a colour looks (0-255), by the eye's weighting of red, green and blue.
        let brightness = |hex: u32| {
            let ch = |s: u32| ((hex >> s) & 0xff) as f32;
            0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
        };
        for (name, don, kat) in PALETTES {
            assert!(brightness(don) + 20.0 < brightness(kat), "{name}: don {} kat {}", brightness(don), brightness(kat));
        }
    }

    #[test]
    fn universes_round_trip_through_the_config() {
        let mut own = defaults();
        own[2].colours = Colours::own(0x123456, 0xabcdef);
        // A palette picked with own colours kept behind it.
        own[3].colours = Colours { palette: 1, custom: [0x111111, 0xeeeeee] };
        for all in [defaults(), classic(), shuffle(12345, 4), own] {
            assert_eq!(parse(&format(&all)), all);
        }
        // Missing or garbled universes fall back to the defaults; a mod a universe can't have
        // is left out.
        let all = parse("neon square 1.5x HD,HR; ???");
        assert_eq!(all[0], Universe { colours: Colours { palette: 1, ..Colours::CLASSIC }, shape: Shape::Square, speed: 4, mods: Mods::parse("HD") });
        assert_eq!(all[1], defaults()[1]);
        assert_eq!(all[7], defaults()[7]);
    }

    #[test]
    fn note_colours_and_presets_round_trip_through_the_config() {
        for c in [Colours::CLASSIC, Colours { palette: 3, ..Colours::CLASSIC }, Colours::own(0xff0000, 0x00ff00)] {
            assert_eq!(Colours::parse(&c.format()), c);
        }
        let p = Preset { name: "my chaos".into(), universes: shuffle(9, 4) };
        assert_eq!(Preset::parse(&p.format()), Some(p));
        assert_eq!(Preset::parse(": neon"), None);
    }

    #[test]
    fn colours_are_typed_as_two_hex_codes() {
        assert_eq!(parse_pair("#ff4020 #40a0ff"), Some([0xff4020, 0x40a0ff]));
        assert_eq!(parse_pair("ff4020/40A0FF"), Some([0xff4020, 0x40a0ff]));
        assert_eq!(parse_pair("ff4020, 40a0ff"), Some([0xff4020, 0x40a0ff]));
        assert_eq!(parse_pair("#ff4020"), None);
        assert_eq!(parse_pair("#ff40 #40a0ff"), None);
    }

    #[test]
    fn shuffled_universes_differ_and_keep_the_first_focus_usual() {
        for seed in 0..50 {
            let all = shuffle(seed, 4);
            assert_eq!(all[4], Universe::CLASSIC);
            let mut palettes: Vec<usize> = all.iter().enumerate().filter(|(i, _)| *i != 4).map(|(_, u)| u.colours.palette).collect();
            palettes.dedup();
            assert_eq!(palettes.len(), COUNT - 1);
            assert!(all.iter().all(|u| u.mods.iter().all(|m| MODS.contains(&m))));
        }
    }

    #[test]
    fn a_universe_mod_switches_off_what_it_conflicts_with() {
        let mut all = classic();
        Setup::change(&mut all, 0, 3, 1);
        assert!(all[0].mods.has(Mod::Hidden));
        // Sudden: Hidden goes (together they'd hide every note).
        Setup::change(&mut all, 0, 4, 1);
        assert!(all[0].mods.has(Mod::Sudden) && !all[0].mods.has(Mod::Hidden));
        // Back from classic: the player's own colours, then the last palette.
        Setup::change(&mut all, 0, 0, -1);
        assert_eq!(all[0].colours.name(), "custom");
        Setup::change(&mut all, 0, 0, -1);
        assert_eq!(all[0].colours.name(), "swapped");
    }

    #[test]
    fn presets_save_load_cycle_and_delete() {
        let (mut all, mut own, mut s) = (defaults(), Vec::new(), Setup::new(0, 0));
        let typed = |s: &mut Setup, all: &mut [Universe; COUNT], own: &mut Vec<Preset>, text: &str| {
            s.key(KeyCode::KeyA, Some(text), false, all, own, 4);
            s.key(KeyCode::Enter, None, false, all, own, 4)
        };
        // Own colours for universe 1, then saved as a preset.
        s.key(KeyCode::KeyE, None, false, &mut all, &mut own, 4);
        assert_eq!(s.typing, Some(Typing::Colours("#ff3fb4 #2fe0ff".into())));
        s.typing = Some(Typing::Colours(String::new()));
        assert_eq!(typed(&mut s, &mut all, &mut own, "#102030 #a0b0c0"), Action::Changed);
        assert_eq!(all[0].colours, Colours::own(0x102030, 0xa0b0c0));
        s.key(KeyCode::KeyS, None, false, &mut all, &mut own, 4);
        assert_eq!(typed(&mut s, &mut all, &mut own, "mine"), Action::Changed);
        assert_eq!(own.len(), 1);
        assert_eq!(Setup::current(&all, &own), Some(BUILTIN_PRESETS));
        // ] goes round: defaults, classic, mine.
        s.key(KeyCode::BracketRight, None, false, &mut all, &mut own, 4);
        assert_eq!(all, defaults());
        s.key(KeyCode::BracketRight, None, false, &mut all, &mut own, 4);
        assert_eq!(all, classic());
        s.key(KeyCode::BracketRight, None, false, &mut all, &mut own, 4);
        assert_eq!(all, own[0].universes);
        // The built-in ones stay; the player's go.
        assert_eq!(Setup::delete(&mut own, 0), Action::None);
        s.key(KeyCode::Delete, None, false, &mut all, &mut own, 4);
        assert!(own.is_empty());
        // A bad colour or an empty name is refused, and typing goes on.
        s.key(KeyCode::KeyE, None, false, &mut all, &mut own, 4);
        s.typing = Some(Typing::Colours(String::new()));
        assert_eq!(typed(&mut s, &mut all, &mut own, "red"), Action::None);
        assert!(s.bad && s.typing.is_some());
    }
}
