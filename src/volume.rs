//! The volume overlay: three notes on a little lane, each filled as far as its volume goes. Alt +
//! wheel turns the one picked; the pointer picks whichever note it's over, and while the lane is
//! up the wheel works over a note without Alt, like osu!. A change hits the note: it glitches like
//! the logo, its fill sloshes, and the ends of the range get a word of their own.

use std::time::{Duration, Instant};

use crate::config::Config;
use crate::game::{DON, KAT};
use crate::menu::pixel_ring;
use crate::render::{self, approach, pixel_size, rgba, snap_to_grid, Canvas, VIRTUAL_HEIGHT};

pub const MUSIC: usize = 0;
pub const MASTER: usize = 1;
pub const HITSOUNDS: usize = 2;
const NOTES: usize = 3;

const GOLD: u32 = 0xFFD34D;
const OUTLINE: u32 = 0x0f0f16;
const EMPTY: u32 = 0x23232f;
const GLITCH_RED: u32 = 0x9f2137;
const GLITCH_CYAN: u32 = 0x2a9fb5;

/// How long the lane stays up after the last change, and how long it fades.
const SHOWN: Duration = Duration::from_millis(1600);
const FADE_S: f32 = 0.3;
const SLIDE_MS: f32 = 160.0;
const GLITCH_MS: f32 = 240.0;
const SLOSH_MS: f32 = 700.0;
const BURST_MS: f32 = 380.0;
const WORD_MS: f32 = 620.0;

const CELL: f32 = 70.0;
const PANEL_W: f32 = CELL * NOTES as f32 + 16.0;
const PANEL_H: f32 = 98.0;
const NOTE_R: f32 = 24.0;
const NOTE_Y: f32 = 47.0;

fn name(note: usize) -> &'static str {
    match note {
        MUSIC => tr!("MUSIC", "音楽", "МУЗЫКА"),
        MASTER => tr!("ALL", "全体", "ВСЁ"),
        _ => tr!("HITSOUNDS", "ヒット音", "ХИТСАУНДЫ"),
    }
}

fn colour(note: usize) -> u32 {
    match note {
        MUSIC => DON,
        MASTER => GOLD,
        _ => KAT,
    }
}

fn level(cfg: &Config, note: usize) -> f32 {
    match note {
        MUSIC => cfg.music_volume,
        MASTER => cfg.master_volume,
        _ => cfg.fx_volume,
    }
}

fn level_mut(cfg: &mut Config, note: usize) -> &mut f32 {
    match note {
        MUSIC => &mut cfg.music_volume,
        MASTER => &mut cfg.master_volume,
        _ => &mut cfg.fx_volume,
    }
}

fn panel(width: f32) -> [f32; 2] {
    [width - PANEL_W - 16.0, VIRTUAL_HEIGHT - PANEL_H - 30.0]
}

/// The note at `x, y` on the lane at `[px, py]`.
fn note_in(px: f32, py: f32, x: f32, y: f32) -> Option<usize> {
    (y >= py && y <= py + PANEL_H && x >= px + 8.0 && x < px + 8.0 + CELL * NOTES as f32).then(|| (((x - px - 8.0) / CELL) as usize).min(NOTES - 1))
}

pub struct Overlay {
    /// The note the wheel turns.
    pub selected: usize,
    /// When it was last touched, and when it came up: None while it's away.
    touched: Option<Instant>,
    opened: Option<Instant>,
    /// ms it has been drawn: the animations below are timed on it.
    clock: f32,
    /// When each note was last hit.
    hit: [f32; NOTES],
    /// How full each note is drawn, catching up with its volume.
    shown: [f32; NOTES],
    /// A word over a note and when it came.
    word: Option<(usize, &'static str, f32)>,
}

impl Overlay {
    pub fn new() -> Overlay {
        Overlay { selected: MASTER, touched: None, opened: None, clock: 0.0, hit: [f32::MIN; NOTES], shown: [f32::NAN; NOTES], word: None }
    }

    pub fn visible(&self, now: Instant) -> bool {
        self.touched.is_some_and(|t| now.saturating_duration_since(t) < SHOWN)
    }

    /// Brings the lane up, or keeps it up.
    pub fn touch(&mut self, now: Instant) {
        if !self.visible(now) {
            self.opened = Some(now);
            self.hit = [self.clock; NOTES];
        }
        self.touched = Some(now);
    }

    /// The pointer moved to `x, y`: the note under it is the picked one, and the lane stays up.
    /// True if it's over a note.
    pub fn point(&mut self, x: f32, y: f32, width: f32, now: Instant) -> bool {
        if !self.visible(now) {
            return false;
        }
        let [px, py] = panel(width);
        let Some(note) = note_in(px, py, x, y) else { return false };
        self.touched = Some(now);
        if note != self.selected {
            self.selected = note;
            self.hit[note] = self.clock;
        }
        true
    }

    /// The next note along (Alt + Left / Right).
    pub fn pick_next(&mut self, dir: i32, now: Instant) {
        self.touch(now);
        self.selected = (self.selected as i32 + dir).clamp(0, NOTES as i32 - 1) as usize;
        self.hit[self.selected] = self.clock;
    }

    /// `notches` of the wheel on the selected note. True if the volume changed.
    pub fn turn(&mut self, cfg: &mut Config, notches: i32, now: Instant) -> bool {
        self.touch(now);
        let note = self.selected;
        let slot = level_mut(cfg, note);
        let was = *slot;
        *slot = (((*slot + 0.05 * notches as f32) * 20.0).round() / 20.0).clamp(0.0, 1.0);
        let turned = *slot;
        self.hit[note] = self.clock;
        if turned == was {
            return false;
        }
        self.word = if turned <= 0.0 {
            Some((note, tr!("MUTE", "ミュート", "ТИШИНА"), self.clock))
        } else if turned >= 1.0 {
            Some((note, "MAX", self.clock))
        } else {
            None
        };
        true
    }

    pub fn draw(&mut self, c: &mut Canvas, cfg: &Config, now: Instant) {
        let Some(touched) = self.touched else { return };
        self.clock += c.dt_ms;
        let [px, py] = panel(c.width);
        let left = (SHOWN.as_secs_f32() - now.saturating_duration_since(touched).as_secs_f32()).max(0.0);
        let a = (left / FADE_S).min(1.0);
        let slide = (1.0 - self.opened.map_or(1.0, |t| render::progress(t, SLIDE_MS))) * 40.0;
        let px = px + slide;
        c.round_rect([px, py], [PANEL_W, PANEL_H], 8.0, rgba(0x101018, 0.92 * a), rgba(0x3a3a4e, a), 1.0);
        self.draw_lane(c, px, py, a);
        for note in 0..NOTES {
            self.draw_cell(c, cfg, note, px + 8.0 + CELL * (note as f32 + 0.5), py, a);
        }
        if let Some((note, word, at)) = self.word {
            let k = (self.clock - at) / WORD_MS;
            if k < 1.0 {
                let cx = px + 8.0 + CELL * (note as f32 + 0.5);
                c.text_centered(cx, py - 14.0 - k * 14.0, 10.0, rgba(GOLD, a * (1.0 - k * k)), word);
            }
        }
    }

    /// The lane under the notes, its bar lines running like a song's.
    fn draw_lane(&self, c: &mut Canvas, px: f32, py: f32, a: f32) {
        const GAP: f32 = 24.0;
        let (top, h) = (py + NOTE_Y - NOTE_R - 3.0, (NOTE_R + 3.0) * 2.0);
        c.rect(px + 1.0, top, PANEL_W - 2.0, h, rgba(0x15151d, a));
        c.rect(px + 1.0, top - 1.0, PANEL_W - 2.0, 1.0, rgba(0x3a3a4e, a));
        c.rect(px + 1.0, top + h, PANEL_W - 2.0, 1.0, rgba(0x3a3a4e, a));
        let run = if render::animations() { self.clock * 0.02 % GAP } else { 0.0 };
        let mut x = px + GAP - run;
        while x < px + PANEL_W - 2.0 {
            c.rect(x, py + NOTE_Y - 4.0, 2.0, 8.0, rgba(0xffffff, 0.12 * a));
            x += GAP;
        }
    }

    fn draw_cell(&mut self, c: &mut Canvas, cfg: &Config, note: usize, cx: f32, py: f32, a: f32) {
        let picked = note == self.selected;
        let value = level(cfg, note);
        let shown = &mut self.shown[note];
        if !shown.is_finite() {
            *shown = value;
        }
        approach(shown, value, c.dt_ms, 45.0);
        let fill = *shown;
        let since = self.clock - self.hit[note];
        let fade = |ms: f32| if render::animations() { (1.0 - since / ms).clamp(0.0, 1.0) } else { 0.0 };
        let cy = py + NOTE_Y;
        let r = NOTE_R + if picked { 2.0 } else { -2.0 } + fade(BURST_MS) * 2.0;
        let label = if picked { 0xffffff } else { 0x8a8aa0 };
        let size = Canvas::fit_size(7.0, 5.0, CELL - 6.0, name(note));
        c.text_centered(cx, py + 7.0 + (7.0 - size) / 2.0, size, rgba(label, a), name(note));
        let px = pixel_size(1.5);
        if fade(BURST_MS) > 0.0 {
            let k = 1.0 - fade(BURST_MS);
            pixel_ring(c, [cx, cy], r + 2.0 + k * 16.0, px * 2.0, 0, px, rgba(colour(note), a * (1.0 - k)));
        }
        let look = Look { col: colour(note), fill, glitch: fade(GLITCH_MS), slosh: fade(SLOSH_MS), clock: self.clock, a };
        draw_note(c, cx, cy, r, &look);
        if picked {
            pixel_ring(c, [cx, cy], r + 4.0, px, 0, px, rgba(0xffffff, 0.75 * a));
        }
        c.text_centered(cx, py + 82.0, 8.0, rgba(if picked { 0xffffff } else { 0xc8c8d8 }, a), &format!("{:.0}%", value * 100.0));
    }
}

/// How a note is drawn this frame.
struct Look {
    col: u32,
    /// How full it is.
    fill: f32,
    /// How much of the hit's glitch and slosh is left (0 to 1).
    glitch: f32,
    slosh: f32,
    clock: f32,
    a: f32,
}

/// How far the glitch throws row `row` this moment, and which way.
fn glitched(row: i32, clock: f32) -> i32 {
    // A new pattern every 40 ms.
    let seed = (clock / 40.0) as u32;
    let mix = (row as u32).wrapping_mul(2_654_435_761) ^ seed.wrapping_mul(40_503);
    match (mix >> 7) % 7 {
        0 => 2,
        1 => -2,
        2 => 1,
        _ => 0,
    }
}

/// A note in pixels like the menu's: a white rim round a face filled to `look.fill`, with the
/// surface rocking after a hit and rows thrown sideways with red and cyan edges.
fn draw_note(c: &mut Canvas, cx: f32, cy: f32, r: f32, look: &Look) {
    let px = pixel_size(1.5);
    let (cx, cy) = (snap_to_grid(cx), snap_to_grid(cy));
    let outer = r / px;
    let face = outer - 3.0;
    let n = outer.ceil() as i32;
    let muted = look.fill <= 0.004;
    let a = look.a;
    for row in -n..n {
        let dy = row as f32 + 0.5;
        if dy.abs() >= outer {
            continue;
        }
        let reach = (outer * outer - dy * dy).sqrt().ceil() as i32;
        let shift = if look.glitch > 0.05 { glitched(row, look.clock) } else { 0 };
        let (left, right) = (cx + (-reach + shift) as f32 * px, cx + (reach + shift) as f32 * px);
        let y = cy + row as f32 * px;
        if shift != 0 {
            let edge = shift.unsigned_abs() as f32 * px * 2.0;
            c.rect(left - edge, y, edge, px, rgba(GLITCH_RED, a));
            c.rect(right, y, edge, px, rgba(GLITCH_CYAN, a));
        }
        let mut run: Option<(i32, u32)> = None;
        for column in -reach..=reach {
            let dx = column as f32 + 0.5;
            let d = dx.hypot(dy);
            let colour = (column < reach && d <= outer).then(|| {
                if d > outer - 1.0 {
                    OUTLINE
                } else if d > face {
                    if muted { 0x6f6f86 } else { 0xffffff }
                } else {
                    let rock = (dx * 0.45 + look.clock / 90.0).sin() * look.slosh * 2.5;
                    let surface = (1.0 - 2.0 * look.fill) * face + rock;
                    match dy - surface {
                        s if s < 0.0 => EMPTY,
                        s if s < 1.0 => crate::menu::tint(look.col, 0xffffff, 0.45),
                        _ => look.col,
                    }
                }
            });
            match (run, colour) {
                (Some((_, was)), Some(now)) if was == now => {}
                (Some((from, was)), _) => {
                    c.rect(cx + (from + shift) as f32 * px, y, (column - from) as f32 * px, px, rgba(was, a));
                    run = colour.map(|now| (column, now));
                }
                (None, Some(now)) => run = Some((column, now)),
                (None, None) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_turns_the_picked_note_in_fives_and_stops_at_the_ends() {
        let (mut v, mut cfg, now) = (Overlay::new(), Config::default(), Instant::now());
        cfg.music_volume = 0.52;
        cfg.master_volume = 0.5;
        v.selected = MUSIC;
        assert!(v.turn(&mut cfg, 1, now));
        assert_eq!(cfg.music_volume, 0.55);
        assert_eq!(cfg.master_volume, 0.5, "the others stay");
        assert!(v.turn(&mut cfg, 100, now));
        assert_eq!(cfg.music_volume, 1.0);
        assert_eq!(v.word.map(|w| w.1), Some("MAX"));
        assert!(!v.turn(&mut cfg, 1, now), "already there");
        v.turn(&mut cfg, -100, now);
        assert_eq!(cfg.music_volume, 0.0);
    }

    #[test]
    fn the_pointer_picks_the_note_it_is_over_only_while_the_lane_is_up() {
        let (mut v, now) = (Overlay::new(), Instant::now());
        let [px, py] = panel(853.3);
        let over = |k: f32| px + 8.0 + CELL * k;
        assert!(!v.point(over(0.5), py + 40.0, 853.3, now));
        assert_eq!(v.selected, MASTER, "nothing to point at yet");
        v.touch(now);
        assert!(v.point(over(0.5), py + 40.0, 853.3, now));
        assert_eq!(v.selected, MUSIC);
        assert!(v.point(over(2.5), py + 40.0, 853.3, now));
        assert_eq!(v.selected, HITSOUNDS);
        assert!(!v.point(px - 50.0, py + 40.0, 853.3, now));
        assert_eq!(v.selected, HITSOUNDS, "moving off keeps the pick");
        // Over a note it stays up; off it, it goes.
        let later = now + SHOWN - Duration::from_millis(1);
        assert!(v.point(over(0.5), py + 40.0, 853.3, later));
        assert!(v.visible(later + SHOWN - Duration::from_millis(1)));
        assert!(!v.point(over(0.5), py + 40.0, 853.3, later + SHOWN));
    }

    #[test]
    fn the_arrows_walk_along_the_notes_and_stop_at_the_last() {
        let (mut v, now) = (Overlay::new(), Instant::now());
        assert_eq!(v.selected, MASTER);
        v.pick_next(1, now);
        assert_eq!(v.selected, HITSOUNDS);
        v.pick_next(1, now);
        assert_eq!(v.selected, HITSOUNDS);
        v.pick_next(-1, now);
        v.pick_next(-1, now);
        assert_eq!(v.selected, MUSIC);
    }

    #[test]
    fn the_glitch_shifts_only_some_rows_and_moves_on() {
        let rows = |clock: f32| (-16..16).map(|r| glitched(r, clock)).collect::<Vec<_>>();
        let a = rows(0.0);
        assert!(a.iter().any(|&s| s != 0) && a.contains(&0));
        assert_ne!(a, rows(80.0));
    }
}
