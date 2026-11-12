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
    /// A `Key:Value` line of the file's other sections (Song setup's metadata and difficulty):
    /// `None` is no such line.
    SetKey {
        section: &'static str,
        key: &'static str,
        before: Option<String>,
        after: Option<String>,
    },
    /// One step made of several (a gimmick tool writes many lines and notes at once).
    Composite(Vec<Command>),
}

fn set_key(c: &mut Chart, section: &str, key: &str, value: Option<&str>) {
    match value {
        Some(v) => c.set_key(section, key, v),
        None => c.remove_key(section, key),
    }
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
    // Moved notes: (from, to). Groups hold their notes by time, so they go along.
    let mut moved: Vec<(f64, f64, bool)> = Vec::new();
    for o in list {
        if let Some(at) = c.objects.iter().position(|q| q.id == o.id) {
            if c.objects[at].time != o.time {
                moved.push((c.objects[at].time, o.time, false));
            }
            c.objects[at] = o.clone();
        }
    }
    if !moved.is_empty()
        && let Some(g) = c.gimmick.as_mut()
    {
        // A note (or fake) that shared the millisecond and stayed keeps its groups there: the
        // one that left takes a copy of the time instead of it.
        let ids: std::collections::HashSet<u64> = list.iter().map(|o| o.id).collect();
        for m in &mut moved {
            m.2 = c
                .objects
                .iter()
                .any(|q| !ids.contains(&q.id) && (q.time - m.0).abs() <= 0.5);
        }
        move_group_notes(g, &moved);
    }
    c.sort_objects();
    c.dirty = true;
}

/// Each group's note at a `from` time is at its `to` now, all at once (so notes swapping places
/// swap groups too), and so is each note drawn over the effects. With `stays` something else is
/// still at `from`, so `to` is added beside it rather than taking its place.
fn move_group_notes(g: &mut crate::tko::gimmick::Gimmick, moved: &[(f64, f64, bool)]) {
    let follow = |list: &mut Vec<f64>| {
        let mut added = Vec::new();
        let mut changed = false;
        for t in list.iter_mut() {
            if let Some(&(_, to, stays)) = moved.iter().find(|m| (m.0 - *t).abs() <= 0.5) {
                if stays {
                    added.push(to);
                } else {
                    *t = to;
                }
                changed = true;
            }
        }
        list.extend(added);
        // No dedup: two notes stacked on one ms and undone must both come back.
        if changed {
            list.sort_by(f64::total_cmp);
        }
    };
    for gr in &mut g.groups {
        follow(&mut gr.notes);
    }
    // Notes drawn over every effect stay so.
    follow(&mut g.above);
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
            Command::SetKey {
                section,
                key,
                after,
                ..
            } => set_key(c, section, key, after.as_deref()),
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
            Command::SetKey {
                section,
                key,
                before,
                ..
            } => set_key(c, section, key, before.as_deref()),
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
    pub(super) fn merged(&self, later: &Command) -> Option<Command> {
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
            // A step that moves lines and the notes following them: each part folds in, taking
            // what the later step touched for the first time too.
            (Command::Composite(a), Command::Composite(b)) if a.len() == b.len() => a
                .iter()
                .zip(b)
                .map(|(x, y)| x.folded(y))
                .collect::<Option<Vec<_>>>()
                .map(Command::Composite),
            _ => None,
        }
    }
}

impl Command {
    /// The command that takes this one back.
    pub fn inverted(&self) -> Command {
        match self {
            Command::InsertObjects(l) => Command::RemoveObjects(l.clone()),
            Command::RemoveObjects(l) => Command::InsertObjects(l.clone()),
            Command::EditObjects { before, after } => Command::EditObjects {
                before: after.clone(),
                after: before.clone(),
            },
            Command::InsertTiming(l) => Command::RemoveTiming(l.clone()),
            Command::RemoveTiming(l) => Command::InsertTiming(l.clone()),
            Command::EditTiming { before, after } => Command::EditTiming {
                before: after.clone(),
                after: before.clone(),
            },
            Command::Gimmick { before, after } => Command::Gimmick {
                before: after.clone(),
                after: before.clone(),
            },
            Command::SetKey {
                section,
                key,
                before,
                after,
            } => Command::SetKey {
                section,
                key,
                before: after.clone(),
                after: before.clone(),
            },
            Command::Composite(l) => {
                Command::Composite(l.iter().rev().map(Command::inverted).collect())
            }
        }
    }

    /// `self` then `later` as one edit of the same kind, whatever each touched: every thing as it
    /// was before the first of them that touched it, and as the last one left it.
    fn folded(&self, later: &Command) -> Option<Command> {
        fn fold<T: Clone>(
            before: &[T],
            after: &[T],
            later_before: &[T],
            later_after: &[T],
            id: impl Fn(&T) -> u64,
        ) -> (Vec<T>, Vec<T>) {
            let mut b = before.to_vec();
            b.extend(
                later_before
                    .iter()
                    .filter(|x| !before.iter().any(|y| id(y) == id(x)))
                    .cloned(),
            );
            let mut a = later_after.to_vec();
            a.extend(
                after
                    .iter()
                    .filter(|x| !later_after.iter().any(|y| id(y) == id(x)))
                    .cloned(),
            );
            (b, a)
        }
        match (self, later) {
            (
                Command::EditObjects { before, after },
                Command::EditObjects {
                    before: lb,
                    after: la,
                },
            ) => {
                let (before, after) = fold(before, after, lb, la, |o| o.id);
                Some(Command::EditObjects { before, after })
            }
            (
                Command::EditTiming { before, after },
                Command::EditTiming {
                    before: lb,
                    after: la,
                },
            ) => {
                let (before, after) = fold(before, after, lb, la, |p| p.id);
                Some(Command::EditTiming { before, after })
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
    /// Counts every change to the chart (and every undo and redo), so a cache of something
    /// derived from the chart knows when it is out of date.
    rev: u64,
    /// In a mapping session (`super::collab`): every change made here, in order, until the
    /// session takes it to send. Undo and redo then only take back this player's own steps, as
    /// far as nobody changed them since.
    outbox: Option<Vec<Command>>,
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

    /// Changes whenever the chart does.
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// The chart changed by others (a mapping session's edits from the other players).
    pub fn touch(&mut self) {
        self.rev += 1;
    }

    /// The changes made here aren't shared any more (the session ended).
    pub fn stop_sharing(&mut self) {
        self.outbox = None;
    }

    /// From now on the changes made here are shared (see `outbox`).
    pub fn share(&mut self) {
        self.outbox.get_or_insert_with(Vec::new);
    }

    /// The changes made since the last call, in order (none outside a session).
    pub fn take_outgoing(&mut self) -> Vec<Command> {
        self.outbox.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Applies `cmd` and keeps it only if `check` passes on the chart it makes; otherwise takes
    /// it back and leaves the history as it was, the redo steps too.
    pub fn run_checked(
        &mut self,
        chart: &mut Chart,
        mut cmd: Command,
        check: impl FnOnce(&Chart) -> Result<(), String>,
    ) -> Result<(), String> {
        cmd.assign_ids(chart);
        cmd.apply(chart);
        if let Err(why) = check(chart) {
            cmd.revert(chart);
            return Err(why);
        }
        self.record(cmd, false);
        Ok(())
    }

    fn push(&mut self, chart: &mut Chart, mut cmd: Command, drag: bool) {
        cmd.assign_ids(chart);
        cmd.apply(chart);
        self.record(cmd, drag);
    }

    /// Keeps `cmd`, applied already, as the newest step.
    fn record(&mut self, cmd: Command, drag: bool) {
        self.rev += 1;
        if let Some(out) = &mut self.outbox {
            out.push(cmd.clone());
        }
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
        if self.outbox.is_some() {
            return self.step_shared(chart, true);
        }
        let Some(cmd) = self.done.pop() else {
            return false;
        };
        cmd.revert(chart);
        self.rev += 1;
        self.undone.push(cmd);
        true
    }

    pub fn redo(&mut self, chart: &mut Chart) -> bool {
        self.open = false;
        if self.outbox.is_some() {
            return self.step_shared(chart, false);
        }
        let Some(cmd) = self.undone.pop() else {
            return false;
        };
        cmd.apply(chart);
        self.rev += 1;
        self.done.push(cmd);
        true
    }

    /// Undo (`back`) or redo in a session: the step is taken back (or made again) only where
    /// the chart still is as it left it (or found it), since others edit it too. A step nothing
    /// is left of is passed over for the one before it.
    fn step_shared(&mut self, chart: &mut Chart, back: bool) -> bool {
        loop {
            let from = if back {
                &mut self.done
            } else {
                &mut self.undone
            };
            let Some(cmd) = from.pop() else {
                return false;
            };
            let wanted = if back { cmd.inverted() } else { cmd.clone() };
            let made = super::collab::apply_guarded(&wanted, chart);
            if back {
                self.undone.push(cmd);
            } else {
                self.done.push(cmd);
            }
            if let Some(made) = made {
                self.rev += 1;
                if let Some(out) = &mut self.outbox {
                    out.push(made);
                }
                return true;
            }
        }
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
        // A drag going on past the save is a step of its own, or it would look saved.
        self.open = false;
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
