//! The shared scroll (mania-style SV): one speed for the whole lane over the song, on top of the
//! map's own slider velocity, which stays as it is.
//!
//! The speed is a track of multipliers over the song (chart ms): 1 plays as mapped, 2 twice as
//! fast, 0 freezes every note where it is, below 0 runs them backwards. Notes are placed by how
//! far the song has scrolled between now and their hit, `X(hit) - X(now)` instead of
//! `hit - now`, so every note still reaches the drum exactly when it is hit.

use super::track::{Ease, Track};

/// The speed a key may ask for, either way.
pub const MAX_SPEED: f64 = 1000.0;
/// Samples each eased stretch between two keys is summed with.
const STEPS: usize = 32;

/// A speed track summed up into how far the song has scrolled by each moment.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollTable {
    /// Per key: its time and how far the song has scrolled there (0 at the first key).
    at: Vec<(f64, f64)>,
    /// Per eased stretch (after key `i`): how far it has scrolled at each of `STEPS` + 1 even
    /// points, from 0. Empty for stretches a straight line or a step covers exactly.
    curves: Vec<Vec<f64>>,
    speeds: Track,
}

impl ScrollTable {
    /// None for no keys: the scroll is the song's own.
    pub fn new(speed: &Track) -> Option<ScrollTable> {
        let keys = &speed.keys;
        let first = keys.first()?;
        let mut at = vec![(first.t, 0.0)];
        let mut curves = Vec::with_capacity(keys.len());
        for w in keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            let span = b.t - a.t;
            let (sum, curve) = match a.ease {
                Ease::Step => (a.v * span, Vec::new()),
                Ease::Linear => ((a.v + b.v) / 2.0 * span, Vec::new()),
                ease => {
                    // Trapezoids on a fine grid of the eased speed.
                    let mut c = Vec::with_capacity(STEPS + 1);
                    let mut s = 0.0;
                    let speed = |f: f64| a.v + (b.v - a.v) * ease.apply(f);
                    c.push(0.0);
                    for k in 1..=STEPS {
                        let (f0, f1) = ((k - 1) as f64 / STEPS as f64, k as f64 / STEPS as f64);
                        s += (speed(f0) + speed(f1)) / 2.0 * span / STEPS as f64;
                        c.push(s);
                    }
                    (s, c)
                }
            };
            let x = at.last().map_or(0.0, |p| p.1) + sum;
            at.push((b.t, x));
            curves.push(curve);
        }
        Some(ScrollTable {
            at,
            curves,
            speeds: speed.clone(),
        })
    }

    /// The speed at chart time `t`.
    pub fn speed(&self, t: f64) -> f64 {
        self.speeds.value(t, 1.0)
    }

    /// How far the song has scrolled by chart time `t`, in ms at speed 1.
    pub fn x(&self, t: f64) -> f64 {
        let keys = &self.speeds.keys;
        let (t0, _) = self.at[0];
        if t <= t0 {
            return keys[0].v * (t - t0);
        }
        let (tn, xn) = *self.at.last().unwrap();
        if t >= tn {
            return xn + keys[keys.len() - 1].v * (t - tn);
        }
        let i = self.at.partition_point(|p| p.0 <= t) - 1;
        let (a, b) = (keys[i], keys[i + 1]);
        let (ta, xa) = self.at[i];
        let span = b.t - a.t;
        if span <= 0.0 {
            return xa;
        }
        let d = t - ta;
        let f = d / span;
        xa + match a.ease {
            Ease::Step => a.v * d,
            Ease::Linear => (a.v + (a.v + (b.v - a.v) * f)) / 2.0 * d,
            _ => {
                let c = &self.curves[i];
                let k = f * STEPS as f64;
                let j = (k as usize).min(STEPS - 1);
                c[j] + (c[j + 1] - c[j]) * (k - j as f64)
            }
        }
    }
}

/// Makes what a file said safe to run.
pub fn sanitize(speed: &mut Track) {
    speed.sanitize();
    for k in &mut speed.keys {
        k.v = k.v.clamp(-MAX_SPEED, MAX_SPEED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tko::track::Key;

    fn key(t: f64, v: f64, ease: Ease) -> Key {
        Key { t, v, ease }
    }

    fn table(keys: &[Key]) -> ScrollTable {
        let mut t = Track::default();
        for &k in keys {
            t.set(k);
        }
        ScrollTable::new(&t).unwrap()
    }

    #[test]
    fn speed_one_scrolls_like_the_song_and_two_twice_as_far() {
        let t = table(&[key(0.0, 1.0, Ease::Step), key(1000.0, 2.0, Ease::Step)]);
        assert_eq!(t.x(500.0), 500.0);
        assert_eq!(t.x(1000.0), 1000.0);
        assert_eq!(t.x(1500.0), 2000.0);
        assert_eq!(t.x(-100.0), -100.0);
    }

    #[test]
    fn a_freeze_holds_and_a_reverse_goes_back() {
        let t = table(&[
            key(0.0, 1.0, Ease::Step),
            key(1000.0, 0.0, Ease::Step),
            key(2000.0, -1.0, Ease::Step),
            key(2500.0, 1.0, Ease::Step),
        ]);
        assert_eq!(t.x(1000.0), t.x(1999.0));
        assert_eq!(t.x(2500.0), 500.0);
        assert_eq!(t.x(3000.0), 1000.0);
    }

    #[test]
    fn eased_and_straight_stretches_add_up_right() {
        // A ramp from 0 to 2 over a second scrolls a second's worth.
        let t = table(&[key(0.0, 0.0, Ease::Linear), key(1000.0, 2.0, Ease::Step)]);
        assert!((t.x(1000.0) - 1000.0).abs() < 1e-9);
        assert!((t.x(500.0) - 250.0).abs() < 1e-9);
        // An eased one that is symmetric too, near enough.
        let t = table(&[key(0.0, 0.0, Ease::SineInOut), key(1000.0, 2.0, Ease::Step)]);
        assert!((t.x(1000.0) - 1000.0).abs() < 1.0);
        assert!(t.x(250.0) < t.x(500.0) && t.x(500.0) < t.x(750.0));
    }
}
