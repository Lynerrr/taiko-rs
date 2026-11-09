//! OD Override's setup: the OD a play uses, whatever the map says. It replaces the OD EZ, HR and
//! Precise would give; the speed still shifts it, as the windows are timed in the song.

use std::sync::atomic::{AtomicU64, Ordering};

pub const MIN: f64 = 0.0;
pub const MAX: f64 = 20.0;
pub const DEFAULT: f64 = 10.0;
/// What Left and Right change it by, and with Shift.
pub(crate) const STEP: f64 = 0.5;
pub(crate) const FINE_STEP: f64 = 0.1;

/// Within the range, in tenths (so steps don't drift); `DEFAULT` for what isn't a number.
pub fn clamped(od: f64) -> f64 {
    if od.is_finite() {
        (od.clamp(MIN, MAX) * 10.0).round() / 10.0
    } else {
        DEFAULT
    }
}

/// From config.txt.
pub fn parse(s: &str) -> f64 {
    s.trim().parse().map_or(DEFAULT, clamped)
}

/// The OD played with, as bits (`f64::to_bits`). Set from the config, read when a play's built.
static CURRENT: AtomicU64 = AtomicU64::new(DEFAULT.to_bits());

pub fn set(od: f64) {
    CURRENT.store(clamped(od).to_bits(), Ordering::Relaxed);
}

pub fn get() -> f64 {
    f64::from_bits(CURRENT.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_stays_in_range_in_tenths() {
        assert_eq!(clamped(25.0), MAX);
        assert_eq!(clamped(-3.0), MIN);
        assert_eq!(clamped(7.349), 7.3);
        assert_eq!(clamped(f64::NAN), DEFAULT);
        assert_eq!(parse("13.5"), 13.5);
        assert_eq!(parse("99"), MAX);
        assert_eq!(parse("nonsense"), DEFAULT);
    }
}
