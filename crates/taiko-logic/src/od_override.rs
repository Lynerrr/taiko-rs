//! OD Override's setup: the OD a play uses, whatever the map says. It replaces the OD EZ, HR and
//! Precise would give; the speed still shifts it, as the windows are timed in the song.

pub const MIN: f64 = 0.0;
pub const MAX: f64 = 20.0;
pub const DEFAULT: f64 = 10.0;
/// What Left and Right change it by, and with Shift.
pub const STEP: f64 = 0.5;
pub const FINE_STEP: f64 = 0.1;

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
