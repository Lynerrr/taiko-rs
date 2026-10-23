//! The playfield area: gameplay renders inside a rectangle of the screen that behaves like a
//! window — drag inside it to move it, drag its edges or corners to resize it, wheel to scale it.
//! Everything inside (lane, HUD, pause menu) is laid out as if that rectangle were the window.

use winit::window::CursorIcon;

use crate::render::{rgba, Canvas, VIRTUAL_HEIGHT};

/// Smallest area, as a fraction of the screen in each direction.
const MIN_SIZE: f32 = 0.2;
/// How close (in pixels) the pointer must be to an edge to grab it.
const GRAB_PX: f64 = 10.0;

/// Position and size as fractions of the screen (0..1), so it survives resolution changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Area {
    pub const FULL: Area = Area { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

    pub fn is_full(&self) -> bool {
        self.w >= 0.999 && self.h >= 0.999
    }

    /// Keeps the area at least `MIN_SIZE` and fully on screen.
    pub fn clamped(mut self) -> Area {
        self.w = self.w.clamp(MIN_SIZE, 1.0);
        self.h = self.h.clamp(MIN_SIZE, 1.0);
        self.x = self.x.clamp(0.0, 1.0 - self.w);
        self.y = self.y.clamp(0.0, 1.0 - self.h);
        self
    }

    pub fn parse(s: &str) -> Option<Area> {
        let v: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        let [x, y, w, h] = v[..] else { return None };
        Some(Area { x, y, w, h }.clamped())
    }

    pub fn to_config(&self) -> String {
        format!("{:.3},{:.3},{:.3},{:.3}", self.x, self.y, self.w, self.h)
    }

    pub fn describe(&self) -> String {
        if self.is_full() {
            "full screen".into()
        } else {
            format!("{:.0}% x {:.0}% at {:.0}%, {:.0}%", self.w * 100.0, self.h * 100.0, self.x * 100.0, self.y * 100.0)
        }
    }

    /// Scales about the centre by `factor`, keeping the shape (wheel).
    pub fn scaled(&self, factor: f32) -> Area {
        let (cx, cy) = (self.x + self.w / 2.0, self.y + self.h / 2.0);
        // Limit the factor so neither side leaves the allowed range and the aspect is kept.
        let max = (1.0 / self.w).min(1.0 / self.h);
        let min = (MIN_SIZE / self.w).max(MIN_SIZE / self.h);
        let f = factor.clamp(min.min(1.0), max.max(1.0));
        let (w, h) = (self.w * f, self.h * f);
        Area { x: cx - w / 2.0, y: cy - h / 2.0, w, h }.clamped()
    }
}

/// What a press at some point grabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Move,
    Resize { left: bool, right: bool, top: bool, bottom: bool },
}

impl Grip {
    pub fn cursor(&self) -> CursorIcon {
        match *self {
            Grip::Move => CursorIcon::Move,
            Grip::Resize { left, right, top, bottom } => match (left || right, top || bottom) {
                (true, true) if (left && top) || (right && bottom) => CursorIcon::NwseResize,
                (true, true) => CursorIcon::NeswResize,
                (true, false) => CursorIcon::EwResize,
                _ => CursorIcon::NsResize,
            },
        }
    }
}

/// Edge/corner within `GRAB_PX` of the pointer, else `Move` inside, else nothing.
pub fn hit(area: Area, px: f64, py: f64, screen_w: f64, screen_h: f64) -> Option<Grip> {
    let (x0, y0) = (area.x as f64 * screen_w, area.y as f64 * screen_h);
    let (x1, y1) = (x0 + area.w as f64 * screen_w, y0 + area.h as f64 * screen_h);
    let in_x = px >= x0 - GRAB_PX && px <= x1 + GRAB_PX;
    let in_y = py >= y0 - GRAB_PX && py <= y1 + GRAB_PX;
    if !(in_x && in_y) {
        return None;
    }
    let left = (px - x0).abs() <= GRAB_PX;
    let right = (px - x1).abs() <= GRAB_PX && !left;
    let top = (py - y0).abs() <= GRAB_PX;
    let bottom = (py - y1).abs() <= GRAB_PX && !top;
    if left || right || top || bottom {
        Some(Grip::Resize { left, right, top, bottom })
    } else if px > x0 && px < x1 && py > y0 && py < y1 {
        Some(Grip::Move)
    } else {
        None
    }
}

/// Applies a drag of (`dx`, `dy`) screen fractions with `grip`, from the area at drag start.
pub fn drag(from: Area, grip: Grip, dx: f32, dy: f32) -> Area {
    let mut a = from;
    match grip {
        Grip::Move => {
            a.x += dx;
            a.y += dy;
        }
        Grip::Resize { left, right, top, bottom } => {
            // Move the grabbed edges, but never past the opposite edge minus the minimum size
            // or off screen, so the far edge stays put like a real window.
            let (r, b) = (from.x + from.w, from.y + from.h);
            if left {
                a.x = (from.x + dx).clamp(0.0, r - MIN_SIZE);
                a.w = r - a.x;
            }
            if right {
                a.w = (from.w + dx).clamp(MIN_SIZE, 1.0 - from.x);
            }
            if top {
                a.y = (from.y + dy).clamp(0.0, b - MIN_SIZE);
                a.h = b - a.y;
            }
            if bottom {
                a.h = (from.h + dy).clamp(MIN_SIZE, 1.0 - from.y);
            }
        }
    }
    a.clamped()
}

/// Starts drawing into `area`: darkens the rest of the screen and switches the canvas to the
/// area's own coordinate space (480 units tall, width from its aspect). Returns the first shape
/// index and the full-screen width for `end`.
pub fn begin(c: &mut Canvas, area: Area) -> (usize, f32) {
    let full = c.width;
    if !area.is_full() {
        c.rect(0.0, 0.0, full, VIRTUAL_HEIGHT, rgba(0x08080b, 1.0));
    }
    let start = c.shapes.len();
    c.width = full * area.w / area.h;
    c.rect(0.0, 0.0, c.width, VIRTUAL_HEIGHT, rgba(0x121219, 1.0));
    (start, full)
}

/// Maps everything drawn since `begin` into the area and restores full-screen drawing. Returns
/// the shape range to clip to the area.
pub fn end(c: &mut Canvas, area: Area, start: usize, full: f32) -> std::ops::Range<usize> {
    c.transform_since(start, [0.0, 0.0], area.h, area.x * full, area.y * VIRTUAL_HEIGHT);
    c.width = full;
    start..c.shapes.len()
}

/// Outline and corner handles, shown while hovering an edge, dragging, or in the editor.
pub fn draw_frame(c: &mut Canvas, area: Area, strong: bool) {
    let (x, y) = (area.x * c.width, area.y * VIRTUAL_HEIGHT);
    let (w, h) = (area.w * c.width, area.h * VIRTUAL_HEIGHT);
    let a = if strong { 0.9 } else { 0.45 };
    c.round_rect(x, y, w, h, 3.0, rgba(0, 0.0), rgba(0xEB452C, a), 1.5);
    for (hx, hy) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
        c.round_rect(hx - 4.0, hy - 4.0, 8.0, 8.0, 2.0, rgba(0xEB452C, a), rgba(0xffffff, a), 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f64 = 1920.0;
    const H: f64 = 1080.0;

    #[test]
    fn grabs_edges_corners_and_inside() {
        let a = Area { x: 0.25, y: 0.25, w: 0.5, h: 0.5 };
        assert_eq!(hit(a, 960.0, 540.0, W, H), Some(Grip::Move));
        assert_eq!(hit(a, 480.0, 540.0, W, H), Some(Grip::Resize { left: true, right: false, top: false, bottom: false }));
        assert_eq!(hit(a, 1440.0, 810.0, W, H), Some(Grip::Resize { left: false, right: true, top: false, bottom: true }));
        assert_eq!(hit(a, 100.0, 100.0, W, H), None);
        assert_eq!(hit(a, 1440.0, 810.0, W, H).unwrap().cursor(), CursorIcon::NwseResize);
    }

    #[test]
    fn resizing_keeps_the_opposite_edge_and_limits() {
        let a = Area { x: 0.25, y: 0.25, w: 0.5, h: 0.5 };
        // Drag the left edge right: right edge stays at 0.75.
        let b = drag(a, Grip::Resize { left: true, right: false, top: false, bottom: false }, 0.1, 0.0);
        assert!((b.x - 0.35).abs() < 1e-6 && (b.x + b.w - 0.75).abs() < 1e-6);
        // Can't shrink below the minimum, can't grow off screen.
        let c = drag(a, Grip::Resize { left: false, right: true, top: false, bottom: true }, -0.9, 0.9);
        assert!((c.w - MIN_SIZE).abs() < 1e-6);
        assert!((c.y + c.h - 1.0).abs() < 1e-6);
        // Moving stops at the screen edge.
        let d = drag(a, Grip::Move, -1.0, 0.0);
        assert_eq!(d.x, 0.0);
        assert_eq!(d.w, 0.5);
    }

    #[test]
    fn wheel_scales_about_the_centre() {
        let a = Area { x: 0.25, y: 0.25, w: 0.5, h: 0.5 };
        let b = a.scaled(0.5);
        assert!((b.w - 0.25).abs() < 1e-6 && (b.x - 0.375).abs() < 1e-6);
        assert_eq!(Area::FULL.scaled(1.5), Area::FULL);
        assert_eq!(Area::parse(&a.to_config()), Some(a));
    }
}
