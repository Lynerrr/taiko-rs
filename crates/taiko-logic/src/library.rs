//! The library's logic apart from its files: the search box's query language (`search`) and
//! the numbers a map is sorted by (`Info`, worked out from the map's bytes).

pub mod search;

use rosu_pp::taiko::Taiko;
use rosu_pp::{Beatmap, Difficulty};

/// Per-map numbers for sorting the whole library.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub stars: f32,
    pub length_ms: f32,
    pub bpm: f32,
}

impl Info {
    /// Star rating (taiko at 1.0x, converts included), drain length and BPM of a map's .osu
    /// text; None for a map that can't be read or rated.
    pub fn of(bytes: &[u8]) -> Option<Info> {
        let map = Beatmap::from_bytes(bytes).ok()?;
        if !crate::stars::calculable(&map) {
            return None;
        }
        let stars = Difficulty::new()
            .calculate_for_mode::<Taiko>(&map)
            .ok()?
            .stars as f32;
        let first = map.hit_objects.first().map(|h| h.start_time).unwrap_or(0.0);
        let last = map.hit_objects.last().map(|h| h.start_time).unwrap_or(0.0);
        Some(Info {
            stars,
            length_ms: (last - first).max(0.0) as f32,
            bpm: map.bpm() as f32,
        })
    }

    /// The same numbers for a chart read straight into a `Beatmap` (a TJA course): rated as a
    /// taiko map of its dons and kats (rolls and balloons add nothing to taiko's star rating),
    /// timed by its timing points.
    pub fn of_chart(chart: &crate::beatmap::Beatmap) -> Option<Info> {
        use crate::beatmap::Kind;
        use rosu_pp::model::control_point::TimingPoint;
        use rosu_pp::model::hit_object::{HitObject, HitObjectKind, Pos};
        use rosu_pp::model::mode::GameMode;
        let hits: Vec<&crate::beatmap::Note> = chart
            .notes
            .iter()
            .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat))
            .collect();
        let mut timing: Vec<TimingPoint> = Vec::new();
        for p in &chart.control_points {
            if timing.last().is_none_or(|t| t.beat_len != p.beat_length) {
                timing.push(TimingPoint::new(p.time, p.beat_length));
            }
        }
        let map = Beatmap {
            mode: GameMode::Taiko,
            od: chart.meta.od as f32,
            hp: chart.meta.hp as f32,
            timing_points: timing,
            hit_objects: hits
                .iter()
                .map(|n| HitObject {
                    pos: Pos::default(),
                    start_time: n.time,
                    kind: HitObjectKind::Circle,
                })
                .collect(),
            // osu!'s hitsounds: whistle or clap make a kat, finish a big note.
            hit_sounds: hits
                .iter()
                .map(|n| {
                    let kat = if n.kind == Kind::Kat { 2 } else { 0 };
                    let big = if n.big { 4 } else { 0 };
                    (kat | big).into()
                })
                .collect(),
            ..Default::default()
        };
        let stars = Difficulty::new()
            .calculate_for_mode::<Taiko>(&map)
            .ok()?
            .stars as f32;
        let first = hits.first().map_or(0.0, |n| n.time);
        let last = chart.notes.last().map_or(0.0, |n| n.end_time);
        Some(Info {
            stars,
            length_ms: (last - first).max(0.0) as f32,
            bpm: map.bpm() as f32,
        })
    }
}
