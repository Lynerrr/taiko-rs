//! What the editor does to a `.tko` map's node graph: add, move, wire, set and delete nodes. Each
//! is one undo step; moving a node folds a whole drag into one.

use super::state::Editor;
use crate::tko::graph::{Kind, Node, Target, clip_preset};

impl Editor {
    pub fn node(&self, id: u32) -> Option<&Node> {
        self.gimmick()?.graph.node(id)
    }

    /// Adds a node of `kind` at canvas position (`x`, `y`). Returns its id.
    pub fn add_node(&mut self, kind: Kind, x: f32, y: f32) -> Result<u32, String> {
        let mut made = None;
        let changed = self.edit_gimmick(false, |g| made = g.graph.add(kind, x, y));
        match (changed, made) {
            (true, Some(id)) => Ok(id),
            (_, None) => Err(format!(
                "a graph holds at most {} nodes",
                crate::tko::graph::MAX_NODES
            )),
            _ => Err("this map has no gimmick to add to".into()),
        }
    }

    /// Moves a node on the canvas (`drag`: part of a drag in progress).
    pub fn move_node(&mut self, id: u32, x: f32, y: f32, drag: bool) -> bool {
        self.edit_gimmick(drag, |g| {
            if let Some(n) = g.graph.node_mut(id) {
                n.x = x;
                n.y = y;
            }
        })
    }

    /// Wires `from`'s output into input `port` of `to`.
    pub fn connect_nodes(&mut self, from: u32, to: u32, port: usize) -> Result<(), String> {
        let mut result = Ok(());
        // Checked on a copy first, so a refused wire is not an undo step.
        if let Some(g) = self.gimmick() {
            let mut probe = g.graph.clone();
            result = probe.connect(from, to, port);
        }
        if result.is_ok() {
            self.edit_gimmick(false, |g| {
                let _ = g.graph.connect(from, to, port);
            });
        }
        result
    }

    /// Takes the wire out of input `port` of `to`.
    pub fn disconnect_node(&mut self, to: u32, port: usize) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(w) = g.graph.node_mut(to).and_then(|n| n.inputs.get_mut(port)) {
                *w = None;
            }
        })
    }

    pub fn delete_node(&mut self, id: u32) -> bool {
        self.edit_gimmick(false, |g| g.graph.remove(id))
    }

    /// Changes parameter `i` of a node by `steps` of its step (the value is held in its range).
    pub fn step_node_param(&mut self, id: u32, i: usize, steps: f64) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(n) = g.graph.node_mut(id)
                && let Some(d) = n.kind.params().get(i)
            {
                let v = n.param(i) + steps * d.step;
                // Away from float dust: 0.1 + 0.05 + 0.05 stays 0.2.
                let v = (v * 1e6).round() / 1e6;
                let v = v.clamp(d.min, d.max);
                n.p.resize(n.kind.params().len(), 0.0);
                n.p[i] = v;
            }
        })
    }

    pub fn set_node_target(&mut self, id: u32, target: Option<Target>) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(n) = g.graph.node_mut(id) {
                n.target = target;
            }
        })
    }

    pub fn next_node_ease(&mut self, id: u32) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(n) = g.graph.node_mut(id) {
                n.ease = n.ease.next();
            }
        })
    }

    /// Gives a Play clip node preset `n` of the clips the editor ships.
    pub fn set_node_clip(&mut self, id: u32, n: usize) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(node) = g.graph.node_mut(id)
                && node.kind == Kind::Clip
            {
                node.clip = clip_preset(n).1;
            }
        })
    }

    /// Changes the seed random nodes and shakes draw from.
    pub fn step_seed(&mut self, by: i64) -> bool {
        self.edit_gimmick(false, |g| {
            g.graph.seed = (g.graph.seed as i64).saturating_add(by).max(0) as u64;
        })
    }

    /// Every target a node can drive: the playfield's properties, then each group's.
    pub fn targets(&self) -> Vec<Target> {
        use crate::tko::gimmick::{FieldProp, NoteProp};
        let groups = self.gimmick().map_or(0, |g| g.groups.len());
        let mut out: Vec<Target> = FieldProp::ALL.iter().map(|p| Target::Field(*p)).collect();
        for g in 0..groups {
            out.extend(NoteProp::ALL.iter().map(|p| Target::Group(g, *p)));
        }
        out
    }

    /// Moves a node's target `dir` along `targets()`.
    pub fn cycle_node_target(&mut self, id: u32, dir: i32) -> bool {
        let all = self.targets();
        let Some(node) = self.node(id) else {
            return false;
        };
        let at = node
            .target
            .and_then(|t| all.iter().position(|q| *q == t))
            // Without a target yet, forwards starts at the first and backwards at the last.
            .map_or(if dir < 0 { 0 } else { -1 }, |i| i as i32);
        let next = (at + dir).rem_euclid(all.len() as i32) as usize;
        self.set_node_target(id, Some(all[next]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::{Chart, tests::MAP};
    use crate::tko::gimmick::{FieldProp, Group};
    use std::path::PathBuf;

    fn plain() -> Editor {
        Editor::new(Chart::parse(MAP), PathBuf::from("t.osu"))
    }

    fn editor() -> Editor {
        let mut ed = plain();
        ed.chart.gimmick = Some(Default::default());
        ed
    }

    #[test]
    fn nodes_are_added_wired_and_every_step_undoes() {
        let mut ed = editor();
        let beat = ed.add_node(Kind::OnBeat, 10.0, 10.0).unwrap();
        let pulse = ed.add_node(Kind::Pulse, 200.0, 10.0).unwrap();
        ed.connect_nodes(beat, pulse, 0).unwrap();
        assert_eq!(ed.node(pulse).unwrap().inputs, [Some(beat)]);
        assert!(ed.cycle_node_target(pulse, 1));
        assert_eq!(
            ed.node(pulse).unwrap().target,
            Some(Target::Field(FieldProp::X))
        );
        assert!(ed.undo() && ed.node(pulse).unwrap().target.is_none());
        assert!(ed.undo() && ed.node(pulse).unwrap().inputs == [None]);
        assert!(ed.undo() && ed.undo() && ed.node(beat).is_none());
        assert!(ed.redo());
        assert!(ed.node(beat).is_some());
    }

    #[test]
    fn a_refused_wire_is_not_an_undo_step() {
        let mut ed = editor();
        let wave = ed.add_node(Kind::Wave, 0.0, 0.0).unwrap();
        let pulse = ed.add_node(Kind::Pulse, 0.0, 0.0).unwrap();
        let before = ed.gimmick().unwrap().clone();
        let err = ed.connect_nodes(wave, pulse, 0).unwrap_err();
        assert!(err.contains("trigger"), "{err}");
        assert!(ed.connect_nodes(pulse, wave, 0).is_err());
        assert_eq!(*ed.gimmick().unwrap(), before);
        // The last undo is still the node that was added.
        assert!(ed.undo() && ed.node(pulse).is_none());
    }

    #[test]
    fn dragging_a_node_is_one_undo_and_deleting_unwires() {
        let mut ed = editor();
        let a = ed.add_node(Kind::OnStart, 0.0, 0.0).unwrap();
        let b = ed.add_node(Kind::Once, 100.0, 0.0).unwrap();
        ed.connect_nodes(a, b, 0).unwrap();
        for i in 1..=10 {
            assert!(ed.move_node(b, 100.0 + i as f32 * 5.0, 7.0, true));
        }
        ed.end_drag();
        assert_eq!((ed.node(b).unwrap().x, ed.node(b).unwrap().y), (150.0, 7.0));
        assert!(ed.undo());
        assert_eq!(ed.node(b).unwrap().x, 100.0, "the whole drag in one step");
        assert!(ed.delete_node(a));
        assert_eq!(ed.node(b).unwrap().inputs, [None]);
        assert!(ed.undo() && ed.node(b).unwrap().inputs == [Some(a)]);
    }

    #[test]
    fn parameters_step_inside_their_range_and_a_plain_osu_has_no_graph() {
        let mut ed = editor();
        let n = ed.add_node(Kind::Counter, 0.0, 0.0).unwrap();
        assert_eq!(ed.node(n).unwrap().param(0), 2.0);
        assert!(ed.step_node_param(n, 0, 3.0));
        assert_eq!(ed.node(n).unwrap().param(0), 5.0);
        assert!(ed.step_node_param(n, 0, -100.0));
        assert_eq!(ed.node(n).unwrap().param(0), 1.0);
        assert!(!ed.step_node_param(n, 0, -1.0), "already at the least");
        let p = ed.add_node(Kind::Pulse, 0.0, 0.0).unwrap();
        for _ in 0..3 {
            ed.step_node_param(p, 0, 1.0);
        }
        assert_eq!(ed.node(p).unwrap().param(0), 0.25, "no float dust");
        let mut osu = plain();
        assert!(osu.add_node(Kind::OnStart, 0.0, 0.0).is_err());
    }

    #[test]
    fn targets_list_the_field_and_each_group_and_cycle_both_ways() {
        let mut ed = editor();
        ed.edit_gimmick(false, |g| g.groups.push(Group::default()));
        assert_eq!(ed.targets().len(), 10);
        let n = ed.add_node(Kind::Drive, 0.0, 0.0).unwrap();
        assert!(ed.cycle_node_target(n, -1));
        assert!(matches!(
            ed.node(n).unwrap().target,
            Some(Target::Group(0, _))
        ));
        assert!(ed.cycle_node_target(n, 1));
        assert_eq!(
            ed.node(n).unwrap().target,
            Some(Target::Field(FieldProp::X))
        );
    }
}
