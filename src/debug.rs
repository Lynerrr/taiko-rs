//! Diagnostics: screenshot encoding and frame/clock smoothness statistics.
//!
//! `TAIKO_STATS=1` prints a report when leaving gameplay (with vsync, its "note step vs refresh
//! grid" line shows how evenly notes move; `TAIKO_NO_PACE=1` turns the vsync pacing off to
//! compare); `TAIKO_SHOTS=dir:ms,ms,...` saves PNGs at the given gameplay times (used for headless
//! testing).

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
    /// Per frame: how long after the one before it got its swapchain image, and how far the
    /// notes moved (the time they're drawn at, real ms).
    shown_steps: Vec<(f64, f64)>,
    last_shown: Option<(f64, f64)>,
}

impl Smoothness {
    /// `shown_ms`: when the frame got its swapchain image; `drawn_ms`: the time its notes are
    /// drawn at.
    pub fn sample(&mut self, wall_ms: f64, game_ms: f64, audio_ms: Option<f64>, shown_ms: f64, drawn_ms: f64) {
        if let Some((s, d)) = self.last_shown {
            self.shown_steps.push((shown_ms - s, drawn_ms - d));
        }
        self.last_shown = Some((shown_ms, drawn_ms));
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
        self.last_shown = None;
    }

    /// How evenly notes move from one frame to the next: with vsync frames are shown whole
    /// refreshes apart (the refresh being the median time between images), so each step of the
    /// notes should be whole refreshes too. Also how many frames showed a refresh or more late.
    fn pacing(&self) -> (f64, Vec<f64>, usize) {
        let mut steps: Vec<f64> = self.shown_steps.iter().map(|s| s.0).collect();
        steps.sort_by(|a, b| a.total_cmp(b));
        let refresh = steps.get(steps.len() / 2).copied().unwrap_or(16.7).max(0.1);
        let mut late = 0;
        let dev = self
            .shown_steps
            .iter()
            .map(|&(_, drawn)| {
                let n = (drawn / refresh).round().max(1.0);
                late += (n >= 2.0) as usize;
                (drawn - n * refresh).abs()
            })
            .collect();
        (refresh, dev, late)
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
        let pacing = self.pacing();
        format!(
            "frames {}  avg {:.0} fps\n\
             frame time  p50 {:.3}  p99 {:.3}  max {:.3} ms\n\
             clock jitter vs wall  p50 {:.4}  p99 {:.4}  max {:.4} ms  (backwards steps: {})\n\
             clock vs raw audio  p50 {:.3}  p99 {:.3}  max {:.3} ms\n\
             blocked in acquire  p50 {:.3}  p99 {:.3} ms;  in present  p50 {:.3}  p99 {:.3} ms\n\
             clock sample -> next refresh  p50 {:.3}  p99 {:.3} ms\n\
             note step vs refresh grid ({:.3} ms)  p50 {:.4}  p99 {:.4}  max {:.4} ms  (frames a refresh late: {})",
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
            pacing.0,
            pct(&pacing.1, 0.5),
            pct(&pacing.1, 0.99),
            pct(&pacing.1, 1.0),
            pacing.2,
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
    /// Right button pressed at / pointer moved to a point (virtual units), and released:
    /// `RightDown:x:y`, `RightMove:x:y`, `RightUp`.
    RightDown(f32, f32),
    RightMove(f32, f32),
    RightUp,
    /// Left button pressed at / pointer moved to a point (virtual units), and released:
    /// `LeftDown:x:y`, `LeftMove:x:y`, `LeftUp`.
    LeftDown(f32, f32),
    LeftMove(f32, f32),
    LeftUp,
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
                    "RightUp" => Action::RightUp,
                    "LeftUp" => Action::LeftUp,
                    k if k.starts_with("LeftDown:") || k.starts_with("LeftMove:") => {
                        let mut p = k.split(':').skip(1).filter_map(|v| v.parse::<f32>().ok());
                        let (x, y) = (p.next()?, p.next()?);
                        if k.starts_with("LeftDown:") { Action::LeftDown(x, y) } else { Action::LeftMove(x, y) }
                    }
                    "AltWheelUp" => Action::AltWheel(1),
                    "AltWheelDown" => Action::AltWheel(-1),
                    k if k.starts_with("RightDown:") || k.starts_with("RightMove:") => {
                        let mut p = k.split(':').skip(1).filter_map(|v| v.parse::<f32>().ok());
                        let (x, y) = (p.next()?, p.next()?);
                        if k.starts_with("RightDown:") { Action::RightDown(x, y) } else { Action::RightMove(x, y) }
                    }
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

/// Text that doesn't fit, as readable lines: off screen, sticking out of the rounded box it sits
/// in, cut off by a clip, or drawn over other text. The canvas must have been drawn with `audit`.
pub fn layout_problems(c: &crate::render::Canvas) -> Vec<String> {
    type Rect = [f32; 4]; // x0, y0, x1, y1
    const EPS: f32 = 0.6;
    let bounds = |i: usize| -> Rect {
        let s = &c.shapes[i];
        [s.center[0] - s.half[0], s.center[1] - s.half[1], s.center[0] + s.half[0], s.center[1] + s.half[1]]
    };
    let contains = |r: Rect, x: f32, y: f32| x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3];
    let inside = |a: Rect, b: Rect| a[0] >= b[0] - EPS && a[2] <= b[2] + EPS && a[1] >= b[1] - EPS && a[3] <= b[3] + EPS;
    let clip_of = |i: usize| -> Option<Rect> {
        c.clips.iter().filter(|(r, _)| r.contains(&i)).fold(None, |acc: Option<Rect>, (_, [x, y, w, h])| {
            let r = [*x, *y, x + w, y + h];
            Some(match acc {
                Some(a) => [a[0].max(r[0]), a[1].max(r[1]), a[2].min(r[2]), a[3].min(r[3])],
                None => r,
            })
        })
    };
    let fmt = |r: Rect| format!("{:.0}..{:.0} x {:.0}..{:.0}", r[0], r[2], r[1], r[3]);
    let short = |s: &str| if s.chars().count() > 48 { format!("{}...", s.chars().take(45).collect::<String>()) } else { s.to_string() };

    // Visible texts: (first shape, last shape + 1, ink rect, visible part, string).
    let mut texts: Vec<(usize, usize, Rect, Rect, &str)> = Vec::new();
    let mut out = Vec::new();
    for (range, s) in &c.texts {
        if range.is_empty() || c.shapes[range.start].fill[3] < 0.15 {
            continue;
        }
        let ink = range.clone().map(bounds).fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
        let mut seen = ink;
        if let Some(clip) = clip_of(range.start) {
            seen = [ink[0].max(clip[0]), ink[1].max(clip[1]), ink[2].min(clip[2]), ink[3].min(clip[3])];
            if seen[0] >= seen[2] || seen[1] >= seen[3] {
                continue;
            }
            if ink[0] < clip[0] - EPS || ink[2] > clip[2] + EPS {
                out.push(format!("cut off: \"{}\" {} by its clip {}", short(s), fmt(ink), fmt(clip)));
            }
        }
        if ink[0] < -EPS || ink[2] > c.width + EPS || ink[1] < -EPS || ink[3] > crate::render::VIRTUAL_HEIGHT + EPS {
            out.push(format!("off screen: \"{}\" {} (width {:.0})", short(s), fmt(ink), c.width));
        }
        // The box under the text's start and under its middle (centred labels): the topmost
        // visible rounded box drawn before it.
        let mid_y = (ink[1] + ink[3]) / 2.0;
        for x in [ink[0] + 1.0, (ink[0] + ink[2]) / 2.0] {
            let holder = c.boxes.iter().rev().filter(|&&b| b < range.start).find(|&&b| {
                let sh = &c.shapes[b];
                let visible = sh.fill[3] > 0.1 || (sh.border[3] > 0.1 && sh.params[1] > 0.0);
                visible && contains(bounds(b), x, mid_y)
            });
            if let Some(&b) = holder
                && !inside(seen, bounds(b)) {
                out.push(format!("sticks out: \"{}\" {} past its box {}", short(s), fmt(seen), fmt(bounds(b))));
                break;
            }
        }
        texts.push((range.start, range.end, ink, seen, s));
    }
    for (i, a) in texts.iter().enumerate() {
        for b in &texts[i + 1..] {
            let r = [a.3[0].max(b.3[0]), a.3[1].max(b.3[1]), a.3[2].min(b.3[2]), a.3[3].min(b.3[3])];
            if r[2] - r[0] < 1.0 || r[3] - r[1] < 1.0 {
                continue;
            }
            // A drop shadow: the same text a pixel or two away.
            if a.4 == b.4 && (a.2[0] - b.2[0]).abs() <= 2.5 && (a.2[1] - b.2[1]).abs() <= 2.5 {
                continue;
            }
            // Something opaque drawn in between hides the first one there (a dialog over a screen).
            let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
            let covered = (a.1..b.0).any(|k| {
                let sh = &c.shapes[k];
                sh.fill[3] >= 0.6 && sh.half[0] >= 3.0 && sh.half[1] >= 3.0 && sh.params[3] < 0.5 && contains(bounds(k), cx, cy)
            });
            if !covered {
                out.push(format!("overlap: \"{}\" {} and \"{}\" {}", short(a.4), fmt(a.3), short(b.4), fmt(b.3)));
            }
        }
    }
    out
}

/// Draws a canvas on the CPU into an RGB image `scale` pixels per virtual unit: boxes, circles
/// and text, no glows or skin images. For looking at test frames without a window.
#[cfg(test)]
pub fn rasterize(c: &crate::render::Canvas, scale: f32) -> (u32, u32, Vec<u8>) {
    let (w, h) = ((c.width * scale).round() as u32, (crate::render::VIRTUAL_HEIGHT * scale).round() as u32);
    let mut px = vec![[0.071f32, 0.071, 0.098]; (w * h) as usize];
    for (i, sh) in c.shapes.iter().enumerate() {
        if sh.params[2] > 0.0 || sh.params[3] >= 9.5 {
            continue;
        }
        let clip = c.clips.iter().filter(|(r, _)| r.contains(&i)).fold([0.0, 0.0, f32::MAX, f32::MAX], |a: [f32; 4], (_, [x, y, cw, ch])| {
            [a[0].max(*x), a[1].max(*y), a[2].min(x + cw), a[3].min(y + ch)]
        });
        let (hx, hy) = (sh.half[0].abs(), sh.half[1].abs());
        let radius = sh.params[0].min(hx).min(hy);
        let border = sh.params[1];
        // A turned shape is looked at in its own frame, within the circle around it.
        let (sin, cos) = sh.angle.sin_cos();
        let (ex, ey) = if sh.angle == 0.0 { (hx, hy) } else { (hx.hypot(hy), hx.hypot(hy)) };
        let x0 = ((sh.center[0] - ex).max(clip[0]) * scale).floor().max(0.0) as u32;
        let x1 = ((sh.center[0] + ex).min(clip[2]) * scale).ceil().clamp(0.0, w as f32) as u32;
        let y0 = ((sh.center[1] - ey).max(clip[1]) * scale).floor().max(0.0) as u32;
        let y1 = ((sh.center[1] + ey).min(clip[3]) * scale).ceil().clamp(0.0, h as f32) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = ((x as f32 + 0.5) / scale - sh.center[0], (y as f32 + 0.5) / scale - sh.center[1]);
                let (lx, ly) = (dx * cos + dy * sin, -dx * sin + dy * cos);
                if (sh.params[3] == 1.0 && lx > 0.0) || (sh.params[3] == 2.0 && lx < 0.0) {
                    continue;
                }
                // Signed distance to the rounded rect.
                let (qx, qy) = (lx.abs() - hx + radius, ly.abs() - hy + radius);
                let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
                if d > 0.0 {
                    continue;
                }
                let col = if border > 0.0 && d > -border { sh.border } else { sh.fill };
                let p = &mut px[(y * w + x) as usize];
                for k in 0..3 {
                    p[k] += (col[k] - p[k]) * col[3];
                }
            }
        }
    }
    (w, h, px.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)).collect())
}
