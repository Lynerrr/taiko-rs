//! OD Override's setup: the OD a play uses, whatever the map says. It replaces the OD EZ, HR and
//! Precise would give; the speed still shifts it, as the windows are timed in the song.

use std::sync::atomic::{AtomicU64, Ordering};

use winit::keyboard::KeyCode;

use crate::render::{Canvas, Hit, accent, rgba};

pub const MIN: f64 = 0.0;
pub const MAX: f64 = 20.0;
pub const DEFAULT: f64 = 10.0;
/// What Left and Right change it by, and with Shift.
const STEP: f64 = 0.5;
const FINE_STEP: f64 = 0.1;

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

/// The setup's open tab; the value itself is the config's.
pub struct Setup;

impl Setup {
    /// Handles a key. Returns the new OD, if it changed (save it).
    pub fn key(&self, code: KeyCode, shift: bool, od: f64) -> Option<f64> {
        let step = if shift { FINE_STEP } else { STEP };
        let new = match code {
            KeyCode::ArrowLeft => clamped(od - step),
            KeyCode::ArrowRight => clamped(od + step),
            KeyCode::KeyD => DEFAULT,
            _ => return None,
        };
        (new != od).then_some(new)
    }

    /// Draws the setup over the mod select, like Tact's.
    pub fn draw_panel(&self, c: &mut Canvas, od: f64) {
        const COLOUR: u32 = 0x4fb4ff;
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.6));
        let (px, py, pw, ph) = (16.0, 44.0, w - 32.0, 410.0);
        c.round_rect(
            [px, py],
            [pw, ph],
            10.0,
            rgba(0x15151d, 0.99),
            rgba(COLOUR, 0.9),
            1.5,
        );
        let title_w = c.text(
            px + 16.0,
            py + 12.0,
            12.0,
            rgba(0xffffff, 1.0),
            "OD OVERRIDE",
        );
        let hint =
            tr!("Left/Right change by 0.5   Shift by 0.1   D default   Tab next tab   Esc back");
        c.text(
            px + 26.0 + title_w,
            py + 16.0,
            6.0,
            rgba(0x6f6f86, 1.0),
            &Canvas::fit(6.0, pw - title_w - 42.0, hint),
        );

        let (x0, y, h) = (px + 16.0, py + 40.0, 24.0);
        let (bw, vw) = (24.0, 70.0);
        c.button(
            x0,
            y,
            bw,
            h,
            "-",
            9.0,
            Hit::Key(KeyCode::ArrowLeft),
            od > MIN,
        );
        c.round_rect(
            [x0 + bw + 6.0, y],
            [vw, h],
            4.0,
            rgba(0x34344a, 1.0),
            rgba(accent(), 1.0),
            1.5,
        );
        c.text_centered(
            x0 + bw + 6.0 + vw / 2.0,
            y + h / 2.0 - 4.5,
            9.0,
            rgba(0xffffff, 1.0),
            &format!("OD {od:.1}"),
        );
        c.button(
            x0 + bw + vw + 12.0,
            y,
            bw,
            h,
            "+",
            9.0,
            Hit::Key(KeyCode::ArrowRight),
            od < MAX,
        );
        let windows = crate::game::Windows::new(od);
        c.text(
            x0 + 2.0 * bw + vw + 24.0,
            y + h / 2.0 - 3.5,
            7.0,
            rgba(0xc8c8d8, 1.0),
            &format!(
                "300 ±{:.1} ms   100 ±{:.1} ms   miss ±{:.1} ms",
                windows.great, windows.ok, windows.miss
            ),
        );
        let about = tr!(
            "Plays every map at this OD, in place of the map's and of what EZ, HR and Precise would make it. The speed still shifts it, so song select shows more than this with DT. Past OD 13.3 the windows shrink ever more slowly instead of reaching zero. Plays aren't saved, and it stays off in multiplayer."
        );
        let mut ty = y + h + 18.0;
        let chars = ((pw - 32.0) / Canvas::char_width(7.0)).max(10.0) as usize;
        for line in crate::social::wrap(about, chars) {
            c.text(x0, ty, 7.0, rgba(0x9a9ab0, 1.0), &line);
            ty += 12.0;
        }
        let (by, back_w) = (py + ph - 26.0, 56.0);
        c.button(
            px + pw - back_w - 12.0,
            by,
            back_w,
            16.0,
            tr!("Back"),
            7.0,
            Hit::Key(KeyCode::Escape),
            true,
        );
    }
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

    #[test]
    fn keys_step_and_stop_at_the_ends() {
        let s = Setup;
        assert_eq!(s.key(KeyCode::ArrowRight, false, 10.0), Some(10.5));
        assert_eq!(s.key(KeyCode::ArrowLeft, true, 10.0), Some(9.9));
        assert_eq!(s.key(KeyCode::ArrowRight, false, MAX), None);
        assert_eq!(s.key(KeyCode::ArrowLeft, false, MIN), None);
        assert_eq!(s.key(KeyCode::KeyD, false, 3.0), Some(DEFAULT));
        assert_eq!(s.key(KeyCode::KeyD, false, DEFAULT), None);
        // Ten fine steps land on a whole number, not next to it.
        let od = (0..10).fold(10.0, |od, _| s.key(KeyCode::ArrowRight, true, od).unwrap());
        assert_eq!(od, 11.0);
    }
}
