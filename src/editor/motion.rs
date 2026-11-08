//! Note motion in the editor (F4 Effects, Motion page): the notes picked on the timeline get a
//! group of their own, and that group an approach path, a pace along it and when it is on.
//!
//! A note flies with one group at a time (the runtime takes the first that holds it), so giving
//! notes a path takes them out of any other group they were in.

use super::chart::{HitObject, Taiko};
use super::keyframes::Subject;
use super::state::Editor;
use crate::tko::clips::Target as ClipTarget;
use crate::tko::gimmick::{
    BEFORE_BEATS, FieldProp, Gimmick, Group, MAX_GROUPS, NoteProp, ShowAs, ShowColour,
};
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
    pub before: BTreeMap<String, Track>,
    pub show: ShowAs,
}

/// When a lying note's truth shows: a name, and beats before its hit.
pub const REVEALS: [(&str, f64); 5] = [
    ("At the hit", 0.0),
    ("A 1/4 before", 0.25),
    ("Half a beat before", 0.5),
    ("A beat before", 1.0),
    ("Two beats before", 2.0),
];

/// Ready-made paths: a name and the points notes come in by (playfield units from the drum, x
/// right, y down; the drum ends each one).
pub const PATHS: [(&str, &[[f64; 2]]); 10] = [
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
    // No points: the lane itself, kept straight while clips bend it (`NotePath::straight`).
    (STRAIGHT_LANE, &[]),
];

/// The ready-made path that keeps notes on a plain straight lane, whatever shape it is bent into.
pub const STRAIGHT_LANE: &str = "Straight lane";

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

/// A key before the hit: beats before it, value, and the ease to the next key out (further from
/// the hit). The keys run backwards in time, so an ease "in" here plays out as "out" on screen.
type BeforeKey = (f64, f64, Ease);
/// A ready-made move: its name and the keys each property gets.
type BeforeMove = (&'static str, &'static [(NoteProp, &'static [BeforeKey])]);

/// Ready-made before-the-hit moves: a name and the keys each property gets. The first clears.
pub const BEFORES: [BeforeMove; 11] = [
    ("None", &[]),
    (
        "Fade in late",
        &[(
            NoteProp::Opacity,
            &[(1.0, 1.0, Ease::Linear), (2.5, 0.0, Ease::Linear)],
        )],
    ),
    (
        "Vanish early",
        &[(
            NoteProp::Opacity,
            &[(0.5, 0.0, Ease::Linear), (1.5, 1.0, Ease::Linear)],
        )],
    ),
    (
        "Pop in",
        &[(
            NoteProp::Scale,
            &[(1.0, 1.0, Ease::BackIn), (2.0, 0.05, Ease::Linear)],
        )],
    ),
    (
        "Drop in",
        &[(
            NoteProp::Dy,
            &[(1.0, 0.0, Ease::QuadOut), (2.5, -200.0, Ease::Linear)],
        )],
    ),
    (
        "Rise from below",
        &[
            (
                NoteProp::Dy,
                &[(1.0, 0.0, Ease::CubicIn), (3.0, 160.0, Ease::Linear)],
            ),
            (
                NoteProp::Opacity,
                &[(1.5, 1.0, Ease::Linear), (3.0, 0.0, Ease::Linear)],
            ),
        ],
    ),
    (
        "Jump up",
        &[(
            NoteProp::Dy,
            &[
                (0.0, 0.0, Ease::QuadOut),
                (0.5, -50.0, Ease::QuadIn),
                (1.0, 0.0, Ease::Linear),
            ],
        )],
    ),
    (
        "Spin in",
        &[(
            NoteProp::Swirl,
            &[(1.0, 0.0, Ease::CubicIn), (3.0, 120.0, Ease::Linear)],
        )],
    ),
    (
        "Flicker in",
        &[(
            NoteProp::Hidden,
            &[
                (1.0, 0.0, Ease::Step),
                (1.5, 1.0, Ease::Step),
                (2.0, 0.0, Ease::Step),
                (2.5, 1.0, Ease::Step),
            ],
        )],
    ),
    (
        "Shake",
        &[(
            NoteProp::Dx,
            &[
                (0.0, 0.0, Ease::SineInOut),
                (0.25, 8.0, Ease::SineInOut),
                (0.5, -8.0, Ease::SineInOut),
                (0.75, 8.0, Ease::SineInOut),
                (1.0, -8.0, Ease::SineInOut),
                (1.25, 0.0, Ease::Linear),
            ],
        )],
    ),
    (
        "Squash on the hit",
        &[
            (
                NoteProp::StretchX,
                &[(0.0, 1.5, Ease::QuadIn), (0.5, 1.0, Ease::Linear)],
            ),
            (
                NoteProp::StretchY,
                &[(0.0, 0.6, Ease::QuadIn), (0.5, 1.0, Ease::Linear)],
            ),
        ],
    ),
];

/// A before-the-hit preset as tracks.
pub fn before_tracks(i: usize) -> BTreeMap<String, Track> {
    let mut out = BTreeMap::new();
    for (prop, keys) in BEFORES.get(i).map_or(&[][..], |b| b.1) {
        let tr: &mut Track = out.entry(prop.name().to_string()).or_default();
        for &(t, v, ease) in keys.iter() {
            tr.set(Key { t, v, ease });
        }
    }
    out
}

pub(crate) fn holds(notes: &[f64], t: f64) -> bool {
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
    /// The selected notes' times (fakes aside: they join groups by place), in order, once each.
    fn selected_times(&self) -> Vec<f64> {
        let mut times: Vec<f64> = self
            .selected()
            .iter()
            .filter(|o| !o.fake)
            .map(|o| o.time)
            .collect();
        times.sort_by(f64::total_cmp);
        times.dedup();
        times
    }

    /// The selected fakes.
    fn selected_fakes(&self) -> Vec<u64> {
        self.selected()
            .iter()
            .filter(|o| o.fake)
            .map(|o| o.id)
            .collect()
    }

    /// Whether group `i` holds `o`: a fake given a group by place by that, anything else by
    /// its time (as the game takes it).
    pub fn group_holds(&self, i: usize, o: &HitObject) -> bool {
        let Some(g) = self.gimmick() else {
            return false;
        };
        let Some(gr) = g.groups.get(i) else {
            return false;
        };
        match o.group {
            Some(n) if o.fake && n as usize <= g.groups.len() => n as usize == i + 1,
            _ => holds(&gr.notes, o.time),
        }
    }

    /// How many objects group `i` holds.
    pub fn group_size(&self, i: usize) -> usize {
        self.chart
            .objects
            .iter()
            .filter(|o| self.group_holds(i, o))
            .count()
    }

    /// The group that holds just the selected notes (and fakes), if there is one.
    pub fn group_of_selection(&self) -> Option<usize> {
        let picked = self.selected();
        if picked.is_empty() {
            return None;
        }
        (0..self.gimmick()?.groups.len()).find(|&i| {
            picked.iter().all(|o| self.group_holds(i, o)) && self.group_size(i) == picked.len()
        })
    }

    /// The group the selected notes fly with: the one that holds just them, else a new one they
    /// move into, out of any other (one undo step). Returns its index.
    pub fn motion_group(&mut self) -> Result<usize, String> {
        self.group_selection("Motion", true, |_, _| {})
    }

    /// The selected notes as a group named `prefix` and its number, out of any other group
    /// (a note goes with the first group that holds it, so being in two would leave the later
    /// one doing nothing to it). `reuse`: the group that already holds just them, if there is
    /// one. `then` changes the gimmick in the same undo step, given the group. Returns the group.
    pub(crate) fn group_selection(
        &mut self,
        prefix: &str,
        reuse: bool,
        then: impl FnOnce(&mut Gimmick, usize),
    ) -> Result<usize, String> {
        if self.selection.is_empty() {
            return Err("select notes on the timeline first".into());
        }
        let (times, fakes) = (self.selected_times(), self.selected_fakes());
        let Some(g) = self.gimmick() else {
            return Err("this chart isn't a .tko yet".into());
        };
        if let Some(i) = self.group_of_selection().filter(|_| reuse) {
            self.edit_gimmick(false, |g| then(g, i));
            return Ok(i);
        }
        if g.groups.len() >= MAX_GROUPS {
            return Err("too many groups".into());
        }
        let at = g.groups.len();
        let name = format!("{prefix} {}", at + 1);
        // The notes by their times; the fakes picked by place, so a fake on a note's
        // millisecond (or on other fakes) gets a group of its own.
        let k = at as u16 + 1;
        self.edit_gimmick_and_fakes(
            |g| {
                for gr in &mut g.groups {
                    gr.notes.retain(|&n| !holds(&times, n));
                }
                g.groups.push(Group {
                    name,
                    notes: times,
                    ..Group::default()
                });
                then(g, at);
            },
            |o| fakes.contains(&o.id).then_some(Some(k)),
        );
        Ok(at)
    }

    /// Puts clip `id`, one on notes, on just the selected notes: the group that holds just
    /// them, else a new one they move into (one undo step). Returns the group.
    pub fn clip_on_selection(&mut self, id: u32) -> Result<usize, String> {
        if !matches!(
            self.clip(id).and_then(|c| c.target()),
            Some(ClipTarget::Notes(..))
        ) {
            return Err("this effect doesn't move notes".into());
        }
        self.group_selection("Group", true, |g, i| {
            if let Some(c) = g.clips.iter_mut().find(|c| c.id == id) {
                c.group = i as u16 + 1;
            }
        })
    }

    /// Moves the selected notes into group `group`, out of any other. Returns how many joined.
    pub fn add_selection_to_group(&mut self, group: usize) -> Result<usize, String> {
        if self.selection.is_empty() {
            return Err("select notes on the timeline first".into());
        }
        let (times, fakes) = (self.selected_times(), self.selected_fakes());
        let Some(gr) = self.gimmick().and_then(|g| g.groups.get(group)) else {
            return Err("no such group".into());
        };
        let k = group as u16 + 1;
        let joined = times.iter().filter(|&&t| !holds(&gr.notes, t)).count()
            + self
                .selected()
                .iter()
                .filter(|o| o.fake && o.group != Some(k))
                .count();
        self.edit_gimmick_and_fakes(
            |g| {
                for (i, gr) in g.groups.iter_mut().enumerate() {
                    if i == group {
                        gr.notes.extend(&times);
                    } else {
                        gr.notes.retain(|&n| !holds(&times, n));
                    }
                }
            },
            |o| fakes.contains(&o.id).then_some(Some(k)),
        );
        Ok(joined)
    }

    /// Takes the selected notes out of group `group`. Returns how many left it.
    pub fn remove_selection_from_group(&mut self, group: usize) -> Result<usize, String> {
        if self.selection.is_empty() {
            return Err("select notes on the timeline first".into());
        }
        let times = self.selected_times();
        let Some(gr) = self.gimmick().and_then(|g| g.groups.get(group)) else {
            return Err("no such group".into());
        };
        // Fakes in it (by place or by time) are put in no group, by place.
        let leaving: Vec<u64> = self
            .selected()
            .iter()
            .filter(|o| o.fake && self.group_holds(group, o))
            .map(|o| o.id)
            .collect();
        let left = gr.notes.iter().filter(|&&n| holds(&times, n)).count() + leaving.len();
        self.edit_gimmick_and_fakes(
            |g| {
                if let Some(gr) = g.groups.get_mut(group) {
                    gr.notes.retain(|&n| !holds(&times, n));
                }
            },
            |o| leaving.contains(&o.id).then_some(Some(0)),
        );
        Ok(left)
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
    /// The one without points is the straight lane.
    pub fn set_path_preset(&mut self, group: usize, i: usize) -> bool {
        let Some((_, points)) = PATHS.get(i) else {
            return false;
        };
        self.edit_path(group, false, |p| {
            p.points = points.to_vec();
            p.straight = points.is_empty();
        })
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
            before: g.before.clone(),
            show: g.show,
        })
    }

    pub fn before_of(&self, group: usize) -> Option<&BTreeMap<String, Track>> {
        Some(&self.gimmick()?.groups.get(group)?.before)
    }

    /// Gives group `group` ready-made before-the-hit move `i` in place of its own.
    pub fn set_before_preset(&mut self, group: usize, i: usize) -> bool {
        if i >= BEFORES.len() {
            return false;
        }
        self.edit_gimmick(false, |g| {
            if let Some(gr) = g.groups.get_mut(group) {
                gr.before = before_tracks(i);
            }
        })
    }

    /// Changes group `group`'s before-the-hit track for `prop`; one emptied goes. `drag`: part of
    /// a drag still going on.
    pub fn edit_before(
        &mut self,
        group: usize,
        prop: NoteProp,
        drag: bool,
        f: impl FnOnce(&mut Track),
    ) -> bool {
        self.edit_gimmick(drag, |g| {
            if let Some(gr) = g.groups.get_mut(group) {
                let tr = gr.before.entry(prop.name().to_string()).or_default();
                f(tr);
                let (far, after) = BEFORE_BEATS;
                let (lo, hi) = prop.range();
                tr.keys.retain(|k| k.t.is_finite() && k.v.is_finite());
                for k in &mut tr.keys {
                    k.t = k.t.clamp(-after, far);
                    k.v = k.v.clamp(lo, hi);
                }
                tr.sanitize();
                if tr.is_empty() {
                    gr.before.remove(prop.name());
                }
            }
        })
    }

    /// Gives group `group` the motion copied before, in place of its own.
    pub fn paste_motion(&mut self, group: usize, m: &Motion) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(gr) = g.groups.get_mut(group) {
                gr.path = m.path.clone();
                gr.tracks = m.tracks.clone();
                gr.before = m.before.clone();
                gr.show = m.show;
            }
        })
    }

    pub fn show_of(&self, group: usize) -> ShowAs {
        self.gimmick()
            .and_then(|g| g.groups.get(group))
            .map_or_else(ShowAs::default, |g| g.show)
    }

    /// Changes the lie group `group`'s notes tell (see `ShowAs`).
    pub fn edit_show(&mut self, group: usize, f: impl FnOnce(&mut ShowAs)) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(gr) = g.groups.get_mut(group) {
                f(&mut gr.show);
                gr.show.reveal = gr.show.reveal.clamp(0.0, BEFORE_BEATS.0);
            }
        })
    }
}

/// What the Motion chapter's example lays over 16 beats, four at a time (see
/// `Editor::motion_example`).
pub const EXAMPLE_PARTS: [&str; 4] = ["path", "pop in + fakes", "freeze + drum", "swapped colours"];

impl Editor {
    /// Selects the real dons and kats from `from` to before `to`. Returns how many.
    fn select_hits(&mut self, from: f64, to: f64) -> usize {
        self.selection = self
            .chart
            .objects
            .iter()
            .filter(|o| {
                !o.fake
                    && o.time >= from
                    && o.time < to
                    && matches!(o.taiko(), Taiko::Don | Taiko::Kat)
            })
            .map(|o| o.id)
            .collect();
        self.selection.len()
    }

    /// The notes picked get their own group, named `name`.
    fn example_group(&mut self, name: &str) -> Option<usize> {
        let g = self.motion_group().ok()?;
        let name = name.to_string();
        self.edit_gimmick(false, |gm| {
            if let Some(gr) = gm.groups.get_mut(g) {
                gr.name = name;
            }
        });
        Some(g)
    }

    /// An example of what the Motion page makes, over the 16 beats from `a`, four at a time (see
    /// `EXAMPLE_PARTS`): each part on the notes there, each its own undo step. Returns the parts
    /// that went in.
    pub fn motion_example(&mut self, a: f64) -> Result<Vec<&'static str>, String> {
        if !self.is_tko() {
            return Err("the example needs a .tko map".into());
        }
        let beat = self.beat_at(a);
        let at = |b: f64| (a + b * beat).round();
        let mut parts = Vec::new();
        if self.select_hits(at(0.0), at(4.0)) > 0
            && let Some(g) = self.example_group("Example: path")
        {
            self.set_path_preset(g, 0);
            parts.push(EXAMPLE_PARTS[0]);
        }
        if self.select_hits(at(4.0), at(8.0)) > 0
            && let Some(g) = self.example_group("Example: pop in")
        {
            let pop = BEFORES.iter().position(|b| b.0 == "Pop in").unwrap_or(0);
            self.set_before_preset(g, pop);
            let _ = self.add_fake_copies(0.5);
            parts.push(EXAMPLE_PARTS[1]);
        }
        if self.select_hits(at(8.0), at(12.0)) > 0 {
            self.scroll_move(ScrollMove::Freeze, at(8.0));
            self.drum_to(at(8.0), 320.0);
            self.drum_to(at(11.0), 0.0);
            parts.push(EXAMPLE_PARTS[2]);
        }
        if self.select_hits(at(12.0), at(16.0)) > 0
            && let Some(g) = self.example_group("Example: swapped")
        {
            self.edit_show(g, |s| {
                *s = ShowAs {
                    colour: ShowColour::Swap,
                    reveal: 1.0,
                    ..ShowAs::default()
                }
            });
            parts.push(EXAMPLE_PARTS[3]);
        }
        self.selection.clear();
        if parts.is_empty() {
            return Err("no notes here to show the example on".into());
        }
        Ok(parts)
    }
}

/// A move of the shared scroll from the playhead (see `Editor::scroll_move`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollMove {
    /// Every note stops for a beat.
    Freeze,
    /// Every note runs back for a beat.
    Reverse,
    /// For a bar, each beat lurches forward fast and settles slow, on time overall.
    Lurch,
    /// Every note jumps a beat closer at once.
    Jump,
}

impl ScrollMove {
    pub const ALL: [ScrollMove; 4] = [
        ScrollMove::Freeze,
        ScrollMove::Reverse,
        ScrollMove::Lurch,
        ScrollMove::Jump,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ScrollMove::Freeze => "Freeze a beat",
            ScrollMove::Reverse => "Back up a beat",
            ScrollMove::Lurch => "Lurch on each beat (a bar)",
            ScrollMove::Jump => "Jump a beat ahead",
        }
    }
}

/// Speeds offered at the playhead, as mania mappers type them.
pub const SPEEDS: [f64; 8] = [0.0, 0.25, 0.5, 1.0, 1.5, 2.0, 4.0, -1.0];

impl Editor {
    /// The shared scroll's speed at `t` (1 with none).
    pub fn scroll_speed(&self, t: f64) -> f64 {
        self.value_of(Subject::Scroll, t)
    }

    /// The speed from `t` on is `v`, until the next key.
    pub fn set_scroll_speed(&mut self, t: f64, v: f64) -> bool {
        self.edit_gimmick(false, |g| {
            g.scroll.set(Key {
                t,
                v,
                ease: Ease::Step,
            });
        })
    }

    /// Lays `keys` between `t0` and `t1` in place of the keys there, one undo step.
    fn scroll_keys(&mut self, t0: f64, t1: f64, keys: &[(f64, f64)]) -> bool {
        self.edit_gimmick(false, |g| {
            g.scroll.keys.retain(|k| k.t < t0 || k.t > t1);
            for &(t, v) in keys {
                g.scroll.set(Key {
                    t,
                    v,
                    ease: Ease::Step,
                });
            }
        })
    }

    /// Puts move `m` on the shared scroll from `t`, coming back to the speed there after it.
    pub fn scroll_move(&mut self, m: ScrollMove, t: f64) -> bool {
        let beat = self.beat_at(t);
        let speed = self.scroll_speed(t);
        match m {
            ScrollMove::Freeze => self.scroll_keys(t, t + beat, &[(t, 0.0), (t + beat, speed)]),
            ScrollMove::Reverse => self.scroll_keys(t, t + beat, &[(t, -speed), (t + beat, speed)]),
            ScrollMove::Lurch => {
                // A quarter of each beat three times as fast, the rest a third: a beat's way.
                let mut keys = Vec::new();
                for b in 0..4 {
                    let at = t + b as f64 * beat;
                    keys.push((at, speed * 3.0));
                    keys.push((at + beat / 4.0, speed / 3.0));
                }
                keys.push((t + 4.0 * beat, speed));
                self.scroll_keys(t, t + 4.0 * beat, &keys)
            }
            ScrollMove::Jump => {
                // A beat's scroll in 2 ms.
                const SPAN: f64 = 2.0;
                let fast = (beat / SPAN).min(crate::tko::scroll::MAX_SPEED);
                self.scroll_keys(t, t + SPAN, &[(t, fast), (t + SPAN, speed)])
            }
        }
    }

    /// The speed key at or before `t` glides to the next one instead of jumping there.
    pub fn scroll_glide(&mut self, t: f64) -> bool {
        self.edit_gimmick(false, |g| {
            let at = g.scroll.keys.partition_point(|k| k.t <= t);
            if let Some(k) = at.checked_sub(1).and_then(|i| g.scroll.keys.get_mut(i)) {
                k.ease = if k.ease == Ease::Step {
                    Ease::SineInOut
                } else {
                    Ease::Step
                };
            }
        })
    }

    /// Slides the hit target along the lane to `to` (virtual units from home) over a beat from
    /// `t`, from where it is then.
    pub fn drum_to(&mut self, t: f64, to: f64) -> bool {
        let beat = self.beat_at(t);
        let s = Subject::Field(FieldProp::Drum);
        let from = self.value_of(s, t);
        self.edit_gimmick(false, |g| {
            let tr = g.field_track_mut(FieldProp::Drum);
            tr.keys.retain(|k| k.t < t || k.t > t + beat);
            tr.set(Key {
                t,
                v: from,
                ease: Ease::SineInOut,
            });
            tr.set(Key {
                t: t + beat,
                v: to,
                ease: Ease::SineInOut,
            });
        })
    }

    pub fn clear_scroll(&mut self) -> bool {
        self.edit_gimmick(false, |g| g.scroll.keys.clear())
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
    fn a_new_group_takes_its_notes_out_of_the_others() {
        let mut e = tko();
        let three = pick(&mut e, 3);
        let g = e.motion_group().unwrap();
        // Two of them again from the Keyframes page: a group of their own, not a second one
        // that would do nothing to them.
        pick(&mut e, 2);
        let h = e.add_group_from_selection().unwrap();
        let groups = &e.gimmick().unwrap().groups;
        assert_eq!(groups[g].notes, [three[2]]);
        assert_eq!(groups[h].notes, three[..2]);
        // The same notes once more: yet another group, the last one emptied.
        let k = e.add_group_from_selection().unwrap();
        let groups = &e.gimmick().unwrap().groups;
        assert!(groups[h].notes.is_empty());
        assert_eq!(groups[k].notes, three[..2]);
    }

    #[test]
    fn a_clip_goes_on_the_selected_notes_and_they_join_or_leave_its_group() {
        use crate::tko::clips::Clip;
        let mut e = tko();
        let hide = Clip::new(ClipTarget::Notes(None, NoteProp::Hidden), 0.0, 5000.0);
        let id = e.add_clips(vec![hide])[0];
        assert!(e.clip_on_selection(id).is_err(), "nothing picked");
        let two = pick(&mut e, 2);
        let g = e.clip_on_selection(id).unwrap();
        assert_eq!(e.clip(id).unwrap().group, g as u16 + 1);
        assert_eq!(e.gimmick().unwrap().groups[g].notes, two);
        assert_eq!(
            e.clip_on_selection(id).unwrap(),
            g,
            "the same notes: the same group"
        );
        assert!(e.undo());
        assert!(e.gimmick().unwrap().groups.is_empty(), "one undo step");
        assert_eq!(e.clip(id).unwrap().group, 0);
        let g = e.clip_on_selection(id).unwrap();
        // A third note joins, then the first leaves.
        let three = pick(&mut e, 3);
        assert_eq!(e.add_selection_to_group(g).unwrap(), 1);
        assert_eq!(e.gimmick().unwrap().groups[g].notes, three);
        pick(&mut e, 1);
        assert_eq!(e.remove_selection_from_group(g).unwrap(), 1);
        assert_eq!(e.gimmick().unwrap().groups[g].notes, three[1..]);
        // A note joining leaves the group it was in.
        let h = e.motion_group().unwrap();
        assert_eq!(e.add_selection_to_group(g).unwrap(), 1);
        let groups = &e.gimmick().unwrap().groups;
        assert!(groups[h].notes.is_empty());
        assert_eq!(groups[g].notes, three);
        // A clip on the playfield takes no notes.
        let zoom = Clip::new(ClipTarget::Field(FieldProp::Zoom), 0.0, 1000.0);
        let zoom = e.add_clips(vec![zoom])[0];
        assert!(e.clip_on_selection(zoom).is_err());
    }

    #[test]
    fn a_moved_note_keeps_its_motion_and_undo_brings_it_back() {
        let mut e = tko();
        let times = pick(&mut e, 2);
        let g = e.motion_group().unwrap();
        e.set_path_preset(g, 2);
        assert!(e.nudge_selected(true));
        let now: Vec<f64> = e.selected().iter().map(|o| o.time).collect();
        assert_ne!(now, times);
        assert_eq!(e.group_of_selection(), Some(g), "the group went with them");
        assert!(e.undo());
        assert_eq!(e.gimmick().unwrap().groups[g].notes, times);
    }

    #[test]
    fn every_ready_made_path_and_pace_lands_on_the_drum() {
        let mut e = tko();
        pick(&mut e, 2);
        let g = e.motion_group().unwrap();
        for (i, (name, points)) in PATHS.iter().enumerate() {
            assert!(e.set_path_preset(g, i), "{name}");
            if points.is_empty() {
                // The straight lane: no points to fly, see `notes_on_a_straight_lane_ignore_its_bend`.
                assert!(e.path_of(g).unwrap().straight, "{name}");
                continue;
            }
            assert!(!e.path_of(g).unwrap().straight, "{name}");
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

    #[test]
    fn before_the_hit_presets_replace_and_keys_stay_in_range() {
        let mut e = tko();
        pick(&mut e, 2);
        let g = e.motion_group().unwrap();
        for (i, (name, _)) in BEFORES.iter().enumerate().skip(1) {
            assert!(e.set_before_preset(g, i), "{name}");
            let b = e.before_of(g).unwrap();
            assert!(!b.is_empty(), "{name}");
            // Every preset leaves notes as they are on the hit, but those that are about it.
            for (prop, tr) in b {
                let p = NoteProp::ALL.iter().find(|p| p.name() == prop).unwrap();
                if !matches!(*name, "Vanish early" | "Squash on the hit") {
                    assert_eq!(tr.value(0.0, 0.0), p.default_value(), "{name}");
                }
            }
        }
        assert!(e.set_before_preset(g, 0));
        assert!(e.before_of(g).unwrap().is_empty());
        // A key far out of range is held to it, and a track emptied goes.
        e.edit_before(g, NoteProp::Opacity, false, |t| {
            t.set(Key {
                t: 99.0,
                v: 5.0,
                ease: Ease::Linear,
            });
        });
        let k = e.before_of(g).unwrap()["opacity"].keys[0];
        assert_eq!((k.t, k.v), (BEFORE_BEATS.0, 1.0));
        e.edit_before(g, NoteProp::Opacity, false, |t| t.keys.clear());
        assert!(e.before_of(g).unwrap().is_empty());
        assert!(e.undo());
        assert!(e.motion_of(g).unwrap().before.contains_key("opacity"));
    }

    #[test]
    fn scroll_moves_come_back_to_the_speed_and_keep_notes_on_time() {
        use crate::tko::scroll::ScrollTable;
        let mut e = tko();
        let t = 4000.0;
        let beat = e.beat_at(t);
        assert!(e.set_scroll_speed(0.0, 2.0));
        assert_eq!(e.scroll_speed(t), 2.0);
        for m in ScrollMove::ALL {
            assert!(e.scroll_move(m, t), "{}", m.name());
            let g = e.gimmick().unwrap();
            let table = ScrollTable::new(&g.scroll).unwrap();
            // After it, the speed it had.
            assert_eq!(e.scroll_speed(t + 4.0 * beat + 1.0), 2.0, "{}", m.name());
            let x = |t: f64| table.x(t);
            match m {
                ScrollMove::Freeze => assert_eq!(x(t + beat), x(t)),
                ScrollMove::Reverse => assert!(x(t + beat) < x(t)),
                // On time overall: a bar's way in a bar.
                ScrollMove::Lurch => {
                    assert!((x(t + 4.0 * beat) - x(t) - 2.0 * 4.0 * beat).abs() < 1e-6)
                }
                ScrollMove::Jump => assert!((x(t + 2.0) - x(t) - beat).abs() < 1e-6),
            }
            assert!(e.undo());
        }
        assert!(e.drum_to(t, 300.0));
        let drum = Subject::Field(FieldProp::Drum);
        assert_eq!(e.value_of(drum, t), 0.0);
        assert_eq!(e.value_of(drum, t + beat), 300.0);
        assert!(e.drum_to(t + 4.0 * beat, 0.0));
        assert_eq!(e.value_of(drum, t + 5.0 * beat), 0.0);
        assert!(e.scroll_glide(t));
        assert_eq!(e.gimmick().unwrap().scroll.keys[0].ease, Ease::SineInOut);
        assert!(e.clear_scroll());
        assert_eq!(e.scroll_speed(t), 1.0);
    }

    #[test]
    fn one_fake_of_a_stack_gets_a_path_of_its_own() {
        let mut e = tko();
        let note = e.chart.objects[0].clone();
        let t = note.time;
        // The note and three fakes on its millisecond share a group by time.
        e.selection = [note.id].into();
        let shared = e.motion_group().unwrap();
        let fakes: Vec<u64> = (0..3)
            .map(|_| e.place_note(t, false, true).unwrap())
            .collect();
        let obj = |e: &Editor, id: u64| e.chart.object(id).unwrap().clone();
        assert!(fakes.iter().all(|f| e.group_holds(shared, &obj(&e, *f))));
        // The second fake alone gets a group, and a path in it: only it.
        e.selection = [fakes[1]].into();
        let own = e.motion_group().unwrap();
        assert_ne!(own, shared);
        assert!(e.set_path_preset(own, 0));
        assert_eq!(obj(&e, fakes[1]).group, Some(own as u16 + 1));
        assert!(e.group_holds(own, &obj(&e, fakes[1])));
        assert!(!e.group_holds(own, &obj(&e, fakes[0])));
        assert!(!e.group_holds(own, &note));
        assert!(e.group_holds(shared, &note) && e.group_holds(shared, &obj(&e, fakes[2])));
        assert_eq!(e.group_of_selection(), Some(own));
        assert_eq!(e.group_size(own), 1);
        // Saved and read back, it keeps its group; the game gives it that one.
        let back = Chart::parse_tko(&e.chart.to_text()).unwrap();
        let groups: Vec<Option<u16>> = back
            .objects
            .iter()
            .filter(|o| o.fake)
            .map(|o| o.group)
            .collect();
        assert_eq!(
            groups
                .iter()
                .filter(|g| **g == Some(own as u16 + 1))
                .count(),
            1
        );
        // Taken out of its group it is in none, not back in the note's.
        assert_eq!(e.remove_selection_from_group(own), Ok(1));
        assert_eq!(obj(&e, fakes[1]).group, Some(0));
        assert!(!e.group_holds(shared, &obj(&e, fakes[1])));
        // Undo puts it back in its own group, in one step with the gimmick.
        assert!(e.undo());
        assert_eq!(obj(&e, fakes[1]).group, Some(own as u16 + 1));
        // Deleting the shared group (before it) moves its number down with it.
        assert!(e.delete_group(shared));
        let own = own - 1;
        assert_eq!(obj(&e, fakes[1]).group, Some(own as u16 + 1));
        assert!(e.group_holds(own, &obj(&e, fakes[1])));
        // Deleting its own group leaves it in none.
        assert!(e.delete_group(own));
        assert_eq!(obj(&e, fakes[1]).group, Some(0));
        // It can't become a note there: the note has that millisecond.
        e.selection = [fakes[1]].into();
        e.toggle_fake().unwrap_err();
    }
}
