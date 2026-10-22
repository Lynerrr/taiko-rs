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
}

impl GameClock {
    pub fn new(start_ms: f64, wait_for_audio: bool) -> Self {
        GameClock { time: start_ms, at: Instant::now(), synced: false, wait_for_audio }
    }

    /// Advances to `now`, nudging toward the audio position if one is known.
    pub fn update(&mut self, now: Instant, audio_ms: Option<f64>) -> f64 {
        let dt = now.saturating_duration_since(self.at).as_secs_f64() * 1000.0;
        let predicted = self.time + dt;
        self.at = now;
        self.time = match audio_ms {
            Some(audio) => {
                let err = audio - predicted;
                if !self.synced || err.abs() > SNAP_THRESHOLD_MS {
                    self.synced = true;
                    audio
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
