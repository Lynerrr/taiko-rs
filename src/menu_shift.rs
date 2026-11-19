//! The way between the main menu and song select. The menu's lane doesn't end at the screen's
//! right edge: it runs on to where song select already stands. Play moves the camera right along
//! it; on the way the part of the lane that comes into view rises and folds up into song select's
//! search row, the song list drops down out of that row as one window with it, and the Play note
//! flies to the Play button, shedding its disc on the way. Everything goes at once. Back to the
//! menu runs it all the other way, a little faster.

use crate::hit::Canvas;
use crate::menu::{self, Item};
use crate::render::{self, rgba};
use crate::style;

/// How long the way to song select takes (ms), and the way back, of that.
pub const MS: f32 = 900.0;
pub const BACK: f32 = 0.7;

/// When each part starts and ends, as parts of the whole way (0 to 1).
const CAM_FROM: f32 = 0.02;
const CAM_TO: f32 = 0.55;
const MORPH_FROM: f32 = 0.12;
const MORPH_TO: f32 = 0.6;
const DROP_FROM: f32 = 0.38;
const FLIGHT_FROM: f32 = 0.05;
const FLIGHT_LEN: f32 = 0.42;
/// How much later the lane's left end folds up than its right (of the fold): what's under the
/// menu's notes stays under them until they're gone.
const STAGGER: f32 = 0.3;
/// How far the lane sways as it rises (units).
const WAVE: f32 = 10.0;
/// How far past the screen's right edge the menu's lane runs, beyond a second screen's width.
const LANE_PAST: f32 = 340.0;
/// Points along the lane as it folds.
const POINTS: usize = 72;

/// A way between the two under way.
pub struct Shift {
    /// Towards song select (else back to the menu).
    pub into_songs: bool,
    /// How far towards song select (0 on the menu, 1 on song select), whichever way it goes.
    t: f32,
    /// The menu that was left, drawn as the camera leaves it (the way to song select only:
    /// going back, the menu is the client's own again).
    pub menu: Option<Box<menu::Menu>>,
    /// The note that was hit, flying to its button (the way to song select only).
    pub flying: Option<Item>,
}

impl Shift {
    /// Off to song select from the menu `left` (hit on `item`), from wherever a way back was.
    pub fn into_songs(left: Option<menu::Menu>, item: Item, from: Option<&Shift>) -> Shift {
        Shift {
            into_songs: true,
            t: from.map_or(0.0, |s| s.t),
            menu: left.map(Box::new),
            // Only from the start: a note that's halfway back doesn't fly out again.
            flying: from.is_none().then_some(item),
        }
    }

    /// Back to the menu, from wherever a way to song select was.
    pub fn back(from: Option<&Shift>) -> Shift {
        Shift {
            into_songs: false,
            t: from.map_or(1.0, |s| s.t),
            menu: None,
            flying: None,
        }
    }

    /// A frame `dt_ms` on. False once it's over.
    pub fn step(&mut self, dt_ms: f32) -> bool {
        if self.into_songs {
            self.t += dt_ms / MS;
            self.t < 1.0
        } else {
            self.t -= dt_ms / (MS * BACK);
            self.t > 0.0
        }
    }

    pub fn t(&self) -> f32 {
        self.t.clamp(0.0, 1.0)
    }
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Past 1 and back, like something dropped that bounces once.
fn out_back(t: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2)
}

/// Where each part of the way is at `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phases {
    /// The camera: 0 on the menu, 1 on song select.
    pub cam: f32,
    /// The lane folding up into the search row (0 to 1).
    pub fold: f32,
    /// The search row's own things coming on over the folded lane (0 to 1).
    pub landed: f32,
    /// How far the list has dropped out of the search row (past 1 as it bounces).
    pub drop: f32,
    /// The hit note on its way to its button (0 to 1).
    pub flight: f32,
}

pub fn phases(t: f32) -> Phases {
    let t = clamp01(t);
    let fold = clamp01((t - MORPH_FROM) / (MORPH_TO - MORPH_FROM));
    // The lane's left end is the last to land.
    let last = in_out_cubic(clamp01((fold - STAGGER) / (1.0 - STAGGER)));
    let drop = clamp01((t - DROP_FROM) / (1.0 - DROP_FROM));
    Phases {
        cam: in_out_cubic(clamp01((t - CAM_FROM) / (CAM_TO - CAM_FROM))),
        fold,
        landed: clamp01((last - 0.8) / 0.2),
        drop: if drop > 0.0 { out_back(drop) } else { 0.0 },
        flight: in_out_cubic(clamp01((t - FLIGHT_FROM) / FLIGHT_LEN)),
    }
}

/// A point of the lane: its middle, its height, and how far it has folded (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LanePoint {
    pub x: f32,
    pub y: f32,
    pub h: f32,
    pub p: f32,
}

/// The menu's lane at `t` on a screen `width` wide, as it folds up into the search row `row`
/// (`[x, y, w, h]`, on song select's screen): the camera has moved `cam` of a screen right along
/// it. The part that was on the menu's screen goes off to the left with the camera, the notes
/// still on it, and gathers at the row's left end last; what comes into view folds up into the
/// row, its right end first.
pub fn lane(t: f32, width: f32, row: [f32; 4]) -> Vec<LanePoint> {
    let ph = phases(t);
    let (x0, mid, h0) = (menu::panel_x(), menu::lane_mid(), menu::lane_h());
    let len = 2.0 * width + LANE_PAST;
    let on_menu = (width - x0) / len;
    (0..=POINTS)
        .map(|i| {
            let u = i as f32 / POINTS as f32;
            let v = clamp01((u - on_menu) / (1.0 - on_menu));
            let p = in_out_cubic(clamp01((ph.fold - STAGGER * (1.0 - v)) / (1.0 - STAGGER)));
            let (ax, bx) = (x0 + u * len - ph.cam * width, row[0] + v * row[2]);
            // A wave runs along it as it moves over, as in the prototype.
            let sway = (std::f32::consts::PI * p).sin()
                * WAVE
                * (std::f32::consts::TAU * (1.3 * v - 0.7 * ph.fold)).sin();
            LanePoint {
                x: ax + (bx - ax) * p,
                y: mid + (row[1] + row[3] / 2.0 - mid) * p + sway,
                h: h0 + (row[3] - h0) * p,
                p,
            }
        })
        .collect()
}

/// The kiai's gold, as the menu lights its panel.
const KIAI_GOLD: u32 = 0xFFD34D;

/// The lane as a ribbon through `points`, as the prototype draws it: across it at each point is
/// the way between its neighbours turned a quarter, so it bends smoothly and comes to a point
/// where it folds back on itself; the panel's fill, an edge along both sides with a faint second
/// line just inside each, its left end closed and its right end closing as it lands. `kiai`: how
/// gold its edges and fill are (0 to 1 each, as `menu::Menu::kiai_glow`), the menu's kiai carried
/// along with it.
pub fn draw_lane(c: &mut Canvas, points: &[LanePoint], alpha: f32, kiai: (f32, f32)) {
    if alpha <= 0.0 || points.len() < 2 {
        return;
    }
    band(
        c,
        points,
        rgba(style::PANEL, alpha),
        rgba(style::PANEL_EDGE, alpha),
        false,
    );
    let (edge, fill) = (kiai.0 * alpha, kiai.1 * alpha);
    if edge > 0.003 || fill > 0.003 {
        band(
            c,
            points,
            rgba(KIAI_GOLD, fill),
            rgba(KIAI_GOLD, edge),
            true,
        );
    }
}

/// How far the ribbon's outer side may run on between two of its pieces (units): where it turns,
/// a stretch is cut into as many pieces as keep the gaps on the outside of the turn this small.
const RIBBON_STEP: f32 = 1.0;

/// `over`: laid over the lane, see-through, so its pieces mustn't overdraw where it's straight,
/// or the seams would show brighter.
fn band(c: &mut Canvas, points: &[LanePoint], fill: [f32; 4], edge: [f32; 4], over: bool) {
    let n = points.len();
    // The way along the lane at each point (an angle), from its neighbours.
    let along: Vec<f32> = (0..n)
        .map(|i| {
            let (a, b) = (points[i.saturating_sub(1)], points[(i + 1).min(n - 1)]);
            (b.y - a.y).atan2(b.x - a.x)
        })
        .collect();
    let faint = [edge[0], edge[1], edge[2], edge[3] * 0.25];
    // A line from `a` to `b`, `w` thick.
    let line = |c: &mut Canvas, a: [f32; 2], b: [f32; 2], w: f32, colour: [f32; 4]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        if len < 0.01 {
            return;
        }
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let start = c.shapes.len();
        c.rect(mid[0] - len / 2.0, mid[1] - w / 2.0, len, w, colour);
        c.rotate_since(start, mid, dy.atan2(dx), 1.0, mid);
    };
    // A point of the ribbon `f` of the way from point `i` to the next: its middle, half its
    // height and the way along it.
    let at = |i: usize, f: f32| {
        let (a, b) = (points[i], points[i + 1]);
        (
            [a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f],
            (a.h + (b.h - a.h) * f) / 2.0,
            along[i] + wrap(along[i + 1] - along[i]) * f,
        )
    };
    // Its two sides there, `off` in from each.
    let side = |(m, h, ang): ([f32; 2], f32, f32), off: f32| {
        let (o, d) = (h - off, [-ang.sin(), ang.cos()]);
        (
            [m[0] - d[0] * o, m[1] - d[1] * o],
            [m[0] + d[0] * o, m[1] + d[1] * o],
        )
    };
    // Each stretch between two points, in pieces short enough round its turn.
    let pieces: Vec<(usize, f32, f32, f32)> = (0..n - 1)
        .flat_map(|i| {
            let (a, b) = (points[i], points[i + 1]);
            let turn = wrap(along[i + 1] - along[i]).abs();
            let count = ((turn * a.h.max(b.h) / 2.0 / RIBBON_STEP).ceil() as usize).clamp(1, 64);
            (0..count).map(move |k| {
                let f = |k: usize| k as f32 / count as f32;
                (i, f(k), f(k + 1), turn)
            })
        })
        .collect();
    // The fill first, all of it, so no piece of it covers the edges of the one before.
    for &(i, f0, f1, turn) in &pieces {
        let (p0, p1) = (at(i, f0), at(i, f1));
        let (mid, h, ang) = at(i, (f0 + f1) / 2.0);
        // Along the lane, the piece is as long as its middle runs; where the lane turns, longer
        // by as much as its sides spread from the next piece's, so none of the background shows
        // through between them (inside the turn, the overlap is under the lane anyway).
        let run = (p1.0[0] - p0.0[0]).hypot(p1.0[1] - p0.0[1]);
        let spread = h * wrap(p1.2 - p0.2).abs();
        let w = run
            + if over && turn < 0.02 {
                0.0
            } else {
                spread + 1.5
            };
        let start = c.shapes.len();
        c.rect(mid[0] - w / 2.0, mid[1] - h, w, 2.0 * h, fill);
        c.rotate_since(start, mid, ang, 1.0, mid);
    }
    for &(i, f0, f1, _) in &pieces {
        let (p0, p1) = (at(i, f0), at(i, f1));
        let ang = at(i, (f0 + f1) / 2.0).2;
        // On the inside of a turn tighter than the lane is wide, an edge would run backwards,
        // over the lane folded there: it's left out, as the lane covers it.
        let forwards =
            |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * ang.cos() + (b[1] - a[1]) * ang.sin() >= 0.0;
        for (off, w, colour) in [(0.75, 1.5, edge), (5.0, 1.0, faint)] {
            let ((t0, b0), (t1, b1)) = (side(p0, off), side(p1, off));
            for (a, b) in [(t0, t1), (b0, b1)] {
                if forwards(a, b) {
                    line(c, a, b, w, colour);
                }
            }
        }
    }
    // The left end, closed: the drum's side. The right end closes as it lands in the search row.
    let end = |c: &mut Canvas, i: usize, f: f32, a: f32| {
        let (t, b) = side(at(i, f), 0.0);
        line(c, t, b, 1.5, [edge[0], edge[1], edge[2], edge[3] * a]);
    };
    end(c, 0, 0.0, 1.0);
    let last = points[n - 1];
    if last.p > 0.0 {
        end(c, n - 2, 1.0, last.p);
    }
}

/// `a` brought into -π..π.
fn wrap(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a + PI).rem_euclid(TAU) - PI
}

/// The hit note on its way from its place on the menu (`from`: middle and radius) to its button
/// (`to`: the icon's middle and size), `f` of the way: it sheds its disc as it goes, and what
/// lands is the bare icon, split red and cyan in the air.
pub fn draw_flight(
    c: &mut Canvas,
    item: Item,
    (from, r0): ([f32; 2], f32),
    (to, size): ([f32; 2], f32),
    f: f32,
) {
    if f <= 0.0 || f >= 1.0 {
        return;
    }
    let ctrl = [(from[0] + to[0]) / 2.0, from[1].min(to[1]) - 30.0];
    let bez = |a: f32, b: f32, k: f32| (1.0 - f).powi(2) * a + 2.0 * (1.0 - f) * f * k + f * f * b;
    let (x, y) = (bez(from[0], to[0], ctrl[0]), bez(from[1], to[1], ctrl[1]));
    // The icon is 16 of the note's 31 art pixels across.
    let r1 = size * menu::NOTE_ART_R / 16.0;
    let r = r0 + (r1 - r0) * f;
    let shed = clamp01(f * 1.6);
    if shed < 1.0 {
        let start = c.shapes.len();
        menu::note(c, item, x, y, r);
        c.fade_since(start, (1.0 - shed).powf(1.5), 1.0);
    }
    let icon = item.icon_art();
    let is = 16.0 * r / menu::NOTE_ART_R;
    let air = (std::f32::consts::PI * f).sin();
    for (col, side) in [(0x9f2137, -1.0), (0x2a9fb5, 1.0)] {
        style::icon(
            c,
            icon,
            [x + side * 3.0 * air, y],
            is,
            col,
            0.7 * shed * air,
        );
    }
    style::icon(c, icon, [x, y], is, 0xffffff, shed);
}

/// Whether the way between the menu and song select is animated at all (else it's a cut, as
/// on a phone, whose screens are laid out otherwise).
pub fn animated() -> bool {
    render::animations() && !crate::mobile::active()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_starts_at_once_and_is_done_at_the_end() {
        let start = phases(0.0);
        assert_eq!(
            (
                start.cam,
                start.fold,
                start.landed,
                start.drop,
                start.flight
            ),
            (0.0, 0.0, 0.0, 0.0, 0.0)
        );
        let early = phases(0.1);
        assert!(
            early.cam > 0.0 && early.flight > 0.0,
            "the camera and the note set off together"
        );
        let end = phases(1.0);
        for v in [end.cam, end.fold, end.landed, end.flight, end.drop] {
            assert!((v - 1.0).abs() < 1e-4, "{end:?}");
        }
        // The list bounces past its place on the way down.
        assert!((0..100).any(|i| phases(i as f32 / 100.0).drop > 1.0));
    }

    #[test]
    fn the_lane_starts_as_the_menus_panel_and_ends_as_the_search_row() {
        let (w, row) = (853.3, [20.0, 14.0, 452.0, 34.0]);
        let first = lane(0.0, w, row);
        assert!(first.iter().all(|p| p.p == 0.0));
        assert!((first[0].x - menu::panel_x()).abs() < 0.01);
        assert!(first.iter().all(|p| (p.y - menu::lane_mid()).abs() < 0.01));
        assert!(
            first.last().unwrap().x > w,
            "it runs on past the screen's edge"
        );
        let last = lane(1.0, w, row);
        for p in &last {
            assert!((p.y - 31.0).abs() < 0.01 && (p.h - 34.0).abs() < 0.01);
            assert!(p.x >= 20.0 - 0.01 && p.x <= 472.0 + 0.01);
        }
        assert!((last.last().unwrap().x - 472.0).abs() < 0.01);
        // It never tears: neighbouring points stay close all the way, however it bends.
        for t in 0..=20 {
            let pts = lane(t as f32 / 20.0, w, row);
            for p in pts.windows(2) {
                let gap = (p[1].x - p[0].x).hypot(p[1].y - p[0].y);
                assert!(gap < 80.0, "at {t}: {gap}");
            }
        }
    }

    #[test]
    fn the_lane_carries_a_kiai_s_gold_along_with_it_and_none_without_one() {
        let pts = lane(0.3, 853.3, [20.0, 14.0, 452.0, 34.0]);
        let gold = |c: &Canvas| {
            c.shapes
                .iter()
                .filter(|s| {
                    let [r, g, b, a] = s.fill.map(|v| v.max(0.0));
                    a > 0.0 && r > 0.0 && g / r > 0.7 && b / r < 0.5
                })
                .count()
        };
        let mut plain = Canvas::default();
        draw_lane(&mut plain, &pts, 1.0, (0.0, 0.0));
        assert_eq!(gold(&plain), 0);
        let mut lit = Canvas::default();
        draw_lane(&mut lit, &pts, 1.0, (0.8, 0.12));
        assert!(gold(&lit) >= pts.len() - 1, "every piece of it");
        assert!(
            lit.shapes.len() > plain.shapes.len(),
            "over the lane, not instead"
        );
    }

    #[test]
    fn the_lane_folds_up_as_one_ribbon_its_edges_unbroken_round_the_fold() {
        let pts = lane(0.35, 853.3, [20.0, 14.0, 452.0, 34.0]);
        let along = |a: LanePoint, b: LanePoint| (b.y - a.y).atan2(b.x - a.x);
        let heading: Vec<f32> = pts.windows(2).map(|w| along(w[0], w[1])).collect();
        let steepest = heading.iter().fold(0.0f32, |m, a| m.max(a.abs()));
        assert!(
            steepest > std::f32::consts::FRAC_PI_3,
            "it bends steeply up here: {steepest}"
        );
        let mut c = Canvas::default();
        draw_lane(&mut c, &pts, 1.0, (0.0, 0.0));
        let edge = rgba(style::PANEL_EDGE, 1.0);
        // The ends of every piece of its edges.
        let ends: Vec<[f32; 2]> = c
            .shapes
            .iter()
            .filter(|s| s.fill == edge && (s.half[1] - 0.75).abs() < 1e-3)
            .flat_map(|s| {
                let (cos, sin) = (s.angle.cos() * s.half[0], s.angle.sin() * s.half[0]);
                [
                    [s.center[0] - cos, s.center[1] - sin],
                    [s.center[0] + cos, s.center[1] + sin],
                ]
            })
            .collect();
        // At every point an edge runs on through it, on both sides: on the inside of a turn
        // tighter than the lane is wide only, where that side would run backwards, the lane
        // folded over it covers it.
        let side = |i: usize, s: f32| {
            let (p, a) = (
                pts[i],
                along(pts[i.saturating_sub(1)], pts[(i + 1).min(pts.len() - 1)]),
            );
            let o = p.h / 2.0 - 0.75;
            ([p.x - a.sin() * o * s, p.y + a.cos() * o * s], a)
        };
        let mut checked = 0;
        for i in 1..pts.len() - 1 {
            for s in [-1.0, 1.0] {
                let ((at, a), (before, _), (after, _)) =
                    (side(i, s), side(i - 1, s), side(i + 1, s));
                let on = (after[0] - before[0]) * a.cos() + (after[1] - before[1]) * a.sin();
                if on <= 0.0 {
                    continue;
                }
                checked += 1;
                assert!(
                    ends.iter()
                        .any(|e| (e[0] - at[0]).hypot(e[1] - at[1]) < 1.0),
                    "no edge at point {i} on side {s}: {at:?}"
                );
            }
        }
        assert!(checked > 2 * (pts.len() - 2) * 9 / 10, "{checked}");
        // And nothing round in it: a fold comes to a point, not a knob.
        assert!(c.shapes.iter().all(|s| s.params[0] == 0.0));
    }

    #[test]
    fn a_way_back_halfway_turns_round_where_it_is() {
        let mut into = Shift::into_songs(None, Item::Play, None);
        assert!(into.step(MS / 2.0));
        let back = Shift::back(Some(&into));
        assert!((back.t() - 0.5).abs() < 1e-4);
        assert!(back.flying.is_none());
        let mut back = back;
        assert!(back.step(MS * BACK * 0.25));
        assert!(!back.step(MS * BACK * 0.3), "back is quicker");
        let again = Shift::into_songs(None, Item::Play, Some(&back));
        assert!(again.flying.is_none(), "a note doesn't fly out halfway");
    }
}
