//! Mods: osu!'s (EZ, HR, HD, FL, DT...), lazer's taiko extras and taiko-rs's own (scroll speed
//! tricks, visual tricks). A set of them is a bit mask; each has a two-letter acronym used in
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
    ComboRush,
    KiaiBoost,
    Finale,
    // Visual tricks.
    Sudden,
    Blink,
    Blackout,
    Spotlight,
    Colorless,
    Memory,
    Mirror,
    Drunk,
    Earthquake,
    Zoom,
}

use Mod::*;

pub const ALL: [Mod; 43] = [
    Easy, NoFail, HalfTime, Daycore, Half, NoFinishers, HardRock, SuddenDeath, Perfect, DoubleTime, Nightcore, Hidden,
    Flashlight, Precise, GreatOrNothing, Relax, Autoplay, Swap, Random, ConstantSpeed, Alternate, WindUp, WindDown, AllBig,
    Accel, Brake, Wave, Pulse, Stutter, SvChaos, ComboRush, KiaiBoost, Finale, Sudden, Blink, Blackout, Spotlight, Colorless,
    Memory, Mirror, Drunk, Earthquake, Zoom,
];

/// Mod select columns: (title, accent colour, mods).
pub const GROUPS: [(&str, u32, &[Mod]); 5] = [
    ("REDUCTION", 0x86E05A, &[Easy, NoFail, HalfTime, Daycore, Half, NoFinishers]),
    ("INCREASE", 0xEB452C, &[HardRock, SuddenDeath, Perfect, DoubleTime, Nightcore, Hidden, Flashlight, Precise, GreatOrNothing]),
    ("CONVERSION", 0x4fb4ff, &[Relax, Autoplay, Swap, Random, ConstantSpeed, Alternate, WindUp, WindDown, AllBig]),
    ("SCROLL", 0xFFD34D, &[Accel, Brake, Wave, Pulse, Stutter, SvChaos, ComboRush, KiaiBoost, Finale]),
    ("VISUAL", 0xc77dff, &[Sudden, Blink, Blackout, Spotlight, Colorless, Memory, Mirror, Drunk, Earthquake, Zoom]),
];

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
            ComboRush => "CR",
            KiaiBoost => "KB",
            Finale => "FN",
            Sudden => "SU",
            Blink => "BL",
            Blackout => "BO",
            Spotlight => "SP",
            Colorless => "CL",
            Memory => "MM",
            Mirror => "MR",
            Drunk => "DK",
            Earthquake => "EQ",
            Zoom => "ZM",
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
            ComboRush => "Combo Rush",
            KiaiBoost => "Kiai Boost",
            Finale => "Finale",
            Sudden => "Sudden",
            Blink => "Blink",
            Blackout => "Blackout",
            Spotlight => "Spotlight",
            Colorless => "Colorless",
            Memory => "Memory",
            Mirror => "Mirror",
            Drunk => "Drunk",
            Earthquake => "Earthquake",
            Zoom => "Zoom",
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
            Alternate => "The same hand can't hit twice in a row.",
            WindUp => "The song speeds up from 1.0x to 1.5x of your speed.",
            WindDown => "The song slows down from 1.0x to 0.75x of your speed.",
            AllBig => "Every don and kat is a big note.",
            Accel => "Notes start slow and rush in near the drum.",
            Brake => "Notes fly in fast and brake near the drum.",
            Wave => "Note speed swells and falls along the song.",
            Pulse => "Notes jump forward on every beat.",
            Stutter => "Notes freeze for half a beat, then catch up.",
            SvChaos => "Every note gets its own speed (+-30%): they overtake each other.",
            ComboRush => "+5% note speed every 50 combo, back to normal on a miss.",
            KiaiBoost => "Notes in kiai time are 1.3x faster.",
            Finale => "The last 15% of the map is 1.5x faster.",
            Sudden => "Notes only show up close to the drum.",
            Blink => "Approaching notes vanish on the second half of every beat.",
            Blackout => "Kiai time turns the lights off: only the drum stays lit.",
            Spotlight => "Flashlight whose circle breathes with the beat.",
            Colorless => "Dons and kats look the same. Play by ear and memory.",
            Memory => "Notes are only shown for the first 15 seconds.",
            Mirror => "The lane is flipped: notes come from the left.",
            Drunk => "The lane sways up and down.",
            Earthquake => "Big notes shake the lane.",
            Zoom => "The lane pulses bigger on every beat.",
        }
    }

    /// Score multiplier (osu!'s for its mods).
    pub fn multiplier(self) -> f64 {
        match self {
            Easy | NoFail => 0.5,
            HalfTime | Daycore => 0.3,
            Half => 0.5,
            NoFinishers => 0.9,
            HardRock | Hidden => 1.06,
            DoubleTime | Nightcore | Flashlight => 1.12,
            Precise => 1.06,
            ConstantSpeed => 0.9,
            Spotlight | Blackout | Memory | Colorless => 1.1,
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
            Hidden => &[Sudden, Memory],
            Sudden => &[Hidden, Memory],
            Memory => &[Hidden, Sudden],
            Flashlight => &[Spotlight],
            Spotlight => &[Flashlight],
            _ => &[],
        }
    }

    fn bit(self) -> u64 {
        1 << ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    pub fn from_acronym(s: &str) -> Option<Mod> {
        ALL.iter().copied().find(|m| m.acronym().eq_ignore_ascii_case(s.trim()))
    }
}

/// A set of mods.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods(u64);

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

    pub fn from_list(list: &[String]) -> Mods {
        Mods::parse(&list.join(","))
    }

    pub fn list(self) -> Vec<String> {
        self.iter().map(|m| m.acronym().to_string()).collect()
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
}
