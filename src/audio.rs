//! Audio output. The song is fully decoded into memory and mixed by hand inside the
//! device callback, so the callback itself is the master clock: every callback publishes
//! "song frame N starts playing at instant T" and the game interpolates from that.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SizedSample, StreamConfig};

/// Interleaved stereo samples at the device sample rate.
pub struct Track {
    pub samples: Vec<f32>,
}

impl Track {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Sfx {
    Don,
    Kat,
    BigDon,
    BigKat,
}

enum Cmd {
    Play { track: Arc<Track>, start_frame: i64 },
    Seek(i64),
    Pause(bool),
    Stop,
    Sfx(Sfx),
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
}

pub struct Engine {
    _stream: cpal::Stream,
    tx: Sender<Cmd>,
    garbage: Receiver<Arc<Track>>,
    stamp: Arc<Mutex<Stamp>>,
    volume: Arc<[AtomicU32; 2]>,
    pub rate: u32,
    pub description: String,
}

impl Engine {
    pub fn new(buffer_frames: u32) -> Result<Engine, String> {
        let host = pick_host();
        let device = host.default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let mut config: StreamConfig = supported.config();
        if let cpal::SupportedBufferSize::Range { min, max } = supported.buffer_size() {
            config.buffer_size = cpal::BufferSize::Fixed(buffer_frames.clamp(*min, *max));
        }

        let stamp = Arc::new(Mutex::new(Stamp { playing: false, frame: 0, at: Instant::now(), latency: Duration::ZERO }));
        let volume = Arc::new([AtomicU32::new(0.8f32.to_bits()), AtomicU32::new(1.0f32.to_bits())]);
        let fmt = supported.sample_format();
        let open = |config: StreamConfig| -> Result<(cpal::Stream, Sender<Cmd>, Receiver<Arc<Track>>), String> {
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
        let (stream, tx, garbage) = match open(config.clone()) {
            Ok(v) => v,
            Err(e) if config.buffer_size != cpal::BufferSize::Default => {
                eprintln!("fixed audio buffer rejected ({e}), using device default");
                config.buffer_size = cpal::BufferSize::Default;
                open(config.clone())?
            }
            Err(e) => return Err(e),
        };
        stream.play().map_err(|e| e.to_string())?;

        let description = format!("{:?} {} Hz, {} ch, buffer {:?}", host.id(), rate, channels, config.buffer_size);
        Ok(Engine { _stream: stream, tx, garbage, stamp, volume, rate, description })
    }

    pub fn play(&self, track: Arc<Track>, start_ms: f64) {
        let start_frame = (start_ms / 1000.0 * self.rate as f64).round() as i64;
        let _ = self.tx.send(Cmd::Play { track, start_frame });
    }

    pub fn seek(&self, ms: f64) {
        let _ = self.tx.send(Cmd::Seek((ms / 1000.0 * self.rate as f64).round() as i64));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }

    /// Freezes the song where it is (output goes silent; hitsounds still play).
    pub fn set_paused(&self, paused: bool) {
        let _ = self.tx.send(Cmd::Pause(paused));
    }

    pub fn sfx(&self, s: Sfx) {
        let _ = self.tx.send(Cmd::Sfx(s));
    }

    pub fn set_volume(&self, music: f32, effects: f32) {
        self.volume[0].store(music.to_bits(), Ordering::Relaxed);
        self.volume[1].store(effects.to_bits(), Ordering::Relaxed);
    }

    /// Song position (ms) audible at `now`, or None when nothing is playing yet.
    pub fn position_ms(&self, now: Instant) -> Option<f64> {
        // Free tracks the callback replaced; deallocation stays off the audio thread.
        while self.garbage.try_recv().is_ok() {}
        let s = *self.stamp.lock().unwrap();
        if !s.playing {
            return None;
        }
        let since = now.saturating_duration_since(s.at).as_secs_f64() - s.latency.as_secs_f64();
        Some((s.frame as f64 / self.rate as f64 + since) * 1000.0)
    }
}

fn pick_host() -> cpal::Host {
    #[cfg(feature = "pipewire")]
    if std::env::var_os("TAIKO_ALSA").is_none() {
        if let Ok(host) = cpal::host_from_id(cpal::HostId::PipeWire) {
            if host.default_output_device().is_some() {
                return host;
            }
        }
    }
    cpal::default_host()
}

fn build<T>(device: &cpal::Device, config: StreamConfig, mut mixer: Mixer) -> Result<cpal::Stream, String>
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

struct Voice {
    sound: Sfx,
    pos: usize,
}

struct Mixer {
    rx: Receiver<Cmd>,
    gc: Sender<Arc<Track>>,
    stamp: Arc<Mutex<Stamp>>,
    volume: Arc<[AtomicU32; 2]>,
    channels: usize,
    track: Option<Arc<Track>>,
    frame: i64,
    paused: bool,
    voices: Vec<Voice>,
    sounds: Arc<[Vec<f32>; 4]>,
}

impl Mixer {
    fn new(
        rx: Receiver<Cmd>,
        gc: Sender<Arc<Track>>,
        stamp: Arc<Mutex<Stamp>>,
        volume: Arc<[AtomicU32; 2]>,
        rate: u32,
        channels: usize,
    ) -> Mixer {
        let sounds = Arc::new([synth_don(rate, false), synth_kat(rate, false), synth_don(rate, true), synth_kat(rate, true)]);
        Mixer { rx, gc, stamp, volume, channels, track: None, frame: 0, paused: false, voices: Vec::with_capacity(64), sounds }
    }

    fn render<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T], info: &OutputCallbackInfo) {
        let now = Instant::now();
        {
            while let Ok(cmd) = self.rx.try_recv() {
                match cmd {
                    Cmd::Play { track, start_frame } => {
                        if let Some(old) = self.track.replace(track) {
                            let _ = self.gc.send(old);
                        }
                        self.frame = start_frame;
                        self.paused = false;
                    }
                    Cmd::Seek(f) => self.frame = f,
                    Cmd::Pause(p) => self.paused = p,
                    Cmd::Stop => {
                        if let Some(old) = self.track.take() {
                            let _ = self.gc.send(old);
                        }
                    }
                    Cmd::Sfx(sound) => {
                        if self.voices.len() < self.voices.capacity() {
                            self.voices.push(Voice { sound, pos: 0 });
                        }
                    }
                }
            }
        }

        let ts = info.timestamp();
        let latency = ts.playback.duration_since(ts.callback);
        if let Ok(mut s) = self.stamp.try_lock() {
            *s = Stamp { playing: self.track.is_some() && !self.paused, frame: self.frame, at: now, latency };
        }

        let music_vol = f32::from_bits(self.volume[0].load(Ordering::Relaxed));
        let fx_vol = f32::from_bits(self.volume[1].load(Ordering::Relaxed));
        let ch = self.channels;
        for frame in out.chunks_mut(ch) {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            if let Some(track) = self.track.as_ref().filter(|_| !self.paused) {
                if self.frame >= 0 && (self.frame as usize) < track.frames() {
                    let i = self.frame as usize * 2;
                    l = track.samples[i] * music_vol;
                    r = track.samples[i + 1] * music_vol;
                }
                self.frame += 1;
            }
            for v in &mut self.voices {
                let buf = &self.sounds[v.sound as usize];
                if let Some(s) = buf.get(v.pos) {
                    l += s * fx_vol;
                    r += s * fx_vol;
                }
                v.pos += 1;
            }
            let (l, r) = (soft_clip(l), soft_clip(r));
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
        self.voices.retain(|v| v.pos < sounds[v.sound as usize].len());
    }
}

fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.9 { x } else { x.signum() * (0.9 + 0.1 * ((x.abs() - 0.9) * 10.0).tanh()) }
}

/// Deterministic white noise so the synthesized hitsounds are identical every run.
struct Noise(u32);
impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// Centre hit: a pitched membrane thump with a falling pitch.
fn synth_don(rate: u32, big: bool) -> Vec<f32> {
    let sr = rate as f32;
    let len = (sr * if big { 0.35 } else { 0.22 }) as usize;
    let mut out = Vec::with_capacity(len);
    let mut phase = 0.0f32;
    let mut noise = Noise(0x1234_5678);
    let mut lp = 0.0f32;
    let gain = if big { 0.95 } else { 0.7 };
    for i in 0..len {
        let t = i as f32 / sr;
        let freq = 95.0 + 110.0 * (-t * 38.0).exp();
        phase += freq / sr * std::f32::consts::TAU;
        let body = phase.sin() + 0.25 * (phase * 2.0).sin();
        let env = (-t * if big { 11.0 } else { 16.0 }).exp() * (t * 2000.0).min(1.0);
        lp += (noise.next() - lp) * 0.08;
        let attack = lp * (-t * 120.0).exp() * 2.5;
        out.push((body * env + attack) * gain);
    }
    out
}

/// Rim hit: a short, bright woody click.
fn synth_kat(rate: u32, big: bool) -> Vec<f32> {
    let sr = rate as f32;
    let len = (sr * if big { 0.16 } else { 0.1 }) as usize;
    let mut out = Vec::with_capacity(len);
    let mut noise = Noise(0x9e37_79b9);
    let (mut hp_prev_in, mut hp_out) = (0.0f32, 0.0f32);
    let gain = if big { 0.6 } else { 0.45 };
    for i in 0..len {
        let t = i as f32 / sr;
        let tone = (t * 1250.0 * std::f32::consts::TAU).sin() * 0.7 + (t * 2090.0 * std::f32::consts::TAU).sin() * 0.45;
        let env = (-t * if big { 32.0 } else { 45.0 }).exp() * (t * 3000.0).min(1.0);
        let n = noise.next();
        hp_out = 0.85 * (hp_out + n - hp_prev_in);
        hp_prev_in = n;
        let click = hp_out * (-t * 260.0).exp() * 1.4;
        out.push((tone * env + click) * gain);
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
            let Some(frame) = samples.get_mut((start + i) * 2..(start + i) * 2 + 2) else { break };
            let t = i as f32 / sr as f32;
            let tone = (t * freq * std::f32::consts::TAU).sin() * (-t * 110.0).exp();
            let tick = noise.next() * (-t * 900.0).exp() * 0.6;
            let v = (tone + tick) * gain;
            frame[0] += v;
            frame[1] += v;
        }
    }
    Track { samples }
}

/// Decodes a whole audio file to interleaved stereo at `rate`.
pub fn decode(path: &Path, rate: u32) -> Result<Track, String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("probe: {e}"))?;
    let track = format.default_track(TrackType::Audio).ok_or("no audio track")?;
    let track_id = track.id;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("no audio codec params")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;

    let mut src_rate = 0u32;
    let mut stereo: Vec<f32> = Vec::new();
    let mut packet_buf: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::IoError(_)) => break,
            Err(e) => return Err(format!("read: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode: {e}")),
        };
        src_rate = buf.spec().rate();
        let ch = buf.spec().channels().count().max(1);
        buf.copy_to_vec_interleaved(&mut packet_buf);
        for frame in packet_buf.chunks_exact(ch) {
            let (l, r) = if ch == 1 { (frame[0], frame[0]) } else { (frame[0], frame[1]) };
            stereo.push(l);
            stereo.push(r);
        }
    }
    if stereo.is_empty() || src_rate == 0 {
        return Err("audio file decoded to nothing".into());
    }
    let samples = if src_rate == rate { stereo } else { resample(&stereo, src_rate, rate) };
    Ok(Track { samples })
}

/// Cubic (Catmull-Rom) resampling of interleaved stereo. Done once at load time.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let frames_in = input.len() / 2;
    let ratio = from as f64 / to as f64;
    let frames_out = (frames_in as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(frames_out * 2);
    let at = |i: isize, c: usize| -> f32 {
        let i = i.clamp(0, frames_in as isize - 1) as usize;
        input[i * 2 + c]
    };
    for n in 0..frames_out {
        let pos = n as f64 * ratio;
        let i = pos.floor() as isize;
        let t = (pos - i as f64) as f32;
        for c in 0..2 {
            let (p0, p1, p2, p3) = (at(i - 1, c), at(i, c), at(i + 1, c), at(i + 2, c));
            let v = p1
                + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)));
            out.push(v);
        }
    }
    out
}
