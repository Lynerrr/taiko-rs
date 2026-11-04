//! Long songs, played as they're decoded. Held whole, a song takes about 22 MB a minute, and as
//! much again at another rate: a 3 hour map is gigabytes. So past `LONG_MS` only the file is kept
//! in memory, and a thread decodes and stretches a few seconds ahead of the music, through the
//! very steps a whole song goes through, into a ring the audio callback reads from.
//!
//! The callback never waits. If the ring runs dry it holds the song's position until the music
//! is there again: a dropout, with the notes still on the music after it.

use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::audio::{Grains, Heard, Reader, Track, cubic};
use crate::mods::Speed;

/// Songs from this long are streamed. Shorter ones stay whole: that's every ordinary map, at
/// about 350 MB at most at another rate.
pub const LONG_MS: f64 = 8.0 * 60.0 * 1000.0;
/// A file smaller than this isn't worth looking into: it can't be that long.
const SMALL_FILE: u64 = 4 << 20;
/// Frames the ring holds, a power of two.
const RING: usize = 1 << 20;
/// How far ahead of the music (seconds) the ring is kept filled.
const AHEAD_SECS: f64 = 8.0;
/// Frames produced at a time.
const BLOCK: usize = 4096;
/// Source frames kept back from the end until the file's over: its last packets may cut some.
const HOLD: i64 = 16384;
/// How far before a seek's target to land (ms), for the decoder to settle.
const PREROLL_MS: f64 = 150.0;

/// A song being played as it's decoded: the ring between the thread filling it and the callback.
pub struct Stream {
    /// Samples, as bits: being atomic is what lets the callback read without a lock.
    ring: Box<[AtomicU32]>,
    /// The frames in it are `from..to`.
    from: AtomicI64,
    to: AtomicI64,
    /// The next frame the callback wants.
    head: AtomicI64,
    frames: AtomicI64,
    /// How many times the callback found the ring dry.
    pub stalls: AtomicU64,
}

impl Stream {
    fn new(frames: i64) -> Stream {
        Stream {
            ring: (0..RING * 2).map(|_| AtomicU32::new(0)).collect(),
            from: AtomicI64::new(0),
            to: AtomicI64::new(0),
            head: AtomicI64::new(0),
            frames: AtomicI64::new(frames),
            stalls: AtomicU64::new(0),
        }
    }

    pub fn frames(&self) -> usize {
        self.frames.load(Ordering::Relaxed).max(0) as usize
    }

    /// The callback is at `at`: where the thread should be working.
    pub fn want(&self, at: i64) {
        self.head.store(at.max(0), Ordering::Relaxed);
    }

    /// Copies what's there from frame `at` into `out`, for the callback; how many frames that was.
    pub fn read(&self, at: i64, out: &mut [f32]) -> usize {
        let (from, to) = (
            self.from.load(Ordering::Acquire),
            self.to.load(Ordering::Acquire),
        );
        if at < from || at >= to {
            return 0;
        }
        let n = (out.len() / 2).min((to - at) as usize);
        for i in 0..n {
            let slot = (at as usize + i) & (RING - 1);
            out[i * 2] = f32::from_bits(self.ring[slot * 2].load(Ordering::Relaxed));
            out[i * 2 + 1] = f32::from_bits(self.ring[slot * 2 + 1].load(Ordering::Relaxed));
        }
        self.head.store(at + n as i64, Ordering::Relaxed);
        n
    }

    fn write(&self, samples: &[f32]) {
        let mut to = self.to.load(Ordering::Relaxed);
        for frame in samples.as_chunks::<2>().0 {
            let slot = to as usize & (RING - 1);
            self.ring[slot * 2].store(frame[0].to_bits(), Ordering::Relaxed);
            self.ring[slot * 2 + 1].store(frame[1].to_bits(), Ordering::Relaxed);
            to += 1;
        }
        self.to.store(to, Ordering::Release);
    }

    /// Starts the ring over at frame `at`.
    fn restart(&self, at: i64) {
        self.to.store(at, Ordering::Release);
        self.from.store(at, Ordering::Release);
    }
}

/// What's between the decoded song and what's heard.
enum Change {
    Nothing,
    Stretch,
    Resample,
}

/// Output frame -> frame of the decoded song.
type InOf = Box<dyn Fn(f64) -> f64 + Send>;

/// The steps a whole song goes through, on a stretch of it at a time.
struct Pipeline {
    bytes: Arc<[u8]>,
    ext: Option<String>,
    sample_rate: u32,
    change: Change,
    in_of: InOf,
    /// Output frames there are.
    out_frames: usize,
    reader: Reader,
    /// Source samples from source frame `src_base`.
    src: Vec<f32>,
    src_base: i64,
    src_over: bool,
    /// Frames at the device's rate from `dev_base`, the next being `dev_next`.
    dev: Vec<f32>,
    dev_base: usize,
    dev_next: i64,
    dev_over: bool,
    /// The coarse copy of them for the stretch, from value `coarse_base`.
    coarse: Vec<f32>,
    coarse_base: usize,
    grains: Option<Grains>,
    /// Output being added up, from frame `out_base`; it's final up to `ready`.
    out: Vec<f32>,
    out_base: usize,
    ready: usize,
    /// The next output frame to hand over, and the first wanted.
    next: usize,
    finished: bool,
}

impl Pipeline {
    fn open_reader(bytes: &Arc<[u8]>, ext: Option<&str>) -> Result<Reader, String> {
        Reader::open(Box::new(std::io::Cursor::new(bytes.clone())), ext)
    }

    fn new(
        bytes: Arc<[u8]>,
        ext: Option<String>,
        sample_rate: u32,
        change: Change,
        in_of: InOf,
        out_frames: usize,
    ) -> Result<Pipeline, String> {
        let reader = Self::open_reader(&bytes, ext.as_deref())?;
        Ok(Pipeline {
            bytes,
            ext,
            sample_rate,
            change,
            in_of,
            out_frames,
            reader,
            src: Vec::new(),
            src_base: 0,
            src_over: false,
            dev: Vec::new(),
            dev_base: 0,
            dev_next: 0,
            dev_over: false,
            coarse: Vec::new(),
            coarse_base: 0,
            grains: None,
            out: Vec::new(),
            out_base: 0,
            ready: 0,
            next: 0,
            finished: false,
        })
    }

    /// Source frames per frame at the device's rate, once a packet's said.
    fn ratio(&self) -> f64 {
        self.reader.src_rate as f64 / self.sample_rate as f64
    }

    /// Goes to output frame `at`.
    fn start(&mut self, at: usize) -> Result<(), String> {
        let in_of = &self.in_of;
        let (first_grain, want) = match self.change {
            Change::Stretch => {
                // Two grains early, to warm up on; from the top they're the whole song's.
                let hop = Grains::new(self.sample_rate, 0).hop_out;
                let grains = Grains::new(self.sample_rate, (at / hop).saturating_sub(2));
                let back = grains.search * 3 + grains.win;
                let want = grains.nominal(in_of.as_ref()).saturating_sub(back);
                (Some(grains), want)
            }
            Change::Nothing => (None, at),
            Change::Resample => (None, (in_of(at as f64) as usize).saturating_sub(4)),
        };
        self.grains = first_grain;
        self.src.clear();
        self.dev.clear();
        self.coarse.clear();
        self.out.clear();
        (self.src_over, self.dev_over, self.finished) = (false, false, false);
        self.next = at;
        let aligned = want / crate::audio::DEC * crate::audio::DEC;
        self.seek_source(aligned)?;
        self.dev_base = aligned;
        self.coarse_base = aligned / crate::audio::DEC;
        self.out_base = self.grains.as_ref().map_or(at, |g| g.k * g.hop_out);
        self.ready = self.out_base;
        Ok(())
    }

    /// Reads the file from just before device frame `want`, so that frame can be worked out.
    fn seek_source(&mut self, want: usize) -> Result<(), String> {
        let sr = self.sample_rate as f64;
        let mut back = PREROLL_MS;
        loop {
            let ms = want as f64 / sr * 1000.0 - back;
            self.reader = Self::open_reader(&self.bytes, self.ext.as_deref())?;
            if ms <= 0.0 {
                self.reader.src_start = Some(0);
            } else {
                self.reader.seek(ms)?;
            }
            self.src.clear();
            // Till something's come out of it, and it's known where that sits.
            while self.src.is_empty() {
                let src = &mut self.src;
                let more = self.reader.next(&mut |l, r| {
                    src.push(l);
                    src.push(r);
                })?;
                if !more {
                    break;
                }
            }
            if self.src.is_empty() {
                return Err("nothing to play there".into());
            }
            self.src_base = self.reader.src_start.unwrap_or(0);
            // The first frame on the device's grid that's in what came, as a whole decode has it.
            let ratio = self.ratio();
            let first = (self.src_base as f64 / ratio).ceil() as i64;
            if first <= want as i64 || ms <= 0.0 {
                self.dev_next = first.max(want as i64);
                return Ok(());
            }
            // It landed past where it was asked to: further back.
            back = back * 4.0 + 500.0;
        }
    }

    /// Reads a packet in. False once the file's over.
    fn read_source(&mut self) -> Result<bool, String> {
        if self.src_over {
            return Ok(false);
        }
        let src = &mut self.src;
        let more = self.reader.next(&mut |l, r| {
            src.push(l);
            src.push(r);
        })?;
        if !more {
            self.src_over = true;
            let keep = self.src.len().saturating_sub(self.reader.tail * 2);
            self.src.truncate(keep);
        }
        Ok(more)
    }

    /// Works out frames at the device's rate up to frame `end`, if the song goes that far.
    fn fill_dev(&mut self, end: usize) -> Result<(), String> {
        let dec = crate::audio::DEC;
        while self.dev_base + self.dev.len() / 2 < end && !self.dev_over {
            let ratio = self.ratio();
            let pos = self.dev_next as f64 * ratio;
            let i = pos.floor() as i64;
            let src_end = self.src_base + self.src.len() as i64 / 2;
            if !self.src_over && i + 2 + HOLD >= src_end {
                self.read_source()?;
                continue;
            }
            // As a whole decode has it: the song ends with its last whole frame.
            if self.src_over && pos >= src_end as f64 {
                self.dev_over = true;
                break;
            }
            let t = (pos - i as f64) as f32;
            let (lo, hi) = (self.src_base, src_end - 1);
            let src = &self.src;
            let at = |i: i64, c: usize| src[(i.clamp(lo, hi) - lo) as usize * 2 + c];
            for c in 0..2 {
                let v = cubic(t, at(i - 1, c), at(i, c), at(i + 1, c), at(i + 2, c));
                self.dev.push(v);
            }
            self.dev_next += 1;
            // What's been read past can go, in lumps.
            let behind = i - 2 - self.src_base;
            if behind > 1 << 16 {
                self.src.drain(..behind as usize * 2);
                self.src_base += behind;
            }
        }
        // The coarse copy keeps up, a group at a time (the last may be short).
        let have = self.dev.len() / 2;
        let from_group = self.coarse_base + self.coarse.len();
        let mut group = from_group * dec - self.dev_base;
        while group + dec <= have || (self.dev_over && group < have) {
            let last = (group + dec).min(have);
            self.coarse
                .extend(crate::audio::coarse_of(&self.dev[group * 2..last * 2]));
            group += dec;
        }
        Ok(())
    }

    /// Drops what the music has gone past, keeping frames from `keep` on.
    fn trim_dev(&mut self, keep: usize) {
        let dec = crate::audio::DEC;
        let keep = keep / dec * dec;
        if keep > self.dev_base + (1 << 16) {
            let gone = keep - self.dev_base;
            self.dev.drain(..gone * 2);
            self.dev_base = keep;
            self.coarse.drain(..gone / dec);
            self.coarse_base = keep / dec;
        }
    }

    /// One more grain. False: the song's over.
    fn stretch_grain(&mut self) -> Result<bool, String> {
        let Some(g) = &self.grains else {
            return Ok(false);
        };
        let nominal = g.nominal(self.in_of.as_ref());
        let room = nominal + 2 * g.win + 6 * g.search + g.hop_out;
        let keep = nominal.saturating_sub(4 * g.search + 2 * g.win);
        self.fill_dev(room)?;
        self.trim_dev(keep);
        let Some(g) = &mut self.grains else {
            return Ok(false);
        };
        let (win, hop) = (g.win, g.hop_out);
        let end = (g.k * hop + win - self.out_base) * 2;
        if self.out.len() < end {
            self.out.resize(end, 0.0);
        }
        let heard = Heard {
            input: &self.dev,
            base: self.dev_base,
            frames: self.dev_base + self.dev.len() / 2,
            coarse: &self.coarse,
            coarse_base: self.coarse_base,
        };
        let out_frames = self.out_frames + win;
        let more = g.next(
            &heard,
            self.in_of.as_ref(),
            &mut self.out,
            self.out_base,
            out_frames,
        );
        let ready = if more {
            g.k * hop
        } else {
            (g.k * hop + hop).min(out_frames)
        };
        let end = (ready - self.out_base) * 2;
        if self.out.len() < end {
            self.out.resize(end, 0.0);
        }
        g.normalise(&mut self.out, self.out_base, self.ready, ready);
        self.ready = ready;
        Ok(more)
    }

    /// The next frames of the song into `to`, up to `max`. None more: the song's over.
    fn produce(&mut self, max: usize, to: &mut Vec<f32>) -> Result<(), String> {
        to.clear();
        match self.change {
            Change::Nothing => {
                self.fill_dev(self.next + max)?;
                let from = (self.next - self.dev_base) * 2;
                let end = (from + max * 2).min(self.dev.len());
                to.extend_from_slice(&self.dev[from.min(end)..end]);
                self.next += to.len() / 2;
                self.trim_dev(self.next);
            }
            Change::Resample => {
                for _ in 0..max {
                    let pos = (self.in_of)(self.next as f64);
                    let i = pos.floor() as i64;
                    self.fill_dev(i.max(0) as usize + 3)?;
                    let end = (self.dev_base + self.dev.len() / 2) as i64;
                    if self.next >= self.out_frames || (self.dev_over && pos >= end as f64) {
                        break;
                    }
                    let t = (pos - i as f64) as f32;
                    let (lo, hi) = (self.dev_base as i64, end - 1);
                    let dev = &self.dev;
                    let at = |i: i64, c: usize| dev[(i.clamp(lo, hi) - lo) as usize * 2 + c];
                    for c in 0..2 {
                        to.push(cubic(t, at(i - 1, c), at(i, c), at(i + 1, c), at(i + 2, c)));
                    }
                    self.next += 1;
                    self.trim_dev((i - 2).max(0) as usize);
                }
            }
            Change::Stretch => {
                let mut more = true;
                while more && self.ready < self.next + max {
                    more = self.stretch_grain()?;
                }
                let start = self.next.max(self.out_base);
                let end = self.ready.min(self.next + max);
                if end > start {
                    let from = (start - self.out_base) * 2;
                    to.extend_from_slice(&self.out[from..(end - self.out_base) * 2]);
                    self.next = end;
                }
                let gone = self.next.saturating_sub(self.out_base);
                if gone > 1 << 15 {
                    self.out.drain(..gone * 2);
                    self.out_base += gone;
                }
                if !more && to.is_empty() {
                    self.finished = true;
                }
                return Ok(());
            }
        }
        if to.is_empty() {
            self.finished = true;
        }
        Ok(())
    }
}

/// Keeps the ring filled ahead of the music, until the song's let go of.
fn run(stream: Weak<Stream>, mut pipeline: Pipeline, sample_rate: u32) {
    let ahead = (AHEAD_SECS * sample_rate as f64) as i64;
    let mut block = Vec::with_capacity(BLOCK * 2);
    let mut started = false;
    let (mut told, mut last_told) = (0, Instant::now());
    loop {
        let Some(s) = stream.upgrade() else { return };
        let stalls = s.stalls.load(Ordering::Relaxed);
        if stalls != told && last_told.elapsed() >= Duration::from_secs(1) {
            eprintln!("stream: the music has run dry {stalls} times");
            (told, last_told) = (stalls, Instant::now());
        }
        let head = s.head.load(Ordering::Relaxed);
        let (from, to) = (s.from.load(Ordering::Relaxed), s.to.load(Ordering::Relaxed));
        if !started || head < from || head > to {
            s.restart(head);
            if let Err(e) = pipeline.start(head as usize) {
                eprintln!("stream: {e}");
                s.frames.store(head, Ordering::Relaxed);
                return;
            }
            started = true;
            continue;
        }
        s.from.store(head, Ordering::Release);
        if pipeline.finished || to - head >= ahead {
            drop(s);
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }
        match pipeline.produce(BLOCK, &mut block) {
            Ok(()) => {
                s.write(&block);
                if pipeline.finished {
                    // Now it's known just where the song ends.
                    s.frames
                        .store(s.to.load(Ordering::Relaxed), Ordering::Relaxed);
                }
            }
            Err(e) => {
                eprintln!("stream: {e}");
                s.frames.store(to, Ordering::Relaxed);
                return;
            }
        }
    }
}

/// How `speed` and `keep_pitch` change the song.
fn change_for(speed: Speed, keep_pitch: bool, sample_rate: u32) -> (Change, InOf) {
    let sr = sample_rate as f64;
    if speed.is_constant() {
        let rate = speed.r0;
        let change = match ((rate - 1.0).abs() < 1e-6, keep_pitch) {
            (true, _) => Change::Nothing,
            (false, true) => Change::Stretch,
            (false, false) => Change::Resample,
        };
        return (change, Box::new(move |n| n * rate));
    }
    let change = if keep_pitch {
        Change::Stretch
    } else {
        Change::Resample
    };
    (
        change,
        Box::new(move |n| speed.map(n / sr * 1000.0) / 1000.0 * sr),
    )
}

/// `bytes` (an audio file) as a track played as it's decoded, `len_ms` long.
fn stream_of(
    bytes: Arc<[u8]>,
    ext: Option<String>,
    len_ms: f64,
    sample_rate: u32,
    speed: Speed,
    keep_pitch: bool,
) -> Result<Arc<Track>, String> {
    let sr = sample_rate as f64;
    let out_frames = (speed.track(len_ms) / 1000.0 * sr) as usize;
    let (change, in_of) = change_for(speed, keep_pitch, sample_rate);
    let pipeline = Pipeline::new(bytes, ext, sample_rate, change, in_of, out_frames)?;
    let stream = Arc::new(Stream::new(out_frames as i64));
    let weak = Arc::downgrade(&stream);
    std::thread::Builder::new()
        .name("stream".into())
        .spawn(move || run(weak, pipeline, sample_rate))
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(Track {
        samples: Vec::new(),
        rate: speed.r0,
        stream: Some(stream),
    }))
}

/// The song at `path` as a stream, if it's long enough to want one. None: load it whole.
pub fn open(
    path: &Path,
    sample_rate: u32,
    speed: Speed,
    keep_pitch: bool,
) -> Result<Option<Arc<Track>>, String> {
    if std::fs::metadata(path).map_or(0, |m| m.len()) < SMALL_FILE {
        return Ok(None);
    }
    let bytes: Arc<[u8]> = std::fs::read(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .into();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_string);
    let source = Box::new(std::io::Cursor::new(bytes.clone()));
    let len_ms = Reader::open(source, ext.as_deref())?.length_ms();
    if len_ms < LONG_MS {
        return Ok(None);
    }
    stream_of(bytes, ext, len_ms, sample_rate, speed, keep_pitch).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{change_rate, decode_source};

    /// A WAV of `secs` seconds at `rate`: a tone with clicks on it, not the same twice.
    fn wav(secs: f64, rate: u32) -> Arc<[u8]> {
        let frames = (secs * rate as f64) as usize;
        let mut out = Vec::with_capacity(44 + frames * 4);
        let bytes = (frames * 4) as u32;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + bytes).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&bytes.to_le_bytes());
        for i in 0..frames {
            let t = i as f64 / rate as f64;
            let click = (-(t % 0.37) * 300.0).exp();
            let l = (t * 220.0 * std::f64::consts::TAU).sin() * 0.3 + click * 0.5;
            let r = (t * 331.0 * std::f64::consts::TAU).sin() * 0.3 + click * 0.5;
            out.extend_from_slice(&((l * 30000.0) as i16).to_le_bytes());
            out.extend_from_slice(&((r * 30000.0) as i16).to_le_bytes());
        }
        out.into()
    }

    /// `open` on a real file of `secs` seconds, big enough to be considered.
    fn opened(secs: f64) -> Option<Arc<Track>> {
        let path =
            std::env::temp_dir().join(format!("taiko-stream-{}-{secs}.wav", std::process::id()));
        std::fs::write(&path, &*wav(secs, 8000)).unwrap();
        let track = open(&path, 48000, Speed::constant(1.1), true);
        std::fs::remove_file(&path).unwrap();
        track.unwrap()
    }

    #[test]
    fn a_long_song_holds_a_fixed_amount_however_long_it_is() {
        let track = opened(9.0 * 60.0).expect("9 minutes is long enough to stream");
        assert!(track.samples.is_empty(), "no decoded song in memory");
        let stream = track.stream.as_ref().unwrap();
        assert_eq!(stream.ring.len(), RING * 2);

        // Nothing else keeps it alive: the thread goes when the song does.
        let gone = Arc::downgrade(stream);
        drop(track);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while gone.strong_count() > 0 && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(gone.strong_count(), 0, "the decoding thread let go");
    }

    #[test]
    fn a_short_song_stays_whole() {
        assert!(opened(7.5 * 60.0).is_none());
    }

    fn whole(bytes: &Arc<[u8]>, sr: u32, rate: f64, keep_pitch: bool) -> Vec<f32> {
        let source = Box::new(std::io::Cursor::new(bytes.clone()));
        let (track, _) = decode_source(source, Some("wav"), sr, None).unwrap();
        change_rate(track, sr, rate, keep_pitch).samples
    }

    fn pipeline(
        bytes: &Arc<[u8]>,
        sr: u32,
        rate: f64,
        keep_pitch: bool,
        frames: usize,
    ) -> Pipeline {
        let (change, in_of) = change_for(Speed::constant(rate), keep_pitch, sr);
        Pipeline::new(bytes.clone(), Some("wav".into()), sr, change, in_of, frames).unwrap()
    }

    /// All of the song from output frame `from`.
    fn streamed(p: &mut Pipeline, from: usize) -> Vec<f32> {
        p.start(from).unwrap();
        let (mut all, mut block) = (Vec::new(), Vec::new());
        while !p.finished {
            p.produce(BLOCK, &mut block).unwrap();
            all.extend_from_slice(&block);
        }
        all
    }

    #[test]
    fn a_streamed_song_is_the_song_loaded_whole() {
        let sr = 48000;
        for src in [44100, 48000] {
            let bytes = wav(12.0, src);
            for (rate, keep) in [
                (1.0, true),
                (1.1, true),
                (0.9, true),
                (1.5, true),
                (1.5, false),
            ] {
                let want = whole(&bytes, sr, rate, keep);
                let mut p = pipeline(&bytes, sr, rate, keep, want.len() / 2);
                let got = streamed(&mut p, 0);
                let n = want.len().min(got.len());
                let off = (0..n).filter(|&i| want[i] != got[i]).count();
                assert_eq!(
                    off, 0,
                    "{src} Hz at {rate} (keep {keep}): {off} of {n} samples differ"
                );
                let short = want.len().abs_diff(got.len()) / 2;
                assert!(
                    short < 64,
                    "{src} Hz at {rate}: {} frames vs {}",
                    got.len() / 2,
                    want.len() / 2
                );
            }
        }
    }

    #[test]
    fn a_seek_lands_on_the_song_where_it_should() {
        let sr = 48000;
        let bytes = wav(12.0, 44100);
        for (rate, keep) in [(1.0, true), (1.5, false)] {
            let want = whole(&bytes, sr, rate, keep);
            let mut p = pipeline(&bytes, sr, rate, keep, want.len() / 2);
            for at in [100_000usize, 200_000, 40_000] {
                let got = streamed(&mut p, at);
                let n = got.len().min(want.len() - at * 2).min(40_000);
                let worst = (0..n)
                    .map(|i| (want[at * 2 + i] - got[i]).abs())
                    .fold(0.0, f32::max);
                assert!(worst < 1e-4, "at {at}, rate {rate}: off by {worst}");
            }
        }
    }

    #[test]
    fn a_seek_while_stretching_stays_in_time_with_the_song() {
        let sr = 48000;
        let bytes = wav(12.0, 44100);
        for rate in [1.1, 0.9, 1.5] {
            let want = whole(&bytes, sr, rate, true);
            let mut p = pipeline(&bytes, sr, rate, true, want.len() / 2);
            let at = 150_000usize;
            let got = streamed(&mut p, at);
            let len = 60_000;
            // The lag (frames) at which it matches the whole song best.
            let (mut best, mut best_v) = (0i64, f64::MIN);
            for lag in -480i64..=480 {
                let v: f64 = (2000..len)
                    .map(|i| {
                        let w = ((at as i64 + i as i64 + lag) * 2) as usize;
                        f64::from(got[i * 2]) * f64::from(want[w])
                    })
                    .sum();
                if v > best_v {
                    (best, best_v) = (lag, v);
                }
            }
            assert!(best.abs() <= 240, "at {rate}: {best} frames off");
        }
    }

    #[test]
    fn the_callback_reads_what_was_written_and_nothing_else() {
        let s = Stream::new(1000);
        let mut out = [0.0f32; 8];
        assert_eq!(s.read(0, &mut out), 0, "nothing there yet");
        s.write(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(s.read(0, &mut out), 2);
        assert_eq!(out[..4], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(s.head.load(Ordering::Relaxed), 2);
        assert_eq!(s.read(2, &mut out), 0, "past what's there");
        s.restart(500);
        assert_eq!(s.read(0, &mut out), 0, "gone");
        s.write(&[5.0, 6.0]);
        assert_eq!(s.read(500, &mut out), 1);
        assert_eq!(out[..2], [5.0, 6.0]);
    }
}
