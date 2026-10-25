//! A continuous game clock slaved to the audio device.
//!
//! The audio callback position advances in steps (one step per buffer) and its timestamps
//! jitter by a fraction of a millisecond. Rendering notes straight from it produces visible
//! micro-stutter, so instead we free-run on the CPU's monotonic clock and steer gently toward
//! the audio position, snapping only on large errors (start, seek, underrun).

use std::time::{Duration, Instant};

const SNAP_THRESHOLD_MS: f64 = 40.0;
/// How quickly drift is corrected; long enough that buffer jitter is invisible.
const CORRECTION_TIME_CONSTANT_MS: f64 = 150.0;

pub struct GameClock {
    time: f64,
    at: Instant,
    synced: bool,
    /// Hold still until the first audio position arrives instead of running ahead and snapping back.
    wait_for_audio: bool,
    /// The error of a reading too far off to steer toward, waiting for the next to confirm it. A
    /// lone bad timestamp (some Windows drivers report them) would otherwise throw the clock
    /// seconds ahead, and every note on the way would be missed before it came on screen.
    jump: Option<f64>,
}

impl GameClock {
    pub fn new(start_ms: f64, wait_for_audio: bool) -> Self {
        GameClock { time: start_ms, at: Instant::now(), synced: false, wait_for_audio, jump: None }
    }

    /// Advances to `now`, nudging toward the audio position if one is known.
    pub fn update(&mut self, now: Instant, audio_ms: Option<f64>) -> f64 {
        let dt = now.saturating_duration_since(self.at).as_secs_f64() * 1000.0;
        let predicted = self.time + dt;
        self.at = now;
        self.time = match audio_ms {
            Some(audio) => {
                let err = audio - predicted;
                let confirmed = self.jump.take().is_some_and(|e| (err - e).abs() <= SNAP_THRESHOLD_MS);
                if !self.synced || (err.abs() > SNAP_THRESHOLD_MS && confirmed) {
                    self.synced = true;
                    audio
                } else if err.abs() > SNAP_THRESHOLD_MS {
                    self.jump = Some(err);
                    predicted
                } else {
                    let k = 1.0 - (-dt / CORRECTION_TIME_CONSTANT_MS).exp();
                    // Never step backwards: a frame that shows notes moving the wrong way is worse
                    // than a few microseconds of extra drift.
                    (predicted + err * k).max(predicted - dt * 0.5)
                }
            }
            None if self.wait_for_audio && !self.synced => self.time,
            None => predicted,
        };
        self.time
    }

    /// Keeps the clock frozen (e.g. while paused): time stands still and `now` becomes the new
    /// reference so the pause duration is never counted.
    pub fn hold(&mut self, now: Instant) {
        self.at = now;
    }

    /// Clock reading at an arbitrary instant (e.g. when a key event arrived), without correction.
    pub fn at(&self, instant: Instant) -> f64 {
        let delta = if instant >= self.at {
            instant.duration_since(self.at).as_secs_f64()
        } else {
            -self.at.duration_since(instant).as_secs_f64()
        };
        self.time + delta * 1000.0
    }
}

/// With vsync, frames are shown exactly one refresh apart (or whole refreshes, when one came
/// too late), however unevenly the swapchain hands out images to draw them in: a little late or
/// early around each refresh, and right away after a slow frame (an image was free). Notes drawn
/// for the moment the image was handed out move by uneven steps on even refreshes: stutter.
/// This gives each frame a time on the refresh grid instead: one refresh after the last, whole
/// refreshes more when it really came late, following the real clock slowly.
pub struct VsyncPacer {
    /// The refresh interval (ms), learned from the screen's (59.94 Hz isn't 60).
    period: f64,
    /// The last frame's time and when it got its image.
    last: Option<(Instant, Instant)>,
}

impl Default for VsyncPacer {
    fn default() -> Self {
        VsyncPacer { period: 1000.0 / 60.0, last: None }
    }
}

impl VsyncPacer {
    /// The time to draw a frame for that got its image at `acquired`, on a screen of
    /// `refresh_hz`.
    pub fn frame_time(&mut self, acquired: Instant, refresh_hz: f64) -> Instant {
        let nominal = 1000.0 / refresh_hz.clamp(24.0, 1000.0);
        if (self.period - nominal).abs() > nominal * 0.1 {
            // Another screen or mode: start over.
            self.period = nominal;
            self.last = None;
        }
        let ms = |a: Instant, b: Instant| if a >= b { (a - b).as_secs_f64() * 1000.0 } else { -((b - a).as_secs_f64() * 1000.0) };
        let shift = |t: Instant, by: f64| if by >= 0.0 { t + Duration::from_secs_f64(by / 1000.0) } else { t - Duration::from_secs_f64(-by / 1000.0) };
        let Some((prev, prev_acquired)) = self.last else {
            self.last = Some((acquired, acquired));
            return acquired;
        };
        // Learn the refresh from images handed out about one refresh apart.
        let step = ms(acquired, prev_acquired);
        if (step - self.period).abs() < 0.15 * self.period {
            self.period += (step - self.period) * 0.002;
        }
        let p = self.period;
        let mut t = shift(prev, p);
        let mut err = ms(acquired, t);
        if err > 0.5 * p {
            // Came late: the frame before stayed on screen for whole refreshes more.
            let n = (err / p).round();
            if n > 8.0 {
                // A stall (or the window was hidden): pick up from now.
                self.last = Some((acquired, acquired));
                return acquired;
            }
            t = shift(t, n * p);
            err -= n * p;
        }
        if err.abs() < 0.5 * p {
            // Near the grid: drift toward the real clock, slowly enough that its jitter doesn't
            // show. (An image handed out right away, after a slow frame, is still shown a refresh
            // after the one before: no pull toward it.)
            t = shift(t, 0.05 * err);
        } else if err < -2.0 * p {
            // Far ahead of the real clock: lost track.
            t = acquired;
        }
        self.last = Some((t, acquired));
        t
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Frame times (ms from the first) for images handed out at `acquired` (ms).
    fn paced(acquired: &[f64]) -> Vec<f64> {
        let start = Instant::now();
        let mut p = VsyncPacer::default();
        acquired.iter().map(|&a| (p.frame_time(start + Duration::from_secs_f64(a / 1000.0), 60.0) - start).as_secs_f64() * 1000.0).collect()
    }

    #[test]
    fn frames_land_on_the_refresh_grid_however_unevenly_images_come() {
        let r = 1000.0 / 60.0;
        // Images handed out up to 3 ms either side of each refresh.
        let jitter = [0.0, 2.1, -1.4, 2.9, -2.6, 0.7, -0.3, 1.8, -2.9, 2.2];
        let acquired: Vec<f64> = (0..200).map(|k| k as f64 * r + jitter[k % jitter.len()]).collect();
        let t = paced(&acquired);
        for w in t.windows(2).skip(1) {
            assert!((w[1] - w[0] - r).abs() < 0.25, "step {}", w[1] - w[0]);
        }
        // And still with the real clock.
        assert!((t[199] - acquired[199]).abs() < 3.0);
    }

    #[test]
    fn a_slow_frame_costs_one_refresh_and_the_quick_one_after_it_none() {
        let r = 1000.0 / 60.0;
        // Frame 10 comes a refresh and a half late; the next image is free at once.
        let mut acquired: Vec<f64> = (0..40).map(|k| k as f64 * r).collect();
        acquired[10] += 1.5 * r;
        for a in &mut acquired[11..] {
            *a += r;
        }
        acquired[11] = acquired[10] + 1.0;
        let t = paced(&acquired);
        let steps: Vec<f64> = t.windows(2).map(|w| ((w[1] - w[0]) / r * 100.0).round() / 100.0).collect();
        // One step of two refreshes (the frame shown twice), all the rest one.
        assert_eq!(steps.iter().filter(|&&s| (s - 2.0).abs() < 0.05).count(), 1, "{steps:?}");
        assert!(steps.iter().all(|&s| (s - 1.0).abs() < 0.05 || (s - 2.0).abs() < 0.05), "{steps:?}");
    }

    #[test]
    fn it_learns_a_refresh_a_little_off_the_nominal_one() {
        let r = 1000.0 / 59.94;
        let acquired: Vec<f64> = (0..3000).map(|k| k as f64 * r).collect();
        let t = paced(&acquired);
        assert!((t[2999] - t[2998] - r).abs() < 0.01);
        assert!((t[2999] - acquired[2999]).abs() < 1.0);
    }

    #[test]
    fn a_lone_bad_reading_is_ignored_but_a_real_jump_is_taken() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut c = GameClock::new(0.0, true);
        c.at = start;
        assert_eq!(c.update(at(10), Some(10.0)), 10.0);
        // One reading a minute ahead (e.g. the previous song's position): not believed.
        assert!((c.update(at(20), Some(60_000.0)) - 20.0).abs() < 1.0);
        assert!((c.update(at(30), Some(30.0)) - 30.0).abs() < 1.0);
        // Two that agree (a seek, or the audio carried on through a stall): snapped to.
        assert!((c.update(at(40), Some(5_040.0)) - 40.0).abs() < 1.0);
        assert_eq!(c.update(at(50), Some(5_050.0)), 5_050.0);
    }
}
