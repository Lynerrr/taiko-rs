//! Replay videos: an in-game editor (cut, crop, size, frame rate, quality...) and the export,
//! which renders the replay frame by frame offscreen and pipes the frames to ffmpeg together
//! with a WAV of the music and the hitsounds as they were judged.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::time::Instant;

use crate::audio::{self, Sfx, Sounds, Track};
use crate::beatmap::Beatmap;
use crate::game::{self, Game};
use crate::layout::{Area, Grip};
use crate::render::{accent, rgba, Canvas, Hit, VIRTUAL_HEIGHT};
use crate::mods::Speed;
use crate::scores::Replay;
use winit::keyboard::KeyCode;

pub type Loaded = Result<(Beatmap, Arc<Track>), String>;

/// Frame sizes offered (always 16:9; the crop picks the part kept).
pub const SIZES: [(u32, u32); 5] = [(854, 480), (1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)];
pub const FRAME_RATES: [u32; 5] = [30, 60, 120, 144, 240];
/// Quality: (name, x264 CRF); None aims at a file size instead (`Settings::target_mb`).
pub const QUALITIES: [(&str, Option<u32>); 4] = [("best", Some(16)), ("high", Some(19)), ("small file", Some(25)), ("target size", None)];
/// Encoding speed: (name, x264 preset). Slower makes smaller files at the same quality.
pub const SPEEDS: [(&str, &str); 3] = [("fast", "veryfast"), ("balanced", "medium"), ("slow", "slow")];
/// Motion blur: (name, frames blended into each, the share of the frame time they span).
pub const BLURS: [(&str, usize, f64); 3] = [("off", 1, 0.0), ("light", 4, 0.5), ("strong", 8, 1.0)];

/// How a video is exported (all remembered in the config).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub size: usize,
    pub fps: usize,
    pub quality: usize,
    /// The size aimed under with the "target size" quality (MB, 1 MB = 1,000,000 bytes).
    pub target_mb: f64,
    pub speed: usize,
    pub blur: usize,
    pub hud: bool,
    pub hitsounds: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { size: 1, fps: 1, quality: 1, target_mb: 10.0, speed: 0, blur: 0, hud: true, hitsounds: true }
    }
}

impl Settings {
    /// The x264 CRF, or None when aiming at a file size.
    pub fn crf(&self) -> Option<u32> {
        QUALITIES[self.quality].1
    }

    /// Reads a `video_*` config line; false if the key isn't one.
    pub fn parse(&mut self, key: &str, v: &str) -> bool {
        fn pick(names: impl Iterator<Item = String>, v: &str, at: &mut usize) {
            if let Some(i) = names.into_iter().position(|n| n.eq_ignore_ascii_case(v)) {
                *at = i;
            }
        }
        match key {
            "video_size" => pick(SIZES.iter().map(|(w, h)| format!("{w}x{h}")), v, &mut self.size),
            "video_fps" => pick(FRAME_RATES.iter().map(|f| f.to_string()), v, &mut self.fps),
            "video_quality" => pick(QUALITIES.iter().map(|q| q.0.to_string()), v, &mut self.quality),
            "video_target_mb" => self.target_mb = v.parse::<f64>().ok().filter(|m| m.is_finite()).unwrap_or(self.target_mb).clamp(1.0, 4000.0),
            "video_speed" => pick(SPEEDS.iter().map(|s| s.0.to_string()), v, &mut self.speed),
            "video_blur" => pick(BLURS.iter().map(|b| b.0.to_string()), v, &mut self.blur),
            "video_hud" => self.hud = v != "false",
            "video_hitsounds" => self.hitsounds = v != "false",
            _ => return false,
        }
        true
    }

    pub fn config_lines(&self) -> String {
        let (w, h) = SIZES[self.size];
        format!(
            "# replay videos: size, fps, quality (best, high, small file, target size), target size in MB, speed (fast, balanced, slow), motion blur (off, light, strong)\n\
             video_size = {w}x{h}\nvideo_fps = {}\nvideo_quality = {}\nvideo_target_mb = {}\nvideo_speed = {}\nvideo_blur = {}\nvideo_hud = {}\nvideo_hitsounds = {}\n",
            FRAME_RATES[self.fps], QUALITIES[self.quality].0, self.target_mb, SPEEDS[self.speed].0, BLURS[self.blur].0, self.hud, self.hitsounds
        )
    }
}

/// Bit rates (kb/s) for `seconds` of video to fit in `mb`: (video, audio). Some room is left for
/// the MP4's own data; None when it can't fit at any watchable rate.
pub fn target_rates(mb: f64, seconds: f64) -> Option<(u32, u32)> {
    let total = mb * 8000.0 * 0.96 / seconds.max(0.1);
    let audio = match total {
        t if t > 2000.0 => 160.0,
        t if t > 700.0 => 128.0,
        t if t > 300.0 => 96.0,
        _ => 64.0,
    };
    let video = total - audio;
    (video >= 100.0).then_some((video as u32, audio as u32))
}

/// A command for ffmpeg that doesn't open a console window on Windows (the game is a window
/// app there, so a console program would get a window of its own).
fn ffmpeg_command(ffmpeg: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Crop presets: the whole frame, the lane strip, a square and a vertical (9:16) frame, both
/// from the left where the hit target is.
pub fn crop_presets() -> [(&'static str, Area); 4] {
    let (top, h) = game::lane_band();
    let lane = Area { x: 0.0, y: ((top - 30.0) / VIRTUAL_HEIGHT).max(0.0), w: 1.0, h: ((h + 80.0) / VIRTUAL_HEIGHT).min(1.0) };
    [
        ("full", Area::FULL),
        ("lane", lane),
        ("square", Area { x: 0.0, y: 0.0, w: 9.0 / 16.0, h: 1.0 }),
        ("vertical", Area { x: 0.0, y: 0.0, w: (9.0 / 16.0) * (9.0 / 16.0), h: 1.0 }),
    ]
}

/// Virtual width of a 16:9 frame (the playfield is laid out in 480-unit-tall space).
pub const FRAME_VW: f32 = VIRTUAL_HEIGHT * 16.0 / 9.0;

/// A replay game with no audio, at its start.
pub fn replay_game(map: &Beatmap, track: &Arc<Track>, r: &Replay) -> Game {
    let mut g = Game::with_mods(map.clone(), track.clone(), None, crate::mods::Mods::from_list(&r.mods));
    g.replay = Some((r.inputs.clone(), 0));
    g.mash = r.mash();
    g.mute_hits = true;
    g
}

pub struct Editor {
    pub replay: Replay,
    pub rx: Option<Receiver<Loaded>>,
    pub map: Option<(Beatmap, Arc<Track>)>,
    /// The frame shown: the replay at `cursor`.
    pub game: Option<Box<Game>>,
    pub cursor: f64,
    /// Timeline range and the part exported (map ms).
    pub first: f64,
    pub last: f64,
    pub start: f64,
    pub end: f64,
    pub crop: Area,
    pub crop_name: &'static str,
    pub settings: Settings,
    /// Typing the target size (MB).
    pub typing: Option<String>,
    /// Playing the preview: when it started and from where.
    pub playing: Option<(Instant, f64)>,
    pub frame_rect: [f32; 4],
    pub timeline_rect: [f32; 4],
    pub scrubbing: bool,
    /// Dragging the crop: grip, pointer start (px), crop at the start.
    pub grip: Option<(Grip, (f64, f64), Area)>,
}

impl Editor {
    pub fn new(replay: Replay, rx: Receiver<Loaded>, settings: Settings) -> Editor {
        Editor {
            replay,
            rx: Some(rx),
            map: None,
            game: None,
            cursor: 0.0,
            first: 0.0,
            last: 1.0,
            start: 0.0,
            end: 1.0,
            crop: Area::FULL,
            crop_name: "full",
            settings,
            typing: None,
            playing: None,
            frame_rect: [0.0; 4],
            timeline_rect: [0.0; 4],
            scrubbing: false,
            grip: None,
        }
    }

    /// Takes the loaded map once it's ready; true if the editor can be used.
    pub fn poll(&mut self) -> Result<bool, String> {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(Ok((map, track))) => {
                    let g = replay_game(&map, &track, &self.replay);
                    self.first = g.time;
                    self.last = g.end_time() + 1500.0;
                    self.start = self.first;
                    self.end = self.last;
                    self.cursor = self.first.max(map.notes.first().map(|n| n.time - 2000.0).unwrap_or(0.0));
                    self.map = Some((map, track));
                    self.rx = None;
                    self.seek(self.cursor);
                }
                Ok(Err(e)) => return Err(e),
                Err(mpsc::TryRecvError::Empty) => return Ok(false),
                Err(mpsc::TryRecvError::Disconnected) => return Err("loading stopped".into()),
            }
        }
        Ok(self.map.is_some())
    }

    /// Shows the replay at map time `t` (rebuilding it to go backwards).
    pub fn seek(&mut self, t: f64) {
        let Some((map, track)) = &self.map else { return };
        let t = t.clamp(self.first, self.last);
        let behind = self.game.as_ref().is_none_or(|g| t < g.time);
        if behind {
            self.game = Some(Box::new(replay_game(map, track, &self.replay)));
        }
        if let Some(g) = &mut self.game {
            g.set_time(t);
        }
        self.cursor = t;
    }

    pub fn time_at(&self, x: f32) -> f64 {
        let [tx, _, tw, _] = self.timeline_rect;
        let f = ((x - tx) / tw.max(1.0)).clamp(0.0, 1.0) as f64;
        self.first + f * (self.last - self.first)
    }

    pub fn in_timeline(&self, x: f32, y: f32) -> bool {
        let [tx, ty, tw, th] = self.timeline_rect;
        x >= tx - 4.0 && x <= tx + tw + 4.0 && y >= ty - 6.0 && y <= ty + th + 6.0
    }

    /// The pixel crop rect of the chosen frame size, with even sides (H.264 needs them).
    pub fn crop_px(&self) -> (u32, u32, u32, u32) {
        let (w, h) = SIZES[self.settings.size];
        let even = |v: f32| ((v.round() as u32) / 2 * 2).max(2);
        let cw = even(self.crop.w * w as f32).min(w);
        let ch = even(self.crop.h * h as f32).min(h);
        let cx = ((self.crop.x * w as f32).round() as u32).min(w - cw);
        let cy = ((self.crop.y * h as f32).round() as u32).min(h - ch);
        (cw, ch, cx, cy)
    }

    /// Seconds of video the cut makes.
    pub fn seconds(&self) -> f64 {
        (self.end - self.start) / 1000.0 / self.replay.rate
    }

    /// `saved`: a video was exported from this editor (offers to show it in its folder).
    /// `exporting`: what the export is doing and how far along it is.
    pub fn draw(&mut self, c: &mut Canvas, scroll_speed: f64, exporting: Option<(String, f32)>, status: &str, saved: bool) {
        let w = c.width;
        let label = &self.replay.map_label;
        let title = format!("VIDEO  -  {label}  -  replay of {}", self.replay.player);
        // Below the FPS overlay in the corner.
        c.text_centered(w / 2.0, 23.0, 9.0, rgba(0xffffff, 1.0), &Canvas::fit(9.0, w - 32.0, &title));
        let fw = (w - 48.0).min(480.0);
        let fh = fw * 9.0 / 16.0;
        let (fx, fy) = ((w - fw) / 2.0, 38.0);
        self.frame_rect = [fx, fy, fw, fh];
        c.rect(fx, fy, fw, fh, rgba(0x0c0c12, 0.6));
        if let Some(g) = &mut self.game {
            let start = c.shapes.len();
            let full = c.width;
            c.width = FRAME_VW;
            g.draw_playfield(c, scroll_speed);
            if self.settings.hud {
                g.draw_hud(c);
            }
            c.width = full;
            let k = fw / FRAME_VW;
            c.transform_since(start, [0.0, 0.0], k, fx, fy);
            c.clip_since(start, fx, fy, fw, fh);
        } else {
            c.text_centered(w / 2.0, fy + fh / 2.0, 10.0, rgba(0x9a9ab0, 1.0), "loading the map...");
        }
        // The crop: outside darkened, border and size.
        let a = self.crop;
        let (cx, cy, cw, ch) = (fx + a.x * fw, fy + a.y * fh, a.w * fw, a.h * fh);
        let shade = rgba(0x000000, 0.55);
        c.rect(fx, fy, fw, cy - fy, shade);
        c.rect(fx, cy + ch, fw, fy + fh - cy - ch, shade);
        c.rect(fx, cy, cx - fx, ch, shade);
        c.rect(cx + cw, cy, fx + fw - cx - cw, ch, shade);
        c.round_rect(cx, cy, cw, ch, 2.0, rgba(0, 0.0), rgba(0x4fb4ff, 1.0), 1.5);
        let (pw, ph, _, _) = self.crop_px();
        c.text(cx + 6.0, cy + 5.0, 7.0, rgba(0x4fb4ff, 1.0), &format!("{pw}x{ph}  (drag to move, edges to resize)"));
        c.round_rect(fx, fy, fw, fh, 2.0, rgba(0, 0.0), rgba(0x3a3a4e, 1.0), 1.0);

        // Timeline: the exported part highlighted, its ends, and the cursor.
        let (tx, ty, tw, th) = (fx, fy + fh + 12.0, fw, 14.0);
        self.timeline_rect = [tx, ty, tw, th];
        let span = (self.last - self.first).max(1.0);
        let x_of = |t: f64| tx + (((t - self.first) / span).clamp(0.0, 1.0) as f32) * tw;
        c.round_rect(tx, ty, tw, th, 4.0, rgba(0x15151d, 0.95), rgba(0x2a2a38, 1.0), 1.0);
        let (sx, ex) = (x_of(self.start), x_of(self.end));
        c.rect(sx, ty + 1.0, (ex - sx).max(1.0), th - 2.0, rgba(accent(), 0.35));
        c.rect(sx - 1.0, ty - 3.0, 2.0, th + 6.0, rgba(0x86E05A, 1.0));
        c.rect(ex - 1.0, ty - 3.0, 2.0, th + 6.0, rgba(0xFF4B4B, 1.0));
        let cx = x_of(self.cursor);
        c.rect(cx - 0.75, ty - 4.0, 1.5, th + 8.0, rgba(0xffffff, 1.0));
        let t = |ms: f64| {
            let s = (ms.max(0.0) / 1000.0) as u64;
            format!("{}:{:02}", s / 60, s % 60)
        };
        let times = format!("{}   in {}   out {}   ({} s)", t(self.cursor), t(self.start), t(self.end), self.seconds().round());
        let times_w = c.text(tx, ty + th + 4.0, 7.0, rgba(0xc8c8d8, 1.0), &times);
        let hint = Canvas::fit(7.0, tw - times_w - 12.0, "click / drag the timeline   Left/Right 1 s (Shift 5 s)");
        c.text_right(tx + tw, ty + th + 4.0, 7.0, rgba(0x8a8aa0, 1.0), &hint);

        // Settings and actions. Shift+key steps back through a setting's choices.
        let s = self.settings;
        let (sw, sh) = SIZES[s.size];
        let play = if self.playing.is_some() { "Pause  (Space)" } else { "Play  (Space)" };
        let crop = format!("Crop: {}  (C)", self.crop_name);
        let size = format!("Size: {sw}x{sh}  (Z)");
        let fps = format!("{} fps  (F)", FRAME_RATES[s.fps]);
        let blur = format!("Motion blur: {}  (B)", BLURS[s.blur].0);
        let quality = match (&self.typing, s.crf()) {
            (Some(text), _) => format!("Target: {text}_ MB  (Enter)"),
            (None, None) => format!("Target: {} MB  (T)", s.target_mb),
            (None, Some(_)) => format!("Quality: {}  (Q)", QUALITIES[s.quality].0),
        };
        let quality_key = if s.crf().is_none() { KeyCode::KeyT } else { KeyCode::KeyQ };
        let speed = format!("Encode: {}  (E)", SPEEDS[s.speed].0);
        let hud = format!("HUD: {}  (U)", if s.hud { "on" } else { "off" });
        let hits = format!("Hitsounds: {}  (H)", if s.hitsounds { "on" } else { "off" });
        let y = ty + th + 18.0;
        c.button_row(y, &[
            (play, Hit::Key(KeyCode::Space), false),
            ("Set in  (I)", Hit::Key(KeyCode::KeyI), false),
            ("Set out  (O)", Hit::Key(KeyCode::KeyO), false),
        ]);
        c.button_row(y + 24.0, &[
            (&crop, Hit::Key(KeyCode::KeyC), false),
            (&size, Hit::Key(KeyCode::KeyZ), false),
            (&fps, Hit::Key(KeyCode::KeyF), false),
            (&blur, Hit::Key(KeyCode::KeyB), false),
        ]);
        c.button_row(y + 48.0, &[
            (&quality, Hit::Key(quality_key), self.typing.is_some()),
            (&speed, Hit::Key(KeyCode::KeyE), false),
            (&hud, Hit::Key(KeyCode::KeyU), false),
            (&hits, Hit::Key(KeyCode::KeyH), false),
        ]);
        let mut actions = vec![("Export MP4  (Enter)", Hit::Key(KeyCode::Enter), true)];
        if saved {
            actions.push(("Show in folder  (S)", Hit::Key(KeyCode::KeyS), false));
        }
        actions.push(("Back  (Esc)", Hit::Key(KeyCode::Escape), false));
        c.button_row(y + 72.0, &actions);
        // What the settings come to, unless there's something to say.
        let (line, col) = if !status.is_empty() {
            (status.to_string(), 0xFFD34D)
        } else if self.typing.is_some() {
            ("type the size in MB   Enter set   Esc cancel   Q other qualities".to_string(), 0x9a9ab0)
        } else if s.crf().is_none() {
            match target_rates(s.target_mb, self.seconds()) {
                Some((v, a)) => (format!("aims under {} MB: video {:.2} Mb/s, audio {a} kb/s, encoded in two passes   Q: other qualities", s.target_mb, v as f64 / 1000.0), 0x9a9ab0),
                None => (format!("{} s can't fit in {} MB: cut it shorter or allow more", self.seconds().round(), s.target_mb), 0xFF4B4B),
            }
        } else {
            let blur = if s.blur > 0 { format!(", {} frames blended into each", BLURS[s.blur].1) } else { String::new() };
            (format!("x264 CRF {}{blur}   Q: quality or a target size (Discord, ...)", s.crf().unwrap_or(0)), 0x9a9ab0)
        };
        c.text_centered(w / 2.0, y + 96.0, 7.0, rgba(col, 1.0), &Canvas::fit(7.0, w - 32.0, &line));
        if let Some((what, p)) = exporting {
            c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x000000, 0.6));
            let (bw, bx, by) = (360.0, (w - 360.0) / 2.0, 220.0);
            c.text_centered(w / 2.0, by - 22.0, 11.0, rgba(0xffffff, 1.0), &format!("{what}  {:.0}%", p * 100.0));
            c.round_rect(bx, by, bw, 12.0, 6.0, rgba(0x15151d, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
            c.round_rect(bx + 1.0, by + 1.0, (bw - 2.0) * p.clamp(0.0, 1.0), 10.0, 5.0, rgba(accent(), 1.0), rgba(0, 0.0), 0.0);
            c.text_centered(w / 2.0, by + 22.0, 7.0, rgba(0x9a9ab0, 1.0), "Esc: cancel");
        }
    }
}

/// ffmpeg on the PATH, or next to taiko.rs's data.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let mut candidates = vec![PathBuf::from(exe)];
    if let Some(dir) = crate::config::user_dir(false) {
        candidates.push(dir.join(exe));
    }
    candidates.into_iter().find(|c| ffmpeg_command(c).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()))
}

/// The music of [start, end] (map ms) with the hitsounds mixed in, as 16-bit stereo at `sr`.
#[allow(clippy::too_many_arguments)]
pub fn mix_audio(track: &Track, sr: u32, speed: &Speed, start: f64, end: f64, music: f32, sfx: &[(f64, Sfx, f32)], sounds: &Sounds, fx: f32) -> Vec<i16> {
    // Frame 0 is map time 0; the speed may change along the song (Wind Up / Down).
    let frame_of = |map_ms: f64| (speed.track(map_ms) / 1000.0 * sr as f64).round() as i64;
    let (f0, f1) = (frame_of(start), frame_of(end));
    let n = (f1 - f0).max(0) as usize;
    let mut out = vec![0f32; n * 2];
    for i in 0..n {
        let src = f0 + i as i64;
        if src >= 0 && (src as usize) < track.frames() {
            out[i * 2] = track.samples[src as usize * 2] * music;
            out[i * 2 + 1] = track.samples[src as usize * 2 + 1] * music;
        }
    }
    // As the game plays them: a sound rings `SFX_CONCURRENCY` times at most, each cut off (fading)
    // when the play that many after it starts.
    let mut order: Vec<usize> = (0..sfx.len()).collect();
    order.sort_by(|&a, &b| sfx[a].0.total_cmp(&sfx[b].0));
    for (p, &e) in order.iter().enumerate() {
        let (t, s, volume) = sfx[e];
        let at = frame_of(t) - f0;
        let cut = order[p + 1..].iter().filter(|&&o| sfx[o].1 == s).nth(audio::SFX_CONCURRENCY - 1).map(|&o| frame_of(sfx[o].0) - f0 - at);
        let sound = &sounds[s as usize];
        for (k, v) in sound.iter().enumerate() {
            let fade = match cut {
                Some(c) if k as i64 >= c => 1.0 - (k as i64 - c) as f32 / audio::CHOKE_FRAMES as f32,
                _ => 1.0,
            };
            if fade <= 0.0 {
                break;
            }
            let i = at + k as i64;
            if i >= 0 && (i as usize) < n {
                out[i as usize * 2] += v * fx * volume * fade;
                out[i as usize * 2 + 1] += v * fx * volume * fade;
            }
        }
    }
    let mut limiter = audio::Limiter::new(sr);
    out.chunks(2).flat_map(|f| {
        let (l, r) = limiter.apply(f[0], f[1]);
        [(l * 32767.0) as i16, (r * 32767.0) as i16]
    }).collect()
}

pub fn write_wav(path: &Path, samples: &[i16], sr: u32) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data = (samples.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&sr.to_le_bytes())?;
    f.write_all(&(sr * 4).to_le_bytes())?;
    f.write_all(&4u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}

/// Feeds RGBA frames to ffmpeg, which makes the MP4: straight away at a quality, or for a
/// target size into a near-lossless file first, then twice more (x264's two passes) to fit.
pub struct Encoder {
    pub tx: Option<SyncSender<Vec<u8>>>,
    pub done: Receiver<Result<PathBuf, String>>,
    /// 0 while frames come in, then the pass running (1, 2), and how far it is (per mille).
    pub pass: Arc<AtomicU32>,
    pub pass_progress: Arc<AtomicU32>,
    /// Set to stop: whatever is running ends and the files are removed.
    pub cancel: Arc<AtomicBool>,
}

/// What `start_encoder` makes.
pub struct Job {
    pub size: (u32, u32),
    pub fps: u32,
    /// The pixel crop (w, h, x, y).
    pub crop: (u32, u32, u32, u32),
    pub settings: Settings,
    pub seconds: f64,
    pub wav: PathBuf,
    pub out: PathBuf,
}

pub fn start_encoder(ffmpeg: &Path, job: Job) -> Result<Encoder, String> {
    let Job { size: (w, h), fps, crop: (cw, ch, cx, cy), settings, seconds, wav, out } = job;
    let preset = SPEEDS[settings.speed].1;
    let target = match settings.crf() {
        Some(_) => None,
        None => Some(target_rates(settings.target_mb, seconds).ok_or_else(|| format!("{} s of video can't fit in {} MB", seconds.round(), settings.target_mb))?),
    };
    // At a target size the frames go into a near-lossless file for the two passes to read.
    let first = out.with_file_name(format!(".video-{}.mkv", std::process::id()));
    let mut cmd = ffmpeg_command(ffmpeg);
    cmd.args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"]).arg(format!("{w}x{h}")).arg("-r").arg(fps.to_string()).args(["-i", "-"]);
    // Inputs first: an option before an -i is that input's.
    if settings.crf().is_some() {
        cmd.arg("-i").arg(&wav).args(["-map", "0:v", "-map", "1:a"]);
    }
    cmd.arg("-vf").arg(format!("crop={cw}:{ch}:{cx}:{cy}"));
    match settings.crf() {
        Some(crf) => {
            cmd.args(["-c:v", "libx264", "-preset", preset, "-crf"]).arg(crf.to_string());
            cmd.args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart"]).arg(&out);
        }
        None => {
            cmd.args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "10", "-pix_fmt", "yuv420p"]).arg(&first);
        }
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("can't run ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no ffmpeg input")?;
    // A few frames of slack: rendering waits for ffmpeg rather than piling frames up.
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(6);
    let (done_tx, done) = mpsc::channel();
    let (pass, pass_progress, cancel) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)), Arc::new(AtomicBool::new(false)));
    let ffmpeg = ffmpeg.to_path_buf();
    let (stage, progress, stop) = (pass.clone(), pass_progress.clone(), cancel.clone());
    std::thread::spawn(move || {
        let mut failed = None;
        for frame in rx {
            if let Err(e) = stdin.write_all(&frame) {
                failed = Some(e.to_string());
                break;
            }
        }
        drop(stdin);
        let mut result = match (failed, child.wait_with_output()) {
            (_, Ok(o)) if o.status.success() => Ok(out.clone()),
            (_, Ok(o)) => Err(last_line(&o.stderr)),
            (Some(e), Err(_)) => Err(e),
            (None, Err(e)) => Err(e.to_string()),
        };
        if let (Some(rates), true) = (target, result.is_ok() && !stop.load(Ordering::Relaxed)) {
            let passes = Passes { ffmpeg: &ffmpeg, input: &first, wav: &wav, out: &out, preset, seconds, stage: &stage, progress: &progress, stop: &stop };
            result = passes.run(rates, settings.target_mb);
        }
        if stop.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(&out);
            result = Err("cancelled".into());
        }
        for f in [&wav, &first] {
            let _ = std::fs::remove_file(f);
        }
        let _ = done_tx.send(result);
    });
    Ok(Encoder { tx: Some(tx), done, pass, pass_progress, cancel })
}

fn last_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr).lines().last().unwrap_or("ffmpeg failed").to_string()
}

/// x264's two passes from the near-lossless file to `out`, at a bit rate that fits the target.
struct Passes<'a> {
    ffmpeg: &'a Path,
    input: &'a Path,
    wav: &'a Path,
    out: &'a Path,
    preset: &'a str,
    seconds: f64,
    stage: &'a AtomicU32,
    progress: &'a AtomicU32,
    stop: &'a AtomicBool,
}

impl Passes<'_> {
    fn run(&self, (mut video, audio): (u32, u32), target_mb: f64) -> Result<PathBuf, String> {
        let log = self.input.with_extension("passlog");
        let result = (|| {
            self.pass(1, video, audio, &log)?;
            self.pass(2, video, audio, &log)?;
            // x264 lands within a few percent; past the target, once more a little lower.
            let bytes = std::fs::metadata(self.out).map(|m| m.len() as f64).unwrap_or(0.0);
            if bytes > target_mb * 1e6 {
                video = (video as f64 * target_mb * 1e6 / bytes * 0.97) as u32;
                self.pass(2, video, audio, &log)?;
            }
            Ok(self.out.to_path_buf())
        })();
        if let Some(dir) = log.parent() {
            let name = log.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if e.file_name().to_string_lossy().starts_with(&name) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        result
    }

    /// One pass; the first only writes x264's notes (no file, no audio).
    fn pass(&self, n: u32, video: u32, audio: u32, log: &Path) -> Result<(), String> {
        if self.stop.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        self.stage.store(n, Ordering::Relaxed);
        self.progress.store(0, Ordering::Relaxed);
        let mut cmd = ffmpeg_command(self.ffmpeg);
        cmd.args(["-y", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-i"]).arg(self.input);
        if n == 2 {
            cmd.arg("-i").arg(self.wav).args(["-map", "0:v", "-map", "1:a"]);
        }
        cmd.args(["-c:v", "libx264", "-preset", self.preset, "-b:v"]).arg(format!("{video}k"));
        cmd.args(["-pass", &n.to_string(), "-passlogfile"]).arg(log).args(["-pix_fmt", "yuv420p"]);
        if n == 1 {
            cmd.args(["-an", "-f", "null", "-"]);
        } else {
            cmd.args(["-c:a", "aac", "-b:a"]).arg(format!("{audio}k")).args(["-shortest", "-movflags", "+faststart"]).arg(self.out);
        }
        let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("can't run ffmpeg: {e}"))?;
        // `-progress` reports how far it got (out_time_us=...) every half a second or so.
        if let Some(stdout) = child.stdout.take() {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if self.stop.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    break;
                }
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                    let done = (us / 1e6 / self.seconds.max(0.1) * 1000.0).clamp(0.0, 1000.0);
                    self.progress.store(done as u32, Ordering::Relaxed);
                }
            }
        }
        let o = child.wait_with_output().map_err(|e| e.to_string())?;
        if self.stop.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if o.status.success() { Ok(()) } else { Err(last_line(&o.stderr)) }
    }
}

/// An export in progress: the replay stepped one frame at a time.
pub struct Export {
    pub game: Box<Game>,
    /// Song ms of the next frame, and song ms per frame (map time is `game.speed().map(t)`).
    pub t: f64,
    pub step: f64,
    /// Song ms of the first frame (motion blur doesn't reach before it).
    pub t0: f64,
    /// Which of the frame's blended views is next.
    pub sub: usize,
    pub settings: Settings,
    pub size: (u32, u32),
    pub encoder: Encoder,
    pub frame: u64,
    pub frames: u64,
}

impl Export {
    /// Song ms of the next view drawn: the frame's own time, or with motion blur one of the
    /// views spread evenly over the shutter, centred on it (so the picture stays on the beat).
    pub fn view_time(&self) -> f64 {
        let (_, views, shutter) = BLURS[self.settings.blur];
        let spread = ((self.sub as f64 + 0.5) / views as f64 - 0.5) * shutter * self.step;
        (self.t + spread).max(self.t0)
    }

    /// Views blended into one frame.
    pub fn views(&self) -> usize {
        BLURS[self.settings.blur].1
    }

    /// Moves on past the view just drawn; true when it finished a frame.
    pub fn advance(&mut self) -> bool {
        self.sub += 1;
        if self.sub < self.views() {
            return false;
        }
        self.sub = 0;
        self.t += self.step;
        self.frame += 1;
        true
    }

    /// What it's doing, and how far along (0 to 1).
    pub fn progress(&self) -> (String, f32) {
        match self.encoder.pass.load(Ordering::Relaxed) {
            0 => ("Exporting...".into(), self.frame as f32 / self.frames.max(1) as f32),
            pass => {
                let what = format!("Fitting it in {} MB: pass {pass} of 2...", self.settings.target_mb);
                (what, self.encoder.pass_progress.load(Ordering::Relaxed) as f32 / 1000.0)
            }
        }
    }
}

/// Adds up views of one frame (`add`) and gives their average (`take`), for motion blur.
#[derive(Default)]
pub struct Blend {
    sum: Vec<u16>,
    views: u16,
}

impl Blend {
    pub fn add(&mut self, px: &[u8]) {
        if self.views == 0 {
            self.sum.clear();
            self.sum.resize(px.len(), 0);
        }
        for (s, &p) in self.sum.iter_mut().zip(px) {
            *s += p as u16;
        }
        self.views += 1;
    }

    pub fn take(&mut self) -> Vec<u8> {
        let n = self.views.max(1);
        self.views = 0;
        self.sum.iter().map(|&s| ((s + n / 2) / n) as u8).collect()
    }
}

/// Where exported videos go.
pub fn videos_dir() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join("videos"))
}

/// A file name for a replay video.
pub fn file_name(r: &Replay) -> String {
    let clean: String = r.map_label.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    format!("{} - {} {}.mp4", clean.trim().chars().take(100).collect::<String>(), r.player, r.date)
}

/// `name` in `dir`, numbered " (2)", " (3)"... past videos already there, so another export
/// of the same replay (smaller, cut differently) doesn't replace the last.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    let stem = name.strip_suffix(".mp4").unwrap_or(name);
    (1..)
        .map(|n| dir.join(if n == 1 { format!("{stem}.mp4") } else { format!("{stem} ({n}).mp4") }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_music_and_hitsounds_in_the_cut() {
        let sr = 1000;
        // 2 s of a constant 0.25 at 1.5x speed: map times 0..3000 ms.
        let track = Track { samples: vec![0.25; 2 * 2000], rate: 1.5 };
        let sounds = [vec![0.5; 10], vec![0.0; 1], vec![0.0; 1], vec![0.0; 1], vec![0.0; 1], vec![0.0; 1], vec![0.0; 1]];
        // Cut 1500..3000 map ms = 1000..2000 track ms; a don at map 2250 ms = track frame 1500,
        // at its timing point's 50% volume (on top of the 0.25 music: 0.25 + 0.5 * 0.5).
        let out = mix_audio(&track, sr, &Speed::constant(1.5), 1500.0, 3000.0, 1.0, &[(2250.0, Sfx::Don, 0.5)], &sounds, 1.0);
        assert_eq!(out.len(), 1000 * 2);
        let at = |i: usize| out[i * 2] as f32 / 32767.0;
        assert!((at(0) - 0.25).abs() < 1e-3);
        assert!((at(500) - 0.5).abs() < 1e-3 && (at(509) - 0.5).abs() < 1e-3);
        assert!((at(510) - 0.25).abs() < 1e-3);
    }

    #[test]
    fn target_size_leaves_room_and_refuses_what_cant_fit() {
        // 10 MB for a minute: 1.28 Mb/s all told, less a little for the MP4 itself.
        let (v, a) = target_rates(10.0, 60.0).unwrap();
        assert!((v + a) as f64 * 60.0 / 8000.0 < 10.0 * 0.97);
        assert!(v > 1000 && a == 128);
        assert!(target_rates(1.0, 600.0).is_none());
    }

    #[test]
    fn settings_round_trip_through_the_config() {
        let s = Settings { size: 4, fps: 3, quality: 3, target_mb: 25.0, speed: 2, blur: 2, hud: false, hitsounds: false };
        let mut back = Settings::default();
        for line in s.config_lines().lines().filter(|l| !l.starts_with('#')) {
            let (k, v) = line.split_once('=').unwrap();
            assert!(back.parse(k.trim(), v.trim()));
        }
        assert_eq!(back, s);
    }

    #[test]
    fn motion_blur_views_spread_around_each_frame_in_order() {
        let (_tx, rx) = mpsc::channel::<Loaded>();
        let (tx2, done) = mpsc::channel();
        drop(tx2);
        let track = Arc::new(Track { samples: vec![0.0; 2 * 48000], rate: 1.0 });
        let map = crate::beatmap::calibration_map(500.0, 0, 8);
        let replay: Replay = serde_json::from_str(r#"{"version":1,"map_hash":1,"map_path":"","map_label":"m","player":"p","date":0,"rate":1.0,"anti_mash":false,"score":0,"accuracy":1.0,"max_combo":0,"great":0,"ok":0,"miss":0,"ur":0.0,"inputs":[]}"#).unwrap();
        drop(rx);
        let encoder = Encoder { tx: None, done, pass: Default::default(), pass_progress: Default::default(), cancel: Default::default() };
        let settings = Settings { blur: 2, fps: 1, ..Default::default() };
        let mut ex = Export { game: Box::new(replay_game(&map, &track, &replay)), t: 1000.0, step: 1000.0 / 60.0, t0: 1000.0, sub: 0, settings, size: (2, 2), encoder, frame: 0, frames: 3 };
        let mut times = Vec::new();
        while ex.frame < 3 {
            times.push(ex.view_time());
            ex.advance();
        }
        assert_eq!(times.len(), 3 * 8);
        assert!(times.windows(2).all(|w| w[0] <= w[1]));
        // The second frame's views are centred on its time.
        let mid: f64 = times[8..16].iter().sum::<f64>() / 8.0;
        assert!((mid - (1000.0 + 1000.0 / 60.0)).abs() < 1e-6);
    }

    #[test]
    fn blending_averages_the_views() {
        let mut b = Blend::default();
        b.add(&[0, 255, 10]);
        b.add(&[255, 255, 20]);
        assert_eq!(b.take(), vec![128, 255, 15]);
        b.add(&[7, 7, 7]);
        assert_eq!(b.take(), vec![7, 7, 7]);
    }

    #[test]
    fn another_export_of_a_replay_gets_its_own_file() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-videos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_path(&dir, "a.mp4"), dir.join("a.mp4"));
        std::fs::write(dir.join("a.mp4"), b"").unwrap();
        assert_eq!(free_path(&dir, "a.mp4"), dir.join("a (2).mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crop_has_even_sides_inside_the_frame() {
        let (_tx, rx2) = mpsc::channel::<Loaded>();
        let replay: Replay = serde_json::from_str(r#"{"version":1,"map_hash":1,"map_path":"","map_label":"m","player":"p","date":0,"rate":1.0,"anti_mash":false,"score":0,"accuracy":1.0,"max_combo":0,"great":0,"ok":0,"miss":0,"ur":0.0,"inputs":[]}"#).unwrap();
        let mut e = Editor::new(replay, rx2, Settings { size: 1, ..Default::default() });
        e.crop = Area { x: 0.333, y: 0.1, w: 0.3337, h: 0.5011 };
        let (cw, ch, cx, cy) = e.crop_px();
        assert!(cw % 2 == 0 && ch % 2 == 0);
        assert!(cx + cw <= 1280 && cy + ch <= 720);
    }
}
