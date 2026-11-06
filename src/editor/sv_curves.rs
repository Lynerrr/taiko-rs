//! Slider-velocity curves: the easing functions Generate SV writes, and reading a chart's own
//! green lines back as the curves they were drawn with.
//!
//! Ported (by logic) from KatsuDonEditor's `chart/sv_curves.py` (MIT, used with the author's
//! permission). Gimmick structures are red lines and every red line resets SV to 1.0x, so the
//! chart's speed has to be written back after each one; `SvModel` says what that speed is at any
//! millisecond, following a ramp where the mapper drew one instead of a staircase.

use super::chart::TimingPoint;
use super::timing::sv_at;

/// The range of `Curve::Exp`'s exponent.
pub const MIN_EXPONENT: f64 = 0.1;
pub const MAX_EXPONENT: f64 = 10.0;

/// A growth function for Generate SV and the oscillating series.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Linear,
    /// `t^n`: below 1 it starts fast and settles (a slowdown), above 1 it starts slow.
    Exp(f64),
    /// Fast at the start, slow at the end (TaikoEditor's naming, the inverse of easings.net).
    SinIn,
    SinOut,
    Sin,
    /// A geometric sweep: reads as an even acceleration whatever the range.
    TrueExp,
}

impl Curve {
    pub const ALL: [Curve; 7] = [
        Curve::Linear,
        Curve::Exp(1.3),
        Curve::Exp(1.6),
        Curve::SinIn,
        Curve::SinOut,
        Curve::Sin,
        Curve::TrueExp,
    ];

    #[cfg(test)]
    pub fn name(&self) -> String {
        match self {
            Curve::Linear => "linear".into(),
            Curve::Exp(n) => format!("exp{n}"),
            Curve::SinIn => "sin_in".into(),
            Curve::SinOut => "sin_out".into(),
            Curve::Sin => "sin".into(),
            Curve::TrueExp => "true_exp".into(),
        }
    }

    /// 0..1 progress -> 0..1 eased position. `initial`/`final_` are the endpoints that are
    /// interpolated between; only `TrueExp` reads them.
    pub fn ease(&self, t: f64, initial: Option<f64>, final_: Option<f64>) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::SinIn => (t * std::f64::consts::FRAC_PI_2).sin(),
            Curve::SinOut => 1.0 - (t * std::f64::consts::FRAC_PI_2).cos(),
            Curve::Exp(n) => t.powf(n.clamp(MIN_EXPONENT, MAX_EXPONENT)),
            Curve::TrueExp => true_exp(t, initial, final_),
            Curve::Sin => -((std::f64::consts::PI * t).cos() - 1.0) / 2.0,
            Curve::Linear => t,
        }
    }
}

/// `initial * (final / initial)^t`, expressed as a 0..1 share. Linear where there is no
/// geometric reading: equal ends, a zero start, a sign change.
fn true_exp(t: f64, initial: Option<f64>, final_: Option<f64>) -> f64 {
    let (Some(a), Some(b)) = (initial, final_) else {
        return t;
    };
    if a == b || a == 0.0 {
        return t;
    }
    let ratio = b / a;
    if ratio <= 0.0 {
        return t;
    }
    let eased = (a - a * ratio.powf(t)) / (a - b);
    if eased.is_finite() { eased } else { t }
}

/// `count` SV values fanning out from `base`, alternating side. Index 0 is the base and
/// `difference` is the total amplitude the last point reaches. Per point the amplitude advances on
/// every value; per pair once per up/down pair, so each pair is symmetric about the base.
pub fn oscillating_series(
    base: f64,
    difference: f64,
    count: usize,
    curve: &Curve,
    per_pair: bool,
) -> Vec<f64> {
    match count {
        0 => return Vec::new(),
        1 => return vec![base],
        _ => {}
    }
    let levels = if per_pair { count / 2 } else { count - 1 } as f64;
    let mut values: Vec<f64> = (0..count)
        .map(|i| {
            let level = if per_pair { i.div_ceil(2) } else { i } as f64;
            let amplitude = difference * curve.ease(level / levels, None, None);
            base + if i % 2 == 1 { amplitude } else { -amplitude }
        })
        .collect();
    values[0] = base;
    values
}

/// SV values from `from` to `to` over `count` points (both ends included) along `curve`.
#[cfg(test)]
pub fn sweep(from: f64, to: f64, count: usize, curve: &Curve) -> Vec<f64> {
    match count {
        0 => Vec::new(),
        1 => vec![from],
        n => (0..n)
            .map(|i| {
                let t = i as f64 / (n - 1) as f64;
                from + (to - from) * curve.ease(t, Some(from), Some(to))
            })
            .collect(),
    }
}

/// How far a green line may sit off a curve and still be on it.
fn tolerance(value: f64) -> f64 {
    (0.002 * value.abs()).max(5e-4)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub time: f64,
    pub value: f64,
    pub source_time: f64,
    /// Changes at every red line: nothing is interpolated across an SV reset.
    pub group: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunKind {
    Step,
    Curve(Curve),
}

/// A stretch of anchors one curve passes through.
#[derive(Clone, Debug)]
pub struct Run {
    pub start: Anchor,
    pub end: Anchor,
    pub kind: RunKind,
}

impl Run {
    pub fn curve_at(&self, time: f64) -> f64 {
        let span = self.end.time - self.start.time;
        let t = if span <= 0.0 {
            0.0
        } else {
            (time - self.start.time) / span
        };
        let (a, b) = (self.start.value, self.end.value);
        let curve = match &self.kind {
            RunKind::Curve(c) => c,
            RunKind::Step => &Curve::Linear,
        };
        a + (b - a) * curve.ease(t, Some(a), Some(b))
    }
}

/// The green lines governing `start..=end`, each at the note it is for: the last green line before
/// a note with no note between them is that note's (mappers place it a few ms early).
pub fn anchors(points: &[TimingPoint], note_times: &[f64], start: f64, end: f64) -> Vec<Anchor> {
    let mut notes: Vec<f64> = note_times.to_vec();
    notes.sort_by(f64::total_cmp);
    notes.dedup();
    let first = notes.partition_point(|&t| t < start);
    let window_start = if first > 0 {
        notes[first - 1]
    } else {
        f64::NEG_INFINITY
    };
    let reds: Vec<f64> = points
        .iter()
        .filter(|p| p.is_red())
        .map(|p| p.time)
        .collect();
    let greens: Vec<&TimingPoint> = points
        .iter()
        .filter(|p| !p.is_red() && p.time > window_start && p.time <= end)
        .collect();
    let mut found = Vec::new();
    for (i, green) in greens.iter().enumerate() {
        let group = reds.partition_point(|&t| t <= green.time);
        let mut at = green.time;
        let next = notes.partition_point(|&t| t < green.time);
        if let Some(&note) = notes.get(next) {
            let later_green = greens.get(i + 1).is_some_and(|g| g.time <= note);
            let red_between = reds.partition_point(|&t| t <= note) != group;
            if !later_green && !red_between {
                at = note;
            }
        }
        found.push(Anchor {
            time: at,
            value: green.sv(),
            source_time: green.time,
            group,
        });
    }
    found
}

fn run_of(anchors: &[Anchor], curve: &Curve) -> Run {
    Run {
        start: anchors[0].clone(),
        end: anchors[anchors.len() - 1].clone(),
        kind: RunKind::Curve(curve.clone()),
    }
}

fn residual(anchors: &[Anchor], curve: &Curve, stop_at_miss: bool) -> f64 {
    let run = run_of(anchors, curve);
    let mut worst = -1.0;
    for a in &anchors[1..anchors.len() - 1] {
        let excess = (a.value - run.curve_at(a.time)).abs() - tolerance(a.value);
        if excess > worst {
            worst = excess;
            if stop_at_miss && excess > 0.0 {
                return excess;
            }
        }
    }
    worst
}

/// Generate SV's typed "Exp x": the exponent that explains the run, kept only if it fits.
fn fitted_exponent(anchors: &[Anchor]) -> Option<Curve> {
    let (first, last) = (&anchors[0], &anchors[anchors.len() - 1]);
    let span = last.time - first.time;
    let rise = last.value - first.value;
    if span <= 0.0 || rise == 0.0 {
        return None;
    }
    let mut named: Vec<f64> = anchors[1..anchors.len() - 1]
        .iter()
        .filter_map(|a| {
            let t = (a.time - first.time) / span;
            let u = (a.value - first.value) / rise;
            (t > 0.05 && t < 0.95 && u > 0.0 && u < 1.0).then(|| u.ln() / t.ln())
        })
        .collect();
    if named.is_empty() {
        return None;
    }
    named.sort_by(f64::total_cmp);
    let exponent = named[named.len() / 2];
    if !(MIN_EXPONENT..=MAX_EXPONENT).contains(&exponent) {
        return None;
    }
    let curve = Curve::Exp((exponent * 100.0).round() / 100.0);
    (residual(anchors, &curve, false) <= 0.0).then_some(curve)
}

/// What these anchors lie on. Two anchors are always a run: a step in barline mode, a ramp in
/// anti-barline mode.
fn fit(anchors: &[Anchor], anti: bool) -> Option<RunKind> {
    match anchors.len() {
        0 | 1 => return None,
        2 => {
            return Some(if anti {
                RunKind::Curve(Curve::Linear)
            } else {
                RunKind::Step
            });
        }
        _ => {}
    }
    if anchors[anchors.len() - 1].time <= anchors[0].time {
        return None;
    }
    let best = Curve::ALL
        .iter()
        .enumerate()
        .map(|(order, c)| (residual(anchors, c, true), order, c))
        .filter(|(r, _, _)| *r <= 0.0)
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    if let Some((_, _, c)) = best {
        return Some(RunKind::Curve(c.clone()));
    }
    if anchors.len() >= 4 {
        return fitted_exponent(anchors).map(RunKind::Curve);
    }
    None
}

fn monotone_reach(group: &[Anchor], start: usize) -> usize {
    let mut direction = 0;
    let mut end = start;
    for i in start + 1..group.len() {
        let mut step = group[i].value - group[i - 1].value;
        if step.abs() <= tolerance(group[i].value) {
            step = 0.0;
        }
        if step != 0.0 {
            let sign = if step > 0.0 { 1 } else { -1 };
            if direction != 0 && sign != direction {
                break;
            }
            direction = sign;
        }
        end = i;
    }
    end
}

/// The curves through `found`, each as long as one curve will reach, per group; consecutive runs
/// share their turning point as Generate SV writes them.
pub fn detect_runs(found: &[Anchor], anti: bool) -> Vec<Run> {
    let mut groups: Vec<(usize, Vec<Anchor>)> = Vec::new();
    for a in found {
        match groups.iter_mut().find(|(g, _)| *g == a.group) {
            Some((_, list)) => list.push(a.clone()),
            None => groups.push((a.group, vec![a.clone()])),
        }
    }
    let mut runs = Vec::new();
    for (_, group) in &groups {
        let mut start = 0;
        while start + 1 < group.len() {
            let reach = monotone_reach(group, start);
            let mut end = start + 1;
            let mut kind = fit(&group[start..start + 2], anti).unwrap_or(RunKind::Step);
            for candidate in (start + 2..=reach).rev() {
                if let Some(longer) = fit(&group[start..=candidate], anti) {
                    end = candidate;
                    kind = longer;
                    break;
                }
            }
            runs.push(Run {
                start: group[start].clone(),
                end: group[end].clone(),
                kind,
            });
            start = end;
        }
    }
    runs
}

/// What gimmick builders ask for the chart's speed at a time.
pub trait SvSource {
    fn value_at(&self, time: f64) -> f64;
}

/// The chart read the way osu! reads it: a step at each green line.
pub struct StepSv<'a>(pub &'a [TimingPoint]);

impl SvSource for StepSv<'_> {
    fn value_at(&self, time: f64) -> f64 {
        sv_at(self.0, time)
    }
}

/// The chart's SV as a function of time. At every note exactly the value osu! reads there; between
/// two neighbouring notes a ramp only if a green line sits in that gap, along the run's curve;
/// every other gap holds the step.
pub struct SvModel {
    runs: Vec<Run>,
    original: Vec<TimingPoint>,
    green_times: Vec<f64>,
    red_times: Vec<f64>,
    note_times: Vec<f64>,
}

impl SvModel {
    pub fn new(mut runs: Vec<Run>, original: &[TimingPoint], note_times: &[f64]) -> SvModel {
        runs.sort_by(|a, b| a.start.time.total_cmp(&b.start.time));
        let mut notes = note_times.to_vec();
        notes.sort_by(f64::total_cmp);
        notes.dedup();
        SvModel {
            runs,
            original: original.to_vec(),
            green_times: original
                .iter()
                .filter(|p| !p.is_red())
                .map(|p| p.time)
                .collect(),
            red_times: original
                .iter()
                .filter(|p| p.is_red())
                .map(|p| p.time)
                .collect(),
            note_times: notes,
        }
    }

    pub fn build(
        points: &[TimingPoint],
        note_times: &[f64],
        start: f64,
        end: f64,
        anti: bool,
    ) -> SvModel {
        let runs = detect_runs(&anchors(points, note_times, start, end), anti);
        SvModel::new(runs, points, note_times)
    }

    fn run_at(&self, time: f64) -> Option<&Run> {
        let index = self.runs.partition_point(|r| r.start.time <= time);
        let lo = index.saturating_sub(2);
        self.runs[lo..index]
            .iter()
            .rev()
            .find(|r| r.start.time <= time && time <= r.end.time)
    }
}

impl SvSource for SvModel {
    fn value_at(&self, time: f64) -> f64 {
        let step = sv_at(&self.original, time);
        let Some(run) = self.run_at(time) else {
            return step;
        };
        if run.kind == RunKind::Step {
            return step;
        }
        let at = self.note_times.partition_point(|&t| t <= time);
        let Some(index) = at.checked_sub(1) else {
            return step;
        };
        if index + 1 >= self.note_times.len() || self.note_times[index] == time {
            return step;
        }
        let (left, right) = (self.note_times[index], self.note_times[index + 1]);
        let count = |list: &[f64]| {
            list.partition_point(|&t| t <= right) - list.partition_point(|&t| t <= left)
        };
        if count(&self.green_times) == 0 || count(&self.red_times) > 0 {
            return step;
        }
        let (start, end) = (sv_at(&self.original, left), sv_at(&self.original, right));
        let (cl, cr) = (run.curve_at(left), run.curve_at(right));
        let share = if (cr - cl).abs() > 1e-12 {
            (run.curve_at(time) - cl) / (cr - cl)
        } else {
            (time - left) / (right - left)
        };
        (start + (end - start) * share.clamp(0.0, 1.0)).max(0.01)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_curves_hit_both_ends() {
        for c in Curve::ALL {
            assert!(close(c.ease(0.0, Some(1.0), Some(4.0)), 0.0), "{c:?}");
            assert!(close(c.ease(1.0, Some(1.0), Some(4.0)), 1.0), "{c:?}");
        }
    }

    #[test]
    fn sin_in_is_fast_at_the_start() {
        // TaikoEditor's naming, the inverse of easings.net.
        assert!(Curve::SinIn.ease(0.25, None, None) > 0.3);
        assert!(Curve::SinOut.ease(0.25, None, None) < 0.1);
    }

    #[test]
    fn true_exp_is_a_geometric_sweep() {
        let v = sweep(1.0, 10.0, 3, &Curve::TrueExp);
        assert!(
            close(v[1], 10f64.sqrt()),
            "midpoint of 1 -> 10 is sqrt(10), got {}",
            v[1]
        );
        // No geometric reading: linear.
        assert!(close(Curve::TrueExp.ease(0.5, Some(0.0), Some(2.0)), 0.5));
        assert!(close(Curve::TrueExp.ease(0.5, Some(-1.0), Some(2.0)), 0.5));
    }

    #[test]
    fn every_curve_has_its_own_name() {
        let mut names: Vec<String> = Curve::ALL.iter().map(Curve::name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Curve::ALL.len());
    }

    #[test]
    fn the_oscillating_series_matches_the_documented_examples() {
        let per_point = oscillating_series(1.40, 0.12, 7, &Curve::Exp(1.6), false);
        let rounded: Vec<f64> = per_point
            .iter()
            .map(|v| (v * 1000.0).round() / 1000.0)
            .collect();
        assert_eq!(rounded, [1.400, 1.407, 1.379, 1.440, 1.337, 1.490, 1.280]);
        let per_pair = oscillating_series(1.0, 0.06, 7, &Curve::Linear, true);
        let rounded: Vec<f64> = per_pair
            .iter()
            .map(|v| (v * 100.0).round() / 100.0)
            .collect();
        assert_eq!(rounded, [1.00, 1.02, 0.98, 1.04, 0.96, 1.06, 0.94]);
        assert_eq!(
            oscillating_series(2.0, 1.0, 1, &Curve::Linear, false),
            [2.0]
        );
        assert!(oscillating_series(2.0, 1.0, 0, &Curve::Linear, false).is_empty());
    }

    fn chart(sv: &[(f64, f64)]) -> Vec<TimingPoint> {
        let mut v = vec![TimingPoint::red(0.0, 120.0)];
        v.extend(sv.iter().map(|&(t, s)| TimingPoint::green(t, s)));
        v
    }

    #[test]
    fn a_green_line_belongs_to_the_note_after_it() {
        let pts = chart(&[(990.0, 1.5), (1990.0, 2.0)]);
        let found = anchors(&pts, &[1000.0, 2000.0], 1000.0, 2000.0);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].time, found[0].source_time), (1000.0, 990.0));
        assert_eq!(found[1].time, 2000.0);
    }

    #[test]
    fn a_generated_sweep_is_detected_as_its_curve() {
        let values = sweep(1.0, 2.0, 6, &Curve::Sin);
        let sv: Vec<(f64, f64)> = values
            .iter()
            .enumerate()
            .map(|(i, &v)| (1000.0 + 100.0 * i as f64 - 10.0, v))
            .collect();
        let notes: Vec<f64> = (0..6).map(|i| 1000.0 + 100.0 * i as f64).collect();
        let pts = chart(&sv);
        let runs = detect_runs(&anchors(&pts, &notes, 1000.0, 1500.0), false);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, RunKind::Curve(Curve::Sin));
        assert_eq!((runs[0].start.time, runs[0].end.time), (1000.0, 1500.0));
    }

    #[test]
    fn the_model_reads_osus_step_at_a_note_and_a_ramp_between_notes() {
        let sv = [(990.0, 1.0), (1090.0, 2.0)];
        let pts = chart(&sv);
        let notes = [1000.0, 1100.0];
        let model = SvModel::build(&pts, &notes, 1000.0, 1100.0, true);
        // At a note: exactly what osu! reads (step).
        assert!(close(model.value_at(1000.0), 1.0));
        assert!(close(model.value_at(1100.0), 2.0));
        // Between two notes where a green line sits: a ramp.
        let mid = model.value_at(1050.0);
        assert!(mid > 1.0 && mid < 2.0, "ramp expected, got {mid}");
        // Outside any run: the step.
        assert!(close(model.value_at(5000.0), 2.0));
        assert!(close(StepSv(&pts).value_at(1050.0), 1.0));
    }
}
