//! Startup splash, timed to its voice clip (`assets/splash.ogg`, ~2.3 s): the screen powers on like
//! a CRT, a boot log of what is really loading types itself out, a pixel portrait assembles from
//! don/kat-coloured pixels, "taiko-rs" drops in with the syllables, the hiss glitches the picture,
//! a loading bar fills to READY and the screen powers off into song select. Effects follow the
//! clip's loudness. Any key or click skips to the power-off. Settings -> Video -> *Startup splash*
//! turns it off.

use std::sync::Arc;
use std::time::Instant;

use crate::audio::{self, Engine, Track};
use crate::render::{rgba, Canvas, VIRTUAL_HEIGHT};

const PORTRAIT: &[u8] = include_bytes!("../assets/splash.png");
const VOICE: &[u8] = include_bytes!("../assets/splash.ogg");

/// Size of one portrait pixel (virtual units); the title's font pixels are a bit bigger.
const PX: f32 = 3.0;
const TEXT_SIZE: f32 = 7.0 * 4.0;
const TITLE: &str = "taiko-rs";

// The timeline, in ms of the clip: a click at 100, the voice from 500 (a long "tai" up to
// ~1150), "ko" around 1250-1400, a hiss 1420-1620, two short sounds around 1700 and 1900,
// quiet from 2000.
const POWER_ON: f32 = 100.0;
const ASSEMBLE: (f32, f32) = (150.0, 560.0);
/// When each title letter lands.
const LETTERS: [f32; 8] = [500.0, 610.0, 720.0, 1220.0, 1320.0, 1400.0, 1450.0, 1540.0];
/// Drum hits on the voice: when, and whether it's a kat (blue) rather than a don (red).
const HITS: [(f32, bool); 5] = [(500.0, false), (1220.0, true), (1450.0, false), (1700.0, false), (1900.0, true)];
const GLITCH: (f32, f32) = (1420.0, 1640.0);
/// Loading bar progress keyframes (ms, 0..1): it stalls and jumps like a real one.
const LOADING: [(f32, f32); 9] =
    [(150.0, 0.0), (420.0, 0.16), (560.0, 0.21), (820.0, 0.52), (1160.0, 0.58), (1300.0, 0.71), (1640.0, 0.86), (1760.0, 0.93), (2000.0, 1.0)];
const POWER_OFF: f32 = 2150.0;
const POWER_OFF_LEN: f32 = 380.0;

const DON: u32 = 0xEB452C;
const KAT: u32 = 0x438EAD;
const BG: u32 = 0x0b0b10;

pub struct Splash {
    /// The first frame drawn: the window can take a moment to appear after startup.
    started: Option<Instant>,
    /// Portrait pixels: grid position and colour.
    pixels: Vec<(u32, u32, [f32; 4])>,
    size: (u32, u32),
    voice: Option<Arc<Track>>,
    /// Loudness of the clip per 10 ms, 0..1.
    envelope: Vec<f32>,
    /// Splash time the power-off starts from when skipped.
    skipped_at: Option<f32>,
}

/// Cheap deterministic randomness in 0..1.
fn hash(a: u32, b: u32, salt: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Overshoots a little before settling, like something landing.
fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * t * t * t + c1 * t * t
}

fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn with_alpha(mut c: [f32; 4], a: f32) -> [f32; 4] {
    c[3] *= a;
    c
}

/// Piecewise-linear lookup in (time, value) keyframes.
fn keyframes(keys: &[(f32, f32)], t: f32) -> f32 {
    let Some(i) = keys.iter().position(|k| k.0 > t) else { return keys.last().map(|k| k.1).unwrap_or(0.0) };
    if i == 0 {
        return keys[0].1;
    }
    let ((t0, v0), (t1, v1)) = (keys[i - 1], keys[i]);
    v0 + (v1 - v0) * (t - t0) / (t1 - t0)
}

impl Splash {
    /// `rate`: the audio device's sample rate, or None to run silent.
    pub fn new(rate: Option<u32>) -> Option<Splash> {
        let img = image::load_from_memory(PORTRAIT).ok()?.to_rgba8();
        let pixels = img
            .enumerate_pixels()
            .filter(|(_, _, p)| p[3] > 127)
            .map(|(x, y, p)| (x, y, [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, 1.0]))
            .collect();
        let voice = rate.and_then(|r| audio::decode_bytes(VOICE, "ogg", r).ok().map(|t| (r, t)));
        let envelope = voice.as_ref().map(|(r, t)| envelope(t, *r)).unwrap_or_default();
        Some(Splash { started: None, pixels, size: img.dimensions(), voice: voice.map(|(_, t)| Arc::new(t)), envelope, skipped_at: None })
    }

    /// Splash time (ms): the clip's position while it plays, so the picture stays on the voice,
    /// else time since the first frame.
    fn clock(&self, now: Instant, engine: Option<&Engine>) -> f32 {
        let wall = self.started.map(|s| now.saturating_duration_since(s).as_secs_f32() * 1000.0).unwrap_or(0.0);
        match engine.and_then(|e| e.position_ms(now)) {
            Some(p) if self.voice.is_some() && self.skipped_at.is_none() => p as f32,
            _ => wall,
        }
    }

    fn loudness(&self, t: f32) -> f32 {
        self.envelope.get((t / 10.0).max(0.0) as usize).copied().unwrap_or(0.0)
    }

    /// Jumps to the power-off (any key or click) and silences the clip.
    pub fn skip(&mut self, now: Instant, engine: Option<&Engine>) {
        let t = self.clock(now, engine);
        if self.skipped_at.is_none() && t < POWER_OFF {
            self.skipped_at = Some(t);
            // The clock is wall time from here on: restart it where the clip was.
            self.started = Some(now - std::time::Duration::from_secs_f32(t / 1000.0));
            if let (Some(e), Some(_)) = (engine, &self.voice) {
                e.stop();
            }
        }
    }

    /// Covers the screen until the power-off is through; false once it's over. `boot`: lines for
    /// the boot log (what is loading, for real).
    pub fn draw(&mut self, c: &mut Canvas, now: Instant, engine: Option<&Engine>, boot: &[String]) -> bool {
        if self.started.is_none() {
            self.started = Some(now);
            if let (Some(e), Some(v)) = (engine, &self.voice) {
                e.play(v.clone(), 0.0);
            }
        }
        let t = self.clock(now, engine);
        let off_from = self.skipped_at.unwrap_or(POWER_OFF);
        let off = ((t - off_from) / POWER_OFF_LEN).clamp(0.0, 1.0);
        if off >= 1.0 {
            return false;
        }
        // Frozen where it was when skipped, so the power-off shows the picture as it was.
        let t = t.min(off_from);
        let w = c.width;
        let bg_alpha = 1.0 - ((off - 0.75) / 0.25).clamp(0.0, 1.0);
        c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(BG, bg_alpha));
        if t < POWER_ON {
            return true;
        }
        let start = c.shapes.len();
        let loud = self.loudness(t);

        let (gw, gh) = (self.size.0 as f32, self.size.1 as f32);
        let (pw, ph) = (gw * PX, gh * PX);
        let (cx, cy) = (w / 2.0, 40.0 + ph / 2.0);

        // Hit rings burst out behind the portrait; the picture bumps and the screen shakes.
        let mut bump = 0.0f32;
        let mut shake = 0.0f32;
        for &(at, kat) in &HITS {
            let p = (t - at) / 450.0;
            if (0.0..1.0).contains(&p) {
                let col = if kat { KAT } else { DON };
                let r = ph * 0.45 + ease_out_cubic(p) * ph * 0.6;
                c.ring(cx, cy, r, rgba(col, (1.0 - p) * 0.8), 3.0 + 6.0 * (1.0 - p));
                bump = bump.max((1.0 - p).powi(3) * 0.07);
                shake = shake.max((1.0 - p).powi(4) * 7.0);
            }
        }
        // A glow that breathes with the voice.
        c.glow(cx, cy, ph * 0.4, 50.0 + 40.0 * loud, mix(rgba(DON, 0.15 + 0.5 * loud), rgba(0xFFB347, 0.6 * loud), loud));

        // The hiss tears the picture: rows slide sideways, colour channels split, static.
        let glitch = if (GLITCH.0..GLITCH.1).contains(&t) { (0.4 + loud * 1.5).min(1.0) } else { 0.0 };
        let bucket = (t / 40.0) as u32;
        let split = (glitch * 5.0).max((loud - 0.45).max(0.0) * 8.0);

        let scale = 1.0 + bump + loud * 0.03;
        let mut placed = Vec::with_capacity(self.pixels.len());
        for &(x, y, col) in &self.pixels {
            let delay = ASSEMBLE.0 + (1.0 - y as f32 / gh) * 160.0 + hash(x, y, 1) * 120.0;
            let p = ((t - delay) / (ASSEMBLE.1 - ASSEMBLE.0 - 200.0)).clamp(0.0, 1.0);
            if p <= 0.0 {
                continue;
            }
            let row_shift = if glitch > 0.0 && hash(y / 3, bucket, 5) > 0.72 { (hash(y / 3, bucket, 6) - 0.5) * 40.0 * glitch } else { 0.0 };
            let tx = cx + ((x as f32 + 0.5) * PX - pw / 2.0) * scale + row_shift;
            let ty = cy + ((y as f32 + 0.5) * PX - ph / 2.0) * scale;
            let angle = hash(x, y, 2) * std::f32::consts::TAU;
            let dist = 180.0 + hash(x, y, 3) * 300.0;
            let (sx, sy) = (cx + angle.cos() * dist, cy + angle.sin() * dist * 0.7);
            let e = ease_out_back(p);
            let (px, py) = (sx + (tx - sx) * e, sy + (ty - sy) * e);
            let note = rgba(if hash(x, y, 4) < 0.5 { DON } else { KAT }, 1.0);
            let mut colour = mix(note, col, ease_out_cubic((p - 0.35) / 0.65));
            colour[3] = (p * 3.0).min(1.0);
            // Slightly oversized so neighbouring pixels leave no seams.
            let s = (PX * scale + 0.4) * (0.4 + 0.6 * ease_out_cubic(p));
            placed.push((px - s / 2.0, py - s / 2.0, s, colour, p >= 1.0));
        }
        // The split colour copies all go underneath, so they only show around the edges.
        if split > 0.5 {
            for &(x, y, s, _, _) in placed.iter().filter(|p| p.4) {
                c.rect(x - split, y, s, s, rgba(0xff2a4a, 0.4));
                c.rect(x + split, y, s, s, rgba(0x2ad4ff, 0.4));
            }
        }
        for &(x, y, s, colour, _) in &placed {
            c.rect(x, y, s, s, colour);
        }
        if glitch > 0.0 {
            for i in 0..140 {
                let (nx, ny) = (hash(i, bucket, 7), hash(i, bucket, 8));
                let v = 0.5 + hash(i, bucket, 9) * 0.5;
                c.rect(cx - pw / 2.0 + nx * pw, cy - ph / 2.0 + ny * ph, PX, PX, [v, v, v, 0.7 * glitch]);
            }
        }

        // The title: letters drop in on the syllables and bounce.
        let tw = Canvas::text_width(TEXT_SIZE, TITLE);
        let mut x = cx - tw / 2.0;
        let ty = cy + ph / 2.0 + 24.0;
        for (i, ch) in TITLE.chars().enumerate() {
            let s = ch.to_string();
            let adv = Canvas::text_width(TEXT_SIZE, &s);
            let p = (t - LETTERS[i] + 120.0) / 360.0;
            if p > 0.0 {
                let drop = (1.0 - ease_out_back(p)) * -70.0;
                let col = if ch == '-' { 0x6f6f86 } else if i % 2 == 0 { 0xffffff } else { DON };
                let a = (p * 4.0).min(1.0);
                let jitter = if glitch > 0.0 && hash(i as u32, bucket, 10) > 0.6 { (hash(i as u32, bucket, 11) - 0.5) * 16.0 } else { 0.0 };
                c.text(x + 2.0 + jitter, ty + drop + 2.0, TEXT_SIZE, rgba(0x000000, 0.5 * a), &s);
                if split > 0.5 {
                    c.text(x - split + jitter, ty + drop, TEXT_SIZE, rgba(0xff2a4a, 0.4 * a), &s);
                    c.text(x + split + jitter, ty + drop, TEXT_SIZE, rgba(0x2ad4ff, 0.4 * a), &s);
                }
                c.text(x + jitter, ty + drop, TEXT_SIZE, rgba(col, a), &s);
            }
            x += adv;
        }

        // A little lane under the title: notes scroll in and get hit on the beats.
        let lane_y = ty + TEXT_SIZE + 22.0;
        let lane_w = tw + 40.0;
        let lane_in = ease_out_cubic((t - 250.0) / 350.0);
        if lane_in > 0.0 {
            let lw = lane_w * lane_in;
            c.round_rect(cx - lw / 2.0, lane_y - 13.0, lw, 26.0, 13.0, rgba(0x20202b, 1.0), rgba(0x3a3a4e, 1.0), 1.5);
            let hit_x = cx - lw / 2.0 + 20.0;
            c.ring(hit_x, lane_y, 9.0, rgba(0xffffff, 0.4 * lane_in), 1.5);
            for &(at, kat) in &HITS {
                // Notes travel 0.35 units per ms and flash when they reach the hit circle.
                let col = if kat { KAT } else { DON };
                let since = t - at;
                let nx = hit_x - since * 0.35;
                if since < 0.0 && nx < cx + lw / 2.0 - 8.0 {
                    c.circle(nx, lane_y, 8.0, rgba(col, 1.0), rgba(0xffffff, 1.0), 1.5);
                } else if (0.0..200.0).contains(&since) {
                    c.glow(hit_x, lane_y, 9.0, 14.0, rgba(col, 1.0 - since / 200.0));
                }
            }
        }

        // Boot log, bottom left: one line every ~300 ms, typed out.
        for (i, line) in boot.iter().enumerate() {
            let at = 180.0 + i as f32 * 300.0;
            if t < at {
                break;
            }
            let shown = (((t - at) * 0.14) as usize).min(line.chars().count());
            let text: String = line.chars().take(shown).collect();
            let cursor = if shown < line.chars().count() || (t / 250.0) as u32 % 2 == 0 { "_" } else { " " };
            let y = 400.0 - (boot.len() - 1 - i) as f32 * 11.0;
            c.text(16.0, y, 7.0, rgba(0x7fbf7f, 0.85), &format!("> {text}{cursor}"));
        }

        // Loading bar, bottom centre.
        let progress = keyframes(&LOADING, t);
        let (bw, by) = (260.0f32.min(w - 40.0), 440.0);
        let bx = cx - bw / 2.0;
        c.round_rect(bx, by, bw, 10.0, 3.0, rgba(0x15151d, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
        if progress > 0.0 {
            c.round_rect(bx + 2.0, by + 2.0, (bw - 4.0) * progress, 6.0, 2.0, rgba(DON, 1.0), rgba(DON, 1.0), 0.0);
            // A shine running along the filled part.
            let shine = bx + 2.0 + ((t * 0.4) % ((bw - 4.0) * progress).max(1.0));
            c.rect(shine, by + 2.0, 6.0, 6.0, rgba(0xffffff, 0.35));
        }
        let label = if progress >= 1.0 {
            if (t / 120.0) as u32 % 2 == 0 { "READY".to_string() } else { String::new() }
        } else {
            format!("LOADING {:>3.0}%", progress * 100.0)
        };
        c.text_centered(cx, by - 14.0, 7.0, rgba(if progress >= 1.0 { 0x86E05A } else { 0x9a9ab0 }, 1.0), &label);

        // CRT: scanlines, a rolling bright band, and the power-on flash.
        for i in 0..(VIRTUAL_HEIGHT / 4.0) as usize {
            c.rect(0.0, i as f32 * 4.0, w, 1.2, rgba(0x000000, 0.22));
        }
        let band = (t * 0.18) % (VIRTUAL_HEIGHT + 80.0) - 40.0;
        c.rect(0.0, band, w, 40.0, rgba(0xffffff, 0.025));
        let on = (t - POWER_ON) / 220.0;
        if on < 1.0 {
            let h = 2.0 + ease_out_cubic(on) * VIRTUAL_HEIGHT;
            c.rect(0.0, VIRTUAL_HEIGHT / 2.0 - h / 2.0, w, h, rgba(0xffffff, (1.0 - on) * 0.85));
        }

        // Shake everything, then the power-off: squash to a line, shrink the line to a dot.
        if shake > 0.0 {
            let (dx, dy) = ((t * 0.09).sin() * shake, (t * 0.13).cos() * shake);
            c.transform_since(start, [0.0, 0.0], 1.0, dx, dy);
        }
        if off > 0.0 {
            let ky = (1.0 - ease_out_cubic(off / 0.55)).max(0.004);
            let kx = 1.0 - ease_out_cubic((off - 0.55) / 0.3);
            let mid = VIRTUAL_HEIGHT / 2.0;
            for sh in &mut c.shapes[start..] {
                sh.center = [cx + (sh.center[0] - cx) * kx, mid + (sh.center[1] - mid) * ky];
                sh.half = [sh.half[0] * kx, sh.half[1] * ky];
                sh.fill = with_alpha(sh.fill, bg_alpha);
                sh.border = with_alpha(sh.border, bg_alpha);
            }
            let line_a = (off / 0.3).min(1.0) * bg_alpha;
            c.rect(cx - w * kx / 2.0, mid - 1.5, w * kx, 3.0, rgba(0xffffff, 0.9 * line_a));
            c.glow(cx, mid, 2.0, 30.0 * kx + 6.0, rgba(0xffffff, 0.5 * line_a));
        }
        true
    }
}

/// Peak-normalised loudness per 10 ms of a stereo track at `rate`.
fn envelope(track: &Track, rate: u32) -> Vec<f32> {
    let window = (rate as usize / 100).max(1) * 2;
    let rms: Vec<f32> = track.samples.chunks(window).map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let peak = rms.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    // A gentle curve so quieter parts still move things a little.
    rms.iter().map(|v| (v / peak).powf(0.7)).collect()
}
