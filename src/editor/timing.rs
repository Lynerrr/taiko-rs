//! Where things sit in time: the timing section in force, the snap grid, and the whole
//! milliseconds osu!stable commits positions to.
//!
//! All of it takes the chart's timing points sorted by time (`Chart::timing` is kept that way)
//! and finds its place by binary search: a gimmick chart has tens of thousands of points and
//! these run per note per frame.

use super::chart::{DEFAULT_BEAT_LENGTH, TimingPoint};

/// Every beat division the editor offers; Alt+wheel steps along this list.
pub const SNAP_DIVISORS: [u32; 22] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 27, 32, 36, 48, 64,
];

/// The divisors an object's own grid is looked for in, coarsest first, so a note on a beat reads
/// as 1/1 rather than 1/16.
pub const OWN_DIVISORS: [u32; 8] = [1, 2, 3, 4, 6, 8, 12, 16];

/// How far from a gridline an object counts as on it. Stable writes a snapped position as a whole
/// millisecond, so one on the grid is always under one millisecond off.
pub const ON_GRID_MS: f64 = 1.0;

/// Nearest whole millisecond, halves up. For a position that isn't on a beat (the millisecond
/// under the pointer on a free placement).
#[cfg(test)]
pub fn osu_round(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

/// The whole millisecond stable's editor commits a beat position to: **down**.
///
/// Stable casts the position to an int, and an editor time is never negative, so it truncates.
/// Rounding instead writes half the notes one millisecond later than stable would for the same
/// snap, which is exactly the kind of drift that breaks gimmicks relying on two objects sharing
/// a millisecond. Every millisecond the editor commits to a beat goes through here.
pub fn osu_snap_ms(v: f64) -> i64 {
    v.floor() as i64
}

fn last_at_or_before(points: &[TimingPoint], t: f64) -> Option<usize> {
    points.partition_point(|p| p.time <= t).checked_sub(1)
}

fn playable_red(p: &TimingPoint) -> bool {
    p.is_red() && p.beat_length > 0.0 && p.beat_length.is_finite()
}

/// The point in force at `t`, of either kind (the last one in file order at a shared time).
pub fn active_at(points: &[TimingPoint], t: f64) -> Option<&TimingPoint> {
    last_at_or_before(points, t).map(|i| &points[i])
}

/// The red line whose section holds `t`; before the first, the first.
pub fn active_red(points: &[TimingPoint], t: f64) -> Option<&TimingPoint> {
    if let Some(mut i) = last_at_or_before(points, t) {
        loop {
            if playable_red(&points[i]) {
                return Some(&points[i]);
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
    }
    points.iter().find(|p| playable_red(p))
}

/// The beat length at `t`, 120 BPM for a chart with no red line.
pub fn beat_length_at(points: &[TimingPoint], t: f64) -> f64 {
    active_red(points, t).map_or(DEFAULT_BEAT_LENGTH, |p| p.beat_length)
}

/// The slider velocity in force at `t`. A red line resets it to 1.
pub fn sv_at(points: &[TimingPoint], t: f64) -> f64 {
    active_at(points, t).map_or(1.0, TimingPoint::sv)
}

/// The grid position nearest `t` at 1/`divisor` of a beat, exact (not yet a whole millisecond).
/// Ties go to the even step, the way the editor's own `Math.Round` does.
pub fn snap_time(points: &[TimingPoint], t: f64, divisor: u32) -> f64 {
    let (origin, beat) = match active_red(points, t) {
        Some(p) => (p.time, p.beat_length),
        None => (0.0, DEFAULT_BEAT_LENGTH),
    };
    let step = beat / divisor.max(1) as f64;
    origin + ((t - origin) / step).round_ties_even() * step
}

/// The grid lines inside `from..to`: (time, 0 sub-beat / 1 beat / 2 bar), at 1/`snap` of a beat,
/// with none closer than 3 `ms_per_px` pixels. Each red line's grid runs to the next one; the
/// first one's also runs backwards before its line, down to the start of the song (or its own
/// time, if that is earlier), the way osu! draws it, so a song that starts before its first red
/// line can be snapped to.
pub fn grid_lines(
    points: &[TimingPoint],
    from: f64,
    to: f64,
    snap: u32,
    ms_per_px: f64,
) -> Vec<(f64, u8)> {
    let reds: Vec<&TimingPoint> = points.iter().filter(|p| playable_red(p)).collect();
    let snap = snap.max(1) as i64;
    let mut out = Vec::new();
    for (i, p) in reds.iter().enumerate() {
        let (lo, end) = (
            if i == 0 { p.time.min(0.0) } else { p.time },
            reds.get(i + 1).map_or(f64::INFINITY, |n| n.time),
        );
        if end < from || (i > 0 && p.time > to) || lo > to {
            continue;
        }
        let step = p.beat_length / snap as f64;
        if step / ms_per_px < 3.0 {
            continue;
        }
        let mut k = ((from.max(lo) - p.time) / step).ceil() as i64;
        while out.len() < 4000 {
            let t = p.time + k as f64 * step;
            if t >= to.min(end) {
                break;
            }
            let beat = k.rem_euclid(snap) == 0;
            let bar = beat && (k.div_euclid(snap)).rem_euclid(p.meter.max(1) as i64) == 0;
            out.push((t, if bar { 2 } else { beat as u8 }));
            k += 1;
        }
    }
    out
}

/// `bpm` stepped by `step` (+1, -0.01, ...) onto the step's own grid, in thousandths: a BPM off
/// that grid goes to the next mark in the step's direction first, so 173.998 and +0.01 make 174
/// (not 174.008), and a detected tempo a hair off a whole number can be fixed with the buttons.
pub fn stepped_bpm(bpm: f64, step: f64) -> f64 {
    let size = step.abs();
    if size <= 0.0 || !bpm.is_finite() {
        return bpm;
    }
    let k = bpm / size;
    let near = k.round();
    let marks = if (k - near).abs() < 1e-6 {
        near + step.signum()
    } else if step > 0.0 {
        k.ceil()
    } else {
        k.floor()
    };
    (marks * size * 1000.0).round() / 1000.0
}

/// A detected tempo within a hair of a whole number is taken as that number: songs are made at
/// whole BPMs, and 173.998 instead of 174 only puts the grid off over a long song.
pub fn whole_bpm_if_near(bpm: f64) -> f64 {
    let whole = bpm.round();
    if (bpm - whole).abs() <= 0.02 {
        whole
    } else {
        bpm
    }
}

/// The red line `by_beats` beats (of its own length) away from `p`, as a time: moved by whole
/// bars or beats, in whole milliseconds the way the editor commits a time.
pub fn moved_by_beats(p: &TimingPoint, by_beats: f64) -> f64 {
    (p.time + by_beats * p.beat_length).round()
}

/// The millisecond a note placed at `t` on 1/`divisor` lands on.
pub fn snapped_ms(points: &[TimingPoint], t: f64, divisor: u32) -> i64 {
    osu_snap_ms(snap_time(points, t, divisor))
}

/// The coarsest divisor with a gridline within `tolerance` ms of `t`.
///
/// A grid whose lines are closer than twice the tolerance matches everywhere, so it says nothing
/// about the object: the search stops before those. That bound keeps a gimmick section's 60000
/// BPM line (a 1 ms beat) from calling everything on it snapped.
pub fn own_divisor(points: &[TimingPoint], t: f64, tolerance: f64) -> Option<u32> {
    let beat = beat_length_at(points, t);
    for d in OWN_DIVISORS {
        if beat / d as f64 <= 2.0 * tolerance {
            break;
        }
        if (t - snap_time(points, t, d)).abs() < tolerance {
            return Some(d);
        }
    }
    None
}

/// Where Resnap puts an object: the millisecond for the nearest line of the object's own grid,
/// or none when it is on no grid within reach (put there on purpose, so left alone).
pub fn resnap(points: &[TimingPoint], t: f64) -> Option<i64> {
    let d = own_divisor(points, t, 2.0 * ON_GRID_MS)?;
    Some(osu_snap_ms(snap_time(points, t, d)))
}

/// One snap division either side of `t` (one beat at divisor 1): what a wheel notch moves by.
///
/// The arithmetic stays in exact fractional milliseconds; only the answer is made whole, so a
/// run of notches doesn't drift. A step forward stops at the next red line (usually off the
/// grid: stepping over it would skip the downbeat it marks), and a step back from a red line
/// goes to the last gridline of the section before.
pub fn step(points: &[TimingPoint], t: f64, forward: bool, divisor: u32) -> f64 {
    let Some(sec) = active_red(points, t) else {
        let beat = DEFAULT_BEAT_LENGTH / divisor.max(1) as f64;
        let s = (t / beat).round_ties_even() * beat;
        return osu_snap_ms((if forward { s + beat } else { s - beat }).max(0.0)) as f64;
    };
    let d = divisor.max(1) as f64;
    let size = sec.beat_length / d;
    let snapped = snap_time(points, t, divisor);
    let target = if forward {
        let mut target = snapped + size;
        let from = points.partition_point(|p| p.time <= t);
        if let Some(next) = points[from..].iter().find(|p| playable_red(p))
            && next.time < target
        {
            target = next.time;
        }
        target
    } else if snapped <= sec.time {
        match active_red(points, sec.time - 1e-6).filter(|p| p.id != sec.id) {
            Some(prev) => {
                let size = prev.beat_length / d;
                prev.time + (((sec.time - prev.time) / size - 1e-6).ceil() - 1.0) * size
            }
            None => snapped - size,
        }
    } else {
        snapped - size
    };
    osu_snap_ms(target.max(0.0)) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(list: &[(f64, f64, bool)]) -> Vec<TimingPoint> {
        let mut v: Vec<TimingPoint> = list
            .iter()
            .map(|&(t, b, red)| TimingPoint::new(t, b, red))
            .collect();
        for (i, p) in v.iter_mut().enumerate() {
            p.id = i as u64 + 1;
        }
        v
    }

    #[test]
    fn stable_commits_a_beat_position_down_not_to_nearest() {
        // 315.789 ms beat: a note a hair under a gridline stays on the ms below it.
        assert_eq!(osu_snap_ms(631.578), 631);
        assert_eq!(osu_snap_ms(631.999), 631);
        assert_eq!(osu_snap_ms(632.0), 632);
        assert_eq!(osu_round(631.5), 632);
        assert_eq!(osu_round(631.49), 631);
    }

    #[test]
    fn snapping_goes_to_the_even_step_on_a_tie() {
        let p = pts(&[(0.0, 500.0, true)]);
        // Exactly between steps 0 and 1 of 1/1: the even one, 0.
        assert_eq!(snap_time(&p, 250.0, 1), 0.0);
        assert_eq!(snap_time(&p, 750.0, 1), 1000.0);
        assert_eq!(snap_time(&p, 260.0, 1), 500.0);
        assert_eq!(snap_time(&p, 1000.0, 4), 1000.0);
    }

    #[test]
    fn the_grid_follows_the_red_line_not_the_green_ones() {
        let p = pts(&[
            (100.0, 400.0, true),
            (1230.0, -50.0, false),
            (3000.0, 600.0, true),
        ]);
        assert_eq!(snap_time(&p, 1700.0, 1), 1700.0);
        assert_eq!(snap_time(&p, 3650.0, 1), 3600.0);
        assert_eq!(sv_at(&p, 2000.0), 2.0);
        assert_eq!(sv_at(&p, 3100.0), 1.0, "a red line resets SV");
        assert_eq!(
            beat_length_at(&p, 0.0),
            400.0,
            "before the first line, the first"
        );
    }

    #[test]
    fn a_wheel_step_stops_at_the_next_red_line_and_steps_back_into_the_last_section() {
        let p = pts(&[(0.0, 500.0, true), (1100.0, 400.0, true)]);
        // 1000 + a beat would be 1500, but the red line at 1100 is next.
        assert_eq!(step(&p, 1000.0, true, 1), 1100.0);
        // Stepping back from the red line: the last gridline of the section before it.
        assert_eq!(step(&p, 1100.0, false, 1), 1000.0);
        assert_eq!(step(&p, 1500.0, false, 1), 1100.0);
        assert_eq!(step(&p, 1500.0, true, 1), 1900.0);
        assert_eq!(step(&p, 0.0, false, 1), 0.0, "never before the start");
    }

    #[test]
    fn a_one_ms_gimmick_beat_is_on_no_grid() {
        let p = pts(&[(0.0, 1.0, true)]);
        assert_eq!(own_divisor(&p, 777.0, 1.0), None);
        let p = pts(&[(0.0, 500.0, true)]);
        assert_eq!(own_divisor(&p, 1000.0, 1.0), Some(1));
        assert_eq!(own_divisor(&p, 1250.0, 1.0), Some(2));
        assert_eq!(own_divisor(&p, 1124.5, 1.0), Some(4));
        assert_eq!(resnap(&p, 1001.0), Some(1000));
        assert_eq!(resnap(&p, 1060.0), None, "put there on purpose");
    }

    #[test]
    fn the_grid_runs_backwards_from_the_first_red_line_to_the_song_start() {
        let p = pts(&[(1000.0, 500.0, true), (3000.0, 400.0, true)]);
        let g = grid_lines(&p, -500.0, 1600.0, 1, 1.0);
        let times: Vec<f64> = g.iter().map(|x| x.0).collect();
        assert_eq!(times, vec![0.0, 500.0, 1000.0, 1500.0]);
        // Bars of 4 count back from the line too: 1000 - 4 * 500 = -1000 is a bar, 0 is a beat.
        assert_eq!(g.iter().map(|x| x.1).collect::<Vec<_>>(), vec![1, 1, 2, 1]);
        // Snapping there uses the same grid.
        assert_eq!(snap_time(&p, 740.0, 1), 500.0);
        assert_eq!(snap_time(&p, 760.0, 1), 1000.0);
        assert_eq!(snap_time(&p, 100.0, 2), 0.0);
        // A first line at a negative time starts the grid there.
        let n = pts(&[(-300.0, 500.0, true)]);
        let g = grid_lines(&n, -1000.0, 800.0, 1, 1.0);
        assert_eq!(
            g.iter().map(|x| x.0).collect::<Vec<_>>(),
            vec![-300.0, 200.0, 700.0]
        );
        assert_eq!(g[0].1, 2);
        // The bars of a middle line sit on its own downbeat.
        let g = grid_lines(&p, 3000.0, 3900.0, 1, 1.0);
        assert_eq!(g.iter().map(|x| x.1).collect::<Vec<_>>(), vec![2, 1, 1]);
    }

    #[test]
    fn a_line_moves_by_bars_and_beats_of_its_own_length() {
        let mut line = TimingPoint::new(1000.0, 333.333, true);
        line.meter = 3;
        assert_eq!(moved_by_beats(&line, 1.0), 1333.0);
        assert_eq!(moved_by_beats(&line, -3.0), 0.0);
        assert_eq!(moved_by_beats(&line, -6.0), -1000.0, "past zero is allowed");
    }

    #[test]
    fn bpm_steps_land_on_their_own_grid() {
        assert_eq!(stepped_bpm(173.998, 0.01), 174.0);
        assert_eq!(stepped_bpm(173.998, -0.01), 173.99);
        assert_eq!(stepped_bpm(173.998, 1.0), 174.0);
        assert_eq!(stepped_bpm(173.998, -1.0), 173.0);
        assert_eq!(stepped_bpm(174.0, 0.01), 174.01);
        assert_eq!(stepped_bpm(120.0, 1.0), 121.0);
        assert_eq!(stepped_bpm(121.0, -0.01), 120.99);
        assert_eq!(stepped_bpm(150.0, 0.001), 150.001);
        assert_eq!(whole_bpm_if_near(173.998), 174.0);
        assert_eq!(whole_bpm_if_near(174.5), 174.5);
        assert_eq!(whole_bpm_if_near(173.95), 173.95);
    }
}
