//! Shapeshift's setup: how often the lane takes another shape, how long it takes to morph into
//! it, and the order the shapes come in (shuffled for each map, or the player's own).

use std::sync::RwLock;

use winit::keyboard::KeyCode;

use crate::render::{accent, rgba, Canvas, Hit};

/// The shapes, by the index `game::SHAPESHIFT` uses.
pub const SHAPES: [&str; 6] = ["Zigzag", "U-Turn", "Spiral", "Loop", "Elbow", "Straight"];
const KEYS: [&str; 6] = ["zigzag", "uturn", "spiral", "loop", "elbow", "straight"];
/// Bars each shape lasts.
pub const EVERY: [usize; 4] = [4, 8, 16, 32];
/// Bars the morph into the next shape takes (at most half the shape's bars).
pub const MORPH: [f64; 4] = [0.5, 1.0, 2.0, 4.0];
const MORPH_NAMES: [&str; 4] = ["1/2 bar", "1 bar", "2 bars", "4 bars"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Index into `EVERY`.
    pub every: usize,
    /// Index into `MORPH`.
    pub morph: usize,
    /// Each map its own order of the shapes, else the order of `shapes`.
    pub shuffle: bool,
    /// The shapes (index into `SHAPES`) in the order they come, each on or off.
    pub shapes: [(u8, bool); 6],
}

impl Default for Config {
    fn default() -> Config {
        Config { every: 2, morph: 2, shuffle: true, shapes: [(0, true), (1, true), (2, true), (3, true), (4, true), (5, true)] }
    }
}

impl Config {
    /// Bars a shape lasts.
    pub fn bars(&self) -> usize {
        EVERY[self.every.min(EVERY.len() - 1)]
    }

    /// Bars the morph takes: never more than half of a shape's.
    pub fn morph_bars(&self) -> f64 {
        MORPH[self.morph.min(MORPH.len() - 1)].min(self.bars() as f64 / 2.0)
    }

    /// The shapes switched on, in order (all of them if none are).
    fn used(&self) -> Vec<u8> {
        let on: Vec<u8> = self.shapes.iter().filter(|s| s.1).map(|s| s.0).collect();
        if on.is_empty() { self.shapes.iter().map(|s| s.0).collect() } else { on }
    }

    /// The shape of each of `sections` (index into `SHAPES`). `seed` (from the map) picks the
    /// shuffled order, so a map always plays (and replays) the same way. The same shape never
    /// comes twice in a row unless it's the only one.
    pub fn sequence(&self, sections: usize, seed: u64) -> Vec<u8> {
        let used = self.used();
        let mut out: Vec<u8> = Vec::with_capacity(sections);
        let mut round = 0u64;
        while out.len() < sections {
            let mut deck = used.clone();
            if self.shuffle {
                // Fisher-Yates, a fresh order every round of the shapes.
                for i in (1..deck.len()).rev() {
                    let j = (mix(seed ^ mix(round * 31 + i as u64)) % (i as u64 + 1)) as usize;
                    deck.swap(i, j);
                }
                if deck.len() > 1 && out.last() == deck.first() {
                    deck.swap(0, 1);
                }
            }
            out.extend(deck);
            round += 1;
        }
        out.truncate(sections);
        out
    }

    /// `every=16 morph=2 order=shuffle shapes=zigzag,uturn,-spiral,...` (- is off).
    pub fn format(&self) -> String {
        let morph = MORPH[self.morph.min(MORPH.len() - 1)];
        let shapes: Vec<String> = self.shapes.iter().map(|&(s, on)| format!("{}{}", if on { "" } else { "-" }, KEYS[s as usize % KEYS.len()])).collect();
        format!("every={} morph={morph} order={} shapes={}", self.bars(), if self.shuffle { "shuffle" } else { "fixed" }, shapes.join(","))
    }

    /// Reads `format`'s line; anything missing or unknown stays as the default.
    pub fn parse(s: &str) -> Config {
        let mut c = Config::default();
        for word in s.split_whitespace() {
            match word.split_once('=') {
                Some(("every", v)) => c.every = EVERY.iter().position(|&e| v.parse() == Ok(e)).unwrap_or(c.every),
                Some(("morph", v)) => c.morph = MORPH.iter().position(|&m| v.parse() == Ok(m)).unwrap_or(c.morph),
                Some(("order", v)) => c.shuffle = v != "fixed",
                Some(("shapes", v)) => {
                    let mut shapes: Vec<(u8, bool)> = Vec::new();
                    for name in v.split(',') {
                        let (on, name) = match name.strip_prefix('-') {
                            Some(n) => (false, n),
                            None => (true, name),
                        };
                        if let Some(k) = KEYS.iter().position(|&n| n == name) {
                            if !shapes.iter().any(|s| s.0 == k as u8) {
                                shapes.push((k as u8, on));
                            }
                        }
                    }
                    // Any left out go at the end, on.
                    for k in 0..SHAPES.len() as u8 {
                        if !shapes.iter().any(|s| s.0 == k) {
                            shapes.push((k, true));
                        }
                    }
                    c.shapes.copy_from_slice(&shapes);
                }
                _ => {}
            }
        }
        c
    }
}

/// splitmix64's finaliser.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

static CURRENT: RwLock<Config> = RwLock::new(Config { every: 2, morph: 2, shuffle: true, shapes: [(0, true), (1, true), (2, true), (3, true), (4, true), (5, true)] });

/// The setup Shapeshift plays with (from the config).
pub fn set(c: Config) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = c;
}

pub fn get() -> Config {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// The rows of the setup.
const ROWS: usize = 4;
const SHAPES_ROW: usize = 3;

fn row_len(row: usize) -> usize {
    match row {
        0 => EVERY.len(),
        1 => MORPH.len(),
        2 => 2,
        _ => SHAPES.len(),
    }
}

/// The Shapeshift setup (a tab of the mod customization): a row per setting, a chip per choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub row: usize,
    pub col: usize,
}

impl Setup {
    /// Picks the choice at `row`, `col`; on the shapes row it switches the shape on or off.
    pub fn pick(c: &mut Config, row: usize, col: usize) {
        match row {
            0 => c.every = col.min(EVERY.len() - 1),
            1 => c.morph = col.min(MORPH.len() - 1),
            2 => c.shuffle = col == 0,
            _ => {
                let s = &mut c.shapes[col.min(SHAPES.len() - 1)];
                s.1 = !s.1;
            }
        }
    }

    /// Moves the selected shape `dir` places along the order. Returns whether it moved.
    pub fn move_shape(&mut self, c: &mut Config, dir: i32) -> bool {
        let to = self.col as i32 + dir;
        if self.row != SHAPES_ROW || !(0..SHAPES.len() as i32).contains(&to) {
            return false;
        }
        c.shapes.swap(self.col, to as usize);
        self.col = to as usize;
        true
    }

    /// Handles a key. Returns whether the setup changed (save it).
    pub fn key(&mut self, code: KeyCode, shift: bool, c: &mut Config) -> bool {
        match code {
            KeyCode::ArrowUp => self.row = (self.row + ROWS - 1) % ROWS,
            KeyCode::ArrowDown => self.row = (self.row + 1) % ROWS,
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let dir = if code == KeyCode::ArrowLeft { -1 } else { 1 };
                if shift {
                    return self.move_shape(c, dir);
                }
                let n = row_len(self.row) as i32;
                self.col = (self.col.min(n as usize - 1) as i32 + dir).rem_euclid(n) as usize;
                return false;
            }
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter => {
                Self::pick(c, self.row, self.col.min(row_len(self.row) - 1));
                return true;
            }
            KeyCode::KeyD => {
                *c = Config::default();
                return true;
            }
            _ => {}
        }
        self.col = self.col.min(row_len(self.row) - 1);
        false
    }

    /// Draws the setup over the mod select, like the Multiverse one.
    pub fn draw_panel(&self, c: &mut Canvas, cfg: &Config, outline: impl Fn(usize) -> Vec<(f32, f32)>) {
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.6));
        let (px, py, pw, ph) = (16.0, 44.0, w - 32.0, 410.0);
        c.round_rect(px, py, pw, ph, 10.0, rgba(0x15151d, 0.99), rgba(0x5ee6c8, 0.9), 1.5);
        let title_w = c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), "SHAPESHIFT");
        let hint = "arrows choose   Space pick   Shift + Left/Right move a shape   D defaults   Tab next tab   Esc back";
        c.text(px + 26.0 + title_w, py + 16.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit(6.0, pw - title_w - 42.0, hint));
        self.draw(c, cfg, px, py + 40.0, pw, outline);
        let (by, bw) = (py + ph - 26.0, 56.0);
        c.button(px + pw - bw - 12.0, by, bw, 16.0, "Back", 7.0, Hit::Key(KeyCode::Escape), true);
        c.button(px + pw - 2.0 * bw - 18.0, by, bw, 16.0, "Defaults", 7.0, Hit::Key(KeyCode::KeyD), false);
    }

    /// Draws the setup's rows in the panel `px, py, pw` wide (below its title).
    /// `outline(shape)` gives a shape's lane, as points to draw a thumbnail with.
    pub fn draw(&self, c: &mut Canvas, cfg: &Config, px: f32, py: f32, pw: f32, outline: impl Fn(usize) -> Vec<(f32, f32)>) {
        let x0 = px + 16.0;
        let label_w = 70.0;
        let mut hovered = None;
        let mut y = py;
        for row in 0..ROWS {
            let h = if row == SHAPES_ROW { 76.0 } else { 24.0 };
            let name = ["EVERY", "MORPH", "ORDER", "SHAPES"][row];
            c.text(x0, y + h / 2.0 - 4.0, 7.0, rgba(0x8a8aa0, 1.0), name);
            let n = row_len(row);
            let room = px + pw - 16.0 - (x0 + label_w);
            let cw = ((room - 6.0 * (n as f32 - 1.0)) / n as f32).min(if row == SHAPES_ROW { 110.0 } else { 90.0 });
            for col in 0..n {
                let cx = x0 + label_w + col as f32 * (cw + 6.0);
                let selected = self.row == row && self.col == col;
                let on = match row {
                    0 => cfg.every == col,
                    1 => cfg.morph == col,
                    2 => cfg.shuffle == (col == 0),
                    _ => cfg.shapes[col].1,
                };
                if c.hovered(cx, y, cw, h) {
                    hovered = Some((row, col));
                }
                let fill = if on { 0x1f4a44 } else if selected { 0x34344a } else { 0x1d1d27 };
                let border = if selected { accent() } else if on { 0x5ee6c8 } else { 0x2a2a38 };
                c.round_rect(cx, y, cw, h, 4.0, rgba(fill, 1.0), rgba(border, 1.0), if selected { 1.5 } else { 1.0 });
                let text = match row {
                    0 => format!("{} bars", EVERY[col]),
                    1 => MORPH_NAMES[col].to_string(),
                    2 => ["Shuffled", "In order"][col].to_string(),
                    _ => format!("{}. {}", col + 1, SHAPES[cfg.shapes[col].0 as usize]),
                };
                let text_col = if on { 0xffffff } else { 0x8a8aa0 };
                if row == SHAPES_ROW {
                    c.text_centered(cx + cw / 2.0, y + h - 13.0, 7.0, rgba(text_col, 1.0), &Canvas::fit(7.0, cw - 6.0, &text));
                    thumbnail(c, &outline(cfg.shapes[col].0 as usize), cx + 4.0, y + 4.0, cw - 8.0, h - 22.0, if on { 0x5ee6c8 } else { 0x4a4a5e });
                } else {
                    c.text_centered(cx + cw / 2.0, y + h / 2.0 - 3.5, 7.0, rgba(text_col, 1.0), &Canvas::fit(7.0, cw - 6.0, &text));
                }
                c.hit(cx, y, cw, h, Hit::ShapeshiftCell(row, col));
            }
            y += h + 10.0;
        }
        // Moving the selected shape along the order, by mouse.
        if self.row == SHAPES_ROW {
            let bx = x0 + label_w;
            c.button(bx, y, 70.0, 16.0, "< earlier", 7.0, Hit::ShapeshiftMove(-1), false);
            c.button(bx + 76.0, y, 70.0, 16.0, "later >", 7.0, Hit::ShapeshiftMove(1), false);
        }
        let morph_note = if cfg.morph_bars() < MORPH[cfg.morph] { format!(" (at most half of {} bars)", cfg.bars()) } else { String::new() };
        let about = match hovered.unwrap_or((self.row, self.col)).0 {
            0 => "Every: how many bars the lane keeps one shape.".to_string(),
            1 => format!("Morph: how long the lane takes to bend into the next shape{morph_note}."),
            2 => "Order: Shuffled gives each map its own order (the same every time you play it); In order goes along the shapes below.".to_string(),
            _ => "Shapes: click or Space to leave one out; Shift + Left/Right moves the selected one earlier or later.".to_string(),
        };
        c.text(x0, y + 26.0, 7.0, rgba(0x9a9ab0, 1.0), &Canvas::fit_end(7.0, pw - 32.0, &about));
    }
}

/// A shape's lane in a `w` x `h` box: its points scaled to fit, as dots.
fn thumbnail(c: &mut Canvas, pts: &[(f32, f32)], x: f32, y: f32, w: f32, h: f32, col: u32) {
    if pts.is_empty() {
        return;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(px, py) in pts {
        (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
    }
    let k = (w / (x1 - x0).max(1.0)).min(h / (y1 - y0).max(1.0));
    let (ox, oy) = (x + (w - (x1 - x0) * k) / 2.0, y + (h - (y1 - y0) * k) / 2.0);
    for &(px, py) in pts {
        c.circle(ox + (px - x0) * k, oy + (py - y0) * k, 1.2, rgba(col, 1.0), rgba(0, 0.0), 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setup_round_trips_through_the_config() {
        let mut c = Config { every: 1, morph: 0, shuffle: false, ..Config::default() };
        c.shapes.swap(0, 3);
        c.shapes[1].1 = false;
        assert_eq!(Config::parse(&c.format()), c);
        assert_eq!(Config::parse(""), Config::default());
        // Unknown words and shapes are skipped; missing shapes come back at the end.
        let p = Config::parse("every=7 shapes=loop,-blob,-zigzag");
        assert_eq!(p.every, Config::default().every);
        assert_eq!(p.shapes[..3], [(3, true), (0, false), (1, true)]);
    }

    #[test]
    fn shuffled_orders_differ_by_map_and_never_repeat_a_shape() {
        let c = Config::default();
        let a = c.sequence(40, 1);
        assert_eq!(a, c.sequence(40, 1), "the same map plays the same way");
        assert_ne!(a, c.sequence(40, 2));
        assert!(a.windows(2).all(|p| p[0] != p[1]), "{a:?}");
        // Every shape comes once per round.
        let mut round: Vec<u8> = a[..6].to_vec();
        round.sort();
        assert_eq!(round, [0, 1, 2, 3, 4, 5]);
        // In order: the player's order, without the shapes left out.
        let mut fixed = Config { shuffle: false, ..c };
        fixed.shapes.swap(0, 5);
        fixed.shapes[2].1 = false;
        assert_eq!(fixed.sequence(7, 9), [5, 1, 3, 4, 0, 5, 1]);
        // Only one shape on: it stays.
        let mut one = c;
        for s in one.shapes.iter_mut().skip(1) {
            s.1 = false;
        }
        assert_eq!(one.sequence(3, 4), [0, 0, 0]);
    }

    #[test]
    fn morphs_take_at_most_half_a_shape() {
        let c = Config { every: 0, morph: 3, ..Config::default() };
        assert_eq!((c.bars(), c.morph_bars()), (4, 2.0));
        assert_eq!(Config::default().morph_bars(), 2.0);
    }

    #[test]
    fn keys_pick_toggle_and_move_shapes() {
        let mut c = Config::default();
        let mut s = Setup::default();
        assert_eq!(s.key(KeyCode::ArrowRight, false, &mut c), false);
        assert_eq!(s.key(KeyCode::Space, false, &mut c), true);
        assert_eq!(c.bars(), 8);
        s = Setup { row: SHAPES_ROW, col: 0 };
        assert_eq!(s.key(KeyCode::ArrowRight, true, &mut c), true);
        assert_eq!((c.shapes[1].0, s.col), (0, 1));
        assert_eq!(s.key(KeyCode::Space, false, &mut c), true);
        assert!(!c.shapes[1].1);
        // Not past the ends.
        s.col = 0;
        assert_eq!(s.key(KeyCode::ArrowLeft, true, &mut c), false);
    }
}
