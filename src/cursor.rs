//! The pointer: a pixel-art arrowhead in the interface's colours (light with an accent edge,
//! accent with a light edge over something clickable), scaled up with hard pixels like the LCD font.

use winit::event_loop::ActiveEventLoop;
use winit::window::{Cursor, CursorIcon, CustomCursor};

/// `X` outline, `w` body, `r` accent edge, `.` clear: an arrowhead pointing up-left. The
/// hotspot is the tip.
const ARROW: [&str; 13] = [
    "XXX..........",
    "XwXXXX.......",
    "XXwwwXXXX....",
    ".XwwwwwwXXXX.",
    ".XwwwwwwwwwXX",
    ".XXwwwwwwrrrX",
    "..XwwwwwrrXXX",
    "..XwwwwrrXX..",
    "..XXwwrrXX...",
    "...XwrrXX....",
    "...XwrXX.....",
    "...XXrX......",
    "....XXX......",
];

const OUTLINE: [u8; 4] = [0x12, 0x12, 0x19, 0xff];
const WHITE: [u8; 4] = [0xe8, 0xe8, 0xf2, 0xff];

/// RGBA pixels of the arrow, each art pixel `scale` x `scale`; `hover` swaps body and edge.
fn arrow_rgba(scale: usize, hover: bool) -> (Vec<u8>, usize, usize) {
    let (w, h) = (ARROW[0].len() * scale, ARROW.len() * scale);
    let a = crate::render::accent();
    let accent = [(a >> 16) as u8, (a >> 8) as u8, a as u8, 0xff];
    let (body, edge) = if hover { (accent, WHITE) } else { (WHITE, accent) };
    let mut px = vec![0u8; w * h * 4];
    for (y, row) in ARROW.iter().enumerate() {
        for (x, ch) in row.bytes().enumerate() {
            let color = match ch {
                b'X' => OUTLINE,
                b'w' => body,
                b'r' => edge,
                _ => continue,
            };
            for dy in 0..scale {
                let start = ((y * scale + dy) * w + x * scale) * 4;
                for p in px[start..start + scale * 4].chunks_exact_mut(4) {
                    p.copy_from_slice(&color);
                }
            }
        }
    }
    (px, w, h)
}

pub struct Cursors {
    arrow: CustomCursor,
    hover: CustomCursor,
}

impl Cursors {
    /// Art pixels are 2 screen pixels at 100% scaling, more on HiDPI screens.
    /// In the current `render::accent`; make them again when it changes.
    pub fn new(event_loop: &ActiveEventLoop, scale_factor: f64) -> Option<Cursors> {
        let scale = (2.0 * scale_factor).round().clamp(1.0, 8.0) as usize;
        let make = |hover| {
            let (px, w, h) = arrow_rgba(scale, hover);
            let tip = (scale / 2) as u16;
            CustomCursor::from_rgba(px, w as u16, h as u16, tip, tip).ok().map(|s| event_loop.create_custom_cursor(s))
        };
        Some(Cursors { arrow: make(false)?, hover: make(true)? })
    }

    /// The pixel arrowhead for the plain and "clickable" pointers; the system ones for resizing
    /// and moving the playfield, where the shape says which way it goes.
    pub fn get(&self, icon: CursorIcon) -> Cursor {
        match icon {
            CursorIcon::Default => self.arrow.clone().into(),
            CursorIcon::Pointer => self.hover.clone().into(),
            other => other.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_is_rectangular_and_scales() {
        assert!(ARROW.iter().all(|r| r.len() == ARROW[0].len()));
        let (px, w, h) = arrow_rgba(3, false);
        assert_eq!((w, h, px.len()), (39, 39, 39 * 39 * 4));
        // The hotspot pixel is outline, the far corner clear.
        assert_eq!(&px[..4], &OUTLINE);
        assert_eq!(&px[(w - 1) * 4..w * 4], &[0, 0, 0, 0]);
    }
}
