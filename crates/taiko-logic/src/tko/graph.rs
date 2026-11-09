//! The node graph: events fire triggers, logic shapes them, values change over time, and actions
//! turn them into motion of the playfield or of groups of notes. It is data in the file (`graph`
//! in the `[Gimmick]` JSON) and evaluated by `runtime`, deterministically.
//!
//! Two kinds of wire: a *trigger* wire carries moments in time (a beat, a hit, a kiai start), a
//! *number* wire carries a value that can be read at any time. An action never changes when a note
//! is hit or how it is judged: like the keyframes, it only moves what is drawn.
//!
//! Hard caps keep a file from hanging the player: nodes, triggers per node, evaluation steps per
//! frame. Wires that would make a loop are dropped when a file is read.

use serde::{Deserialize, Serialize};

use super::gimmick::{FieldProp, NoteProp};
use super::track::{Ease, Track};

pub const MAX_NODES: usize = 2000;
pub const MAX_TRIGGERS_PER_NODE: usize = 20_000;
/// Evaluation steps (nodes visited) a frame may spend.
pub const MAX_STEPS: usize = 10_000;
pub const MAX_PARAMS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    /// Moments in time.
    Trigger,
    /// A value over time.
    Number,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Event,
    Logic,
    Value,
    Math,
    Action,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Event,
        Category::Logic,
        Category::Value,
        Category::Math,
        Category::Action,
    ];

    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Category::Event => "Events",
            Category::Logic => "Triggers",
            Category::Value => "Values",
            Category::Math => "Math",
            Category::Action => "Actions",
        }
    }
}

/// A number a node takes: what it is called, where it starts, what it may be and how far a click
/// on - or + moves it.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

const fn p(name: &'static str, default: f64, min: f64, max: f64, step: f64) -> Param {
    Param {
        name,
        default,
        min,
        max,
        step,
    }
}

/// What an action drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Target {
    Field(FieldProp),
    Group(usize, NoteProp),
}

impl Target {
    #[inline]
    pub fn label(self) -> String {
        match self {
            Target::Field(p) => format!("field {}", p.name()),
            Target::Group(g, p) => format!("group {} {}", g + 1, p.name()),
        }
    }

    pub fn range(self) -> (f64, f64) {
        match self {
            Target::Field(p) => p.range(),
            Target::Group(_, p) => p.range(),
        }
    }
}

impl From<Target> for String {
    fn from(t: Target) -> String {
        match t {
            Target::Field(p) => format!("field.{}", p.name()),
            Target::Group(g, p) => format!("group.{g}.{}", p.name()),
        }
    }
}

impl TryFrom<String> for Target {
    type Error = String;

    fn try_from(s: String) -> Result<Target, String> {
        let mut it = s.split('.');
        match (it.next(), it.next(), it.next(), it.next()) {
            (Some("field"), Some(p), None, None) => FieldProp::ALL
                .iter()
                .find(|q| q.name() == p)
                .map(|q| Target::Field(*q))
                .ok_or_else(|| format!("no playfield property {p}")),
            (Some("group"), Some(g), Some(p), None) => {
                let g: usize = g.parse().map_err(|_| format!("bad group {g}"))?;
                NoteProp::ALL
                    .iter()
                    .find(|q| q.name() == p)
                    .map(|q| Target::Group(g, *q))
                    .ok_or_else(|| format!("no note property {p}"))
            }
            _ => Err(format!("bad target {s}")),
        }
    }
}

macro_rules! kinds {
    ($($v:ident => $name:literal, $cat:ident, [$($in:ident $inname:literal),*], $out:expr, [$($par:expr),*];)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum Kind { $($v),* }

        impl Kind {
            pub const ALL: &'static [Kind] = &[$(Kind::$v),*];

            #[inline]
            pub fn label(self) -> &'static str {
                match self { $(Kind::$v => $name),* }
            }

            pub fn category(self) -> Category {
                match self { $(Kind::$v => Category::$cat),* }
            }

            /// The input ports, in order.
            pub fn inputs(self) -> &'static [(&'static str, Port)] {
                match self { $(Kind::$v => &[$(($inname, Port::$in)),*]),* }
            }

            /// What the node gives out (an action gives nothing).
            pub fn output(self) -> Option<Port> {
                match self { $(Kind::$v => $out),* }
            }

            pub fn params(self) -> &'static [Param] {
                match self { $(Kind::$v => { const PARAMS: &[Param] = &[$($par),*]; PARAMS }),* }
            }
        }
    };
}

kinds! {
    // Events: moments in time.
    OnStart => "Start", Event, [], Some(Port::Trigger), [];
    OnTime => "At time", Event, [], Some(Port::Trigger), [p("ms", 1000.0, 0.0, 3_600_000.0, 500.0)];
    OnBeat => "Every beat", Event, [], Some(Port::Trigger), [p("every", 1.0, 1.0, 64.0, 1.0), p("offset", 0.0, 0.0, 63.0, 1.0)];
    OnKiaiStart => "Kiai starts", Event, [], Some(Port::Trigger), [];
    OnKiaiEnd => "Kiai ends", Event, [], Some(Port::Trigger), [];
    OnNoteReach => "Note arrives", Event, [], Some(Port::Trigger), [p("group", 0.0, 0.0, 256.0, 1.0), p("type", 0.0, 0.0, 4.0, 1.0), p("nth", 0.0, 0.0, 1000.0, 1.0)];
    OnHit => "Hit", Event, [], Some(Port::Trigger), [p("group", 0.0, 0.0, 256.0, 1.0), p("judge", 0.0, 0.0, 2.0, 1.0), p("type", 0.0, 0.0, 4.0, 1.0)];
    OnMiss => "Miss", Event, [], Some(Port::Trigger), [p("group", 0.0, 0.0, 256.0, 1.0), p("type", 0.0, 0.0, 4.0, 1.0)];
    OnCombo => "Combo reaches", Event, [], Some(Port::Trigger), [p("every", 50.0, 1.0, 1000.0, 10.0)];
    OnMark => "At mark", Event, [], Some(Port::Trigger), [p("mark", 0.0, 0.0, 256.0, 1.0)];
    // Triggers: reshape moments.
    Delay => "Delay", Logic, [Trigger "in"], Some(Port::Trigger), [p("ms", 250.0, 0.0, 60_000.0, 50.0)];
    Counter => "Every Nth", Logic, [Trigger "in"], Some(Port::Trigger), [p("n", 2.0, 1.0, 1000.0, 1.0)];
    Once => "Only first", Logic, [Trigger "in"], Some(Port::Trigger), [];
    Gate => "Gate", Logic, [Trigger "in", Number "open"], Some(Port::Trigger), [];
    Merge => "Either", Logic, [Trigger "a", Trigger "b"], Some(Port::Trigger), [];
    // Values: a number over time.
    Const => "Number", Value, [], Some(Port::Number), [p("value", 1.0, -1000.0, 1000.0, 0.5)];
    Time => "Seconds", Value, [], Some(Port::Number), [];
    BeatPhase => "Beat phase", Value, [], Some(Port::Number), [];
    Beats => "Beat count", Value, [], Some(Port::Number), [];
    Kiai => "In kiai", Value, [], Some(Port::Number), [];
    Combo => "Combo", Value, [], Some(Port::Number), [];
    Accuracy => "Accuracy", Value, [], Some(Port::Number), [];
    Random => "Random", Value, [], Some(Port::Number), [p("seed", 1.0, 0.0, 1_000_000.0, 1.0), p("beats", 1.0, 0.25, 64.0, 0.25)];
    Noise => "Smooth noise", Value, [], Some(Port::Number), [p("beats", 1.0, 0.25, 64.0, 0.25), p("seed", 1.0, 0.0, 1_000_000.0, 1.0)];
    Wave => "Wave", Value, [], Some(Port::Number), [p("beats", 4.0, 0.25, 64.0, 0.25), p("amount", 1.0, -1000.0, 1000.0, 0.5), p("shape", 0.0, 0.0, 3.0, 1.0)];
    Toggle => "Flip-flop", Value, [Trigger "flip"], Some(Port::Number), [];
    Envelope => "Fade out", Value, [Trigger "in"], Some(Port::Number), [p("ms", 300.0, 1.0, 60_000.0, 50.0)];
    Count => "Count", Value, [Trigger "in"], Some(Port::Number), [];
    Progress => "Progress", Value, [Trigger "in"], Some(Port::Number), [];
    Between => "Between marks", Value, [], Some(Port::Number), [p("from", 1.0, 0.0, 256.0, 1.0), p("to", 2.0, 0.0, 256.0, 1.0)];
    Span => "Span progress", Value, [], Some(Port::Number), [p("from", 1.0, 0.0, 256.0, 1.0), p("to", 2.0, 0.0, 256.0, 1.0)];
    // Math.
    Add => "Add", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Sub => "Subtract", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Mul => "Multiply", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Compare => "Greater than", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Not => "Not", Math, [Number "a"], Some(Port::Number), [];
    Abs => "Absolute", Math, [Number "a"], Some(Port::Number), [];
    Min => "Smaller", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Max => "Larger", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Mix => "Blend", Math, [Number "a", Number "b", Number "by"], Some(Port::Number), [];
    Pow => "Power", Math, [Number "a"], Some(Port::Number), [p("exp", 2.0, -8.0, 8.0, 0.5)];
    Steps => "Steps", Math, [Number "a"], Some(Port::Number), [p("steps", 4.0, 1.0, 256.0, 1.0)];
    Clamp => "Clamp", Math, [Number "a"], Some(Port::Number), [p("min", 0.0, -1000.0, 1000.0, 0.5), p("max", 1.0, -1000.0, 1000.0, 0.5)];
    Map => "Scale", Math, [Number "a"], Some(Port::Number), [p("times", 1.0, -1000.0, 1000.0, 0.5), p("plus", 0.0, -1000.0, 1000.0, 0.5)];
    // Actions: motion, added to the property's own keyframes.
    Drive => "Drive", Action, [Number "value"], None, [p("gain", 1.0, -1000.0, 1000.0, 0.5)];
    Pulse => "Pulse", Action, [Trigger "on", Number "scale"], None, [p("amount", 0.1, -1000.0, 1000.0, 0.05), p("ms", 300.0, 1.0, 60_000.0, 50.0)];
    Tween => "Tween", Action, [Trigger "flip", Number "scale"], None, [p("to", 1.0, -1000.0, 1000.0, 0.5), p("ms", 300.0, 1.0, 60_000.0, 50.0)];
    Shake => "Shake", Action, [Trigger "on", Number "scale"], None, [p("amount", 12.0, 0.0, 400.0, 2.0), p("ms", 300.0, 1.0, 60_000.0, 50.0)];
    Clip => "Play clip", Action, [Trigger "on", Number "scale"], None, [p("hold", 0.0, 0.0, 1.0, 1.0)];
}

impl Kind {
    /// What the node does, in a sentence or two, for the editor's palette.
    pub fn about(self) -> &'static str {
        match self {
            Kind::OnStart => "Fires once, when the map starts.",
            Kind::OnTime => {
                "Fires once, at this many milliseconds into the map. Prefer marks and notes: a time stops matching the moment the chart changes."
            }
            Kind::OnBeat => {
                "Fires on the beat grid: every N beats, starting this many beats in. The grid follows the red lines."
            }
            Kind::OnKiaiStart => "Fires each time kiai time begins.",
            Kind::OnKiaiEnd => "Fires each time kiai time ends.",
            Kind::OnNoteReach => {
                "Fires when a note arrives at the hit circle. Group 0 is every note, or pick a group; type narrows it to dons, kats, finishers or rolls; nth picks only the Nth of those (0 is all of them)."
            }
            Kind::OnHit => {
                "Fires when a note is hit. Group 0 is any note; judge 0 is any hit, 1 only great, 2 only ok; type narrows it to dons, kats, finishers or rolls."
            }
            Kind::OnMiss => {
                "Fires when a note is missed. Group 0 is any note; type narrows it to dons, kats, finishers or rolls."
            }
            Kind::OnCombo => "Fires each time the combo reaches a multiple of this number.",
            Kind::OnMark => {
                "Fires at a mark you placed in the editor (M). Mark 0 fires at every mark, 1 at the first, and so on."
            }
            Kind::Delay => "Passes each moment on after a pause.",
            Kind::Counter => "Passes on every Nth moment and swallows the rest.",
            Kind::Once => "Passes on only the first moment.",
            Kind::Gate => "Passes moments on only while the number wired in is at least 0.5.",
            Kind::Merge => "Fires when either input fires.",
            Kind::Const => "A fixed number.",
            Kind::Time => "Seconds since the map started.",
            Kind::BeatPhase => "Climbs from 0 to 1 through each beat, then starts again.",
            Kind::Beats => "How many beats of the map's grid have passed, with the fraction.",
            Kind::Kiai => "1 during kiai time, 0 otherwise.",
            Kind::Combo => "The player's current combo.",
            Kind::Accuracy => "The player's accuracy so far, from 0 to 1.",
            Kind::Random => {
                "A number that changes every few beats. It comes from the map's seed, so replays match."
            }
            Kind::Noise => {
                "A number from 0 to 1 that drifts smoothly, picking a new target every few beats (from the map's seed, so replays match)."
            }
            Kind::Wave => {
                "Swings between minus and plus the amount, once per this many beats. Shape 0 is a sine, 1 a triangle, 2 a saw, 3 a square."
            }
            Kind::Toggle => "Flips between 0 and 1 at each moment.",
            Kind::Envelope => "Jumps to 1 at each moment and falls to 0 over this time.",
            Kind::Count => "How many moments have happened so far: 0, then 1, 2, 3 ...",
            Kind::Progress => {
                "How far through all its moments the trigger is: 0 before the first, 1 after the last, in even steps between. Wire it to a scale input for a build-up."
            }
            Kind::Between => {
                "1 between two marks, 0 outside. Mark 0 stands for the start of the map (from) or its end (to)."
            }
            Kind::Span => {
                "Climbs from 0 to 1 between two marks and stays there. Mark 0 stands for the start of the map (from) or its end (to)."
            }
            Kind::Add => "a plus b.",
            Kind::Sub => "a minus b.",
            Kind::Mul => "a times b.",
            Kind::Compare => "1 while a is greater than b, else 0.",
            Kind::Not => "1 when a is 0, else 0.",
            Kind::Abs => "a without its sign.",
            Kind::Min => "The smaller of a and b.",
            Kind::Max => "The larger of a and b.",
            Kind::Mix => "Slides from a to b as the third input goes from 0 to 1.",
            Kind::Pow => "a to this power (a below 0 gives 0). Bends a 0 to 1 number into a curve.",
            Kind::Steps => "Rounds a down to this many even steps between 0 and 1.",
            Kind::Clamp => "Keeps a between min and max.",
            Kind::Map => "a times this, plus that.",
            Kind::Drive => {
                "Holds the target at this number times the gain, all the time. Added to its keyframes."
            }
            Kind::Pulse => {
                "Kicks the target by an amount at each moment, then lets it settle back. The scale input multiplies the kick, read at the moment it happens."
            }
            Kind::Tween => {
                "Moves the target to a value and back, one way at each moment. The scale input multiplies the value, read at the moment it happens."
            }
            Kind::Shake => {
                "Jolts the whole playfield in random directions at each moment, fading out. The scale input multiplies the strength, read at the moment it happens."
            }
            Kind::Clip => {
                "Plays a short ready-made run of keyframes at each moment. The scale input multiplies it, read at the moment it happens."
            }
        }
    }

    /// Whether the node drives a property.
    pub fn is_action(self) -> bool {
        self.category() == Category::Action
    }

    /// An action that moves something named by its target (Shake moves the playfield on its own).
    pub fn needs_target(self) -> bool {
        self.is_action() && self != Kind::Shake
    }

    pub fn uses_ease(self) -> bool {
        matches!(self, Kind::Envelope | Kind::Pulse | Kind::Tween)
    }

    /// Fed by what the player does, so its moments are not known until they happen.
    pub fn is_play_event(self) -> bool {
        matches!(self, Kind::OnHit | Kind::OnMiss | Kind::OnCombo)
    }

    pub fn reads_play_state(self) -> bool {
        matches!(self, Kind::Combo | Kind::Accuracy)
    }
}

/// A preset for `Kind::Clip`: keys relative to the moment it is triggered.
pub fn clip_preset(n: usize) -> (&'static str, Track) {
    use super::track::Key;
    let k = |t: f64, v: f64, ease: Ease| Key { t, v, ease };
    match n % 3 {
        0 => (
            "kick",
            Track {
                keys: vec![
                    k(0.0, 0.0, Ease::QuadOut),
                    k(120.0, 1.0, Ease::QuadOut),
                    k(420.0, 0.0, Ease::Linear),
                ],
            },
        ),
        1 => (
            "swing",
            Track {
                keys: vec![
                    k(0.0, 0.0, Ease::SineInOut),
                    k(200.0, 1.0, Ease::SineInOut),
                    k(600.0, -1.0, Ease::SineInOut),
                    k(900.0, 0.0, Ease::Linear),
                ],
            },
        ),
        _ => (
            "pop",
            Track {
                keys: vec![k(0.0, 0.0, Ease::BackOut), k(250.0, 1.0, Ease::Linear)],
            },
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: u32,
    pub kind: Kind,
    /// Where it sits on the editor's canvas.
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub p: Vec<f64>,
    #[serde(default)]
    pub ease: Ease,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    /// For each input port, the node feeding it.
    #[serde(default)]
    pub inputs: Vec<Option<u32>>,
    #[serde(default, skip_serializing_if = "Track::is_empty")]
    pub clip: Track,
}

impl Node {
    #[inline]
    pub fn new(id: u32, kind: Kind, x: f32, y: f32) -> Node {
        let mut n = Node {
            id,
            kind,
            x,
            y,
            p: kind.params().iter().map(|q| q.default).collect(),
            ease: if kind == Kind::Pulse || kind == Kind::Envelope {
                Ease::QuadOut
            } else {
                Ease::Linear
            },
            target: None,
            inputs: vec![None; kind.inputs().len()],
            clip: Track::default(),
        };
        if kind == Kind::Clip {
            n.clip = clip_preset(0).1;
        }
        n
    }

    /// Parameter `i`, its default when the file left it out.
    pub fn param(&self, i: usize) -> f64 {
        self.p
            .get(i)
            .copied()
            .or_else(|| self.kind.params().get(i).map(|q| q.default))
            .unwrap_or(0.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default)]
    pub nodes: Vec<Node>,
    /// Randomness comes from here, so a replay looks the same.
    #[serde(default)]
    pub seed: u64,
}

impl Graph {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Why node `id` can't do anything yet, in words for the editor; None when it is fine.
    /// A target that is missing, a first input nothing feeds, an output nothing uses.
    pub fn problem(&self, id: u32) -> Option<String> {
        let n = self.node(id)?;
        if n.kind.needs_target() && n.target.is_none() {
            return Some("No target yet: pick what it moves in the list on the right.".into());
        }
        let wired = |port: usize| n.inputs.get(port).copied().flatten().is_some();
        let inputs = n.kind.inputs();
        if n.kind == Kind::Merge {
            if !wired(0) && !wired(1) {
                return Some("Nothing comes in: wire an event into 'a' or 'b'.".into());
            }
        } else if let Some((name, port)) = inputs.first()
            && !wired(0)
        {
            return Some(match port {
                Port::Trigger => {
                    format!("Nothing sets it off: wire an event (orange dot) into '{name}'.")
                }
                Port::Number => {
                    format!("Nothing feeds it: wire a number (cyan dot) into '{name}'.")
                }
            });
        }
        if n.kind.output().is_some() && !self.nodes.iter().any(|m| m.inputs.contains(&Some(id))) {
            return Some("Nothing uses its output: wire it into another node.".into());
        }
        None
    }

    /// How many nodes can't do anything yet.
    pub fn problem_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| self.problem(n.id).is_some())
            .count()
    }

    /// The first steps of a first effect, each with whether the graph has got that far.
    pub fn first_steps(&self) -> [(bool, &'static str); 4] {
        let has = |f: &dyn Fn(&Node) -> bool| self.nodes.iter().any(f);
        let event = has(&|n| n.kind.category() == Category::Event);
        let action = has(&|n| n.kind.is_action());
        let wired = has(&|n| {
            n.kind.is_action()
                && n.inputs.first().copied().flatten().is_some_and(|src| {
                    self.node(src)
                        .is_some_and(|s| s.kind.category() == Category::Event)
                })
        });
        let aimed = has(&|n| n.kind.is_action() && (!n.kind.needs_target() || n.target.is_some()));
        [
            (event, "Add an event: click \"+ Every beat\" on the right"),
            (action, "Add an action: click \"+ Pulse\""),
            (
                wired,
                "Wire them: drag from the event's right dot to the action's \"on\" dot",
            ),
            (
                aimed,
                "Pick the action's target, for example Zoom, in the list on the right",
            ),
        ]
    }

    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u32) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Adds a node with its defaults. Returns its id.
    pub fn add(&mut self, kind: Kind, x: f32, y: f32) -> Option<u32> {
        if self.nodes.len() >= MAX_NODES {
            return None;
        }
        let id = self.nodes.iter().map(|n| n.id).max().map_or(1, |m| m + 1);
        self.nodes.push(Node::new(id, kind, x, y));
        Some(id)
    }

    /// Removes a node and every wire from it.
    pub fn remove(&mut self, id: u32) {
        self.nodes.retain(|n| n.id != id);
        for n in &mut self.nodes {
            for w in &mut n.inputs {
                if *w == Some(id) {
                    *w = None;
                }
            }
        }
    }

    /// Whether wiring `from` into `to` would make a loop.
    pub fn would_loop(&self, from: u32, to: u32) -> bool {
        // A loop if `from` already (through its inputs) depends on `to`.
        let mut stack = vec![from];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if id == to {
                return true;
            }
            if !seen.insert(id) {
                continue;
            }
            if let Some(n) = self.node(id) {
                stack.extend(n.inputs.iter().flatten().copied());
            }
        }
        false
    }

    /// Wires `from`'s output into input `port` of `to`, if the types fit and it makes no loop.
    pub fn connect(&mut self, from: u32, to: u32, port: usize) -> Result<(), String> {
        let out = self
            .node(from)
            .and_then(|n| n.kind.output())
            .ok_or("that node has no output")?;
        let want = self
            .node(to)
            .and_then(|n| n.kind.inputs().get(port))
            .map(|(_, t)| *t)
            .ok_or("no such input")?;
        if out != want {
            return Err(match want {
                Port::Trigger => "that input takes a trigger, not a number".into(),
                Port::Number => "that input takes a number, not a trigger".into(),
            });
        }
        if from == to || self.would_loop(from, to) {
            return Err("that would make a loop".into());
        }
        if let Some(n) = self.node_mut(to) {
            n.inputs.resize(n.kind.inputs().len(), None);
            n.inputs[port] = Some(from);
        }
        Ok(())
    }

    /// Makes what a file said safe to run: caps, unique ids, parameters and wires that fit their
    /// kinds, no loops.
    pub fn sanitize(&mut self, groups: usize) {
        self.nodes.truncate(MAX_NODES);
        let mut seen = std::collections::HashSet::new();
        let mut next = self.nodes.iter().map(|n| n.id).max().unwrap_or(0);
        for n in &mut self.nodes {
            if !seen.insert(n.id) {
                next += 1;
                n.id = next;
                seen.insert(n.id);
            }
            if !n.x.is_finite() || !n.y.is_finite() {
                n.x = 0.0;
                n.y = 0.0;
            }
            n.x = n.x.clamp(-100_000.0, 100_000.0);
            n.y = n.y.clamp(-100_000.0, 100_000.0);
            let defs = n.kind.params();
            n.p.truncate(MAX_PARAMS);
            for (i, d) in defs.iter().enumerate() {
                let v =
                    n.p.get(i)
                        .copied()
                        .filter(|v| v.is_finite())
                        .unwrap_or(d.default);
                let v = v.clamp(d.min, d.max);
                if i < n.p.len() {
                    n.p[i] = v;
                } else {
                    n.p.push(v);
                }
            }
            n.p.truncate(defs.len());
            n.inputs.resize(n.kind.inputs().len(), None);
            n.clip.sanitize();
            if !n.kind.needs_target() {
                n.target = None;
            }
            if let Some(Target::Group(g, _)) = n.target
                && g >= groups
            {
                n.target = None;
            }
            if n.kind != Kind::Clip {
                n.clip = Track::default();
            }
        }
        // Wires: the source must exist, give the right kind of thing, and not be the node itself.
        let outputs: std::collections::HashMap<u32, Option<Port>> =
            self.nodes.iter().map(|n| (n.id, n.kind.output())).collect();
        for n in &mut self.nodes {
            let id = n.id;
            for (w, (_, want)) in n.inputs.iter_mut().zip(n.kind.inputs()) {
                if let Some(from) = *w
                    && (from == id || outputs.get(&from).copied().flatten() != Some(*want))
                {
                    *w = None;
                }
            }
        }
        // Loops: walk from each node along its inputs; a wire back into the walk is dropped.
        let order: Vec<u32> = self.nodes.iter().map(|n| n.id).collect();
        let mut state: std::collections::HashMap<u32, u8> = std::collections::HashMap::new();
        for start in order {
            self.break_loops(start, &mut state);
        }
    }

    fn break_loops(&mut self, id: u32, state: &mut std::collections::HashMap<u32, u8>) {
        // Iterative depth-first search (a long chain mustn't overflow the stack): state 1 is on
        // the path, 2 is done.
        let mut path: Vec<(u32, usize)> = vec![(id, 0)];
        if state.get(&id).copied().unwrap_or(0) != 0 {
            return;
        }
        state.insert(id, 1);
        while let Some(&(cur, i)) = path.last() {
            let Some(at) = self.nodes.iter().position(|n| n.id == cur) else {
                path.pop();
                continue;
            };
            if i >= self.nodes[at].inputs.len() {
                state.insert(cur, 2);
                path.pop();
                continue;
            }
            if let Some(top) = path.last_mut() {
                top.1 += 1;
            }
            let Some(from) = self.nodes[at].inputs[i] else {
                continue;
            };
            match state.get(&from).copied().unwrap_or(0) {
                1 => self.nodes[at].inputs[i] = None,
                0 if self.nodes.iter().any(|n| n.id == from) => {
                    state.insert(from, 1);
                    path.push((from, 0));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nodes_that_do_nothing_say_why() {
        let mut g = Graph::default();
        let beat = g.add(Kind::OnBeat, 0.0, 0.0).unwrap();
        let pulse = g.add(Kind::Pulse, 100.0, 0.0).unwrap();
        // The event goes nowhere; the action has no event and no target.
        assert!(g.problem(beat).unwrap().contains("Nothing uses"));
        assert!(g.problem(pulse).unwrap().contains("No target"));
        g.node_mut(pulse).unwrap().target = Some(Target::Field(FieldProp::Zoom));
        assert!(g.problem(pulse).unwrap().contains("Nothing sets it off"));
        g.connect(beat, pulse, 0).unwrap();
        assert_eq!((g.problem(beat), g.problem(pulse)), (None, None));
        assert_eq!(g.problem_count(), 0);
        // A drive wants a number.
        let drive = g.add(Kind::Drive, 0.0, 80.0).unwrap();
        g.node_mut(drive).unwrap().target = Some(Target::Field(FieldProp::Zoom));
        assert!(g.problem(drive).unwrap().contains("Nothing feeds it"));
        // Either input of "Either" will do.
        let merge = g.add(Kind::Merge, 0.0, 160.0).unwrap();
        assert!(g.problem(merge).unwrap().contains("Nothing comes in"));
        assert_eq!(g.problem(999), None);
    }

    #[test]
    fn the_first_steps_tick_off_as_the_graph_grows() {
        let mut g = Graph::default();
        let done = |g: &Graph| g.first_steps().iter().filter(|s| s.0).count();
        assert_eq!(done(&g), 0);
        let beat = g.add(Kind::OnBeat, 0.0, 0.0).unwrap();
        assert_eq!(done(&g), 1);
        let pulse = g.add(Kind::Pulse, 100.0, 0.0).unwrap();
        assert_eq!(done(&g), 2);
        g.connect(beat, pulse, 0).unwrap();
        assert_eq!(done(&g), 3);
        g.node_mut(pulse).unwrap().target = Some(Target::Field(FieldProp::Zoom));
        assert_eq!(done(&g), 4);
    }

    use super::*;

    #[test]
    fn every_kind_has_consistent_ports_and_params() {
        for &k in Kind::ALL {
            assert!(k.params().len() <= MAX_PARAMS, "{k:?}");
            for d in k.params() {
                assert!(d.min <= d.default && d.default <= d.max, "{k:?} {}", d.name);
            }
            if k.is_action() {
                assert!(k.output().is_none(), "{k:?}");
            } else {
                assert!(k.output().is_some(), "{k:?}");
            }
            if k.category() == Category::Event {
                assert!(k.inputs().is_empty());
            }
        }
    }

    #[test]
    fn a_wire_must_fit_and_may_not_make_a_loop() {
        let mut g = Graph::default();
        let beat = g.add(Kind::OnBeat, 0.0, 0.0).unwrap();
        let delay = g.add(Kind::Delay, 100.0, 0.0).unwrap();
        let wave = g.add(Kind::Wave, 0.0, 50.0).unwrap();
        let pulse = g.add(Kind::Pulse, 200.0, 0.0).unwrap();
        assert!(g.connect(beat, delay, 0).is_ok());
        assert!(g.connect(delay, pulse, 0).is_ok());
        assert!(g.connect(wave, pulse, 0).unwrap_err().contains("trigger"));
        assert!(g.connect(beat, wave, 0).is_err(), "a wave has no input");
        assert!(
            g.connect(pulse, delay, 0)
                .unwrap_err()
                .contains("no output")
        );
        let counter = g.add(Kind::Counter, 0.0, 0.0).unwrap();
        g.connect(delay, counter, 0).unwrap();
        assert!(g.connect(counter, delay, 0).unwrap_err().contains("loop"));
        assert!(g.connect(delay, delay, 0).unwrap_err().contains("loop"));
    }

    #[test]
    fn removing_a_node_unwires_what_it_fed() {
        let mut g = Graph::default();
        let a = g.add(Kind::OnStart, 0.0, 0.0).unwrap();
        let b = g.add(Kind::Once, 0.0, 0.0).unwrap();
        g.connect(a, b, 0).unwrap();
        g.remove(a);
        assert_eq!(g.node(b).unwrap().inputs, [None]);
        assert!(g.node(a).is_none());
    }

    #[test]
    fn sanitizing_drops_loops_bad_wires_and_out_of_range_values() {
        let mut g = Graph::default();
        let a = g.add(Kind::Delay, 0.0, 0.0).unwrap();
        let b = g.add(Kind::Delay, 0.0, 0.0).unwrap();
        let c = g.add(Kind::Wave, 0.0, 0.0).unwrap();
        // A loop and a number wired where a trigger goes, written by hand.
        g.node_mut(a).unwrap().inputs = vec![Some(b)];
        g.node_mut(b).unwrap().inputs = vec![Some(a)];
        let d = g.add(Kind::Once, 0.0, 0.0).unwrap();
        g.node_mut(d).unwrap().inputs = vec![Some(c)];
        g.node_mut(c).unwrap().p = vec![f64::NAN, 1e12];
        g.node_mut(a).unwrap().x = f32::INFINITY;
        g.nodes.push(Node {
            id: a,
            ..Node::new(a, Kind::OnStart, 0.0, 0.0)
        });
        g.sanitize(0);
        let ids: std::collections::HashSet<u32> = g.nodes.iter().map(|n| n.id).collect();
        assert_eq!(ids.len(), g.nodes.len(), "ids are unique");
        let (na, nb) = (g.node(a).unwrap(), g.node(b).unwrap());
        assert!(
            na.inputs[0].is_none() || nb.inputs[0].is_none(),
            "the loop is broken"
        );
        assert_eq!(
            g.node(d).unwrap().inputs,
            [None],
            "a number can't feed a trigger"
        );
        assert_eq!(g.node(c).unwrap().p, [4.0, 1000.0, 0.0]);
        assert_eq!(na.x, 0.0);
    }

    #[test]
    fn a_long_chain_is_checked_without_overflowing_the_stack() {
        let mut g = Graph::default();
        let mut prev = g.add(Kind::OnStart, 0.0, 0.0).unwrap();
        for _ in 0..1500 {
            let n = g.add(Kind::Once, 0.0, 0.0).unwrap();
            g.connect(prev, n, 0).unwrap();
            prev = n;
        }
        // And back to the start: a loop 1500 long.
        let first = g.nodes[0].id;
        g.node_mut(first).unwrap().kind = Kind::Once;
        g.node_mut(first).unwrap().inputs = vec![Some(prev)];
        g.sanitize(0);
        assert!(
            g.nodes[0].inputs[0].is_none()
                || g.nodes
                    .iter()
                    .any(|n| n.inputs[0].is_none() && n.id != first)
        );
    }

    #[test]
    fn the_node_cap_holds() {
        let mut g = Graph::default();
        for _ in 0..MAX_NODES {
            assert!(g.add(Kind::OnStart, 0.0, 0.0).is_some());
        }
        assert!(g.add(Kind::OnStart, 0.0, 0.0).is_none());
        for _ in 0..10 {
            g.nodes.push(Node::new(99_999, Kind::OnStart, 0.0, 0.0));
        }
        g.sanitize(0);
        assert_eq!(g.nodes.len(), MAX_NODES);
    }

    #[test]
    fn targets_round_trip_as_text_and_bad_ones_are_errors() {
        for t in [
            Target::Field(FieldProp::Zoom),
            Target::Group(3, NoteProp::Opacity),
        ] {
            let s: String = t.into();
            assert_eq!(Target::try_from(s).unwrap(), t);
        }
        assert!(Target::try_from("field.nope".to_string()).is_err());
        assert!(Target::try_from("group.x.dx".to_string()).is_err());
        assert!(Target::try_from("junk".to_string()).is_err());
    }

    #[test]
    fn a_graph_round_trips_through_json() {
        let mut g = Graph::default();
        let beat = g.add(Kind::OnBeat, 10.0, 20.0).unwrap();
        let pulse = g.add(Kind::Pulse, 200.0, 20.0).unwrap();
        g.connect(beat, pulse, 0).unwrap();
        g.node_mut(pulse).unwrap().target = Some(Target::Field(FieldProp::Zoom));
        let json = serde_json::to_string(&g).unwrap();
        assert!(json.contains("\"field.zoom\"") && json.contains("on_beat"));
        assert_eq!(serde_json::from_str::<Graph>(&json).unwrap(), g);
    }
}
