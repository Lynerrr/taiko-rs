//! What a `.tko` map adds to its chart: keyframed motion of the playfield, and of groups of notes.

use super::clips::Clip;
use super::graph::Graph;
use super::path::NotePath;
use super::track::{Key, Track};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_GROUPS: usize = 256;
/// Pieces the picture can be cut into.
pub const MAX_SLICES: usize = 12;
/// Notes across all groups: a file can't make the player hold an unbounded list.
pub const MAX_GROUPED_NOTES: usize = 200_000;
pub const MAX_NAME: usize = 64;
pub const MAX_MARKS: usize = 256;

/// A property of the whole playfield.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldProp {
    /// Virtual units right.
    X,
    /// Virtual units down.
    Y,
    /// Degrees clockwise, about the hit target.
    Rotation,
    /// Size multiplier about the hit target.
    Zoom,
    /// How fast notes scroll, as a multiple of the map's own (what is drawn, never when a note is
    /// hit).
    Scroll,
    /// Width and height multipliers: circles become ovals.
    StretchX,
    StretchY,
    /// Turned over left to right like a page, 0 as it is, 1 mirrored (the drum on the right).
    Flip,
    /// A white wash over everything, added to the picture (0 none, 1 white).
    Flash,
    /// The screen's edges darkened (0 none, 1 nearly black).
    Vignette,
    /// A colour laid over the playfield, and its hue in degrees.
    Wash,
    WashHue,
    /// The playfield torn into shifted slices (0 none, 1 wrecked).
    Glitch,
    /// Red and blue copies pushed apart, in virtual units.
    Chroma,
    /// How see-through the playfield is (0 solid, 1 gone; still there to hit).
    Fade,
    /// How see-through the lane alone is, the notes kept.
    LaneFade,
    /// Black over the whole screen, the playfield too (0 none, 1 black).
    Blackout,
    /// Black behind the playfield, over the background (0 none, 1 black).
    Backdrop,
    /// Fading copies of the playfield trailing behind it: how many, and what each step further
    /// back does to them (move, turn, size, how much of their opacity is left).
    Echo,
    EchoX,
    EchoY,
    EchoRot,
    EchoZoom,
    EchoFade,
    /// Rotated copies about the middle of the screen, like a kaleidoscope: how many in all.
    Kaleido,
    /// The picture cut into this many upright pieces across the screen (below 2: whole), each
    /// pushed up or down by `SliceY` (every other one the other way) and away from the middle by
    /// `SliceX`.
    Slices,
    SliceY,
    SliceX,
    /// How far along the lane the hit target sits, in virtual units: notes still meet it on
    /// time.
    Drum,
}

/// Where a property sits in the editor's lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropGroup {
    Move,
    Look,
    Clones,
}

impl PropGroup {
    pub const ALL: [PropGroup; 3] = [PropGroup::Move, PropGroup::Look, PropGroup::Clones];

    pub fn label(self) -> &'static str {
        match self {
            PropGroup::Move => "Move",
            PropGroup::Look => "Look",
            PropGroup::Clones => "Clones",
        }
    }
}

impl FieldProp {
    /// How much of its change from the default a calmer playfield keeps (Settings, Reduce map
    /// effects): flashes and glitches the least, camera moves half. What decides when notes are
    /// hit or where they meet the drum is never touched.
    pub fn calm(self) -> f64 {
        match self {
            FieldProp::Flash | FieldProp::Glitch | FieldProp::Chroma => 0.25,
            FieldProp::X
            | FieldProp::Y
            | FieldProp::Rotation
            | FieldProp::Zoom
            | FieldProp::StretchX
            | FieldProp::StretchY
            | FieldProp::Blackout
            | FieldProp::EchoRot
            | FieldProp::SliceY
            | FieldProp::SliceX => 0.5,
            _ => 1.0,
        }
    }

    pub const ALL: [FieldProp; 29] = [
        FieldProp::X,
        FieldProp::Y,
        FieldProp::Rotation,
        FieldProp::Zoom,
        FieldProp::Scroll,
        FieldProp::StretchX,
        FieldProp::StretchY,
        FieldProp::Flip,
        FieldProp::Flash,
        FieldProp::Vignette,
        FieldProp::Wash,
        FieldProp::WashHue,
        FieldProp::Glitch,
        FieldProp::Chroma,
        FieldProp::Fade,
        FieldProp::LaneFade,
        FieldProp::Blackout,
        FieldProp::Backdrop,
        FieldProp::Echo,
        FieldProp::EchoX,
        FieldProp::EchoY,
        FieldProp::EchoRot,
        FieldProp::EchoZoom,
        FieldProp::EchoFade,
        FieldProp::Kaleido,
        FieldProp::Slices,
        FieldProp::SliceY,
        FieldProp::SliceX,
        FieldProp::Drum,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FieldProp::X => "x",
            FieldProp::Y => "y",
            FieldProp::Rotation => "rotation",
            FieldProp::Zoom => "zoom",
            FieldProp::Scroll => "scroll",
            FieldProp::StretchX => "stretch_x",
            FieldProp::StretchY => "stretch_y",
            FieldProp::Flip => "flip",
            FieldProp::Flash => "flash",
            FieldProp::Vignette => "vignette",
            FieldProp::Wash => "wash",
            FieldProp::WashHue => "wash_hue",
            FieldProp::Glitch => "glitch",
            FieldProp::Chroma => "chroma",
            FieldProp::Fade => "fade",
            FieldProp::LaneFade => "lane_fade",
            FieldProp::Blackout => "blackout",
            FieldProp::Backdrop => "backdrop",
            FieldProp::Echo => "echo",
            FieldProp::EchoX => "echo_x",
            FieldProp::EchoY => "echo_y",
            FieldProp::EchoRot => "echo_rot",
            FieldProp::EchoZoom => "echo_zoom",
            FieldProp::EchoFade => "echo_fade",
            FieldProp::Kaleido => "kaleido",
            FieldProp::Slices => "slices",
            FieldProp::SliceY => "slice_y",
            FieldProp::SliceX => "slice_x",
            FieldProp::Drum => "drum",
        }
    }

    pub fn group(self) -> PropGroup {
        match self {
            FieldProp::X
            | FieldProp::Y
            | FieldProp::Rotation
            | FieldProp::Zoom
            | FieldProp::Scroll
            | FieldProp::StretchX
            | FieldProp::StretchY
            | FieldProp::Flip
            | FieldProp::Drum => PropGroup::Move,
            FieldProp::Flash
            | FieldProp::Vignette
            | FieldProp::Wash
            | FieldProp::WashHue
            | FieldProp::Glitch
            | FieldProp::Chroma
            | FieldProp::Fade
            | FieldProp::LaneFade
            | FieldProp::Blackout
            | FieldProp::Backdrop => PropGroup::Look,
            _ => PropGroup::Clones,
        }
    }

    /// One line on what it does, for tooltips.
    pub fn about(self) -> &'static str {
        match self {
            FieldProp::X => "Slides the whole playfield right (left below 0), in virtual units.",
            FieldProp::Y => "Slides the whole playfield down (up below 0), in virtual units.",
            FieldProp::Rotation => {
                "Turns the playfield clockwise about the hit target, in degrees."
            }
            FieldProp::Zoom => "Scales the playfield about the hit target. 1 is normal.",
            FieldProp::Scroll => {
                "How fast notes scroll, as a multiple of the map's. Draws only: hit times never move."
            }
            FieldProp::StretchX => "Makes the playfield wider (above 1) or thinner (below 1).",
            FieldProp::StretchY => "Makes the playfield taller (above 1) or flatter (below 1).",
            FieldProp::Flip => {
                "Turns the playfield over like a page: 0 as it is, 0.5 edge on, 1 mirrored with the drum on the right."
            }
            FieldProp::Flash => "A white flash added over everything. 0 none, 1 white-out.",
            FieldProp::Vignette => "Darkens the edges of the screen. 0 none, 1 nearly black.",
            FieldProp::Wash => "How strongly a colour is laid over the playfield.",
            FieldProp::WashHue => {
                "The wash's colour as a hue in degrees (0 red, 120 green, 240 blue)."
            }
            FieldProp::Glitch => {
                "Tears the playfield into shifted slices that flicker. 0 none, 1 wrecked."
            }
            FieldProp::Chroma => {
                "Pushes a red and a blue copy apart from the picture, in virtual units."
            }
            FieldProp::Fade => {
                "How see-through the whole playfield is: 0 solid, 1 gone. The notes are still there to hit."
            }
            FieldProp::LaneFade => {
                "How see-through the lane is, the notes and drum kept: 0 solid, 1 gone."
            }
            FieldProp::Blackout => {
                "Black over the whole screen, the playfield too: 0 none, 1 black. Text over the lane and flashes still show."
            }
            FieldProp::Backdrop => {
                "Black behind the playfield, over the background: 0 none, 1 only the playfield on black."
            }
            FieldProp::Echo => "How many fading copies trail behind the playfield (up to 8).",
            FieldProp::EchoX => "How far each step back in the trail moves sideways.",
            FieldProp::EchoY => "How far each step back in the trail moves down.",
            FieldProp::EchoRot => "How many degrees each step back in the trail turns.",
            FieldProp::EchoZoom => {
                "How much each step back in the trail scales (1 keeps the size)."
            }
            FieldProp::EchoFade => {
                "How much of its opacity each step back keeps (0.6: each is 60% of the last)."
            }
            FieldProp::Kaleido => {
                "Rotated copies about the middle of the screen: how many in all (2 to 8)."
            }
            FieldProp::Slices => {
                "Cuts the picture into this many upright pieces across the screen (2 to 12; below 2 it stays whole)."
            }
            FieldProp::SliceY => {
                "How far the pieces are pushed up and down, every other one the other way, in virtual units."
            }
            FieldProp::SliceX => {
                "How far the pieces spread out from the middle (together below 0), in virtual units."
            }
            FieldProp::Drum => {
                "Slides the hit target along the lane, in virtual units. Notes still meet it on time."
            }
        }
    }

    pub fn default_value(self) -> f64 {
        match self {
            FieldProp::X
            | FieldProp::Y
            | FieldProp::Rotation
            | FieldProp::Flash
            | FieldProp::Vignette
            | FieldProp::Wash
            | FieldProp::WashHue
            | FieldProp::Glitch
            | FieldProp::Chroma
            | FieldProp::Fade
            | FieldProp::LaneFade
            | FieldProp::Blackout
            | FieldProp::Backdrop
            | FieldProp::Echo
            | FieldProp::EchoY
            | FieldProp::EchoRot
            | FieldProp::Flip
            | FieldProp::Kaleido
            | FieldProp::Slices
            | FieldProp::SliceY
            | FieldProp::SliceX
            | FieldProp::Drum => 0.0,
            FieldProp::Zoom
            | FieldProp::Scroll
            | FieldProp::StretchX
            | FieldProp::StretchY
            | FieldProp::EchoZoom => 1.0,
            FieldProp::EchoX => -30.0,
            FieldProp::EchoFade => 0.6,
        }
    }

    /// What a key may hold.
    pub fn range(self) -> (f64, f64) {
        match self {
            FieldProp::X | FieldProp::Y => (-2000.0, 2000.0),
            FieldProp::Rotation | FieldProp::WashHue => (-3600.0, 3600.0),
            FieldProp::Zoom => (0.05, 8.0),
            FieldProp::Scroll => (0.1, 8.0),
            FieldProp::StretchX | FieldProp::StretchY => (0.1, 4.0),
            FieldProp::Flash
            | FieldProp::Vignette
            | FieldProp::Wash
            | FieldProp::Glitch
            | FieldProp::Flip
            | FieldProp::Fade
            | FieldProp::LaneFade
            | FieldProp::Blackout
            | FieldProp::Backdrop
            | FieldProp::EchoFade => (0.0, 1.0),
            FieldProp::Chroma => (0.0, 80.0),
            FieldProp::Echo | FieldProp::Kaleido => (0.0, 8.0),
            FieldProp::EchoX | FieldProp::EchoY | FieldProp::SliceY | FieldProp::SliceX => {
                (-400.0, 400.0)
            }
            FieldProp::Slices => (0.0, MAX_SLICES as f64),
            FieldProp::EchoRot => (-90.0, 90.0),
            FieldProp::EchoZoom => (0.5, 1.5),
            FieldProp::Drum => (0.0, 600.0),
        }
    }
}

/// A property of one group of notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteProp {
    Dx,
    Dy,
    Scale,
    Opacity,
    /// 1 hides the note (it is still there to hit).
    Hidden,
    /// Degrees the note is turned about the drum.
    Swirl,
    /// Width and height multipliers of each note: circles become ovals.
    StretchX,
    StretchY,
    /// How far the notes' colour goes to `TintHue` (0 their own, 1 all of it), and that colour as
    /// a hue in degrees.
    Tint,
    TintHue,
}

impl NoteProp {
    pub const ALL: [NoteProp; 10] = [
        NoteProp::Dx,
        NoteProp::Dy,
        NoteProp::Scale,
        NoteProp::Opacity,
        NoteProp::Hidden,
        NoteProp::Swirl,
        NoteProp::StretchX,
        NoteProp::StretchY,
        NoteProp::Tint,
        NoteProp::TintHue,
    ];

    pub fn name(self) -> &'static str {
        match self {
            NoteProp::Dx => "dx",
            NoteProp::Dy => "dy",
            NoteProp::Scale => "scale",
            NoteProp::Opacity => "opacity",
            NoteProp::Hidden => "hidden",
            NoteProp::Swirl => "swirl",
            NoteProp::StretchX => "stretch_x",
            NoteProp::StretchY => "stretch_y",
            NoteProp::Tint => "tint",
            NoteProp::TintHue => "tint_hue",
        }
    }

    /// One line on what it does, for tooltips.
    pub fn about(self) -> &'static str {
        match self {
            NoteProp::Dx => "Slides the notes of the group sideways on screen.",
            NoteProp::Dy => "Slides the notes of the group up or down on screen.",
            NoteProp::Scale => "Makes the notes bigger (above 1) or smaller.",
            NoteProp::Opacity => "How see-through the notes are. 0 is invisible, 1 solid.",
            NoteProp::Hidden => "1 hides the notes completely. They are still there to hit.",
            NoteProp::Swirl => {
                "Turns each note about the drum, in degrees: the further out, the wider the swing."
            }
            NoteProp::StretchX => "Makes each note wider (above 1) or thinner (below 1).",
            NoteProp::StretchY => "Makes each note taller (above 1) or flatter (below 1).",
            NoteProp::Tint => {
                "How far the notes turn to the tint colour: 0 their own colour, 1 all of it."
            }
            NoteProp::TintHue => {
                "The tint colour as a hue in degrees (0 red, 60 yellow, 120 green, 240 blue)."
            }
        }
    }

    pub fn default_value(self) -> f64 {
        match self {
            NoteProp::Dx
            | NoteProp::Dy
            | NoteProp::Hidden
            | NoteProp::Swirl
            | NoteProp::Tint
            | NoteProp::TintHue => 0.0,
            NoteProp::Scale | NoteProp::Opacity | NoteProp::StretchX | NoteProp::StretchY => 1.0,
        }
    }

    /// A value at the song's time `song` with the before-the-hit value `near` on top: moves,
    /// turns and tint add, sizes and opacity multiply, either one hides, and a hue replaces.
    pub fn before(self, song: f64, near: f64) -> f64 {
        let v = match self {
            NoteProp::Dx | NoteProp::Dy | NoteProp::Swirl | NoteProp::Tint => song + near,
            NoteProp::Scale | NoteProp::Opacity | NoteProp::StretchX | NoteProp::StretchY => {
                song * near
            }
            NoteProp::Hidden => song.max(near),
            NoteProp::TintHue => near,
        };
        let (lo, hi) = self.range();
        v.clamp(lo, hi)
    }

    pub fn range(self) -> (f64, f64) {
        match self {
            NoteProp::Dx | NoteProp::Dy => (-1000.0, 1000.0),
            NoteProp::Scale => (0.05, 6.0),
            NoteProp::Opacity | NoteProp::Hidden | NoteProp::Tint => (0.0, 1.0),
            NoteProp::Swirl => (-720.0, 720.0),
            NoteProp::StretchX | NoteProp::StretchY => (0.1, 4.0),
            NoteProp::TintHue => (-3600.0, 3600.0),
        }
    }
}

/// Notes picked in the editor, with the motion they share. Notes are held by their start time
/// (what a note is in an `.osu`), so the group survives edits that reorder the chart.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Group {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub notes: Vec<f64>,
    #[serde(default)]
    pub tracks: BTreeMap<String, Track>,
    /// The way its notes fly in instead of along the lane (see `path`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<NotePath>,
    /// What each note does as its own hit nears: keyed by beats before that note's hit (0 the
    /// hit, negative after it), on top of `tracks` (see [`NoteProp::before`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub before: BTreeMap<String, Track>,
    /// What its dons and kats look like until each one's truth shows (see [`ShowAs`]).
    #[serde(default, skip_serializing_if = "ShowAs::is_true")]
    pub show: ShowAs,
}

/// A lie a group's notes tell: drawn in another colour or size until `reveal` beats before
/// each one's hit, then turning into what they are over [`REVEAL_BEATS`]. Only the look: what
/// hits them, the score and the sound stay the note's own. Rolls and swells don't lie.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShowAs {
    #[serde(default, skip_serializing_if = "ShowColour::is_same")]
    pub colour: ShowColour,
    #[serde(default, skip_serializing_if = "ShowSize::is_same")]
    pub size: ShowSize,
    /// Beats before each note's hit its truth shows (0: at the hit).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reveal: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// How long a lying note takes to turn into what it is, in beats.
pub const REVEAL_BEATS: f64 = 0.25;

impl ShowAs {
    /// Tells no lie.
    pub fn is_true(&self) -> bool {
        self.colour.is_same() && self.size.is_same()
    }

    /// What a note (`kat`, `big`) is drawn as, as a lie: the colour (kat) and the size (big),
    /// each only if it lies about it.
    pub fn lie(&self, kat: bool, big: bool) -> (Option<bool>, Option<bool>) {
        let colour = match self.colour {
            ShowColour::Same | ShowColour::Unknown => None,
            ShowColour::Don => Some(false),
            ShowColour::Kat => Some(true),
            ShowColour::Swap => Some(!kat),
        };
        let size = match self.size {
            ShowSize::Same | ShowSize::Unknown => None,
            ShowSize::Small => Some(false),
            ShowSize::Big => Some(true),
            ShowSize::Swap => Some(!big),
        };
        (colour.filter(|&c| c != kat), size.filter(|&b| b != big))
    }

    /// How far the truth shows `near` beats before the hit: 0 the lie, 1 the truth.
    pub fn truth(&self, near: f64) -> f32 {
        ((self.reveal + REVEAL_BEATS - near) / REVEAL_BEATS).clamp(0.0, 1.0) as f32
    }
}

/// The colour a group's notes show as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowColour {
    /// Their own.
    #[default]
    Same,
    Don,
    Kat,
    /// Dons as kats, kats as dons.
    Swap,
    /// What a newer map says that this game doesn't know: their own (`sanitize` makes it `Same`).
    #[serde(other)]
    Unknown,
}

impl ShowColour {
    pub const ALL: [ShowColour; 4] = [Self::Same, Self::Don, Self::Kat, Self::Swap];

    fn is_same(&self) -> bool {
        matches!(self, Self::Same | Self::Unknown)
    }
}

/// The size a group's notes show as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowSize {
    #[default]
    Same,
    Small,
    /// As finishers.
    Big,
    /// Finishers small, small ones as finishers.
    Swap,
    /// Unknown to this game: their own (`sanitize` makes it `Same`).
    #[serde(other)]
    Unknown,
}

impl ShowSize {
    pub const ALL: [ShowSize; 4] = [Self::Same, Self::Small, Self::Big, Self::Swap];

    fn is_same(&self) -> bool {
        matches!(self, Self::Same | Self::Unknown)
    }
}

impl Group {
    pub fn track(&self, p: NoteProp) -> Option<&Track> {
        self.tracks.get(p.name())
    }

    pub fn track_mut(&mut self, p: NoteProp) -> &mut Track {
        self.tracks.entry(p.name().to_string()).or_default()
    }

    pub fn before_track(&self, p: NoteProp) -> Option<&Track> {
        self.before.get(p.name())
    }
}

/// How far from its hit a before-the-hit key may be, in beats (before, and after).
pub const BEFORE_BEATS: (f64, f64) = (32.0, 4.0);

/// A moment in the map the author named: where a section starts, a drop, a break. Graph nodes
/// refer to marks instead of typed times, and the editor shows them on the timeline.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    #[serde(default)]
    pub name: String,
    /// Chart time, ms.
    pub time: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gimmick {
    #[serde(default)]
    pub field: BTreeMap<String, Track>,
    #[serde(default)]
    pub groups: Vec<Group>,
    /// Named moments, ascending by time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    /// Events, logic and actions that move the same things as the keyframes (see `graph`).
    #[serde(default, skip_serializing_if = "Graph::is_empty")]
    pub graph: Graph,
    /// Effect blocks on the timeline (see `clips`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<Clip>,
    /// The shared scroll (mania-style SV): the whole lane's speed over the song, on top of the
    /// map's own slider velocity (see `scroll`). No keys: the song's own.
    #[serde(default, skip_serializing_if = "Track::is_empty")]
    pub scroll: Track,
    /// Notes that are only drawn: never hit, never judged, silent, gone when they reach the drum.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fakes: Vec<FakeNote>,
    /// Notes (chart ms, ascending) drawn over every effect: a blackout, a flash or a glitch
    /// doesn't hide them, and effects on all the notes leave them be. They still move with the
    /// playfield, and with effects on a group they are in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub above: Vec<f64>,
}

/// How many fake notes a map may have.
pub const MAX_FAKES: usize = 20_000;

/// A note that is only drawn (see `Gimmick::fakes`). Groups take it by its time like a real one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FakeNote {
    /// Chart ms.
    pub t: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub kat: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub big: bool,
    /// The motion group it flies with, named by place instead of found by its time: 0 none,
    /// n the n-th group. Unset (older maps, and a fake never given one), the first group that
    /// holds its time, as for a note. So fakes stacked on one millisecond, or on a note, can
    /// each have a path and moves of their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u16>,
}

impl Gimmick {
    pub fn is_empty(&self) -> bool {
        self.field.values().all(Track::is_empty)
            && self.groups.iter().all(|g| {
                g.tracks.values().all(Track::is_empty)
                    && g.before.values().all(Track::is_empty)
                    && g.path.is_none()
                    && g.show.is_true()
            })
            && self.graph.is_empty()
            && self.marks.is_empty()
            && self.clips.is_empty()
            && self.scroll.is_empty()
            && self.fakes.is_empty()
            && self.above.is_empty()
    }

    /// Whether the note at chart time `t` is drawn over every effect (see `above`).
    pub fn is_above(&self, t: f64) -> bool {
        let at = self.above.partition_point(|&n| n < t - 0.5);
        self.above.get(at).is_some_and(|&n| (n - t).abs() <= 0.5)
    }

    pub fn field_track(&self, p: FieldProp) -> Option<&Track> {
        self.field.get(p.name())
    }

    pub fn field_track_mut(&mut self, p: FieldProp) -> &mut Track {
        self.field.entry(p.name().to_string()).or_default()
    }

    /// Adds a key to a field property, held inside what the property allows.
    pub fn key_field(&mut self, p: FieldProp, key: Key) {
        let (lo, hi) = p.range();
        let key = Key {
            v: key.v.clamp(lo, hi),
            ..key
        };
        self.field_track_mut(p).set(key);
    }

    pub fn key_note(&mut self, group: usize, p: NoteProp, key: Key) {
        let (lo, hi) = p.range();
        let key = Key {
            v: key.v.clamp(lo, hi),
            ..key
        };
        if let Some(g) = self.groups.get_mut(group) {
            g.track_mut(p).set(key);
        }
    }

    /// Makes what a file said safe to run: every track sorted and finite and within its caps,
    /// values held in range, unknown properties dropped, empty tracks removed.
    pub fn sanitize(&mut self) {
        self.groups.truncate(MAX_GROUPS);
        let mut budget = MAX_GROUPED_NOTES;
        for g in &mut self.groups {
            g.name.truncate(
                g.name
                    .char_indices()
                    .nth(MAX_NAME)
                    .map_or(g.name.len(), |(i, _)| i),
            );
            g.notes.retain(|t| t.is_finite());
            g.notes.sort_by(f64::total_cmp);
            g.notes.dedup();
            g.notes.truncate(budget);
            budget -= g.notes.len();
            g.tracks
                .retain(|k, _| NoteProp::ALL.iter().any(|p| p.name() == k));
            for (name, t) in g.tracks.iter_mut() {
                t.sanitize();
                if let Some(p) = NoteProp::ALL.iter().find(|p| p.name() == name) {
                    let (lo, hi) = p.range();
                    t.keys.iter_mut().for_each(|k| k.v = k.v.clamp(lo, hi));
                }
            }
            g.tracks.retain(|_, t| !t.is_empty());
            g.before
                .retain(|k, _| NoteProp::ALL.iter().any(|p| p.name() == k));
            for (name, t) in g.before.iter_mut() {
                t.sanitize();
                let (far, after) = BEFORE_BEATS;
                t.keys.retain(|k| (-after..=far).contains(&k.t));
                if let Some(p) = NoteProp::ALL.iter().find(|p| p.name() == name) {
                    let (lo, hi) = p.range();
                    t.keys.iter_mut().for_each(|k| k.v = k.v.clamp(lo, hi));
                }
            }
            g.before.retain(|_, t| !t.is_empty());
            if let Some(p) = &mut g.path {
                p.sanitize();
            }
            if g.show.colour == ShowColour::Unknown {
                g.show.colour = ShowColour::Same;
            }
            if g.show.size == ShowSize::Unknown {
                g.show.size = ShowSize::Same;
            }
            g.show.reveal = if g.show.reveal.is_finite() {
                g.show.reveal.clamp(0.0, BEFORE_BEATS.0)
            } else {
                0.0
            };
        }
        self.field
            .retain(|k, _| FieldProp::ALL.iter().any(|p| p.name() == k));
        for (name, t) in self.field.iter_mut() {
            t.sanitize();
            if let Some(p) = FieldProp::ALL.iter().find(|p| p.name() == name) {
                let (lo, hi) = p.range();
                t.keys.iter_mut().for_each(|k| k.v = k.v.clamp(lo, hi));
            }
        }
        self.field.retain(|_, t| !t.is_empty());
        self.above.retain(|t| t.is_finite());
        self.above.sort_by(f64::total_cmp);
        self.above.dedup();
        self.above.truncate(MAX_GROUPED_NOTES);
        self.marks.retain(|m| m.time.is_finite());
        self.marks.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.marks.truncate(MAX_MARKS);
        for m in &mut self.marks {
            m.name.truncate(
                m.name
                    .char_indices()
                    .nth(MAX_NAME)
                    .map_or(m.name.len(), |(i, _)| i),
            );
        }
        let groups = self.groups.len();
        self.graph.sanitize(groups);
        super::clips::sanitize(&mut self.clips);
        super::scroll::sanitize(&mut self.scroll);
        self.fakes.retain(|f| f.t.is_finite());
        // No dedup: any number of fakes may be stacked on one millisecond on purpose.
        self.fakes
            .sort_by(|a, b| a.t.total_cmp(&b.t).then(a.kat.cmp(&b.kat)));
        self.fakes.truncate(MAX_FAKES);
        // A clip on a group of notes that isn't there moves nothing (a copy's clip holds which
        // copy in the same place).
        self.clips
            .retain(|c| c.what.starts_with("copy.") || (c.group as usize) <= groups);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lie_is_only_told_where_it_changes_something() {
        let swap = ShowAs {
            colour: ShowColour::Swap,
            size: ShowSize::Big,
            reveal: 1.0,
        };
        assert_eq!(swap.lie(false, false), (Some(true), Some(true)));
        // A finisher shown big tells no lie about its size.
        assert_eq!(swap.lie(true, true), (Some(false), None));
        let dons = ShowAs {
            colour: ShowColour::Don,
            ..ShowAs::default()
        };
        assert_eq!(dons.lie(false, true), (None, None));
        assert!(ShowAs::default().is_true());
        // The truth shows over a 1/4 beat, ending `reveal` beats before the hit.
        assert_eq!(swap.truth(2.0), 0.0);
        assert_eq!(swap.truth(1.25), 0.0);
        assert!((swap.truth(1.125) - 0.5).abs() < 1e-6);
        assert_eq!(swap.truth(1.0), 1.0);
        assert_eq!(swap.truth(-1.0), 1.0);
    }

    #[test]
    fn an_unknown_lie_reads_as_none_and_a_wild_reveal_is_held() {
        let json =
            r#"{"groups":[{"notes":[1],"show":{"colour":"rainbow","size":"small","reveal":1e9}}]}"#;
        let mut g: Gimmick = serde_json::from_str(json).unwrap();
        g.sanitize();
        let show = g.groups[0].show;
        assert_eq!(show.colour, ShowColour::Same);
        assert_eq!(show.size, ShowSize::Small);
        assert_eq!(show.reveal, BEFORE_BEATS.0);
        let back = serde_json::to_string(&g.groups[0]).unwrap();
        assert!(
            back.contains(r#""show":{"size":"small","reveal":32.0}"#),
            "{back}"
        );
        g.groups[0].show = ShowAs::default();
        assert!(
            !serde_json::to_string(&g.groups[0])
                .unwrap()
                .contains("show")
        );
    }
    use crate::tko::track::Ease;

    fn key(t: f64, v: f64) -> Key {
        Key {
            t,
            v,
            ease: Ease::Linear,
        }
    }

    #[test]
    fn keys_are_held_inside_what_a_property_allows() {
        let mut g = Gimmick::default();
        g.key_field(FieldProp::Zoom, key(0.0, 100.0));
        g.key_field(FieldProp::Zoom, key(1000.0, -3.0));
        let z = g.field_track(FieldProp::Zoom).unwrap();
        assert_eq!((z.keys[0].v, z.keys[1].v), (8.0, 0.05));
        assert!(!g.is_empty());
    }

    #[test]
    fn sanitizing_drops_what_a_file_should_not_carry() {
        let mut g = Gimmick::default();
        g.groups.push(Group {
            name: "x".repeat(500),
            notes: vec![300.0, f64::NAN, 100.0, 100.0],
            tracks: BTreeMap::from([
                (
                    "dx".to_string(),
                    Track {
                        keys: vec![key(0.0, 99999.0)],
                    },
                ),
                (
                    "evil".to_string(),
                    Track {
                        keys: vec![key(0.0, 1.0)],
                    },
                ),
                ("dy".to_string(), Track::default()),
            ]),
            ..Group::default()
        });
        g.field.insert(
            "unknown".into(),
            Track {
                keys: vec![key(0.0, 1.0)],
            },
        );
        g.sanitize();
        assert_eq!(g.groups[0].name.chars().count(), MAX_NAME);
        assert_eq!(g.groups[0].notes, [100.0, 300.0]);
        assert_eq!(g.groups[0].tracks.len(), 1);
        assert_eq!(g.groups[0].track(NoteProp::Dx).unwrap().keys[0].v, 1000.0);
        assert!(g.field.is_empty());
    }

    #[test]
    fn the_caps_hold() {
        let mut g = Gimmick::default();
        for _ in 0..MAX_GROUPS + 10 {
            g.groups.push(Group::default());
        }
        g.sanitize();
        assert_eq!(g.groups.len(), MAX_GROUPS);
    }

    #[test]
    fn a_gimmick_round_trips_through_json_and_tolerates_missing_parts() {
        let mut g = Gimmick::default();
        g.key_field(FieldProp::Rotation, key(1000.0, 90.0));
        g.groups.push(Group {
            name: "a".into(),
            notes: vec![1000.0, 2000.0],
            ..Group::default()
        });
        g.key_note(0, NoteProp::Opacity, key(0.0, 0.5));
        let text = serde_json::to_string(&g).unwrap();
        assert_eq!(serde_json::from_str::<Gimmick>(&text).unwrap(), g);
        assert_eq!(
            serde_json::from_str::<Gimmick>("{}").unwrap(),
            Gimmick::default()
        );
    }
}
