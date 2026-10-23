//! The pointer: a pixel-art arrow in the interface's colours (white with a red edge, red with a
//! white edge over something clickable), scaled up with hard pixels like the LCD font.

use winit::event_loop::ActiveEventLoop;
use winit::window::{Cursor, CursorIcon, CustomCursor};

/// `X` outline, `w` body, `r` accent edge, `.` clear. The hotspot is the top-left pixel.
const ARROW: [&str; 19] = [
    "X...........",
    "XX..........",
    "XwX.........",
    "XwrX........",
    "XwwrX.......",
    "XwwwrX......",
    "XwwwwrX.....",
    "XwwwwwrX....",
    "XwwwwwwrX...",
    "XwwwwwwwrX..",
    "XwwwwwwwwrX.",
    "XwwwwwwXXXXX",
    "XwwwXwrX....",
    "XwwXXwrX....",
    "XwX..XwrX...",
    "XX...XwrX...",
    "X.....XwrX..",
    "......XwrX..",
    ".......XX...",
];

const OUTLINE: [u8; 4] = [0x12, 0x12, 0x19, 0xff];
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const ACCENT: [u8; 4] = [0xeb, 0x45, 0x2c, 0xff];

/// RGBA pixels of the arrow, each art pixel `scale` x `scale`; `hover` swaps body and edge.
fn arrow_rgba(scale: usize, hover: bool) -> (Vec<u8>, usize, usize) {
    let (w, h) = (ARROW[0].len() * scale, ARROW.len() * scale);
    let (body, edge) = if hover { (ACCENT, WHITE) } else { (WHITE, ACCENT) };
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
    pub fn new(event_loop: &ActiveEventLoop, scale_factor: f64) -> Option<Cursors> {
        let scale = (2.0 * scale_factor).round().clamp(1.0, 8.0) as usize;
        let make = |hover| {
            let (px, w, h) = arrow_rgba(scale, hover);
            CustomCursor::from_rgba(px, w as u16, h as u16, 0, 0).ok().map(|s| event_loop.create_custom_cursor(s))
        };
        Some(Cursors { arrow: make(false)?, hover: make(true)? })
    }

    /// The pixel arrow for the plain and "clickable" pointers; the system ones for resizing
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
        assert_eq!((w, h, px.len()), (36, 57, 36 * 57 * 4));
        // The hotspot pixel is outline, the far corner clear.
        assert_eq!(&px[..4], &OUTLINE);
        assert_eq!(&px[(w - 1) * 4..w * 4], &[0, 0, 0, 0]);
    }
}
