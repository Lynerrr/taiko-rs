//! The main menu's easter eggs, for the secret medals (`ura`): the logo that falls apart when
//! it's tapped too much, the gold note among the background notes that has to be clicked,
//! and the menu that turns over when both rims are hit at once. They live as long as the game
//! runs, across visits to the menu (`State::menu_eggs`), so the menu stays upside down.

use std::f32::consts::PI;
use std::time::{Duration, Instant};

use crate::hit::{Canvas, Hit};
use crate::logo::Pixel;
use crate::medals::{SPLIT_CYAN, SPLIT_RED, TEAR_CYAN, TEAR_RED};
use crate::render::{self, VIRTUAL_HEIGHT, rgba};
use crate::splash::hash;

/// Taps on the logo that break it, and the most time between two of them (ms).
const LOGO_TAPS: usize = 10;
const LOGO_GAP_MS: f64 = 700.0;
/// The logo breaking (ms after the last tap): its pixels fly and fall until `FALL_MS`, it's gone
/// until `GONE_MS`, comes back together by `BACK_MS`, and the screen jolts for `CRUNCH_MS`.
const FALL_MS: f64 = 1100.0;
const GONE_MS: f64 = 1400.0;
const BACK_MS: f64 = 2000.0;
const CRUNCH_MS: f64 = 180.0;
/// What pulls the falling pixels down (units per ms²).
const GRAVITY: f32 = 0.0007;

/// Both rims at once this many times turns the menu over; a longer pause than `PAIR_GAP`
/// between two starts the count again.
const FLIP_PAIRS: u32 = 10;
const PAIR_GAP: Duration = Duration::from_secs(2);
/// How long a twitch shakes, and the turn over takes (ms).
const TWITCH_MS: f32 = 520.0;
const TURN_MS: f32 = 1100.0;

/// The stray note: how long before it comes (the first time, and again after a miss or a burst;
/// ms of menu), how soon the next one comes once one is caught, its size, how many in a row it
/// takes, and how long it may take to show before it's given up on (no song, notes off).
const FIRST_MS: (f64, f64) = (40_000.0, 120_000.0);
const AGAIN_MS: (f64, f64) = (90_000.0, 240_000.0);
const NEXT_MS: (f64, f64) = (1_500.0, 4_000.0);
const STRAY_R: f32 = 13.0;
const CATCHES: u32 = 3;
const LOST_MS: f64 = 20_000.0;
/// How long a catch short of the last bursts (ms).
const CATCH_MS: f64 = 350.0;
/// The confetti a caught note bursts into: pieces and how long they last (ms).
const CONFETTI: u32 = 48;
const CONFETTI_MS: f64 = 1500.0;

const GOLD: u32 = 0xf5c542;
const DON: u32 = 0xeb452c;
const KAT: u32 = 0x44b9e3;
const INK: u32 = 0x1c1c28;

/// Eases into place, overshooting a little first.
fn ease_out_back(k: f32) -> f32 {
    let (c1, c3) = (1.70158, 2.70158);
    let k = k - 1.0;
    1.0 + c3 * k * k * k + c1 * k * k
}

pub struct MenuEggs {
    /// Time spent in the menu (ms), the eggs' clock.
    clock: f64,
    pub logo: LogoEgg,
    pub flip: FlipEgg,
    pub stray: StrayNote,
    /// Where the menu's shapes start this frame, for the turn.
    start: usize,
}

impl MenuEggs {
    pub fn new(seed: u32) -> MenuEggs {
        let mut stray = StrayNote {
            state: Stray::Waiting { at: 0.0, caught: 0 },
            rng: seed | 1,
            pos: [0.0; 2],
            gold: Default::default(),
        };
        stray.wait(FIRST_MS, 0.0, 0);
        MenuEggs {
            clock: 0.0,
            logo: LogoEgg::default(),
            flip: FlipEgg::default(),
            stray,
            start: 0,
        }
    }

    /// Before the menu draws anything this frame.
    pub fn begin(&mut self, c: &Canvas) {
        self.clock += f64::from(c.dt_ms);
        self.start = c.shapes.len();
    }

    /// The logo of the top bar (`settings::brand_with`).
    pub fn draw_logo(&mut self, c: &mut Canvas, x: f32, y: f32, size: f32) {
        self.logo.draw(c, self.clock, x, y, size);
    }

    /// After the menu drew everything: the logo's hit area, the stray note's (where no button
    /// lies) and its burst, and the whole menu turned as the flip has it.
    pub fn end(&mut self, c: &mut Canvas) {
        let [x, y, size] = crate::settings::BRAND_LOGO;
        c.hit(x, y, size, size, Hit::MenuLogo);
        let pivot = [c.width / 2.0, VIRTUAL_HEIGHT / 2.0];
        let angle = self.flip.angle(self.clock);
        self.stray.draw(c, self.clock);
        if angle != 0.0 {
            c.turn_since(self.start, pivot, angle);
        }
        if let Some(k) = self.logo.crunch(self.clock)
            && render::animations()
        {
            c.glitch = Some([k, (self.clock / 30.0) as u32 as f32 % 4096.0]);
        }
    }

    /// A click on the logo; true when it broke.
    pub fn tap_logo(&mut self) -> bool {
        self.logo.tap(self.clock)
    }

    /// A rim hit on `side` (left -1, right 1) at `at`; true when the menu turned over.
    pub fn kat(&mut self, side: i32, at: Instant) -> bool {
        self.flip.kat(side, at, self.clock)
    }

    /// A click on the stray note; true when it was caught.
    pub fn catch_stray(&mut self) -> bool {
        self.stray.click(self.clock)
    }
}

#[derive(Default)]
pub struct LogoEgg {
    /// The taps so far (egg clock), each soon after the one before.
    taps: Vec<f64>,
    /// When it broke, while it's broken.
    broken: Option<f64>,
}

impl LogoEgg {
    fn tap(&mut self, now: f64) -> bool {
        if self.broken.is_some() {
            return false;
        }
        if self.taps.last().is_some_and(|&t| now - t > LOGO_GAP_MS) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() < LOGO_TAPS {
            return false;
        }
        self.taps.clear();
        self.broken = Some(now);
        true
    }

    /// How hard the logo glitches from the taps, 0 to 1: more with each, fading when let be.
    fn strain(&self, now: f64) -> f32 {
        let Some(&last) = self.taps.last() else {
            return 0.0;
        };
        let fade = (1.0 - (now - last) / LOGO_GAP_MS).clamp(0.0, 1.0) as f32;
        self.taps.len() as f32 / LOGO_TAPS as f32 * fade
    }

    /// The screen's jolt as it snaps back together, 1 to 0.
    fn crunch(&self, now: f64) -> Option<f32> {
        let t = now - self.broken? - BACK_MS;
        (0.0..CRUNCH_MS)
            .contains(&t)
            .then(|| 1.0 - (t / CRUNCH_MS) as f32)
    }

    fn draw(&mut self, c: &mut Canvas, now: f64, x: f32, y: f32, size: f32) {
        if let Some(at) = self.broken {
            if now - at < BACK_MS {
                return draw_broken(c, (now - at) as f32, x, y, size);
            }
            if now - at >= BACK_MS + CRUNCH_MS {
                self.broken = None;
            }
        }
        let strain = self.strain(now);
        if strain <= 0.0 || !render::animations() {
            return crate::logo::draw(c, x, y, size);
        }
        let px = size / 32.0;
        let frame = (now / 45.0) as u32;
        // Split into the logo's glitch colours behind it, further the harder it's tapped.
        for (side, colour) in [(-1.0, SPLIT_RED), (1.0, SPLIT_CYAN)] {
            let dx = side * (0.6 + 2.4 * strain) * px;
            crate::logo::draw_each(c, x + dx, y, size, |_, _, col| {
                Some(Pixel {
                    offset: [0.0; 2],
                    angle: 0.0,
                    colour: rgba(colour, col[3] * 0.55 * strain),
                })
            });
        }
        // Rows torn sideways, a few at a time, in the tear colours.
        crate::logo::draw_each(c, x, y, size, |_, row, col| {
            let band = row / 3;
            let torn = hash(band, frame, 77) < strain * 0.55;
            let shift = (hash(band, frame, 78) - 0.5) * 9.0 * strain * px;
            let colour = match (torn, hash(band, frame, 79) < 0.5) {
                (true, true) if strain > 0.5 => rgba(TEAR_RED, col[3]),
                (true, false) if strain > 0.5 => rgba(TEAR_CYAN, col[3]),
                _ => col,
            };
            Some(Pixel {
                offset: [if torn { shift } else { 0.0 }, 0.0],
                angle: 0.0,
                colour,
            })
        });
    }
}

/// The broken logo `t` ms after the last tap: pixels flung out and falling, then flying back.
fn draw_broken(c: &mut Canvas, t: f32, x: f32, y: f32, size: f32) {
    if !render::animations() {
        return;
    }
    crate::logo::draw_each(c, x, y, size, |ix, iy, colour| {
        let vx = (hash(ix, iy, 11) - 0.5) * 0.3;
        let vy = -0.05 - hash(ix, iy, 12) * 0.25;
        let spin = (hash(ix, iy, 13) - 0.5) * 0.03;
        let fall = |t: f32| ([vx * t, vy * t + 0.5 * GRAVITY * t * t], spin * t);
        let (offset, angle) = if t < FALL_MS as f32 {
            fall(t)
        } else if t < GONE_MS as f32 {
            return None;
        } else {
            // From where it fell, every pixel landing at once.
            let k = (t - GONE_MS as f32) / (BACK_MS - GONE_MS) as f32;
            let left = 1.0 - ease_out_back(k);
            let (d, a) = fall(FALL_MS as f32);
            ([d[0] * left, d[1] * left], a * left)
        };
        Some(Pixel {
            offset,
            angle,
            colour,
        })
    });
}

#[derive(Default)]
pub struct FlipEgg {
    /// The last rim hit not yet paired: its side and when.
    rim: Option<(i32, Instant)>,
    /// Both rims at once so far, and when last.
    pairs: u32,
    last_pair: Option<Instant>,
    /// When it last twitched (egg clock).
    twitched: f64,
    /// When it turned over: it stays so.
    turned: Option<f64>,
}

impl FlipEgg {
    fn kat(&mut self, side: i32, at: Instant, now: f64) -> bool {
        if self.turned.is_some() {
            return false;
        }
        // Together as a big note's two hits are.
        let window = Duration::from_secs_f64(taiko_logic::sim::STRONG_WINDOW / 1000.0);
        match self.rim.take() {
            Some((first, then))
                if first != side && at.saturating_duration_since(then) <= window =>
            {
                if self
                    .last_pair
                    .is_some_and(|p| at.saturating_duration_since(p) > PAIR_GAP)
                {
                    self.pairs = 0;
                }
                self.pairs += 1;
                self.last_pair = Some(at);
                self.twitched = now;
                if self.pairs >= FLIP_PAIRS {
                    self.turned = Some(now);
                    return true;
                }
            }
            _ => self.rim = Some((side, at)),
        }
        false
    }

    /// How far the menu is turned now (radians, clockwise).
    fn angle(&self, now: f64) -> f32 {
        if let Some(at) = self.turned {
            if !render::animations() {
                return PI;
            }
            let k = ((now - at) as f32 / TURN_MS).clamp(0.0, 1.0);
            return PI * ease_out_back(k);
        }
        let t = (now - self.twitched) as f32;
        if self.pairs == 0 || !(0.0..TWITCH_MS).contains(&t) || !render::animations() {
            return 0.0;
        }
        // A jolt one way and back, bigger with each pair, as a hint.
        let amp = 0.012 + 0.007 * self.pairs as f32;
        let side = if self.pairs.is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        side * amp * (-t / 120.0).exp() * (t / 26.0).sin()
    }
}

enum Stray {
    /// Not here; comes at this time, `caught` in a row so far.
    Waiting { at: f64, caught: u32 },
    /// One of the song's notes on a route, gold, since `since`; `seen` once it showed.
    Riding { caught: u32, since: f64, seen: bool },
    /// Caught at `at`, the `caught`th in a row; the last bursts into confetti.
    Popped { at: f64, caught: u32 },
}

/// The stray note: now and then one of the song's notes riding the menu's routes turns gold
/// (`menu_notes::Gold`). Clicked before it reaches the end of its route, it bursts and the next
/// comes soon after on another; missed, the run starts over. Three in a row make the egg.
pub struct StrayNote {
    state: Stray,
    rng: u32,
    /// Where it was last drawn.
    pos: [f32; 2],
    /// Its note on the routes.
    pub gold: crate::menu_notes::Gold,
}

impl StrayNote {
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn wait(&mut self, (lo, hi): (f64, f64), now: f64, caught: u32) {
        let at = now + lo + (hi - lo) * f64::from(self.random());
        self.state = Stray::Waiting { at, caught };
        self.gold = Default::default();
    }

    /// Whether it's on the screen to be clicked.
    #[cfg(test)]
    pub fn shown(&self) -> bool {
        matches!(self.state, Stray::Riding { seen: true, .. }) && self.gold.at.is_some()
    }

    /// How many were caught in a row.
    #[cfg(test)]
    pub fn caught(&self) -> u32 {
        match self.state {
            Stray::Waiting { caught, .. }
            | Stray::Riding { caught, .. }
            | Stray::Popped { caught, .. } => caught,
        }
    }

    /// Brings it in now (tests, and the next frame).
    #[cfg(test)]
    pub fn come_now(&mut self) {
        let caught = self.caught();
        self.state = Stray::Waiting {
            at: f64::MIN,
            caught,
        };
    }

    fn click(&mut self, now: f64) -> bool {
        let Stray::Riding {
            caught, seen: true, ..
        } = self.state
        else {
            return false;
        };
        if self.gold.at.is_none() {
            return false;
        }
        let caught = caught + 1;
        self.state = Stray::Popped { at: now, caught };
        self.gold.shown = false;
        caught == CATCHES
    }

    /// After the routes were drawn this frame (`menu_notes::draw`), over the rest of the menu:
    /// what it does next, its burst, and its hit area where no button lies.
    fn draw(&mut self, c: &mut Canvas, now: f64) {
        match self.state {
            Stray::Waiting { at, caught } => {
                // It's all movement: with animations off it doesn't come.
                if now >= at && render::animations() {
                    let route = (self.random() * 3.0) as usize % 3;
                    self.gold = crate::menu_notes::Gold {
                        want: Some(route),
                        ..Default::default()
                    };
                    self.state = Stray::Riding {
                        caught,
                        since: now,
                        seen: false,
                    };
                }
            }
            Stray::Riding {
                caught,
                since,
                ref mut seen,
            } => match self.gold.at {
                Some(at) => {
                    *seen = true;
                    self.pos = at;
                    let r = STRAY_R + 6.0;
                    let area = [at[0] - r, at[1] - r, 2.0 * r, 2.0 * r];
                    // Only on the screen (its route starts past the right edge), and never
                    // over a button: a click there is the button's.
                    let on_screen = (0.0..c.width).contains(&at[0]);
                    if on_screen && !c.hits.iter().any(|h| overlap(h.rect, area)) {
                        c.hit(area[0], area[1], area[2], area[3], Hit::StrayNote);
                    }
                }
                // Gone past the end of its route: missed, the run starts over.
                None if *seen => self.wait(AGAIN_MS, now, 0),
                // It never came: no song, or the notes are off.
                None if now - since > LOST_MS => self.wait(AGAIN_MS, now, caught),
                None => {}
            },
            Stray::Popped { at, caught } => {
                let t = (now - at) as f32;
                if caught < CATCHES {
                    if t >= CATCH_MS as f32 {
                        return self.wait(NEXT_MS, now, caught);
                    }
                    return burst(c, self.pos, t / CATCH_MS as f32);
                }
                if t >= CONFETTI_MS as f32 {
                    return self.wait(AGAIN_MS, now, 0);
                }
                confetti(c, self.pos, t, at as u32);
            }
        }
    }
}

/// Whether two rectangles (x, y, w, h) overlap.
fn overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// A catch short of the last: a gold ring spreading out, `k` of the way.
fn burst(c: &mut Canvas, [x, y]: [f32; 2], k: f32) {
    c.ring(
        x,
        y,
        STRAY_R + 22.0 * k,
        rgba(GOLD, 1.0 - k),
        3.0 * (1.0 - k) + 0.5,
    );
    c.glow(x, y, STRAY_R * (1.0 - k), 8.0, rgba(GOLD, 0.6 * (1.0 - k)));
}

/// The stray note: a gold don note with eyes looking where it goes, glinting, a few sparks
/// behind it.
pub fn draw_stray(c: &mut Canvas, x: f32, y: f32, now: f32, look: f32, squash: f32) {
    let r = STRAY_R;
    for i in 0..4 {
        let k = i as f32 / 4.0;
        let sx = x - look * (10.0 + 9.0 * i as f32);
        let sy = y + ((now / 90.0) + i as f32 * 1.7).sin() * 4.0;
        c.circle(sx, sy, 2.2 - k, rgba(GOLD, 0.6 * (1.0 - k)), [0.0; 4], 0.0);
    }
    c.glow(
        x,
        y,
        r + 7.0,
        9.0,
        rgba(GOLD, 0.35 + 0.15 * (now / 160.0).sin()),
    );
    let start = c.shapes.len();
    c.circle(x, y, r, rgba(GOLD, 1.0), rgba(INK, 1.0), 2.0);
    c.ring(x, y, r - 3.5, rgba(0xfff3c4, 1.0), 2.0);
    for side in [-1.0, 1.0] {
        let ex = x + look * 2.5 + side * 3.6;
        c.circle(ex, y - 1.5, 2.6, rgba(0xffffff, 1.0), [0.0; 4], 0.0);
        c.circle(ex + look * 0.9, y - 1.3, 1.3, rgba(INK, 1.0), [0.0; 4], 0.0);
    }
    // A glint sweeping over it now and then.
    let glint = (now / 1300.0).fract();
    if glint < 0.2 {
        let gx = x - r + 2.0 * r * glint / 0.2;
        c.circle(gx, y - r * 0.45, 2.0, rgba(0xffffff, 0.8), [0.0; 4], 0.0);
    }
    if (squash - 1.0).abs() > 1e-3 {
        c.stretch_since(start, [x, y], squash, 1.0 / squash);
    }
}

/// A caught note's burst, `t` ms in.
fn confetti(c: &mut Canvas, [x, y]: [f32; 2], t: f32, seed: u32) {
    if t < 300.0 {
        let k = t / 300.0;
        c.ring(
            x,
            y,
            STRAY_R + 40.0 * k,
            rgba(GOLD, 1.0 - k),
            3.0 * (1.0 - k) + 0.5,
        );
    }
    let fade = (1.0 - (t - (CONFETTI_MS as f32 - 400.0)) / 400.0).clamp(0.0, 1.0);
    for i in 0..CONFETTI {
        let vx = (hash(i, seed, 21) - 0.5) * 0.7;
        let vy = -0.15 - hash(i, seed, 22) * 0.4;
        let spin = (hash(i, seed, 23) - 0.5) * 0.04;
        let colour = [DON, KAT, GOLD, 0xffffff][(hash(i, seed, 24) * 4.0) as usize % 4];
        c.shapes.push(crate::render::Shape {
            center: [x + vx * t, y + vy * t + 0.5 * 0.0011 * t * t],
            half: [1.6, 3.2],
            fill: rgba(colour, fade),
            border: [0.0; 4],
            params: [0.0; 4],
            angle: spin * t,
        });
    }
}

/// Pokes at an unknown medal's question mark that hatch it, and the most time between two.
const POKES: u32 = 7;
const POKE_GAP: Duration = Duration::from_millis(1500);
/// How long a poke shakes it, and the hatching takes (ms).
const SHAKE_MS: f32 = 260.0;
const HATCH_MS: f32 = 900.0;

/// The medals screen's egg: an unknown medal's question mark poked until it cracks open.
#[derive(Default)]
pub struct Curiosity {
    /// The medal being poked (its id), how often, and when last.
    poking: Option<(String, u32, Instant)>,
    /// The medal that hatched, and when.
    hatched: Option<(String, Instant)>,
}

impl Curiosity {
    /// Nothing poked: rows outside a section's tab.
    pub const UNTOUCHED: Curiosity = Curiosity {
        poking: None,
        hatched: None,
    };

    /// A poke at medal `id` at `at`; true when it hatches.
    pub fn poke(&mut self, id: &str, at: Instant) -> bool {
        if self.hatched.is_some() {
            return false;
        }
        let pokes = match &self.poking {
            Some((was, n, last))
                if was == id && at.saturating_duration_since(*last) <= POKE_GAP =>
            {
                n + 1
            }
            _ => 1,
        };
        if pokes >= POKES {
            self.poking = None;
            self.hatched = Some((id.to_string(), at));
            return true;
        }
        self.poking = Some((id.to_string(), pokes, at));
        false
    }

    /// Draws medal `id`'s question mark at (`cx`, `cy`), `scale` screen pixels per badge pixel,
    /// as poked: shaking, cracked, or bursting open. False when it's untouched, for the plain
    /// one (`medals::mystery`).
    pub fn draw(
        &self,
        c: &mut Canvas,
        id: &str,
        [cx, cy]: [f32; 2],
        scale: f32,
        now: Instant,
    ) -> bool {
        let ms = |at: Instant| now.saturating_duration_since(at).as_secs_f32() * 1000.0;
        if let Some((hatched, at)) = &self.hatched
            && hatched == id
        {
            hatch(c, [cx, cy], scale, ms(*at));
            return true;
        }
        let Some((poked, n, at)) = &self.poking else {
            return false;
        };
        if poked != id || now.saturating_duration_since(*at) > POKE_GAP {
            return false;
        }
        let t = ms(*at);
        let shake = if t < SHAKE_MS && render::animations() {
            (1.0 - t / SHAKE_MS) * (t / 18.0).sin() * (1.0 + 0.4 * *n as f32)
        } else {
            0.0
        };
        let start = c.shapes.len();
        crate::medals::mystery_badge(c, cx, cy, scale);
        cracks(c, [cx, cy], scale, *n, id);
        c.turn_since(start, [cx, cy], shake * 0.06);
        true
    }
}

/// `n` cracks over a question mark: jagged lines from near the middle out to the rim, the same
/// for the same medal.
fn cracks(c: &mut Canvas, [cx, cy]: [f32; 2], scale: f32, n: u32, id: &str) {
    let seed = id
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    for k in 0..n {
        let mut a = hash(k, seed, 31) * 2.0 * PI;
        let mut p = [cx + (hash(k, seed, 32) - 0.5) * 3.0 * scale, cy];
        for s in 0..3 {
            let len = (2.5 + 2.0 * hash(k, s, seed)) * scale;
            let q = [p[0] + a.cos() * len, p[1] + a.sin() * len];
            let mid = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
            c.shapes.push(crate::render::Shape {
                center: mid,
                half: [len / 2.0 + 0.3, 0.45 * scale.max(1.0)],
                fill: rgba(INK, 0.95),
                border: [0.0; 4],
                params: [0.0; 4],
                angle: a,
            });
            p = q;
            a += (hash(k, s, seed ^ 0x55) - 0.5) * 1.4;
        }
    }
}

/// The question mark bursting `t` ms after the last poke: its shell flying apart over a flash,
/// then a gold "!" where it was.
fn hatch(c: &mut Canvas, [cx, cy]: [f32; 2], scale: f32, t: f32) {
    let r = 8.0 * scale;
    let k = (t / HATCH_MS).min(1.0);
    if !render::animations() || k >= 1.0 {
        c.glow(cx, cy, r, 6.0, rgba(GOLD, 0.35));
        c.text_centered(
            cx,
            cy - 7.0 * scale / 1.5,
            10.0 * scale / 1.5,
            rgba(GOLD, 1.0),
            "!",
        );
        return;
    }
    c.glow(
        cx,
        cy,
        r * (1.0 + 1.5 * k),
        10.0,
        rgba(0xfff3c4, 0.9 * (1.0 - k)),
    );
    for i in 0..12 {
        let a = i as f32 / 12.0 * 2.0 * PI + hash(i, 5, 41);
        let v = (0.05 + 0.05 * hash(i, 6, 42)) * scale;
        let (dx, dy) = (a.cos() * v * t, a.sin() * v * t + 0.0004 * scale * t * t);
        c.shapes.push(crate::render::Shape {
            center: [cx + a.cos() * r * 0.6 + dx, cy + a.sin() * r * 0.6 + dy],
            half: [1.6 * scale, 1.0 * scale],
            fill: rgba(0x3a3a4c, 1.0 - k),
            border: rgba(INK, 1.0 - k),
            params: [0.3, 0.5, 0.0, 0.0],
            angle: a + t * 0.01,
        });
    }
    if k > 0.3 {
        let show = (k - 0.3) / 0.7;
        c.text_centered(
            cx,
            cy - 7.0 * scale / 1.5,
            10.0 * scale / 1.5 * ease_out_back(show),
            rgba(GOLD, show),
            "!",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_quick_pokes_at_one_question_mark_hatch_it() {
        let start = Instant::now();
        let ms = |t: u64| start + Duration::from_millis(t);
        let mut egg = Curiosity::default();
        for i in 0..6 {
            assert!(!egg.poke("s_a", ms(i * 400)));
        }
        // Another medal starts over, as does a long pause.
        assert!(!egg.poke("s_b", ms(2500)));
        assert!(!egg.poke("s_b", ms(5000)));
        // Too long after the last: this one counts as the first.
        let mut t = 7000;
        for _ in 1..POKES {
            assert!(!egg.poke("s_b", ms(t)));
            t += 300;
        }
        assert!(egg.poke("s_b", ms(t)));
        // Hatched once and for all.
        assert!(!egg.poke("s_b", ms(t + 100)));
        let mut c = Canvas::default();
        c.clear(854.0);
        assert!(egg.draw(&mut c, "s_b", [50.0, 50.0], 1.3, ms(t + 2000)));
        assert!(!egg.draw(&mut c, "s_a", [50.0, 50.0], 1.3, ms(t + 2000)));
    }

    fn canvas(dt: f32) -> Canvas {
        let mut c = Canvas::default();
        c.clear(854.0);
        c.dt_ms = dt;
        c
    }

    #[test]
    fn ten_quick_taps_break_the_logo_and_slow_ones_dont() {
        let mut egg = LogoEgg::default();
        for i in 0..9 {
            assert!(!egg.tap(f64::from(i) * 300.0));
        }
        // Too slow: the count starts over.
        assert!(!egg.tap(9.0 * 300.0 + 800.0));
        let t = 9.0 * 300.0 + 800.0;
        for i in 1..9 {
            assert!(!egg.tap(t + f64::from(i) * 200.0));
        }
        assert!(egg.tap(t + 9.0 * 200.0));
        // Broken, it takes no taps until it's whole again.
        assert!(!egg.tap(t + 9.0 * 200.0 + 50.0));
        assert!(egg.crunch(t + 9.0 * 200.0 + BACK_MS + 10.0).is_some());
    }

    #[test]
    fn both_rims_within_a_big_notes_window_ten_times_turn_the_menu() {
        let start = Instant::now();
        let ms = |t: u64| start + Duration::from_millis(t);
        let mut egg = FlipEgg::default();
        // Further apart than a big note allows: no pair.
        assert!(!egg.kat(-1, ms(0), 0.0));
        assert!(!egg.kat(1, ms(31), 0.0));
        assert_eq!(egg.pairs, 0);
        // The same rim twice isn't both.
        assert!(!egg.kat(1, ms(500), 0.0));
        assert!(!egg.kat(1, ms(505), 0.0));
        assert_eq!(egg.pairs, 0);
        let mut t = 1000;
        for n in 1..FLIP_PAIRS {
            assert!(!egg.kat(-1, ms(t), 0.0));
            assert!(!egg.kat(1, ms(t + 30), 0.0));
            assert_eq!(egg.pairs, n);
            t += 400;
        }
        // A long pause starts the count again.
        t += 2500;
        assert!(!egg.kat(1, ms(t), 0.0));
        assert!(!egg.kat(-1, ms(t + 5), 0.0));
        assert_eq!(egg.pairs, 1);
        for _ in 1..FLIP_PAIRS - 1 {
            t += 300;
            egg.kat(-1, ms(t), 0.0);
            egg.kat(1, ms(t), 0.0);
        }
        assert!(egg.turned.is_none());
        t += 300;
        assert!(!egg.kat(1, ms(t), 100.0));
        assert!(egg.kat(-1, ms(t + 12), 100.0));
        assert!(!egg.kat(1, ms(t + 400), 200.0));
        render::edit_frame_theme(|t| t.animations = true);
        assert!((egg.angle(100.0 + f64::from(TURN_MS)) - PI).abs() < 1e-4);
        assert!((egg.angle(1e9) - PI).abs() < 1e-4);
    }

    /// A frame of the menu at song time `time`: the routes of `notes`, then `buttons`, then
    /// the eggs.
    fn menu_frame(
        eggs: &mut MenuEggs,
        notes: &[crate::menu_notes::Ghost],
        time: f64,
        buttons: &[[f32; 4]],
    ) -> Canvas {
        let mut c = canvas(16.0);
        eggs.begin(&c);
        let view = crate::menu_notes::View::of(notes, time);
        crate::menu_notes::draw(&mut c, &view, 0.0, 0.0, 0.0, 0, &mut eggs.stray.gold);
        for (i, &[x, y, w, h]) in buttons.iter().enumerate() {
            c.hit(x, y, w, h, Hit::MenuItem(i));
        }
        eggs.end(&mut c);
        c
    }

    /// Frames from song time `*time` until the gold note can be clicked; None if it never can.
    fn until_gold(
        eggs: &mut MenuEggs,
        notes: &[crate::menu_notes::Ghost],
        time: &mut f64,
    ) -> Option<Canvas> {
        for _ in 0..2000 {
            *time += 16.0;
            let c = menu_frame(eggs, notes, *time, &[]);
            if c.hits.iter().any(|h| h.hit == Hit::StrayNote) {
                return Some(c);
            }
        }
        None
    }

    fn song() -> Vec<crate::menu_notes::Ghost> {
        (0..4000)
            .map(|i| crate::menu_notes::Ghost {
                time: f64::from(i) * 150.0,
                kat: i % 3 == 0,
                big: false,
            })
            .collect()
    }

    #[test]
    fn the_stray_note_is_caught_on_the_routes() {
        render::edit_frame_theme(|t| t.animations = true);
        let notes = song();
        let mut eggs = MenuEggs::new(7);
        let mut time = 0.0;
        assert!(!eggs.stray.shown());
        assert!(!eggs.catch_stray());
        eggs.stray.come_now();
        for n in 1..=CATCHES {
            let c = until_gold(&mut eggs, &notes, &mut time).expect("a gold note rides in");
            assert!(eggs.stray.shown());
            // It is one of the song's notes, where its route has it.
            let (route, at) = eggs.stray.gold.note.unwrap();
            assert!(route < 3);
            assert!(notes.iter().any(|g| g.time == at));
            let [x, y, w, _] = c
                .hits
                .iter()
                .find(|h| h.hit == Hit::StrayNote)
                .unwrap()
                .rect;
            assert!(x + w / 2.0 < c.width && x > 0.0 && y > 0.0, "on the screen");
            assert_eq!(eggs.catch_stray(), n == CATCHES, "catch {n}");
            assert!(!eggs.stray.shown());
            assert!(!eggs.catch_stray(), "a burst one can't be caught again");
        }
        let c = menu_frame(&mut eggs, &notes, time, &[]);
        assert!(!c.hits.iter().any(|h| h.hit == Hit::StrayNote));
        assert!(c.shapes.len() >= CONFETTI as usize);
    }

    #[test]
    fn the_stray_note_let_through_is_lost() {
        render::edit_frame_theme(|t| t.animations = true);
        let notes = song();
        let mut eggs = MenuEggs::new(11);
        let mut time = 0.0;
        eggs.stray.come_now();
        until_gold(&mut eggs, &notes, &mut time).unwrap();
        assert!(!eggs.catch_stray());
        assert_eq!(eggs.stray.caught(), 1);
        until_gold(&mut eggs, &notes, &mut time).expect("the next comes soon");
        // Left alone, it rides off the end of its route.
        for _ in 0..1000 {
            time += 16.0;
            menu_frame(&mut eggs, &notes, time, &[]);
        }
        assert!(!eggs.stray.shown());
        assert_eq!(eggs.stray.caught(), 0);
    }

    #[test]
    fn the_stray_note_under_a_button_leaves_the_click_to_it() {
        render::edit_frame_theme(|t| t.animations = true);
        let notes = song();
        let mut eggs = MenuEggs::new(5);
        let mut time = 0.0;
        eggs.stray.come_now();
        until_gold(&mut eggs, &notes, &mut time).unwrap();
        let everywhere = [0.0, 0.0, 854.0, VIRTUAL_HEIGHT];
        let c = menu_frame(&mut eggs, &notes, time + 16.0, &[everywhere]);
        assert!(eggs.stray.gold.at.is_some(), "still riding");
        assert!(!c.hits.iter().any(|h| h.hit == Hit::StrayNote));
    }

    #[test]
    fn the_stray_note_needs_the_menu_s_notes() {
        render::edit_frame_theme(|t| t.animations = true);
        let mut eggs = MenuEggs::new(9);
        eggs.stray.come_now();
        let mut time = 0.0;
        assert!(until_gold(&mut eggs, &[], &mut time).is_none());
        assert!(!eggs.catch_stray());
    }

    #[test]
    fn a_turned_menu_turns_its_hit_areas_with_it() {
        render::edit_frame_theme(|t| t.animations = false);
        let mut eggs = MenuEggs::new(3);
        eggs.flip.turned = Some(0.0);
        let mut c = canvas(16.0);
        eggs.begin(&c);
        c.hit(10.0, 20.0, 30.0, 40.0, Hit::MenuItem(0));
        eggs.end(&mut c);
        let item = c.hits.iter().find(|h| h.hit == Hit::MenuItem(0)).unwrap();
        let [x, y, w, h] = item.rect;
        assert!((x - (854.0 - 40.0)).abs() < 1e-3, "{x}");
        assert!((y - (VIRTUAL_HEIGHT - 60.0)).abs() < 1e-3, "{y}");
        assert_eq!((w, h), (30.0, 40.0));
    }
}
