//! Filtering the installed font picker keeps original choice indices for keys and clicks.
use super::harness;
use crate::*;

struct ResetFont;
impl Drop for ResetFont {
    fn drop(&mut self) {
        system_font::select("");
    }
}

fn choices() -> Choices {
    Choices::new(
        Item::UiFont,
        [
            "built-in",
            "Arial",
            "Arial Black",
            "Noto Sans",
            "游ゴシック",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        Some(3),
        String::new(),
    )
}

#[test]
fn font_search_filters_case_insensitively_and_navigation_keeps_original_indices() {
    lang::set(lang::Lang::En);
    let mut p = choices();
    p.set_search("aRiAl".into());
    assert_eq!(p.visible(), [1, 2]);
    assert_eq!(p.selected(), Some(1));
    p.move_cursor(1);
    assert_eq!(p.selected(), Some(2));
    p.move_cursor(100);
    assert_eq!(p.selected(), Some(2));
    p.move_cursor(-100);
    assert_eq!(p.selected(), Some(1));
    p.set_search("Arial Black".into());
    assert_eq!(p.visible(), [2]);
    assert_eq!(
        p.current,
        Some(3),
        "search doesn't change the saved selection"
    );
    p.set_search("missing font".into());
    assert!(p.visible().is_empty());
    assert_eq!(p.selected(), None);
    p.move_cursor(-100);
    assert_eq!(p.selected(), None);
    let mut c = hit::Canvas {
        audit: true,
        ..Default::default()
    };
    c.clear(853.3);
    p.draw(&mut c);
    assert!(c.texts.iter().any(|(_, s)| s == "No fonts found"));
    assert!(
        !c.hits
            .iter()
            .any(|h| matches!(h.hit, hit::Hit::ChoiceRow(_)))
    );
    p.set_search("ゴシック".into());
    assert_eq!(p.visible(), [4]);
    p.set_search(String::new());
    assert_eq!(p.visible(), [0, 1, 2, 3, 4]);
}

#[test]
fn filtered_font_rows_draw_and_click_with_the_original_family_index() {
    let _reset = ResetFont;
    let mut s = harness::state_with(harness::config(), Vec::new());
    let mut now = Instant::now();
    harness::ready(&mut s, &mut now);
    s.open_settings();
    s.open_choices(Item::UiFont);
    assert!(
        s.takes_text(),
        "font search accepts text and clipboard paste"
    );
    harness::type_text(&mut s, "aBe", now);
    let p = s.settings_ui.choices.as_ref().unwrap();
    let i = p.names.iter().position(|name| name == "Abel").unwrap();
    assert_eq!(p.visible(), [i]);
    harness::frame(&mut s, now);
    now += Duration::from_secs(1);
    let c = harness::frame(&mut s, now);
    assert!(c.hits.iter().any(|h| h.hit == hit::Hit::ChoiceSearch));
    assert!(c.hits.iter().any(|h| h.hit == hit::Hit::ChoiceRow(i)));
    assert!(!c.hits.iter().any(|h| h.hit == hit::Hit::ChoiceRow(0)));
    let search = c
        .hits
        .iter()
        .find(|h| h.hit == hit::Hit::ChoiceSearch)
        .unwrap();
    let [x, y, w, h] = search.rect;
    s.mouse = (
        ((x + w / 2.0) / c.width * s.window_w) as f64,
        ((y + h / 2.0) / 480.0 * s.window_h) as f64,
    );
    s.mouse_button(true, None);
    s.mouse_button(false, None);
    assert_eq!(
        s.settings_ui.choices.as_ref().unwrap().search,
        "aBe",
        "clicking the search keeps the font picker open"
    );
    s.click(hit::Hit::ChoiceRow(i), None);
    assert_eq!(s.cfg.ui_font, "Abel");
    assert!(s.settings_ui.choices.is_none());
    s.open_choices(Item::UiFont);
    assert!(s.settings_ui.choices.as_ref().unwrap().search.is_empty());
    assert_eq!(s.settings_ui.choices.as_ref().unwrap().current, Some(i));
}

#[test]
fn font_search_empty_results_enter_backspace_space_and_escape_are_safe() {
    let _reset = ResetFont;
    let mut s = harness::state_with(harness::config(), Vec::new());
    let mut now = Instant::now();
    harness::ready(&mut s, &mut now);
    s.open_settings();
    s.open_choices(Item::UiFont);
    harness::type_text(&mut s, "no such family", now);
    harness::press(&mut s, KeyCode::Enter, now);
    assert!(s.settings_ui.choices.is_some());
    assert!(s.cfg.ui_font.is_empty());
    harness::press(&mut s, KeyCode::Escape, now);
    assert!(s.settings_ui.choices.as_ref().unwrap().search.is_empty());
    harness::type_text(&mut s, "A", now);
    harness::press(&mut s, KeyCode::Space, now);
    assert_eq!(s.settings_ui.choices.as_ref().unwrap().search, "A ");
    harness::press(&mut s, KeyCode::Backspace, now);
    assert_eq!(s.settings_ui.choices.as_ref().unwrap().search, "A");
    harness::press(&mut s, KeyCode::Backspace, now);
    harness::press(&mut s, KeyCode::Backspace, now);
    assert!(
        s.settings_ui.choices.is_some(),
        "Backspace on an empty search keeps the picker open"
    );
    harness::type_text(&mut s, "Ab", now);
    harness::press(&mut s, KeyCode::Enter, now);
    assert_eq!(s.cfg.ui_font, "Abel");
    s.open_choices(Item::UiFont);
    harness::press(&mut s, KeyCode::Escape, now);
    assert!(s.settings_ui.choices.is_none());
    assert!(s.in_settings());
}

#[test]
fn font_search_does_not_change_other_settings_lists() {
    let mut p = Choices::new(
        Item::Language,
        vec!["English".into(), "日本語".into()],
        Some(0),
        String::new(),
    );
    assert!(!p.searchable());
    p.move_cursor(1);
    assert_eq!(p.selected(), Some(1));
    let mut c = hit::Canvas::default();
    c.clear(853.3);
    p.draw(&mut c);
    assert!(!c.hits.iter().any(|h| h.hit == hit::Hit::ChoiceSearch));
}

#[test]
fn phone_font_search_draws_filtered_rows_and_empty_state() {
    lang::set(lang::Lang::En);
    system_font::select("");
    render::set_text_grid(None);
    let mut s = harness::state_with(harness::config(), Vec::new());
    let mut p = choices();
    p.set_search("Arial Black".into());
    s.settings_ui.choices = Some(p);
    let mut c = hit::Canvas {
        audit: true,
        ..Default::default()
    };
    c.clear(1066.7);
    s.draw_mobile_choices(&mut c);
    assert!(c.hits.iter().any(|h| h.hit == hit::Hit::ChoiceSearch));
    let rows: Vec<_> = c
        .hits
        .iter()
        .filter_map(|h| {
            if let hit::Hit::ChoiceRow(i) = h.hit {
                Some(i)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(rows, [2]);
    super::snapshot::assert_snapshot("phone-font-picker-search", &harness::screen(&c, &[]));
    s.settings_ui
        .choices
        .as_mut()
        .unwrap()
        .set_search("nothing matches".into());
    s.mob.reset_choices_scroll();
    c.clear(1066.7);
    s.draw_mobile_choices(&mut c);
    assert!(
        !c.hits
            .iter()
            .any(|h| matches!(h.hit, hit::Hit::ChoiceRow(_)))
    );
    assert!(c.texts.iter().any(|(_, s)| s == "No fonts found"));
}

#[test]
fn desktop_font_picker_search_snapshots() {
    lang::set(lang::Lang::En);
    system_font::select("");
    render::set_text_grid(None);
    let mut p = choices();
    let mut c = hit::Canvas {
        audit: true,
        ..Default::default()
    };
    for (name, query) in [
        ("font-picker-all", ""),
        ("font-picker-search", "Arial"),
        ("font-picker-empty", "no-such-font"),
    ] {
        p.set_search(query.into());
        c.clear(853.3);
        p.draw(&mut c);
        super::snapshot::assert_snapshot(name, &harness::screen(&c, &[]));
    }
}

/// Discord idea: with many skins installed, the skin list and each skin part's list can be
/// searched like the font list, typing straight into it; Enter takes the row found, and
/// Backspace with nothing typed still goes from a part's skins back to the parts.
#[test]
fn skin_lists_are_searchable_and_backspace_still_goes_back_to_the_parts() {
    lang::set(lang::Lang::En);
    let skins = [
        "built-in",
        "Aqua Drums",
        "Blue Lagoon",
        "aqua classic",
        "Ember",
    ];
    let mut p = Choices::new(
        Item::Skin,
        skins.iter().map(|s| s.to_string()).collect(),
        Some(0),
        String::new(),
    );
    assert!(p.searchable());
    p.set_search("AQUA".into());
    assert_eq!(p.visible(), [1, 3]);
    assert_eq!(p.selected(), Some(1));
    let mut c = hit::Canvas::default();
    c.clear(853.3);
    p.draw(&mut c);
    assert!(c.hits.iter().any(|h| h.hit == hit::Hit::ChoiceSearch));

    // Through the settings screen: one part's skins, typed into.
    let mut s = harness::state_with(harness::config(), Vec::new());
    harness::frame(&mut s, Instant::now());
    s.open_settings();
    s.settings_ui.choices = Some(Choices::new(
        Item::SkinPart(0),
        skins.iter().map(|s| s.to_string()).collect(),
        Some(0),
        String::new(),
    ));
    let typed = |s: &mut State, code: KeyCode, text: Option<&str>| {
        s.key(code, false, text, Instant::now(), None);
    };
    typed(&mut s, KeyCode::KeyE, Some("em"));
    let list = s.settings_ui.choices.as_ref().unwrap();
    assert_eq!(list.search, "em");
    assert_eq!(list.visible(), [4], "Ember alone");
    typed(&mut s, KeyCode::Backspace, None);
    typed(&mut s, KeyCode::Backspace, None);
    let list = s.settings_ui.choices.as_ref().unwrap();
    assert_eq!(list.search, "", "Backspace takes what's typed first");
    assert!(matches!(list.item, Item::SkinPart(0)));
    typed(&mut s, KeyCode::Backspace, None);
    assert!(
        matches!(
            s.settings_ui.choices.as_ref().map(|c| c.item),
            Some(Item::SkinParts)
        ),
        "then back to the parts"
    );
}
