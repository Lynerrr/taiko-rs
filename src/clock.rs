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
///
/// The refresh it goes by is the one it sees, not the one it's told: the window system can name
/// another screen's (60 Hz while the window is on a 180 Hz one) or none at all (Wayland before
/// the window is shown: 60), and a grid of the wrong refresh makes notes leap and snap back.
pub struct VsyncPacer {
    /// The refresh interval (ms): the told one at first, then what the images show (and 59.94 Hz
    /// isn't 60).
    period: f64,
    /// The refresh it was told last (another one starts over).
    told_hz: f64,
    /// The last frame's time and when it got its image.
    last: Option<(Instant, Instant)>,
    /// The latest steps (ms) between images.
    steps: std::collections::VecDeque<f64>,
}

impl Default for VsyncPacer {
    fn default() -> Self {
        VsyncPacer { period: 1000.0 / 60.0, told_hz: 0.0, last: None, steps: Default::default() }
    }
}

/// How many steps between images it takes to judge the refresh, and how many it's judged by once
/// there are (more average the jitter out finer).
const PACE_STEPS: usize = 24;
const PACE_STEPS_KEPT: usize = 120;

impl VsyncPacer {
    /// The time to draw a frame for that got its image at `acquired`, on a screen the window
    /// system says is `refresh_hz`.
    pub fn frame_time(&mut self, acquired: Instant, refresh_hz: f64) -> Instant {
        if refresh_hz != self.told_hz {
            // Another screen or mode: start over.
            self.told_hz = refresh_hz;
            self.period = 1000.0 / refresh_hz.clamp(24.0, 1000.0);
            self.last = None;
            self.steps.clear();
        }
        let ms = |a: Instant, b: Instant| if a >= b { (a - b).as_secs_f64() * 1000.0 } else { -((b - a).as_secs_f64() * 1000.0) };
        let shift = |t: Instant, by: f64| if by >= 0.0 { t + Duration::from_secs_f64(by / 1000.0) } else { t - Duration::from_secs_f64(-by / 1000.0) };
        let Some((prev, prev_acquired)) = self.last else {
            self.last = Some((acquired, acquired));
            return acquired;
        };
        let step = ms(acquired, prev_acquired);
        if (0.5..250.0).contains(&step) {
            self.steps.push_back(step);
            if self.steps.len() > PACE_STEPS_KEPT {
                self.steps.pop_front();
            }
        }
        if let Some(seen) = self.seen_refresh() {
            if (seen - self.period).abs() > 0.15 * self.period {
                // The images come at another refresh than the one told: go by theirs, from
                // here.
                self.period = seen;
                self.last = Some((acquired, acquired));
                return acquired;
            }
            // And to the fraction of a millisecond (59.94 Hz isn't 60), gliding: what's seen
            // moves a little with each image's jitter.
            self.period += (seen - self.period) * 0.05;
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

    /// The refresh (ms) the images come at, once there are enough steps between them: the mean
    /// of those near their median (the median alone lands on one step's jitter; the mean of them
    /// all would count slow frames).
    fn seen_refresh(&self) -> Option<f64> {
        if self.steps.len() < PACE_STEPS {
            return None;
        }
        let mut sorted: Vec<f64> = self.steps.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        // A slow frame (two refreshes or more) and the quick one after it are well outside; any
        // jitter, however lopsided, inside.
        let near: Vec<f64> = sorted.into_iter().filter(|s| (s - median).abs() < 0.5 * median).collect();
        Some(near.iter().sum::<f64>() / near.len() as f64)
    }

    /// The refresh interval (ms) frames are paced to, once it's been seen (for the vsync
    /// compensation: the told one can be wrong).
    pub fn refresh_ms(&self) -> Option<f64> {
        self.seen_refresh().map(|_| self.period)
    }

    /// Out of vsync (a song in mailbox, menus in vsync): starts over at the next frame, still
    /// knowing the refresh it saw.
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

    /// Frame times (ms from the first) for images handed out at `acquired` (ms), told the screen
    /// is `told_hz`.
    fn paced_told(acquired: &[f64], told_hz: f64) -> (Vec<f64>, VsyncPacer) {
        let start = Instant::now();
        let mut p = VsyncPacer::default();
        let t = acquired.iter().map(|&a| (p.frame_time(start + Duration::from_secs_f64(a / 1000.0), told_hz) - start).as_secs_f64() * 1000.0).collect();
        (t, p)
    }

    #[test]
    fn a_screen_other_than_the_one_told_still_steps_evenly() {
        // The window on a 180 Hz screen, told 60 Hz (another screen's, or none read: 60), and
        // the other way round: it used to leap a 60 Hz refresh a frame and snap back.
        for (real, told) in [(180.0, 60.0), (60.0, 180.0), (144.0, 60.0), (240.0, 144.0)] {
            let r = 1000.0 / real;
            let jitter = [0.0, 0.4, -0.3, 0.6, -0.5, 0.2];
            let acquired: Vec<f64> = (0..600).map(|k| k as f64 * r + jitter[k % jitter.len()]).collect();
            let (t, p) = paced_told(&acquired, told);
            // Once it has seen a few dozen frames, every step is one of its refreshes.
            for (k, w) in t.windows(2).enumerate().skip(40) {
                assert!((w[1] - w[0] - r).abs() < 0.2, "{real} Hz told {told}: step {k} is {}", w[1] - w[0]);
            }
            assert!((t[599] - acquired[599]).abs() < 2.0, "{real} Hz told {told}: drifted off");
            assert!((p.refresh_ms().unwrap() - r).abs() < 0.05, "{real} Hz told {told}");
        }
    }

    #[test]
    fn a_game_drawing_every_other_refresh_still_steps_evenly() {
        // A 240 Hz screen, frames every second refresh: shown 2 refreshes apart, evenly.
        let r = 2.0 * 1000.0 / 240.0;
        let acquired: Vec<f64> = (0..400).map(|k| k as f64 * r + if k % 2 == 0 { 0.3 } else { -0.3 }).collect();
        let (t, _) = paced_told(&acquired, 240.0);
        for w in t.windows(2).skip(40) {
            assert!((w[1] - w[0] - r).abs() < 0.2, "step {}", w[1] - w[0]);
        }
    }

    #[test]
    fn the_refresh_it_saw_outlasts_a_spell_out_of_vsync() {
        let r = 1000.0 / 180.0;
        let start = Instant::now();
        let at = |ms: f64| start + Duration::from_secs_f64(ms / 1000.0);
        let mut p = VsyncPacer::default();
        for k in 0..60 {
            p.frame_time(at(k as f64 * r), 60.0);
        }
        // A song in mailbox, then vsync again: straight back on the 180 Hz grid.
        p.reset();
        let t0 = 10_000.0;
        let t: Vec<f64> = (0..10).map(|k| (p.frame_time(at(t0 + k as f64 * r), 60.0) - start).as_secs_f64() * 1000.0).collect();
        assert!(t.windows(2).all(|w| (w[1] - w[0] - r).abs() < 0.2), "{t:?}");
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
