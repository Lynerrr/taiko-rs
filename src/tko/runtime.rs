//! A gimmick at play time: what the playfield and each note are doing at a moment.
//!
//! Functions of map time, the file and the play's own events (what was hit or missed, and when),
//! so a replay looks the same every time. Nothing here is read by judging: the game only tells
//! the runtime what happened.

use std::sync::Arc;

use std::collections::HashMap;

use super::gimmick::{FieldProp, Gimmick, NoteProp};
use super::graph::{Kind, MAX_STEPS, MAX_TRIGGERS_PER_NODE, Node, Port, Target};

/// What the whole playfield is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldFx {
    pub x: f32,
    pub y: f32,
    /// Degrees clockwise.
    pub rotation: f32,
    pub zoom: f32,
    pub scroll: f64,
    pub stretch_x: f32,
    pub stretch_y: f32,
    pub flash: f32,
    pub vignette: f32,
    pub wash: f32,
    /// Degrees.
    pub wash_hue: f32,
    pub glitch: f32,
    pub chroma: f32,
    /// Trail copies: how many (the fraction is the last one's strength), and what each step back
    /// does.
    pub echo: f32,
    pub echo_x: f32,
    pub echo_y: f32,
    pub echo_rot: f32,
    pub echo_zoom: f32,
    pub echo_fade: f32,
    pub kaleido: f32,
    /// The graph's seed, for the glitch's randomness.
    pub seed: u64,
}

impl Default for FieldFx {
    fn default() -> Self {
        FieldFx {
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            zoom: 1.0,
            scroll: 1.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
            flash: 0.0,
            vignette: 0.0,
            wash: 0.0,
            wash_hue: 0.0,
            glitch: 0.0,
            chroma: 0.0,
            echo: 0.0,
            echo_x: FieldProp::EchoX.default_value() as f32,
            echo_y: 0.0,
            echo_rot: 0.0,
            echo_zoom: 1.0,
            echo_fade: FieldProp::EchoFade.default_value() as f32,
            kaleido: 0.0,
            seed: 0,
        }
    }
}

/// What one note is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteFx {
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    pub opacity: f32,
    pub hidden: bool,
    /// Degrees about the drum.
    pub swirl: f32,
}

impl Default for NoteFx {
    fn default() -> Self {
        NoteFx {
            dx: 0.0,
            dy: 0.0,
            scale: 1.0,
            opacity: 1.0,
            hidden: false,
            swirl: 0.0,
        }
    }
}

/// A stretch of the map with one beat grid and one kiai state, on the game's clock (what a
/// `beatmap::ControlPoint` says).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatSection {
    pub start: f64,
    pub origin: f64,
    pub length: f64,
    pub kiai: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Great,
    Ok,
    Miss,
}

/// What the player did, as the graph's play events see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayEvent {
    /// Chart time of the judgement.
    pub time: f64,
    /// Index of the note, in the map's order.
    pub note: usize,
    pub verdict: Verdict,
    /// Combo after it.
    pub combo: u32,
    /// Accuracy so far, 0..1.
    pub accuracy: f64,
}

/// More than this many judgements aren't kept (a map has far fewer notes).
const MAX_EVENTS: usize = 100_000;
const MAX_DEPTH: usize = 512;
/// With no beat grid a beat is this long.
const FALLBACK_BEAT_MS: f64 = 500.0;
/// How long a shake holds one random offset.
const SHAKE_STEP_MS: f64 = 33.0;

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A number in 0..1 from a seed and two keys: the graph's only source of randomness.
pub fn hash01(seed: u64, a: u64, b: u64) -> f64 {
    let h = mix(seed ^ mix(a ^ mix(b.wrapping_add(0x51))));
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// What one evaluation spends and remembers.
struct Eval {
    memo: Option<Vec<f64>>,
    steps: usize,
    depth: usize,
}

impl Eval {
    /// For one moment, many reads: each node is worked out once.
    fn frame(nodes: usize) -> Eval {
        Eval {
            memo: Some(vec![f64::NAN; nodes]),
            steps: 0,
            depth: 0,
        }
    }

    /// For a read at some other moment.
    fn bare() -> Eval {
        Eval {
            memo: None,
            steps: 0,
            depth: 0,
        }
    }
}

/// Nodes with their inputs before them; nodes in a loop (a file that wasn't sanitized) are left
/// out.
fn topological(nodes: &[Node], index: &HashMap<u32, usize>) -> Vec<usize> {
    let mut pending: Vec<usize> = nodes
        .iter()
        .map(|n| {
            n.inputs
                .iter()
                .flatten()
                .filter(|id| index.contains_key(id))
                .count()
        })
        .collect();
    let mut fed: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for id in n.inputs.iter().flatten() {
            if let Some(&j) = index.get(id) {
                fed[j].push(i);
            }
        }
    }
    let mut ready: Vec<usize> = (0..nodes.len()).filter(|&i| pending[i] == 0).collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(i) = ready.pop() {
        order.push(i);
        for &k in &fed[i] {
            pending[k] -= 1;
            if pending[k] == 0 {
                ready.push(k);
            }
        }
    }
    order
}

/// A gimmick bound to the notes of one play.
#[derive(Clone, Debug)]
pub struct Runtime {
    gimmick: Arc<Gimmick>,
    /// Per note (in the map's order): the group it is in, plus one; 0 for none.
    group_of: Vec<u16>,
    /// How far the game's clock is ahead of the chart's (the game shifts every time of a loaded
    /// map; a gimmick's times are the chart's).
    shift: f64,
    sections: Vec<BeatSection>,
    /// Where beat events stop: a little past the last note.
    end: f64,
    /// Node index by id.
    index: HashMap<u32, usize>,
    /// Nodes with inputs before the nodes they feed.
    order: Vec<usize>,
    /// Per node: depends on what the player does.
    dynamic: Vec<bool>,
    /// Per node: the moments of a trigger node (empty for the rest), ascending, chart time.
    lists: Vec<Vec<f64>>,
    events: Vec<PlayEvent>,
    dirty: bool,
    /// What the graph adds to each playfield property, and to each group's, at the last
    /// `prepare`.
    field_delta: [f64; FieldProp::ALL.len()],
    group_delta: Vec<[f64; NoteProp::ALL.len()]>,
}

/// How far a note's time may be from the one a group holds (the chart's whole milliseconds).
const SAME_NOTE_MS: f64 = 0.5;

fn field_slot(p: FieldProp) -> usize {
    FieldProp::ALL.iter().position(|q| *q == p).unwrap_or(0)
}

fn note_slot(p: NoteProp) -> usize {
    NoteProp::ALL.iter().position(|q| *q == p).unwrap_or(0)
}

impl Runtime {
    /// `note_times`: the map's notes, in order, on the game's clock, which is `shift` ms ahead of
    /// the chart's. Without beat sections the graph's beat and kiai events never fire.
    #[cfg(test)]
    pub fn new(
        gimmick: Arc<Gimmick>,
        note_times: impl Iterator<Item = f64>,
        shift: f64,
    ) -> Runtime {
        Runtime::with_beats(gimmick, note_times, shift, &[])
    }

    /// `beats`: the map's timing, on the game's clock.
    pub fn with_beats(
        gimmick: Arc<Gimmick>,
        note_times: impl Iterator<Item = f64>,
        shift: f64,
        beats: &[BeatSection],
    ) -> Runtime {
        let times: Vec<f64> = note_times.map(|t| t - shift).collect();
        let end = times.iter().copied().fold(0.0, f64::max) + 5000.0;
        let group_of = times
            .iter()
            .map(|&t| {
                gimmick
                    .groups
                    .iter()
                    .position(|g| {
                        let at = g.notes.partition_point(|&n| n < t - SAME_NOTE_MS);
                        g.notes
                            .get(at)
                            .is_some_and(|&n| (n - t).abs() <= SAME_NOTE_MS)
                    })
                    .map_or(0, |i| i as u16 + 1)
            })
            .collect();
        let sections: Vec<BeatSection> = beats
            .iter()
            .filter(|b| b.length.is_finite() && b.length > 0.0)
            .map(|b| BeatSection {
                start: b.start - shift,
                origin: b.origin - shift,
                ..*b
            })
            .collect();
        let nodes = &gimmick.graph.nodes;
        let index: HashMap<u32, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let order = topological(nodes, &index);
        let mut dynamic = vec![false; nodes.len()];
        for &i in &order {
            let n = &nodes[i];
            dynamic[i] = n.kind.is_play_event()
                || n.kind.reads_play_state()
                || n.inputs
                    .iter()
                    .flatten()
                    .any(|id| index.get(id).is_some_and(|&j| dynamic[j]));
        }
        let groups = gimmick.groups.len();
        let mut rt = Runtime {
            lists: vec![Vec::new(); nodes.len()],
            gimmick,
            group_of,
            shift,
            sections,
            end,
            index,
            order,
            dynamic,
            events: Vec::new(),
            dirty: false,
            field_delta: Default::default(),
            group_delta: vec![Default::default(); groups],
        };
        rt.rebuild(false);
        rt
    }

    #[cfg(test)]
    pub fn gimmick(&self) -> &Gimmick {
        &self.gimmick
    }

    // ---- the beat grid ----

    fn section_at(&self, t: f64) -> Option<&BeatSection> {
        let at = self.sections.partition_point(|s| s.start <= t);
        self.sections.get(at.checked_sub(1)?)
    }

    /// Beats since the grid's origin (or since 0, with no grid).
    fn beats_at(&self, t: f64) -> f64 {
        match self.section_at(t) {
            Some(s) => (t - s.origin) / s.length,
            None => t / FALLBACK_BEAT_MS,
        }
    }

    // ---- moments ----

    /// Works the moments of every trigger node out again: all of them, or only those that depend
    /// on what the player did.
    fn rebuild(&mut self, only_dynamic: bool) {
        for k in 0..self.order.len() {
            let i = self.order[k];
            if self.gimmick.graph.nodes[i].kind.output() != Some(Port::Trigger)
                || (only_dynamic && !self.dynamic[i])
            {
                continue;
            }
            self.lists[i] = self.build(i);
        }
    }

    fn in_list(&self, n: &Node, port: usize) -> &[f64] {
        n.inputs
            .get(port)
            .copied()
            .flatten()
            .and_then(|id| self.index.get(&id))
            .map_or(&[][..], |&j| &self.lists[j])
    }

    fn in_group(&self, e: &PlayEvent, group: usize) -> bool {
        group == 0 || self.group_of(e.note) == Some(group - 1)
    }

    /// The moments of the trigger node at `idx`, from its kind and what feeds it.
    fn build(&self, idx: usize) -> Vec<f64> {
        let n = &self.gimmick.graph.nodes[idx];
        let mut out: Vec<f64> = match n.kind {
            Kind::OnStart => vec![0.0],
            Kind::OnTime => vec![n.param(0)],
            Kind::OnBeat => self.beat_moments(n.param(0).max(1.0) as i64, n.param(1) as i64),
            Kind::OnKiaiStart | Kind::OnKiaiEnd => {
                let want = n.kind == Kind::OnKiaiStart;
                let mut was = false;
                let mut out = Vec::new();
                for s in &self.sections {
                    if s.kiai != was && s.kiai == want {
                        out.push(s.start);
                    }
                    was = s.kiai;
                }
                out
            }
            Kind::OnNoteReach => (n.param(0) as usize)
                .checked_sub(1)
                .and_then(|g| self.gimmick.groups.get(g))
                .map_or_else(Vec::new, |g| g.notes.clone()),
            Kind::OnHit => {
                let (group, judge) = (n.param(0) as usize, n.param(1) as u8);
                self.events
                    .iter()
                    .filter(|e| {
                        e.verdict != Verdict::Miss
                            && match judge {
                                1 => e.verdict == Verdict::Great,
                                2 => e.verdict == Verdict::Ok,
                                _ => true,
                            }
                            && self.in_group(e, group)
                    })
                    .map(|e| e.time)
                    .collect()
            }
            Kind::OnMiss => {
                let group = n.param(0) as usize;
                self.events
                    .iter()
                    .filter(|e| e.verdict == Verdict::Miss && self.in_group(e, group))
                    .map(|e| e.time)
                    .collect()
            }
            Kind::OnCombo => {
                let every = n.param(0).max(1.0) as u32;
                self.events
                    .iter()
                    .filter(|e| e.verdict != Verdict::Miss && e.combo > 0 && e.combo % every == 0)
                    .map(|e| e.time)
                    .collect()
            }
            Kind::Delay => self.in_list(n, 0).iter().map(|t| t + n.param(0)).collect(),
            Kind::Counter => {
                let every = n.param(0).max(1.0) as usize;
                self.in_list(n, 0)
                    .iter()
                    .skip(every - 1)
                    .step_by(every)
                    .copied()
                    .collect()
            }
            Kind::Once => self.in_list(n, 0).first().copied().into_iter().collect(),
            Kind::Gate => {
                let open = n
                    .inputs
                    .get(1)
                    .copied()
                    .flatten()
                    .and_then(|id| self.index.get(&id).copied());
                self.in_list(n, 0)
                    .iter()
                    .copied()
                    .filter(|&t| open.is_some_and(|j| self.value(j, t, &mut Eval::bare()) >= 0.5))
                    .collect()
            }
            Kind::Merge => {
                let mut all: Vec<f64> = self
                    .in_list(n, 0)
                    .iter()
                    .chain(self.in_list(n, 1))
                    .copied()
                    .collect();
                all.sort_by(f64::total_cmp);
                all
            }
            _ => Vec::new(),
        };
        out.retain(|t| t.is_finite());
        out.truncate(MAX_TRIGGERS_PER_NODE);
        out
    }

    /// Every `every`th beat (starting at beat `offset`) of the map's timing.
    fn beat_moments(&self, every: i64, offset: i64) -> Vec<f64> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.sections.len() {
            // A run of sections sharing a beat grid: a green line doesn't restart the beats.
            let s = self.sections[i];
            let mut j = i + 1;
            while j < self.sections.len()
                && self.sections[j].origin == s.origin
                && self.sections[j].length == s.length
            {
                j += 1;
            }
            let stop = self.sections.get(j).map_or(self.end, |e| e.start);
            let mut k = ((s.start - s.origin) / s.length).ceil() as i64;
            loop {
                let t = s.origin + k as f64 * s.length;
                if t >= stop || out.len() >= MAX_TRIGGERS_PER_NODE {
                    break;
                }
                if t >= s.start && (k - offset).rem_euclid(every) == 0 {
                    out.push(t);
                }
                k += 1;
            }
            i = j;
        }
        out
    }

    // ---- play events ----

    /// The player judged note `note` at game time `time`; `combo` and `accuracy` are as they
    /// stand after it.
    pub fn event(&mut self, time: f64, note: usize, verdict: Verdict, combo: u32, accuracy: f64) {
        if self.events.len() >= MAX_EVENTS || !self.dynamic.iter().any(|&d| d) {
            return;
        }
        let e = PlayEvent {
            time: time - self.shift,
            note,
            verdict,
            combo,
            accuracy,
        };
        let at = self.events.partition_point(|x| x.time <= e.time);
        self.events.insert(at, e);
        self.dirty = true;
    }

    // ---- values ----

    fn last_event(&self, t: f64) -> Option<&PlayEvent> {
        let at = self.events.partition_point(|e| e.time <= t);
        self.events.get(at.checked_sub(1)?)
    }

    /// The last moment of `list` at or before `t`, and how many there are up to it.
    fn last_in(list: &[f64], t: f64) -> (Option<f64>, usize) {
        let at = list.partition_point(|&m| m <= t);
        (at.checked_sub(1).map(|i| list[i]), at)
    }

    fn input_value(&self, n: &Node, port: usize, t: f64, ev: &mut Eval) -> Option<f64> {
        let j = n
            .inputs
            .get(port)
            .copied()
            .flatten()
            .and_then(|id| self.index.get(&id).copied())?;
        Some(self.value(j, t, ev))
    }

    /// The number node `idx` gives at chart time `t`.
    fn value(&self, idx: usize, t: f64, ev: &mut Eval) -> f64 {
        if ev.steps >= MAX_STEPS || ev.depth >= MAX_DEPTH {
            return 0.0;
        }
        if let Some(&v) = ev.memo.as_ref().map(|m| &m[idx]).filter(|v| !v.is_nan()) {
            return v;
        }
        ev.steps += 1;
        ev.depth += 1;
        let n = &self.gimmick.graph.nodes[idx];
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let v = match n.kind {
            Kind::Const => n.param(0),
            Kind::Time => t / 1000.0,
            Kind::BeatPhase => self.beats_at(t).rem_euclid(1.0),
            Kind::Combo => self.last_event(t).map_or(0.0, |e| f64::from(e.combo)),
            Kind::Accuracy => self.last_event(t).map_or(1.0, |e| e.accuracy),
            Kind::Random => {
                let slot = (self.beats_at(t) / n.param(1).max(0.25)).floor() as i64;
                hash01(self.gimmick.graph.seed, n.param(0) as u64, slot as u64)
            }
            Kind::Wave => {
                let turns = self.beats_at(t) / n.param(0).max(0.25);
                (turns * std::f64::consts::TAU).sin() * n.param(1)
            }
            Kind::Toggle => flag(Self::last_in(self.in_list(n, 0), t).1 % 2 == 1),
            Kind::Envelope => match Self::last_in(self.in_list(n, 0), t).0 {
                Some(m) if t - m < n.param(0) => 1.0 - n.ease.apply((t - m) / n.param(0)),
                _ => 0.0,
            },
            Kind::Add => {
                self.input_value(n, 0, t, ev).unwrap_or(0.0)
                    + self.input_value(n, 1, t, ev).unwrap_or(0.0)
            }
            Kind::Mul => {
                self.input_value(n, 0, t, ev).unwrap_or(1.0)
                    * self.input_value(n, 1, t, ev).unwrap_or(1.0)
            }
            Kind::Compare => flag(
                self.input_value(n, 0, t, ev).unwrap_or(0.0)
                    > self.input_value(n, 1, t, ev).unwrap_or(0.0),
            ),
            Kind::Not => flag(self.input_value(n, 0, t, ev).unwrap_or(0.0) < 0.5),
            Kind::Clamp => {
                let (lo, hi) = (n.param(0), n.param(1));
                self.input_value(n, 0, t, ev)
                    .unwrap_or(0.0)
                    .clamp(lo.min(hi), lo.max(hi))
            }
            Kind::Map => self.input_value(n, 0, t, ev).unwrap_or(0.0) * n.param(0) + n.param(1),
            _ => 0.0,
        };
        let v = if v.is_finite() { v } else { 0.0 };
        ev.depth -= 1;
        if let Some(m) = &mut ev.memo {
            m[idx] = v;
        }
        v
    }

    // ---- frames ----

    /// Works out what the graph adds at game time `t`. Call it once a frame before `field` and
    /// `note`.
    pub fn prepare(&mut self, t: f64) {
        if self.dirty {
            self.dirty = false;
            self.rebuild(true);
        }
        let nodes = &self.gimmick.graph.nodes;
        let mut field = [0.0; FieldProp::ALL.len()];
        let mut groups = vec![[0.0; NoteProp::ALL.len()]; self.group_delta.len()];
        if !nodes.is_empty() {
            let t = t - self.shift;
            let mut ev = Eval::frame(nodes.len());
            let seed = self.gimmick.graph.seed;
            for &i in &self.order {
                let n = &nodes[i];
                if !n.kind.is_action() {
                    continue;
                }
                let (last, count) = Self::last_in(self.in_list(n, 0), t);
                let since = last.map(|m| t - m);
                let amount = match n.kind {
                    Kind::Drive => self.input_value(n, 0, t, &mut ev).unwrap_or(0.0) * n.param(0),
                    Kind::Pulse => match since {
                        Some(d) if d < n.param(1) => {
                            n.param(0) * (1.0 - n.ease.apply(d / n.param(1)))
                        }
                        _ => 0.0,
                    },
                    Kind::Tween => {
                        let to = |k: usize| if k % 2 == 1 { n.param(0) } else { 0.0 };
                        match since {
                            Some(d) => {
                                let x = n.ease.apply((d / n.param(1)).min(1.0));
                                to(count - 1) + (to(count) - to(count - 1)) * x
                            }
                            None => 0.0,
                        }
                    }
                    Kind::Clip => match (since, n.clip.keys.last()) {
                        (Some(d), Some(end)) if d <= end.t || n.param(0) >= 0.5 => {
                            n.clip.value(d, 0.0)
                        }
                        _ => 0.0,
                    },
                    Kind::Shake => {
                        if let Some(d) = since.filter(|d| *d < n.param(1)) {
                            let slot = ((t / SHAKE_STEP_MS).floor() as i64) as u64;
                            let fade = 1.0 - d / n.param(1);
                            let (a, id) = (n.param(0) * fade, u64::from(n.id));
                            field[field_slot(FieldProp::X)] +=
                                a * (hash01(seed, id * 2, slot) * 2.0 - 1.0);
                            field[field_slot(FieldProp::Y)] +=
                                a * (hash01(seed, id * 2 + 1, slot) * 2.0 - 1.0);
                        }
                        0.0
                    }
                    _ => 0.0,
                };
                match n.target {
                    Some(Target::Field(p)) => field[field_slot(p)] += amount,
                    Some(Target::Group(g, p)) => {
                        if let Some(row) = groups.get_mut(g) {
                            row[note_slot(p)] += amount;
                        }
                    }
                    None => {}
                }
            }
        }
        self.field_delta = field;
        self.group_delta = groups;
    }

    /// How long ago (ms) the trigger node `id` last fired, at game time `t`; for showing which
    /// nodes are alive.
    pub fn fired_ago(&self, id: u32, t: f64) -> Option<f64> {
        let list = self.lists.get(*self.index.get(&id)?)?;
        Self::last_in(list, t - self.shift)
            .0
            .map(|m| t - self.shift - m)
    }

    pub fn field(&self, t: f64) -> FieldFx {
        let t = t - self.shift;
        let v = |p: FieldProp| {
            let base = self
                .gimmick
                .field_track(p)
                .map_or(p.default_value(), |tr| tr.value(t, p.default_value()));
            let add = self.field_delta[field_slot(p)];
            if add == 0.0 {
                base
            } else {
                let (lo, hi) = p.range();
                (base + add).clamp(lo, hi)
            }
        };
        FieldFx {
            x: v(FieldProp::X) as f32,
            y: v(FieldProp::Y) as f32,
            rotation: v(FieldProp::Rotation) as f32,
            zoom: v(FieldProp::Zoom) as f32,
            scroll: v(FieldProp::Scroll),
            stretch_x: v(FieldProp::StretchX) as f32,
            stretch_y: v(FieldProp::StretchY) as f32,
            flash: v(FieldProp::Flash) as f32,
            vignette: v(FieldProp::Vignette) as f32,
            wash: v(FieldProp::Wash) as f32,
            wash_hue: v(FieldProp::WashHue) as f32,
            glitch: v(FieldProp::Glitch) as f32,
            chroma: v(FieldProp::Chroma) as f32,
            echo: v(FieldProp::Echo) as f32,
            echo_x: v(FieldProp::EchoX) as f32,
            echo_y: v(FieldProp::EchoY) as f32,
            echo_rot: v(FieldProp::EchoRot) as f32,
            echo_zoom: v(FieldProp::EchoZoom) as f32,
            echo_fade: v(FieldProp::EchoFade) as f32,
            kaleido: v(FieldProp::Kaleido) as f32,
            seed: self.gimmick.graph.seed,
        }
    }

    /// The group of note `i`, if any.
    pub fn group_of(&self, i: usize) -> Option<usize> {
        match self.group_of.get(i) {
            Some(&g) if g > 0 => Some(g as usize - 1),
            _ => None,
        }
    }

    /// What note `i` is doing at game time `t`; nothing for a note in no group.
    pub fn note(&self, i: usize, t: f64) -> Option<NoteFx> {
        let gi = self.group_of(i)?;
        let g = &self.gimmick.groups[gi];
        let t = t - self.shift;
        let v = |p: NoteProp| {
            let base = g
                .track(p)
                .map_or(p.default_value(), |tr| tr.value(t, p.default_value()));
            let add = self
                .group_delta
                .get(gi)
                .map_or(0.0, |row| row[note_slot(p)]);
            if add == 0.0 {
                base
            } else {
                let (lo, hi) = p.range();
                (base + add).clamp(lo, hi)
            }
        };
        Some(NoteFx {
            dx: v(NoteProp::Dx) as f32,
            dy: v(NoteProp::Dy) as f32,
            scale: v(NoteProp::Scale) as f32,
            opacity: v(NoteProp::Opacity) as f32,
            hidden: v(NoteProp::Hidden) >= 0.5,
            swirl: v(NoteProp::Swirl) as f32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tko::gimmick::Group;
    use crate::tko::track::{Ease, Key};

    fn key(t: f64, v: f64) -> Key {
        Key {
            t,
            v,
            ease: Ease::Linear,
        }
    }

    fn runtime() -> Runtime {
        let mut g = Gimmick::default();
        g.key_field(FieldProp::Rotation, key(1000.0, 0.0));
        g.key_field(FieldProp::Rotation, key(2000.0, 90.0));
        g.groups.push(Group {
            name: "a".into(),
            notes: vec![1500.0, 2500.0],
            ..Group::default()
        });
        g.key_note(0, NoteProp::Opacity, key(1000.0, 1.0));
        g.key_note(0, NoteProp::Opacity, key(2000.0, 0.0));
        // Before its first key a track holds its first value, so a switch needs a key at 0.
        g.key_note(
            0,
            NoteProp::Hidden,
            Key {
                ease: Ease::Step,
                ..key(0.0, 0.0)
            },
        );
        g.key_note(0, NoteProp::Hidden, key(2400.0, 1.0));
        g.sanitize();
        Runtime::new(
            Arc::new(g),
            [500.0, 1500.0, 2000.0, 2500.0].into_iter(),
            0.0,
        )
    }

    #[test]
    fn without_keys_the_field_is_untouched() {
        let rt = Runtime::new(Arc::new(Gimmick::default()), [1.0].into_iter(), 0.0);
        assert_eq!(rt.field(1234.0), FieldFx::default());
        assert_eq!(rt.note(0, 1.0), None);
    }

    #[test]
    fn the_field_follows_its_keys() {
        let rt = runtime();
        assert_eq!(rt.field(0.0).rotation, 0.0);
        assert_eq!(rt.field(1500.0).rotation, 45.0);
        assert_eq!(rt.field(9000.0).rotation, 90.0);
        assert_eq!(
            rt.field(1500.0).zoom,
            1.0,
            "other properties keep their default"
        );
    }

    #[test]
    fn the_new_properties_follow_their_keys_and_default_to_nothing() {
        let plain = FieldFx::default();
        assert_eq!(
            (
                plain.flash,
                plain.vignette,
                plain.glitch,
                plain.chroma,
                plain.echo,
                plain.kaleido
            ),
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        );
        assert_eq!((plain.stretch_x, plain.stretch_y), (1.0, 1.0));
        let mut g = Gimmick::default();
        g.key_field(FieldProp::Flash, key(0.0, 0.0));
        g.key_field(FieldProp::Flash, key(1000.0, 1.0));
        g.key_field(FieldProp::Echo, key(0.0, 99.0));
        g.key_field(FieldProp::EchoFade, key(0.0, 0.25));
        g.key_field(FieldProp::StretchX, key(0.0, 2.0));
        g.groups.push(Group {
            notes: vec![1000.0],
            ..Group::default()
        });
        g.key_note(0, NoteProp::Swirl, key(0.0, 30.0));
        g.sanitize();
        let rt = Runtime::new(Arc::new(g), [1000.0].into_iter(), 0.0);
        let f = rt.field(500.0);
        assert_eq!(f.flash, 0.5);
        assert_eq!(f.echo, 8.0, "held inside what the property allows");
        assert_eq!((f.echo_fade, f.stretch_x, f.echo_x), (0.25, 2.0, -30.0));
        assert_eq!(rt.note(0, 0.0).unwrap().swirl, 30.0);
    }

    #[test]
    fn notes_are_found_by_their_time_and_only_those_in_a_group_animate() {
        let rt = runtime();
        assert_eq!(rt.group_of(0), None);
        assert_eq!(rt.group_of(1), Some(0));
        assert_eq!(rt.group_of(2), None);
        assert_eq!(rt.group_of(3), Some(0));
        assert_eq!(rt.note(1, 1500.0).unwrap().opacity, 0.5);
        assert!(!rt.note(1, 2000.0).unwrap().hidden);
        assert!(rt.note(3, 2600.0).unwrap().hidden);
        assert_eq!(rt.note(0, 1500.0), None);
    }

    #[test]
    fn the_games_clock_is_shifted_back_to_the_charts() {
        let mut g = Gimmick::default();
        g.key_field(FieldProp::X, key(1000.0, 10.0));
        g.key_field(FieldProp::X, key(2000.0, 20.0));
        g.groups.push(Group {
            notes: vec![1000.0],
            ..Group::default()
        });
        let rt = Runtime::new(Arc::new(g), [1023.0].into_iter(), 23.0);
        assert_eq!(rt.group_of(0), Some(0));
        assert_eq!(rt.field(1523.0).x, 15.0);
    }

    #[test]
    fn two_evaluations_agree() {
        let (a, b) = (runtime(), runtime());
        for i in 0..200 {
            let t = i as f64 * 37.0;
            assert_eq!(a.field(t), b.field(t));
            assert_eq!(a.note(1, t), b.note(1, t));
        }
    }

    // ---- the graph ----

    use crate::tko::graph::{Graph, Target};

    /// Adds a node with parameters, a target and what feeds it.
    fn add(g: &mut Graph, kind: Kind, p: &[f64], target: Option<Target>, from: &[u32]) -> u32 {
        let id = g.add(kind, 0.0, 0.0).unwrap();
        let n = g.node_mut(id).unwrap();
        n.p = p.to_vec();
        n.target = target;
        n.ease = Ease::Linear;
        for (port, &f) in from.iter().enumerate() {
            g.connect(f, id, port).unwrap();
        }
        id
    }

    fn zoom() -> Option<Target> {
        Some(Target::Field(FieldProp::Zoom))
    }

    const GRID: [BeatSection; 1] = [BeatSection {
        start: 0.0,
        origin: 0.0,
        length: 500.0,
        kiai: false,
    }];

    fn with(build: impl FnOnce(&mut Graph), beats: &[BeatSection]) -> Runtime {
        let mut gm = Gimmick::default();
        gm.groups.push(Group {
            notes: vec![1000.0, 3000.0],
            ..Group::default()
        });
        build(&mut gm.graph);
        if gm.graph.seed == 0 {
            gm.graph.seed = 7;
        }
        gm.sanitize();
        Runtime::with_beats(
            Arc::new(gm),
            [1000.0, 2000.0, 3000.0].into_iter(),
            0.0,
            beats,
        )
    }

    fn zoom_at(rt: &mut Runtime, t: f64) -> f32 {
        rt.prepare(t);
        rt.field(t).zoom
    }

    #[test]
    fn a_beat_pulse_kicks_the_zoom_and_settles() {
        let mut rt = with(
            |g| {
                let b = add(g, Kind::OnBeat, &[1.0, 0.0], None, &[]);
                add(g, Kind::Pulse, &[0.5, 200.0], zoom(), &[b]);
            },
            &GRID,
        );
        assert_eq!(zoom_at(&mut rt, 1000.0), 1.5);
        assert_eq!(zoom_at(&mut rt, 1100.0), 1.25);
        assert_eq!(zoom_at(&mut rt, 1300.0), 1.0);
        assert_eq!(zoom_at(&mut rt, 1500.0), 1.5, "the next beat");
    }

    #[test]
    fn every_nth_and_offset_pick_beats_and_a_green_line_does_not_restart_them() {
        let grid = [
            GRID[0],
            BeatSection {
                start: 1750.0,
                ..GRID[0]
            },
        ];
        let rt = with(
            |g| {
                add(g, Kind::OnBeat, &[2.0, 1.0], None, &[]);
            },
            &grid,
        );
        let id = rt.gimmick().graph.nodes[0].id;
        assert_eq!(rt.fired_ago(id, 1600.0), Some(100.0), "beat 3 at 1500");
        assert_eq!(rt.fired_ago(id, 2600.0), Some(100.0), "beat 5 at 2500");
        assert_eq!(rt.fired_ago(id, 2400.0), Some(900.0), "none at 2000");
        let moments = &rt.lists[0];
        assert!(moments.iter().all(|m| (m - 500.0) % 1000.0 == 0.0));
    }

    #[test]
    fn a_new_red_line_moves_the_beats() {
        let grid = [
            GRID[0],
            BeatSection {
                start: 1000.0,
                origin: 1100.0,
                length: 400.0,
                kiai: false,
            },
        ];
        let rt = with(
            |g| {
                add(g, Kind::OnBeat, &[1.0, 0.0], None, &[]);
            },
            &grid,
        );
        let l = &rt.lists[0];
        assert!(l.contains(&500.0) && !l.contains(&1000.0));
        assert!(l.contains(&1100.0) && l.contains(&1500.0) && !l.contains(&1250.0));
        assert!(l.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn kiai_events_fire_at_the_sections_that_change_it() {
        let grid = [
            GRID[0],
            BeatSection {
                start: 1000.0,
                kiai: true,
                ..GRID[0]
            },
            BeatSection {
                start: 2000.0,
                ..GRID[0]
            },
        ];
        let rt = with(
            |g| {
                add(g, Kind::OnKiaiStart, &[], None, &[]);
                add(g, Kind::OnKiaiEnd, &[], None, &[]);
            },
            &grid,
        );
        assert_eq!(rt.lists[0], [1000.0]);
        assert_eq!(rt.lists[1], [2000.0]);
    }

    #[test]
    fn start_time_note_arrival_delay_counter_once_and_merge_shape_moments() {
        let rt = with(
            |g| {
                let start = add(g, Kind::OnStart, &[], None, &[]);
                let at = add(g, Kind::OnTime, &[1500.0], None, &[]);
                let reach = add(g, Kind::OnNoteReach, &[1.0], None, &[]);
                add(g, Kind::Delay, &[250.0], None, &[reach]);
                add(g, Kind::Counter, &[2.0], None, &[reach]);
                add(g, Kind::Once, &[], None, &[reach]);
                add(g, Kind::Merge, &[], None, &[start, at]);
            },
            &GRID,
        );
        assert_eq!(rt.lists[0], [0.0]);
        assert_eq!(rt.lists[1], [1500.0]);
        assert_eq!(rt.lists[2], [1000.0, 3000.0]);
        assert_eq!(rt.lists[3], [1250.0, 3250.0]);
        assert_eq!(rt.lists[4], [3000.0]);
        assert_eq!(rt.lists[5], [1000.0]);
        assert_eq!(rt.lists[6], [0.0, 1500.0]);
    }

    #[test]
    fn a_wave_drives_a_property_and_a_scale_reshapes_it() {
        let mut rt = with(
            |g| {
                let w = add(g, Kind::Wave, &[1.0, 1.0], None, &[]);
                let m = add(g, Kind::Map, &[0.5, 1.0], None, &[w]);
                add(g, Kind::Drive, &[0.2], zoom(), &[m]);
            },
            &GRID,
        );
        // At a quarter of a beat the sine is 1: (1 * 0.5 + 1) * 0.2 = 0.3.
        assert!((zoom_at(&mut rt, 125.0) - 1.3).abs() < 1e-6);
        assert!((zoom_at(&mut rt, 375.0) - 1.1).abs() < 1e-6);
    }

    #[test]
    fn math_nodes_compute_and_a_missing_wire_reads_as_nothing() {
        let mut rt = with(
            |g| {
                let a = add(g, Kind::Const, &[3.0], None, &[]);
                let b = add(g, Kind::Const, &[4.0], None, &[]);
                let sum = add(g, Kind::Add, &[], None, &[a, b]);
                let prod = add(g, Kind::Mul, &[], None, &[a, b]);
                let gt = add(g, Kind::Compare, &[], None, &[sum, prod]);
                let not = add(g, Kind::Not, &[], None, &[gt]);
                let clamped = add(g, Kind::Clamp, &[0.0, 5.0], None, &[prod]);
                // 7 > 12 is 0, not is 1, clamp(12) is 5.
                let total = add(g, Kind::Add, &[], None, &[not, clamped]);
                add(g, Kind::Drive, &[0.1], zoom(), &[total]);
                // A drive with nothing wired adds nothing.
                add(g, Kind::Drive, &[5.0], zoom(), &[]);
            },
            &GRID,
        );
        assert!((zoom_at(&mut rt, 10.0) - 1.6).abs() < 1e-6);
    }

    #[test]
    fn a_tween_flips_between_nothing_and_its_target() {
        let mut rt = with(
            |g| {
                let b = add(g, Kind::OnBeat, &[2.0, 0.0], None, &[]);
                add(g, Kind::Tween, &[1.0, 100.0], zoom(), &[b]);
            },
            &GRID,
        );
        assert_eq!(zoom_at(&mut rt, 0.0), 1.0, "the start of the first flip");
        assert_eq!(zoom_at(&mut rt, 50.0), 1.5);
        assert_eq!(zoom_at(&mut rt, 500.0), 2.0, "held until the next flip");
        assert_eq!(zoom_at(&mut rt, 1050.0), 1.5, "and back");
        assert_eq!(zoom_at(&mut rt, 1900.0), 1.0);
        assert_eq!(zoom_at(&mut rt, 2100.0), 2.0);
    }

    #[test]
    fn a_toggle_gates_triggers_and_an_envelope_fades() {
        let mut rt = with(
            |g| {
                let flip = add(g, Kind::OnTime, &[1000.0], None, &[]);
                let tog = add(g, Kind::Toggle, &[], None, &[flip]);
                let beat = add(g, Kind::OnBeat, &[1.0, 0.0], None, &[]);
                let gate = add(g, Kind::Gate, &[], None, &[beat, tog]);
                add(g, Kind::Pulse, &[1.0, 100.0], zoom(), &[gate]);
                let env = add(g, Kind::Envelope, &[200.0], None, &[beat]);
                add(
                    g,
                    Kind::Drive,
                    &[0.5],
                    Some(Target::Field(FieldProp::X)),
                    &[env],
                );
            },
            &GRID,
        );
        // Gate closed before 1000: the pulse does not play at beat 1 (500).
        assert_eq!(zoom_at(&mut rt, 550.0), 1.0);
        // Open from 1000 on: it plays at 1500 (the beat at 1000 is the flip's moment, and the
        // flip counts at that same moment, so it is open for it too).
        assert_eq!(zoom_at(&mut rt, 1000.0), 2.0);
        assert_eq!(zoom_at(&mut rt, 1550.0), 1.5);
        // The envelope: 1 at the beat, 0 after 200 ms.
        rt.prepare(1100.0);
        assert!((rt.field(1100.0).x - 0.25).abs() < 1e-5);
        rt.prepare(1300.0);
        assert_eq!(rt.field(1300.0).x, 0.0);
    }

    #[test]
    fn a_clip_plays_relative_to_each_trigger() {
        let mut rt = with(
            |g| {
                let r = add(g, Kind::OnNoteReach, &[1.0], None, &[]);
                let c = add(g, Kind::Clip, &[0.0], zoom(), &[r]);
                g.node_mut(c).unwrap().clip = crate::tko::track::Track {
                    keys: vec![key(0.0, 0.0), key(100.0, 1.0), key(200.0, 0.0)],
                };
            },
            &GRID,
        );
        assert_eq!(zoom_at(&mut rt, 900.0), 1.0, "before the first trigger");
        assert_eq!(zoom_at(&mut rt, 1050.0), 1.5);
        assert_eq!(zoom_at(&mut rt, 1150.0), 1.5);
        assert_eq!(zoom_at(&mut rt, 1500.0), 1.0, "over");
        assert_eq!(zoom_at(&mut rt, 3100.0), 2.0, "again, for the second note");
    }

    #[test]
    fn a_shake_is_bounded_fades_and_repeats_the_same_way() {
        let build = |g: &mut Graph| {
            let b = add(g, Kind::OnBeat, &[1.0, 0.0], None, &[]);
            add(g, Kind::Shake, &[20.0, 300.0], None, &[b]);
        };
        let (mut a, mut b) = (with(build, &GRID), with(build, &GRID));
        let mut moved = false;
        for i in 0..300 {
            let t = i as f64 * 7.0;
            a.prepare(t);
            b.prepare(t);
            assert_eq!(a.field(t), b.field(t));
            let f = a.field(t);
            assert!(f.x.abs() <= 20.0 && f.y.abs() <= 20.0);
            if t % 500.0 >= 300.0 {
                assert_eq!((f.x, f.y), (0.0, 0.0), "still after it fades, t={t}");
            }
            moved |= f.x != 0.0;
        }
        assert!(moved);
        let mut other = with(
            |g| {
                build(g);
                g.seed = 99;
            },
            &GRID,
        );
        let mut differs = false;
        for i in 0..100 {
            let t = i as f64 * 7.0;
            a.prepare(t);
            other.prepare(t);
            differs |= a.field(t) != other.field(t);
        }
        assert!(differs, "another seed shakes another way");
    }

    #[test]
    fn random_depends_on_its_seed_and_the_beat_only() {
        let rt = |seed: f64| {
            with(
                |g| {
                    let r = add(g, Kind::Random, &[seed, 1.0], None, &[]);
                    add(g, Kind::Drive, &[1.0], zoom(), &[r]);
                },
                &GRID,
            )
        };
        let (mut a, mut b, mut c) = (rt(1.0), rt(1.0), rt(2.0));
        let mut seen = std::collections::HashSet::new();
        let mut differs = false;
        for beat in 0..40 {
            let t = beat as f64 * 500.0 + 10.0;
            let (za, zb, zc) = (zoom_at(&mut a, t), zoom_at(&mut b, t), zoom_at(&mut c, t));
            assert_eq!(za, zb);
            assert_eq!(za, zoom_at(&mut a, t + 400.0), "constant inside a beat");
            assert!((1.0..2.0).contains(&za));
            differs |= za != zc;
            seen.insert(za.to_bits());
        }
        assert!(differs && seen.len() > 20);
    }

    #[test]
    fn play_events_drive_hits_misses_combo_and_accuracy() {
        let mut rt = with(
            |g| {
                let hit = add(g, Kind::OnHit, &[0.0, 1.0], None, &[]);
                add(g, Kind::Pulse, &[0.5, 200.0], zoom(), &[hit]);
                let miss = add(g, Kind::OnMiss, &[1.0], None, &[]);
                add(
                    g,
                    Kind::Pulse,
                    &[1.0, 200.0],
                    Some(Target::Field(FieldProp::X)),
                    &[miss],
                );
                let combo = add(g, Kind::Combo, &[], None, &[]);
                add(
                    g,
                    Kind::Drive,
                    &[1.0],
                    Some(Target::Field(FieldProp::Y)),
                    &[combo],
                );
                let acc = add(g, Kind::Accuracy, &[], None, &[]);
                add(
                    g,
                    Kind::Drive,
                    &[10.0],
                    Some(Target::Field(FieldProp::Rotation)),
                    &[acc],
                );
            },
            &GRID,
        );
        rt.prepare(500.0);
        assert_eq!(rt.field(500.0).zoom, 1.0);
        assert_eq!(rt.field(500.0).y, 0.0);
        assert_eq!(
            rt.field(500.0).rotation,
            10.0,
            "no notes yet: perfect accuracy"
        );
        // A great on note 0, then a miss on note 1 (not in group 1) and on note 2 (in it).
        rt.event(1000.0, 0, Verdict::Great, 1, 1.0);
        rt.event(2000.0, 1, Verdict::Miss, 0, 0.5);
        rt.event(3000.0, 2, Verdict::Miss, 0, 0.33);
        rt.prepare(1050.0);
        assert_eq!(rt.field(1050.0).zoom, 1.375);
        assert_eq!(rt.field(1050.0).y, 1.0);
        rt.prepare(2050.0);
        assert_eq!(
            rt.field(2050.0).x,
            0.0,
            "the miss was of a note outside group 1"
        );
        assert_eq!(rt.field(2050.0).y, 0.0, "combo reset");
        assert!((rt.field(2050.0).rotation - 5.0).abs() < 1e-5);
        rt.prepare(3050.0);
        assert_eq!(rt.field(3050.0).x, 0.75);
        // Looking back at an earlier moment still sees what was true then.
        rt.prepare(1050.0);
        assert_eq!(rt.field(1050.0).y, 1.0);
    }

    #[test]
    fn combo_milestones_and_ok_hits_are_told_apart() {
        let mut rt = with(
            |g| {
                let m = add(g, Kind::OnCombo, &[2.0], None, &[]);
                add(g, Kind::Pulse, &[1.0, 100.0], zoom(), &[m]);
                let ok = add(g, Kind::OnHit, &[0.0, 2.0], None, &[]);
                add(
                    g,
                    Kind::Pulse,
                    &[1.0, 100.0],
                    Some(Target::Field(FieldProp::X)),
                    &[ok],
                );
            },
            &GRID,
        );
        rt.event(1000.0, 0, Verdict::Ok, 1, 0.8);
        rt.event(2000.0, 1, Verdict::Great, 2, 0.9);
        rt.prepare(1000.0);
        assert_eq!((rt.field(1000.0).zoom, rt.field(1000.0).x), (1.0, 1.0));
        rt.prepare(2000.0);
        assert_eq!((rt.field(2000.0).zoom, rt.field(2000.0).x), (2.0, 0.0));
    }

    #[test]
    fn the_games_clock_shifts_events_too() {
        let mut gm = Gimmick::default();
        let hit = add(&mut gm.graph, Kind::OnHit, &[0.0, 0.0], None, &[]);
        add(&mut gm.graph, Kind::Pulse, &[1.0, 100.0], zoom(), &[hit]);
        let mut rt = Runtime::new(Arc::new(gm), [1023.0].into_iter(), 23.0);
        rt.event(1023.0, 0, Verdict::Great, 1, 1.0);
        rt.prepare(1023.0);
        assert_eq!(rt.field(1023.0).zoom, 2.0);
        rt.prepare(1123.0);
        assert_eq!(rt.field(1123.0).zoom, 1.0);
    }

    #[test]
    fn what_the_graph_adds_stays_inside_the_property_range() {
        let mut rt = with(
            |g| {
                let c = add(g, Kind::Const, &[1000.0], None, &[]);
                add(g, Kind::Drive, &[1000.0], zoom(), &[c]);
                add(
                    g,
                    Kind::Drive,
                    &[-1000.0],
                    Some(Target::Field(FieldProp::Scroll)),
                    &[c],
                );
            },
            &GRID,
        );
        rt.prepare(0.0);
        assert_eq!(rt.field(0.0).zoom, FieldProp::Zoom.range().1 as f32);
        assert_eq!(rt.field(0.0).scroll, FieldProp::Scroll.range().0);
    }

    #[test]
    fn a_group_target_moves_only_that_groups_notes() {
        let mut rt = with(
            |g| {
                let b = add(g, Kind::OnStart, &[], None, &[]);
                add(
                    g,
                    Kind::Pulse,
                    &[40.0, 1000.0],
                    Some(Target::Group(0, NoteProp::Dx)),
                    &[b],
                );
            },
            &GRID,
        );
        rt.prepare(0.0);
        assert_eq!(rt.note(0, 0.0).unwrap().dx, 40.0);
        assert_eq!(rt.note(1, 0.0), None, "a note in no group");
        assert_eq!(rt.note(2, 0.0).unwrap().dx, 40.0);
        rt.prepare(1000.0);
        assert_eq!(rt.note(0, 1000.0).unwrap().dx, 0.0);
    }

    #[test]
    fn an_unprepared_runtime_changes_nothing_and_an_empty_graph_costs_nothing() {
        let rt = with(
            |g| {
                let b = add(g, Kind::OnStart, &[], None, &[]);
                add(g, Kind::Pulse, &[1.0, 1000.0], zoom(), &[b]);
            },
            &GRID,
        );
        let untouched = FieldFx {
            seed: 0,
            ..rt.field(10.0)
        };
        assert_eq!(untouched, FieldFx::default());
        let mut empty = Runtime::new(Arc::new(Gimmick::default()), [1.0].into_iter(), 0.0);
        empty.event(1.0, 0, Verdict::Great, 1, 1.0);
        assert!(empty.events.is_empty(), "nothing listens, nothing is kept");
        empty.prepare(5.0);
        assert_eq!(empty.field(5.0), FieldFx::default());
    }

    #[test]
    fn the_caps_hold_for_a_hostile_graph() {
        let t0 = std::time::Instant::now();
        let mut rt = with(
            |g| {
                // A beat every ms for a long map, run through a chain of delays.
                let b = add(g, Kind::OnBeat, &[1.0, 0.0], None, &[]);
                let mut prev = b;
                for _ in 0..50 {
                    prev = add(g, Kind::Delay, &[1.0], None, &[prev]);
                }
                add(g, Kind::Pulse, &[1.0, 1000.0], zoom(), &[prev]);
                // A diamond ladder: without the per-frame memo this is 2^1000 reads.
                let mut v = add(g, Kind::Const, &[1.0], None, &[]);
                for _ in 0..1000 {
                    v = add(g, Kind::Add, &[], None, &[v, v]);
                }
                add(
                    g,
                    Kind::Drive,
                    &[1.0],
                    Some(Target::Field(FieldProp::X)),
                    &[v],
                );
                // The same ladder behind a gate is read at each trigger without a memo.
                let flip = add(g, Kind::Toggle, &[], None, &[prev]);
                let gate = add(g, Kind::Gate, &[], None, &[b, flip]);
                add(g, Kind::Pulse, &[1.0, 10.0], zoom(), &[gate]);
            },
            &[BeatSection {
                length: 1.0,
                ..GRID[0]
            }],
        );
        for n in &rt.lists {
            assert!(n.len() <= MAX_TRIGGERS_PER_NODE);
        }
        for i in 0..20 {
            rt.prepare(i as f64 * 100.0);
            let f = rt.field(i as f64 * 100.0);
            assert!(f.x.is_finite() && f.zoom.is_finite());
        }
        assert!(t0.elapsed().as_secs() < 20, "took {:?}", t0.elapsed());
    }

    #[test]
    fn a_graph_that_still_has_a_loop_is_skipped_rather_than_run() {
        let mut gm = Gimmick::default();
        let a = add(&mut gm.graph, Kind::Delay, &[1.0], None, &[]);
        let b = add(&mut gm.graph, Kind::Delay, &[1.0], None, &[a]);
        // Wired by hand behind the editor's back and never sanitized.
        gm.graph.node_mut(a).unwrap().inputs = vec![Some(b)];
        let mut rt = Runtime::new(Arc::new(gm), [1.0].into_iter(), 0.0);
        rt.prepare(10.0);
        assert_eq!(rt.field(10.0), FieldFx::default());
    }
}
