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
}
