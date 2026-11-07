//! The interface during play and what its toggle (Shift+Tab by default) hides: the player picks
//! which parts go: everything but GREAT / OK / MISS (like osu!, which keeps judgements on with
//! the HUD hidden) unless they choose otherwise.
//!
//! Also where the parts go: a `Layout` holds a `Place` for each part the player moved in the
//! interface editor. A place hangs each axis on a `Target`, so a part attached to the playfield,
//! its lane, the drum or the hit point follows them (and scales with them) as the playfield is
//! moved and resized.

use crate::game::{HIT_X, LANE_H, LANE_Y, NOTE_R, PANEL_W};
use crate::layout::{Area, Guide};
use crate::render::VIRTUAL_HEIGHT;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Score,
    Accuracy,
    Pp,
    Combo,
    Progress,
    Health,
    Mods,
    HitError,
    OffsetHint,
    Judgements,
    HitAnimation,
    /// A multiplayer match's scoreboard.
    Scoreboard,
    /// The friends watching a solo play.
    Spectators,
    /// The unstable rate so far.
    Ur,
    /// How many 300s, 100s and misses so far.
    Counts,
}

impl Part {
    pub const ALL: [Part; 15] = [
        Part::Score,
        Part::Accuracy,
        Part::Pp,
        Part::Combo,
        Part::Progress,
        Part::Health,
        Part::Mods,
        Part::HitError,
        Part::OffsetHint,
        Part::Judgements,
        Part::HitAnimation,
        Part::Scoreboard,
        Part::Spectators,
        Part::Ur,
        Part::Counts,
    ];

    /// The parts the interface editor places: all but the hit animation, which isn't anywhere.
    pub const PLACED: [Part; 14] = [
        Part::Score,
        Part::Accuracy,
        Part::Pp,
        Part::Combo,
        Part::Progress,
        Part::Health,
        Part::Mods,
        Part::HitError,
        Part::OffsetHint,
        Part::Judgements,
        Part::Ur,
        Part::Counts,
        Part::Scoreboard,
        Part::Spectators,
    ];

    /// For the settings list.
    pub fn label(self) -> &'static str {
        match self {
            Part::Score => tr!("Score"),
            Part::Accuracy => tr!("Accuracy"),
            Part::Pp => tr!("pp counter"),
            Part::Combo => tr!("Combo (on the drum)"),
            Part::Progress => tr!("Progress bar"),
            Part::Health => tr!("Health bar"),
            Part::Mods => tr!("Mods"),
            Part::HitError => tr!("Hit error bar"),
            Part::OffsetHint => tr!("Offset suggestion"),
            Part::Judgements => "GREAT / OK / MISS",
            Part::HitAnimation => tr!("Note hit animation"),
            Part::Scoreboard => tr!("Match scoreboard"),
            Part::Spectators => tr!("Spectators"),
            Part::Ur => tr!("UR counter"),
            Part::Counts => tr!("Hit counter (300 / 100 / MISS)"),
        }
    }

    /// Whether it has text to colour.
    pub fn has_text(self) -> bool {
        matches!(
            self,
            Part::Score
                | Part::Accuracy
                | Part::Pp
                | Part::Combo
                | Part::Mods
                | Part::OffsetHint
                | Part::Judgements
                | Part::Ur
                | Part::Counts
        )
    }

    /// For the config.
    fn key(self) -> &'static str {
        match self {
            Part::Score => "score",
            Part::Accuracy => "accuracy",
            Part::Pp => "pp",
            Part::Combo => "combo",
            Part::Progress => "progress",
            Part::Health => "health",
            Part::Mods => "mods",
            Part::HitError => "hit_error",
            Part::OffsetHint => "offset_hint",
            Part::Judgements => "judgements",
            Part::HitAnimation => "hit_animation",
            Part::Scoreboard => "scoreboard",
            Part::Spectators => "spectators",
            Part::Ur => "ur",
            Part::Counts => "counts",
        }
    }

    fn bit(self) -> u16 {
        1 << Part::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }
}

/// A set of interface parts.
/// The parts a saved config's line may be older than.
const NEWER: [Part; 5] = [
    Part::Pp,
    Part::Scoreboard,
    Part::Spectators,
    Part::Ur,
    Part::Counts,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Parts(u16);

impl Parts {
    pub const NONE: Parts = Parts(0);
    pub const ALL: Parts = Parts((1 << Part::ALL.len()) - 1);
    /// What the toggle hides to begin with: all but the judgements (`Part::Judgements`, 10th).
    pub const DEFAULT_HIDES: Parts = Parts(Parts::ALL.0 & !(1 << 9));

    pub fn has(self, p: Part) -> bool {
        self.0 & p.bit() != 0
    }

    pub fn toggle(&mut self, p: Part) {
        self.0 ^= p.bit();
    }

    /// The parts of both sets.
    pub fn with(self, other: Parts) -> Parts {
        Parts(self.0 | other.0)
    }

    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// "score,accuracy,..." (empty: none).
    pub fn format(self) -> String {
        Part::ALL
            .iter()
            .filter(|&&p| self.has(p))
            .map(|p| p.key())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The config's line: `format`, and the parts that came later and aren't hidden, marked with
    /// `!`, so that `parse_saved` can tell those from a part the line is older than.
    pub fn format_saved(self) -> String {
        let shown = NEWER
            .iter()
            .filter(|&&p| !self.has(p))
            .map(|p| format!("!{}", p.key()));
        std::iter::once(self.format())
            .filter(|s| !s.is_empty())
            .chain(shown)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The config's line: saved before parts that came later (the pp counter, the scoreboard, the
    /// spectators), hiding all it knew of still hides those too. All but the judgements counts:
    /// that's what the toggle hides to begin with.
    pub fn parse_saved(s: &str) -> Parts {
        let mut parts = Parts::parse(s);
        let saved = |p: Part| {
            s.split(',')
                .map(str::trim)
                .any(|k| k.strip_prefix('!').unwrap_or(k) == p.key())
        };
        if Part::ALL
            .iter()
            .all(|&p| p == Part::Judgements || parts.has(p) || (NEWER.contains(&p) && !saved(p)))
        {
            for &p in NEWER.iter().filter(|&&p| !saved(p)) {
                parts.0 |= p.bit();
            }
        }
        parts
    }

    pub fn parse(s: &str) -> Parts {
        let mut out = Parts::NONE;
        for word in s.split(',').map(str::trim) {
            if let Some(&p) = Part::ALL.iter().find(|p| p.key() == word) {
                out.0 |= p.bit();
            }
        }
        out
    }
}

/// What one axis of a part is attached to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Screen,
    Playfield,
    Lane,
    Drum,
    HitPoint,
}

impl Target {
    pub const ALL: [Target; 5] = [
        Target::Screen,
        Target::Playfield,
        Target::Lane,
        Target::Drum,
        Target::HitPoint,
    ];
    /// The order snapping prefers them in when lines coincide, as the lane's sides do the
    /// playfield's and the screen's: the lane first, the one that mods move too, so that a part
    /// placed on a full screen still follows it when it's bent or made smaller.
    const SNAP_ORDER: [Target; 5] = [
        Target::Lane,
        Target::Drum,
        Target::HitPoint,
        Target::Playfield,
        Target::Screen,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Target::Screen => tr!("Screen"),
            Target::Playfield => tr!(hud: "Playfield"),
            Target::Lane => tr!("Lane"),
            Target::Drum => tr!("Drum"),
            Target::HitPoint => tr!("Hit point"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Target::Screen => "screen",
            Target::Playfield => "playfield",
            Target::Lane => "lane",
            Target::Drum => "drum",
            Target::HitPoint => "hit",
        }
    }

    /// One of what mods move about with the lane.
    fn on_the_lane(self) -> bool {
        matches!(self, Target::Lane | Target::Drum | Target::HitPoint)
    }

    pub fn next(self) -> Target {
        let i = Target::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Target::ALL[(i + 1) % Target::ALL.len()]
    }
}

/// One axis of a place: the part's `pivot` (a fraction of its size) sits `off` units from the
/// point `at` (a fraction) of the target.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Axis {
    pub target: Target,
    pub at: f32,
    pub pivot: f32,
    pub off: f32,
}

/// Against the screen's edge at `at`: the part's own edge there, `off` from it.
pub const fn on_screen(at: f32, off: f32) -> Axis {
    Axis {
        target: Target::Screen,
        at,
        pivot: at,
        off,
    }
}

pub const fn axis(target: Target, at: f32, pivot: f32, off: f32) -> Axis {
    Axis {
        target,
        at,
        pivot,
        off,
    }
}

/// Where a part goes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Place {
    pub x: Axis,
    pub y: Axis,
    pub scale: f32,
}

/// How close to the lane (units) a dragged part has to be to go on it.
const NEAR_LANE: f32 = 40.0;

pub const MIN_SCALE: f32 = 0.25;
pub const MAX_SCALE: f32 = 4.0;

impl Place {
    pub const fn new(x: Axis, y: Axis) -> Place {
        Place { x, y, scale: 1.0 }
    }

    /// On anything but the screen: it grows and shrinks with the playfield.
    pub fn follows_playfield(&self) -> bool {
        self.x.target != Target::Screen || self.y.target != Target::Screen
    }

    fn parse(s: &str) -> Option<Place> {
        let v: Vec<&str> = s.split(',').map(str::trim).collect();
        let [xt, xa, xp, xo, yt, ya, yp, yo, scale] = v[..] else {
            return None;
        };
        let target = |k: &str| Target::ALL.into_iter().find(|t| t.key() == k);
        let num = |n: &str| n.parse::<f32>().ok().filter(|n| n.is_finite());
        let unit = |n: &str| num(n).map(|n| n.clamp(0.0, 1.0));
        Some(Place {
            x: axis(target(xt)?, unit(xa)?, unit(xp)?, num(xo)?),
            y: axis(target(yt)?, unit(ya)?, unit(yp)?, num(yo)?),
            scale: num(scale)?.clamp(MIN_SCALE, MAX_SCALE),
        })
    }

    fn format(&self) -> String {
        let a = |a: Axis| format!("{},{},{},{:.2}", a.target.key(), a.at, a.pivot, a.off);
        format!("{},{},{:.3}", a(self.x), a(self.y), self.scale)
    }
}

/// A part's box on the screen, and the scale from its own units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub k: f32,
}

/// Where mods have taken the lane, the drum and the hit point this frame: x, y, width, height in
/// the playfield's own units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FieldBoxes {
    pub lane: [f32; 4],
    pub drum: [f32; 4],
    pub hit: [f32; 4],
}

/// Where the targets are on a screen `w` wide with the playfield at `area` (x, y, width, height).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Frames {
    rects: [[f32; 4]; 5],
    /// The playfield's scale.
    pub k: f32,
    pub w: f32,
}

impl Frames {
    pub fn new(w: f32, area: Area) -> Frames {
        let k = area.h;
        let (ox, oy) = (area.x * w, area.y * VIRTUAL_HEIGHT);
        let field_w = area.w * w;
        let lane_y = oy + (LANE_Y - LANE_H / 2.0) * k;
        Frames {
            rects: [
                [0.0, 0.0, w, VIRTUAL_HEIGHT],
                [ox, oy, field_w, area.h * VIRTUAL_HEIGHT],
                [ox, lane_y, field_w, LANE_H * k],
                [ox, lane_y, PANEL_W * k, LANE_H * k],
                [
                    ox + (HIT_X - NOTE_R) * k,
                    oy + (LANE_Y - NOTE_R) * k,
                    2.0 * NOTE_R * k,
                    2.0 * NOTE_R * k,
                ],
            ],
            k,
            w,
        }
    }

    /// With the lane, drum and hit point where mods put them, in the playfield `area`.
    pub fn with_field(mut self, area: Area, boxes: Option<FieldBoxes>) -> Frames {
        let Some(b) = boxes else { return self };
        let (ox, oy) = (area.x * self.w, area.y * VIRTUAL_HEIGHT);
        let on_screen =
            |[x, y, w, h]: [f32; 4]| [ox + x * self.k, oy + y * self.k, w * self.k, h * self.k];
        self.rects[2] = on_screen(b.lane);
        self.rects[3] = on_screen(b.drum);
        self.rects[4] = on_screen(b.hit);
        self
    }

    pub fn rect(&self, t: Target) -> [f32; 4] {
        self.rects[Target::ALL.iter().position(|&a| a == t).unwrap_or(0)]
    }

    /// What `place`'s units come to on the screen.
    pub fn follow(&self, place: &Place) -> f32 {
        if place.follows_playfield() {
            self.k
        } else {
            1.0
        }
    }

    /// The box of a part `size` big (in its own units) at `place`. `keep_on` holds it to the
    /// screen, wherever a smaller playfield or another window size would take it.
    pub fn resolve(&self, place: &Place, size: [f32; 2], keep_on: bool) -> Placed {
        let f = self.follow(place);
        let k = place.scale * f;
        let (w, h) = (size[0] * k, size[1] * k);
        let along = |a: Axis, start: usize, len: f32, room: f32| {
            let r = self.rect(a.target);
            let pos = r[start] + a.at * r[start + 2] + a.off * f - a.pivot * len;
            if keep_on {
                pos.clamp(0.0, (room - len).max(0.0))
            } else {
                pos
            }
        };
        Placed {
            x: along(place.x, 0, w, self.w),
            y: along(place.y, 1, h, VIRTUAL_HEIGHT),
            w,
            h,
            k,
        }
    }

    /// A place with these axes (their `off`s left to work out) that puts a box, `k` to the unit,
    /// at `x`, `y` on the screen.
    fn placing(&self, x: Axis, y: Axis, b: Placed) -> Place {
        let mut p = Place::new(x, y);
        let f = self.follow(&p);
        p.scale = (b.k / f).clamp(MIN_SCALE, MAX_SCALE);
        let off = |a: Axis, start: usize, pos: f32, len: f32| {
            let r = self.rect(a.target);
            (pos + a.pivot * len - r[start] - a.at * r[start + 2]) / f
        };
        p.x.off = off(x, 0, b.x, b.w);
        p.y.off = off(y, 1, b.y, b.h);
        p
    }

    /// `place`, whose box is `b`, dragged to `x`, `y`. With `snap`, each axis of the box catches
    /// on a target's edges and centre within `tol` units, and is attached there; or on a line of
    /// `snap`'s own (the other parts'). Close to the lane, a part goes on it, whatever it caught
    /// on or if nothing, so that it goes where the lane does under mods. Away from it, nothing of
    /// the lane catches it and it lets go of the lane. Without `snap` it stays attached as it was.
    pub fn dragged(
        &self,
        place: &Place,
        b: Placed,
        (x, y): (f32, f32),
        snap: Option<&Snapping>,
    ) -> (Place, Vec<Guide>) {
        let mut to = Placed {
            x: x.clamp(0.0, (self.w - b.w).max(0.0)),
            y: y.clamp(0.0, (VIRTUAL_HEIGHT - b.h).max(0.0)),
            ..b
        };
        let (mut ax, mut ay) = (place.x, place.y);
        let mut guides = Vec::new();
        if let Some(s) = snap {
            let near = self.near_lane(to);
            let (mut on_x, mut on_y) = (None, None);
            if let Some((line, on)) = self.catch(to.x, to.w, 0, &s.xs, s.tol, near) {
                to.x = line - on.map_or(snap_pivot(line, to.x, to.w), |a| a.pivot) * to.w;
                on_x = on;
                guides.push(Guide::Vertical(line / self.w.max(1.0)));
            }
            if let Some((line, on)) = self.catch(to.y, to.h, 1, &s.ys, s.tol, near) {
                to.y = line - on.map_or(snap_pivot(line, to.y, to.h), |a| a.pivot) * to.h;
                on_y = on;
                guides.push(Guide::Horizontal(line / VIRTUAL_HEIGHT));
            }
            ax = on_x.unwrap_or_else(|| {
                if near {
                    self.along_lane(to)
                } else {
                    off_the_lane(ax, (to.x + to.w / 2.0) / self.w.max(1.0))
                }
            });
            ay = on_y.unwrap_or_else(|| {
                if near {
                    self.off_lane(to)
                } else {
                    off_the_lane(ay, (to.y + to.h / 2.0) / VIRTUAL_HEIGHT)
                }
            });
        }
        (self.placing(ax, ay, to), guides)
    }

    /// Whether `b` is over the lane, or this close above or under it.
    fn near_lane(&self, b: Placed) -> bool {
        let r = self.rect(Target::Lane);
        let reach = NEAR_LANE * self.k;
        b.y < r[1] + r[3] + reach && b.y + b.h > r[1] - reach
    }

    /// Attached along the lane: by its left end, middle or right end, whichever `b` is nearest.
    fn along_lane(&self, b: Placed) -> Axis {
        let r = self.rect(Target::Lane);
        let middle = b.x + b.w / 2.0;
        let at = ((middle - r[0]) / r[2].max(1.0) * 2.0)
            .round()
            .clamp(0.0, 2.0)
            / 2.0;
        axis(Target::Lane, at, at, 0.0)
    }

    /// Attached across the lane: over it from above its middle, else under it.
    fn off_lane(&self, b: Placed) -> Axis {
        let r = self.rect(Target::Lane);
        if b.y + b.h / 2.0 < r[1] + r[3] / 2.0 {
            axis(Target::Lane, 0.0, 1.0, 0.0)
        } else {
            axis(Target::Lane, 1.0, 0.0, 0.0)
        }
    }

    /// The nearest line a box from `lo`, `len` long, catches on along one axis (`start`: 0 for
    /// x, 1 for y): the line, and the axis that attaches to it if it's a target's.
    fn catch(
        &self,
        lo: f32,
        len: f32,
        start: usize,
        others: &[f32],
        tol: f32,
        near: bool,
    ) -> Option<(f32, Option<Axis>)> {
        const STOPS: [f32; 3] = [0.0, 0.5, 1.0];
        let mut best: Option<(f32, f32, Option<Axis>)> = None;
        let mut offer = |dist: f32, line: f32, on: Option<Axis>| {
            // Strictly nearer: on a tie the earlier one stays.
            if dist <= tol && best.is_none_or(|b| dist < b.0 - 0.01) {
                best = Some((dist, line, on));
            }
        };
        for target in Target::SNAP_ORDER {
            if !near && target.on_the_lane() {
                continue;
            }
            let r = self.rect(target);
            for at in STOPS {
                let line = r[start] + at * r[start + 2];
                for pivot in STOPS {
                    let dist = (lo + pivot * len - line).abs();
                    offer(dist, line, Some(axis(target, at, pivot, 0.0)));
                }
            }
        }
        for &line in others {
            for pivot in STOPS {
                offer((lo + pivot * len - line).abs(), line, None);
            }
        }
        best.map(|(_, line, on)| (line, on))
    }

    /// `place` (box `b`) with one axis attached to `target` instead, where it is on the screen:
    /// at the target's nearest edge or centre.
    pub fn attached(&self, place: &Place, b: Placed, horizontal: bool, target: Target) -> Place {
        let (start, lo, len, old) = if horizontal {
            (0, b.x, b.w, place.x)
        } else {
            (1, b.y, b.h, place.y)
        };
        let r = self.rect(target);
        let at = ((lo + old.pivot * len - r[start]) / r[start + 2].max(1.0) * 2.0)
            .round()
            .clamp(0.0, 2.0)
            / 2.0;
        let new = axis(target, at, at, 0.0);
        if horizontal {
            self.placing(new, place.y, b)
        } else {
            self.placing(place.x, new, b)
        }
    }
}

/// `a`, or if it's on the lane the screen instead, by the side `at` (a fraction of it) is on.
fn off_the_lane(a: Axis, at: f32) -> Axis {
    if a.target.on_the_lane() {
        on_screen((at * 2.0).round().clamp(0.0, 2.0) / 2.0, 0.0)
    } else {
        a
    }
}

/// Of a box from `lo`, `len` long: which of its edges or centre lies on `line`.
fn snap_pivot(line: f32, lo: f32, len: f32) -> f32 {
    [0.0, 0.5, 1.0]
        .into_iter()
        .min_by(|a, b| {
            (lo + a * len - line)
                .abs()
                .total_cmp(&(lo + b * len - line).abs())
        })
        .unwrap_or(0.0)
}

/// What a dragged part catches on, beside the targets.
pub struct Snapping {
    pub tol: f32,
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
}

/// What the hit counter shows: which counts, and whether each has its title. Without titles, the
/// counts' colours say which is which.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Counter {
    pub great: bool,
    pub ok: bool,
    pub miss: bool,
    pub titles: bool,
}

impl Default for Counter {
    fn default() -> Counter {
        Counter {
            great: true,
            ok: true,
            miss: true,
            titles: true,
        }
    }
}

impl Counter {
    fn format(self) -> String {
        [
            (self.great, "300"),
            (self.ok, "100"),
            (self.miss, "miss"),
            (self.titles, "titles"),
        ]
        .iter()
        .filter(|w| w.0)
        .map(|w| w.1)
        .collect::<Vec<_>>()
        .join(",")
    }

    fn parse(s: &str) -> Counter {
        let on = |word: &str| s.split(',').any(|w| w.trim() == word);
        Counter {
            great: on("300"),
            ok: on("100"),
            miss: on("miss"),
            titles: on("titles"),
        }
    }
}

/// Where everything goes: a place for each part the player moved (the rest are where the game
/// puts them), and the parts switched off.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Layout {
    places: [Option<Place>; Part::ALL.len()],
    colours: [Option<u32>; Part::ALL.len()],
    pub off: Parts,
    pub counter: Counter,
}

impl Default for Layout {
    fn default() -> Layout {
        Layout {
            places: [None; Part::ALL.len()],
            colours: [None; Part::ALL.len()],
            off: Parts(Part::Ur.bit() | Part::Counts.bit()),
            counter: Counter::default(),
        }
    }
}

impl Layout {
    pub fn index(p: Part) -> usize {
        Part::ALL.iter().position(|&a| a == p).unwrap_or(0)
    }

    pub fn place(&self, p: Part) -> Option<Place> {
        self.places[Layout::index(p)]
    }

    pub fn set_place(&mut self, p: Part, place: Option<Place>) {
        self.places[Layout::index(p)] = place;
    }

    /// The colour the player picked for a part's text.
    pub fn colour(&self, p: Part) -> Option<u32> {
        self.colours[Layout::index(p)]
    }

    pub fn set_colour(&mut self, p: Part, colour: Option<u32>) {
        self.colours[Layout::index(p)] = colour;
    }

    pub fn shown(&self, p: Part) -> bool {
        !self.off.has(p)
    }

    /// `off=ur,counts|score=screen,1,1,-18,screen,0,0,18,1|...`
    pub fn format(&self) -> String {
        let mut out = format!("off={}", self.off.format());
        if self.counter != Counter::default() {
            out.push_str(&format!("|counter={}", self.counter.format()));
        }
        for p in Part::ALL {
            if let Some(place) = self.place(p) {
                out.push_str(&format!("|{}={}", p.key(), place.format()));
            }
            if let Some(colour) = self.colour(p) {
                out.push_str(&format!("|colour:{}={colour:06x}", p.key()));
            }
        }
        out
    }

    /// `format`'s text; what's not understood in it is left where the game puts it.
    pub fn parse(s: &str) -> Option<Layout> {
        let mut out = Layout::default();
        let mut sections = s.trim().split('|');
        out.off = Parts::parse(sections.next()?.strip_prefix("off=")?);
        for section in sections {
            let Some((key, place)) = section.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if key == "counter" {
                out.counter = Counter::parse(place);
                continue;
            }
            let part = |k: &str| Part::ALL.into_iter().find(|p| p.key() == k);
            if let Some(p) = key.strip_prefix("colour:").and_then(part) {
                let colour = u32::from_str_radix(place.trim(), 16).ok();
                out.set_colour(p, colour.filter(|&c| c <= 0xffffff));
            } else if let Some(p) = part(key) {
                out.set_place(p, Place::parse(place));
            }
        }
        Some(out)
    }
}

/// A saved layout's config line: `name: layout`.
pub fn format_preset(name: &str, layout: &Layout) -> String {
    format!("{name}: {}", layout.format())
}

pub fn parse_preset(s: &str) -> Option<(String, Layout)> {
    let (name, layout) = s.rsplit_once(": ")?;
    let name = name.trim();
    (!name.is_empty()).then(|| Layout::parse(layout).map(|l| (name.to_string(), l)))?
}

/// Makes a name fit for that line.
pub fn clean_name(name: &str) -> String {
    name.replace([':', '#', '\n', '\r'], " ").trim().to_string()
}

/// The layouts that come with the game.
pub fn builtin_presets() -> Vec<(&'static str, Layout)> {
    let default = Layout::default();

    let mut stats = default;
    stats.off = Parts::NONE;

    let mut classic = stats;
    classic.set_place(
        Part::Health,
        Some(Place::new(on_screen(0.0, 8.0), on_screen(0.0, 8.0))),
    );
    classic.set_place(
        Part::Progress,
        Some(Place::new(on_screen(0.0, 0.0), on_screen(1.0, 0.0))),
    );

    let mut lane = stats;
    let edge = 8.0;
    let (left, right, centre) = (
        axis(Target::Lane, 0.0, 0.0, edge),
        axis(Target::Lane, 1.0, 1.0, -edge),
        axis(Target::Lane, 0.5, 0.5, 0.0),
    );
    let above = |off: f32| axis(Target::Lane, 0.0, 1.0, -off);
    let below = |off: f32| axis(Target::Lane, 1.0, 0.0, off);
    for (part, x, y) in [
        (Part::Score, right, above(26.0)),
        (Part::Accuracy, right, above(6.0)),
        (Part::Pp, axis(Target::Lane, 1.0, 1.0, -90.0), above(6.0)),
        (Part::Mods, left, above(6.0)),
        (Part::Health, centre, above(8.0)),
        (
            Part::Progress,
            axis(Target::Lane, 0.0, 0.0, 0.0),
            below(4.0),
        ),
        (Part::Counts, left, below(14.0)),
        (Part::Ur, right, below(14.0)),
        (Part::HitError, centre, below(14.0)),
        (Part::OffsetHint, centre, below(62.0)),
    ] {
        lane.set_place(part, Some(Place::new(x, y)));
    }

    let mut clean = default;
    for p in [
        Part::Score,
        Part::Accuracy,
        Part::Pp,
        Part::Health,
        Part::Mods,
        Part::OffsetHint,
    ] {
        clean.off.0 |= p.bit();
    }

    vec![
        (tr!("Default"), default),
        (tr!("Full stats"), stats),
        (tr!("Classic"), classic),
        (tr!("Follow the lane"), lane),
        (tr!("Clean"), clean),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_everything_saved_before_newer_parts_still_hides_them() {
        let old = "score,accuracy,combo,progress,health,mods,hit_error,offset_hint,judgements,hit_animation";
        assert_eq!(Parts::parse_saved(old), Parts::ALL);
        assert_eq!(Parts::parse_saved(&format!("{old},pp")), Parts::ALL);
        let some = Parts::parse_saved("score,pp");
        assert!(some.has(Part::Score) && !some.has(Part::Scoreboard) && some.len() == 2);
        assert_eq!(
            Parts::parse_saved(&Parts::DEFAULT_HIDES.format()),
            Parts::DEFAULT_HIDES
        );
        // The old default, which leaves the judgements out, takes in the newer parts too.
        let default = "score,accuracy,pp,combo,progress,health,mods,hit_error,offset_hint,hit_animation,scoreboard";
        assert_eq!(Parts::parse_saved(default), Parts::DEFAULT_HIDES);
    }

    #[test]
    fn a_newer_part_taken_off_the_list_stays_off() {
        let mut parts = Parts::ALL;
        parts.toggle(Part::Spectators);
        assert_eq!(Parts::parse_saved(&parts.format_saved()), parts);
        parts.toggle(Part::Judgements);
        assert_eq!(Parts::parse_saved(&parts.format_saved()), parts);
        for p in [Parts::NONE, Parts::ALL, Parts::DEFAULT_HIDES] {
            assert_eq!(Parts::parse_saved(&p.format_saved()), p);
        }
    }

    #[test]
    fn hiding_the_interface_keeps_the_judgements_to_begin_with() {
        let d = Parts::DEFAULT_HIDES;
        assert!(!d.has(Part::Judgements) && d.len() == Part::ALL.len() - 1);
    }

    #[test]
    fn parts_round_trip_through_the_config() {
        let mut some = Parts::NONE;
        some.toggle(Part::Score);
        some.toggle(Part::HitAnimation);
        for p in [Parts::NONE, Parts::ALL, some] {
            assert_eq!(Parts::parse(&p.format()), p);
        }
        assert_eq!(Parts::ALL.len(), Part::ALL.len());
        assert!(Parts::ALL.has(Part::HitAnimation));
    }

    const FULL: Area = Area::FULL;
    const SMALL: Area = Area {
        x: 0.25,
        y: 0.25,
        w: 0.5,
        h: 0.5,
    };

    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 0.02, "{a} is not {b}");
    }

    #[test]
    fn a_part_on_the_screen_stays_put_as_the_playfield_changes() {
        let p = Place::new(on_screen(1.0, -18.0), on_screen(0.0, 18.0));
        let size = [100.0, 20.0];
        assert_eq!(
            Frames::new(800.0, FULL).resolve(&p, size, true),
            Frames::new(800.0, SMALL).resolve(&p, size, true)
        );
        let b = Frames::new(800.0, FULL).resolve(&p, size, true);
        near(b.x, 800.0 - 18.0 - 100.0);
        near(b.y, 18.0);
    }

    #[test]
    fn a_part_on_the_lane_follows_and_scales_with_the_playfield() {
        let p = Place::new(
            axis(Target::Lane, 1.0, 1.0, -8.0),
            axis(Target::Lane, 1.0, 0.0, 10.0),
        );
        let size = [100.0, 20.0];
        let full = Frames::new(800.0, FULL);
        let small = Frames::new(800.0, SMALL);
        let (a, b) = (full.resolve(&p, size, true), small.resolve(&p, size, true));
        near(a.x + a.w, 792.0);
        near(a.y, LANE_Y + LANE_H / 2.0 + 10.0);
        near(b.w, 50.0);
        near(b.x + b.w, 600.0 - 4.0);
        near(b.y, 120.0 + (LANE_Y + LANE_H / 2.0) * 0.5 + 5.0);
    }

    #[test]
    fn a_dragged_part_attaches_to_what_it_snaps_to() {
        let f = Frames::new(800.0, FULL);
        let p = Place::new(on_screen(0.0, 100.0), on_screen(0.0, 300.0));
        let size = [100.0, 20.0];
        let b = f.resolve(&p, size, true);
        let snap = Snapping {
            tol: 6.0,
            xs: Vec::new(),
            ys: Vec::new(),
        };
        // Just under the lane, and centred.
        let lane_bottom = LANE_Y + LANE_H / 2.0;
        let (moved, guides) = f.dragged(&p, b, (352.0, lane_bottom + 3.0), Some(&snap));
        assert_eq!(moved.y.target, Target::Lane);
        assert_eq!((moved.y.at, moved.y.pivot), (1.0, 0.0));
        assert_eq!(moved.x.target, Target::Lane);
        assert_eq!((moved.x.at, moved.x.pivot), (0.5, 0.5));
        near(moved.x.off, 0.0);
        near(moved.y.off, 0.0);
        assert_eq!(guides.len(), 2);
        // So a smaller playfield takes it along, centred under the smaller lane.
        let small = Frames::new(800.0, SMALL);
        let there = small.resolve(&moved, size, true);
        near(there.x + there.w / 2.0, 400.0);
        near(there.y, 120.0 + lane_bottom * 0.5);
    }

    #[test]
    fn a_part_dropped_near_the_lane_goes_on_it_from_the_side_its_on() {
        let f = Frames::new(800.0, FULL);
        let p = Place::new(on_screen(0.0, 100.0), on_screen(0.0, 100.0));
        let size = [100.0, 20.0];
        let b = f.resolve(&p, size, true);
        let snap = Snapping {
            tol: 6.0,
            xs: Vec::new(),
            ys: Vec::new(),
        };
        let (top, bottom) = (LANE_Y - LANE_H / 2.0, LANE_Y + LANE_H / 2.0);
        for (to, at_y, at_x) in [
            ((20.0, top - 45.0), 0.0, 0.0),
            ((620.0, top - 45.0), 0.0, 1.0),
            ((20.0, bottom + 20.0), 1.0, 0.0),
            ((340.0, bottom + 20.0), 1.0, 0.5),
        ] {
            let (moved, guides) = f.dragged(&p, b, to, Some(&snap));
            assert!(guides.is_empty(), "nothing to catch at {to:?}");
            assert_eq!(
                (moved.x.target, moved.y.target),
                (Target::Lane, Target::Lane)
            );
            assert_eq!((moved.y.at, moved.x.at), (at_y, at_x), "{to:?}");
            let there = f.resolve(&moved, size, true);
            near(there.x, to.0);
            near(there.y, to.1);
        }
    }

    #[test]
    fn a_part_dropped_away_from_the_lane_isnt_on_it() {
        let f = Frames::new(800.0, FULL);
        let size = [100.0, 20.0];
        let snap = Snapping {
            tol: 6.0,
            xs: Vec::new(),
            ys: Vec::new(),
        };
        let to = (340.0, 400.0);
        // Left where it was attached, off the lane...
        let p = Place::new(on_screen(0.0, 100.0), on_screen(0.0, 100.0));
        let (moved, _) = f.dragged(&p, f.resolve(&p, size, true), to, Some(&snap));
        assert_eq!(
            (moved.x.target, moved.y.target),
            (Target::Screen, Target::Screen)
        );
        // ...and let go by the lane, staying where it's dropped and at its size.
        let on = Place::new(
            axis(Target::Lane, 0.5, 0.5, 0.0),
            axis(Target::Lane, 1.0, 0.0, 0.0),
        );
        let b = f.resolve(&on, size, true);
        let (moved, _) = f.dragged(&on, b, to, Some(&snap));
        assert!(!moved.follows_playfield());
        let there = f.resolve(&moved, size, true);
        near(there.x, to.0);
        near(there.y, to.1);
        near(there.w, b.w);
        // Nothing of the lane catches it there, its ends in line with the screen's or not.
        let edge = (0.0, 400.0);
        let (moved, _) = f.dragged(&p, f.resolve(&p, size, true), edge, Some(&snap));
        assert_eq!(moved.x.target, Target::Playfield);
        // Holding Shift leaves it attached as it was.
        let (moved, _) = f.dragged(&on, b, to, None);
        assert_eq!(
            (moved.x.target, moved.y.target),
            (Target::Lane, Target::Lane)
        );
    }

    #[test]
    fn a_part_keeps_its_size_as_it_snaps_on_a_small_playfield() {
        let f = Frames::new(800.0, SMALL);
        let p = Place::new(on_screen(0.0, 5.0), on_screen(0.0, 5.0));
        let b = f.resolve(&p, [100.0, 20.0], true);
        let snap = Snapping {
            tol: 6.0,
            xs: Vec::new(),
            ys: Vec::new(),
        };
        let (moved, _) = f.dragged(&p, b, (200.0, 120.0), Some(&snap));
        assert!(moved.follows_playfield());
        near(moved.scale, 2.0);
        near(f.resolve(&moved, [100.0, 20.0], true).w, 100.0);
    }

    #[test]
    fn dragging_free_of_snapping_lands_where_the_pointer_says() {
        let f = Frames::new(800.0, SMALL);
        let p = Place::new(
            axis(Target::Lane, 0.0, 0.0, 0.0),
            axis(Target::Lane, 0.0, 1.0, 0.0),
        );
        let b = f.resolve(&p, [100.0, 20.0], true);
        let (moved, guides) = f.dragged(&p, b, (333.0, 77.0), None);
        assert!(guides.is_empty());
        assert_eq!(moved.x.target, Target::Lane);
        let there = f.resolve(&moved, [100.0, 20.0], true);
        near(there.x, 333.0);
        near(there.y, 77.0);
    }

    #[test]
    fn another_parts_line_aligns_without_changing_what_its_attached_to() {
        let f = Frames::new(800.0, FULL);
        let p = Place::new(on_screen(0.0, 100.0), on_screen(0.0, 100.0));
        let b = f.resolve(&p, [50.0, 10.0], true);
        let snap = Snapping {
            tol: 6.0,
            xs: vec![203.0],
            ys: Vec::new(),
        };
        let (moved, guides) = f.dragged(&p, b, (200.0, 70.0), Some(&snap));
        assert_eq!(moved.x.target, Target::Screen);
        near(f.resolve(&moved, [50.0, 10.0], true).x, 203.0);
        assert_eq!(guides.len(), 1);
    }

    #[test]
    fn attaching_an_axis_elsewhere_leaves_the_part_where_it_is() {
        let f = Frames::new(800.0, SMALL);
        let p = Place::new(on_screen(1.0, -40.0), on_screen(0.0, 50.0));
        let size = [80.0, 20.0];
        let b = f.resolve(&p, size, true);
        for t in Target::ALL {
            for horizontal in [true, false] {
                let q = f.attached(&p, b, horizontal, t);
                let there = f.resolve(&q, size, false);
                near(there.x, b.x);
                near(there.y, b.y);
                near(there.w, b.w);
            }
        }
    }

    #[test]
    fn nothing_is_left_off_the_screen() {
        let p = Place::new(
            axis(Target::Lane, 1.0, 0.0, 500.0),
            axis(Target::Lane, 1.0, 0.0, 500.0),
        );
        let b = Frames::new(640.0, FULL).resolve(&p, [100.0, 20.0], true);
        near(b.x, 540.0);
        near(b.y, 460.0);
    }

    #[test]
    fn layouts_round_trip_through_the_config() {
        for (name, layout) in builtin_presets() {
            let line = format_preset(name, &layout);
            let (back, parsed) = parse_preset(&line).unwrap();
            assert_eq!(back, name);
            assert_eq!(parsed.format(), layout.format());
            assert_eq!(parsed.off, layout.off);
        }
        assert_eq!(
            Layout::parse(&Layout::default().format()),
            Some(Layout::default())
        );
        assert_eq!(Layout::parse("not a layout"), None);
        assert_eq!(parse_preset("no layout here"), None);
    }

    #[test]
    fn colours_go_through_the_config_and_tell_layouts_apart() {
        let mut l = Layout::default();
        l.set_colour(Part::Ur, Some(0xff8800));
        let back = Layout::parse(&l.format()).unwrap();
        assert_eq!(back, l);
        assert_eq!(back.colour(Part::Ur), Some(0xff8800));
        assert_ne!(l, Layout::default());
        let bad = Layout::parse("off=|colour:ur=nonsense|colour:score=fffffffff").unwrap();
        assert_eq!(
            (bad.colour(Part::Ur), bad.colour(Part::Score)),
            (None, None)
        );
    }

    #[test]
    fn the_hit_counters_options_go_through_the_config() {
        let mut l = Layout::default();
        assert!(!l.format().contains("counter"), "nothing said till changed");
        l.counter.ok = false;
        l.counter.titles = false;
        assert_eq!(Layout::parse(&l.format()), Some(l));
        l.counter = Counter {
            great: false,
            ok: false,
            miss: false,
            titles: false,
        };
        assert_eq!(Layout::parse(&l.format()), Some(l));
    }

    #[test]
    fn a_layout_from_a_later_version_keeps_what_it_can() {
        let l =
            Layout::parse("off=ur|sparkles=screen,0,0,0,screen,0,0,0,1|score=nonsense").unwrap();
        assert!(!l.shown(Part::Ur) && l.shown(Part::Counts));
        assert_eq!(l.place(Part::Score), None);
    }

    #[test]
    fn the_counters_start_off_and_every_preset_is_distinct() {
        let d = Layout::default();
        assert!(!d.shown(Part::Ur) && !d.shown(Part::Counts) && d.shown(Part::Score));
        let presets = builtin_presets();
        for (i, a) in presets.iter().enumerate() {
            assert!(presets[i + 1..].iter().all(|b| b.1 != a.1), "{}", a.0);
        }
        assert_eq!(presets[0].1, d);
    }

    #[test]
    fn preset_names_cant_break_their_config_line() {
        let name = clean_name("my: layout # 1");
        let (back, _) = parse_preset(&format_preset(&name, &Layout::default())).unwrap();
        assert_eq!(back, name);
    }

    #[test]
    fn every_part_but_the_hit_animation_can_be_placed() {
        for p in Part::ALL {
            assert_eq!(Part::PLACED.contains(&p), p != Part::HitAnimation, "{p:?}");
        }
    }
}
