//! The taiko.rs logo: a glitched don note with "rs" on it, as pixel art (`assets/icon-*.png`).
//! The window and taskbar use it as their icon, the top bar draws it pixel by pixel.

use std::sync::OnceLock;

use crate::render::Canvas;

const ICON_32: &[u8] = include_bytes!("../assets/icon-32.png");
const ICON_64: &[u8] = include_bytes!("../assets/icon-64.png");

/// The window icon (Windows and X11; Wayland takes it from the desktop entry).
pub fn window_icon() -> Option<winit::window::Icon> {
    let img = image::load_from_memory(ICON_64).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

type PixelRun = (u32, u32, u32, [f32; 4]);

/// The 32 px logo as horizontal runs of one colour: (x, y, length, colour).
fn runs() -> &'static [PixelRun] {
    static RUNS: OnceLock<Vec<PixelRun>> = OnceLock::new();
    RUNS.get_or_init(|| {
        let Ok(img) = image::load_from_memory(ICON_32) else { return Vec::new() };
        let img = img.to_rgba8();
        let mut out: Vec<PixelRun> = Vec::new();
        for (x, y, p) in img.enumerate_pixels() {
            if p[3] == 0 {
                continue;
            }
            let col = p.0.map(|v| v as f32 / 255.0);
            match out.last_mut() {
                Some(run) if run.1 == y && run.0 + run.2 == x && run.3 == col => run.2 += 1,
                _ => out.push((x, y, 1, col)),
            }
        }
        out
    })
}

/// Draws the logo `size` units square with its top left at (`x`, `y`).
pub fn draw(c: &mut Canvas, x: f32, y: f32, size: f32) {
    let px = size / 32.0;
    for &(rx, ry, len, col) in runs() {
        // A hair oversized so neighbouring runs leave no seams.
        c.rect(x + rx as f32 * px, y + ry as f32 * px, len as f32 * px + 0.2, px + 0.2, col);
    }
}
