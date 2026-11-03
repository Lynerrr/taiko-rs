//! The look the main menu set, shared by every screen: dark panels, corner brackets that slide to
//! what's selected (brighter on the beat, gold in a kiai), pills for what's on, and titles that
//! split red and cyan on a kiai's beat.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::render::{self, pixel_size, rgba, snap_to_grid, Canvas};

const GOLD: u32 = 0xFFD34D;
const GLITCH_RED: u32 = 0x9f2137;
const GLITCH_CYAN: u32 = 0x2a9fb5;
/// A panel: the main menu's.
pub const PANEL: u32 = 0x15151d;
pub const PANEL_EDGE: u32 = 0x3a3a4e;

/// What's left of the music's beat (1 on it, falling to 0) and how far into a kiai (0 to 1), set
/// once a frame.
static PULSE: AtomicU32 = AtomicU32::new(0);
static KIAI: AtomicU32 = AtomicU32::new(0);

/// Once a frame: where the music is in its beat (`phase` 0 to 1, None: nothing playing) and
/// whether it's in a kiai.
pub fn set_beat(phase: Option<f32>, kiai: bool) {
    let pulse = pulse_at(phase.filter(|_| render::animations()));
    PULSE.store(pulse.to_bits(), Ordering::Relaxed);
    KIAI.store(if kiai && render::animations() { 1.0f32 } else { 0.0 }.to_bits(), Ordering::Relaxed);
}

/// The beat's light at `phase` (0 on the beat to 1), none without one.
fn pulse_at(phase: Option<f32>) -> f32 {
    // NaN would pass through clamp into every colour lit by the beat (drawn black).
    phase.filter(|p| p.is_finite()).map_or(0.0, |p| (1.0 - p.clamp(0.0, 1.0)).powi(3))
}

pub fn pulse() -> f32 {
    f32::from_bits(PULSE.load(Ordering::Relaxed))
}

pub fn kiai() -> f32 {
    f32::from_bits(KIAI.load(Ordering::Relaxed))
}

/// A dark panel with the main menu's rounded edge.
pub fn panel(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    c.round_rect([x, y], [w, h], 10.0, rgba(PANEL, 0.95), rgba(PANEL_EDGE, 1.0), 1.5);
}

/// The selector's colour now: the accent, or gold in a kiai.
pub fn selector_colour() -> u32 {
    if kiai() > 0.5 { GOLD } else { render::accent() }
}

/// Corner brackets round `[x, y, w, h]`, `alpha` of their strength, brighter on the beat.
pub fn brackets(c: &mut Canvas, [x, y, w, h]: [f32; 4], alpha: f32) {
    let t = pixel_size(2.0);
    let arm = (w.min(h) * 0.35).clamp(5.0, 10.0);
    let (l, r, top, bot) = (snap_to_grid(x), snap_to_grid(x + w), snap_to_grid(y), snap_to_grid(y + h));
    let col = rgba(selector_colour(), alpha * (0.75 + 0.25 * pulse()));
    for (cx, sx) in [(l, 1.0), (r, -1.0)] {
        for (cy, sy) in [(top, 1.0), (bot, -1.0)] {
            let hx = if sx > 0.0 { cx } else { cx - arm };
            let vy = if sy > 0.0 { cy } else { cy - arm };
            c.rect(hx, if sy > 0.0 { cy } else { cy - t }, arm, t, col);
            c.rect(if sx > 0.0 { cx } else { cx - t }, vy, t, arm, col);
        }
    }
}

thread_local! {
    /// Where each screen's selector is on its way to its target.
    static SLIDES: RefCell<HashMap<&'static str, [f32; 4]>> = RefCell::new(HashMap::new());
}

/// The selector of the list `key`: brackets that slide from wherever they were to `target`
/// (`[x, y, w, h]`). Returns where they are now.
pub fn selector(c: &mut Canvas, key: &'static str, target: [f32; 4]) -> [f32; 4] {
    let now = SLIDES.with(|s| {
        let mut s = s.borrow_mut();
        let at = s.entry(key).or_insert(target);
        *at = slide(*at, target, c.dt_ms, render::animations());
        *at
    });
    // A soft light of its colour over what it's on, breathing with the beat.
    let [x, y, w, h] = now;
    c.round_rect([x + 3.0, y + 3.0], [w - 6.0, h - 6.0], 7.0, rgba(selector_colour(), 0.06 + 0.06 * pulse()), rgba(0, 0.0), 0.0);
    brackets(c, now, 1.0);
    now
}

/// `at` a frame (`dt_ms`) further on its way to `target`; there at once from far away (another
/// screen's layout, a jump round the end) or without animations.
fn slide(mut at: [f32; 4], target: [f32; 4], dt_ms: f32, animate: bool) -> [f32; 4] {
    let far = (at[1] - target[1]).abs() > 200.0 || (at[0] - target[0]).abs() > 400.0;
    for (v, goal) in at.iter_mut().zip(target) {
        if far || !animate {
            *v = goal;
        } else {
            // `render::approach`'s easing, without its animations switch (the caller's).
            *v += (goal - *v) * (1.0 - (-dt_ms / 40.0).exp());
        }
    }
    at
}

/// A small note as pixel art, like the main menu's: a dark outline, a white rim lit top left and
/// shaded bottom right, the logo's red and cyan glitch down its sides. `SMALL` pixels square:
/// enough for a round outline that still shows its pixels.
const SMALL: usize = 22;
/// Its radius in pixels, and where its middle is.
const SMALL_R: f32 = 10.0;
const SMALL_MID: f32 = 10.5;
type SmallArt = [[u32; SMALL]; SMALL];

fn small_art(fill: u32) -> SmallArt {
    use crate::menu::tint;
    const OUTLINE: u32 = 0x0f0f16;
    let mut art = [[0; SMALL]; SMALL];
    for (y, row) in art.iter_mut().enumerate() {
        for (x, px) in row.iter_mut().enumerate() {
            let (dx, dy) = (x as f32 - SMALL_MID, y as f32 - SMALL_MID);
            let d = dx.hypot(dy);
            let slant = dx + dy;
            *px = if d > SMALL_R {
                0
            } else if d > 8.9 {
                OUTLINE
            } else if d > 7.0 {
                if slant > 6.5 { 0xc8c8d8 } else { 0xffffff }
            } else if d > 5.8 && slant < -3.5 {
                tint(fill, 0xffffff, 0.3)
            } else if d > 5.8 && slant > 3.5 {
                tint(fill, 0x000000, 0.28)
            } else {
                fill
            };
        }
    }
    // The glitch: a few rows pushed out, red on the left and cyan on the right.
    for (y, by) in [(6, 1), (10, 2), (15, 1)] {
        let row = &mut art[y];
        let rim = |p: &u32| *p != 0 && *p != OUTLINE;
        let (Some(left), Some(right)) = (row.iter().position(rim), row.iter().rposition(rim)) else { continue };
        row[left.saturating_sub(by + 1)..left].fill(GLITCH_RED);
        row[right + 1..(right + 2 + by).min(SMALL)].fill(GLITCH_CYAN);
    }
    art
}

/// `art` as as few rectangles as it takes (x, y, w, h in pixels, colour): runs of a colour, grown
/// down while the rows below repeat them. Drawn row by row, the seams between the rows showed.
/// Rectangles of a note's pixels: x, y, width and height in pixels, and colour.
type Rects = Vec<(usize, usize, usize, usize, u32)>;

fn merged(art: &SmallArt) -> Rects {
    let mut done = [[false; SMALL]; SMALL];
    let mut out = Vec::new();
    for y in 0..SMALL {
        let mut x = 0;
        while x < SMALL {
            let col = art[y][x];
            if col == 0 || done[y][x] {
                x += 1;
                continue;
            }
            let mut end = x + 1;
            while end < SMALL && art[y][end] == col && !done[y][end] {
                end += 1;
            }
            let mut h = 1;
            while y + h < SMALL && (x..end).all(|k| art[y + h][k] == col && !done[y + h][k]) {
                h += 1;
            }
            for row in &mut done[y..y + h] {
                row[x..end].fill(true);
            }
            out.push((x, y, end - x, h, col));
            x = end;
        }
    }
    out
}

thread_local! {
    static ARTS: RefCell<HashMap<u32, Rects>> = RefCell::new(HashMap::new());
}

/// A pixel note `r` in radius in `col` at (`x`, `y`), its pixels whole screen pixels. `lit`: the
/// chosen one, with a soft glow of its colour close round it; otherwise it's dimmed to
/// `brightness`.
pub fn note(c: &mut Canvas, x: f32, y: f32, r: f32, col: u32, lit: bool, brightness: f32) {
    if lit {
        c.glow(x, y, r * 1.7, r * 0.6, rgba(col, 0.35));
    }
    let start = c.shapes.len();
    let px = pixel_size(r / SMALL_R);
    let half = SMALL as f32 * px / 2.0;
    let (left, top) = (snap_to_grid(x - half), snap_to_grid(y - half));
    // A disc of the note's own colour under the pixels: where two rectangles meet, any seam shows
    // the same colour.
    let mid = [left + SMALL_MID * px, top + SMALL_MID * px];
    c.circle(mid[0], mid[1], 7.0 * px, rgba(col, 1.0), rgba(col, 1.0), 0.0);
    ARTS.with(|arts| {
        let mut arts = arts.borrow_mut();
        let rects = arts.entry(col).or_insert_with(|| merged(&small_art(col)));
        for &(rx, ry, rw, rh, colour) in rects.iter() {
            c.rect(left + rx as f32 * px, top + ry as f32 * px, rw as f32 * px, rh as f32 * px, rgba(colour, 1.0));
        }
    });
    if !lit && brightness < 1.0 {
        c.fade_since(start, 1.0, brightness);
    }
}

/// A pill behind a label that's on: `col`'s fill, a faint white edge.
pub fn pill(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, col: u32) {
    c.round_rect([x, y], [w, h], h / 2.0, rgba(crate::menu::tint(col, 0x000000, 0.25), 0.95), rgba(0xffffff, 0.35), 1.0);
}

/// A screen's title at (`x`, `y`): in a kiai, each beat splits it red and cyan for a moment.
/// Returns its width.
pub fn title(c: &mut Canvas, x: f32, y: f32, size: f32, col: u32, text: &str) -> f32 {
    let split = kiai() * pulse() * size * 0.12;
    if split > 0.2 {
        c.text(x - split, y, size, rgba(GLITCH_RED, 0.9), text);
        c.text(x + split, y, size, rgba(GLITCH_CYAN, 0.9), text);
    }
    c.text(x, y, size, rgba(col, 1.0), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beat_that_isnt_a_number_lights_nothing() {
        assert_eq!(pulse_at(Some(f32::NAN)), 0.0);
        assert_eq!(pulse_at(Some(f32::INFINITY)), 0.0);
        assert_eq!(pulse_at(Some(0.0)), 1.0);
    }

    #[test]
    fn the_selector_slides_to_its_target_and_jumps_from_far() {
        let (from, to) = ([10.0, 10.0, 100.0, 20.0], [10.0, 40.0, 100.0, 20.0]);
        let mut at = slide(from, to, 16.0, true);
        assert!(at[1] > 10.0 && at[1] < 40.0, "on its way: {at:?}");
        for _ in 0..100 {
            at = slide(at, to, 16.0, true);
        }
        assert!((at[1] - 40.0).abs() < 0.01);
        assert_eq!(slide(from, to, 16.0, false), to, "without animations: there");
        assert_eq!(slide(from, [10.0, 900.0, 100.0, 20.0], 16.0, true)[1], 900.0, "from far: at once");
    }

    #[test]
    fn a_small_note_is_round_and_its_rectangles_cover_it_exactly() {
        let art = small_art(0xEB452C);
        // Round: as wide across the middle as from top to bottom, and its corners empty.
        let row = |y: usize| art[y].iter().filter(|&&p| p != 0 && p != GLITCH_RED && p != GLITCH_CYAN).count();
        let column = |x: usize| (0..SMALL).filter(|&y| art[y][x] != 0).count();
        assert_eq!(row(11), column(11), "as wide as it is tall (a row without the glitch)");
        assert!(art[0][0] == 0 && art[0][SMALL - 1] == 0 && art[SMALL - 1][0] == 0 && art[SMALL - 1][SMALL - 1] == 0);
        // The rectangles paint every pixel once, in its colour, and there are far fewer of them.
        let rects = merged(&art);
        let mut painted = [[0u32; SMALL]; SMALL];
        for (x, y, w, h, col) in &rects {
            for row in &mut painted[*y..y + h] {
                for p in &mut row[*x..x + w] {
                    assert_eq!(*p, 0, "painted twice");
                    *p = *col;
                }
            }
        }
        assert_eq!(painted, art);
        let runs: usize = art.iter().map(|r| r.windows(2).filter(|w| w[0] != w[1]).count() + 1).sum();
        assert!(rects.len() < runs, "{} rectangles for {runs} row runs", rects.len());
    }

    #[test]
    fn the_beat_lights_on_it_and_fades() {
        assert_eq!(pulse_at(Some(0.0)), 1.0);
        assert!(pulse_at(Some(0.5)) < 0.2);
        assert_eq!(pulse_at(Some(1.0)), 0.0);
        assert_eq!(pulse_at(None), 0.0);
    }
}
