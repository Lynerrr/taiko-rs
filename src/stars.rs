//! Official osu!taiko star ratings and strain curves via rosu-pp, a precise port of osu!lazer's
//! difficulty calculator (converts included). Computed on a background worker and cached.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use rosu_pp::taiko::Taiko;
use rosu_pp::{Beatmap, Difficulty};

use crate::render::{rgba, Canvas};

/// Section length of lazer's strain skills (in rate-adjusted time).
const SECTION_MS: f64 = 400.0;

fn load(path: &Path) -> Option<Beatmap> {
    let map = Beatmap::from_path(path).ok()?;
    // Maps built to break calculators (absurd density/length) are skipped, as osu! would time out.
    map.check_suspicion().ok()?;
    Some(map)
}

pub fn star_rating(path: &Path, rate: f64) -> Option<f64> {
    star_rating_with(path, rate, 0)
}

/// With mods (osu!'s bits, `mod_bits`): osu!taiko's star rating counts Hard Rock and Easy (their
/// scroll speed and hit windows) and Relax, besides the speed.
pub fn star_rating_with(path: &Path, rate: f64, mods: u32) -> Option<f64> {
    let map = load(path)?;
    Difficulty::new().mods(mods).clock_rate(rate).calculate_for_mode::<Taiko>(&map).ok().map(|a| a.stars)
}

/// The mods of `mods` osu!taiko's star rating counts, as osu!'s mod bits (the speed is passed as
/// a rate).
pub fn mod_bits(mods: crate::mods::Mods) -> u32 {
    use crate::mods::Mod;
    [(Mod::Easy, 2), (Mod::HardRock, 16), (Mod::Relax, 128)].into_iter().filter(|(m, _)| mods.has(*m)).map(|(_, b)| b).sum()
}

/// Combined skill strain per section, the map time the first section starts at, and the
/// section length in map time — for the song select difficulty graph.
pub fn strain_curve(path: &Path, rate: f64, first_object_ms: f64) -> Option<(Vec<f64>, f64, f64)> {
    let map = load(path)?;
    let s = Difficulty::new().clock_rate(rate).strains_for_mode::<Taiko>(&map).ok()?;
    let at = |v: &Vec<f64>, i: usize| v.get(i).copied().unwrap_or(0.0);
    let n = s.color.len().max(s.stamina.len()).max(s.rhythm.len());
    let combined = (0..n).map(|i| at(&s.color, i) + at(&s.rhythm, i) + at(&s.stamina, i) + at(&s.reading, i)).collect();
    // lazer's first section ends at the first multiple of the section length after the first
    // object (times divided by the clock rate).
    let first_end = (first_object_ms / rate / SECTION_MS).ceil() * SECTION_MS;
    Some((combined, (first_end - SECTION_MS) * rate, SECTION_MS * rate))
}

fn rate_key(rate: f64) -> u32 {
    (rate * 100.0).round() as u32
}

/// Background star rating calculator with a cache keyed by library index, rate and mods.
pub struct Stars {
    tx: Sender<(usize, PathBuf, f64, u32)>,
    rx: Receiver<((usize, u32, u32), Option<f32>)>,
    cache: HashMap<(usize, u32, u32), Option<f32>>,
    requested: HashSet<(usize, u32, u32)>,
    /// The mods ratings are for now (`mod_bits`).
    mods: u32,
}

impl Default for Stars {
    fn default() -> Self {
        let (tx, jobs) = mpsc::channel::<(usize, PathBuf, f64, u32)>();
        let (done, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("stars".into())
            .spawn(move || {
                for (i, path, rate, mods) in jobs {
                    let stars = std::panic::catch_unwind(|| star_rating_with(&path, rate, mods)).ok().flatten().map(|s| s as f32);
                    if done.send(((i, rate_key(rate), mods), stars)).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn star worker");
        Stars { tx, rx, cache: HashMap::new(), requested: HashSet::new(), mods: 0 }
    }
}

impl Stars {
    /// Queues a calculation unless it is cached or already queued.
    /// Ratings from now on are with `mods` (the ones osu!taiko's rating counts).
    pub fn set_mods(&mut self, mods: crate::mods::Mods) {
        self.mods = mod_bits(mods);
    }

    pub fn request(&mut self, index: usize, path: &Path, rate: f64) {
        if self.requested.insert((index, rate_key(rate), self.mods)) {
            let _ = self.tx.send((index, path.to_path_buf(), rate, self.mods));
        }
    }

    pub fn poll(&mut self) {
        while let Ok((key, stars)) = self.rx.try_recv() {
            self.cache.insert(key, stars);
        }
    }

    pub fn get(&self, index: usize, rate: f64) -> Option<f32> {
        self.cache.get(&(index, rate_key(rate), self.mods)).copied().flatten()
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
        (0.1, 0x4290FB), (1.25, 0x4FC0FF), (2.0, 0x4FFFD5), (2.5, 0x7CFF4F), (3.3, 0xF6F05C), (4.2, 0xFF8068),
        (4.9, 0xFF4E6F), (5.8, 0xC645B8), (6.7, 0x6563DE), (7.7, 0x18158E), (9.0, 0x000000),
    ];
    if stars <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for w in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if stars <= b {
            let t = (stars - a) / (b - a);
            let ch = |s: u32| ((ca >> s & 0xff) as f32 + ((cb >> s & 0xff) as f32 - (ca >> s & 0xff) as f32) * t) as u32;
            return (ch(16) << 16) | (ch(8) << 8) | ch(0);
        }
    }
    STOPS[10].1
}

/// A small star-rating pill ("4.82") in the spectrum colour, right-aligned at `right`.
/// Returns its width. `None` draws a placeholder while it is being calculated.
pub fn draw_pill(c: &mut Canvas, right: f32, y: f32, size: f32, stars: Option<f32>) -> f32 {
    let text = stars.map(|s| format!("{s:.2}")).unwrap_or_else(|| "...".into());
    let tw = Canvas::text_width(size, &text);
    let (w, h) = (tw + size * 1.6 + 6.0, size + 6.0);
    let x = right - w;
    let (bg, fg) = match stars {
        Some(s) => {
            let col = color(s);
            // Dark text on the light end of the spectrum, light text on the dark end.
            let luma = 0.299 * (col >> 16 & 0xff) as f32 + 0.587 * (col >> 8 & 0xff) as f32 + 0.114 * (col & 0xff) as f32;
            (col, if luma > 140.0 { 0x15151d } else { 0xffffff })
        }
        None => (0x2a2a38, 0x8a8aa0),
    };
    c.round_rect(x, y - 3.0, w, h, h / 2.0, rgba(bg, 1.0), rgba(bg, 1.0), 0.0);
    // A small diamond stands in for the star glyph the pixel font doesn't have.
    let d = size * 0.32;
    let (dx, dy) = (x + size * 0.6 + 2.0, y - 3.0 + h / 2.0);
    c.round_rect(dx - d, dy - d, d * 2.0, d * 2.0, d * 0.4, rgba(fg, 1.0), rgba(fg, 1.0), 0.0);
    c.text(x + size * 1.3 + 4.0, y, size, rgba(fg, 1.0), &text);
    w
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
        let nomod = star_rating(&path, 1.0).unwrap();
        let hr = star_rating_with(&path, 1.0, mod_bits(crate::mods::Mods::parse("HR"))).unwrap();
        let ez = star_rating_with(&path, 1.0, mod_bits(crate::mods::Mods::parse("EZ"))).unwrap();
        assert!(hr > nomod && ez < nomod, "nomod {nomod} HR {hr} EZ {ez}");
        // Mods the rating doesn't count don't change it.
        assert_eq!(mod_bits(crate::mods::Mods::parse("HD,FL,PF")), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn spectrum_runs_blue_to_black() {
        assert_eq!(color(0.0), 0x4290FB);
        assert_eq!(color(20.0), 0x000000);
        assert_eq!(color(2.0), 0x4FFFD5);
    }
}
