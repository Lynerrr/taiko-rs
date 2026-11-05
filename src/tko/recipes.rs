//! Ready-made node groups: a few wired nodes that do one familiar thing, dropped into a graph in
//! one click. The editor's Recipe menu and the showcase generator both build from these, so what a
//! demo does is what anyone can add to their own map.

use super::gimmick::FieldProp;
use super::graph::{Graph, Kind, MAX_NODES, Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipe {
    /// The playfield breathes on every beat.
    BeatPulse,
    /// A flash, a jolt and a zoom the moment kiai starts.
    KiaiDrop,
    /// Every finisher hits harder than the one before.
    FinisherBuild,
    /// The playfield grows with the player's combo.
    ComboZoom,
    /// A miss tears the picture and shakes it.
    MissGlitch,
    /// Kiai leaves echo trails that kick on each beat.
    EchoTrail,
    /// The playfield turns once from mark 1 to mark 2.
    SectionSweep,
}

impl Recipe {
    pub const ALL: [Recipe; 7] = [
        Recipe::BeatPulse,
        Recipe::KiaiDrop,
        Recipe::FinisherBuild,
        Recipe::ComboZoom,
        Recipe::MissGlitch,
        Recipe::EchoTrail,
        Recipe::SectionSweep,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Recipe::BeatPulse => "Beat pulse",
            Recipe::KiaiDrop => "Kiai drop",
            Recipe::FinisherBuild => "Finisher build-up",
            Recipe::ComboZoom => "Combo zoom",
            Recipe::MissGlitch => "Miss glitch",
            Recipe::EchoTrail => "Kiai echo trail",
            Recipe::SectionSweep => "Section sweep",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Recipe::BeatPulse => "The playfield breathes: a small zoom kick on every beat.",
            Recipe::KiaiDrop => {
                "The moment kiai starts: a white flash, a zoom kick and a hard shake."
            }
            Recipe::FinisherBuild => {
                "Every finisher kicks the zoom, flashes and shakes, each harder than the one before (progress through the finishers scales them)."
            }
            Recipe::ComboZoom => {
                "The playfield grows as the combo climbs and falls back on a miss."
            }
            Recipe::MissGlitch => "A miss tears the picture into slices and shakes it.",
            Recipe::EchoTrail => {
                "In kiai the playfield leaves 4 fading echoes, and each beat flicks them round."
            }
            Recipe::SectionSweep => {
                "The playfield turns once, from mark 1 to mark 2 (place the marks with M)."
            }
        }
    }

    fn size(self) -> usize {
        match self {
            Recipe::BeatPulse => 2,
            Recipe::KiaiDrop => 4,
            Recipe::FinisherBuild => 5,
            Recipe::ComboZoom => 3,
            Recipe::MissGlitch => 3,
            Recipe::EchoTrail => 4,
            Recipe::SectionSweep => 2,
        }
    }

    /// Adds the recipe's nodes to `g` with the first at (`x`, `y`). Returns their ids, or says why
    /// there is no room.
    pub fn add(self, g: &mut Graph, x: f32, y: f32) -> Result<Vec<u32>, String> {
        if g.nodes.len() + self.size() > MAX_NODES {
            return Err("the graph is full".into());
        }
        let mut ids = Vec::new();
        let mut put = |g: &mut Graph,
                       col: f32,
                       row: f32,
                       kind: Kind,
                       p: &[f64],
                       target: Option<Target>,
                       from: &[u32]| {
            let id = g.add(kind, x + col * 190.0, y + row * 86.0).unwrap_or(0);
            if let Some(n) = g.node_mut(id) {
                for (slot, v) in n.p.iter_mut().zip(p) {
                    *slot = *v;
                }
                n.target = target;
            }
            for (port, f) in from.iter().enumerate() {
                if *f != 0 {
                    let _ = g.connect(*f, id, port);
                }
            }
            ids.push(id);
            id
        };
        let field = |p: FieldProp| Some(Target::Field(p));
        match self {
            Recipe::BeatPulse => {
                let b = put(g, 0.0, 0.0, Kind::OnBeat, &[1.0, 0.0], None, &[]);
                put(
                    g,
                    1.0,
                    0.0,
                    Kind::Pulse,
                    &[0.06, 260.0],
                    field(FieldProp::Zoom),
                    &[b],
                );
            }
            Recipe::KiaiDrop => {
                let k = put(g, 0.0, 0.0, Kind::OnKiaiStart, &[], None, &[]);
                put(
                    g,
                    1.0,
                    -0.5,
                    Kind::Pulse,
                    &[0.9, 500.0],
                    field(FieldProp::Flash),
                    &[k],
                );
                put(
                    g,
                    1.0,
                    0.5,
                    Kind::Pulse,
                    &[0.3, 600.0],
                    field(FieldProp::Zoom),
                    &[k],
                );
                put(g, 1.0, 1.5, Kind::Shake, &[40.0, 700.0], None, &[k]);
            }
            Recipe::FinisherBuild => {
                let n = put(g, 0.0, 0.0, Kind::OnNoteReach, &[0.0, 3.0, 0.0], None, &[]);
                let p = put(g, 1.0, 0.0, Kind::Progress, &[], None, &[n]);
                put(
                    g,
                    2.0,
                    -0.5,
                    Kind::Pulse,
                    &[0.6, 450.0],
                    field(FieldProp::Zoom),
                    &[n, p],
                );
                put(
                    g,
                    2.0,
                    0.5,
                    Kind::Pulse,
                    &[0.7, 350.0],
                    field(FieldProp::Flash),
                    &[n, p],
                );
                put(g, 2.0, 1.5, Kind::Shake, &[50.0, 500.0], None, &[n, p]);
            }
            Recipe::ComboZoom => {
                let c = put(g, 0.0, 0.0, Kind::Combo, &[], None, &[]);
                let m = put(g, 1.0, 0.0, Kind::Map, &[0.002, 0.0], None, &[c]);
                put(
                    g,
                    2.0,
                    0.0,
                    Kind::Drive,
                    &[1.0],
                    field(FieldProp::Zoom),
                    &[m],
                );
            }
            Recipe::MissGlitch => {
                let m = put(g, 0.0, 0.0, Kind::OnMiss, &[0.0, 0.0], None, &[]);
                put(
                    g,
                    1.0,
                    -0.5,
                    Kind::Pulse,
                    &[1.0, 450.0],
                    field(FieldProp::Glitch),
                    &[m],
                );
                put(g, 1.0, 0.5, Kind::Shake, &[24.0, 350.0], None, &[m]);
            }
            Recipe::EchoTrail => {
                let k = put(g, 0.0, 0.0, Kind::Kiai, &[], None, &[]);
                put(
                    g,
                    1.0,
                    -0.5,
                    Kind::Drive,
                    &[4.0],
                    field(FieldProp::Echo),
                    &[k],
                );
                let b = put(g, 0.0, 1.0, Kind::OnBeat, &[1.0, 0.0], None, &[]);
                put(
                    g,
                    1.0,
                    1.0,
                    Kind::Pulse,
                    &[14.0, 300.0],
                    field(FieldProp::EchoRot),
                    &[b],
                );
            }
            Recipe::SectionSweep => {
                let s = put(g, 0.0, 0.0, Kind::Span, &[1.0, 2.0], None, &[]);
                put(
                    g,
                    1.0,
                    0.0,
                    Kind::Drive,
                    &[360.0],
                    field(FieldProp::Rotation),
                    &[s],
                );
            }
        }
        debug_assert_eq!(ids.len(), self.size());
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recipe_builds_a_valid_graph_that_sanitizing_leaves_alone() {
        for r in Recipe::ALL {
            let mut g = Graph::default();
            let ids = r.add(&mut g, 0.0, 0.0).unwrap();
            assert_eq!(ids.len(), r.size(), "{r:?}");
            assert!(!r.label().is_empty() && !r.about().is_empty());
            for n in &g.nodes {
                if n.kind.needs_target() {
                    assert!(n.target.is_some(), "{r:?}: {:?} has no target", n.kind);
                }
                for (w, (_, want)) in n.inputs.iter().zip(n.kind.inputs()) {
                    // Every input a recipe leaves empty is a scale input, which reads as 1.
                    if w.is_none() {
                        assert!(
                            *want == super::super::graph::Port::Number
                                || n.kind.inputs().len() == 1,
                            "{r:?}: {:?} has an open input",
                            n.kind
                        );
                    }
                }
            }
            let before = g.clone();
            g.sanitize(0);
            assert_eq!(g, before, "{r:?} changes when sanitized");
        }
    }

    #[test]
    fn recipes_stack_without_clashing_and_stop_at_the_cap() {
        let mut g = Graph::default();
        for r in Recipe::ALL {
            r.add(&mut g, 0.0, 0.0).unwrap();
        }
        let ids: std::collections::HashSet<u32> = g.nodes.iter().map(|n| n.id).collect();
        assert_eq!(ids.len(), g.nodes.len());
        let mut full = Graph::default();
        for _ in 0..MAX_NODES {
            full.add(Kind::OnStart, 0.0, 0.0);
        }
        assert!(Recipe::BeatPulse.add(&mut full, 0.0, 0.0).is_err());
        assert_eq!(full.nodes.len(), MAX_NODES);
    }

    #[test]
    fn the_finisher_recipe_builds_up_with_each_finisher() {
        use crate::tko::gimmick::Gimmick;
        use crate::tko::runtime::{NoteClass, NoteInfo, Runtime};
        use std::sync::Arc;
        let mut gm = Gimmick::default();
        Recipe::FinisherBuild.add(&mut gm.graph, 0.0, 0.0).unwrap();
        gm.graph.seed = 1;
        gm.sanitize();
        let notes = [
            (1000.0, true),
            (2000.0, false),
            (3000.0, true),
            (4000.0, true),
        ]
        .map(|(time, big)| NoteInfo {
            time,
            class: NoteClass::Don,
            big,
        });
        let mut rt = Runtime::with_beats(Arc::new(gm), notes.into_iter(), 0.0, &[]);
        let kick = |rt: &mut Runtime, t: f64| {
            rt.prepare(t);
            rt.field(t).zoom - 1.0
        };
        let (a, b, c) = (
            kick(&mut rt, 1050.0),
            kick(&mut rt, 3050.0),
            kick(&mut rt, 4050.0),
        );
        assert!(a > 0.0 && b > a && c > b, "{a} {b} {c}");
        assert!(
            kick(&mut rt, 2050.0) < b,
            "the plain note at 2000 isn't a finisher"
        );
    }
}
