//! Performance points: osu!lazer's taiko pp (via rosu-pp) for a play's hits, speed and osu!'s
//! difficulty mods, times the score multipliers of taiko-rs's own mods.

use std::path::Path;

use rosu_pp::taiko::TaikoPerformance;

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
}

/// osu!'s mods that change the pp formula itself, with their (legacy) bits.
const FORMULA: [(Mod, u32); 4] = [(Mod::Easy, 2), (Mod::Hidden, 8), (Mod::HardRock, 16), (Mod::Flashlight, 1024)];
/// osu!'s mods whose only effect on pp is the speed (or none).
const NEUTRAL: [Mod; 7] = [Mod::NoFail, Mod::SuddenDeath, Mod::Perfect, Mod::DoubleTime, Mod::Nightcore, Mod::HalfTime, Mod::Daycore];

/// What the play's other mods (taiko-rs's own, and osu!'s that pp doesn't know) do to its pp:
/// their score multipliers.
pub fn custom_factor(mods: Mods) -> f64 {
    mods.iter().filter(|m| !FORMULA.iter().any(|f| f.0 == *m) && !NEUTRAL.contains(m)).map(|m| m.multiplier()).product()
}

/// The pp of a play on the map at `path`. None for Autoplay / Relax, or a map that can't be read.
pub fn play_pp(path: &Path, p: &Play) -> Option<f64> {
    if p.mods.unranked() {
        return None;
    }
    let map = crate::stars::load(path)?;
    let bits: u32 = FORMULA.iter().filter(|f| p.mods.has(f.0)).map(|f| f.1).sum();
    let attrs = TaikoPerformance::new(&map)
        .mods(bits)
        .clock_rate(p.rate)
        .n300(p.great)
        .n100(p.ok)
        .misses(p.miss)
        .combo(p.max_combo)
        .calculate()
        .ok()?;
    Some(attrs.pp * custom_factor(p.mods))
}

/// A player's total: their best pp per map, highest first, each counting 5% less than the one
/// before (osu!'s weighting).
pub fn total(best: Vec<f64>) -> f64 {
    // The server's formula, so local and online totals agree.
    crate::online::weighted_pp(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_mods_scale_pp_and_osu_ones_go_to_the_formula() {
        assert_eq!(custom_factor(Mods::parse("HD,HR,DT,NF")), 1.0);
        assert!((custom_factor(Mods::parse("HD,CH")) - Mod::SvChaos.multiplier()).abs() < 1e-12);
        assert!(custom_factor(Mods::parse("HA")) < 1.0, "Half removes notes: less pp");
    }

    #[test]
    fn totals_weight_the_best_plays_most() {
        assert_eq!(total(vec![]), 0.0);
        assert!((total(vec![100.0, 200.0]) - (200.0 + 95.0)).abs() < 1e-9);
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
        let play = Play { great: 400, ok: 0, miss: 0, max_combo: 400, rate: 1.0, mods: Mods::NONE };
        let ss = play_pp(&path, &play).unwrap();
        assert!(ss > 1.0, "{ss}");
        let worse = play_pp(&path, &Play { great: 360, ok: 30, miss: 10, max_combo: 200, ..play }).unwrap();
        assert!(worse < ss);
        assert!(play_pp(&path, &Play { mods: Mods::parse("HR"), ..play }).unwrap() > ss);
        assert!(play_pp(&path, &Play { rate: 1.5, ..play }).unwrap() > ss);
        assert!(play_pp(&path, &Play { mods: Mods::parse("AT"), ..play }).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
