//! Effect clips in the editor: adding, changing and removing them (each one undo step), and the
//! shelf of ready-made effects a click drops onto the timeline.

use super::chart::Taiko;
use super::state::Editor;
use crate::tko::clips::{self, Clip, CopyProp, LANE_SHAPES, Motion, Notes, Target, When};
use crate::tko::gimmick::{FieldProp, NoteProp, PropGroup};
use crate::tko::track::Ease;

/// What a clip can move, in the order the editor offers them.
pub fn targets() -> Vec<Target> {
    let mut out: Vec<Target> = FieldProp::ALL.iter().map(|p| Target::Field(*p)).collect();
    out.insert(4, Target::Shake);
    out.extend((0..LANE_SHAPES.len()).map(Target::Lane));
    out.extend(NoteProp::ALL.iter().map(|p| Target::Notes(None, *p)));
    out.extend(CopyProp::ALL.iter().map(|p| Target::Copy(0, *p)));
    out
}

/// A target as the editor's lists hold it: of all the notes, of the first copy (the group of
/// notes or the copy is picked apart).
pub fn listed(t: Target) -> Target {
    match t {
        Target::Notes(_, p) => Target::Notes(None, p),
        Target::Copy(_, p) => Target::Copy(0, p),
        t => t,
    }
}

/// What a clip can move, in the columns of the editor's menu: a heading, its colour, and the
/// targets under it (all of `targets`, each once).
pub fn target_columns() -> Vec<(&'static str, u32, Vec<Target>)> {
    let field = |g: PropGroup| -> Vec<Target> {
        FieldProp::ALL
            .iter()
            .filter(|p| p.group() == g)
            .map(|p| Target::Field(*p))
            .collect()
    };
    let mut moves = field(PropGroup::Move);
    moves.push(Target::Shake);
    vec![
        ("Playfield", target_colour(Target::Shake), moves),
        (
            "Look",
            target_colour(Target::Field(FieldProp::Flash)),
            field(PropGroup::Look),
        ),
        (
            "Echoes & cuts",
            target_colour(Target::Field(FieldProp::Echo)),
            field(PropGroup::Clones),
        ),
        (
            "Lane",
            target_colour(Target::Lane(0)),
            (0..LANE_SHAPES.len()).map(Target::Lane).collect(),
        ),
        (
            "Notes",
            target_colour(Target::Notes(None, NoteProp::Dx)),
            NoteProp::ALL
                .iter()
                .map(|p| Target::Notes(None, *p))
                .collect(),
        ),
        (
            "Copies",
            target_colour(Target::Copy(0, CopyProp::Show)),
            CopyProp::ALL.iter().map(|p| Target::Copy(0, *p)).collect(),
        ),
    ]
}

/// The amount a clip keeps when what it moves changes from `from` to `to`: the same number of
/// steps, so a big zoom becomes a big turn rather than a default.
pub fn rescale_amount(amount: f64, from: Option<Target>, to: Target) -> f64 {
    let Some(from) = from else {
        return amount_step(to) * 10.0;
    };
    let v = amount / amount_step(from) * amount_step(to);
    let cap = amount_step(to) * 100.0;
    (v.clamp(-cap, cap) * 1000.0).round() / 1000.0
}

/// A clip's target for people.
pub fn target_label(t: Target) -> String {
    match t {
        Target::Layer => "Text / picture".into(),
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
            FieldProp::Fade => "See-through",
            FieldProp::LaneFade => "Lane see-through",
            FieldProp::Blackout => "Blackout",
            FieldProp::Backdrop => "Dark backdrop",
            FieldProp::Echo => "Echoes",
            FieldProp::EchoX => "Echo spread X",
            FieldProp::EchoY => "Echo spread Y",
            FieldProp::EchoRot => "Echo turn",
            FieldProp::EchoZoom => "Echo size",
            FieldProp::EchoFade => "Echo fade",
            FieldProp::Kaleido => "Kaleidoscope",
            FieldProp::Slices => "Cut in pieces",
            FieldProp::SliceY => "Pieces up / down",
            FieldProp::SliceX => "Pieces apart",
            FieldProp::Drum => "Move the drum",
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
                NoteProp::StretchX => "stretch wide",
                NoteProp::StretchY => "stretch tall",
                NoteProp::Tint => "tint",
                NoteProp::TintHue => "tint colour",
            };
            match g {
                None => format!("Notes: {what}"),
                Some(g) => format!("Group {}: {what}", g + 1),
            }
        }
        Target::Copy(k, p) => {
            let what = match p {
                CopyProp::Show => "shows",
                CopyProp::X => "move sideways",
                CopyProp::Y => "move up / down",
                CopyProp::Turn => "turn",
                CopyProp::Zoom => "size",
                CopyProp::LaneFade => "lane see-through",
            };
            format!("Copy {}: {what}", k + 1)
        }
    }
}

/// What a target is about, for its tooltip.
pub fn target_about(t: Target) -> &'static str {
    match t {
        Target::Layer => {
            "Shows text or a picture from the map's folder for the block's length. Repeating, it pulses: the amount is how much bigger, in percent."
        }
        Target::Lane(_) => {
            "Bends the lane into this shape: the notes travel along it. The amount is how far, 0 straight to 1 the whole shape; fade it in and out to morph."
        }
        Target::Shake => "Jolts the whole playfield in random directions. The amount is how far.",
        Target::Field(p) => p.about(),
        Target::Notes(_, NoteProp::Opacity) => {
            "Takes this much away from how solid the notes are: -0.7 leaves them faint."
        }
        Target::Notes(_, p) => p.about(),
        Target::Copy(_, p) => p.about(),
    }
}

/// The colour of a target's clips: moves blue, looks pink, clones violet, shakes orange, notes
/// green.
pub fn target_colour(t: Target) -> u32 {
    match t {
        Target::Layer => 0xE8E8F0,
        Target::Shake => 0xFFA040,
        Target::Lane(_) => 0xFFD34D,
        Target::Notes(..) => 0x5FD49A,
        Target::Copy(..) => 0x4DE0E0,
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
        Target::Copy(_, p) => p.range().1 - p.range().0,
        Target::Shake | Target::Layer => 400.0,
        Target::Lane(_) => 1.0,
    };
    10f64.powf((span / 100.0).log10().floor()).clamp(0.01, 10.0)
}

/// A clip in words: what, and how often ("Zoom, every 2 beats").
pub fn describe(c: &Clip) -> String {
    let what = match c.layer.as_deref().map(|l| &l.show) {
        Some(clips::Show::Text { text }) => format!("\u{201c}{text}\u{201d}"),
        Some(clips::Show::Picture { file }) => file.clone(),
        None => c.target().map_or_else(|| c.what.clone(), target_label),
    };
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
    /// The shelf's tab it is on.
    pub shelf: Shelf,
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

/// A clip on copy `k` of the playfield.
fn cp(k: usize, p: CopyProp, start: f64, end: f64, amount: f64) -> Clip {
    c(Target::Copy(k, p), start, end, amount)
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
const FADE: Target = Target::Field(FieldProp::Fade);
const LANE_FADE: Target = Target::Field(FieldProp::LaneFade);
const BLACKOUT: Target = Target::Field(FieldProp::Blackout);
const BACKDROP: Target = Target::Field(FieldProp::Backdrop);
const SLICES: Target = Target::Field(FieldProp::Slices);
const SLICE_Y: Target = Target::Field(FieldProp::SliceY);
const SLICE_X: Target = Target::Field(FieldProp::SliceX);

/// The tabs of the shelf, by what the effects in them do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shelf {
    /// The whole playfield moves: pulses, turns, zooms, shakes.
    Move,
    /// The lane bends into a shape.
    Lane,
    /// The notes themselves.
    Notes,
    /// Flashes, colours, echoes and glitches.
    Look,
    /// Copies of the playfield moved on their own, and the picture cut in pieces.
    Copies,
    /// Words and pictures on screen.
    Text,
}

impl Shelf {
    pub const ALL: [Shelf; 6] = [
        Shelf::Move,
        Shelf::Lane,
        Shelf::Notes,
        Shelf::Look,
        Shelf::Copies,
        Shelf::Text,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Shelf::Move => "Playfield",
            Shelf::Lane => "Lane",
            Shelf::Notes => "Notes",
            Shelf::Look => "Look",
            Shelf::Copies => "Copies & cuts",
            Shelf::Text => "Text & pictures",
        }
    }

    /// The colour its tab wears: that of what its effects move.
    pub fn colour(self) -> u32 {
        match self {
            Shelf::Move => 0x5BA8FF,
            Shelf::Lane => 0xFFD34D,
            Shelf::Notes => 0x5FD49A,
            Shelf::Look => 0xFF6FB0,
            Shelf::Copies => 0x4DE0E0,
            Shelf::Text => 0xE8E8F0,
        }
    }

    /// The shelf's effects in this tab, by their index in `PRESETS`.
    pub fn presets(self) -> Vec<usize> {
        (0..PRESETS.len())
            .filter(|&i| PRESETS[i].shelf == self)
            .collect()
    }
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Beat pulse",
        shelf: Shelf::Move,
        about: "The playfield breathes: a small zoom kick on every beat.",
        make: |s, e, _| vec![every(c(ZOOM, s, e, 0.06), 1.0, 0.75, Motion::Kick)],
    },
    Preset {
        name: "Every 2nd beat",
        shelf: Shelf::Move,
        about: "A bigger zoom kick on every second beat.",
        make: |s, e, _| vec![every(c(ZOOM, s, e, 0.12), 2.0, 1.0, Motion::Kick)],
    },
    Preset {
        name: "Sway",
        shelf: Shelf::Move,
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
        shelf: Shelf::Move,
        about: "The playfield rocks, tilting one way then the other every 2 beats.",
        make: |s, e, _| {
            let mut c = every(c(TURN, s, e, 6.0), 2.0, 2.0, Motion::Swing);
            c.ease = Ease::SineInOut;
            vec![c]
        },
    },
    Preset {
        name: "Spin",
        shelf: Shelf::Move,
        about: "One full turn of the playfield across the whole clip.",
        make: |s, e, b| vec![faded(c(TURN, s, e, 360.0), b, 0.0, Ease::SineInOut)],
    },
    Preset {
        name: "Quarter turns",
        shelf: Shelf::Move,
        about: "A quarter turn more every bar; where it ends up stays.",
        make: |s, e, _| {
            let mut c = every(c(TURN, s, e, 90.0), 4.0, 1.0, Motion::Step);
            c.ease = Ease::BackOut;
            c.keep = true;
            vec![c]
        },
    },
    Preset {
        name: "Zoom in",
        shelf: Shelf::Move,
        about: "The playfield grows to half as big again over the whole clip.",
        make: |s, e, b| vec![faded(c(ZOOM, s, e, 0.5), b, 0.0, Ease::SineInOut)],
    },
    Preset {
        name: "Zoom out",
        shelf: Shelf::Move,
        about: "The playfield shrinks to two thirds over the whole clip.",
        make: |s, e, b| vec![faded(c(ZOOM, s, e, -0.33), b, 0.0, Ease::SineInOut)],
    },
    Preset {
        name: "Spin + zoom",
        shelf: Shelf::Move,
        about: "One full turn while the playfield grows, over the whole clip: two clips, one row each.",
        make: |s, e, b| {
            vec![
                faded(c(TURN, s, e, 360.0), b, 0.0, Ease::SineInOut),
                faded(c(ZOOM, s, e, 0.5), b, 0.0, Ease::SineInOut),
            ]
        },
    },
    Preset {
        name: "Turn over",
        shelf: Shelf::Move,
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
        shelf: Shelf::Move,
        about: "The playfield shakes the whole time.",
        make: |s, e, _| vec![faded(c(Target::Shake, s, e, 9.0), 0.5, 0.5, Ease::Linear)],
    },
    Preset {
        name: "Big note slam",
        shelf: Shelf::Move,
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
        name: "Spiral lane",
        shelf: Shelf::Lane,
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
        shelf: Shelf::Lane,
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
        shelf: Shelf::Lane,
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
        name: "Hide the notes",
        shelf: Shelf::Notes,
        about: "The notes are hidden: play it by memory and by ear. They are still there to hit.",
        make: |s, e, _| vec![c(Target::Notes(None, NoteProp::Hidden), s, e, 1.0)],
    },
    Preset {
        name: "Faint notes",
        shelf: Shelf::Notes,
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
        shelf: Shelf::Notes,
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
        name: "Rainbow notes",
        shelf: Shelf::Notes,
        about: "The notes take on a colour that walks all the way round the rainbow.",
        make: |s, e, b| {
            vec![
                faded(
                    c(Target::Notes(None, NoteProp::Tint), s, e, 0.8),
                    1.0,
                    1.0,
                    Ease::SineInOut,
                ),
                faded(
                    c(Target::Notes(None, NoteProp::TintHue), s, e, 360.0),
                    b,
                    0.0,
                    Ease::Linear,
                ),
            ]
        },
    },
    Preset {
        name: "Squash on the beat",
        shelf: Shelf::Notes,
        about: "On every beat the notes squash flat and wide, then spring back.",
        make: |s, e, _| {
            vec![
                every(
                    c(Target::Notes(None, NoteProp::StretchX), s, e, 0.5),
                    1.0,
                    0.5,
                    Motion::Kick,
                ),
                every(
                    c(Target::Notes(None, NoteProp::StretchY), s, e, -0.35),
                    1.0,
                    0.5,
                    Motion::Kick,
                ),
            ]
        },
    },
    Preset {
        name: "Swirl in",
        shelf: Shelf::Notes,
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
        shelf: Shelf::Notes,
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
        name: "Kat flash",
        shelf: Shelf::Look,
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
        shelf: Shelf::Look,
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
        shelf: Shelf::Look,
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
        name: "Echo trail",
        shelf: Shelf::Look,
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
        shelf: Shelf::Look,
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
        shelf: Shelf::Look,
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
        shelf: Shelf::Look,
        about: "A miss tears the picture into slices for a moment.",
        make: |s, e, _| {
            vec![on(
                c(Target::Field(FieldProp::Glitch), s, e, 0.8),
                When::Misses,
                0.75,
            )]
        },
    },
    Preset {
        name: "Fade to black",
        shelf: Shelf::Look,
        about: "The screen sinks into black over the block's length, notes and all.",
        make: |s, e, b| vec![faded(c(BLACKOUT, s, e, 1.0), b, 0.0, Ease::SineIn)],
    },
    Preset {
        name: "Blackout",
        shelf: Shelf::Look,
        about: "The screen goes black at once and comes back over half a beat at the end. Text over the lane still shows.",
        make: |s, e, _| vec![faded(c(BLACKOUT, s, e, 1.0), 0.0, 0.5, Ease::Linear)],
    },
    Preset {
        name: "Dark pulse",
        shelf: Shelf::Look,
        about: "The screen dips towards black on every beat and comes back.",
        make: |s, e, _| vec![every(c(BLACKOUT, s, e, 0.85), 1.0, 0.5, Motion::Kick)],
    },
    Preset {
        name: "Ghost playfield",
        shelf: Shelf::Look,
        about: "The whole playfield turns faint, notes too. They are still there to hit.",
        make: |s, e, _| vec![faded(c(FADE, s, e, 0.75), 1.0, 1.0, Ease::SineInOut)],
    },
    Preset {
        name: "Only the notes",
        shelf: Shelf::Look,
        about: "Black behind the playfield and the lane gone: notes and drum alone on black.",
        make: |s, e, _| {
            vec![
                faded(c(BACKDROP, s, e, 1.0), 1.0, 1.0, Ease::SineInOut),
                faded(c(LANE_FADE, s, e, 1.0), 1.0, 1.0, Ease::SineInOut),
            ]
        },
    },
    Preset {
        name: "Mirror twin",
        shelf: Shelf::Copies,
        about: "An upside-down copy of the playfield above it, the drum on the right, its notes coming the other way.",
        make: |s, e, _| {
            vec![
                faded(cp(0, CopyProp::Show, s, e, 1.0), 1.0, 1.0, Ease::SineInOut),
                cp(0, CopyProp::Turn, s, e, 180.0),
                cp(0, CopyProp::Y, s, e, -130.0),
            ]
        },
    },
    Preset {
        name: "Triple lane",
        shelf: Shelf::Copies,
        about: "Two copies slide out above and below the playfield.",
        make: |s, e, _| {
            vec![
                faded(cp(0, CopyProp::Show, s, e, 1.0), 1.0, 1.0, Ease::SineInOut),
                faded(cp(0, CopyProp::Y, s, e, -110.0), 1.0, 1.0, Ease::BackOut),
                faded(cp(1, CopyProp::Show, s, e, 1.0), 1.0, 1.0, Ease::SineInOut),
                faded(cp(1, CopyProp::Y, s, e, 110.0), 1.0, 1.0, Ease::BackOut),
            ]
        },
    },
    Preset {
        name: "Shadow clones",
        shelf: Shelf::Copies,
        about: "Two faint copies sway out to either side and back, every two beats.",
        make: |s, e, _| {
            vec![
                faded(cp(0, CopyProp::Show, s, e, 0.45), 1.0, 1.0, Ease::SineInOut),
                every(cp(0, CopyProp::X, s, e, 60.0), 2.0, 2.0, Motion::Swing),
                faded(cp(1, CopyProp::Show, s, e, 0.45), 1.0, 1.0, Ease::SineInOut),
                every(cp(1, CopyProp::X, s, e, -60.0), 2.0, 2.0, Motion::Swing),
            ]
        },
    },
    Preset {
        name: "Spinning ghost",
        shelf: Shelf::Copies,
        about: "A faint, smaller copy of the notes alone turns round behind the playfield.",
        make: |s, e, b| {
            vec![
                faded(cp(0, CopyProp::Show, s, e, 0.5), 1.0, 1.0, Ease::SineInOut),
                cp(0, CopyProp::LaneFade, s, e, 1.0),
                cp(0, CopyProp::Zoom, s, e, -0.3),
                faded(cp(0, CopyProp::Turn, s, e, 360.0), b, 0.0, Ease::Linear),
            ]
        },
    },
    Preset {
        name: "Broken lane",
        shelf: Shelf::Copies,
        about: "The picture cut into four pieces that sway up and down against each other.",
        make: |s, e, _| {
            vec![
                c(SLICES, s, e, 4.0),
                every(c(SLICE_Y, s, e, 14.0), 2.0, 2.0, Motion::Swing),
            ]
        },
    },
    Preset {
        name: "Shatter on the beat",
        shelf: Shelf::Copies,
        about: "Six pieces jump apart on every beat and snap back together.",
        make: |s, e, _| {
            vec![
                c(SLICES, s, e, 6.0),
                every(c(SLICE_Y, s, e, 28.0), 1.0, 0.75, Motion::Kick),
                every(c(SLICE_X, s, e, 30.0), 1.0, 0.75, Motion::Kick),
            ]
        },
    },
    Preset {
        name: "Split open",
        shelf: Shelf::Copies,
        about: "The playfield splits down the middle and the halves slide apart, then close again.",
        make: |s, e, _| {
            vec![
                c(SLICES, s, e, 2.0),
                faded(c(SLICE_X, s, e, 140.0), 2.0, 2.0, Ease::SineInOut),
            ]
        },
    },
    Preset {
        name: "Text",
        shelf: Shelf::Text,
        about: "Words on screen in the game's font, fading in and out. Click it to type them.",
        make: |s, e, _| vec![faded(layer(text("Text"), s, e), 1.0, 1.0, Ease::QuadOut)],
    },
    Preset {
        name: "Fly-in text",
        shelf: Shelf::Text,
        about: "Words that slide in from the left and settle in the middle.",
        make: |s, e, _| {
            let mut c = faded(layer(text("Here we go"), s, e), 0.5, 1.0, Ease::CubicOut);
            if let Some(l) = c.layer.as_deref_mut() {
                l.to = Some(l.from);
                l.from.x -= 300.0;
            }
            vec![c]
        },
    },
    Preset {
        name: "Pulsing text",
        shelf: Shelf::Text,
        about: "Words that jump a little bigger on every beat.",
        make: |s, e, _| {
            vec![every(
                Clip {
                    amount: 15.0,
                    ..faded(layer(text("Text"), s, e), 0.5, 0.5, Ease::QuadOut)
                },
                1.0,
                0.75,
                Motion::Kick,
            )]
        },
    },
    Preset {
        name: "Picture",
        shelf: Shelf::Text,
        about: "A picture from the map's folder (put a .png or .jpg there), fading in and out.",
        make: |s, e, _| vec![faded(layer(picture(), s, e), 1.0, 1.0, Ease::QuadOut)],
    },
    Preset {
        name: "Logo on the beat",
        shelf: Shelf::Text,
        about: "A picture from the map's folder, behind the notes, kicking bigger on every beat.",
        make: |s, e, _| {
            let mut c = every(
                Clip {
                    amount: 8.0,
                    ..faded(layer(picture(), s, e), 1.0, 1.0, Ease::QuadOut)
                },
                1.0,
                0.75,
                Motion::Kick,
            );
            if let Some(l) = c.layer.as_deref_mut() {
                l.behind = true;
                l.from.size = 160.0;
                l.from.y = 240.0;
                l.from.alpha = 0.5;
            }
            vec![c]
        },
    },
    // ---- added later: kept at the end so the older ones keep their place ----
    Preset {
        name: "Heartbeat",
        shelf: Shelf::Move,
        about: "On every beat the playfield thumps bigger and the edges of the screen darken with it.",
        make: |s, e, _| {
            vec![
                every(c(ZOOM, s, e, 0.12), 1.0, 0.5, Motion::Kick),
                every(
                    c(Target::Field(FieldProp::Vignette), s, e, 0.45),
                    1.0,
                    0.75,
                    Motion::Kick,
                ),
            ]
        },
    },
    Preset {
        name: "Drunk",
        shelf: Shelf::Move,
        about: "The playfield sways, drifts and bobs out of step with itself, as if the floor moved.",
        make: |s, e, _| {
            let sway = |t, a, beats| {
                let mut c = every(c(t, s, e, a), beats, beats, Motion::Swing);
                c.ease = Ease::SineInOut;
                c
            };
            vec![
                sway(TURN, 6.0, 2.0),
                sway(Target::Field(FieldProp::X), 28.0, 3.0),
                sway(Target::Field(FieldProp::Y), 12.0, 1.5),
            ]
        },
    },
    Preset {
        name: "Barrel roll",
        shelf: Shelf::Move,
        about: "A whole turn every bar, quick in the middle and easing out at the ends.",
        make: |s, e, _| {
            let mut c = every(c(TURN, s, e, 360.0), 4.0, 2.0, Motion::Step);
            c.ease = Ease::QuadInOut;
            vec![c]
        },
    },
    Preset {
        name: "Vertigo",
        shelf: Shelf::Move,
        about: "The playfield slowly comes closer and turns, leaving a trail that shrinks away behind it.",
        make: |s, e, b| {
            vec![
                faded(c(ZOOM, s, e, 0.6), b, 0.0, Ease::QuadIn),
                faded(c(TURN, s, e, 90.0), b, 0.0, Ease::SineInOut),
                faded(
                    c(Target::Field(FieldProp::Echo), s, e, 3.0),
                    2.0,
                    1.0,
                    Ease::Linear,
                ),
                c(Target::Field(FieldProp::EchoZoom), s, e, -0.08),
            ]
        },
    },
    Preset {
        name: "Mega drop",
        shelf: Shelf::Move,
        about: "The drop of drops: a flash, a zoom kick, a hard shake, colour fringes and a tear at the start, then a thump on every beat with a trail.",
        make: |s, e, _| {
            let once = |t, a, len| on(c(t, s, e, a), When::Beats { every: 256.0 }, len);
            vec![
                once(Target::Field(FieldProp::Flash), 1.0, 2.0),
                once(ZOOM, 0.5, 2.0),
                once(Target::Shake, 30.0, 2.0),
                once(Target::Field(FieldProp::Chroma), 40.0, 2.0),
                once(Target::Field(FieldProp::Glitch), 0.7, 0.5),
                every(c(ZOOM, s, e, 0.08), 1.0, 0.5, Motion::Kick),
                faded(
                    c(Target::Field(FieldProp::Echo), s, e, 3.0),
                    1.0,
                    1.0,
                    Ease::Linear,
                ),
            ]
        },
    },
    Preset {
        name: "Chaos",
        shelf: Shelf::Move,
        about: "Every note tips the playfield one way and the next, kicks the zoom and splits the colours; big notes shake it.",
        make: |s, e, _| {
            let all = When::Notes { notes: Notes::All };
            let mut tip = on(c(TURN, s, e, 10.0), all, 1.0);
            tip.motion = Motion::Swing;
            vec![
                tip,
                on(c(ZOOM, s, e, 0.07), all, 0.5),
                on(c(Target::Field(FieldProp::Chroma), s, e, 18.0), all, 0.5),
                on(
                    c(Target::Shake, s, e, 14.0),
                    When::Notes { notes: Notes::Big },
                    1.0,
                ),
            ]
        },
    },
    Preset {
        name: "Strobe",
        shelf: Shelf::Look,
        about: "Hard white flashes twice a beat. Use it short: it is a lot.",
        make: |s, e, _| {
            let mut c = every(
                c(Target::Field(FieldProp::Flash), s, e, 0.55),
                0.5,
                0.25,
                Motion::Kick,
            );
            c.ease = Ease::Linear;
            vec![c]
        },
    },
    Preset {
        name: "Neon night",
        shelf: Shelf::Look,
        about: "The background goes dark, a colour lies over the playfield and jumps to a new hue on every beat, with a faint colour fringe.",
        make: |s, e, _| {
            vec![
                faded(c(BACKDROP, s, e, 0.85), 1.0, 1.0, Ease::SineInOut),
                faded(
                    c(Target::Field(FieldProp::Wash), s, e, 0.3),
                    1.0,
                    1.0,
                    Ease::SineInOut,
                ),
                every(
                    c(Target::Field(FieldProp::WashHue), s, e, 60.0),
                    1.0,
                    0.25,
                    Motion::Step,
                ),
                faded(
                    c(Target::Field(FieldProp::Chroma), s, e, 6.0),
                    1.0,
                    1.0,
                    Ease::Linear,
                ),
            ]
        },
    },
    Preset {
        name: "Hyperspace",
        shelf: Shelf::Look,
        about: "Six copies trail straight back behind the playfield, each smaller than the last, like a tunnel, with a push on every beat.",
        make: |s, e, _| {
            vec![
                faded(
                    c(Target::Field(FieldProp::Echo), s, e, 6.0),
                    1.0,
                    1.0,
                    Ease::Linear,
                ),
                // The trail goes straight back (its default steps it to the side).
                c(Target::Field(FieldProp::EchoX), s, e, 30.0),
                c(Target::Field(FieldProp::EchoZoom), s, e, -0.1),
                c(Target::Field(FieldProp::EchoFade), s, e, 0.25),
                every(c(ZOOM, s, e, 0.05), 1.0, 0.5, Motion::Kick),
            ]
        },
    },
    Preset {
        name: "VHS tape",
        shelf: Shelf::Look,
        about: "A worn tape: colour fringes that swim, a cold blue tint and a tear every bar.",
        make: |s, e, _| {
            let mut swim = every(
                c(Target::Field(FieldProp::Chroma), s, e, 10.0),
                2.0,
                2.0,
                Motion::Swing,
            );
            swim.ease = Ease::SineInOut;
            vec![
                swim,
                every(
                    c(Target::Field(FieldProp::Glitch), s, e, 0.35),
                    4.0,
                    0.25,
                    Motion::Kick,
                ),
                faded(
                    c(Target::Field(FieldProp::Wash), s, e, 0.12),
                    1.0,
                    1.0,
                    Ease::Linear,
                ),
                c(Target::Field(FieldProp::WashHue), s, e, 210.0),
            ]
        },
    },
    Preset {
        name: "Spin trail",
        shelf: Shelf::Look,
        about: "The playfield makes a whole turn, leaving a fan of copies, each turned a little further back.",
        make: |s, e, b| {
            vec![
                faded(
                    c(Target::Field(FieldProp::Echo), s, e, 5.0),
                    1.0,
                    1.0,
                    Ease::Linear,
                ),
                c(Target::Field(FieldProp::EchoX), s, e, 30.0),
                c(Target::Field(FieldProp::EchoRot), s, e, 12.0),
                faded(c(TURN, s, e, 360.0), b, 0.0, Ease::SineInOut),
            ]
        },
    },
    Preset {
        name: "Disco notes",
        shelf: Shelf::Notes,
        about: "The notes take a bright colour that jumps to the next one on every beat.",
        make: |s, e, _| {
            vec![
                faded(
                    c(Target::Notes(None, NoteProp::Tint), s, e, 0.9),
                    0.5,
                    0.5,
                    Ease::Linear,
                ),
                every(
                    c(Target::Notes(None, NoteProp::TintHue), s, e, 90.0),
                    1.0,
                    0.25,
                    Motion::Step,
                ),
            ]
        },
    },
    Preset {
        name: "Jelly notes",
        shelf: Shelf::Notes,
        about: "The notes wobble like jelly: taller, then wider, back and forth on every beat.",
        make: |s, e, _| {
            let wobble = |p, a| {
                let mut c = every(c(Target::Notes(None, p), s, e, a), 1.0, 1.0, Motion::Swing);
                c.ease = Ease::SineInOut;
                c
            };
            vec![
                wobble(NoteProp::StretchY, 0.25),
                wobble(NoteProp::StretchX, -0.2),
            ]
        },
    },
    Preset {
        name: "Floating notes",
        shelf: Shelf::Notes,
        about: "The notes bob gently up and down over the lane as they come in.",
        make: |s, e, _| {
            let mut c = every(
                c(Target::Notes(None, NoteProp::Dy), s, e, 30.0),
                2.0,
                2.0,
                Motion::Swing,
            );
            c.ease = Ease::SineInOut;
            vec![c]
        },
    },
    Preset {
        name: "Blinking notes",
        shelf: Shelf::Notes,
        about: "The notes all but vanish on every beat and fade back in: read the rhythm, not the lane.",
        make: |s, e, _| {
            vec![every(
                c(Target::Notes(None, NoteProp::Opacity), s, e, -0.9),
                1.0,
                0.6,
                Motion::Kick,
            )]
        },
    },
    Preset {
        name: "Clone army",
        shelf: Shelf::Copies,
        about: "Four faint copies slide out, two above and two below the playfield.",
        make: |s, e, _| {
            [
                (0, -100.0, 0.5),
                (1, 100.0, 0.5),
                (2, -200.0, 0.3),
                (3, 200.0, 0.3),
            ]
            .into_iter()
            .flat_map(|(k, y, show)| {
                [
                    faded(cp(k, CopyProp::Show, s, e, show), 1.0, 1.0, Ease::SineInOut),
                    faded(cp(k, CopyProp::Y, s, e, y), 1.0, 1.0, Ease::BackOut),
                    cp(k, CopyProp::LaneFade, s, e, 0.3),
                ]
            })
            .collect()
        },
    },
    Preset {
        name: "Tunnel",
        shelf: Shelf::Copies,
        about: "Three ever bigger, ever fainter copies frame the playfield and rock against each other.",
        make: |s, e, _| {
            [
                (0, 0.35, 0.4, 6.0),
                (1, 0.8, 0.25, -9.0),
                (2, 1.4, 0.15, 12.0),
            ]
            .into_iter()
            .flat_map(|(k, zoom, show, rock)| {
                let mut turn = every(cp(k, CopyProp::Turn, s, e, rock), 4.0, 4.0, Motion::Swing);
                turn.ease = Ease::SineInOut;
                [
                    faded(cp(k, CopyProp::Show, s, e, show), 1.0, 1.0, Ease::SineInOut),
                    faded(cp(k, CopyProp::Zoom, s, e, zoom), 2.0, 1.0, Ease::QuadOut),
                    turn,
                ]
            })
            .collect()
        },
    },
];

fn layer(show: clips::Show, s: f64, e: f64) -> Clip {
    let mut c = Clip::new(Target::Layer, s, e);
    if let Some(l) = c.layer.as_deref_mut() {
        l.show = show;
    }
    c
}

fn text(t: &str) -> clips::Show {
    clips::Show::Text { text: t.into() }
}

/// A picture to be filled in with one from the map's folder when it is dropped.
fn picture() -> clips::Show {
    clips::Show::Picture {
        file: String::new(),
    }
}

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
    pub fn add_clips(&mut self, new: Vec<Clip>) -> Vec<u32> {
        self.add_clips_as(new, false)
    }

    /// Adds `new` (`drag`: as the first step of a drag still going on, so moving them after
    /// is the same undo step); returns their ids.
    pub fn add_clips_as(&mut self, mut new: Vec<Clip>, drag: bool) -> Vec<u32> {
        let mut next = self.clips().iter().map(|c| c.id).max().unwrap_or(0);
        for c in &mut new {
            next += 1;
            c.id = next;
        }
        let ids: Vec<u32> = new.iter().map(|c| c.id).collect();
        if self.edit_gimmick(drag, |g| g.clips.extend(new)) {
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

    /// Deletes clips `ids` in one undo step.
    pub fn delete_clips(&mut self, ids: &[u32]) -> bool {
        self.edit_gimmick(false, |g| g.clips.retain(|c| !ids.contains(&c.id)))
    }

    /// Whether blocks at `places` (start, end, row), moved by `drow` rows, stay on the rows and
    /// meet no other block on theirs (blocks `skip` aside; touching ends are fine).
    fn rows_free(&self, places: &[(f64, f64, u8)], drow: i32, skip: &[u32]) -> bool {
        places.iter().all(|&(start, end, row)| {
            let r = row as i32 + drow;
            (0..clips::LANES as i32).contains(&r)
                && self.clips().iter().all(|c| {
                    skip.contains(&c.id)
                        || c.lane as i32 != r
                        || c.start >= end - 0.5
                        || c.end <= start + 0.5
                })
        })
    }

    /// The row shift nearest `drow` that leaves blocks at `places` on rows of their own: `drow`
    /// itself when nothing is in the way, else the closest free one (below first). None when
    /// no shift fits.
    pub fn free_rows(&self, places: &[(f64, f64, u8)], drow: i32, skip: &[u32]) -> Option<i32> {
        (0..clips::LANES as i32 * 2)
            .flat_map(|d| [drow + d, drow - d])
            .find(|&d| self.rows_free(places, d, skip))
    }

    /// Adds `new` where each one has its row to itself, moving them all up or down together as
    /// little as it takes (`drag`: the first step of a drag still going on).
    pub fn add_clips_free(&mut self, mut new: Vec<Clip>, drag: bool) -> Vec<u32> {
        let places: Vec<(f64, f64, u8)> = new.iter().map(|c| (c.start, c.end, c.lane)).collect();
        if let Some(d) = self.free_rows(&places, 0, &[]) {
            for c in &mut new {
                c.lane = (c.lane as i32 + d) as u8;
            }
        }
        self.add_clips_as(new, drag)
    }

    /// The earliest a block may start: where play starts, before the song when the first note
    /// comes early or the map has an audio lead-in (`beatmap::play_start`), so the playfield can
    /// be set before the song begins. 0 otherwise.
    pub fn clip_floor(&self) -> f64 {
        let first = self
            .chart
            .objects
            .iter()
            .map(|o| o.time)
            .fold(f64::INFINITY, f64::min);
        let lead_in = self
            .chart
            .get_key("General", "AudioLeadIn")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(0.0);
        let first = if first.is_finite() { first } else { 0.0 };
        crate::beatmap::play_start(first, lead_in.max(0.0))
    }

    /// The stretch block `id` can grow into on its row, up to the blocks either side, and back no
    /// further than `clip_floor`.
    pub fn clip_room(&self, id: u32) -> (f64, f64) {
        let floor = self.clip_floor();
        let Some(me) = self.clip(id) else {
            return (floor, f64::INFINITY);
        };
        let row = self
            .clips()
            .iter()
            .filter(|c| c.id != id && c.lane == me.lane);
        row.fold((floor, f64::INFINITY), |(lo, hi), c| {
            if c.end <= me.start + 0.5 {
                (lo.max(c.end), hi)
            } else if c.start >= me.end - 0.5 {
                (lo, hi.min(c.start))
            } else {
                (lo, hi)
            }
        })
    }

    /// Blocks copied before (`copied`), laid down from `at` on with their spacing and rows kept,
    /// moved to free rows if those are taken; one undo step. Returns the new ids in order.
    pub fn paste_clips(&mut self, copied: &[Clip], at: f64) -> Vec<u32> {
        let first = copied.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
        if !first.is_finite() {
            return Vec::new();
        }
        let shift = at.max(0.0) - first;
        let new = copied
            .iter()
            .cloned()
            .map(|mut c| {
                c.start += shift;
                c.end += shift;
                c
            })
            .collect();
        self.add_clips_free(new, false)
    }

    /// Copies of clips `ids` right after the stretch they cover together, on the same rows (or
    /// the nearest free ones), in one undo step; returns the copies' ids.
    pub fn duplicate_clips(&mut self, ids: &[u32]) -> Vec<u32> {
        let picked: Vec<Clip> = self
            .clips()
            .iter()
            .filter(|c| ids.contains(&c.id))
            .cloned()
            .collect();
        let (a, b) = picked
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| {
                (a.min(c.start), b.max(c.end))
            });
        if picked.is_empty() {
            return Vec::new();
        }
        let shift = b - a;
        let copies = picked
            .into_iter()
            .map(|mut c| {
                c.start += shift;
                c.end += shift;
                c
            })
            .collect();
        self.add_clips_free(copies, false)
    }

    /// Moves clips by `dt` ms and `drows` rows from where `from` (id, start, end, row) had them,
    /// kept on the timeline and the rows (`drag`: folded into the drag going on).
    pub fn move_clips(
        &mut self,
        from: &[(u32, f64, f64, u8)],
        dt: f64,
        drows: i32,
        drag: bool,
    ) -> bool {
        let earliest = from.iter().map(|f| f.1).fold(f64::INFINITY, f64::min);
        let dt = dt.max(self.clip_floor().min(earliest) - earliest);
        let low = from.iter().map(|f| f.3 as i32).min().unwrap_or(0);
        let high = from.iter().map(|f| f.3 as i32).max().unwrap_or(0);
        let drows = drows.clamp(-low, clips::LANES as i32 - 1 - high);
        // Over another block, they go to the nearest rows that are free there.
        let ids: Vec<u32> = from.iter().map(|f| f.0).collect();
        let places: Vec<(f64, f64, u8)> = from
            .iter()
            .map(|&(_, start, end, row)| (start + dt, end + dt, row))
            .collect();
        let drows = self.free_rows(&places, drows, &ids).unwrap_or(drows);
        self.edit_gimmick(drag, |g| {
            for &(id, start, end, row) in from {
                if let Some(c) = g.clips.iter_mut().find(|c| c.id == id) {
                    c.start = start + dt;
                    c.end = end + dt;
                    c.lane = (row as i32 + drows) as u8;
                }
            }
        })
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
                // The game acts on real notes only.
                .filter(|o| !o.fake)
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
            let mut clips = (p.make)(1000.0, 9000.0, 16.0);
            assert!(!clips.is_empty(), "{}", p.name);
            // A picture is picked from the map's folder when it is dropped.
            for c in &mut clips {
                if let Some(l) = c.layer.as_deref_mut()
                    && let clips::Show::Picture { file } = &mut l.show
                {
                    assert!(file.is_empty(), "{}", p.name);
                    *file = "bg.png".into();
                }
            }
            let mut kept = clips.clone();
            crate::tko::clips::sanitize(&mut kept);
            assert_eq!(
                kept.len(),
                clips.len(),
                "{} has a clip the runtime drops",
                p.name
            );
            for c in &clips {
                // Text and pictures show something even without an amount.
                assert!(
                    c.amount != 0.0 || c.layer.is_some(),
                    "{} does nothing",
                    p.name
                );
                let t = c.target().unwrap();
                assert!(!target_label(t).is_empty() && !target_about(t).is_empty());
            }
        }
    }

    #[test]
    fn copy_targets_keep_which_copy_they_move() {
        use crate::tko::clips::CopyProp;
        let c = Clip::new(Target::Copy(2, CopyProp::Turn), 0.0, 1.0);
        assert_eq!((c.what.as_str(), c.group), ("copy.turn", 3));
        assert_eq!(c.target(), Some(Target::Copy(2, CopyProp::Turn)));
        assert_eq!(listed(c.target().unwrap()), Target::Copy(0, CopyProp::Turn));
        assert_eq!(
            target_label(Target::Copy(2, CopyProp::Turn)),
            "Copy 3: turn"
        );
        let all: usize = target_columns().iter().map(|(_, _, t)| t.len()).sum();
        assert_eq!(all, targets().len(), "every target in one column");
    }

    #[test]
    fn clips_are_added_changed_copied_and_removed_one_undo_step_each() {
        let mut e = tko();
        let made = e.add_clips((PRESETS[0].make)(1000.0, 3000.0, 4.0));
        assert_eq!(made.len(), 1);
        let id = made[0];
        assert!(e.set_clip(id, false, |c| c.amount = 0.2));
        assert_eq!(e.clip(id).unwrap().amount, 0.2);
        let copy = e.duplicate_clips(&[id])[0];
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
    fn blocks_never_share_a_row_when_added_pasted_or_moved() {
        let mut e = tko();
        let block = |row: u8, start: f64, end: f64| {
            let mut c = c(ZOOM, start, end, 0.1);
            c.lane = row;
            c
        };
        let rows = |e: &Editor, ids: &[u32]| -> Vec<u8> {
            ids.iter().map(|&k| e.clip(k).unwrap().lane).collect()
        };
        let a = e.add_clips(vec![block(0, 1000.0, 3000.0)])[0];
        // Over block a: the next row down.
        let b = e.add_clips_free(vec![block(0, 2000.0, 4000.0)], false)[0];
        assert_eq!(rows(&e, &[b]), [1]);
        // After it, end to end: the same row.
        let after = e.add_clips_free(vec![block(0, 3000.0, 3500.0)], false)[0];
        assert_eq!(rows(&e, &[after]), [0]);
        assert!(e.undo());

        // Pasted at 2500 with their spacing; rows 0 and 2 are taken there, so they go down as
        // one until both are free.
        let copied = vec![block(0, 0.0, 1000.0), block(2, 500.0, 1500.0)];
        let pasted = e.paste_clips(&copied, 2500.0);
        let at: Vec<(f64, f64)> = pasted
            .iter()
            .map(|&k| (e.clip(k).unwrap().start, e.clip(k).unwrap().end))
            .collect();
        assert_eq!(at, [(2500.0, 3500.0), (3000.0, 4000.0)]);
        assert_eq!(rows(&e, &pasted), [2, 4]);
        assert!(e.undo(), "a paste is one undo step");
        assert!(pasted.iter().all(|&k| e.clip(k).is_none()));
        assert!(e.paste_clips(&[], 0.0).is_empty());

        // Moved onto block a: the nearest free row there instead.
        let from = [(b, 2000.0, 4000.0, 1)];
        assert!(e.move_clips(&from, -1000.0, -1, false));
        let moved = e.clip(b).unwrap();
        assert_eq!((moved.start, moved.lane), (1000.0, 1));
        // An end grows only up to the next block on its row.
        let c = e.add_clips(vec![block(0, 5000.0, 6000.0)])[0];
        assert_eq!(e.clip_room(c), (3000.0, f64::INFINITY));
        // Back to where play starts: 2 s before the first note at 1000.
        assert_eq!(e.clip_room(a), (-1000.0, 5000.0));
    }

    #[test]
    fn blocks_can_start_before_the_song_as_early_as_play_does() {
        use crate::tko::gimmick::FieldProp;
        let mut e = tko();
        // The first note at 1000: play starts 2 s before it.
        assert_eq!(e.clip_floor(), -1000.0);
        let id = e.add_clips(vec![Clip::new(
            clips::Target::Field(FieldProp::Zoom),
            0.0,
            2000.0,
        )])[0];
        assert!(e.move_clips(&[(id, 0.0, 2000.0, 0)], -5000.0, 0, false));
        let c = e.clip(id).unwrap();
        assert_eq!(
            (c.start, c.end),
            (-1000.0, 1000.0),
            "no earlier than play starts"
        );
        // An audio lead-in starts play earlier still.
        e.chart.set_key("General", "AudioLeadIn", "3000");
        assert_eq!(e.clip_floor(), -3000.0);
        assert_eq!(e.clip_room(id).0, -3000.0);
        // A map whose first note is late starts at the song: nothing before 0.
        e.chart.set_key("General", "AudioLeadIn", "0");
        e.chart.objects.retain(|o| o.time >= 9000.0);
        assert_eq!(e.clip_floor(), 0.0);
        // The runtime keeps a block before 0 as it is.
        let mut kept = vec![Clip::new(
            clips::Target::Field(FieldProp::Zoom),
            -800.0,
            500.0,
        )];
        crate::tko::clips::sanitize(&mut kept);
        assert_eq!((kept[0].start, kept[0].end), (-800.0, 500.0));
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
