//! Notes that follow their timing: stable's "Move already placed notes when changing the
//! offset/BPM" (the timing setup panel's checkbox). With it on, a change to a red line's time or
//! BPM carries everything in its section along, so each note keeps its place in the beat.
//!
//! What moves: the hit objects (a denden's end too) and the green lines of every red line that
//! changed, from that red line up to the next one (before the first red line, the first one's).
//! A drumroll keeps its length in beats by itself: its written length is in beats already.

use std::collections::HashMap;

use super::chart::{HitObject, TimingPoint};
use super::history::Command;
use super::state::Editor;
use super::timing;

/// How the red lines moved: each one's time and beat length before and after, by id.
pub struct Follow<'a> {
    old: &'a [TimingPoint],
    new: &'a [TimingPoint],
    moved: HashMap<u64, (f64, f64)>,
}

impl<'a> Follow<'a> {
    /// `old` and `new` are the whole timing, sorted, before and after the change. `None` when no
    /// red line changed its time or beat length (nothing to follow).
    pub fn new(old: &'a [TimingPoint], new: &'a [TimingPoint]) -> Option<Follow<'a>> {
        let before: HashMap<u64, &TimingPoint> = old
            .iter()
            .filter(|p| p.is_red())
            .map(|p| (p.id, p))
            .collect();
        let moved: HashMap<u64, (f64, f64)> = new
            .iter()
            .filter(|p| p.is_red() && p.beat_length > 0.0 && p.beat_length.is_finite())
            .filter(|p| {
                before
                    .get(&p.id)
                    .is_some_and(|b| b.time != p.time || b.beat_length != p.beat_length)
            })
            .map(|p| (p.id, (p.time, p.beat_length)))
            .collect();
        (!moved.is_empty()).then_some(Follow { old, new, moved })
    }

    /// Where what sat at `t` goes. On a beat division (the ones the editor snaps to) it lands on
    /// the same division of the changed line, as the whole millisecond stable would write it;
    /// off the grid it keeps its fraction of a beat, rounded.
    pub fn at(&self, t: f64) -> f64 {
        let Some(red) = timing::active_red(self.old, t) else {
            return t;
        };
        let Some(&(time, beat)) = self.moved.get(&red.id) else {
            return t;
        };
        if let Some(d) = timing::own_divisor(self.old, t, timing::ON_GRID_MS) {
            let old_step = red.beat_length / d as f64;
            let k = ((t - red.time) / old_step).round();
            let exact = time + k * (beat / d as f64);
            // The tiny nudge keeps a position that is a whole millisecond from flooring to the one
            // below over the last bit of a division.
            if timing::active_red(self.new, exact).is_some_and(|p| p.id == red.id) {
                return timing::osu_snap_ms(exact + 1e-6) as f64;
            }
            return exact.round();
        }
        (time + (t - red.time) / red.beat_length * beat).round()
    }

    /// The object as it is after following.
    pub fn object(&self, o: &HitObject) -> HitObject {
        let mut n = o.clone();
        n.time = self.at(o.time);
        if let Some(end) = o.spinner_end() {
            n.set_spinner_end(self.at(end).max(n.time + 1.0));
        }
        n
    }
}

/// Edits of objects or lines: (before, after) of the ones that changed.
type Changes<T> = (Vec<T>, Vec<T>);

impl Editor {
    /// The command that changes the timing lines `before` into `after`: just that, or, with
    /// `follow_timing` on and a red line's time or BPM changed, that and everything in the
    /// changed sections following it (a composite of the lines, then the notes, always both so a
    /// drag's steps fold into one). Refuses to push a note before 0:00.
    pub(super) fn timing_command(
        &self,
        before: Vec<TimingPoint>,
        after: Vec<TimingPoint>,
    ) -> Result<Command, String> {
        if !self.follow_timing || !after.iter().any(TimingPoint::is_red) {
            return Ok(Command::EditTiming { before, after });
        }
        let mut new = self.chart.timing.clone();
        let edited: HashMap<u64, &TimingPoint> = after.iter().map(|p| (p.id, p)).collect();
        for p in &mut new {
            if let Some(a) = edited.get(&p.id) {
                *p = (*a).clone();
            }
        }
        new.sort_by(|a, b| a.time.total_cmp(&b.time));
        let Some(f) = Follow::new(&self.chart.timing, &new) else {
            return Ok(Command::EditTiming { before, after });
        };
        let (objects_before, objects_after) = self.followed_objects(&f)?;
        let (greens_before, greens_after) = followed_greens(&f, &self.chart.timing, &edited);
        let (mut lines_before, mut lines_after) = (before, after);
        lines_before.extend(greens_before);
        lines_after.extend(greens_after);
        Ok(Command::Composite(vec![
            Command::EditTiming {
                before: lines_before,
                after: lines_after,
            },
            Command::EditObjects {
                before: objects_before,
                after: objects_after,
            },
        ]))
    }

    fn followed_objects(&self, f: &Follow) -> Result<Changes<HitObject>, String> {
        let (mut b, mut a) = (Vec::new(), Vec::new());
        for o in &self.chart.objects {
            let n = f.object(o);
            if n.time < 0.0 {
                return Err("that would push notes before 0:00.000".into());
            }
            if n.time != o.time || n.spinner_end() != o.spinner_end() {
                b.push(o.clone());
                a.push(n);
            }
        }
        Ok((b, a))
    }
}

/// The green lines (other than those being edited) that follow the changed red lines.
fn followed_greens(
    f: &Follow,
    old: &[TimingPoint],
    edited: &HashMap<u64, &TimingPoint>,
) -> Changes<TimingPoint> {
    let (mut b, mut a) = (Vec::new(), Vec::new());
    for p in old {
        if p.is_red() || edited.contains_key(&p.id) {
            continue;
        }
        let to = f.at(p.time).max(0.0);
        if to != p.time {
            let mut n = p.clone();
            n.time = to;
            b.push(p.clone());
            a.push(n);
        }
    }
    (b, a)
}

#[cfg(test)]
mod tests {
    use super::super::chart::{Chart, Taiko};
    use super::*;
    use std::path::PathBuf;

    /// 120 BPM from 1000; a 1/4 note, a beat note, an off-grid note, a green line on a note and a
    /// second red line (150 BPM) at 5000 with a note on its beat 1.
    const MAP: &str = "osu file format v14\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n1000,500,4,1,0,100,1,0\n2000,-50,4,1,0,100,0,0\n5000,400,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1125,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n256,192,2320,1,0,0:0:0:0:\n256,192,3000,12,0,4000,0:0:0:0:\n256,192,5400,1,0,0:0:0:0:\n";

    fn editor(follow: bool) -> Editor {
        let mut e = Editor::new(Chart::parse(MAP), PathBuf::from("map.osu"));
        e.follow_timing = follow;
        e
    }

    fn times(e: &Editor) -> Vec<f64> {
        e.chart.objects.iter().map(|o| o.time).collect()
    }

    fn first_red(e: &Editor) -> u64 {
        e.chart.timing.iter().find(|p| p.is_red()).unwrap().id
    }

    #[test]
    fn off_the_notes_stay_put_when_the_offset_moves() {
        let mut e = editor(false);
        let id = first_red(&e);
        e.edit_timing(id, |p| p.time += 10.0, false).unwrap();
        assert_eq!(times(&e), vec![1125.0, 2000.0, 2320.0, 3000.0, 5400.0]);
    }

    #[test]
    fn with_follow_on_the_section_moves_with_its_offset_and_the_next_one_stays() {
        let mut e = editor(true);
        let id = first_red(&e);
        e.edit_timing(id, |p| p.time += 10.0, false).unwrap();
        assert_eq!(times(&e), vec![1135.0, 2010.0, 2330.0, 3010.0, 5400.0]);
        // The denden's end went along, and so did the green line on the note at 2000.
        let swell = e
            .chart
            .objects
            .iter()
            .find(|o| o.taiko() == Taiko::Swell)
            .unwrap();
        assert_eq!(swell.spinner_end(), Some(4010.0));
        assert!(
            e.chart
                .timing
                .iter()
                .any(|p| !p.is_red() && p.time == 2010.0)
        );
        // One undo takes the line and the notes back.
        assert!(e.undo());
        assert_eq!(times(&e), vec![1125.0, 2000.0, 2320.0, 3000.0, 5400.0]);
        assert!(
            e.chart
                .timing
                .iter()
                .any(|p| !p.is_red() && p.time == 2000.0)
        );
    }

    #[test]
    fn with_follow_on_a_new_bpm_keeps_every_note_on_its_beat() {
        let mut e = editor(true);
        let id = first_red(&e);
        // 120 -> 150 BPM: a beat is 400 ms.
        e.edit_timing(id, |p| p.beat_length = 400.0, false).unwrap();
        let t = times(&e);
        // 1/4 after the line: 1100. Two beats: 1800. The swell's 4 beats: 2600.
        assert_eq!(t[0], 1100.0);
        assert_eq!(t[1], 1800.0);
        // Off the grid (2.64 beats): kept as a fraction of the beat.
        assert_eq!(t[2], 2056.0);
        assert_eq!(t[3], 2600.0);
        // The next section is not this line's.
        assert_eq!(t[4], 5400.0);
    }

    #[test]
    fn a_division_lands_on_the_millisecond_a_fresh_placement_would() {
        // 190 BPM: a 1/4 is 78.947... ms; stable writes positions floored.
        let mut e = editor(true);
        let id = first_red(&e);
        e.edit_timing(id, |p| p.beat_length = 60_000.0 / 190.0, false)
            .unwrap();
        e.snap = 4;
        let note = e.chart.objects[0].time;
        assert_eq!(note, e.snapped(note));
        assert_eq!(note, 1078.0);
    }

    #[test]
    fn a_dragged_offset_with_follow_folds_into_one_undo() {
        let mut e = editor(true);
        let id = first_red(&e);
        for step in 1..=5 {
            e.edit_timing(id, |p| p.time = 1000.0 + step as f64 * 3.0, true)
                .unwrap();
        }
        e.end_drag();
        assert_eq!(times(&e)[0], 1140.0);
        assert!(e.undo());
        assert_eq!(times(&e)[0], 1125.0);
        assert!(!e.history.can_undo());
    }

    #[test]
    fn follow_refuses_to_push_notes_before_zero() {
        let mut e = editor(true);
        let id = first_red(&e);
        assert!(e.edit_timing(id, |p| p.time = -1200.0, false).is_err());
        assert_eq!(times(&e)[0], 1125.0);
    }
}
