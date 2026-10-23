//! Diagnostics: screenshot encoding and frame/clock smoothness statistics.
//!
//! `TAIKO_STATS=1` prints a report when leaving gameplay; `TAIKO_SHOTS=dir:ms,ms,...` saves PNGs
//! at the given gameplay times (used for headless testing).

use std::path::{Path, PathBuf};

/// Minimal PNG writer (uncompressed deflate) so screenshots need no extra dependencies.
pub fn write_png(path: &Path, width: u32, height: u32, rgb: &[u8]) -> std::io::Result<()> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for row in rgb.chunks_exact(width as usize * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push((i == blocks.len() - 1) as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)
}

pub struct ShotPlan {
    pub dir: PathBuf,
    pub times: Vec<f64>,
}

impl ShotPlan {
    pub fn from_env() -> Option<ShotPlan> {
        let v = std::env::var("TAIKO_SHOTS").ok()?;
        let (dir, times) = v.rsplit_once(':')?;
        let mut times: Vec<f64> = times.split(',').filter_map(|t| t.trim().parse().ok()).collect();
        times.sort_by(|a, b| b.total_cmp(a));
        Some(ShotPlan { dir: PathBuf::from(dir), times })
    }
}

/// Collects per-frame samples during gameplay.
#[derive(Default)]
pub struct Smoothness {
    frame_ms: Vec<f64>,
    /// |game-time advance − wall-time advance| per frame.
    jitter_ms: Vec<f64>,
    /// Smoothed clock minus raw audio position.
    drift_ms: Vec<f64>,
    backwards: u32,
    last: Option<(f64, f64)>,
    /// Time blocked waiting for a swapchain image / inside present, per frame.
    acquire_ms: Vec<f64>,
    present_ms: Vec<f64>,
    /// Clock sample to the next refresh boundary (next acquire returning).
    sample_age_ms: Vec<f64>,
}

impl Smoothness {
    pub fn sample(&mut self, wall_ms: f64, game_ms: f64, audio_ms: Option<f64>) {
        if let Some((w, g)) = self.last {
            let dw = wall_ms - w;
            let dg = game_ms - g;
            self.frame_ms.push(dw);
            self.jitter_ms.push((dg - dw).abs());
            if dg < 0.0 {
                self.backwards += 1;
            }
        }
        if let Some(a) = audio_ms {
            self.drift_ms.push(game_ms - a);
        }
        self.last = Some((wall_ms, game_ms));
    }

    pub fn break_sequence(&mut self) {
        self.last = None;
    }

    pub fn sample_waits(&mut self, acquire_ms: f64, present_ms: f64, sample_age_ms: f64) {
        self.acquire_ms.push(acquire_ms);
        self.present_ms.push(present_ms);
        if sample_age_ms > 0.0 {
            self.sample_age_ms.push(sample_age_ms);
        }
    }

    pub fn report(&self) -> String {
        fn pct(v: &[f64], p: f64) -> f64 {
            if v.is_empty() {
                return 0.0;
            }
            let mut s = v.to_vec();
            s.sort_by(|a, b| a.total_cmp(b));
            s[((s.len() - 1) as f64 * p).round() as usize]
        }
        let n = self.frame_ms.len().max(1) as f64;
        let mean = self.frame_ms.iter().sum::<f64>() / n;
        let drift_abs: Vec<f64> = self.drift_ms.iter().map(|d| d.abs()).collect();
        format!(
            "frames {}  avg {:.0} fps\n\
             frame time  p50 {:.3}  p99 {:.3}  max {:.3} ms\n\
             clock jitter vs wall  p50 {:.4}  p99 {:.4}  max {:.4} ms  (backwards steps: {})\n\
             clock vs raw audio  p50 {:.3}  p99 {:.3}  max {:.3} ms\n\
             blocked in acquire  p50 {:.3}  p99 {:.3} ms;  in present  p50 {:.3}  p99 {:.3} ms\n\
             clock sample -> next refresh  p50 {:.3}  p99 {:.3} ms",
            self.frame_ms.len(),
            1000.0 / mean.max(1e-9),
            pct(&self.frame_ms, 0.5),
            pct(&self.frame_ms, 0.99),
            pct(&self.frame_ms, 1.0),
            pct(&self.jitter_ms, 0.5),
            pct(&self.jitter_ms, 0.99),
            pct(&self.jitter_ms, 1.0),
            self.backwards,
            pct(&drift_abs, 0.5),
            pct(&drift_abs, 0.99),
            pct(&drift_abs, 1.0),
            pct(&self.acquire_ms, 0.5),
            pct(&self.acquire_ms, 0.99),
            pct(&self.present_ms, 0.5),
            pct(&self.present_ms, 0.99),
            pct(&self.sample_age_ms, 0.5),
            pct(&self.sample_age_ms, 0.99),
        )
    }
}

/// `TAIKO_INPUT=ms:Key,ms:Key,...` injects key presses at wall-clock times after launch, for
/// driving menus in headless tests. Keys use winit `KeyCode` names (`Escape`, `ArrowDown`, `KeyS`).
pub struct InputScript {
    events: Vec<(f64, Action)>,
}

/// A scripted input: a key, or mouse pseudo-keys: `WheelUp`/`WheelDown` (one notch),
/// `MoveDown`/`MoveRight` (drag the playfield area by 60 px), `ShrinkCorner` (drag its
/// bottom-right corner up-left by 300x200 px).
#[derive(Clone, Debug)]
pub enum Action {
    Key(winit::keyboard::KeyCode),
    Wheel(i32),
    /// Press at the area's centre (or its bottom-right corner), drag by (dx, dy) px, and release
    /// unless `hold` (then `Release` ends it), so screenshots can catch an in-progress drag.
    Drag { corner: bool, dx: f64, dy: f64, hold: bool },
    Release,
    /// Click the song select difficulty timeline at this fraction of its width.
    ClickGraph(f32),
    /// Seek the song select preview by this many ms (what Ctrl+Left/Right does).
    SeekBy(f64),
    /// Left click at a point in virtual units (480 tall): `Click:x:y`.
    Click(f32, f32),
    /// Wheel notches with Alt held (`AltWheelUp` / `AltWheelDown`).
    AltWheel(i32),
    /// A key with Shift held: `Shift+Enter`.
    ShiftKey(winit::keyboard::KeyCode),
    /// A key with Ctrl held: `Ctrl+KeyD`.
    CtrlKey(winit::keyboard::KeyCode),
    /// Typed text (search boxes): `Type:frums`.
    Type(String),
}

impl InputScript {
    pub fn from_env() -> Option<InputScript> {
        let v = std::env::var("TAIKO_INPUT").ok()?;
        let mut events: Vec<(f64, Action)> = v
            .split(',')
            .filter_map(|e| {
                let (t, k) = e.split_once(':')?;
                let action = match k.trim() {
                    "WheelUp" => Action::Wheel(1),
                    "WheelDown" => Action::Wheel(-1),
                    "MoveDown" => Action::Drag { corner: false, dx: 0.0, dy: 60.0, hold: false },
                    "MoveRight" => Action::Drag { corner: false, dx: 60.0, dy: 0.0, hold: false },
                    "ShrinkCorner" => Action::Drag { corner: true, dx: -300.0, dy: -200.0, hold: false },
                    "Release" => Action::Release,
                    "AltWheelUp" => Action::AltWheel(1),
                    "AltWheelDown" => Action::AltWheel(-1),
                    k if k.starts_with("Click:") => {
                        let mut p = k.split(':').skip(1).filter_map(|v| v.parse::<f32>().ok());
                        Action::Click(p.next()?, p.next()?)
                    }
                    k if k.starts_with("ClickGraph:") => Action::ClickGraph(k[11..].parse().ok()?),
                    k if k.starts_with("SeekBy:") => Action::SeekBy(k[7..].parse().ok()?),
                    // Move:dx:dy and Hold:dx:dy (drag without releasing).
                    k if k.starts_with("Move:") || k.starts_with("Hold:") => {
                        let mut p = k.split(':').skip(1).filter_map(|v| v.parse::<f64>().ok());
                        let (dx, dy) = (p.next()?, p.next()?);
                        Action::Drag { corner: false, dx, dy, hold: k.starts_with("Hold:") }
                    }
                    k if k.starts_with("Shift+") => Action::ShiftKey(parse_keycode(&k[6..])?),
                    k if k.starts_with("Ctrl+") => Action::CtrlKey(parse_keycode(&k[5..])?),
                    k if k.starts_with("Type:") => Action::Type(k[5..].replace('_', " ")),
                    k => Action::Key(parse_keycode(k)?),
                };
                Some((t.trim().parse().ok()?, action))
            })
            .collect();
        events.sort_by(|a, b| b.0.total_cmp(&a.0));
        Some(InputScript { events })
    }

    /// Keys due by `elapsed_ms`, oldest first.
    pub fn due(&mut self, elapsed_ms: f64) -> Vec<Action> {
        let mut out = Vec::new();
        while self.events.last().is_some_and(|e| e.0 <= elapsed_ms) {
            out.push(self.events.pop().unwrap().1);
        }
        out
    }
}

fn parse_keycode(name: &str) -> Option<winit::keyboard::KeyCode> {
    use winit::keyboard::KeyCode::*;
    let fixed = [
        ("Escape", Escape), ("Tab", Tab), ("Enter", Enter), ("Space", Space), ("Backspace", Backspace),
        ("ArrowUp", ArrowUp), ("ArrowDown", ArrowDown), ("ArrowLeft", ArrowLeft), ("ArrowRight", ArrowRight),
        ("F1", F1), ("F3", F3), ("F4", F4), ("F7", F7), ("F8", F8), ("F9", F9), ("F10", F10), ("F12", F12),
    ];
    fixed.iter().find(|(n, _)| *n == name).map(|(_, k)| *k).or_else(|| crate::config::parse_key(name))
}

/// `TAIKO_WALL_SHOTS=ms,ms` captures screenshots at wall-clock times after launch (any screen).
pub fn wall_shots_from_env() -> Vec<f64> {
    let mut v: Vec<f64> = std::env::var("TAIKO_WALL_SHOTS")
        .map(|s| s.split(',').filter_map(|t| t.trim().parse().ok()).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| b.total_cmp(a));
    v
}
