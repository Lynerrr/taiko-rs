//! SVT: slider velocity and volume interpolated between a start and an end, onto the notes, the
//! beat grid or the green lines that are already there.
//!
//! This is Dekovner's svt-linux (itself a port of jmagine's svt-rs, v1.0.3) working on the chart in
//! memory instead of a file. What it keeps from the original:
//!
//! - Speeds are interpolated as *effective* speed (SV x BPM), so a ramp across a BPM change keeps
//!   the scroll speed on its curve; each line is written back relative to the BPM in force there.
//!   `ignore_bpm` reads the end's SV against the start's BPM instead.
//! - A new line goes at `object + offset`, but never before the red line in force or the last
//!   kiai change (a line there would change what the object plays with).
//! - Objects are taken from `start - buffer` to `end + buffer`.
//! - Lines closer together than `min_spacing` collapse to the first, and existing green lines
//!   that close to a new one are replaced by it; every other line is left alone.
//! - Volume is interpolated linearly alongside, rounded.
//! - Flat modes add to or scale the green lines already in range instead of drawing a curve.
//!
//! The result is a [`Plan`], like every other tool's: the caller previews it, commits it as one
//! undo step or drops it.

use super::chart::TimingPoint;
use super::sv_curves::{self, Curve, MAX_EXPONENT, MIN_EXPONENT, StepSv, SvSource};
use super::timing;
use super::tools::Plan;

/// The most snap lines one segment may produce (a 1 ms beat would make millions).
const MAX_SNAPS: usize = 20_000;
/// The slider velocity osu! allows.
const SV_RANGE: (f64, f64) = (0.01, 10.0);

/// One end of a segment: where, the slider velocity there and the volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Endpoint {
    pub time: f64,
    pub sv: f64,
    pub volume: i32,
}

/// What SVT calls a pair of timing points: the stretch it interpolates over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start: Endpoint,
    pub end: Endpoint,
}

/// How the speed changes across a segment.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Start to end along a curve (linear, geometric, `t^n`, sine...).
    Between(Curve),
    /// Add this to each green line in range.
    Add(f64),
    /// Multiply each green line in range by this.
    Scale(f64),
    /// Fan out from the start by `end - start`, alternating side.
    Fan { curve: Curve, per_pair: bool },
}

impl Shape {
    /// The shapes that act on the lines already there rather than on a target set.
    pub fn is_flat(&self) -> bool {
        matches!(self, Shape::Add(_) | Shape::Scale(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvtParams {
    pub shape: Shape,
    pub write_sv: bool,
    pub write_volume: bool,
    /// Targets: the notes, the beat grid, the green lines already there.
    pub hits: bool,
    pub snaps: bool,
    pub lines: bool,
    /// The grid is `numer / denom` of a beat.
    pub snap_numer: f64,
    pub snap_denom: f64,
    /// New lines go this many ms from their object (negative: before).
    pub offset: i32,
    /// Objects this far outside the segment are included, for ones not perfectly snapped.
    pub buffer: i32,
    pub min_spacing: i32,
    pub ignore_bpm: bool,
    /// Multiply the speed already there instead of replacing it.
    pub relative: bool,
}

impl Default for SvtParams {
    fn default() -> Self {
        SvtParams {
            shape: Shape::Between(Curve::Linear),
            write_sv: true,
            write_volume: false,
            hits: true,
            snaps: false,
            lines: false,
            snap_numer: 1.0,
            snap_denom: 4.0,
            offset: -1,
            buffer: 3,
            min_spacing: 3,
            ignore_bpm: false,
            relative: false,
        }
    }
}

/// A segment between two times, its ends read from the chart as it is.
pub fn segment_between(points: &[TimingPoint], a: f64, b: f64) -> Segment {
    let end = |time: f64| Endpoint {
        time,
        sv: timing::sv_at(points, time),
        volume: timing::active_at(points, time).map_or(100, |p| p.volume),
    };
    Segment {
        start: end(a),
        end: end(b),
    }
}

/// One segment for each neighbouring pair of `lines` (any order, equal times once).
pub fn segments_from_lines(lines: &[&TimingPoint]) -> Vec<Segment> {
    let mut sorted: Vec<&TimingPoint> = lines.to_vec();
    sorted.sort_by(|a, b| a.time.total_cmp(&b.time));
    sorted.dedup_by(|a, b| a.time == b.time);
    sorted
        .windows(2)
        .map(|w| {
            let end = |p: &TimingPoint| Endpoint {
                time: p.time,
                sv: p.sv(),
                volume: p.volume,
            };
            Segment {
                start: end(w[0]),
                end: end(w[1]),
            }
        })
        .collect()
}

/// The value of `curve` at `u` (0 at the start, 1 at the end; the buffer reaches a little past
/// either) between `a` and `b`. Unlike `Curve::ease` it carries on outside 0..1 as SVT does.
fn along(curve: &Curve, u: f64, a: f64, b: f64) -> f64 {
    match curve {
        Curve::Linear => a + (b - a) * u,
        Curve::TrueExp => {
            if a > 0.0 && b > 0.0 {
                a * (b / a).powf(u)
            } else {
                a + (b - a) * u
            }
        }
        Curve::Exp(n) => a + (b - a) * u.max(0.0).powf(n.clamp(MIN_EXPONENT, MAX_EXPONENT)),
        Curve::Sin => a + (b - a) * (1.0 - (std::f64::consts::PI * u).cos()) / 2.0,
        other => a + (b - a) * other.ease(u, Some(a), Some(b)),
    }
}

/// The timing lines with what SVT's loop keeps as it walks them: the last red line and the last
/// time kiai was switched, as of each one.
struct Lines<'a> {
    points: &'a [TimingPoint],
    floor: Vec<f64>,
}

impl<'a> Lines<'a> {
    fn new(points: &'a [TimingPoint]) -> Lines<'a> {
        let (mut red, mut kiai_at, mut kiai) = (0.0f64, 0.0f64, false);
        let floor = points
            .iter()
            .map(|p| {
                if p.kiai() != kiai {
                    kiai_at = p.time;
                    kiai = p.kiai();
                }
                if p.is_red() {
                    red = p.time;
                }
                red.max(kiai_at)
            })
            .collect();
        Lines { points, floor }
    }

    /// The earliest a line for an object at `t` may go.
    fn floor_at(&self, t: f64) -> f64 {
        match self.points.partition_point(|p| p.time <= t) {
            0 => 0.0,
            n => self.floor[n - 1],
        }
    }

    fn bpm_at(&self, t: f64) -> f64 {
        60_000.0 / timing::beat_length_at(self.points, t)
    }

    /// Grid positions `numer / denom` of a beat apart inside each red line's section, within
    /// `lo..=hi`. Positions before the first red line have no grid.
    fn snaps(&self, lo: i64, hi: i64, numer: f64, denom: f64) -> Vec<i64> {
        let reds: Vec<&TimingPoint> = self.points.iter().filter(|p| p.is_red()).collect();
        let mut out = Vec::new();
        for (i, red) in reds.iter().enumerate() {
            let step = red.beat_length * numer / denom;
            if !(step.is_finite() && step >= 1.0) {
                continue;
            }
            let until = reds.get(i + 1).map_or(f64::INFINITY, |n| n.time);
            if until <= lo as f64 || red.time > hi as f64 {
                continue;
            }
            let mut k = ((lo as f64 - red.time) / step).floor().max(0.0);
            loop {
                let at = red.time + k * step;
                if at >= until || at.round() > hi as f64 || out.len() >= MAX_SNAPS {
                    break;
                }
                if at.round() >= lo as f64 {
                    out.push(at.round() as i64);
                }
                k += 1.0;
            }
        }
        out
    }
}

/// What `p` would write for `segments`, over the chart's `points` and notes at `note_times`.
pub fn plan(
    points: &[TimingPoint],
    note_times: &[f64],
    segments: &[Segment],
    p: &SvtParams,
) -> Result<Plan, String> {
    if !p.write_sv && !p.write_volume {
        return Err("nothing to write: turn on SV or volume".into());
    }
    if segments.is_empty() {
        return Err("no segment: select notes or two or more timing lines".into());
    }
    if !(p.snap_numer > 0.0 && p.snap_denom > 0.0) {
        return Err("the snap must be above zero".into());
    }
    if !(0..=1000).contains(&p.min_spacing) {
        return Err("min spacing must be 0 to 1000 ms".into());
    }
    if !p.hits && !p.snaps && !p.lines && !p.shape.is_flat() {
        return Err("nothing to place on: turn on notes, snaps or lines".into());
    }
    if points.iter().all(|q| !q.is_red()) {
        return Err("the map has no red line".into());
    }
    let lines = Lines::new(points);
    let step = StepSv(points);
    let mut notes: Vec<i64> = note_times.iter().map(|t| t.round() as i64).collect();
    notes.sort_unstable();
    notes.dedup();

    let mut made: Vec<(i64, TimingPoint)> = Vec::new();
    for seg in segments {
        segment(points, &lines, &step, &notes, seg, p, &mut made)?;
    }
    made.sort_by_key(|m| m.0);
    made.dedup_by_key(|m| m.0);

    let mut kept: Vec<(i64, TimingPoint)> = Vec::new();
    let mut last = -1000;
    for (t, point) in made {
        if t - last > i64::from(p.min_spacing) {
            last = t;
            kept.push((t, point));
        }
    }
    if kept.is_empty() {
        return Err("nothing to write: no notes, snaps or lines in the segment".into());
    }
    let spacing = f64::from(p.min_spacing);
    let times: Vec<f64> = kept.iter().map(|k| k.0 as f64).collect();
    let remove = points
        .iter()
        .filter(|g| !g.is_red())
        .filter(|g| {
            let i = times.partition_point(|t| *t < g.time - spacing);
            times.get(i).is_some_and(|t| *t <= g.time + spacing)
        })
        .cloned()
        .collect();
    Ok(Plan {
        remove,
        timing: kept.into_iter().map(|k| k.1).collect(),
        objects: Vec::new(),
    })
}

fn segment(
    points: &[TimingPoint],
    lines: &Lines,
    step: &StepSv,
    notes: &[i64],
    seg: &Segment,
    p: &SvtParams,
    out: &mut Vec<(i64, TimingPoint)>,
) -> Result<(), String> {
    let (st, en) = (seg.start.time.round() as i64, seg.end.time.round() as i64);
    if en <= st {
        return Err("the end of a segment must come after its start".into());
    }
    let flat = p.shape.is_flat();
    if p.write_sv && !flat && (seg.start.sv <= 0.0 || seg.end.sv <= 0.0) {
        return Err("slider velocity must be above zero".into());
    }
    if p.write_volume
        && !((0..=100).contains(&seg.start.volume) && (0..=100).contains(&seg.end.volume))
    {
        return Err("volume must be 0 to 100".into());
    }
    let s_bpm = lines.bpm_at(st as f64);
    let e_bpm = if p.ignore_bpm {
        s_bpm
    } else {
        lines.bpm_at(en as f64)
    };
    let (s_raw, e_raw) = (seg.start.sv * s_bpm, seg.end.sv * e_bpm);
    let (lo, hi) = (st - i64::from(p.buffer), en + i64::from(p.buffer));

    // Where to put lines: each is an object's time.
    let mut at: Vec<i64> = Vec::new();
    let greens = || {
        points
            .iter()
            .filter(|g| !g.is_red())
            .map(|g| g.time.round() as i64)
            .filter(|t| (lo..=hi).contains(t))
    };
    if flat {
        at.extend(greens());
    } else {
        if p.hits {
            at.extend(notes.iter().copied().filter(|t| (lo..=hi).contains(t)));
        }
        if p.snaps {
            at.extend(lines.snaps(lo, hi, p.snap_numer, p.snap_denom));
        }
        if p.lines {
            at.extend(greens());
        }
    }
    at.sort_unstable();
    at.dedup();

    let fan = match &p.shape {
        Shape::Fan { curve, per_pair } => Some(sv_curves::oscillating_series(
            s_raw,
            e_raw - s_raw,
            at.len(),
            curve,
            *per_pair,
        )),
        _ => None,
    };
    let vol_per_ms = f64::from(seg.end.volume - seg.start.volume) / (en - st) as f64;
    for (i, &obj) in at.iter().enumerate() {
        let t = obj as f64;
        let u = (obj - st) as f64 / (en - st) as f64;
        let existing = step.value_at(t);
        let sv = if !p.write_sv {
            existing
        } else {
            let bpm = lines.bpm_at(t);
            let drawn = match &p.shape {
                Shape::Between(curve) => along(curve, u, s_raw, e_raw) / bpm,
                Shape::Fan { .. } => fan.as_ref().map_or(existing, |f| f[i] / bpm),
                Shape::Add(v) => existing + v,
                Shape::Scale(v) => existing * v,
            };
            if p.relative && !flat {
                drawn * existing
            } else {
                drawn
            }
        };
        if !sv.is_finite() {
            continue;
        }
        let sv = sv.clamp(SV_RANGE.0, SV_RANGE.1);
        let new_t = (obj + i64::from(p.offset)).max(lines.floor_at(t).round() as i64);
        let template = timing::active_at(points, t);
        let mut point = match template {
            Some(q) => TimingPoint::green_like(new_t as f64, sv, q),
            None => TimingPoint::green(new_t as f64, sv),
        };
        if p.write_volume {
            let v = f64::from(seg.start.volume) + (obj - st) as f64 * vol_per_ms;
            point.volume = v.round().clamp(0.0, 100.0) as i32;
        }
        out.push((new_t, point));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::sv_curves::{RunKind, anchors, detect_runs};

    fn base() -> Vec<TimingPoint> {
        vec![TimingPoint::red(0.0, 120.0)]
    }

    fn notes(a: i64, b: i64, every: i64) -> Vec<f64> {
        (a..=b).step_by(every as usize).map(|t| t as f64).collect()
    }

    fn seg(points: &[TimingPoint], a: f64, b: f64, sv: (f64, f64)) -> Segment {
        let mut s = segment_between(points, a, b);
        s.start.sv = sv.0;
        s.end.sv = sv.1;
        s
    }

    fn times(plan: &Plan) -> Vec<f64> {
        plan.timing.iter().map(|p| p.time).collect()
    }

    fn svs(plan: &Plan) -> Vec<f64> {
        plan.timing.iter().map(|p| p.sv()).collect()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn linear_speeds_land_one_ms_before_each_note() {
        let pts = base();
        let n = notes(1000, 2000, 125);
        let plan = plan(
            &pts,
            &n,
            &[seg(&pts, 1000.0, 2000.0, (1.0, 2.0))],
            &SvtParams::default(),
        )
        .unwrap();
        assert_eq!(plan.timing.len(), 9);
        assert_eq!(times(&plan)[0], 999.0);
        assert_eq!(times(&plan)[8], 1999.0);
        let v = svs(&plan);
        assert!(near(v[0], 1.0) && near(v[8], 2.0) && near(v[1], 1.125));
    }

    #[test]
    fn a_bpm_change_keeps_the_scroll_speed_on_its_curve_unless_ignored() {
        let mut pts = base();
        pts.push(TimingPoint::red(1500.0, 240.0));
        let n = notes(1000, 2000, 500);
        let s = seg(&pts, 1000.0, 2000.0, (1.0, 2.0));
        // 120 -> 480 effective: at 1500 it is 300, written against 240 BPM.
        let a = plan(&pts, &n, &[s], &SvtParams::default()).unwrap();
        assert!(near(svs(&a)[1], 300.0 / 240.0), "{:?}", svs(&a));
        // Ignoring the end's BPM: 120 -> 240, 180 at the middle.
        let p = SvtParams {
            ignore_bpm: true,
            ..SvtParams::default()
        };
        let b = plan(&pts, &n, &[s], &p).unwrap();
        assert!(near(svs(&b)[1], 180.0 / 240.0), "{:?}", svs(&b));
    }

    #[test]
    fn a_line_never_goes_before_the_red_line_or_a_kiai_change() {
        let mut pts = base();
        let mut kiai = TimingPoint::green(1000.0, 1.0);
        kiai.set_kiai(true);
        pts.push(kiai);
        let n = [1000.0, 1100.0];
        let plan = plan(
            &pts,
            &n,
            &[seg(&pts, 1000.0, 1100.0, (1.0, 2.0))],
            &SvtParams::default(),
        )
        .unwrap();
        // 1000 - 1 would fall before the kiai line at 1000.
        assert_eq!(times(&plan), vec![1000.0, 1099.0]);
        // At 0 nothing goes before the red line.
        let n = [0.0, 100.0];
        let pts = base();
        let at_zero = super::plan(
            &pts,
            &n,
            &[seg(&pts, 0.0, 100.0, (1.0, 2.0))],
            &SvtParams::default(),
        )
        .unwrap();
        assert_eq!(times(&at_zero)[0], 0.0);
    }

    #[test]
    fn lines_too_close_collapse_and_old_greens_beside_a_new_one_are_replaced() {
        let mut pts = base();
        pts.push(TimingPoint::green(1008.0, 1.5));
        pts.push(TimingPoint::green(1100.0, 1.5));
        let n = [1000.0, 1002.0, 1010.0];
        let plan = plan(
            &pts,
            &n,
            &[seg(&pts, 1000.0, 1010.0, (1.0, 2.0))],
            &SvtParams::default(),
        )
        .unwrap();
        // 999 stays, 1001 is 2 ms after it so goes, 1009 stays.
        assert_eq!(times(&plan), vec![999.0, 1009.0]);
        // The green at 1008 is within 3 ms of 1009 and goes; the one at 1100 stays.
        let gone: Vec<f64> = plan.remove.iter().map(|g| g.time).collect();
        assert_eq!(gone, vec![1008.0]);
    }

    #[test]
    fn volume_ramps_beside_or_instead_of_the_speed() {
        let mut pts = base();
        pts[0].volume = 100;
        let n = notes(1000, 2000, 500);
        let mut s = seg(&pts, 1000.0, 2000.0, (1.0, 1.0));
        s.start.volume = 100;
        s.end.volume = 50;
        let p = SvtParams {
            write_sv: false,
            write_volume: true,
            ..SvtParams::default()
        };
        let plan = plan(&pts, &n, &[s], &p).unwrap();
        let vols: Vec<i32> = plan.timing.iter().map(|p| p.volume).collect();
        assert_eq!(vols, vec![100, 75, 50]);
        assert!(svs(&plan).iter().all(|v| near(*v, 1.0)));
        // Nothing to write at all is an error, not an empty plan.
        let p = SvtParams {
            write_sv: false,
            ..SvtParams::default()
        };
        assert!(super::plan(&pts, &n, &[s], &p).is_err());
    }

    #[test]
    fn flat_changes_act_on_the_green_lines_already_there() {
        let mut pts = base();
        pts.push(TimingPoint::green(1000.0, 1.0));
        pts.push(TimingPoint::green(1500.0, 1.5));
        pts.push(TimingPoint::green(5000.0, 3.0));
        let s = segment_between(&pts, 1000.0, 2000.0);
        let add = SvtParams {
            shape: Shape::Add(0.5),
            ..SvtParams::default()
        };
        let a = plan(&pts, &[], &[s], &add).unwrap();
        assert_eq!(svs(&a).len(), 2);
        assert!(near(svs(&a)[0], 1.5) && near(svs(&a)[1], 2.0));
        let scale = SvtParams {
            shape: Shape::Scale(2.0),
            ..SvtParams::default()
        };
        let b = plan(&pts, &[], &[s], &scale).unwrap();
        assert!(near(svs(&b)[0], 2.0) && near(svs(&b)[1], 3.0));
        // The lines that were there are the ones replaced.
        assert_eq!(a.remove.len(), 2);
    }

    #[test]
    fn selected_lines_make_a_segment_per_neighbouring_pair() {
        let mut pts = base();
        let l = [
            TimingPoint::green(1000.0, 1.0),
            TimingPoint::green(2000.0, 2.0),
            TimingPoint::green(3000.0, 1.0),
        ];
        pts.extend(l.clone());
        let refs: Vec<&TimingPoint> = l.iter().collect();
        let segs = segments_from_lines(&refs);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[1].start.time, 2000.0);
        let n = notes(1000, 3000, 250);
        let plan = plan(&pts, &n, &segs, &SvtParams::default()).unwrap();
        // The shared end is written once.
        assert_eq!(plan.timing.len(), 9);
        assert!(near(svs(&plan)[4], 2.0));
    }

    #[test]
    fn snaps_follow_each_red_lines_beat_grid() {
        let mut pts = base();
        pts.push(TimingPoint::red(1000.0, 60.0));
        let s = seg(&pts, 0.0, 2000.0, (1.0, 2.0));
        let p = SvtParams {
            hits: false,
            snaps: true,
            snap_numer: 1.0,
            snap_denom: 2.0,
            offset: 0,
            ..SvtParams::default()
        };
        let plan = plan(&pts, &[], &[s], &p).unwrap();
        // 120 BPM: every 250 until 1000; 60 BPM: every 500 from 1000.
        assert_eq!(
            times(&plan),
            vec![0.0, 250.0, 500.0, 750.0, 1000.0, 1500.0, 2000.0]
        );
    }

    #[test]
    fn what_it_writes_is_read_back_as_the_same_curve() {
        let pts = base();
        let n = notes(1000, 3000, 125);
        for (shape, want) in [
            (Curve::Linear, Curve::Linear),
            (Curve::TrueExp, Curve::TrueExp),
            (Curve::Sin, Curve::Sin),
            (Curve::Exp(0.5), Curve::Exp(0.5)),
            (Curve::Exp(2.0), Curve::Exp(2.0)),
        ] {
            let p = SvtParams {
                shape: Shape::Between(shape.clone()),
                ..SvtParams::default()
            };
            let s = seg(&pts, 1000.0, 3000.0, (1.0, 3.0));
            let plan = plan(&pts, &n, &[s], &p).unwrap();
            let mut all = pts.clone();
            all.extend(plan.timing);
            all.sort_by(|a, b| a.time.total_cmp(&b.time));
            let found = anchors(&all, &n, 1000.0, 3000.0);
            let runs = detect_runs(&found, false);
            assert_eq!(runs.len(), 1, "{shape:?}: {runs:?}");
            assert_eq!(runs[0].kind, RunKind::Curve(want), "{shape:?}");
        }
    }

    #[test]
    fn a_fan_alternates_around_the_start_and_relative_multiplies_what_was_there() {
        let mut pts = base();
        pts.push(TimingPoint::green(0.0, 2.0));
        let n = notes(1000, 1500, 125);
        let s = seg(&pts, 1000.0, 1500.0, (1.0, 2.0));
        let fan = SvtParams {
            shape: Shape::Fan {
                curve: Curve::Linear,
                per_pair: false,
            },
            ..SvtParams::default()
        };
        let v = svs(&plan(&pts, &n, &[s], &fan).unwrap());
        assert!(near(v[0], 1.0) && v[1] > 1.0 && v[2] < 1.0, "{v:?}");
        let rel = SvtParams {
            relative: true,
            ..SvtParams::default()
        };
        let v = svs(&plan(&pts, &n, &[s], &rel).unwrap());
        assert!(near(v[0], 2.0) && near(v[4], 4.0), "{v:?}");
    }

    #[test]
    fn bad_input_is_said_not_written() {
        let pts = base();
        let n = notes(1000, 2000, 500);
        let ok = seg(&pts, 1000.0, 2000.0, (1.0, 2.0));
        let backwards = seg(&pts, 2000.0, 1000.0, (1.0, 2.0));
        assert!(plan(&pts, &n, &[backwards], &SvtParams::default()).is_err());
        let zero = seg(&pts, 1000.0, 2000.0, (0.0, 2.0));
        assert!(plan(&pts, &n, &[zero], &SvtParams::default()).is_err());
        assert!(plan(&pts, &n, &[], &SvtParams::default()).is_err());
        let none = SvtParams {
            hits: false,
            ..SvtParams::default()
        };
        assert!(plan(&pts, &n, &[ok], &none).is_err());
        assert!(plan(&[], &n, &[ok], &SvtParams::default()).is_err());
    }
}
