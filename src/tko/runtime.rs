//! A gimmick at play time: what the playfield and each note are doing at a moment.
//!
//! Pure functions of map time and the file (and, from stage 4, the play's own events), so a replay
//! looks the same every time. Nothing here is read by judging.

use std::sync::Arc;

use super::gimmick::{FieldProp, Gimmick, NoteProp};

/// What the whole playfield is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldFx {
    pub x: f32,
    pub y: f32,
    /// Degrees clockwise.
    pub rotation: f32,
    pub zoom: f32,
    pub scroll: f64,
}

impl Default for FieldFx {
    fn default() -> Self {
        FieldFx {
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            zoom: 1.0,
            scroll: 1.0,
        }
    }
}

/// What one note is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteFx {
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    pub opacity: f32,
    pub hidden: bool,
}

impl Default for NoteFx {
    fn default() -> Self {
        NoteFx {
            dx: 0.0,
            dy: 0.0,
            scale: 1.0,
            opacity: 1.0,
            hidden: false,
        }
    }
}

/// A gimmick bound to the notes of one play.
#[derive(Clone, Debug)]
pub struct Runtime {
    gimmick: Arc<Gimmick>,
    /// Per note (in the map's order): the group it is in, plus one; 0 for none.
    group_of: Vec<u16>,
    /// How far the game's clock is ahead of the chart's (the game shifts every time of a loaded
    /// map; a gimmick's times are the chart's).
    shift: f64,
}

/// How far a note's time may be from the one a group holds (the chart's whole milliseconds).
const SAME_NOTE_MS: f64 = 0.5;

impl Runtime {
    /// `note_times`: the map's notes, in order, on the game's clock, which is `shift` ms ahead of
    /// the chart's.
    pub fn new(
        gimmick: Arc<Gimmick>,
        note_times: impl Iterator<Item = f64>,
        shift: f64,
    ) -> Runtime {
        let group_of = note_times
            .map(|t| t - shift)
            .map(|t| {
                gimmick
                    .groups
                    .iter()
                    .position(|g| {
                        let at = g.notes.partition_point(|&n| n < t - SAME_NOTE_MS);
                        g.notes
                            .get(at)
                            .is_some_and(|&n| (n - t).abs() <= SAME_NOTE_MS)
                    })
                    .map_or(0, |i| i as u16 + 1)
            })
            .collect();
        Runtime {
            gimmick,
            group_of,
            shift,
        }
    }

    pub fn field(&self, t: f64) -> FieldFx {
        let t = t - self.shift;
        let v = |p: FieldProp| {
            self.gimmick
                .field_track(p)
                .map_or(p.default_value(), |tr| tr.value(t, p.default_value()))
        };
        FieldFx {
            x: v(FieldProp::X) as f32,
            y: v(FieldProp::Y) as f32,
            rotation: v(FieldProp::Rotation) as f32,
            zoom: v(FieldProp::Zoom) as f32,
            scroll: v(FieldProp::Scroll),
        }
    }

    /// The group of note `i`, if any.
    pub fn group_of(&self, i: usize) -> Option<usize> {
        match self.group_of.get(i) {
            Some(&g) if g > 0 => Some(g as usize - 1),
            _ => None,
        }
    }

    /// What note `i` is doing at map time `t`; nothing for a note in no group.
    pub fn note(&self, i: usize, t: f64) -> Option<NoteFx> {
        let g = &self.gimmick.groups[self.group_of(i)?];
        let t = t - self.shift;
        let v = |p: NoteProp| {
            g.track(p)
                .map_or(p.default_value(), |tr| tr.value(t, p.default_value()))
        };
        Some(NoteFx {
            dx: v(NoteProp::Dx) as f32,
            dy: v(NoteProp::Dy) as f32,
            scale: v(NoteProp::Scale) as f32,
            opacity: v(NoteProp::Opacity) as f32,
            hidden: v(NoteProp::Hidden) >= 0.5,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tko::gimmick::Group;
    use crate::tko::track::{Ease, Key};

    fn key(t: f64, v: f64) -> Key {
        Key {
            t,
            v,
            ease: Ease::Linear,
        }
    }

    fn runtime() -> Runtime {
        let mut g = Gimmick::default();
        g.key_field(FieldProp::Rotation, key(1000.0, 0.0));
        g.key_field(FieldProp::Rotation, key(2000.0, 90.0));
        g.groups.push(Group {
            name: "a".into(),
            notes: vec![1500.0, 2500.0],
            ..Group::default()
        });
        g.key_note(0, NoteProp::Opacity, key(1000.0, 1.0));
        g.key_note(0, NoteProp::Opacity, key(2000.0, 0.0));
        // Before its first key a track holds its first value, so a switch needs a key at 0.
        g.key_note(
            0,
            NoteProp::Hidden,
            Key {
                ease: Ease::Step,
                ..key(0.0, 0.0)
            },
        );
        g.key_note(0, NoteProp::Hidden, key(2400.0, 1.0));
        g.sanitize();
        Runtime::new(
            Arc::new(g),
            [500.0, 1500.0, 2000.0, 2500.0].into_iter(),
            0.0,
        )
    }

    #[test]
    fn without_keys_the_field_is_untouched() {
        let rt = Runtime::new(Arc::new(Gimmick::default()), [1.0].into_iter(), 0.0);
        assert_eq!(rt.field(1234.0), FieldFx::default());
        assert_eq!(rt.note(0, 1.0), None);
    }

    #[test]
    fn the_field_follows_its_keys() {
        let rt = runtime();
        assert_eq!(rt.field(0.0).rotation, 0.0);
        assert_eq!(rt.field(1500.0).rotation, 45.0);
        assert_eq!(rt.field(9000.0).rotation, 90.0);
        assert_eq!(
            rt.field(1500.0).zoom,
            1.0,
            "other properties keep their default"
        );
    }

    #[test]
    fn notes_are_found_by_their_time_and_only_those_in_a_group_animate() {
        let rt = runtime();
        assert_eq!(rt.group_of(0), None);
        assert_eq!(rt.group_of(1), Some(0));
        assert_eq!(rt.group_of(2), None);
        assert_eq!(rt.group_of(3), Some(0));
        assert_eq!(rt.note(1, 1500.0).unwrap().opacity, 0.5);
        assert!(!rt.note(1, 2000.0).unwrap().hidden);
        assert!(rt.note(3, 2600.0).unwrap().hidden);
        assert_eq!(rt.note(0, 1500.0), None);
    }

    #[test]
    fn the_games_clock_is_shifted_back_to_the_charts() {
        let mut g = Gimmick::default();
        g.key_field(FieldProp::X, key(1000.0, 10.0));
        g.key_field(FieldProp::X, key(2000.0, 20.0));
        g.groups.push(Group {
            notes: vec![1000.0],
            ..Group::default()
        });
        let rt = Runtime::new(Arc::new(g), [1023.0].into_iter(), 23.0);
        assert_eq!(rt.group_of(0), Some(0));
        assert_eq!(rt.field(1523.0).x, 15.0);
    }

    #[test]
    fn two_evaluations_agree() {
        let (a, b) = (runtime(), runtime());
        for i in 0..200 {
            let t = i as f64 * 37.0;
            assert_eq!(a.field(t), b.field(t));
            assert_eq!(a.note(1, t), b.note(1, t));
        }
    }
}
