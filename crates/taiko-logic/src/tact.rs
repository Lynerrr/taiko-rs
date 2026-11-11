//! Tact's setup: the length of a step (a bar by default). With Tact the notes don't scroll
//! evenly: over each step they crawl slowly, then rush to where they'd be at its end, so the
//! lane moves in surges with the music. Judging is untouched: only where notes are drawn changes.

/// A step: whole bars, or a part of a bar or of a beat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Bars(usize),
    /// This much of a bar.
    OfBar(f64),
    /// This much of a beat.
    OfBeat(f64),
}

/// The steps to pick from, as config.txt writes them.
pub const STEPS: [(&str, Step); 6] = [
    ("2bars", Step::Bars(2)),
    ("1bar", Step::Bars(1)),
    ("1/2bar", Step::OfBar(0.5)),
    ("1beat", Step::OfBeat(1.0)),
    ("1/2beat", Step::OfBeat(0.5)),
    ("1/4beat", Step::OfBeat(0.25)),
];

/// One bar, at first.
pub const DEFAULT: usize = 1;

/// The step index config.txt's `tact` names (the default for anything else).
pub fn parse(s: &str) -> usize {
    STEPS
        .iter()
        .position(|(key, _)| *key == s.trim())
        .unwrap_or(DEFAULT)
}

pub fn format(i: usize) -> &'static str {
    STEPS[i.min(STEPS.len() - 1)].0
}

/// The step at index `i` of `STEPS` (config.txt's `tact`), the last one past the end.
pub fn step(i: usize) -> Step {
    STEPS[i.min(STEPS.len() - 1)].1
}

/// The map time the notes stand at for map time `t`: the start of the step `t` is in, never
/// after `t`. `bar`: the bar `t` is in (its index among the barlines, start, length), `None`
/// before the first one; `barline(i)` the time of barline `i`; `beat`: the beat grid (where it's
/// counted from and a beat's length).
pub fn step_start(
    step: Step,
    t: f64,
    bar: Option<(usize, f64, f64)>,
    barline: impl Fn(usize) -> f64,
    beat: (f64, f64),
) -> f64 {
    let grid = |origin: f64, len: f64| {
        if len > 1.0 {
            origin + ((t - origin) / len).floor() * len
        } else {
            t
        }
    };
    let start = match (step, bar) {
        (Step::Bars(n), Some((i, _, _))) => barline(i - i % n.max(1)),
        (Step::OfBar(f), Some((_, start, len))) => grid(start, len * f),
        (Step::OfBeat(f), _) => grid(beat.0, beat.1 * f),
        // Before the first barline: by the beat, a bar being four of them.
        (Step::Bars(n), None) => grid(beat.0, beat.1 * 4.0 * n as f64),
        (Step::OfBar(f), None) => grid(beat.0, beat.1 * 4.0 * f),
    };
    start.min(t)
}

/// How much of a step the notes cover at an even crawl; the rest comes in a rush at its end.
const CRAWL: f64 = 0.25;
/// How sharp the rush at a step's end is: higher keeps them slower for longer, then faster.
const RUSH: i32 = 8;

/// The map time the notes are drawn at for map time `t`. Over each step they go from where they'd
/// be at its start to where they'd be at its end: a slow crawl most of the way, then a sharp rush
/// that lands them exactly there as the next step begins, so they never jump. `start(x)` is
/// `step_start` for map time `x`. Never after `t`.
pub fn shown_time(t: f64, start: impl Fn(f64) -> f64) -> f64 {
    let now = start(t);
    // The step's end: the first time that starts another one (found by halving).
    let mut hi = now + 1.0;
    while start(hi) <= now {
        if hi - now > 120_000.0 {
            return now;
        }
        hi = now + (hi - now) * 2.0;
    }
    let mut lo = t.max(now);
    while hi - lo > 1e-3 {
        let mid = (lo + hi) / 2.0;
        if start(mid) <= now {
            lo = mid
        } else {
            hi = mid
        }
    }
    let len = hi - now;
    if len <= 0.0 {
        return t;
    }
    let p = ((t - now) / len).clamp(0.0, 1.0);
    let eased = CRAWL * p + (1.0 - CRAWL) * p.powi(RUSH);
    (now + len * eased).min(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bars of 2000 ms from 1000 ms on; beats of 500 ms counted from 1000 ms.
    fn at(step: Step, t: f64) -> f64 {
        let bar = (t >= 1000.0).then(|| {
            let i = ((t - 1000.0) / 2000.0).floor() as usize;
            (i, 1000.0 + 2000.0 * i as f64, 2000.0)
        });
        step_start(
            step,
            t,
            bar,
            |i| 1000.0 + 2000.0 * i as f64,
            (1000.0, 500.0),
        )
    }

    #[test]
    fn notes_stand_at_the_start_of_each_step() {
        assert_eq!(at(Step::Bars(1), 1000.0), 1000.0);
        assert_eq!(at(Step::Bars(1), 2999.0), 1000.0);
        assert_eq!(at(Step::Bars(1), 3000.0), 3000.0);
        assert_eq!(
            at(Step::Bars(2), 4500.0),
            1000.0,
            "bars 0 and 1 are one step"
        );
        assert_eq!(at(Step::Bars(2), 5000.0), 5000.0);
        assert_eq!(at(Step::OfBar(0.5), 2400.0), 2000.0);
        assert_eq!(at(Step::OfBeat(1.0), 1740.0), 1500.0);
        assert_eq!(at(Step::OfBeat(0.25), 1740.0), 1625.0);
    }

    #[test]
    fn a_step_never_starts_after_the_time_and_never_goes_back() {
        for (_, step) in STEPS {
            let mut last = f64::NEG_INFINITY;
            let mut t = 0.0;
            while t < 12_000.0 {
                let s = at(step, t);
                assert!(s <= t && s >= last, "{step:?} at {t}: {s} after {last}");
                last = s;
                t += 7.0;
            }
        }
    }

    #[test]
    fn notes_crawl_through_a_step_then_rush_to_its_end_without_jumping() {
        let shown = |step: Step, t: f64| shown_time(t, |x| at(step, x));
        let bar = Step::Bars(1);
        // A step's start: where the notes would be then.
        assert_eq!(shown(bar, 3000.0), 3000.0);
        // Halfway: still crawling, well behind the real time.
        let half = shown(bar, 4000.0);
        assert!(half > 3000.0 && half < 3000.0 + 0.2 * 2000.0, "{half}");
        // Nearly at the end: the rush has them close to it.
        let late = shown(bar, 4980.0);
        assert!(late > 3000.0 + 0.9 * 2000.0 && late <= 4980.0, "{late}");
        // They always move forward, never ahead of the real time, and the next step takes
        // over from where the last one ended (no jump).
        for (_, step) in STEPS {
            let mut last = f64::NEG_INFINITY;
            let mut t = 1000.0;
            while t < 12_000.0 {
                let s = shown(step, t);
                assert!(s <= t + 1e-9, "{step:?} at {t}: {s}");
                assert!(s >= last - 1e-9, "{step:?} at {t}: {s} after {last}");
                // At most the rush's top speed (6.25x) per ms: no jump.
                assert!(
                    last == f64::NEG_INFINITY || s - last < 7.0,
                    "{step:?} jumps at {t}: {last} -> {s}"
                );
                last = s;
                t += 1.0;
            }
        }
    }

    #[test]
    fn the_config_names_round_trip() {
        for i in 0..STEPS.len() {
            assert_eq!(parse(format(i)), i);
        }
        assert_eq!(parse("nonsense"), DEFAULT);
    }
}
