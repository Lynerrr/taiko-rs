//! Offset calibration: play a click track with notes exactly on the beat, then turn the hit
//! errors into a suggested offset.

use crate::game::Game;
use crate::render::{rgba, Canvas, Hit};
use winit::keyboard::KeyCode;

/// Clicks before the first note, so the pulse is established before you have to hit.
pub const COUNT_IN_BEATS: u32 = 8;
pub const BPM_RANGE: (f64, f64) = (60.0, 240.0);

/// About 30 seconds of notes at any tempo (at least 24), after the count-in.
pub fn total_beats(beat_ms: f64) -> u32 {
    COUNT_IN_BEATS + ((30_000.0 / beat_ms).ceil() as u32).max(24)
}
/// Hits needed before a result is trusted.
pub const MIN_HITS: usize = 16;

#[derive(Clone, Copy)]
pub struct Estimate {
    pub hits: usize,
    /// Robust average error: positive means you hit late.
    pub error_ms: f64,
    /// Consistency, as osu! unstable rate (10x standard deviation).
    pub ur: f64,
    /// 95% confidence half-width of `error_ms`: 1.96 * sigma / sqrt(hits).
    pub margin_ms: f64,
    pub suggested_offset: f64,
}

/// Median first, then the mean of hits within 60 ms of it, so stray early/late taps don't skew
/// the result.
pub fn estimate(errors: &[f64], current_offset: f64) -> Option<Estimate> {
    if errors.is_empty() {
        return None;
    }
    let mut sorted = errors.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let kept: Vec<f64> = errors.iter().copied().filter(|e| (e - median).abs() <= 60.0).collect();
    let mean = kept.iter().sum::<f64>() / kept.len() as f64;
    let var = kept.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / kept.len() as f64;
    Some(Estimate {
        hits: kept.len(),
        error_ms: mean,
        ur: var.sqrt() * 10.0,
        margin_ms: 1.96 * var.sqrt() / (kept.len() as f64).sqrt(),
        // Hitting late by e ms means the audio reaches you e ms later than assumed.
        suggested_offset: (current_offset + mean).round(),
    })
}

pub fn draw_overlay(c: &mut Canvas, game: &Game, current_offset: f64, hide_notes: bool) {
    let w = c.width;
    c.round_rect(w / 2.0 - 250.0, 26.0, 500.0, 70.0, 10.0, rgba(0x000000, 0.6), rgba(0xEB452C, 0.8), 1.5);
    c.text_centered(w / 2.0, 36.0, 12.0, rgba(0xffffff, 1.0), "OFFSET CALIBRATION");
    let line = if hide_notes {
        "Notes hidden: hit centre (F/J) on every click, by ear"
    } else {
        "Hit centre (F/J) on every click. Follow the sound, not the notes"
    };
    c.text_centered(w / 2.0, 56.0, 8.0, rgba(0xc8c8d8, 1.0), line);
    let status = match estimate(&game.stats.errors, current_offset) {
        Some(e) => {
            let dir = if e.error_ms >= 0.0 { "late" } else { "early" };
            format!(
                "{} hits   {:.1} ms {dir}   suggested offset {:+.0} ms{}",
                e.hits,
                e.error_ms.abs(),
                e.suggested_offset,
                if e.hits < MIN_HITS { "  (keep going)" } else { "" }
            )
        }
        None => format!("current offset {current_offset:+.0} ms   waiting for hits..."),
    };
    c.text_centered(w / 2.0, 74.0, 9.0, rgba(0xFFD34D, 1.0), &status);
    // Above the playfield size readout in the bottom-right corner.
    c.text_centered(w / 2.0, 448.0, 8.0, rgba(0x6f6f86, 1.0), &Canvas::fit_hints(8.0, w - 48.0, "H hide/show notes    Enter finish early    Esc cancel"));
}

pub fn draw_result(c: &mut Canvas, e: &Estimate, current_offset: f64, bpm: f64) {
    let w = c.width;
    let cx = w / 2.0;
    c.text_centered(cx, 70.0, 14.0, rgba(0x9a9ab0, 1.0), "OFFSET CALIBRATION");
    let dir = if e.error_ms >= 0.0 { "late" } else { "early" };
    c.text_centered(cx, 120.0, 16.0, rgba(0xffffff, 1.0), &format!("You hit {:.1} +-{:.1} ms {dir} on average", e.error_ms.abs(), e.margin_ms));
    c.text_centered(cx, 170.0, 40.0, rgba(0xFFD34D, 1.0), &format!("{:+.0} ms", e.suggested_offset));
    c.text_centered(cx, 228.0, 10.0, rgba(0xc8c8d8, 1.0), &format!("suggested offset (currently {current_offset:+.0} ms)"));
    let consistency = match e.ur {
        u if u < 100.0 => "very consistent",
        u if u < 160.0 => "consistent",
        u if u < 240.0 => "a bit loose - retry for a more reliable value",
        _ => "inconsistent - retry and focus on the click",
    };
    c.text_centered(cx, 270.0, 10.0, rgba(0x9a9ab0, 1.0), &format!("{} hits, UR {:.0} ({consistency})", e.hits, e.ur));
    if e.hits < MIN_HITS {
        c.text_centered(cx, 292.0, 9.0, rgba(0xFF4B4B, 1.0), &format!("Only {} hits; at least {MIN_HITS} recommended", e.hits));
    }
    c.button_row(352.0, &[
        ("Apply  (Enter)", Hit::Key(KeyCode::Enter), true),
        ("Retry  (R)", Hit::Key(KeyCode::KeyR), false),
        ("Discard  (Esc)", Hit::Key(KeyCode::Escape), false),
    ]);
    let tempo = format!("retry tempo: {bpm:.0} BPM   (Left/Right)");
    let tw = Canvas::text_width(9.0, &tempo);
    c.text_centered(cx, 388.0, 9.0, rgba(0x9a9ab0, 1.0), &tempo);
    c.button(cx - tw / 2.0 - 30.0, 384.0, 22.0, 16.0, "<", 8.0, Hit::Key(KeyCode::ArrowLeft), false);
    c.button(cx + tw / 2.0 + 8.0, 384.0, 22.0, 16.0, ">", 8.0, Hit::Key(KeyCode::ArrowRight), false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_land_on_click_samples() {
        for bpm in [60.0, 120.0, 175.0, 240.0] {
            let beat_ms = 60000.0 / bpm;
            let beats = total_beats(beat_ms);
            let map = crate::beatmap::calibration_map(beat_ms, COUNT_IN_BEATS, beats);
            let track = crate::audio::metronome(48000, beat_ms, beats, 2000.0);
            assert_eq!(map.notes.len() as u32, beats - COUNT_IN_BEATS);
            for n in &map.notes {
                // The click's first non-silent sample is the note's exact time.
                let frame = (n.time / 1000.0 * 48000.0).round() as usize;
                let before = track.samples[(frame - 1) * 2].abs();
                let after = track.samples[(frame + 1) * 2].abs();
                assert!(before < 1e-6 && after > 1e-4, "bpm {bpm}: click not at note {}", n.time);
            }
        }
    }

    #[test]
    fn estimate_ignores_stray_hits() {
        let mut errors = vec![12.0; 20];
        errors.extend([8.0, 16.0, 180.0, -140.0]);
        let e = estimate(&errors, 20.0).unwrap();
        assert_eq!(e.hits, 22);
        assert!((e.error_ms - 12.0).abs() < 1e-9);
        assert_eq!(e.suggested_offset, 32.0);
    }
}
