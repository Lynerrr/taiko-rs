//! What a `.tko` map adds to its chart: keyframed motion of the playfield, and of groups of notes.

use super::track::{Key, Track};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_GROUPS: usize = 256;
/// Notes across all groups: a file can't make the player hold an unbounded list.
pub const MAX_GROUPED_NOTES: usize = 200_000;
pub const MAX_NAME: usize = 64;

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
}

impl FieldProp {
    pub const ALL: [FieldProp; 5] = [
        FieldProp::X,
        FieldProp::Y,
        FieldProp::Rotation,
        FieldProp::Zoom,
        FieldProp::Scroll,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FieldProp::X => "x",
            FieldProp::Y => "y",
            FieldProp::Rotation => "rotation",
            FieldProp::Zoom => "zoom",
            FieldProp::Scroll => "scroll",
        }
    }

    pub fn default_value(self) -> f64 {
        match self {
            FieldProp::X | FieldProp::Y | FieldProp::Rotation => 0.0,
            FieldProp::Zoom | FieldProp::Scroll => 1.0,
        }
    }

    /// What a key may hold.
    pub fn range(self) -> (f64, f64) {
        match self {
            FieldProp::X | FieldProp::Y => (-2000.0, 2000.0),
            FieldProp::Rotation => (-3600.0, 3600.0),
            FieldProp::Zoom => (0.05, 8.0),
            FieldProp::Scroll => (0.1, 8.0),
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
}

impl NoteProp {
    pub const ALL: [NoteProp; 5] = [
        NoteProp::Dx,
        NoteProp::Dy,
        NoteProp::Scale,
        NoteProp::Opacity,
        NoteProp::Hidden,
    ];

    pub fn name(self) -> &'static str {
        match self {
            NoteProp::Dx => "dx",
            NoteProp::Dy => "dy",
            NoteProp::Scale => "scale",
            NoteProp::Opacity => "opacity",
            NoteProp::Hidden => "hidden",
        }
    }

    pub fn default_value(self) -> f64 {
        match self {
            NoteProp::Dx | NoteProp::Dy | NoteProp::Hidden => 0.0,
            NoteProp::Scale | NoteProp::Opacity => 1.0,
        }
    }

    pub fn range(self) -> (f64, f64) {
        match self {
            NoteProp::Dx | NoteProp::Dy => (-1000.0, 1000.0),
            NoteProp::Scale => (0.05, 6.0),
            NoteProp::Opacity | NoteProp::Hidden => (0.0, 1.0),
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gimmick {
    #[serde(default)]
    pub field: BTreeMap<String, Track>,
    #[serde(default)]
    pub groups: Vec<Group>,
    /// The node graph (see `tko::graph`), kept as the file has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<serde_json::Value>,
}

impl Gimmick {
    pub fn is_empty(&self) -> bool {
        self.field.values().all(Track::is_empty)
            && self
                .groups
                .iter()
                .all(|g| g.tracks.values().all(Track::is_empty))
            && self.graph.is_none()
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
