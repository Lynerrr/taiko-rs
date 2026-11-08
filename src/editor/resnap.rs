//! Snapping a map back onto its grid (Timing > Snap notes to the grid), as Mapping Tools' Map
//! Cleaner does it: every object goes to the nearest tick of the beat divisions picked, a note
//! just before a red line goes onto the line, and drumroll and denden ends are snapped too.
//!
//! It is careful where a blind resnap breaks maps:
//! - an object farther from every tick than `max_shift` was put there on purpose (a gimmick, a
//!   ghost note) and stays;
//! - a green line on a note's millisecond goes with the note, so the note keeps its speed;
//! - a green line close to another line is part of an SV pattern (a wall, a spike) and stays;
//! - no note lands on another note (fakes stack freely, on notes and on each other), and no line
//!   on another line: of two notes that would, the one with the smaller move goes;
//! - a drumroll never ends past the next note.
//!
//! Positions are written the way stable writes a snapped one (floored), so a snapped note sits
//! exactly where a fresh click on that tick would put it. The whole thing is one undo step.

use std::collections::HashMap;

use super::chart::{HitObject, Taiko, TimingPoint};
use super::history::Command;
use super::state::Editor;
use super::timing;

/// The beat divisions offered, coarsest first.
pub const DIVISORS: [u32; 11] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 16];

/// The largest moves offered, in ms (the last: any distance).
pub const SHIFTS: [f64; 5] = [2.0, 5.0, 10.0, 20.0, f64::INFINITY];

/// A green line with another line this close is part of a pattern and is never snapped alone.
pub const PATTERN_MS: f64 = 10.0;

/// A tick this close before the next red line means the red line (Map Cleaner's rule).
const RED_PULL_MS: f64 = 10.0;

#[derive(Clone, Debug, PartialEq)]
pub struct SnapOpts {
    /// The divisions an object may land on (any of them: a 1/3 note isn't pulled to 1/4).
    pub divisors: Vec<u32>,
    /// Objects farther than this from every tick stay where they are.
    pub max_shift: f64,
    /// Only the selected notes (and the lines among them), when any are selected.
    pub only_selected: bool,
    /// Snap the ends of drumrolls and dendens too.
    pub ends: bool,
    /// Snap the green lines that are on no note.
    pub greens: bool,
}

impl Default for SnapOpts {
    /// Map Cleaner's default (1/16 and 1/12), a 10 ms reach.
    fn default() -> SnapOpts {
        SnapOpts {
            divisors: vec![1, 2, 3, 4, 6, 8, 12, 16],
            max_shift: 10.0,
            only_selected: true,
            ends: true,
            greens: true,
        }
    }
}

/// What a snap would do (and, applied, did).
#[derive(Clone, Debug, Default)]
pub struct SnapPlan {
    pub objects_before: Vec<HitObject>,
    pub objects_after: Vec<HitObject>,
    pub lines_before: Vec<TimingPoint>,
    pub lines_after: Vec<TimingPoint>,
    /// Objects looked at.
    pub looked: usize,
    /// Objects whose start moves.
    pub moved: usize,
    /// Drumrolls and dendens whose end is snapped.
    pub ends: usize,
    /// Green lines moved (with their note or on their own).
    pub lines: usize,
    /// The largest move of a start, in ms.
    pub largest: f64,
    /// Objects left alone: farther than the reach from every tick.
    pub far: usize,
    /// Objects left alone: they would land on another.
    pub clashes: usize,
    /// Green lines left alone: part of a pattern of close lines.
    pub patterns: usize,
    /// Only the selection was looked at.
    pub selection: bool,
}

impl SnapPlan {
    pub fn is_empty(&self) -> bool {
        self.objects_after.is_empty() && self.lines_after.is_empty()
    }
}

/// The exact tick nearest `t` among `divisors` (coarsest wins a tie), or the next red line when
/// the tick is within 10 ms before it. `None` without a red line.
pub fn nearest_tick(points: &[TimingPoint], t: f64, divisors: &[u32]) -> Option<f64> {
    let red = timing::active_red(points, t)?;
    let mut best: Option<(f64, f64)> = None;
    for &d in divisors {
        let tick = timing::snap_time(points, t, d);
        let dist = (t - tick).abs();
        if best.is_none_or(|(bd, _)| dist < bd - 1e-9) {
            best = Some((dist, tick));
        }
    }
    let (_, mut tick) = best?;
    let after = points.partition_point(|p| p.time <= t);
    if let Some(next) = points[after..]
        .iter()
        .find(|p| p.is_red() && p.beat_length > 0.0 && p.beat_length.is_finite())
        && tick > red.time + RED_PULL_MS
        && tick >= next.time - RED_PULL_MS
    {
        tick = next.time;
    }
    Some(tick)
}

/// Where `t` snaps to within `reach`: the millisecond stable would write for the tick, or `None`
/// when no tick is that close.
fn snap_within(points: &[TimingPoint], t: f64, divisors: &[u32], reach: f64) -> Option<f64> {
    let tick = nearest_tick(points, t, divisors)?;
    ((t - tick).abs() <= reach).then(|| timing::osu_snap_ms(tick) as f64)
}

impl Editor {
    /// What `opts` would do to the chart, without doing it.
    pub fn plan_resnap(&self, opts: &SnapOpts) -> SnapPlan {
        let mut divisors = opts.divisors.clone();
        divisors.sort_unstable();
        divisors.dedup();
        let mut plan = SnapPlan::default();
        if divisors.is_empty() {
            return plan;
        }
        let points = &self.chart.timing;
        let selection = opts.only_selected && !self.selection.is_empty();
        plan.selection = selection;
        let objs = &self.chart.objects;
        let in_scope = |o: &HitObject| !selection || self.selection.contains(&o.id);

        // Starts: where each object in scope would go (its own time: it stays).
        let mut target: Vec<f64> = objs.iter().map(|o| o.time).collect();
        let mut moving: Vec<bool> = vec![false; objs.len()];
        for (i, o) in objs.iter().enumerate() {
            if !in_scope(o) {
                continue;
            }
            plan.looked += 1;
            match snap_within(points, o.time, &divisors, opts.max_shift) {
                Some(to) if to >= 0.0 => {
                    if to != o.time {
                        target[i] = to;
                        moving[i] = true;
                    }
                }
                Some(_) => {}
                None => {
                    if timing::active_red(points, o.time).is_some() {
                        plan.far += 1;
                    }
                }
            }
        }
        // No note on another note: of the ones that would share a millisecond, only the smallest
        // move goes (none, when one is already there and stays). Fakes stack freely.
        loop {
            let mut at: HashMap<i64, Vec<usize>> = HashMap::new();
            for (i, _) in objs.iter().enumerate().filter(|(_, o)| !o.fake) {
                at.entry(target[i].round() as i64).or_default().push(i);
            }
            let mut reverted = false;
            for list in at.values().filter(|l| l.len() > 1) {
                let movers: Vec<usize> = list.iter().copied().filter(|&i| moving[i]).collect();
                if movers.is_empty() {
                    continue;
                }
                let keep = if movers.len() < list.len() {
                    None
                } else {
                    movers.iter().copied().min_by(|&a, &b| {
                        (target[a] - objs[a].time)
                            .abs()
                            .total_cmp(&(target[b] - objs[b].time).abs())
                            .then(a.cmp(&b))
                    })
                };
                for i in movers {
                    if Some(i) != keep {
                        target[i] = objs[i].time;
                        moving[i] = false;
                        plan.clashes += 1;
                        reverted = true;
                    }
                }
            }
            if !reverted {
                break;
            }
        }

        // Lines: a green line on a note's millisecond goes with it (a note before a fake on the
        // same millisecond); the others snap by themselves.
        let mut carried: HashMap<i64, f64> = HashMap::new();
        for (i, o) in objs.iter().enumerate() {
            let key = o.time.round() as i64;
            if o.fake && carried.contains_key(&key) {
                continue;
            }
            if !o.fake || !objs.iter().any(|q| !q.fake && q.time == o.time) {
                carried.insert(key, target[i]);
            }
        }
        let (lo, hi) = if selection {
            objs.iter()
                .filter(|o| self.selection.contains(&o.id))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), o| {
                    (lo.min(o.time), hi.max(self.end_of(o)))
                })
        } else {
            (f64::NEG_INFINITY, f64::INFINITY)
        };
        let mut line_to: Vec<f64> = points.iter().map(|p| p.time).collect();
        for (i, p) in points.iter().enumerate() {
            if p.is_red() || p.time < lo || p.time > hi {
                continue;
            }
            if let Some(&to) = carried.get(&(p.time.round() as i64)) {
                line_to[i] = to;
                continue;
            }
            if !opts.greens {
                continue;
            }
            let near = |q: &TimingPoint| q.id != p.id && (q.time - p.time).abs() <= PATTERN_MS;
            if points[i.saturating_sub(64)..(i + 64).min(points.len())]
                .iter()
                .any(near)
            {
                plan.patterns += 1;
                continue;
            }
            if let Some(to) = snap_within(points, p.time, &divisors, opts.max_shift)
                && to >= 0.0
                && !points.iter().any(|q| q.id != p.id && q.time == to)
            {
                line_to[i] = to;
            }
        }
        let mut new_points = points.clone();
        for (i, p) in points.iter().enumerate() {
            if line_to[i] != p.time {
                let mut n = p.clone();
                n.time = line_to[i];
                new_points[i] = n.clone();
                plan.lines_before.push(p.clone());
                plan.lines_after.push(n);
            }
        }
        plan.lines = plan.lines_after.len();
        new_points.sort_by(|a, b| a.time.total_cmp(&b.time));

        // Ends, then everything that changed.
        let next_note = |i: usize| -> Option<f64> {
            objs[i + 1..]
                .iter()
                .enumerate()
                .filter(|(_, q)| !q.fake)
                .map(|(k, _)| target[i + 1 + k])
                .reduce(f64::min)
        };
        let sm = self.sm();
        for (i, o) in objs.iter().enumerate() {
            let delta = target[i] - o.time;
            let mut n = o.shifted(delta);
            let mut end_snapped = false;
            if opts.ends && in_scope(o) {
                match o.taiko() {
                    Taiko::Roll => {
                        let end = self.end_of(o) + delta;
                        if let Some(to) = snap_within(points, end, &divisors, opts.max_shift)
                            && to > n.time
                            && next_note(i).is_none_or(|next| to <= next)
                        {
                            let length = HitObject::length_for(
                                to - n.time,
                                o.slides(),
                                sm,
                                timing::beat_length_at(&new_points, n.time),
                                timing::sv_at(&new_points, n.time),
                            );
                            if o.length().is_some_and(|l| (l - length).abs() > 1e-6) {
                                n.set_length(length);
                                end_snapped = true;
                            }
                        }
                    }
                    Taiko::Swell => {
                        if let Some(end) = n.spinner_end()
                            && let Some(to) = snap_within(points, end, &divisors, opts.max_shift)
                            && to > n.time
                            && to != end
                        {
                            n.set_spinner_end(to);
                            end_snapped = true;
                        }
                    }
                    Taiko::Don | Taiko::Kat => {}
                }
            }
            if moving[i] || end_snapped {
                if moving[i] {
                    plan.moved += 1;
                    plan.largest = plan.largest.max(delta.abs());
                }
                plan.ends += end_snapped as usize;
                plan.objects_before.push(o.clone());
                plan.objects_after.push(n);
            }
        }
        plan
    }

    /// Snaps the chart as `opts` says, as one undo step. Returns what was done.
    pub fn resnap(&mut self, opts: &SnapOpts) -> SnapPlan {
        let plan = self.plan_resnap(opts);
        let mut steps = Vec::new();
        if !plan.lines_after.is_empty() {
            steps.push(Command::EditTiming {
                before: plan.lines_before.clone(),
                after: plan.lines_after.clone(),
            });
        }
        if !plan.objects_after.is_empty() {
            steps.push(Command::EditObjects {
                before: plan.objects_before.clone(),
                after: plan.objects_after.clone(),
            });
        }
        if !steps.is_empty() {
            self.history.run(&mut self.chart, Command::Composite(steps));
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::super::chart::Chart;
    use super::*;
    use std::path::PathBuf;

    /// 120 BPM from 1000, a second red line at 5003 (off the first one's grid).
    fn editor(objects: &str, lines: &str) -> Editor {
        let text = format!(
            "osu file format v14\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n1000,500,4,1,0,100,1,0\n{lines}5003,500,4,1,0,100,1,0\n\n[HitObjects]\n{objects}"
        );
        Editor::new(Chart::parse(&text), PathBuf::from("map.osu"))
    }

    fn note(t: i32) -> String {
        format!("256,192,{t},1,0,0:0:0:0:\n")
    }

    fn times(e: &Editor) -> Vec<f64> {
        e.chart.objects.iter().map(|o| o.time).collect()
    }

    #[test]
    fn a_note_a_hair_off_goes_on_the_grid_and_one_far_off_stays() {
        // 1126: 1 ms past a 1/4. 1145: 11.25 ms from the nearest 1/16, past the 10 ms reach.
        let mut e = editor(&(note(1126) + &note(1145) + &note(2000)), "");
        let opts = SnapOpts::default();
        let plan = e.plan_resnap(&opts);
        assert_eq!((plan.moved, plan.far, plan.largest), (1, 1, 1.0));
        e.resnap(&opts);
        assert_eq!(times(&e), vec![1125.0, 1145.0, 2000.0]);
        // A longer reach takes the far one too, onto its 1/16.
        e.resnap(&SnapOpts {
            max_shift: 20.0,
            ..SnapOpts::default()
        });
        assert_eq!(times(&e), vec![1125.0, 1156.0, 2000.0]);
        // Each run is one undo.
        assert!(e.undo());
        assert_eq!(times(&e), vec![1125.0, 1145.0, 2000.0]);
    }

    #[test]
    fn a_triplet_stays_a_triplet_and_only_ticked_divisions_count() {
        // 1/3 of a 500 ms beat: 1166.67, written 1166. 1168 is 1.3 ms past it.
        let mut e = editor(&note(1168), "");
        e.resnap(&SnapOpts::default());
        assert_eq!(times(&e), vec![1166.0]);
        // Without 1/3 (and 1/6, 1/12) it is 11.75 ms from a 1/16: left alone.
        let mut e = editor(&note(1168), "");
        let plan = e.resnap(&SnapOpts {
            divisors: vec![1, 2, 4, 8, 16],
            ..SnapOpts::default()
        });
        assert_eq!((plan.moved, plan.far), (0, 1));
    }

    #[test]
    fn a_note_just_before_a_red_line_goes_onto_it() {
        // The first line's grid has a beat at 5000; the red line is at 5003.
        let mut e = editor(&note(4999), "");
        e.resnap(&SnapOpts::default());
        assert_eq!(times(&e), vec![5003.0]);
    }

    #[test]
    fn two_notes_never_snap_onto_one_millisecond_but_a_fake_may_join_a_note() {
        let mut e = editor(&(note(1124) + &note(1126)), "");
        let plan = e.resnap(&SnapOpts::default());
        assert_eq!((plan.moved, plan.clashes), (1, 1));
        // Both are 1 ms off 1125: the first goes, the second stays.
        assert_eq!(times(&e), vec![1125.0, 1126.0]);
        // A note already on the tick stays, and the one beside it keeps off it.
        let mut e = editor(&(note(1125) + &note(1126)), "");
        let plan = e.resnap(&SnapOpts::default());
        assert_eq!((plan.moved, plan.clashes), (0, 1));
        // A fake goes onto the note's millisecond.
        let mut e = editor(&(note(1125) + &note(1126)), "");
        e.chart.gimmick = Some(Default::default());
        e.chart.objects[1].fake = true;
        let plan = e.resnap(&SnapOpts::default());
        assert_eq!((plan.moved, plan.clashes), (1, 0));
        assert_eq!(times(&e), vec![1125.0, 1125.0]);
    }

    #[test]
    fn a_green_line_on_a_note_goes_with_it_and_a_pattern_stays() {
        // A line on the note at 1126; a free line at 2002; a wall of two lines at 3001 and 3003.
        let lines = "1126,-50,4,1,0,100,0,0\n2002,-80,4,1,0,100,0,0\n3001,-10,4,1,0,100,0,0\n3003,-100,4,1,0,100,0,0\n";
        let mut e = editor(&(note(1126) + &note(4000)), lines);
        let plan = e.resnap(&SnapOpts::default());
        assert_eq!((plan.lines, plan.patterns), (2, 2));
        let greens: Vec<f64> = e
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red())
            .map(|p| p.time)
            .collect();
        assert_eq!(greens, vec![1125.0, 2000.0, 3001.0, 3003.0]);
        // Without green lines, the one on the note still goes with it.
        let mut e = editor(&note(1126), lines);
        e.resnap(&SnapOpts {
            greens: false,
            ..SnapOpts::default()
        });
        let greens: Vec<f64> = e
            .chart
            .timing
            .iter()
            .filter(|p| !p.is_red())
            .map(|p| p.time)
            .collect();
        assert_eq!(greens, vec![1125.0, 2002.0, 3001.0, 3003.0]);
    }

    #[test]
    fn roll_and_denden_ends_snap_and_a_roll_never_runs_into_the_next_note() {
        // A roll from 1000 written 2 ms longer than a beat (1.4 * 100 px per beat), a denden to
        // 2252, and a note at 3000.
        let roll = "256,192,1000,2,0,L|396:192,1,140.56\n";
        let swell = "256,192,2000,12,0,2252,0:0:0:0:\n";
        let mut e = editor(&(roll.to_string() + swell + &note(3000)), "");
        let plan = e.resnap(&SnapOpts::default());
        assert_eq!(plan.ends, 2);
        let roll = &e.chart.objects[0];
        assert!((e.end_of(roll) - 1500.0).abs() < 0.01);
        assert_eq!(e.chart.objects[1].spinner_end(), Some(2250.0));
    }

    #[test]
    fn only_the_selection_is_snapped_when_asked() {
        let mut e = editor(&(note(1126) + &note(2001)), "");
        e.selection = [e.chart.objects[1].id].into();
        e.resnap(&SnapOpts::default());
        assert_eq!(times(&e), vec![1126.0, 2000.0]);
    }
}
