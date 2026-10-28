//! Difficulty over time for the song select timeline. Uses osu!'s own strain peaks (via
//! `stars::strain_curve`) when available; otherwise a small fallback model in the same spirit —
//! hit rate, colour and rhythm changes, decaying, sampled per 400 ms section.

use crate::beatmap::{Beatmap, Kind};

/// Section length used by lazer's strain skills.
const SECTION_MS: f64 = 400.0;
/// Strain left after one second without notes.
const DECAY_PER_SECOND: f64 = 0.3;

pub struct Graph {
    /// Map time of the first and last object.
    pub start: f64,
    pub end: f64,
    /// Peak strain per section, normalised to 0..1 (1 = hardest section).
    pub peaks: Vec<f32>,
    /// Map time where the first section starts, and section length in map time.
    pub peaks_start: f64,
    pub section: f64,
    /// Densest second: notes per second and where it starts.
    pub peak_nps: f32,
    pub peak_nps_at: f64,
    /// Kiai time ranges.
    pub kiai: Vec<(f64, f64)>,
}

impl Graph {
    #[cfg(test)]
    pub fn section_ms(&self) -> f64 {
        SECTION_MS
    }

    /// Peak strain over `bins` equal slices (max of the sections in each), for drawing.
    pub fn bins(&self, bins: usize) -> Vec<f32> {
        if self.peaks.is_empty() || bins == 0 {
            return vec![0.0; bins];
        }
        (0..bins)
            .map(|b| {
                let lo = b * self.peaks.len() / bins;
                let hi = ((b + 1) * self.peaks.len() / bins).max(lo + 1).min(self.peaks.len());
                self.peaks[lo..hi].iter().copied().fold(0.0, f32::max)
            })
            .collect()
    }
}

pub fn analyze(map: &Beatmap) -> Graph {
    let start = map.notes.first().map(|n| n.time).unwrap_or(0.0);
    let end = map.notes.iter().map(|n| n.end_time).fold(start, f64::max);
    let sections = (((end - start) / SECTION_MS).ceil() as usize).max(1);
    let mut peaks = vec![0.0f64; sections];
    let section_of = |t: f64| (((t - start) / SECTION_MS) as usize).min(sections - 1);
    let decay = |strain: f64, ms: f64| strain * DECAY_PER_SECOND.powf(ms / 1000.0);

    let mut strain = 0.0f64;
    let mut last_t: Option<f64> = None;
    let mut last_dt: Option<f64> = None;
    let mut last_kat: Option<bool> = None;
    let mut run = 0u32;
    for n in &map.notes {
        let t = n.time;
        if let Some(prev) = last_t {
            // Sections passed with no notes still show the decaying strain.
            let mut s = section_of(prev) + 1;
            while s <= section_of(t) && s < sections {
                let section_start = start + s as f64 * SECTION_MS;
                peaks[s] = peaks[s].max(decay(strain, section_start - prev));
                s += 1;
            }
        }
        let dt = last_t.map(|p| (t - p).max(25.0));
        if let Some(dt) = dt {
            strain = decay(strain, dt);
        }
        let contribution = match n.kind {
            Kind::Don | Kind::Kat => {
                let kat = n.kind == Kind::Kat;
                // Stamina: faster notes push harder.
                let rate = dt.map(|dt| 1000.0 / dt).unwrap_or(1.0).min(40.0);
                // Colour: switching colour (especially after a long same-colour run) is harder.
                let colour = match last_kat {
                    Some(k) if k != kat => {
                        let bonus = 1.25 + 0.05 * run.min(4) as f64;
                        run = 0;
                        bonus
                    }
                    _ => {
                        run += 1;
                        1.0
                    }
                };
                last_kat = Some(kat);
                // Rhythm: a change in spacing between notes is harder than a steady stream.
                let rhythm = match (dt, last_dt) {
                    (Some(a), Some(b)) => {
                        let ratio = a.max(b) / a.min(b);
                        if ratio > 1.15 { 1.0 + 0.25 * (ratio - 1.0).min(2.0) } else { 1.0 }
                    }
                    _ => 1.0,
                };
                last_dt = dt;
                if n.big { 0.12 * rate * colour * rhythm * 1.1 } else { 0.12 * rate * colour * rhythm }
            }
            // Drumrolls and swells are mashing: a modest, steady load.
            Kind::Roll | Kind::Swell => 0.6,
        };
        strain += contribution;
        let s = section_of(t);
        peaks[s] = peaks[s].max(strain);
        last_t = Some(t);
    }

    let max = peaks.iter().copied().fold(0.0, f64::max).max(1e-9);
    let peaks = peaks.iter().map(|&p| (p / max) as f32).collect();

    // Densest one-second window of hits.
    let hits: Vec<f64> = map.notes.iter().filter(|n| matches!(n.kind, Kind::Don | Kind::Kat)).map(|n| n.time).collect();
    let (mut peak_nps, mut peak_nps_at, mut j) = (0usize, start, 0usize);
    for (i, &t) in hits.iter().enumerate() {
        // Half-open window (t - 1000, t]: exactly one second.
        while hits[j] <= t - 1000.0 {
            j += 1;
        }
        if i + 1 - j > peak_nps {
            peak_nps = i + 1 - j;
            peak_nps_at = hits[j];
        }
    }

    // Kiai ranges from the effect points.
    let mut kiai = Vec::new();
    let mut open: Option<f64> = None;
    for cp in &map.control_points {
        match (cp.kiai, open) {
            (true, None) => open = Some(cp.time),
            (false, Some(s)) => {
                kiai.push((s, cp.time));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(s) = open {
        kiai.push((s, end));
    }

    Graph { start, end, peaks, peaks_start: start, section: SECTION_MS, peak_nps: peak_nps as f32, peak_nps_at, kiai }
}

impl Graph {
    /// Replaces the fallback model with osu!'s strain peaks (`stars::strain_curve`).
    pub fn use_curve(&mut self, (peaks, start, section): (Vec<f64>, f64, f64)) {
        let max = peaks.iter().copied().fold(0.0, f64::max);
        if peaks.is_empty() || max <= 0.0 {
            return;
        }
        self.peaks = peaks.iter().map(|&p| (p / max) as f32).collect();
        self.peaks_start = start;
        self.section = section;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beatmap::{Beatmap, ControlPoint, Meta, Note};

    fn note(time: f64, kat: bool) -> Note {
        Note { time, end_time: time, kind: if kat { Kind::Kat } else { Kind::Don }, big: false, velocity: 0.5, tick_spacing: 0.0, required_hits: 0, fake_len: 0.0, fake_fades: false, fake_length: 0.0, tiny: false, unhittable: false }
    }

    fn map(notes: Vec<Note>) -> Beatmap {
        let control_points = vec![
            ControlPoint { time: 0.0, beat_origin: 0.0, beat_length: 500.0, sv: 1.0, kiai: false, volume: 1.0 },
            ControlPoint { time: 20_000.0, beat_origin: 0.0, beat_length: 500.0, sv: 1.0, kiai: true, volume: 1.0 },
            ControlPoint { time: 24_000.0, beat_origin: 0.0, beat_length: 500.0, sv: 1.0, kiai: false, volume: 1.0 },
        ];
        Beatmap { meta: Meta::default(), notes, barlines: Vec::new(), control_points }
    }

    #[test]
    fn peak_is_where_the_dense_alternating_stream_is() {
        // Slow single notes for 20 s, a fast alternating burst at 20-24 s, then slow again.
        let mut notes: Vec<Note> = (0..40).map(|i| note(i as f64 * 500.0, false)).collect();
        notes.extend((0..64).map(|i| note(20_000.0 + i as f64 * 62.5, i % 2 == 1)));
        notes.extend((0..20).map(|i| note(24_500.0 + i as f64 * 500.0, false)));
        let g = analyze(&map(notes));
        let hardest = g.peaks.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let hardest_ms = g.start + hardest as f64 * g.section_ms();
        assert!((20_000.0..24_500.0).contains(&hardest_ms), "peak at {hardest_ms}");
        assert!(g.peaks[5] < 0.3, "slow intro should be easy: {}", g.peaks[5]);
        assert_eq!(g.peak_nps, 16.0);
        assert!((20_000.0..21_000.0).contains(&g.peak_nps_at));
        assert_eq!(g.kiai, vec![(20_000.0, 24_000.0)]);
        // Binning keeps the peak.
        assert!(g.bins(10).iter().any(|&v| v > 0.99));
    }
}
