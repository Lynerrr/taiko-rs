//! Audio output. The song is fully decoded into memory and mixed by hand inside the
//! device callback, so the callback itself is the master clock: every callback publishes
//! "song frame N starts playing at instant T" and the game interpolates from that.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SizedSample, StreamConfig};

use crate::game;

/// Interleaved stereo samples at the device sample rate.
pub struct Track {
    pub samples: Vec<f32>,
    /// Playback rate the samples were prepared for (1.0 = original speed). One second of this
    /// track covers `rate` seconds of the beatmap.
    pub rate: f64,
    /// A long song isn't in `samples`: it's played as it's decoded.
    pub stream: Option<Arc<crate::stream::Stream>>,
}

impl Track {
    pub fn frames(&self) -> usize {
        match &self.stream {
            Some(s) => s.frames(),
            None => self.samples.len() / 2,
        }
    }
}

/// How many times one sound rings at once; a new play fades the oldest out. osu! does the same
/// (osu-framework's `Sample.PlaybackConcurrency`), so mashing a finisher stays one loud hit
/// rather than copies piling up into clipping.
pub const SFX_CONCURRENCY: usize = 2;
/// How long (frames) a sound cut off that way takes to fade, so the cut doesn't click.
pub const CHOKE_FRAMES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sfx {
    Don,
    Kat,
    BigDon,
    BigKat,
    /// A combo of 20 or more broken by a miss.
    ComboBreak,
    /// Overtime's jumpscare.
    Scream,
    /// A friend's message.
    Message,
    /// A medal landing, by tier.
    MedalKantan,
    MedalFutsuu,
    MedalMuzukashii,
    MedalOni,
    /// A secret medal.
    MedalUra,
}

impl Sfx {
    /// The sound of a medal: `tier` is 1 to 4.
    pub fn medal(tier: u8, secret: bool) -> Sfx {
        match (secret, tier) {
            (true, _) => Sfx::MedalUra,
            (_, 1) => Sfx::MedalKantan,
            (_, 2) => Sfx::MedalFutsuu,
            (_, 3) => Sfx::MedalMuzukashii,
            _ => Sfx::MedalOni,
        }
    }
}

impl From<crate::game::Sound> for Sfx {
    fn from(s: crate::game::Sound) -> Sfx {
        use crate::game::Sound;
        match s {
            Sound::Don => Sfx::Don,
            Sound::Kat => Sfx::Kat,
            Sound::BigDon => Sfx::BigDon,
            Sound::BigKat => Sfx::BigKat,
            Sound::ComboBreak => Sfx::ComboBreak,
            Sound::Scream => Sfx::Scream,
        }
    }
}

/// One sound per `Sfx`.
pub type Sounds = [Arc<[f32]>; 12];

enum Cmd {
    /// `base`: the song frame the track's first sample belongs at (a partial decode).
    Play {
        track: Arc<Track>,
        start_frame: i64,
        base: i64,
    },
    /// Replaces the playing track without moving the play position.
    Swap {
        track: Arc<Track>,
        base: i64,
    },
    Seek(i64),
    /// Sent after each Play / Seek: the stamp describes the new position from here on.
    Epoch(u64),
    Pause(bool),
    Stop,
    /// A sound and its volume (0..1, the map's timing point volume for hits).
    Sfx(Sfx, f32),
    /// A sound to start when the stream gets to this song frame.
    SfxAt(i64, Sfx, f32),
    /// Replaces the hitsounds (indexed by `Sfx`).
    Sounds(Arc<Sounds>),
}

/// What the callback replaced, sent back so deallocation stays off the audio thread.
#[allow(dead_code)] // only held to be dropped
enum Garbage {
    Track(Arc<Track>),
    Sounds(Arc<Sounds>),
}

/// What the callback last told us about playback position.
#[derive(Clone, Copy)]
struct Stamp {
    playing: bool,
    /// Song frame written at the start of the last callback.
    frame: i64,
    /// When that callback ran.
    at: Instant,
    /// Delay from callback to the samples reaching the speakers.
    latency: Duration,
    /// That delay as the stream reported it (`SteadyLatency`).
    reported: Duration,
    /// `latency` is past the stream's start-up and won't change any more.
    settled: bool,
    /// The last Play / Seek the callback had applied.
    epoch: u64,
}

pub struct Engine {
    _stream: cpal::Stream,
    tx: Sender<Cmd>,
    garbage: Receiver<Garbage>,
    stamp: Arc<Mutex<Stamp>>,
    /// The last Play / Seek sent.
    epoch: AtomicU64,
    volume: Arc<[AtomicU32; 3]>,
    pub rate: u32,
    pub description: String,
}

impl Engine {
    pub fn new(buffer_frames: u32) -> Result<Engine, String> {
        let host = pick_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let mut config: StreamConfig = supported.config();
        if let cpal::SupportedBufferSize::Range { min, max } = supported.buffer_size() {
            config.buffer_size = cpal::BufferSize::Fixed(buffer_frames.clamp(*min, *max));
        }

        let stamp = Arc::new(Mutex::new(Stamp {
            playing: false,
            frame: 0,
            at: Instant::now(),
            latency: Duration::ZERO,
            reported: Duration::ZERO,
            settled: false,
            epoch: 0,
        }));
        let volume = Arc::new([
            AtomicU32::new(0.8f32.to_bits()),
            AtomicU32::new(1.0f32.to_bits()),
            AtomicU32::new(1.0f32.to_bits()),
        ]);
        let fmt = supported.sample_format();
        let open = |config: StreamConfig| -> Result<(cpal::Stream, Sender<Cmd>, Receiver<Garbage>), String> {
            let (tx, rx) = mpsc::channel();
            let (gc_tx, garbage) = mpsc::channel();
            let mixer = Mixer::new(rx, gc_tx, stamp.clone(), volume.clone(), rate, channels);
            let stream = match fmt {
                cpal::SampleFormat::F32 => build::<f32>(&device, config, mixer),
                cpal::SampleFormat::I16 => build::<i16>(&device, config, mixer),
                cpal::SampleFormat::I32 => build::<i32>(&device, config, mixer),
                cpal::SampleFormat::U16 => build::<u16>(&device, config, mixer),
                other => Err(format!("unsupported sample format {other}")),
            }?;
            Ok((stream, tx, garbage))
        };
        let (stream, tx, garbage) = match open(config) {
            Ok(v) => v,
            Err(e) if config.buffer_size != cpal::BufferSize::Default => {
                eprintln!("fixed audio buffer rejected ({e}), using device default");
                config.buffer_size = cpal::BufferSize::Default;
                open(config)?
            }
            Err(e) => return Err(e),
        };
        stream.play().map_err(|e| e.to_string())?;

        let description = format!(
            "{:?} {} Hz, {} ch, buffer {:?}",
            host.id(),
            rate,
            channels,
            config.buffer_size
        );
        if crate::audiolog::start() {
            let name = device
                .description()
                .map(|d| d.name().to_string())
                .unwrap_or_default();
            crate::audiolog::line(&format!("audio {description} device={name}"));
            let stamp = Arc::downgrade(&stamp);
            crate::audiolog::every(rate, move || {
                let s = *stamp.upgrade()?.lock().ok()?;
                Some(format!(
                    "clock playing={} frame={} latency_ms={:.3}",
                    u8::from(s.playing),
                    s.frame,
                    s.latency.as_secs_f64() * 1000.0
                ))
            });
        }
        Ok(Engine {
            _stream: stream,
            tx,
            garbage,
            stamp,
            epoch: AtomicU64::new(0),
            volume,
            rate,
            description,
        })
    }

    pub fn play(&self, track: Arc<Track>, start_ms: f64) {
        self.play_part(track, 0, start_ms);
    }

    /// Plays part of a song: `base` is the song frame the track's first sample belongs at.
    pub fn play_part(&self, track: Arc<Track>, base: i64, start_ms: f64) {
        let start_frame = (start_ms / 1000.0 * self.rate as f64).round() as i64;
        self.jump(Cmd::Play {
            track,
            start_frame,
            base,
        });
    }

    /// Sends a command that moves the play position. Until the callback has applied it, the stamp
    /// still describes the old position (the previous song, or where a retried play had got to),
    /// so `position_ms` reports nothing rather than that.
    fn jump(&self, cmd: Cmd) {
        let epoch = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send(cmd);
        let _ = self.tx.send(Cmd::Epoch(epoch));
    }

    /// Swaps in another decode of the playing song (the full one for a part), seamlessly.
    pub fn swap(&self, track: Arc<Track>, base: i64) {
        let _ = self.tx.send(Cmd::Swap { track, base });
    }

    pub fn seek(&self, ms: f64) {
        self.jump(Cmd::Seek((ms / 1000.0 * self.rate as f64).round() as i64));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }

    /// Freezes the song where it is (output goes silent; hitsounds still play).
    pub fn set_paused(&self, paused: bool) {
        let _ = self.tx.send(Cmd::Pause(paused));
    }

    pub fn sfx(&self, s: Sfx) {
        self.sfx_at(s, 1.0);
    }

    /// Plays a sound at `volume` times the hitsound volume (1 = as recorded, more boosts it).
    pub fn sfx_at(&self, s: Sfx, volume: f32) {
        let _ = self.tx.send(Cmd::Sfx(s, volume));
    }

    /// Plays a sound when the song reaches `song_ms` (the units of `position_ms`), so that it is
    /// heard with the music at that moment, whatever the output latency. A sound whose moment has
    /// passed plays at once. Dropped by a seek, pause, restart or stop.
    pub fn sfx_at_song(&self, s: Sfx, volume: f32, song_ms: f64) {
        let frame = (song_ms / 1000.0 * self.rate as f64).round() as i64;
        let _ = self.tx.send(Cmd::SfxAt(frame, s, volume));
    }

    /// Hands over prepared mono samples at the device rate without rebuilding them here.
    pub fn set_hitsounds(&self, sounds: Arc<Sounds>) {
        let _ = self.tx.send(Cmd::Sounds(sounds));
    }

    pub fn set_volume(&self, music: f32, effects: f32) {
        self.volume[0].store(music.to_bits(), Ordering::Relaxed);
        self.volume[1].store(effects.to_bits(), Ordering::Relaxed);
    }

    /// The share of both volumes heard while the window has the focus or not (1 = all). The
    /// mixer fades to it: down over `FOCUS_FADE_OUT`, back up over `FOCUS_FADE_IN`.
    pub fn set_focus(&self, level: f32) {
        self.volume[2].store(level.to_bits(), Ordering::Relaxed);
    }

    /// The output latency (ms) the clock goes by, and what the stream reports.
    pub fn latencies_ms(&self) -> (f64, f64) {
        let s = *self.stamp.lock().unwrap();
        (
            s.latency.as_secs_f64() * 1000.0,
            s.reported.as_secs_f64() * 1000.0,
        )
    }

    /// Whether `latencies_ms().0` is done changing for this stream (the first second or so of a
    /// stream it moves about while it starts up).
    pub fn latency_settled(&self) -> bool {
        self.stamp.lock().unwrap().settled
    }

    /// Song position (ms) audible at `now`, or None when nothing is playing yet.
    pub fn position_ms(&self, now: Instant) -> Option<f64> {
        // Free tracks the callback replaced; deallocation stays off the audio thread.
        while self.garbage.try_recv().is_ok() {}
        let s = *self.stamp.lock().unwrap();
        if !s.playing || s.epoch != self.epoch.load(Ordering::Relaxed) {
            return None;
        }
        let since = now.saturating_duration_since(s.at).as_secs_f64() - s.latency.as_secs_f64();
        Some((s.frame as f64 / self.rate as f64 + since) * 1000.0)
    }
}

fn pick_host() -> cpal::Host {
    // cpal only has a PipeWire host on Linux; elsewhere (WASAPI on Windows) the default host is right.
    #[cfg(all(feature = "pipewire", target_os = "linux"))]
    if std::env::var_os("TAIKO_ALSA").is_none()
        && let Ok(host) = cpal::host_from_id(cpal::HostId::PipeWire)
        && host.default_output_device().is_some()
    {
        return host;
    }
    cpal::default_host()
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut mixer: Mixer,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info: &OutputCallbackInfo| mixer.render(data, info),
            |e| eprintln!("audio stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// A sound waiting for its song frame.
struct Pending {
    frame: i64,
    sound: Sfx,
    volume: f32,
}

/// Room for sounds waiting on the audio thread (no allocating there).
const PENDING_CAPACITY: usize = 64;

struct Voice {
    sound: Sfx,
    pos: usize,
    volume: f32,
    /// Frames left of its fade out once cut off (`SFX_CONCURRENCY`); 0 while it plays on.
    fade: usize,
}

/// Starts `sound`, fading out its oldest play if it already rings `SFX_CONCURRENCY` times.
fn start_voice(voices: &mut Vec<Voice>, sound: Sfx, volume: f32) {
    let ringing = voices
        .iter()
        .filter(|v| v.sound == sound && v.fade == 0)
        .count();
    if let Some(oldest) = voices
        .iter_mut()
        .filter(|v| v.sound == sound && v.fade == 0)
        .max_by_key(|v| v.pos)
        .filter(|_| ringing >= SFX_CONCURRENCY)
    {
        oldest.fade = CHOKE_FRAMES;
    }
    if voices.len() < voices.capacity() {
        voices.push(Voice {
            sound,
            pos: 0,
            volume,
            fade: 0,
        });
    }
}

/// Callbacks left to a stream's start (about 0.6 s of WASAPI's), then to judge its latency by.
const LATENCY_SETTLE: u32 = 64;
const LATENCY_SAMPLES: usize = 64;

/// The latency of a stream from what it held once settled. WASAPI's device position can't be
/// relied on over a session: on a player's Realtek it slipped by whole engine periods every few
/// minutes (multiples of 2), 130 ms over 78 minutes, while the sound itself came no later. They
/// went on hitting the same spot in the music and the clock moved away from it, even with the
/// game sat in a menu, down to 26% accuracy. What a stream holds doesn't grow: its buffer is
/// always topped up, so only the frames it asks for say how late a callback came.
struct SteadyLatency {
    callbacks: u32,
    seen: Vec<f64>,
    /// Seconds of latency plus frames asked for: the same in every callback.
    total: Option<f64>,
}

impl SteadyLatency {
    fn new() -> SteadyLatency {
        SteadyLatency {
            callbacks: 0,
            seen: Vec::with_capacity(LATENCY_SAMPLES),
            total: None,
        }
    }

    /// The latency of a callback asked for `frames`, `reported` being what the stream says.
    fn of(&mut self, reported: Duration, frames: usize, rate: u32) -> Duration {
        let asked = frames as f64 / rate as f64;
        if let Some(total) = self.total {
            return Duration::from_secs_f64((total - asked).max(0.0));
        }
        self.callbacks += 1;
        if self.callbacks > LATENCY_SETTLE {
            self.seen.push(reported.as_secs_f64() + asked);
            if self.seen.len() == LATENCY_SAMPLES {
                self.seen.sort_by(f64::total_cmp);
                self.total = Some(self.seen[LATENCY_SAMPLES / 2]);
            }
        }
        reported
    }
}

struct Mixer {
    rx: Receiver<Cmd>,
    gc: Sender<Garbage>,
    stamp: Arc<Mutex<Stamp>>,
    volume: Arc<[AtomicU32; 3]>,
    channels: usize,
    rate: u32,
    steady: SteadyLatency,
    track: Option<Arc<Track>>,
    /// Song frame of the track's first sample.
    base: i64,
    frame: i64,
    epoch: u64,
    paused: bool,
    /// A stream's next frames, read once a callback.
    streamed: Vec<f32>,
    voices: Vec<Voice>,
    /// Sounds for song frames not written yet.
    pending: Vec<Pending>,
    sounds: Arc<Sounds>,
    limiter: Limiter,
    /// The focus's share of the volume as heard now, on its way to `volume[2]`.
    focus: f32,
}

/// How long the volume takes to fall all the way when the window loses the focus: slow, so
/// a glance at another window doesn't cut the music.
const FOCUS_FADE_OUT: f32 = 1.5;
/// And to come back with the focus: quick, the player is back.
const FOCUS_FADE_IN: f32 = 0.25;

/// The focus's share of the volume `secs` later, from `at` towards `to`.
fn focus_step(at: f32, to: f32, secs: f32) -> f32 {
    if to < at {
        (at - secs / FOCUS_FADE_OUT).max(to)
    } else {
        (at + secs / FOCUS_FADE_IN).min(to)
    }
}

impl Mixer {
    fn new(
        rx: Receiver<Cmd>,
        gc: Sender<Garbage>,
        stamp: Arc<Mutex<Stamp>>,
        volume: Arc<[AtomicU32; 3]>,
        rate: u32,
        channels: usize,
    ) -> Mixer {
        let sounds = Arc::new(hitsound_set(rate, Custom::default()));
        Mixer {
            rx,
            gc,
            stamp,
            volume,
            channels,
            rate,
            steady: SteadyLatency::new(),
            track: None,
            base: 0,
            frame: 0,
            epoch: 0,
            paused: false,
            streamed: vec![0.0; 1 << 16],
            voices: Vec::with_capacity(64),
            pending: Vec::with_capacity(PENDING_CAPACITY),
            sounds,
            limiter: Limiter::new(rate),
            focus: 1.0,
        }
    }

    fn apply(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play {
                track,
                start_frame,
                base,
            } => {
                if let Some(old) = self.track.replace(track) {
                    let _ = self.gc.send(Garbage::Track(old));
                }
                self.base = base;
                self.frame = start_frame;
                self.paused = false;
                self.pending.clear();
            }
            Cmd::Swap { track, base } => {
                if let Some(old) = self.track.replace(track) {
                    let _ = self.gc.send(Garbage::Track(old));
                }
                self.base = base;
            }
            Cmd::Seek(f) => {
                self.frame = f;
                self.pending.clear();
            }
            Cmd::Epoch(e) => self.epoch = e,
            Cmd::Pause(p) => {
                self.paused = p;
                if p {
                    self.pending.clear();
                }
            }
            Cmd::Stop => {
                self.pending.clear();
                if let Some(old) = self.track.take() {
                    let _ = self.gc.send(Garbage::Track(old));
                }
            }
            Cmd::Sounds(sounds) => {
                let _ = self
                    .gc
                    .send(Garbage::Sounds(std::mem::replace(&mut self.sounds, sounds)));
            }
            Cmd::Sfx(sound, volume) => start_voice(&mut self.voices, sound, volume),
            Cmd::SfxAt(frame, sound, volume) => {
                // Nothing moving the song on (or no room to wait): now is the best there is.
                if self.track.is_none()
                    || self.paused
                    || frame <= self.frame
                    || self.pending.len() == self.pending.capacity()
                {
                    start_voice(&mut self.voices, sound, volume);
                } else {
                    self.pending.push(Pending {
                        frame,
                        sound,
                        volume,
                    });
                }
            }
        }
    }

    /// Starts the waiting sounds whose song frame the stream is at.
    fn start_due(&mut self) {
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].frame <= self.frame {
                let p = self.pending.swap_remove(i);
                start_voice(&mut self.voices, p.sound, p.volume);
            } else {
                i += 1;
            }
        }
    }

    fn render<T: SizedSample + FromSample<f32>>(
        &mut self,
        out: &mut [T],
        info: &OutputCallbackInfo,
    ) {
        let now = Instant::now();
        {
            while let Ok(cmd) = self.rx.try_recv() {
                self.apply(cmd);
            }
        }

        let ts = info.timestamp();
        let reported = ts.playback.duration_since(ts.callback);
        let latency = if cfg!(windows) {
            self.steady
                .of(reported, out.len() / self.channels, self.rate)
        } else {
            self.steady.callbacks = self.steady.callbacks.saturating_add(1);
            reported
        };
        let settled = if cfg!(windows) {
            self.steady.total.is_some()
        } else {
            self.steady.callbacks > LATENCY_SETTLE
        };
        if let Ok(mut s) = self.stamp.try_lock() {
            *s = Stamp {
                playing: self.track.is_some() && !self.paused,
                frame: self.frame,
                at: now,
                latency,
                reported,
                settled,
                epoch: self.epoch,
            };
        }

        let ch = self.channels;
        let focus_to = f32::from_bits(self.volume[2].load(Ordering::Relaxed));
        let secs = (out.len() / ch.max(1)) as f32 / self.rate.max(1) as f32;
        self.focus = focus_step(self.focus, focus_to, secs);
        let music_vol = f32::from_bits(self.volume[0].load(Ordering::Relaxed)) * self.focus;
        let fx_vol = f32::from_bits(self.volume[1].load(Ordering::Relaxed)) * self.focus;
        // A stream's frames for this callback. Where they run out inside the song, the position
        // holds, so that the music comes back still on the notes.
        let stream = self.track.as_ref().and_then(|t| t.stream.as_ref());
        let mut streamed = 0;
        if let Some(s) = stream {
            let at = self.frame - self.base;
            s.want(at);
            if !self.paused {
                let n = (out.len() / ch.max(1) * 2).min(self.streamed.len());
                streamed = s.read(at.max(0), &mut self.streamed[..n]);
                if at >= 0 && streamed == 0 && (at as usize) < s.frames() {
                    s.stalls.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        let mut read = 0;
        for frame in out.chunks_mut(ch) {
            if !self.pending.is_empty() {
                self.start_due();
            }
            let (mut l, mut r) = (0.0f32, 0.0f32);
            if let Some(track) = self.track.as_ref().filter(|_| !self.paused) {
                let at = self.frame - self.base;
                if let Some(s) = &track.stream {
                    let in_song = at >= 0 && (at as usize) < s.frames();
                    if read < streamed && in_song {
                        l = self.streamed[read * 2] * music_vol;
                        r = self.streamed[read * 2 + 1] * music_vol;
                        read += 1;
                        self.frame += 1;
                    } else if !in_song {
                        self.frame += 1;
                    }
                } else {
                    if at >= 0 && (at as usize) < track.frames() {
                        let i = at as usize * 2;
                        l = track.samples[i] * music_vol;
                        r = track.samples[i + 1] * music_vol;
                    }
                    self.frame += 1;
                }
            }
            for v in &mut self.voices {
                let buf = &self.sounds[v.sound as usize];
                let fade = if v.fade == 0 {
                    1.0
                } else {
                    v.fade as f32 / CHOKE_FRAMES as f32
                };
                if let Some(s) = buf.get(v.pos) {
                    l += s * fx_vol * v.volume * fade;
                    r += s * fx_vol * v.volume * fade;
                }
                v.pos += 1;
                if v.fade > 0 {
                    v.fade -= 1;
                    if v.fade == 0 {
                        v.pos = buf.len();
                    }
                }
            }
            let (l, r) = self.limiter.apply(l, r);
            match ch {
                1 => frame[0] = T::from_sample((l + r) * 0.5),
                _ => {
                    frame[0] = T::from_sample(l);
                    frame[1] = T::from_sample(r);
                    for s in &mut frame[2..] {
                        *s = T::from_sample(0.0);
                    }
                }
            }
        }
        let sounds = &self.sounds;
        self.voices
            .retain(|v| v.pos < sounds[v.sound as usize].len());
        if crate::audiolog::STALL.load(Ordering::Relaxed) != 0 {
            std::thread::sleep(Duration::from_millis(
                crate::audiolog::STALL.swap(0, Ordering::Relaxed) as u64,
            ));
        }
    }
}

/// The mix's ceiling. A hit landing on a loud chorus turns the whole mix down at once and lets it
/// back up over ~150 ms, instead of bending the waveform past 0.9 (a soft clip did that here,
/// and it's what made finishers, and every hit in a kiai, sound distorted).
pub struct Limiter {
    gain: f32,
    release: f32,
}

impl Limiter {
    pub const CEILING: f32 = 0.95;

    pub fn new(rate: u32) -> Limiter {
        Limiter {
            gain: 1.0,
            release: 1.0 - (-1.0 / (0.15 * rate as f32)).exp(),
        }
    }

    pub fn apply(&mut self, l: f32, r: f32) -> (f32, f32) {
        let peak = l.abs().max(r.abs());
        let target = if peak > Self::CEILING {
            Self::CEILING / peak
        } else {
            1.0
        };
        self.gain = if target < self.gain {
            target
        } else {
            self.gain + (target - self.gain) * self.release
        };
        (l * self.gain, r * self.gain)
    }
}

impl crate::game::Song for Track {
    fn frames(&self) -> usize {
        Track::frames(self)
    }

    fn rate(&self) -> f64 {
        self.rate
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

impl crate::game::Audio for Engine {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn play(&self, song: Arc<dyn crate::game::Song>, start_ms: f64) {
        // The client's plays keep time to its own tracks.
        if let Ok(track) = song.into_any().downcast::<Track>() {
            Engine::play(self, track, start_ms);
        }
    }

    fn seek(&self, ms: f64) {
        Engine::seek(self, ms);
    }

    fn set_paused(&self, paused: bool) {
        Engine::set_paused(self, paused);
    }

    fn sfx_at(&self, s: crate::game::Sound, volume: f32) {
        Engine::sfx_at(self, s.into(), volume);
    }

    fn sfx_at_song(&self, s: crate::game::Sound, volume: f32, song_ms: f64) {
        Engine::sfx_at_song(self, s.into(), volume, song_ms);
    }

    fn latencies_ms(&self) -> (f64, f64) {
        Engine::latencies_ms(self)
    }

    fn position_ms(&self, now: Instant) -> Option<f64> {
        Engine::position_ms(self, now)
    }
}

/// The sound, as plays take it (borrowing only the `audio` field, so a play in the scene can be
/// borrowed beside it).
pub(crate) fn sound(audio: &Option<Engine>) -> Option<&dyn game::Audio> {
    audio.as_ref().map(|a| a as &dyn game::Audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_is_a_song_that_comes_back_as_itself() {
        use crate::game::Song;
        let track = Arc::new(Track {
            samples: vec![0.0; 2 * 4800],
            rate: 1.5,
            stream: None,
        });
        let song: Arc<dyn Song> = track.clone();
        assert_eq!(song.frames(), 4800);
        assert_eq!(song.rate(), 1.5);
        let back = song.into_any().downcast::<Track>().expect("the same track");
        assert!(Arc::ptr_eq(&back, &track));
    }

    #[test]
    fn each_sound_of_a_play_is_the_hitsound_of_its_name() {
        use crate::game::Sound;
        for s in [
            Sound::Don,
            Sound::Kat,
            Sound::BigDon,
            Sound::BigKat,
            Sound::ComboBreak,
            Sound::Scream,
        ] {
            assert_eq!(format!("{:?}", Sfx::from(s)), format!("{s:?}"));
        }
    }

    #[test]
    fn cached_sounds_share_samples_but_custom_hits_keep_their_mix() {
        let a = hitsound_set(48_000, Custom::default());
        let b = hitsound_set(48_000, Custom::default());
        assert!(a.iter().zip(&b).all(|(a, b)| Arc::ptr_eq(a, b)));
        let other = hitsound_set(44_100, Custom::default());
        assert_ne!(a[Sfx::Don as usize].len(), other[Sfx::Don as usize].len());
        let custom = hitsound_set(
            48_000,
            Custom {
                don: Some(vec![0.2, 0.3]),
                finish: Some(vec![0.1]),
                ..Default::default()
            },
        );
        assert_eq!(&*custom[Sfx::Don as usize], &[0.2, 0.3]);
        assert_eq!(&*custom[Sfx::BigDon as usize], mix(&[0.2, 0.3], &[0.1]));
        assert_eq!(
            &*custom[Sfx::BigKat as usize],
            mix(&a[Sfx::Kat as usize], &[0.1])
        );
        assert!(Arc::ptr_eq(
            &custom[Sfx::Kat as usize],
            &a[Sfx::Kat as usize]
        ));
        assert!(Arc::ptr_eq(
            &custom[Sfx::ComboBreak as usize],
            &a[Sfx::ComboBreak as usize]
        ));
        assert!(Arc::ptr_eq(
            &hitsound_set(48_000, Custom::default())[0],
            &a[0]
        ));
    }

    #[test]
    fn a_loud_mix_is_turned_down_whole_not_bent() {
        // A finisher (peak 1.56) on a chorus at full scale: 2.5 at the top of a sine.
        let mut lim = super::Limiter::new(48000);
        let wave: Vec<f32> = (0..9600).map(|i| 2.5 * (i as f32 * 0.0576).sin()).collect();
        let out: Vec<f32> = wave.iter().map(|&x| lim.apply(x, x).0).collect();
        assert!(
            out.iter()
                .all(|y| y.abs() <= super::Limiter::CEILING + 1e-6)
        );
        // Once it has caught the peak, the shape is the input's, only quieter.
        let ratios: Vec<f32> = wave
            .iter()
            .zip(&out)
            .skip(200)
            .filter(|(x, _)| x.abs() > 0.5)
            .map(|(x, y)| y / x)
            .collect();
        let (lo, hi) = ratios
            .iter()
            .fold((f32::MAX, 0f32), |(lo, hi), &r| (lo.min(r), hi.max(r)));
        assert!(hi / lo < 1.02, "{lo}..{hi}");
    }

    /// Losing the focus fades the volume over a second or so instead of cutting it; it comes
    /// back quickly.
    #[test]
    fn the_volume_fades_out_slowly_without_the_focus_and_back_quickly() {
        let callback = 0.01;
        let after = |mut at: f32, to: f32, secs: f32| {
            for _ in 0..(secs / callback).round() as usize {
                at = focus_step(at, to, callback);
            }
            at
        };
        assert!(
            after(1.0, 0.35, 0.1) > 0.9,
            "a glance away barely lowers it"
        );
        let half = after(1.0, 0.35, 0.5);
        assert!(half > 0.6 && half < 0.7, "{half}");
        assert_eq!(after(1.0, 0.35, 1.5), 0.35);
        assert_eq!(after(0.35, 1.0, 0.25), 1.0);
        assert!(after(0.35, 1.0, 0.1) > 0.6);
        // Never past where it's going.
        assert_eq!(after(0.36, 0.35, 1.0), 0.35);
        assert_eq!(after(0.99, 1.0, 1.0), 1.0);
    }

    fn mixer() -> Mixer {
        let (_tx, rx) = mpsc::channel();
        let (gc, _garbage) = mpsc::channel();
        let stamp = Arc::new(Mutex::new(Stamp {
            playing: false,
            frame: 0,
            at: Instant::now(),
            latency: Duration::ZERO,
            settled: false,
            reported: Duration::ZERO,
            epoch: 0,
        }));
        let volume = Arc::new([
            AtomicU32::new(1),
            AtomicU32::new(1),
            AtomicU32::new(1.0f32.to_bits()),
        ]);
        Mixer::new(rx, gc, stamp, volume, 48_000, 2)
    }

    fn silence(frames: usize) -> Arc<Track> {
        Arc::new(Track {
            samples: vec![0.0; frames * 2],
            rate: 1.0,
            stream: None,
        })
    }

    fn playing(m: &mut Mixer, at: i64) {
        let track = silence(48_000 * 10);
        m.apply(Cmd::Play {
            track,
            start_frame: at,
            base: 0,
        });
    }

    /// Runs the frames of a callback the way `render` does for sounds: the song frame each voice
    /// starts on (None if it hasn't).
    fn run(m: &mut Mixer, frames: usize) -> Vec<i64> {
        let mut started = Vec::new();
        for _ in 0..frames {
            let before = m.voices.len();
            if !m.pending.is_empty() {
                m.start_due();
            }
            if m.voices.len() > before {
                started.push(m.frame);
            }
            m.frame += 1;
        }
        started
    }

    #[test]
    fn a_scheduled_sound_starts_on_its_song_frame_inside_a_buffer() {
        let mut m = mixer();
        playing(&mut m, 1000);
        m.apply(Cmd::SfxAt(1300, Sfx::Don, 1.0));
        assert_eq!(m.pending.len(), 1);
        assert!(m.voices.is_empty());
        assert_eq!(run(&mut m, 256), Vec::<i64>::new());
        assert_eq!(run(&mut m, 256), vec![1300]);
        assert!(m.pending.is_empty());
        // Never again.
        assert_eq!(run(&mut m, 1000), Vec::<i64>::new());
        assert_eq!(m.voices.len(), 1);
    }

    #[test]
    fn a_sound_whose_frame_has_passed_plays_at_once() {
        let mut m = mixer();
        playing(&mut m, 1000);
        m.apply(Cmd::SfxAt(900, Sfx::Kat, 1.0));
        m.apply(Cmd::SfxAt(1000, Sfx::Kat, 1.0));
        assert!(m.pending.is_empty());
        assert_eq!(m.voices.len(), 2);
    }

    #[test]
    fn seek_pause_restart_and_stop_drop_waiting_sounds() {
        let cmds: [fn() -> Cmd; 4] = [
            || Cmd::Seek(50),
            || Cmd::Pause(true),
            || Cmd::Stop,
            || Cmd::Play {
                track: silence(100),
                start_frame: 0,
                base: 0,
            },
        ];
        for cmd in cmds {
            let mut m = mixer();
            playing(&mut m, 1000);
            m.apply(Cmd::SfxAt(5000, Sfx::Don, 1.0));
            m.apply(Cmd::SfxAt(6000, Sfx::Kat, 1.0));
            assert_eq!(m.pending.len(), 2);
            m.apply(cmd());
            assert!(m.pending.is_empty());
            m.frame = 10_000;
            m.start_due();
            assert!(m.voices.is_empty());
        }
    }

    #[test]
    fn a_swap_keeps_waiting_sounds_and_a_full_queue_plays_at_once() {
        let mut m = mixer();
        playing(&mut m, 0);
        m.apply(Cmd::SfxAt(5000, Sfx::Don, 1.0));
        m.apply(Cmd::Swap {
            track: silence(100),
            base: 0,
        });
        assert_eq!(m.pending.len(), 1);
        for _ in 0..m.pending.capacity() {
            m.apply(Cmd::SfxAt(5000, Sfx::Kat, 1.0));
        }
        assert_eq!(m.pending.len(), m.pending.capacity());
        assert!(!m.voices.is_empty());
    }

    #[test]
    fn mashing_a_sound_rings_it_twice_at_most() {
        let mut voices = Vec::with_capacity(64);
        for _ in 0..6 {
            start_voice(&mut voices, Sfx::BigDon, 1.0);
            for v in &mut voices {
                v.pos += 100;
            }
        }
        start_voice(&mut voices, Sfx::Kat, 1.0);
        let ringing = |s: Sfx| {
            voices
                .iter()
                .filter(|v| v.sound == s && v.fade == 0)
                .count()
        };
        assert_eq!((ringing(Sfx::BigDon), ringing(Sfx::Kat)), (2, 1));
        // The two playing on are the newest.
        assert!(
            voices
                .iter()
                .filter(|v| v.fade == 0 && v.sound == Sfx::BigDon)
                .all(|v| v.pos <= 200)
        );
    }

    #[test]
    fn built_in_hitsounds_decode() {
        // don, kat, finisher, combo break (assets/hitsounds/synth.py): short, and not silent.
        for i in 0..BUILT_IN.len() {
            let s = built_in(i, 48_000);
            let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(
                (4_800..48_000).contains(&s.len()) && peak > 0.5,
                "sound {i}: {} samples, peak {peak}",
                s.len()
            );
        }
    }

    fn sine(freq: f32, seconds: f32, sr: u32) -> Vec<f32> {
        let n = (seconds * sr as f32) as usize;
        (0..n)
            .flat_map(|i| {
                let v = (i as f32 / sr as f32 * freq * std::f32::consts::TAU).sin() * 0.5;
                [v, v]
            })
            .collect()
    }

    /// Dominant frequency from upward zero crossings of the left channel.
    fn frequency(samples: &[f32], sr: u32) -> f32 {
        let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        crossings as f32 / (left.len() as f32 / sr as f32)
    }

    #[test]
    fn keep_pitch_changes_length_not_pitch() {
        let sr = 48000;
        let input = Track {
            samples: sine(440.0, 4.0, sr),
            rate: 1.0,
            stream: None,
        };
        for rate in [0.75, 1.5, 2.0] {
            let out = change_rate(
                Track {
                    samples: input.samples.clone(),
                    rate: 1.0,
                    stream: None,
                },
                sr,
                rate,
                true,
            );
            let len_ratio = input.frames() as f64 / out.frames() as f64;
            assert!(
                (len_ratio - rate).abs() < 0.03,
                "rate {rate}: length ratio {len_ratio}"
            );
            let f = frequency(&out.samples, sr);
            assert!(
                (f - 440.0).abs() < 8.0,
                "rate {rate}: pitch moved to {f} Hz"
            );
            assert_eq!(out.rate, rate);
        }
    }

    #[test]
    fn partial_decode_lines_up_with_full_decode() {
        // A 6 s stereo WAV with a sweep, so any misalignment shows up as a difference.
        let sr = 44100u32;
        let frames = sr as usize * 6;
        let mut wav = Vec::new();
        let data_len = (frames * 4) as u32;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&sr.to_le_bytes());
        wav.extend_from_slice(&(sr * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let t = i as f32 / sr as f32;
            let v = ((t * (200.0 + 300.0 * t)) * std::f32::consts::TAU).sin() * 0.5;
            for c in [v, -v] {
                wav.extend_from_slice(&((c * 32767.0) as i16).to_le_bytes());
            }
        }
        let dir = std::env::temp_dir().join(format!("taiko-rs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sweep.wav");
        std::fs::write(&path, wav).unwrap();
        for rate in [44100u32, 48000] {
            let full = decode(&path, rate).unwrap();
            let (part, base) = decode_part(&path, rate, Some((2500.0, 2000.0))).unwrap();
            let start_ms = base as f64 / rate as f64 * 1000.0;
            assert!(
                (2400.0..=2500.0).contains(&start_ms),
                "starts at {start_ms} ms"
            );
            assert!(
                part.frames() as u32 >= rate * 19 / 10,
                "only {} frames",
                part.frames()
            );
            // Away from the edges (where resampling clamps), the samples are the same.
            for i in 8..part.frames() - 8 {
                let j = (base as usize + i) * 2;
                assert!(
                    (full.samples[j] - part.samples[i * 2]).abs() < 1e-4,
                    "frame {i} differs at {rate} Hz"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slipping_device_position_leaves_the_latency_alone() {
        // A player's stream: 448 frames asked per callback, 1429 frames (32.4 ms) reported.
        let rate = 44100;
        let frames = |n: usize| Duration::from_secs_f64(n as f64 / rate as f64);
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let mut steady = SteadyLatency::new();
        // While it starts up, what it says goes.
        assert_eq!(steady.of(frames(0), 448, rate), frames(0));
        for _ in 1..LATENCY_SETTLE {
            steady.of(frames(700), 448, rate);
        }
        for k in 0..LATENCY_SAMPLES {
            // A stray reading while judging it doesn't count.
            steady.of(frames(if k == 3 { 2000 } else { 1429 }), 448, rate);
        }
        // Slipped either way, by whole periods: it's the position that moved, not the sound.
        for slipped in [1429 + 12 * 448, 1429 - 448, 1429 - 2 * 448] {
            assert!((ms(steady.of(frames(slipped), 448, rate)) - 32.4).abs() < 0.05);
        }
        // A callback a period late is asked for a period more, and the buffer really is that low.
        let expected = (1429 - 448) as f64 / rate as f64 * 1000.0;
        assert!((ms(steady.of(frames(1429), 896, rate)) - expected).abs() < 0.05);
    }

    /// `ogg` with the position of its `page`th page moved `by` frames (and the page's checksum put
    /// right, or readers would skip it).
    fn with_ogg_position_moved(mut ogg: Vec<u8>, page: usize, by: i64) -> Vec<u8> {
        let mut at = 0;
        for _ in 0..page {
            at += 27
                + ogg[at + 26] as usize
                + ogg[at + 27..at + 27 + ogg[at + 26] as usize]
                    .iter()
                    .map(|&s| s as usize)
                    .sum::<usize>();
        }
        let segments = ogg[at + 26] as usize;
        let end = at
            + 27
            + segments
            + ogg[at + 27..at + 27 + segments]
                .iter()
                .map(|&s| s as usize)
                .sum::<usize>();
        let position = i64::from_le_bytes(ogg[at + 6..at + 14].try_into().unwrap()) + by;
        ogg[at + 6..at + 14].copy_from_slice(&position.to_le_bytes());
        ogg[at + 22..at + 26].fill(0);
        let crc = ogg[at..end].iter().fold(0u32, |crc, &byte| {
            (0..8).fold(crc ^ (byte as u32) << 24, |c, _| {
                if c & 0x8000_0000 != 0 {
                    c << 1 ^ 0x04c1_1db7
                } else {
                    c << 1
                }
            })
        });
        ogg[at + 22..at + 26].copy_from_slice(&crc.to_le_bytes());
        ogg
    }

    #[test]
    fn a_wrong_position_on_an_ogg_page_loses_no_music() {
        // Real maps' OGGs have such pages: Clublionx lost 412 frames at each of 27 of them, so its
        // music ended 252 ms ahead of its notes and of BASS.
        let clean = include_bytes!("../assets/splash.ogg").to_vec();
        let bent = with_ogg_position_moved(clean.clone(), 2, 412);
        let decoded = |ogg: Vec<u8>| {
            decode_source(
                Box::new(std::io::Cursor::new(ogg)),
                Some("ogg"),
                48000,
                None,
            )
            .unwrap()
            .0
        };
        let (clean, bent) = (decoded(clean), decoded(bent));
        assert_eq!(clean.frames(), bent.frames());
        assert_eq!(clean.samples, bent.samples);
    }

    /// A silent MP3 of `frames` frames after an Info tag naming `encoder`, with `trim` in the tag's
    /// delay and padding field (12 bits each).
    fn mp3_with_info_tag(encoder: &[u8; 9], trim: [u8; 3], frames: usize) -> Vec<u8> {
        const HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x44]; // MPEG-1 layer III, 128 kb/s, 44100 Hz.
        const FRAME: usize = 417;
        const TAG: usize = 4 + 32; // Past the header and the stereo side information.
        let mut silent = vec![0; FRAME];
        silent[..4].copy_from_slice(&HEADER);
        let mut mp3 = silent.clone();
        mp3[TAG..TAG + 4].copy_from_slice(b"Info");
        mp3[TAG + 7] = 1; // Flags: the number of frames comes next, and nothing else.
        mp3[TAG + 8..TAG + 12].copy_from_slice(&(frames as u32).to_be_bytes());
        mp3[TAG + 12..TAG + 21].copy_from_slice(encoder);
        mp3[TAG + 33..TAG + 36].copy_from_slice(&trim);
        mp3.extend(silent.repeat(frames));
        mp3
    }

    #[test]
    fn the_delay_in_an_mp3_tag_is_cut_whoever_encoded_it() {
        // Mythologia's End's own tag: 576 + 529 frames of delay, 2088 - 529 of padding. BASS (and
        // so osu!, stable and lazer alike) cuts them; left in, its music came 25 ms late.
        let frames = |encoder: &[u8; 9], trim| {
            let mp3 = mp3_with_info_tag(encoder, trim, 20);
            decode_source(
                Box::new(std::io::Cursor::new(mp3)),
                Some("mp3"),
                44100,
                None,
            )
            .unwrap()
            .0
            .frames()
        };
        let whole = 20 * 1152;
        assert_eq!(
            frames(b"SIT4.50.0", [0x24, 0x08, 0x28]),
            whole - 1105 - 1559
        );
        assert_eq!(
            frames(b"GOGO\0\0\0\0\0", [0x24, 0x08, 0x28]),
            whole - 1105 - 1559
        );
        // And what BASS doesn't cut, left alone: no name, or an empty field.
        assert_eq!(frames(b"\0ABCDEFGH", [0x24, 0x08, 0x28]), whole);
        assert_eq!(frames(b"SIT4.50.0", [0, 0, 0]), whole);
    }

    fn pieces_of(samples: &[f32]) -> Pieces {
        let mut p = Pieces::default();
        for [l, r] in samples.as_chunks::<2>().0 {
            p.push(*l, *r);
        }
        p
    }

    /// Past a piece's end, where the reading crosses from one to the next.
    fn long_samples() -> Vec<f32> {
        (0..Pieces::SIZE * 2 + 5000)
            .map(|i| ((i as f32) * 0.37).sin())
            .collect()
    }

    #[test]
    fn a_song_in_pieces_comes_out_as_it_went_in() {
        let samples = long_samples();
        assert_eq!(pieces_of(&samples).into_samples(), samples);
        for keep in [samples.len() - 2, Pieces::SIZE, Pieces::SIZE + 2, 10, 0] {
            let mut p = pieces_of(&samples);
            p.truncate(keep);
            assert_eq!(p.len, keep);
            assert_eq!(p.into_samples(), samples[..keep]);
        }
    }

    #[test]
    fn resampling_in_pieces_is_resampling() {
        let samples = long_samples();
        for (ratio, phase) in [
            (44100.0 / 48000.0, 0.0),
            (48000.0 / 44100.0, 0.25),
            (0.5, 0.0),
        ] {
            assert!(
                pieces_of(&samples).resampled(ratio, phase)
                    == resample_from(&samples, ratio, phase)
            );
        }
    }

    #[test]
    fn stretching_stops_when_its_called_off() {
        let track = metronome(48000, 500.0, 8, 0.0);
        let called_off = Arc::new(AtomicBool::new(true));
        let on_a_thread = {
            let called_off = called_off.clone();
            std::thread::spawn(move || {
                while_wanted(called_off);
                (
                    change_rate(track, 48000, 1.25, true).samples.len(),
                    prepare_alone().is_none(),
                )
            })
        };
        let (samples, turned_away) = on_a_thread.join().unwrap();
        assert_eq!(samples, 0, "nothing worked out for nobody");
        assert!(turned_away);
        called_off.store(false, Ordering::Relaxed);
    }

    #[test]
    fn a_track_nobody_else_has_isnt_copied() {
        let track = Arc::new(Track {
            samples: vec![1.0; 8],
            rate: 1.0,
            stream: None,
        });
        let at = track.samples.as_ptr();
        assert_eq!(owned(track).samples.as_ptr(), at);
        let shared = Arc::new(Track {
            samples: vec![1.0; 8],
            rate: 1.0,
            stream: None,
        });
        let copy = owned(shared.clone());
        assert_ne!(copy.samples.as_ptr(), shared.samples.as_ptr());
        assert_eq!(copy.samples, shared.samples);
    }

    /// Real files (paths in TAIKO_TEST_AUDIO, one per line in that file): a part decoded after a
    /// seek must line up with the full decode. `cargo test -- --ignored --nocapture part_of_real`.
    #[test]
    #[ignore]
    fn part_of_real_files_lines_up() {
        let Some(list) = std::env::var_os("TAIKO_TEST_AUDIO") else {
            return;
        };
        let rate = 48000;
        for path in std::fs::read_to_string(list)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
        {
            let path = std::path::Path::new(path);
            let full = decode(path, rate).unwrap();
            for from in [30_000.0, 61_234.0] {
                let (part, base) = decode_part(path, rate, Some((from, 3000.0))).unwrap();
                // The shift (in frames) at which the part best matches the full decode.
                let window = (rate as usize / 2).min(part.frames().saturating_sub(4000));
                let best = (-3000i64..=3000)
                    .min_by(|&a, &b| {
                        let err = |lag: i64| -> f64 {
                            (2000..2000 + window)
                                .map(|i| {
                                    let j = base + i as i64 + lag;
                                    let f = full
                                        .samples
                                        .get(j.max(0) as usize * 2)
                                        .copied()
                                        .unwrap_or(0.0);
                                    ((f - part.samples[i * 2]) as f64).powi(2)
                                })
                                .sum()
                        };
                        err(a).total_cmp(&err(b))
                    })
                    .unwrap();
                println!(
                    "{:60} from {from:>6} ms: part is {best:+} frames ({:+.1} ms) off the full decode",
                    path.display()
                        .to_string()
                        .chars()
                        .rev()
                        .take(60)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>(),
                    best as f64 / rate as f64 * 1000.0
                );
            }
        }
    }

    /// Music under the beats, for the stretch's search to lock on to: where the beats themselves
    /// are too sparse to show it walking off its notes.
    #[test]
    fn keep_pitch_stays_on_the_beat_at_rates_close_to_1x() {
        let sr = 48000;
        let mut input = metronome(sr, 500.0, 40, 1000.0);
        for (i, v) in input.samples.chunks_mut(2).enumerate() {
            let tone = 0.15 * (i as f32 * std::f32::consts::TAU * 196.0 / sr as f32).sin();
            v[0] += tone;
            v[1] += tone;
        }
        for rate in [0.9, 0.95, 1.05, 1.1] {
            let track = Track {
                samples: input.samples.clone(),
                rate: 1.0,
                stream: None,
            };
            let out = change_rate(track, sr, rate, true);
            let ms = |t: f64| (t / 1000.0 * sr as f64) as usize;
            let (mut sum, mut n) = (0.0, 0);
            for b in 4..36 {
                let expected = (1000.0 + b as f64 * 500.0) / rate;
                let (from, to) = (ms(expected - 40.0), ms(expected + 60.0));
                let at = |i: usize| out.samples[i * 2].abs();
                let peak = (from..to).map(at).fold(0.0, f32::max);
                if let Some(onset) = (from..to).find(|&i| at(i) >= peak * 0.7) {
                    sum += onset as f64 / sr as f64 * 1000.0 - expected;
                    n += 1;
                }
            }
            let mean = sum / f64::from(n);
            assert!(
                mean.abs() < 3.5,
                "rate {rate}: beats {mean:+.1} ms off on average"
            );
        }
    }

    #[test]
    fn keep_pitch_keeps_beats_on_time() {
        // Clicks every 500 ms: after stretching, each must start where its time / rate says,
        // or every DT/HT map would play off its notes.
        let sr = 48000;
        let input = metronome(sr, 500.0, 16, 1000.0);
        for rate in [0.75, 1.3, 1.5] {
            let out = change_rate(
                Track {
                    samples: input.samples.clone(),
                    rate: 1.0,
                    stream: None,
                },
                sr,
                rate,
                true,
            );
            let mut sum = 0.0;
            for b in 2..14 {
                let expected = b as f64 * 500.0 / rate;
                // Where the click reaches half its peak: grains fade in, so its first faint
                // sample can come early without being heard early.
                let ms = |t: f64| (t / 1000.0 * sr as f64) as usize;
                let (from, to) = (ms(expected - 40.0), ms(expected + 60.0));
                let peak = (from..to)
                    .map(|i| out.samples[i * 2].abs())
                    .fold(0.0, f32::max);
                let onset = (from..to)
                    .find(|&i| out.samples[i * 2].abs() >= peak * 0.5)
                    .unwrap();
                let err = onset as f64 / sr as f64 * 1000.0 - expected;
                // A click can straddle two grains: each lands within a few ms, on time on average.
                assert!(err.abs() < 6.0, "rate {rate}: beat {b} off by {err:.1} ms");
                sum += err;
            }
            let mean = sum / 12.0;
            assert!(
                mean.abs() < 3.5,
                "rate {rate}: beats {mean:.1} ms off on average"
            );
        }
    }

    #[test]
    fn wind_up_puts_beats_where_the_speed_map_says() {
        let sr = 48000;
        let input = metronome(sr, 500.0, 24, 1000.0);
        let speed = crate::mods::Speed {
            r0: 1.0,
            r1: 1.5,
            m0: 1000.0,
            m1: 11_000.0,
        };
        for keep_pitch in [true, false] {
            let out = change_speed(
                Track {
                    samples: input.samples.clone(),
                    rate: 1.0,
                    stream: None,
                },
                sr,
                &speed,
                keep_pitch,
            );
            assert_eq!(out.rate, 1.0);
            for b in 2..22 {
                let expected = speed.track(b as f64 * 500.0);
                let ms = |t: f64| (t / 1000.0 * sr as f64) as usize;
                let (from, to) = (ms(expected - 40.0), ms(expected + 60.0));
                let peak = (from..to)
                    .map(|i| out.samples[i * 2].abs())
                    .fold(0.0, f32::max);
                let onset = (from..to)
                    .find(|&i| out.samples[i * 2].abs() >= peak * 0.5)
                    .unwrap();
                let err = onset as f64 / sr as f64 * 1000.0 - expected;
                assert!(
                    err.abs() < 6.0,
                    "keep_pitch {keep_pitch}: beat {b} off by {err:.1} ms"
                );
            }
        }
    }

    #[test]
    fn without_keep_pitch_pitch_follows_speed() {
        let sr = 48000;
        let out = change_rate(
            Track {
                samples: sine(440.0, 2.0, sr),
                rate: 1.0,
                stream: None,
            },
            sr,
            1.5,
            false,
        );
        let f = frequency(&out.samples, sr);
        assert!((f - 660.0).abs() < 8.0, "expected ~660 Hz, got {f}");
    }
}

/// Deterministic white noise so the metronome clicks are identical every run.
struct Noise(u32);
impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// A skin's sounds as mono samples at the device rate; `None` keeps the built-in one.
#[derive(Default)]
pub struct Custom {
    pub don: Option<Vec<f32>>,
    pub kat: Option<Vec<f32>>,
    /// Played on top of don / kat for big notes.
    pub finish: Option<Vec<f32>>,
    pub combobreak: Option<Vec<f32>>,
    /// Overtime's scream (`overtime.wav`, from the skin of its own part).
    pub scream: Option<Vec<f32>>,
}

/// The built-in hitsounds (`assets/hitsounds`, from the quassbot v2 skin).
const BUILT_IN: [(&[u8], &str); 4] = [
    (include_bytes!("../assets/hitsounds/don.wav"), "wav"),
    (include_bytes!("../assets/hitsounds/kat.wav"), "wav"),
    (include_bytes!("../assets/hitsounds/finish.wav"), "wav"),
    (include_bytes!("../assets/hitsounds/combobreak.ogg"), "ogg"),
];

/// A built-in sound as mono samples at `rate`.
fn built_in(index: usize, rate: u32) -> Vec<f32> {
    let (bytes, ext) = BUILT_IN[index];
    decode_bytes(bytes, ext, rate)
        .map(|t| mono(&t))
        .unwrap_or_default()
}

/// Stereo samples folded down to mono.
pub fn mono(track: &Track) -> Vec<f32> {
    track
        .samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|f| (f[0] + f[1]) * 0.5)
        .collect()
}

/// The sounds (indexed by `Sfx`) at `rate`: a skin's where given, else built-in. Like osu!, a
/// big note is the normal hit with `finish` on top.
pub fn hitsound_set(rate: u32, custom: Custom) -> Sounds {
    struct Builtin {
        sounds: Sounds,
        finish: Arc<[f32]>,
    }
    static CACHE: OnceLock<Mutex<std::collections::HashMap<u32, Arc<Builtin>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let cached = cache.lock().unwrap().get(&rate).cloned();
    let base = cached.unwrap_or_else(|| {
        // Build outside the cache lock: different-rate preparation need not block a lookup.
        let don = built_in(0, rate);
        let kat = built_in(1, rate);
        let finish = built_in(2, rate);
        let [kantan, futsuu, muzukashii, oni, ura] = crate::medal_sound::sounds(rate);
        let sounds = [
            don.clone(),
            kat.clone(),
            mix(&don, &finish),
            mix(&kat, &finish),
            built_in(3, rate),
            screech(rate),
            chime(rate),
            kantan,
            futsuu,
            muzukashii,
            oni,
            ura,
        ]
        .map(Arc::from);
        let value = Arc::new(Builtin {
            sounds,
            finish: finish.into(),
        });
        cache.lock().unwrap().entry(rate).or_insert(value).clone()
    });
    let mut sounds = base.sounds.clone();
    let custom_don = custom.don.is_some();
    let custom_kat = custom.kat.is_some();
    let custom_finish = custom.finish.is_some();
    if let Some(don) = custom.don {
        sounds[Sfx::Don as usize] = don.into();
    }
    if let Some(kat) = custom.kat {
        sounds[Sfx::Kat as usize] = kat.into();
    }
    let finish = custom
        .finish
        .map(Arc::from)
        .unwrap_or_else(|| base.finish.clone());
    if custom_don || custom_finish {
        sounds[Sfx::BigDon as usize] = mix(&sounds[Sfx::Don as usize], &finish).into();
    }
    if custom_kat || custom_finish {
        sounds[Sfx::BigKat as usize] = mix(&sounds[Sfx::Kat as usize], &finish).into();
    }
    if let Some(combo) = custom.combobreak {
        sounds[Sfx::ComboBreak as usize] = combo.into();
    }
    if let Some(scream) = custom.scream {
        sounds[Sfx::Scream as usize] = scream.into();
    }
    sounds
}

/// Overtime's scream, made up rather than sampled: detuned saws sweeping up with a fast
/// wobble, ring-modulated, over noise, then driven into clipping.
fn screech(rate: u32) -> Vec<f32> {
    use std::f32::consts::TAU;
    const LEN: f32 = 1.3;
    let sr = rate as f32;
    let mut noise = Noise(0x5eed_f00d);
    let mut phase = [0.0f32; 3];
    (0..(sr * LEN) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            let env = (t / 0.012).min(1.0) * (1.0 - t / LEN).max(0.0).powf(0.6);
            let f = 380.0 + 260.0 * t + 90.0 * (t * 11.0 * TAU).sin();
            let mut saw = 0.0;
            for (p, k) in phase.iter_mut().zip([1.0, 1.49, 2.03]) {
                *p = (*p + f * k / sr).fract();
                saw += *p * 2.0 - 1.0;
            }
            let ring = (t * 57.0 * TAU).sin();
            let x = saw * 0.5 * (0.6 + 0.4 * ring) + noise.next() * 0.45;
            (x * 2.5).tanh() * 0.7 * env
        })
        .collect()
}

/// A friend's message: two soft bell notes going up a fourth (E6, A6), made up rather than
/// sampled.
fn chime(rate: u32) -> Vec<f32> {
    use std::f32::consts::TAU;
    let sr = rate as f32;
    let mut out = vec![0.0f32; (sr * 0.6) as usize];
    for (start, freq) in [(0.0, 1318.5), (0.09, 1760.0)] {
        let from = (start * sr) as usize;
        for (i, o) in out[from..].iter_mut().enumerate() {
            let t = i as f32 / sr;
            // A quick swell in, a bell-like fade out; a little of the octave for shimmer.
            let env = (t / 0.004).min(1.0) * (-t * 9.0).exp();
            *o += env * 0.22 * ((t * freq * TAU).sin() + 0.25 * (t * freq * 2.0 * TAU).sin());
        }
    }
    out
}

/// Two mono sounds played together.
fn mix(a: &[f32], b: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; a.len().max(b.len())];
    for (i, o) in out.iter_mut().enumerate() {
        *o = a.get(i).copied().unwrap_or(0.0) + b.get(i).copied().unwrap_or(0.0);
    }
    out
}

/// A metronome track: a sharp click exactly on every beat (accented every 4th), for offset
/// calibration. Each click's transient starts on the exact sample of its beat.
pub fn metronome(rate: u32, beat_ms: f64, beats: u32, tail_ms: f64) -> Track {
    let sr = rate as f64;
    let len = ((beats as f64 * beat_ms + tail_ms) / 1000.0 * sr) as usize;
    let mut samples = vec![0.0f32; len * 2];
    let click_len = (sr * 0.045) as usize;
    let mut noise = Noise(0x2545_f491);
    for b in 0..beats {
        let start = (b as f64 * beat_ms / 1000.0 * sr).round() as usize;
        let accent = b % 4 == 0;
        let freq = if accent { 1760.0 } else { 1320.0 };
        let gain = if accent { 0.75 } else { 0.55 };
        for i in 0..click_len {
            let Some(frame) = samples.get_mut((start + i) * 2..(start + i) * 2 + 2) else {
                break;
            };
            let t = i as f32 / sr as f32;
            let tone = (t * freq * std::f32::consts::TAU).sin() * (-t * 110.0).exp();
            let tick = noise.next() * (-t * 900.0).exp() * 0.6;
            let v = (tone + tick) * gain;
            frame[0] += v;
            frame[1] += v;
        }
    }
    Track {
        samples,
        rate: 1.0,
        stream: None,
    }
}

/// What a job gives back when it's called off.
const CANCELLED: &str = "cancelled";

thread_local! {
    /// Set on a thread preparing music nobody may want by the time it's done (`while_wanted`).
    static CALLED_OFF: std::cell::RefCell<Option<Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
}

/// Decoding and stretching on this thread stop when `flag` is set: a long song takes gigabytes
/// and a good while, and a player scrolling past it shouldn't pay for that.
pub fn while_wanted(flag: Arc<AtomicBool>) {
    CALLED_OFF.with(|c| *c.borrow_mut() = Some(flag));
}

fn called_off() -> bool {
    CALLED_OFF.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
    })
}

/// Only one song's music is prepared for a preview at a time: each can take gigabytes.
static PREPARING: Mutex<()> = Mutex::new(());

/// Holds the others back while a song's music is prepared. None: this job was called off waiting.
pub fn prepare_alone() -> Option<std::sync::MutexGuard<'static, ()>> {
    let turn = PREPARING.lock().unwrap_or_else(|e| e.into_inner());
    (!called_off()).then_some(turn)
}

/// A track's samples to change: its own if nobody else has it, else a copy. A long song is
/// gigabytes.
pub fn owned(track: Arc<Track>) -> Track {
    Arc::try_unwrap(track).unwrap_or_else(|t| Track {
        samples: t.samples.clone(),
        rate: t.rate,
        stream: None,
    })
}

/// Samples as they're decoded, in pieces: the pieces can go one by one once they've been read,
/// where a single buffer would stay whole, next to its resampled copy, until the end. A long song
/// is gigabytes, and growing one buffer would also take twice its size at the worst moment.
#[derive(Default)]
struct Pieces {
    pieces: Vec<Vec<f32>>,
    len: usize,
}

impl Pieces {
    /// Samples to a piece: a whole number of frames, and a power of two to find them by.
    const SHIFT: usize = 22;
    const SIZE: usize = 1 << Self::SHIFT;

    fn push(&mut self, l: f32, r: f32) {
        if self.pieces.last().is_none_or(|p| p.len() >= Self::SIZE) {
            self.pieces.push(Vec::with_capacity(Self::SIZE));
        }
        if let Some(piece) = self.pieces.last_mut() {
            piece.push(l);
            piece.push(r);
        }
        self.len += 2;
    }

    fn truncate(&mut self, len: usize) {
        self.len = self.len.min(len);
        let keep = self.len.div_ceil(Self::SIZE);
        self.pieces.truncate(keep);
        if let Some(last) = self.pieces.last_mut() {
            last.truncate(self.len - (keep - 1) * Self::SIZE);
        }
    }

    /// All the samples in one buffer, each piece going as it's moved.
    fn into_samples(self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.len);
        for piece in self.pieces {
            out.extend_from_slice(&piece);
        }
        out
    }

    /// `resample_from`, each piece going once it's behind the reading position.
    fn resampled(mut self, ratio: f64, phase: f64) -> Vec<f32> {
        let frames_in = self.len / 2;
        let frames_out = ((frames_in as f64 - phase) / ratio).floor().max(0.0) as usize;
        let mut out = Vec::with_capacity(frames_out * 2);
        let mut gone = 0;
        for n in 0..frames_out {
            let pos = phase + n as f64 * ratio;
            let i = pos.floor() as isize;
            let t = (pos - i as f64) as f32;
            // Frames are read from i - 1 on, from here on.
            let reading = ((i - 1).max(0) as usize * 2) >> Self::SHIFT;
            while gone < reading {
                self.pieces[gone] = Vec::new();
                gone += 1;
            }
            let pieces = &self.pieces;
            let at = |i: isize, c: usize| -> f32 {
                let at = i.clamp(0, frames_in as isize - 1) as usize * 2 + c;
                pieces[at >> Self::SHIFT][at & (Self::SIZE - 1)]
            };
            for c in 0..2 {
                out.push(cubic(t, at(i - 1, c), at(i, c), at(i + 1, c), at(i + 2, c)));
            }
        }
        out
    }
}

/// Decodes a whole audio file to interleaved stereo at `rate`.
pub fn decode(path: &Path, rate: u32) -> Result<Track, String> {
    decode_part(path, rate, None).map(|(track, _)| track)
}

/// Full decode like `decode`, but when another thread is already decoding the same file at the
/// same rate it waits for that result instead of decoding it again (starting a song while its
/// preview is still being decoded).
pub fn decode_shared(path: &Path, rate: u32) -> Result<Arc<Track>, String> {
    type Slot = Arc<std::sync::OnceLock<Result<Arc<Track>, String>>>;
    static IN_FLIGHT: Mutex<Vec<(std::path::PathBuf, u32, Slot)>> = Mutex::new(Vec::new());
    let (slot, mine) = {
        let mut v = IN_FLIGHT.lock().unwrap();
        match v.iter().find(|(p, r, _)| p == path && *r == rate) {
            Some((_, _, s)) => (s.clone(), false),
            None => {
                let s: Slot = Arc::default();
                v.push((path.to_path_buf(), rate, s.clone()));
                (s, true)
            }
        }
    };
    let result = slot
        .get_or_init(|| decode(path, rate).map(Arc::new))
        .clone();
    if mine {
        IN_FLIGHT
            .lock()
            .unwrap()
            .retain(|(_, _, s)| !Arc::ptr_eq(s, &slot));
    }
    // Whoever it was that was decoding it no longer wanted it. We do.
    if !mine && result.as_ref().is_err_and(|e| e == CANCELLED) {
        return decode_shared(path, rate);
    }
    result
}

/// `track` for a speed modifier (see `change_rate`); the same track at normal speed.
pub fn with_rate(track: Arc<Track>, sample_rate: u32, rate: f64, keep_pitch: bool) -> Arc<Track> {
    if (rate - 1.0).abs() < 1e-6 {
        return track;
    }
    // Read where it is: a copy of a long song is gigabytes.
    Arc::new(rate_changed(&track, sample_rate, rate, keep_pitch))
}

/// Decodes `len_ms` of audio starting at (or just before) `from_ms`, or the whole file with
/// `None`. Also returns the device-rate frame the first sample belongs at: the samples are
/// identical to the same stretch of a full decode, so one can be swapped for the other mid-song.
pub fn decode_part(
    path: &Path,
    rate: u32,
    part: Option<(f64, f64)>,
) -> Result<(Track, i64), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    decode_source(
        Box::new(file),
        path.extension().and_then(|e| e.to_str()),
        rate,
        part,
    )
}

/// Decodes audio built into the binary (`ext`: its format's file extension).
pub fn decode_bytes(bytes: &'static [u8], ext: &str, rate: u32) -> Result<Track, String> {
    decode_source(Box::new(std::io::Cursor::new(bytes)), Some(ext), rate, None)
        .map(|(track, _)| track)
}

/// An audio file being read, a packet at a time.
pub(crate) struct Reader {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    track_id: u32,
    time_base: Option<symphonia::core::units::TimeBase>,
    /// The file's sample rate (0 until a packet's been decoded).
    pub src_rate: u32,
    /// Source frame of the first decoded sample, once it's known (0 from the top).
    pub src_start: Option<i64>,
    packet_buf: Vec<f32>,
    /// Frames to cut off the end of the song. Only the end: an OGG page whose position is a little
    /// off has the reader cut the packet it ends on, mid-song. That's music, BASS plays it, and
    /// everything after it would come that much early.
    pub tail: usize,
}

impl Reader {
    pub fn open(
        source: Box<dyn symphonia::core::io::MediaSource>,
        ext: Option<&str>,
    ) -> Result<Reader, String> {
        use symphonia::core::codecs::audio::AudioDecoderOptions;
        use symphonia::core::formats::probe::Hint;
        use symphonia::core::formats::{FormatOptions, TrackType};
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;

        let mss = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = ext {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| format!("probe: {e}"))?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or("no audio track")?;
        let track_id = track.id;
        let time_base = track.time_base;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or("no audio codec params")?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| format!("decoder: {e}"))?;
        Ok(Reader {
            format,
            decoder,
            track_id,
            time_base,
            src_rate: 0,
            src_start: None,
            packet_buf: Vec::new(),
            tail: 0,
        })
    }

    /// How long the file is (ms), going through its packets without decoding them.
    pub fn length_ms(&mut self) -> f64 {
        let Some(tb) = self.time_base else { return 0.0 };
        let mut ticks = 0i128;
        while let Ok(Some(packet)) = self.format.next_packet() {
            if packet.track_id == self.track_id {
                ticks += packet.dur.get() as i128;
            }
        }
        ticks as f64 * f64::from(tb.numer.get()) / f64::from(tb.denom.get()) * 1000.0
    }

    /// To (or just before) `from_ms`. The first sample's frame is worked out from what comes next.
    pub fn seek(&mut self, from_ms: f64) -> Result<(), String> {
        use symphonia::core::formats::{SeekMode, SeekTo};
        use symphonia::core::units::Time;
        let time = Time::try_from_secs_f64(from_ms.max(0.0) / 1000.0).ok_or("bad seek time")?;
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| format!("seek: {e}"))?;
        self.src_start = None;
        self.tail = 0;
        self.decoder.reset();
        Ok(())
    }

    /// Decodes the next packet, each stereo frame of it going to `frame`. False: the file's over.
    pub fn next(&mut self, frame: &mut dyn FnMut(f32, f32)) -> Result<bool, String> {
        use symphonia::core::errors::Error;
        loop {
            let mut packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(Error::IoError(_)) => return Ok(false),
                Err(e) => return Err(format!("read: {e}")),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let cut = std::mem::take(&mut packet.trim_end).get() as usize;
            let pts = packet.pts.get();
            let buf = match self.decoder.decode(&packet) {
                Ok(b) => b,
                Err(Error::DecodeError(_)) => continue,
                Err(e) => return Err(format!("decode: {e}")),
            };
            // (A packet can ask to cut more than it holds: the rest of the padding, counted again.)
            self.tail = if cut == 0 {
                0
            } else {
                self.tail + cut.min(buf.frames())
            };
            self.src_rate = buf.spec().rate();
            if self.src_start.is_none() {
                // After a seek, a decoder may give back less than the packet's frames (Vorbis's
                // first packet decodes to nothing: it only primes the overlap). The part starts
                // where the output does, not at the packet's timestamp, or it sits a packet
                // (13-23 ms) early.
                let out = buf.frames() as i128;
                if out == 0 {
                    continue;
                }
                let tb = self.time_base.ok_or("no time base")?;
                let src_rate = self.src_rate;
                let to_frames = |t: i128| {
                    t * tb.numer.get() as i128 * src_rate as i128 / tb.denom.get() as i128
                };
                let dur = to_frames(packet.dur.get() as i128);
                let at = to_frames(pts as i128) + (dur - out).max(0);
                self.src_start = Some(at as i64);
            }
            let ch = buf.spec().channels().count().max(1);
            buf.copy_to_vec_interleaved(&mut self.packet_buf);
            for f in self.packet_buf.chunks_exact(ch) {
                let (l, r) = if ch == 1 { (f[0], f[0]) } else { (f[0], f[1]) };
                frame(l, r);
            }
            return Ok(true);
        }
    }
}

pub(crate) fn decode_source(
    source: Box<dyn symphonia::core::io::MediaSource>,
    ext: Option<&str>,
    rate: u32,
    part: Option<(f64, f64)>,
) -> Result<(Track, i64), String> {
    let mut reader = Reader::open(source, ext)?;
    match part {
        Some((from_ms, _)) => reader.seek(from_ms)?,
        None => reader.src_start = Some(0),
    }
    let mut stereo = Pieces::default();
    loop {
        if called_off() {
            return Err(CANCELLED.into());
        }
        if !reader.next(&mut |l, r| stereo.push(l, r))? {
            break;
        }
        if let Some((_, len_ms)) = part
            && stereo.len as f64 / 2.0 >= len_ms / 1000.0 * reader.src_rate as f64
        {
            break;
        }
    }
    let (src_rate, tail) = (reader.src_rate, reader.tail);
    stereo.truncate(stereo.len.saturating_sub(tail * 2));
    let Some(src_start) = reader.src_start.filter(|_| stereo.len > 0 && src_rate != 0) else {
        return Err("audio file decoded to nothing".into());
    };
    if src_rate == rate {
        return Ok((
            Track {
                samples: stereo.into_samples(),
                rate: 1.0,
                stream: None,
            },
            src_start,
        ));
    }
    // Resample on the same grid as a full decode: output frame n reads source position n * ratio.
    let ratio = src_rate as f64 / rate as f64;
    let first = (src_start as f64 / ratio).ceil() as i64;
    let samples = stereo.resampled(ratio, first as f64 * ratio - src_start as f64);
    Ok((
        Track {
            samples,
            rate: 1.0,
            stream: None,
        },
        first,
    ))
}

/// Speeds a track up (`rate` > 1) or slows it down. With `keep_pitch` the music is time-stretched
/// so it sounds like the original, just faster (osu!'s DT/HT); otherwise it is resampled and the
/// pitch moves with the speed (nightcore-style). Runs once at load time.
pub fn change_rate(track: Track, sample_rate: u32, rate: f64, keep_pitch: bool) -> Track {
    if (rate - 1.0).abs() < 1e-6 || track.samples.is_empty() {
        return track;
    }
    rate_changed(&track, sample_rate, rate, keep_pitch)
}

fn rate_changed(track: &Track, sample_rate: u32, rate: f64, keep_pitch: bool) -> Track {
    let samples = if keep_pitch {
        let frames = track.samples.len() / 2;
        time_stretch(
            &track.samples,
            sample_rate,
            |n| n * rate,
            (frames as f64 / rate) as usize,
        )
    } else {
        resample_ratio(&track.samples, rate)
    };
    Track {
        samples,
        rate,
        stream: None,
    }
}

/// `change_rate` with the pitch moved `shift` times further on top, the speed staying: resampling
/// moves both, stretching puts the speed back.
pub fn shifted(track: Track, sample_rate: u32, rate: f64, keep_pitch: bool, shift: f64) -> Track {
    if (shift - 1.0).abs() < 1e-6 {
        return change_rate(track, sample_rate, rate, keep_pitch);
    }
    if track.samples.is_empty() {
        return track;
    }
    let pitch = if keep_pitch { shift } else { rate * shift };
    let moved = resample_ratio(&track.samples, pitch);
    let stretch = rate / pitch;
    let frames = moved.len() / 2;
    Track {
        samples: time_stretch(
            &moved,
            sample_rate,
            |n| n * stretch,
            (frames as f64 / stretch) as usize,
        ),
        rate,
        stream: None,
    }
}

/// Like `change_rate`, for a speed that changes over the song (Wind Up / Down): output (song)
/// time t plays the map at `speed.map(t)`. The track's `rate` is the starting speed.
pub fn change_speed(
    track: Track,
    sample_rate: u32,
    speed: &crate::mods::Speed,
    keep_pitch: bool,
) -> Track {
    if speed.is_constant() || track.samples.is_empty() {
        return change_rate(track, sample_rate, speed.r0, keep_pitch);
    }
    let sr = sample_rate as f64;
    let frames = track.samples.len() / 2;
    // Output frame -> input frame, through map time.
    let in_of = |n: f64| speed.map(n / sr * 1000.0) / 1000.0 * sr;
    let out_frames = (speed.track(frames as f64 / sr * 1000.0) / 1000.0 * sr) as usize;
    let samples = if keep_pitch {
        time_stretch(&track.samples, sample_rate, in_of, out_frames)
    } else {
        resample_by(&track.samples, out_frames, |n| in_of(n as f64))
    };
    Track {
        samples,
        rate: speed.r0,
        stream: None,
    }
}

/// The stretch's search works on a mono copy decimated this many times, refined at full rate.
pub(crate) const DEC: usize = 4;
/// How much of each grain's error from `nominal` the search leans against (see `Grains::lean`).
const LEAN: f64 = 0.15;

/// `input` as the stretch's search wants it: frame pairs averaged to mono, `DEC` frames to a value.
pub(crate) fn coarse_of(input: &[f32]) -> Vec<f32> {
    input
        .chunks(2 * DEC)
        .map(|c| {
            let n = c.len() / 2;
            (0..n).map(|i| 0.5 * (c[2 * i] + c[2 * i + 1])).sum::<f32>() / n as f32
        })
        .collect()
}

/// What a grain can read: some of the song's frames, from frame `base`, up to frame `frames`, and
/// the coarse copy of them from value `coarse_base`. (All of the song, or a stretch of it.)
pub(crate) struct Heard<'a> {
    pub input: &'a [f32],
    pub base: usize,
    pub frames: usize,
    pub coarse: &'a [f32],
    pub coarse_base: usize,
}

impl Heard<'_> {
    fn mono(&self, i: usize) -> f32 {
        let at = (i - self.base) * 2;
        0.5 * (self.input[at] + self.input[at + 1])
    }

    fn coarse_at(&self, i: usize) -> f32 {
        self.coarse[i - self.coarse_base]
    }

    fn coarse_end(&self) -> usize {
        self.coarse_base + self.coarse.len()
    }

    /// How alike two stretches of the coarse copy are, and the second one's energy.
    fn corr(&self, a: usize, b: usize, len: usize) -> (f32, f64) {
        let (mut s, mut e) = (0.0f32, 0.0f64);
        for i in 0..len {
            let v = self.coarse_at(b + i);
            s += self.coarse_at(a + i) * v;
            e += f64::from(v) * f64::from(v);
        }
        (s, e)
    }

    /// The same on the song itself, every other frame.
    fn fine_corr(&self, a: usize, b: usize, len: usize) -> f32 {
        let mut s = 0.0f32;
        let mut i = 0;
        while i < len {
            s += self.mono(a + i) * self.mono(b + i);
            i += 2;
        }
        s
    }
}

/// WSOLA time stretch of interleaved stereo, a grain at a time: overlap-adds Hann-windowed ~40 ms
/// grains, choosing each grain's source position within +-8 ms so its waveform lines up with the
/// previous grain. `in_of` maps an output frame to the input frame that should sound there.
pub(crate) struct Grains {
    pub win: usize,
    pub hop_out: usize,
    pub search: usize,
    window: Vec<f32>,
    prev: Option<usize>,
    lean: f64,
    /// The next grain, and the first there was.
    pub k: usize,
    first: usize,
}

impl Grains {
    pub fn new(sample_rate: u32, first: usize) -> Grains {
        let sr = sample_rate as f64;
        let win = ((sr * 0.040) as usize).max(64) & !1;
        Grains {
            win,
            hop_out: win / 2,
            search: (sr * 0.008) as usize,
            window: (0..win)
                .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / win as f32).cos())
                .collect(),
            prev: None,
            lean: 0.0,
            k: first,
            first,
        }
    }

    /// Where the next grain should start in the input: grains are placed so their centres (where
    /// the Hann window weighs most) land exactly where `in_of` says; anchoring their starts instead
    /// would play the music ~win/2 * (rate - 1) late (about 7 ms at 1.5x), off its notes.
    pub fn nominal(&self, in_of: &dyn Fn(f64) -> f64) -> usize {
        let half = self.win as f64 / 2.0;
        (in_of((self.k * self.hop_out) as f64 + half) - half).max(0.0) as usize
    }

    /// Adds the next grain to `out` (which starts at output frame `out_base`). False: there are no
    /// more, the song or the `out_frames` of output being over.
    pub fn next(
        &mut self,
        heard: &Heard,
        in_of: &dyn Fn(f64) -> f64,
        out: &mut [f32],
        out_base: usize,
        out_frames: usize,
    ) -> bool {
        let (win, hop_out, search) = (self.win, self.hop_out, self.search);
        let frames = heard.frames;
        let nominal = self.nominal(in_of);
        let out_pos = self.k * hop_out;
        if nominal + win + search >= frames || out_pos + win > out_frames {
            return false;
        }
        // Candidates nearest `nominal` first, so ties (silence) keep the grain on time.
        let around = |centre: usize, lo: usize, hi: usize| {
            (0..=(hi - lo))
                .flat_map(move |d| [centre.checked_sub(d), Some(centre + d)])
                .flatten()
                .filter(move |&c| c >= lo && c <= hi)
        };
        let chosen = match self.prev {
            None => nominal,
            Some(p) => {
                // Where the previous grain would naturally continue; find the candidate around
                // `nominal` whose start looks most like it.
                let target = p + hop_out;
                // The search leans against where grains have been landing (see `lean`).
                let centre = (nominal as f64 + self.lean).max(0.0) as usize;
                let lo = centre.saturating_sub(search);
                let hi = centre + search;
                let ov = hop_out;
                let (mut best, mut best_c) = (nominal, f32::MIN);
                let len = ov / DEC;
                let at = target / DEC;
                let coarse_end = heard.coarse_end();
                let loud_at = heard.corr(at, at, len).1.max(1e-12);
                for cand in around(centre / DEC, lo / DEC, hi / DEC) {
                    if (target + ov) / DEC < coarse_end && (cand * DEC + ov) / DEC < coarse_end {
                        let (dot, loud) = heard.corr(at, cand, len);
                        let like = dot / (loud_at * loud.max(1e-12)).sqrt() as f32;
                        if like > best_c {
                            best_c = like;
                            best = cand * DEC;
                        }
                    }
                }
                let (mut fine, mut fine_c) = (best, f32::MIN);
                let fine_lo = best.saturating_sub(DEC);
                for c in around(
                    best,
                    fine_lo,
                    (best + DEC).min(frames - win - 1).max(fine_lo),
                ) {
                    let v = heard.fine_corr(target.min(frames - ov - 1), c, ov);
                    if v > fine_c {
                        fine_c = v;
                        fine = c;
                    }
                }
                fine
            }
        };
        // The search reaches further than the check above allows for (it leans, and goes by the
        // hop, not the window): near the end of the song a grain must still fit.
        let chosen = chosen.min(frames - win);
        let (input, base) = (heard.input, heard.base);
        for (i, &wgt) in self.window[..win].iter().enumerate() {
            let (to, from) = ((out_pos + i - out_base) * 2, (chosen + i - base) * 2);
            out[to] += input[from] * wgt;
            out[to + 1] += input[from + 1] * wgt;
        }
        // Near 1x the continuation sits just behind `nominal` every time, so grains picked by
        // likeness alone walk to the edge of the search and stay there, up to 8 ms off the notes.
        // Leaning the search against that keeps every grain free to fit, and them on time on
        // average, at any rate.
        self.lean = (self.lean - LEAN * (chosen as f64 - nominal as f64))
            .clamp(-(search as f64), search as f64);
        self.prev = Some(chosen);
        self.k += 1;
        true
    }

    /// Evens out output frames `from..to` of `out` (which starts at output frame `out_base`), once
    /// every grain over them is in: what the windows over each add up to (`win` is two hops: two
    /// at most).
    pub fn normalise(&self, out: &mut [f32], out_base: usize, from: usize, to: usize) {
        let (hop_out, win, k) = (self.hop_out, self.win, self.k);
        for i in from..to {
            let last = (i / hop_out).min(k.saturating_sub(1));
            let n: f32 = (last.saturating_sub(1)..=last)
                .filter(|&g| g >= self.first && g < k && g * hop_out <= i && i < g * hop_out + win)
                .map(|g| self.window[i - g * hop_out])
                .sum();
            if n > 1e-3 {
                let at = (i - out_base) * 2;
                out[at] /= n;
                out[at + 1] /= n;
            }
        }
    }
}

/// `Grains` over a whole song, `out_len` frames of it. (Nothing here is a full copy of the song
/// but the output: for a long one that's gigabytes each.)
fn time_stretch(
    input: &[f32],
    sample_rate: u32,
    in_of: impl Fn(f64) -> f64,
    out_len: usize,
) -> Vec<f32> {
    let mut grains = Grains::new(sample_rate, 0);
    let coarse = coarse_of(input);
    let heard = Heard {
        input,
        base: 0,
        frames: input.len() / 2,
        coarse: &coarse,
        coarse_base: 0,
    };
    let out_frames = out_len + grains.win;
    let mut out = vec![0.0f32; out_frames * 2];
    loop {
        if grains.k.is_multiple_of(256) && called_off() {
            return Vec::new();
        }
        if !grains.next(&heard, &in_of, &mut out, 0, out_frames) {
            break;
        }
    }
    let used = (grains.k * grains.hop_out + grains.hop_out).min(out_frames);
    out.truncate(used * 2);
    grains.normalise(&mut out, 0, 0, used);
    out
}

/// Cubic (Catmull-Rom) resampling of interleaved stereo by an arbitrary ratio: output frame n
/// reads input position n * ratio. Done once at load time.
fn resample_ratio(input: &[f32], ratio: f64) -> Vec<f32> {
    resample_from(input, ratio, 0.0)
}

/// Like `resample_ratio`, with output frame n reading input position `phase + n * ratio`.
fn resample_from(input: &[f32], ratio: f64, phase: f64) -> Vec<f32> {
    let frames_out = ((input.len() / 2) as f64 - phase) / ratio;
    resample_by(input, frames_out.floor().max(0.0) as usize, |n| {
        phase + n as f64 * ratio
    })
}

/// Cubic resampling where output frame n reads input position `pos(n)`.
fn resample_by(input: &[f32], frames_out: usize, pos: impl Fn(usize) -> f64) -> Vec<f32> {
    let frames_in = input.len() / 2;
    let mut out = Vec::with_capacity(frames_out * 2);
    let at = |i: isize, c: usize| -> f32 {
        let i = i.clamp(0, frames_in as isize - 1) as usize;
        input[i * 2 + c]
    };
    for n in 0..frames_out {
        let pos = pos(n);
        let i = pos.floor() as isize;
        let t = (pos - i as f64) as f32;
        for c in 0..2 {
            out.push(cubic(t, at(i - 1, c), at(i, c), at(i + 1, c), at(i + 2, c)));
        }
    }
    out
}

/// Catmull-Rom between `p1` and `p2`.
#[inline]
pub(crate) fn cubic(t: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
    p1 + 0.5
        * t
        * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

#[cfg(test)]
mod stretch_end_tests {
    use super::*;

    /// Random music that never settles, so grains are free to land anywhere the search lets them.
    fn noise(frames: usize, mut seed: u32) -> Vec<f32> {
        (0..frames * 2)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / 8_388_608.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn a_grain_never_reads_past_the_end_of_the_song() {
        let rate = 48_000;
        for speed in [0.75, 0.9, 1.1, 1.25, 1.5, 2.0] {
            for seed in 0..40u32 {
                let frames = 30_000 + seed as usize * 997;
                let samples = noise(frames, seed);
                let out_frames = (frames as f64 / speed) as usize;
                let _ = time_stretch(&samples, rate, |t| t * speed, out_frames);
            }
        }
    }
}
