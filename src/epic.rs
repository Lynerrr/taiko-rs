//! The look of a map marked EPIC on taiko.rs (`online::Request::EpicMaps`): a prism that runs
//! along whatever it colours and drifts slowly, with the glint group names have passing over it.
//! The EPIC pill sits next to the map's status; on the selected map the music's spectrum plays
//! round it and it bumps on the beat, and landing on such a map sends one glint across the panel
//! and a handful of sparks out of the pill. Everything else stays as it was: the effects go on
//! top of the existing screens, and with animations off they stand still.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use crate::hit::Canvas;
use crate::render::{self, pixel_size, rgba, snap_to_grid};

/// The pill's word.
pub const LABEL: &str = "EPIC";
/// The status pills' height, which the EPIC pill shares.
pub const PILL_H: f32 = 13.0;
/// The star in front of the word, a pixel a unit.
const STAR: [&str; 5] = ["..#..", "..#..", "#####", ".###.", ".#.#."];
/// The spectrum round the pill: at most this many blocks out from its edge.
const AURA_BLOCKS: usize = 4;
/// How many of the spectrum's bands go round the pill (the top ones are mostly empty).
const AURA_BANDS: usize = 48;
/// The glint across the panel on landing on an epic map (ms).
const SWEEP_MS: f32 = 650.0;
/// The sparks out of the pill on landing on one.
const SPARKS: usize = 24;

/// Seconds the effects have run, for their slow motion: 0 with animations off, a still look.
pub fn secs() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    if !render::animations() {
        return 0.0;
    }
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// 0xRRGGBB of a hue (degrees), saturation and lightness (0 to 1).
fn hsl(h: f32, s: f32, l: f32) -> u32 {
    let a = s * l.min(1.0 - l);
    let f = |n: f32| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        let v = l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        (v.clamp(0.0, 1.0) * 255.0).round() as u32
    };
    (f(0.0) << 16) | (f(8.0) << 8) | f(4.0)
}

/// The prism at `u` (0 to 1 along what it colours), `secs` in: the hue turns most of the way
/// round along the shape and drifts on slowly, soft enough to sit on the dark panels.
pub fn prism_at(u: f32, secs: f32) -> u32 {
    hsl((u * 280.0 + secs * 30.0).rem_euclid(360.0), 0.72, 0.68)
}

/// The prism at `u` now.
pub fn prism(u: f32) -> u32 {
    prism_at(u, secs())
}

/// The prism at `u` with the shimmer's glint (as group members' names have it) crossing in the
/// first part of every 4.5 s, then resting.
pub fn shimmer_at(u: f32, secs: f32) -> u32 {
    let at = (secs / 4.5).fract() * 2.2 - 0.3;
    let d = (u - at) / 0.13;
    crate::menu::tint(prism_at(u, secs), 0xffffff, 0.32 * (-d * d).exp())
}

/// Recolours the shapes from `first` on (glyphs `x` to `x + w` drawn in one colour) in the
/// shimmer, by where each is across them.
fn shimmer_since(c: &mut Canvas, first: usize, x: f32, w: f32, a: f32) {
    let t = secs();
    for sh in &mut c.shapes[first..] {
        let u = ((sh.center[0] - x) / w.max(1.0)).clamp(0.0, 1.0);
        let [r, g, b, _] = rgba(shimmer_at(u, t), a);
        sh.fill = [r, g, b, sh.fill[3]];
        sh.border = [r, g, b, sh.border[3]];
    }
}

/// `s` at (`x`, `y`) in the shimmer, drawn as one text (for the layout checks and goldens).
/// Returns its width.
pub fn text(c: &mut Canvas, x: f32, y: f32, size: f32, s: &str, a: f32) -> f32 {
    let first = c.shapes.len();
    let w = c.text(x, y, size, rgba(0xffffff, a), s);
    shimmer_since(c, first, x, w, a);
    w
}

/// The EPIC pill's width.
pub fn pill_width() -> f32 {
    6.0 + 5.0 + 3.0 + Canvas::text_width(7.0, LABEL) + 6.0
}

/// The EPIC pill at (`x`, `y`), as tall as the status pills; `bump` (0 to 1) swells it a little,
/// for the beat. Returns its width.
pub fn pill(c: &mut Canvas, x: f32, y: f32, a: f32, bump: f32) -> f32 {
    let w = pill_width();
    let grow = 1.2 * bump.clamp(0.0, 1.0);
    let t = secs();
    let edge = shimmer_at(0.5, t);
    c.round_rect(
        [x - grow, y - grow],
        [w + 2.0 * grow, PILL_H + 2.0 * grow],
        PILL_H / 2.0 + grow,
        rgba(prism_at(0.3, t), 0.15 * a),
        rgba(edge, 0.9 * a),
        1.0,
    );
    let px = pixel_size(1.0);
    let star = rgba(shimmer_at(0.05, t), a);
    for (j, row) in STAR.iter().enumerate() {
        for (i, ch) in row.bytes().enumerate() {
            if ch == b'#' {
                c.rect(
                    snap_to_grid(x + 6.0 + i as f32),
                    snap_to_grid(y + 4.0 + j as f32),
                    px,
                    px,
                    star,
                );
            }
        }
    }
    text(c, x + 14.0, y + 3.0, 7.0, LABEL, a);
    w
}

/// The music's spectrum round the pill at (`x`, `y`): blocks out of its edge as the idle
/// screen's rays come out of the disc, the lows at the top middle, mirrored left and right, in
/// the prism along the edge and whiter further out. Only over the top and round the ends, a
/// crown: the line under the pill (the difficulty and its stars) sits close below it.
pub fn aura(c: &mut Canvas, x: f32, y: f32, bands: &[f32], a: f32) {
    let w = pill_width();
    let r = PILL_H / 2.0;
    let straight = w / 2.0 - r;
    let (cx, cy) = (x + w / 2.0, y + r);
    // From the top middle round to just past the end's middle.
    let half = straight + 0.6 * std::f32::consts::PI * r;
    let n = ((half / 2.8) as usize).max(6);
    let bands = &bands[..bands.len().min(AURA_BANDS)];
    if bands.is_empty() {
        return;
    }
    let (block, step, gap) = (pixel_size(1.5), 2.3, 2.6);
    let t = secs();
    for k in 0..n {
        let b0 = k * bands.len() / n;
        let b1 = ((k + 1) * bands.len() / n).max(b0 + 1).min(bands.len());
        let v = bands[b0..b1].iter().copied().fold(0.0, f32::max);
        let blocks = (v * AURA_BLOCKS as f32).round() as usize;
        if blocks == 0 {
            continue;
        }
        // Along the right half of the edge, from the top middle round to the bottom middle.
        let s = (k as f32 + 0.5) / n as f32 * half;
        let (px, py, nx, ny) = if s < straight {
            (cx + s, cy - r, 0.0, -1.0)
        } else {
            let angle = -std::f32::consts::FRAC_PI_2 + (s - straight) / r;
            let (sin, cos) = angle.sin_cos();
            (cx + straight + cos * r, cy + sin * r, cos, sin)
        };
        let u = k as f32 / n as f32;
        for j in 0..blocks.min(AURA_BLOCKS) {
            let out = gap + j as f32 * step;
            let f = j as f32 / AURA_BLOCKS as f32;
            let col = rgba(
                crate::menu::tint(prism_at(u, t), 0xffffff, f * f * 0.8),
                (0.9 - j as f32 * 0.16) * a,
            );
            let (bx, by) = (px + nx * out, py + ny * out);
            for x in [bx, 2.0 * cx - bx] {
                c.rect(
                    snap_to_grid(x - block / 2.0),
                    snap_to_grid(by - block / 2.0),
                    block,
                    block,
                    col,
                );
            }
        }
    }
}

/// A stripe of the prism down the left edge of a list row at (`x`, `y`), `h` tall.
pub fn stripe(c: &mut Canvas, x: f32, y: f32, h: f32, a: f32) {
    const PIECES: usize = 6;
    let t = secs();
    let piece = h / PIECES as f32;
    for j in 0..PIECES {
        c.rect(
            x,
            y + j as f32 * piece,
            2.0,
            piece + 0.2,
            rgba(prism_at(j as f32 / PIECES as f32 + 0.1, t), a),
        );
    }
}

/// A spark running round the edge of `[x, y, w, h]`, its tail fading behind it.
pub fn spark_round(c: &mut Canvas, [x, y, w, h]: [f32; 4], a: f32) {
    const TAIL: usize = 16;
    let per = 2.0 * (w + h);
    if per <= 0.0 {
        return;
    }
    let at = |q: f32| {
        let mut d = q.rem_euclid(1.0) * per;
        if d < w {
            return (x + d, y);
        }
        d -= w;
        if d < h {
            return (x + w, y + d);
        }
        d -= h;
        if d < w {
            return (x + w - d, y + h);
        }
        d -= w;
        (x, y + h - d)
    };
    let t = secs();
    let head = (t * 0.22).fract();
    let size = pixel_size(1.5);
    for j in 0..TAIL {
        let q = head - j as f32 * 0.005;
        let (px, py) = at(q);
        let col = if j == 0 {
            crate::menu::tint(prism_at(q, t), 0xffffff, 0.5)
        } else {
            prism_at(q, t)
        };
        c.rect(
            snap_to_grid(px - size / 2.0),
            snap_to_grid(py - size / 2.0),
            size,
            size,
            rgba(col, (1.0 - j as f32 / TAIL as f32) * 0.95 * a),
        );
    }
}

/// The colour of the idle screen's rays for band `u` (0 to 1) of an epic map.
pub fn ray(u: f32) -> u32 {
    prism(u)
}

struct Spark {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max: f32,
    u: f32,
    size: f32,
}

/// What landing on an epic map sets off: one glint across the panel and sparks out of the pill,
/// then nothing until another map is shown.
#[derive(Default)]
pub struct Burst {
    /// The map the panel shows (its file), to notice another.
    shown: Option<PathBuf>,
    sparks: Vec<Spark>,
    /// How far the glint has crossed, 0 to 1 (None: not crossing).
    sweep: Option<f32>,
}

/// A number from 0 to 1 for `n` and `salt`, the same every time (no randomness to tame in the
/// screen goldens).
fn hash01(n: u32, salt: u32) -> f32 {
    let mut h = n.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65_535.0
}

impl Burst {
    /// The panel shows map `map` now (`epic`: marked), its pill at `pill` (left, top). Another
    /// epic map than before sets off the glint and the sparks (when `animate`: the UI animations
    /// are on); another map of any kind ends them.
    pub fn show(&mut self, map: Option<&Path>, epic: bool, pill: Option<[f32; 2]>, animate: bool) {
        if self.shown.as_deref() == map {
            return;
        }
        self.shown = map.map(Path::to_path_buf);
        self.sparks.clear();
        self.sweep = None;
        let (true, Some([px, py])) = (epic && animate, pill) else {
            return;
        };
        self.sweep = Some(0.0);
        let w = pill_width();
        let (cx, cy) = (px + w / 2.0, py + PILL_H / 2.0);
        let seed = (secs() * 1000.0) as u32;
        for i in 0..SPARKS as u32 {
            let angle = hash01(seed, i * 5) * std::f32::consts::TAU;
            let speed = 22.0 + hash01(seed, i * 5 + 1) * 45.0;
            let (sin, cos) = angle.sin_cos();
            self.sparks.push(Spark {
                x: cx + cos * w * 0.3,
                y: cy,
                vx: cos * speed,
                vy: sin * speed * 0.7 - 10.0,
                life: 0.0,
                max: 0.55 + hash01(seed, i * 5 + 2) * 0.5,
                u: hash01(seed, i * 5 + 3),
                size: if hash01(seed, i * 5 + 4) < 0.3 {
                    2.0
                } else {
                    1.3
                },
            });
        }
    }

    /// Whether anything is still moving.
    #[cfg(test)]
    fn is_active(&self) -> bool {
        self.sweep.is_some() || !self.sparks.is_empty()
    }

    /// Moves it on by `dt_ms` and draws it: the glint over `panel` (`[x, y, w, h]`, clipped to
    /// it), the sparks over everything.
    pub fn draw(&mut self, c: &mut Canvas, panel: [f32; 4], dt_ms: f32) {
        let dt = dt_ms.max(0.0) / 1000.0;
        let t = secs();
        if let Some(p) = self.sweep {
            let [x, y, w, h] = panel;
            let start = c.shapes.len();
            let eased = 1.0 - (1.0 - p) * (1.0 - p);
            let gx = x - 30.0 + (w + 90.0) * eased;
            let tint = crate::menu::tint(prism_at(0.3, t), 0xffffff, 0.6);
            // A slanted band, brightest down its middle.
            for (half, a) in [(16.0, 0.04), (9.0, 0.06), (3.0, 0.08)] {
                c.rect(gx - half, y - h * 0.2, 2.0 * half, h * 1.4, rgba(tint, a));
                if let Some(sh) = c.shapes.last_mut() {
                    sh.angle = 0.35;
                }
            }
            c.clip_since(start, x, y, w, h);
            let next = p + dt * 1000.0 / SWEEP_MS;
            self.sweep = (next < 1.0).then_some(next);
        }
        self.sparks.retain_mut(|s| {
            s.life += dt;
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            s.vy += 50.0 * dt;
            s.vx *= (0.97f32).powf(dt * 60.0);
            s.life < s.max
        });
        for s in &self.sparks {
            let size = pixel_size(s.size);
            c.rect(
                snap_to_grid(s.x - size / 2.0),
                snap_to_grid(s.y - size / 2.0),
                size,
                size,
                rgba(
                    crate::menu::tint(prism_at(s.u, t), 0xffffff, 0.3),
                    1.0 - s.life / s.max,
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prism_is_soft_and_turns_along_and_over_time() {
        for i in 0..=20 {
            let c = prism_at(i as f32 / 20.0, 3.0);
            let (r, g, b) = (c >> 16, (c >> 8) & 255, c & 255);
            // Never black or white, never fully saturated: it sits on the dark panels.
            assert!(
                r.max(g).max(b) <= 0xF0 && r.min(g).min(b) >= 0x50,
                "{c:06x}"
            );
        }
        assert_ne!(prism_at(0.0, 0.0), prism_at(0.5, 0.0));
        assert_ne!(prism_at(0.0, 0.0), prism_at(0.0, 2.0));
        // A full turn of the hue comes back round.
        assert_eq!(prism_at(0.0, 0.0), prism_at(0.0, 12.0));
    }

    #[test]
    fn the_glint_passes_then_rests() {
        // In the glint's way it lightens; once it has gone, the plain prism.
        let lit = shimmer_at(0.5, 4.5 * 0.36);
        assert_ne!(lit, prism_at(0.5, 4.5 * 0.36));
        assert_eq!(shimmer_at(0.0, 4.5 * 0.9), prism_at(0.0, 4.5 * 0.9));
    }

    #[test]
    fn a_burst_goes_off_once_per_epic_map() {
        let mut b = Burst::default();
        let a = Path::new("a.osu");
        b.show(Some(a), true, Some([10.0, 10.0]), true);
        assert!(b.is_active());
        assert_eq!(b.sparks.len(), SPARKS);
        // The same map again: it carries on, not starting over.
        b.sweep = Some(0.5);
        b.show(Some(a), true, Some([10.0, 10.0]), true);
        assert_eq!(b.sweep, Some(0.5));
        // Another map, not epic: it stops.
        b.show(Some(Path::new("b.osu")), false, None, true);
        assert!(!b.is_active());
        // Landing on the epic one again sets it off again.
        b.show(Some(a), true, Some([10.0, 10.0]), true);
        assert!(b.is_active());
    }

    #[test]
    fn a_burst_runs_out() {
        let mut b = Burst::default();
        b.show(Some(Path::new("a.osu")), true, Some([10.0, 10.0]), true);
        let mut c = Canvas::default();
        for _ in 0..120 {
            b.draw(&mut c, [0.0, 0.0, 200.0, 300.0], 16.0);
        }
        assert!(!b.is_active());
    }
}
