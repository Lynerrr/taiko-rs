//! A TJA course as its leaderboard knows it (`BoardChart`): everything judging, scoring and the
//! mods read from the chart, and nothing else (no title, audio or file text). A TJA score is sent
//! with it, so the server can check it against the board's hash and play the replay back on it,
//! with no file to keep.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ChartHash, CourseKind, SLIDER_MULTIPLIER};
use crate::beatmap::{Barline, Beatmap, ControlPoint, Kind, Meta, Note, Special, WindowSource};
use crate::branch::{Branching, Measure};

/// What `BoardChart::hash` hashes first: a new normalization gets a new tag, so old and new
/// hashes never mix. v1 covered the notes only, so a chart with gogo time added would have
/// scored more on the same board.
const HASH_TAG: &[u8] = b"taiko.rs tja chart v2\n";
/// The most notes, barlines or timing points a chart sent with a score may have: the longest
/// charts have a few thousand notes.
pub const MAX_ITEMS: usize = 50_000;
/// The furthest from the song's start a time may be, in ms (a little over a day).
const MAX_TIME_MS: f64 = 1e8;
/// The most pops a balloon may ask for.
const MAX_HITS: u32 = 100_000;

/// A course's chart for its leaderboard. Times in map ms, speeds in osu!px per ms, as in
/// `Beatmap`; the tuples go over the wire as JSON arrays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardChart {
    /// The course (`CourseKind` as its number): its hit windows.
    pub course: u8,
    /// (time, end time, kind: 0 don 1 kat 2 roll 3 balloon, big, velocity, tick spacing, pops).
    pub notes: Vec<(f64, f64, u8, bool, f64, f64, u32)>,
    /// (time, velocity).
    pub barlines: Vec<(f64, f64)>,
    /// (time, beat origin, beat length, scroll, gogo).
    pub points: Vec<(f64, f64, f64, f64, bool)>,
    pub first_velocity: f64,
    /// A branching chart's branches (`crate::branch`): `notes` has every branch's. Absent
    /// for the rest, whose hash it leaves as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branches: Option<Branching>,
    /// OpenTaiko's notes (`Special`), as (note index, `special_id`), in note order. Absent for
    /// a chart with none, whose hash it leaves as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specials: Option<Vec<(u32, u8)>>,
}

impl BoardChart {
    /// The board chart of course `kind` read into `map`.
    pub fn of(kind: CourseKind, map: &Beatmap) -> BoardChart {
        BoardChart {
            course: kind.id(),
            notes: map
                .notes
                .iter()
                .map(|n| {
                    let kind = match n.kind {
                        Kind::Don => 0,
                        Kind::Kat => 1,
                        Kind::Roll => 2,
                        Kind::Swell => 3,
                    };
                    (
                        n.time,
                        n.end_time,
                        kind,
                        n.big,
                        n.velocity,
                        n.tick_spacing,
                        n.required_hits,
                    )
                })
                .collect(),
            barlines: map.barlines.iter().map(|b| (b.time, b.velocity)).collect(),
            points: map
                .control_points
                .iter()
                .map(|p| (p.time, p.beat_origin, p.beat_length, p.sv, p.kiai))
                .collect(),
            first_velocity: map.first_velocity,
            branches: map.branching.as_deref().cloned(),
            specials: {
                let s: Vec<(u32, u8)> = (map.notes.iter().enumerate())
                    .filter(|(_, n)| n.special != Special::None)
                    .map(|(i, n)| (i as u32, special_id(n.special)))
                    .collect();
                (!s.is_empty()).then_some(s)
            },
        }
    }

    /// The course it is, if its number is one.
    pub fn kind(&self) -> Option<CourseKind> {
        [
            CourseKind::Easy,
            CourseKind::Normal,
            CourseKind::Hard,
            CourseKind::Oni,
            CourseKind::Edit,
        ]
        .into_iter()
        .find(|k| k.id() == self.course)
    }

    /// The hash its board is known by (leaderboards, local scores): the same for the same chart
    /// however its file was written, and different as soon as anything played changes. Times to
    /// 1/100 ms and speeds to 1e-6, so float noise can't split a board.
    pub fn hash(&self) -> ChartHash {
        let q = |v: f64, scale: f64| ((v * scale).round() as i64).to_le_bytes();
        let mut h = Sha256::new();
        h.update(HASH_TAG);
        h.update([self.course]);
        h.update(q(self.first_velocity, 1e6));
        h.update((self.notes.len() as u64).to_le_bytes());
        for &(time, end, kind, big, velocity, ticks, hits) in &self.notes {
            h.update([kind, big as u8]);
            h.update(q(time, 100.0));
            h.update(q(end, 100.0));
            h.update(hits.to_le_bytes());
            h.update(q(velocity, 1e6));
            h.update(q(ticks, 100.0));
        }
        h.update((self.barlines.len() as u64).to_le_bytes());
        for &(time, velocity) in &self.barlines {
            h.update(q(time, 100.0));
            h.update(q(velocity, 1e6));
        }
        h.update((self.points.len() as u64).to_le_bytes());
        for &(time, origin, beat, scroll, gogo) in &self.points {
            h.update(q(time, 100.0));
            h.update(q(origin, 100.0));
            h.update(q(beat, 1e4));
            h.update(q(scroll, 1e6));
            h.update([gogo as u8]);
        }
        if let Some(b) = &self.branches {
            h.update(b"branches\n");
            for tag in &b.notes {
                match tag {
                    None => h.update([0xFF]),
                    Some((point, branch)) => {
                        h.update([*branch]);
                        h.update(point.to_le_bytes());
                    }
                }
            }
            h.update((b.points.len() as u64).to_le_bytes());
            for p in &b.points {
                let c = &p.condition;
                h.update(q(p.judge, 100.0));
                h.update(q(p.start, 100.0));
                h.update([measure_id(c.measure), c.big_only as u8, c.below as u8]);
                h.update(q(c.expert, 1e4));
                h.update(q(c.master, 1e4));
                h.update(p.hold.map(u8::from));
            }
            for times in [&b.resets, &b.holds] {
                h.update((times.len() as u64).to_le_bytes());
                for &t in times {
                    h.update(q(t, 100.0));
                }
            }
        }
        if let Some(s) = &self.specials {
            h.update(b"specials\n");
            h.update((s.len() as u64).to_le_bytes());
            for &(i, id) in s {
                h.update(i.to_le_bytes());
                h.update([id]);
            }
        }
        ChartHash(h.finalize().into())
    }

    /// The chart to play, with the course's arcade windows; an error for one no TJA file gives
    /// (what a server checks before it plays a replay on a chart it was sent).
    pub fn beatmap(&self) -> Result<Beatmap, String> {
        let kind = self.kind().ok_or("not a course")?;
        if self.notes.is_empty() || self.points.is_empty() {
            return Err("no notes or no timing".into());
        }
        if [self.notes.len(), self.barlines.len(), self.points.len()]
            .iter()
            .any(|&n| n > MAX_ITEMS)
        {
            return Err("too long a chart".into());
        }
        let time = |t: f64| t.is_finite() && t.abs() <= MAX_TIME_MS;
        let speed = |v: f64| v.is_finite() && v > 0.0 && v < 1e6;
        let notes_ok = self.notes.iter().all(|&(t, end, kind, _, v, ticks, hits)| {
            time(t)
                && time(end)
                && end >= t
                && kind <= 3
                && speed(v)
                && ticks.is_finite()
                && (0.0..=MAX_TIME_MS).contains(&ticks)
                && hits <= MAX_HITS
        });
        let bars_ok = self.barlines.iter().all(|&(t, v)| time(t) && speed(v));
        let points_ok = self.points.iter().all(|&(t, origin, beat, scroll, _)| {
            time(t) && time(origin) && speed(beat) && speed(scroll)
        });
        if !notes_ok || !bars_ok || !points_ok || !speed(self.first_velocity) {
            return Err("a time or speed out of range".into());
        }
        if let Some(s) = &self.specials {
            let mut last = None;
            for &(i, id) in s {
                let kind = self.notes.get(i as usize).map(|n| n.2);
                let fits = match id {
                    1..=3 => matches!(kind, Some(0 | 1)),
                    4 => kind == Some(3),
                    _ => false,
                };
                if !fits || last.is_some_and(|l| l >= i) {
                    return Err("a special note out of place".into());
                }
                last = Some(i);
            }
        }
        if let Some(b) = &self.branches {
            b.check(self.notes.len())?;
            if b.resets.len() + b.holds.len() > MAX_ITEMS {
                return Err("too long a chart".into());
            }
        }
        if !sorted(self.notes.iter().map(|n| n.0))
            || !sorted(self.barlines.iter().map(|b| b.0))
            || !sorted(self.points.iter().map(|p| p.0))
        {
            return Err("out of order".into());
        }
        let mut special = vec![Special::None; self.notes.len()];
        for &(i, id) in self.specials.iter().flatten() {
            special[i as usize] = SPECIALS[usize::from(id)];
        }
        Ok(Beatmap {
            meta: meta(kind),
            notes: self
                .notes
                .iter()
                .zip(special)
                .map(
                    |(&(time, end_time, k, big, velocity, tick_spacing, hits), special)| Note {
                        time,
                        end_time,
                        kind: [Kind::Don, Kind::Kat, Kind::Roll, Kind::Swell][usize::from(k)],
                        big,
                        velocity,
                        tick_spacing,
                        required_hits: hits,
                        fake_len: 0.0,
                        fake_fades: false,
                        fake_length: 0.0,
                        tiny: false,
                        unhittable: false,
                        special,
                    },
                )
                .collect(),
            barlines: self
                .barlines
                .iter()
                .map(|&(time, velocity)| Barline { time, velocity })
                .collect(),
            control_points: self
                .points
                .iter()
                .map(|&(time, beat_origin, beat_length, sv, kiai)| ControlPoint {
                    time,
                    beat_origin,
                    beat_length,
                    sv,
                    kiai,
                    volume: 1.0,
                })
                .collect(),
            first_velocity: self.first_velocity,
            gimmick: None,
            fakes: Vec::new(),
            branching: self.branches.clone().map(std::sync::Arc::new),
        })
    }
}

/// `Special`s by their number in `BoardChart::specials` (0 is none).
const SPECIALS: [Special; 5] = [
    Special::None,
    Special::Kadon,
    Special::Adlib,
    Special::Bomb,
    Special::Fuse,
];

/// A special note's number in `BoardChart::specials` and the hash: fixed, whatever the enum's
/// order.
fn special_id(s: Special) -> u8 {
    match s {
        Special::None => 0,
        Special::Kadon => 1,
        Special::Adlib => 2,
        Special::Bomb => 3,
        Special::Fuse => 4,
    }
}

/// A condition's number in the hash: fixed, whatever the enum's order.
fn measure_id(m: Measure) -> u8 {
    match m {
        Measure::Keep => 0,
        Measure::Accuracy => 1,
        Measure::GreatShare => 2,
        Measure::Greats => 3,
        Measure::Oks => 4,
        Measure::Misses => 5,
        Measure::Rolls => 6,
        Measure::BalloonHits => 7,
        Measure::Score => 8,
    }
}

/// Whether `times` never go back.
fn sorted(mut times: impl Iterator<Item = f64>) -> bool {
    let mut last = f64::NEG_INFINITY;
    times.all(|t| std::mem::replace(&mut last, t) <= t)
}

/// What every TJA course plays with: osu!'s defaults for what a TJA file doesn't say, and the
/// course's arcade windows.
pub(super) fn meta(kind: CourseKind) -> Meta {
    Meta {
        version: kind.name().to_string(),
        mode: 1,
        od: 5.0,
        hp: 5.0,
        slider_multiplier: SLIDER_MULTIPLIER,
        slider_tick_rate: 1.0,
        format_version: 14,
        windows: WindowSource::Arcade {
            narrow: kind.narrow_windows(),
        },
        ..Default::default()
    }
}
