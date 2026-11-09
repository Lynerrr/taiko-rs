//! Shapeshift's setup: how often the lane takes another shape, how long it takes to morph into
//! it, and the order the shapes come in (shuffled for each map, or the player's own).

use std::sync::RwLock;

/// Shape `i`'s name, by the index `game::SHAPESHIFT` uses.
pub fn shape_name(i: usize) -> &'static str {
    match i {
        0 => tr!("Zigzag", "ジグザグ", "Зигзаг"),
        1 => tr!("U-Turn", "Uターン", "Разворот"),
        2 => tr!("Spiral", "らせん", "Спираль"),
        3 => tr!("Loop", "ループ", "Петля"),
        4 => tr!("Elbow", "L字", "Колено"),
        6 => tr!("Meadow", "草原", "Луг"),
        _ => tr!("Straight", "まっすぐ", "Прямая"),
    }
}
pub(crate) const KEYS: [&str; 7] = [
    "zigzag", "uturn", "spiral", "loop", "elbow", "straight", "meadow",
];
/// Bars each shape lasts.
pub const EVERY: [usize; 4] = [4, 8, 16, 32];
/// Bars the morph into the next shape takes (at most half the shape's bars).
pub const MORPH: [f64; 4] = [0.5, 1.0, 2.0, 4.0];
pub(crate) fn morph_name(i: usize) -> &'static str {
    match i {
        0 => tr!("1/2 bar", "1/2小節", "1/2 такта"),
        1 => tr!("1 bar", "1小節", "1 такт"),
        2 => tr!("2 bars", "2小節", "2 такта"),
        _ => tr!("4 bars", "4小節", "4 такта"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Index into `EVERY`.
    pub every: usize,
    /// Index into `MORPH`.
    pub morph: usize,
    /// Each map its own order of the shapes, else the order of `shapes`.
    pub shuffle: bool,
    /// The shapes (index into `SHAPES`) in the order they come, each on or off.
    pub shapes: [(u8, bool); 7],
}

impl Default for Config {
    fn default() -> Config {
        Config {
            every: 2,
            morph: 2,
            shuffle: true,
            shapes: [
                (0, true),
                (1, true),
                (2, true),
                (3, true),
                (4, true),
                (5, true),
                (6, true),
            ],
        }
    }
}

impl Config {
    /// Bars a shape lasts.
    pub fn bars(&self) -> usize {
        EVERY[self.every.min(EVERY.len() - 1)]
    }

    /// Bars the morph takes: never more than half of a shape's.
    pub fn morph_bars(&self) -> f64 {
        MORPH[self.morph.min(MORPH.len() - 1)].min(self.bars() as f64 / 2.0)
    }

    /// The shapes switched on, in order (all of them if none are).
    fn used(&self) -> Vec<u8> {
        let on: Vec<u8> = self.shapes.iter().filter(|s| s.1).map(|s| s.0).collect();
        if on.is_empty() {
            self.shapes.iter().map(|s| s.0).collect()
        } else {
            on
        }
    }

    /// The shape of each of `sections` (index into `SHAPES`). `seed` (from the map) picks the
    /// shuffled order, so a map always plays (and replays) the same way. The same shape never
    /// comes twice in a row unless it's the only one.
    pub fn sequence(&self, sections: usize, seed: u64) -> Vec<u8> {
        let used = self.used();
        let mut out: Vec<u8> = Vec::with_capacity(sections);
        let mut round = 0u64;
        while out.len() < sections {
            let mut deck = used.clone();
            if self.shuffle {
                // Fisher-Yates, a fresh order every round of the shapes.
                for i in (1..deck.len()).rev() {
                    let j = (mix(seed ^ mix(round * 31 + i as u64)) % (i as u64 + 1)) as usize;
                    deck.swap(i, j);
                }
                if deck.len() > 1 && out.last() == deck.first() {
                    deck.swap(0, 1);
                }
            }
            out.extend(deck);
            round += 1;
        }
        out.truncate(sections);
        out
    }

    /// `every=16 morph=2 order=shuffle shapes=zigzag,uturn,-spiral,...` (- is off).
    pub fn format(&self) -> String {
        let morph = MORPH[self.morph.min(MORPH.len() - 1)];
        let shapes: Vec<String> = self
            .shapes
            .iter()
            .map(|&(s, on)| {
                format!(
                    "{}{}",
                    if on { "" } else { "-" },
                    KEYS[s as usize % KEYS.len()]
                )
            })
            .collect();
        format!(
            "every={} morph={morph} order={} shapes={}",
            self.bars(),
            if self.shuffle { "shuffle" } else { "fixed" },
            shapes.join(",")
        )
    }

    /// Reads `format`'s line; anything missing or unknown stays as the default.
    pub fn parse(s: &str) -> Config {
        let mut c = Config::default();
        for word in s.split_whitespace() {
            match word.split_once('=') {
                Some(("every", v)) => {
                    c.every = EVERY
                        .iter()
                        .position(|&e| v.parse() == Ok(e))
                        .unwrap_or(c.every)
                }
                Some(("morph", v)) => {
                    c.morph = MORPH
                        .iter()
                        .position(|&m| v.parse() == Ok(m))
                        .unwrap_or(c.morph)
                }
                Some(("order", v)) => c.shuffle = v != "fixed",
                Some(("shapes", v)) => {
                    let mut shapes: Vec<(u8, bool)> = Vec::new();
                    for name in v.split(',') {
                        let (on, name) = match name.strip_prefix('-') {
                            Some(n) => (false, n),
                            None => (true, name),
                        };
                        if let Some(k) = KEYS.iter().position(|&n| n == name)
                            && !shapes.iter().any(|s| s.0 == k as u8)
                        {
                            shapes.push((k as u8, on));
                        }
                    }
                    // Any left out go at the end, off: a setup saved (or a lobby host's sent) before
                    // a shape was added plays as it did.
                    for k in 0..KEYS.len() as u8 {
                        if !shapes.iter().any(|s| s.0 == k) {
                            shapes.push((k, false));
                        }
                    }
                    c.shapes.copy_from_slice(&shapes);
                }
                _ => {}
            }
        }
        c
    }
}

/// splitmix64's finaliser.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

static CURRENT: RwLock<Config> = RwLock::new(Config {
    every: 2,
    morph: 2,
    shuffle: true,
    shapes: [
        (0, true),
        (1, true),
        (2, true),
        (3, true),
        (4, true),
        (5, true),
        (6, true),
    ],
});

/// The setup Shapeshift plays with (from the config).
pub fn set(c: Config) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = c;
}

pub fn get() -> Config {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setup_round_trips_through_the_config() {
        let mut c = Config {
            every: 1,
            morph: 0,
            shuffle: false,
            ..Config::default()
        };
        c.shapes.swap(0, 3);
        c.shapes[1].1 = false;
        assert_eq!(Config::parse(&c.format()), c);
        assert_eq!(Config::parse(""), Config::default());
        // Unknown words and shapes are skipped; missing shapes come back at the end, off.
        let p = Config::parse("every=7 shapes=loop,-blob,-zigzag");
        assert_eq!(p.every, Config::default().every);
        assert_eq!(p.shapes[..3], [(3, true), (0, false), (1, false)]);
        let old = Config::parse("shapes=zigzag,uturn,spiral,loop,elbow,straight");
        assert_eq!(
            old.shapes[6],
            (6, false),
            "a setup from before Meadow plays without it"
        );
    }

    #[test]
    fn shuffled_orders_differ_by_map_and_never_repeat_a_shape() {
        let c = Config::default();
        let a = c.sequence(40, 1);
        assert_eq!(a, c.sequence(40, 1), "the same map plays the same way");
        assert_ne!(a, c.sequence(40, 2));
        assert!(a.windows(2).all(|p| p[0] != p[1]), "{a:?}");
        // Every shape comes once per round.
        let mut round: Vec<u8> = a[..7].to_vec();
        round.sort();
        assert_eq!(round, [0, 1, 2, 3, 4, 5, 6]);
        // In order: the player's order, without the shapes left out.
        let mut fixed = Config {
            shuffle: false,
            ..c
        };
        fixed.shapes.swap(0, 5);
        fixed.shapes[2].1 = false;
        assert_eq!(fixed.sequence(7, 9), [5, 1, 3, 4, 0, 6, 5]);
        // Only one shape on: it stays.
        let mut one = c;
        for s in one.shapes.iter_mut().skip(1) {
            s.1 = false;
        }
        assert_eq!(one.sequence(3, 4), [0, 0, 0]);
    }

    #[test]
    fn morphs_take_at_most_half_a_shape() {
        let c = Config {
            every: 0,
            morph: 3,
            ..Config::default()
        };
        assert_eq!((c.bars(), c.morph_bars()), (4, 2.0));
        assert_eq!(Config::default().morph_bars(), 2.0);
    }
}
