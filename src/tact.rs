//! Tact's setup: the length of a step (a bar by default). With Tact the notes don't scroll
//! evenly: over each step they crawl slowly, then rush to where they'd be at its end, so the
//! lane moves in surges with the music. Judging is untouched: only where notes are drawn changes.

use std::sync::atomic::{AtomicUsize, Ordering};

use winit::keyboard::KeyCode;

use crate::render::{Canvas, Hit, accent, rgba};

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

/// Step `i`'s name.
pub fn step_name(i: usize) -> &'static str {
    match i {
        0 => tr!("2 bars", "2小節", "2 такта"),
        1 => tr!("1 bar", "1小節", "1 такт"),
        2 => tr!("1/2 bar", "1/2小節", "1/2 такта"),
        3 => tr!("1 beat", "1拍", "1 доля"),
        4 => tr!("1/2 beat", "1/2拍", "1/2 доли"),
        _ => tr!("1/4 beat", "1/4拍", "1/4 доли"),
    }
}

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

static CURRENT: AtomicUsize = AtomicUsize::new(DEFAULT);

/// The step Tact plays with (from the config).
pub fn set(i: usize) {
    CURRENT.store(i.min(STEPS.len() - 1), Ordering::Relaxed);
}

pub fn get() -> Step {
    STEPS[CURRENT.load(Ordering::Relaxed).min(STEPS.len() - 1)].1
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

/// The Tact setup (a tab of the mod customization): one row of steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub col: usize,
}

impl Setup {
    pub fn new(current: usize) -> Setup {
        Setup {
            col: current.min(STEPS.len() - 1),
        }
    }

    /// Handles a key. Returns the step picked, if one was (save it).
    pub fn key(&mut self, code: KeyCode) -> Option<usize> {
        let n = STEPS.len();
        match code {
            KeyCode::ArrowLeft => self.col = (self.col + n - 1) % n,
            KeyCode::ArrowRight => self.col = (self.col + 1) % n,
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter => return Some(self.col),
            KeyCode::KeyD => {
                self.col = DEFAULT;
                return Some(DEFAULT);
            }
            _ => {}
        }
        None
    }

    /// Draws the setup over the mod select, like Shapeshift's. `current`: the step played with.
    pub fn draw_panel(&self, c: &mut Canvas, current: usize) {
        const COLOUR: u32 = 0xFFD34D;
        let w = c.width;
        c.rect(
            0.0,
            0.0,
            w,
            480.0,
            rgba(0x000000, crate::mobile::pick(0.6, 0.97)),
        );
        let (px, py, pw, ph) = crate::mobile::setup_rect(w);
        c.round_rect(
            [px, py],
            [pw, ph],
            10.0,
            rgba(0x15151d, 0.99),
            rgba(COLOUR, 0.9),
            1.5,
        );
        let title_w = c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), "TACT");
        let hint = tr!(
            "Left/Right choose   Space pick   D default   Tab next tab   Esc back",
            "左右 選択   Space 決定   D 初期設定   Tab 次のタブ   Esc 戻る",
            "влево/вправо выбор   Space отметить   D сброс   Tab вкладка   Esc назад"
        );
        c.text(
            px + 26.0 + title_w,
            py + 16.0,
            6.0,
            rgba(0x6f6f86, 1.0),
            &crate::mobile::key_hint(6.0, pw - title_w - 42.0, hint),
        );

        let phone = crate::mobile::active();
        let (x0, y, h) = if phone {
            (px + 16.0, py + 44.0, 48.0)
        } else {
            (px + 16.0, py + 40.0, 24.0)
        };
        let label_w = crate::mobile::pick(70.0, 130.0);
        c.text(
            x0,
            y + h / 2.0 - 4.0,
            7.0,
            rgba(0x8a8aa0, 1.0),
            tr!("MOVE EVERY", "移動", "ШАГ"),
        );
        let n = STEPS.len();
        let room = px + pw - 16.0 - (x0 + label_w);
        let cw = ((room - 6.0 * (n as f32 - 1.0)) / n as f32).min(crate::mobile::pick(90.0, 150.0));
        for col in 0..n {
            let cx = x0 + label_w + col as f32 * (cw + 6.0);
            let (selected, on) = (self.col == col, current == col);
            let fill = if on {
                0x4a3f18
            } else if selected {
                0x34344a
            } else {
                0x1d1d27
            };
            let border = if selected {
                accent()
            } else if on {
                COLOUR
            } else {
                0x2a2a38
            };
            c.round_rect(
                [cx, y],
                [cw, h],
                4.0,
                rgba(fill, 1.0),
                rgba(border, 1.0),
                if selected { 1.5 } else { 1.0 },
            );
            let text_col = if on { 0xffffff } else { 0x8a8aa0 };
            c.text_centered(
                cx + cw / 2.0,
                y + h / 2.0 - 3.5,
                7.0,
                rgba(text_col, 1.0),
                &Canvas::fit(7.0, cw - 6.0, step_name(col)),
            );
            c.hit(cx, y, cw, h, Hit::TactCell(col));
        }
        let about = tr!(
            "Far off, the notes crawl through each step, then rush the rest of the way at its end. Closer in they move as usual, so each one is in the hit circle on time. A shorter step rushes more often.",
            "遠くのノーツは各ステップでゆっくり進み、終わりに一気に残りを進みます。近づくといつも通りに動くので、どのノーツも時間通りに判定円に来ます。短いほど頻繁に加速します。",
            "Вдали ноты медленно ползут на каждом шаге, а в конце рывком проходят остаток. Ближе они движутся как обычно, так что каждая нота вовремя в круге удара. Чем короче шаг, тем чаще рывки."
        );
        let mut ty = y + h + 18.0;
        let chars = ((pw - 32.0) / Canvas::char_width(7.0)).max(10.0) as usize;
        for line in crate::social::wrap(about, chars) {
            c.text(x0, ty, 7.0, rgba(0x9a9ab0, 1.0), &line);
            ty += crate::mobile::pick(12.0, 20.0);
        }
        let (by, bw, bh, size) = crate::mobile::setup_button(py + ph);
        c.button(
            px + pw - bw - 12.0,
            by,
            bw,
            bh,
            tr!("Back", "戻る", "Назад"),
            size,
            Hit::Key(KeyCode::Escape),
            true,
        );
        c.button(
            px + pw - 2.0 * bw - 18.0,
            by,
            bw,
            bh,
            tr!("Default", "初期設定", "Сброс"),
            size,
            Hit::Key(KeyCode::KeyD),
            false,
        );
    }
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
