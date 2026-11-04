//! taiko-rs patch: what an output stream's clock said, for the game's audio log (dev builds).
//! Only WASAPI fills it in.

use std::sync::Mutex;

#[cfg(windows)]
use crate::StreamInstant;

/// The stream's figures, its latest callback's, and what its callbacks saw since the last `take`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClockLog {
    pub clock_frequency: u64,
    pub max_frames: u32,
    pub period_frames: u32,
    /// `IAudioClient::GetStreamLatency`.
    pub stream_latency_us: u64,
    /// `IAudioClock::GetPosition`, in `clock_frequency` units.
    pub position: u64,
    pub frames_written: u64,
    /// Frames the engine hadn't read yet.
    pub padding: u32,
    pub callbacks: u32,
    /// Callbacks that found the buffer empty, the first aside: the engine had run dry.
    pub dry: u32,
    /// What was written and not yet played (what the game takes for the latency).
    pub buffered_min_us: u64,
    pub buffered_max_us: u64,
    /// How old `position` was by the time it was read.
    pub age_sum_us: u64,
    pub age_max_us: u64,
    /// The longest between two callbacks.
    pub gap_max_us: u64,
    #[cfg(windows)]
    last: Option<StreamInstant>,
}

static CLOCK: Mutex<Option<ClockLog>> = Mutex::new(None);

/// The latest figures, starting the counts over. `None` until a stream has reported any.
pub fn take() -> Option<ClockLog> {
    let mut clock = CLOCK.lock().ok()?;
    let c = clock.as_mut()?;
    let out = *c;
    *c = ClockLog { callbacks: 0, dry: 0, buffered_min_us: 0, buffered_max_us: 0, age_sum_us: 0, age_max_us: 0, gap_max_us: 0, ..out };
    Some(out)
}

#[cfg(windows)]
pub(crate) struct Reading {
    pub clock_frequency: u64,
    pub max_frames: u32,
    pub period_frames: u32,
    pub stream_latency: std::time::Duration,
    pub position: u64,
    pub frames_written: u64,
    pub padding: u32,
    pub buffered: std::time::Duration,
    pub age: std::time::Duration,
    pub now: StreamInstant,
}

/// Called from the audio thread: skips a reading rather than wait for the log.
#[cfg(windows)]
pub(crate) fn record(r: Reading) {
    let Ok(mut clock) = CLOCK.try_lock() else { return };
    let c = clock.get_or_insert_with(ClockLog::default);
    let (buffered, age) = (r.buffered.as_micros() as u64, r.age.as_micros() as u64);
    if c.callbacks == 0 {
        (c.buffered_min_us, c.buffered_max_us) = (buffered, buffered);
    }
    c.buffered_min_us = c.buffered_min_us.min(buffered);
    c.buffered_max_us = c.buffered_max_us.max(buffered);
    c.age_sum_us += age;
    c.age_max_us = c.age_max_us.max(age);
    if let Some(last) = c.last {
        c.gap_max_us = c.gap_max_us.max(r.now.duration_since(last).as_micros() as u64);
        c.dry += u32::from(r.padding == 0);
    }
    c.last = Some(r.now);
    c.callbacks += 1;
    c.clock_frequency = r.clock_frequency;
    c.max_frames = r.max_frames;
    c.period_frames = r.period_frames;
    c.stream_latency_us = r.stream_latency.as_micros() as u64;
    c.position = r.position;
    c.frames_written = r.frames_written;
    c.padding = r.padding;
}
