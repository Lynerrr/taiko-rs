//! Effect clips: the simple way to make a `.tko` map's effects, as blocks on the timeline.
//!
//! A clip says *what* it moves (a playfield property, a shake, or the notes), *when* (from its
//! start to its end, chart ms), *how much*, and *how often*: for its whole length, or again on
//! every few beats, every note, every hit or every miss inside it. "Kick the zoom on every
//! second beat of the kiai" is one clip laid over the kiai, set to every 2 beats. Lengths are in
//! beats of the map, so a clip keeps time with the music.
//!
//! What clips give is added to the keyframes and the node graph, like the graph's actions, and
//! is visual only: judging never reads it.

use serde::{Deserialize, Serialize};

use super::gimmick::{FieldProp, NoteProp};
use super::track::Ease;

pub const MAX_CLIPS: usize = 4096;
/// Rows of clips on the timeline.
pub const LANES: u8 = 8;
/// What a clip may say, at the most (beats).
const MAX_BEATS: f64 = 256.0;

/// How often a clip acts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum When {
    /// For its whole length: in over `fade_in` beats, out over `fade_out`.
    #[default]
    Span,
    /// Every `every` beats from its start.
    Beats {
        #[serde(default = "one")]
        every: f64,
    },
    /// On each note that reaches the drum inside it (`notes`: which).
    Notes {
        #[serde(default)]
        notes: Notes,
    },
    /// On each note the player hits inside it.
    Hits {
        #[serde(default)]
        notes: Notes,
    },
    /// On each note the player misses inside it.
    Misses,
}

fn one() -> f64 {
    1.0
}

/// Which notes a clip counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notes {
    #[default]
    All,
    Dons,
    Kats,
    Big,
}

impl Notes {
    pub const ALL: [Notes; 4] = [Notes::All, Notes::Dons, Notes::Kats, Notes::Big];

    pub fn label(self) -> &'static str {
        match self {
            Notes::All => "all notes",
            Notes::Dons => "dons",
            Notes::Kats => "kats",
            Notes::Big => "big notes",
        }
    }
}

/// What a repeating clip does at each moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    /// Jumps by the amount and settles back over `length` beats.
    #[default]
    Kick,
    /// Swings to the amount, then to minus it, and so on, taking `length` beats each way.
    Swing,
    /// Moves on by the amount each time, taking `length` beats: 90 on rotation is a quarter
    /// turn more at every moment.
    Step,
}

impl Motion {
    pub const ALL: [Motion; 3] = [Motion::Kick, Motion::Swing, Motion::Step];

    pub fn label(self) -> &'static str {
        match self {
            Motion::Kick => "kick",
            Motion::Swing => "swing",
            Motion::Step => "step",
        }
    }
}

/// What a clip moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Field(FieldProp),
    /// The whole playfield jolted in random directions, the amount in virtual units.
    Shake,
    /// The notes: all of them (`None`) or a group's.
    Notes(Option<usize>, NoteProp),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Stays with the clip through edits (the editor's selection).
    #[serde(default)]
    pub id: u32,
    /// The row it sits on in the editor.
    #[serde(default)]
    pub lane: u8,
    /// A playfield property's name (`zoom`), `shake`, or `notes.<property>` (`notes.dy`).
    pub what: String,
    /// For `notes.`: 0 all notes, else group `group - 1`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub group: u16,
    /// Chart ms.
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub amount: f64,
    #[serde(default)]
    pub when: When,
    #[serde(default)]
    pub motion: Motion,
    /// Beats a kick, swing or step takes.
    #[serde(default = "one")]
    pub length: f64,
    /// Beats a span takes to come in and to go out.
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
    #[serde(default)]
    pub ease: Ease,
    /// What it reached stays after its end (a turn that ends half way round stays there).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
}

fn is_zero(v: &u16) -> bool {
    *v == 0
}

impl Clip {
    /// A clip of `what` from `start` to `end`, doing nothing yet (amount 0).
    pub fn new(what: Target, start: f64, end: f64) -> Clip {
        let (what, group) = match what {
            Target::Field(p) => (p.name().to_string(), 0),
            Target::Shake => ("shake".to_string(), 0),
            Target::Notes(g, p) => (format!("notes.{}", p.name()), g.map_or(0, |g| g as u16 + 1)),
        };
        Clip {
            id: 0,
            lane: 0,
            what,
            group,
            start,
            end,
            amount: 0.0,
            when: When::Span,
            motion: Motion::Kick,
            length: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            ease: Ease::QuadOut,
            keep: false,
        }
    }

    pub fn target(&self) -> Option<Target> {
        if self.what == "shake" {
            return Some(Target::Shake);
        }
        if let Some(p) = self.what.strip_prefix("notes.") {
            let p = NoteProp::ALL.into_iter().find(|q| q.name() == p)?;
            let g = (self.group > 0).then(|| self.group as usize - 1);
            return Some(Target::Notes(g, p));
        }
        FieldProp::ALL
            .into_iter()
            .find(|p| p.name() == self.what)
            .map(Target::Field)
    }

    /// Whether what it does depends on how the player plays.
    pub fn is_dynamic(&self) -> bool {
        matches!(self.when, When::Hits { .. } | When::Misses)
    }

    /// What it adds at `t` (chart ms), given its `moments` (ascending, from `moments`) and the
    /// length of a beat at a time.
    pub fn value(&self, t: f64, moments: &[f64], beat: impl Fn(f64) -> f64) -> f64 {
        if t < self.start {
            return 0.0;
        }
        let a = self.amount;
        if self.when == When::Span {
            let b = beat(self.start).max(1.0);
            let (fin, fout) = (self.fade_in * b, self.fade_out * b);
            let into = if fin > 0.0 {
                self.ease.apply((t - self.start) / fin)
            } else {
                1.0
            };
            if self.keep && t >= self.end {
                let reached = if fin > 0.0 {
                    self.ease.apply((self.end - self.start) / fin)
                } else {
                    1.0
                };
                return a * reached;
            }
            if t >= self.end {
                return 0.0;
            }
            let out = if fout > 0.0 && !self.keep {
                self.ease.apply((self.end - t) / fout)
            } else {
                1.0
            };
            return a * into.min(out);
        }
        let at = moments.partition_point(|&m| m <= t);
        let Some(k) = at.checked_sub(1) else {
            return 0.0;
        };
        let m = moments[k];
        let len = (self.length * beat(m)).max(1.0);
        let x = (t - m) / len;
        match self.motion {
            Motion::Kick => {
                if x < 1.0 {
                    a * (1.0 - self.ease.apply(x))
                } else {
                    0.0
                }
            }
            Motion::Swing => {
                let side = |k: usize| if k.is_multiple_of(2) { a } else { -a };
                let from = if k == 0 { 0.0 } else { side(k - 1) };
                // After the clip it swings back to the middle.
                if t >= self.end {
                    let last = moments.last().copied().unwrap_or(m);
                    let back = ((t - self.end.max(last)) / len).min(1.0);
                    let here =
                        from + (side(k) - from) * self.ease.apply(((self.end - m) / len).min(1.0));
                    return if self.keep {
                        here
                    } else {
                        here * (1.0 - self.ease.apply(back))
                    };
                }
                from + (side(k) - from) * self.ease.apply(x)
            }
            Motion::Step => {
                if t >= self.end && !self.keep {
                    return 0.0;
                }
                a * (k as f64 + self.ease.apply(x.min(1.0)))
            }
        }
    }

    /// Brings what a file said into range. False for a clip to drop (unknown target, no length).
    fn sanitize(&mut self) -> bool {
        if !self.start.is_finite() || !self.end.is_finite() || self.target().is_none() {
            return false;
        }
        if self.end < self.start {
            std::mem::swap(&mut self.start, &mut self.end);
        }
        if self.end - self.start < 1.0 {
            return false;
        }
        self.lane = self.lane.min(LANES - 1);
        let num =
            |v: f64, lo: f64, hi: f64, or: f64| if v.is_finite() { v.clamp(lo, hi) } else { or };
        self.length = num(self.length, 1.0 / 64.0, MAX_BEATS, 1.0);
        self.fade_in = num(self.fade_in, 0.0, MAX_BEATS, 0.0);
        self.fade_out = num(self.fade_out, 0.0, MAX_BEATS, 0.0);
        if let When::Beats { every } = &mut self.when {
            *every = num(*every, 1.0 / 16.0, MAX_BEATS, 1.0);
        }
        // As far as the property itself goes, either way.
        let span = match self.target() {
            Some(Target::Field(p)) => {
                let (lo, hi) = p.range();
                hi - lo
            }
            Some(Target::Notes(_, p)) => {
                let (lo, hi) = p.range();
                hi - lo
            }
            _ => 400.0,
        };
        // A step can go round and round.
        let span = if self.motion == Motion::Step {
            span * 64.0
        } else {
            span
        };
        self.amount = num(self.amount, -span, span, 0.0);
        true
    }
}

/// Makes a file's clips safe to run: known targets, sane numbers, at most `MAX_CLIPS`, by start,
/// each with its own id.
pub fn sanitize(clips: &mut Vec<Clip>) {
    clips.retain_mut(Clip::sanitize);
    clips.truncate(MAX_CLIPS);
    clips.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.lane.cmp(&b.lane)));
    let mut seen = std::collections::HashSet::new();
    let mut next = clips.iter().map(|c| c.id).max().unwrap_or(0);
    for c in clips.iter_mut() {
        if c.id == 0 || !seen.insert(c.id) {
            next += 1;
            c.id = next;
            seen.insert(next);
        }
    }
}

/// Every `every` beats from `start` until `end`, with `beat` the length of a beat at a time.
pub fn beat_moments(
    start: f64,
    end: f64,
    every: f64,
    beat: impl Fn(f64) -> f64,
    cap: usize,
) -> Vec<f64> {
    let mut out = Vec::new();
    let mut t = start;
    while t < end && out.len() < cap {
        out.push(t);
        t += (every * beat(t)).max(1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEAT: f64 = 500.0;

    fn beat(_: f64) -> f64 {
        BEAT
    }

    fn clip(when: When, motion: Motion) -> Clip {
        Clip {
            amount: 1.0,
            when,
            motion,
            length: 1.0,
            ease: Ease::Linear,
            ..Clip::new(Target::Field(FieldProp::Zoom), 1000.0, 5000.0)
        }
    }

    #[test]
    fn a_span_fades_in_holds_and_fades_out() {
        let c = Clip {
            fade_in: 2.0,
            fade_out: 2.0,
            ..clip(When::Span, Motion::Kick)
        };
        let v = |t| c.value(t, &[], beat);
        assert_eq!(v(500.0), 0.0);
        assert!((v(1500.0) - 0.5).abs() < 1e-9, "half way in");
        assert_eq!(v(3000.0), 1.0);
        assert!((v(4500.0) - 0.5).abs() < 1e-9, "half way out");
        assert_eq!(v(5000.0), 0.0);
        let kept = Clip {
            keep: true,
            ..c.clone()
        };
        assert_eq!(kept.value(9000.0, &[], beat), 1.0, "kept after its end");
    }

    #[test]
    fn every_second_beat_kicks_and_settles() {
        let c = clip(When::Beats { every: 2.0 }, Motion::Kick);
        let m = beat_moments(c.start, c.end, 2.0, beat, 100);
        assert_eq!(m, [1000.0, 2000.0, 3000.0, 4000.0]);
        assert_eq!(c.value(1000.0, &m, beat), 1.0);
        assert!((c.value(1250.0, &m, beat) - 0.5).abs() < 1e-9);
        assert_eq!(c.value(1600.0, &m, beat), 0.0, "settled before the next");
        assert_eq!(c.value(2000.0, &m, beat), 1.0);
        assert_eq!(c.value(900.0, &m, beat), 0.0, "not before it starts");
    }

    #[test]
    fn a_swing_goes_from_side_to_side_and_a_step_keeps_going() {
        let s = clip(When::Beats { every: 1.0 }, Motion::Swing);
        let m = beat_moments(s.start, s.end, 1.0, beat, 100);
        // A beat out to one side, the next to the other.
        assert!((s.value(1250.0, &m, beat) - 0.5).abs() < 1e-9);
        assert_eq!(s.value(1500.0, &m, beat), 1.0, "out to one side");
        assert!(s.value(1750.0, &m, beat).abs() < 1e-9, "through the middle");
        assert_eq!(s.value(2000.0, &m, beat), -1.0, "then the other");
        assert_eq!(s.value(2500.0, &m, beat), 1.0);
        assert!(
            s.value(9000.0, &m, beat).abs() < 1e-9,
            "and back to it after"
        );
        let st = Clip {
            amount: 90.0,
            what: "rotation".into(),
            ..clip(When::Beats { every: 4.0 }, Motion::Step)
        };
        let m = beat_moments(st.start, st.end, 4.0, beat, 100);
        assert_eq!(m, [1000.0, 3000.0]);
        assert_eq!(st.value(2000.0, &m, beat), 90.0);
        assert_eq!(st.value(3250.0, &m, beat), 135.0);
        assert_eq!(st.value(4000.0, &m, beat), 180.0);
        assert_eq!(st.value(6000.0, &m, beat), 0.0, "gone after the clip");
    }

    #[test]
    fn targets_are_read_and_unknown_ones_dropped() {
        assert_eq!(
            Clip::new(Target::Notes(Some(2), NoteProp::Dy), 0.0, 1.0).target(),
            Some(Target::Notes(Some(2), NoteProp::Dy))
        );
        assert_eq!(
            Clip::new(Target::Notes(None, NoteProp::Hidden), 0.0, 1.0).target(),
            Some(Target::Notes(None, NoteProp::Hidden))
        );
        let mut clips = vec![
            Clip::new(Target::Shake, 2000.0, 1000.0),
            Clip {
                what: "evil".into(),
                ..Clip::new(Target::Shake, 0.0, 1000.0)
            },
            Clip {
                amount: f64::NAN,
                length: 1e9,
                lane: 200,
                ..Clip::new(Target::Field(FieldProp::Zoom), 0.0, 500.0)
            },
            Clip::new(Target::Field(FieldProp::Zoom), 300.0, 300.5),
        ];
        sanitize(&mut clips);
        assert_eq!(
            clips.len(),
            2,
            "the unknown and the zero-length are dropped"
        );
        assert_eq!((clips[0].start, clips[0].amount), (0.0, 0.0));
        assert_eq!(clips[0].length, MAX_BEATS);
        assert_eq!(clips[0].lane, LANES - 1);
        assert_eq!(
            (clips[1].start, clips[1].end),
            (1000.0, 2000.0),
            "turned the right way"
        );
        assert_ne!(clips[0].id, clips[1].id);
        assert!(clips.iter().all(|c| c.id > 0));
    }

    #[test]
    fn clips_round_trip_through_json() {
        let c = Clip {
            id: 4,
            lane: 2,
            when: When::Hits { notes: Notes::Kats },
            motion: Motion::Swing,
            keep: true,
            ..clip(When::Span, Motion::Kick)
        };
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Clip>(&text).unwrap(), c);
        let short: Clip =
            serde_json::from_str(r#"{"what":"zoom","start":0,"end":1000,"when":{"kind":"beats"}}"#)
                .unwrap();
        assert_eq!(short.when, When::Beats { every: 1.0 });
        assert_eq!(short.length, 1.0);
    }
}
