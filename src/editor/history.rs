//! Undo and redo: every change to the chart is a `Command` that knows how to apply and revert
//! itself, so what the editor does and what it undoes can't drift apart.
//!
//! Edits store the objects before and after (by id), which covers every kind of change (moving,
//! retyping, resizing a roll, editing a line) with one command and makes a drag that merges into
//! a single step trivial.

use super::chart::{Chart, HitObject, TimingPoint};

#[derive(Clone, Debug)]
pub enum Command {
    InsertObjects(Vec<HitObject>),
    RemoveObjects(Vec<HitObject>),
    EditObjects {
        before: Vec<HitObject>,
        after: Vec<HitObject>,
    },
    InsertTiming(Vec<TimingPoint>),
    RemoveTiming(Vec<TimingPoint>),
    EditTiming {
        before: Vec<TimingPoint>,
        after: Vec<TimingPoint>,
    },
    /// The `.tko` gimmick changed (keyframes, groups): the whole of it before and after.
    Gimmick {
        before: Box<crate::tko::gimmick::Gimmick>,
        after: Box<crate::tko::gimmick::Gimmick>,
    },
    /// One step made of several (a gimmick tool writes many lines and notes at once).
    Composite(Vec<Command>),
}

fn insert_objects(c: &mut Chart, list: &[HitObject]) {
    for o in list {
        c.insert_object(o.clone());
    }
}

fn remove_objects(c: &mut Chart, list: &[HitObject]) {
    for o in list {
        c.remove_object(o.id);
    }
}

fn set_objects(c: &mut Chart, list: &[HitObject]) {
    for o in list {
        if let Some(at) = c.objects.iter().position(|q| q.id == o.id) {
            c.objects[at] = o.clone();
        }
    }
    c.sort_objects();
    c.dirty = true;
}

fn insert_timing(c: &mut Chart, list: &[TimingPoint]) {
    for p in list {
        c.insert_timing(p.clone());
    }
}

fn remove_timing(c: &mut Chart, list: &[TimingPoint]) {
    for p in list {
        c.remove_timing(p.id);
    }
}

fn set_timing(c: &mut Chart, list: &[TimingPoint]) {
    for p in list {
        if let Some(at) = c.timing.iter().position(|q| q.id == p.id) {
            c.timing[at] = p.clone();
        }
    }
    c.sort_timing();
    c.dirty = true;
}

impl Command {
    pub fn apply(&self, c: &mut Chart) {
        match self {
            Command::InsertObjects(l) => insert_objects(c, l),
            Command::RemoveObjects(l) => remove_objects(c, l),
            Command::EditObjects { after, .. } => set_objects(c, after),
            Command::InsertTiming(l) => insert_timing(c, l),
            Command::RemoveTiming(l) => remove_timing(c, l),
            Command::EditTiming { after, .. } => set_timing(c, after),
            Command::Gimmick { after, .. } => {
                c.gimmick = Some((**after).clone());
                c.dirty = true;
            }
            Command::Composite(l) => l.iter().for_each(|x| x.apply(c)),
        }
    }

    pub fn revert(&self, c: &mut Chart) {
        match self {
            Command::InsertObjects(l) => remove_objects(c, l),
            Command::RemoveObjects(l) => insert_objects(c, l),
            Command::EditObjects { before, .. } => set_objects(c, before),
            Command::InsertTiming(l) => remove_timing(c, l),
            Command::RemoveTiming(l) => insert_timing(c, l),
            Command::EditTiming { before, .. } => set_timing(c, before),
            Command::Gimmick { before, .. } => {
                c.gimmick = Some((**before).clone());
                c.dirty = true;
            }
            Command::Composite(l) => l.iter().rev().for_each(|x| x.revert(c)),
        }
    }

    /// Gives every new object and point the id it keeps from now on, so reverting the insert
    /// finds exactly what it put there.
    fn assign_ids(&mut self, c: &mut Chart) {
        match self {
            Command::InsertObjects(l) => {
                for o in l.iter_mut().filter(|o| o.id == 0) {
                    o.id = c.take_id();
                }
            }
            Command::InsertTiming(l) => {
                for p in l.iter_mut().filter(|p| p.id == 0) {
                    p.id = c.take_id();
                }
            }
            Command::Composite(l) => l.iter_mut().for_each(|x| x.assign_ids(c)),
            _ => {}
        }
    }

    /// The command a drag step folds into: the same things edited again, so the whole drag is
    /// one undo.
    fn merged(&self, later: &Command) -> Option<Command> {
        match (self, later) {
            (
                Command::EditObjects { before, after },
                Command::EditObjects {
                    after: later_after, ..
                },
            ) if same_ids(after.iter().map(|o| o.id), later_after.iter().map(|o| o.id)) => {
                Some(Command::EditObjects {
                    before: before.clone(),
                    after: later_after.clone(),
                })
            }
            (
                Command::EditTiming { before, after },
                Command::EditTiming {
                    after: later_after, ..
                },
            ) if same_ids(after.iter().map(|p| p.id), later_after.iter().map(|p| p.id)) => {
                Some(Command::EditTiming {
                    before: before.clone(),
                    after: later_after.clone(),
                })
            }
            (Command::Gimmick { before, .. }, Command::Gimmick { after, .. }) => {
                Some(Command::Gimmick {
                    before: before.clone(),
                    after: after.clone(),
                })
            }
            _ => None,
        }
    }
}

fn same_ids(a: impl Iterator<Item = u64>, b: impl Iterator<Item = u64>) -> bool {
    let (mut a, mut b): (Vec<u64>, Vec<u64>) = (a.collect(), b.collect());
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

#[derive(Default)]
pub struct History {
    done: Vec<Command>,
    undone: Vec<Command>,
    /// Length of `done` when the chart was last saved (None: that state was undone away).
    saved: Option<usize>,
    /// The last command may still take a drag step.
    open: bool,
}

impl History {
    pub fn new() -> History {
        History {
            saved: Some(0),
            ..History::default()
        }
    }

    /// Applies `cmd` as its own undo step.
    pub fn run(&mut self, chart: &mut Chart, cmd: Command) {
        self.push(chart, cmd, false);
    }

    /// Applies `cmd`, folding it into the previous step if that was a drag of the same things
    /// still going on (`end_drag` closes it).
    pub fn run_drag(&mut self, chart: &mut Chart, cmd: Command) {
        self.push(chart, cmd, true);
    }

    pub fn end_drag(&mut self) {
        self.open = false;
    }

    fn push(&mut self, chart: &mut Chart, mut cmd: Command, drag: bool) {
        cmd.assign_ids(chart);
        cmd.apply(chart);
        if !self.undone.is_empty() {
            self.undone.clear();
            // The saved state was among the redo steps: it can't be reached by undoing now.
            if self.saved.is_some_and(|s| s > self.done.len()) {
                self.saved = None;
            }
        }
        if drag
            && self.open
            && let Some(last) = self.done.last_mut()
            && let Some(m) = last.merged(&cmd)
        {
            *last = m;
            return;
        }
        self.done.push(cmd);
        self.open = drag;
    }

    pub fn undo(&mut self, chart: &mut Chart) -> bool {
        self.open = false;
        let Some(cmd) = self.done.pop() else {
            return false;
        };
        cmd.revert(chart);
        self.undone.push(cmd);
        true
    }

    pub fn redo(&mut self, chart: &mut Chart) -> bool {
        self.open = false;
        let Some(cmd) = self.undone.pop() else {
            return false;
        };
        cmd.apply(chart);
        self.done.push(cmd);
        true
    }

    #[cfg(test)]
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    #[cfg(test)]
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.done.len());
    }

    /// Changed since the last save.
    pub fn modified(&self) -> bool {
        self.saved != Some(self.done.len())
    }
}

#[cfg(test)]
mod tests {
    use super::super::chart::tests::MAP;
    use super::*;

    fn times(c: &Chart) -> Vec<f64> {
        c.objects.iter().map(|o| o.time).collect()
    }

    #[test]
    fn undo_and_redo_walk_back_and_forth() {
        let mut c = Chart::parse(MAP);
        let mut h = History::new();
        let o = HitObject::hit(3000.0, false, false);
        h.run(&mut c, Command::InsertObjects(vec![o]));
        assert_eq!(c.objects.len(), 6);
        assert!(h.modified());
        assert!(h.undo(&mut c));
        assert_eq!(c.objects.len(), 5);
        assert!(!h.modified());
        assert!(h.redo(&mut c));
        assert_eq!(times(&c).last(), Some(&9000.0));
        assert!(c.objects.iter().any(|o| o.time == 3000.0));
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut c = Chart::parse(MAP);
        let mut h = History::new();
        let id = c.objects[0].id;
        let before = c.objects[0].clone();
        for t in [1010.0, 1020.0, 1035.0] {
            let mut after = before.clone();
            after.time = t;
            h.run_drag(
                &mut c,
                Command::EditObjects {
                    before: vec![before.clone()],
                    after: vec![after],
                },
            );
        }
        h.end_drag();
        assert_eq!(c.object(id).unwrap().time, 1035.0);
        assert!(h.undo(&mut c));
        assert_eq!(c.object(id).unwrap().time, 1000.0);
        assert!(!h.can_undo(), "the whole drag was one step");
        // Back where it started, the object writes its own original line again.
        assert!(c.to_text().contains("256,192,1000,1,0,0:0:0:0:\r\n"));
    }

    #[test]
    fn an_edit_that_changes_the_order_keeps_the_chart_sorted() {
        let mut c = Chart::parse(MAP);
        let mut h = History::new();
        let before = c.objects[0].clone();
        let mut after = before.clone();
        after.time = 1750.0;
        h.run(
            &mut c,
            Command::EditObjects {
                before: vec![before],
                after: vec![after],
            },
        );
        assert_eq!(times(&c)[..3], [1500.0, 1750.0, 2000.0]);
        h.undo(&mut c);
        assert_eq!(times(&c)[..3], [1000.0, 1500.0, 2000.0]);
    }

    #[test]
    fn a_composite_reverts_in_reverse_order() {
        let mut c = Chart::parse(MAP);
        let mut h = History::new();
        let gone = c.objects[1].clone();
        let green = TimingPoint::green(5000.0, 1.5);
        let cmd = Command::Composite(vec![
            Command::RemoveObjects(vec![gone]),
            Command::InsertTiming(vec![green]),
            Command::InsertObjects(vec![HitObject::hit(1500.0, true, true)]),
        ]);
        h.run(&mut c, cmd);
        assert_eq!((c.objects.len(), c.timing.len()), (5, 3));
        h.undo(&mut c);
        assert_eq!((c.objects.len(), c.timing.len()), (5, 2));
        assert_eq!(c.to_text(), MAP, "back to the original text");
    }

    #[test]
    fn a_new_step_drops_what_was_undone_and_the_saved_mark_follows() {
        let mut c = Chart::parse(MAP);
        let mut h = History::new();
        h.run(
            &mut c,
            Command::InsertObjects(vec![HitObject::hit(3000.0, false, false)]),
        );
        h.mark_saved();
        assert!(!h.modified());
        h.undo(&mut c);
        assert!(h.modified());
        h.run(
            &mut c,
            Command::InsertObjects(vec![HitObject::hit(3500.0, false, false)]),
        );
        assert!(!h.can_redo());
        assert!(h.modified(), "the saved state can't be reached any more");
        h.undo(&mut c);
        assert!(h.modified());
    }
}
