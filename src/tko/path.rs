//! Approach paths: a group's notes fly in along a curve the mapper draws and land on the drum on
//! time, instead of scrolling along the lane.
//!
//! A note on a path is as far along it as it is along the lane: it comes in at the path's start
//! where it would come in at the lane's end, and is at the drum when it is hit. Its slider
//! velocity, the scroll speed and anything that moves notes along the lane therefore move it along
//! the path just the same. `pace` bends that (stop half way, rush the end, back up), and `weight`
//! takes the notes from the lane onto the path and back over the song.

use super::track::Track;
use serde::{Deserialize, Serialize};

/// Points a path may have.
pub const MAX_POINTS: usize = 32;
/// How far from the drum a point may be, in playfield units.
pub const REACH: f64 = 2000.0;
/// Pieces each stretch between two points is drawn with.
const STEPS: usize = 16;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NotePath {
    /// Where the notes come in from and the way they take, in playfield units from the drum (x
    /// right, y down), first to last; the drum ends the path.
    #[serde(default)]
    pub points: Vec<[f64; 2]>,
    /// How much of the path a note has left (value) by how much of the lane it has left (time),
    /// both 1 where it comes in and 0 at the drum. No keys: the same.
    #[serde(default, skip_serializing_if = "Track::is_empty")]
    pub pace: Track,
    /// Over the song (chart ms): 0 the notes keep to the lane, 1 they fly the path. No keys: 1.
    #[serde(default, skip_serializing_if = "Track::is_empty")]
    pub weight: Track,
    /// Draws the path faintly, for the player to read where notes will come from.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show: bool,
    /// The notes keep to a plain straight lane instead, whatever shape the lane is bent into,
    /// as far from the drum and as fast as on it; `points` and `pace` aren't used.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub straight: bool,
}

impl NotePath {
    /// Makes what a file said safe to run: points finite and within reach, pace and weight
    /// within 0..1.
    pub fn sanitize(&mut self) {
        self.points.retain(|p| p[0].is_finite() && p[1].is_finite());
        self.points.truncate(MAX_POINTS);
        for p in &mut self.points {
            p[0] = p[0].clamp(-REACH, REACH);
            p[1] = p[1].clamp(-REACH, REACH);
        }
        for t in [&mut self.pace, &mut self.weight] {
            t.sanitize();
            t.keys.iter_mut().for_each(|k| k.v = k.v.clamp(0.0, 1.0));
        }
        self.pace.keys.retain(|k| (0.0..=1.0).contains(&k.t));
    }

    /// How far the notes are on the path at chart time `t`.
    pub fn weight_at(&self, t: f64) -> f64 {
        self.weight.value(t, 1.0).clamp(0.0, 1.0)
    }

    /// How much of the path is left for a note with `u` of the lane left (0..1).
    pub fn pace_at(&self, u: f64) -> f64 {
        if self.pace.is_empty() {
            u
        } else {
            self.pace.value(u.clamp(0.0, 1.0), u).clamp(0.0, 1.0)
        }
    }
}

/// A path laid out as short straight pieces, measured, for placing notes along it fast.
#[derive(Clone, Debug, PartialEq)]
pub struct PathTable {
    /// From where notes come in to the drum (0, 0).
    pub pts: Vec<[f32; 2]>,
    /// Distance from the first point to each one.
    cum: Vec<f32>,
}

impl PathTable {
    /// None for a path without points (only the drum).
    pub fn new(path: &NotePath) -> Option<PathTable> {
        if path.points.is_empty() {
            return None;
        }
        let mut ctrl: Vec<[f32; 2]> = path
            .points
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32])
            .collect();
        ctrl.push([0.0, 0.0]);
        ctrl.dedup();
        if ctrl.len() < 2 {
            return None;
        }
        // Catmull-Rom through every point, the ends held.
        let at = |k: isize| ctrl[k.clamp(0, ctrl.len() as isize - 1) as usize];
        let mut pts = vec![ctrl[0]];
        for k in 0..ctrl.len() as isize - 1 {
            let (p0, p1, p2, p3) = (at(k - 1), at(k), at(k + 1), at(k + 2));
            for s in 1..=STEPS {
                let t = s as f32 / STEPS as f32;
                let (t2, t3) = (t * t, t * t * t);
                let c = |i: usize| {
                    0.5 * (2.0 * p1[i]
                        + (p2[i] - p0[i]) * t
                        + (2.0 * p0[i] - 5.0 * p1[i] + 4.0 * p2[i] - p3[i]) * t2
                        + (3.0 * p1[i] - p0[i] - 3.0 * p2[i] + p3[i]) * t3)
                };
                pts.push([c(0), c(1)]);
            }
        }
        let mut cum = Vec::with_capacity(pts.len());
        let mut d = 0.0;
        for (k, p) in pts.iter().enumerate() {
            if k > 0 {
                let q = pts[k - 1];
                d += ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
            }
            cum.push(d);
        }
        Some(PathTable { pts, cum })
    }

    pub fn len(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// The point `d` along the path from its start; before the start and past the drum it goes
    /// on straight the way the path heads there.
    pub fn at(&self, d: f32) -> [f32; 2] {
        let n = self.pts.len();
        let along = |a: [f32; 2], b: [f32; 2], over: f32| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-6);
            [b[0] + dx / l * over, b[1] + dy / l * over]
        };
        if d <= 0.0 {
            return along(self.pts[1], self.pts[0], -d);
        }
        let len = self.len();
        if d >= len {
            return along(self.pts[n - 2], self.pts[n - 1], d - len);
        }
        let k = self.cum.partition_point(|&c| c <= d).clamp(1, n - 1);
        let (a, b) = (self.pts[k - 1], self.pts[k]);
        let span = (self.cum[k] - self.cum[k - 1]).max(1e-6);
        let f = (d - self.cum[k - 1]) / span;
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
    }

    /// Where a note `s` along a lane `lane` long sits on the path, from the drum.
    pub fn place(&self, path: &NotePath, s: f32, lane: f32) -> [f32; 2] {
        let lane = lane.max(1.0);
        let u = s / lane;
        let len = self.len();
        let d = if u > 1.0 {
            -(u - 1.0) * lane
        } else if u < 0.0 {
            len - u * lane
        } else {
            (1.0 - path.pace_at(u as f64) as f32) * len
        };
        self.at(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tko::track::{Ease, Key};

    fn key(t: f64, v: f64) -> Key {
        Key {
            t,
            v,
            ease: Ease::Linear,
        }
    }

    fn arc() -> NotePath {
        NotePath {
            points: vec![[400.0, -300.0], [200.0, -200.0]],
            ..NotePath::default()
        }
    }

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 0.5 && (a[1] - b[1]).abs() < 0.5
    }

    #[test]
    fn a_note_comes_in_at_the_start_and_is_on_the_drum_when_hit() {
        let p = arc();
        let t = PathTable::new(&p).unwrap();
        assert!(near(t.place(&p, 800.0, 800.0), [400.0, -300.0]));
        assert!(near(t.place(&p, 0.0, 800.0), [0.0, 0.0]));
        // Half way along the lane, half way along the path.
        let mid = t.place(&p, 400.0, 800.0);
        assert!(near(mid, t.at(t.len() / 2.0)));
        // Past the drum it goes on the way it was heading: down and left here.
        let past = t.place(&p, -50.0, 800.0);
        assert!(past[0] < 0.0 && past[1] > 0.0);
        // Before it comes in it waits on the line in, further out.
        let early = t.place(&p, 900.0, 800.0);
        assert!(early[0] > 400.0 && early[1] < -300.0);
    }

    #[test]
    fn the_pace_can_hold_notes_still_half_way() {
        let mut p = arc();
        p.pace.set(key(1.0, 1.0));
        p.pace.set(key(0.6, 0.5));
        p.pace.set(key(0.4, 0.5));
        p.pace.set(key(0.0, 0.0));
        let t = PathTable::new(&p).unwrap();
        assert_eq!(t.place(&p, 450.0, 1000.0), t.place(&p, 550.0, 1000.0));
        assert!(near(t.place(&p, 0.0, 1000.0), [0.0, 0.0]));
    }

    #[test]
    fn a_path_without_points_is_none_and_files_are_made_safe() {
        assert!(PathTable::new(&NotePath::default()).is_none());
        let mut p = NotePath {
            points: vec![[f64::NAN, 0.0], [1e9, -1e9]],
            ..NotePath::default()
        };
        p.weight.set(key(0.0, 7.0));
        p.pace.set(key(5.0, 0.5));
        p.sanitize();
        assert_eq!(p.points, [[REACH, -REACH]]);
        assert_eq!(p.weight_at(0.0), 1.0);
        assert!(p.pace.is_empty());
        // A path is the drum and its points, so a point on the drum alone is no path.
        let only_drum = NotePath {
            points: vec![[0.0, 0.0]],
            ..NotePath::default()
        };
        assert!(PathTable::new(&only_drum).is_none());
    }
}
