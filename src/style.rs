//! The look the main menu set, shared by every screen: dark panels, corner brackets that slide to
//! what's selected (brighter on the beat, gold in a kiai), pills for what's on, and titles that
//! split red and cyan on a kiai's beat.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::hit::Canvas;
use crate::render::{self, pixel_size, rgba, snap_to_grid};

const GOLD: u32 = 0xFFD34D;
/// A kiai's gold, for what lights up in one.
pub const KIAI_GOLD: u32 = GOLD;
const GLITCH_RED: u32 = 0x9f2137;
const GLITCH_CYAN: u32 = 0x2a9fb5;
/// A panel: the main menu's.
pub const PANEL: u32 = 0x15151d;
pub const PANEL_EDGE: u32 = 0x3a3a4e;
/// What deletes: its button, its menu item, the edge of the box asking.
pub const DANGER: u32 = 0xff6b6b;

// Design tokens: what song select in the main menu's look (all but Legacy) is drawn with, so its
// surfaces, spacing, text, colours and motion come from one small set instead of ad-hoc values.
// Surfaces are told apart by their tone and the room between them; an outline is kept for what
// has the focus or is selected.

/// The surfaces a screen is built of, back to front: the map's picture under its dim (drawn by
/// the background, nothing to paint), what sits on it (cards, lanes, the list's window) and what
/// lifts off that (under the pointer, a field to type in).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Back,
    Raised,
    Lifted,
}

/// The raised surface's tint and how much of what's behind it shows through.
pub const SURFACE_1: u32 = 0x14121c;
pub const SURFACE_1_ALPHA: f32 = 0.8;
/// The lifted surface: the raised one about 6% lighter.
pub const SURFACE_2: u32 = 0x221f2c;
pub const SURFACE_2_ALPHA: f32 = 0.85;

/// A surface's fill (the back one is clear).
pub fn surface_fill(level: Surface) -> [f32; 4] {
    match level {
        Surface::Back => rgba(0, 0.0),
        Surface::Raised => rgba(SURFACE_1, SURFACE_1_ALPHA),
        Surface::Lifted => rgba(SURFACE_2, SURFACE_2_ALPHA),
    }
}

/// `level` over `[x, y, w, h]` with corners of `radius`: its tone alone, no edge round it.
pub fn surface(c: &mut Canvas, [x, y, w, h]: [f32; 4], radius: f32, level: Surface) {
    if level == Surface::Back {
        return;
    }
    let fill = surface_fill(level);
    c.round_rect([x, y], [w, h], radius, fill, fill, 0.0);
}

/// When the skeletons' light band set off (`skeleton`): one sweep for every block on screen.
static SKELETON_EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// How long the light band takes across the screen, and how wide it is.
const SKELETON_MS: f32 = 1400.0;
const SKELETON_BAND: f32 = 60.0;

/// Something on its way (a value being read): a block of the lifted surface, a soft light band
/// sweeping across it with the others on screen (still, with animations off).
pub fn skeleton(c: &mut Canvas, [x, y, w, h]: [f32; 4], radius: f32) {
    surface(c, [x, y, w, h], radius, Surface::Lifted);
    if !crate::render::animations() || w <= 0.0 || h <= 0.0 {
        return;
    }
    let now = crate::render::frame_now();
    let epoch = *SKELETON_EPOCH.get_or_init(|| now);
    let t = now.saturating_duration_since(epoch).as_secs_f32() * 1000.0 % SKELETON_MS;
    let mid = t / SKELETON_MS * (c.width + 2.0 * SKELETON_BAND) - SKELETON_BAND;
    if mid + SKELETON_BAND / 2.0 < x || mid - SKELETON_BAND / 2.0 > x + w {
        return;
    }
    let start = c.shapes.len();
    // Steps rising to the band's middle and down again.
    const STEPS: usize = 6;
    let step = SKELETON_BAND / STEPS as f32;
    for k in 0..STEPS {
        let d = (k as f32 + 0.5) / STEPS as f32 * 2.0 - 1.0;
        let a = 0.07 * (1.0 - d.abs());
        let sx = mid - SKELETON_BAND / 2.0 + k as f32 * step;
        c.rect(sx, y, step, h, rgba(0xffffff, a));
    }
    c.clip_since(start, x, y, w, h);
}

/// Corners, one scale for every surface of the main menu's look: the small things (key caps,
/// chips' insides, tiles) and everything a card or strip big.
pub mod radius {
    pub const SMALL: f32 = 4.0;
    pub const CARD: f32 = 8.0;
}

/// Room, on a grid of 4 units.
pub mod space {
    /// The grid every gap and inset is a step of.
    #[allow(
        dead_code,
        reason = "the song select redesign's cards are laid out on it next"
    )]
    pub const GRID: f32 = 4.0;
    /// Between two cards of a list.
    #[allow(
        dead_code,
        reason = "the song select redesign's cards are parted by it next"
    )]
    pub const CARD_GAP: f32 = 4.0;
    /// Inside a panel, from its edge to what's in it; and between two panels side by side.
    pub const PAD: f32 = 12.0;
    /// Between two sections of a panel.
    pub const SECTION: f32 = 8.0;

    /// `v` on the nearest step of the grid.
    #[allow(
        dead_code,
        reason = "the song select redesign's cards are laid out on it next"
    )]
    pub fn snap(v: f32) -> f32 {
        (v / GRID).round() * GRID
    }
}

/// The pixel font's sizes: four, so each step reads as one. Text is snapped to device pixels
/// (`render::font_px`), so every size stays crisp.
pub mod text {
    /// Small labels over values, counts, hints (in capitals where they name something).
    pub const CAPTION: f32 = 6.0;
    /// Rows, values, buttons.
    pub const BODY: f32 = 8.0;
    /// A panel's heading, a song's title in the list.
    pub const HEADING: f32 = 12.0;
    /// The one title of the screen.
    #[allow(
        dead_code,
        reason = "the song select redesign's banner draws with it next"
    )]
    pub const TITLE: f32 = 18.0;
}

/// Text's brightness, for its rank: hierarchy by brightness rather than by size.
pub mod ink {
    /// What it is: titles, values, the selection's text.
    pub const PRIMARY: u32 = 0xffffff;
    /// About what it is: artist, mapper, the rows not selected (~70%).
    pub const SECONDARY: u32 = 0xb4b2c4;
    /// What names the rest: labels, counts, hints (~45%).
    pub const CAPTION: u32 = 0x74728a;
}

/// What a colour means. The accent is the primary action and the selection, a star rating's
/// colour is the selected difficulty's, a status keeps its chip's own; the rest is neutral.
pub mod role {
    /// The neutral of a chip or bar that means nothing by its colour (a rating not selected).
    pub const NEUTRAL: u32 = 0x2c2a38;

    /// The primary action (Play) and what's selected: the accent (`selector_colour` is the same
    /// with a kiai's gold).
    pub fn primary() -> u32 {
        crate::render::accent()
    }

    /// A difficulty's rating colour: its TJA `level` or its `stars`, neutral without either.
    pub fn rating(level: Option<f32>, stars: Option<f32>) -> u32 {
        level.or(stars).map_or(NEUTRAL, crate::stars::color)
    }

    /// The selected difficulty's text in its rating colour, white without a rating.
    pub fn rating_ink(level: Option<f32>, stars: Option<f32>) -> u32 {
        match level.or(stars) {
            Some(_) => rating(level, stars),
            None => super::ink::PRIMARY,
        }
    }
}

/// How things move: three durations and two easings for entrances, `render::approach` for what
/// follows a target (scroll, selection). With animations off everything is there at once.
pub mod motion {
    /// A hover, a press, a toggle.
    #[allow(
        dead_code,
        reason = "the song select redesign's cards move with it next"
    )]
    pub const MICRO_MS: f32 = 120.0;
    /// Content coming in: details, cards, a panel's page.
    pub const CONTENT_MS: f32 = 250.0;
    /// A scene's move (the camera from the menu into song select).
    pub const SCENE_MS: f32 = 900.0;
    /// The time constant of `render::approach` for what follows a target.
    pub const FOLLOW_TAU_MS: f32 = 45.0;

    /// Quick out, long settle: entrances.
    pub fn out_quint(t: f32) -> f32 {
        1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
    }

    /// Sharper still: what has to be there at once and then settle.
    #[allow(
        dead_code,
        reason = "the song select redesign's song change moves with it next"
    )]
    pub fn out_expo(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t >= 1.0 {
            1.0
        } else {
            1.0 - 2f32.powf(-10.0 * t)
        }
    }

    /// How far an entrance started at `since` and `ms` long is, through `ease`: 1 at once with
    /// animations off.
    pub fn entrance(since: std::time::Instant, ms: f32, ease: fn(f32) -> f32) -> f32 {
        if !crate::render::animations() {
            return 1.0;
        }
        let now = crate::render::frame_now();
        ease((now.saturating_duration_since(since).as_secs_f32() * 1000.0 / ms).clamp(0.0, 1.0))
    }
}

/// Once a frame: where the music is in its beat (`phase` 0 to 1, None: nothing playing) and
/// whether it's in a kiai (the frame's `render::Theme`).
pub fn set_beat(phase: Option<f32>, kiai: bool) {
    let on = render::animations();
    render::edit_frame_theme(|t| {
        t.pulse = pulse_at(phase.filter(|_| on));
        t.kiai = if kiai && on { 1.0 } else { 0.0 };
    });
}

/// The beat's light at `phase` (0 on the beat to 1), none without one.
fn pulse_at(phase: Option<f32>) -> f32 {
    // NaN would pass through clamp into every colour lit by the beat (drawn black).
    phase
        .filter(|p| p.is_finite())
        .map_or(0.0, |p| (1.0 - p.clamp(0.0, 1.0)).powi(3))
}

/// What's left of the music's beat: 1 on it, falling to 0.
pub fn pulse() -> f32 {
    render::theme().pulse
}

/// How far into a kiai: 0 to 1.
pub fn kiai() -> f32 {
    render::theme().kiai
}

/// A button that deletes: red, and ringed in white while it has the keys (`focused`).
pub fn danger_button(
    c: &mut Canvas,
    [x, y, w, h]: [f32; 4],
    label: &str,
    hit: crate::hit::Hit,
    focused: bool,
) {
    let hover = c.hovered(x, y, w, h);
    let bg = if hover { 0x5a2026 } else { 0x3a1519 };
    let (border, width) = if focused {
        (0xffffff, 2.0)
    } else {
        (DANGER, 1.0)
    };
    let radius = (h * 0.3).min(6.0);
    c.round_rect(
        [x, y],
        [w, h],
        radius,
        rgba(bg, 1.0),
        rgba(border, 1.0),
        width,
    );
    let room = w - 8.0;
    let size = Canvas::fit_size(9.0, 7.2, room, label);
    c.text_centered(
        x + w / 2.0,
        y + (h - size) / 2.0,
        size,
        rgba(DANGER, 1.0),
        &Canvas::fit(size, room, label),
    );
    c.hit(x, y, w, h, hit);
}

/// A dark panel with the main menu's rounded edge.
pub fn panel(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    c.round_rect(
        [x, y],
        [w, h],
        10.0,
        rgba(PANEL, 0.95),
        rgba(PANEL_EDGE, 1.0),
        1.5,
    );
}

/// An on/off switch's size: the settings'.
pub const SWITCH: [f32; 2] = [22.0, 11.0];

/// An on/off switch at `[x, y]`, as in the settings: the accent with the knob right when on.
pub fn switch(c: &mut Canvas, [x, y]: [f32; 2], on: bool) {
    let [w, h] = SWITCH;
    let fill = if on { render::accent() } else { PANEL_EDGE };
    c.round_rect(
        [x, y],
        [w, h],
        h / 2.0,
        rgba(fill, 1.0),
        rgba(fill, 1.0),
        0.0,
    );
    let knob = if on { x + w - h + 1.5 } else { x + 1.5 };
    let white = rgba(0xffffff, 1.0);
    c.round_rect(
        [knob, y + 1.5],
        [h - 3.0, h - 3.0],
        (h - 3.0) / 2.0,
        white,
        white,
        0.0,
    );
}

/// The selector's colour now: the accent, or gold in a kiai.
pub fn selector_colour() -> u32 {
    if kiai() > 0.5 { GOLD } else { role::primary() }
}

/// Corner brackets round `[x, y, w, h]`, `alpha` of their strength, brighter on the beat.
/// No fill: an outline alone.
const CLEAR_FILL: [f32; 4] = [0.0; 4];

pub fn brackets(c: &mut Canvas, [x, y, w, h]: [f32; 4], alpha: f32) {
    // Bolder on a phone, to be seen from arm's length.
    let t = pixel_size(crate::mobile::pick(2.0, 3.0));
    let arm = (w.min(h) * 0.35).clamp(5.0, crate::mobile::pick(10.0, 20.0));
    let (l, r, top, bot) = (
        snap_to_grid(x),
        snap_to_grid(x + w),
        snap_to_grid(y),
        snap_to_grid(y + h),
    );
    let col = rgba(selector_colour(), alpha * (0.75 + 0.25 * pulse()));
    // Each corner a piece of one outline as round as the cards' (a little more, as it's round
    // them), cut to the corner: the brackets turn their corners round as everything else does.
    let round = (radius::SMALL + 3.0).min(arm);
    for (cx, sx) in [(l, 1.0), (r, -1.0)] {
        for (cy, sy) in [(top, 1.0), (bot, -1.0)] {
            let hx = if sx > 0.0 { cx } else { cx - arm };
            let vy = if sy > 0.0 { cy } else { cy - arm };
            let start = c.shapes.len();
            c.round_rect([l, top], [r - l, bot - top], round, CLEAR_FILL, col, t);
            c.clip_since(start, hx, vy, arm, arm);
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
    c.round_rect(
        [x + 3.0, y + 3.0],
        [w - 6.0, h - 6.0],
        7.0,
        rgba(selector_colour(), 0.06 + 0.06 * pulse()),
        rgba(0, 0.0),
        0.0,
    );
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
        let (Some(left), Some(right)) = (row.iter().position(rim), row.iter().rposition(rim))
        else {
            continue;
        };
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
    c.circle(
        mid[0],
        mid[1],
        7.0 * px,
        rgba(col, 1.0),
        rgba(col, 1.0),
        0.0,
    );
    ARTS.with(|arts| {
        let mut arts = arts.borrow_mut();
        let rects = arts.entry(col).or_insert_with(|| merged(&small_art(col)));
        for &(rx, ry, rw, rh, colour) in rects.iter() {
            c.rect(
                left + rx as f32 * px,
                top + ry as f32 * px,
                rw as f32 * px,
                rh as f32 * px,
                rgba(colour, 1.0),
            );
        }
    });
    if !lit && brightness < 1.0 {
        c.fade_since(start, 1.0, brightness);
    }
}

/// 16 x 16 pixels, a row to a string, `#` lit: the icons the main menu's notes carry, drawn bare
/// on the buttons of the screens they open.
pub type Icon = [&'static str; 16];

/// Back: an arrow left.
pub const BACK: Icon = [
    "................",
    "................",
    "......##........",
    ".....###........",
    "....###.........",
    "...###..........",
    "..###########...",
    ".############...",
    ".############...",
    "..###########...",
    "...###..........",
    "....###.........",
    ".....###........",
    "......##........",
    "................",
    "................",
];
/// Search: a magnifying glass.
pub const SEARCH: Icon = [
    "................",
    "................",
    "....#####.......",
    "...#######......",
    "..###...###.....",
    "..##.....##.....",
    "..##.....##.....",
    "..##.....##.....",
    "..###...###.....",
    "...#######......",
    "....######......",
    ".........###....",
    "..........###...",
    "...........###..",
    "............##..",
    "................",
];
/// Sort: bars getting shorter.
pub const SORT: Icon = [
    "................",
    "................",
    "................",
    ".##############.",
    ".##############.",
    "................",
    ".###########....",
    ".###########....",
    "................",
    ".########.......",
    ".########.......",
    "................",
    ".#####..........",
    ".#####..........",
    "................",
    "................",
];
/// A collection: a folder.
pub const FOLDER: Icon = [
    "................",
    "................",
    "................",
    ".######.........",
    ".#######........",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    "................",
    "................",
    "................",
];
/// Mods: a four-pointed spark.
pub const SPARK: Icon = [
    "................",
    ".......##.......",
    ".......##.......",
    "......####......",
    "......####......",
    ".....######.....",
    "...##########...",
    ".##############.",
    ".##############.",
    "...##########...",
    ".....######.....",
    "......####......",
    "......####......",
    ".......##.......",
    ".......##.......",
    "................",
];
/// Random: a die.
pub const DICE: Icon = [
    "................",
    "................",
    "..############..",
    ".##############.",
    ".##..######..##.",
    ".##..######..##.",
    ".##############.",
    ".######..######.",
    ".######..######.",
    ".##############.",
    ".##..######..##.",
    ".##..######..##.",
    ".##############.",
    "..############..",
    "................",
    "................",
];
const DICE_ONE: Icon = [
    "................",
    "................",
    "..############..",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".######..######.",
    ".######..######.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    "..############..",
    "................",
    "................",
];
const DICE_TWO: Icon = [
    "................",
    "................",
    "..############..",
    ".##############.",
    ".##..##########.",
    ".##..##########.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##########..##.",
    ".##########..##.",
    ".##############.",
    "..############..",
    "................",
    "................",
];
const DICE_THREE: Icon = [
    "................",
    "................",
    "..############..",
    ".##############.",
    ".##..##########.",
    ".##..##########.",
    ".##############.",
    ".######..######.",
    ".######..######.",
    ".##############.",
    ".##########..##.",
    ".##########..##.",
    ".##############.",
    "..############..",
    "................",
    "................",
];
const DICE_FOUR: Icon = [
    "................",
    "................",
    "..############..",
    ".##############.",
    ".##..######..##.",
    ".##..######..##.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##############.",
    ".##..######..##.",
    ".##..######..##.",
    ".##############.",
    "..############..",
    "................",
    "................",
];
const DICE_SIX: Icon = [
    "................",
    "................",
    "..############..",
    ".##############.",
    ".##..######..##.",
    ".##..######..##.",
    ".##############.",
    ".##..######..##.",
    ".##..######..##.",
    ".##############.",
    ".##..######..##.",
    ".##..######..##.",
    ".##############.",
    "..############..",
    "................",
    "................",
];
/// The die's faces as it rolls (Random pressed), the five (`DICE`) where it stops.
pub const DICE_ROLL: [&Icon; 5] = [&DICE_THREE, &DICE_SIX, &DICE_TWO, &DICE_FOUR, &DICE_ONE];
/// Rankings: a podium.
pub const PODIUM: Icon = [
    "................",
    "................",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
    "......####......",
    ".####.####......",
    ".####.####......",
    ".####.####.####.",
    ".####.####.####.",
    ".####.####.####.",
    ".####.####.####.",
    ".####.####.####.",
    "................",
    "................",
];
/// Play: the main menu's own.
pub const PLAY: Icon = [
    "................",
    "...##...........",
    "...####.........",
    "...######.......",
    "...########.....",
    "...##########...",
    "...############.",
    "...#############",
    "...#############",
    "...############.",
    "...##########...",
    "...########.....",
    "...######.......",
    "...####.........",
    "...##...........",
    "................",
];

/// An icon's lit pixels as rectangles (x, y, w, h in pixels): runs grown down while the rows
/// below repeat them, so no seams show between rows.
/// Rectangles of an icon's lit pixels: x, y, width and height in pixels.
type IconRects = Vec<(usize, usize, usize, usize)>;

fn icon_rects(icon: &Icon) -> IconRects {
    let lit = |x: usize, y: usize| icon[y].as_bytes().get(x) == Some(&b'#');
    let mut done = [[false; 16]; 16];
    let mut out = Vec::new();
    for y in 0..16 {
        let mut x = 0;
        while x < 16 {
            if !lit(x, y) || done[y][x] {
                x += 1;
                continue;
            }
            let mut end = x + 1;
            while end < 16 && lit(end, y) && !done[y][end] {
                end += 1;
            }
            let mut h = 1;
            while y + h < 16 && (x..end).all(|k| lit(k, y + h) && !done[y + h][k]) {
                h += 1;
            }
            for row in &mut done[y..y + h] {
                row[x..end].fill(true);
            }
            out.push((x, y, end - x, h));
            x = end;
        }
    }
    out
}

thread_local! {
    static ICONS: RefCell<HashMap<usize, IconRects>> = RefCell::new(HashMap::new());
}

/// `icon` bare, `size` across, its middle at (`cx`, `cy`): its pixels in `col` over a dark
/// shadow a pixel down and right, as on the menu's notes.
pub fn icon(
    c: &mut Canvas,
    icon: &'static Icon,
    [cx, cy]: [f32; 2],
    size: f32,
    col: u32,
    alpha: f32,
) {
    if alpha <= 0.0 || size <= 0.0 {
        return;
    }
    let px = pixel_size(size / 16.0);
    let (left, top) = (snap_to_grid(cx - 8.0 * px), snap_to_grid(cy - 8.0 * px));
    ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        let rects = icons
            .entry(std::ptr::from_ref(icon) as usize)
            .or_insert_with(|| icon_rects(icon));
        for (fill, by) in [(rgba(0x000000, 0.55 * alpha), px), (rgba(col, alpha), 0.0)] {
            for &(x, y, w, h) in rects.iter() {
                c.rect(
                    left + x as f32 * px + by,
                    top + y as f32 * px + by,
                    w as f32 * px,
                    h as f32 * px,
                    fill,
                );
            }
        }
    });
}

/// How big a bare icon button's icon and label are, and how far apart.
const ICON_BUTTON_ICON: f32 = 11.0;
const ICON_BUTTON_GAP: f32 = 6.0;

/// The width of `icon_button` with this label (and caption).
pub fn icon_button_width(label: &str, caption: Option<&str>, size: f32) -> f32 {
    let text = caption
        .map_or(0.0, |c| Canvas::text_width(5.0, c))
        .max(Canvas::text_width(size, label));
    ICON_BUTTON_ICON + ICON_BUTTON_GAP + text + 12.0
}

/// A button as the main menu draws its notes' icons: `icon` bare and its `label` beside it,
/// no box round them; under the pointer the label lights white and the selector's brackets close
/// round it. `caption`: a small grey word above the label (what the label is the value of).
/// `lit`: what it sets is on (gold). Its middle is at `y`, its left edge at `x`. Returns its width.
#[allow(clippy::too_many_arguments)]
pub fn icon_button(
    c: &mut Canvas,
    [x, y]: [f32; 2],
    icon_art: &'static Icon,
    label: &str,
    caption: Option<&str>,
    size: f32,
    lit: bool,
    hit: crate::hit::Hit,
) -> f32 {
    let w = icon_button_width(label, caption, size);
    let rect = [x - 6.0, y - 12.0, w, 24.0];
    let hover = c.hovered(rect[0], rect[1], rect[2], rect[3]);
    let col = match (lit, hover) {
        (true, _) => GOLD,
        (false, true) => 0xffffff,
        (false, false) => 0xc8c8d8,
    };
    let ink = if hover || lit { col } else { 0xe8e8f2 };
    icon(
        c,
        icon_art,
        [x + ICON_BUTTON_ICON / 2.0, y],
        ICON_BUTTON_ICON,
        ink,
        1.0,
    );
    let lx = x + ICON_BUTTON_ICON + ICON_BUTTON_GAP;
    let split = if lit { pulse() * 1.2 } else { 0.0 };
    match caption {
        Some(cap) => {
            c.text(lx, y - 8.0, 5.0, rgba(0x6a6a88, 1.0), cap);
            glitch_text(c, lx, y - 1.0, size, col, label, split);
        }
        None => glitch_text(c, lx, y - size / 2.0, size, col, label, split),
    }
    if hover {
        brackets(c, rect, 0.9);
    }
    c.hit(rect[0], rect[1], rect[2], rect[3], hit);
    w
}

thread_local! {
    /// How lit each lane-end button is (0 to 1), easing towards whether the pointer is on it.
    static LIGHTS: RefCell<HashMap<&'static str, f32>> = RefCell::new(HashMap::new());
}

/// A strip in the menu lane's place as a surface: the raised tone alone, no lines round it, its
/// corners as round as the cards' (song select's lane of actions in the main menu's look).
pub fn lane_surface(c: &mut Canvas, rect: [f32; 4]) {
    surface(c, rect, radius::CARD, Surface::Raised);
}

/// The middle and radius at rest of the main menu's Play note at the end of a lane `[x, y, h]`
/// (where the menu's hit note lands).
pub fn lane_end_disc([x, y, h]: [f32; 3]) -> ([f32; 2], f32) {
    let r = (h * 0.34).clamp(8.0, 17.0);
    ([x + 12.0 + r, y + h / 2.0], r)
}

/// How much bigger the lane end's Play note is on the beat, and under the pointer.
pub const LANE_END_BEAT_GROW: f32 = 0.06;
pub const LANE_END_HOVER_GROW: f32 = 0.12;

/// The main action as the end of a lane: a bar of the accent colour over `[x, y, w, h]` (rounded
/// as the cards), the main menu's Play note and `label` on it in white. The note swells a little on
/// the beat; under the pointer it grows and the bar lights up and glows (no frame round it),
/// easing in and out. `disc_shown`: the note is in its place (not on its way).
pub fn lane_end_button(
    c: &mut Canvas,
    key: &'static str,
    [x, y, w, h]: [f32; 4],
    label: &str,
    hit: crate::hit::Hit,
    disc_shown: bool,
) {
    use crate::menu::tint;
    let hover = c.hovered(x, y, w, h);
    let dt = c.dt_ms;
    let g = LIGHTS.with(|l| {
        let mut l = l.borrow_mut();
        let g = l.entry(key).or_insert(0.0);
        render::approach(g, if hover { 1.0 } else { 0.0 }, dt, motion::FOLLOW_TAU_MS);
        *g
    });
    let col = role::primary();
    for k in 1..=3 {
        let by = k as f32 * 2.5 * g;
        c.round_rect(
            [x - by, y - by],
            [w + by * 2.0, h + by * 2.0],
            by,
            rgba(col, 0.12 * g),
            rgba(0, 0.0),
            0.0,
        );
    }
    let fill = tint(col, 0x000000, 0.5 - 0.5 * g);
    // Rounded as the lane it ends (its right end is off the screen).
    let r = radius::CARD;
    let fill = rgba(fill, 0.9 + 0.1 * pulse());
    c.round_rect([x, y], [w, h], r, fill, fill, 0.0);
    // Its edge only lights under the pointer: at rest the fill alone tells it apart.
    let edge = rgba(tint(col, 0xffffff, 0.35 * g), g);
    c.round_rect([x, y], [w, h], r, rgba(0, 0.0), edge, 1.5);
    let line = rgba(tint(col, 0xffffff, 0.3), 0.6 * g);
    c.rect(x + r, y + 4.0, w - 2.0 * r, 1.0, line);
    c.rect(x + r, y + h - 5.0, w - 2.0 * r, 1.0, line);
    let size = (h * 0.42).clamp(8.0, 11.0);
    let (at, r) = lane_end_disc([x, y, h]);
    if disc_shown {
        let beat = if render::animations() { pulse() } else { 0.0 };
        let grow = 1.0 + LANE_END_BEAT_GROW * beat + LANE_END_HOVER_GROW * g;
        crate::menu::note(c, crate::menu::Item::Play, at[0], at[1], r * grow);
    }
    let lx = at[0] + r + 10.0;
    let room = w - (lx - x) - 8.0;
    glitch_text(
        c,
        lx,
        y + (h - size) / 2.0,
        size,
        0xffffff,
        &Canvas::fit(size, room, label),
        pulse() * 0.8,
    );
    c.hit(x, y, w, h, hit);
}

/// A pill behind a label that's on: `col`'s fill, a faint white edge.
pub fn pill(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, col: u32) {
    c.round_rect(
        [x, y],
        [w, h],
        (h * 0.3).min(5.0),
        rgba(crate::menu::tint(col, 0x000000, 0.25), 0.95),
        rgba(0xffffff, 0.35),
        1.0,
    );
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

/// Text split red and cyan by `split` either way, the glitch of a title coming on.
pub fn glitch_text(c: &mut Canvas, x: f32, y: f32, size: f32, col: u32, text: &str, split: f32) {
    if split > 0.2 {
        c.text(x - split, y, size, rgba(GLITCH_RED, 0.9), text);
        c.text(x + split, y, size, rgba(GLITCH_CYAN, 0.9), text);
    }
    c.text(x, y, size, rgba(col, 1.0), text);
}

/// A flicker across `rect` as a picture changes: thin red and cyan bands, gone at `t` = 1.
pub fn glitch_bars(c: &mut Canvas, [x, y, w, h]: [f32; 4], t: f32) {
    let fade = (1.0 - t).clamp(0.0, 1.0);
    // Where the bands are jumps a few times on the way, the same on every run.
    let step = (t * 4.0).floor();
    for k in 0..4 {
        let r = ((k as f32 * 7.31 + step * 3.17).sin() * 43_758.547)
            .fract()
            .abs();
        let by = y + r * (h - 6.0);
        let bh = 2.0 + (k % 3) as f32 * 2.0;
        let col = if k % 2 == 0 { GLITCH_RED } else { GLITCH_CYAN };
        let dx = (r - 0.5) * 24.0;
        c.rect(
            x + dx.max(0.0),
            by,
            w - dx.abs(),
            bh,
            rgba(col, 0.35 * fade),
        );
    }
}

/// A small star-rating pill ("4.82") in the spectrum colour, right-aligned at `right`.
/// Returns its width. `None` draws a placeholder while it is being calculated.
pub fn star_pill(c: &mut Canvas, right: f32, y: f32, size: f32, stars: Option<f32>) -> f32 {
    let text = stars
        .map(|s| format!("{s:.2}"))
        .unwrap_or_else(|| "...".into());
    rating_pill(c, right, y, size, stars, &text, true)
}

/// A map's rating pill: a TJA course's `level` (its charter's, "8" or "9.5") when it has one,
/// else its star rating (`star_pill`).
pub fn map_pill(
    c: &mut Canvas,
    right: f32,
    y: f32,
    size: f32,
    level: Option<f32>,
    stars: Option<f32>,
) -> f32 {
    map_pill_lit(c, right, y, size, level, stars, true)
}

/// `map_pill`, in its rating's colour when `lit` (the selected difficulty's), else neutral:
/// the colour roles of song select in the main menu's look.
pub fn map_pill_lit(
    c: &mut Canvas,
    right: f32,
    y: f32,
    size: f32,
    level: Option<f32>,
    stars: Option<f32>,
    lit: bool,
) -> f32 {
    match level {
        Some(level) => rating_pill(c, right, y, size, Some(level), &level_text(level), lit),
        None => {
            let text = stars
                .map(|s| format!("{s:.2}"))
                .unwrap_or_else(|| "...".into());
            rating_pill(c, right, y, size, stars, &text, lit)
        }
    }
}

/// The selected difficulty's rating in the details of the main menu's look: a chip of the lifted
/// surface as round as the stat tiles, a diamond of its rating's colour, the value in the primary
/// ink (the difficulty's own chip beside it has the colour; two filled blocks of it would shout).
/// Drawn ending at `right`, as `map_pill` and as tall; gives its width.
pub fn star_chip(
    c: &mut Canvas,
    right: f32,
    y: f32,
    size: f32,
    level: Option<f32>,
    stars: Option<f32>,
) -> f32 {
    let text = match level {
        Some(level) => level_text(level),
        None => stars.map_or_else(|| "...".into(), |s| format!("{s:.2}")),
    };
    let tw = Canvas::text_width(size, &text);
    let (w, h) = (tw + size * 1.6 + 6.0, size + 6.0);
    let x = right - w;
    surface(c, [x, y - 3.0, w, h], radius::SMALL, Surface::Lifted);
    let col = role::rating(level, stars);
    let d = size * 0.32;
    let (dx, dy) = (x + size * 0.6 + 2.0, y - 3.0 + h / 2.0);
    let fill = rgba(col, 1.0);
    c.round_rect(
        [dx - d, dy - d],
        [d * 2.0, d * 2.0],
        d * 0.4,
        fill,
        fill,
        0.0,
    );
    c.text(
        x + size * 1.3 + 4.0,
        y,
        size,
        rgba(ink::PRIMARY, 1.0),
        &text,
    );
    w
}

/// A TJA level as charters write it: whole, or with the fraction it was given.
pub fn level_text(level: f32) -> String {
    let text = format!("{level:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn rating_pill(
    c: &mut Canvas,
    right: f32,
    y: f32,
    size: f32,
    stars: Option<f32>,
    text: &str,
    lit: bool,
) -> f32 {
    let text = text.to_string();
    let tw = Canvas::text_width(size, &text);
    let (w, h) = (tw + size * 1.6 + 6.0, size + 6.0);
    let x = right - w;
    let (bg, fg) = match stars {
        Some(_) if !lit => (role::NEUTRAL, ink::SECONDARY),
        Some(s) => {
            let col = role::rating(None, Some(s));
            // Dark text on the light end of the spectrum, light text on the dark end.
            let luma = 0.299 * (col >> 16 & 0xff) as f32
                + 0.587 * (col >> 8 & 0xff) as f32
                + 0.114 * (col & 0xff) as f32;
            (col, if luma > 140.0 { 0x15151d } else { 0xffffff })
        }
        None => (0x2a2a38, 0x8a8aa0),
    };
    c.round_rect(
        [x, y - 3.0],
        [w, h],
        h / 2.0,
        rgba(bg, 1.0),
        rgba(bg, 1.0),
        0.0,
    );
    // A small diamond stands in for the star glyph the pixel font doesn't have.
    let d = size * 0.32;
    let (dx, dy) = (x + size * 0.6 + 2.0, y - 3.0 + h / 2.0);
    c.round_rect(
        [dx - d, dy - d],
        [d * 2.0, d * 2.0],
        d * 0.4,
        rgba(fg, 1.0),
        rgba(fg, 1.0),
        0.0,
    );
    c.text(x + size * 1.3 + 4.0, y, size, rgba(fg, 1.0), &text);
    w
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_details_rating_is_a_quiet_chip_with_its_colour_on_a_diamond_only() {
        let mut c = Canvas::default();
        let w = star_chip(&mut c, 400.0, 100.0, 10.0, None, Some(5.36));
        let mut pill = Canvas::default();
        assert_eq!(
            w,
            map_pill(&mut pill, 400.0, 100.0, 10.0, None, Some(5.36)),
            "as wide"
        );
        let col = rgba(role::rating(None, Some(5.36)), 1.0);
        assert_eq!(c.shapes[0].fill, surface_fill(Surface::Lifted), "the chip");
        let coloured: Vec<_> = c.shapes.iter().filter(|s| s.fill == col).collect();
        assert_eq!(coloured.len(), 1, "only the diamond");
        assert!(coloured[0].half[0] < 5.0);
    }

    #[test]
    fn the_lane_of_actions_and_its_play_end_are_as_round_as_the_cards() {
        let mut c = Canvas {
            width: 853.3,
            ..Canvas::default()
        };
        lane_surface(&mut c, [480.0, 400.0, 375.0, 50.0]);
        let strip = c.shapes.len();
        lane_end_button(
            &mut c,
            "round-test",
            [700.0, 400.0, 129.3, 50.0],
            "Play",
            crate::hit::Hit::Key(winit::keyboard::KeyCode::Enter),
            true,
        );
        assert_eq!(c.shapes[0].params[0], radius::CARD, "the lane");
        // The Play bar: the widest shape from its left edge.
        let bar = c.shapes[strip..]
            .iter()
            .filter(|s| (s.center[0] - s.half[0] - 700.0).abs() < 0.01)
            .max_by(|a, b| a.half[0].total_cmp(&b.half[0]))
            .expect("the Play bar");
        assert_eq!(bar.params[0], radius::CARD, "Play");
    }

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
        assert_eq!(
            slide(from, to, 16.0, false),
            to,
            "without animations: there"
        );
        assert_eq!(
            slide(from, [10.0, 900.0, 100.0, 20.0], 16.0, true)[1],
            900.0,
            "from far: at once"
        );
    }

    #[test]
    fn a_small_note_is_round_and_its_rectangles_cover_it_exactly() {
        let art = small_art(0xEB452C);
        // Round: as wide across the middle as from top to bottom, and its corners empty.
        let row = |y: usize| {
            art[y]
                .iter()
                .filter(|&&p| p != 0 && p != GLITCH_RED && p != GLITCH_CYAN)
                .count()
        };
        let column = |x: usize| (0..SMALL).filter(|&y| art[y][x] != 0).count();
        assert_eq!(
            row(11),
            column(11),
            "as wide as it is tall (a row without the glitch)"
        );
        assert!(
            art[0][0] == 0
                && art[0][SMALL - 1] == 0
                && art[SMALL - 1][0] == 0
                && art[SMALL - 1][SMALL - 1] == 0
        );
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
        let runs: usize = art
            .iter()
            .map(|r| r.windows(2).filter(|w| w[0] != w[1]).count() + 1)
            .sum();
        assert!(
            rects.len() < runs,
            "{} rectangles for {runs} row runs",
            rects.len()
        );
    }

    #[test]
    fn a_bare_icons_rectangles_cover_its_lit_pixels_exactly() {
        for art in [&BACK, &SORT, &FOLDER, &SPARK, &DICE, &PODIUM, &PLAY] {
            assert!(art.iter().all(|row| row.len() == 16), "16 pixels a row");
            let mut painted = [[false; 16]; 16];
            for (x, y, w, h) in icon_rects(art) {
                for row in &mut painted[y..y + h] {
                    for p in &mut row[x..x + w] {
                        assert!(!*p, "painted twice");
                        *p = true;
                    }
                }
            }
            for (y, row) in art.iter().enumerate() {
                for (x, b) in row.bytes().enumerate() {
                    assert_eq!(painted[y][x], b == b'#', "pixel {x},{y}");
                }
            }
        }
    }

    /// The box round the white of the Play note's rim at a lane end (its text left out).
    fn note_box(c: &Canvas) -> [f32; 4] {
        let in_text = |i: usize| c.texts.iter().any(|(r, _)| r.contains(&i));
        let white = rgba(0xffffff, 1.0);
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for (i, s) in c.shapes.iter().enumerate() {
            if in_text(i) || s.fill != white {
                continue;
            }
            for k in 0..2 {
                lo[k] = lo[k].min(s.center[k] - s.half[k]);
                hi[k] = hi[k].max(s.center[k] + s.half[k]);
            }
        }
        [lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]]
    }

    fn lane_end(mouse: (f32, f32)) -> Canvas {
        let mut c = Canvas {
            width: 853.3,
            dt_ms: 1000.0,
            mouse,
            audit: true,
            ..Default::default()
        };
        lane_end_button(
            &mut c,
            "test-play-note",
            [700.0, 408.0, 129.3, 50.0],
            "Play",
            crate::hit::Hit::ModsButton,
            true,
        );
        c
    }

    #[test]
    fn the_lane_end_holds_the_menus_play_note_which_grows_under_the_pointer() {
        let ([mx, my], r) = lane_end_disc([700.0, 408.0, 50.0]);
        let away = note_box(&lane_end((0.0, 0.0)));
        let mid = [away[0] + away[2] / 2.0, away[1] + away[3] / 2.0];
        assert!(
            (mid[0] - mx).abs() < 2.0 && (mid[1] - my).abs() < 2.0,
            "{away:?} round {mx},{my}"
        );
        // The white of its rim is 0.9 of the note across.
        assert!((away[2] - 1.8 * r).abs() < 3.0, "{} across, r {r}", away[2]);
        let over = note_box(&lane_end((760.0, 430.0)));
        let grown = over[2] / away[2];
        assert!(
            (grown - (1.0 + LANE_END_HOVER_GROW)).abs() < 0.06,
            "under the pointer: {grown}"
        );
        let c = lane_end((0.0, 0.0));
        let [_, label] = c
            .texts
            .iter()
            .find(|(_, t)| t == "Play")
            .map(|(r, _)| [c.shapes[r.start].center[0], c.shapes[r.start].center[0]])
            .expect("its label");
        assert!(label > away[0] + away[2], "the label after the note");
    }

    #[test]
    fn the_lane_ends_play_note_swells_on_the_beat() {
        let theme = |pulse: f32| render::Theme {
            accent: 0x13c4a7,
            animations: true,
            legacy_menu: false,
            pulse,
            kiai: 0.0,
        };
        render::set_frame_theme(Some(theme(0.0)));
        let rest = note_box(&lane_end((0.0, 0.0)));
        render::set_frame_theme(Some(theme(1.0)));
        let beat = note_box(&lane_end((0.0, 0.0)));
        render::set_frame_theme(None);
        let grown = beat[2] / rest[2];
        assert!(
            grown > 1.0 + LANE_END_BEAT_GROW / 2.0 && grown < 1.0 + LANE_END_BEAT_GROW * 2.0,
            "on the beat: {grown}"
        );
    }

    #[test]
    fn the_lane_end_lights_up_under_the_pointer_with_no_frame() {
        let draw = |mouse: (f32, f32)| {
            let mut c = Canvas {
                width: 853.3,
                dt_ms: 1000.0,
                mouse,
                ..Default::default()
            };
            lane_end_button(
                &mut c,
                "test-play",
                [700.0, 440.0, 129.3, 26.0],
                "Play",
                crate::hit::Hit::ModsButton,
                true,
            );
            c
        };
        let away = draw((0.0, 0.0));
        let over = draw((760.0, 450.0));
        // No brackets: the same shapes, only brighter.
        assert_eq!(away.shapes.len(), over.shapes.len());
        let lum = |c: &Canvas| c.shapes.iter().map(|s| s.fill[0] * s.fill[3]).sum::<f32>();
        assert!(lum(&over) > lum(&away), "lit under the pointer");
        assert_eq!(over.hits.len(), 1);
        assert!(
            over.hits[0].rect[0] == 700.0 && over.hits[0].rect[2] == 129.3,
            "as wide as it's given (to the screen's margin), not to the edge"
        );
    }

    #[test]
    fn a_rating_pill_is_in_its_colour_only_when_lit() {
        let fills = |lit: bool| {
            let mut c = Canvas::default();
            map_pill_lit(&mut c, 100.0, 10.0, 8.0, None, Some(5.2), lit);
            c.shapes.first().map(|s| s.fill).expect("a pill")
        };
        assert_eq!(fills(true), rgba(crate::stars::color(5.2), 1.0));
        assert_eq!(fills(false), rgba(role::NEUTRAL, 1.0));
        // The same width either way: a row doesn't move when its difficulty is selected.
        let width = |lit: bool| {
            map_pill_lit(
                &mut Canvas::default(),
                100.0,
                10.0,
                8.0,
                None,
                Some(5.2),
                lit,
            )
        };
        assert_eq!(width(true), width(false));
    }

    #[test]
    fn the_easings_run_from_nothing_to_all() {
        for ease in [motion::out_quint, motion::out_expo] {
            assert_eq!(ease(0.0), 0.0);
            assert_eq!(ease(1.0), 1.0);
            assert!(ease(0.5) > 0.5, "out: past halfway at half the time");
            assert_eq!(ease(2.0), 1.0);
        }
        assert_eq!(space::snap(13.0), 12.0);
        assert_eq!(space::snap(14.5), 16.0);
    }

    #[test]
    fn the_beat_lights_on_it_and_fades() {
        assert_eq!(pulse_at(Some(0.0)), 1.0);
        assert!(pulse_at(Some(0.5)) < 0.2);
        assert_eq!(pulse_at(Some(1.0)), 0.0);
        assert_eq!(pulse_at(None), 0.0);
    }
}
