//! What the editor does to a `.tko` map's node graph: add, move, wire, set and delete nodes. Each
//! is one undo step; moving a node folds a whole drag into one.

use super::state::Editor;
use crate::tko::gimmick::{MAX_MARKS, Mark};
use crate::tko::graph::{Kind, Node, Target, clip_preset};
use crate::tko::recipes::Recipe;

/// Gives the nodes that name a mark by number (1 is the first; 0 is the map's start or end)
/// the number `to` says, or with None one no mark has.
fn renumber_marks(g: &mut crate::tko::gimmick::Gimmick, to: impl Fn(usize) -> Option<usize>) {
    for n in &mut g.graph.nodes {
        let count = match n.kind {
            Kind::OnMark => 1,
            Kind::Between | Kind::Span => 2,
            _ => 0,
        };
        for p in n.p.iter_mut().take(count) {
            let m = *p as usize;
            if m > 0 {
                *p = to(m).map_or(256.0, |m| m as f64);
            }
        }
    }
}

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

    /// Sets parameter `i` of a node to `v`, held in its range.
    pub fn set_node_param(&mut self, id: u32, i: usize, v: f64) -> bool {
        self.edit_gimmick(false, |g| {
            if let Some(n) = g.graph.node_mut(id)
                && let Some(d) = n.kind.params().get(i)
                && v.is_finite()
            {
                n.p.resize(n.kind.params().len(), 0.0);
                n.p[i] = ((v * 1e6).round() / 1e6).clamp(d.min, d.max);
            }
        })
    }

    /// Adds a recipe's nodes with the first at (`x`, `y`) as one undo step. Returns their ids.
    pub fn add_recipe(&mut self, r: Recipe, x: f32, y: f32) -> Result<Vec<u32>, String> {
        let mut result = Err("this map has no gimmick to add to".to_string());
        self.edit_gimmick(false, |g| result = r.add(&mut g.graph, x, y));
        result
    }

    /// Puts a mark at chart time `t` (nodes refer to marks by their order). False if one is
    /// already there.
    pub fn add_mark(&mut self, t: f64) -> bool {
        let t = t.round();
        self.edit_gimmick(false, |g| {
            if g.marks.len() < MAX_MARKS && !g.marks.iter().any(|m| (m.time - t).abs() < 1.0) {
                // Marks go by time and nodes name them by number: those after it move up one.
                let k = g.marks.iter().filter(|m| m.time < t).count() + 1;
                renumber_marks(g, |m| if m >= k { Some(m + 1) } else { Some(m) });
                g.marks.push(Mark {
                    name: String::new(),
                    time: t,
                });
            }
        })
    }

    /// Takes away the mark nearest `t`, if one is within `reach` ms.
    pub fn remove_mark(&mut self, t: f64, reach: f64) -> bool {
        self.edit_gimmick(false, |g| {
            let near = g
                .marks
                .iter()
                .enumerate()
                .filter(|(_, m)| (m.time - t).abs() <= reach)
                .min_by(|a, b| (a.1.time - t).abs().total_cmp(&(b.1.time - t).abs()))
                .map(|(i, _)| i);
            if let Some(i) = near {
                g.marks.remove(i);
                // Nodes on it name none there is (so they never fire); those after move down.
                let k = i + 1;
                renumber_marks(g, |m| match m.cmp(&k) {
                    std::cmp::Ordering::Less => Some(m),
                    std::cmp::Ordering::Equal => None,
                    std::cmp::Ordering::Greater => Some(m - 1),
                });
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::{Chart, tests::MAP};
    use crate::tko::gimmick::FieldProp;
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
        assert_eq!(ed.node(pulse).unwrap().inputs, [Some(beat), None]);
        assert!(ed.set_node_target(pulse, Some(Target::Field(FieldProp::X))));
        assert_eq!(
            ed.node(pulse).unwrap().target,
            Some(Target::Field(FieldProp::X))
        );
        assert!(ed.undo() && ed.node(pulse).unwrap().target.is_none());
        assert!(ed.undo() && ed.node(pulse).unwrap().inputs == [None, None]);
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
    fn a_param_can_be_typed_in_and_is_held_in_its_range() {
        let mut ed = editor();
        let w = ed.add_node(Kind::Wave, 0.0, 0.0).unwrap();
        assert!(ed.set_node_param(w, 1, 3.25));
        assert_eq!(ed.node(w).unwrap().param(1), 3.25);
        assert!(ed.set_node_param(w, 1, 1e9));
        assert_eq!(ed.node(w).unwrap().param(1), 1000.0);
        assert!(!ed.set_node_param(w, 1, f64::NAN));
        assert!(!ed.set_node_param(w, 9, 1.0), "no such parameter");
        assert!(ed.undo() && ed.undo());
        assert_eq!(ed.node(w).unwrap().param(1), 1.0);
    }

    #[test]
    fn a_recipe_is_one_undo_step_and_refused_on_a_plain_osu() {
        let mut ed = editor();
        let ids = ed.add_recipe(Recipe::KiaiDrop, 0.0, 0.0).unwrap();
        assert_eq!(ids.len(), 4);
        assert!(ed.node(ids[1]).unwrap().target.is_some());
        assert!(ed.undo());
        assert!(ed.gimmick().unwrap().graph.is_empty());
        assert!(plain().add_recipe(Recipe::BeatPulse, 0.0, 0.0).is_err());
    }

    #[test]
    fn marks_are_placed_listed_in_order_removed_and_undone() {
        let mut ed = editor();
        assert!(ed.add_mark(3000.4));
        assert!(ed.add_mark(1000.0));
        assert!(!ed.add_mark(1000.0), "one place, one mark");
        let times: Vec<f64> = ed.gimmick().unwrap().marks.iter().map(|m| m.time).collect();
        assert_eq!(times, [1000.0, 3000.0], "kept in time order");
        assert!(!ed.remove_mark(2000.0, 500.0), "nothing near");
        assert!(ed.remove_mark(3100.0, 500.0));
        assert_eq!(ed.gimmick().unwrap().marks.len(), 1);
        assert!(ed.undo());
        assert_eq!(ed.gimmick().unwrap().marks.len(), 2);
        assert!(!plain().add_mark(1.0));
    }

    #[test]
    fn nodes_keep_their_marks_when_marks_come_and_go() {
        let mut ed = editor();
        ed.add_mark(1000.0);
        ed.add_mark(5000.0);
        let at = ed.add_node(Kind::OnMark, 0.0, 0.0).unwrap();
        ed.set_node_param(at, 0, 2.0);
        let span = ed.add_node(Kind::Between, 0.0, 0.0).unwrap();
        ed.set_node_param(span, 0, 1.0);
        ed.set_node_param(span, 1, 2.0);
        let p = |ed: &Editor, id: u32| ed.node(id).unwrap().p.clone();
        // A mark between them: the 5 s one is the third now.
        ed.add_mark(3000.0);
        assert_eq!(p(&ed, at)[0], 3.0);
        assert_eq!(p(&ed, span)[..2], [1.0, 3.0]);
        // The first goes: the rest move down, and one on it names no mark.
        ed.remove_mark(1000.0, 10.0);
        assert_eq!(p(&ed, at)[0], 2.0);
        assert_eq!(p(&ed, span)[0], 256.0);
        assert_eq!(p(&ed, span)[1], 2.0);
    }
}
