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
const POINTS: usize = 48;

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
            let sway = (std::f32::consts::PI * p).sin() * WAVE * (std::f32::consts::TAU * v).sin();
            LanePoint {
                x: ax + (bx - ax) * p,
                y: mid + (row[1] + row[3] / 2.0 - mid) * p + sway,
                h: h0 + (row[3] - h0) * p,
                p,
            }
        })
        .collect()
}

/// The lane as a band through `points`: the panel's fill, its edges along both sides. Where it
/// turns, the pieces meet in round joints (as a thick line drawn with a round pen), and the edges
/// leave off where the turn is sharp.
pub fn draw_lane(c: &mut Canvas, points: &[LanePoint], alpha: f32) {
    if alpha <= 0.0 || points.len() < 2 {
        return;
    }
    let fill = rgba(style::PANEL, alpha);
    let edge = rgba(style::PANEL_EDGE, alpha);
    let angle_at = |i: usize| {
        let (a, b) = (points[i], points[i + 1]);
        (b.y - a.y).atan2(b.x - a.x)
    };
    let turn = |a: f32, b: f32| {
        let d = (b - a).rem_euclid(std::f32::consts::TAU);
        d.min(std::f32::consts::TAU - d)
    };
    let last = points.len() - 2;
    // The joints first, under the pieces.
    for (i, p) in points.iter().enumerate().take(last + 1).skip(1) {
        if turn(angle_at(i - 1), angle_at(i)) > 0.02 {
            c.circle(p.x, p.y, p.h / 2.0, fill, fill, 0.0);
        }
    }
    for i in 0..=last {
        let (a, b) = (points[i], points[i + 1]);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len = dx.hypot(dy);
        if len < 0.01 {
            continue;
        }
        let angle = angle_at(i);
        let mid = [(a.x + b.x) / 2.0, (a.y + b.y) / 2.0];
        let h = (a.h + b.h) / 2.0;
        let w = len + 1.0;
        let start = c.shapes.len();
        c.rect(mid[0] - w / 2.0, mid[1] - h / 2.0, w, h, fill);
        let sharp = [i.checked_sub(1), (i < last).then_some(i + 1)]
            .iter()
            .flatten()
            .any(|&j| turn(angle_at(j), angle) > 0.25);
        if !sharp {
            c.rect(mid[0] - w / 2.0, mid[1] - h / 2.0, w, 1.5, edge);
            c.rect(mid[0] - w / 2.0, mid[1] + h / 2.0 - 1.5, w, 1.5, edge);
        }
        c.rotate_since(start, mid, angle, 1.0, mid);
    }
    // The left end's edge.
    let a = points[0];
    c.rect(a.x, a.y - a.h / 2.0, 1.5, a.h, edge);
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
