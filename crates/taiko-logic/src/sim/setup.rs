//! Getting a map ready to play: the mods' changes to its notes and speeds, and what the
//! playfield needs worked out from it ahead (Multiverse's focus, Roulette, flows).

use super::*;

/// Multiverse: the focus moving to another playfield.
#[derive(Clone, Copy, Debug)]
pub struct FocusSwitch {
    /// The bar it lands on (map ms).
    pub at: f64,
    /// The playfield that gets it.
    pub field: usize,
    /// The beat length there.
    pub beat: f64,
}

/// Roulette's 4-bar sections: where each starts (map ms) and its speed. The first is 1x; no
/// two in a row are alike.
#[inline]
pub fn roulette_sections(map: &Beatmap) -> Vec<(f64, f64)> {
    const SPEEDS: [f64; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];
    let mut out: Vec<(f64, f64)> = Vec::new();
    for b in map.barlines.iter().step_by(4) {
        let mut k = (hash01(b.time + 0.4) * SPEEDS.len() as f64) as usize % SPEEDS.len();
        let prev = out.last().map_or(1.0, |s| s.1);
        if out.is_empty() {
            k = 2;
        } else if SPEEDS[k] == prev {
            k = (k + 1) % SPEEDS.len();
        }
        out.push((b.time, SPEEDS[k]));
    }
    out
}
/// Multiverse: where the focus moves to another playfield. Every 2 bars, every bar in kiai
/// time, from the first note to the last, to one that parks well above or below the one that
/// has it, so the one in focus never hides the one that's called.
#[inline]
pub fn focus_switches(map: &Beatmap) -> Vec<FocusSwitch> {
    let first = map.notes.first().map_or(0.0, |n| n.time);
    let last = map
        .notes
        .iter()
        .map(|n| n.end_time)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut focus = FIRST_FOCUS;
    let mut out = Vec::new();
    for (i, b) in map.barlines.iter().enumerate().skip(1) {
        let cp = beatmap::control_point_at(&map.control_points, b.time);
        if i % if cp.kiai { 1 } else { 2 } != 0 || b.time <= first + 1000.0 || b.time >= last {
            continue;
        }
        let away: Vec<usize> = (0..FIELD_COUNT)
            .filter(|&f| (parked(f).1 - parked(focus).1).abs() >= 80.0)
            .collect();
        focus = away[(hash01(b.time + 0.83) * away.len() as f64) as usize % away.len()];
        out.push(FocusSwitch {
            at: b.time,
            field: focus,
            beat: cp.beat_length,
        });
    }
    out
}
/// The speed most notes have: Constant Speed's, and what Anti-SV / Hyper SV bend around.
pub(crate) fn main_velocity(map: &Beatmap) -> f64 {
    // Counted to 4 decimals; the first exact velocity of the most common one is the answer.
    let mut counts: std::collections::HashMap<i64, (usize, f64)> = std::collections::HashMap::new();
    for n in &map.notes {
        counts
            .entry((n.velocity * 10_000.0).round() as i64)
            .or_insert((0, n.velocity))
            .0 += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(v, (c, _))| (c, v))
        .map(|(_, (_, v))| v)
        .unwrap_or(1.0)
        .max(0.01)
}
/// The scroll speed mods that only depend on the map change the notes' (and barlines') own
/// velocities, so drawing needs nothing more for them.
pub(crate) fn bake_speeds(map: &mut Beatmap, mods: Mods) {
    let main = main_velocity(map);
    let first = map.notes.first().map(|n| n.time).unwrap_or(0.0);
    let end = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
    let sections = roulette_sections(map);
    let bars = map.barlines.clone();
    let cps = map.control_points.clone();
    // By map time, for notes and barlines alike.
    let at = |v: f64, t: f64| -> f64 {
        let mut v = if mods.has(Mod::ConstantSpeed) {
            main
        } else {
            v
        };
        if mods.has(Mod::AntiSv) {
            v = (main * main / v).clamp(main / 10.0, main * 10.0);
        }
        if mods.has(Mod::HyperSv) {
            v = (v * v / main).clamp(main / 10.0, main * 10.0);
        }
        let cp = beatmap::control_point_at(&cps, t);
        if mods.has(Mod::Crescendo) {
            v *= 0.8 + 0.8 * ((t - first) / (end - first).max(1.0)).clamp(0.0, 1.0);
        }
        if mods.has(Mod::Drop) && cp.kiai {
            v *= 1.4;
        }
        if mods.has(Mod::Gearbox)
            && let Some((_, start, _)) = bar_at(&bars, t, 1.0)
        {
            v *= 1.0
                + 0.15
                    * ((t - start) / cp.beat_length + 0.01)
                        .floor()
                        .clamp(0.0, 7.0);
        }
        if mods.has(Mod::Roulette) {
            let i = sections.partition_point(|s| s.0 <= t + 1.0);
            v *= i.checked_sub(1).map_or(1.0, |i| sections[i].1);
        }
        if mods.has(Mod::Swing) {
            v *= 1.0 + 0.15 * (std::f64::consts::TAU * cp.beat_phase(t)).cos();
        }
        v
    };
    for b in &mut map.barlines {
        b.velocity = at(b.velocity, b.time);
    }
    for n in map.notes.iter_mut().chain(&mut map.fakes) {
        n.velocity = at(n.velocity, n.time);
    }
    // Per note: dons and kats only.
    let hits: Vec<usize> = (0..map.notes.len())
        .filter(|&i| matches!(map.notes[i].kind, Kind::Don | Kind::Kat))
        .collect();
    // Even: the closer a note is to its neighbours, the faster, so they end up evenly spaced.
    let gaps: Vec<f64> = (0..hits.len())
        .map(|k| {
            let prev = k
                .checked_sub(1)
                .map(|j| map.notes[hits[k]].time - map.notes[hits[j]].time);
            let next = hits
                .get(k + 1)
                .map(|&j| map.notes[j].time - map.notes[hits[k]].time);
            match (prev, next) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => a.or(b).unwrap_or(250.0),
            }
            .clamp(15.0, 4000.0)
        })
        .collect();
    let typical = {
        let mut g = gaps.clone();
        g.sort_by(f64::total_cmp);
        g.get(g.len() / 2).copied().unwrap_or(250.0)
    };
    let flow = flows(map);
    for (k, &i) in hits.iter().enumerate() {
        let n = &mut map.notes[i];
        if mods.has(Mod::SnapSpeed) {
            n.velocity *= match flow[i] {
                3 => 0.8,
                6 | 12 => 1.25,
                8 | 16 => 1.1,
                _ => 1.0,
            };
        }
        if mods.has(Mod::Split) {
            n.velocity *= if n.kind == Kind::Don { 1.2 } else { 0.85 };
        }
        if mods.has(Mod::Leapfrog) {
            n.velocity *= if k % 2 == 0 { 1.25 } else { 0.8 };
        }
        if mods.has(Mod::Even) {
            n.velocity *= (typical / gaps[k]).clamp(0.4, 2.5);
        }
    }
}
/// Decoy's fake notes: in about half the gaps between notes, one halfway, of a random colour.
pub fn decoys(map: &Beatmap) -> Vec<beatmap::Note> {
    let hits: Vec<&beatmap::Note> = map
        .notes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat))
        .collect();
    let mut out = Vec::new();
    // Inside streams (gaps under 120 ms) only now and then, and never in two gaps running, so
    // they stay readable.
    let mut last_in_stream = false;
    for p in hits.windows(2) {
        let gap = p[1].time - p[0].time;
        let stream = gap < 120.0;
        let chance = if !stream {
            0.5
        } else if gap >= 40.0 && !last_in_stream {
            0.12
        } else {
            0.0
        };
        if hash01(p[0].time + 0.25) >= chance {
            last_in_stream = false;
            continue;
        }
        last_in_stream = stream;
        let mut n = p[0].clone();
        n.time = (p[0].time + p[1].time) / 2.0;
        n.end_time = n.time;
        n.kind = if hash01(n.time + 0.75) < 0.5 {
            Kind::Don
        } else {
            Kind::Kat
        };
        n.big = !stream && hash01(n.time + 0.6) < 0.12;
        n.velocity *= 0.9 + 0.2 * hash01(n.time + 0.1);
        out.push(n);
    }
    out
}
/// The rhythm of a note at map time `t`: the smallest of 1, 2, 3, 4, 6, 8, 12 and 16 whose 1/n
/// beat grid (from its red line) it sits on, within 3 ms; 0 when on none.
pub fn snap_of(cps: &[beatmap::ControlPoint], t: f64) -> u8 {
    let cp = beatmap::control_point_at(cps, t);
    let beats = (t - cp.beat_origin) / cp.beat_length;
    [1u8, 2, 3, 4, 6, 8, 12, 16]
        .into_iter()
        .find(|&d| {
            let x = beats * d as f64;
            (x - x.round()).abs() * cp.beat_length / d as f64 <= 3.0
        })
        .unwrap_or(0)
}
/// The rhythm each note is part of: how far it is from its nearest neighbour, as the 1/n of a
/// beat that gap is closest to (1, 2, 3, 4, 6, 8, 12 or 16). Every note of a 1/4 stream gets 4,
/// of a triplet run 3, a lone note 1 or 2. 0 for rolls and swells.
pub(crate) fn flows(map: &Beatmap) -> Vec<u8> {
    let hits: Vec<usize> = (0..map.notes.len())
        .filter(|&i| matches!(map.notes[i].kind, Kind::Don | Kind::Kat))
        .collect();
    let mut out = vec![0u8; map.notes.len()];
    for (k, &i) in hits.iter().enumerate() {
        let t = map.notes[i].time;
        let prev = k.checked_sub(1).map(|j| t - map.notes[hits[j]].time);
        let next = hits.get(k + 1).map(|&j| map.notes[j].time - t);
        let Some(gap) = [prev, next].into_iter().flatten().reduce(f64::min) else {
            continue;
        };
        let beats = gap / beatmap::control_point_at(&map.control_points, t).beat_length;
        out[i] = [1u8, 2, 3, 4, 6, 8, 12, 16]
            .into_iter()
            .min_by(|&a, &b| {
                (beats * a as f64 - 1.0)
                    .abs()
                    .total_cmp(&(beats * b as f64 - 1.0).abs())
            })
            .unwrap_or(1);
    }
    out
}
/// Liar's far-off times: each triplet note (1/3, 1/6, 1/12) moved to the nearest 1/4 (1/8,
/// 1/16) of the beat, unless a real note is already there; other notes keep their own.
#[inline]
pub fn lies(map: &Beatmap) -> Vec<f64> {
    let hit_at = |t: f64| {
        let i = map.notes.partition_point(|n| n.time < t - 5.0);
        map.notes[i..]
            .iter()
            .take_while(|n| n.time < t + 5.0)
            .any(|n| matches!(n.kind, Kind::Don | Kind::Kat))
    };
    map.notes
        .iter()
        .map(|n| {
            let d = snap_of(&map.control_points, n.time);
            if !matches!(n.kind, Kind::Don | Kind::Kat) || d == 0 || !d.is_multiple_of(3) {
                return n.time;
            }
            let cp = beatmap::control_point_at(&map.control_points, n.time);
            let grid = (d / 3 * 4) as f64;
            let beats = (n.time - cp.beat_origin) / cp.beat_length;
            let lie = cp.beat_origin + (beats * grid).round() / grid * cp.beat_length;
            if hit_at(lie) { n.time } else { lie }
        })
        .collect()
}
/// A repeatable pseudo-random 0-1 for a map time (SV Chaos, Random).
#[inline]
pub fn hash01(t: f64) -> f64 {
    let mut x = (t * 16.0).round() as i64 as u64 ^ 0x9e37_79b9_7f4a_7c15;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}
/// Shapeshift: the shape of each section, by `setup`.
pub(crate) fn shape_order(map: &Beatmap, mods: Mods, setup: shapeshift::Config) -> Vec<u8> {
    if !mods.has(Mod::Shapeshift) {
        return Vec::new();
    }
    // Each map its own shuffle, the same every time it's played.
    let m = &map.meta;
    let seed = crate::beatmap::map_hash(
        format!("{}\n{}\n{}\n{}", m.artist, m.title, m.creator, m.version).as_bytes(),
    );
    setup.sequence(map.barlines.len() / setup.bars() + 1, seed)
}
/// The speed a play of `map` with `mods` at `rate` runs at (what `Game::with_mods` will use), to
/// prepare its audio with.
pub fn speed_for(map: &Beatmap, mods: Mods, rate: f64) -> Speed {
    if mods.wind().is_none() {
        return Speed::constant(rate);
    }
    let mut map = map.clone();
    apply_to_map(&mut map, mods, 0, Default::default());
    let first = map.notes.first().map(|n| n.time).unwrap_or(0.0);
    let end = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
    Speed::for_play(mods, rate.clamp(0.25, 4.0), first, end)
}
/// Changes the notes themselves for the mods that do (before anything is judged).
/// `stars::load_played` does the same to the map pp and star ratings are worked out on.
pub(crate) fn apply_to_map(map: &mut Beatmap, mods: Mods, seed: u64, half: crate::mods::HalfRule) {
    let hit = |n: &beatmap::Note| matches!(n.kind, Kind::Don | Kind::Kat);
    if mods.has(Mod::Half) {
        let points = &map.control_points;
        let beat_at = |t: f64| {
            let k = points.partition_point(|p| p.time <= t).saturating_sub(1);
            points
                .get(k)
                .map_or((0.0, 500.0), |p| (p.beat_origin, p.beat_length))
        };
        let times: Vec<f64> = map
            .notes
            .iter()
            .filter(|n| hit(n))
            .map(|n| n.time)
            .collect();
        let keep = crate::mods::half_keeps(&times, half, &beat_at);
        let mut k = 0;
        map.notes.retain(|n| {
            if !hit(n) {
                return true;
            }
            k += 1;
            keep[k - 1]
        });
    }
    for n in map.notes.iter_mut().filter(|n| hit(n)) {
        if mods.has(Mod::Swap) {
            n.kind = if n.kind == Kind::Don {
                Kind::Kat
            } else {
                Kind::Don
            };
        }
        if mods.has(Mod::Random) {
            // Seed 0: the colouring every play of the map had before seeds (old replays).
            n.kind = if random_don(n.time, seed) {
                Kind::Don
            } else {
                Kind::Kat
            };
        }
        if mods.has(Mod::NoFinishers) {
            n.big = false;
        }
        if mods.has(Mod::AllBig) {
            n.big = true;
        }
    }
    bake_speeds(map, mods);
}
