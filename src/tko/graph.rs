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
    OnNoteReach => "Note arrives", Event, [], Some(Port::Trigger), [p("group", 1.0, 1.0, 256.0, 1.0)];
    OnHit => "Hit", Event, [], Some(Port::Trigger), [p("group", 0.0, 0.0, 256.0, 1.0), p("judge", 0.0, 0.0, 2.0, 1.0)];
    OnMiss => "Miss", Event, [], Some(Port::Trigger), [p("group", 0.0, 0.0, 256.0, 1.0)];
    OnCombo => "Combo reaches", Event, [], Some(Port::Trigger), [p("every", 50.0, 1.0, 1000.0, 10.0)];
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
    Combo => "Combo", Value, [], Some(Port::Number), [];
    Accuracy => "Accuracy", Value, [], Some(Port::Number), [];
    Random => "Random", Value, [], Some(Port::Number), [p("seed", 1.0, 0.0, 1_000_000.0, 1.0), p("beats", 1.0, 0.25, 64.0, 0.25)];
    Wave => "Wave", Value, [], Some(Port::Number), [p("beats", 4.0, 0.25, 64.0, 0.25), p("amount", 1.0, -1000.0, 1000.0, 0.5)];
    Toggle => "Flip-flop", Value, [Trigger "flip"], Some(Port::Number), [];
    Envelope => "Fade out", Value, [Trigger "in"], Some(Port::Number), [p("ms", 300.0, 1.0, 60_000.0, 50.0)];
    // Math.
    Add => "Add", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Mul => "Multiply", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Compare => "Greater than", Math, [Number "a", Number "b"], Some(Port::Number), [];
    Not => "Not", Math, [Number "a"], Some(Port::Number), [];
    Clamp => "Clamp", Math, [Number "a"], Some(Port::Number), [p("min", 0.0, -1000.0, 1000.0, 0.5), p("max", 1.0, -1000.0, 1000.0, 0.5)];
    Map => "Scale", Math, [Number "a"], Some(Port::Number), [p("times", 1.0, -1000.0, 1000.0, 0.5), p("plus", 0.0, -1000.0, 1000.0, 0.5)];
    // Actions: motion, added to the property's own keyframes.
    Drive => "Drive", Action, [Number "value"], None, [p("gain", 1.0, -1000.0, 1000.0, 0.5)];
    Pulse => "Pulse", Action, [Trigger "on"], None, [p("amount", 0.1, -1000.0, 1000.0, 0.05), p("ms", 300.0, 1.0, 60_000.0, 50.0)];
    Tween => "Tween", Action, [Trigger "flip"], None, [p("to", 1.0, -1000.0, 1000.0, 0.5), p("ms", 300.0, 1.0, 60_000.0, 50.0)];
    Shake => "Shake", Action, [Trigger "on"], None, [p("amount", 12.0, 0.0, 400.0, 2.0), p("ms", 300.0, 1.0, 60_000.0, 50.0)];
    Clip => "Play clip", Action, [Trigger "on"], None, [p("hold", 0.0, 0.0, 1.0, 1.0)];
}

impl Kind {
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
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
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
        assert_eq!(g.node(c).unwrap().p, [4.0, 1000.0]);
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
