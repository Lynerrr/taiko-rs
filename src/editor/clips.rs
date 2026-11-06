//! Effect clips in the editor: adding, changing and removing them (each one undo step), and the
//! shelf of ready-made effects a click drops onto the timeline.

use super::chart::Taiko;
use super::state::Editor;
use crate::tko::clips::{self, Clip, LANE_SHAPES, Motion, Notes, Target, When};
use crate::tko::gimmick::{FieldProp, NoteProp, PropGroup};
use crate::tko::track::Ease;

/// What a clip can move, in the order the editor offers them.
pub fn targets() -> Vec<Target> {
    let mut out: Vec<Target> = FieldProp::ALL.iter().map(|p| Target::Field(*p)).collect();
    out.insert(4, Target::Shake);
    out.extend((0..LANE_SHAPES.len()).map(Target::Lane));
    out.extend(NoteProp::ALL.iter().map(|p| Target::Notes(None, *p)));
    out
}

/// A clip's target for people.
pub fn target_label(t: Target) -> String {
    match t {
        Target::Shake => "Shake".into(),
        Target::Lane(k) => format!(
            "Lane: {}",
            match LANE_SHAPES.get(k).copied().unwrap_or("") {
                "zigzag" => "zigzag",
                "uturn" => "U-turn",
                "spiral" => "spiral",
                "loop" => "loop",
                "elbow" => "elbow",
                "meadow" => "meadow",
                other => other,
            }
        ),
        Target::Field(p) => match p {
            FieldProp::X => "Move sideways",
            FieldProp::Y => "Move up / down",
            FieldProp::Rotation => "Turn",
            FieldProp::Zoom => "Zoom",
            FieldProp::Scroll => "Scroll speed",
            FieldProp::StretchX => "Stretch wide",
            FieldProp::StretchY => "Stretch tall",
            FieldProp::Flip => "Turn over",
            FieldProp::Flash => "Flash",
            FieldProp::Vignette => "Dark edges",
            FieldProp::Wash => "Colour wash",
            FieldProp::WashHue => "Wash colour",
            FieldProp::Glitch => "Glitch",
            FieldProp::Chroma => "Colour split",
            FieldProp::Echo => "Echoes",
            FieldProp::EchoX => "Echo spread X",
            FieldProp::EchoY => "Echo spread Y",
            FieldProp::EchoRot => "Echo turn",
            FieldProp::EchoZoom => "Echo size",
            FieldProp::EchoFade => "Echo fade",
            FieldProp::Kaleido => "Kaleidoscope",
        }
        .into(),
        Target::Notes(g, p) => {
            let what = match p {
                NoteProp::Dx => "move sideways",
                NoteProp::Dy => "move up / down",
                NoteProp::Scale => "size",
                NoteProp::Opacity => "see-through",
                NoteProp::Hidden => "hidden",
                NoteProp::Swirl => "swirl",
            };
            match g {
                None => format!("Notes: {what}"),
                Some(g) => format!("Group {}: {what}", g + 1),
            }
        }
    }
}

/// What a target is about, for its tooltip.
pub fn target_about(t: Target) -> &'static str {
    match t {
        Target::Lane(_) => {
            "Bends the lane into this shape: the notes travel along it. The amount is how far, 0 straight to 1 the whole shape; fade it in and out to morph."
        }
        Target::Shake => "Jolts the whole playfield in random directions. The amount is how far.",
        Target::Field(p) => p.about(),
        Target::Notes(_, NoteProp::Opacity) => {
            "Takes this much away from how solid the notes are: -0.7 leaves them faint."
        }
        Target::Notes(_, p) => p.about(),
    }
}

/// The colour of a target's clips: moves blue, looks pink, clones violet, shakes orange, notes
/// green.
pub fn target_colour(t: Target) -> u32 {
    match t {
        Target::Shake => 0xFFA040,
        Target::Lane(_) => 0xFFD34D,
        Target::Notes(..) => 0x5FD49A,
        Target::Field(p) => match p.group() {
            PropGroup::Move => 0x5BA8FF,
            PropGroup::Look => 0xFF6FB0,
            PropGroup::Clones => 0xB36BFF,
        },
    }
}

/// How much one step of the amount is, for a target.
pub fn amount_step(t: Target) -> f64 {
    let span = match t {
        Target::Field(p) => p.range().1 - p.range().0,
        Target::Notes(_, p) => p.range().1 - p.range().0,
        Target::Shake => 400.0,
        Target::Lane(_) => 1.0,
    };
    10f64.powf((span / 100.0).log10().floor()).clamp(0.01, 10.0)
}

/// A clip in words: what, and how often ("Zoom, every 2 beats").
pub fn describe(c: &Clip) -> String {
    let what = c.target().map_or_else(|| c.what.clone(), target_label);
    let beats = |b: f64| {
        if b == 1.0 {
            "beat".to_string()
        } else {
            format!("{} beats", trim(b))
        }
    };
    match c.when {
        When::Span => what,
        When::Beats { every } => format!("{what}, every {}", beats(every)),
        When::Notes { notes } => format!("{what}, on {}", notes.label()),
        When::Hits { notes } => format!("{what}, on hits of {}", notes.label()),
        When::Misses => format!("{what}, on misses"),
    }
}

/// A number without trailing zeros.
pub fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The length of a beat at `t` by the red lines `reds` (ascending): the last one at or before
/// it, else the first.
pub fn beat_in(reds: &[(f64, f64)], t: f64) -> f64 {
    let at = reds.partition_point(|r| r.0 <= t);
    reds.get(at.saturating_sub(1)).map_or(500.0, |r| r.1)
}

/// A ready-made effect on the shelf.
pub struct Preset {
    pub name: &'static str,
    pub about: &'static str,
    /// The clips it is, for a stretch `start..end` that is `beats` beats long.
    pub make: fn(f64, f64, f64) -> Vec<Clip>,
}

fn c(t: Target, start: f64, end: f64, amount: f64) -> Clip {
    Clip {
        amount,
        ..Clip::new(t, start, end)
    }
}

fn every(mut c: Clip, beats: f64, length: f64, motion: Motion) -> Clip {
    c.when = When::Beats { every: beats };
    c.length = length;
    c.motion = motion;
    c
}

fn on(mut c: Clip, when: When, length: f64) -> Clip {
    c.when = when;
    c.length = length;
    c
}

fn faded(mut c: Clip, fade_in: f64, fade_out: f64, ease: Ease) -> Clip {
    c.fade_in = fade_in;
    c.fade_out = fade_out;
    c.ease = ease;
    c
}

const ZOOM: Target = Target::Field(FieldProp::Zoom);
const TURN: Target = Target::Field(FieldProp::Rotation);

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Beat pulse",
        about: "The playfield breathes: a small zoom kick on every beat.",
        make: |s, e, _| vec![every(c(ZOOM, s, e, 0.06), 1.0, 0.75, Motion::Kick)],
    },
    Preset {
        name: "Every 2nd beat",
        about: "A bigger zoom kick on every second beat.",
        make: |s, e, _| vec![every(c(ZOOM, s, e, 0.12), 2.0, 1.0, Motion::Kick)],
    },
    Preset {
        name: "Sway",
        about: "The playfield sways left and right, a beat each way.",
        make: |s, e, _| {
            let mut c = every(
                c(Target::Field(FieldProp::X), s, e, 40.0),
                1.0,
                1.0,
                Motion::Swing,
            );
            c.ease = Ease::SineInOut;
            vec![c]
        },
    },
    Preset {
        name: "Tilt",
        about: "The playfield rocks, tilting one way then the other every 2 beats.",
        make: |s, e, _| {
            let mut c = every(c(TURN, s, e, 6.0), 2.0, 2.0, Motion::Swing);
            c.ease = Ease::SineInOut;
            vec![c]
        },
    },
    Preset {
        name: "Spin",
        about: "One full turn of the playfield across the whole clip.",
        make: |s, e, b| vec![faded(c(TURN, s, e, 360.0), b, 0.0, Ease::SineInOut)],
    },
    Preset {
        name: "Quarter turns",
        about: "A quarter turn more every bar; where it ends up stays.",
        make: |s, e, _| {
            let mut c = every(c(TURN, s, e, 90.0), 4.0, 1.0, Motion::Step);
            c.ease = Ease::BackOut;
            c.keep = true;
            vec![c]
        },
    },
    Preset {
        name: "Spiral lane",
        about: "The lane curls into a spiral and back out: the notes ride round it.",
        make: |s, e, _| {
            vec![faded(
                c(Target::Lane(2), s, e, 1.0),
                4.0,
                4.0,
                Ease::SineInOut,
            )]
        },
    },
    Preset {
        name: "Zigzag lane",
        about: "The lane bends into a zigzag, and straightens out at the end.",
        make: |s, e, _| {
            vec![faded(
                c(Target::Lane(0), s, e, 1.0),
                2.0,
                2.0,
                Ease::SineInOut,
            )]
        },
    },
    Preset {
        name: "Loop lane",
        about: "The notes come round a loop before they reach the drum.",
        make: |s, e, _| {
            vec![faded(
                c(Target::Lane(3), s, e, 1.0),
                4.0,
                4.0,
                Ease::SineInOut,
            )]
        },
    },
    Preset {
        name: "Turn over",
        about: "The playfield turns over like a page: the drum ends up on the right.",
        make: |s, e, _| {
            vec![faded(
                c(Target::Field(FieldProp::Flip), s, e, 1.0),
                1.0,
                1.0,
                Ease::SineInOut,
            )]
        },
    },
    Preset {
        name: "Earthquake",
        about: "The playfield shakes the whole time.",
        make: |s, e, _| vec![faded(c(Target::Shake, s, e, 9.0), 0.5, 0.5, Ease::Linear)],
    },
    Preset {
        name: "Big note slam",
        about: "Every big note shakes the playfield and kicks the zoom.",
        make: |s, e, _| {
            let big = When::Notes { notes: Notes::Big };
            vec![
                on(c(Target::Shake, s, e, 18.0), big, 1.0),
                on(c(ZOOM, s, e, 0.1), big, 1.0),
            ]
        },
    },
    Preset {
        name: "Kat flash",
        about: "A white flash on every kat.",
        make: |s, e, _| {
            vec![on(
                c(Target::Field(FieldProp::Flash), s, e, 0.3),
                When::Notes { notes: Notes::Kats },
                0.5,
            )]
        },
    },
    Preset {
        name: "Drop",
        about: "One big hit at the start: a flash, a zoom kick and a hard shake.",
        make: |s, e, _| {
            let once = |t, a, len| on(c(t, s, e, a), When::Beats { every: 256.0 }, len);
            vec![
                once(Target::Field(FieldProp::Flash), 0.8, 2.0),
                once(ZOOM, 0.3, 2.0),
                once(Target::Shake, 26.0, 2.0),
            ]
        },
    },
    Preset {
        name: "Build-up",
        about: "The zoom and the dark edges grow slowly at first, then fast, to the end.",
        make: |s, e, b| {
            vec![
                faded(c(ZOOM, s, e, 0.25), b, 0.0, Ease::QuadIn),
                faded(
                    c(Target::Field(FieldProp::Vignette), s, e, 0.7),
                    b,
                    0.0,
                    Ease::QuadIn,
                ),
            ]
        },
    },
    Preset {
        name: "Hide the notes",
        about: "The notes are hidden: play it by memory and by ear. They are still there to hit.",
        make: |s, e, _| vec![c(Target::Notes(None, NoteProp::Hidden), s, e, 1.0)],
    },
    Preset {
        name: "Faint notes",
        about: "The notes fade until they are barely there.",
        make: |s, e, _| {
            vec![faded(
                c(Target::Notes(None, NoteProp::Opacity), s, e, -0.75),
                1.0,
                1.0,
                Ease::Linear,
            )]
        },
    },
    Preset {
        name: "Bouncing notes",
        about: "The notes bob up and down, a beat each way.",
        make: |s, e, _| {
            let mut c = every(
                c(Target::Notes(None, NoteProp::Dy), s, e, 24.0),
                1.0,
                1.0,
                Motion::Swing,
            );
            c.ease = Ease::SineInOut;
            vec![c]
        },
    },
    Preset {
        name: "Swirl in",
        about: "The notes come in curving round the drum.",
        make: |s, e, _| {
            vec![faded(
                c(Target::Notes(None, NoteProp::Swirl), s, e, 70.0),
                1.0,
                1.0,
                Ease::SineInOut,
            )]
        },
    },
    Preset {
        name: "Speed up",
        about: "The notes scroll half again as fast (only what is drawn: the timing is the same).",
        make: |s, e, _| {
            vec![faded(
                c(Target::Field(FieldProp::Scroll), s, e, 0.5),
                1.0,
                1.0,
                Ease::QuadInOut,
            )]
        },
    },
    Preset {
        name: "Echo trail",
        about: "The playfield leaves four fading copies behind it.",
        make: |s, e, _| {
            vec![faded(
                c(Target::Field(FieldProp::Echo), s, e, 4.0),
                1.0,
                1.0,
                Ease::Linear,
            )]
        },
    },
    Preset {
        name: "Kaleidoscope",
        about: "Six copies of the playfield turned about the middle of the screen.",
        make: |s, e, _| {
            vec![faded(
                c(Target::Field(FieldProp::Kaleido), s, e, 6.0),
                0.5,
                0.5,
                Ease::Linear,
            )]
        },
    },
    Preset {
        name: "Colour wash",
        about: "A colour laid over the playfield, its hue drifting all the way round.",
        make: |s, e, b| {
            vec![
                faded(
                    c(Target::Field(FieldProp::Wash), s, e, 0.35),
                    1.0,
                    1.0,
                    Ease::Linear,
                ),
                faded(
                    c(Target::Field(FieldProp::WashHue), s, e, 360.0),
                    b,
                    0.0,
                    Ease::Linear,
                ),
            ]
        },
    },
    Preset {
        name: "Miss glitch",
        about: "A miss tears the picture into slices for a moment.",
        make: |s, e, _| {
            vec![on(
                c(Target::Field(FieldProp::Glitch), s, e, 0.8),
                When::Misses,
                0.75,
            )]
        },
    },
];

impl Editor {
    pub fn clips(&self) -> &[Clip] {
        self.gimmick().map_or(&[][..], |g| &g.clips[..])
    }

    pub fn clip(&self, id: u32) -> Option<&Clip> {
        self.clips().iter().find(|c| c.id == id)
    }

    /// The red lines: when, and how long a beat is from there (for `beat_in`).
    pub fn reds(&self) -> Vec<(f64, f64)> {
        self.chart
            .timing
            .iter()
            .filter(|p| p.is_red())
            .map(|p| (p.time, p.beat_length.max(1.0)))
            .collect()
    }

    /// The length of a beat (ms) at `t`, by the red line in force there.
    pub fn beat_at(&self, t: f64) -> f64 {
        beat_in(&self.reds(), t)
    }

    /// Adds `new` as one undo step; returns their ids.
    pub fn add_clips(&mut self, mut new: Vec<Clip>) -> Vec<u32> {
        let mut next = self.clips().iter().map(|c| c.id).max().unwrap_or(0);
        for c in &mut new {
            next += 1;
            c.id = next;
        }
        let ids: Vec<u32> = new.iter().map(|c| c.id).collect();
        if self.edit_gimmick(false, |g| g.clips.extend(new)) {
            ids
        } else {
            Vec::new()
        }
    }

    /// Changes clip `id` (`drag`: folded into the drag going on).
    pub fn set_clip(&mut self, id: u32, drag: bool, f: impl FnOnce(&mut Clip)) -> bool {
        self.edit_gimmick(drag, |g| {
            if let Some(c) = g.clips.iter_mut().find(|c| c.id == id) {
                f(c);
            }
        })
    }

    pub fn delete_clip(&mut self, id: u32) -> bool {
        self.edit_gimmick(false, |g| g.clips.retain(|c| c.id != id))
    }

    /// A copy of clip `id` right after it; returns the copy's id.
    pub fn duplicate_clip(&mut self, id: u32) -> Option<u32> {
        let mut copy = self.clip(id)?.clone();
        let len = copy.end - copy.start;
        copy.start = copy.end;
        copy.end = copy.start + len;
        self.add_clips(vec![copy]).first().copied()
    }

    /// The moments clip `c` acts at that the chart alone decides (beats and notes; hits and
    /// misses only happen in play). `reds`: from `reds`.
    pub fn clip_moments(&self, c: &Clip, reds: &[(f64, f64)]) -> Vec<f64> {
        match c.when {
            When::Beats { every } => {
                clips::beat_moments(c.start, c.end, every, |t| beat_in(reds, t), 4096)
            }
            When::Notes { notes } => self.chart.objects
                [self.chart.objects.partition_point(|o| o.time < c.start)..]
                .iter()
                .take_while(|o| o.time < c.end)
                .filter(|o| match notes {
                    Notes::All => true,
                    Notes::Dons => o.taiko() == Taiko::Don,
                    Notes::Kats => o.taiko() == Taiko::Kat,
                    Notes::Big => o.big(),
                })
                .map(|o| o.time)
                .take(4096)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The kiai stretch around `t`, if it is in one.
    pub fn kiai_around(&self, t: f64) -> Option<(f64, f64)> {
        let mut from: Option<f64> = None;
        for p in &self.chart.timing {
            match (p.kiai(), from) {
                (true, None) => from = Some(p.time),
                (false, Some(s)) => {
                    if t >= s && t < p.time {
                        return Some((s, p.time));
                    }
                    from = None;
                }
                _ => {}
            }
        }
        let end = self
            .chart
            .objects
            .iter()
            .map(|o| self.end_of(o))
            .fold(0.0, f64::max);
        from.filter(|&s| t >= s && t < end).map(|s| (s, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::Chart;
    use crate::editor::chart::tests::MAP;

    fn tko() -> Editor {
        let mut e = Editor::new(Chart::parse(MAP), std::path::PathBuf::from("a.osu"));
        e.chart.gimmick = Some(Default::default());
        e
    }

    #[test]
    fn every_preset_makes_clips_the_runtime_keeps() {
        for p in PRESETS {
            let clips = (p.make)(1000.0, 9000.0, 16.0);
            assert!(!clips.is_empty(), "{}", p.name);
            let mut kept = clips.clone();
            crate::tko::clips::sanitize(&mut kept);
            assert_eq!(
                kept.len(),
                clips.len(),
                "{} has a clip the runtime drops",
                p.name
            );
            for c in &clips {
                assert!(c.amount != 0.0, "{} does nothing", p.name);
                let t = c.target().unwrap();
                assert!(!target_label(t).is_empty() && !target_about(t).is_empty());
            }
        }
    }

    #[test]
    fn clips_are_added_changed_copied_and_removed_one_undo_step_each() {
        let mut e = tko();
        let made = e.add_clips((PRESETS[0].make)(1000.0, 3000.0, 4.0));
        assert_eq!(made.len(), 1);
        let id = made[0];
        assert!(e.set_clip(id, false, |c| c.amount = 0.2));
        assert_eq!(e.clip(id).unwrap().amount, 0.2);
        let copy = e.duplicate_clip(id).unwrap();
        assert_eq!(
            (e.clip(copy).unwrap().start, e.clip(copy).unwrap().end),
            (3000.0, 5000.0)
        );
        assert_ne!(copy, id);
        assert!(e.delete_clip(id));
        assert!(e.clip(id).is_none());
        assert!(e.undo() && e.clip(id).is_some(), "the delete is undone");
        assert!(e.undo() && e.clip(copy).is_none(), "then the copy");
        assert!(
            e.undo() && e.clip(id).unwrap().amount == 0.06,
            "then the change"
        );
        // A plain .osu has no clips to add to.
        let mut plain = Editor::new(Chart::parse(MAP), std::path::PathBuf::from("a.osu"));
        assert!(
            plain
                .add_clips((PRESETS[0].make)(0.0, 1000.0, 2.0))
                .is_empty()
        );
    }

    #[test]
    fn moments_follow_the_beat_and_the_notes() {
        let mut e = tko();
        let beat = e.beat_at(e.chart.objects[0].time);
        let first = e.chart.objects[0].time;
        let c = every(
            c(ZOOM, first, first + beat * 4.0, 0.1),
            2.0,
            1.0,
            Motion::Kick,
        );
        let reds = e.reds();
        assert_eq!(e.clip_moments(&c, &reds), [first, first + 2.0 * beat]);
        let notes = on(c.clone(), When::Notes { notes: Notes::All }, 1.0);
        let inside = e
            .chart
            .objects
            .iter()
            .filter(|o| o.time >= c.start && o.time < c.end)
            .count();
        assert_eq!(e.clip_moments(&notes, &reds).len(), inside);
        let _ = e.add_clips(vec![c]);
        assert!(describe(&e.clips()[0]).contains("every 2 beats"));
    }
}
