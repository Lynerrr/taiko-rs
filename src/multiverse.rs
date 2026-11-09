//! Multiverse's universes: each of its playfields with note colours, a note shape, a scroll speed
//! and visual mods of its own, set up by the player (M in the mod select) and kept in the config,
//! along with presets of whole setups. They only change how a playfield is drawn: every universe
//! plays the same map, judged the same. Also the note colours of a play without Multiverse.

use std::sync::RwLock;

use crate::mods::{Mod, Mods};

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

/// Palette `i`'s name as shown (`CUSTOM`: the player's own colours).
pub fn palette_label(i: usize) -> &'static str {
    match i {
        0 => tr!("classic", "クラシック", "классика"),
        1 => tr!("neon", "ネオン", "неон"),
        2 => tr!("sunset", "夕焼け", "закат"),
        3 => tr!("ice", "氷", "лёд"),
        4 => tr!("forest", "森", "лес"),
        5 => tr!("candy", "キャンディ", "конфеты"),
        6 => tr!("ember", "残り火", "угли"),
        7 => tr!("mono", "モノクロ", "моно"),
        8 => tr!("toxic", "毒々しい", "токсик"),
        9 => tr!("swapped", "入れ替え", "наоборот"),
        _ => tr!("custom", "自分の色", "свои"),
    }
}

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
    pub const CLASSIC: Colours = Colours {
        palette: 0,
        custom: [crate::game::DON, crate::game::KAT],
    };

    pub fn don(self) -> u32 {
        PALETTES.get(self.palette).map_or(self.custom[0], |p| p.1)
    }

    pub fn kat(self) -> u32 {
        PALETTES.get(self.palette).map_or(self.custom[1], |p| p.2)
    }

    pub fn name(self) -> &'static str {
        PALETTES.get(self.palette).map_or("custom", |p| p.0)
    }

    /// Its name as shown (`name` is the config's).
    pub fn label(self) -> &'static str {
        palette_label(self.palette)
    }

    /// Dons and kats alike, out of focus: grey with a hint of the colours (but not which is
    /// which).
    pub fn idle(self) -> u32 {
        if self.palette == 0 {
            GREY
        } else {
            crate::game::mix(GREY, crate::game::mix(self.don(), self.kat(), 0.5), 0.35)
        }
    }

    /// The next (`dir` 1) or previous palette; the player's own come after the last.
    pub fn step(self, dir: i32) -> Colours {
        Colours {
            palette: (self.palette as i32 + dir).rem_euclid(CUSTOM as i32 + 1) as usize,
            ..self
        }
    }

    /// The player's own colours, picked.
    pub fn own(don: u32, kat: u32) -> Colours {
        Colours {
            palette: CUSTOM,
            custom: [don, kat],
        }
    }

    /// "neon", "custom #ff4020/#40a0ff", or a palette and the own colours kept behind it.
    pub fn format(self) -> String {
        if self.palette != CUSTOM && self.custom == Colours::CLASSIC.custom {
            return self.name().to_string();
        }
        format!(
            "{} #{:06x}/#{:06x}",
            self.name(),
            self.custom[0],
            self.custom[1]
        )
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
    (h.len() == 6)
        .then(|| u32::from_str_radix(h, 16).ok())
        .flatten()
}

/// Two colours, don then kat: "#ff4020 #40a0ff", "ff4020/40a0ff", "ff4020, 40a0ff"...
pub fn parse_pair(s: &str) -> Option<[u32; 2]> {
    let parts: Vec<&str> = s
        .split(|c: char| c.is_whitespace() || c == '/' || c == ',')
        .filter(|p| !p.is_empty())
        .collect();
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
    pub const ALL: [Shape; 5] = [
        Shape::Round,
        Shape::Square,
        Shape::Diamond,
        Shape::Ring,
        Shape::Pill,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Shape::Round => "round",
            Shape::Square => "square",
            Shape::Diamond => "diamond",
            Shape::Ring => "ring",
            Shape::Pill => "pill",
        }
    }

    /// Its name as shown (`name` is the config's).
    pub fn label(self) -> &'static str {
        match self {
            Shape::Round => tr!("round", "丸", "круг"),
            Shape::Square => tr!("square", "四角", "квадрат"),
            Shape::Diamond => tr!("diamond", "ひし形", "ромб"),
            Shape::Ring => tr!("ring", "輪", "кольцо"),
            Shape::Pill => tr!("pill", "カプセル", "капсула"),
        }
    }
}

/// Scroll speeds, times the player's own.
pub const SPEEDS: [f64; 5] = [0.75, 0.9, 1.0, 1.25, 1.5];
const NORMAL_SPEED: usize = 2;

/// The mods a universe can have: those that only change how notes are drawn and move, worked
/// out as each playfield is drawn (nothing about them is set up when the play starts).
pub const MODS: [Mod; 10] = [
    Mod::Hidden,
    Mod::Sudden,
    Mod::Blink,
    Mod::Snake,
    Mod::Party,
    Mod::Wave,
    Mod::Accel,
    Mod::Brake,
    Mod::Pulse,
    Mod::Stutter,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Universe {
    pub colours: Colours,
    pub shape: Shape,
    pub speed: usize,
    pub mods: Mods,
}

impl Universe {
    /// The usual playfield (and every playfield without Multiverse, in the player's colours).
    pub const CLASSIC: Universe = Universe {
        colours: Colours::CLASSIC,
        shape: Shape::Round,
        speed: NORMAL_SPEED,
        mods: Mods::NONE,
    };

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
        let mods = if self.mods.is_empty() {
            "-".to_string()
        } else {
            self.mods.list().join(",")
        };
        format!(
            "{} {} {}x {mods}",
            self.colours.format(),
            self.shape.name(),
            SPEEDS[self.speed]
        )
    }

    /// Reads `format`'s form; what isn't understood stays as in `base`.
    fn parse(s: &str, base: Universe) -> Universe {
        let mut u = base;
        for word in s.split_whitespace() {
            if u.colours.read(word) {
                continue;
            }
            if let Some(&shape) = Shape::ALL
                .iter()
                .find(|s| s.name().eq_ignore_ascii_case(word))
            {
                u.shape = shape;
            } else if let Some(speed) = word.strip_suffix('x').and_then(|v| v.parse::<f64>().ok()) {
                u.speed = SPEEDS
                    .iter()
                    .position(|&s| (s - speed).abs() < 1e-6)
                    .unwrap_or(u.speed);
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
        colours: Colours {
            palette,
            ..Colours::CLASSIC
        },
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
        let mods = if next(2) == 0 {
            Mods::NONE
        } else {
            Mods::parse(MODS[next(MODS.len())].acronym())
        };
        Universe {
            colours: Colours {
                palette,
                ..Colours::CLASSIC
            },
            shape,
            speed,
            mods,
        }
    })
}

/// The universes as the config keeps them: `; ` between them.
pub fn format(all: &[Universe; COUNT]) -> String {
    all.iter()
        .map(Universe::format)
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn parse(s: &str) -> [Universe; COUNT] {
    let parts: Vec<&str> = s.split(';').collect();
    let base = defaults();
    std::array::from_fn(|i| {
        parts
            .get(i)
            .map_or(base[i], |p| Universe::parse(p, base[i]))
    })
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
        (!name.is_empty()).then(|| Preset {
            name,
            universes: parse(rest),
        })
    }
}

/// A preset name as typed, without what the config uses to separate things.
pub(crate) fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !matches!(c, ':' | ';'))
        .collect::<String>()
        .trim()
        .chars()
        .take(24)
        .collect()
}

/// Every preset in the order the setup lists them: the built-in ones, then the player's.
pub(crate) fn presets(own: &[Preset]) -> Vec<(String, [Universe; COUNT])> {
    let builtin = [
        (
            tr!("defaults", "初期設定", "исходные").to_string(),
            defaults(),
        ),
        (
            tr!("classic", "クラシック", "классика").to_string(),
            classic(),
        ),
    ];
    builtin
        .into_iter()
        .chain(own.iter().map(|p| (p.name.clone(), p.universes)))
        .collect()
}
pub(crate) const BUILTIN_PRESETS: usize = 2;

static CURRENT: RwLock<[Universe; COUNT]> = RwLock::new([Universe::CLASSIC; COUNT]);
static NOTE_COLOURS: RwLock<Colours> = RwLock::new(Colours::CLASSIC);

/// The universes Multiverse plays are drawn with (from the config).
pub fn set(all: [Universe; COUNT]) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = all;
}

pub fn get() -> [Universe; COUNT] {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// The configured note colours of plays without Multiverse (Settings > Skin), for threads not
/// building a frame (`game::Look`).
pub fn set_note_colours(colours: Colours) {
    *NOTE_COLOURS.write().unwrap_or_else(|e| e.into_inner()) = colours;
}

pub fn configured_note_colours() -> Colours {
    *NOTE_COLOURS.read().unwrap_or_else(|e| e.into_inner())
}

/// The note colours of plays without Multiverse, as this thread draws them (`game::playfield_look`).
pub fn note_colours() -> Colours {
    crate::game::playfield_look().note_colours
}

/// Text being typed in the setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typing {
    /// The selected universe's own colours, don then kat.
    Colours(String),
    /// A name to save the universes under.
    Name(String),
}

/// What a key or click in the setup asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The universes or the presets changed (save them).
    Changed,
    Close,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_s_note_colours_hold_while_another_thread_sets_them() {
        let mine = Colours {
            palette: 0,
            custom: [0x112233, 0x445566],
        };
        crate::game::set_frame_look(Some(crate::game::Look {
            note_colours: mine,
            ..crate::game::Look::default()
        }));
        std::thread::spawn(|| {
            for i in 0..1000 {
                set_note_colours(Colours {
                    palette: 0,
                    custom: [i, i],
                });
            }
        })
        .join()
        .unwrap();
        assert_eq!(note_colours(), mine);
        crate::game::set_frame_look(None);
        assert_ne!(
            note_colours(),
            mine,
            "without a frame: the configured colours"
        );
        set_note_colours(Colours::CLASSIC);
    }

    #[test]
    fn dons_are_darker_than_kats_in_every_palette() {
        // How bright a colour looks (0-255), by the eye's weighting of red, green and blue.
        let brightness = |hex: u32| {
            let ch = |s: u32| ((hex >> s) & 0xff) as f32;
            0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
        };
        for (name, don, kat) in PALETTES {
            assert!(
                brightness(don) + 20.0 < brightness(kat),
                "{name}: don {} kat {}",
                brightness(don),
                brightness(kat)
            );
        }
    }

    #[test]
    fn universes_round_trip_through_the_config() {
        let mut own = defaults();
        own[2].colours = Colours::own(0x123456, 0xabcdef);
        // A palette picked with own colours kept behind it.
        own[3].colours = Colours {
            palette: 1,
            custom: [0x111111, 0xeeeeee],
        };
        for all in [defaults(), classic(), shuffle(12345, 4), own] {
            assert_eq!(parse(&format(&all)), all);
        }
        // Missing or garbled universes fall back to the defaults; a mod a universe can't have
        // is left out.
        let all = parse("neon square 1.5x HD,HR; ???");
        assert_eq!(
            all[0],
            Universe {
                colours: Colours {
                    palette: 1,
                    ..Colours::CLASSIC
                },
                shape: Shape::Square,
                speed: 4,
                mods: Mods::parse("HD")
            }
        );
        assert_eq!(all[1], defaults()[1]);
        assert_eq!(all[7], defaults()[7]);
    }

    #[test]
    fn note_colours_and_presets_round_trip_through_the_config() {
        for c in [
            Colours::CLASSIC,
            Colours {
                palette: 3,
                ..Colours::CLASSIC
            },
            Colours::own(0xff0000, 0x00ff00),
        ] {
            assert_eq!(Colours::parse(&c.format()), c);
        }
        let p = Preset {
            name: "my chaos".into(),
            universes: shuffle(9, 4),
        };
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
            let mut palettes: Vec<usize> = all
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 4)
                .map(|(_, u)| u.colours.palette)
                .collect();
            palettes.dedup();
            assert_eq!(palettes.len(), COUNT - 1);
            assert!(all.iter().all(|u| u.mods.iter().all(|m| MODS.contains(&m))));
        }
    }
}
