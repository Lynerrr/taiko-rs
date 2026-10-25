//! Multiverse's universes: each of its playfields with note colours, a note shape, a scroll speed
//! and visual mods of its own, set up by the player (M in the mod select) and kept in the config.
//! They only change how a playfield is drawn: every universe plays the same map, judged the same.

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
    ("sunset", 0x8e5cff, 0xff7a2f),
    ("ice", 0x2f5fd0, 0x9fe8ff),
    ("forest", 0x2fa86a, 0xe8c547),
    ("candy", 0xff6f91, 0x5fd4a8),
    ("ember", 0xff4a2a, 0xffb02f),
    ("mono", 0x5c5c78, 0xf2f2f2),
    ("toxic", 0xc04dff, 0xb8ff3a),
    ("swapped", 0x2f6f96, 0xff7a5c),
];

/// Grey with no colour in it: notes out of focus (and Colorless).
const GREY: u32 = 0x8c8ca6;

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
    pub palette: usize,
    pub shape: Shape,
    pub speed: usize,
    pub mods: Mods,
}

impl Universe {
    /// The usual playfield (and every playfield without Multiverse).
    pub const CLASSIC: Universe = Universe { palette: 0, shape: Shape::Round, speed: NORMAL_SPEED, mods: Mods::NONE };

    pub fn don(&self) -> u32 {
        PALETTES[self.palette].1
    }

    pub fn kat(&self) -> u32 {
        PALETTES[self.palette].2
    }

    /// Dons and kats alike, out of focus: grey with a hint of the universe's colours (but not
    /// which is which).
    pub fn idle(&self) -> u32 {
        if self.palette == 0 { GREY } else { crate::game::mix(GREY, crate::game::mix(self.don(), self.kat(), 0.5), 0.35) }
    }

    pub fn speed(&self) -> f64 {
        SPEEDS[self.speed]
    }

    /// "neon diamond 1.25x HD,SN" (the config's form).
    fn format(&self) -> String {
        let mods = if self.mods.is_empty() { "-".to_string() } else { self.mods.list().join(",") };
        format!("{} {} {}x {mods}", PALETTES[self.palette].0, self.shape.name(), SPEEDS[self.speed])
    }

    /// Reads `format`'s form; what isn't understood stays as in `base`.
    fn parse(s: &str, base: Universe) -> Universe {
        let mut u = base;
        for word in s.split_whitespace() {
            if let Some(p) = PALETTES.iter().position(|p| p.0.eq_ignore_ascii_case(word)) {
                u.palette = p;
            } else if let Some(&shape) = Shape::ALL.iter().find(|s| s.name().eq_ignore_ascii_case(word)) {
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
    let u = |palette, shape, speed, mods: &str| Universe { palette, shape, speed, mods: only_universe_mods(Mods::parse(mods)) };
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
        Universe { palette, shape, speed, mods }
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

static CURRENT: RwLock<[Universe; COUNT]> = RwLock::new([Universe::CLASSIC; COUNT]);

/// The universes Multiverse plays are drawn with (from the config).
pub fn set(all: [Universe; COUNT]) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = all;
}

pub fn get() -> [Universe; COUNT] {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
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

/// The Multiverse setup (M in the mod select): a row per universe, a column per setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub row: usize,
    pub col: usize,
}

/// What a key or click in the setup asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The universes changed (save them).
    Changed,
    Close,
}

impl Setup {
    /// Changes the setting at `row`, `col`: the next (`dir` 1) or previous (-1) colours, shape or
    /// speed; a mod switches on or off.
    pub fn change(all: &mut [Universe; COUNT], row: usize, col: usize, dir: i32) {
        let u = &mut all[row];
        let step = |i: usize, n: usize| (i as i32 + dir).rem_euclid(n as i32) as usize;
        match col {
            0 => u.palette = step(u.palette, PALETTES.len()),
            1 => u.shape = Shape::ALL[step(Shape::ALL.iter().position(|&s| s == u.shape).unwrap_or(0), Shape::ALL.len())],
            2 => u.speed = step(u.speed, SPEEDS.len()),
            c => u.mods.toggle(MODS[(c - 3).min(MODS.len() - 1)]),
        }
    }

    pub fn key(&mut self, code: KeyCode, shift: bool, all: &mut [Universe; COUNT], first_focus: usize) -> Action {
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
    pub fn draw(&self, c: &mut Canvas, all: &[Universe; COUNT], first_focus: usize) {
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.6));
        let (px, py, pw, ph) = (16.0, 44.0, w - 32.0, 410.0);
        c.round_rect(px, py, pw, ph, 10.0, rgba(0x15151d, 0.99), rgba(0xff5fa2, 0.9), 1.5);
        let title_w = c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), "MULTIVERSE");
        let hint = "arrows choose   Space change (Shift back)   R shuffle   C classic   D defaults   Esc back";
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
        const ROW: f32 = 37.0;
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
                    0 => PALETTES[u.palette].0.to_string(),
                    1 => u.shape.name().to_string(),
                    2 => format!("{}x", SPEEDS[u.speed]),
                    c => MODS[c - 3].acronym().to_string(),
                };
                let col_text = if col >= 3 && !on { 0x6f6f86 } else { 0xffffff };
                c.text_centered(cx + cw / 2.0, mid - 3.5, 7.0, rgba(col_text, 1.0), &Canvas::fit(7.0, cw - 4.0, &text));
                c.hit(cx, y + 3.0, cw, h - 6.0, Hit::UniverseCell(i, col));
            }
        }

        // What the pointed-at (else the selected) setting is.
        let col = hovered.unwrap_or(self.col);
        let about = match col {
            0 => "Colours: the dons' and kats' colours in this universe (out of focus its notes stay alike, faintly tinted).".to_string(),
            1 => "Shape: how its notes look. Round ones use your skin.".to_string(),
            2 => "Speed: how fast its notes scroll, times your scroll speed. The timing is the map's either way.".to_string(),
            c => format!("{}: {}", MODS[c - 3].name(), MODS[c - 3].description()),
        };
        let dy = py + ph - 44.0;
        c.text(px + 16.0, dy, 7.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(7.0, pw - 32.0, &about));
        let note = "* focus starts here   (looks only: same map, same judging)";
        let bw = 60.0;
        c.text(px + 16.0, py + ph - 18.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit(6.0, pw - 4.0 * bw - 60.0, note));
        c.button(px + pw - bw - 12.0, py + ph - 24.0, bw, 16.0, "Back", 7.0, Hit::Key(KeyCode::Escape), true);
        c.button(px + pw - 2.0 * bw - 18.0, py + ph - 24.0, bw, 16.0, "Shuffle", 7.0, Hit::Key(KeyCode::KeyR), false);
        c.button(px + pw - 3.0 * bw - 24.0, py + ph - 24.0, bw, 16.0, "Defaults", 7.0, Hit::Key(KeyCode::KeyD), false);
        c.button(px + pw - 4.0 * bw - 30.0, py + ph - 24.0, bw, 16.0, "Classic", 7.0, Hit::Key(KeyCode::KeyC), false);
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
        for all in [defaults(), classic(), shuffle(12345, 4)] {
            assert_eq!(parse(&format(&all)), all);
        }
        // Missing or garbled universes fall back to the defaults; a mod a universe can't have
        // is left out.
        let all = parse("neon square 1.5x HD,HR; ???");
        assert_eq!(all[0], Universe { palette: 1, shape: Shape::Square, speed: 4, mods: Mods::parse("HD") });
        assert_eq!(all[1], defaults()[1]);
        assert_eq!(all[7], defaults()[7]);
    }

    #[test]
    fn shuffled_universes_differ_and_keep_the_first_focus_usual() {
        for seed in 0..50 {
            let all = shuffle(seed, 4);
            assert_eq!(all[4], Universe::CLASSIC);
            let mut palettes: Vec<usize> = all.iter().enumerate().filter(|(i, _)| *i != 4).map(|(_, u)| u.palette).collect();
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
        Setup::change(&mut all, 0, 0, -1);
        assert_eq!(PALETTES[all[0].palette].0, "swapped");
    }
}
