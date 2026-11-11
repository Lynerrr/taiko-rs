//! Coverage and measurement for glyphs drawn by the client.

use crate::render;

#[test]
fn a_chinese_map_title_draws_at_its_measured_width() {
    let title = "谱面";
    let mut canvas = crate::hit::Canvas {
        audit: true,
        ..Default::default()
    };
    assert!(title.chars().all(crate::font::drawable));
    let drawn = canvas.text(0.0, 0.0, 14.0, render::rgba(0xffffff, 1.0), title);
    assert_eq!(drawn, crate::hit::Canvas::text_width(14.0, title));
    assert_eq!(drawn, 32.0);
}
