//! Keyframe tracks: a value over map time, set at keys, eased between them.
//!
//! The way an animation track in a video editor works: a key holds a value at a time, and the
//! easing on a key shapes the segment that starts there. Before the first key the track holds its
//! first value, after the last its last, and a track with no keys is not there (the property's
//! default applies).

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// The most keys one track keeps: a file can't make the player hold an unbounded list.
pub const MAX_KEYS: usize = 10_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    #[default]
    Linear,
    /// Holds the key's value until the next key.
    Step,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    SineIn,
    SineOut,
    SineInOut,
    BackIn,
    BackOut,
    ElasticOut,
    BounceOut,
}

impl Ease {
    pub const ALL: [Ease; 15] = [
        Ease::Linear,
        Ease::Step,
        Ease::QuadIn,
        Ease::QuadOut,
        Ease::QuadInOut,
        Ease::CubicIn,
        Ease::CubicOut,
        Ease::CubicInOut,
        Ease::SineIn,
        Ease::SineOut,
        Ease::SineInOut,
        Ease::BackIn,
        Ease::BackOut,
        Ease::ElasticOut,
        Ease::BounceOut,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Ease::Linear => "linear",
            Ease::Step => "step",
            Ease::QuadIn => "quad in",
            Ease::QuadOut => "quad out",
            Ease::QuadInOut => "quad in-out",
            Ease::CubicIn => "cubic in",
            Ease::CubicOut => "cubic out",
            Ease::CubicInOut => "cubic in-out",
            Ease::SineIn => "sine in",
            Ease::SineOut => "sine out",
            Ease::SineInOut => "sine in-out",
            Ease::BackIn => "back in",
            Ease::BackOut => "back out",
            Ease::ElasticOut => "elastic out",
            Ease::BounceOut => "bounce out",
        }
    }

    pub fn next(self) -> Ease {
        let at = Ease::ALL.iter().position(|e| *e == self).unwrap_or(0);
        Ease::ALL[(at + 1) % Ease::ALL.len()]
    }

    /// 0..1 progress -> 0..1 eased position (back and elastic overshoot it on the way).
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::Step => 0.0,
            Ease::QuadIn => t * t,
            Ease::QuadOut => 1.0 - (1.0 - t) * (1.0 - t),
            Ease::QuadInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Ease::CubicIn => t * t * t,
            Ease::CubicOut => 1.0 - (1.0 - t).powi(3),
            Ease::CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Ease::SineIn => 1.0 - (t * PI / 2.0).cos(),
            Ease::SineOut => (t * PI / 2.0).sin(),
            Ease::SineInOut => -((PI * t).cos() - 1.0) / 2.0,
            Ease::BackIn => {
                let (c1, c3) = (1.70158, 2.70158);
                c3 * t * t * t - c1 * t * t
            }
            Ease::BackOut => {
                let (c1, c3) = (1.70158, 2.70158);
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
            Ease::ElasticOut => {
                if t == 0.0 || t == 1.0 {
                    t
                } else {
                    2f64.powf(-10.0 * t) * ((t * 10.0 - 0.75) * (2.0 * PI / 3.0)).sin() + 1.0
                }
            }
            Ease::BounceOut => {
                let (n1, d1) = (7.5625, 2.75);
                if t < 1.0 / d1 {
                    n1 * t * t
                } else if t < 2.0 / d1 {
                    let t = t - 1.5 / d1;
                    n1 * t * t + 0.75
                } else if t < 2.5 / d1 {
                    let t = t - 2.25 / d1;
                    n1 * t * t + 0.9375
                } else {
                    let t = t - 2.625 / d1;
                    n1 * t * t + 0.984375
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Map time (ms).
    pub t: f64,
    pub v: f64,
    /// Shapes the segment from this key to the next.
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    #[serde(default)]
    pub keys: Vec<Key>,
}

impl Track {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The value at map time `t`; `default` for a track with no keys.
    pub fn value(&self, t: f64, default: f64) -> f64 {
        let keys = &self.keys;
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
            return default;
        };
        if t <= first.t {
            return first.v;
        }
        if t >= last.t {
            return last.v;
        }
        let at = keys.partition_point(|k| k.t <= t) - 1;
        let (a, b) = (&keys[at], &keys[at + 1]);
        let span = b.t - a.t;
        if span <= 0.0 {
            return b.v;
        }
        a.v + (b.v - a.v) * a.ease.apply((t - a.t) / span)
    }

    /// Adds a key, or replaces the one at the same time. Returns its index.
    pub fn set(&mut self, key: Key) -> usize {
        match self.keys.binary_search_by(|k| k.t.total_cmp(&key.t)) {
            Ok(i) => {
                self.keys[i] = key;
                i
            }
            Err(i) => {
                self.keys.insert(i, key);
                i
            }
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<Key> {
        (index < self.keys.len()).then(|| self.keys.remove(index))
    }

    /// Index of the key within `slack` ms of `t`, the nearest one.
    pub fn near(&self, t: f64, slack: f64) -> Option<usize> {
        self.keys
            .iter()
            .enumerate()
            .filter(|(_, k)| (k.t - t).abs() <= slack)
            .min_by(|a, b| (a.1.t - t).abs().total_cmp(&(b.1.t - t).abs()))
            .map(|(i, _)| i)
    }

    /// What a file may have wrong: keys out of order, not finite, too many.
    pub fn sanitize(&mut self) {
        self.keys.retain(|k| k.t.is_finite() && k.v.is_finite());
        self.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.keys.dedup_by(|b, a| a.t == b.t);
        self.keys.truncate(MAX_KEYS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(t: f64, v: f64, ease: Ease) -> Key {
        Key { t, v, ease }
    }

    #[test]
    fn every_easing_starts_at_zero_and_ends_at_one() {
        for e in Ease::ALL {
            if e == Ease::Step {
                assert_eq!(e.apply(0.99), 0.0);
                continue;
            }
            assert!(e.apply(0.0).abs() < 1e-9, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-9, "{e:?}");
        }
    }

    #[test]
    fn overshooting_easings_overshoot_and_the_rest_stay_inside() {
        assert!(Ease::BackOut.apply(0.7) > 1.0);
        assert!(Ease::BackIn.apply(0.2) < 0.0);
        assert!(Ease::ElasticOut.apply(0.2) > 1.0);
        for e in [
            Ease::QuadIn,
            Ease::CubicInOut,
            Ease::SineOut,
            Ease::BounceOut,
        ] {
            for i in 0..=20 {
                let v = e.apply(i as f64 / 20.0);
                assert!((-1e-9..=1.0 + 1e-9).contains(&v), "{e:?} {v}");
            }
        }
    }

    #[test]
    fn a_track_holds_before_and_after_and_eases_between() {
        let mut t = Track::default();
        assert_eq!(t.value(5.0, 7.0), 7.0, "no keys: the default");
        t.set(key(1000.0, 10.0, Ease::Linear));
        t.set(key(2000.0, 20.0, Ease::Linear));
        assert_eq!(t.value(0.0, 0.0), 10.0);
        assert_eq!(t.value(1500.0, 0.0), 15.0);
        assert_eq!(t.value(9999.0, 0.0), 20.0);
        t.keys[0].ease = Ease::Step;
        assert_eq!(
            t.value(1999.0, 0.0),
            10.0,
            "a step holds until the next key"
        );
        assert_eq!(t.value(2000.0, 0.0), 20.0);
    }

    #[test]
    fn setting_a_key_at_the_same_time_replaces_it_and_keys_stay_sorted() {
        let mut t = Track::default();
        t.set(key(2000.0, 2.0, Ease::Linear));
        t.set(key(1000.0, 1.0, Ease::Linear));
        let i = t.set(key(2000.0, 5.0, Ease::QuadIn));
        assert_eq!((i, t.keys.len(), t.keys[1].v), (1, 2, 5.0));
        assert!(t.keys[0].t < t.keys[1].t);
        assert_eq!(t.near(1010.0, 20.0), Some(0));
        assert_eq!(t.near(1500.0, 20.0), None);
        assert!(t.remove(0).is_some() && t.remove(5).is_none());
    }

    #[test]
    fn a_bad_track_is_made_safe() {
        let mut t = Track {
            keys: vec![
                key(300.0, 1.0, Ease::Linear),
                key(f64::NAN, 1.0, Ease::Linear),
                key(100.0, f64::INFINITY, Ease::Linear),
                key(100.0, 2.0, Ease::Linear),
                key(100.0, 3.0, Ease::Linear),
            ],
        };
        t.sanitize();
        let times: Vec<f64> = t.keys.iter().map(|k| k.t).collect();
        assert_eq!(times, [100.0, 300.0]);
    }

    #[test]
    fn easings_are_named_in_the_file_and_default_to_linear() {
        let k: Key = serde_json::from_str(r#"{"t":1,"v":2,"ease":"back_out"}"#).unwrap();
        assert_eq!(k.ease, Ease::BackOut);
        let k: Key = serde_json::from_str(r#"{"t":1,"v":2}"#).unwrap();
        assert_eq!(k.ease, Ease::Linear);
        assert!(
            serde_json::to_string(&Ease::QuadInOut)
                .unwrap()
                .contains("quad_in_out")
        );
    }
}
