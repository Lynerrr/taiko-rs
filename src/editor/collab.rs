//! Live collaborative mapping: the editor's side of a mapping session (the protocol is
//! `taiko_proto::mapping`, the design the web backend's `docs/collab-mapping.md`).
//!
//! Every edit is a `Command` already, so sharing one is sending it. The server puts everyone's
//! ops in one order; each game keeps
//!
//! - the **confirmed** chart of every difficulty: the snapshot and every op the server ordered,
//!   applied in that order;
//! - its own **pending** ops, made here and not yet back from the server, in the order made;
//! - the editor's chart: the confirmed one with the pending ops on top.
//!
//! Applying an op only sets things (inserts what isn't there, removes what is, sets what is to
//! its new value) and passes over what is gone, so the same ops in the same order give every
//! game the same chart, however their own edits interleaved. Undo and redo take back only this
//! player's steps, and only where nobody changed things since (`apply_guarded`). A `.tko`'s
//! gimmick is merged rather than replaced (`merged_gimmick`), so effects keyed by several
//! players at once all stay.
//!
//! Nothing here runs outside a session: the single-player editor never builds a `Sync`, and the
//! game only opens a session with a server that lists `mapping::FEATURE`.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use taiko_proto::mapping::{self, DocSnapshot, Line, MapOp, MapUp, Obj, Sequenced, SnapshotData};

use super::chart::{Chart, HitObject, TimingPoint};
use super::history::Command;
use super::state::Editor;
use crate::tko::gimmick::Gimmick;
use serde_json::Value;

/// While dragging, at most one op this often (a later step of the drag replaces an unsent one).
pub const DRAG_SEND_MS: f64 = 50.0;

pub mod connect;
pub mod session;

// ---- the wire form of a command ----

pub fn obj(o: &HitObject) -> Obj {
    Obj {
        id: o.id,
        line: o.file_line(),
        fake: o.fake,
        group: o.group,
    }
}

fn object(o: &Obj) -> Result<HitObject, String> {
    let mut h = HitObject::parse(&o.line)
        .filter(|_| o.id != 0)
        .ok_or_else(|| format!("unreadable object {:?}", o.line))?;
    h.id = o.id;
    h.fake = o.fake;
    h.group = o.group;
    Ok(h)
}

fn line(p: &TimingPoint) -> Line {
    Line {
        id: p.id,
        line: p.file_line(),
    }
}

fn point(l: &Line) -> Result<TimingPoint, String> {
    let mut p = TimingPoint::parse(&l.line)
        .filter(|_| l.id != 0)
        .ok_or_else(|| format!("unreadable timing line {:?}", l.line))?;
    p.id = l.id;
    Ok(p)
}

fn objects(l: &[Obj]) -> Result<Vec<HitObject>, String> {
    l.iter().map(object).collect()
}

fn points(l: &[Line]) -> Result<Vec<TimingPoint>, String> {
    l.iter().map(point).collect()
}

fn gimmick(json: &str) -> Result<Box<Gimmick>, String> {
    serde_json::from_str(json).map_err(|e| format!("unreadable gimmick: {e}"))
}

/// A command as it goes on the wire.
pub fn to_wire(cmd: &Command) -> MapOp {
    match cmd {
        Command::InsertObjects(l) => MapOp::InsertObjects(l.iter().map(obj).collect()),
        Command::RemoveObjects(l) => MapOp::RemoveObjects(l.iter().map(obj).collect()),
        Command::EditObjects { before, after } => MapOp::EditObjects {
            before: before.iter().map(obj).collect(),
            after: after.iter().map(obj).collect(),
        },
        Command::InsertTiming(l) => MapOp::InsertTiming(l.iter().map(line).collect()),
        Command::RemoveTiming(l) => MapOp::RemoveTiming(l.iter().map(line).collect()),
        Command::EditTiming { before, after } => MapOp::EditTiming {
            before: before.iter().map(line).collect(),
            after: after.iter().map(line).collect(),
        },
        Command::Gimmick { before, after } => MapOp::Gimmick {
            before: serde_json::to_string(&**before).unwrap_or_default(),
            after: serde_json::to_string(&**after).unwrap_or_default(),
        },
        Command::SetKey {
            section,
            key,
            before,
            after,
        } => MapOp::SetKey {
            section: (*section).into(),
            key: (*key).into(),
            before: before.clone(),
            after: after.clone(),
        },
        Command::Composite(l) => MapOp::Composite(l.iter().map(to_wire).collect()),
    }
}

/// The command an op from the wire is, if it is one this game can apply.
pub fn from_wire(op: &MapOp) -> Result<Command, String> {
    Ok(match op {
        MapOp::InsertObjects(l) => Command::InsertObjects(objects(l)?),
        MapOp::RemoveObjects(l) => Command::RemoveObjects(objects(l)?),
        MapOp::EditObjects { before, after } => Command::EditObjects {
            before: objects(before)?,
            after: objects(after)?,
        },
        MapOp::InsertTiming(l) => Command::InsertTiming(points(l)?),
        MapOp::RemoveTiming(l) => Command::RemoveTiming(points(l)?),
        MapOp::EditTiming { before, after } => Command::EditTiming {
            before: points(before)?,
            after: points(after)?,
        },
        MapOp::Gimmick { before, after } => Command::Gimmick {
            before: gimmick(before)?,
            after: gimmick(after)?,
        },
        MapOp::SetKey {
            section,
            key,
            before,
            after,
        } => {
            let (section, key) = mapping::known_key(section, key)
                .ok_or_else(|| format!("{section}/{key} isn't a setting an op may change"))?;
            Command::SetKey {
                section,
                key,
                before: before.clone(),
                after: after.clone(),
            }
        }
        MapOp::Composite(l) => {
            Command::Composite(l.iter().map(from_wire).collect::<Result<_, _>>()?)
        }
    })
}

// ---- applying ----

/// Applies `cmd` as a session does: an insert of an id already there is passed over, and so is
/// anything about an id that is gone (`Command::apply` already passes over those).
pub fn apply_tolerant(cmd: &Command, c: &mut Chart) {
    match cmd {
        Command::InsertObjects(l) => {
            for o in l {
                if c.object(o.id).is_none() {
                    c.insert_object(o.clone());
                }
            }
        }
        Command::InsertTiming(l) => {
            for p in l {
                if c.timing_point(p.id).is_none() {
                    c.insert_timing(p.clone());
                }
            }
        }
        Command::Composite(l) => l.iter().for_each(|x| apply_tolerant(x, c)),
        Command::Gimmick { before, after } => {
            // A chart with no gimmick yet takes one as from an empty gimmick.
            let now = c.gimmick.clone().unwrap_or_default();
            if let Some(g) = merged_gimmick(&now, before, after, false) {
                c.gimmick = Some(g);
                c.dirty = true;
            }
        }
        other => other.apply(c),
    }
}

/// A gimmick change made on `before` to `after`, applied to `now`, which others may have
/// changed since: what the change touched goes in and the rest stays as `now` has it, so two
/// players keying different effects at once both keep theirs. Where both changed the same
/// value, `theirs_win` keeps `now`'s (an undo leaves others' work alone), otherwise the change
/// wins (ops in the server's order: the same result in every game). `None`: nothing to change.
pub fn merged_gimmick(
    now: &Gimmick,
    before: &Gimmick,
    after: &Gimmick,
    theirs_win: bool,
) -> Option<Gimmick> {
    if before == after {
        return None;
    }
    if now == before {
        return Some(after.clone());
    }
    let value = |g: &Gimmick| serde_json::to_value(g).ok();
    let (Some(n), Some(b), Some(a)) = (value(now), value(before), value(after)) else {
        return (!theirs_win).then(|| after.clone());
    };
    let mut g: Gimmick = serde_json::from_value(merge3(n, &b, &a, theirs_win)).ok()?;
    g.sanitize();
    (g != *now).then_some(g)
}

/// Three-way merge of JSON values: `now` with the change from `before` to `after` put in.
/// Objects merge key by key; arrays of the same length element by element; keyframe lists
/// (objects with a time `t`) as sets of keys, so keys added by two players are both kept.
fn merge3(now: Value, before: &Value, after: &Value, theirs_win: bool) -> Value {
    if before == after {
        return now;
    }
    if now == *before {
        return after.clone();
    }
    if now == *after {
        return now;
    }
    match (now, before, after) {
        (Value::Object(mut n), Value::Object(b), Value::Object(a)) => {
            let keys: std::collections::BTreeSet<&String> = b.keys().chain(a.keys()).collect();
            for k in keys {
                match (b.get(k), a.get(k)) {
                    (None, None) => {}
                    (x, y) if x == y => {}
                    (Some(x), None) => {
                        // Taken out by the change, unless changed meanwhile and theirs wins.
                        if !(theirs_win && n.get(k).is_some_and(|v| v != x)) {
                            n.remove(k);
                        }
                    }
                    (x, Some(y)) => {
                        let merged = match n.remove(k) {
                            // Made by both at once (a first key on one effect, say): as
                            // if it had been there empty.
                            Some(cur) => {
                                let empty = match y {
                                    Value::Object(_) => Value::Object(Default::default()),
                                    Value::Array(_) => Value::Array(Vec::new()),
                                    _ => Value::Null,
                                };
                                merge3(cur, x.unwrap_or(&empty), y, theirs_win)
                            }
                            None if theirs_win && x.is_some() => continue,
                            None => y.clone(),
                        };
                        n.insert(k.clone(), merged);
                    }
                }
            }
            Value::Object(n)
        }
        (Value::Array(n), Value::Array(b), Value::Array(a))
            if n.len() == b.len() && b.len() == a.len() =>
        {
            Value::Array(
                n.into_iter()
                    .zip(b.iter().zip(a))
                    .map(|(n, (b, a))| merge3(n, b, a, theirs_win))
                    .collect(),
            )
        }
        (Value::Array(mut n), Value::Array(b), Value::Array(a)) if keyframes(b, a, &n) => {
            n.retain(|x| a.contains(x) || !b.contains(x));
            for x in a.iter().filter(|x| !b.contains(x)) {
                if !n.contains(x) {
                    n.push(x.clone());
                }
            }
            // Sorted by time; one key to a moment, the change's where both have one.
            let time = |x: &Value| x.get("t").and_then(Value::as_f64).unwrap_or(0.0);
            n.sort_by(|x, y| time(x).total_cmp(&time(y)));
            let mut out: Vec<Value> = Vec::with_capacity(n.len());
            for x in n {
                match out.last_mut() {
                    Some(last) if time(last) == time(&x) => {
                        if a.contains(&x) != theirs_win {
                            *last = x;
                        }
                    }
                    _ => out.push(x),
                }
            }
            Value::Array(out)
        }
        (now, _, after) => {
            if theirs_win {
                now
            } else {
                after.clone()
            }
        }
    }
}

/// Whether these are lists of keyframes: every element an object with a time `t`.
fn keyframes(b: &[Value], a: &[Value], n: &[Value]) -> bool {
    let key = |x: &Value| x.get("t").is_some_and(Value::is_number);
    let all = b.iter().chain(a).chain(n);
    all.clone().next().is_some() && all.into_iter().all(key)
}

fn same_object(a: &HitObject, b: &HitObject) -> bool {
    a.fake == b.fake && a.group == b.group && a.to_line("") == b.to_line("")
}

/// Applies the part of `cmd` that finds the chart as `cmd` expects it (what it inserts absent,
/// what it removes or edits as it was), and returns that part; `None` when nothing was left.
/// That is a session's undo and redo: what others changed since is theirs and stays.
pub fn apply_guarded(cmd: &Command, c: &mut Chart) -> Option<Command> {
    let made = match cmd {
        Command::InsertObjects(l) => kept(
            l.iter()
                .filter(|o| c.object(o.id).is_none())
                .cloned()
                .collect(),
        )
        .map(Command::InsertObjects),
        Command::RemoveObjects(l) => kept(
            l.iter()
                .filter(|o| c.object(o.id).is_some_and(|now| same_object(now, o)))
                .cloned()
                .collect(),
        )
        .map(Command::RemoveObjects),
        Command::EditObjects { before, after } => {
            let (mut b, mut a) = (Vec::new(), Vec::new());
            for x in after {
                let Some(was) = before.iter().find(|o| o.id == x.id) else {
                    continue;
                };
                if c.object(x.id).is_some_and(|now| same_object(now, was)) {
                    b.push(was.clone());
                    a.push(x.clone());
                }
            }
            kept(a).map(|after| Command::EditObjects { before: b, after })
        }
        Command::InsertTiming(l) => kept(
            l.iter()
                .filter(|p| c.timing_point(p.id).is_none())
                .cloned()
                .collect(),
        )
        .map(Command::InsertTiming),
        Command::RemoveTiming(l) => kept(
            l.iter()
                .filter(|p| c.timing_point(p.id).is_some_and(|now| !now.differs_from(p)))
                .cloned()
                .collect(),
        )
        .map(Command::RemoveTiming),
        Command::EditTiming { before, after } => {
            let (mut b, mut a) = (Vec::new(), Vec::new());
            for x in after {
                let Some(was) = before.iter().find(|p| p.id == x.id) else {
                    continue;
                };
                if c.timing_point(x.id)
                    .is_some_and(|now| !now.differs_from(was))
                {
                    b.push(was.clone());
                    a.push(x.clone());
                }
            }
            kept(a).map(|after| Command::EditTiming { before: b, after })
        }
        Command::Gimmick { before, after } => {
            let now = c.gimmick.clone().unwrap_or_default();
            merged_gimmick(&now, before, after, true).map(|g| Command::Gimmick {
                before: Box::new(now.clone()),
                after: Box::new(g),
            })
        }
        Command::SetKey {
            section,
            key,
            before,
            ..
        } => (c.get_key(section, key) == *before).then(|| cmd.clone()),
        Command::Composite(l) => {
            // Each part is applied as it is guarded, so the next one sees what it left.
            let parts: Vec<Command> = l.iter().filter_map(|x| apply_guarded(x, c)).collect();
            return kept(parts).map(Command::Composite);
        }
    };
    if let Some(m) = &made {
        m.apply(c);
    }
    made
}

fn kept<T>(v: Vec<T>) -> Option<Vec<T>> {
    (!v.is_empty()).then_some(v)
}

/// The red lines `cmd` inserts, removes or edits: the set's shared timing, which goes to every
/// difficulty.
pub fn red_part(cmd: &Command) -> Option<Command> {
    let reds = |l: &[TimingPoint]| -> Option<Vec<TimingPoint>> {
        let v: Vec<_> = l.iter().filter(|p| p.is_red()).cloned().collect();
        (!v.is_empty()).then_some(v)
    };
    match cmd {
        Command::InsertTiming(l) => reds(l).map(Command::InsertTiming),
        Command::RemoveTiming(l) => reds(l).map(Command::RemoveTiming),
        Command::EditTiming { before, after } => {
            let red: HashSet<u64> = before
                .iter()
                .chain(after)
                .filter(|p| p.is_red())
                .map(|p| p.id)
                .collect();
            let pick = |l: &[TimingPoint]| -> Vec<TimingPoint> {
                l.iter().filter(|p| red.contains(&p.id)).cloned().collect()
            };
            (!red.is_empty()).then(|| Command::EditTiming {
                before: pick(before),
                after: pick(after),
            })
        }
        Command::Composite(l) => {
            let v: Vec<Command> = l.iter().filter_map(red_part).collect();
            (!v.is_empty()).then_some(Command::Composite(v))
        }
        _ => None,
    }
}

/// The inserted objects' ids.
fn inserted(cmd: &Command, out: &mut Vec<u64>) {
    match cmd {
        Command::InsertObjects(l) => out.extend(l.iter().map(|o| o.id)),
        Command::Composite(l) => l.iter().for_each(|x| inserted(x, out)),
        _ => {}
    }
}

// ---- snapshots ----

/// The snapshot a host starts a session with: each difficulty's file (name, text), its timing
/// points and objects numbered from 1 up. A red line found the same in several difficulties
/// takes one id in all of them, so the host's timing changes reach them all as one op.
pub fn host_snapshot(files: &[(String, String)]) -> Result<SnapshotData, String> {
    let mut next = 1u64;
    let mut shared: HashMap<String, u64> = HashMap::new();
    let mut docs = Vec::new();
    for (diff, (name, text)) in files.iter().enumerate() {
        let chart = Chart::parse_file(text)?;
        let mut used = HashSet::new();
        let mut timing_ids = Vec::new();
        for p in &chart.timing {
            let same = p
                .is_red()
                .then(|| shared.get(&p.to_line("")).copied())
                .flatten();
            let id = match same {
                Some(id) if used.insert(id) => id,
                _ => {
                    let id = next;
                    next += 1;
                    used.insert(id);
                    if p.is_red() {
                        shared.entry(p.to_line("")).or_insert(id);
                    }
                    id
                }
            };
            timing_ids.push(id);
        }
        let object_ids: Vec<u64> = (0..chart.objects.len() as u64).map(|i| next + i).collect();
        next += chart.objects.len() as u64;
        docs.push(DocSnapshot {
            diff: diff as u32,
            name: name.clone(),
            text: text.clone(),
            timing_ids,
            object_ids,
            authors: Vec::new(),
        });
    }
    if next >= 1 << mapping::EPOCH_SHIFT {
        return Err("the map set is too big for a session".into());
    }
    Ok(SnapshotData { docs })
}

fn load(d: &DocSnapshot) -> Result<Chart, String> {
    let mut c = Chart::parse_file(&d.text)?;
    c.set_ids(&d.timing_ids, &d.object_ids)?;
    Ok(c)
}

type ObjectKey = (String, bool, Option<u16>);

fn object_key(o: &HitObject) -> ObjectKey {
    (o.to_line(""), o.fake, o.group)
}

/// `c` as a snapshot: its text, and its ids in the order a parse of that text holds them.
fn doc_snapshot(diff: u32, name: &str, c: &Chart) -> Result<DocSnapshot, String> {
    let text = c.to_text();
    let read = Chart::parse_file(&text)?;
    let mut timing: HashMap<String, VecDeque<u64>> = HashMap::new();
    for p in &c.timing {
        timing.entry(p.to_line("")).or_default().push_back(p.id);
    }
    let mut objects: HashMap<ObjectKey, VecDeque<u64>> = HashMap::new();
    for o in &c.objects {
        objects.entry(object_key(o)).or_default().push_back(o.id);
    }
    let lost = || format!("difficulty {name} doesn't read back as it is");
    let timing_ids = read
        .timing
        .iter()
        .map(|p| timing.get_mut(&p.to_line("")).and_then(VecDeque::pop_front))
        .collect::<Option<Vec<u64>>>()
        .ok_or_else(lost)?;
    let object_ids = read
        .objects
        .iter()
        .map(|o| {
            objects
                .get_mut(&object_key(o))
                .and_then(VecDeque::pop_front)
        })
        .collect::<Option<Vec<u64>>>()
        .ok_or_else(lost)?;
    Ok(DocSnapshot {
        diff,
        name: name.to_string(),
        text,
        timing_ids,
        object_ids,
        authors: Vec::new(),
    })
}

// ---- the session ----

struct Pending {
    client_seq: u64,
    diff: Option<u32>,
    cmd: Command,
    sent: bool,
}

/// The server's order and ours don't meet: the game resumes the session (snapshot and tail).
#[derive(Debug, PartialEq, Eq)]
pub struct OutOfStep;

pub struct Sync {
    me: u32,
    epoch: u32,
    /// The difficulty the editor has open.
    open: u32,
    docs: BTreeMap<u32, Chart>,
    names: BTreeMap<u32, String>,
    pending: VecDeque<Pending>,
    next_client_seq: u64,
    last_seq: u64,
    /// Who made each object, by (difficulty, id).
    authors: BTreeMap<(u32, u64), u32>,
    last_send: f64,
}

impl Sync {
    /// Joins at `seq` with `data`, as participant `me` on its join `epoch`, editing `open`: the
    /// session, and the editor on that difficulty (saving to `path`).
    pub fn join(
        data: &SnapshotData,
        seq: u64,
        me: u32,
        epoch: u32,
        open: u32,
        path: PathBuf,
    ) -> Result<(Sync, Editor), String> {
        let mut s = Sync {
            me,
            epoch,
            open,
            docs: BTreeMap::new(),
            names: BTreeMap::new(),
            pending: VecDeque::new(),
            next_client_seq: 1,
            last_seq: seq,
            authors: BTreeMap::new(),
            last_send: f64::NEG_INFINITY,
        };
        s.load(data)?;
        let mut local = s
            .docs
            .get(&open)
            .ok_or("no such difficulty in the session")?
            .clone();
        local.set_next_id(mapping::first_id(epoch));
        let mut ed = Editor::new(local, path);
        ed.history.share();
        Ok((s, ed))
    }

    fn load(&mut self, data: &SnapshotData) -> Result<(), String> {
        let mut docs = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut authors = BTreeMap::new();
        for d in &data.docs {
            docs.insert(d.diff, load(d)?);
            names.insert(d.diff, d.name.clone());
            authors.extend(d.authors.iter().map(|&(id, who)| ((d.diff, id), who)));
        }
        self.docs = docs;
        self.names = names;
        self.authors = authors;
        Ok(())
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// The difficulty the editor has open.
    pub fn open(&self) -> u32 {
        self.open
    }

    /// Who made an object of a difficulty, if the session knows.
    pub fn author(&self, diff: u32, id: u64) -> Option<u32> {
        self.authors.get(&(diff, id)).copied()
    }

    /// Who made a chart of a difficulty, by how many of its objects, most first (ties by
    /// participant). Objects the session has no maker for are `host`'s (the set's own).
    pub fn credits(&self, diff: u32, chart: &Chart, host: u32) -> Vec<(u32, usize)> {
        let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
        for o in &chart.objects {
            *counts
                .entry(self.author(diff, o.id).unwrap_or(host))
                .or_default() += 1;
        }
        let mut out: Vec<(u32, usize)> = counts.into_iter().collect();
        out.sort_by_key(|&(who, n)| (std::cmp::Reverse(n), who));
        out
    }

    /// Every difficulty's confirmed chart and name, but the open one.
    pub fn others(&self) -> impl Iterator<Item = (u32, &str, &Chart)> {
        self.docs
            .iter()
            .filter(|(d, _)| **d != self.open)
            .map(|(d, c)| (*d, self.names.get(d).map_or("", String::as_str), c))
    }

    /// Gives a difficulty's confirmed chart a gimmick, as if it were a `.tko`.
    #[cfg(test)]
    pub fn set_gimmick_for_tests(&mut self, diff: u32, g: crate::tko::gimmick::Gimmick) {
        if let Some(c) = self.docs.get_mut(&diff) {
            c.gimmick = Some(g);
        }
    }

    /// The confirmed chart of a difficulty: everything the server ordered so far.
    #[cfg(test)]
    pub fn confirmed(&self, diff: u32) -> Option<&Chart> {
        self.docs.get(&diff)
    }

    /// Own ops not yet back from the server.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Takes the editor's new edits into the pending ops. A drag step still unsent folds into
    /// the one before it. A red line change of a set with several difficulties also goes to
    /// all of them.
    pub fn collect(&mut self, ed: &mut Editor) {
        for cmd in ed.history.take_outgoing() {
            let red = if self.docs.len() > 1 {
                red_part(&cmd)
            } else {
                None
            };
            if red.is_none()
                && let Some(last) = self.pending.back_mut()
                && !last.sent
                && last.diff == Some(self.open)
                && let Some(m) = last.cmd.merged(&cmd)
            {
                last.cmd = m;
                continue;
            }
            let diff = Some(self.open);
            self.push(diff, cmd);
            if let Some(r) = red {
                self.push(None, r);
            }
        }
    }

    fn push(&mut self, diff: Option<u32>, cmd: Command) {
        self.pending.push_back(Pending {
            client_seq: self.next_client_seq,
            diff,
            cmd,
            sent: false,
        });
        self.next_client_seq += 1;
    }

    /// What to send now (`now` in ms). While `dragging`, the newest op waits until
    /// `DRAG_SEND_MS` after the last send, so a drag goes out a few times a second.
    pub fn outgoing(&mut self, now: f64, dragging: bool) -> Vec<MapUp> {
        let n = self.pending.len();
        let held = dragging && now - self.last_send < DRAG_SEND_MS;
        let mut out = Vec::new();
        for (i, p) in self.pending.iter_mut().enumerate() {
            if p.sent {
                continue;
            }
            if held && i + 1 == n {
                break;
            }
            p.sent = true;
            out.push(MapUp::Op {
                client_seq: p.client_seq,
                diff: p.diff,
                op: to_wire(&p.cmd),
            });
        }
        if !out.is_empty() {
            self.last_send = now;
        }
        out
    }

    fn touches_open(&self, diff: Option<u32>) -> bool {
        diff.is_none_or(|d| d == self.open)
    }

    /// An op in the server's order: into the confirmed charts, and into the editor's.
    pub fn receive(&mut self, ed: &mut Editor, s: &Sequenced) -> Result<(), OutOfStep> {
        self.collect(ed);
        if s.seq <= self.last_seq {
            return Ok(());
        }
        if s.seq != self.last_seq + 1 {
            return Err(OutOfStep);
        }
        self.last_seq = s.seq;
        // An op this game can't read is passed over by every game alike.
        let cmd = from_wire(&s.op).ok();
        if let Some(cmd) = &cmd {
            let mut ids = Vec::new();
            inserted(cmd, &mut ids);
            for (&diff, chart) in &mut self.docs {
                if s.diff.is_some_and(|d| d != diff) {
                    continue;
                }
                apply_tolerant(cmd, chart);
                for &id in &ids {
                    self.authors.entry((diff, id)).or_insert(s.author);
                }
            }
        }
        if s.author == self.me
            && s.epoch == self.epoch
            && let Some(i) = self
                .pending
                .iter()
                .position(|p| p.client_seq == s.client_seq)
        {
            self.pending.remove(i);
            if i == 0 {
                // Ours, next in line: the editor's chart holds it already, on the same chart.
                return Ok(());
            }
            self.rebuild(ed);
            return Ok(());
        }
        if !self.touches_open(s.diff) {
            return Ok(());
        }
        let ours_on_top = self.pending.iter().any(|p| self.touches_open(p.diff));
        match &cmd {
            _ if ours_on_top => self.rebuild(ed),
            Some(cmd) => {
                apply_tolerant(cmd, &mut ed.chart);
                Self::refresh(ed);
            }
            None => {}
        }
        Ok(())
    }

    /// The server refused one of our ops (`why` is shown): it is taken out of the editor's chart.
    pub fn refused(&mut self, ed: &mut Editor, client_seq: u64) {
        self.collect(ed);
        if let Some(i) = self.pending.iter().position(|p| p.client_seq == client_seq) {
            self.pending.remove(i);
            self.rebuild(ed);
        }
    }

    /// Back after a lost connection: the server's snapshot at `seq`, every op after it, and the
    /// last of our ops it had taken (`done`, by `client_seq`: taken in, or refused). Those are
    /// in the snapshot or the tail already, or were never to be; the rest of our pending ops
    /// stay on top and are sent again.
    pub fn resync(
        &mut self,
        ed: &mut Editor,
        data: &SnapshotData,
        seq: u64,
        tail: &[Sequenced],
        done: u64,
    ) -> Result<(), String> {
        self.collect(ed);
        self.load(data)?;
        self.last_seq = seq;
        self.pending.retain(|p| p.client_seq > done);
        for p in &mut self.pending {
            p.sent = false;
        }
        for s in tail {
            self.receive(ed, s)
                .map_err(|_| "the server's ops don't follow its snapshot".to_string())?;
        }
        self.rebuild(ed);
        Ok(())
    }

    /// Back on a resumed connection that the server answered with `done`, the last of our ops
    /// it had taken: those are in its log already (and come back with the ops we missed) or
    /// were refused, so they leave the pending list. The rest are sent again.
    pub fn resumed(&mut self, ed: &mut Editor, done: u64) {
        self.collect(ed);
        self.pending.retain(|p| p.client_seq > done);
        for p in &mut self.pending {
            p.sent = false;
        }
        self.rebuild(ed);
    }

    /// The editor's chart made again: the confirmed one with our pending ops on top.
    fn rebuild(&self, ed: &mut Editor) {
        let Some(base) = self.docs.get(&self.open) else {
            return;
        };
        let next = ed.chart.next_id();
        let mut c = base.clone();
        for p in self.pending.iter().filter(|p| self.touches_open(p.diff)) {
            apply_tolerant(&p.cmd, &mut c);
        }
        // Our own range of ids, not the confirmed chart's (which holds everyone's).
        c.set_next_id(next);
        ed.chart = c;
        Self::refresh(ed);
    }

    fn refresh(ed: &mut Editor) {
        ed.history.touch();
        let chart = &ed.chart;
        ed.selection.retain(|id| chart.object(*id).is_some());
    }

    /// The confirmed state as a snapshot, at `last_seq` (what the server asks for to shorten
    /// its log).
    pub fn snapshot(&self) -> Result<SnapshotData, String> {
        let mut docs = Vec::new();
        for (&diff, c) in &self.docs {
            let name = self.names.get(&diff).map_or("", String::as_str);
            let mut d = doc_snapshot(diff, name, c)?;
            d.authors = c
                .objects
                .iter()
                .filter_map(|o| self.author(diff, o.id).map(|who| (o.id, who)))
                .collect();
            docs.push(d);
        }
        Ok(SnapshotData { docs })
    }
}

#[cfg(test)]
mod tests;
