//! A running log of what the audio clock says, to find out why a player's offset moves over a
//! session: dev builds on Windows (or TAIKO_AUDIO_LOG=1 on any build), in audio-log.txt in the
//! config folder. One line every 2 s, and one for each play, offset change and calibration.
//!
//! TAIKO_AUDIO_STALL=N also holds up the audio callback for 60 ms every N seconds: it makes the
//! engine run dry on purpose, to see what the device position does through it.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const EVERY_SECS: u64 = 2;
/// A log past this starts over, the last one kept as audio-log.old.txt.
const MAX_BYTES: u64 = 8 << 20;
const STALL_MS: u32 = 60;

static LOG: Mutex<Option<(std::fs::File, Instant)>> = Mutex::new(None);
/// A hold-up for the audio callback to sit through (ms).
pub static STALL: AtomicU32 = AtomicU32::new(0);

pub fn enabled() -> bool {
    std::env::var_os("TAIKO_AUDIO_LOG").is_some() || (cfg!(windows) && crate::update::BUILD_CHANNEL == crate::update::Channel::Dev)
}

/// Opens the log if this build keeps one. True if it's open.
pub fn start() -> bool {
    if !enabled() {
        return false;
    }
    let Ok(mut log) = LOG.lock() else { return false };
    if log.is_some() {
        return true;
    }
    let Some(dir) = crate::config::data_dir() else { return false };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("audio-log.txt");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, dir.join("audio-log.old.txt"));
    }
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return false };
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = writeln!(file, "=== taiko.rs {} build {} channel {} started {unix}", env!("CARGO_PKG_VERSION"), crate::update::BUILD.unwrap_or("local"), crate::update::BUILD_CHANNEL.name());
    *log = Some((file, Instant::now()));
    true
}

/// A line, stamped with the seconds since the game started. Nothing if the log isn't open.
pub fn line(text: &str) {
    if let Ok(mut log) = LOG.lock()
        && let Some((file, start)) = log.as_mut()
    {
        let _ = writeln!(file, "{:.3} {text}", start.elapsed().as_secs_f64());
    }
}

/// What cpal's clock said since the last time, on the line every 2 s.
fn clock_fields(c: &cpal::taiko_clock::ClockLog, rate: u32) -> String {
    let ms = |us: u64| us as f64 / 1000.0;
    format!(
        " cb={} dry={} written_ms={:.3} pos_ms={:.3} pad={} buf_ms={:.3}..{:.3} age_ms={:.3}/{:.3} gap_ms={:.3}",
        c.callbacks,
        c.dry,
        c.frames_written as f64 * 1000.0 / rate as f64,
        c.position as f64 * 1000.0 / c.clock_frequency.max(1) as f64,
        c.padding,
        ms(c.buffered_min_us),
        ms(c.buffered_max_us),
        ms(c.age_sum_us) / c.callbacks.max(1) as f64,
        ms(c.age_max_us),
        ms(c.gap_max_us),
    )
}

/// Every 2 s, logs what `report` says with cpal's clock, until `report` says None.
pub fn every(rate: u32, mut report: impl FnMut() -> Option<String> + Send + 'static) {
    let stall_every = std::env::var("TAIKO_AUDIO_STALL").ok().and_then(|s| s.parse::<u64>().ok()).filter(|&s| s > 0);
    std::thread::spawn(move || {
        let mut ticks = 0;
        let mut told = false;
        loop {
            std::thread::sleep(Duration::from_secs(EVERY_SECS));
            let Some(mut text) = report() else { return };
            if let Some(c) = cpal::taiko_clock::take() {
                if !told {
                    told = true;
                    line(&format!("stream max_frames={} period_frames={} clock_hz={} stream_latency_ms={:.3}", c.max_frames, c.period_frames, c.clock_frequency, c.stream_latency_us as f64 / 1000.0));
                }
                text.push_str(&clock_fields(&c, rate));
            }
            line(&text);
            ticks += 1;
            if stall_every.is_some_and(|s| ticks * EVERY_SECS >= s && (ticks * EVERY_SECS) % s < EVERY_SECS) {
                STALL.store(STALL_MS, Ordering::Relaxed);
                line(&format!("stall {STALL_MS}"));
            }
        }
    });
}
