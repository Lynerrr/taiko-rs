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
    /// Playback rate the samples were prepared for (1.0 = original speed). One second of this
    /// track covers `rate` seconds of the beatmap.
    pub rate: f64,
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
    // cpal only has a PipeWire host on Linux; elsewhere (WASAPI on Windows) the default host is right.
    #[cfg(all(feature = "pipewire", target_os = "linux"))]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, seconds: f32, sr: u32) -> Vec<f32> {
        let n = (seconds * sr as f32) as usize;
        (0..n).flat_map(|i| {
            let v = (i as f32 / sr as f32 * freq * std::f32::consts::TAU).sin() * 0.5;
            [v, v]
        }).collect()
    }

    /// Dominant frequency from upward zero crossings of the left channel.
    fn frequency(samples: &[f32], sr: u32) -> f32 {
        let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
        let crossings = left.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        crossings as f32 / (left.len() as f32 / sr as f32)
    }

    #[test]
    fn keep_pitch_changes_length_not_pitch() {
        let sr = 48000;
        let input = Track { samples: sine(440.0, 4.0, sr), rate: 1.0 };
        for rate in [0.75, 1.5, 2.0] {
            let out = change_rate(Track { samples: input.samples.clone(), rate: 1.0 }, sr, rate, true);
            let len_ratio = input.frames() as f64 / out.frames() as f64;
            assert!((len_ratio - rate).abs() < 0.03, "rate {rate}: length ratio {len_ratio}");
            let f = frequency(&out.samples, sr);
            assert!((f - 440.0).abs() < 8.0, "rate {rate}: pitch moved to {f} Hz");
            assert_eq!(out.rate, rate);
        }
    }

    #[test]
    fn without_keep_pitch_pitch_follows_speed() {
        let sr = 48000;
        let out = change_rate(Track { samples: sine(440.0, 2.0, sr), rate: 1.0 }, sr, 1.5, false);
        let f = frequency(&out.samples, sr);
        assert!((f - 660.0).abs() < 8.0, "expected ~660 Hz, got {f}");
    }
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
    Track { samples, rate: 1.0 }
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
    Ok(Track { samples, rate: 1.0 })
}

/// Speeds a track up (`rate` > 1) or slows it down. With `keep_pitch` the music is time-stretched
/// so it sounds like the original, just faster (osu!'s DT/HT); otherwise it is resampled and the
/// pitch moves with the speed (nightcore-style). Runs once at load time.
pub fn change_rate(track: Track, sample_rate: u32, rate: f64, keep_pitch: bool) -> Track {
    if (rate - 1.0).abs() < 1e-6 || track.samples.is_empty() {
        return track;
    }
    let samples = if keep_pitch { time_stretch(&track.samples, sample_rate, rate) } else { resample_ratio(&track.samples, rate) };
    Track { samples, rate }
}

/// WSOLA time stretch of interleaved stereo: overlap-adds Hann-windowed ~40 ms grains, choosing
/// each grain's source position within +-8 ms so its waveform lines up with the previous grain.
fn time_stretch(input: &[f32], sample_rate: u32, rate: f64) -> Vec<f32> {
    let frames = input.len() / 2;
    let sr = sample_rate as f64;
    let win = ((sr * 0.040) as usize).max(64) & !1;
    let hop_out = win / 2;
    let hop_in = hop_out as f64 * rate;
    let search = (sr * 0.008) as usize;
    let window: Vec<f32> = (0..win).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / win as f32).cos()).collect();
    // Alignment is searched on a 4x-decimated mono copy (then refined at full rate) to stay fast.
    const DEC: usize = 4;
    let mono: Vec<f32> = (0..frames).map(|i| 0.5 * (input[2 * i] + input[2 * i + 1])).collect();
    let coarse: Vec<f32> = mono.chunks(DEC).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let corr = |sig: &[f32], a: usize, b: usize, len: usize, step: usize| -> f32 {
        let mut s = 0.0f32;
        let mut i = 0;
        while i < len {
            s += sig[a + i] * sig[b + i];
            i += step;
        }
        s
    };

    let out_frames = (frames as f64 / rate) as usize + win;
    let mut out = vec![0.0f32; out_frames * 2];
    let mut norm = vec![0.0f32; out_frames];
    let mut prev: Option<usize> = None;
    let mut k = 0usize;
    loop {
        let nominal = (k as f64 * hop_in) as usize;
        let out_pos = k * hop_out;
        if nominal + win + search >= frames || out_pos + win > out_frames {
            break;
        }
        let chosen = match prev {
            None => nominal,
            Some(p) => {
                // Where the previous grain would naturally continue; find the candidate around
                // `nominal` whose start looks most like it.
                let target = p + hop_out;
                let lo = nominal.saturating_sub(search);
                let hi = nominal + search;
                let ov = hop_out;
                let (mut best, mut best_c) = (nominal, f32::MIN);
                let mut cand = lo / DEC;
                while cand * DEC <= hi {
                    if (target + ov) / DEC < coarse.len() && (cand * DEC + ov) / DEC < coarse.len() {
                        let c = corr(&coarse, target / DEC, cand, ov / DEC, 1);
                        if c > best_c {
                            best_c = c;
                            best = cand * DEC;
                        }
                    }
                    cand += 1;
                }
                let (mut fine, mut fine_c) = (best, f32::MIN);
                for c in best.saturating_sub(DEC)..=(best + DEC).min(frames - win - 1) {
                    let v = corr(&mono, target.min(frames - ov - 1), c, ov, 2);
                    if v > fine_c {
                        fine_c = v;
                        fine = c;
                    }
                }
                fine
            }
        };
        for i in 0..win {
            let wgt = window[i];
            out[(out_pos + i) * 2] += input[(chosen + i) * 2] * wgt;
            out[(out_pos + i) * 2 + 1] += input[(chosen + i) * 2 + 1] * wgt;
            norm[out_pos + i] += wgt;
        }
        prev = Some(chosen);
        k += 1;
    }
    let used = (k * hop_out + hop_out).min(out_frames);
    out.truncate(used * 2);
    for (i, n) in norm.iter().take(used).enumerate() {
        if *n > 1e-3 {
            out[i * 2] /= n;
            out[i * 2 + 1] /= n;
        }
    }
    out
}

/// Cubic (Catmull-Rom) resampling of interleaved stereo. Done once at load time.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    resample_ratio(input, from as f64 / to as f64)
}

/// Resamples by an arbitrary ratio: output frame n reads input position n * ratio.
fn resample_ratio(input: &[f32], ratio: f64) -> Vec<f32> {
    let frames_in = input.len() / 2;
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
