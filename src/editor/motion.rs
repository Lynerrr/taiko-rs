//! Note motion in the editor (F4 Effects, Motion page): the notes picked on the timeline get a
//! group of their own, and that group an approach path, a pace along it and when it is on.
//!
//! A note flies with one group at a time (the runtime takes the first that holds it), so giving
//! notes a path takes them out of any other group they were in.

use super::state::Editor;
use crate::tko::gimmick::{Group, MAX_GROUPS};
use crate::tko::path::NotePath;
use crate::tko::track::{Ease, Key, Track};
use std::collections::BTreeMap;

/// How far a note's time may be from one a group holds (the chart's whole milliseconds).
const SAME: f64 = 0.5;

/// What "Copy motion" keeps: a group's path and keyframes, to give other notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    pub path: Option<NotePath>,
    pub tracks: BTreeMap<String, Track>,
}

/// Ready-made paths: a name and the points notes come in by (playfield units from the drum, x
/// right, y down; the drum ends each one).
pub const PATHS: [(&str, &[[f64; 2]]); 9] = [
    ("From above", &[[0.0, -190.0]]),
    ("From below", &[[0.0, 280.0]]),
    ("From behind", &[[-155.0, 0.0]]),
    ("From the corner", &[[640.0, -190.0]]),
    ("Arc over", &[[600.0, 0.0], [330.0, -150.0], [90.0, -60.0]]),
    ("Drop", &[[330.0, -190.0], [150.0, -175.0], [40.0, -90.0]]),
    (
        "Zigzag",
        &[
            [620.0, 0.0],
            [500.0, -90.0],
            [380.0, 90.0],
            [260.0, -90.0],
            [140.0, 90.0],
        ],
    ),
    (
        "Spiral in",
        &[
            [0.0, -180.0],
            [170.0, -40.0],
            [70.0, 130.0],
            [-90.0, 60.0],
            [-50.0, -60.0],
            [40.0, -45.0],
        ],
    ),
    (
        "Loop",
        &[
            [620.0, 0.0],
            [320.0, 0.0],
            [210.0, -100.0],
            [110.0, -60.0],
            [140.0, 40.0],
            [60.0, 10.0],
        ],
    ),
];

/// Ready-made paces: how much of the path is left (value) by how much of the lane is left
/// (time), both 1 coming in and 0 at the drum. The first is even.
pub const PACES: [(&str, &[[f64; 2]]); 5] = [
    ("Even", &[]),
    (
        "Stop half way",
        &[[0.0, 0.0], [0.35, 0.5], [0.65, 0.5], [1.0, 1.0]],
    ),
    ("Rush the end", &[[0.0, 0.0], [0.3, 0.75], [1.0, 1.0]]),
    ("Soft landing", &[[0.0, 0.0], [0.6, 0.2], [1.0, 1.0]]),
    ("Back up", &[[0.0, 0.0], [0.4, 0.6], [0.6, 0.4], [1.0, 1.0]]),
];

fn holds(notes: &[f64], t: f64) -> bool {
    let at = notes.partition_point(|&n| n < t - SAME);
    notes.get(at).is_some_and(|&n| (n - t).abs() <= SAME)
}

/// A pace preset as a track.
pub fn pace_track(i: usize) -> Track {
    let mut t = Track::default();
    for &[u, v] in PACES.get(i).map_or(&[][..], |p| p.1) {
        t.set(Key {
            t: u,
            v,
            ease: Ease::Linear,
        });
    }
    t
}

impl Editor {
    /// The group the selected notes fly with: the one that holds just them, else a new one they
    /// move into, out of any other (one undo step). Returns its index.
    pub fn motion_group(&mut self) -> Result<usize, String> {
        let mut times: Vec<f64> = self.selected().iter().map(|o| o.time).collect();
        if times.is_empty() {
            return Err("select notes on the timeline first".into());
        }
        times.sort_by(f64::total_cmp);
        times.dedup();
        let Some(g) = self.gimmick() else {
            return Err("this chart isn't a .tko yet".into());
        };
        if let Some(i) = g.groups.iter().position(|gr| {
            gr.notes.len() == times.len() && times.iter().all(|&t| holds(&gr.notes, t))
        }) {
            return Ok(i);
        }
        if g.groups.len() >= MAX_GROUPS {
            return Err("too many groups".into());
        }
        let at = g.groups.len();
        let name = format!("Motion {}", at + 1);
        self.edit_gimmick(false, |g| {
            for gr in &mut g.groups {
                gr.notes.retain(|&n| !holds(&times, n));
            }
            g.groups.push(Group {
                name,
                notes: times,
                ..Group::default()
            });
        });
        Ok(at)
    }

    pub fn path_of(&self, group: usize) -> Option<&NotePath> {
        self.gimmick()?.groups.get(group)?.path.as_ref()
    }

    /// Changes group `group`'s path (made, empty, if it has none). `drag`: part of a drag still
    /// going on, one undo step with the rest of it.
    pub fn edit_path(&mut self, group: usize, drag: bool, f: impl FnOnce(&mut NotePath)) -> bool {
        self.edit_gimmick(drag, |g| {
            if let Some(gr) = g.groups.get_mut(group) {
                let p = gr.path.get_or_insert_with(NotePath::default);
                f(p);
                p.sanitize();
            }
        })
    }

    /// Lays ready-made path `i` on group `group`, keeping its pace, weight and whether it shows.
    pub fn set_path_preset(&mut self, group: usize, i: usize) -> bool {
        let Some((_, points)) = PATHS.get(i) else {
            return false;
        };
        self.edit_path(group, false, |p| p.points = points.to_vec())
    }

    pub fn clear_path(&mut self, group: usize) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(gr) = g.groups.get_mut(group) {
                gr.path = None;
            }
        })
    }

    /// Takes the notes of group `group` onto its path (`on`) or back to the lane over one beat
    /// from `t`, keeping what it does before.
    pub fn path_switch(&mut self, group: usize, t: f64, on: bool) -> bool {
        let beat = self.beat_at(t);
        let before = self.path_of(group).map_or(1.0, |p| p.weight_at(t - 1.0));
        let to = if on { 1.0 } else { 0.0 };
        self.edit_path(group, false, |p| {
            p.weight.keys.retain(|k| k.t < t || k.t > t + beat);
            for (at, v) in [(t, before), (t + beat, to)] {
                p.weight.set(Key {
                    t: at,
                    v,
                    ease: Ease::QuadInOut,
                });
            }
        })
    }

    /// Group `group`'s path and keyframes.
    pub fn motion_of(&self, group: usize) -> Option<Motion> {
        let g = self.gimmick()?.groups.get(group)?;
        Some(Motion {
            path: g.path.clone(),
            tracks: g.tracks.clone(),
        })
    }

    /// Gives group `group` the motion copied before, in place of its own.
    pub fn paste_motion(&mut self, group: usize, m: &Motion) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(gr) = g.groups.get_mut(group) {
                gr.path = m.path.clone();
                gr.tracks = m.tracks.clone();
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::Chart;
    use crate::editor::chart::tests::MAP;
    use crate::tko::path::PathTable;

    fn tko() -> Editor {
        let mut e = Editor::new(Chart::parse(MAP), std::path::PathBuf::from("a.osu"));
        e.chart.gimmick = Some(Default::default());
        e
    }

    fn pick(e: &mut Editor, n: usize) -> Vec<f64> {
        let ids: Vec<u64> = e.chart.objects.iter().take(n).map(|o| o.id).collect();
        e.selection = ids.into_iter().collect();
        e.chart.objects.iter().take(n).map(|o| o.time).collect()
    }

    #[test]
    fn picked_notes_get_a_group_of_their_own_once() {
        let mut e = tko();
        assert!(e.motion_group().is_err(), "nothing picked");
        let times = pick(&mut e, 3);
        let g = e.motion_group().unwrap();
        assert_eq!(e.gimmick().unwrap().groups[g].notes, times);
        assert_eq!(
            e.motion_group().unwrap(),
            g,
            "the same notes: the same group"
        );
        // Two of them in a new group leave the first one's.
        let mut two = pick(&mut e, 2);
        two.sort_by(f64::total_cmp);
        let h = e.motion_group().unwrap();
        assert_ne!(h, g);
        let groups = &e.gimmick().unwrap().groups;
        assert_eq!(groups[h].notes, two);
        assert_eq!(groups[g].notes, [times[2]]);
        assert!(e.undo());
        assert_eq!(e.gimmick().unwrap().groups.len(), 1, "one undo step");
    }

    #[test]
    fn every_ready_made_path_and_pace_lands_on_the_drum() {
        let mut e = tko();
        pick(&mut e, 2);
        let g = e.motion_group().unwrap();
        for (i, (name, points)) in PATHS.iter().enumerate() {
            assert!(e.set_path_preset(g, i), "{name}");
            for (k, (pace, _)) in PACES.iter().enumerate() {
                let mut p = e.path_of(g).unwrap().clone();
                p.pace = pace_track(k);
                let t = PathTable::new(&p).unwrap();
                let [x, y] = t.place(&p, 0.0, 700.0);
                assert!(x.abs() < 0.5 && y.abs() < 0.5, "{name} {pace}");
                let start = points[0];
                let [x, y] = t.place(&p, 700.0, 700.0);
                assert!((x - start[0] as f32).abs() < 0.5 && (y - start[1] as f32).abs() < 0.5);
            }
        }
    }

    #[test]
    fn a_path_switches_on_and_off_over_a_beat_and_motion_copies() {
        let mut e = tko();
        pick(&mut e, 2);
        let g = e.motion_group().unwrap();
        e.set_path_preset(g, 0);
        let beat = e.beat_at(4000.0);
        assert!(e.path_switch(g, 4000.0, false));
        let p = e.path_of(g).unwrap();
        assert_eq!(p.weight_at(3000.0), 1.0);
        assert_eq!(p.weight_at(4000.0 + beat), 0.0);
        assert!(e.path_switch(g, 8000.0, true));
        let p = e.path_of(g).unwrap();
        assert_eq!(p.weight_at(6000.0), 0.0);
        assert_eq!(p.weight_at(8000.0 + beat), 1.0);
        let m = e.motion_of(g).unwrap();
        pick(&mut e, 1);
        let one = e.chart.objects[0].time;
        // The first note alone: a new group, out of the first one.
        let h = e.motion_group().unwrap();
        assert!(e.paste_motion(h, &m));
        assert_eq!(e.motion_of(h).unwrap(), m);
        assert!(!e.gimmick().unwrap().groups[g].notes.contains(&one));
        assert!(e.clear_path(h));
        assert!(e.path_of(h).is_none());
    }
}
