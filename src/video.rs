//! Replay videos: an in-game editor (cut, crop, size, frame rate) and the export, which renders
//! the replay frame by frame offscreen and pipes the frames to ffmpeg together with a WAV of
//! the music and the hitsounds as they were judged.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::time::Instant;

use crate::audio::{Sfx, Sounds, Track};
use crate::beatmap::Beatmap;
use crate::game::{self, Game};
use crate::layout::{Area, Grip};
use crate::render::{rgba, Canvas, Hit, VIRTUAL_HEIGHT};
use crate::mods::Speed;
use crate::scores::Replay;
use winit::keyboard::KeyCode;

pub type Loaded = Result<(Beatmap, Arc<Track>), String>;

/// Frame sizes offered (always 16:9; the crop picks the part kept).
pub const SIZES: [(u32, u32); 3] = [(1280, 720), (1920, 1080), (2560, 1440)];
pub const FRAME_RATES: [u32; 3] = [60, 30, 120];

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
    g.anti_mash = r.anti_mash;
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
    pub size: usize,
    pub fps: usize,
    pub hitsounds: bool,
    /// Playing the preview: when it started and from where.
    pub playing: Option<(Instant, f64)>,
    pub frame_rect: [f32; 4],
    pub timeline_rect: [f32; 4],
    pub scrubbing: bool,
    /// Dragging the crop: grip, pointer start (px), crop at the start.
    pub grip: Option<(Grip, (f64, f64), Area)>,
}

impl Editor {
    pub fn new(replay: Replay, rx: Receiver<Loaded>) -> Editor {
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
            size: 0,
            fps: 0,
            hitsounds: true,
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
        let (w, h) = SIZES[self.size];
        let even = |v: f32| ((v.round() as u32) / 2 * 2).max(2);
        let cw = even(self.crop.w * w as f32).min(w);
        let ch = even(self.crop.h * h as f32).min(h);
        let cx = ((self.crop.x * w as f32).round() as u32).min(w - cw);
        let cy = ((self.crop.y * h as f32).round() as u32).min(h - ch);
        (cw, ch, cx, cy)
    }

    /// `saved`: a video was exported from this editor (offers to show it in its folder).
    pub fn draw(&mut self, c: &mut Canvas, scroll_speed: f64, exporting: Option<f32>, status: &str, saved: bool) {
        let w = c.width;
        let label = &self.replay.map_label;
        c.text_centered(w / 2.0, 12.0, 9.0, rgba(0xffffff, 1.0), &format!("VIDEO  -  {label}  -  replay of {}", self.replay.player));
        let fw = (w - 48.0).min(540.0);
        let fh = fw * 9.0 / 16.0;
        let (fx, fy) = ((w - fw) / 2.0, 28.0);
        self.frame_rect = [fx, fy, fw, fh];
        c.rect(fx, fy, fw, fh, rgba(0x0c0c12, 0.6));
        if let Some(g) = &mut self.game {
            let start = c.shapes.len();
            let full = c.width;
            c.width = FRAME_VW;
            g.draw_playfield(c, scroll_speed);
            g.draw_hud(c);
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
        c.rect(sx, ty + 1.0, (ex - sx).max(1.0), th - 2.0, rgba(0xEB452C, 0.35));
        c.rect(sx - 1.0, ty - 3.0, 2.0, th + 6.0, rgba(0x86E05A, 1.0));
        c.rect(ex - 1.0, ty - 3.0, 2.0, th + 6.0, rgba(0xFF4B4B, 1.0));
        let cx = x_of(self.cursor);
        c.rect(cx - 0.75, ty - 4.0, 1.5, th + 8.0, rgba(0xffffff, 1.0));
        let t = |ms: f64| {
            let s = (ms.max(0.0) / 1000.0) as u64;
            format!("{}:{:02}", s / 60, s % 60)
        };
        c.text(tx, ty + th + 4.0, 7.0, rgba(0xc8c8d8, 1.0), &format!("{}   in {}   out {}   ({} s)", t(self.cursor), t(self.start), t(self.end), ((self.end - self.start) / 1000.0 / self.replay.rate).round()));
        c.text_right(tx + tw, ty + th + 4.0, 7.0, rgba(0x8a8aa0, 1.0), "click / drag the timeline   Left/Right 1 s (Shift 5 s)");

        // Settings and actions.
        let (sw, sh) = SIZES[self.size];
        let play = if self.playing.is_some() { "Pause  (Space)" } else { "Play  (Space)" };
        let crop = format!("Crop: {}  (C)", self.crop_name);
        let size = format!("Size: {sw}x{sh}  (Z)");
        let fps = format!("{} fps  (F)", FRAME_RATES[self.fps]);
        let hits = format!("Hitsounds: {}  (H)", if self.hitsounds { "on" } else { "off" });
        c.button_row(ty + th + 18.0, &[
            (play, Hit::Key(KeyCode::Space), false),
            ("Set in  (I)", Hit::Key(KeyCode::KeyI), false),
            ("Set out  (O)", Hit::Key(KeyCode::KeyO), false),
        ]);
        c.button_row(ty + th + 42.0, &[
            (&crop, Hit::Key(KeyCode::KeyC), false),
            (&size, Hit::Key(KeyCode::KeyZ), false),
            (&fps, Hit::Key(KeyCode::KeyF), false),
            (&hits, Hit::Key(KeyCode::KeyH), false),
        ]);
        let mut actions = vec![("Export MP4  (Enter)", Hit::Key(KeyCode::Enter), true)];
        if saved {
            actions.push(("Show in folder  (S)", Hit::Key(KeyCode::KeyS), false));
        }
        actions.push(("Back  (Esc)", Hit::Key(KeyCode::Escape), false));
        c.button_row(ty + th + 66.0, &actions);
        if !status.is_empty() {
            c.text_centered(w / 2.0, ty + th + 90.0, 7.0, rgba(0xFFD34D, 1.0), status);
        }
        if let Some(p) = exporting {
            c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x000000, 0.6));
            let (bw, bx, by) = (360.0, (w - 360.0) / 2.0, 220.0);
            c.text_centered(w / 2.0, by - 22.0, 11.0, rgba(0xffffff, 1.0), &format!("Exporting...  {:.0}%", p * 100.0));
            c.round_rect(bx, by, bw, 12.0, 6.0, rgba(0x15151d, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
            c.round_rect(bx + 1.0, by + 1.0, (bw - 2.0) * p.clamp(0.0, 1.0), 10.0, 5.0, rgba(0xEB452C, 1.0), rgba(0, 0.0), 0.0);
            c.text_centered(w / 2.0, by + 22.0, 7.0, rgba(0x9a9ab0, 1.0), "Esc: cancel");
        }
    }
}

/// ffmpeg on the PATH, or next to taiko-rs's data.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let mut candidates = vec![PathBuf::from(exe)];
    if let Some(dir) = crate::config::user_dir(false) {
        candidates.push(dir.join(exe));
    }
    candidates.into_iter().find(|c| Command::new(c).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()))
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
    for &(t, s, volume) in sfx {
        let at = frame_of(t) - f0;
        let sound = &sounds[s as usize];
        for (k, v) in sound.iter().enumerate() {
            let i = at + k as i64;
            if i >= 0 && (i as usize) < n {
                out[i as usize * 2] += v * fx * volume;
                out[i as usize * 2 + 1] += v * fx * volume;
            }
        }
    }
    out.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
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

/// Feeds RGBA frames to an ffmpeg process making the MP4.
pub struct Encoder {
    pub tx: Option<SyncSender<Vec<u8>>>,
    pub done: Receiver<Result<PathBuf, String>>,
}

#[allow(clippy::too_many_arguments)]
pub fn start_encoder(ffmpeg: &Path, (w, h): (u32, u32), fps: u32, (cw, ch, cx, cy): (u32, u32, u32, u32), wav: &Path, out: &Path) -> Result<Encoder, String> {
    let mut child = Command::new(ffmpeg)
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"])
        .arg(format!("{w}x{h}"))
        .arg("-r")
        .arg(fps.to_string())
        .args(["-i", "-", "-i"])
        .arg(wav)
        .arg("-vf")
        .arg(format!("crop={cw}:{ch}:{cx}:{cy}"))
        .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-shortest"])
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("can't run ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no ffmpeg input")?;
    // A few frames of slack: rendering waits for ffmpeg rather than piling frames up.
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(6);
    let (done_tx, done) = mpsc::channel();
    let (out, wav) = (out.to_path_buf(), wav.to_path_buf());
    std::thread::spawn(move || {
        let mut failed = None;
        for frame in rx {
            if let Err(e) = stdin.write_all(&frame) {
                failed = Some(e.to_string());
                break;
            }
        }
        drop(stdin);
        let output = child.wait_with_output();
        let _ = std::fs::remove_file(&wav);
        let result = match (failed, output) {
            (_, Ok(o)) if o.status.success() => Ok(out),
            (_, Ok(o)) => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("ffmpeg failed").to_string()),
            (Some(e), Err(_)) => Err(e),
            (None, Err(e)) => Err(e.to_string()),
        };
        let _ = done_tx.send(result);
    });
    Ok(Encoder { tx: Some(tx), done })
}

/// An export in progress: the replay stepped one frame at a time.
pub struct Export {
    pub game: Box<Game>,
    /// Song ms of the next frame, and song ms per frame (map time is `game.speed().map(t)`).
    pub t: f64,
    pub step: f64,
    pub size: (u32, u32),
    pub encoder: Encoder,
    pub frame: u64,
    pub frames: u64,
    pub out: PathBuf,
}

/// Where exported videos go.
pub fn videos_dir() -> Option<PathBuf> {
    Some(crate::config::user_dir(false)?.join("videos"))
}

/// A file name for a replay video.
pub fn file_name(r: &Replay) -> String {
    let clean: String = r.map_label.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    format!("{} - {} {}.mp4", clean.trim().chars().take(100).collect::<String>(), r.player, r.date)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_music_and_hitsounds_in_the_cut() {
        let sr = 1000;
        // 2 s of a constant 0.25 at 1.5x speed: map times 0..3000 ms.
        let track = Track { samples: vec![0.25; 2 * 2000], rate: 1.5 };
        let sounds = [vec![0.5; 10], vec![0.0; 1], vec![0.0; 1], vec![0.0; 1], vec![0.0; 1]];
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
    fn crop_has_even_sides_inside_the_frame() {
        let (_tx, rx2) = mpsc::channel::<Loaded>();
        let replay: Replay = serde_json::from_str(r#"{"version":1,"map_hash":1,"map_path":"","map_label":"m","player":"p","date":0,"rate":1.0,"anti_mash":false,"score":0,"accuracy":1.0,"max_combo":0,"great":0,"ok":0,"miss":0,"ur":0.0,"inputs":[]}"#).unwrap();
        let mut e = Editor::new(replay, rx2);
        e.crop = Area { x: 0.333, y: 0.1, w: 0.3337, h: 0.5011 };
        let (cw, ch, cx, cy) = e.crop_px();
        assert!(cw % 2 == 0 && ch % 2 == 0);
        assert!(cx + cw <= 1280 && cy + ch <= 720);
    }
}
