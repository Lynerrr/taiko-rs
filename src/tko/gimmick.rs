//! What a `.tko` map adds to its chart: keyframed motion of the playfield, and of groups of notes.

use super::clips::Clip;
use super::graph::Graph;
use super::track::{Key, Track};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_GROUPS: usize = 256;
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
    pub const ALL: [FieldProp; 20] = [
        FieldProp::X,
        FieldProp::Y,
        FieldProp::Rotation,
        FieldProp::Zoom,
        FieldProp::Scroll,
        FieldProp::StretchX,
        FieldProp::StretchY,
        FieldProp::Flash,
        FieldProp::Vignette,
        FieldProp::Wash,
        FieldProp::WashHue,
        FieldProp::Glitch,
        FieldProp::Chroma,
        FieldProp::Echo,
        FieldProp::EchoX,
        FieldProp::EchoY,
        FieldProp::EchoRot,
        FieldProp::EchoZoom,
        FieldProp::EchoFade,
        FieldProp::Kaleido,
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
            FieldProp::Flash => "flash",
            FieldProp::Vignette => "vignette",
            FieldProp::Wash => "wash",
            FieldProp::WashHue => "wash_hue",
            FieldProp::Glitch => "glitch",
            FieldProp::Chroma => "chroma",
            FieldProp::Echo => "echo",
            FieldProp::EchoX => "echo_x",
            FieldProp::EchoY => "echo_y",
            FieldProp::EchoRot => "echo_rot",
            FieldProp::EchoZoom => "echo_zoom",
            FieldProp::EchoFade => "echo_fade",
            FieldProp::Kaleido => "kaleido",
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
            | FieldProp::StretchY => PropGroup::Move,
            FieldProp::Flash
            | FieldProp::Vignette
            | FieldProp::Wash
            | FieldProp::WashHue
            | FieldProp::Glitch
            | FieldProp::Chroma => PropGroup::Look,
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
            | FieldProp::Echo
            | FieldProp::EchoY
            | FieldProp::EchoRot
            | FieldProp::Kaleido => 0.0,
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
            | FieldProp::EchoFade => (0.0, 1.0),
            FieldProp::Chroma => (0.0, 80.0),
            FieldProp::Echo | FieldProp::Kaleido => (0.0, 8.0),
            FieldProp::EchoX | FieldProp::EchoY => (-400.0, 400.0),
            FieldProp::EchoRot => (-90.0, 90.0),
            FieldProp::EchoZoom => (0.5, 1.5),
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
}

impl NoteProp {
    pub const ALL: [NoteProp; 6] = [
        NoteProp::Dx,
        NoteProp::Dy,
        NoteProp::Scale,
        NoteProp::Opacity,
        NoteProp::Hidden,
        NoteProp::Swirl,
    ];

    pub fn name(self) -> &'static str {
        match self {
            NoteProp::Dx => "dx",
            NoteProp::Dy => "dy",
            NoteProp::Scale => "scale",
            NoteProp::Opacity => "opacity",
            NoteProp::Hidden => "hidden",
            NoteProp::Swirl => "swirl",
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
        }
    }

    pub fn default_value(self) -> f64 {
        match self {
            NoteProp::Dx | NoteProp::Dy | NoteProp::Hidden | NoteProp::Swirl => 0.0,
            NoteProp::Scale | NoteProp::Opacity => 1.0,
        }
    }

    pub fn range(self) -> (f64, f64) {
        match self {
            NoteProp::Dx | NoteProp::Dy => (-1000.0, 1000.0),
            NoteProp::Scale => (0.05, 6.0),
            NoteProp::Opacity | NoteProp::Hidden => (0.0, 1.0),
            NoteProp::Swirl => (-720.0, 720.0),
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
}

impl Group {
    pub fn track(&self, p: NoteProp) -> Option<&Track> {
        self.tracks.get(p.name())
    }

    pub fn track_mut(&mut self, p: NoteProp) -> &mut Track {
        self.tracks.entry(p.name().to_string()).or_default()
    }
}

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
}

impl Gimmick {
    pub fn is_empty(&self) -> bool {
        self.field.values().all(Track::is_empty)
            && self
                .groups
                .iter()
                .all(|g| g.tracks.values().all(Track::is_empty))
            && self.graph.is_empty()
            && self.marks.is_empty()
            && self.clips.is_empty()
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
        // A clip on a group that isn't there moves nothing.
        self.clips.retain(|c| (c.group as usize) <= groups);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
