//! Performance points: osu!lazer's taiko pp (via rosu-pp) for a play's hits, speed, osu!'s
//! difficulty mods and scroll speed, times the score multipliers of taiko.rs's own mods.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use rosu_pp::Difficulty;
use rosu_pp::taiko::{TaikoGradualPerformance, TaikoPerformance, TaikoScoreState};

use crate::mods::{Mod, Mods};

/// A finished play, as far as pp go.
#[derive(Clone, Copy, Debug)]
pub struct Play {
    pub great: u32,
    pub ok: u32,
    pub miss: u32,
    pub max_combo: u32,
    /// Speed, DT/HT included.
    pub rate: f64,
    pub mods: Mods,
    /// The scroll speed setting played at (1 = osu!'s): notes moving slower are easier to read
    /// and give less pp, faster ones more (lazer's Difficulty Adjust). HR's faster notes slowed
    /// back down don't count as HR's.
    pub scroll_speed: f64,
    /// Random's colouring (`Game::seed`).
    pub seed: u64,
}

/// The mods pp is worked out with: osu!'s that change the formula (`FORMULA`), and a scroll
/// speed other than the usual as lazer's Difficulty Adjust. At the usual speed, osu!'s plain mod
/// bits (the same numbers either way).
pub(crate) fn formula_mods(mods: Mods, scroll_speed: f64) -> rosu_pp::model::mods::GameMods {
    let bits: u32 = FORMULA.iter().filter(|f| mods.has(f.0)).map(|f| f.1).sum();
    if !scroll_speed.is_finite() || scroll_speed <= 0.0 || (scroll_speed - 1.0).abs() < 1e-6 {
        return bits.into();
    }
    use rosu_pp::model::mods::rosu_mods::generated_mods::DifficultyAdjustTaiko;
    use rosu_pp::model::mods::rosu_mods::{GameMod, GameMods};
    let mut lazer = GameMods::new();
    for (m, gm) in [
        (Mod::Easy, GameMod::EasyTaiko(Default::default())),
        (Mod::Hidden, GameMod::HiddenTaiko(Default::default())),
        (Mod::HardRock, GameMod::HardRockTaiko(Default::default())),
        (Mod::Flashlight, GameMod::FlashlightTaiko(Default::default())),
    ] {
        if mods.has(m) {
            lazer.insert(gm);
        }
    }
    lazer.insert(GameMod::DifficultyAdjustTaiko(DifficultyAdjustTaiko { scroll_speed: Some(scroll_speed), ..Default::default() }));
    lazer.into()
}

/// osu!'s mods that change the pp formula itself, with their (legacy) bits.
const FORMULA: [(Mod, u32); 4] = [(Mod::Easy, 2), (Mod::Hidden, 8), (Mod::HardRock, 16), (Mod::Flashlight, 1024)];
/// Mods whose only effect on pp is the speed or the notes (`stars::load_played`), or none.
const NEUTRAL: [Mod; 8] = [Mod::NoFail, Mod::SuddenDeath, Mod::Perfect, Mod::DoubleTime, Mod::Nightcore, Mod::HalfTime, Mod::Daycore, Mod::Half];

/// What the play's other mods (taiko.rs's own, and osu!'s that pp doesn't know) do to its pp:
/// their score multipliers.
pub fn custom_factor(mods: Mods) -> f64 {
    mods.iter().filter(|m| !FORMULA.iter().any(|f| f.0 == *m) && !NEUTRAL.contains(m)).map(|m| m.multiplier()).product()
}

/// The pp of a play on the map at `path`. None for Autoplay / Relax, or a map that can't be read.
pub fn play_pp(path: &Path, p: &Play) -> Option<f64> {
    if p.mods.unranked() {
        return None;
    }
    let map = crate::stars::load_played(path, p.mods, p.seed)?;
    let attrs = TaikoPerformance::new(&map)
        .mods(formula_mods(p.mods, p.scroll_speed))
        .clock_rate(p.rate)
        .n300(p.great)
        .n100(p.ok)
        .misses(p.miss)
        .combo(p.max_combo)
        .calculate()
        .ok()?;
    Some(attrs.pp * custom_factor(p.mods))
}

/// The pp of a play, taken apart like osu!'s results screen does: the parts it is made of, and
/// what it would have been as a full combo and as an SS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Breakdown {
    pub pp: f64,
    /// The difficulty (strain) part and the accuracy part, before they are combined.
    pub difficulty: f64,
    pub accuracy: f64,
    /// The misses turned into GREATs (and the whole combo), and every note GREAT.
    pub if_fc: f64,
    pub if_ss: f64,
    /// The SS's difficulty and accuracy parts (what the play's are measured against).
    pub ss_difficulty: f64,
    pub ss_accuracy: f64,
    pub stars: f64,
    pub max_combo: u32,
}

/// The breakdown of a play's pp (None where `play_pp` is None).
pub fn breakdown(path: &Path, p: &Play) -> Option<Breakdown> {
    if p.mods.unranked() {
        return None;
    }
    let map = crate::stars::load_played(path, p.mods, p.seed)?;
    let k = custom_factor(p.mods);
    let base = TaikoPerformance::new(&map).mods(formula_mods(p.mods, p.scroll_speed)).clock_rate(p.rate);
    let attrs = base.clone().n300(p.great).n100(p.ok).misses(p.miss).combo(p.max_combo).calculate().ok()?;
    let max_combo = attrs.difficulty.max_combo;
    let fc = base.clone().n300(p.great + p.miss).n100(p.ok).misses(0).combo(max_combo).calculate().ok()?;
    let ss = base.n300(p.great + p.ok + p.miss).n100(0).misses(0).combo(max_combo).calculate().ok()?;
    Some(Breakdown {
        pp: attrs.pp * k,
        difficulty: attrs.pp_difficulty * k,
        accuracy: attrs.pp_acc * k,
        if_fc: fc.pp * k,
        if_ss: ss.pp * k,
        ss_difficulty: ss.pp_difficulty * k,
        ss_accuracy: ss.pp_acc * k,
        stars: attrs.difficulty.stars,
        max_combo,
    })
}

/// The accuracies song select tells the pp of, full combo, before a play.
pub const POTENTIAL: [f64; 3] = [100.0, 99.0, 98.0];

/// What a full combo of the map at `path` gives at each of `POTENTIAL`'s accuracies, with
/// `mods` at `rate` and `scroll_speed` (None where `play_pp` would be None).
pub fn potential(path: &Path, rate: f64, mods: Mods, scroll_speed: f64) -> Option<[f64; 3]> {
    if mods.unranked() {
        return None;
    }
    let map = crate::stars::load_played(path, mods, 0)?;
    let base = TaikoPerformance::new(&map).mods(formula_mods(mods, scroll_speed)).clock_rate(rate).misses(0);
    let mut out = [0.0; 3];
    for (pp, acc) in out.iter_mut().zip(POTENTIAL) {
        *pp = base.clone().accuracy(acc).calculate().ok()?.pp * custom_factor(mods);
    }
    Some(out)
}

/// `potential` for the selected map, worked out on a thread of its own and kept per map, speed,
/// mods and scroll speed, so changing them back and forth costs nothing.
pub struct Potentials {
    tx: Sender<(PotentialKey, PathBuf)>,
    rx: Receiver<(PotentialKey, Option<[f64; 3]>)>,
    known: std::collections::HashMap<PotentialKey, Option<[f64; 3]>>,
}

/// The map, the speed (in hundredths), the mods and the scroll speed (in hundredths).
type PotentialKey = (PathBuf, u32, String, u32);

impl Default for Potentials {
    fn default() -> Self {
        let (tx, jobs) = mpsc::channel::<(PotentialKey, PathBuf)>();
        let (done, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("pp-potential".into())
            .spawn(move || {
                for (key, path) in jobs {
                    let (rate, scroll) = (key.1 as f64 / 100.0, key.3 as f64 / 100.0);
                    let pp = std::panic::catch_unwind(|| potential(&path, rate, Mods::parse_set(&key.2), scroll)).ok().flatten();
                    if done.send((key, pp)).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn pp worker");
        Potentials { tx, rx, known: std::collections::HashMap::new() }
    }
}

impl Potentials {
    /// The answer for these, once it's there (asked for the first time it isn't).
    pub fn get(&mut self, path: &Path, rate: f64, mods: Mods, scroll_speed: f64) -> Option<[f64; 3]> {
        while let Ok((key, pp)) = self.rx.try_recv() {
            self.known.insert(key, pp);
        }
        let key = (path.to_path_buf(), (rate * 100.0).round() as u32, mods.list().join(","), (scroll_speed * 100.0).round() as u32);
        match self.known.get(&key) {
            Some(pp) => *pp,
            None => {
                self.known.insert(key.clone(), None);
                let _ = self.tx.send((key, path.to_path_buf()));
                None
            }
        }
    }
}

/// The pp of a play while it is played, like osu!'s pp counter: what the notes judged so far
/// are worth. Worked out on a thread of its own (the map's difficulty is walked note by note),
/// so the game only hands it the counts and reads the latest answer.
pub struct Live {
    tx: Sender<Counts>,
    rx: Receiver<(Counts, f64)>,
    sent: Counts,
    /// The latest answer, and the counts it is for.
    pp: Option<(Counts, f64)>,
}

/// GREATs, OKs, misses and the max combo.
type Counts = (u32, u32, u32, u32);

impl Live {
    /// None for Autoplay / Relax (they get no pp).
    pub fn start(path: PathBuf, rate: f64, mods: Mods, scroll_speed: f64, seed: u64) -> Option<Live> {
        if mods.unranked() {
            return None;
        }
        let (tx, jobs) = mpsc::channel::<Counts>();
        let (done, rx) = mpsc::channel();
        let formula = formula_mods(mods, scroll_speed);
        let k = custom_factor(mods);
        std::thread::Builder::new()
            .name("live pp".into())
            .spawn(move || {
                let Some(map) = crate::stars::load_played(&path, mods, seed) else { return };
                let fresh = || TaikoGradualPerformance::new(Difficulty::new().mods(formula.clone()).clock_rate(rate), &map).ok();
                let Some(mut gradual) = fresh() else { return };
                // Notes the calculator has walked past.
                let mut walked = 0usize;
                let mut last = 0.0;
                while let Ok(mut c) = jobs.recv() {
                    // Only the latest counts matter.
                    while let Ok(newer) = jobs.try_recv() {
                        c = newer;
                    }
                    let judged = (c.0 + c.1 + c.2) as usize;
                    // A replay seeked back: walk the map again from the start.
                    if judged < walked {
                        let Some(g) = fresh() else { return };
                        gradual = g;
                        walked = 0;
                        last = 0.0;
                    }
                    let judged = judged.min(walked + gradual.len());
                    if judged == walked {
                        if done.send((c, last)).is_err() {
                            return;
                        }
                        continue;
                    }
                    let mut state = TaikoScoreState::new();
                    state.max_combo = c.3;
                    state.hitresults.n300 = c.0;
                    state.hitresults.n100 = c.1;
                    state.hitresults.misses = c.2;
                    let attrs = gradual.nth(state, judged - walked - 1);
                    walked = judged;
                    if let Some(a) = attrs {
                        last = a.pp * k;
                    }
                    if done.send((c, last)).is_err() {
                        return;
                    }
                }
            })
            .ok()?;
        Some(Live { tx, rx, sent: (u32::MAX, 0, 0, 0), pp: None })
    }

    /// Hands over the play's counts (when they changed) and returns the latest pp worked out.
    pub fn update(&mut self, great: u32, ok: u32, miss: u32, max_combo: u32) -> Option<f64> {
        let counts = (great, ok, miss, max_combo);
        if counts != self.sent {
            self.sent = counts;
            let _ = self.tx.send(counts);
        }
        while let Ok(pp) = self.rx.try_recv() {
            self.pp = Some(pp);
        }
        self.pp.map(|(_, pp)| pp)
    }

    /// Whether the pp returned is for the counts last handed over.
    #[cfg(test)]
    pub fn is_current(&self) -> bool {
        self.pp.is_some_and(|(c, _)| c == self.sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_mods_scale_pp_and_osu_ones_go_to_the_formula() {
        assert_eq!(custom_factor(Mods::parse("HD,HR,DT,NF")), 1.0);
        assert!((custom_factor(Mods::parse("HD,CH")) - Mod::SvChaos.multiplier()).abs() < 1e-12);
        assert_eq!(custom_factor(Mods::parse("HA")), 1.0, "Half's pp go by the notes it leaves");
    }

    #[test]
    fn half_counts_the_notes_it_leaves() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-pp-half-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("map.osu");
        let mut text = String::from("osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:5\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,300,4,1,0,100,1,0\n\n[HitObjects]\n");
        for k in 0..800 {
            text += &format!("256,192,{},1,{},0:0:0:0:\n", 1000 + k * 100, if k % 3 == 0 { 2 } else { 0 });
        }
        std::fs::write(&path, text).unwrap();
        let (none, half) = (Mods::NONE, Mods::parse("HA"));
        let full = crate::stars::star_rating_for(&path, 1.0, none, 1.0).unwrap();
        let cut = crate::stars::star_rating_for(&path, 1.0, half, 1.0).unwrap();
        assert!(cut < full * 0.8, "{cut} of {full}");
        let play = Play { great: 400, ok: 0, miss: 0, max_combo: 400, rate: 1.0, mods: half, scroll_speed: 1.0, seed: 0 };
        let b = breakdown(&path, &play).unwrap();
        assert_eq!(b.max_combo, 400);
        assert!((b.stars - cut).abs() < 1e-9);
        assert!((b.pp - b.if_ss).abs() < 1e-6, "400 GREATs are all of what's left");
        assert!(b.pp < play_pp(&path, &Play { great: 800, max_combo: 800, mods: none, ..play }).unwrap());
        // Colours count too, and Random goes by its seed.
        let random = |seed| crate::stars::star_rating_played(&path, 1.0, Mods::parse("RD"), 1.0, seed).unwrap();
        assert!(random(1) != full && random(1) != random(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_play_gets_pp_from_its_map() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-pp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("map.osu");
        let mut text = String::from("osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:6\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,300,4,1,0,100,1,0\n\n[HitObjects]\n");
        for k in 0..400 {
            let sound = if k % 3 == 0 { 2 } else { 0 };
            text += &format!("256,192,{},1,{sound},0:0:0:0:\n", 1000 + k * 150);
        }
        std::fs::write(&path, text).unwrap();
        let play = Play { great: 400, ok: 0, miss: 0, max_combo: 400, rate: 1.0, mods: Mods::NONE, scroll_speed: 1.0, seed: 0 };
        let ss = play_pp(&path, &play).unwrap();
        assert!(ss > 1.0, "{ss}");
        let worse = play_pp(&path, &Play { great: 360, ok: 30, miss: 10, max_combo: 200, ..play }).unwrap();
        assert!(worse < ss);
        assert!(play_pp(&path, &Play { mods: Mods::parse("HR"), ..play }).unwrap() > ss);
        assert!(play_pp(&path, &Play { rate: 1.5, ..play }).unwrap() > ss);
        assert!(play_pp(&path, &Play { mods: Mods::parse("AT"), ..play }).is_none());

        let b = breakdown(&path, &Play { great: 360, ok: 30, miss: 10, max_combo: 200, ..play }).unwrap();
        assert!((b.pp - worse).abs() < 1e-9);
        assert!(b.pp < b.if_fc && b.if_fc < b.if_ss && (b.if_ss - ss).abs() < 1e-9, "{b:?}");
        assert!(b.difficulty > 0.0 && b.accuracy > 0.0 && b.max_combo == 400);

        // Before a play: SS, 99% and 98% full combos, the SS the same as the play's.
        let [p100, p99, p98] = potential(&path, 1.0, Mods::NONE, 1.0).unwrap();
        assert!((p100 - ss).abs() < 1e-6 && p100 > p99 && p99 > p98, "{p100} {p99} {p98}");
        assert!(potential(&path, 1.5, Mods::parse("HR,DT"), 1.0).unwrap()[1] > p99);
        assert!(potential(&path, 1.0, Mods::parse("AT"), 1.0).is_none());
        let mut worker = Potentials::default();
        let got = (0..1000).find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(5));
            worker.get(&path, 1.0, Mods::NONE, 1.0)
        });
        assert_eq!(got, Some([p100, p99, p98]));

        // The live counter climbs as notes are hit and ends where the whole play's pp is.
        let mut live = Live::start(path.clone(), 1.0, Mods::NONE, 1.0, 0).unwrap();
        let read = |live: &mut Live, n: u32| {
            for _ in 0..1000 {
                let pp = live.update(n, 0, 0, n);
                if live.is_current() {
                    return pp.unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("no live pp");
        };
        let half = read(&mut live, 200);
        let all = read(&mut live, 400);
        assert!(half > 0.0 && half < all, "{half} {all}");
        assert!((all - ss).abs() / ss < 0.01, "{all} vs {ss}");
        // Seeking back (a replay) works it out again from the start.
        assert!((read(&mut live, 200) - half).abs() < 1e-9);
        assert!(Live::start(path.clone(), 1.0, Mods::parse("AT"), 1.0, 0).is_none());

        // Scroll speed: HR's faster notes slowed back down are worth about nomod's reading, not
        // HR's; the usual speed is exactly what it always was; faster notes are worth more.
        let hr = Play { mods: Mods::parse("HR"), ..play };
        let hr_pp = play_pp(&path, &hr).unwrap();
        let hr_slow = play_pp(&path, &Play { scroll_speed: 0.5, ..hr }).unwrap();
        assert!(hr_slow < hr_pp, "{hr_slow} vs {hr_pp}");
        assert!(play_pp(&path, &Play { scroll_speed: 1.5, ..play }).unwrap() > ss);
        let bits = TaikoPerformance::new(&crate::stars::load(&path).unwrap()).mods(16u32).n300(400).calculate().unwrap().pp;
        assert!((hr_pp - bits).abs() < 1e-9, "the usual speed changes nothing");
        // The lazer form of HR at a scroll speed of exactly 1 is HR.
        let lazer_one = TaikoPerformance::new(&crate::stars::load(&path).unwrap()).mods(formula_mods(Mods::parse("HR"), 1.0 + 1e-3)).n300(400).calculate().unwrap().pp;
        assert!((lazer_one - bits).abs() / bits < 0.01, "{lazer_one} vs {bits}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
