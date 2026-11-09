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
pub const LANES: u8 = 64;
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

    #[inline]
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

    #[inline]
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
    /// The lane bent into shape `LANE_SHAPES[k]`: the amount is how far, 0 straight to 1 the
    /// whole shape.
    Lane(usize),
    /// Text or a picture on screen for the clip's length (`Clip::layer`). Repeating, it pulses:
    /// the amount is how much bigger it gets, in percent.
    Layer,
    /// Copy `k` (of `MAX_COPIES`) of the playfield: drawn behind it, moved on its own.
    Copy(usize, CopyProp),
}

/// Copies of the playfield a map can show at once.
pub const MAX_COPIES: usize = 4;

/// What a copy of the playfield does, on top of where the playfield is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyProp {
    /// How much of it shows: 0 none (there is no copy), 1 as solid as the playfield.
    Show,
    /// Virtual units right and down from the playfield.
    X,
    Y,
    /// Degrees clockwise about the middle of the screen.
    Turn,
    /// Size, 1 as the playfield.
    Zoom,
    /// How see-through its lane is (0 solid, 1 gone: only its notes).
    LaneFade,
}

impl CopyProp {
    pub const ALL: [CopyProp; 6] = [
        CopyProp::Show,
        CopyProp::X,
        CopyProp::Y,
        CopyProp::Turn,
        CopyProp::Zoom,
        CopyProp::LaneFade,
    ];

    #[inline]
    pub fn name(self) -> &'static str {
        match self {
            CopyProp::Show => "show",
            CopyProp::X => "x",
            CopyProp::Y => "y",
            CopyProp::Turn => "turn",
            CopyProp::Zoom => "zoom",
            CopyProp::LaneFade => "lane_fade",
        }
    }

    /// What it is with no clip on it.
    pub fn default_value(self) -> f64 {
        match self {
            CopyProp::Zoom => 1.0,
            _ => 0.0,
        }
    }

    pub fn range(self) -> (f64, f64) {
        match self {
            CopyProp::Show | CopyProp::LaneFade => (0.0, 1.0),
            CopyProp::X | CopyProp::Y => (-2000.0, 2000.0),
            CopyProp::Turn => (-3600.0, 3600.0),
            CopyProp::Zoom => (0.05, 8.0),
        }
    }

    #[inline]
    pub fn about(self) -> &'static str {
        match self {
            CopyProp::Show => {
                "Shows a copy of the playfield behind it: 0 none, 1 solid. Only the real one is played; the copy just shows the same notes."
            }
            CopyProp::X => {
                "Slides the copy right of the playfield (left below 0), in virtual units."
            }
            CopyProp::Y => {
                "Slides the copy below the playfield (above it below 0), in virtual units."
            }
            CopyProp::Turn => {
                "Turns the copy about the middle of the screen, in degrees: 180 is upside down with the drum on the right."
            }
            CopyProp::Zoom => {
                "How big the copy is: the amount adds to 1, so -0.5 is half the size."
            }
            CopyProp::LaneFade => "How see-through the copy's lane is: 1 leaves only its notes.",
        }
    }
}

/// The longest text a layer shows.
pub const MAX_TEXT: usize = 120;
/// The fastest a layer may spin, in degrees a beat.
pub const MAX_SPIN: f64 = 3600.0;

/// What a layer shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Show {
    /// In the game's own font.
    Text { text: String },
    /// An image file in the map's folder.
    Picture { file: String },
}

/// Where and how a layer is, in units of the 480 tall screen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pose {
    /// From the middle of the screen, right.
    pub x: f64,
    /// From the top of the screen, down.
    pub y: f64,
    /// How tall: a line of text, or the picture.
    pub size: f64,
    /// Degrees, clockwise.
    pub turn: f64,
    /// 0 unseen to 1.
    pub alpha: f64,
}

impl Default for Pose {
    fn default() -> Self {
        Pose {
            x: 0.0,
            y: 110.0,
            size: 36.0,
            turn: 0.0,
            alpha: 1.0,
        }
    }
}

impl Pose {
    fn sanitize(&mut self) {
        let num =
            |v: f64, lo: f64, hi: f64, or: f64| if v.is_finite() { v.clamp(lo, hi) } else { or };
        self.x = num(self.x, -4000.0, 4000.0, 0.0);
        self.y = num(self.y, -4000.0, 4000.0, 110.0);
        self.size = num(self.size, 1.0, 4000.0, 36.0);
        self.turn = num(self.turn, -36000.0, 36000.0, 0.0);
        self.alpha = num(self.alpha, 0.0, 1.0, 1.0);
    }

    /// Between `self` (0) and `b` (1).
    pub fn lerp(&self, b: &Pose, k: f64) -> Pose {
        let m = |a: f64, b: f64| a + (b - a) * k;
        Pose {
            x: m(self.x, b.x),
            y: m(self.y, b.y),
            size: m(self.size, b.size),
            turn: m(self.turn, b.turn),
            alpha: m(self.alpha, b.alpha),
        }
    }
}

/// Text or a picture a clip puts on screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub show: Show,
    /// At the clip's start.
    #[serde(default)]
    pub from: Pose,
    /// At its end, reached by its easing; none: it stays as it starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Pose>,
    /// RGB the text or picture is tinted.
    #[serde(default = "white")]
    pub colour: u32,
    /// Under the lane and notes instead of over them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub behind: bool,
    /// Degrees it keeps turning by each beat, on top of its pose (0: it doesn't spin).
    #[serde(default, skip_serializing_if = "no_spin")]
    pub spin: f64,
}

fn no_spin(v: &f64) -> bool {
    *v == 0.0
}

fn white() -> u32 {
    0xffffff
}

impl Default for Layer {
    fn default() -> Self {
        Layer {
            show: Show::Text {
                text: "Text".into(),
            },
            from: Pose::default(),
            to: None,
            colour: white(),
            behind: false,
            spin: 0.0,
        }
    }
}

impl Layer {
    fn sanitize(&mut self) -> bool {
        match &mut self.show {
            Show::Text { text } => {
                let clean: String = text
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_TEXT)
                    .collect();
                *text = clean;
            }
            Show::Picture { file } => {
                if !safe_name(file) {
                    return false;
                }
            }
        }
        self.from.sanitize();
        if let Some(to) = &mut self.to {
            to.sanitize();
        }
        self.colour &= 0xffffff;
        self.spin = if self.spin.is_finite() {
            self.spin.clamp(-MAX_SPIN, MAX_SPIN)
        } else {
            0.0
        };
        true
    }
}

/// A layer as it is at a moment: where, and how strongly seen (fades in).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerAt {
    pub pose: Pose,
    /// Pulses: how many times its size it is.
    pub scale: f64,
}

/// The shapes a lane can be bent into, by name: the Shapeshift mod's, in its order.
pub const LANE_SHAPES: [&str; 6] = ["zigzag", "uturn", "spiral", "loop", "elbow", "meadow"];

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
    /// For a `layer` clip: what it shows, and where.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<Box<Layer>>,
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
            Target::Lane(k) => (
                format!("lane.{}", LANE_SHAPES[k.min(LANE_SHAPES.len() - 1)]),
                0,
            ),
            Target::Notes(g, p) => (format!("notes.{}", p.name()), g.map_or(0, |g| g as u16 + 1)),
            Target::Layer => ("layer".to_string(), 0),
            Target::Copy(k, p) => (
                format!("copy.{}", p.name()),
                k.min(MAX_COPIES - 1) as u16 + 1,
            ),
        };
        let layer = (what == "layer").then(Box::default);
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
            layer,
        }
    }

    #[inline]
    pub fn target(&self) -> Option<Target> {
        if self.what == "layer" {
            return self.layer.is_some().then_some(Target::Layer);
        }
        if self.what == "shake" {
            return Some(Target::Shake);
        }
        if let Some(shape) = self.what.strip_prefix("lane.") {
            return LANE_SHAPES
                .iter()
                .position(|s| *s == shape)
                .map(Target::Lane);
        }
        if let Some(p) = self.what.strip_prefix("copy.") {
            let p = CopyProp::ALL.into_iter().find(|q| q.name() == p)?;
            let k = (self.group as usize).saturating_sub(1).min(MAX_COPIES - 1);
            return Some(Target::Copy(k, p));
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
    /// A layer clip's layer at `t`: none before its start, after its end (unless it keeps), or
    /// for a clip that isn't a layer. It fades in and out over its fades, moves from its start
    /// pose to its end pose by its easing, and, repeating, pulses by its amount (percent).
    pub fn layer_at(&self, t: f64, moments: &[f64], beat: impl Fn(f64) -> f64) -> Option<LayerAt> {
        let l = self.layer.as_deref()?;
        if t < self.start || (t > self.end && !self.keep) {
            return None;
        }
        let t = t.min(self.end);
        let b = beat(self.start).max(1.0);
        let (fin, fout) = (self.fade_in * b, self.fade_out * b);
        let mut seen = 1.0f64;
        if fin > 0.0 {
            seen = seen.min((t - self.start) / fin);
        }
        if fout > 0.0 && !self.keep {
            seen = seen.min((self.end - t) / fout);
        }
        let k = self
            .ease
            .apply((t - self.start) / (self.end - self.start).max(1.0));
        let mut pose = match &l.to {
            Some(to) => l.from.lerp(to, k),
            None => l.from,
        };
        if l.spin != 0.0 {
            pose.turn += l.spin * (t - self.start) / b;
        }
        pose.alpha *= seen.clamp(0.0, 1.0);
        let scale = if self.when == When::Span {
            1.0
        } else {
            (1.0 + self.value(t, moments, beat) / 100.0).max(0.0)
        };
        Some(LayerAt { pose, scale })
    }

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
        if let Some(l) = &mut self.layer
            && !l.sanitize()
        {
            return false;
        }
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
            Some(Target::Lane(_)) => 1.0,
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

/// A file name a map may give for a picture: inside its folder, nothing climbing out of it.
pub fn safe_name(name: &str) -> bool {
    let p = std::path::Path::new(name);
    !name.is_empty()
        && !p.is_absolute()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
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

    fn text_layer(start: f64, end: f64) -> Clip {
        let mut c = Clip {
            ease: Ease::Linear,
            fade_in: 2.0,
            fade_out: 2.0,
            ..Clip::new(Target::Layer, start, end)
        };
        let l = c.layer.as_deref_mut().unwrap();
        l.to = Some(Pose { x: 100.0, ..l.from });
        c
    }

    #[test]
    fn a_layer_fades_in_moves_to_its_end_pose_and_goes() {
        let c = text_layer(1000.0, 5000.0);
        assert_eq!(c.target(), Some(Target::Layer));
        let at = |t| c.layer_at(t, &[], beat);
        assert!(at(999.0).is_none());
        assert!(at(5001.0).is_none(), "gone after its end");
        let a = at(1500.0).unwrap();
        assert!((a.pose.alpha - 0.5).abs() < 1e-9, "half way in");
        let mid = at(3000.0).unwrap();
        assert_eq!(mid.pose.alpha, 1.0);
        assert!((mid.pose.x - 50.0).abs() < 1e-9, "half way to its end pose");
        assert_eq!(mid.scale, 1.0, "a layer that stays doesn't pulse");
        let kept = Clip {
            keep: true,
            ..c.clone()
        };
        let after = kept.layer_at(9000.0, &[], beat).unwrap();
        assert_eq!((after.pose.x, after.pose.alpha), (100.0, 1.0));
        // Repeating, it pulses by its amount in percent.
        let pulse = Clip {
            amount: 20.0,
            when: When::Beats { every: 1.0 },
            motion: Motion::Kick,
            fade_in: 0.0,
            ..c
        };
        let moments = [1000.0, 1500.0];
        let p = pulse.layer_at(1000.0, &moments, beat).unwrap();
        assert!((p.scale - 1.2).abs() < 1e-9);
        // Not a layer: nothing.
        assert!(
            clip(When::Span, Motion::Kick)
                .layer_at(3000.0, &[], beat)
                .is_none()
        );
    }

    #[test]
    fn a_spinning_layer_keeps_turning_by_its_spin_each_beat() {
        let mut c = text_layer(1000.0, 9000.0);
        let l = c.layer.as_deref_mut().unwrap();
        l.to = None;
        l.from.turn = 10.0;
        l.spin = 90.0;
        let turn = |t| c.layer_at(t, &[], beat).unwrap().pose.turn;
        assert_eq!(turn(1000.0), 10.0);
        assert!((turn(1000.0 + BEAT) - 100.0).abs() < 1e-9);
        assert!(
            (turn(1000.0 + 4.0 * BEAT) - 370.0).abs() < 1e-9,
            "past a whole turn"
        );
        // Saved only when it spins; maps from before read as still.
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"spin\":90"));
        c.layer.as_deref_mut().unwrap().spin = 0.0;
        assert!(!serde_json::to_string(&c).unwrap().contains("spin"));
        // Made safe like the rest.
        c.layer.as_deref_mut().unwrap().spin = f64::INFINITY;
        let mut list = vec![c];
        sanitize(&mut list);
        assert_eq!(list[0].layer.as_deref().unwrap().spin, 0.0);
    }

    #[test]
    fn layers_are_saved_and_what_a_file_says_is_made_safe() {
        let mut c = text_layer(0.0, 2000.0);
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"what\":\"layer\""));
        assert!(json.contains("\"kind\":\"text\""));
        let back: Clip = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        // Words without control characters, numbers in range.
        let l = c.layer.as_deref_mut().unwrap();
        l.show = Show::Text {
            text: format!("a\nb{}", "x".repeat(500)),
        };
        l.from.alpha = 7.0;
        l.from.size = f64::NAN;
        l.colour = 0xff123456;
        let mut list = vec![c.clone()];
        sanitize(&mut list);
        let l = list[0].layer.as_deref().unwrap();
        match &l.show {
            Show::Text { text } => {
                assert!(text.starts_with("ab"));
                assert_eq!(text.chars().count(), MAX_TEXT);
            }
            Show::Picture { .. } => panic!(),
        }
        assert_eq!((l.from.alpha, l.from.size, l.colour), (1.0, 36.0, 0x123456));
        // A picture outside the map's folder, or a layer clip without its layer, goes.
        let mut bad = c.clone();
        bad.layer.as_deref_mut().unwrap().show = Show::Picture {
            file: "../../secret.png".into(),
        };
        let mut bare = c;
        bare.layer = None;
        let mut list = vec![bad, bare];
        sanitize(&mut list);
        assert!(list.is_empty());
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
