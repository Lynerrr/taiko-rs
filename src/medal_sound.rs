//! What earning a medal sounds like, one sound per tier, made up rather than sampled. All hit on
//! the moment the medal lands, and they fall apart the way the picture does: the rarer the
//! medal, the more of the signal is stuttered, crushed and slowed down.

use std::f32::consts::TAU;

/// When the medal lands, in seconds: its scene is timed to this.
pub const LAND: f32 = 0.72;

/// A little noise source, the same every run.
struct Hiss(u32);

impl Hiss {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// A sound being put together.
struct Track {
    sr: f32,
    out: Vec<f32>,
}

impl Track {
    fn new(sr: f32, seconds: f32) -> Track {
        Track {
            sr,
            out: vec![0.0; (seconds * sr) as usize],
        }
    }

    fn at(&self, seconds: f32) -> usize {
        ((seconds * self.sr).max(0.0) as usize).min(self.out.len())
    }

    fn add(&mut self, at: f32, sound: &[f32], level: f32) {
        let from = self.at(at);
        for (o, s) in self.out[from..].iter_mut().zip(sound) {
            *o += s * level;
        }
    }

    /// Silence from `from` to `to`: the held breath before a big hit.
    fn cut(&mut self, from: f32, to: f32) {
        let (from, to) = (self.at(from), self.at(to));
        self.out[from..to].fill(0.0);
    }

    /// The `slice` seconds at `at` repeat over the next `times` slices: the buffer got stuck.
    fn stutter(&mut self, at: f32, slice: f32, times: usize) {
        let (from, len) = (self.at(at), (slice * self.sr) as usize);
        for k in 1..=times {
            for i in 0..len {
                let to = from + k * len + i;
                if to < self.out.len() && from + i < self.out.len() {
                    self.out[to] = self.out[from + i];
                }
            }
        }
    }

    /// From `from` on, fewer bits and a coarser sample rate, worse the further it goes, from
    /// `bits` down to `bits_end`.
    fn crush(&mut self, from: f32, bits: f32, bits_end: f32, hold: usize) {
        let from = self.at(from);
        let len = (self.out.len() - from).max(1);
        let mut held = 0.0;
        for (i, s) in self.out[from..].iter_mut().enumerate() {
            let k = i as f32 / len as f32;
            let steps = 2f32.powf(bits + (bits_end - bits) * k);
            let every = 1 + (hold as f32 * k) as usize;
            if i % every == 0 {
                held = (*s * steps).round() / steps;
            }
            *s = held;
        }
    }

    /// From `from`, the tape slows to a halt over the rest of the sound.
    fn tape_stop(&mut self, from: f32) {
        let from = self.at(from);
        let tail = self.out[from..].to_vec();
        let len = tail.len().max(1) as f32;
        let mut read = 0.0f32;
        for (i, s) in self.out[from..].iter_mut().enumerate() {
            let speed = (1.0 - i as f32 / len).powf(1.4);
            read += speed;
            let at = read as usize;
            let t = read.fract();
            *s = tail.get(at).copied().unwrap_or(0.0) * (1.0 - t)
                + tail.get(at + 1).copied().unwrap_or(0.0) * t;
        }
    }

    /// Ring modulation from `from` on: the metal edge of the screech.
    fn ring(&mut self, from: f32, freq: f32, depth: f32) {
        let from = self.at(from);
        let sr = self.sr;
        for (i, s) in self.out[from..].iter_mut().enumerate() {
            *s *= 1.0 - depth + depth * (i as f32 / sr * freq * TAU).sin();
        }
    }

    /// A room around it: four combs into two allpasses.
    fn reverb(&mut self, size: f32, feedback: f32, wet: f32) {
        let mut bank = vec![0.0f32; self.out.len()];
        for delay in [0.0297, 0.0371, 0.0411, 0.0437] {
            let d = ((delay * size * self.sr) as usize).max(1);
            let mut line = self.out.clone();
            for i in d..line.len() {
                line[i] += line[i - d] * feedback;
            }
            for (b, l) in bank.iter_mut().zip(&line) {
                *b += l / 4.0;
            }
        }
        for (delay, g) in [(0.005, 0.7), (0.0017, 0.7)] {
            let d = ((delay * self.sr) as usize).max(1);
            let dry = bank.clone();
            for i in 0..bank.len() {
                let (back, past) = if i >= d {
                    (dry[i - d], bank[i - d])
                } else {
                    (0.0, 0.0)
                };
                bank[i] = -g * dry[i] + back + g * past;
            }
        }
        for (o, b) in self.out.iter_mut().zip(&bank) {
            *o += b * wet;
        }
    }

    /// Driven and scaled to `peak`, with a short fade so it doesn't end in a click.
    fn finish(mut self, drive: f32, peak: f32) -> Vec<f32> {
        let most = self.out.iter().fold(1e-6f32, |m, s| m.max(s.abs()));
        let top = drive.tanh();
        let len = self.out.len();
        let fade = (0.04 * self.sr) as usize;
        for (i, s) in self.out.iter_mut().enumerate() {
            *s =
                (*s / most * drive).tanh() / top * peak * ((len - i) as f32 / fade as f32).min(1.0);
        }
        self.out
    }
}

/// A struck skin: a pitch falling fast, a second mode, and a slap of noise.
fn drum(sr: f32, seconds: f32, freq: f32, decay: f32, slap: f32, hiss: &mut Hiss) -> Vec<f32> {
    let mut phase = 0.0f32;
    let mut low = 0.0f32;
    (0..(seconds * sr) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            phase += freq * (1.0 + 2.8 * (-t * 34.0).exp()) / sr;
            low += 0.35 * (hiss.next() - low);
            (-t * decay).exp() * (phase * TAU).sin()
                + 0.4 * (-t * decay * 1.8).exp() * (phase * 1.59 * TAU).sin()
                + slap * (-t * 55.0).exp() * low * 2.0
        })
        .collect()
}

/// The rim of the drum: a short high knock.
fn rim(sr: f32, hiss: &mut Hiss) -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..(0.09 * sr) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            phase += 900.0 * (1.0 + 0.6 * (-t * 60.0).exp()) / sr;
            (-t * 42.0).exp() * ((phase * TAU).sin() + 0.5 * hiss.next() * (-t * 120.0).exp())
        })
        .collect()
}

/// A bell chord ringing out.
fn bells(sr: f32, seconds: f32, freqs: &[f32], decay: f32) -> Vec<f32> {
    (0..(seconds * sr) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            let env = (t / 0.004).min(1.0) * (-t * decay).exp();
            env * freqs
                .iter()
                .map(|f| {
                    (t * f * TAU).sin()
                        + 0.3 * (t * f * 2.0 * TAU).sin()
                        + 0.1 * (t * f * 3.01 * TAU).sin()
                })
                .sum::<f32>()
        })
        .collect()
}

/// Detuned saws through a filter closing as it fades: a heavy chord.
fn saws(sr: f32, seconds: f32, freqs: &[f32], decay: f32) -> Vec<f32> {
    let mut phases = vec![0.0f32; freqs.len() * 2];
    let mut low = 0.0f32;
    (0..(seconds * sr) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            let mut x = 0.0;
            for (k, p) in phases.iter_mut().enumerate() {
                let detune = if k % 2 == 0 { 0.996 } else { 1.004 };
                *p = (*p + freqs[k / 2] * detune / sr).fract();
                x += *p * 2.0 - 1.0;
            }
            let cut = 300.0 + 3200.0 * (-t * 3.5).exp();
            low += (1.0 - (-TAU * cut / sr).exp()) * (x - low);
            low * (t / 0.006).min(1.0) * (-t * decay).exp()
        })
        .collect()
}

/// Noise opening up as the medal comes in.
fn riser(sr: f32, seconds: f32, hiss: &mut Hiss) -> Vec<f32> {
    let n = (seconds * sr) as usize;
    let mut low = 0.0f32;
    (0..n)
        .map(|i| {
            let k = i as f32 / n as f32;
            let cut = 250.0 * 26f32.powf(k);
            low += (1.0 - (-TAU * cut / sr).exp()) * (hiss.next() - low);
            low * k * k * k
        })
        .collect()
}

/// Rim knocks closing in on the landing, faster and faster: the signal winding up.
fn wind_up(t: &mut Track, hiss: &mut Hiss, knocks: usize, level: f32) {
    let knock = rim(t.sr, hiss);
    let mut at = LAND;
    let mut gap = 0.02;
    let mut when = Vec::with_capacity(knocks);
    for _ in 0..knocks {
        at -= gap;
        gap *= 1.32;
        when.push(at);
    }
    for (k, at) in when.iter().rev().enumerate() {
        t.add(*at, &knock, level * (0.5 + 0.5 * k as f32 / knocks as f32));
    }
}

/// Kantan: one clean hit and a small chord.
fn kantan(sr: f32) -> Vec<f32> {
    let mut hiss = Hiss(0x51ab_1e01);
    let mut t = Track::new(sr, 1.5);
    let swell = riser(sr, LAND, &mut hiss);
    t.add(0.0, &swell, 0.1);
    let hit = drum(sr, 0.8, 150.0, 9.0, 0.35, &mut hiss);
    t.add(LAND, &hit, 0.8);
    t.add(LAND, &bells(sr, 0.7, &[1046.5, 1568.0], 7.0), 0.1);
    t.reverb(0.8, 0.6, 0.18);
    t.finish(1.0, 0.6)
}

/// Futsuu: a knock or two first, and the tail catches once.
fn futsuu(sr: f32) -> Vec<f32> {
    let mut hiss = Hiss(0x0f00_d002);
    let mut t = Track::new(sr, 1.8);
    let swell = riser(sr, LAND, &mut hiss);
    t.add(0.0, &swell, 0.16);
    wind_up(&mut t, &mut hiss, 3, 0.26);
    let hit = drum(sr, 1.0, 118.0, 7.0, 0.42, &mut hiss);
    t.add(LAND, &hit, 1.0);
    t.add(LAND, &bells(sr, 0.9, &[1046.5, 1318.5, 1568.0], 5.0), 0.1);
    t.stutter(LAND + 0.24, 0.045, 3);
    t.crush(LAND + 0.3, 11.0, 7.0, 3);
    t.reverb(1.0, 0.68, 0.22);
    t.finish(1.2, 0.72)
}

/// Muzukashii: a roll into a big hit and a heavy chord, and the bits start running out.
fn muzukashii(sr: f32) -> Vec<f32> {
    let mut hiss = Hiss(0x7ead_0003);
    let mut t = Track::new(sr, 2.3);
    let swell = riser(sr, LAND, &mut hiss);
    t.add(0.0, &swell, 0.32);
    wind_up(&mut t, &mut hiss, 8, 0.3);
    let hit = drum(sr, 1.4, 82.0, 4.6, 0.55, &mut hiss);
    t.add(LAND, &hit, 1.3);
    t.add(
        LAND,
        &saws(sr, 1.5, &[130.81, 196.0, 261.63, 329.63, 392.0], 2.6),
        0.14,
    );
    t.add(LAND, &bells(sr, 1.2, &[1046.5, 1568.0, 2093.0], 3.5), 0.08);
    t.reverb(1.2, 0.74, 0.28);
    t.stutter(LAND + 0.3, 0.05, 3);
    t.stutter(LAND + 0.62, 0.025, 6);
    t.crush(LAND + 0.2, 10.0, 4.5, 8);
    t.finish(1.5, 0.82)
}

/// Oni: a roll winding up to a breath of silence, then the hit tears the signal apart, stuck and
/// crushed, until the tape runs down.
fn oni(sr: f32) -> Vec<f32> {
    let mut hiss = Hiss(0xdead_0004);
    let mut t = Track::new(sr, 3.0);
    let swell = riser(sr, LAND, &mut hiss);
    t.add(0.0, &swell, 0.5);
    wind_up(&mut t, &mut hiss, 14, 0.42);
    let hit = drum(sr, 1.9, 52.0, 2.9, 0.8, &mut hiss);
    t.add(LAND, &hit, 1.7);
    let second = drum(sr, 1.0, 110.0, 6.0, 0.5, &mut hiss);
    t.add(LAND, &second, 0.6);
    t.add(
        LAND,
        &saws(sr, 2.2, &[73.42, 110.0, 146.83, 220.0, 293.66, 311.13], 1.5),
        0.2,
    );
    let crash: Vec<f32> = (0..(1.6 * sr) as usize)
        .map(|i| hiss.next() * (-(i as f32) / sr * 4.0).exp())
        .collect();
    t.add(LAND, &crash, 0.09);
    t.ring(LAND + 0.05, 57.0, 0.35);
    t.reverb(1.3, 0.76, 0.3);
    t.cut(LAND - 0.05, LAND);
    t.stutter(LAND + 0.28, 0.06, 2);
    t.stutter(LAND + 0.55, 0.02, 11);
    t.stutter(LAND + 1.1, 0.04, 4);
    t.crush(LAND + 0.25, 8.0, 3.2, 14);
    t.tape_stop(LAND + 1.3);
    t.finish(2.2, 0.95)
}

/// Ura: a chord played backwards into dead silence, then something low and wrong, under radio
/// static that keeps cutting out. Everything else is in a major key; this isn't.
fn ura(sr: f32) -> Vec<f32> {
    let mut hiss = Hiss(0xc0de_0005);
    let mut t = Track::new(sr, 3.3);
    let mut back = bells(sr, LAND, &[293.66, 415.3, 554.37, 830.61], 4.0);
    back.reverse();
    t.add(0.0, &back, 0.24);
    let hit = drum(sr, 2.2, 41.0, 2.1, 0.5, &mut hiss);
    t.add(LAND, &hit, 1.6);
    t.add(
        LAND,
        &saws(sr, 2.5, &[73.42, 77.78, 103.83, 155.56], 1.1),
        0.24,
    );
    let bucket = (0.04 * sr) as usize;
    let mut static_on = true;
    let burst: Vec<f32> = (0..(2.0 * sr) as usize)
        .map(|i| {
            if i % bucket == 0 {
                static_on = hiss.next() > 0.1;
            }
            let x = hiss.next();
            if static_on {
                x * (-(i as f32) / sr * 1.6).exp()
            } else {
                0.0
            }
        })
        .collect();
    t.add(LAND, &burst, 0.2);
    t.ring(LAND + 0.1, 31.0, 0.5);
    t.reverb(1.5, 0.8, 0.34);
    t.cut(LAND - 0.09, LAND);
    t.stutter(LAND + 0.4, 0.09, 2);
    t.stutter(LAND + 0.86, 0.015, 16);
    t.crush(LAND + 0.1, 7.0, 3.0, 20);
    t.tape_stop(LAND + 1.4);
    t.finish(2.0, 0.92)
}

/// The sounds at `rate`: the four tiers, then secrets.
pub fn sounds(rate: u32) -> [Vec<f32>; 5] {
    let sr = rate as f32;
    [kantan(sr), futsuu(sr), muzukashii(sr), oni(sr), ura(sr)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// The loudest sample in a stretch of seconds.
    fn peak(s: &[f32], from: f32, to: f32) -> f32 {
        let (from, to) = (
            (from * RATE as f32) as usize,
            ((to * RATE as f32) as usize).min(s.len()),
        );
        s[from..to].iter().fold(0.0, |m, x| m.max(x.abs()))
    }

    #[test]
    fn every_sound_hits_on_the_landing_and_stays_in_range() {
        for (tier, s) in sounds(RATE).iter().enumerate() {
            assert!(
                s.iter().all(|x| x.is_finite() && x.abs() <= 1.0),
                "tier {tier}"
            );
            let before = peak(s, 0.0, LAND - 0.01);
            let after = peak(s, LAND, LAND + 0.12);
            assert!(
                after > before * 1.3,
                "tier {tier}: {before} before, {after} on the hit"
            );
        }
    }

    #[test]
    fn rarer_medals_are_longer_and_louder() {
        let s = sounds(RATE);
        for pair in s[..4].windows(2) {
            assert!(pair[1].len() > pair[0].len());
            assert!(peak(&pair[1], 0.0, 9.0) > peak(&pair[0], 0.0, 9.0));
        }
    }

    #[test]
    fn the_big_ones_hold_their_breath_before_the_hit() {
        let s = sounds(RATE);
        assert_eq!(peak(&s[3], LAND - 0.04, LAND - 0.002), 0.0);
        assert_eq!(peak(&s[4], LAND - 0.08, LAND - 0.002), 0.0);
    }

    #[test]
    fn the_tape_runs_down_to_nothing() {
        let mut t = Track::new(RATE as f32, 1.0);
        let tone: Vec<f32> = (0..RATE as usize)
            .map(|i| (i as f32 / 100.0).sin())
            .collect();
        t.add(0.0, &tone, 1.0);
        t.tape_stop(0.5);
        assert!(
            peak(&t.out, 0.96, 0.999) < peak(&t.out, 0.5, 0.6),
            "slower and slower, so it changes less and less"
        );
    }

    #[test]
    fn a_stutter_repeats_its_slice() {
        let mut t = Track::new(1000.0, 1.0);
        let ramp: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        t.add(0.0, &ramp, 1.0);
        t.stutter(0.1, 0.01, 2);
        assert_eq!(t.out[110..120], t.out[100..110]);
        assert_eq!(t.out[120..130], t.out[100..110]);
        assert_eq!(t.out[130], 130.0);
    }
}
