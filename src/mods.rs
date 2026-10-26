//! Mods: osu!'s (EZ, HR, HD, FL, DT...), lazer's taiko extras and taiko.rs's own (scroll speed
//! tricks, SV experiments, visual tricks, note paths, playfield moves and copies). A set of them is a bit mask; each has a two-letter acronym used in
//! the config, replays and multiplayer.

/// Every mod, in the order the mod select shows them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mod {
    // Difficulty reduction.
    Easy,
    NoFail,
    HalfTime,
    Daycore,
    Half,
    NoFinishers,
    // Difficulty increase.
    HardRock,
    SuddenDeath,
    Perfect,
    DoubleTime,
    Nightcore,
    Hidden,
    Flashlight,
    Precise,
    GreatOrNothing,
    // Automation and conversion.
    Relax,
    Autoplay,
    Swap,
    Random,
    ConstantSpeed,
    Alternate,
    WindUp,
    WindDown,
    AllBig,
    // Scroll speed tricks.
    Accel,
    Brake,
    Wave,
    Pulse,
    Stutter,
    SvChaos,
    Crescendo,
    Drop,
    Gearbox,
    Roulette,
    // SV experiments.
    AntiSv,
    HyperSv,
    Even,
    Split,
    Swing,
    Leapfrog,
    Panic,
    Mercy,
    LagSpike,
    Moonwalk,
    SnapSpeed,
    // Visual tricks.
    Sudden,
    Blink,
    Blackout,
    Spotlight,
    Colorless,
    Memory,
    Decoy,
    Party,
    Overtime,
    Liar,
    Radar,
    Echo,
    // Rails: the lane bent into a shape.
    Zigzag,
    UTurn,
    Spiral,
    Loop,
    Elbow,
    Shapeshift,
    Snake,
    // Playfield moves.
    Mirror,
    Tilt,
    Spin,
    Mania,
    BarrelRoll,
    FlipFlop,
    Crossfire,
    Shatter,
    Cracked,
    Crumble,
    Portal,
    Multiverse,
}

use Mod::*;

pub const ALL: [Mod; 76] = [
    Easy, NoFail, HalfTime, Daycore, Half, NoFinishers, HardRock, SuddenDeath, Perfect, DoubleTime, Nightcore, Hidden,
    Flashlight, Precise, GreatOrNothing, Relax, Autoplay, Swap, Random, ConstantSpeed, Alternate, WindUp, WindDown, AllBig,
    Accel, Brake, Wave, Pulse, Stutter, SvChaos, Crescendo, Drop, Gearbox, Roulette, AntiSv, HyperSv, Even, Split, Swing,
    Leapfrog, Panic, Mercy, LagSpike, Moonwalk, SnapSpeed, Sudden, Blink, Blackout, Spotlight, Colorless, Memory, Decoy,
    Party, Overtime, Liar, Radar, Echo, Zigzag, UTurn, Spiral, Loop, Elbow, Shapeshift, Snake, Mirror, Tilt, Spin, Mania, BarrelRoll, FlipFlop,
    Crossfire, Shatter, Cracked, Crumble, Portal, Multiverse,
];

/// Mod select columns: (title, accent colour, mods).
pub const GROUPS: [(&str, u32, &[Mod]); 8] = [
    ("REDUCTION", 0x86E05A, &[Easy, NoFail, HalfTime, Daycore, Half, NoFinishers]),
    ("INCREASE", 0xEB452C, &[HardRock, SuddenDeath, Perfect, DoubleTime, Nightcore, Hidden, Flashlight, Precise, GreatOrNothing]),
    ("CONVERSION", 0x4fb4ff, &[Relax, Autoplay, Swap, Random, ConstantSpeed, Alternate, WindUp, WindDown, AllBig]),
    ("SCROLL", 0xFFD34D, &[Accel, Brake, Wave, Pulse, Stutter, SvChaos, Crescendo, Drop, Gearbox, Roulette]),
    ("SV LAB", 0xFF8A3D, &[AntiSv, HyperSv, Even, Split, Swing, Leapfrog, Panic, Mercy, LagSpike, Moonwalk, SnapSpeed]),
    ("VISUAL", 0xc77dff, &[Sudden, Blink, Blackout, Spotlight, Colorless, Memory, Decoy, Party, Overtime, Liar, Radar, Echo]),
    ("RAILS", 0x5ee6c8, &[Zigzag, UTurn, Spiral, Loop, Elbow, Shapeshift, Snake, Crossfire]),
    ("PLAYFIELD", 0xff5fa2, &[Mirror, Tilt, Spin, Mania, BarrelRoll, FlipFlop, Shatter, Cracked, Crumble, Portal, Multiverse]),
];

/// The mods with settings of their own, in the order of the mod customization's tabs.
const CUSTOMIZABLE: [Mod; 2] = [Mod::Multiverse, Mod::Shapeshift];

/// The tabs of the mod customization: the mods with settings that are on (only those show).
pub fn customizable(mods: Mods) -> Vec<Mod> {
    CUSTOMIZABLE.into_iter().filter(|&m| mods.has(m)).collect()
}

/// Mod select keyboard navigation: the mod `dx` columns and `dy` rows from `index` (into `ALL`).
pub fn step(index: usize, dx: i32, dy: i32) -> usize {
    let m = ALL[index.min(ALL.len() - 1)];
    let (col, row) = GROUPS.iter().enumerate().find_map(|(c, g)| g.2.iter().position(|&x| x == m).map(|r| (c, r))).unwrap_or((0, 0));
    let col = (col as i32 + dx).rem_euclid(GROUPS.len() as i32) as usize;
    let list = GROUPS[col].2;
    let row = (row as i32 + dy).clamp(0, list.len() as i32 - 1) as usize;
    ALL.iter().position(|&x| x == list[row]).unwrap_or(0)
}

impl Mod {
    pub fn acronym(self) -> &'static str {
        match self {
            Easy => "EZ",
            NoFail => "NF",
            HalfTime => "HT",
            Daycore => "DC",
            Half => "HA",
            NoFinishers => "NB",
            HardRock => "HR",
            SuddenDeath => "SD",
            Perfect => "PF",
            DoubleTime => "DT",
            Nightcore => "NC",
            Hidden => "HD",
            Flashlight => "FL",
            Precise => "PR",
            GreatOrNothing => "GN",
            Relax => "RX",
            Autoplay => "AT",
            Swap => "SW",
            Random => "RD",
            ConstantSpeed => "CS",
            Alternate => "AL",
            WindUp => "WU",
            WindDown => "WD",
            AllBig => "AB",
            Accel => "AC",
            Brake => "BR",
            Wave => "WV",
            Pulse => "PU",
            Stutter => "ST",
            SvChaos => "CH",
            Crescendo => "CC",
            Drop => "DR",
            Gearbox => "GB",
            Roulette => "RL",
            AntiSv => "AS",
            HyperSv => "HS",
            Even => "EV",
            Split => "SL",
            Swing => "SG",
            Leapfrog => "LF",
            Panic => "PN",
            Mercy => "MC",
            LagSpike => "LG",
            Moonwalk => "MW",
            SnapSpeed => "SR",
            Sudden => "SU",
            Blink => "BL",
            Blackout => "BO",
            Spotlight => "SP",
            Colorless => "CL",
            Memory => "MM",
            Decoy => "DY",
            Party => "PY",
            Overtime => "OT",
            Liar => "LI",
            Radar => "RA",
            Echo => "EC",
            Zigzag => "ZZ",
            UTurn => "UT",
            Spiral => "SI",
            Loop => "LP",
            Elbow => "EB",
            Shapeshift => "SS",
            Snake => "SK",
            Mirror => "MR",
            Tilt => "TL",
            Spin => "SN",
            Mania => "MN",
            BarrelRoll => "BA",
            FlipFlop => "FF",
            Crossfire => "XF",
            Shatter => "SH",
            Cracked => "CK",
            Crumble => "CR",
            Portal => "PT",
            Multiverse => "MV",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Easy => "Easy",
            NoFail => "No Fail",
            HalfTime => "Half Time",
            Daycore => "Daycore",
            Half => "Half Notes",
            NoFinishers => "No Finishers",
            HardRock => "Hard Rock",
            SuddenDeath => "Sudden Death",
            Perfect => "Perfect",
            DoubleTime => "Double Time",
            Nightcore => "Nightcore",
            Hidden => "Hidden",
            Flashlight => "Flashlight",
            Precise => "Precise",
            GreatOrNothing => "Great or Nothing",
            Relax => "Relax",
            Autoplay => "Autoplay",
            Swap => "Swap",
            Random => "Random",
            ConstantSpeed => "Constant Speed",
            Alternate => "Alternate",
            WindUp => "Wind Up",
            WindDown => "Wind Down",
            AllBig => "All Big",
            Accel => "Accel",
            Brake => "Brake",
            Wave => "Wave",
            Pulse => "Pulse",
            Stutter => "Stutter",
            SvChaos => "SV Chaos",
            Crescendo => "Crescendo",
            Drop => "Drop",
            Gearbox => "Gearbox",
            Roulette => "Roulette",
            AntiSv => "Anti-SV",
            HyperSv => "Hyper SV",
            Even => "Even",
            Split => "Split",
            Swing => "Swing",
            Leapfrog => "Leapfrog",
            Panic => "Panic",
            Mercy => "Mercy",
            LagSpike => "Lag Spike",
            Moonwalk => "Moonwalk",
            SnapSpeed => "Snap Speed",
            Sudden => "Sudden",
            Blink => "Blink",
            Blackout => "Blackout",
            Spotlight => "Spotlight",
            Colorless => "Colorless",
            Memory => "Memory",
            Decoy => "Decoy",
            Party => "Party",
            Overtime => "Overtime",
            Liar => "Liar",
            Radar => "Radar",
            Echo => "Echo",
            Zigzag => "Zigzag",
            UTurn => "U-Turn",
            Spiral => "Spiral",
            Loop => "Loop",
            Elbow => "Elbow",
            Shapeshift => "Shapeshift",
            Snake => "Snake",
            Mirror => "Mirror",
            Tilt => "Tilt",
            Spin => "Spin",
            Mania => "Mania",
            BarrelRoll => "Barrel Roll",
            FlipFlop => "Flip Flop",
            Crossfire => "Crossfire",
            Shatter => "Shatter",
            Cracked => "Cracked",
            Crumble => "Crumble",
            Portal => "Portal",
            Multiverse => "Multiverse",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Easy => "Wider timing windows (OD x0.5), slower notes, a kinder HP bar.",
            NoFail => "You can't fail, whatever the HP bar says.",
            HalfTime => "0.75x speed, same pitch.",
            Daycore => "0.75x speed, lower pitch.",
            Half => "Every other don/kat is gone. For learning hard maps.",
            NoFinishers => "Big notes become normal ones.",
            HardRock => "Tighter windows (OD x1.4), much faster notes, a stricter HP bar.",
            SuddenDeath => "The first miss fails the play.",
            Perfect => "Anything but a GREAT fails the play.",
            DoubleTime => "1.5x speed, same pitch.",
            Nightcore => "1.5x speed, higher pitch.",
            Hidden => "Notes fade out long before they reach the drum.",
            Flashlight => "Only a circle around the drum is lit; it shrinks at 100 and 200 combo.",
            Precise => "OD +2.",
            GreatOrNothing => "An OK counts as a miss (but doesn't fail you).",
            Relax => "Colours don't matter: any key hits any note.",
            Autoplay => "Watch the map play itself.",
            Swap => "Dons become kats and kats become dons.",
            Random => "Every note gets a random colour.",
            ConstantSpeed => "All notes scroll at one speed: no SV changes.",
            Alternate => "Hands must alternate; either may follow a finisher, drumroll or swell.",
            WindUp => "The song speeds up from 1.0x to 1.5x of your speed.",
            WindDown => "The song slows down from 1.0x to 0.75x of your speed.",
            AllBig => "Every don and kat is a big note.",
            Accel => "Notes start slow and rush in near the drum.",
            Brake => "Notes fly in fast and brake near the drum.",
            Wave => "Note speed swells and falls along the song.",
            Pulse => "Notes jump forward on every beat.",
            Stutter => "Notes freeze for half a beat, then catch up.",
            SvChaos => "Every note gets its own speed (+-30%): they overtake each other.",
            Crescendo => "Notes speed up steadily over the map, from 0.8x to 1.6x.",
            Drop => "Notes freeze two beats before every kiai, then slam in 1.4x faster.",
            Gearbox => "Every beat of a bar shifts up a gear (+15%); the next bar starts over.",
            Roulette => "Every 4 bars a random speed (0.5x-2x); a marker shows where it changes.",
            AntiSv => "The map's speed changes are turned upside down: 2x becomes 0.5x.",
            HyperSv => "The map's speed changes are exaggerated: 1.5x becomes 2.25x.",
            Even => "Notes are evenly spaced: bursts fly in fast, slow parts crawl. Play by ear.",
            Split => "Dons are 20% faster and kats 15% slower: colours overtake each other.",
            Swing => "Notes on the beat are faster, notes between beats slower.",
            Leapfrog => "Every other note is faster: pairs swap places on the way in.",
            Panic => "Every miss makes notes 15% faster for 4 seconds; misses stack.",
            Mercy => "Every miss slows notes down for a few seconds.",
            LagSpike => "The lane freezes now and then, then skips ahead. Like a bad stream.",
            Moonwalk => "On every bar the notes slide backwards for a moment.",
            SnapSpeed => "Speed by rhythm: 1/3s slower, 1/6s faster, 1/8s a bit faster than the rest.",
            Sudden => "Notes only show up close to the drum.",
            Blink => "Approaching notes vanish on the second half of every beat.",
            Blackout => "Kiai time turns the lights off: only the drum stays lit.",
            Spotlight => "Flashlight whose circle breathes with the beat.",
            Colorless => "Dons and kats look the same. Play by ear and memory.",
            Memory => "Notes are only shown for the first 15 seconds.",
            Decoy => "See-through fake notes fly in between the real ones and vanish before the drum.",
            Overtime => "It's me.",
            Liar => "Far off, triplets are spaced like 1/4s; their real spacing shows near the drum.",
            Radar => "The lane is dark: a beam sweeps it every beat and lights notes up as it passes.",
            Echo => "Notes are grey; the next bar shows in colour under the lane, a cursor running along it.",
            Party => "Everything flashy at once; notes pulse to their own rhythm, 1/4s four times a beat.",
            Zigzag => "The lane zigzags across the screen.",
            UTurn => "Notes come in along the top, make a U-turn and come back to the drum.",
            Spiral => "The lane coils into a spiral; notes wind their way in.",
            Loop => "The lane has a loop-the-loop in the middle.",
            Elbow => "Notes fall from the top and turn into the lane at a corner.",
            Shapeshift => "Every few bars the lane bends into another rails shape (C: how often, which order).",
            Snake => "Streams slither along the lane, triplets the other way; 1/1 notes fly straight.",
            Mirror => "The lane is flipped: notes come from the left.",
            Tilt => "The lane rocks from side to side with the beat.",
            Spin => "The lane moves to the middle and turns full circle every 24 seconds.",
            Mania => "The lane stands up: notes fall down onto the drum.",
            BarrelRoll => "Every kiai starts with a full spin of the lane.",
            FlipFlop => "Every 8 bars the lane flips over to the other side.",
            Crossfire => "The drum sits in the middle: kats come from the left, dons from the right.",
            Shatter => "The lane breaks into pieces that slide apart and back; further where streams are.",
            Cracked => "Every miss cracks the lane and knocks a piece out of line; 50 hits mend it.",
            Crumble => "Hits knock pieces out of the lane, harder as the combo grows; a miss puts them back.",
            Portal => "Notes dive into one portal and come out of another; they move every 8 bars.",
            Multiverse => "Eight playfields circle about, each with its own look and mods (C sets them up).",
        }
    }

    /// Score multiplier: osu!'s for its mods, and on the same scale for taiko.rs's own by how
    /// much harder they make a map to read or play (HR's 1.06 is "clearly harder", DT's 1.12
    /// "a lot"). Mods that help score below 1; ones that only look different stay at 1.
    pub fn multiplier(self) -> f64 {
        match self {
            Easy | NoFail => 0.5,
            HalfTime | Daycore => 0.3,
            Half => 0.5,
            NoFinishers | ConstantSpeed | Mercy => 0.9,
            WindDown => 0.85,
            HardRock | Hidden | Precise => 1.06,
            DoubleTime | Nightcore | Flashlight | Multiverse => 1.12,
            Spotlight | Blackout | Memory | Colorless => 1.1,
            // Hard to read or to play: notes that lie about where or when they'll arrive.
            SvChaos | Even => 1.08,
            Sudden | Blink | Decoy | Liar | Radar | Echo | Alternate | WindUp | Stutter | Roulette | HyperSv | Split | Leapfrog | LagSpike | Moonwalk => 1.06,
            Spiral | Shapeshift | Spin | Portal => 1.06,
            // Noticeably harder.
            GreatOrNothing | Random | Accel | Brake | Pulse | Crescendo | Drop | Gearbox | AntiSv | Swing | Panic | SnapSpeed => 1.04,
            UTurn | Loop | Snake | FlipFlop | Crossfire | Shatter | Crumble => 1.04,
            // A little harder: the lane moves or bends, the notes read as usual.
            AllBig | Wave | Zigzag | Elbow | Mirror | Tilt | Mania | BarrelRoll | Cracked | Party => 1.02,
            _ => 1.0,
        }
    }

    /// Mods this one can't be combined with.
    fn conflicts(self) -> &'static [Mod] {
        match self {
            Easy => &[HardRock],
            HardRock => &[Easy],
            NoFail => &[SuddenDeath, Perfect, Relax, Autoplay],
            SuddenDeath => &[NoFail, Perfect, Relax, Autoplay],
            Perfect => &[NoFail, SuddenDeath, Relax, Autoplay],
            HalfTime | Daycore | DoubleTime | Nightcore => &[HalfTime, Daycore, DoubleTime, Nightcore, WindUp, WindDown],
            WindUp | WindDown => &[HalfTime, Daycore, DoubleTime, Nightcore, WindUp, WindDown],
            Relax => &[Autoplay, NoFail, SuddenDeath, Perfect, Alternate],
            Autoplay => &[Relax, NoFail, SuddenDeath, Perfect, Alternate],
            Alternate => &[Relax, Autoplay],
            Swap => &[Random],
            Random => &[Swap],
            NoFinishers => &[AllBig],
            AllBig => &[NoFinishers],
            Accel => &[Brake],
            Brake => &[Accel],
            Pulse => &[Stutter],
            Stutter => &[Pulse],
            Hidden => &[Sudden, Memory, Radar],
            Sudden => &[Hidden, Memory, Radar],
            Memory => &[Hidden, Sudden, Radar],
            Flashlight => &[Spotlight, Multiverse, Radar],
            Spotlight => &[Flashlight, Multiverse, Radar],
            // Radar decides when far notes show and lights the whole lane: nothing else may hide
            // them or keep the light to one spot. Its window-sized lanes are Multiverse's own.
            Radar => &[Hidden, Sudden, Memory, Flashlight, Spotlight, Blackout, Multiverse],
            // Echo's bar sits under the lane: nothing may move the lane over it. Colorless does
            // its grey already, and Multiverse's colours are what the windows go by.
            Echo => &[Colorless, Multiverse, Spin, Mania, Zigzag, UTurn, Spiral, Loop, Elbow, Shapeshift],
            // The map's own speeds are what these bend, so they don't mix with each other or
            // with one speed for everything.
            ConstantSpeed => &[AntiSv, HyperSv],
            AntiSv => &[HyperSv, ConstantSpeed],
            HyperSv => &[AntiSv, ConstantSpeed],
            Panic => &[Mercy],
            Mercy => &[Panic],
            Spin => &[Mania, Zigzag, UTurn, Spiral, Loop, Elbow, Shapeshift, Multiverse, Crossfire, Echo],
            Mania => &[Spin, Zigzag, UTurn, Spiral, Loop, Elbow, Shapeshift, Multiverse, Crossfire, Echo],
            Zigzag | UTurn | Spiral | Loop | Elbow | Shapeshift => &[Zigzag, UTurn, Spiral, Loop, Elbow, Shapeshift, Mania, Spin, Multiverse, Crossfire, Echo],
            // The drum in the middle, a lane each way: nothing else may move the drum, bend the
            // lane or break it into pieces.
            Crossfire => &[Spin, Mania, Zigzag, UTurn, Spiral, Loop, Elbow, Shapeshift, Multiverse, Shatter, Cracked, Crumble, Portal],
            Shatter | Portal => &[Crossfire],
            // Its playfields are small windows side by side: nothing may move them off their
            // spots, light only one of them or take its colours away.
            Tilt | BarrelRoll => &[Multiverse],
            Colorless => &[Multiverse, Echo],
            Blackout => &[Multiverse, Radar],
            Multiverse => &[
                Spin, Mania, Tilt, BarrelRoll, Zigzag, UTurn, Spiral, Loop, Elbow, Shapeshift, Colorless, Flashlight, Spotlight, Blackout, Crossfire, Radar, Echo,
            ],
            Cracked => &[Crumble, Crossfire],
            Crumble => &[Cracked, Crossfire],
            _ => &[],
        }
    }

    /// How much harder this mod makes a map to play, for the shuffle (`Mods::shuffle`): easier
    /// mods count against the rest. `None`: never shuffled in (nobody plays, the play just ends
    /// early, or there's nothing left to read).
    fn shuffle_cost(self) -> Option<i32> {
        Some(match self {
            Autoplay | Relax | SuddenDeath | Perfect | GreatOrNothing | Memory | Overtime => return None,
            Half => -3,
            Easy | HalfTime | Daycore => -2,
            NoFail | NoFinishers | ConstantSpeed | WindDown | Mercy => 0,
            DoubleTime | Nightcore | Flashlight | Spotlight | Colorless => 3,
            HardRock | Hidden | Sudden | Blink | Decoy | Radar | Echo | Random | Alternate | WindUp => 2,
            Stutter | SvChaos | Roulette | HyperSv | Even | Split | Leapfrog | Panic | LagSpike | Moonwalk => 2,
            Spiral | Shapeshift | Spin | FlipFlop | Portal | Multiverse => 2,
            _ => 1,
        })
    }

    /// Mods that hide or disguise notes: one of them at a time is plenty.
    fn obscures(self) -> bool {
        matches!(self, Hidden | Sudden | Flashlight | Spotlight | Blink | Colorless | Decoy | Liar | Radar | Echo | Multiverse)
    }

    /// Mods that change where and how fast notes travel.
    fn moves_notes(self) -> bool {
        GROUPS.iter().filter(|g| ["SCROLL", "SV LAB", "RAILS", "PLAYFIELD"].contains(&g.0)).any(|g| g.2.contains(&self))
    }

    fn bit(self) -> u128 {
        1 << ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    pub fn from_acronym(s: &str) -> Option<Mod> {
        ALL.iter().copied().find(|m| m.acronym().eq_ignore_ascii_case(s.trim()))
    }
}

/// A set of mods.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods(u128);

impl Mods {
    pub const NONE: Mods = Mods(0);

    pub fn has(self, m: Mod) -> bool {
        self.0 & m.bit() != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Mod> {
        ALL.into_iter().filter(move |&m| self.has(m))
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
        Mods(s.split([',', ' ']).filter_map(Mod::from_acronym).fold(0, |bits, m| bits | m.bit()))
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
                    let Some(c) = m.shuffle_cost() else { return false };
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

    /// OD after EZ / HR / Precise.
    pub fn od(self, od: f64) -> f64 {
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
    pub fn unranked(self) -> bool {
        self.has(Autoplay) || self.has(Relax)
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
        Speed { r0: rate, r1: rate, m0: 0.0, m1: 0.0 }
    }

    /// For a play of `mods` at base `rate` whose notes run from `first` to `last` (map ms).
    pub fn for_play(mods: Mods, rate: f64, first: f64, last: f64) -> Speed {
        match mods.wind() {
            Some(f) if last > first => Speed { r0: rate, r1: rate * f, m0: first, m1: last },
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

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(ok, "{}: x{x} for a shuffle cost of {:?}", m.name(), m.shuffle_cost());
        }
        // A heavy custom set adds up, but stays in reach of osu!'s HDHRDTFL (x1.42).
        let set = Mods::parse("CH,EV,SS,SU,ST");
        assert!((1.3..1.5).contains(&set.multiplier()), "{}", set.multiplier());
    }

    #[test]
    fn favourites_keep_conflicting_mods_and_round_trip() {
        // Easy and Hard Rock conflict, but both can be favourites.
        let favs = Mods::NONE.flip(Mod::Easy).flip(Mod::HardRock).flip(Mod::Hidden);
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
                assert!(c == m || c.conflicts().contains(&m), "{} -> {}", m.acronym(), c.acronym());
            }
            // The mod select's description line at 5:4 (600 wide).
            assert!(crate::render::Canvas::text_width(7.0, m.description()) <= 600.0 - 64.0, "{}", m.name());
        }
    }

    #[test]
    fn conflicts_switch_each_other_off() {
        let mut m = Mods::parse("EZ,DT");
        m.toggle(HardRock);
        assert!(m.has(HardRock) && !m.has(Easy));
        m.toggle(HalfTime);
        assert!(m.has(HalfTime) && !m.has(DoubleTime));
        assert_eq!(m.speed(1.2, true), (0.75, true));
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
            assert!(picked.iter().filter(|x| x.obscures()).count() <= 1, "{}", m.to_config());
            assert!(picked.iter().filter(|x| x.moves_notes()).count() <= 3, "{}", m.to_config());
            for g in GROUPS {
                assert!(picked.iter().filter(|x| g.2.contains(x)).count() <= 1, "{}", m.to_config());
            }
            for x in m.iter() {
                assert!(!x.conflicts().iter().any(|&c| c != x && m.has(c)), "{}", m.to_config());
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
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("server/site/mods.json");
        // Line endings aside: a Windows checkout may have turned them into CRLF.
        if std::fs::read_to_string(&path).ok().map(|t| t.replace("\r\n", "\n")).as_deref() != Some(text.as_str()) {
            std::fs::write(&path, &text).unwrap();
            panic!("{} was out of date and has been rewritten: commit it", path.display());
        }
    }
}
