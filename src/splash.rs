//! Startup splash: a pixel portrait assembles from flying don/kat-coloured pixels, gets hit like a
//! drum twice, "taiko-rs" drops in underneath, then it all fades into song select. Any key or
//! click skips to the fade. Settings -> Video -> *Startup splash* turns it off.

use std::time::Instant;

use crate::audio::Sfx;
use crate::render::{rgba, Canvas, VIRTUAL_HEIGHT};

const PORTRAIT: &[u8] = include_bytes!("../assets/splash.png");
/// Size of one portrait pixel and one font pixel (virtual units).
const PX: f32 = 3.0;
const TEXT_SIZE: f32 = 7.0 * 4.0;
const TITLE: &str = "taiko-rs";

/// Timeline (ms after start).
const ASSEMBLE_MS: f32 = 1000.0;
/// Drum hits: when, and whether it's a kat (blue) rather than a don (red).
const HITS: [(f32, bool); 3] = [(1050.0, false), (1300.0, true), (1550.0, false)];
const TITLE_MS: f32 = 1100.0;
const FADE_MS: f32 = 2300.0;
const FADE_LEN: f32 = 400.0;

const DON: u32 = 0xEB452C;
const KAT: u32 = 0x438EAD;
const BG: u32 = 0x121219;

pub struct Splash {
    /// The first frame drawn: the window can take a moment to appear after startup.
    started: Option<Instant>,
    /// Portrait pixels: grid position and colour.
    pixels: Vec<(u32, u32, [f32; 4])>,
    size: (u32, u32),
    /// Drum hits already played.
    hits_played: usize,
    /// Set when skipped: the fade starts from here instead.
    skipped_at: Option<f32>,
}

/// Cheap deterministic per-pixel randomness in 0..1.
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

impl Splash {
    pub fn new() -> Option<Splash> {
        let img = image::load_from_memory(PORTRAIT).ok()?.to_rgba8();
        let pixels = img
            .enumerate_pixels()
            .filter(|(_, _, p)| p[3] > 127)
            .map(|(x, y, p)| (x, y, [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, 1.0]))
            .collect();
        Some(Splash { started: None, pixels, size: img.dimensions(), hits_played: 0, skipped_at: None })
    }

    fn elapsed(&self, now: Instant) -> f32 {
        self.started.map(|s| now.saturating_duration_since(s).as_secs_f32() * 1000.0).unwrap_or(0.0)
    }

    /// Jumps to the fade-out (any key or click).
    pub fn skip(&mut self, now: Instant) {
        let t = self.elapsed(now);
        if self.skipped_at.is_none() && t < FADE_MS {
            self.skipped_at = Some(t);
        }
    }

    /// Covers the screen until the fade-out is through; false once it's over. Plays the drum hits
    /// through `sfx`.
    pub fn draw(&mut self, c: &mut Canvas, now: Instant, sfx: &dyn Fn(Sfx)) -> bool {
        self.started.get_or_insert(now);
        let t = self.elapsed(now);
        let fade_from = self.skipped_at.map(|s| s.min(FADE_MS)).unwrap_or(FADE_MS);
        let fade = ((t - fade_from) / FADE_LEN).clamp(0.0, 1.0);
        if fade >= 1.0 {
            return false;
        }
        let alpha = 1.0 - ease_out_cubic(fade);
        let w = c.width;
        c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(BG, alpha));

        while self.skipped_at.is_none() && self.hits_played < HITS.len() && t >= HITS[self.hits_played].0 {
            let kat = HITS[self.hits_played].1;
            sfx(match (kat, self.hits_played == HITS.len() - 1) {
                (false, true) => Sfx::BigDon,
                (false, false) => Sfx::Don,
                (true, _) => Sfx::Kat,
            });
            self.hits_played += 1;
        }

        let (gw, gh) = (self.size.0 as f32, self.size.1 as f32);
        let (pw, ph) = (gw * PX, gh * PX);
        let (cx, cy) = (w / 2.0, 52.0 + ph / 2.0);
        // Skipping during the build-up lands everything at once instead of mid-flight.
        let t_anim = if self.skipped_at.is_some() { t.max(FADE_MS) } else { t };

        // Each drum hit: a ring bursts out from behind the portrait and the portrait bumps.
        let mut bump = 0.0f32;
        for &(at, kat) in &HITS {
            let p = (t_anim - at) / 450.0;
            if (0.0..1.0).contains(&p) {
                let col = if kat { KAT } else { DON };
                let r = ph * 0.45 + ease_out_cubic(p) * ph * 0.5;
                c.ring(cx, cy, r, rgba(col, (1.0 - p) * 0.8 * alpha), 3.0 + 5.0 * (1.0 - p));
                c.glow(cx, cy, ph * 0.42, 40.0, rgba(col, 0.35 * (1.0 - p) * alpha));
                bump = bump.max((1.0 - p).powi(3) * 0.06);
            }
        }

        // Pixels fly in from a scattered cloud, coloured like notes, and settle into the face:
        // the bottom first, the hair last.
        let scale = 1.0 + bump;
        for &(x, y, col) in &self.pixels {
            let delay = (1.0 - y as f32 / gh) * 380.0 + hash(x, y, 1) * 220.0;
            let p = ((t_anim - delay) / (ASSEMBLE_MS - 600.0)).clamp(0.0, 1.0);
            if p <= 0.0 {
                continue;
            }
            let tx = cx + ((x as f32 + 0.5) * PX - pw / 2.0) * scale;
            let ty = cy + ((y as f32 + 0.5) * PX - ph / 2.0) * scale;
            let angle = hash(x, y, 2) * std::f32::consts::TAU;
            let dist = 160.0 + hash(x, y, 3) * 260.0;
            let (sx, sy) = (cx + angle.cos() * dist, cy + angle.sin() * dist * 0.7);
            let e = ease_out_back(p);
            let (px, py) = (sx + (tx - sx) * e, sy + (ty - sy) * e);
            let note = rgba(if hash(x, y, 4) < 0.5 { DON } else { KAT }, 1.0);
            let mut colour = mix(note, col, ease_out_cubic((p - 0.35) / 0.65));
            colour[3] = (p * 3.0).min(1.0) * alpha;
            // Slightly oversized so neighbouring pixels leave no seams.
            let s = (PX * scale + 0.4) * (0.4 + 0.6 * ease_out_cubic(p));
            c.rect(px - s / 2.0, py - s / 2.0, s, s, colour);
        }

        // The title: letters drop in one after another and bounce, every other one in red.
        let tw = Canvas::text_width(TEXT_SIZE, TITLE);
        let mut x = cx - tw / 2.0;
        let ty = cy + ph / 2.0 + 30.0;
        for (i, ch) in TITLE.chars().enumerate() {
            let s = ch.to_string();
            let adv = Canvas::text_width(TEXT_SIZE, &s);
            let p = (t_anim - TITLE_MS - i as f32 * 70.0) / 380.0;
            if p > 0.0 {
                let drop = (1.0 - ease_out_back(p)) * -60.0;
                let col = if ch == '-' { 0x6f6f86 } else if i % 2 == 0 { 0xffffff } else { DON };
                let a = (p * 4.0).min(1.0) * alpha;
                c.text(x + 2.0, ty + drop + 2.0, TEXT_SIZE, rgba(0x000000, 0.5 * a), &s);
                c.text(x, ty + drop, TEXT_SIZE, rgba(col, a), &s);
            }
            x += adv;
        }

        // A little lane under the title: three notes scroll in and get hit with the beats.
        let lane_y = ty + TEXT_SIZE + 26.0;
        let lane_w = tw + 40.0;
        let lane_x = cx - lane_w / 2.0;
        let lane_in = ease_out_cubic((t_anim - 600.0) / 400.0);
        if lane_in > 0.0 {
            let lw = lane_w * lane_in;
            c.round_rect(cx - lw / 2.0, lane_y - 13.0, lw, 26.0, 13.0, rgba(0x20202b, alpha), rgba(0x3a3a4e, alpha), 1.5);
            let hit_x = lane_x + 20.0;
            c.ring(hit_x, lane_y, 9.0, rgba(0xffffff, 0.4 * alpha * lane_in), 1.5);
            for &(at, kat) in &HITS {
                // Notes travel 0.35 units per ms and flash when they reach the hit circle.
                let col = if kat { KAT } else { DON };
                let since = t_anim - at;
                let nx = hit_x - since * 0.35;
                if since < 0.0 && nx < lane_x + lw - 8.0 {
                    c.circle(nx, lane_y, 8.0, rgba(col, alpha), rgba(0xffffff, alpha), 1.5);
                } else if (0.0..200.0).contains(&since) {
                    c.glow(hit_x, lane_y, 9.0, 14.0, rgba(col, (1.0 - since / 200.0) * alpha));
                }
            }
        }
        true
    }
}
