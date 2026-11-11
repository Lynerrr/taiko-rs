//! Offset calibration's math: how long the click track runs, and what the hit errors say the
//! offset should be. The client draws the calibration screens around it.

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
    let kept: Vec<f64> = errors
        .iter()
        .copied()
        .filter(|e| (e - median).abs() <= 60.0)
        .collect();
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

/// With no note to tell centre from rim by, any drum key is a hit: the rims (the outer keys of
/// `Config::keys`) count as the centre next to them.
pub fn centre_key(key: usize) -> usize {
    match key {
        0 => 1,
        3 => 2,
        k => k,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
