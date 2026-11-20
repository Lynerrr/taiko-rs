//! The pills and the selection's brackets as song select drew them in 1.0, for its look under
//! Settings > System > Legacy menu (`Config::legacy_menu`).

use super::*;

pub(crate) fn selector_colour() -> u32 {
    if kiai() > 0.5 { GOLD } else { render::accent() }
}

pub(crate) fn brackets(c: &mut Canvas, [x, y, w, h]: [f32; 4], alpha: f32) {
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
    for (cx, sx) in [(l, 1.0), (r, -1.0)] {
        for (cy, sy) in [(top, 1.0), (bot, -1.0)] {
            let hx = if sx > 0.0 { cx } else { cx - arm };
            let vy = if sy > 0.0 { cy } else { cy - arm };
            c.rect(hx, if sy > 0.0 { cy } else { cy - t }, arm, t, col);
            c.rect(if sx > 0.0 { cx } else { cx - t }, vy, t, arm, col);
        }
    }
}

pub(crate) fn star_pill(c: &mut Canvas, right: f32, y: f32, size: f32, stars: Option<f32>) -> f32 {
    let text = stars
        .map(|s| format!("{s:.2}"))
        .unwrap_or_else(|| "...".into());
    rating_pill(c, right, y, size, stars, &text)
}

pub(crate) fn map_pill(
    c: &mut Canvas,
    right: f32,
    y: f32,
    size: f32,
    level: Option<f32>,
    stars: Option<f32>,
) -> f32 {
    match level {
        Some(level) => rating_pill(c, right, y, size, Some(level), &level_text(level)),
        None => star_pill(c, right, y, size, stars),
    }
}

fn rating_pill(
    c: &mut Canvas,
    right: f32,
    y: f32,
    size: f32,
    stars: Option<f32>,
    text: &str,
) -> f32 {
    let text = text.to_string();
    let tw = Canvas::text_width(size, &text);
    let (w, h) = (tw + size * 1.6 + 6.0, size + 6.0);
    let x = right - w;
    let (bg, fg) = match stars {
        Some(s) => {
            let col = crate::stars::color(s);
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
