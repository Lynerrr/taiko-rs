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

/// Reset thread-local drawing state even if an assertion fails.
struct ResetFont;
impl Drop for ResetFont {
    fn drop(&mut self) {
        crate::system_font::select("");
        crate::system_font::set_pixel_scale(1.0);
    }
}

#[test]
fn installed_font_draws_and_measures_proportional_text_with_pixel_fallback() {
    use crate::{hit::Canvas, system_font};
    let _reset = ResetFont;
    assert!(system_font::select("Abel"));
    assert!(Canvas::text_width(14.0, "WWW") > Canvas::text_width(14.0, "iii"));
    assert!(system_font::glyph('谱').is_none());
    for audit in [false, true] {
        let mut canvas = Canvas {
            audit,
            ..Default::default()
        };
        let text = "Wi, Привет! 谱面";
        let drawn = canvas.text(0.0, 0.0, 14.0, render::rgba(0xffffff, 1.0), text);
        assert!((drawn - Canvas::text_width(14.0, text)).abs() < 1e-3);
        assert!(!canvas.shapes.is_empty());
        if audit {
            assert_eq!(canvas.texts.len(), 1);
            assert_eq!(canvas.texts[0].1, text);
            assert!(
                canvas
                    .shapes
                    .iter()
                    .any(|s| s.fill[3] > 0.0 && s.fill[3] < 1.0)
            );
        } else {
            assert!(
                canvas
                    .shapes
                    .iter()
                    .any(|s| s.params[3] >= system_font::BASE)
            );
            assert!(
                canvas
                    .shapes
                    .iter()
                    .any(|s| s.params[3] < system_font::BASE)
            );
        }
    }
    let room = Canvas::text_width(14.0, "Wi");
    assert_eq!(Canvas::fitting_prefix(14.0, room, "Wi谱"), 2);
    let fit = Canvas::fit(14.0, room * 3.0, "WiWiWiWiWiWiWi");
    assert!(Canvas::text_width(14.0, &fit) <= room * 3.0 + 1e-3);
    for line in Canvas::wrap_words(14.0, room * 3.0, "Wide words 谱面 Wide") {
        assert!(Canvas::text_width(14.0, &line) <= room * 3.0 + 1e-3);
    }
    assert!(system_font::select(""));
    assert_eq!(
        Canvas::text_width(14.0, "WWW"),
        Canvas::text_width(14.0, "iii")
    );
    assert!(!system_font::select("Missing Font Family"));
    assert!(!system_font::active());
    assert_eq!(Canvas::text_width(14.0, "WWW"), 36.0);
}

#[test]
fn glyph_cache_reuses_bitmaps_and_refreshes_after_font_switch() {
    use crate::system_font;
    let _reset = ResetFont;
    assert!(system_font::select("Abel"));
    let first = system_font::glyph('W').unwrap();
    assert!(std::rc::Rc::ptr_eq(
        &first,
        &system_font::glyph('W').unwrap()
    ));
    assert!(system_font::uploads(first.revision).is_empty());
    assert!(system_font::select(""));
    assert!(system_font::select("Abel"));
    let next = system_font::glyph('W').unwrap();
    assert!(next.revision > first.revision);
    assert_eq!(system_font::uploads(first.revision).len(), 1);
    assert_eq!(first.pixels, next.pixels);
}

#[test]
fn font_setting_offers_installed_families_and_restores_default() {
    use crate::*;
    let _reset = ResetFont;
    let mut s = layout_tests::state();
    s.open_choices(Item::UiFont);
    let list = s.settings_ui.choices.as_ref().unwrap();
    assert_eq!(list.current, Some(0));
    let i = list.names.iter().position(|name| name == "Abel").unwrap();
    s.pick_choice(i);
    assert_eq!(s.cfg.ui_font, "Abel");
    assert!(system_font::active());
    s.open_choices(Item::UiFont);
    assert_eq!(s.settings_ui.choices.as_ref().unwrap().current, Some(i));
    s.pick_choice(0);
    assert!(s.cfg.ui_font.is_empty());
    assert!(!system_font::active());
}

#[test]
fn selected_font_is_applied_on_the_next_frame_and_can_be_reset() {
    use crate::*;
    let _reset = ResetFont;
    let mut s = super::harness::state_with(super::harness::config(), Vec::new());
    let mut ready_at = Instant::now();
    super::harness::ready(&mut s, &mut ready_at);
    s.open_settings();
    s.settings_ui.cursor = settings::ITEMS
        .iter()
        .position(|&(_, item)| item == Item::UiFont)
        .unwrap();
    s.cfg.ui_font = "Abel".into();
    let now = Instant::now();
    let mut c = hit::Canvas {
        audit: true,
        ..Default::default()
    };
    s.build_frame(&mut c, now, 853.3);
    s.build_frame(&mut c, now + Duration::from_secs(1), 853.3);
    assert!(system_font::active());
    assert!(c.texts.iter().any(|(_, text)| text == "Interface font"));
    assert!(c.texts.iter().any(|(_, text)| text.contains("Abel")));
    if let Some(dir) = std::env::var_os("TAIKO_LAYOUT_SHOTS") {
        let (w, h, rgb) = debug::rasterize(&c, 2.0);
        debug::write_png(
            &std::path::Path::new(&dir).join("system-font-settings.png"),
            w,
            h,
            &rgb,
        )
        .unwrap();
    }
    s.cfg.ui_font.clear();
    s.build_frame(&mut c, now + Duration::from_secs(2), 853.3);
    assert!(!system_font::active());
    assert_eq!(hit::Canvas::text_width(14.0, "WWW"), 36.0);
}

#[test]
fn scaling_and_rotating_custom_text_preserve_atlas_coordinates() {
    use crate::{hit::Canvas, system_font};
    let _reset = ResetFont;
    assert!(system_font::select("Abel"));
    let mut c = Canvas::default();
    c.text(20.0, 20.0, 14.0, render::rgba(0xffffff, 1.0), "Wi");
    let original = c.shapes.clone();
    assert!(!original.is_empty());
    for scale in [0.5, 1.5, 3.0] {
        c.shapes = original.clone();
        c.transform_since(0, [0.0; 2], scale, 5.0, 8.0);
        for (before, after) in original.iter().zip(&c.shapes) {
            assert_eq!(
                after.params, before.params,
                "atlas dimensions are texture pixels"
            );
            assert_eq!(after.half, before.half.map(|n| n * scale));
        }
        c.shapes = original.clone();
        render::rotate_shapes(&mut c.shapes, [0.0; 2], 0.7, scale, [4.0; 2]);
        for (before, after) in original.iter().zip(&c.shapes) {
            assert_eq!(
                after.params, before.params,
                "rotation must preserve atlas dimensions"
            );
            assert_eq!(after.half, before.half.map(|n| n * scale));
        }
    }
}

#[test]
fn custom_font_raster_resolution_tracks_text_size_and_display_density() {
    use crate::{hit::Canvas, system_font};
    let _reset = ResetFont;
    assert!(system_font::select("Abel"));
    let expected_width = Canvas::text_width(48.0, "SETTINGS");
    for density in [1.0, 2.0, 3.0, 4.0] {
        system_font::set_pixel_scale(density);
        let mut c = Canvas::default();
        let drawn = c.text(10.25, 10.25, 48.0, render::rgba(0xffffff, 1.0), "SETTINGS");
        assert!((drawn - expected_width).abs() < 1e-3);
        assert_eq!(Canvas::text_width(48.0, "SETTINGS"), expected_width);
        for shape in &c.shapes {
            let physical_height = shape.half[1] * 2.0 * density;
            assert!(
                (physical_height / shape.params[1] - 1.0).abs() < 0.025,
                "large text must sample a native-resolution raster, not enlarge a 32px bitmap"
            );
            let x = (shape.center[0] - shape.half[0]) * density;
            let y = (shape.center[1] - shape.half[1]) * density;
            assert!((x - x.round()).abs() < 1e-3);
            assert!((y - y.round()).abs() < 1e-3);
        }
    }
    system_font::set_pixel_scale(3.0);
    let small = system_font::glyph_at('S', 8.0).unwrap();
    let large = system_font::glyph_at('S', 48.0).unwrap();
    assert!(large.metrics.height > small.metrics.height * 4);
    assert!(std::rc::Rc::ptr_eq(
        &large,
        &system_font::glyph_at('S', 48.0).unwrap()
    ));
}
