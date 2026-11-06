//! Builds a showcase `.tko` from the structure of a taiko `.osu`: it finds the sections, the kiai
//! windows, runs and bursts of finishers, swells, busy slider-velocity stretches and quiet gaps, and
//! gives each one its own piece of the effect set through the same API the editor uses.
//!
//!     cargo run --release --example tko_showcase -- "map.osu" [out.tko]
//!
//! The output is visual only (judging, score and pp are untouched) and strobes in places: it is a
//! demo of what the editor can do, not a map for everyone. The map itself is never copied.

use std::{env, fs};
use taiko_rs::tko::format;
use taiko_rs::tko::gimmick::{FieldProp as F, Gimmick, Group, Mark, NoteProp as N};
use taiko_rs::tko::graph::{Graph, Kind, Target};
use taiko_rs::tko::recipes::Recipe;
use taiko_rs::tko::track::{Ease, Key};

// --- reading the map ------------------------------------------------------------------------

struct Note {
    time: f64,
    big: bool,
    /// A swell's end.
    swell: Option<f64>,
}

#[derive(Default)]
struct Map {
    /// Kiai on/off at each timing point, in file order.
    kiai: Vec<(f64, bool)>,
    /// Green lines (time only).
    greens: Vec<f64>,
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
        let (Some(t), Some(effects)) = (
            f.first().and_then(|t| t.parse::<f64>().ok()),
            f.get(7).and_then(|e| e.parse::<i32>().ok()),
        ) else {
            continue;
        };
        m.kiai.push((t, effects & 1 != 0));
        if f.get(6) == Some(&"0") {
            m.greens.push(t);
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
        let swell = (kind & 8 != 0)
            .then(|| f.get(5).and_then(|e| e.parse::<f64>().ok()))
            .flatten();
        m.notes.push(Note {
            time: t,
            big: sound & 4 != 0 && swell.is_none(),
            swell,
        });
    }
    m.notes.sort_by(|a, b| a.time.total_cmp(&b.time));
    m
}

/// Kiai as windows: the map's own on/off flicker (a few 60 ms toggles) is merged into one.
fn kiai_windows(m: &Map, merge_ms: f64) -> Vec<(f64, f64)> {
    let end = m.notes.last().map_or(0.0, |n| n.time) + 1000.0;
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
        w.push((s, end));
    }
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in w {
        match merged.last_mut() {
            Some(last) if s - last.1 < merge_ms => last.1 = e,
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Runs of finishers where each follows the last within `gap` ms, at least `min` long.
fn finisher_runs(m: &Map, gap: f64, min: usize) -> Vec<Vec<f64>> {
    let mut runs: Vec<Vec<f64>> = Vec::new();
    for n in m.notes.iter().filter(|n| n.big) {
        match runs.last_mut() {
            Some(r) if n.time - r.last().copied().unwrap_or(0.0) < gap => r.push(n.time),
            _ => runs.push(vec![n.time]),
        }
    }
    runs.retain(|r| r.len() >= min);
    runs
}

/// Stretches with many slider-velocity lines (a gimmick map's busy parts): `(start, end)`.
fn busy_sv(m: &Map) -> Vec<(f64, f64)> {
    const WINDOW: f64 = 5000.0;
    const BUSY: usize = 40;
    let last = m.greens.last().copied().unwrap_or(0.0);
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut s = 0.0;
    while s <= last {
        let n = m
            .greens
            .iter()
            .filter(|&&t| t >= s && t < s + WINDOW)
            .count();
        if n >= BUSY {
            match out.last_mut() {
                Some(l) if (l.1 - s).abs() < 1.0 => l.1 = s + WINDOW,
                _ => out.push((s, s + WINDOW)),
            }
        }
        s += WINDOW;
    }
    out
}

/// The longest stretch with no note at all, if it is at least `min` ms: `(start, end)`.
fn longest_gap(m: &Map, min: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    let mut prev_end: Option<f64> = None;
    for n in &m.notes {
        if let Some(p) = prev_end
            && n.time - p >= min
            && best.is_none_or(|b| n.time - p > b.1 - b.0)
        {
            best = Some((p, n.time));
        }
        prev_end = Some(prev_end.unwrap_or(0.0).max(n.swell.unwrap_or(n.time)));
    }
    best
}

// --- building the gimmick -------------------------------------------------------------------

fn key(g: &mut Gimmick, p: F, t: f64, v: f64, ease: Ease) {
    g.key_field(p, Key { t, v, ease });
}

/// Holds `p` at `from` until `start`, takes it to `to` by `end`, then leaves it at `to`.
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

/// Places wired nodes on a grid: one band of rows per scene.
struct B<'a> {
    g: &'a mut Graph,
    x: f32,
    y: f32,
    row: f32,
}

impl B<'_> {
    /// Starts a new band of `rows` rows below the last one.
    fn band(&mut self, rows: f32) {
        self.x = 0.0;
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
}

struct Marks(Vec<(String, f64)>);

impl Marks {
    /// 1-based, as the nodes number them.
    fn idx(&self, name: &str) -> f64 {
        self.0
            .iter()
            .position(|(n, _)| n == name)
            .map_or(0.0, |i| (i + 1) as f64)
    }

    fn time(&self, name: &str) -> Option<f64> {
        self.0.iter().find(|(n, _)| n == name).map(|m| m.1)
    }
}

struct Report(Vec<String>);

impl Report {
    fn say(&mut self, s: String) {
        println!("{s}");
        self.0.push(s);
    }
}

fn build(m: &Map, r: &mut Report) -> Gimmick {
    let mut gm = Gimmick::default();
    let first = m.notes.first().map_or(0.0, |n| n.time);
    let last = m.notes.last().map_or(0.0, |n| n.time);
    let end = last + 800.0;
    let swells: Vec<(f64, f64)> = m
        .notes
        .iter()
        .filter_map(|n| n.swell.map(|e| (n.time, e)))
        .collect();
    let kiai = kiai_windows(m, 200.0);
    let runs = finisher_runs(m, 2600.0, 6);
    let bursts = finisher_runs(m, 250.0, 3);
    let sv = busy_sv(m);
    let calm = longest_gap(m, 2500.0);
    r.say(format!(
        "{} notes, {} finishers, {} swells, {} kiai windows, {} long runs, {} bursts, {} busy SV stretches, calm gap {:?}",
        m.notes.len(),
        m.notes.iter().filter(|n| n.big).count(),
        swells.len(),
        kiai.len(),
        runs.len(),
        bursts.len(),
        sv.len(),
        calm.map(|c| (c.0 as i64, c.1 as i64)),
    ));

    // Marks: the sections the nodes below refer to.
    let intro_end = swells
        .first()
        .filter(|s| s.0 - first < 3000.0)
        .map_or(first + 6000.0, |s| s.1 + 600.0);
    let outro = (last - 6000.0).max(intro_end + 1000.0);
    let mut marks = vec![
        ("verse".to_string(), intro_end),
        ("outro".to_string(), outro),
    ];
    if let Some(run) = runs.iter().max_by_key(|r| r.len()) {
        marks.push(("run".into(), run[0] - 400.0));
        marks.push(("run end".into(), run[run.len() - 1] + 2000.0));
    }
    if let Some(c) = calm {
        marks.push(("calm".into(), c.0 + 300.0));
        marks.push(("calm end".into(), c.1 - 300.0));
    }
    for (i, k) in kiai.iter().enumerate() {
        marks.push((format!("kiai {}", i + 1), k.0));
        marks.push((format!("kiai {} end", i + 1), k.1));
    }
    for (i, s) in sv.iter().enumerate() {
        marks.push((format!("sv {}", i + 1), s.0));
        marks.push((format!("sv {} end", i + 1), s.1));
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
    let (verse, outro_m) = (mk.idx("verse"), mk.idx("outro"));
    let mut graph = Graph {
        seed: 4242,
        ..Graph::default()
    };
    let mut b = B {
        g: &mut graph,
        x: 0.0,
        y: 0.0,
        row: 0.0,
    };

    // The playfield is only driven between the verse and the outro: a "live" flag every scene uses.
    b.band(1.0);
    let live = b.n(0.0, 0.0, Kind::Between, &[verse, outro_m], None, &[]);

    // The pulse of the song: every beat kicks the zoom (twice as hard in kiai), every second beat
    // squeezes the playfield sideways.
    b.band(3.0);
    let beat = b.n(0.0, 0.0, Kind::OnBeat, &[1.0, 0.0], None, &[]);
    let gate = b.n(1.0, 0.0, Kind::Gate, &[], None, &[beat, live]);
    let kiai_v = b.n(0.0, 1.0, Kind::Kiai, &[], None, &[]);
    let kick = b.n(1.0, 1.0, Kind::Map, &[1.4, 1.0], None, &[kiai_v]);
    b.field(
        2.0,
        0.0,
        Kind::Pulse,
        &[0.03, 240.0],
        F::Zoom,
        &[gate, kick],
    );
    let beat2 = b.n(0.0, 2.0, Kind::OnBeat, &[2.0, 0.0], None, &[]);
    let gate2 = b.n(1.0, 2.0, Kind::Gate, &[], None, &[beat2, live]);
    b.field(
        2.0,
        2.0,
        Kind::Pulse,
        &[-0.05, 300.0],
        F::StretchX,
        &[gate2, kick],
    );
    b.field(
        2.0,
        3.0,
        Kind::Pulse,
        &[0.05, 300.0],
        F::StretchY,
        &[gate2, kick],
    );

    // The player's side: a miss tears the picture, combos flash, accuracy darkens the corners.
    b.band(3.0);
    Recipe::MissGlitch.add(b.g, 0.0, b.y).expect("room");
    b.band(3.0);
    let combo = b.n(0.0, 0.0, Kind::OnCombo, &[100.0], None, &[]);
    let cg = b.n(1.0, 0.0, Kind::Gate, &[], None, &[combo, live]);
    b.field(2.0, -0.5, Kind::Pulse, &[0.45, 500.0], F::Flash, &[cg]);
    b.field(2.0, 0.5, Kind::Pulse, &[0.14, 600.0], F::Zoom, &[cg]);
    b.field(2.0, 1.5, Kind::Pulse, &[35.0, 500.0], F::Chroma, &[cg]);
    let acc = b.n(0.0, 2.0, Kind::Accuracy, &[], None, &[]);
    let worse = b.n(1.0, 2.0, Kind::Map, &[-8.0, 8.0], None, &[acc]);
    let worse_live = b.n(2.0, 2.0, Kind::Mul, &[], None, &[worse, live]);
    b.field(3.0, 2.0, Kind::Drive, &[1.0], F::Vignette, &[worse_live]);

    // Section by section from here.
    let run = (mk.time("run"), mk.time("run end"));
    if let (Some(_), Some(_)) = run {
        b.band(6.0);
        let (ri, re) = (mk.idx("run"), mk.idx("run end"));
        let inside = b.n(0.0, 0.0, Kind::Between, &[ri, re], None, &[]);
        let big = b.n(0.0, 1.0, Kind::OnNoteReach, &[0.0, 3.0, 0.0], None, &[]);
        let in_run = b.n(1.0, 1.0, Kind::Gate, &[], None, &[big, inside]);
        let prog = b.n(2.0, 1.0, Kind::Progress, &[], None, &[in_run]);
        // Every finisher of the run lands harder than the last.
        b.field(
            3.0,
            0.0,
            Kind::Pulse,
            &[0.55, 480.0],
            F::Zoom,
            &[in_run, prog],
        );
        b.field(
            3.0,
            1.0,
            Kind::Pulse,
            &[0.8, 380.0],
            F::Flash,
            &[in_run, prog],
        );
        b.field(
            3.0,
            2.0,
            Kind::Pulse,
            &[55.0, 420.0],
            F::Chroma,
            &[in_run, prog],
        );
        b.field(
            3.0,
            3.0,
            Kind::Pulse,
            &[0.75, 320.0],
            F::Glitch,
            &[in_run, prog],
        );
        b.field(
            3.0,
            4.0,
            Kind::Pulse,
            &[5.0, 900.0],
            F::Echo,
            &[in_run, prog],
        );
        b.field(
            3.0,
            5.0,
            Kind::Pulse,
            &[22.0, 520.0],
            F::Rotation,
            &[in_run, prog],
        );
        b.n(3.0, 6.0, Kind::Shake, &[60.0, 600.0], None, &[in_run, prog]);
        // And the whole stretch winds up: a slow turn, a creeping zoom and a hue that runs away.
        let span = b.n(0.0, 3.0, Kind::Span, &[ri, re], None, &[]);
        let wind = b.n(1.0, 3.0, Kind::Pow, &[2.0], None, &[span]);
        let eased = b.n(2.0, 3.0, Kind::Mul, &[], None, &[wind, inside]);
        b.field(1.0, 4.0, Kind::Drive, &[360.0], F::Rotation, &[eased]);
        b.field(1.0, 5.0, Kind::Drive, &[0.18], F::Zoom, &[eased]);
        b.field(1.0, 6.0, Kind::Drive, &[0.22], F::Wash, &[eased]);
        let beats = b.n(0.0, 5.0, Kind::Beats, &[], None, &[]);
        let hue = b.n(0.0, 6.0, Kind::Mul, &[], None, &[beats, inside]);
        b.field(0.0, 7.0, Kind::Drive, &[22.0], F::WashHue, &[hue]);
        r.say(
            "scene: finisher run crescendo (Progress -> zoom, flash, chroma, glitch, echo, shake)"
                .into(),
        );
    }

    // The bursts of finishers: each gets a group, notes that spiral in and a rising chain.
    for (bi, burst) in bursts.iter().enumerate() {
        let gi = gm.groups.len();
        gm.groups.push(Group {
            name: format!("Burst {}", bi + 1),
            notes: burst.clone(),
            ..Group::default()
        });
        for &t in burst {
            gm.key_note(
                gi,
                N::Swirl,
                Key {
                    t: t - 500.0,
                    v: 300.0,
                    ease: Ease::CubicOut,
                },
            );
            gm.key_note(
                gi,
                N::Swirl,
                Key {
                    t,
                    v: 0.0,
                    ease: Ease::Step,
                },
            );
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t: t - 200.0,
                    v: 1.0,
                    ease: Ease::BackOut,
                },
            );
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t,
                    v: 1.7,
                    ease: Ease::QuadOut,
                },
            );
            gm.key_note(
                gi,
                N::Scale,
                Key {
                    t: t + 260.0,
                    v: 1.0,
                    ease: Ease::Step,
                },
            );
        }
        b.band(6.0);
        let g1 = (gi + 1) as f64;
        let ev = b.n(0.0, 1.0, Kind::OnNoteReach, &[g1, 0.0, 0.0], None, &[]);
        let prog = b.n(1.0, 1.0, Kind::Progress, &[], None, &[ev]);
        b.field(2.0, 0.0, Kind::Pulse, &[0.7, 420.0], F::Zoom, &[ev, prog]);
        b.field(2.0, 1.0, Kind::Pulse, &[0.6, 300.0], F::Flash, &[ev, prog]);
        b.field(
            2.0,
            2.0,
            Kind::Pulse,
            &[70.0, 360.0],
            F::Chroma,
            &[ev, prog],
        );
        b.field(
            2.0,
            3.0,
            Kind::Pulse,
            &[8.0, 700.0],
            F::Kaleido,
            &[ev, prog],
        );
        b.field(
            2.0,
            4.0,
            Kind::Pulse,
            &[40.0, 500.0],
            F::EchoRot,
            &[ev, prog],
        );
        b.field(2.0, 5.0, Kind::Pulse, &[1.0, 420.0], F::Glitch, &[ev, prog]);
        b.n(2.0, 6.0, Kind::Shake, &[90.0, 600.0], None, &[ev, prog]);
    }
    if !bursts.is_empty() {
        r.say(format!(
            "scene: {} finisher bursts (swirling notes, kaleidoscope, chroma)",
            bursts.len()
        ));
    }

    // Swells: the playfield spins and folds in on itself while the player mashes.
    for &(s, e) in &swells {
        if s - first < 3000.0 {
            continue;
        }
        ramp(&mut gm, F::Rotation, s, e, 0.0, 720.0, Ease::QuadInOut);
        key(&mut gm, F::Rotation, e + 1.0, 0.0, Ease::Step);
        ramp(&mut gm, F::Zoom, s, e, 1.0, 0.7, Ease::SineInOut);
        key(&mut gm, F::Zoom, e + 1.0, 1.0, Ease::Step);
        hold(&mut gm, F::Kaleido, s, e, 6.0);
        ramp(&mut gm, F::Chroma, s, e, 0.0, 45.0, Ease::QuadIn);
        key(&mut gm, F::Chroma, e + 1.0, 0.0, Ease::Step);
    }
    r.say(format!(
        "scene: {} swells spin the field",
        swells.len().saturating_sub(1)
    ));

    // The intro: the field falls into place out of a dark spin during the opening swell.
    ramp(
        &mut gm,
        F::Zoom,
        first - 1.0,
        intro_end,
        1.9,
        1.0,
        Ease::CubicOut,
    );
    ramp(
        &mut gm,
        F::Rotation,
        first - 1.0,
        intro_end,
        -540.0,
        0.0,
        Ease::CubicOut,
    );
    ramp(
        &mut gm,
        F::Vignette,
        first - 1.0,
        intro_end,
        1.0,
        0.0,
        Ease::SineOut,
    );
    ramp(
        &mut gm,
        F::Chroma,
        first - 1.0,
        intro_end,
        36.0,
        0.0,
        Ease::QuadOut,
    );
    ramp(
        &mut gm,
        F::Echo,
        first - 1.0,
        intro_end,
        4.0,
        0.0,
        Ease::SineIn,
    );
    ramp(
        &mut gm,
        F::EchoRot,
        first - 1.0,
        intro_end,
        24.0,
        0.0,
        Ease::SineOut,
    );
    r.say("scene: intro".into());

    // Busy slider-velocity stretches: the picture wobbles and stretches with the map's own mess.
    for (i, _) in sv.iter().enumerate() {
        b.band(4.0);
        let inside = b.n(
            0.0,
            0.0,
            Kind::Between,
            &[
                mk.idx(&format!("sv {}", i + 1)),
                mk.idx(&format!("sv {} end", i + 1)),
            ],
            None,
            &[],
        );
        let wob = b.n(0.0, 1.0, Kind::Wave, &[2.0, 0.18, 0.0], None, &[]);
        let wob_in = b.n(1.0, 1.0, Kind::Mul, &[], None, &[wob, inside]);
        b.field(2.0, 1.0, Kind::Drive, &[1.0], F::StretchX, &[wob_in]);
        let wob2 = b.n(0.0, 2.0, Kind::Wave, &[3.0, 0.14, 1.0], None, &[]);
        let wob2_in = b.n(1.0, 2.0, Kind::Mul, &[], None, &[wob2, inside]);
        b.field(2.0, 2.0, Kind::Drive, &[-1.0], F::StretchY, &[wob2_in]);
        let noise = b.n(0.0, 3.0, Kind::Noise, &[1.5, (7 + i) as f64], None, &[]);
        let noise_in = b.n(1.0, 3.0, Kind::Mul, &[], None, &[noise, inside]);
        b.field(2.0, 3.0, Kind::Drive, &[0.5], F::Glitch, &[noise_in]);
        b.field(2.0, 0.0, Kind::Drive, &[2.0], F::Echo, &[inside]);
        let sway = b.n(1.0, 4.0, Kind::Wave, &[6.0, 6.0, 0.0], None, &[]);
        let sway_in = b.n(2.0, 4.0, Kind::Mul, &[], None, &[sway, inside]);
        b.field(3.0, 4.0, Kind::Drive, &[1.0], F::Rotation, &[sway_in]);
    }
    if !sv.is_empty() {
        r.say(format!(
            "scene: {} busy SV stretches wobble and stretch",
            sv.len()
        ));
    }

    // The quiet gap: one slow turn, the lights low.
    if let Some((cs, ce)) = calm {
        b.band(2.0);
        let ids = Recipe::SectionSweep.add(b.g, 0.0, b.y).expect("room");
        let (ci, cei) = (mk.idx("calm"), mk.idx("calm end"));
        if let Some(n) = b.g.node_mut(ids[0]) {
            n.p[0] = ci;
            n.p[1] = cei;
        }
        ramp(&mut gm, F::Vignette, cs, ce, 0.55, 0.55, Ease::SineInOut);
        key(&mut gm, F::Vignette, ce + 1.0, 0.0, Ease::SineOut);
        ramp(&mut gm, F::Zoom, cs, ce, 0.88, 0.88, Ease::SineInOut);
        key(&mut gm, F::Zoom, ce + 1.0, 1.0, Ease::SineOut);
        hold(&mut gm, F::Wash, cs, ce, 0.16);
        hold(&mut gm, F::WashHue, cs, ce, 215.0);
        r.say("scene: calm gap (section sweep, dim, cold wash)".into());
    }

    // Kiai: trails that kick on the beat, a hue that runs, a breathing vignette, sway, a drop.
    for (i, k) in kiai.iter().enumerate() {
        let (ks, ke) = (
            mk.idx(&format!("kiai {}", i + 1)),
            mk.idx(&format!("kiai {} end", i + 1)),
        );
        b.band(9.0);
        let inside = b.n(0.0, 0.0, Kind::Between, &[ks, ke], None, &[]);
        let drop = b.n(0.0, 1.0, Kind::OnMark, &[ks], None, &[]);
        b.field(1.0, 1.0, Kind::Pulse, &[0.65, 650.0], F::Flash, &[drop]);
        b.field(1.0, 2.0, Kind::Pulse, &[0.45, 900.0], F::Zoom, &[drop]);
        b.field(1.0, 3.0, Kind::Pulse, &[70.0, 800.0], F::Chroma, &[drop]);
        b.field(1.0, 4.0, Kind::Pulse, &[1.0, 600.0], F::Glitch, &[drop]);
        b.field(
            1.0,
            5.0,
            Kind::Pulse,
            &[360.0, 1200.0],
            F::Rotation,
            &[drop],
        );
        b.n(1.0, 6.0, Kind::Shake, &[110.0, 900.0], None, &[drop]);
        // Echo trails, thicker while it lasts.
        b.field(1.0, 0.0, Kind::Drive, &[3.0], F::Echo, &[inside]);
        let bar = b.n(2.0, 0.0, Kind::OnBeat, &[4.0, 0.0], None, &[]);
        let bar_in = b.n(3.0, 0.0, Kind::Gate, &[], None, &[bar, inside]);
        b.field(4.0, 0.0, Kind::Pulse, &[18.0, 420.0], F::EchoRot, &[bar_in]);
        b.field(4.0, 1.0, Kind::Pulse, &[6.0, 500.0], F::Kaleido, &[bar_in]);
        b.field(4.0, 2.0, Kind::Pulse, &[0.4, 500.0], F::EchoZoom, &[bar_in]);
        // The wash colour turns with the beat count.
        let beats = b.n(2.0, 2.0, Kind::Beats, &[], None, &[]);
        let hue = b.n(3.0, 2.0, Kind::Mul, &[], None, &[beats, inside]);
        b.field(4.0, 3.0, Kind::Drive, &[18.0], F::WashHue, &[hue]);
        b.field(3.0, 3.0, Kind::Drive, &[0.2], F::Wash, &[inside]);
        // A vignette that breathes every two beats, and a sway.
        let breath = b.n(2.0, 4.0, Kind::Wave, &[2.0, 0.5, 0.0], None, &[]);
        let breath_m = b.n(3.0, 4.0, Kind::Map, &[1.0, 0.5], None, &[breath]);
        let breath_in = b.n(4.0, 4.0, Kind::Mul, &[], None, &[breath_m, inside]);
        b.field(5.0, 4.0, Kind::Drive, &[0.6], F::Vignette, &[breath_in]);
        let sway = b.n(2.0, 5.0, Kind::Wave, &[8.0, 5.0, 1.0], None, &[]);
        let sway_in = b.n(3.0, 5.0, Kind::Mul, &[], None, &[sway, inside]);
        b.field(4.0, 5.0, Kind::Drive, &[1.0], F::Rotation, &[sway_in]);
        // Every other beat the field lurches sideways.
        let lurch = b.n(2.0, 6.0, Kind::OnBeat, &[2.0, 1.0], None, &[]);
        let lurch_in = b.n(3.0, 6.0, Kind::Gate, &[], None, &[lurch, inside]);
        b.field(4.0, 6.0, Kind::Pulse, &[34.0, 260.0], F::X, &[lurch_in]);
        let _ = k;
    }
    if !kiai.is_empty() {
        r.say(format!(
            "scene: {} kiai windows (drop, trails, kaleidoscope, hue wash, breath)",
            kiai.len()
        ));
    }

    // Every finisher gets its notes bigger, on top of everything else.
    let gi = gm.groups.len();
    gm.groups.push(Group {
        name: "Finishers".into(),
        notes: m.notes.iter().filter(|n| n.big).map(|n| n.time).collect(),
        ..Group::default()
    });
    for n in m.notes.iter().filter(|n| n.big) {
        gm.key_note(
            gi,
            N::Scale,
            Key {
                t: n.time - 300.0,
                v: 1.0,
                ease: Ease::BackOut,
            },
        );
        gm.key_note(
            gi,
            N::Scale,
            Key {
                t: n.time,
                v: 1.35,
                ease: Ease::QuadOut,
            },
        );
        gm.key_note(
            gi,
            N::Scale,
            Key {
                t: n.time + 250.0,
                v: 1.0,
                ease: Ease::Step,
            },
        );
    }

    // The outro: the picture drains and sinks.
    ramp(&mut gm, F::Vignette, outro, end, 0.0, 0.9, Ease::SineIn);
    ramp(&mut gm, F::Zoom, outro, end, 1.0, 1.18, Ease::SineInOut);
    ramp(&mut gm, F::Rotation, outro, end, 0.0, -8.0, Ease::SineInOut);
    hold(&mut gm, F::Wash, outro, end, 0.22);
    hold(&mut gm, F::WashHue, outro, end, 35.0);
    r.say("scene: outro".into());

    gm.graph = graph;
    // The editor's own clamp: values in range, tracks sorted, caps held.
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
        eprintln!("usage: tko_showcase <map.osu> [out.tko]");
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
    if m.notes.is_empty() {
        eprintln!("no hit objects in {src}");
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
