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
}
