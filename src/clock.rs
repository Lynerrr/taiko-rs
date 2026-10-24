//! A continuous game clock slaved to the audio device.
//!
//! The audio callback position advances in steps (one step per buffer) and its timestamps
//! jitter by a fraction of a millisecond. Rendering notes straight from it produces visible
//! micro-stutter, so instead we free-run on the CPU's monotonic clock and steer gently toward
//! the audio position, snapping only on large errors (start, seek, underrun).

use std::time::Instant;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
