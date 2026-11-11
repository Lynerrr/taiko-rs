//! Official osu!taiko star ratings and strain curves via rosu-pp, a precise port of osu!lazer's
//! difficulty calculator (converts included). Computed on a background worker and cached.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use rosu_pp::model::mode::GameMode;
use rosu_pp::taiko::Taiko;
use rosu_pp::{Beatmap, Difficulty};

/// Section length of lazer's strain skills (in rate-adjusted time).
const SECTION_MS: f64 = 400.0;

/// A map's .osu (or .tko) bytes, ready for rosu-pp: tiny sliders dropped, None if it can't
/// be rated.
pub fn load(map: &[u8]) -> Option<Beatmap> {
    let mut map = Beatmap::from_bytes(map).ok()?;
    drop_tiny_sliders(&mut map);
    calculable(&map).then_some(map)
}

/// The longest a slider can be written and still be a tiny one (`beatmap::Note::tiny`).
const TINY_SLIDER_LENGTH: f64 = 0.1;

/// Takes out the sliders written under 0.1 long (negative ones included, which rosu-pp reads
/// as no length): fake sliders, a gimmick map staple. In the game they're tiny rolls, with
/// nothing to hit, but rosu-pp counts each as an object of its own, a ms or so after the note
/// under it, and the strain of those gaps rated Kyoufuu All Back [Taiko All Back] (set
/// 1973536: 703 notes, 1,198 fake sliders) 9.92 stars instead of 4.29, 16.26 at 2x.
fn drop_tiny_sliders(map: &mut Beatmap) {
    use rosu_pp::model::hit_object::HitObjectKind;
    let tiny = |kind: &HitObjectKind| matches!(kind, HitObjectKind::Slider(s) if s.expected_dist.is_none_or(|d| d <= TINY_SLIDER_LENGTH));
    if !map.hit_objects.iter().any(|h| tiny(&h.kind)) {
        return;
    }
    let objects = std::mem::take(&mut map.hit_objects);
    let sounds = std::mem::take(&mut map.hit_sounds);
    for (object, sound) in objects.into_iter().zip(sounds) {
        if !tiny(&object.kind) {
            map.hit_objects.push(object);
            map.hit_sounds.push(sound);
        }
    }
}

/// Taiko maps up to this many objects are rated. rosu-pp turns away those over 30,000 as too
/// slow to work out, but marathons are far from it: Nanahira - Nana Party's 63,841 take ~50 ms.
const MAX_TAIKO_OBJECTS: usize = 200_000;

/// Whether a map can be rated: maps built to break calculators (absurd density, length or
/// object count) are skipped, as osu! would time out.
pub fn calculable(map: &Beatmap) -> bool {
    use rosu_pp::model::beatmap::TooSuspicious;
    match map.check_suspicion() {
        Ok(()) => true,
        // rosu-pp stops at the count, before the length and density checks: those still apply.
        Err(TooSuspicious::ObjectCount)
            if map.mode == GameMode::Taiko && map.hit_objects.len() <= MAX_TAIKO_OBJECTS =>
        {
            let times: Vec<f64> = map.hit_objects.iter().map(|h| h.start_time).collect();
            reasonable_taiko(&times)
        }
        Err(_) => false,
    }
}

/// rosu-pp's length and density checks for taiko (`TooSuspicious::new`): no more than a day long,
/// and no more than 400 objects within a second or 1,000 within ten.
fn reasonable_taiko(times: &[f64]) -> bool {
    const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
    const PER_1S: usize = 400;
    const PER_10S: usize = 1000;
    if let [first, .., last] = times
        && last - first > DAY_MS
    {
        return false;
    }
    let dense = |i: usize, n: usize, ms: f64| times.get(i + n).is_some_and(|t| t - times[i] < ms);
    !(0..times.len()).any(|i| dense(i, PER_1S, 1000.0) || dense(i, PER_10S, 10_000.0))
}

/// The map at `path` as a play with `mods` has it: Half takes notes off (by `half`), Swap and
/// Random change their colours (like `game::apply_to_map`), and pp and star ratings go by what's
/// left. `seed`: Random's colouring (`Game::seed`).
pub fn load_played(
    map: &[u8],
    mods: crate::mods::Mods,
    seed: u64,
    half_rule: crate::mods::HalfRule,
) -> Option<Beatmap> {
    use crate::mods::Mod;
    let mut map = load(map)?;
    let (half, swap, random) = (
        mods.has(Mod::Half),
        mods.has(Mod::Swap),
        mods.has(Mod::Random),
    );
    if !(half || swap || random) {
        return Some(map);
    }
    // A convert's notes only exist once it's converted.
    map.convert_mut(GameMode::Taiko, &Default::default()).ok()?;
    const RIM: u8 = 2 | 8; // Whistle and clap: kats.
    let keep = if half {
        let points = &map.timing_points;
        let beat_at = |t: f64| {
            let k = points.partition_point(|p| p.time <= t).saturating_sub(1);
            points.get(k).map_or((0.0, 500.0), |p| (p.time, p.beat_len))
        };
        let times: Vec<f64> = map
            .hit_objects
            .iter()
            .filter(|o| o.is_circle())
            .map(|o| o.start_time)
            .collect();
        crate::mods::half_keeps(&times, half_rule, &beat_at)
    } else {
        Vec::new()
    };
    let objects = std::mem::take(&mut map.hit_objects);
    let sounds = std::mem::take(&mut map.hit_sounds);
    let mut k = 0;
    for (object, sound) in objects.into_iter().zip(sounds) {
        let mut bits = u8::from(sound);
        if object.is_circle() {
            k += 1;
            if half && !keep[k - 1] {
                continue;
            }
            let kat = (bits & RIM != 0) != swap;
            let kat = if random {
                !crate::sim::random_don(object.start_time, seed)
            } else {
                kat
            };
            bits = if kat { bits | 8 } else { bits & !RIM };
        }
        map.hit_objects.push(object);
        map.hit_sounds.push(bits.into());
    }
    Some(map)
}

/// The star rating at 1x, no mods, of a map held as `text` (the editor's, saved or not).
pub fn star_rating_text(text: &str) -> Option<f64> {
    let mut map = Beatmap::from_bytes(text.as_bytes()).ok()?;
    drop_tiny_sliders(&mut map);
    if !calculable(&map) {
        return None;
    }
    Difficulty::new()
        .calculate_for_mode::<Taiko>(&map)
        .ok()
        .map(|a| a.stars)
}

pub fn star_rating(map: &[u8], rate: f64) -> Option<f64> {
    star_rating_with(map, rate, 0)
}

/// With mods (osu!'s bits, `mod_bits`): osu!taiko's star rating counts Hard Rock and Easy (their
/// scroll speed and hit windows) and Relax, besides the speed.
pub fn star_rating_with(map: &[u8], rate: f64, mods: u32) -> Option<f64> {
    let map = load(map)?;
    Difficulty::new()
        .mods(mods)
        .clock_rate(rate)
        .calculate_for_mode::<Taiko>(&map)
        .ok()
        .map(|a| a.stars)
}

/// With mods and the scroll speed setting, the way pp counts them (`pp::formula_mods`: slower
/// notes read easier, faster ones harder). Relax goes by its mod bits, scroll speed aside.
pub fn star_rating_for(
    map: &[u8],
    rate: f64,
    mods: crate::mods::Mods,
    scroll_speed: f64,
) -> Option<f64> {
    star_rating_played(map, rate, mods, scroll_speed, 0, Default::default())
}

/// The same for a play, whose `seed` says how Random coloured it.
pub fn star_rating_played(
    map: &[u8],
    rate: f64,
    mods: crate::mods::Mods,
    scroll_speed: f64,
    seed: u64,
    half_rule: crate::mods::HalfRule,
) -> Option<f64> {
    let map = load_played(map, mods, seed, half_rule)?;
    let difficulty = if mods.has(crate::mods::Mod::Relax) {
        Difficulty::new().mods(mod_bits(mods))
    } else {
        Difficulty::new().mods(crate::pp::formula_mods(mods, scroll_speed))
    };
    difficulty
        .clock_rate(rate)
        .calculate_for_mode::<Taiko>(&map)
        .ok()
        .map(|a| a.stars)
}

/// The mods of `mods` osu!taiko's star rating counts, as osu!'s mod bits (the speed is passed as
/// a rate).
pub fn mod_bits(mods: crate::mods::Mods) -> u32 {
    use crate::mods::Mod;
    [(Mod::Easy, 2), (Mod::HardRock, 16), (Mod::Relax, 128)]
        .into_iter()
        .filter(|(m, _)| mods.has(*m))
        .map(|(_, b)| b)
        .sum()
}

/// The mods that change the notes (`load_played`), as bits next to `mod_bits` in a rating's key.
fn note_bits(mods: crate::mods::Mods) -> u32 {
    use crate::mods::Mod;
    [
        (Mod::Half, 1 << 16),
        (Mod::Swap, 1 << 17),
        (Mod::Random, 1 << 18),
    ]
    .into_iter()
    .filter(|(m, _)| mods.has(*m))
    .map(|(_, b)| b)
    .sum()
}

/// Combined skill strain per section, the map time the first section starts at, and the
/// section length in map time — for the song select difficulty graph.
pub fn strain_curve(map: &[u8], rate: f64, first_object_ms: f64) -> Option<(Vec<f64>, f64, f64)> {
    let map = load(map)?;
    let s = Difficulty::new()
        .clock_rate(rate)
        .strains_for_mode::<Taiko>(&map)
        .ok()?;
    let at = |v: &Vec<f64>, i: usize| v.get(i).copied().unwrap_or(0.0);
    let n = s.color.len().max(s.stamina.len()).max(s.rhythm.len());
    let combined = (0..n)
        .map(|i| at(&s.color, i) + at(&s.rhythm, i) + at(&s.stamina, i) + at(&s.reading, i))
        .collect();
    // lazer's first section ends at the first multiple of the section length after the first
    // object (times divided by the clock rate).
    let first_end = (first_object_ms / rate / SECTION_MS).ceil() * SECTION_MS;
    Some((combined, (first_end - SECTION_MS) * rate, SECTION_MS * rate))
}

fn rate_key(rate: f64) -> u32 {
    (rate * 100.0).round() as u32
}

/// Library index, rate, mods (`mod_bits`, `note_bits`) and scroll speed (both in hundredths).
type Key = (usize, u32, u32, u32);

/// Background star rating calculator with a cache keyed by library index, rate, mods and
/// scroll speed.
pub struct Stars {
    tx: Sender<(Key, PathBuf, f64, crate::mods::Mods, f64)>,
    rx: Receiver<(Key, Option<f32>)>,
    cache: HashMap<Key, Option<f32>>,
    requested: HashSet<Key>,
    /// The mods and scroll speed ratings are for now.
    mods: crate::mods::Mods,
    scroll_speed: f64,
}

impl Stars {
    /// The worker reads the maps it is asked about with `read`.
    pub fn new(read: crate::ReadFile) -> Self {
        let (tx, jobs) = mpsc::channel::<(Key, PathBuf, f64, crate::mods::Mods, f64)>();
        let (done, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("stars".into())
            .spawn(move || {
                for (key, path, rate, mods, scroll_speed) in jobs {
                    let stars = std::panic::catch_unwind(|| {
                        read(&path).and_then(|map| star_rating_for(&map, rate, mods, scroll_speed))
                    })
                    .ok()
                    .flatten()
                    .map(|s| s as f32);
                    if done.send((key, stars)).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn star worker");
        Stars {
            tx,
            rx,
            cache: HashMap::new(),
            requested: HashSet::new(),
            mods: crate::mods::Mods::NONE,
            scroll_speed: 1.0,
        }
    }
}

impl Stars {
    /// Queues a calculation unless it is cached or already queued.
    /// Ratings from now on are with `mods` and at `scroll_speed`.
    pub fn set_mods(&mut self, mods: crate::mods::Mods, scroll_speed: f64) {
        (self.mods, self.scroll_speed) = (mods, scroll_speed);
    }

    fn key(&self, index: usize, rate: f64) -> Key {
        (
            index,
            rate_key(rate),
            mod_bits(self.mods) | note_bits(self.mods),
            rate_key(self.scroll_speed),
        )
    }

    pub fn request(&mut self, index: usize, path: &Path, rate: f64) {
        let key = self.key(index, rate);
        if self.requested.insert(key) {
            let _ = self
                .tx
                .send((key, path.to_path_buf(), rate, self.mods, self.scroll_speed));
        }
    }

    pub fn poll(&mut self) {
        while let Ok((key, stars)) = self.rx.try_recv() {
            self.cache.insert(key, stars);
        }
    }

    pub fn get(&self, index: usize, rate: f64) -> Option<f32> {
        self.cache.get(&self.key(index, rate)).copied().flatten()
    }

    /// Library indices changed (rescan): forget everything.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.requested.clear();
    }
}

/// osu!'s star rating colour spectrum.
pub fn color(stars: f32) -> u32 {
    const STOPS: [(f32, u32); 11] = [
        (0.1, 0x4290FB),
        (1.25, 0x4FC0FF),
        (2.0, 0x4FFFD5),
        (2.5, 0x7CFF4F),
        (3.3, 0xF6F05C),
        (4.2, 0xFF8068),
        (4.9, 0xFF4E6F),
        (5.8, 0xC645B8),
        (6.7, 0x6563DE),
        (7.7, 0x18158E),
        (9.0, 0x000000),
    ];
    if stars <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for w in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if stars <= b {
            let t = (stars - a) / (b - a);
            let ch = |s: u32| {
                ((ca >> s & 0xff) as f32 + ((cb >> s & 0xff) as f32 - (ca >> s & 0xff) as f32) * t)
                    as u32
            };
            return (ch(16) << 16) | (ch(8) << 8) | ch(0);
        }
    }
    STOPS[10].1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_rock_and_easy_change_the_star_rating_like_in_osu() {
        // A short taiko map: a 1/4 stream of dons and kats at 180 BPM.
        let mut osu = String::from(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:5\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,333.333,4,1,0,100,1,0\n\n[HitObjects]\n",
        );
        for k in 0..400 {
            let sound = if k % 3 == 0 { 2 } else { 0 };
            osu.push_str(&format!("256,192,{},1,{sound},0:0:0:0:\n", 1000 + k * 83));
        }
        let path = std::env::temp_dir().join(format!("taiko-rs-stars-{}.osu", std::process::id()));
        std::fs::write(&path, osu).unwrap();
        let nomod = star_rating(&std::fs::read(&path).unwrap(), 1.0).unwrap();
        let hr = star_rating_with(
            &std::fs::read(&path).unwrap(),
            1.0,
            mod_bits(crate::mods::Mods::parse("HR")),
        )
        .unwrap();
        let ez = star_rating_with(
            &std::fs::read(&path).unwrap(),
            1.0,
            mod_bits(crate::mods::Mods::parse("EZ")),
        )
        .unwrap();
        assert!(hr > nomod && ez < nomod, "nomod {nomod} HR {hr} EZ {ez}");
        // The scroll speed counts as pp has it: slower reads easier.
        let none = crate::mods::Mods::NONE;
        let slow = star_rating_for(&std::fs::read(&path).unwrap(), 1.0, none, 0.5).unwrap();
        assert!(slow < nomod, "nomod {nomod} at 0.5x scroll {slow}");
        assert!(
            (star_rating_for(&std::fs::read(&path).unwrap(), 1.0, none, 1.0).unwrap() - nomod)
                .abs()
                < 1e-9
        );
        assert!(
            (star_rating_for(
                &std::fs::read(&path).unwrap(),
                1.0,
                crate::mods::Mods::parse("HR"),
                1.0
            )
            .unwrap()
                - hr)
                .abs()
                < 1e-9
        );
        // Mods the rating doesn't count don't change it.
        assert_eq!(mod_bits(crate::mods::Mods::parse("HD,FL,PF")), 0);
        let _ = std::fs::remove_file(&path);
    }

    /// A taiko map of `n` dons, `gap_ms` apart.
    fn stream(n: usize, gap_ms: f64) -> Beatmap {
        map_of((0..n).map(|k| 1000.0 + k as f64 * gap_ms))
    }

    /// A taiko map of dons at `times`.
    fn map_of(times: impl Iterator<Item = f64>) -> Beatmap {
        let mut osu = String::from(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:5\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,333.333,4,1,0,100,1,0\n\n[HitObjects]\n",
        );
        for t in times {
            osu.push_str(&format!("256,192,{t},1,0,0:0:0:0:\n"));
        }
        osu.parse().unwrap()
    }

    #[test]
    fn fake_sliders_dont_rate_a_map_up() {
        let osu = |extra: &dyn Fn(u32) -> String| {
            let mut text = String::from(
                "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:5\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,333.333,4,1,0,100,1,0\n\n[HitObjects]\n",
            );
            for k in 0..400 {
                let t = 1000 + k * 111;
                text.push_str(&format!(
                    "256,192,{t},1,{},0:0:0:0:\n",
                    if k % 3 == 0 { 2 } else { 0 }
                ));
                text.push_str(&extra(t));
            }
            text
        };
        let stars = |text: String| {
            let path = std::env::temp_dir().join(format!(
                "taiko-rs-fake-sliders-{}-{}.osu",
                std::process::id(),
                text.len()
            ));
            std::fs::write(&path, text).unwrap();
            let stars = star_rating(&std::fs::read(&path).unwrap(), 1.0).unwrap();
            let _ = std::fs::remove_file(&path);
            stars
        };
        let plain = stars(osu(&|_| String::new()));
        // A fake slider a ms after every note, written negative, zero and 0.05 long.
        for length in ["-1", "0", "0.05", "-0.0001"] {
            let faked = stars(osu(&|t| {
                format!("256,320,{},6,4,L|256:384,1,{length}\n", t + 1)
            }));
            assert_eq!(faked, plain, "length {length}");
        }
        // A real (short) roll is still there for the calculator.
        let rolls = stars(osu(&|t| {
            if t % 2 == 0 {
                format!("256,320,{},2,0,L|256:384,1,10\n", t + 50)
            } else {
                String::new()
            }
        }));
        assert_ne!(rolls, plain);
    }

    #[test]
    fn marathons_over_rosu_pps_object_count_are_rated() {
        // Nana Party's size: 63,841 notes in three hours.
        let map = stream(63_841, 170.0);
        assert!(matches!(
            map.check_suspicion(),
            Err(rosu_pp::model::beatmap::TooSuspicious::ObjectCount)
        ));
        assert!(calculable(&map));
        assert!(
            Difficulty::new()
                .calculate_for_mode::<Taiko>(&map)
                .unwrap()
                .stars
                > 0.0
        );
        // Under rosu-pp's count, its own checks decide as before.
        assert!(calculable(&stream(1000, 100.0)));
    }

    #[test]
    fn maps_built_to_break_calculators_still_are_not() {
        // Past the count it's not tried at all.
        assert!(!calculable(&stream(MAX_TAIKO_OBJECTS + 1, 100.0)));
        // Over the count, the checks rosu-pp skips for it still apply: 401 notes within a second
        // (a burst in a 20 ms stream)...
        let burst = |n: usize| {
            map_of((0..40_000).map(move |k| {
                if k < n {
                    k as f64 * 2.0
                } else {
                    1e6 + k as f64 * 20.0
                }
            }))
        };
        assert!(!calculable(&burst(401)));
        assert!(calculable(&burst(400)));
        // ...1,001 within ten seconds...
        assert!(!calculable(&stream(40_000, 9.99)));
        assert!(calculable(&stream(40_000, 10.01)));
        // ...and over a day long.
        assert!(!calculable(&stream(40_000, 2200.0)));
    }

    #[test]
    fn spectrum_runs_blue_to_black() {
        assert_eq!(color(0.0), 0x4290FB);
        assert_eq!(color(20.0), 0x000000);
        assert_eq!(color(2.0), 0x4FFFD5);
    }
}
