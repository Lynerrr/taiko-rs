//! Mods: osu!'s (EZ, HR, HD, FL, DT...), lazer's taiko extras and taiko.rs's own (scroll speed
//! tricks, SV experiments, visual tricks, note paths, playfield moves and copies). A set of them
//! is a bit mask; each has a two-letter acronym used in the config, replays and multiplayer.
//!
//! Each mod has a file of its own here with its definition (`DEF`: acronym, name, score
//! multiplier, conflicts...), and a line in the registry below. Adding a mod is a new file and
//! a new line; nothing else lists the mods.

/// Ties each mod to its number and its file, column by column as the mod select shows them, and
/// makes `Mod`, `ALL` and `GROUPS` of it.
macro_rules! registry {
    ($($title:literal $colour:literal { $($m:ident = $bit:literal => $file:ident,)* })*) => {
        $($(mod $file;)*)*

        /// Every mod, column by column as the mod select shows them. A mod's number is its bit in
        /// `Mods`: presets, favourites and plays keep these, so a number never changes and a new
        /// mod takes the next one (the build fails on a gap or a number used twice).
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u8)]
        pub enum Mod {
            $($($m = $bit,)*)*
        }

        /// Each mod's definition, in the registry's order.
        const REGISTRY: &[(Mod, &ModDef)] = &[$($((Mod::$m, &$file::DEF),)*)*];

        /// Mod select columns: (title, accent colour, mods).
        pub const GROUPS: [(&str, u32, &[Mod]); [$($title),*].len()] =
            [$(($title, $colour, &[$(Mod::$m),*]),)*];
    };
}

registry! {
    "REDUCTION" 0x86E05A {
        Easy = 0 => easy,
        NoFail = 1 => no_fail,
        HalfTime = 2 => half_time,
        Daycore = 3 => daycore,
        Half = 4 => half,
        NoFinishers = 5 => no_finishers,
    }
    "INCREASE" 0xEB452C {
        HardRock = 6 => hard_rock,
        SuddenDeath = 7 => sudden_death,
        Perfect = 8 => perfect,
        DoubleTime = 9 => double_time,
        Nightcore = 10 => nightcore,
        Hidden = 11 => hidden,
        Flashlight = 12 => flashlight,
        Precise = 13 => precise,
        GreatOrNothing = 14 => great_or_nothing,
    }
    "CONVERSION" 0x4fb4ff {
        Relax = 15 => relax,
        Autoplay = 16 => autoplay,
        Swap = 17 => swap,
        Random = 18 => random,
        ConstantSpeed = 19 => constant_speed,
        Alternate = 20 => alternate,
        WindUp = 21 => wind_up,
        WindDown = 22 => wind_down,
        AllBig = 23 => all_big,
        OdOverride = 78 => od_override,
    }
    "SCROLL" 0xFFD34D {
        Accel = 24 => accel,
        Brake = 25 => brake,
        Wave = 26 => wave,
        Pulse = 27 => pulse,
        Stutter = 28 => stutter,
        SvChaos = 29 => sv_chaos,
        Crescendo = 30 => crescendo,
        Drop = 31 => drop,
        Gearbox = 32 => gearbox,
        Roulette = 33 => roulette,
        Tact = 77 => tact,
    }
    "SV LAB" 0xFF8A3D {
        AntiSv = 34 => anti_sv,
        HyperSv = 35 => hyper_sv,
        Even = 36 => even,
        Split = 37 => split,
        Swing = 38 => swing,
        Leapfrog = 39 => leapfrog,
        Panic = 40 => panic,
        Mercy = 41 => mercy,
        LagSpike = 42 => lag_spike,
        Moonwalk = 43 => moonwalk,
        SnapSpeed = 44 => snap_speed,
    }
    "VISUAL" 0xc77dff {
        Sudden = 45 => sudden,
        Blink = 46 => blink,
        Blackout = 47 => blackout,
        Spotlight = 48 => spotlight,
        Colorless = 49 => colorless,
        Memory = 50 => memory,
        Decoy = 51 => decoy,
        Party = 52 => party,
        Overtime = 53 => overtime,
        Liar = 54 => liar,
        Radar = 55 => radar,
        Echo = 56 => echo,
    }
    "RAILS" 0x5ee6c8 {
        Zigzag = 57 => zigzag,
        UTurn = 58 => u_turn,
        Spiral = 59 => spiral,
        Loop = 60 => loop_the_loop,
        Elbow = 61 => elbow,
        Maze = 64 => maze,
        Shapeshift = 62 => shapeshift,
        Snake = 63 => snake,
        Crossfire = 71 => crossfire,
    }
    "PLAYFIELD" 0xff5fa2 {
        Mirror = 65 => mirror,
        Tilt = 66 => tilt,
        Spin = 67 => spin,
        Mania = 68 => mania,
        BarrelRoll = 69 => barrel_roll,
        FlipFlop = 70 => flip_flop,
        Shatter = 72 => shatter,
        Cracked = 73 => cracked,
        Crumble = 74 => crumble,
        Portal = 75 => portal,
        Multiverse = 76 => multiverse,
    }
}

use Mod::*;

/// How many mods there are.
const COUNT: usize = REGISTRY.len();

/// Every mod by its number (`Mod::bit`), as the mod select's keyboard and the lists of mods go.
pub const ALL: [Mod; COUNT] = {
    let mut all = [Mod::Easy; COUNT];
    let mut set = [false; COUNT];
    let mut i = 0;
    while i < COUNT {
        let m = REGISTRY[i].0;
        assert!(
            (m as usize) < COUNT,
            "mods are numbered from 0 without gaps"
        );
        // The enum already refuses a number used twice; this keeps the table honest too.
        assert!(!set[m as usize], "two mods share a number");
        set[m as usize] = true;
        all[m as usize] = m;
        i += 1;
    }
    all
};

/// Each mod's definition by its number.
const DEFS: [&ModDef; COUNT] = {
    let mut defs = [&ModDef::DEFAULT; COUNT];
    let mut i = 0;
    while i < COUNT {
        defs[REGISTRY[i].0 as usize] = REGISTRY[i].1;
        i += 1;
    }
    defs
};

/// The bits of the mods whose definition says so.
macro_rules! mask {
    ($def:ident => $test:expr) => {{
        let mut bits = 0u128;
        let mut i = 0;
        while i < COUNT {
            let $def = DEFS[i];
            if $test {
                bits |= 1 << i;
            }
            i += 1;
        }
        bits
    }};
}

/// Mods that change the song's speed: everyone in a lobby plays the host's (`ModDef::lobby_wide`).
const LOBBY_WIDE: u128 = mask!(d => d.lobby_wide);
/// Mods a Defense match can't have (`ModDef::in_defense`).
const DEFENSE_BANNED: u128 = mask!(d => !d.in_defense);

/// What a mod is: its file's `DEF`. Everything here is data the mod select, the shuffle, lobbies
/// and scores read through `Mod`'s methods.
struct ModDef {
    /// Two letters, in the config, replays, labels and multiplayer.
    acronym: &'static str,
    name: &'static str,
    /// The mod select's line about it.
    description: &'static str,
    /// Score multiplier: osu!'s for its mods, and on the same scale for taiko.rs's own by how
    /// much harder they make a map to read or play (HR's 1.06 is "clearly harder", DT's 1.12
    /// "a lot"). Mods that help score below 1; ones that only look different stay at 1.
    multiplier: f64,
    /// How much harder it makes a map to play, for the shuffle (`Mods::shuffle`): easier mods
    /// count against the rest. `None`: never shuffled in (nobody plays, the play just ends
    /// early, or there's nothing left to read).
    shuffle_cost: Option<i32>,
    /// Mods it can't be combined with (both ways: each lists the other).
    conflicts: &'static [Mod],
    /// It hides or disguises notes: the shuffle takes one of those at a time.
    obscures: bool,
    /// It changes the song's speed: a lobby's host picks it for everyone, so the song stays in
    /// step for all. The rest each player picks for themselves.
    lobby_wide: bool,
    /// It can be on in a Defense match. A team shares its lives and gold, so nothing may play
    /// for a player, spare a player the misses or end one player's run alone.
    in_defense: bool,
    /// It has settings of its own: its tab in the mod customization (tabs go by this number).
    setup_tab: Option<u8>,
}

impl ModDef {
    /// What a definition leaves out: no conflicts, none of the flags.
    const DEFAULT: ModDef = ModDef {
        acronym: "",
        name: "",
        description: "",
        multiplier: 1.0,
        shuffle_cost: Some(1),
        conflicts: &[],
        obscures: false,
        lobby_wide: false,
        in_defense: true,
        setup_tab: None,
    };
}

/// How Half picks the notes it takes off.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HalfRule {
    /// Off each stream, its finest subdivision (see `half_keeps`).
    #[default]
    OnTheBeat,
    /// Every other note, plays of pp version 1 and before, replay version 3 and before.
    EveryOther,
}

impl HalfRule {
    /// The rule of a play sent with `pp_version` (`online::PP_VERSION`).
    pub fn of_pp_version(pp_version: u32) -> HalfRule {
        if pp_version >= 2 {
            HalfRule::OnTheBeat
        } else {
            HalfRule::EveryOther
        }
    }
}

/// How far from a stream's grid still counts as on it, in ms (`.osu` times are whole ms).
const GRID_TOLERANCE: f64 = 3.0;
/// Notes closer than this are a stream whatever the beat says (a map with a made-up BPM), in ms.
const ALWAYS_THINNED: f64 = 100.0;

/// Which of the notes at `times` (ascending; dons and kats) Half keeps. `beat_at`: the beat a
/// time is in, as where it starts and its length.
///
/// A stream (notes evenly spaced under a beat apart) keeps the notes on the grid twice as coarse:
/// 1/4 becomes 1/2, 1/2 becomes 1/1, 1/6 becomes 1/3. Where that isn't a grid (1/3), it goes to
/// the beat; a stream off the beat keeps every other note. Notes a beat or more apart all stay,
/// so what's left is still on the music.
pub fn half_keeps(times: &[f64], rule: HalfRule, beat_at: &dyn Fn(f64) -> (f64, f64)) -> Vec<bool> {
    if rule == HalfRule::EveryOther {
        return (0..times.len()).map(|k| k % 2 == 0).collect();
    }
    let mut keep = vec![true; times.len()];
    let gap = |k: usize| times[k + 1] - times[k];
    let mut start = 0;
    while start + 1 < times.len() {
        let spacing = gap(start);
        let mut end = start + 1;
        while end + 1 < times.len()
            && (gap(end) - spacing).abs() <= GRID_TOLERANCE.max(spacing * 0.08)
        {
            end += 1;
        }
        let (origin, beat) = beat_at(times[start]);
        let steps = (beat / spacing).round() as i64;
        if spacing < beat * 0.98 || spacing < ALWAYS_THINNED {
            let step = beat / steps.max(1) as f64;
            // The notes' places on the stream's grid, if they're on it (a grid too fine to tell
            // isn't one).
            let places: Option<Vec<i64>> = times[start..=end]
                .iter()
                .map(|t| {
                    let place = (t - origin) / step;
                    (steps >= 2
                        && step > GRID_TOLERANCE * 2.0
                        && (place - place.round()).abs() * step <= GRID_TOLERANCE)
                        .then_some(place.round() as i64)
                })
                .collect();
            for k in start..=end {
                keep[k] &= match &places {
                    Some(p) if steps % 2 == 0 => p[k - start].rem_euclid(2) == 0,
                    Some(p) => p[k - start].rem_euclid(steps) == 0,
                    None => (k - start) % 2 == 0,
                };
            }
        }
        // The note between two streams is in both.
        start = end;
    }
    keep
}

/// The tabs of the mod customization: the mods with settings that are on (only those show).
pub fn customizable(mods: Mods) -> Vec<Mod> {
    let mut tabs: Vec<(u8, Mod)> = mods
        .iter()
        .filter_map(|m| m.def().setup_tab.map(|tab| (tab, m)))
        .collect();
    tabs.sort_by_key(|&(tab, _)| tab);
    tabs.into_iter().map(|(_, m)| m).collect()
}

/// Mod select keyboard navigation: the mod `dx` columns and `dy` rows from `index` (into `ALL`).
pub fn step(index: usize, dx: i32, dy: i32) -> usize {
    let m = ALL[index.min(ALL.len() - 1)];
    let (col, row) = GROUPS
        .iter()
        .enumerate()
        .find_map(|(c, g)| g.2.iter().position(|&x| x == m).map(|r| (c, r)))
        .unwrap_or((0, 0));
    let col = (col as i32 + dx).rem_euclid(GROUPS.len() as i32) as usize;
    let list = GROUPS[col].2;
    let row = (row as i32 + dy).clamp(0, list.len() as i32 - 1) as usize;
    ALL.iter().position(|&x| x == list[row]).unwrap_or(0)
}

impl Mod {
    /// Its definition (its file's `DEF`).
    #[inline]
    fn def(self) -> &'static ModDef {
        DEFS[self as usize]
    }

    pub fn acronym(self) -> &'static str {
        self.def().acronym
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn description(self) -> &'static str {
        self.def().description
    }

    /// Score multiplier (`ModDef::multiplier`).
    pub fn multiplier(self) -> f64 {
        self.def().multiplier
    }

    /// Mods this one can't be combined with.
    fn conflicts(self) -> &'static [Mod] {
        self.def().conflicts
    }

    /// How much harder this mod makes a map to play, for the shuffle (`ModDef::shuffle_cost`).
    fn shuffle_cost(self) -> Option<i32> {
        self.def().shuffle_cost
    }

    /// Mods that hide or disguise notes: one of them at a time is plenty.
    fn obscures(self) -> bool {
        self.def().obscures
    }

    /// Mods that change where and how fast notes travel.
    fn moves_notes(self) -> bool {
        GROUPS
            .iter()
            .filter(|g| ["SCROLL", "SV LAB", "RAILS", "PLAYFIELD"].contains(&g.0))
            .any(|g| g.2.contains(&self))
    }

    /// The mod's bit in `Mods`: its number.
    #[inline]
    fn bit(self) -> u128 {
        1 << self as u8
    }

    pub fn from_acronym(s: &str) -> Option<Mod> {
        ALL.iter()
            .copied()
            .find(|m| m.acronym().eq_ignore_ascii_case(s.trim()))
    }

    /// A lobby's host picks it for everyone (it changes the song's speed).
    pub fn shared(self) -> bool {
        self.def().lobby_wide
    }

    /// Whether it can be on in a Defense match (`ModDef::in_defense`).
    pub fn defense_ok(self) -> bool {
        self.def().in_defense
    }
}

/// A set of mods.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods(u128);

impl Mods {
    pub const NONE: Mods = Mods(0);

    /// The mods everyone in a lobby plays with (the host's): the speed's.
    pub fn shared(self) -> Mods {
        Mods(self.0 & LOBBY_WIDE)
    }

    /// The rest: each player's own in a lobby. Not OD Override: everyone's judged by the map's.
    pub fn free(self) -> Mods {
        Mods(self.0 & !self.shared().0 & !Mod::OdOverride.bit())
    }

    /// What the host's mods make of the lobby's match: in an ordinary one just the speed's (the
    /// others are each player's own), in Defense all of them but the ones a team can't have,
    /// for everyone, so the whole team plays the same notes. With the Maze lane on, Maze is one
    /// of them and any other rail it clashes with is dropped.
    pub fn lobby(self, defense: Option<crate::defense::Defense>) -> Mods {
        let Some(d) = defense else {
            return self.shared();
        };
        let mut out = Mods(self.0 & !DEFENSE_BANNED);
        if d.maze && !out.has(Mod::Maze) {
            out.toggle(Mod::Maze);
        }
        out
    }

    #[inline]
    pub fn has(self, m: Mod) -> bool {
        self.0 & m.bit() != 0
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn iter(self) -> impl Iterator<Item = Mod> {
        ALL.into_iter().filter(move |&m| self.has(m))
    }

    /// Whether selecting `m` would switch off an active mod. Active mods stay highlighted.
    pub fn incompatible(self, m: Mod) -> bool {
        !self.has(m) && m.conflicts().iter().any(|&c| self.has(c))
    }

    /// Turns `m` on (switching off what it conflicts with) or off.
    pub fn toggle(&mut self, m: Mod) {
        if self.has(m) {
            self.0 &= !m.bit();
        } else {
            for &c in m.conflicts() {
                self.0 &= !c.bit();
            }
            self.0 |= m.bit();
        }
    }

    /// `self` with `extra`'s mods added, but not those that clash with one of `self`'s (a
    /// Multiverse universe's mods on top of the play's).
    #[inline]
    pub fn plus(self, extra: Mods) -> Mods {
        let mut out = self;
        for m in extra.iter() {
            if !m.conflicts().iter().any(|&c| self.has(c)) {
                out.0 |= m.bit();
            }
        }
        out
    }

    /// From "HD,HR" (unknown acronyms are skipped, conflicts resolved in order).
    pub fn parse(s: &str) -> Mods {
        let mut out = Mods::NONE;
        for m in s.split([',', ' ']).filter_map(Mod::from_acronym) {
            if !out.has(m) {
                out.toggle(m);
            }
        }
        out
    }

    /// Just a set of mods, conflicting ones too (the favourites): "HD,HR,EZ".
    pub fn parse_set(s: &str) -> Mods {
        Mods(
            s.split([',', ' '])
                .filter_map(Mod::from_acronym)
                .fold(0, |bits, m| bits | m.bit()),
        )
    }

    /// `self` with `m` added or taken out, conflicts or not.
    pub fn flip(self, m: Mod) -> Mods {
        Mods(self.0 ^ m.bit())
    }

    pub fn from_list(list: &[String]) -> Mods {
        Mods::parse(&list.join(","))
    }

    pub fn list(self) -> Vec<String> {
        self.iter().map(|m| m.acronym().to_string()).collect()
    }

    /// A random set of two to five mods that stays playable (R in the mod select): they add up to
    /// a difficulty of 3-6 (`shuffle_cost`, so an easier mod makes room for more), come from
    /// different mod select columns, at most one hides notes and at most three move them. A heavy
    /// set gets No Fail on top. `seed` picks the set.
    pub fn shuffle(seed: u64) -> Mods {
        // xorshift64*, like the song select's F2.
        let mut rng = seed | 1;
        let mut next = |n: usize| {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            (rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n
        };
        let budget = 3 + next(4) as i32;
        let want = 2 + next(4);
        let mut columns: Vec<usize> = (0..GROUPS.len()).collect();
        for i in (1..columns.len()).rev() {
            columns.swap(i, next(i + 1));
        }
        let (mut out, mut cost, mut count) = (Mods::NONE, 0, 0);
        for g in columns {
            // Making it easier is the exception, not half the shuffles.
            if count == want || (g == 0 && next(2) == 0) {
                continue;
            }
            let fits: Vec<Mod> = GROUPS[g]
                .2
                .iter()
                .copied()
                .filter(|&m| {
                    let Some(c) = m.shuffle_cost() else {
                        return false;
                    };
                    m != NoFail
                        && cost + c <= budget
                        && !m.conflicts().iter().any(|&x| out.has(x))
                        && !(m.obscures() && out.iter().any(Mod::obscures))
                        && !(m.moves_notes() && out.iter().filter(|x| x.moves_notes()).count() >= 3)
                })
                .collect();
            if fits.is_empty() {
                continue;
            }
            let m = fits[next(fits.len())];
            out.toggle(m);
            cost += m.shuffle_cost().unwrap_or(0);
            count += 1;
        }
        if cost >= 5 {
            out.toggle(NoFail);
        }
        out
    }

    /// "HD,HR" (config, replays).
    pub fn to_config(self) -> String {
        self.list().join(",")
    }

    /// "HDHR", or "" without mods.
    pub fn label(self) -> String {
        self.iter().map(|m| m.acronym()).collect()
    }

    pub fn multiplier(self) -> f64 {
        self.iter().map(|m| m.multiplier()).product()
    }

    /// Speed and pitch the mod forces: DT/NC 1.5x, HT/DC 0.75x (keep pitch or not).
    pub fn rate_override(self) -> Option<(f64, bool)> {
        if self.has(DoubleTime) {
            Some((1.5, true))
        } else if self.has(Nightcore) {
            Some((1.5, false))
        } else if self.has(HalfTime) {
            Some((0.75, true))
        } else if self.has(Daycore) {
            Some((0.75, false))
        } else {
            None
        }
    }

    /// The speed a play runs at, from the selected speed (`rate`, `keep_pitch`) and the mods.
    #[inline]
    pub fn speed(self, rate: f64, keep_pitch: bool) -> (f64, bool) {
        self.rate_override().unwrap_or((rate, keep_pitch))
    }

    /// Wind Up / Down: the factor the speed reaches by the last note.
    pub fn wind(self) -> Option<f64> {
        if self.has(WindUp) {
            Some(1.5)
        } else if self.has(WindDown) {
            Some(0.75)
        } else {
            None
        }
    }

    /// OD after EZ / HR / Precise, or OD Override's default instead of all of them
    /// (`od_with` for the one a player picked).
    #[inline]
    pub fn od(self, od: f64) -> f64 {
        self.od_with(od, crate::od_override::DEFAULT)
    }

    /// OD after EZ / HR / Precise, or `override_od` for OD Override instead of all of them.
    #[inline]
    pub fn od_with(self, od: f64, override_od: f64) -> f64 {
        if self.has(OdOverride) {
            return crate::od_override::clamped(override_od);
        }
        let mut od = od;
        if self.has(Easy) {
            od *= 0.5;
        }
        if self.has(HardRock) {
            od = (od * 1.4).min(10.0);
        }
        if self.has(Precise) {
            od = (od + 2.0).min(10.0);
        }
        od
    }

    /// HP drain after EZ / HR.
    pub fn hp(self, hp: f64) -> f64 {
        if self.has(Easy) {
            hp * 0.5
        } else if self.has(HardRock) {
            (hp * 1.4).min(10.0)
        } else {
            hp
        }
    }

    /// Scroll speed factor of EZ / HR (osu!lazer's taiko slider multipliers).
    #[inline]
    pub fn scroll(self) -> f64 {
        if self.has(Easy) {
            0.8
        } else if self.has(HardRock) {
            1.4 * 4.0 / 3.0
        } else {
            1.0
        }
    }

    /// Mods that make a play's score not count (no local score, replay or leaderboard row).
    /// Relax counts, on boards and a pp total of its own (`relax`). OD Override's value isn't in
    /// the mods, so a saved play couldn't be judged again, and OD 0 would be free accuracy.
    pub fn unranked(self) -> bool {
        self.has(Autoplay) || self.has(OdOverride)
    }

    /// Whether a play goes on the Relax boards instead of the usual ones.
    pub fn relax(self) -> bool {
        self.has(Relax)
    }
}

/// How map time relates to song (track) time: `r0` map ms per ms until map time `m0`, then
/// changing linearly to `r1` at `m1`, `r1` after. A constant speed has `r0 == r1`. Wind Up/Down
/// ramp it from the first note to the last; everything else plays at one speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speed {
    pub r0: f64,
    pub r1: f64,
    pub m0: f64,
    pub m1: f64,
}

impl Speed {
    pub fn constant(rate: f64) -> Speed {
        Speed {
            r0: rate,
            r1: rate,
            m0: 0.0,
            m1: 0.0,
        }
    }

    /// For a play of `mods` at base `rate` whose notes run from `first` to `last` (map ms).
    #[inline]
    pub fn for_play(mods: Mods, rate: f64, first: f64, last: f64) -> Speed {
        match mods.wind() {
            Some(f) if last > first => Speed {
                r0: rate,
                r1: rate * f,
                m0: first,
                m1: last,
            },
            _ => Speed::constant(rate),
        }
    }

    pub fn is_constant(&self) -> bool {
        self.r0 == self.r1 || self.m1 <= self.m0
    }

    fn slope(&self) -> f64 {
        (self.r1 - self.r0) / (self.m1 - self.m0)
    }

    /// Map ms per song ms at map time `m`.
    pub fn rate_at(&self, m: f64) -> f64 {
        if self.is_constant() || m <= self.m0 {
            self.r0
        } else if m >= self.m1 {
            self.r1
        } else {
            self.r0 + self.slope() * (m - self.m0)
        }
    }

    /// Song ms at map ms `m`.
    pub fn track(&self, m: f64) -> f64 {
        if self.is_constant() || m <= self.m0 {
            return m / self.r0;
        }
        let ramp_end = m.min(self.m1);
        // Within the ramp the speed is linear in map time: ∫ dm / r(m) = ln(r(m) / r0) / slope.
        let ramp = (self.rate_at(ramp_end) / self.r0).ln() / self.slope();
        self.m0 / self.r0 + ramp + (m - ramp_end).max(0.0) / self.r1
    }

    /// Map ms at song ms `t` (the inverse of `track`).
    #[inline]
    pub fn map(&self, t: f64) -> f64 {
        let t0 = self.m0 / self.r0;
        if self.is_constant() || t <= t0 {
            return t * self.r0;
        }
        let t1 = self.track(self.m1);
        if t >= t1 {
            return self.m1 + (t - t1) * self.r1;
        }
        let s = self.slope();
        self.m0 + self.r0 * ((s * (t - t0)).exp() - 1.0) / s
    }
}

/// How many mod presets the player can keep (1-9 apply them in the mod select).
pub const MAX_PRESETS: usize = 9;

/// A saved set of mods and the song speed to go with it (the mod select's PRESETS row).
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    pub mods: Mods,
    pub rate: f64,
}

impl Preset {
    /// What a preset is called until the player names it: its mods and speed, "HDHR 1.20x".
    pub fn auto_name(mods: Mods, rate: f64) -> String {
        let speed = ((rate - 1.0).abs() > 1e-6).then(|| format!("{rate:.2}x"));
        let label = Some(mods.label()).filter(|l| !l.is_empty());
        match [label, speed].into_iter().flatten().collect::<Vec<_>>() {
            parts if parts.is_empty() => "No mod".to_string(),
            parts => parts.join(" "),
        }
    }

    /// The config's "1.2|HD,HR|name" (the name last: it may hold anything).
    pub fn parse(s: &str) -> Option<Preset> {
        let mut f = s.splitn(3, '|');
        let rate: f64 = f.next()?.trim().parse().ok()?;
        let mods = Mods::parse(f.next()?);
        let name = f.next()?.trim().to_string();
        (rate.is_finite() && !name.is_empty()).then_some(Preset { name, mods, rate })
    }

    pub fn format(&self) -> String {
        format!("{}|{}|{}", self.rate, self.mods.to_config(), self.name)
    }

    /// What the mod select shows for it: its mods, and the speed when it isn't 1x.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self
            .mods
            .iter()
            .map(|m| format!("{} {}", m.acronym(), m.name()))
            .collect();
        if parts.is_empty() {
            parts.push("no mods".into());
        }
        if (self.rate - 1.0).abs() > 1e-6 {
            parts.push(format!("speed {:.2}x", self.rate));
        }
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    /// A mod's bit is its place in `ALL` (saved presets and scores keep these bits), and every
    /// mod has a bit of its own.
    #[test]
    fn each_mod_s_bit_is_its_place_in_the_list() {
        use super::{ALL, Mods};
        let mut seen = 0u128;
        for (i, &m) in ALL.iter().enumerate() {
            assert_eq!(m.bit(), 1 << i, "{m:?}");
            assert_eq!(seen & m.bit(), 0, "{m:?} shares a bit");
            seen |= m.bit();
            assert!(Mods::default().flip(m).has(m), "{m:?}");
        }
    }

    /// The numbers (bits) of the mods, in order: what presets, favourites and plays saved.
    #[test]
    fn mod_numbers_never_change() {
        let by_number: Vec<&str> = super::ALL.iter().map(|m| m.acronym()).collect();
        assert_eq!(
            by_number.concat(),
            "EZNFHTDCHANBHRSDPFDTNCHDFLPRGNRXATSWRDCSALWUWDABACBRWVPUSTCHCCDRGBRLASHSEVSLSGLFPNMCLGMWSRSUBLBOSPCLMMDYPYOTLIRAECZZUTSILPEBSSSKMZMRTLSNMNBAFFXFSHCKCRPTMVTCOV"
        );
    }

    #[test]
    fn presets_keep_their_mods_speed_and_name_through_the_config() {
        use super::{Mods, Preset};
        let p = Preset {
            name: "85 acc | fail".into(),
            mods: Mods::parse("HD,HR,PF"),
            rate: 1.24,
        };
        assert_eq!(Preset::parse(&p.format()), Some(p.clone()));
        assert_eq!(
            Preset::auto_name(p.mods, p.rate),
            format!("{} 1.24x", p.mods.label())
        );
        assert_eq!(Preset::auto_name(Mods::NONE, 1.0), "No mod");
        assert_eq!(Preset::auto_name(Mods::NONE, 0.9), "0.90x");
        assert!(p.summary().contains("speed 1.24x"));
        assert_eq!(Preset::parse("x|HD|a"), None);
        assert_eq!(Preset::parse("1|HD|  "), None);
    }

    use super::*;

    /// Half of notes at these divisions of a 500 ms beat: the numbers of the ones it keeps.
    fn kept(beats: &[f64]) -> Vec<f64> {
        let times: Vec<f64> = beats.iter().map(|b| (b * 500.0_f64).round()).collect();
        let keep = half_keeps(&times, HalfRule::OnTheBeat, &|_| (0.0, 500.0));
        beats
            .iter()
            .zip(keep)
            .filter(|(_, k)| *k)
            .map(|(b, _)| *b)
            .collect()
    }

    #[test]
    fn half_takes_the_finest_division_off_a_stream() {
        let quarters: Vec<f64> = (0..=8).map(|k| k as f64 / 4.0).collect();
        assert_eq!(kept(&quarters), [0.0, 0.5, 1.0, 1.5, 2.0]);
        let halves: Vec<f64> = (0..=4).map(|k| k as f64 / 2.0).collect();
        assert_eq!(kept(&halves), [0.0, 1.0, 2.0]);
        let sixths: Vec<f64> = (0..=6).map(|k| k as f64 / 6.0).collect();
        let thirds: Vec<f64> = (0..=3).map(|k| k as f64 * 2.0 / 6.0).collect();
        assert_eq!(kept(&sixths), thirds);
        // A triplet goes to the beat.
        assert_eq!(kept(&[0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]), [0.0, 1.0]);
        // Whatever the length of the stream, the beat stays.
        let odd: Vec<f64> = (0..=4)
            .map(|k| 1.0 + k as f64 / 4.0)
            .chain([3.0, 3.25, 3.5])
            .collect();
        assert_eq!(kept(&odd), [1.0, 1.5, 2.0, 3.0, 3.5]);
    }

    #[test]
    fn half_leaves_notes_a_beat_apart_alone() {
        assert_eq!(kept(&[0.0, 1.0, 2.0, 3.0, 5.0]), [0.0, 1.0, 2.0, 3.0, 5.0]);
        // A stream off the beat, all on one division, keeps every other one.
        assert_eq!(kept(&[0.25, 0.75, 1.25, 1.75]), [0.25, 1.25]);
    }

    #[test]
    fn half_thins_a_stream_with_no_grid_to_go_by() {
        // A made-up BPM: 1 ms apart, 30 ms apart a beat of 30 ms.
        let times: Vec<f64> = (0..100).map(|k| k as f64).collect();
        let keep = half_keeps(&times, HalfRule::OnTheBeat, &|_| (0.0, 60.0));
        assert_eq!(keep.iter().filter(|k| **k).count(), 50);
        let times: Vec<f64> = (0..100).map(|k| k as f64 * 30.0).collect();
        let keep = half_keeps(&times, HalfRule::OnTheBeat, &|_| (0.0, 30.0));
        assert_eq!(keep.iter().filter(|k| **k).count(), 50);
    }

    #[test]
    fn old_plays_keep_every_other_note() {
        let times = [0.0, 100.0, 300.0, 500.0, 700.0];
        assert_eq!(
            half_keeps(&times, HalfRule::EveryOther, &|_| (0.0, 500.0)),
            [true, false, true, false, true]
        );
        assert_eq!(HalfRule::of_pp_version(1), HalfRule::EveryOther);
        // That the server's pp version cuts on the beat: the client's mods_tests.
    }

    #[test]
    fn lobbies_share_the_speed_and_leave_the_rest_to_each_player() {
        let host = Mods::parse("DT,HD,HR");
        assert_eq!(host.shared(), Mods::parse("DT"));
        assert_eq!(host.free(), Mods::parse("HD,HR"));
        // A player's game: the host's speed, their own others (not the host's).
        let mine = Mods::parse("HT,EZ,FL");
        assert_eq!(host.shared().plus(mine.free()), Mods::parse("DT,EZ,FL"));
        assert!(
            Mod::WindUp.shared()
                && Mod::Nightcore.shared()
                && !Mod::Hidden.shared()
                && !Mod::Random.shared()
        );
    }

    #[test]
    fn acronyms_are_two_letters_for_a_label_to_be_read_back() {
        for m in ALL {
            assert_eq!(m.acronym().len(), 2, "{}", m.acronym());
        }
    }

    #[test]
    fn only_autoplay_keeps_a_play_off_the_boards() {
        assert!(Mods::parse("AT").unranked());
        assert!(Mods::parse("OV").unranked());
        assert!(!Mods::parse("RX").unranked());
        assert!(Mods::parse("RX").relax());
        assert!(!Mods::parse("HD").relax());
    }

    #[test]
    fn multipliers_follow_how_hard_a_mod_is() {
        for m in ALL {
            let x = m.multiplier();
            let ok = match m.shuffle_cost() {
                Some(c) if c < 0 => x < 1.0,
                Some(0) => x <= 1.0,
                Some(1) => (1.0..=1.1).contains(&x),
                Some(2) => (1.04..=1.12).contains(&x),
                Some(_) => x >= 1.1,
                None => true,
            };
            assert!(
                ok,
                "{}: x{x} for a shuffle cost of {:?}",
                m.name(),
                m.shuffle_cost()
            );
        }
        // A heavy custom set adds up, but stays in reach of osu!'s HDHRDTFL (x1.42).
        let set = Mods::parse("CH,EV,SS,SU,ST");
        assert!(
            (1.3..1.5).contains(&set.multiplier()),
            "{}",
            set.multiplier()
        );
    }

    #[test]
    fn favourites_keep_conflicting_mods_and_round_trip() {
        // Easy and Hard Rock conflict, but both can be favourites.
        let favs = Mods::NONE
            .flip(Mod::Easy)
            .flip(Mod::HardRock)
            .flip(Mod::Hidden);
        assert!(favs.has(Mod::Easy) && favs.has(Mod::HardRock));
        assert_eq!(Mods::parse_set(&favs.to_config()), favs);
        assert!(!favs.flip(Mod::Hidden).has(Mod::Hidden));
        assert_eq!(customizable(Mods::parse("SS")), [Mod::Shapeshift]);
        assert!(customizable(Mods::parse("HD")).is_empty());
    }

    #[test]
    fn acronyms_are_unique_and_parse_back() {
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert_ne!(a.acronym(), b.acronym());
            }
            assert_eq!(Mod::from_acronym(a.acronym()), Some(*a));
        }
        let m = Mods::parse("hd, HR,xx");
        assert!(m.has(Hidden) && m.has(HardRock));
        assert_eq!(Mods::parse(&m.to_config()), m);
        assert!(GROUPS.iter().map(|g| g.2.len()).sum::<usize>() == ALL.len());
    }

    #[test]
    fn conflicts_go_both_ways_and_descriptions_fit_the_mod_select() {
        for m in ALL {
            for &c in m.conflicts() {
                assert!(
                    c == m || c.conflicts().contains(&m),
                    "{} -> {}",
                    m.acronym(),
                    c.acronym()
                );
            }
            // That its description fits the mod select: the client's mods_tests.
        }
    }

    #[test]
    fn incompatible_hints_match_what_toggling_removes() {
        for active in ALL {
            let selected = Mods::NONE.flip(active);
            assert!(!selected.incompatible(active));
            for candidate in ALL {
                if candidate == active {
                    continue;
                }
                let mut next = selected;
                next.toggle(candidate);
                assert_eq!(
                    selected.incompatible(candidate),
                    !next.has(active),
                    "{} -> {}",
                    active.acronym(),
                    candidate.acronym()
                );
                assert!(next.has(candidate));
            }
        }
        assert_eq!(customizable(Mods::parse("RD")), [Mod::Random]);
    }

    #[test]
    fn conflicts_switch_each_other_off() {
        let mut m = Mods::parse("EZ,DT");
        m.toggle(HardRock);
        assert!(m.has(HardRock) && !m.has(Easy));
        m.toggle(HalfTime);
        assert!(m.has(HalfTime) && !m.has(DoubleTime));
        assert_eq!(m.speed(1.2, true), (0.75, true));
        // DT plays at 1.5x whatever speed was picked: the two never multiply (nor their pp).
        assert_eq!(Mods::parse("DT").speed(2.0, true), (1.5, true));
        assert_eq!(Mods::parse("NC").speed(1.2, true), (1.5, false));
        m.toggle(WindUp);
        assert!(!m.has(HalfTime) && m.wind() == Some(1.5));
        assert!((Mods::parse("HD,HR").multiplier() - 1.06 * 1.06).abs() < 1e-9);
    }

    #[test]
    fn shuffles_stay_playable() {
        let mut seen = Mods::NONE;
        for seed in 0..5000u64 {
            let m = Mods::shuffle(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let picked: Vec<Mod> = m.iter().filter(|&x| x != NoFail).collect();
            assert!((2..=5).contains(&picked.len()), "{}", m.to_config());
            let cost: i32 = picked.iter().map(|x| x.shuffle_cost().unwrap()).sum();
            assert!(cost <= 6, "{}", m.to_config());
            assert_eq!(m.has(NoFail), cost >= 5, "{}", m.to_config());
            assert!(
                picked.iter().filter(|x| x.obscures()).count() <= 1,
                "{}",
                m.to_config()
            );
            assert!(
                picked.iter().filter(|x| x.moves_notes()).count() <= 3,
                "{}",
                m.to_config()
            );
            for g in GROUPS {
                assert!(
                    picked.iter().filter(|x| g.2.contains(x)).count() <= 1,
                    "{}",
                    m.to_config()
                );
            }
            for x in m.iter() {
                assert!(
                    !x.conflicts().iter().any(|&c| c != x && m.has(c)),
                    "{}",
                    m.to_config()
                );
            }
            seen = Mods(seen.0 | m.0);
        }
        // Every mod that can come up does, and the rest never do.
        for x in ALL {
            assert_eq!(seen.has(x), x.shuffle_cost().is_some(), "{}", x.name());
        }
    }

    #[test]
    fn speed_map_is_invertible_and_ramps() {
        let s = Speed::for_play(Mods::parse("WU"), 1.0, 1000.0, 61_000.0);
        assert_eq!(s.rate_at(0.0), 1.0);
        assert!((s.rate_at(31_000.0) - 1.25).abs() < 1e-9);
        assert_eq!(s.rate_at(90_000.0), 1.5);
        for m in [-2000.0, 0.0, 999.0, 5000.0, 30_000.0, 60_999.0, 80_000.0] {
            assert!((s.map(s.track(m)) - m).abs() < 1e-6, "map(track({m}))");
        }
        // Song time is shorter than map time once it speeds up; a 60 s ramp 1.0 -> 1.5 takes
        // 60 / 0.5 * ln 1.5 = 48.66 s.
        assert!((s.track(61_000.0) - s.track(1000.0) - 48_655.8).abs() < 1.0);
        let c = Speed::constant(1.5);
        assert_eq!(c.track(1500.0), 1000.0);
        assert_eq!(c.map(1000.0), 1500.0);
    }

    /// The website's mod select (server/site/mods.json) is this one; a change here rewrites it,
    /// and the test fails once so the new file gets committed.
    #[test]
    fn website_mod_list_is_current() {
        let groups: Vec<serde_json::Value> = GROUPS
            .iter()
            .map(|(title, colour, mods)| {
                let mods: Vec<serde_json::Value> = mods
                    .iter()
                    .map(|m| serde_json::json!({ "acronym": m.acronym(), "name": m.name(), "description": m.description() }))
                    .collect();
                serde_json::json!({ "title": title, "colour": format!("#{colour:06x}"), "mods": mods })
            })
            .collect();
        let text = serde_json::to_string_pretty(&groups).unwrap() + "\n";
        // The crate sits in crates/taiko-logic; the site is the repository's.
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server/site/mods.json");
        // Line endings aside: a Windows checkout may have turned them into CRLF.
        if std::fs::read_to_string(&path)
            .ok()
            .map(|t| t.replace("\r\n", "\n"))
            .as_deref()
            != Some(text.as_str())
        {
            std::fs::write(&path, &text).unwrap();
            panic!(
                "{} was out of date and has been rewritten: commit it",
                path.display()
            );
        }
    }

    #[test]
    fn a_lobby_plays_the_hosts_speed_and_in_defense_all_of_their_mods_but_banned_ones() {
        use crate::defense::Defense;
        let host = Mods::parse("DT,HD,HR,AT,SD,ZZ");
        assert_eq!(host.lobby(None), Mods::parse("DT"), "ordinary: the speed's");
        let lane = host.lobby(Some(Defense::DEFAULT));
        for m in [Mod::DoubleTime, Mod::Hidden, Mod::HardRock, Mod::Maze] {
            assert!(lane.has(m), "{} stays", m.acronym());
        }
        for m in [Mod::Autoplay, Mod::SuddenDeath, Mod::Zigzag] {
            assert!(
                !lane.has(m),
                "{} goes (a banned mod, or a rail Maze replaces)",
                m.acronym()
            );
        }
        let own = host.lobby(Some(Defense {
            maze: false,
            ..Defense::DEFAULT
        }));
        assert!(
            own.has(Mod::Zigzag) && !own.has(Mod::Maze),
            "without the Maze lane the rail stays"
        );
        let banned: Vec<Mod> = ALL.into_iter().filter(|m| !m.defense_ok()).collect();
        assert_eq!(
            banned,
            [
                Mod::NoFail,
                Mod::SuddenDeath,
                Mod::Perfect,
                Mod::Relax,
                Mod::Autoplay,
                Mod::OdOverride
            ]
        );
        assert_eq!(
            lane.0 & DEFENSE_BANNED,
            0,
            "a Defense lobby drops the banned mods"
        );
    }

    #[test]
    fn od_override_replaces_the_od_and_stays_out_of_lobbies() {
        let m = Mods::parse("OV");
        assert_eq!(m.od_with(3.0, 17.5), 17.5);
        // Over EZ, HR and Precise, and past the 10 they stop at.
        assert_eq!(Mods::parse("OV,HR,PR").od_with(3.0, 17.5), 17.5);
        assert_eq!(Mods::parse("OV,EZ").od_with(3.0, 17.5), 17.5);
        assert!(!m.free().has(Mod::OdOverride) && !m.shared().has(Mod::OdOverride));
        assert_eq!(Mods::parse("OV,HD").free(), Mods::parse("HD"));
        assert!(!Mod::OdOverride.defense_ok());
        assert_eq!(m.multiplier(), 1.0);
        // A shuffle mustn't make a play unsaved by chance.
        assert!((0..5000).all(|seed| !Mods::shuffle(seed).has(Mod::OdOverride)));
        assert!(customizable(m).contains(&Mod::OdOverride));
        // Last in the list: every other mod keeps its bit.
        assert_eq!(ALL[ALL.len() - 1], Mod::OdOverride);
        assert_eq!(ALL[ALL.len() - 2], Mod::Tact);
    }
}
