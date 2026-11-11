//! The rails mods' lanes: the shapes the lane bends into, and the path along one.

use super::*;

/// Rails: the lane bent into a curve. Distances along it (`s`) count from the hit target; it
/// leaves the hit target heading right, so the drum stays where it is, and behind the hit
/// target it runs straight on to the left.
pub struct Rail {
    /// Position and heading (radians, anticlockwise from right) every `RAIL_STEP` from s = 0.
    pub pts: Vec<(f32, f32, f32)>,
}
pub const RAIL_STEP: f32 = 4.0;
impl Rail {
    /// From its curvature (radians per unit) along the way, `len` long.
    #[inline]
    pub fn build(bend: impl Fn(f32) -> f32, len: f32) -> Rail {
        let n = (len / RAIL_STEP).ceil() as usize + 2;
        let mut pts = Vec::with_capacity(n);
        let (mut x, mut y, mut a) = (HIT_X, LANE_Y, 0.0f32);
        for k in 0..n {
            pts.push((x, y, a));
            let turn = bend((k as f32 + 0.5) * RAIL_STEP) * RAIL_STEP;
            let mid = a + turn / 2.0;
            x += mid.cos() * RAIL_STEP;
            y -= mid.sin() * RAIL_STEP;
            a += turn;
        }
        Rail { pts }
    }

    /// Point and heading `s` along.
    #[inline]
    pub fn at(&self, s: f32) -> (f32, f32, f32) {
        if s <= 0.0 {
            return (HIT_X + s, LANE_Y, 0.0);
        }
        let f = s / RAIL_STEP;
        let k = f as usize;
        let Some(&(x1, y1, a1)) = self.pts.get(k + 1) else {
            let (x, y, a) = *self.pts.last().unwrap_or(&(HIT_X, LANE_Y, 0.0));
            let d = s - (self.pts.len() - 1) as f32 * RAIL_STEP;
            return (x + a.cos() * d, y - a.sin() * d, a);
        };
        let (x0, y0, a0) = self.pts[k];
        let u = f - k as f32;
        (x0 + (x1 - x0) * u, y0 + (y1 - y0) * u, a0 + (a1 - a0) * u)
    }
}
/// The lane shapes of the rails mods.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RailShape {
    Straight,
    Zigzag,
    UTurn,
    Spiral,
    Loop,
    Elbow,
    Maze,
    Meadow,
}
/// Maze: the lane runs along a grid of `MAZE_CELL` squares, turning a quarter at each step of
/// `MAZE` (cells straight on, then +1 left / -1 right) round a corner of radius `MAZE_R`, and
/// leaves `MAZE_EXIT` off the right of the screen. From the drum: a long straight (read like
/// an ordinary lane, nothing beside it), then up and down the columns on the right (below the
/// top row past the fifth: the score, accuracy and HP are over it) and out to the right.
pub const MAZE_CELL: f32 = 160.0;
pub const MAZE_R: f32 = 80.0;
pub(crate) const MAZE: [(f32, f32); 10] = [
    (4.0, 1.0),
    (2.0, -1.0),
    (1.0, -1.0),
    (4.0, 1.0),
    (1.0, 1.0),
    (3.0, -1.0),
    (1.0, -1.0),
    (3.0, 1.0),
    (1.0, 1.0),
    (3.0, -1.0),
];
pub const MAZE_EXIT: f32 = 500.0;
/// Maze: the straight before each corner of `MAZE` (the corners take `MAZE_R` off each end).
pub fn maze_straight(k: usize) -> f32 {
    MAZE[k].0 * MAZE_CELL - if k == 0 { MAZE_R } else { 2.0 * MAZE_R }
}
/// Loop: how far from the hit target it starts, its radius, and the angle the lane into it comes
/// up at (short of a full turn by that). Full size on a 16:9 screen, with the lane coming in right
/// of the hit error meter.
pub const LOOP_AT: f32 = 390.0;
pub const LOOP_R: f32 = 110.0;
pub(crate) const LOOP_EXIT: f32 = 0.6;
pub const LOOP_TURN: f32 = std::f32::consts::TAU - LOOP_EXIT;
/// Meadow: square-cornered runs that cross back over themselves, as a straight run then a
/// quarter turn (1 left, -1 right, 0 none) each. Traced from a tower-defence track, with the first
/// run long enough that the loops below the lane clear the drum. Both loops' holes are the same
/// size (runs 2 and 4, 1 and 5 alike), and the lanes stacked under the drum's have the same gaps
/// between them (but for the corners' 2.6-unit rounding: runs are whole steps).
pub(crate) const MEADOW: [(f32, f32); 12] = [
    (516.0, 1.0),
    (68.0, 1.0),
    (96.0, 1.0),
    (424.0, -1.0),
    (96.0, -1.0),
    (68.0, -1.0),
    (528.0, 1.0),
    (76.0, -1.0),
    (20.0, -1.0),
    (220.0, -1.0),
    (288.0, 1.0),
    (212.0, 0.0),
];
/// Meadow: how long each corner is. A whole number of `RAIL_STEP`s, like every run, so each turns
/// exactly a quarter (the rail samples its curvature a step at a time) and the runs stay square.
/// Its radius, about 71, is over half the lane, or the inside of a corner would fold over.
pub(crate) const MEADOW_TURN: f32 = 28.0 * RAIL_STEP;
/// Meadow: how far right it reaches (lane edge included), how much it shrinks besides to fit the
/// screen's height (clear of the hit error meter but for the lane out), and how far down the
/// playfield moves.
pub(crate) const MEADOW_RIGHT: f32 = 1163.0;
pub(crate) const MEADOW_TALL: f32 = 0.53;
pub(crate) const MEADOW_DROP: f32 = 28.0;
/// The bent shapes are laid out from a lane at 160: this takes them back up there from `LANE_Y`.
pub(crate) const RAIL_LIFT: f32 = LANE_Y - 160.0;
/// Shapeshift's shapes, by the index `shapeshift::SHAPES` names them with.
pub const SHAPESHIFT: [RailShape; 7] = [
    RailShape::Zigzag,
    RailShape::UTurn,
    RailShape::Spiral,
    RailShape::Loop,
    RailShape::Elbow,
    RailShape::Straight,
    RailShape::Meadow,
];
/// Shapeshift shape `k`'s lane on a 16:9 screen, as points along its middle (for the setup's
/// thumbnails).
pub fn rail_outline(k: usize) -> Vec<(f32, f32)> {
    let shape = SHAPESHIFT[k % SHAPESHIFT.len()];
    let w = 853.3;
    let rail = Rail::build(|s| shape.bend(s), shape.length(w));
    rail.pts
        .iter()
        .step_by(3)
        .map(|&(x, y, _)| (x, y))
        .collect()
}
/// The lane shapes a .tko map's lane clips name (`tko::clips::LANE_SHAPES`), in that order.
pub const LANE_SHAPES_OF_CLIPS: [RailShape; 6] = [
    RailShape::Zigzag,
    RailShape::UTurn,
    RailShape::Spiral,
    RailShape::Loop,
    RailShape::Elbow,
    RailShape::Meadow,
];
/// The lane `u` (0..1) of the way from shape `from` to `to` on a canvas `w` wide: rail, length,
/// and how far the playfield moves down and shrinks for it.
pub fn morph_rail(from: RailShape, to: RailShape, u: f32, w: f32) -> (Option<Rail>, f32, f32, f32) {
    let ease = |x: f32| smooth(0.0, 1.0, x as f64) as f32;
    // The Loop can't be bent into or out of: half way its lane would swing round across the
    // drum and the HUD. So the other shape straightens out over the first half of the morph and
    // the loop slides in whole from off the right of the screen over the second (the other way
    // round on the way out). The lane's length doesn't jump in between.
    let lp = RailShape::Loop;
    if (from == lp) != (to == lp) {
        let other = if to == lp { from } else { to };
        // 0: the other shape, 1: straight with the loop off screen, 2: the loop in place.
        let p = 2.0 * if to == lp { u } else { 1.0 - u };
        let away = 1.5 * (w - HIT_X);
        if p < 1.0 {
            let k = ease(p);
            let mix = |a: f32, b: f32| a + (b - a) * k;
            let len = mix(other.length(w), lp.length(w) + away);
            return (
                Some(Rail::build(|s| other.bend(s) * (1.0 - k), len)),
                len,
                other.drop() * (1.0 - k),
                mix(other.zoom(w), 1.0),
            );
        }
        let k = ease(p - 1.0);
        let shift = (1.0 - k) * away;
        let len = lp.length(w) + shift;
        return (
            Some(Rail::build(|s| lp.bend(s - shift), len)),
            len,
            lp.drop() * k,
            1.0 + (lp.zoom(w) - 1.0) * k,
        );
    }
    let m = ease(u);
    let mix = |a: f32, b: f32| a + (b - a) * m;
    let len = mix(from.length(w), to.length(w));
    (
        Some(Rail::build(|s| mix(from.bend(s), to.bend(s)), len)),
        len,
        mix(from.drop(), to.drop()),
        mix(from.zoom(w), to.zoom(w)),
    )
}
impl RailShape {
    #[inline]
    pub fn of(mods: Mods) -> Option<RailShape> {
        [
            (Mod::Zigzag, RailShape::Zigzag),
            (Mod::UTurn, RailShape::UTurn),
            (Mod::Spiral, RailShape::Spiral),
            (Mod::Loop, RailShape::Loop),
            (Mod::Elbow, RailShape::Elbow),
            (Mod::Maze, RailShape::Maze),
        ]
        .into_iter()
        .find(|&(m, _)| mods.has(m))
        .map(|(_, shape)| shape)
    }

    /// Curvature (radians per unit, anticlockwise) `s` along the lane.
    pub fn bend(self, s: f32) -> f32 {
        use std::f32::consts::PI;
        // Turning by `angle` over `len` units from `at`.
        let turn = |at: f32, len: f32, angle: f32| {
            if (at..at + len).contains(&s) {
                angle / len
            } else {
                0.0
            }
        };
        match self {
            RailShape::Straight => 0.0,
            RailShape::Zigzag => {
                // Up 30 degrees, then 60-degree corners every 150 units.
                if s < 120.0 {
                    return 0.0;
                }
                let k = ((s - 120.0) / 150.0).floor();
                let angle = if k == 0.0 {
                    PI / 6.0
                } else if k as i64 % 2 == 1 {
                    -PI / 3.0
                } else {
                    PI / 3.0
                };
                turn(120.0 + 150.0 * k, 36.0, angle)
            }
            RailShape::UTurn => turn(240.0, PI * 100.0, PI),
            // Far enough out that its turns pass well right of the hit target, each a lane wider
            // than the last.
            RailShape::Spiral => {
                if s < 340.0 {
                    0.0
                } else {
                    1.0 / (90.0 + 0.15 * (s - 340.0))
                }
            }
            // All the way round but for `LOOP_EXIT`: the lane coming in crosses the straight to
            // the drum at that angle, not along it (notes a second apart side by side).
            RailShape::Loop => turn(LOOP_AT, LOOP_TURN * LOOP_R, LOOP_TURN),
            RailShape::Elbow => turn(220.0, PI * 40.0, PI / 2.0),
            // The average over the `RAIL_STEP` round `s` (what `Rail::build` turns by), so each
            // corner turns exactly a quarter: a dozen of them a little off and the far end of
            // the maze would wander tens of units out of place.
            RailShape::Maze => {
                let arc = PI / 2.0 * MAZE_R;
                let (lo, hi) = (s - RAIL_STEP / 2.0, s + RAIL_STEP / 2.0);
                let mut at = 0.0;
                let mut turned = 0.0;
                for (k, &(_, side)) in MAZE.iter().enumerate() {
                    at += maze_straight(k);
                    if at >= hi {
                        break;
                    }
                    turned += side * (hi.min(at + arc) - lo.max(at)).max(0.0);
                    at += arc;
                }
                turned / (RAIL_STEP * MAZE_R)
            }
            RailShape::Meadow => {
                let mut at = 0.0;
                for (run, dir) in MEADOW {
                    at += run;
                    if (at..at + MEADOW_TURN).contains(&s) {
                        return dir * PI / 2.0 / MEADOW_TURN;
                    }
                    at += MEADOW_TURN;
                }
                0.0
            }
        }
    }

    /// How long the lane is on a canvas `w` wide.
    #[inline]
    pub fn length(self, w: f32) -> f32 {
        use std::f32::consts::PI;
        match self {
            RailShape::Straight => w - HIT_X,
            RailShape::Zigzag => (w - HIT_X) * 1.15,
            // Back along the top to the left edge.
            RailShape::UTurn => 240.0 + PI * 100.0 + HIT_X + 240.0,
            RailShape::Spiral => 1320.0,
            // On out of the bottom right of the screen.
            RailShape::Loop => LOOP_AT + LOOP_TURN * LOOP_R + 500.0,
            RailShape::Elbow => 220.0 + PI * 40.0 + 300.0,
            RailShape::Maze => {
                (0..MAZE.len()).map(maze_straight).sum::<f32>()
                    + MAZE.len() as f32 * PI / 2.0 * MAZE_R
                    + MAZE_EXIT
            }
            // Out of the bottom of the screen.
            RailShape::Meadow => {
                MEADOW
                    .iter()
                    .map(|&(run, _)| run + MEADOW_TURN)
                    .sum::<f32>()
                    - MEADOW_TURN
            }
        }
    }

    /// How far down the playfield moves to keep the shape on screen.
    #[inline]
    pub fn drop(self) -> f32 {
        match self {
            RailShape::Straight => 0.0,
            RailShape::Zigzag => 35.0 - RAIL_LIFT,
            RailShape::UTurn => 150.0 - RAIL_LIFT,
            RailShape::Spiral => 105.0 - RAIL_LIFT,
            RailShape::Loop => 135.0 - RAIL_LIFT,
            RailShape::Elbow => 200.0 - RAIL_LIFT,
            // The drum half way down: the maze reaches as far above it as below.
            RailShape::Maze => 65.0 - RAIL_LIFT,
            RailShape::Meadow => MEADOW_DROP - RAIL_LIFT,
        }
    }

    /// How much the playfield shrinks to fit the shape on a canvas `w` wide: the Spiral is
    /// tall, and narrow playfields squeeze the rest about the hit target.
    #[inline]
    pub fn zoom(self, w: f32) -> f32 {
        // How far right the shape reaches, lane edge included.
        let right = match self {
            RailShape::Straight | RailShape::Zigzag => return 1.0,
            RailShape::UTurn => 560.0,
            RailShape::Spiral => 660.0,
            RailShape::Loop => HIT_X + LOOP_AT + LOOP_R + LANE_H / 2.0,
            RailShape::Elbow => 520.0,
            RailShape::Maze => HIT_X + 8.0 * MAZE_CELL + LANE_H / 2.0,
            RailShape::Meadow => MEADOW_RIGHT,
        };
        // Maze: two and a half cells above the drum and below it, between the HUD at the top
        // and the hit error meter.
        let tall: f32 = match self {
            RailShape::Spiral => 0.85,
            RailShape::Maze => 0.48,
            RailShape::Meadow => MEADOW_TALL,
            _ => 1.0,
        };
        tall.min((w - 8.0 - HIT_X) / (right - HIT_X)).max(0.3)
    }
}
