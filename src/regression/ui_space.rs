//! The interface's own space (`render::Canvas::begin_space`): a screen laid out in more units
//! than the screen's 480 (song select's 720), mapped back onto it. What's drawn, clicked and
//! clipped lands where the same layout in screen units would, the pointer is in the space's
//! units meanwhile, and text and lines snap to whole screen pixels of the space's units.

use crate::render::{self, Canvas, Shape, VIRTUAL_HEIGHT, rgba};

/// A 16:9 screen in its 480 units, the pointer at `mouse`.
fn screen(mouse: (f32, f32)) -> Canvas<u8> {
    Canvas {
        width: VIRTUAL_HEIGHT * 16.0 / 9.0,
        mouse,
        ..Canvas::default()
    }
}

fn rect(s: &Shape) -> [f32; 4] {
    [
        s.center[0] - s.half[0],
        s.center[1] - s.half[1],
        2.0 * s.half[0],
        2.0 * s.half[1],
    ]
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
}

/// Whole pixels, give or take float noise.
fn whole(px: f32) -> bool {
    (px - px.round()).abs() < 1e-3
}

/// A click before any shape, a box with an edge, a clipped run of shapes and a click after it,
/// all at `k` times the sizes given (`k` 1.5: the same layout in 720 units).
fn draw(c: &mut Canvas<u8>, k: f32) {
    c.hit(10.0 * k, 12.0 * k, 50.0 * k, 20.0 * k, 1);
    c.round_rect(
        [10.0 * k, 12.0 * k],
        [50.0 * k, 20.0 * k],
        4.0 * k,
        rgba(0x23232f, 1.0),
        rgba(0x3a3a4e, 1.0),
        k,
    );
    let start = c.shapes.len();
    c.rect(0.0, 40.0 * k, 300.0 * k, 30.0 * k, rgba(0xffffff, 1.0));
    c.rect(20.0 * k, 50.0 * k, 400.0 * k, 10.0 * k, rgba(0xffffff, 1.0));
    c.clip_since(start, 0.0, 40.0 * k, 200.0 * k, 30.0 * k);
    c.hit(20.0 * k, 50.0 * k, 400.0 * k, 10.0 * k, 2);
}

#[test]
fn a_space_as_tall_as_the_screen_draws_as_the_screen_does() {
    let mut plain = screen((5.0, 5.0));
    draw(&mut plain, 1.0);
    let mut spaced = screen((5.0, 5.0));
    let space = spaced.begin_space(VIRTUAL_HEIGHT);
    draw(&mut spaced, 1.0);
    spaced.end_space(space);
    assert_eq!(plain.shapes.len(), spaced.shapes.len());
    for (a, b) in plain.shapes.iter().zip(&spaced.shapes) {
        assert!(
            near(rect(a), rect(b)) && a.params == b.params,
            "{:?} {:?}",
            rect(a),
            rect(b)
        );
    }
    let hits = |c: &Canvas<u8>| c.hits.iter().map(|h| (h.hit, h.rect)).collect::<Vec<_>>();
    assert_eq!(hits(&plain), hits(&spaced));
    assert_eq!(plain.clips, spaced.clips);
}

#[test]
fn a_720_tall_space_lays_out_in_its_units_and_lands_where_the_screens_layout_would() {
    let mut c = screen((300.0, 240.0));
    let full = c.width;
    let space = c.begin_space(render::UI_SPACE_HEIGHT);
    assert!((space.scale() - 2.0 / 3.0).abs() < 1e-6, "480 / 720");
    // 16:9 in 720 units: 1280 wide; the pointer in them too.
    assert!((c.width - 1280.0).abs() < 1e-3, "{}", c.width);
    assert_eq!(c.mouse, (450.0, 360.0));
    assert!(
        c.hovered(440.0, 350.0, 20.0, 20.0),
        "hover in the space's units"
    );
    draw(&mut c, 1.5);
    let at = space.to_screen([1280.0, 720.0]);
    assert!(near(
        [at[0], at[1], 0.0, 0.0],
        [full, VIRTUAL_HEIGHT, 0.0, 0.0]
    ));
    c.end_space(space);
    assert_eq!(c.width, full, "the screen's width back");
    assert_eq!(c.mouse, (300.0, 240.0), "the pointer in screen units again");
    // As if laid out at 1x in screen units: shapes, both clicks (the one made before any shape
    // too), the clip, the box's corner and edge.
    let mut want = screen((300.0, 240.0));
    draw(&mut want, 1.0);
    for (a, b) in c.shapes.iter().zip(&want.shapes) {
        assert!(near(rect(a), rect(b)), "{:?} {:?}", rect(a), rect(b));
        assert!(
            (a.params[0] - b.params[0]).abs() < 1e-3,
            "corner {:?}",
            a.params
        );
        assert!(
            (a.params[1] - b.params[1]).abs() < 1e-3,
            "edge {:?}",
            a.params
        );
    }
    for (a, b) in c.hits.iter().zip(&want.hits) {
        assert!(
            a.hit == b.hit && near(a.rect, b.rect),
            "{:?} {:?}",
            a.rect,
            b.rect
        );
    }
    assert_eq!(c.clips.len(), want.clips.len());
    for ((ra, a), (rb, b)) in c.clips.iter().zip(&want.clips) {
        assert!(ra == rb && near(*a, *b), "{a:?} {b:?}");
    }
}

#[test]
fn in_a_720_space_text_snaps_to_whole_screen_pixels_of_its_smaller_units() {
    // 1080p with text on the pixel grid: 2.25 render pixels per screen unit.
    let ppu = 1080.0 / VIRTUAL_HEIGHT;
    render::set_text_grid(Some(ppu));
    crate::system_font::set_pixel_scale(ppu);
    let mut c = screen((0.0, 0.0));
    let space = c.begin_space(render::UI_SPACE_HEIGHT);
    // A unit of the space is 1.5 pixels (1080 / 720): what snaps is that, not 2.25 * 1.5 (the
    // scale applied the wrong way round) nor 2.25 (the screen's).
    for v in [0.3, 1.0, 2.2, 7.7, 123.4] {
        let s = render::snap_to_grid(v);
        assert!(whole(s * 1.5), "{v} snaps to {s}: {} pixels", s * 1.5);
        assert!(
            (s - v).abs() <= 0.5 / 1.5 + 1e-4,
            "{v} to the nearest pixel: {s}"
        );
    }
    // 0.4 units: 0.6 pixels, so one; one pixel of 2.25 * 1.5 or of 2.25 would be another size.
    let s = render::snap_to_grid(0.4);
    assert!(
        (s - 1.0 / 1.5).abs() < 1e-5,
        "one pixel of 1.5 to the unit: {s}"
    );
    assert!(
        (crate::system_font::pixel_scale() - 1.5).abs() < 1e-6,
        "the font's pixels too"
    );
    // A box snapped in the space lands on whole pixels of the screen.
    let y = render::snap_to_grid(37.3);
    let h = render::pixel_size(1.0);
    c.rect(10.0, y, 100.0, h, rgba(0xffffff, 1.0));
    c.end_space(space);
    let [_, top, _, height] = rect(&c.shapes[0]);
    assert!(
        whole(top * ppu) && whole(height * ppu),
        "{} {}",
        top * ppu,
        height * ppu
    );
    // And the screen's grid back.
    assert!(whole(render::snap_to_grid(1.0) * ppu));
    assert!((crate::system_font::pixel_scale() - ppu).abs() < 1e-6);
    render::set_text_grid(None);
    crate::system_font::set_pixel_scale(1.0);
}

#[test]
fn a_one_unit_line_in_a_720_space_is_at_least_a_whole_pixel_from_720p_up() {
    for screen_h in [720.0, 1080.0, 1440.0] {
        let ppu = screen_h / VIRTUAL_HEIGHT;
        // Without the pixel grid (the usual): a unit is screen_h / 720 pixels, never under one.
        render::set_text_grid(None);
        let mut c = screen((0.0, 0.0));
        let space = c.begin_space(render::UI_SPACE_HEIGHT);
        c.rect(0.0, 100.0, 200.0, 1.0, rgba(0x3a3a4e, 1.0));
        c.end_space(space);
        let px = rect(&c.shapes[0])[3] * ppu;
        assert!(px >= 1.0 - 1e-4, "{screen_h}p: a line {px} pixels thick");
        assert!((px - screen_h / 720.0).abs() < 1e-4);
        // On the grid: a line of one unit made whole pixels, one or more.
        render::set_text_grid(Some(ppu));
        let mut c = screen((0.0, 0.0));
        let space = c.begin_space(render::UI_SPACE_HEIGHT);
        let (y, h) = (render::snap_to_grid(100.2), render::pixel_size(1.0));
        c.rect(0.0, y, 200.0, h, rgba(0x3a3a4e, 1.0));
        c.end_space(space);
        let [_, top, _, height] = rect(&c.shapes[0]);
        let (top, px) = (top * ppu, height * ppu);
        assert!(
            whole(top) && whole(px) && px >= 1.0,
            "{screen_h}p: {px} pixels at {top}"
        );
    }
    render::set_text_grid(None);
}

#[test]
fn the_interface_s_space_is_720_tall_at_scale_one_and_shrinks_as_the_scale_grows() {
    assert_eq!(render::ui_space_height(1.0), 720.0);
    assert_eq!(render::ui_space_height(1.5), 480.0);
    assert_eq!(render::ui_space_height(2.0), 360.0);
    // Out of range or not a number: clamped, or 1.
    assert_eq!(render::ui_space_height(9.0), 360.0);
    assert_eq!(render::ui_space_height(f32::NAN), 720.0);
}
