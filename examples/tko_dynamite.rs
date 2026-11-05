//! Builds the showcase `.tko` for "USAO - Dynamite (Camellia's MACHO TNT REMIX) [kaBOOM]".
//!
//!     cargo run --release --example tko_dynamite -- "map.osu" [out.tko]
//!
//! Unlike `tko_showcase` (which dresses any taiko map the same way) this one first *reads* the
//! chart, then gives every part of the song its own treatment through the same API the editor
//! uses. What it found in the chart:
//!
//! - 222 BPM, one red line, **a green line on almost every note** (per-note slider velocity that
//!   makes each phrase surge and decay). The playfield's `scroll` is therefore never touched.
//! - Seven kiai windows (flickers merged) = seven "acts"; each opens with a drop.
//! - Long 1/4 streams (up to 33 notes). They get the *least* motion: a gentle sway at most.
//! - Clusters of finishers, short swells, and one 11 s break with no notes at all.
//!
//! The rule for playability: loud effects are short punches on events (a drop, a finisher, a
//! swell); what stays on while notes are dense is a faint wash, a breathing vignette, a one-degree
//! sway and a beat pulse of a couple of percent. Nothing changes when a note is hit or judged.

use std::{env, fs};
use taiko_rs::tko::format;
use taiko_rs::tko::gimmick::{FieldProp as F, Gimmick, Group, Mark, NoteProp as N};
use taiko_rs::tko::graph::{Graph, Kind, Target};
use taiko_rs::tko::recipes::Recipe;
use taiko_rs::tko::track::{Ease, Key};

// --- reading the chart ----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Type {
    Don,
    Kat,
    Roll,
    Swell,
}

#[derive(Clone)]
struct Note {
    time: f64,
    big: bool,
    kind: Type,
    /// Where a roll or swell ends.
    end: f64,
}

#[derive(Default)]
struct Map {
    beat: f64,
    kiai: Vec<(f64, bool)>,
    greens: usize,
    breaks: Vec<(f64, f64)>,
    notes: Vec<Note>,
}

fn section<'a>(text: &'a str, name: &str) -> impl Iterator<Item = &'a str> {
    let head = format!("[{name}]");
    text.lines()
        .map(str::trim)
        .skip_while(move |l| *l != head)
        .skip(1)
        .take_while(|l| !l.starts_with('['))
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
}

fn read(text: &str) -> Map {
    let mut m = Map::default();
    for l in section(text, "TimingPoints") {
        let f: Vec<&str> = l.split(',').collect();
        let (Some(t), Some(len), Some(effects)) = (
            f.first().and_then(|t| t.parse::<f64>().ok()),
            f.get(1).and_then(|t| t.parse::<f64>().ok()),
            f.get(7).and_then(|e| e.parse::<i32>().ok()),
        ) else {
            continue;
        };
        m.kiai.push((t, effects & 1 != 0));
        if f.get(6) == Some(&"1") {
            if m.beat == 0.0 {
                m.beat = len;
            }
        } else {
            m.greens += 1;
        }
    }
    for l in section(text, "Events") {
        let f: Vec<&str> = l.split(',').collect();
        if f.first() == Some(&"2")
            && let (Some(a), Some(b)) = (
                f.get(1).and_then(|t| t.parse::<f64>().ok()),
                f.get(2).and_then(|t| t.parse::<f64>().ok()),
            )
        {
            m.breaks.push((a, b));
        }
    }
    for l in section(text, "HitObjects") {
        let f: Vec<&str> = l.split(',').collect();
        let (Some(t), Some(kind), Some(sound)) = (
            f.get(2).and_then(|t| t.parse::<f64>().ok()),
            f.get(3).and_then(|t| t.parse::<i32>().ok()),
            f.get(4).and_then(|t| t.parse::<i32>().ok()),
        ) else {
            continue;
        };
        let (kind, end) = if kind & 8 != 0 {
            (
                Type::Swell,
                f.get(5).and_then(|e| e.parse().ok()).unwrap_or(t),
            )
        } else if kind & 2 != 0 {
            (Type::Roll, t + 1.0)
        } else if sound & 10 != 0 {
            (Type::Kat, t)
        } else {
            (Type::Don, t)
        };
        m.notes.push(Note {
            time: t,
            big: sound & 4 != 0 && matches!(kind, Type::Don | Type::Kat),
            kind,
            end,
        });
    }
    m.notes.sort_by(|a, b| a.time.total_cmp(&b.time));
    m
}

/// Kiai windows with the map's own on/off flicker (gaps under `merge` ms) closed up.
fn kiai_windows(m: &Map, merge: f64) -> Vec<(f64, f64)> {
    let last = m.notes.last().map_or(0.0, |n| n.end) + 800.0;
    let mut w: Vec<(f64, f64)> = Vec::new();
    let mut on: Option<f64> = None;
    for &(t, k) in &m.kiai {
        match (on, k) {
            (None, true) => on = Some(t),
            (Some(s), false) => {
                w.push((s, t));
                on = None;
            }
            _ => {}
        }
    }
    if let Some(s) = on {
        w.push((s, last));
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (s, e) in w {
        match out.last_mut() {
            Some(l) if s - l.1 < merge => l.1 = e,
            _ => out.push((s, e)),
        }
    }
    out
}

/// Consecutive hit notes closer than `gap` ms, at least `min` of them: `(first, last, count)`.
fn streams(m: &Map, gap: f64, min: usize) -> Vec<(f64, f64, usize)> {
    let hits: Vec<f64> = m
        .notes
        .iter()
        .filter(|n| matches!(n.kind, Type::Don | Type::Kat))
        .map(|n| n.time)
        .collect();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..=hits.len() {
        if i == hits.len() || hits[i] - hits[i - 1] > gap {
            if i - start >= min {
                out.push((hits[start], hits[i - 1], i - start));
            }
            start = i;
        }
    }
    out
}

/// Finishers that follow one another within `gap` ms, at least `min` of them.
fn finisher_clusters(m: &Map, gap: f64, min: usize) -> Vec<Vec<f64>> {
    let mut out: Vec<Vec<f64>> = Vec::new();
    for n in m.notes.iter().filter(|n| n.big) {
        match out.last_mut() {
            Some(c) if n.time - c.last().copied().unwrap_or(0.0) <= gap => c.push(n.time),
            _ => out.push(vec![n.time]),
        }
    }
    out.retain(|c| c.len() >= min);
    out
}

// --- what the chart does, as a plan ---------------------------------------------------------

/// How much of its opacity each step back in a trail keeps: faint enough that a ghost never
/// passes for a note.
const ECHO_FADE: f64 = 0.4;

/// The long stream that builds into an act's drop, if there is one just before it.
fn riser_of(streams: &[(f64, f64, usize)], a: &Act) -> Option<(f64, f64, usize)> {
    streams
        .iter()
        .rfind(|s| s.1 < a.start && a.start - s.1 < 4000.0)
        .copied()
}

/// One kiai window and what comes before it.
struct Act {
    start: f64,
    end: f64,
    /// Silence before the first note of the window (the drop is the longer the gap).
    gap: f64,
    /// 0..1: how big the drop is (from the length of the window and the notes it holds).
    power: f64,
    /// The colour of the window, a hue in degrees.
    hue: f64,
}

/// Times measured from the audio (a 0.5 s spectrum of the mp3), where the chart alone is silent:
/// the song lifts, dips and builds before its first drop.
struct Cues {
    lift: f64,
    verse: f64,
    dip: f64,
}

const CUES: Cues = Cues {
    lift: 16_000.0,
    verse: 20_000.0,
    dip: 36_000.0,
};

/// Hues the acts cycle through: ember, magenta, gold, cyan-hot, red, then white-gold for the end.
const HUES: [f64; 7] = [28.0, 330.0, 48.0, 190.0, 8.0, 285.0, 45.0];

// --- keyframe helpers -----------------------------------------------------------------------

fn key(g: &mut Gimmick, p: F, t: f64, v: f64, ease: Ease) {
    g.key_field(p, Key { t, v, ease });
}

/// `p` holds `from` until `start`, eases to `to` by `end` and stays there.
fn ramp(g: &mut Gimmick, p: F, start: f64, end: f64, from: f64, to: f64, ease: Ease) {
    key(g, p, start - 1.0, from, Ease::Step);
    key(g, p, start, from, ease);
    key(g, p, end, to, Ease::Step);
}

/// `p` is `v` from `start` to `end` and its default outside.
fn hold(g: &mut Gimmick, p: F, start: f64, end: f64, v: f64) {
    key(g, p, start - 1.0, p.default_value(), Ease::Step);
    key(g, p, start, v, Ease::Step);
    key(g, p, end, p.default_value(), Ease::Step);
}

/// `p` goes `from` -> `to` over a window and drops back to its default.
fn pass(g: &mut Gimmick, p: F, start: f64, end: f64, from: f64, to: f64, ease: Ease) {
    ramp(g, p, start, end, from, to, ease);
    key(g, p, end + 1.0, p.default_value(), Ease::Step);
}

// --- placing nodes --------------------------------------------------------------------------

/// Wired nodes laid out on a grid, one band per scene.
struct B<'a> {
    g: &'a mut Graph,
    x: f32,
    y: f32,
    row: f32,
    /// Where the current column of bands starts.
    col_x: f32,
}

/// A column of bands is at most this many rows tall; the next band that does not fit starts a new
/// column to the right, so the canvas is a few screens wide instead of one endless strip.
const COLUMN_ROWS: f32 = 56.0;
const COLUMN_WIDTH: f32 = 1500.0;

impl B<'_> {
    fn band(&mut self, rows: f32) {
        if self.row > 0.0 && self.row + rows > COLUMN_ROWS {
            self.col_x += COLUMN_WIDTH;
            self.row = 0.0;
        }
        self.x = self.col_x;
        self.y = self.row * 86.0;
        self.row += rows + 1.0;
    }

    fn n(
        &mut self,
        col: f32,
        row: f32,
        kind: Kind,
        p: &[f64],
        target: Option<Target>,
        from: &[u32],
    ) -> u32 {
        let id = self
            .g
            .add(kind, self.x + col * 190.0, self.y + row * 86.0)
            .expect("graph full");
        let node = self.g.node_mut(id).expect("just added");
        for (slot, v) in node.p.iter_mut().zip(p) {
            *slot = *v;
        }
        node.target = target;
        for (port, f) in from.iter().enumerate() {
            if *f != 0 {
                self.g.connect(*f, id, port).expect("wire");
            }
        }
        id
    }

    fn field(&mut self, col: f32, row: f32, kind: Kind, p: &[f64], f: F, from: &[u32]) -> u32 {
        self.n(col, row, kind, p, Some(Target::Field(f)), from)
    }

    /// A pulse with its own settling curve.
    #[allow(clippy::too_many_arguments)]
    fn pulse(
        &mut self,
        col: f32,
        row: f32,
        amount: f64,
        ms: f64,
        ease: Ease,
        f: F,
        from: &[u32],
    ) -> u32 {
        let id = self.field(col, row, Kind::Pulse, &[amount, ms], f, from);
        if let Some(n) = self.g.node_mut(id) {
            n.ease = ease;
        }
        id
    }
}

struct Marks(Vec<(String, f64)>);

impl Marks {
    /// 1-based, as the nodes number them.
    fn idx(&self, name: &str) -> f64 {
        self.0
            .iter()
            .position(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no mark {name}")) as f64
            + 1.0
    }
}

struct Report(Vec<String>);

impl Report {
    fn say(&mut self, s: String) {
        println!("{s}");
        self.0.push(s);
    }
}

fn secs(ms: f64) -> String {
    format!("{:.1}s", ms / 1000.0)
}

// --- the build ------------------------------------------------------------------------------

fn build(m: &Map, r: &mut Report) -> Gimmick {
    let beat = m.beat;
    let first = m.notes.first().map_or(0.0, |n| n.time);
    let last_note = m.notes.last().map_or(0.0, |n| n.time);
    let end = last_note + 900.0;

    // ---- analysis -----------------------------------------------------------------------
    let windows = kiai_windows(m, 1000.0);
    let long_streams = streams(m, beat / 3.5, 20);
    let all_streams = streams(m, beat / 3.5, 8);
    let clusters = finisher_clusters(m, beat * 2.0, 3);
    let swells: Vec<&Note> = m.notes.iter().filter(|n| n.kind == Type::Swell).collect();
    let brk = m
        .breaks
        .iter()
        .copied()
        .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)));
    let notes_in = |a: f64, b: f64| m.notes.iter().filter(|n| n.time >= a && n.time < b).count();
    let acts: Vec<Act> = windows
        .iter()
        .enumerate()
        .map(|(i, &(s, e))| {
            // The longest silence in the 3.5 s before the drop (a pause, then maybe a pick-up and a
            // swell, then the drop): the field holds its breath from the start of it.
            let near: Vec<&Note> = m
                .notes
                .iter()
                .filter(|n| n.end >= s - 3500.0 && n.end < s - 1.0 && n.kind != Type::Swell)
                .collect();
            let quiet = near
                .windows(2)
                .map(|w| (w[0].end, w[1].time - w[0].end))
                .chain(near.last().map(|n| (n.end, s - n.end)))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|q| q.1 >= 900.0);
            let before = quiet.map_or(s, |q| q.0);
            let len = e - s;
            let finishers = m
                .notes
                .iter()
                .filter(|n| n.big && n.time >= s - 3000.0 && n.time < e)
                .count();
            // A long window with finishers is a big drop; a short tease is a small one.
            let power = ((len / 14_000.0).min(1.0) * 0.6
                + (finishers as f64 / 12.0).min(1.0) * 0.4)
                .clamp(0.35, 1.0);
            Act {
                start: s,
                end: e,
                gap: s - before,
                power,
                hue: HUES[i % HUES.len()],
            }
        })
        .collect();
    r.say(format!(
        "chart: {} notes, beat {:.0} ms ({:.0} BPM), {} green lines, first note {}, last {}",
        m.notes.len(),
        beat,
        60_000.0 / beat,
        m.greens,
        secs(first),
        secs(last_note)
    ));
    r.say(format!(
        "found: {} kiai windows, {} streams >= 8 ({} long), {} finisher clusters, {} swells, break {:?}",
        acts.len(),
        all_streams.len(),
        long_streams.len(),
        clusters.len(),
        swells.len(),
        brk.map(|b| (secs(b.0), secs(b.1)))
    ));
    for (i, a) in acts.iter().enumerate() {
        r.say(format!(
            "  act {}: {}-{} ({:.1}s, {} notes), silence before {:.0} ms, drop power {:.2}, hue {}",
            i + 1,
            secs(a.start),
            secs(a.end),
            (a.end - a.start) / 1000.0,
            notes_in(a.start, a.end),
            a.gap,
            a.power,
            a.hue
        ));
    }

    let mut gm = Gimmick::default();
    // Trails go straight back, not sideways down the lane, so a ghost never lands where the next
    // note will be.
    key(&mut gm, F::EchoX, 0.0, 0.0, Ease::Step);
    key(&mut gm, F::EchoFade, 0.0, ECHO_FADE, Ease::Step);

    // ---- marks: every section the graph refers to ---------------------------------------
    let mut marks: Vec<(String, f64)> = vec![
        ("fuse".into(), first),
        ("lift".into(), CUES.lift),
        ("verse".into(), CUES.verse),
        ("pre-chorus".into(), CUES.dip),
        ("outro".into(), (last_note - 5_000.0).max(first)),
    ];
    for (i, a) in acts.iter().enumerate() {
        marks.push((format!("drop {}", i + 1), a.start));
        marks.push((format!("act {} end", i + 1), a.end));
    }
    if let Some((bs, be)) = brk {
        marks.push(("break".into(), bs));
        marks.push(("break end".into(), be));
    }
    // The longest stream before each act's drop is its riser.
    for (i, a) in acts.iter().enumerate() {
        if let Some(s) = riser_of(&long_streams, a) {
            marks.push((format!("riser {}", i + 1), s.0));
        }
    }
    // The run of swells (each within 5 s of the last) colours the picture while it lasts.
    let run: Vec<&&Note> = swells
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            (*i > 0 && n.time - swells[i - 1].end < 5000.0)
                || swells.get(i + 1).is_some_and(|x| x.time - n.end < 5000.0)
        })
        .map(|(_, n)| n)
        .collect();
    if let (Some(a), Some(z)) = (run.first(), run.last()) {
        marks.push(("swells".into(), a.time - 600.0));
        marks.push(("swells end".into(), z.end + 1800.0));
    }
    marks.sort_by(|a, b| a.1.total_cmp(&b.1));
    marks.dedup_by(|a, b| (a.1 - b.1).abs() < 1.0);
    gm.marks = marks
        .iter()
        .map(|(n, t)| Mark {
            name: n.clone(),
            time: *t,
        })
        .collect();
    let mk = Marks(marks);
    let has = |name: &str| mk.0.iter().any(|(n, _)| n == name);

    let mut graph = Graph {
        seed: 2222,
        ..Graph::default()
    };
    let mut b = B {
        g: &mut graph,
        x: 0.0,
        y: 0.0,
        row: 0.0,
        col_x: 0.0,
    };

    // ---- 1. the song's pulse ------------------------------------------------------------
    // A "live" flag (1 from the first note to the outro) every scene gates with, and the heart:
    // a zoom kick on every beat, an off-beat ghost of it half a beat later, and a head-bang
    // that tips the field left and right on alternate beats. Kiai makes all of it about twice
    // as strong, because the number wired into each pulse's scale input follows the kiai flag.
    let (fuse, outro) = (mk.idx("fuse"), mk.idx("outro"));
    b.band(9.0);
    let live = b.n(0.0, 0.0, Kind::Between, &[fuse, outro], None, &[]);
    let kiai = b.n(0.0, 1.0, Kind::Kiai, &[], None, &[]);
    let power = b.n(1.0, 1.0, Kind::Map, &[0.9, 1.0], None, &[kiai]);
    let beat1 = b.n(0.0, 2.0, Kind::OnBeat, &[1.0, 0.0], None, &[]);
    let beat1g = b.n(1.0, 2.0, Kind::Gate, &[], None, &[beat1, live]);
    b.pulse(
        2.0,
        2.0,
        0.018,
        230.0,
        Ease::QuadOut,
        F::Zoom,
        &[beat1g, power],
    );
    let ghost = b.n(2.0, 3.0, Kind::Delay, &[beat / 2.0], None, &[beat1g]);
    b.pulse(
        3.0,
        3.0,
        0.009,
        200.0,
        Ease::QuadOut,
        F::Zoom,
        &[ghost, power],
    );
    let beat2 = b.n(0.0, 4.0, Kind::OnBeat, &[2.0, 0.0], None, &[]);
    let beat2g = b.n(1.0, 4.0, Kind::Gate, &[], None, &[beat2, live]);
    let flip = b.n(2.0, 4.0, Kind::Toggle, &[], None, &[beat2g]);
    let side = b.n(3.0, 4.0, Kind::Map, &[2.0, -1.0], None, &[flip]);
    let side_k = b.n(4.0, 4.0, Kind::Mul, &[], None, &[side, power]);
    b.pulse(
        5.0,
        4.0,
        1.1,
        420.0,
        Ease::SineOut,
        F::Rotation,
        &[beat2g, side_k],
    );
    // The downbeat of each bar lands a little harder, with a breath of white.
    let bar = b.n(0.0, 6.0, Kind::OnBeat, &[4.0, 0.0], None, &[]);
    let barg = b.n(1.0, 6.0, Kind::Gate, &[], None, &[bar, live]);
    b.pulse(
        2.0,
        6.0,
        0.025,
        320.0,
        Ease::QuadOut,
        F::Zoom,
        &[barg, power],
    );
    b.pulse(
        2.0,
        7.0,
        0.05,
        260.0,
        Ease::QuadOut,
        F::Flash,
        &[barg, power],
    );
    // Every sixteen beats (four bars, ~4.3 s) the field flicks its trails round.
    let phrase = b.n(0.0, 8.0, Kind::OnBeat, &[16.0, 0.0], None, &[]);
    let phraseg = b.n(1.0, 8.0, Kind::Gate, &[], None, &[phrase, live]);
    b.pulse(
        2.0,
        8.0,
        14.0,
        900.0,
        Ease::QuadOut,
        F::EchoRot,
        &[phraseg, power],
    );
    r.say(
        "scene: heartbeat (beat zoom, off-beat ghost, head-bang, bar flash, 4-bar trail flick)"
            .into(),
    );

    // ---- 2. the fuse (intro) ------------------------------------------------------------
    // A lit fuse: the picture starts pulled in and dark, an ember-coloured wash flickers like a
    // flame (smooth noise), and the dark lifts as the song climbs. Over by "lift".
    let (lift, verse) = (mk.idx("lift"), mk.idx("verse"));
    b.band(5.0);
    let intro = b.n(0.0, 0.0, Kind::Between, &[fuse, lift], None, &[]);
    let flame = b.n(0.0, 1.0, Kind::Noise, &[0.5, 11.0], None, &[]);
    let flame_in = b.n(1.0, 1.0, Kind::Mul, &[], None, &[flame, intro]);
    b.field(2.0, 1.0, Kind::Drive, &[0.16], F::Wash, &[flame_in]);
    let hue_n = b.n(0.0, 2.0, Kind::Noise, &[1.5, 12.0], None, &[]);
    let hue_in = b.n(1.0, 2.0, Kind::Mul, &[], None, &[hue_n, intro]);
    b.field(2.0, 2.0, Kind::Drive, &[40.0], F::WashHue, &[hue_in]);
    let spark = b.n(0.0, 3.0, Kind::OnBeat, &[8.0, 4.0], None, &[]);
    let spark_g = b.n(1.0, 3.0, Kind::Gate, &[], None, &[spark, intro]);
    b.pulse(2.0, 3.0, 0.07, 500.0, Ease::QuadOut, F::Flash, &[spark_g]);
    b.pulse(2.0, 4.0, 5.0, 450.0, Ease::QuadOut, F::Chroma, &[spark_g]);
    key(&mut gm, F::WashHue, 0.0, 22.0, Ease::Step);
    ramp(
        &mut gm,
        F::Vignette,
        first - 1.0,
        CUES.lift,
        0.85,
        0.2,
        Ease::SineOut,
    );
    ramp(
        &mut gm,
        F::Zoom,
        first - 1.0,
        CUES.lift,
        1.14,
        1.0,
        Ease::CubicOut,
    );
    ramp(
        &mut gm,
        F::Rotation,
        first - 1.0,
        CUES.lift * 0.6,
        -7.0,
        0.0,
        Ease::SineOut,
    );
    key(&mut gm, F::Vignette, CUES.lift + 1.0, 0.0, Ease::SineOut);
    r.say("scene: fuse (flame-flicker wash, dark lifting, sparks every 8 beats)".into());

    // ---- 3. verse and pre-chorus: tension that only ever grows --------------------------
    // The beat gets heavier (its scale rises with a span), and in the dip before the first drop a
    // cold vignette and a fine chromatic fringe creep in. All of it is below what a stream needs
    // to read.
    let pre = mk.idx("pre-chorus");
    b.band(5.0);
    let verse_in = b.n(0.0, 0.0, Kind::Between, &[verse, pre], None, &[]);
    let verse_prog = b.n(0.0, 1.0, Kind::Span, &[verse, pre], None, &[]);
    let build_up = b.n(1.0, 1.0, Kind::Map, &[1.0, 0.0], None, &[verse_prog]);
    let h_beat = b.n(0.0, 2.0, Kind::OnBeat, &[1.0, 0.0], None, &[]);
    let h_g = b.n(1.0, 2.0, Kind::Gate, &[], None, &[h_beat, verse_in]);
    b.pulse(
        2.0,
        2.0,
        0.02,
        260.0,
        Ease::QuadOut,
        F::Zoom,
        &[h_g, build_up],
    );
    // Hue drifts through the verse: a slow wave, only here.
    let drift = b.n(0.0, 3.0, Kind::Wave, &[24.0, 20.0, 0.0], None, &[]);
    let drift_in = b.n(1.0, 3.0, Kind::Mul, &[], None, &[drift, verse_in]);
    b.field(2.0, 3.0, Kind::Drive, &[1.0], F::WashHue, &[drift_in]);
    b.field(2.0, 4.0, Kind::Drive, &[0.07], F::Wash, &[verse_in]);
    hold(&mut gm, F::WashHue, CUES.verse, CUES.dip, 200.0);
    if has("riser 1") {
        pass(
            &mut gm,
            F::Vignette,
            CUES.dip,
            acts[0].start - acts[0].gap,
            0.0,
            0.45,
            Ease::SineIn,
        );
    }
    r.say("scene: verse + pre-chorus (growing beat, hue drift, tension vignette)".into());

    // ---- 4. risers and inhales -----------------------------------------------------------
    // Before a drop the long stream is a riser: trails and a colour fringe grow over it. The
    // silence between the last note and the drop is an *inhale*: the field is pulled in and goes
    // dark, so the drop has somewhere to burst out from.
    let mut risers = 0;
    let mut inhales = 0;
    for a in &acts {
        if let Some(rs) = riser_of(&long_streams, a) {
            let (s, e) = (rs.0, rs.1);
            ramp(&mut gm, F::Echo, s, e, 0.0, 2.0, Ease::QuadIn);
            ramp(&mut gm, F::EchoRot, s, e, 0.0, 4.0, Ease::SineIn);
            ramp(&mut gm, F::EchoFade, s, e, 0.3, 0.5, Ease::Linear);
            ramp(&mut gm, F::Chroma, s, e, 0.0, 9.0, Ease::QuadIn);
            ramp(&mut gm, F::Zoom, s, e, 1.0, 1.05, Ease::SineIn);
            ramp(&mut gm, F::Wash, s, e, 0.0, 0.14, Ease::SineIn);
            ramp(&mut gm, F::WashHue, s, e, a.hue - 40.0, a.hue, Ease::Linear);
            key(&mut gm, F::Echo, e + 1.0, 0.0, Ease::Step);
            key(&mut gm, F::EchoRot, e + 1.0, 0.0, Ease::Step);
            key(&mut gm, F::EchoFade, e + 1.0, ECHO_FADE, Ease::Step);
            key(&mut gm, F::Chroma, e + 1.0, 0.0, Ease::Step);
            key(&mut gm, F::Wash, e + 1.0, 0.0, Ease::Step);
            key(&mut gm, F::WashHue, e + 1.0, 0.0, Ease::Step);
            key(&mut gm, F::Zoom, e + 1.0, 1.0, Ease::Step);
            risers += 1;
        }
        if a.gap >= 900.0 {
            let (s, e) = (a.start - a.gap + 80.0, a.start);
            let deep = 0.78 + 0.1 * (1.0 - a.power);
            pass(&mut gm, F::Zoom, s, e, 1.0, deep, Ease::CubicIn);
            pass(&mut gm, F::Vignette, s, e, 0.0, 0.8, Ease::SineIn);
            pass(&mut gm, F::Wash, s, e, 0.0, 0.0, Ease::Linear);
            inhales += 1;
        }
    }
    r.say(format!(
        "scene: {risers} risers (trails + fringe grow over the stream), {inhales} inhales (pull in and dim in the silence)"
    ));

    // ---- 5. the drops ---------------------------------------------------------------------
    // The first note of every act: white-out, a shove, tearing, a colour split that closes up,
    // a ring of kaleidoscope copies. All short; the louder the act the more of it (the power
    // constant wired into every pulse's scale input).
    let mut ring = 0;
    for (i, a) in acts.iter().enumerate() {
        let d = mk.idx(&format!("drop {}", i + 1));
        b.band(8.0);
        let ev = b.n(0.0, 0.0, Kind::OnMark, &[d], None, &[]);
        let pw = b.n(0.0, 1.0, Kind::Const, &[a.power], None, &[]);
        b.pulse(1.0, 0.0, 0.65, 600.0, Ease::QuadOut, F::Flash, &[ev, pw]);
        b.pulse(1.0, 1.0, 0.34, 900.0, Ease::BackOut, F::Zoom, &[ev, pw]);
        b.pulse(1.0, 2.0, 76.0, 800.0, Ease::QuadOut, F::Chroma, &[ev, pw]);
        b.pulse(1.0, 3.0, 0.7, 260.0, Ease::QuadOut, F::Glitch, &[ev, pw]);
        b.n(1.0, 4.0, Kind::Shake, &[96.0, 650.0], None, &[ev, pw]);
        b.pulse(1.0, 5.0, 7.0, 700.0, Ease::QuadOut, F::Rotation, &[ev, pw]);
        if a.power >= 0.7 {
            b.pulse(1.0, 6.0, 4.0, 380.0, Ease::QuadOut, F::Kaleido, &[ev, pw]);
            b.pulse(1.0, 7.0, 0.3, 1100.0, Ease::QuadOut, F::Wash, &[ev, pw]);
            ring += 1;
        }
    }
    // The very last finisher is the map's own full stop.
    r.say(format!(
        "scene: {} drops (flash, zoom, chroma, tear, shake, turn; {ring} with a kaleidoscope ring)",
        acts.len()
    ));

    // ---- 6. inside the acts --------------------------------------------------------------
    // What stays on while notes are dense: a wash in the act's colour that slowly turns, one or
    // two faint trails, a breathing vignette, a sway of a couple of degrees and a sideways
    // lurch every other beat. As the window goes on, a span ramps the wash and the trails up, so
    // a long act builds instead of repeating.
    for (i, a) in acts.iter().enumerate() {
        let (s, e) = (
            mk.idx(&format!("drop {}", i + 1)),
            mk.idx(&format!("act {} end", i + 1)),
        );
        b.band(9.0);
        let inside = b.n(0.0, 0.0, Kind::Between, &[s, e], None, &[]);
        let prog = b.n(0.0, 1.0, Kind::Span, &[s, e], None, &[]);
        // Colour: the act's hue, turning 45 degrees (one way or the other) as the window goes on.
        let sweep = if i % 2 == 0 { 45.0 } else { -45.0 };
        let turn = b.n(1.0, 2.0, Kind::Map, &[sweep, a.hue], None, &[prog]);
        let turn_in = b.n(2.0, 2.0, Kind::Mul, &[], None, &[turn, inside]);
        b.field(3.0, 2.0, Kind::Drive, &[1.0], F::WashHue, &[turn_in]);
        let wash = b.n(1.0, 1.0, Kind::Map, &[0.1, 0.07], None, &[prog]);
        let wash_in = b.n(2.0, 1.0, Kind::Mul, &[], None, &[wash, inside]);
        b.field(3.0, 1.0, Kind::Drive, &[1.0], F::Wash, &[wash_in]);
        // Trails: one echo, a second after the first third of the window.
        let thirds = b.n(1.0, 0.0, Kind::Steps, &[3.0], None, &[prog]);
        let echo = b.n(2.0, 0.0, Kind::Map, &[1.0, 1.0], None, &[thirds]);
        let echo_in = b.n(3.0, 0.0, Kind::Mul, &[], None, &[echo, inside]);
        b.field(4.0, 0.0, Kind::Drive, &[1.0], F::Echo, &[echo_in]);
        // The vignette breathes every two beats.
        let breath = b.n(0.0, 3.0, Kind::Wave, &[2.0, 0.5, 0.0], None, &[]);
        let breath_m = b.n(1.0, 3.0, Kind::Map, &[1.0, 0.5], None, &[breath]);
        let breath_in = b.n(2.0, 3.0, Kind::Mul, &[], None, &[breath_m, inside]);
        b.field(3.0, 3.0, Kind::Drive, &[0.22], F::Vignette, &[breath_in]);
        // A sway: a slow sine, a degree or two, one act in three a triangle.
        let sway = b.n(0.0, 4.0, Kind::Wave, &[8.0, 1.6, (i % 2) as f64], None, &[]);
        let sway_in = b.n(1.0, 4.0, Kind::Mul, &[], None, &[sway, inside]);
        b.field(2.0, 4.0, Kind::Drive, &[1.0], F::Rotation, &[sway_in]);
        // A lurch every other beat.
        let lurch = b.n(0.0, 5.0, Kind::OnBeat, &[2.0, 1.0], None, &[]);
        let lurch_g = b.n(1.0, 5.0, Kind::Gate, &[], None, &[lurch, inside]);
        b.pulse(2.0, 5.0, 14.0, 250.0, Ease::QuadOut, F::X, &[lurch_g]);
        // Every sixth bar the trails whip and the chroma blooms.
        let big = b.n(0.0, 6.0, Kind::OnBeat, &[24.0, 0.0], None, &[]);
        let big_g = b.n(1.0, 6.0, Kind::Gate, &[], None, &[big, inside]);
        b.pulse(2.0, 6.0, 26.0, 700.0, Ease::QuadOut, F::Chroma, &[big_g]);
        b.pulse(2.0, 7.0, 24.0, 900.0, Ease::SineOut, F::EchoRot, &[big_g]);
        // A slow sideways drift of the whole picture from noise.
        let nx = b.n(0.0, 7.0, Kind::Noise, &[3.0, 30.0 + i as f64], None, &[]);
        let nx_in = b.n(1.0, 7.0, Kind::Mul, &[], None, &[nx, inside]);
        b.field(2.0, 8.0, Kind::Drive, &[10.0], F::Y, &[nx_in]);
    }
    r.say(format!(
        "scene: {} act interiors (turning wash, trails that multiply, breath, sway, lurch)",
        acts.len()
    ));

    // ---- 7. finisher clusters -------------------------------------------------------------
    // Each cluster is a group; "the Nth of this group's notes" drives a Progress, so every
    // finisher kicks harder than the one before. The notes themselves swell and spiral in.
    for (ci, c) in clusters.iter().enumerate() {
        let gi = gm.groups.len();
        gm.groups.push(Group {
            name: format!("Finishers {}", ci + 1),
            notes: c.clone(),
            ..Group::default()
        });
        for &t in c {
            let ks = [
                (N::Scale, t - 300.0, 1.0, Ease::BackOut),
                (N::Scale, t, 1.4, Ease::QuadOut),
                (N::Scale, t + 260.0, 1.0, Ease::Step),
            ];
            for (p, tt, v, e) in ks {
                gm.key_note(gi, p, Key { t: tt, v, ease: e });
            }
        }
        b.band(6.0);
        let g1 = (gi + 1) as f64;
        let ev = b.n(0.0, 1.0, Kind::OnNoteReach, &[g1, 0.0, 0.0], None, &[]);
        let prog = b.n(1.0, 1.0, Kind::Progress, &[], None, &[ev]);
        let boost = b.n(2.0, 1.0, Kind::Map, &[1.0, 0.35], None, &[prog]);
        b.pulse(3.0, 0.0, 0.5, 520.0, Ease::BackOut, F::Zoom, &[ev, boost]);
        b.pulse(3.0, 1.0, 0.3, 340.0, Ease::QuadOut, F::Flash, &[ev, boost]);
        b.pulse(
            3.0,
            2.0,
            32.0,
            380.0,
            Ease::QuadOut,
            F::Chroma,
            &[ev, boost],
        );
        b.pulse(3.0, 3.0, 0.5, 260.0, Ease::QuadOut, F::Glitch, &[ev, boost]);
        b.pulse(3.0, 4.0, 3.0, 700.0, Ease::QuadOut, F::Echo, &[ev, boost]);
        b.n(3.0, 5.0, Kind::Shake, &[54.0, 480.0], None, &[ev, boost]);
        let flip = b.n(1.0, 3.0, Kind::Toggle, &[], None, &[ev]);
        let sgn = b.n(2.0, 3.0, Kind::Map, &[2.0, -1.0], None, &[flip]);
        let turn = b.n(2.0, 4.0, Kind::Mul, &[], None, &[sgn, boost]);
        b.pulse(
            3.0,
            6.0,
            9.0,
            600.0,
            Ease::BackOut,
            F::Rotation,
            &[ev, turn],
        );
    }
    r.say(format!(
        "scene: {} finisher clusters (Progress makes each one harder)",
        clusters.len()
    ));

    // Lone finishers (outside the clusters) get one small flash.
    let lone: Vec<f64> = m
        .notes
        .iter()
        .filter(|n| n.big && !clusters.iter().any(|c| c.contains(&n.time)))
        .map(|n| n.time)
        .collect();
    if !lone.is_empty() {
        let gi = gm.groups.len();
        gm.groups.push(Group {
            name: "Lone finishers".into(),
            notes: lone.clone(),
            ..Group::default()
        });
        for &t in &lone {
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t: t - 250.0,
                    v: 1.0,
                    ease: Ease::BackOut,
                },
            );
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t,
                    v: 1.3,
                    ease: Ease::QuadOut,
                },
            );
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t: t + 240.0,
                    v: 1.0,
                    ease: Ease::Step,
                },
            );
        }
        b.band(3.0);
        let ev = b.n(
            0.0,
            0.0,
            Kind::OnNoteReach,
            &[(gi + 1) as f64, 0.0, 0.0],
            None,
            &[],
        );
        b.pulse(1.0, 0.0, 0.3, 300.0, Ease::QuadOut, F::Zoom, &[ev]);
        b.pulse(1.0, 1.0, 0.2, 260.0, Ease::QuadOut, F::Flash, &[ev]);
        b.pulse(1.0, 2.0, 30.0, 320.0, Ease::QuadOut, F::Chroma, &[ev]);
        b.n(1.0, 3.0, Kind::Shake, &[30.0, 350.0], None, &[ev]);
    }

    // ---- 8. the swells ------------------------------------------------------------------
    // Short bursts to mash. One group of all of them; each snaps the picture round, throws a
    // white flash and steps the colour on, and a counter keeps the colour stepping for the
    // whole stretch (only while a swell is near).
    if !swells.is_empty() {
        let gi = gm.groups.len();
        gm.groups.push(Group {
            name: "Swells".into(),
            notes: swells.iter().map(|n| n.time).collect(),
            ..Group::default()
        });
        for n in &swells {
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t: n.time - 350.0,
                    v: 1.0,
                    ease: Ease::BackOut,
                },
            );
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t: n.time,
                    v: 1.25,
                    ease: Ease::QuadOut,
                },
            );
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t: n.end + 220.0,
                    v: 1.0,
                    ease: Ease::Step,
                },
            );
        }
        b.band(7.0);
        let ev = b.n(
            0.0,
            1.0,
            Kind::OnNoteReach,
            &[(gi + 1) as f64, 0.0, 0.0],
            None,
            &[],
        );
        let flip = b.n(1.0, 0.0, Kind::Toggle, &[], None, &[ev]);
        let sgn = b.n(2.0, 0.0, Kind::Map, &[2.0, -1.0], None, &[flip]);
        b.pulse(
            3.0,
            0.0,
            16.0,
            560.0,
            Ease::BackOut,
            F::Rotation,
            &[ev, sgn],
        );
        b.pulse(3.0, 1.0, 0.25, 300.0, Ease::QuadOut, F::Flash, &[ev]);
        b.pulse(3.0, 2.0, 0.16, 420.0, Ease::BackOut, F::Zoom, &[ev]);
        b.pulse(3.0, 3.0, 34.0, 400.0, Ease::QuadOut, F::Chroma, &[ev]);
        b.pulse(3.0, 4.0, 3.0, 360.0, Ease::QuadOut, F::Kaleido, &[ev]);
        // Through the run each swell steps the colour on; outside it nothing is left over.
        if has("swells") {
            let (sa, sz) = (mk.idx("swells"), mk.idx("swells end"));
            let in_run = b.n(1.0, 5.0, Kind::Between, &[sa, sz], None, &[]);
            let count = b.n(1.0, 2.0, Kind::Count, &[], None, &[ev]);
            let step = b.n(2.0, 2.0, Kind::Map, &[34.0, 0.0], None, &[count]);
            let step_in = b.n(3.0, 2.0, Kind::Mul, &[], None, &[step, in_run]);
            b.field(4.0, 2.0, Kind::Drive, &[1.0], F::WashHue, &[step_in]);
            b.field(2.0, 6.0, Kind::Drive, &[0.13], F::Wash, &[in_run]);
        }
        b.n(3.0, 6.0, Kind::Shake, &[26.0, 380.0], None, &[ev]);
    }
    r.say(format!(
        "scene: {} swells (snap-turn, flash, kaleido, hue stepping)",
        swells.len()
    ));

    // ---- 9. streams -----------------------------------------------------------------------
    // The streams are the part you have to read, so they move least: each long one is a group
    // of its own, swirled by a few degrees in a slow sine (a snake that stays near the drum),
    // and the beat inside it ticks a barely visible zoom, one per four notes.
    let mut snake_keys = 0;
    for (si, s) in long_streams.iter().enumerate() {
        let gi = gm.groups.len();
        let notes: Vec<f64> = m
            .notes
            .iter()
            .filter(|n| n.time >= s.0 && n.time <= s.1)
            .map(|n| n.time)
            .collect();
        gm.groups.push(Group {
            name: format!("Stream {} ({} notes)", si + 1, s.2),
            notes,
            ..Group::default()
        });
        let (from, to) = (s.0 - 900.0, s.1 + 50.0);
        let step = beat / 4.0;
        let mut t = from;
        while t <= to {
            let phase = (t - s.0) / (beat * 4.0) * std::f64::consts::TAU;
            gm.key_note(
                gi,
                N::Swirl,
                Key {
                    t,
                    v: 3.2 * phase.sin(),
                    ease: Ease::Linear,
                },
            );
            snake_keys += 1;
            t += step;
        }
        gm.key_note(
            gi,
            N::Swirl,
            Key {
                t: to + 1.0,
                v: 0.0,
                ease: Ease::Step,
            },
        );
        gm.key_note(
            gi,
            N::Scale,
            Key {
                t: from,
                v: 1.0,
                ease: Ease::Step,
            },
        );
        let _ = gi;
    }
    r.say(format!(
        "scene: {} long streams get a {snake_keys}-key snake (3 degrees about the drum); the rest of the map leaves them alone",
        long_streams.len()
    ));

    // ---- 10. the break --------------------------------------------------------------------
    // 11 s with nothing to hit, so this is where the picture can turn into something else: it
    // goes cold, the field floats on noise, the trails and a slow kaleidoscope appear, and it
    // warms up again before the notes come back.
    if let Some((bs, be)) = brk {
        let (bk, bke) = (mk.idx("break"), mk.idx("break end"));
        b.band(8.0);
        let inside = b.n(0.0, 0.0, Kind::Between, &[bk, bke], None, &[]);
        let prog = b.n(0.0, 1.0, Kind::Span, &[bk, bke], None, &[]);
        let fx = b.n(0.0, 2.0, Kind::Noise, &[2.0, 77.0], None, &[]);
        let fy = b.n(0.0, 3.0, Kind::Noise, &[2.5, 78.0], None, &[]);
        let fxm = b.n(1.0, 2.0, Kind::Map, &[70.0, -35.0], None, &[fx]);
        let fym = b.n(1.0, 3.0, Kind::Map, &[40.0, -20.0], None, &[fy]);
        let fx_in = b.n(2.0, 2.0, Kind::Mul, &[], None, &[fxm, inside]);
        let fy_in = b.n(2.0, 3.0, Kind::Mul, &[], None, &[fym, inside]);
        b.field(3.0, 2.0, Kind::Drive, &[1.0], F::X, &[fx_in]);
        b.field(3.0, 3.0, Kind::Drive, &[1.0], F::Y, &[fy_in]);
        let breathe = b.n(0.0, 4.0, Kind::Wave, &[6.0, 0.05, 0.0], None, &[]);
        let breathe_in = b.n(1.0, 4.0, Kind::Mul, &[], None, &[breathe, inside]);
        b.field(2.0, 4.0, Kind::Drive, &[1.0], F::Zoom, &[breathe_in]);
        let spin = b.n(0.0, 5.0, Kind::Pow, &[1.5], None, &[prog]);
        let spin_in = b.n(1.0, 5.0, Kind::Mul, &[], None, &[spin, inside]);
        b.field(2.0, 5.0, Kind::Drive, &[150.0], F::Rotation, &[spin_in]);
        // Cold blue (212) at the start, warm again (~2) by the time the notes return.
        let slow = b.n(1.0, 6.0, Kind::Map, &[150.0, 0.0], None, &[prog]);
        let slow_in = b.n(2.0, 6.0, Kind::Mul, &[], None, &[slow, inside]);
        b.field(3.0, 6.0, Kind::Drive, &[1.0], F::WashHue, &[slow_in]);
        // The first bar of the break and the last bar before the notes return get an accent.
        let start = b.n(0.0, 7.0, Kind::OnMark, &[bk], None, &[]);
        b.pulse(1.0, 7.0, 0.5, 900.0, Ease::QuadOut, F::Flash, &[start]);
        b.pulse(2.0, 7.0, 40.0, 1200.0, Ease::QuadOut, F::Chroma, &[start]);
        let (c0, c1) = (bs + 400.0, be + 500.0);
        ramp(
            &mut gm,
            F::Wash,
            c0,
            bs + 3500.0,
            0.0,
            0.24,
            Ease::SineInOut,
        );
        key(&mut gm, F::Wash, be - 500.0, 0.24, Ease::SineInOut);
        key(&mut gm, F::Wash, c1, 0.0, Ease::Step);
        hold(&mut gm, F::WashHue, bs, be + 400.0, 212.0);
        ramp(
            &mut gm,
            F::Vignette,
            bs,
            bs + 2500.0,
            0.0,
            0.5,
            Ease::SineOut,
        );
        key(&mut gm, F::Vignette, be - 300.0, 0.5, Ease::SineInOut);
        key(&mut gm, F::Vignette, c1, 0.0, Ease::Step);
        ramp(
            &mut gm,
            F::Echo,
            bs + 800.0,
            bs + 4000.0,
            0.0,
            4.0,
            Ease::SineInOut,
        );
        key(&mut gm, F::Echo, be - 900.0, 4.0, Ease::SineInOut);
        key(&mut gm, F::Echo, be + 200.0, 0.0, Ease::Step);
        ramp(
            &mut gm,
            F::EchoRot,
            bs + 800.0,
            bs + 5000.0,
            0.0,
            22.0,
            Ease::SineInOut,
        );
        key(&mut gm, F::EchoRot, be + 200.0, 0.0, Ease::Step);
        ramp(
            &mut gm,
            F::Kaleido,
            bs + 1800.0,
            bs + 5500.0,
            0.0,
            6.0,
            Ease::SineInOut,
        );
        key(&mut gm, F::Kaleido, be - 1200.0, 6.0, Ease::SineInOut);
        key(&mut gm, F::Kaleido, be + 100.0, 0.0, Ease::Step);
        r.say(format!(
            "scene: break {}-{} (cold wash, floating field, trails, slow kaleidoscope, warms before the notes return)",
            secs(bs),
            secs(be)
        ));
    }

    // ---- 11. the player's side ------------------------------------------------------------
    // A miss tears the picture (the recipe), combos mark themselves, accuracy darkens the corners
    // only when it falls under ~92%.
    b.band(3.0);
    Recipe::MissGlitch.add(b.g, b.x, b.y).expect("room");
    b.band(4.0);
    let c100 = b.n(0.0, 0.0, Kind::OnCombo, &[100.0], None, &[]);
    let c100g = b.n(1.0, 0.0, Kind::Gate, &[], None, &[c100, live]);
    b.pulse(2.0, -0.5, 0.2, 500.0, Ease::QuadOut, F::Flash, &[c100g]);
    b.pulse(2.0, 0.5, 0.06, 500.0, Ease::BackOut, F::Zoom, &[c100g]);
    b.pulse(2.0, 1.5, 14.0, 450.0, Ease::QuadOut, F::Chroma, &[c100g]);
    let c500 = b.n(0.0, 2.0, Kind::OnCombo, &[500.0], None, &[]);
    let c500g = b.n(1.0, 2.0, Kind::Gate, &[], None, &[c500, live]);
    b.pulse(2.0, 2.5, 0.5, 700.0, Ease::QuadOut, F::Flash, &[c500g]);
    b.pulse(2.0, 3.5, 40.0, 700.0, Ease::QuadOut, F::Chroma, &[c500g]);
    b.pulse(2.0, 4.5, 3.0, 800.0, Ease::QuadOut, F::Echo, &[c500g]);
    b.band(2.0);
    let acc = b.n(0.0, 0.0, Kind::Accuracy, &[], None, &[]);
    let worse = b.n(1.0, 0.0, Kind::Map, &[-6.0, 5.52], None, &[acc]);
    let capped = b.n(2.0, 0.0, Kind::Clamp, &[0.0, 0.5], None, &[worse]);
    let worse_live = b.n(3.0, 0.0, Kind::Mul, &[], None, &[capped, live]);
    b.field(4.0, 0.0, Kind::Drive, &[1.0], F::Vignette, &[worse_live]);
    r.say(
        "scene: player side (miss tear, combo 100/500 marks, danger vignette under 92% accuracy)"
            .into(),
    );

    // ---- 12. the full stop ---------------------------------------------------------------
    // The last note is a finisher: one last burst, then the picture sinks and goes dark.
    let outro_t = last_note;
    pass(
        &mut gm,
        F::Vignette,
        outro_t + 200.0,
        end + 4000.0,
        0.0,
        1.0,
        Ease::SineIn,
    );
    ramp(
        &mut gm,
        F::Zoom,
        outro_t + 200.0,
        end + 4000.0,
        1.0,
        1.15,
        Ease::SineInOut,
    );
    ramp(
        &mut gm,
        F::Wash,
        outro_t + 200.0,
        end + 4000.0,
        0.0,
        0.3,
        Ease::SineIn,
    );
    hold(&mut gm, F::WashHue, outro_t, end + 4500.0, 35.0);
    let final_group = gm.groups.len();
    gm.groups.push(Group {
        name: "Last note".into(),
        notes: vec![last_note],
        ..Group::default()
    });
    gm.key_note(
        final_group,
        N::Scale,
        Key {
            t: last_note - 600.0,
            v: 1.0,
            ease: Ease::BackOut,
        },
    );
    gm.key_note(
        final_group,
        N::Scale,
        Key {
            t: last_note,
            v: 1.8,
            ease: Ease::QuadOut,
        },
    );
    gm.key_note(
        final_group,
        N::Scale,
        Key {
            t: last_note + 400.0,
            v: 1.0,
            ease: Ease::Step,
        },
    );
    b.band(5.0);
    let ev = b.n(
        0.0,
        0.0,
        Kind::OnNoteReach,
        &[(final_group + 1) as f64, 0.0, 0.0],
        None,
        &[],
    );
    b.pulse(1.0, 0.0, 1.0, 1400.0, Ease::QuadOut, F::Flash, &[ev]);
    b.pulse(1.0, 1.0, 0.5, 1200.0, Ease::BackOut, F::Zoom, &[ev]);
    b.pulse(1.0, 2.0, 80.0, 1400.0, Ease::QuadOut, F::Chroma, &[ev]);
    b.pulse(1.0, 3.0, 8.0, 1200.0, Ease::QuadOut, F::Kaleido, &[ev]);
    b.n(1.0, 4.0, Kind::Shake, &[110.0, 900.0], None, &[ev]);
    r.say("scene: full stop (last finisher bursts, then the picture sinks)".into());

    gm.graph = graph;
    gm.sanitize();
    r.say(format!(
        "{} nodes, {} marks, {} groups, {} field tracks",
        gm.graph.nodes.len(),
        gm.marks.len(),
        gm.groups.len(),
        gm.field.len()
    ));
    gm
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(src) = args.first() else {
        eprintln!("usage: tko_dynamite <map.osu> [out.tko]");
        std::process::exit(2);
    };
    let text = fs::read_to_string(src).unwrap_or_else(|e| {
        eprintln!("can't read {src}: {e}");
        std::process::exit(1)
    });
    let out = args.get(1).cloned().unwrap_or_else(|| {
        std::path::Path::new(src)
            .with_extension("tko")
            .to_string_lossy()
            .into_owned()
    });
    let m = read(&text);
    if m.notes.is_empty() || m.beat <= 0.0 {
        eprintln!("no hit objects or no red line in {src}");
        std::process::exit(1);
    }
    let mut r = Report(Vec::new());
    let gm = build(&m, &mut r);
    let tko = format::write(&text, &gm).unwrap_or_else(|e| {
        eprintln!("can't write the .tko: {e}");
        std::process::exit(1)
    });
    fs::write(&out, tko).unwrap_or_else(|e| {
        eprintln!("can't write {out}: {e}");
        std::process::exit(1)
    });
    println!("wrote {out}");
}
