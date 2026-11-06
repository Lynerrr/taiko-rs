//! Stable gimmick structures: barline notes, anti-barline walls, fake sliders, plain red lines.
//!
//! Ported (by logic) from KatsuDonEditor's `chart/gimmick_session.py` (MIT, used with the author's
//! permission). Everything here writes ordinary `.osu` timing lines and objects that osu!stable
//! reads the way our player does: a gimmick is red lines (each resets SV to 1.0x and draws a
//! barline), green lines (SV) and the odd negative-length slider.
//!
//! A builder takes a `base` timing snapshot: the chart as it was before any gimmick. A gimmick
//! difficulty is the worst possible source of its own grid (by the second placement it is full of
//! 60000 BPM lines), so BPMs and SV come from the snapshot and never from the lines already
//! written.
//!
//! Wherever a green line shares a millisecond with a red one it is written *after* it: stable
//! resolves a shared timestamp by file order and a red line resets SV, so the other way round
//! would silently cancel it. `Chart::insert_timing` keeps insertion order at a shared time, and
//! every builder returns its points in file order.

use super::chart::{EFFECT_KIAI, HitObject, SOUND_FINISH, SOUND_WHISTLE, TimingPoint};
use super::sv_curves::{StepSv, SvSource};
use super::timing::{active_at, active_red};

pub const DEFAULT_GIMMICK_BPM: f64 = 60_000.0;
pub const DEFAULT_FAKE_SLIDER_LENGTH: f64 = -0.001;
pub const FAKE_SLIDER_MAX_LENGTH: f64 = 0.001;
pub const DEFAULT_ANTI_LINES_PER_BEAT: i32 = 36;
pub const ANTI_WALL_ANCHOR_OFFSET_MS: i64 = 2;
pub const ANTI_RESTORE_OFFSET_MS: i64 = 1;
pub const ANTI_RESTORE_METER: i32 = 999;
pub const HIDDEN_WALL_METER: i32 = 1;
pub const HIDDEN_HIDE_METER: i32 = 999;
pub const HIDDEN_WALL_OFFSET_MS: i64 = 1;
pub const FAKE_SLIDER_RESTORE_OFFSET_MS: i64 = 1;
/// A snapshot with a BPM above this probably already contains gimmick lines.
pub const DIRTY_BASE_BPM: f64 = 1000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct GimmickConfig {
    pub gimmick_bpm: f64,
    /// A don's mirrored restore pair sits at +/- this.
    pub spacing_ms: i64,
    pub kat_spacing1_ms: i64,
    pub kat_spacing2_ms: i64,
    pub kat_spacing3_ms: i64,
    pub fake_slider_length: f64,
    pub red_line_offset_ms: i64,
    pub fake_slider_offset_ms: i64,
    pub fake_slider_sv: f64,
    pub fake_slider_red_line: bool,
    pub fake_slider_restore_after: bool,
    pub shiny_red_line: bool,
    pub place_notes: bool,
    pub mirror_don_lines: bool,
    pub mirror_kat_lines: bool,
    pub red_line_bpm: Option<f64>,
    pub red_line_meter: i32,
    pub shiny_offset_ms: i64,
    pub shiny_count: usize,
    pub shiny_bpm_multiplier: f64,
    pub fake_slider_bpm_multiplier: f64,
    pub restore_bpm: Option<f64>,
    pub omit_barline: bool,
    pub anti_lines_per_beat: i32,
    pub anti_wall_bpm: Option<f64>,
    pub anti_don_ticks: usize,
    pub anti_kat_ticks: usize,
    pub hidden_hide_bpm: f64,
    pub hidden_wall_bpm: f64,
    pub hidden_base_sv: f64,
    pub hidden_don_sv: f64,
    pub hidden_kat_sv: f64,
    pub hidden_window_divisor: i32,
    /// Squash the real note out of sight (the structure replaces it).
    pub hide_note: bool,
    pub omit_note_barline: bool,
}

impl Default for GimmickConfig {
    fn default() -> Self {
        GimmickConfig {
            gimmick_bpm: DEFAULT_GIMMICK_BPM,
            spacing_ms: 1,
            kat_spacing1_ms: 1,
            kat_spacing2_ms: 3,
            kat_spacing3_ms: 5,
            fake_slider_length: DEFAULT_FAKE_SLIDER_LENGTH,
            red_line_offset_ms: 0,
            fake_slider_offset_ms: 2,
            fake_slider_sv: 10.0,
            fake_slider_red_line: true,
            fake_slider_restore_after: false,
            shiny_red_line: true,
            place_notes: true,
            mirror_don_lines: false,
            mirror_kat_lines: true,
            red_line_bpm: None,
            red_line_meter: 4,
            shiny_offset_ms: 1,
            shiny_count: 3,
            shiny_bpm_multiplier: 1.0,
            fake_slider_bpm_multiplier: 1.0,
            restore_bpm: None,
            omit_barline: true,
            anti_lines_per_beat: DEFAULT_ANTI_LINES_PER_BEAT,
            anti_wall_bpm: None,
            anti_don_ticks: 1,
            anti_kat_ticks: 2,
            hidden_hide_bpm: 66_666.0,
            hidden_wall_bpm: 12_345.0,
            hidden_base_sv: 0.01,
            hidden_don_sv: 100.0 / 9900.0,
            hidden_kat_sv: 100.0 / 9700.0,
            hidden_window_divisor: 8,
            hide_note: true,
            omit_note_barline: false,
        }
    }
}

impl GimmickConfig {
    /// Why this configuration can't be used, if it can't.
    pub fn validate(&self) -> Result<(), String> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if !positive(self.gimmick_bpm) {
            return Err("Gimmick BPM must be positive".into());
        }
        if self.spacing_ms < 1 {
            return Err("Don spacing must be at least 1 ms".into());
        }
        let kat = [
            self.kat_spacing1_ms,
            self.kat_spacing2_ms,
            self.kat_spacing3_ms,
        ];
        if kat.iter().any(|&s| s < 1) {
            return Err("Kat spacing must be at least 1 ms".into());
        }
        if kat[0] == kat[1] || kat[1] == kat[2] || kat[0] == kat[2] {
            return Err("Kat spacings must be different from each other".into());
        }
        if !self.fake_slider_length.is_finite() || self.fake_slider_length > FAKE_SLIDER_MAX_LENGTH
        {
            return Err("Fake slider length must be 0.001 or less".into());
        }
        if self.fake_slider_offset_ms < 1 {
            return Err("Fake slider offset must be at least 1 ms".into());
        }
        if !positive(self.fake_slider_sv) {
            return Err("Fake slider SV must be positive".into());
        }
        if self.shiny_count < 1 || self.shiny_count > 64 {
            return Err("Shiny count must be 1..64".into());
        }
        if self.shiny_offset_ms < 1 {
            return Err("Shiny offset must be at least 1 ms".into());
        }
        for (name, v) in [
            ("Shiny BPM multiplier", self.shiny_bpm_multiplier),
            (
                "Fake slider BPM multiplier",
                self.fake_slider_bpm_multiplier,
            ),
            ("Hide BPM", self.hidden_hide_bpm),
            ("Wall BPM", self.hidden_wall_bpm),
            ("Wall base SV", self.hidden_base_sv),
            ("Don slit SV", self.hidden_don_sv),
            ("Kat slit SV", self.hidden_kat_sv),
        ] {
            if !positive(v) {
                return Err(format!("{name} must be positive"));
            }
        }
        for (name, v) in [
            ("Red line BPM", self.red_line_bpm),
            ("Restore BPM", self.restore_bpm),
            ("Wall BPM", self.anti_wall_bpm),
        ] {
            if v.is_some_and(|v| !positive(v)) {
                return Err(format!("{name} must be positive"));
            }
        }
        if self.red_line_meter < 1 {
            return Err("Meter must be at least 1".into());
        }
        if !(1..=512).contains(&self.anti_lines_per_beat) {
            return Err("Wall lines per beat must be 1..512".into());
        }
        if self.anti_don_ticks < 1 || self.anti_kat_ticks < 1 {
            return Err("Slit width must be at least 1 tick".into());
        }
        if !(1..=64).contains(&self.hidden_window_divisor) {
            return Err("Slit window divisor must be 1..64".into());
        }
        Ok(())
    }
}

/// What a builder writes. Timing points are in file order.
#[derive(Clone, Debug, Default)]
pub struct Built {
    pub timing: Vec<TimingPoint>,
    pub objects: Vec<HitObject>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Don,
    Kat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeKind {
    Regular,
    Don,
    Kat,
}

fn sorted_by_time(points: &[TimingPoint]) -> Vec<TimingPoint> {
    let mut v = points.to_vec();
    v.sort_by(|a, b| a.time.total_cmp(&b.time));
    v
}

fn red_at(time: i64, bpm: f64, meter: i32, omit: bool) -> TimingPoint {
    TimingPoint::red_with(time as f64, bpm, meter, omit)
}

fn green_at(time: i64, sv: f64) -> TimingPoint {
    TimingPoint::green(time as f64, sv)
}

/// A whole millisecond the way Python's `round` (and stable's `Math.Round`) makes one: halves to even.
fn round_ms(v: f64) -> i64 {
    v.round_ties_even() as i64
}

/// BPM of the base snapshot at `time`, for the restore lines. 120 with no red line.
pub fn base_bpm_at(base: &[TimingPoint], time: f64) -> f64 {
    active_red(base, time)
        .and_then(TimingPoint::bpm)
        .unwrap_or(120.0)
}

/// The BPM a structure's restore line carries: the typed `restore_bpm`, else the base's times
/// `multiplier`.
pub fn restore_bpm_at(
    base: &[TimingPoint],
    time: f64,
    cfg: &GimmickConfig,
    multiplier: f64,
) -> f64 {
    cfg.restore_bpm
        .unwrap_or_else(|| base_bpm_at(base, time) * multiplier)
}

/// Whether a snapshot appears to already hold gimmick lines.
pub fn looks_gimmicked(points: &[TimingPoint]) -> bool {
    points
        .iter()
        .any(|p| p.bpm().is_some_and(|b| b > DIRTY_BASE_BPM))
}

fn hit_sound(kind: FakeKind) -> i32 {
    if kind == FakeKind::Kat {
        SOUND_WHISTLE
    } else {
        0
    }
}

fn slider_sound(kind: FakeKind) -> i32 {
    if kind == FakeKind::Kat {
        SOUND_FINISH
    } else {
        0
    }
}

/// A drumroll of negative length: what draws without ever being hittable.
///
/// Three structures share this builder: a plain fake slider (one red line on the slider), a
/// *shiny* (a glow around a note that stays visible: no squash, `shiny_count` sliders sit
/// `shiny_offset_ms` later beside the note), and Don/Kat (the one that hides its note: a gimmick
/// BPM line squashes it, an SV line takes it off screen, the chart's BPM comes back
/// `fake_slider_offset_ms` later where the fake slider, big for Kat, is all that is left).
#[allow(clippy::too_many_arguments)]
pub fn fake_slider(
    time: i64,
    base: &[TimingPoint],
    cfg: &GimmickConfig,
    kind: FakeKind,
    x: f64,
    y: f64,
    shiny: bool,
    copies: Option<usize>,
    big: bool,
) -> Built {
    let copies = copies
        .unwrap_or(if shiny { cfg.shiny_count } else { 1 })
        .max(1);
    let omit = cfg.omit_barline;
    let tf = time as f64;
    let slider_at;
    let timing: Vec<TimingPoint>;
    if shiny {
        slider_at = time + cfg.shiny_offset_ms;
        timing = if cfg.shiny_red_line {
            let bpm = base_bpm_at(base, tf) * cfg.shiny_bpm_multiplier;
            vec![red_at(time, bpm, 4, omit)]
        } else {
            vec![]
        };
    } else if kind == FakeKind::Regular {
        slider_at = time + cfg.fake_slider_offset_ms;
        timing = if cfg.fake_slider_red_line {
            let bpm = restore_bpm_at(base, slider_at as f64, cfg, cfg.fake_slider_bpm_multiplier);
            let mut v = vec![red_at(slider_at, bpm, 4, omit)];
            if cfg.fake_slider_restore_after {
                let at = slider_at + FAKE_SLIDER_RESTORE_OFFSET_MS;
                v.push(red_at(at, base_bpm_at(base, at as f64), 4, omit));
            }
            v
        } else {
            vec![]
        };
    } else {
        slider_at = time + cfg.fake_slider_offset_ms;
        let restore = restore_bpm_at(base, slider_at as f64, cfg, cfg.fake_slider_bpm_multiplier);
        let mut v = vec![red_at(
            time,
            if cfg.hide_note {
                cfg.gimmick_bpm
            } else {
                base_bpm_at(base, tf)
            },
            4,
            omit || !cfg.hide_note,
        )];
        if cfg.hide_note {
            v.push(green_at(time, cfg.fake_slider_sv));
        }
        v.push(red_at(slider_at, restore, 4, omit));
        timing = v;
    }
    let mut sound = slider_sound(kind);
    if big && kind == FakeKind::Regular {
        sound |= SOUND_FINISH;
    }
    let mut objects = Vec::new();
    if kind != FakeKind::Regular {
        objects.push(HitObject::circle_at(tf, x, y, hit_sound(kind)));
    }
    for _ in 0..copies {
        objects.push(HitObject::slider_at(
            slider_at as f64,
            x,
            y,
            sound,
            cfg.fake_slider_length,
        ));
    }
    Built { timing, objects }
}

/// A note drawn out of barlines: one gimmick BPM line plus restore lines at +/- the spacings
/// (every uninherited point emits a barline at its own time). Don is one pair, kat three.
pub fn barline_note(time: i64, base: &[TimingPoint], cfg: &GimmickConfig, kind: NoteKind) -> Built {
    let (spacings, mirrored): (Vec<i64>, bool) = match kind {
        NoteKind::Don => (vec![cfg.spacing_ms], cfg.mirror_don_lines),
        NoteKind::Kat => (
            vec![
                cfg.kat_spacing1_ms,
                cfg.kat_spacing2_ms,
                cfg.kat_spacing3_ms,
            ],
            cfg.mirror_kat_lines,
        ),
    };
    let offsets: Vec<i64> = if mirrored {
        spacings
            .iter()
            .map(|s| -s)
            .chain(spacings.iter().copied())
            .collect()
    } else {
        spacings
    };
    let tf = time as f64;
    let mut timing = vec![if cfg.hide_note {
        red_at(time, cfg.gimmick_bpm, 4, cfg.omit_note_barline)
    } else {
        red_at(time, base_bpm_at(base, tf), 4, true)
    }];
    for offset in offsets {
        let at = time + offset;
        timing.push(red_at(
            at,
            restore_bpm_at(base, at as f64, cfg, 1.0),
            4,
            false,
        ));
    }
    timing.sort_by(|a, b| a.time.total_cmp(&b.time));
    let objects = if cfg.place_notes {
        vec![HitObject::circle_at(
            tf,
            256.0,
            192.0,
            if kind == NoteKind::Kat {
                SOUND_WHISTLE
            } else {
                0
            },
        )]
    } else {
        vec![]
    };
    Built { timing, objects }
}

/// A green line after every red line in `points`, at the chart's SV there. `at_note`: the
/// structure is one object drawn around that note, so every line takes the note's value. A green
/// line the builder already put on a red line's millisecond is replaced rather than stacked.
pub fn keep_sv(
    points: Vec<TimingPoint>,
    model: &dyn SvSource,
    at_note: Option<f64>,
) -> Vec<TimingPoint> {
    let taken: std::collections::HashSet<i64> = points
        .iter()
        .filter(|p| p.is_red())
        .map(|p| round_ms(p.time))
        .collect();
    let mut kept: Vec<TimingPoint> = points
        .into_iter()
        .filter(|p| p.is_red() || !taken.contains(&round_ms(p.time)))
        .collect();
    kept.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut out = Vec::with_capacity(kept.len() * 2);
    for p in kept {
        let red = p.is_red();
        let t = p.time;
        out.push(p);
        if red {
            out.push(TimingPoint::green(t, model.value_at(at_note.unwrap_or(t))));
        }
    }
    out
}

/// `barline_note` in negative: a solid wall of bars with each plain circle as a slit. Timing
/// points only: the chart's own notes are not rewritten, so what is played doesn't change.
///
/// * The wall: a red line every `beat / anti_lines_per_beat`, each at `anti_wall_bpm` (or the
///   chart's), each followed by a green line carrying the chart's SV.
/// * The notes: a squash line on the note's ms and a restore one after it, both omitting their
///   first barline (each would otherwise stamp a bar in the middle of the slit it opens).
///
/// The slit is the wall ticks nearest the note left out; its width is the note's colour.
pub fn anti_barline(
    notes: &[HitObject],
    start_ms: f64,
    end_ms: f64,
    base: &[TimingPoint],
    cfg: &GimmickConfig,
    model: Option<&dyn SvSource>,
) -> Vec<TimingPoint> {
    let mut eligible: Vec<&HitObject> =
        notes.iter().filter(|n| n.is_circle() && !n.big()).collect();
    eligible.sort_by(|a, b| a.time.total_cmp(&b.time));
    if eligible.is_empty() {
        return Vec::new();
    }
    let full = base;
    let reds: Vec<TimingPoint> = sorted_by_time(base)
        .into_iter()
        .filter(TimingPoint::is_red)
        .collect();
    let base: &[TimingPoint] = if reds.is_empty() { full } else { &reds };

    let step_at = |t: f64| 60_000.0 / base_bpm_at(base, t) / cfg.anti_lines_per_beat as f64;
    let slit_of = |n: &HitObject| {
        if n.taiko() == super::chart::Taiko::Kat {
            cfg.anti_kat_ticks
        } else {
            cfg.anti_don_ticks
        }
    };
    let reach_of = |n: &HitObject| {
        let slit = slit_of(n);
        slit.max(slit.div_ceil(2) + 1)
    };
    let (first, last) = (eligible[0], eligible[eligible.len() - 1]);
    let wall_start = start_ms
        .min(first.time - reach_of(first) as f64 * step_at(first.time))
        .max(0.0);
    let wall_end = end_ms.max(last.time + reach_of(last) as f64 * step_at(last.time));

    let anchor = (round_ms(first.time) + ANTI_WALL_ANCHOR_OFFSET_MS) as f64;
    let mut positions = Vec::new();
    let mut at = anchor;
    while at >= wall_start {
        positions.push(at);
        let step = step_at(at);
        if step <= 0.0 || !step.is_finite() {
            break;
        }
        at -= step;
    }
    positions.reverse();
    let mut at = anchor + step_at(anchor);
    while at <= wall_end {
        positions.push(at);
        let step = step_at(at);
        if step <= 0.0 || !step.is_finite() {
            break;
        }
        at += step;
    }
    let mut ticks: Vec<i64> = Vec::new();
    for p in positions {
        let t = round_ms(p);
        if ticks.last() != Some(&t) {
            ticks.push(t);
        }
    }
    if ticks.is_empty() {
        return Vec::new();
    }

    let mut dropped = std::collections::HashSet::new();
    for n in &eligible {
        let wanted = slit_of(n);
        let mut after = ticks.partition_point(|&t| (t as f64) < n.time);
        let mut before = after as isize - 1;
        for _ in 0..wanted {
            if before >= 0
                && (after >= ticks.len()
                    || n.time - ticks[before as usize] as f64 <= ticks[after] as f64 - n.time)
            {
                dropped.insert(before as usize);
                before -= 1;
            } else if after < ticks.len() {
                dropped.insert(after);
                after += 1;
            } else {
                break;
            }
        }
    }

    let step_sv = StepSv(full);
    let sv: &dyn SvSource = model.unwrap_or(&step_sv);
    let mut points = Vec::new();
    for (i, &at) in ticks.iter().enumerate() {
        if dropped.contains(&i) {
            continue;
        }
        let bpm = cfg
            .anti_wall_bpm
            .unwrap_or_else(|| base_bpm_at(base, at as f64));
        points.push(red_at(at, bpm, 4, false));
        points.push(green_at(at, sv.value_at(at as f64)));
    }
    for n in &eligible {
        let t = round_ms(n.time);
        if !cfg.hide_note {
            points.push(red_at(
                t,
                base_bpm_at(base, t as f64),
                ANTI_RESTORE_METER,
                true,
            ));
            continue;
        }
        let restore_at = t + ANTI_RESTORE_OFFSET_MS;
        points.push(red_at(t, cfg.gimmick_bpm, 4, true));
        points.push(red_at(
            restore_at,
            base_bpm_at(base, restore_at as f64),
            ANTI_RESTORE_METER,
            true,
        ));
    }
    if let Some(model) = model {
        let converted: std::collections::HashSet<i64> =
            eligible.iter().map(|n| round_ms(n.time)).collect();
        for n in notes {
            let at = round_ms(n.time);
            if start_ms <= n.time && n.time <= end_ms && !converted.contains(&at) {
                points.push(green_at(at, model.value_at(at as f64)));
            }
        }
        return keep_sv(points, model, None);
    }
    points.sort_by(|a, b| a.time.total_cmp(&b.time));
    points
}

/// `anti_barline` with the wall drawn by osu! and the slit cut with SV: one meter-1 red line lets
/// osu! emit every bar of the wall, and each note's slit is a burst of higher SV after it. Four
/// lines per note against hundreds. Timing points only.
///
/// Per plain circle at `T`: `T` hide BPM (meter 999, omit-first-barline: the note, gone);
/// `T+1` wall BPM meter 1 (the sheet resumes) with SV = Don's or Kat's (the slit opens); `T+1/n`
/// beat SV = the base (it closes again).
pub fn hidden_anti_barline(
    notes: &[HitObject],
    start_ms: f64,
    end_ms: f64,
    base: &[TimingPoint],
    cfg: &GimmickConfig,
) -> Vec<TimingPoint> {
    let mut eligible: Vec<&HitObject> =
        notes.iter().filter(|n| n.is_circle() && !n.big()).collect();
    eligible.sort_by(|a, b| a.time.total_cmp(&b.time));
    if eligible.is_empty() {
        return Vec::new();
    }
    let beat_at = |t: f64| 60_000.0 / base_bpm_at(base, t);
    let (first, last) = (eligible[0], eligible[eligible.len() - 1]);
    let wall_start = round_ms(start_ms.min(first.time - beat_at(first.time)));
    let mut points = vec![
        red_at(wall_start, cfg.hidden_wall_bpm, HIDDEN_WALL_METER, false),
        green_at(wall_start, cfg.hidden_base_sv),
    ];
    for n in &eligible {
        let t = round_ms(n.time);
        let wall_at = t + HIDDEN_WALL_OFFSET_MS;
        let restore_at = (wall_at + 1).max(round_ms(
            t as f64 + beat_at(t as f64) / cfg.hidden_window_divisor as f64,
        ));
        points.push(red_at(
            t,
            if cfg.hide_note {
                cfg.hidden_hide_bpm
            } else {
                base_bpm_at(base, t as f64)
            },
            HIDDEN_HIDE_METER,
            true,
        ));
        points.push(red_at(
            wall_at,
            cfg.hidden_wall_bpm,
            HIDDEN_WALL_METER,
            false,
        ));
        points.push(green_at(
            wall_at,
            if n.taiko() == super::chart::Taiko::Kat {
                cfg.hidden_kat_sv
            } else {
                cfg.hidden_don_sv
            },
        ));
        points.push(green_at(restore_at, cfg.hidden_base_sv));
    }
    let wall_end = end_ms.max(last.time + beat_at(last.time));
    if let Some(closing) = active_red(base, wall_end) {
        let measure = closing.beat_length * closing.meter.max(1) as f64;
        let mut at = closing.time;
        if measure > 0.0 {
            at += ((wall_end - closing.time) / measure).ceil() * measure;
        }
        let floor = points.last().map_or(0, |p| p.time as i64 + 1);
        points.push(red_at(
            round_ms(at).max(floor),
            60_000.0 / closing.beat_length,
            closing.meter.max(1),
            false,
        ));
    }
    points.sort_by(|a, b| a.time.total_cmp(&b.time));
    points
}

/// Carries the map's kiai and hitsound volume at each point's time onto that point: otherwise the
/// first generated line inside a chorus switches the chorus off, and a structure placed in a quiet
/// section slams it back to full volume.
pub fn carry_active_state(points: &mut [TimingPoint], document: &[TimingPoint]) {
    let ordered = sorted_by_time(document);
    for p in points {
        if let Some(active) = active_at(&ordered, p.time) {
            if active.effects & EFFECT_KIAI != 0 {
                p.set_kiai(true);
            }
            p.volume = active.volume;
        }
    }
}

/// The plain red-line tool: one uninherited point at snap + `red_line_offset_ms`. `bpm` is the
/// base's own BPM there; `red_line_bpm` overrides it (a typed BPM is a standalone speed change).
pub fn red_line(time: i64, bpm: f64, cfg: &GimmickConfig) -> Built {
    Built {
        timing: vec![red_at(
            time + cfg.red_line_offset_ms,
            cfg.red_line_bpm.unwrap_or(bpm),
            cfg.red_line_meter,
            false,
        )],
        objects: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::chart::{Chart, Taiko};

    fn base() -> Vec<TimingPoint> {
        // 120 BPM from 0 (500 ms beat), SV 1.5 from 2000.
        vec![
            TimingPoint::red(0.0, 120.0),
            TimingPoint::green(2000.0, 1.5),
        ]
    }

    fn cfg() -> GimmickConfig {
        GimmickConfig::default()
    }

    fn assert_file_order(points: &[TimingPoint]) {
        for w in points.windows(2) {
            assert!(w[0].time <= w[1].time, "not sorted by time");
        }
        // A green line sharing a millisecond with a red is after it.
        for (i, p) in points.iter().enumerate() {
            if !p.is_red() {
                continue;
            }
            assert!(
                points[..i].iter().all(|q| q.is_red() || q.time != p.time),
                "a green line precedes the red one at {}",
                p.time
            );
        }
    }

    #[test]
    fn the_default_config_is_valid_and_bad_ones_are_refused() {
        assert!(cfg().validate().is_ok());
        let mut c = cfg();
        c.kat_spacing2_ms = c.kat_spacing1_ms;
        assert!(c.validate().unwrap_err().contains("different"));
        let mut c = cfg();
        c.fake_slider_length = 5.0;
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.fake_slider_offset_ms = 0;
        assert!(c.validate().is_err());
        let mut c = cfg();
        c.gimmick_bpm = 0.0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn a_don_barline_note_is_a_gimmick_line_and_a_restore_after_it() {
        let b = barline_note(1000, &base(), &cfg(), NoteKind::Don);
        // Don is not mirrored by default: gimmick line at T, restore at T+1.
        let got: Vec<(f64, f64)> = b.timing.iter().map(|p| (p.time, p.beat_length)).collect();
        assert_eq!(got, [(1000.0, 1.0), (1001.0, 500.0)]);
        assert_eq!(b.objects.len(), 1);
        assert_eq!(b.objects[0].taiko(), Taiko::Don);
        assert!(!b.timing[0].omits_barline());
    }

    #[test]
    fn a_kat_barline_note_is_three_mirrored_pairs() {
        let b = barline_note(1000, &base(), &cfg(), NoteKind::Kat);
        let times: Vec<f64> = b.timing.iter().map(|p| p.time).collect();
        assert_eq!(times, [995.0, 997.0, 999.0, 1000.0, 1001.0, 1003.0, 1005.0]);
        assert_eq!(b.timing[3].beat_length, 1.0, "the gimmick line is at T");
        assert_eq!(b.objects[0].taiko(), Taiko::Kat);
    }

    #[test]
    fn a_barline_note_without_a_note_or_hiding_changes_what_it_says() {
        let mut c = cfg();
        c.place_notes = false;
        c.hide_note = false;
        let b = barline_note(1000, &base(), &c, NoteKind::Don);
        assert!(b.objects.is_empty());
        // The line at T keeps the chart's BPM and omits its own bar.
        assert_eq!(b.timing[0].beat_length, 500.0);
        assert!(b.timing[0].omits_barline());
    }

    #[test]
    fn a_restore_bpm_overrides_the_base() {
        let mut c = cfg();
        c.restore_bpm = Some(200.0);
        let b = barline_note(1000, &base(), &c, NoteKind::Don);
        assert_eq!(b.timing[1].beat_length, 300.0);
    }

    #[test]
    fn a_plain_fake_slider_is_one_red_line_and_a_negative_roll() {
        let b = fake_slider(
            1000,
            &base(),
            &cfg(),
            FakeKind::Regular,
            256.0,
            192.0,
            false,
            None,
            false,
        );
        assert_eq!(b.timing.len(), 1);
        assert_eq!(b.timing[0].time, 1002.0);
        assert!(b.timing[0].omits_barline());
        assert_eq!(b.objects.len(), 1);
        let o = &b.objects[0];
        assert_eq!((o.time, o.taiko()), (1002.0, Taiko::Roll));
        assert!(o.length().unwrap() < 0.0);
        assert!(!o.big());
    }

    #[test]
    fn shift_makes_a_plain_fake_slider_big() {
        let b = fake_slider(
            1000,
            &base(),
            &cfg(),
            FakeKind::Regular,
            256.0,
            192.0,
            false,
            None,
            true,
        );
        assert!(b.objects[0].big());
    }

    #[test]
    fn a_shiny_stacks_sliders_beside_a_note_that_stays() {
        let b = fake_slider(
            1000,
            &base(),
            &cfg(),
            FakeKind::Don,
            256.0,
            192.0,
            true,
            None,
            false,
        );
        // The real note on the snap and three sliders one ms later; one red line on the snap.
        assert_eq!(b.objects.len(), 4);
        assert_eq!(b.objects[0].taiko(), Taiko::Don);
        assert!(b.objects[1..].iter().all(|o| o.time == 1001.0));
        assert_eq!(b.timing.len(), 1);
        assert_eq!(b.timing[0].time, 1000.0);
    }

    #[test]
    fn a_hidden_don_fake_slider_squashes_its_note_and_restores_after_the_slider() {
        let b = fake_slider(
            1000,
            &base(),
            &cfg(),
            FakeKind::Don,
            256.0,
            192.0,
            false,
            None,
            false,
        );
        let lines: Vec<(f64, bool)> = b.timing.iter().map(|p| (p.time, p.is_red())).collect();
        assert_eq!(lines, [(1000.0, true), (1000.0, false), (1002.0, true)]);
        assert_eq!(b.timing[0].beat_length, 1.0);
        assert_eq!(b.timing[1].sv(), 10.0);
        assert_eq!(b.timing[2].beat_length, 500.0);
        assert_file_order(&b.timing);
        // Kat's slider is the big one.
        let k = fake_slider(
            1000,
            &base(),
            &cfg(),
            FakeKind::Kat,
            256.0,
            192.0,
            false,
            None,
            false,
        );
        assert!(k.objects[1].big());
        assert_eq!(k.objects[0].taiko(), Taiko::Kat);
    }

    #[test]
    fn a_red_line_is_placed_at_snap_plus_offset() {
        let mut c = cfg();
        c.red_line_offset_ms = 3;
        c.red_line_bpm = Some(180.0);
        let b = red_line(1000, 120.0, &c);
        assert_eq!(b.timing[0].time, 1003.0);
        assert!((b.timing[0].beat_length - 60_000.0 / 180.0).abs() < 1e-9);
    }

    fn circles(times: &[f64]) -> Vec<HitObject> {
        times
            .iter()
            .map(|&t| HitObject::hit(t, false, false))
            .collect()
    }

    #[test]
    fn the_anti_barline_wall_has_a_slit_at_each_note() {
        let notes = circles(&[1000.0, 1500.0]);
        let pts = anti_barline(&notes, 900.0, 1600.0, &base(), &cfg(), None);
        assert!(!pts.is_empty());
        assert_file_order(&pts);
        // Each note: a gimmick-BPM line on its ms and a restore after it, both omitting the bar.
        for n in &notes {
            let squash = pts
                .iter()
                .find(|p| p.time == n.time && p.beat_length == 1.0)
                .expect("squash line");
            assert!(squash.omits_barline());
            let restore = pts
                .iter()
                .find(|p| p.time == n.time + 1.0 && p.meter == ANTI_RESTORE_METER)
                .expect("restore line");
            assert!(restore.omits_barline());
        }
        // Wall ticks are every 500/36 ms; one is missing around each note (the slit).
        let wall: Vec<f64> = pts
            .iter()
            .filter(|p| p.is_red() && p.beat_length > 1.0 && p.meter == 4)
            .map(|p| p.time)
            .collect();
        let near = |t: f64| wall.iter().filter(|&&w| (w - t).abs() < 7.0).count();
        assert!(
            near(1000.0) == 0,
            "wall thins out around a note: {}",
            near(1000.0)
        );
        // Spacing between consecutive wall ticks away from the slits is about a 36th of a beat.
        let gaps: Vec<f64> = wall.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().any(|&g| (g - 500.0 / 36.0).abs() < 1.1));
    }

    #[test]
    fn the_anti_barline_wall_carries_the_charts_sv() {
        let notes = circles(&[2200.0]);
        let pts = anti_barline(&notes, 2100.0, 2300.0, &base(), &cfg(), None);
        let greens: Vec<&TimingPoint> = pts.iter().filter(|p| !p.is_red()).collect();
        assert!(!greens.is_empty());
        assert!(greens.iter().all(|g| (g.sv() - 1.5).abs() < 1e-9));
    }

    #[test]
    fn finishers_and_rolls_are_left_alone_by_the_anti_barline() {
        let mut notes = circles(&[1000.0]);
        notes.push(HitObject::hit(1500.0, false, true));
        notes.push(HitObject::roll(2000.0, 100.0, false));
        let pts = anti_barline(&notes, 900.0, 2100.0, &base(), &cfg(), None);
        assert!(pts.iter().all(|p| p.time != 1500.0 || p.beat_length != 1.0));
        assert!(pts.iter().all(|p| p.time != 2000.0 || p.beat_length != 1.0));
        assert!(anti_barline(&[], 0.0, 100.0, &base(), &cfg(), None).is_empty());
    }

    #[test]
    fn the_hidden_anti_barline_is_four_lines_per_note_and_a_wall_line() {
        let notes = circles(&[1000.0]);
        let pts = anti_barline_hidden(&notes);
        assert_file_order(&pts);
        let at = |t: f64| pts.iter().filter(|p| p.time == t).collect::<Vec<_>>();
        // The note: hide line, then wall line + the slit's SV on the next ms.
        assert_eq!(at(1000.0).len(), 1);
        assert_eq!(at(1000.0)[0].meter, HIDDEN_HIDE_METER);
        assert!(at(1000.0)[0].omits_barline());
        let slit = at(1001.0);
        assert_eq!(slit.len(), 2);
        assert!(slit[0].is_red() && slit[0].meter == HIDDEN_WALL_METER);
        assert!((slit[1].sv() - 100.0 / 9900.0).abs() < 1e-6);
        // 500 / 8 = 62.5 ms later (rounded) the slit closes.
        let close = pts.iter().find(|p| p.time == 1062.0 || p.time == 1063.0);
        assert!(close.is_some_and(|p| (p.sv() - 0.01).abs() < 1e-6));
        // The wall starts a beat before the first note.
        assert_eq!(pts[0].time, 500.0);
    }

    fn anti_barline_hidden(notes: &[HitObject]) -> Vec<TimingPoint> {
        hidden_anti_barline(notes, 900.0, 1200.0, &base(), &cfg())
    }

    #[test]
    fn kiai_and_volume_are_carried_onto_generated_lines() {
        let mut doc = base();
        doc[0].set_kiai(true);
        doc[0].volume = 60;
        let mut b = barline_note(1000, &base(), &cfg(), NoteKind::Don);
        carry_active_state(&mut b.timing, &doc);
        assert!(b.timing.iter().all(|p| p.kiai() && p.volume == 60));
    }

    #[test]
    fn keep_sv_writes_a_green_line_after_each_red_and_replaces_a_stacked_one() {
        let pts = vec![
            TimingPoint::red(1000.0, 60_000.0),
            TimingPoint::green(1000.0, 9.0),
            TimingPoint::red(1002.0, 120.0),
        ];
        let sv = StepSv(&[TimingPoint::red(0.0, 120.0), TimingPoint::green(500.0, 2.0)]);
        let kept = keep_sv(pts, &sv, None);
        let shape: Vec<(f64, bool)> = kept.iter().map(|p| (p.time, p.is_red())).collect();
        assert_eq!(
            shape,
            [
                (1000.0, true),
                (1000.0, false),
                (1002.0, true),
                (1002.0, false)
            ]
        );
        assert!(
            kept.iter()
                .filter(|p| !p.is_red())
                .all(|p| (p.sv() - 2.0).abs() < 1e-9)
        );
    }

    /// Every structure goes through the game's own parser: bars, fake bodies and velocities come
    /// out as osu! would draw them.
    #[test]
    fn built_structures_parse_in_the_game() {
        let mut chart = Chart::parse(crate::editor::chart::tests::MAP);
        let b = barline_note(1000, &chart.timing.clone(), &cfg(), NoteKind::Kat);
        for p in b.timing {
            chart.insert_timing(p);
        }
        for o in b.objects {
            chart.insert_object(o);
        }
        let f = fake_slider(
            3000,
            &chart.timing.clone(),
            &cfg(),
            FakeKind::Kat,
            256.0,
            192.0,
            false,
            None,
            false,
        );
        for p in f.timing {
            chart.insert_timing(p);
        }
        for o in f.objects {
            chart.insert_object(o);
        }
        chart.validate().unwrap();
        let text = chart.to_text();
        let map = crate::beatmap::from_text(&text).expect("the game reads it");
        assert!(!map.notes.is_empty());
    }
}
