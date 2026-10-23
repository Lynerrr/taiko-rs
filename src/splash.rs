//! Startup splash, timed to its voice clip (`assets/splash.ogg`, ~2.3 s, an angry shout): the screen
//! powers on like a CRT, a boot log of what is really loading types itself out and a pixel portrait
//! (one of several, picked at random) assembles from don/kat-coloured pixels. Then the shout: every
//! word punches the portrait at the camera with impact frames, shockwaves and screen shake, the
//! loudest stretch turns into a deep-fried rage (speed lines, red vignette, colour split), the hiss
//! glitches the picture, the title stamps in, a loading bar jumps to READY and the screen powers off
//! into song select. Effects follow the clip's loudness. Any key or click skips to the power-off.
//! Settings -> Video -> *Startup splash* turns it off.

use std::sync::Arc;
use std::time::Instant;

use crate::audio::{self, Engine, Track};
use crate::render::{rgba, Canvas, VIRTUAL_HEIGHT};

/// Pixel portraits, one picked at random each start.
const PORTRAITS: [&[u8]; 9] = [
    include_bytes!("../assets/splash.png"),
    include_bytes!("../assets/splash2.png"),
    include_bytes!("../assets/splash3.png"),
    include_bytes!("../assets/splash4.png"),
    include_bytes!("../assets/splash5.png"),
    include_bytes!("../assets/splash6.png"),
    include_bytes!("../assets/splash7.png"),
    include_bytes!("../assets/splash8.png"),
    include_bytes!("../assets/splash9.png"),
];
const VOICE: &[u8] = include_bytes!("../assets/splash.ogg");

/// Size of one portrait pixel (virtual units); the title's font pixels are a bit bigger.
const PX: f32 = 3.0;
const TEXT_SIZE: f32 = 7.0 * 4.0;

// The clip, in ms: a mic click at 100, then three words. The first 470-700 (syllables at 470 and
// 610), the second and loudest 710-1190 (syllables at 710 and 910), a decay, then the third
// 1430-1900: a hiss 1430-1630, a low vowel at 1650, a stop and a short vowel at 1830. Quiet
// from 2000.
const POWER_ON: f32 = 100.0;
const ASSEMBLE: (f32, f32) = (120.0, 470.0);
/// Every syllable hits: when, a kat (blue) rather than a don (red), and how hard (1 = impact
/// frame, big shockwave, heavy shake).
const HITS: [(f32, bool, f32); 7] =
    [(470.0, false, 0.4), (610.0, true, 0.4), (710.0, false, 1.0), (910.0, false, 1.0), (1430.0, true, 0.5), (1650.0, false, 0.6), (1830.0, false, 1.0)];
/// The loudest word: rage mode.
const RAGE: (f32, f32) = (700.0, 1200.0);
/// The hiss: glitches.
const GLITCH: (f32, f32) = (1420.0, 1640.0);
/// The title, in parts that stamp in on syllables.
const TITLE: [(&str, f32); 2] = [("taiko", 710.0), ("-rs", 1830.0)];
/// Loading bar progress keyframes (ms, 0..1): it crawls, then the words kick it forward.
const LOADING: [(f32, f32); 10] = [
    (120.0, 0.0),
    (460.0, 0.17),
    (700.0, 0.24),
    (730.0, 0.55),
    (900.0, 0.6),
    (930.0, 0.74),
    (1420.0, 0.78),
    (1640.0, 0.86),
    (1820.0, 0.88),
    (1850.0, 1.0),
];
const POWER_OFF: f32 = 2150.0;
const POWER_OFF_LEN: f32 = 380.0;

const DON: u32 = 0xEB452C;
const KAT: u32 = 0x438EAD;
const BG: u32 = 0x0b0b10;

pub struct Splash {
    /// When it started: once the window is up and focused (see `draw`).
    started: Option<Instant>,
    /// When the window was first ready to show it.
    ready_since: Option<Instant>,
    /// The clip's playback position drives the timeline, unless it drifts from the wall clock.
    audio_clock: bool,
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
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t.clamp(0.0, 1.0))
}

fn with_alpha(mut c: [f32; 4], a: f32) -> [f32; 4] {
    c[3] *= a;
    c
}

/// "Deep-fried": more contrast and saturation, pushed toward red, by `amount` (0..1).
fn fry(c: [f32; 4], amount: f32) -> [f32; 4] {
    let l = (c[0] + c[1] + c[2]) / 3.0;
    let mut out = c;
    for v in &mut out[..3] {
        let sat = l + (*v - l) * (1.0 + 1.2 * amount);
        *v = ((sat - 0.5) * (1.0 + 0.8 * amount) + 0.5).clamp(0.0, 1.0);
    }
    mix(out, [1.0, 0.2, 0.1, c[3]], 0.25 * amount)
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

/// How much of a fast-decaying kick from the hits is left at `t`: (punch 0..1, the strongest
/// hit's impact frame 0..1 for its first 110 ms).
fn kicks(t: f32) -> (f32, f32) {
    let mut punch = 0.0f32;
    let mut impact = 0.0f32;
    for &(at, _, strength) in &HITS {
        let since = t - at;
        if since >= 0.0 {
            punch = punch.max(strength * (-since / 140.0).exp());
            if strength >= 1.0 && since < 110.0 {
                impact = impact.max(1.0 - since / 110.0);
            }
        }
    }
    (punch, impact)
}

impl Splash {
    /// `rate`: the audio device's sample rate, or None to run silent.
    pub fn new(rate: Option<u32>) -> Option<Splash> {
        // TAIKO_SPLASH=<n> picks portrait n (1-based) instead.
        let pick = std::env::var("TAIKO_SPLASH").ok().and_then(|v| v.parse::<usize>().ok()).filter(|n| (1..=PORTRAITS.len()).contains(n));
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let index = pick.map(|n| n - 1).unwrap_or(nanos as usize % PORTRAITS.len());
        let img = image::load_from_memory(PORTRAITS[index]).ok()?.to_rgba8();
        let pixels = img
            .enumerate_pixels()
            .filter(|(_, _, p)| p[3] > 127)
            .map(|(x, y, p)| (x, y, [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, 1.0]))
            .collect();
        let voice = rate.and_then(|r| audio::decode_bytes(VOICE, "ogg", r).ok().map(|t| (r, t)));
        let envelope = voice.as_ref().map(|(r, t)| envelope(t, *r)).unwrap_or_default();
        Some(Splash { started: None, ready_since: None, audio_clock: true, pixels, size: img.dimensions(), voice: voice.map(|(_, t)| Arc::new(t)), envelope, skipped_at: None })
    }

    /// Splash time (ms): the clip's position while it plays, so the picture stays on the voice,
    /// else time since it started. A position far from the wall clock (an audio device reporting
    /// odd timestamps) would race through the whole splash, so then the wall clock takes over.
    fn clock(&mut self, now: Instant, engine: Option<&Engine>) -> f32 {
        let wall = self.started.map(|s| now.saturating_duration_since(s).as_secs_f32() * 1000.0).unwrap_or(0.0);
        match engine.and_then(|e| e.position_ms(now)) {
            Some(p) if self.audio_clock && self.voice.is_some() && self.skipped_at.is_none() => {
                if (p as f32 - wall).abs() > 300.0 {
                    eprintln!("splash: audio clock {p:.0} ms vs wall {wall:.0} ms, using the wall clock");
                    self.audio_clock = false;
                    return wall;
                }
                p as f32
            }
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
    /// the boot log (what is loading, for real). `ready`: the window has focus. Until then (a
    /// window opened behind others, as Windows does when the launcher lost the foreground) and for
    /// a moment after, it holds a black screen, so the splash isn't over before anyone sees it.
    pub fn draw(&mut self, c: &mut Canvas, now: Instant, engine: Option<&Engine>, boot: &[String], ready: bool) -> bool {
        if self.started.is_none() && self.skipped_at.is_none() {
            let since = if ready { *self.ready_since.get_or_insert(now) } else { now };
            if now.saturating_duration_since(since).as_millis() < 150 {
                c.rect(0.0, 0.0, c.width, VIRTUAL_HEIGHT, rgba(BG, 1.0));
                return true;
            }
        }
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
        let (punch, impact) = kicks(t);
        let rage = if (RAGE.0..RAGE.1).contains(&t) { loud } else { 0.0 };
        let glitch = if (GLITCH.0..GLITCH.1).contains(&t) { (0.5 + loud * 1.5).min(1.0) } else { 0.0 };
        let bucket = (t / 40.0) as u32;

        let (gw, gh) = (self.size.0 as f32, self.size.1 as f32);
        let (pw, ph) = (gw * PX, gh * PX);
        let (cx, cy) = (w / 2.0, 40.0 + ph / 2.0);

        // Red flash after an impact frame, and a red vignette that pulses with the rage.
        let flash = if impact > 0.0 { 0.0 } else { kicks(t - 110.0).1 };
        if flash > 0.0 {
            c.rect(-20.0, -20.0, w + 40.0, VIRTUAL_HEIGHT + 40.0, rgba(0x8a0f05, 0.7 * flash));
        }

        // Speed lines rushing out from the face while the loudest word lasts.
        if rage > 0.05 {
            let rays = 56;
            let shift = (t / 60.0) as u32;
            for i in 0..rays {
                let angle = (i as f32 + hash(i, shift, 20) * 0.6) / rays as f32 * std::f32::consts::TAU;
                let (dx, dy) = (angle.cos(), angle.sin());
                let inner = ph * 0.55 + hash(i, shift, 21) * 40.0;
                let len = 80.0 + hash(i, shift, 22) * 260.0 * rage;
                let col = if i % 3 == 0 { DON } else { 0xffffff };
                let steps = (len / 6.0) as usize;
                for k in 0..steps {
                    let d = inner + k as f32 * 6.0;
                    let fade = 1.0 - k as f32 / steps as f32;
                    c.rect(cx + dx * d - 1.5, cy + dy * d - 1.5, 3.0, 3.0, rgba(col, 0.5 * rage * fade));
                }
            }
        }

        // Shockwaves burst out behind the portrait on every hit.
        for &(at, kat, strength) in &HITS {
            let p = (t - at) / (350.0 + 250.0 * strength);
            if (0.0..1.0).contains(&p) {
                let col = if kat { KAT } else { DON };
                let r = ph * 0.45 + ease_out_cubic(p) * ph * (0.4 + 0.9 * strength);
                c.ring(cx, cy, r, rgba(col, (1.0 - p) * 0.85), (3.0 + 9.0 * strength) * (1.0 - p));
                if strength >= 1.0 {
                    c.ring(cx, cy, r * 0.8, rgba(0xffffff, (1.0 - p) * 0.5), 2.0);
                }
            }
        }
        // A glow that breathes with the voice.
        c.glow(cx, cy, ph * 0.4, 50.0 + 60.0 * loud, mix(rgba(DON, 0.15 + 0.5 * loud), rgba(0xFFB347, 0.6 * loud), loud));

        // The portrait: assembles from flying pixels, then gets punched at the camera on every
        // syllable, zooms and wobbles with the rage, tears on the hiss.
        let split = (glitch * 6.0).max(rage * 7.0).max(punch * 5.0);
        let scale = 1.0 + punch * 0.28 + rage * 0.12;
        let wobble = rage * (t * 0.045).sin() * 0.18;
        let mut placed = Vec::with_capacity(self.pixels.len());
        for &(x, y, col) in &self.pixels {
            let delay = ASSEMBLE.0 + (1.0 - y as f32 / gh) * 130.0 + hash(x, y, 1) * 90.0;
            let p = ((t - delay) / (ASSEMBLE.1 - ASSEMBLE.0 - 200.0)).clamp(0.0, 1.0);
            if p <= 0.0 {
                continue;
            }
            let row_shift = if glitch > 0.0 && hash(y / 3, bucket, 5) > 0.7 { (hash(y / 3, bucket, 6) - 0.5) * 48.0 * glitch } else { 0.0 };
            let ly = ((y as f32 + 0.5) * PX - ph / 2.0) * scale;
            let tx = cx + ((x as f32 + 0.5) * PX - pw / 2.0) * scale + ly * wobble + row_shift;
            let ty = cy + ly;
            let angle = hash(x, y, 2) * std::f32::consts::TAU;
            let dist = 180.0 + hash(x, y, 3) * 300.0;
            let (sx, sy) = (cx + angle.cos() * dist, cy + angle.sin() * dist * 0.7);
            let e = ease_out_back(p);
            let (px, py) = (sx + (tx - sx) * e, sy + (ty - sy) * e);
            let note = rgba(if hash(x, y, 4) < 0.5 { DON } else { KAT }, 1.0);
            let mut colour = fry(mix(note, col, ease_out_cubic((p - 0.35) / 0.65)), rage.max(punch * 0.6));
            colour[3] = (p * 3.0).min(1.0);
            // An impact frame: the face goes to a black silhouette on a white screen.
            if impact > 0.0 {
                colour = [0.0, 0.0, 0.0, 1.0];
            }
            // Slightly oversized so neighbouring pixels leave no seams.
            let s = (PX * scale + 0.4) * (0.4 + 0.6 * ease_out_cubic(p));
            placed.push((px - s / 2.0, py - s / 2.0, s, colour, p >= 1.0));
        }
        if impact > 0.0 {
            // Oversized: the screen shake mustn't uncover its edges.
            c.rect(-20.0, -20.0, w + 40.0, VIRTUAL_HEIGHT + 40.0, rgba(0xffffff, 0.9 * impact.sqrt()));
        }
        // The split colour copies all go underneath, so they only show around the edges.
        if split > 0.5 && impact <= 0.0 {
            for &(x, y, s, _, _) in placed.iter().filter(|p| p.4) {
                c.rect(x - split, y, s, s, rgba(0xff2a4a, 0.4));
                c.rect(x + split, y, s, s, rgba(0x2ad4ff, 0.4));
            }
        }
        for &(x, y, s, colour, _) in &placed {
            c.rect(x, y, s, s, colour);
        }
        if glitch > 0.0 {
            for i in 0..160 {
                let (nx, ny) = (hash(i, bucket, 7), hash(i, bucket, 8));
                let v = 0.5 + hash(i, bucket, 9) * 0.5;
                c.rect(cx - pw / 2.0 + nx * pw, cy - ph / 2.0 + ny * ph, PX, PX, [v, v, v, 0.7 * glitch]);
            }
            // Whole-screen flicker on the hiss.
            if hash(bucket, 0, 12) > 0.6 {
                c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0xffffff, 0.12 * glitch));
            }
        }

        // The title stamps in: each part slams down from huge, then shakes with the voice.
        let full: String = TITLE.iter().map(|(s, _)| *s).collect();
        let tw = Canvas::text_width(TEXT_SIZE, &full);
        let ty = cy + ph / 2.0 + 24.0;
        let mut x = cx - tw / 2.0;
        for (part, at) in TITLE {
            let pw_part = Canvas::text_width(TEXT_SIZE, part);
            let p = (t - at + 60.0) / 220.0;
            if p > 0.0 {
                let k = 1.0 + (1.0 - ease_out_cubic(p)) * 2.5;
                let size = TEXT_SIZE * k;
                let (pcx, pcy) = (x + pw_part / 2.0, ty + TEXT_SIZE / 2.0);
                let jitter = |salt: u32| (hash(salt, bucket, 13) - 0.5) * (10.0 * rage + 16.0 * glitch + 8.0 * punch);
                let (jx, jy) = (jitter(1), jitter(2));
                let a = (p * 3.0).min(1.0);
                let mut lx = pcx - Canvas::text_width(size, part) / 2.0 + jx;
                let ly = pcy - size / 2.0 + jy;
                for (i, ch) in part.chars().enumerate() {
                    let s = ch.to_string();
                    let col = if ch == '-' { 0x6f6f86 } else if i % 2 == 0 { 0xffffff } else { DON };
                    c.text(lx + 3.0, ly + 3.0, size, rgba(0x000000, 0.6 * a), &s);
                    if split > 0.5 {
                        c.text(lx - split, ly, size, rgba(0xff2a4a, 0.4 * a), &s);
                        c.text(lx + split, ly, size, rgba(0x2ad4ff, 0.4 * a), &s);
                    }
                    c.text(lx, ly, size, rgba(if impact > 0.0 { 0x000000 } else { col }, a), &s);
                    lx += Canvas::text_width(size, &s);
                }
            }
            x += pw_part;
        }

        // A little lane under the title: notes scroll in and get hit on the syllables.
        let lane_y = ty + TEXT_SIZE + 22.0;
        let lane_w = tw + 40.0;
        let lane_in = ease_out_cubic((t - 200.0) / 300.0);
        if lane_in > 0.0 {
            let lw = lane_w * lane_in;
            c.round_rect(cx - lw / 2.0, lane_y - 13.0, lw, 26.0, 13.0, rgba(0x20202b, 1.0), rgba(0x3a3a4e, 1.0), 1.5);
            let hit_x = cx - lw / 2.0 + 20.0;
            c.ring(hit_x, lane_y, 9.0, rgba(0xffffff, 0.4 * lane_in), 1.5);
            for &(at, kat, strength) in &HITS {
                // Notes travel 0.35 units per ms and flash when they reach the hit circle.
                let col = if kat { KAT } else { DON };
                let r = if strength >= 1.0 { 11.0 } else { 8.0 };
                let since = t - at;
                let nx = hit_x - since * 0.35;
                if since < 0.0 && nx < cx + lw / 2.0 - r {
                    c.circle(nx, lane_y, r, rgba(col, 1.0), rgba(0xffffff, 1.0), 1.5);
                } else if (0.0..200.0).contains(&since) {
                    c.glow(hit_x, lane_y, r + 1.0, 10.0 + 10.0 * strength, rgba(col, 1.0 - since / 200.0));
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

        // Red vignette closing in with the rage.
        let edge = rage.max(punch * 0.5);
        if edge > 0.02 {
            for i in 0..8 {
                let d = i as f32 * 7.0;
                let a = 0.12 * edge * (1.0 - i as f32 / 8.0);
                let col = rgba(0xff1a00, a);
                c.rect(0.0, d, w, 7.0, col);
                c.rect(0.0, VIRTUAL_HEIGHT - d - 7.0, w, 7.0, col);
                c.rect(d, 0.0, 7.0, VIRTUAL_HEIGHT, col);
                c.rect(w - d - 7.0, 0.0, 7.0, VIRTUAL_HEIGHT, col);
            }
        }

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

        // Shake everything (harder on the big hits and the rage), then the power-off: squash to a
        // line, shrink the line to a dot.
        let shake = punch * 12.0 + rage * 5.0 + glitch * 3.0;
        if shake > 0.1 {
            let (dx, dy) = ((t * 0.11).sin() * shake, (t * 0.17).cos() * shake);
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
